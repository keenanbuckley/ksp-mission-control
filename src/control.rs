use std::sync::Arc;

use anyhow::{Context, Result};
use krpc_client::{
    services::{kipc::KIPC, space_center::SpaceCenter},
    Client,
};
use tracing::warn;

/// Wraps a logical kerboscript-Lexicon payload in kIPC's required dict
/// envelope: `{"type":"dict","data":<payload>,"keys":[],"values":[]}`.
///
/// kIPC's deserializer rejects top-level JSON objects without a `type`
/// field, and the `dict` handler requires `data`/`keys`/`values`. The
/// receiving kerboscript script sees `msg:CONTENT` as a Lexicon whose
/// fields are exactly those of `payload`.
pub fn encode_dict(payload: serde_json::Value) -> Result<String> {
    let envelope = serde_json::json!({
        "type": "dict",
        "data": payload,
        "keys": [],
        "values": [],
    });
    Ok(serde_json::to_string(&envelope)?)
}

pub async fn send_command(client: &Arc<Client>, json: &str) -> Result<()> {
    let sc = SpaceCenter::new(client.clone());
    let kipc = KIPC::new(client.clone());

    let vessel = sc.get_active_vessel().await.context("get active vessel")?;
    let parts = kipc
        .get_parts_tagged(&vessel, "mc".to_string())
        .await
        .context("get parts tagged \"mc\"")?;
    if parts.len() != 1 {
        warn!(
            found = parts.len(),
            "expected exactly one mc-tagged part on active vessel"
        );
    }
    let part = parts
        .into_iter()
        .next()
        .context("no mc-tagged part on active vessel")?;
    let processor = kipc
        .get_processor(&part)
        .await
        .context("get processor for mc part")?
        .into_iter()
        .next()
        .context("mc-tagged part has no kOS processor")?;
    let _delivered = processor
        .send_message(json.to_string())
        .await
        .context("send kIPC message")?;
    Ok(())
}
