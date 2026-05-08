use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use serde_json::json;
use tokio::sync::broadcast;

use crate::krpc::ConnStatus;
use crate::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| client_socket(socket, state))
}

async fn client_socket(mut socket: WebSocket, state: AppState) {
    let mut tel_rx = state.telemetry_tx.subscribe();
    let mut status_rx = state.status_tx.subscribe();

    // Send the current status snapshot up front so a freshly connected client
    // immediately knows whether KSP is reachable.
    let snapshot = status_rx.borrow_and_update().clone();
    if socket
        .send(Message::Text(status_json(&snapshot).to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            tel = tel_rx.recv() => match tel {
                Ok(frame) => {
                    let payload = serde_json::to_string(&frame)
                        .expect("TelemetryFrame is always serializable");
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            changed = status_rx.changed() => match changed {
                Ok(()) => {
                    let status = status_rx.borrow_and_update().clone();
                    if socket
                        .send(Message::Text(status_json(&status).to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            },
        }
    }
}

fn status_json(status: &ConnStatus) -> serde_json::Value {
    match status {
        ConnStatus::Disconnected => json!({ "kind": "status", "connected": false }),
        ConnStatus::Connected { calendar } => json!({
            "kind": "status",
            "connected": true,
            "calendar": calendar,
        }),
    }
}
