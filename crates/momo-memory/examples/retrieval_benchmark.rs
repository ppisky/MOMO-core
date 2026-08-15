use std::{collections::BTreeMap, env, fs, process::ExitCode, time::Instant};

use momo_memory::{ConservativeTokenCounter, MemoryDocument, MemoryWorkspace, Metadata};
use tempfile::TempDir;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("benchmark failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let document_count = env::args()
        .nth(1)
        .ok_or("usage: retrieval_benchmark <document-count>")?
        .parse::<usize>()?;
    if document_count == 0 {
        return Err("document-count must be greater than zero".into());
    }
    let trials = env::args()
        .nth(2)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(25);
    if trials == 0 {
        return Err("trials must be greater than zero".into());
    }

    let temp = TempDir::new()?;
    let memory = MemoryWorkspace::initialize(temp.path())?;
    let fixture_started = Instant::now();
    for index in 0..document_count {
        let id = format!("benchmark_event_{index:05}");
        let title = if index + 1 == document_count {
            format!("Needle memory {index:05}")
        } else {
            format!("Ordinary memory {index:05}")
        };
        let document = MemoryDocument {
            metadata: Metadata {
                id,
                kind: "event".to_owned(),
                importance: Some(0.5),
                weight: Some(1.0),
                touch_at: 1_700_000_000,
                decay_at: Some(1_700_000_000),
                archived_at: None,
                relations: BTreeMap::new(),
                tags: vec![format!("fixture-{index:05}")],
                aliases: Vec::new(),
                injection_scope: None,
                injection_conversation_id: None,
                injection_character_id: None,
                status: "active".to_owned(),
            },
            body: format!("# {title}\n\nSynthetic DMW retrieval benchmark record {index:05}.\n"),
        };
        fs::write(
            memory.root().join("events").join(format!("{index:05}.md")),
            document.encode()?,
        )?;
    }
    let fixture_ms = fixture_started.elapsed().as_millis();

    let index_started = Instant::now();
    let indexed = memory.rebuild_index()?;
    let index_ms = index_started.elapsed().as_millis();

    // Warm the filesystem and parser caches once, then record repeated
    // end-to-end retrievals, including activity-index writes.
    let counter = ConservativeTokenCounter;
    let query = format!("Please recall Needle memory {:05}", document_count - 1);
    let warm = memory.retrieve(&query, 2_048, &counter)?;
    if !warm
        .iter()
        .any(|item| item.id == format!("benchmark_event_{:05}", document_count - 1))
    {
        return Err("the expected benchmark document was not retrieved".into());
    }

    let mut samples_us = Vec::with_capacity(trials);
    for _ in 0..trials {
        let started = Instant::now();
        let results = memory.retrieve(&query, 2_048, &counter)?;
        if results.is_empty() {
            return Err("retrieval unexpectedly returned no documents".into());
        }
        samples_us.push(started.elapsed().as_micros());
    }
    samples_us.sort_unstable();

    println!("documents={document_count}");
    println!("indexed_documents={indexed}");
    println!("fixture_write_ms={fixture_ms}");
    println!("index_rebuild_ms={index_ms}");
    println!("retrieval_trials={trials}");
    println!("retrieval_p50_us={}", percentile(&samples_us, 50));
    println!("retrieval_p95_us={}", percentile(&samples_us, 95));
    println!("retrieval_p99_us={}", percentile(&samples_us, 99));
    println!("retrieval_max_us={}", samples_us[samples_us.len() - 1]);
    Ok(())
}

fn percentile(samples: &[u128], percentile: usize) -> u128 {
    let rank = (samples.len() * percentile).div_ceil(100).max(1);
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}
