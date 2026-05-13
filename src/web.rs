use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

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
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => handle_inbound(&text, &state.command_tx),
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {} // ignore Binary/Ping/Pong
                Some(Err(e)) => {
                    warn!(error = %e, "ws recv error; closing");
                    break;
                }
            },
        }
    }
}

fn handle_inbound(text: &str, command_tx: &mpsc::Sender<serde_json::Value>) {
    let frame: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "ws inbound: bad json; dropping");
            return;
        }
    };
    if frame.get("kind").and_then(|k| k.as_str()) != Some("command") {
        warn!("ws inbound: kind != \"command\"; dropping");
        return;
    }
    let Some(command) = frame.get("command") else {
        warn!("ws inbound: missing command field; dropping");
        return;
    };
    if !command.is_object() {
        warn!("ws inbound: command not an object; dropping");
        return;
    }
    match command_tx.try_send(command.clone()) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            warn!("command queue full; dropping");
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            warn!("command channel closed; dropping");
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
