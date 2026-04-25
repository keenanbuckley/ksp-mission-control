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
            Ok(frame) => {
                let mut value = serde_json::to_value(&frame)
                    .expect("TelemetryFrame is always serializable");
                let obj = value
                    .as_object_mut()
                    .expect("TelemetryFrame serializes as an object");
                obj.insert("secs_per_day".into(), state.calendar.secs_per_day.into());
                obj.insert("secs_per_year".into(), state.calendar.secs_per_year.into());
                if socket.send(Message::Text(value.to_string().into())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
