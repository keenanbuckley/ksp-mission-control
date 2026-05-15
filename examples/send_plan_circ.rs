use anyhow::{Context, Result};
use krpc_client::Client;
use ksp_mission_control::{control, planning};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new("ksp-mc-send-plan-circ", "127.0.0.1", 50000, 50001)
        .await
        .context("connecting to kRPC")?;

    let plan = planning::plan_circ(&client).await?;
    println!("planned: dv={} ut={}", plan.dv, plan.ut);
    let payload = json!({ "op": "add_node", "dv": plan.dv, "ut": plan.ut });
    let json = control::encode_dict(payload)?;
    control::send_command(&client, &json).await?;
    println!("sent: {json}");
    Ok(())
}
