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
/// fields are exactly those of `payload`. Nested container values
/// (lists, sub-dicts) inside `payload` must already be wrapped via
/// `encode_list` or a sub-`encode_dict`. kIPC's per-element deserializer
/// only treats primitives (number/string/bool) as untagged.
pub fn encode_dict(payload: serde_json::Value) -> Result<String> {
    let envelope = serde_json::json!({
        "type": "dict",
        "data": payload,
        "keys": [],
        "values": [],
    });
    Ok(serde_json::to_string(&envelope)?)
}

/// Wraps a JSON array in kIPC's list envelope: `{"type":"list","data":[...]}`.
/// Use when embedding a list as a value inside a payload passed to
/// `encode_dict`.
pub fn encode_list(items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "list",
        "data": items,
    })
}

/// Inverse of `encode_dict`. Reads a kIPC envelope string and returns the
/// inner Lexicon as a JSON object. Handles both encoder shapes seen in the
/// wild: payload in `data`, or split across parallel `keys` / `values`
/// arrays (which is how kIPC's kerboscript-side encoder serializes a
/// `Lexicon`).
pub fn decode_dict(json: &str) -> Result<serde_json::Map<String, serde_json::Value>> {
    let envelope: serde_json::Value = serde_json::from_str(json).context("parse envelope json")?;
    let t = envelope
        .get("type")
        .and_then(|v| v.as_str())
        .context("envelope missing type")?;
    if t != "dict" {
        anyhow::bail!("envelope type is {t}, expected dict");
    }
    if let Some(data) = envelope.get("data").and_then(|v| v.as_object()) {
        if !data.is_empty() {
            return Ok(data.clone());
        }
    }
    let keys = envelope
        .get("keys")
        .and_then(|v| v.as_array())
        .context("envelope missing keys")?;
    let values = envelope
        .get("values")
        .and_then(|v| v.as_array())
        .context("envelope missing values")?;
    if keys.len() != values.len() {
        anyhow::bail!(
            "envelope keys/values length mismatch: {} vs {}",
            keys.len(),
            values.len()
        );
    }
    let mut out = serde_json::Map::with_capacity(keys.len());
    for (k, v) in keys.iter().zip(values.iter()) {
        let key = k
            .as_str()
            .with_context(|| format!("non-string key in envelope: {k}"))?;
        out.insert(key.to_string(), v.clone());
    }
    Ok(out)
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
