use anyhow::{Context, Result};
use krpc_client::Client;
use ksp_mission_control::control;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new("ksp-mc-send-toggle", "127.0.0.1", 50000, 50001)
        .await
        .context("connecting to kRPC")?;

    let json = control::encode_dict(serde_json::json!({
        "op": "toggle_ag",
        "n": 1,
    }))?;
    control::send_command(&client, &json).await?;
    println!("sent: {json}");
    Ok(())
}
