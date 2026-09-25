//! Scoped-memory retrieval, maintenance, patch review, and document management.

use super::*;

pub fn memory_recovery_status(
    runtime: &MomoRuntime,
) -> std::collections::BTreeMap<uuid::Uuid, String> {
    runtime.core().memory_recovery_status()
}

pub async fn retry_memory_recovery(
    runtime: &MomoRuntime,
    space_id: uuid::Uuid,
) -> RuntimeApiResult<()> {
    let _guard = runtime
        .lock_space(space_id)
        .await
        .map_err(RuntimeApiError::recovery)?;
    Ok(())
}

const MAX_MEMORY_SPACES: usize = 16;
const HYBRID_DMW_BUDGET_PERCENT: usize = 60;

/// A client-defined memory namespace participating in one retrieval. Core does
/// not attach platform semantics to the namespace; labels are returned only so
/// the host can keep personal, room, project, or other memories distinguishable.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySpaceSource {
    pub space_id: String,
    pub label: String,
    pub weight: u8,
    pub memory: bool,
    pub semantic_graph: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedMemoryRequest {
    pub spaces: Vec<MemorySpaceSource>,
    #[serde(default)]
    pub observe_space_ids: Vec<String>,
    pub query: String,
    pub max_tokens: usize,
    pub vector_space_id: Option<String>,
    pub query_vector: Option<Vec<f64>>,
    #[serde(default)]
    pub embedding: Option<EmbeddingRequestConfig>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ScopedMemorySnapshot {
    pub(crate) items: Vec<serde_json::Value>,
    pub(crate) source_observations: Vec<momo_storage::MoStateSourceObservation>,
}

struct VectorQuery {
    space_id: String,
    vector: Vec<f64>,
}

struct RetrievalPlan {
    scope_id: uuid::Uuid,
    query: String,
    max_tokens: usize,
    include_memory: bool,
    include_semantic_graph: bool,
    vector: Option<VectorQuery>,
}

/// Retrieve from several isolated memory workspaces while preserving the
/// source namespace on every result. The total token budget is divided by
/// caller-provided weights; platform identity and ACL rules stay in the host.
pub async fn retrieve_scoped_memory(
    runtime: &MomoRuntime,
    request: ScopedMemoryRequest,
) -> Result<Vec<serde_json::Value>, RuntimeApiError> {
    let snapshot = retrieve_scoped_memory_snapshot(runtime, request).await?;
    Ok(snapshot.items)
}

pub(crate) async fn retrieve_scoped_memory_snapshot(
    runtime: &MomoRuntime,
    request: ScopedMemoryRequest,
) -> Result<ScopedMemorySnapshot, RuntimeApiError> {
    if request.spaces.is_empty() {
        if request.observe_space_ids.is_empty() {
            return Err(RuntimeApiError::invalid(
                "memory retrieval requires at least one Space",
            ));
        }
    } else {
        validate_memory_spaces(&request.spaces)?;
    }
    let mut observed_space_ids = request
        .spaces
        .iter()
        .map(|source| source.space_id.clone())
        .chain(request.observe_space_ids.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    // Deduplicate and order locks by UUID identity, not its input spelling.
    // Keep the original strings for source labels and observation identities.
    let lock_space_ids = observed_space_ids
        .iter()
        .map(|space_id| {
            uuid::Uuid::parse_str(space_id)
                .map_err(|error| RuntimeApiError::invalid(error.to_string()))
        })
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    let has_semantic_graph = request.spaces.iter().any(|source| source.semantic_graph);
    match (&request.vector_space_id, &request.query_vector) {
        (Some(_), Some(_)) | (None, None) => {}
        _ => {
            return Err(RuntimeApiError::invalid(
                "vector_space_id and query_vector must be provided together",
            ));
        }
    }
    if request.embedding.is_some()
        && (request.vector_space_id.is_some() || request.query_vector.is_some())
    {
        return Err(RuntimeApiError::invalid(
            "embedding configuration cannot be combined with a caller-supplied raw vector",
        ));
    }
    if !has_semantic_graph && (request.vector_space_id.is_some() || request.embedding.is_some()) {
        return Err(RuntimeApiError::invalid(
            "vector retrieval requires semantic graph retrieval",
        ));
    }

    let generated_vector = if let Some(embedding) = request.embedding.as_ref() {
        Some(embed_query(embedding, &request.query).await?)
    } else {
        None
    };
    let vector_space_id = generated_vector
        .as_ref()
        .map(|(space_id, _)| space_id)
        .or(request.vector_space_id.as_ref())
        .cloned();
    let query_vector = generated_vector
        .as_ref()
        .map(|(_, vector)| vector)
        .or(request.query_vector.as_ref())
        .cloned();

    // All authoritative file-backed sources are held under the same ordered
    // lock set from retrieval through fingerprinting. The single Core process
    // owns the data directory, so writers cannot interleave a different
    // revision between a returned body and its recorded source identity.
    let state_guards = runtime
        .core()
        .lock_spaces(lock_space_ids)
        .await
        .map_err(RuntimeApiError::recovery)?;
    let core = runtime.core().clone();
    runtime
        .finish_commit(async move {
            let _state_guards = state_guards;
            let budgets = weighted_space_budgets(&request.spaces, request.max_tokens);
            let mut combined = Vec::new();
            for (source, budget) in request.spaces.into_iter().zip(budgets) {
                if budget == 0 {
                    continue;
                }
                let scope_id = uuid::Uuid::parse_str(&source.space_id)
                    .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
                let vector = vector_space_id.as_ref().zip(query_vector.as_ref()).map(
                    |(space_id, vector)| VectorQuery {
                        space_id: space_id.clone(),
                        vector: vector.clone(),
                    },
                );
                let values = retrieve_memory(
                    &core,
                    RetrievalPlan {
                        scope_id,
                        query: request.query.clone(),
                        max_tokens: budget,
                        include_memory: source.memory,
                        include_semantic_graph: source.semantic_graph,
                        vector,
                    },
                )
                .await?;
                for mut value in values {
                    let object = value.as_object_mut().ok_or_else(|| {
                        RuntimeApiError::internal("memory retrieval result was not an object")
                    })?;
                    object.insert(
                        "memory_space".to_owned(),
                        serde_json::json!({
                            "id": source.space_id,
                            "label": source.label,
                        }),
                    );
                    combined.push(value);
                }
            }
            let observed_spaces = std::mem::take(&mut observed_space_ids)
                .into_iter()
                .map(|space_id| {
                    uuid::Uuid::parse_str(&space_id)
                        .map(|parsed| (space_id, parsed))
                        .map_err(|error| RuntimeApiError::internal(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let core = core.clone();
            let source_observations = tokio::task::spawn_blocking(move || {
                observed_spaces
                    .into_iter()
                    .map(|(space_id, parsed_space_id)| {
                        let source = core
                            .memory_for_space(parsed_space_id)
                            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                            .mo_state_source_fingerprint()
                            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
                        Ok(momo_storage::MoStateSourceObservation {
                            space_id,
                            dmw_fingerprint: source.dmw,
                            nsg_fingerprint: source.nsg,
                            scene_fingerprint: source.scene,
                            scene_json: serde_json::to_string(&source.scene_snapshot)
                                .map_err(|error| RuntimeApiError::internal(error.to_string()))?,
                        })
                    })
                    .collect::<Result<Vec<_>, RuntimeApiError>>()
            })
            .await
            .map_err(|error| {
                RuntimeApiError::internal(format!("memory source observation task failed: {error}"))
            })??;
            Ok(ScopedMemorySnapshot {
                items: combined,
                source_observations,
            })
        })
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
}

fn validate_memory_spaces(sources: &[MemorySpaceSource]) -> Result<(), RuntimeApiError> {
    if sources.is_empty() || sources.len() > MAX_MEMORY_SPACES {
        return Err(RuntimeApiError::invalid(format!(
            "memory retrieval requires between 1 and {MAX_MEMORY_SPACES} Spaces"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    for source in sources {
        let scope_id = uuid::Uuid::parse_str(&source.space_id)
            .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
        if !seen.insert(scope_id) {
            return Err(RuntimeApiError::invalid(
                "memory retrieval Spaces must be unique",
            ));
        }
        if source.label.trim().is_empty() || source.label.chars().count() > 64 {
            return Err(RuntimeApiError::invalid(
                "memory Space labels must contain 1 to 64 characters",
            ));
        }
        if !(1..=100).contains(&source.weight) || (!source.memory && !source.semantic_graph) {
            return Err(RuntimeApiError::invalid(
                "memory Space weights must be between 1 and 100 and enable DMW or NSG",
            ));
        }
    }
    Ok(())
}

fn weighted_space_budgets(sources: &[MemorySpaceSource], max_tokens: usize) -> Vec<usize> {
    if sources.is_empty() {
        return Vec::new();
    }
    let total_weight = sources
        .iter()
        .map(|source| usize::from(source.weight))
        .sum::<usize>();
    let mut budgets = sources
        .iter()
        .map(|source| {
            let weight = usize::from(source.weight);
            // Divide first so even usize::MAX budgets cannot overflow. The
            // remainder product is bounded by the validated Space weights.
            (max_tokens / total_weight) * weight
                + (max_tokens % total_weight) * weight / total_weight
        })
        .collect::<Vec<_>>();
    let assigned = budgets.iter().sum::<usize>();
    let remainder = max_tokens.saturating_sub(assigned);
    // Sum-of-floors rounding leaves fewer tokens than there are Spaces.
    for budget in budgets.iter_mut().take(remainder) {
        *budget += 1;
    }
    budgets
}

fn dmw_retrieval_budget(
    max_tokens: usize,
    include_memory: bool,
    include_semantic_graph: bool,
) -> usize {
    match (include_memory, include_semantic_graph) {
        // DMW and NSG often hold complementary facts. Keep enough room for a
        // complete graph node instead of allowing current DMW documents to
        // consume nearly the whole retrieval budget before NSG runs.
        (true, true) => max_tokens.saturating_mul(HYBRID_DMW_BUDGET_PERCENT) / 100,
        (true, false) => max_tokens,
        (false, _) => 0,
    }
}

fn effective_dmw_retrieval_budget(
    max_tokens: usize,
    include_memory: bool,
    include_semantic_graph: bool,
    nsg_tokens: usize,
) -> usize {
    if include_memory && include_semantic_graph {
        // Reserve NSG room before it runs, then return every unused token to
        // DMW. An empty graph result must not strand forty percent of the
        // context window and evict a directly related memory document.
        max_tokens.saturating_sub(nsg_tokens)
    } else {
        dmw_retrieval_budget(max_tokens, include_memory, include_semantic_graph)
    }
}

pub async fn retrieve_memory_items(
    runtime: &MomoRuntime,
    scope_id: uuid::Uuid,
    query: String,
    max_tokens: usize,
) -> Result<Vec<serde_json::Value>, RuntimeApiError> {
    let guard = runtime
        .lock_space(scope_id)
        .await
        .map_err(RuntimeApiError::recovery)?;
    let core = runtime.core().clone();
    runtime
        .finish_commit(async move {
            let _guard = guard;
            retrieve_memory(
                &core,
                RetrievalPlan {
                    scope_id,
                    query,
                    max_tokens,
                    include_memory: true,
                    include_semantic_graph: true,
                    vector: None,
                },
            )
            .await
        })
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
}

async fn retrieve_memory(
    core: &MomoCore,
    plan: RetrievalPlan,
) -> Result<Vec<serde_json::Value>, RuntimeApiError> {
    let workspace_core = core.clone();
    let scope_id = plan.scope_id;
    let workspace = tokio::task::spawn_blocking(move || workspace_core.memory_for_space(scope_id))
        .await
        .map_err(|error| {
            RuntimeApiError::internal(format!("memory workspace task failed: {error}"))
        })?
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let vector_ranked_ids = if let Some(vector_query) = &plan.vector {
        validate_query_vector(vector_query)?;
        let vector_workspace = Arc::clone(&workspace);
        let hashes = tokio::task::spawn_blocking(move || {
            let nsg = momo_memory::nsg::NsgWorkspace::initialize(vector_workspace.root())?;
            Ok::<_, momo_memory::MemoryError>(
                nsg.embedding_documents()?
                    .into_iter()
                    .map(|document| (document.node_id, document.source_hash))
                    .collect::<std::collections::HashMap<_, _>>(),
            )
        })
        .await
        .map_err(|error| {
            RuntimeApiError::internal(format!("semantic graph indexing task failed: {error}"))
        })?
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
        core.vector_store()
            .rank_nsg_vectors(
                plan.scope_id,
                &vector_query.space_id,
                &vector_query.vector,
                &hashes,
                DEFAULT_NSG_VECTOR_TOP_K,
            )
            .await
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
    } else {
        Vec::new()
    };
    tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, RuntimeApiError> {
        let reserved_memory_budget = dmw_retrieval_budget(
            plan.max_tokens,
            plan.include_memory,
            plan.include_semantic_graph,
        );
        let nsg = if plan.include_semantic_graph {
            momo_memory::nsg::NsgWorkspace::initialize(workspace.root())
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                .retrieve(
                    &plan.query,
                    &vector_ranked_ids,
                    plan.max_tokens.saturating_sub(reserved_memory_budget),
                    &momo_memory::ConservativeTokenCounter,
                )
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        } else {
            Vec::new()
        };
        let nsg_tokens = nsg.iter().map(|item| item.estimated_tokens).sum::<usize>();
        let memory_budget = effective_dmw_retrieval_budget(
            plan.max_tokens,
            plan.include_memory,
            plan.include_semantic_graph,
            nsg_tokens,
        );
        let memories = if plan.include_memory {
            workspace
                .retrieve(
                    &plan.query,
                    memory_budget,
                    &momo_memory::ConservativeTokenCounter,
                )
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        } else {
            Vec::new()
        };
        let mut combined = serde_json::to_value(memories)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .as_array()
            .cloned()
            .unwrap_or_default();
        combined.extend(
            serde_json::to_value(nsg)
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        Ok(combined)
    })
    .await
    .map_err(|error| RuntimeApiError::internal(format!("memory retrieval task failed: {error}")))?
    .map_err(|error| RuntimeApiError::internal(error.to_string()))
}

pub async fn compile_mo_state_with_ddm(
    runtime: &MomoRuntime,
    scope_id: uuid::Uuid,
    retrieved_memory: Vec<momo_memory::RetrievedMemory>,
    retrieved_nsg: Vec<momo_memory::nsg::RetrievedNsg>,
    max_context_tokens: usize,
    ddm_profile: Option<momo_memory::DdmProfile>,
    ddm_runtime: Option<momo_memory::DdmRuntimeInput>,
) -> Result<momo_memory::MoStateContext, RuntimeApiError> {
    let core = runtime.core_handle();
    run_blocking("compile MO state", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .compile_mo_state_with_ddm(
                &retrieved_memory,
                &retrieved_nsg,
                max_context_tokens,
                &momo_memory::ConservativeTokenCounter,
                ddm_profile.as_ref(),
                ddm_runtime.as_ref(),
            )
            .map_err(|error| RuntimeApiError::internal(error.to_string()))
    })
    .await
}

fn validate_query_vector(query: &VectorQuery) -> Result<(), RuntimeApiError> {
    if query.space_id.trim().is_empty()
        || query.vector.is_empty()
        || query.vector.len() > 8192
        || query.vector.iter().any(|value| !value.is_finite())
        || query.vector.iter().all(|value| *value == 0.0)
    {
        return Err(RuntimeApiError::invalid(
            "invalid semantic-graph query vector",
        ));
    }
    Ok(())
}

pub async fn apply_memory_patch(
    runtime: &MomoRuntime,
    scope_id: String,
    patch_yaml: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "apply memory patch", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .apply_patch(&patch_yaml)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn validate_memory_patch(
    runtime: &MomoRuntime,
    scope_id: String,
    patch_yaml: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let _state_guard = runtime
        .lock_space(scope_id)
        .await
        .map_err(RuntimeApiError::recovery)?;
    let core = runtime.core_handle();
    run_blocking("validate memory patch", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .validate_patch(&patch_yaml)
            .map_err(|error| RuntimeApiError::invalid(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn submit_memory_patch_review(
    runtime: &MomoRuntime,
    scope_id: String,
    conversation_id: String,
    patch_yaml: String,
    review_mode: String,
) -> Result<momo_storage::MemoryPatchReview, RuntimeApiError> {
    if !matches!(
        review_mode.as_str(),
        "auto_approve" | "require_confirmation" | "reject"
    ) {
        return Err(RuntimeApiError::invalid(format!(
            "unsupported memory patch review mode: {review_mode}"
        )));
    }
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let patch_to_summarize = patch_yaml.clone();
    let summary = run_space_write(runtime, scope_id, "summarize memory patch", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .summarize_patch(&patch_to_summarize)
            .map_err(|error| RuntimeApiError::invalid(error.to_string()))
    })
    .await?;
    let operation_count = i64::try_from(summary.operation_count)
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    let review = runtime
        .core()
        .store()
        .create_memory_patch_review(
            scope_id,
            &conversation_id,
            &patch_yaml,
            &summary.targets,
            operation_count,
            &review_mode,
        )
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;

    match review_mode.as_str() {
        "auto_approve" => approve_memory_patch_review_inner(runtime, scope_id, review.id).await,
        "reject" => {
            let resolved = runtime
                .core()
                .store()
                .resolve_memory_patch_review(
                    scope_id,
                    review.id,
                    momo_storage::MemoryPatchReviewStatus::Rejected,
                    Some("rejected_by_policy"),
                    None,
                )
                .await
                .map_err(|error| RuntimeApiError::internal(error.to_string()))?
                .ok_or_else(|| {
                    RuntimeApiError::conflict("memory patch review was already resolved")
                })?;
            Ok(resolved)
        }
        _ => Ok(review),
    }
}

pub async fn list_memory_patch_reviews(
    runtime: &MomoRuntime,
    scope_id: String,
    include_resolved: bool,
) -> Result<Vec<momo_storage::MemoryPatchReview>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let reviews = runtime
        .core()
        .store()
        .list_memory_patch_reviews(scope_id, include_resolved)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
    Ok(reviews)
}

pub async fn approve_memory_patch_review(
    runtime: &MomoRuntime,
    scope_id: String,
    review_id: String,
) -> Result<momo_storage::MemoryPatchReview, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let review_id = uuid::Uuid::parse_str(&review_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    approve_memory_patch_review_inner(runtime, scope_id, review_id).await
}

pub async fn reject_memory_patch_review(
    runtime: &MomoRuntime,
    scope_id: String,
    review_id: String,
) -> Result<momo_storage::MemoryPatchReview, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let review_id = uuid::Uuid::parse_str(&review_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let _guard = runtime
        .lock_memory_patch_reviews(&scope_id.to_string())
        .await;
    let existing = runtime
        .core()
        .store()
        .memory_patch_review(scope_id, review_id)
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .ok_or_else(|| RuntimeApiError::not_found("memory patch review was not found"))?;
    if existing.status != momo_storage::MemoryPatchReviewStatus::Pending {
        return Ok(existing);
    }
    let resolved = runtime
        .core()
        .store()
        .resolve_memory_patch_review(
            scope_id,
            review_id,
            momo_storage::MemoryPatchReviewStatus::Rejected,
            Some("rejected_by_user"),
            None,
        )
        .await
        .map_err(|error| RuntimeApiError::internal(error.to_string()))?
        .ok_or_else(|| RuntimeApiError::conflict("memory patch review was already resolved"))?;
    Ok(resolved)
}

pub async fn list_memory_documents(
    runtime: &MomoRuntime,
    scope_id: String,
) -> Result<Vec<momo_memory::DocumentSummary>, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let documents = run_space_write(runtime, scope_id, "list memory documents", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .list_documents()
            .map_err(|error| RuntimeApiError::internal(error.to_string()))
    })
    .await?;
    Ok(documents)
}

pub async fn read_memory_document(
    runtime: &MomoRuntime,
    scope_id: String,
    document_id: String,
) -> Result<serde_json::Value, RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    let document = run_space_write(runtime, scope_id, "read memory document", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .read_document_by_id(&document_id)
            .map_err(|error| RuntimeApiError::not_found(error.to_string()))
    })
    .await?;
    Ok(json!({
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
}

pub async fn update_memory_document(
    runtime: &MomoRuntime,
    scope_id: String,
    document_id: String,
    markdown: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "update memory document", move || {
        let workspace = core
            .memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
        workspace
            .read_document_by_id(&document_id)
            .map_err(|error| RuntimeApiError::not_found(error.to_string()))?;
        workspace
            .replace_document_body(&document_id, &markdown)
            .map_err(|error| RuntimeApiError::invalid(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn archive_memory_document(
    runtime: &MomoRuntime,
    scope_id: String,
    document_id: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "archive memory document", move || {
        let workspace = core
            .memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
        workspace
            .read_document_by_id(&document_id)
            .map_err(|error| RuntimeApiError::not_found(error.to_string()))?;
        let index_path = workspace.root().join("indexes/memory_index.yaml");
        let index_text = std::fs::read_to_string(&index_path)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
        let entry_path = find_entry_path(&index_text, &document_id)?;
        let yaml_patch = format!(
            "patches:\n  - target_file: \"{entry_path}\"\n    operations:\n      - type: update_frontmatter\n        fields:\n          status: archived\n          weight: 0.1\n",
        );
        workspace
            .apply_patch(&yaml_patch)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?;
        workspace
            .run_maintenance()
            .map_err(|error| RuntimeApiError::internal(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn restore_memory_document(
    runtime: &MomoRuntime,
    scope_id: String,
    document_id: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "restore memory document", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .restore_archived_authorized(&document_id)
            .map_err(|error| RuntimeApiError::not_found(error.to_string()))
    })
    .await?;
    Ok(())
}

pub async fn delete_memory_document(
    runtime: &MomoRuntime,
    scope_id: String,
    document_id: String,
) -> Result<(), RuntimeApiError> {
    let scope_id = uuid::Uuid::parse_str(&scope_id)
        .map_err(|error| RuntimeApiError::invalid(error.to_string()))?;
    let core = runtime.core_handle();
    run_space_write(runtime, scope_id, "delete memory document", move || {
        core.memory_for_space(scope_id)
            .map_err(|error| RuntimeApiError::internal(error.to_string()))?
            .delete_document_authorized(&document_id)
            .map_err(|error| RuntimeApiError::not_found(error.to_string()))
    })
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/api_simple_memory.rs"]
mod space_tests;
