use std::sync::Arc;

use anyhow::{Context, Result};
use krpc_client::Client;
use ksp_mission_control::{control, launch_planning};

#[tokio::main]
async fn main() -> Result<()> {
    let client = Arc::new(
        Client::new("ksp-mc-send-launch", "127.0.0.1", 50000, 50001)
            .await
            .context("connecting to kRPC")?,
    );

    // Mirror the dispatcher: compute the derived constants from the body, fold
    // them into the config Lexicon, wrap it as a nested dict for kIPC.
    let params = launch_planning::LaunchParams::default();
    let derived = launch_planning::plan_launch(&client, params).await?;
    let cfg = launch_planning::build_launch_payload(&params, &derived);

    let json = control::encode_dict(serde_json::json!({
        "op": "run_script",
        "path": "launch.ks",
        "args": control::encode_dict_value(cfg),
    }))?;
    control::send_command(&client, &json).await?;
    println!("sent: {json}");
    Ok(())
}
