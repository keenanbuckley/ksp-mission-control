mod krpc;
mod web;

use anyhow::{anyhow, Result};
use axum::{routing::get, Router};
use tokio::sync::{broadcast, watch};
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::krpc::{run_telemetry_supervisor, ConnStatus, TelemetryFrame};
use crate::web::ws_handler;

const KRPC_HOST: &str = "127.0.0.1";
const KRPC_RPC_PORT: u16 = 50000;
const KRPC_STREAM_PORT: u16 = 50001;
const BIND_ADDR: &str = "127.0.0.1:8080";

#[derive(Clone)]
pub struct AppState {
    pub telemetry_tx: broadcast::Sender<TelemetryFrame>,
    pub status_tx: watch::Sender<ConnStatus>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let (telemetry_tx, _) = broadcast::channel::<TelemetryFrame>(64);
    let (status_tx, _) = watch::channel(ConnStatus::Disconnected);

    let supervisor = tokio::spawn(run_telemetry_supervisor(
        KRPC_HOST.to_string(),
        KRPC_RPC_PORT,
        KRPC_STREAM_PORT,
        telemetry_tx.clone(),
        status_tx.clone(),
    ));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new("static"))
        .with_state(AppState {
            telemetry_tx,
            status_tx,
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
