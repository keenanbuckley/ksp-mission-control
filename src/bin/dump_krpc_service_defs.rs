use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use krpc_client::{Client, services::krpc::KRPC};
use serde_json::{Map, Value};

fn type_code_str(value: i32) -> &'static str {
    match value {
        0 => "NONE",
        1 => "DOUBLE",
        2 => "FLOAT",
        3 => "SINT32",
        4 => "SINT64",
        5 => "UINT32",
        6 => "UINT64",
        7 => "BOOL",
        8 => "STRING",
        9 => "BYTES",
        100 => "CLASS",
        101 => "ENUMERATION",
        200 => "EVENT",
        201 => "PROCEDURE_CALL",
        202 => "STREAM",
        203 => "STATUS",
        204 => "SERVICES",
        300 => "TUPLE",
        301 => "LIST",
        302 => "SET",
        303 => "DICTIONARY",
        _ => "NONE",
    }
}

// Iterative post-order DFS that serializes a kRPC schema Type tree to a
// serde_json Value. A macro because krpc-client keeps its protobuf-generated
// `schema::Type` private (`mod schema`, not `pub mod`), so we cannot name the
// parameter type for a recursive helper fn — but inline at the call site,
// `vec![$root]`'s element type is inferred from `$root`.
macro_rules! encode_type {
    ($root:expr) => {{
        let mut work = vec![$root];
        let mut visited: Vec<usize> = vec![0];
        let mut results: Vec<Value> = vec![];
        while !work.is_empty() {
            let current = *work.last().unwrap();
            let depth = work.len() - 1;
            let next = visited[depth];
            if next < current.types.len() {
                visited[depth] += 1;
                work.push(&current.types[next]);
                visited.push(0);
            } else {
                let n = current.types.len();
                let split = results.len() - n;
                let children: Vec<Value> = results.split_off(split);
                let mut o = Map::new();
                o.insert(
                    "code".into(),
                    Value::String(type_code_str(current.code.value()).into()),
                );
                if !current.service.is_empty() {
                    o.insert("service".into(), Value::String(current.service.clone()));
                }
                if !current.name.is_empty() {
                    o.insert("name".into(), Value::String(current.name.clone()));
                }
                if !children.is_empty() {
                    o.insert("types".into(), Value::Array(children));
                }
                results.push(Value::Object(o));
                work.pop();
                visited.pop();
            }
        }
        results.pop().unwrap()
    }};
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new("ksp-mc-dump-defs", "127.0.0.1", 50000, 50001).await?;
    let krpc = KRPC::new(client);
    let services = krpc.get_services().await?;

    let out_dir = PathBuf::from("service_definitions");
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("create {}", out_dir.display()))?;

    for service in &services.services {
        let mut procedures = Map::new();
        for p in &service.procedures {
            let parameters: Vec<Value> = p
                .parameters
                .iter()
                .map(|param| {
                    let mut po = Map::new();
                    po.insert("name".into(), Value::String(param.name.clone()));
                    if let Some(t) = param.type_.as_ref() {
                        po.insert("type".into(), encode_type!(t));
                    }
                    po.insert("nullable".into(), Value::Bool(param.nullable));
                    Value::Object(po)
                })
                .collect();

            let mut po = Map::new();
            po.insert("parameters".into(), Value::Array(parameters));
            if let Some(rt) = p.return_type.as_ref() {
                po.insert("return_type".into(), encode_type!(rt));
                po.insert(
                    "return_is_nullable".into(),
                    Value::Bool(p.return_is_nullable),
                );
            }
            if !p.documentation.is_empty() {
                po.insert(
                    "documentation".into(),
                    Value::String(p.documentation.clone()),
                );
            }
            procedures.insert(p.name.clone(), Value::Object(po));
        }

        let mut classes = Map::new();
        for c in &service.classes {
            let mut co = Map::new();
            if !c.documentation.is_empty() {
                co.insert(
                    "documentation".into(),
                    Value::String(c.documentation.clone()),
                );
            }
            classes.insert(c.name.clone(), Value::Object(co));
        }

        let mut enumerations = Map::new();
        for e in &service.enumerations {
            let values: Vec<Value> = e
                .values
                .iter()
                .map(|v| {
                    let mut vo = Map::new();
                    vo.insert("name".into(), Value::String(v.name.clone()));
                    vo.insert("value".into(), Value::Number(v.value.into()));
                    if !v.documentation.is_empty() {
                        vo.insert(
                            "documentation".into(),
                            Value::String(v.documentation.clone()),
                        );
                    }
                    Value::Object(vo)
                })
                .collect();
            let mut eo = Map::new();
            eo.insert("values".into(), Value::Array(values));
            if !e.documentation.is_empty() {
                eo.insert(
                    "documentation".into(),
                    Value::String(e.documentation.clone()),
                );
            }
            enumerations.insert(e.name.clone(), Value::Object(eo));
        }

        let mut so = Map::new();
        if !service.documentation.is_empty() {
            so.insert(
                "documentation".into(),
                Value::String(service.documentation.clone()),
            );
        }
        so.insert("procedures".into(), Value::Object(procedures));
        so.insert("classes".into(), Value::Object(classes));
        so.insert("enumerations".into(), Value::Object(enumerations));

        let mut top = Map::new();
        top.insert(service.name.clone(), Value::Object(so));
        let value = Value::Object(top);

        let path = out_dir.join(format!("{}.json", service.name));
        let tmp = path.with_extension("json.tmp");
        let pretty = serde_json::to_string_pretty(&value)?;
        fs::write(&tmp, pretty).with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, &path)
            .with_context(|| format!("rename to {}", path.display()))?;
        println!(
            "wrote {} ({} procedures, {} classes, {} enumerations)",
            path.display(),
            service.procedures.len(),
            service.classes.len(),
            service.enumerations.len()
        );
    }

    Ok(())
}
