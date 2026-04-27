use anyhow::{Context, Result};
use krpc_client::{Client, services::space_center::SpaceCenter};

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new("ksp-mc-probe", "127.0.0.1", 50000, 50001)
        .await
        .context("connecting to kRPC")?;
    let sc = SpaceCenter::new(client);

    let ut = sc.get_ut().await?;
    println!("kRPC ok    UT = {ut:.3}");

    let vessel = sc.get_active_vessel().await?;
    let name = vessel.get_name().await?;
    let situation = vessel.get_situation().await?;
    println!("vessel ok  \"{name}\"  situation = {situation:?}");

    let parts = vessel.get_parts().await?;
    let kos_parts = parts.with_module("kOSProcessor".to_string()).await?;
    if kos_parts.is_empty() {
        println!("kOS warn   no part on this vessel exposes a kOSProcessor module");
        return Ok(());
    }
    println!("kOS ok     {} processor part(s):", kos_parts.len());
    for part in &kos_parts {
        let title = part.get_title().await?;
        println!("           - {title}");
        for module in part.get_modules().await? {
            let mname = module.get_name().await?;
            let fields = module.get_fields().await?;
            let events = module.get_events().await?;
            let actions = module.get_actions().await?;
            println!("               module {mname}");
            println!("                 fields:  {fields:?}");
            println!("                 events:  {events:?}");
            println!("                 actions: {actions:?}");
        }
    }
    Ok(())
}
