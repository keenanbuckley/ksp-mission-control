use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use krpc_client::{services::krpc::KRPC, Client};
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

fn insert_doc(map: &mut Map<String, Value>, doc: &str) {
    if !doc.is_empty() {
        map.insert("documentation".into(), Value::String(doc.into()));
    }
}

fn encode_parameter(name: &str, type_value: Option<Value>, nullable: bool) -> Value {
    let mut po = Map::new();
    po.insert("name".into(), Value::String(name.into()));
    if let Some(t) = type_value {
        po.insert("type".into(), t);
    }
    po.insert("nullable".into(), Value::Bool(nullable));
    Value::Object(po)
}

fn encode_procedure(
    parameters: Vec<Value>,
    return_type: Option<Value>,
    return_is_nullable: bool,
    documentation: &str,
) -> Value {
    let mut po = Map::new();
    po.insert("parameters".into(), Value::Array(parameters));
    if let Some(rt) = return_type {
        po.insert("return_type".into(), rt);
        po.insert("return_is_nullable".into(), Value::Bool(return_is_nullable));
    }
    insert_doc(&mut po, documentation);
    Value::Object(po)
}

fn encode_class(documentation: &str) -> Value {
    let mut co = Map::new();
    insert_doc(&mut co, documentation);
    Value::Object(co)
}

fn encode_enum_entry(name: &str, value: i32, documentation: &str) -> Value {
    let mut vo = Map::new();
    vo.insert("name".into(), Value::String(name.into()));
    vo.insert("value".into(), Value::Number(value.into()));
    insert_doc(&mut vo, documentation);
    Value::Object(vo)
}

fn encode_enumeration(values: Vec<Value>, documentation: &str) -> Value {
    let mut eo = Map::new();
    eo.insert("values".into(), Value::Array(values));
    insert_doc(&mut eo, documentation);
    Value::Object(eo)
}

fn wrap_service(
    name: &str,
    documentation: &str,
    procedures: Map<String, Value>,
    classes: Map<String, Value>,
    enumerations: Map<String, Value>,
) -> Value {
    let mut so = Map::new();
    insert_doc(&mut so, documentation);
    so.insert("procedures".into(), Value::Object(procedures));
    so.insert("classes".into(), Value::Object(classes));
    so.insert("enumerations".into(), Value::Object(enumerations));
    let mut top = Map::new();
    top.insert(name.into(), Value::Object(so));
    Value::Object(top)
}

fn write_service_json(out_dir: &Path, name: &str, value: &Value) -> Result<()> {
    let path = out_dir.join(format!("{name}.json"));
    let tmp = path.with_extension("json.tmp");
    let pretty = serde_json::to_string_pretty(value)?;
    fs::write(&tmp, pretty).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("rename to {}", path.display()))?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new("ksp-mc-dump-defs", "127.0.0.1", 50000, 50001).await?;
    let krpc = KRPC::new(client);
    let services = krpc.get_services().await?;

    let out_dir = PathBuf::from("service_definitions");
    fs::create_dir_all(&out_dir).with_context(|| format!("create {}", out_dir.display()))?;

    for service in &services.services {
        let procedures: Map<String, Value> = service
            .procedures
            .iter()
            .map(|p| {
                let parameters = p
                    .parameters
                    .iter()
                    .map(|param| {
                        encode_parameter(
                            &param.name,
                            param.type_.as_ref().map(|t| encode_type!(t)),
                            param.nullable,
                        )
                    })
                    .collect();
                let return_type = p.return_type.as_ref().map(|rt| encode_type!(rt));
                let value = encode_procedure(
                    parameters,
                    return_type,
                    p.return_is_nullable,
                    &p.documentation,
                );
                (p.name.clone(), value)
            })
            .collect();

        let classes: Map<String, Value> = service
            .classes
            .iter()
            .map(|c| (c.name.clone(), encode_class(&c.documentation)))
            .collect();

        let enumerations: Map<String, Value> = service
            .enumerations
            .iter()
            .map(|e| {
                let values = e
                    .values
                    .iter()
                    .map(|v| encode_enum_entry(&v.name, v.value, &v.documentation))
                    .collect();
                (e.name.clone(), encode_enumeration(values, &e.documentation))
            })
            .collect();

        let value = wrap_service(
            &service.name,
            &service.documentation,
            procedures,
            classes,
            enumerations,
        );
        write_service_json(&out_dir, &service.name, &value)?;
        println!(
            "wrote {}/{}.json ({} procedures, {} classes, {} enumerations)",
            out_dir.display(),
            service.name,
            service.procedures.len(),
            service.classes.len(),
            service.enumerations.len()
        );
    }

    Ok(())
}
