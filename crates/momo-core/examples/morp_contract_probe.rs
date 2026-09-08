//! Offline JSONL bridge to the real context, response validation and DMW code.
//! No model endpoint, oracle answer, or evaluator code is linked into this probe.
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead},
};

use momo_core::{MomoResponseRequest, api::simple::prepare_context_json};
use momo_memory::{ConservativeTokenCounter, MemoryDocument, MemoryWorkspace, Metadata};
use serde_json::{Value, json};
use tempfile::TempDir;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let observation = observe(&request)?;
        println!(
            "{}",
            json!({"id": request["id"], "observation": observation})
        );
    }
    Ok(())
}

fn observe(request: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    match request["op"].as_str().ok_or("op required")? {
        "context" => Ok(serde_json::from_str(&prepare_context_json(
            request["input"].to_string(),
        )?)?),
        "validate_response" => {
            let result = serde_json::from_value::<MomoResponseRequest>(request["input"].clone())
                .map_err(|e| e.to_string())
                .and_then(|r| r.validate().map_err(|e| e.to_string()));
            Ok(json!({"accepted": result.is_ok()}))
        }
        "retrieve" => {
            let temp = TempDir::new()?;
            let memory = MemoryWorkspace::initialize(temp.path())?;
            for (index, value) in request["documents"]
                .as_array()
                .ok_or("documents required")?
                .iter()
                .enumerate()
            {
                let document = MemoryDocument {
                    metadata: Metadata {
                        id: value["id"]
                            .as_str()
                            .ok_or("document id required")?
                            .to_owned(),
                        kind: "event".to_owned(),
                        importance: Some(0.5),
                        weight: Some(1.0),
                        touch_at: 1_700_000_000,
                        decay_at: Some(1_700_000_000),
                        archived_at: None,
                        relations: BTreeMap::new(),
                        tags: serde_json::from_value(value["tags"].clone())?,
                        aliases: Vec::new(),
                        injection_scope: None,
                        injection_conversation_id: None,
                        injection_character_id: None,
                        status: "active".to_owned(),
                    },
                    body: value["body"].as_str().ok_or("body required")?.to_owned(),
                };
                fs::write(
                    memory
                        .root()
                        .join("events")
                        .join(format!("fixture_{index:05}.md")),
                    document.encode()?,
                )?;
            }
            memory.rebuild_index()?;
            let results = memory.retrieve(
                request["query"].as_str().ok_or("query required")?,
                request["budget"]
                    .as_u64()
                    .ok_or("budget required")?
                    .try_into()?,
                &ConservativeTokenCounter,
            )?;
            Ok(json!({"ids": results.iter().map(|r| &r.id).collect::<Vec<_>>()}))
        }
        _ => Err("unknown operation".into()),
    }
}
