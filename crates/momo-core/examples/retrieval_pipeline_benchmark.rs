use std::{collections::BTreeMap, fs, time::Instant};

use momo_core::{
    MomoRuntime,
    api::runtime_api::{MemorySpaceSource, ScopedMemoryRequest, retrieve_scoped_memory},
};
use momo_domain::provenance::MemoryIdentity;
use momo_memory::{MemoryDocument, MemoryWorkspace, Metadata};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|size| size.parse())
        .collect::<Result<_, _>>()?;
    let sizes = if sizes.is_empty() {
        vec![50, 100, 200, 1000]
    } else {
        sizes
    };
    println!(
        "Offline synthetic native retrieval; explicit legacy identity binding; no provider calls."
    );
    for count in sizes {
        assert!(count > 0, "document count must be positive");
        let temp = tempfile::tempdir()?;
        let space = Uuid::now_v7();
        let conversation = Uuid::now_v7();
        let character = Uuid::now_v7();
        let identity = MemoryIdentity {
            personal_space_id: space,
            conversation_id: conversation,
            character_id: character,
            continuity_id: conversation,
            function_id: Some("chat".into()),
        };
        let path = temp
            .path()
            .join("spaces")
            .join(space.to_string())
            .join("memory");
        let memory = MemoryWorkspace::initialize(&path)?;
        for index in 0..count {
            let document = MemoryDocument {
                metadata: Metadata {
                    id: format!("review_event_{index:05}"),
                    kind: "event".into(),
                    importance: Some(0.5),
                    weight: Some(1.0),
                    touch_at: 1_700_000_000,
                    decay_at: Some(1_700_000_000),
                    archived_at: None,
                    relations: BTreeMap::new(),
                    tags: vec![format!("fixture-{index:05}")],
                    aliases: vec![],
                    injection_scope: None,
                    injection_conversation_id: Some(conversation.to_string()),
                    injection_character_id: Some(character.to_string()),
                    status: "active".into(),
                },
                body: format!("# Synthetic record {index:05}\n\nOffline synthetic narrative.\n"),
            };
            fs::write(
                path.join("events").join(format!("{index:05}.md")),
                document.encode()?,
            )?;
        }
        memory.rebuild_index()?;
        drop(memory);
        let runtime = MomoRuntime::initialize(temp.path()).await?;
        let request = |scoped: bool| ScopedMemoryRequest {
            identity: scoped.then(|| identity.clone()),
            spaces: vec![MemorySpaceSource {
                space_id: space.to_string(),
                label: "review".into(),
                weight: 100,
                memory: true,
                semantic_graph: false,
            }],
            observe_space_ids: vec![],
            query: format!("fixture-{:05}", count - 1),
            max_tokens: 2048,
            vector_space_id: None,
            query_vector: None,
            embedding: None,
        };
        for scoped in [false, true] {
            let items = retrieve_scoped_memory(&runtime, request(scoped)).await?;
            assert!(items.iter().any(|v| {
                v["id"]
                    .as_str()
                    .is_some_and(|id| id.ends_with(&format!("review_event_{:05}", count - 1)))
            }));
        }
        for scoped in [false, true] {
            let mut samples = Vec::new();
            for _ in 0..3 {
                let start = Instant::now();
                let items = retrieve_scoped_memory(&runtime, request(scoped)).await?;
                assert!(items.iter().any(|v| {
                    v["id"]
                        .as_str()
                        .is_some_and(|id| id.ends_with(&format!("review_event_{:05}", count - 1)))
                }));
                samples.push(start.elapsed().as_millis());
            }
            samples.sort_unstable();
            println!(
                "documents={count} identity={scoped} samples_ms={samples:?} median_ms={}",
                samples[1]
            );
        }
    }
    Ok(())
}
