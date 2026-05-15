use anyhow::{Context, Result};
use krpc_client::Client;
use ksp_mission_control::{control, planning};

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new("ksp-mc-send-plan-circ", "127.0.0.1", 50000, 50001)
        .await
        .context("connecting to kRPC")?;

    let payload = planning::plan_circ_payload(&client).await?;
    println!("planned: {payload}");
    let json = control::encode_dict(payload)?;
    control::send_command(&client, &json).await?;
    println!("sent: {json}");
    Ok(())
}
