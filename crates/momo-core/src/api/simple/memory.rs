//! User-memory retrieval, maintenance, patch review, and document management.

use super::*;

const MAX_MEMORY_SCOPES: usize = 8;

/// A client-defined memory namespace participating in one retrieval. Core does
/// not attach platform semantics to the namespace; labels are returned only so
/// the host can keep personal, room, project, or other memories distinguishable.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MemoryScopeSource {
    pub scope_id: String,
    pub label: String,
    #[serde(default = "default_scope_weight")]
    pub weight: usize,
}

const fn default_scope_weight() -> usize {
    1
}

/// Retrieve from several isolated memory workspaces while preserving the
/// source namespace on every result. The total token budget is divided by
/// caller-provided weights; platform identity and ACL rules stay in the host.
pub async fn retrieve_scoped_memory_json(
    sources_json: String,
    query: String,
    max_tokens: usize,
    include_memory: bool,
    include_semantic_graph: bool,
    vector_space_id: Option<String>,
    query_vector_json: Option<String>,
) -> Result<String, String> {
    let sources: Vec<MemoryScopeSource> =
        serde_json::from_str(&sources_json).map_err(|error| error.to_string())?;
    validate_memory_scopes(&sources)?;
    if !include_memory && !include_semantic_graph {
        return Err("at least one retrieval component must be enabled".to_owned());
    }
    match (&vector_space_id, &query_vector_json) {
        (Some(_), Some(_)) | (None, None) => {}
        _ => {
            return Err("vector_space_id and query_vector must be provided together".to_owned());
        }
    }
    if !include_semantic_graph && vector_space_id.is_some() {
        return Err("vector retrieval requires semantic graph retrieval".to_owned());
    }

    let budgets = weighted_scope_budgets(&sources, max_tokens);
    let mut combined = Vec::new();
    for (source, budget) in sources.into_iter().zip(budgets) {
        if budget == 0 {
            continue;
        }
        let retrieved = match (&vector_space_id, &query_vector_json) {
            (Some(space), Some(vector)) => {
                retrieve_memory_with_vector_json(
                    source.scope_id.clone(),
                    query.clone(),
                    budget,
                    include_memory,
                    include_semantic_graph,
                    space.clone(),
                    vector.clone(),
                )
                .await?
            }
            (None, None) => {
                let scope_id =
                    uuid::Uuid::parse_str(&source.scope_id).map_err(|error| error.to_string())?;
                retrieve_memory_with_ranked_nsg(
                    scope_id,
                    query.clone(),
                    budget,
                    Vec::new(),
                    include_memory,
                    include_semantic_graph,
                )
                .await?
            }
            _ => unreachable!("validated vector arguments"),
        };
        let values: Vec<serde_json::Value> =
            serde_json::from_str(&retrieved).map_err(|error| error.to_string())?;
        for mut value in values {
            let object = value
                .as_object_mut()
                .ok_or_else(|| "memory retrieval result was not an object".to_owned())?;
            object.insert(
                "memory_scope".to_owned(),
                serde_json::json!({
                    "id": source.scope_id,
                    "label": source.label,
                }),
            );
            combined.push(value);
        }
    }
    serde_json::to_string(&combined).map_err(|error| error.to_string())
}

fn validate_memory_scopes(sources: &[MemoryScopeSource]) -> Result<(), String> {
    if sources.is_empty() || sources.len() > MAX_MEMORY_SCOPES {
        return Err(format!(
            "memory retrieval requires between 1 and {MAX_MEMORY_SCOPES} scopes"
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for source in sources {
        let scope_id =
            uuid::Uuid::parse_str(&source.scope_id).map_err(|error| error.to_string())?;
        if !seen.insert(scope_id) {
            return Err("memory retrieval scopes must be unique".to_owned());
        }
        if source.label.trim().is_empty() || source.label.chars().count() > 64 {
            return Err("memory scope labels must contain 1 to 64 characters".to_owned());
        }
        if source.weight == 0 || source.weight > 1_000 {
            return Err("memory scope weights must be between 1 and 1000".to_owned());
        }
    }
    Ok(())
}

fn weighted_scope_budgets(sources: &[MemoryScopeSource], max_tokens: usize) -> Vec<usize> {
    let total_weight = sources.iter().map(|source| source.weight).sum::<usize>();
    let mut budgets = sources
        .iter()
        .map(|source| max_tokens.saturating_mul(source.weight) / total_weight)
        .collect::<Vec<_>>();
    let assigned = budgets.iter().sum::<usize>();
    let remainder = max_tokens.saturating_sub(assigned);
    for index in 0..remainder {
        let target = index % budgets.len();
        budgets[target] = budgets[target].saturating_add(1);
    }
    budgets
}

pub async fn retrieve_memory_json(
    scope_id: String,
    query: String,
    max_tokens: usize,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    retrieve_memory_with_ranked_nsg(scope_id, query, max_tokens, Vec::new(), true, true).await
}

async fn retrieve_memory_with_ranked_nsg(
    scope_id: uuid::Uuid,
    query: String,
    max_tokens: usize,
    vector_ranked_ids: Vec<String>,
    include_memory: bool,
    include_semantic_graph: bool,
) -> Result<String, String> {
    let workspace = core()?
        .memory_for(scope_id)
        .map_err(|error| error.to_string())?;
    let memory_budget = match (include_memory, include_semantic_graph) {
        (true, true) => max_tokens.saturating_mul(3) / 4,
        (true, false) => max_tokens,
        (false, _) => 0,
    };
    let memories = if include_memory {
        workspace
            .retrieve(
                &query,
                memory_budget,
                &momo_memory::ConservativeTokenCounter,
            )
            .map_err(|error| error.to_string())?
    } else {
        Vec::new()
    };
    let memory_tokens = memories
        .iter()
        .map(|item| item.estimated_tokens)
        .sum::<usize>();
    let nsg = if include_semantic_graph {
        momo_memory::nsg::NsgWorkspace::initialize(workspace.root())
            .map_err(|error| error.to_string())?
            .retrieve(
                &query,
                &vector_ranked_ids,
                max_tokens.saturating_sub(memory_tokens),
                &momo_memory::ConservativeTokenCounter,
            )
            .map_err(|error| error.to_string())?
    } else {
        Vec::new()
    };
    let mut combined = serde_json::to_value(memories)
        .map_err(|error| error.to_string())?
        .as_array()
        .cloned()
        .unwrap_or_default();
    combined.extend(
        serde_json::to_value(nsg)
            .map_err(|error| error.to_string())?
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    serde_json::to_string(&combined).map_err(|error| error.to_string())
}

pub async fn compile_mo_state_json(
    scope_id: String,
    retrieved_memory_json: String,
    retrieved_nsg_json: String,
    max_context_tokens: usize,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let retrieved_memory: Vec<momo_memory::RetrievedMemory> =
        serde_json::from_str(&retrieved_memory_json).map_err(|error| error.to_string())?;
    let retrieved_nsg: Vec<momo_memory::nsg::RetrievedNsg> =
        serde_json::from_str(&retrieved_nsg_json).map_err(|error| error.to_string())?;
    let workspace = core()?
        .memory_for(scope_id)
        .map_err(|error| error.to_string())?;
    let context = workspace
        .compile_mo_state(
            &retrieved_memory,
            &retrieved_nsg,
            max_context_tokens,
            &momo_memory::ConservativeTokenCounter,
        )
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&context).map_err(|error| error.to_string())
}

pub async fn retrieve_memory_with_vector_json(
    scope_id: String,
    query: String,
    max_tokens: usize,
    include_memory: bool,
    include_semantic_graph: bool,
    vector_space_id: String,
    query_vector_json: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    if !include_semantic_graph {
        return Err("vector retrieval requires semantic graph retrieval".to_owned());
    }
    let query_vector: Vec<f64> =
        serde_json::from_str(&query_vector_json).map_err(|error| error.to_string())?;
    if vector_space_id.trim().is_empty()
        || query_vector.is_empty()
        || query_vector.len() > 8192
        || query_vector.iter().any(|value| !value.is_finite())
    {
        return Err("invalid semantic-graph query vector".to_owned());
    }
    let workspace = core()?
        .memory_for(scope_id)
        .map_err(|error| error.to_string())?;
    let nsg = momo_memory::nsg::NsgWorkspace::initialize(workspace.root())
        .map_err(|error| error.to_string())?;
    let documents = nsg
        .embedding_documents()
        .map_err(|error| error.to_string())?;
    let hashes = documents
        .iter()
        .map(|document| (document.node_id.clone(), document.source_hash.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let ranked = core()?
        .vector_store()
        .rank_nsg_vectors(
            scope_id,
            &vector_space_id,
            &query_vector,
            &hashes,
            DEFAULT_NSG_VECTOR_TOP_K,
        )
        .await
        .map_err(|error| error.to_string())?;
    retrieve_memory_with_ranked_nsg(
        scope_id,
        query,
        max_tokens,
        ranked,
        include_memory,
        include_semantic_graph,
    )
    .await
}

pub async fn apply_memory_patch_json(
    scope_id: String,
    patch_yaml: String,
) -> Result<String, String> {
    let scope_id = uuid::Uuid::parse_str(&scope_id).map_err(|error| error.to_string())?;
    let workspace = core()?
        .memory_for(scope_id)
        .map_err(|error| error.to_string())?;
    workspace
        .apply_patch(&patch_yaml)
        .map_err(|error| error.to_string())?;
    stage_memory_snapshot(scope_id).await?;
    Ok("ok".to_owned())
}

pub async fn submit_memory_patch_review_json(
    owner_id: String,
    conversation_id: String,
    patch_yaml: String,
    review_mode: String,
) -> Result<String, String> {
    if !matches!(
        review_mode.as_str(),
        "auto_approve" | "require_confirmation" | "reject"
    ) {
        return Err(format!(
            "unsupported memory patch review mode: {review_mode}"
        ));
    }
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let summary = core()?
        .memory_for(owner_id)
        .map_err(|error| error.to_string())?
        .summarize_patch(&patch_yaml)
        .map_err(|error| error.to_string())?;
    let operation_count =
        i64::try_from(summary.operation_count).map_err(|error| error.to_string())?;
    let review = core()?
        .store()
        .create_memory_patch_review(
            owner_id,
            &conversation_id,
            &patch_yaml,
            &summary.targets,
            operation_count,
            &review_mode,
        )
        .await
        .map_err(|error| error.to_string())?;

    match review_mode.as_str() {
        "auto_approve" => approve_memory_patch_review(owner_id, review.id).await,
        "reject" => {
            let resolved = core()?
                .store()
                .resolve_memory_patch_review(
                    owner_id,
                    review.id,
                    momo_storage::MemoryPatchReviewStatus::Rejected,
                    Some("rejected_by_policy"),
                    None,
                )
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "memory patch review was already resolved".to_owned())?;
            serde_json::to_string(&resolved).map_err(|error| error.to_string())
        }
        _ => serde_json::to_string(&review).map_err(|error| error.to_string()),
    }
}

pub async fn list_memory_patch_reviews_json(
    owner_id: String,
    include_resolved: bool,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let reviews = core()?
        .store()
        .list_memory_patch_reviews(owner_id, include_resolved)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&reviews).map_err(|error| error.to_string())
}

pub async fn approve_memory_patch_review_json(
    owner_id: String,
    review_id: String,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let review_id = uuid::Uuid::parse_str(&review_id).map_err(|error| error.to_string())?;
    approve_memory_patch_review(owner_id, review_id).await
}

pub async fn reject_memory_patch_review_json(
    owner_id: String,
    review_id: String,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let review_id = uuid::Uuid::parse_str(&review_id).map_err(|error| error.to_string())?;
    let _guard = MEMORY_PATCH_REVIEW_LOCK.lock().await;
    let existing = core()?
        .store()
        .memory_patch_review(owner_id, review_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "memory patch review was not found".to_owned())?;
    if existing.status != momo_storage::MemoryPatchReviewStatus::Pending {
        return serde_json::to_string(&existing).map_err(|error| error.to_string());
    }
    let resolved = core()?
        .store()
        .resolve_memory_patch_review(
            owner_id,
            review_id,
            momo_storage::MemoryPatchReviewStatus::Rejected,
            Some("rejected_by_user"),
            None,
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "memory patch review was already resolved".to_owned())?;
    serde_json::to_string(&resolved).map_err(|error| error.to_string())
}

pub async fn list_memory_documents_json(owner_id: String) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let documents = core()?
        .memory_for(owner_id)
        .map_err(|error| error.to_string())?
        .list_documents()
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&documents).map_err(|error| error.to_string())
}

pub async fn read_memory_document_json(
    owner_id: String,
    document_id: String,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let document = core()?
        .memory_for(owner_id)
        .map_err(|error| error.to_string())?
        .read_document_by_id(&document_id)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&serde_json::json!({
        "id": document.metadata.id,
        "type": document.metadata.kind,
        "importance": document.metadata.importance,
        "weight": document.metadata.weight,
        "touch_at": document.metadata.touch_at,
        "status": document.metadata.status,
        "tags": document.metadata.tags,
        "relations": document.metadata.relations,
        "injection_scope": document.metadata.injection_scope,
        "injection_conversation_id": document.metadata.injection_conversation_id,
        "injection_character_id": document.metadata.injection_character_id,
        "body": document.body,
    }))
    .map_err(|error| error.to_string())
}

pub async fn update_memory_document_json(
    owner_id: String,
    document_id: String,
    markdown: String,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let workspace = core()?
        .memory_for(owner_id)
        .map_err(|error| error.to_string())?;
    let _document = workspace
        .read_document_by_id(&document_id)
        .map_err(|error| error.to_string())?;
    workspace
        .replace_document_body(&document_id, &markdown)
        .map_err(|error| error.to_string())?;
    stage_memory_snapshot(owner_id).await?;
    Ok("ok".to_owned())
}

pub async fn archive_memory_document_json(
    owner_id: String,
    document_id: String,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let workspace = core()?
        .memory_for(owner_id)
        .map_err(|error| error.to_string())?;
    let _document = workspace
        .read_document_by_id(&document_id)
        .map_err(|error| error.to_string())?;
    let index_path = workspace.root().join("indexes/memory_index.yaml");
    let index_text = std::fs::read_to_string(&index_path).map_err(|error| error.to_string())?;
    let entry_path = find_entry_path(&index_text, &document_id)?;
    let yaml_patch = format!(
        "patches:\n  - target_file: \"{entry_path}\"\n    operations:\n      - type: update_frontmatter\n        fields:\n          status: archived\n          weight: 0.1\n",
    );
    workspace
        .apply_patch(&yaml_patch)
        .map_err(|error| error.to_string())?;
    workspace
        .run_maintenance()
        .map_err(|error| error.to_string())?;
    stage_memory_snapshot(owner_id).await?;
    Ok("ok".to_owned())
}

pub async fn restore_memory_document_json(
    owner_id: String,
    document_id: String,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let workspace = core()?
        .memory_for(owner_id)
        .map_err(|error| error.to_string())?;
    workspace
        .restore_archived_authorized(&document_id)
        .map_err(|error| error.to_string())?;
    stage_memory_snapshot(owner_id).await?;
    Ok("ok".to_owned())
}

pub async fn delete_memory_document_json(
    owner_id: String,
    document_id: String,
) -> Result<String, String> {
    let owner_id = uuid::Uuid::parse_str(&owner_id).map_err(|error| error.to_string())?;
    let workspace = core()?
        .memory_for(owner_id)
        .map_err(|error| error.to_string())?;
    workspace
        .delete_document_authorized(&document_id)
        .map_err(|error| error.to_string())?;
    stage_memory_snapshot(owner_id).await?;
    Ok("ok".to_owned())
}

#[cfg(test)]
mod scoped_tests {
    use super::*;

    fn source(id: &str, label: &str, weight: usize) -> MemoryScopeSource {
        MemoryScopeSource {
            scope_id: id.to_owned(),
            label: label.to_owned(),
            weight,
        }
    }

    #[test]
    fn scoped_budget_is_weighted_and_conserves_total() {
        let sources = [
            source("00000000-0000-4000-8000-000000000001", "personal", 3),
            source("00000000-0000-4000-8000-000000000002", "room", 2),
        ];
        let budgets = weighted_scope_budgets(&sources, 1_024);
        assert_eq!(budgets, [615, 409]);
        assert_eq!(budgets.iter().sum::<usize>(), 1_024);
    }

    #[test]
    fn scoped_sources_require_unique_valid_ids() {
        let sources = [
            source("00000000-0000-4000-8000-000000000001", "personal", 1),
            source("00000000-0000-4000-8000-000000000001", "room", 1),
        ];
        assert!(validate_memory_scopes(&sources).is_err());
    }
}
