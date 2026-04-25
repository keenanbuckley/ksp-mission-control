mod krpc;
mod web;

use anyhow::Result;
use axum::{Router, routing::get};
use krpc_client::Client;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

use crate::krpc::{Calendar, detect_calendar, run_ut_stream};
use crate::web::ws_handler;

const KRPC_HOST: &str = "127.0.0.1";
const KRPC_RPC_PORT: u16 = 50000;
const KRPC_STREAM_PORT: u16 = 50001;
const BIND_ADDR: &str = "127.0.0.1:8080";

#[derive(Clone)]
pub struct AppState {
    pub telemetry_tx: broadcast::Sender<f64>,
    pub calendar: Calendar,
}

#[tokio::main]
async fn main() -> Result<()> {
    let krpc = Client::new(
        "ksp-mission-control",
        KRPC_HOST,
        KRPC_RPC_PORT,
        KRPC_STREAM_PORT,
    )
    .await?;
    eprintln!("connected to kRPC at {KRPC_HOST}:{KRPC_RPC_PORT}");

    let calendar = detect_calendar(krpc.clone()).await?;
    eprintln!(
        "calendar: {} s/day, {} s/year",
        calendar.secs_per_day, calendar.secs_per_year
    );

    let (telemetry_tx, _) = broadcast::channel::<f64>(64);
    tokio::spawn(run_ut_stream(krpc.clone(), telemetry_tx.clone()));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new("static"))
        .with_state(AppState {
            telemetry_tx,
            calendar,
        });

    let listener = tokio::net::TcpListener::bind(BIND_ADDR).await?;
    eprintln!("listening on http://{BIND_ADDR}");
    axum::serve(listener, app).await?;
    Ok(())
}
