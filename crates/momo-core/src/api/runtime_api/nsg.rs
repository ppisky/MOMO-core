//! Semantic-graph node management and vector ranking data.

use super::*;

fn nsg_workspace(
    core: &MomoCore,
    scope_id: uuid::Uuid,
) -> Result<momo_memory::nsg::NsgWorkspace, RuntimeApiError> {
    let memory = core
        .memory_for_space(scope_id)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    momo_memory::nsg::NsgWorkspace::initialize(memory.root())
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn list_nsg_embedding_documents(
    runtime: &MomoRuntime,
    scope_id: String,
) -> Result<Vec<momo_memory::nsg::NsgEmbeddingDocument>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let documents = run_space_write(
        runtime,
        scope_id,
        "list NSG embedding documents",
        move || {
            nsg_workspace(core.as_ref(), scope_id)?
                .embedding_documents()
                .map_err(|error| RuntimeApiError::internal(error.to_string()))
        },
    )
    .await?;
    Ok(documents)
}

pub async fn nsg_vector_status(
    runtime: &MomoRuntime,
    scope_id: String,
    vector_space_id: String,
) -> Result<serde_json::Value, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let documents = run_space_write(
        runtime,
        scope_id,
        "read NSG vector status sources",
        move || {
            nsg_workspace(core.as_ref(), scope_id)?
                .embedding_documents()
                .map_err(|error| RuntimeApiError::internal(error.to_string()))
        },
    )
    .await?;
    let hashes = documents
        .iter()
        .map(|document| (document.node_id.clone(), document.source_hash.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let status = runtime
        .core()
        .vector_store()
        .nsg_vector_status(scope_id, &vector_space_id, &hashes)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(json!({
        "vector_space_id": status.vector_space_id,
        "node_count": status.node_count,
        "indexed_count": status.indexed_count,
        "stale_count": status.stale_count,
        "missing_count": status.missing_count,
        "dimension": status.dimension,
        "enabled": !vector_space_id.trim().is_empty(),
    }))
}

pub async fn run_memory_maintenance(
    runtime: &MomoRuntime,
    scope_id: String,
) -> Result<momo_memory::MaintenanceReport, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let report = run_space_write(runtime, scope_id, "run memory maintenance", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .run_maintenance()
            .map_err(|error| RuntimeApiError::internal(error.to_string()))
    })
    .await?;
    Ok(report)
}

pub async fn apply_nsg_patch(
    runtime: &MomoRuntime,
    scope_id: String,
    patch_yaml: String,
    manual_authority: bool,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "apply NSG patch", move || {
        let nsg = nsg_workspace(core.as_ref(), scope_id)?;
        if manual_authority {
            nsg.apply_patch_authorized(&patch_yaml)
        } else {
            nsg.apply_patch(&patch_yaml)
        }
        .map_err(|error| RuntimeApiError::internal(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn validate_nsg_patch(
    runtime: &MomoRuntime,
    scope_id: String,
    patch_yaml: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "validate NSG patch", move || {
        nsg_workspace(core.as_ref(), scope_id)?
            .validate_patch(&patch_yaml)
            .map_err(|error| RuntimeApiError::invalid(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn list_nsg_pending_candidates(
    runtime: &MomoRuntime,
    scope_id: String,
) -> Result<Vec<momo_memory::nsg::NsgPendingCandidate>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let candidates = run_space_write(
        runtime,
        scope_id,
        "list NSG pending candidates",
        move || {
            nsg_workspace(core.as_ref(), scope_id)?
                .list_pending_candidates()
                .map_err(|error| RuntimeApiError::internal(error.to_string()))
        },
    )
    .await?;
    Ok(candidates)
}

pub async fn approve_nsg_pending_candidate(
    runtime: &MomoRuntime,
    scope_id: String,
    pending_path: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(
        runtime,
        scope_id,
        "approve NSG pending candidate",
        move || {
            nsg_workspace(core.as_ref(), scope_id)?
                .approve_pending_candidate(&pending_path)
                .map_err(|error| RuntimeApiError::invalid(error.to_string()))
        },
    )
    .await?;
    Ok(())
}

pub async fn reject_nsg_pending_candidate(
    runtime: &MomoRuntime,
    scope_id: String,
    pending_path: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(
        runtime,
        scope_id,
        "reject NSG pending candidate",
        move || {
            nsg_workspace(core.as_ref(), scope_id)?
                .reject_pending_candidate(&pending_path)
                .map_err(|error| RuntimeApiError::invalid(error.to_string()))
        },
    )
    .await?;
    Ok(())
}

pub async fn list_nsg_nodes(
    runtime: &MomoRuntime,
    scope_id: String,
    include_archived: bool,
) -> Result<Vec<momo_memory::nsg::ManagedNsgNode>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let nodes = run_space_write(runtime, scope_id, "list NSG nodes", move || {
        nsg_workspace(core.as_ref(), scope_id)?
            .list_nodes(include_archived)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))
    })
    .await?;
    Ok(nodes)
}

pub async fn write_nsg_node(
    runtime: &MomoRuntime,
    scope_id: String,
    target_file: String,
    node: momo_memory::nsg::NsgNode,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "write NSG node", move || {
        nsg_workspace(core.as_ref(), scope_id)?
            .write_node(&target_file, node)
            .map_err(|error| RuntimeApiError::invalid(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn archive_nsg_node(
    runtime: &MomoRuntime,
    scope_id: String,
    target_file: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "archive NSG node", move || {
        nsg_workspace(core.as_ref(), scope_id)?
            .archive_node(&target_file)
            .map_err(|error| RuntimeApiError::not_found(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn delete_nsg_node(
    runtime: &MomoRuntime,
    scope_id: String,
    target_file: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "delete NSG node", move || {
        nsg_workspace(core.as_ref(), scope_id)?
            .delete_node(&target_file)
            .map_err(|error| RuntimeApiError::not_found(error.to_string()))
    })
    .await?;
    Ok(())
}
