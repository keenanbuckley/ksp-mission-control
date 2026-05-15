mod krpc;
mod web;

use std::path::Path;

use anyhow::{anyhow, Result};
use axum::{routing::get, Router};
use ksp_mission_control::config;
use tokio::sync::{broadcast, mpsc, watch};
use tower_http::services::ServeDir;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::krpc::{run_telemetry_supervisor, ConnStatus, OutboundEvent};
use crate::web::ws_handler;

const KRPC_HOST: &str = "127.0.0.1";
const KRPC_RPC_PORT: u16 = 50000;
const KRPC_STREAM_PORT: u16 = 50001;
const BIND_ADDR: &str = "127.0.0.1:8080";
const COMMAND_QUEUE_DEPTH: usize = 16;

#[derive(Clone)]
pub struct AppState {
    pub event_tx: broadcast::Sender<OutboundEvent>,
    pub status_tx: watch::Sender<ConnStatus>,
    pub command_tx: mpsc::Sender<serde_json::Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    if let Err(e) = config::bootstrap_if_missing(Path::new(".kos.toml")) {
        warn!(error = %e, ".kos.toml bootstrap failed; deploy-kos will need a path source");
    }

    let (event_tx, _) = broadcast::channel::<OutboundEvent>(64);
    let (status_tx, _) = watch::channel(ConnStatus::Disconnected);
    let (command_tx, command_rx) = mpsc::channel::<serde_json::Value>(COMMAND_QUEUE_DEPTH);

    let supervisor = tokio::spawn(run_telemetry_supervisor(
        KRPC_HOST.to_string(),
        KRPC_RPC_PORT,
        KRPC_STREAM_PORT,
        event_tx.clone(),
        status_tx.clone(),
        command_rx,
    ));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new("static"))
        .with_state(AppState {
            event_tx,
            status_tx,
            command_tx,
        });

    let listener = tokio::net::TcpListener::bind(BIND_ADDR).await?;
    info!("listening on http://{BIND_ADDR}");

    tokio::select! {
        res = axum::serve(listener, app) => res?,
        join = supervisor => match join {
            Ok(()) => return Err(anyhow!("telemetry supervisor exited unexpectedly")),
            Err(e) => return Err(anyhow!("telemetry supervisor panicked: {e}")),
        },
    }
    Ok(())
}
