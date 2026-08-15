use std::{collections::HashMap, env, time::Instant};

use chrono::Utc;
use momo_domain::new_id;
use momo_storage::{LocalStore, NsgVectorRecord, NsgVectorStore};

const DEFAULT_NODE_COUNT: usize = 5_000;
const DEFAULT_DIMENSION: usize = 384;
const DEFAULT_TOP_K: usize = 64;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node_count = argument(1, DEFAULT_NODE_COUNT)?;
    let dimension = argument(2, DEFAULT_DIMENSION)?;
    let top_k = argument(3, DEFAULT_TOP_K)?;
    if node_count == 0 || dimension == 0 || dimension > 8_192 {
        return Err("node_count must be positive and dimension must be between 1 and 8192".into());
    }

    let owner_id = new_id();
    let vector_space_id = format!("benchmark|deterministic|{dimension}");
    let records = (0..node_count)
        .map(|node| NsgVectorRecord {
            owner_id,
            node_id: format!("node_{node:08}"),
            source_hash: format!("{:064x}", node + 1),
            vector_space_id: vector_space_id.clone(),
            dimension,
            vector: deterministic_vector(node, dimension),
            created_at: Utc::now(),
        })
        .collect::<Vec<_>>();
    let current_hashes = records
        .iter()
        .map(|record| (record.node_id.clone(), record.source_hash.clone()))
        .collect::<HashMap<_, _>>();
    let query = deterministic_vector(node_count, dimension);
    let store = LocalStore::in_memory().await?;

    let write_started = Instant::now();
    store.upsert_nsg_vectors(&records).await?;
    let write_elapsed = write_started.elapsed();

    let search_started = Instant::now();
    let ranked = store
        .rank_nsg_vectors(owner_id, &vector_space_id, &query, &current_hashes, top_k)
        .await?;
    let search_elapsed = search_started.elapsed();

    println!("nodes={node_count} dimension={dimension} top_k={top_k}");
    println!("write_ms={:.3}", write_elapsed.as_secs_f64() * 1_000.0);
    println!(
        "exact_search_ms={:.3}",
        search_elapsed.as_secs_f64() * 1_000.0
    );
    println!("results={}", ranked.len());
    Ok(())
}

fn argument(position: usize, default: usize) -> Result<usize, Box<dyn std::error::Error>> {
    env::args()
        .nth(position)
        .map_or(Ok(default), |value| Ok(value.parse()?))
}

fn deterministic_vector(seed: usize, dimension: usize) -> Vec<f64> {
    (0..dimension)
        .map(|axis| {
            let value = ((seed + 1).wrapping_mul(axis + 3) % 997) as f64;
            value / 997.0 - 0.5
        })
        .collect()
}
