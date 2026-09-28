use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use krpc_client::{
    services::{kipc::KIPC, krpc::KRPC, space_center::SpaceCenter},
    stream::Stream,
    Client,
};
use ksp_mission_control::script_watchdog::{Failure, Heartbeat, KosLink, ScriptWatchdog};
use ksp_mission_control::{control, launch_planning, planning};
use serde::Serialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

const STREAM_RATE_HZ: f32 = 5.0;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(15);
const INBOX_POLL_INTERVAL: Duration = Duration::from_millis(200);
const KOS_PING_INTERVAL: Duration = Duration::from_secs(2);
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum OutboundEvent {
    Ut(f64),
    NodePlanned {
        dv: f64,
        ut: f64,
    },
    CommandAck {
        op: String,
    },
    CommandError {
        op: String,
        reason: String,
    },
    ScriptDone {
        path: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Calendar {
    pub secs_per_day: f64,
    pub secs_per_year: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnStatus {
    Disconnected,
    Connected { calendar: Calendar, kos: KosLink },
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
    event_tx: broadcast::Sender<OutboundEvent>,
    status_tx: watch::Sender<ConnStatus>,
    mut command_rx: mpsc::Receiver<serde_json::Value>,
) {
    let mut backoff = INITIAL_BACKOFF;
    // Outlives each kRPC session: kOS keeps flying through a kRPC hiccup, and
    // a KSP restart shows up as a UT rewind or as silence.
    let watchdog = Mutex::new(ScriptWatchdog::new());
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

        let connected = ConnStatus::Connected {
            calendar,
            kos: lock(&watchdog).link().clone(),
        };
        status_tx.send_if_modified(|s| {
            if *s == connected {
                false
            } else {
                *s = connected.clone();
                true
            }
        });
        backoff = INITIAL_BACKOFF;

        if let Err(e) = run_session(
            client,
            event_tx.clone(),
            &mut command_rx,
            &watchdog,
            &status_tx,
        )
        .await
        {
            warn!(error = format!("{e:#}"), "kRPC session ended; reconnecting");
        }
    }
}

fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

async fn run_session(
    client: Arc<Client>,
    tx: broadcast::Sender<OutboundEvent>,
    command_rx: &mut mpsc::Receiver<serde_json::Value>,
    watchdog: &Mutex<ScriptWatchdog>,
    status_tx: &watch::Sender<ConnStatus>,
) -> Result<()> {
    let space_center = SpaceCenter::new(client.clone());
    let krpc = KRPC::new(client.clone());
    let stream = space_center.get_ut_stream().await?;
    stream.set_rate(STREAM_RATE_HZ).await?;

    tokio::select! {
        res = run_stream_loop(&stream, &tx, watchdog, status_tx) => res,
        res = run_heartbeat(&krpc) => res,
        res = run_dispatcher(&client, command_rx, &tx, watchdog) => res,
        res = run_inbox_loop(&client, &tx, watchdog, status_tx) => res,
        res = run_kos_ping_loop(&client, watchdog) => res,
    }
}

/// The lock is only held across synchronous watchdog calls, never an await.
fn lock(watchdog: &Mutex<ScriptWatchdog>) -> MutexGuard<'_, ScriptWatchdog> {
    watchdog.lock().unwrap_or_else(PoisonError::into_inner)
}

fn emit_failures(tx: &broadcast::Sender<OutboundEvent>, failures: Vec<Failure>) {
    for f in failures {
        warn!(path = %f.path, reason = f.reason, "script declared dead");
        let _ = tx.send(OutboundEvent::ScriptDone {
            path: f.path,
            ok: false,
            reason: Some(f.reason.to_string()),
        });
    }
}

fn publish_link(status_tx: &watch::Sender<ConnStatus>, link: KosLink) {
    status_tx.send_if_modified(|s| match s {
        ConnStatus::Connected { kos, .. } if *kos != link => {
            info!(up = link.up, running = ?link.running, "kOS link changed");
            *kos = link;
            true
        }
        _ => false,
    });
}

async fn run_dispatcher(
    client: &Arc<Client>,
    command_rx: &mut mpsc::Receiver<serde_json::Value>,
    event_tx: &broadcast::Sender<OutboundEvent>,
    watchdog: &Mutex<ScriptWatchdog>,
) -> Result<()> {
    while let Some(cmd) = command_rx.recv().await {
        if !cmd.is_object() {
            warn!(payload = %cmd, "command not an object; dropping");
            continue;
        }
        let op = cmd
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let script_path = if op == "run_script" {
            cmd.get("path").and_then(|v| v.as_str()).map(str::to_owned)
        } else {
            None
        };
        let mut planned: Option<planning::CircPlan> = None;
        let payload = match op.as_str() {
            "plan_circ" => match planning::plan_circ(client).await {
                Ok(plan) => {
                    info!(
                        dv = plan.dv,
                        ut = plan.ut,
                        "plan_circ: dispatching add_node"
                    );
                    planned = Some(plan);
                    json!({ "op": "add_node", "dv": plan.dv, "ut": plan.ut })
                }
                Err(e) => {
                    let reason = format!("{e:#}");
                    warn!(error = %reason, "plan_circ planning failed");
                    let _ = event_tx.send(OutboundEvent::CommandError { op, reason });
                    continue;
                }
            },
            // Launch carries derived constants the kOS script can't compute
            // cheaply. Pre-compute them from mission params plus live body data
            // (several RPC round-trips, one shot) and ship them in the config
            // Lexicon. Other run_script paths pass through untouched.
            "run_script" if cmd.get("path").and_then(|v| v.as_str()) == Some("launch.ks") => {
                let params = match launch_planning::LaunchParams::from_args(
                    cmd.get("args").unwrap_or(&serde_json::Value::Null),
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        let reason = format!("{e:#}");
                        warn!(error = %reason, "launch args rejected");
                        let _ = event_tx.send(OutboundEvent::CommandError { op, reason });
                        continue;
                    }
                };
                info!(?params, "launch params received");
                match launch_planning::plan_launch(client, params).await {
                    Ok(derived) => {
                        let cfg = launch_planning::build_launch_payload(&params, &derived);
                        json!({
                            "op": "run_script",
                            "path": "launch.ks",
                            "args": control::encode_dict_value(cfg),
                        })
                    }
                    Err(e) => {
                        let reason = format!("{e:#}");
                        warn!(error = %reason, "launch planning failed");
                        let _ = event_tx.send(OutboundEvent::CommandError { op, reason });
                        continue;
                    }
                }
            }
            _ => cmd,
        };
        let json = match control::encode_dict(payload) {
            Ok(j) => j,
            Err(e) => {
                let reason = e.to_string();
                warn!(error = %reason, "command encode failed; dropping");
                let _ = event_tx.send(OutboundEvent::CommandError { op, reason });
                continue;
            }
        };
        if let Err(e) = control::send_command(client, &json).await {
            let reason = format!("{e:#}");
            warn!(error = %reason, "command dispatch failed");
            let _ = event_tx.send(OutboundEvent::CommandError { op, reason });
            continue;
        }
        if let Some(path) = script_path {
            lock(watchdog).on_dispatch(path);
        }
        if let Some(plan) = planned {
            let _ = event_tx.send(OutboundEvent::NodePlanned {
                dv: plan.dv,
                ut: plan.ut,
            });
        }
    }
    Ok(())
}

async fn run_inbox_loop(
    client: &Arc<Client>,
    event_tx: &broadcast::Sender<OutboundEvent>,
    watchdog: &Mutex<ScriptWatchdog>,
    status_tx: &watch::Sender<ConnStatus>,
) -> Result<()> {
    let kipc = KIPC::new(client.clone());
    let mut tick = tokio::time::interval(INBOX_POLL_INTERVAL);
    tick.tick().await;
    loop {
        tick.tick().await;
        let count = kipc
            .get_count_messages()
            .await
            .context("kipc get_count_messages")?;
        if count <= 0 {
            continue;
        }
        for _ in 0..count {
            let raw = kipc.pop_message().await.context("kipc pop_message")?;
            if raw.is_empty() {
                break;
            }
            match parse_inbound(&raw) {
                Some(Inbound::Event(event)) => {
                    let _ = event_tx.send(event);
                }
                Some(Inbound::ScriptDone {
                    path,
                    ok,
                    reason,
                    boot,
                }) => {
                    let link = {
                        let mut wd = lock(watchdog);
                        wd.on_done(&path, boot.as_deref());
                        wd.link().clone()
                    };
                    publish_link(status_tx, link);
                    let _ = event_tx.send(OutboundEvent::ScriptDone { path, ok, reason });
                }
                Some(Inbound::Heartbeat { boot, path, active }) => {
                    let link = {
                        let mut wd = lock(watchdog);
                        wd.on_heartbeat(Heartbeat {
                            boot: &boot,
                            path: path.as_deref(),
                            active,
                        });
                        wd.link().clone()
                    };
                    publish_link(status_tx, link);
                }
                None => {}
            }
        }
    }
}

/// A decoded kIPC message. `ScriptDone` and `Heartbeat` carry the dispatcher's
/// boot id, which the watchdog needs and the browser doesn't.
enum Inbound {
    Event(OutboundEvent),
    ScriptDone {
        path: String,
        ok: bool,
        reason: Option<String>,
        boot: Option<String>,
    },
    Heartbeat {
        boot: String,
        path: Option<String>,
        active: bool,
    },
}

fn parse_inbound(raw: &str) -> Option<Inbound> {
    let payload = match control::decode_dict(raw) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, raw = %raw, "inbox: envelope decode failed");
            return None;
        }
    };
    let str_field = |key: &str| payload.get(key).and_then(|v| v.as_str());
    let kind = str_field("kind")?;
    match kind {
        "command_ack" => {
            let op = str_field("op")?.to_string();
            Some(Inbound::Event(OutboundEvent::CommandAck { op }))
        }
        "script_done" => Some(Inbound::ScriptDone {
            path: str_field("path")?.to_string(),
            ok: payload.get("ok").and_then(|v| v.as_bool()).unwrap_or(true),
            reason: str_field("reason").map(str::to_owned),
            boot: str_field("boot").map(str::to_owned),
        }),
        "heartbeat" => Some(Inbound::Heartbeat {
            boot: str_field("boot")?.to_string(),
            path: str_field("path")
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
            active: payload
                .get("active")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }),
        other => {
            warn!(kind = %other, "inbox: unknown event kind; dropping");
            None
        }
    }
}

async fn run_stream_loop(
    stream: &Stream<f64>,
    tx: &broadcast::Sender<OutboundEvent>,
    watchdog: &Mutex<ScriptWatchdog>,
    status_tx: &watch::Sender<ConnStatus>,
) -> Result<()> {
    loop {
        stream.wait().await;
        let ut = stream.get().await?;
        let _ = tx.send(OutboundEvent::Ut(ut));
        let (failures, link) = {
            let mut wd = lock(watchdog);
            (wd.on_ut(ut, Instant::now()), wd.link().clone())
        };
        emit_failures(tx, failures);
        publish_link(status_tx, link);
    }
}

/// An idle dispatcher only heartbeats in reply to a ping. kIPC's outbound
/// queue is unbounded and persisted into the save file, so unsolicited idle
/// beats with no server draining them would pile up there.
async fn run_kos_ping_loop(client: &Arc<Client>, watchdog: &Mutex<ScriptWatchdog>) -> Result<()> {
    let ping = control::encode_dict(json!({ "op": "ping" }))?;
    let mut tick = tokio::time::interval(KOS_PING_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if !lock(watchdog).ping_due() {
            continue;
        }
        // Routine outside flight or on a vessel without an mc CPU; the link
        // goes down on its own when replies stop.
        if let Err(e) = control::send_command(client, &ping).await {
            debug!(error = format!("{e:#}"), "kOS ping not sent");
        }
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
