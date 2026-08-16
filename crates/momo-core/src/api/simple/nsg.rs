//! Semantic-graph node management and vector ranking data.

use super::*;

pub async fn list_nsg_embedding_documents_json(scope_id: String) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    serde_json::to_string(
        &nsg.embedding_documents()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

pub async fn save_nsg_vectors_json(
    scope_id: String,
    vector_space_id: String,
    vectors_json: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let records: Vec<momo_storage::NsgVectorRecord> =
        serde_json::from_str(&vectors_json).map_err(|error| error.to_string())?;
    if records.len() > 512 {
        return Err("too many semantic-graph vectors in one batch".to_owned());
    }
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    let source_hashes = nsg
        .embedding_documents()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|document| (document.node_id, document.source_hash))
        .collect::<std::collections::HashMap<_, _>>();
    for record in &records {
        if record.scope_id != scope_id
            || record.vector_space_id != vector_space_id
            || source_hashes.get(&record.node_id) != Some(&record.source_hash)
        {
            return Err(
                "semantic-graph vector no longer matches this account, node, or source".to_owned(),
            );
        }
    }
    core()?
        .vector_store()
        .upsert_nsg_vectors(&records)
        .await
        .map_err(|error| error.to_string())?;
    Ok(records.len().to_string())
}

pub async fn nsg_vector_status_json(
    scope_id: String,
    vector_space_id: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    let documents = nsg
        .embedding_documents()
        .map_err(|error| error.to_string())?;
    let hashes = documents
        .iter()
        .map(|document| (document.node_id.clone(), document.source_hash.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let status = core()?
        .vector_store()
        .nsg_vector_status(scope_id, &vector_space_id, &hashes)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&serde_json::json!({
        "node_count": status.node_count,
        "indexed_count": status.indexed_count,
        "stale_count": status.stale_count,
        "missing_count": status.missing_count,
        "dimension": status.dimension,
        "enabled": !vector_space_id.trim().is_empty(),
    }))
    .map_err(|error| error.to_string())
}

pub async fn run_memory_maintenance_json(scope_id: String) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let report = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?
        .run_maintenance()
        .map_err(|error| error.to_string())?;
    if !report.decayed_ids.is_empty()
        || !report.archived_ids.is_empty()
        || !report.forgotten_ids.is_empty()
    {
        stage_memory_snapshot(scope_id).await?;
    }
    serde_json::to_string(&report).map_err(|error| error.to_string())
}

pub async fn apply_nsg_patch_json(
    scope_id: String,
    patch_yaml: String,
    manual_authority: bool,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    if manual_authority {
        nsg.apply_patch_authorized(&patch_yaml)
    } else {
        nsg.apply_patch(&patch_yaml)
    }
    .map_err(|error| error.to_string())?;
    stage_semantic_graph_snapshot(scope_id).await?;
    Ok("ok".to_owned())
}

pub async fn list_nsg_pending_candidates_json(scope_id: String) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    serde_json::to_string(
        &nsg.list_pending_candidates()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

pub async fn approve_nsg_pending_candidate_json(
    scope_id: String,
    pending_path: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?
        .approve_pending_candidate(&pending_path)
        .map_err(|error| error.to_string())?;
    stage_semantic_graph_snapshot(scope_id).await?;
    Ok("ok".to_owned())
}

pub async fn reject_nsg_pending_candidate_json(
    scope_id: String,
    pending_path: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?
        .reject_pending_candidate(&pending_path)
        .map_err(|error| error.to_string())?;
    stage_semantic_graph_snapshot(scope_id).await?;
    Ok("ok".to_owned())
}

pub async fn list_nsg_nodes_json(
    scope_id: String,
    include_archived: bool,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    serde_json::to_string(
        &nsg.list_nodes(include_archived)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

pub async fn write_nsg_node_json(
    scope_id: String,
    target_file: String,
    node_json: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let node = serde_json::from_str(&node_json).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    nsg.write_node(&target_file, node)
        .map_err(|error| error.to_string())?;
    stage_semantic_graph_snapshot(scope_id).await?;
    Ok("ok".to_owned())
}

pub async fn archive_nsg_node_json(
    scope_id: String,
    target_file: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    nsg.archive_node(&target_file)
        .map_err(|error| error.to_string())?;
    stage_semantic_graph_snapshot(scope_id).await?;
    Ok("ok".to_owned())
}

pub async fn delete_nsg_node_json(scope_id: String, target_file: String) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let memory = core()?
        .memory_for_scope(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| error.to_string())?;
    nsg.delete_node(&target_file)
        .map_err(|error| error.to_string())?;
    stage_semantic_graph_snapshot(scope_id).await?;
    Ok("ok".to_owned())
}
