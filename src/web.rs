use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use tokio::sync::broadcast;

use crate::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| client_socket(socket, state))
}

async fn client_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.telemetry_tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(ut) => {
                let payload = serde_json::json!({
                    "ut": ut,
                    "secs_per_day": state.calendar.secs_per_day,
                    "secs_per_year": state.calendar.secs_per_year,
                })
                .to_string();
                if socket.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
