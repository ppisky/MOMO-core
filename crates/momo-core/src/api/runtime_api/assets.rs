//! Manual content and authorization changes share one durable Space transaction.
use super::*;
use momo_domain::provenance::RecordPolicy;
use momo_memory::{
    MemoryDocument, MemoryError, Metadata,
    nsg::{NsgNode, NsgWorkspace},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssetContent {
    Memory { metadata: Metadata, body: String },
    SemanticGraph { node: NsgNode },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetEdit {
    pub space_id: uuid::Uuid,
    /// None creates a new identity; Some(0) edits an untracked legacy record.
    pub expected_revision: Option<u64>,
    pub content: AssetContent,
    pub policy: RecordPolicy,
}

pub async fn configure_asset(runtime: &MomoRuntime, edit: AssetEdit) -> RuntimeApiResult<()> {
    let space = edit.space_id;
    if edit.policy.local_space_id != space {
        return Err(RuntimeApiError::invalid("policy Space differs from target"));
    }
    edit.policy.validate().map_err(RuntimeApiError::invalid)?;
    let core = runtime.core_handle();
    run_space_write(runtime, space, "configure asset", move || {
        core.memory_for_space(space)
            .map_err(asset_error)?
            .call(move |workspace| {
                let (graph, id) = match &edit.content {
                    AssetContent::Memory { metadata, .. } => (false, metadata.id.clone()),
                    AssetContent::SemanticGraph { node } => (true, node.id.clone()),
                };
                let ledger = workspace.provenance(graph)?;
                let history = ledger.records.get(&id);
                let revision = history.map_or(0, |history| history.revisions.len() as u64);
                let nodes = NsgWorkspace::initialize(workspace.root())?.list_nodes(true)?;
                let exists = if graph {
                    nodes.iter().any(|entry| entry.node.id == id)
                } else {
                    workspace
                        .list_documents()?
                        .iter()
                        .any(|entry| entry.id == id)
                };
                if (edit.expected_revision.is_none() && (exists || history.is_some()))
                    || (edit.expected_revision.is_some()
                        && (!exists || edit.expected_revision != Some(revision)))
                {
                    return Err(MemoryError::RevisionConflict {
                        expected: format!("{:?}", edit.expected_revision),
                        actual: revision.to_string(),
                    });
                }
                match edit.content {
                    AssetContent::Memory { metadata, body } => {
                        workspace.write_document(MemoryDocument { metadata, body })?;
                    }
                    AssetContent::SemanticGraph { node } => {
                        let original = nodes.iter().find(|entry| entry.node.id == id);
                        if original.map_or(!node.source_character_ids.is_empty(), |entry| {
                            entry.node.source_character_ids != node.source_character_ids
                                || entry.node.graph_id != node.graph_id
                        }) {
                            return Err(MemoryError::InvalidPatch(
                                "source attribution and graph identity are immutable".into(),
                            ));
                        }
                        for edge in &node.edges {
                            if edge.target != node.id
                                && !nodes.iter().any(|entry| {
                                    entry.node.id == edge.target
                                        && entry.node.graph_id == node.graph_id
                                })
                            {
                                return Err(MemoryError::InvalidPatch(
                                    "edge target must exist in the selected graph".into(),
                                ));
                            }
                        }
                        let target = nodes
                            .iter()
                            .find(|entry| entry.node.id == id)
                            .map(|entry| entry.path.clone())
                            .unwrap_or_else(|| format!("rules/{}.nsg", uuid::Uuid::now_v7()));
                        NsgWorkspace::initialize(workspace.root())?.write_node(&target, node)?;
                    }
                }
                // This runs against the worker's private staged snapshot. A failed
                // policy validation rolls back content too, before any publication.
                workspace.set_record_policy(graph, &id, edit.policy)
            })
            .map_err(asset_error)
    })
    .await
}

fn asset_error(error: MemoryError) -> RuntimeApiError {
    match error {
        MemoryError::RevisionConflict { .. } => RuntimeApiError::conflict(error.to_string()),
        _ => RuntimeApiError::invalid(error.to_string()),
    }
}
