mod krpc;
mod web;

use anyhow::Result;
use axum::{Router, routing::get};
use krpc_client::Client;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::krpc::{Calendar, TelemetryFrame, detect_calendar, run_ut_stream};
use crate::web::ws_handler;

const KRPC_HOST: &str = "127.0.0.1";
const KRPC_RPC_PORT: u16 = 50000;
const KRPC_STREAM_PORT: u16 = 50001;
const BIND_ADDR: &str = "127.0.0.1:8080";

#[derive(Clone)]
pub struct AppState {
    pub telemetry_tx: broadcast::Sender<TelemetryFrame>,
    pub calendar: Calendar,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let krpc = Client::new(
        "ksp-mission-control",
        KRPC_HOST,
        KRPC_RPC_PORT,
        KRPC_STREAM_PORT,
    )
    .await?;
    info!(host = KRPC_HOST, port = KRPC_RPC_PORT, "connected to kRPC");

    let calendar = detect_calendar(krpc.clone()).await?;
    info!(
        secs_per_day = calendar.secs_per_day,
        secs_per_year = calendar.secs_per_year,
        "calendar detected"
    );

    let (telemetry_tx, _) = broadcast::channel::<TelemetryFrame>(64);
    tokio::spawn(run_ut_stream(krpc.clone(), telemetry_tx.clone()));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new("static"))
        .with_state(AppState {
            telemetry_tx,
            calendar,
        });

    let listener = tokio::net::TcpListener::bind(BIND_ADDR).await?;
    info!("listening on http://{BIND_ADDR}");
    axum::serve(listener, app).await?;
    Ok(())
}
