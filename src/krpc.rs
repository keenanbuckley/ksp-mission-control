use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use krpc_client::{
    services::{krpc::KRPC, space_center::SpaceCenter},
    stream::Stream,
    Client,
};
use ksp_mission_control::control;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{info, warn};

const STREAM_RATE_HZ: f32 = 5.0;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TelemetryFrame {
    Ut(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Calendar {
    pub secs_per_day: f64,
    pub secs_per_year: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnStatus {
    Disconnected,
    Connected { calendar: Calendar },
}

const KERBIN_CALENDAR: Calendar = Calendar {
    secs_per_day: 21_600.0,
    secs_per_year: 9_201_600.0,
};

const EARTH_CALENDAR: Calendar = Calendar {
    secs_per_day: 86_400.0,
    secs_per_year: 31_536_000.0,
};

async fn detect_calendar(krpc: Arc<Client>) -> Result<Calendar> {
    let space_center = SpaceCenter::new(krpc);
    let bodies = space_center.get_bodies().await?;
    if bodies.contains_key("Kerbin") {
        Ok(KERBIN_CALENDAR)
    } else if bodies.contains_key("Earth") {
        Ok(EARTH_CALENDAR)
    } else {
        warn!(
            "neither Kerbin nor Earth found in SpaceCenter.Bodies; \
             defaulting to Kerbin calendar"
        );
        Ok(KERBIN_CALENDAR)
    }
}

pub async fn run_telemetry_supervisor(
    host: String,
    rpc_port: u16,
    stream_port: u16,
    telemetry_tx: broadcast::Sender<TelemetryFrame>,
    status_tx: watch::Sender<ConnStatus>,
    mut command_rx: mpsc::Receiver<serde_json::Value>,
) {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        status_tx.send_if_modified(|s| {
            if matches!(s, ConnStatus::Disconnected) {
                false
            } else {
                *s = ConnStatus::Disconnected;
                true
            }
        });

        let client = match Client::new("ksp-mission-control", &host, rpc_port, stream_port).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    error = %e,
                    backoff_secs = backoff.as_secs(),
                    "kRPC connect failed; retrying"
                );
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff);
                continue;
            }
        };
        info!(host = %host, port = rpc_port, "connected to kRPC");

        let calendar = match detect_calendar(client.clone()).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "calendar detect failed; retrying");
                drop(client);
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff);
                continue;
            }
        };
        info!(
            secs_per_day = calendar.secs_per_day,
            secs_per_year = calendar.secs_per_year,
            "calendar detected"
        );

        let connected = ConnStatus::Connected { calendar };
        status_tx.send_if_modified(|s| {
            if *s == connected {
                false
            } else {
                *s = connected.clone();
                true
            }
        });
        backoff = INITIAL_BACKOFF;

        if let Err(e) = run_session(client, telemetry_tx.clone(), &mut command_rx).await {
            warn!(error = format!("{e:#}"), "kRPC session ended; reconnecting");
        }
    }
}

fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

async fn run_session(
    client: Arc<Client>,
    tx: broadcast::Sender<TelemetryFrame>,
    command_rx: &mut mpsc::Receiver<serde_json::Value>,
) -> Result<()> {
    let space_center = SpaceCenter::new(client.clone());
    let krpc = KRPC::new(client.clone());
    let stream = space_center.get_ut_stream().await?;
    stream.set_rate(STREAM_RATE_HZ).await?;

    tokio::select! {
        res = run_stream_loop(&stream, &tx) => res,
        res = run_heartbeat(&krpc) => res,
        res = run_dispatcher(&client, command_rx) => res,
    }
}

async fn run_dispatcher(
    client: &Arc<Client>,
    command_rx: &mut mpsc::Receiver<serde_json::Value>,
) -> Result<()> {
    while let Some(cmd) = command_rx.recv().await {
        if !cmd.is_object() {
            warn!(payload = %cmd, "command not an object; dropping");
            continue;
        }
        let json = match control::encode_dict(cmd) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "command encode failed; dropping");
                continue;
            }
        };
        if let Err(e) = control::send_command(client, &json).await {
            warn!(error = format!("{e:#}"), "command dispatch failed");
        }
    }
    Ok(())
}

async fn run_stream_loop(
    stream: &Stream<f64>,
    tx: &broadcast::Sender<TelemetryFrame>,
) -> Result<()> {
    loop {
        stream.wait().await;
        let ut = stream.get().await?;
        let _ = tx.send(TelemetryFrame::Ut(ut));
    }
}

async fn run_heartbeat(krpc: &KRPC) -> Result<()> {
    let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    tick.tick().await; // first tick fires immediately; skip the freebie
    loop {
        tick.tick().await;
        // get_client_id over get_current_game_scene: the latter's response
        // doesn't decode in the main menu (no MainMenu variant in the crate's
        // GameScene enum), which would false-positive a disconnect every time
        // the user exits to the menu and trigger a stale-client leak.
        tokio::time::timeout(HEARTBEAT_TIMEOUT, krpc.get_client_id())
            .await
            .context("kRPC heartbeat timed out")?
            .context("kRPC heartbeat failed")?;
    }
}
