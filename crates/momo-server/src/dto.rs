use super::*;

#[derive(Serialize)]
pub(crate) struct HealthResponse {
    pub(crate) ok: bool,
    pub(crate) service: &'static str,
    pub(crate) core_version: String,
    pub(crate) data_dir: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateConversationRequest {
    pub(crate) space_id: String,
    pub(crate) title: String,
    pub(crate) character_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateMessageRequest {
    pub(crate) space_id: String,
    pub(crate) conversation_id: String,
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCharacterRequest {
    pub(crate) owner_space_id: String,
    pub(crate) name: String,
    pub(crate) author_name: String,
    pub(crate) character_markdown: String,
    #[serde(default)]
    pub(crate) user_markdown: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DdmProfileRequest {
    pub(crate) owner_space_id: String,
    pub(crate) profile_yaml: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportExternalCharacterRequest {
    pub(crate) owner_space_id: String,
    pub(crate) input_path: String,
    pub(crate) format: momo_core::ExternalCharacterImportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportExternalCharacterRequest {
    pub(crate) owner_space_id: String,
    pub(crate) output_path: String,
    pub(crate) format: momo_core::ExternalCharacterExportFormat,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportPreservedCharacterSourceRequest {
    pub(crate) owner_space_id: String,
    pub(crate) output_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveCapabilityRequest {
    pub(crate) provider_id: String,
    pub(crate) model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveScopedMemoryRequest {
    pub(crate) spaces: Vec<runtime_api::MemorySpaceSource>,
    pub(crate) query: String,
    #[serde(default = "default_memory_tokens")]
    pub(crate) max_tokens: usize,
    pub(crate) vector_space_id: Option<String>,
    pub(crate) query_vector: Option<Vec<f64>>,
    #[serde(default)]
    pub(crate) embedding: Option<runtime_api::EmbeddingRequestConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompileMoStateRequest {
    pub(crate) space_id: String,
    #[serde(default)]
    pub(crate) retrieved_memory: Vec<momo_core::momo_memory::RetrievedMemory>,
    #[serde(default)]
    pub(crate) retrieved_nsg: Vec<momo_core::momo_memory::nsg::RetrievedNsg>,
    #[serde(default = "default_context_window")]
    pub(crate) max_context_tokens: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrepareContextRequest {
    #[serde(default)]
    pub(crate) runtime_instructions: String,
    #[serde(default)]
    pub(crate) character_markdown: String,
    #[serde(default)]
    pub(crate) user_markdown: String,
    #[serde(default)]
    pub(crate) memory_markdown: String,
    #[serde(default)]
    pub(crate) state_context: String,
    #[serde(default)]
    pub(crate) nsg_markdown: String,
    #[serde(default)]
    pub(crate) messages: Vec<momo_core::ChatInput>,
    #[serde(default = "default_context_window")]
    pub(crate) context_window: usize,
    #[serde(default = "default_reserve_output_tokens")]
    pub(crate) reserve_output_tokens: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateMemoryDocumentRequest {
    pub(crate) space_id: String,
    pub(crate) markdown: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyMemoryPatchRequest {
    pub(crate) space_id: String,
    pub(crate) patch_yaml: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmitMemoryPatchReviewRequest {
    pub(crate) space_id: String,
    pub(crate) conversation_id: String,
    pub(crate) patch_yaml: String,
    pub(crate) review_mode: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IncludeResolvedQuery {
    pub(crate) space_id: String,
    pub(crate) include_resolved: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteNsgNodeRequest {
    pub(crate) space_id: String,
    pub(crate) target_file: String,
    pub(crate) node: momo_core::momo_memory::nsg::NsgNode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyNsgPatchRequest {
    pub(crate) space_id: String,
    pub(crate) patch_yaml: String,
    #[serde(default)]
    pub(crate) manual_authority: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpaceRequest {
    pub(crate) space_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordMaintenanceTurnRequest {
    pub(crate) request_id: String,
    pub(crate) space_id: String,
    pub(crate) user_content: String,
    pub(crate) assistant_content: String,
    pub(crate) memory_enabled: bool,
    pub(crate) nsg_enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NsgTargetRequest {
    pub(crate) space_id: String,
    pub(crate) target_file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpaceNsgTargetRequest {
    pub(crate) space_id: String,
    pub(crate) target_file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NsgVectorStatusQuery {
    pub(crate) space_id: String,
    pub(crate) vector_space_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MocExportRequest {
    pub(crate) output_path: String,
    pub(crate) plan: momo_core::MocExportPlan,
    #[serde(default)]
    pub(crate) protection: momo_core::MocProtection,
    #[serde(default)]
    pub(crate) host_modules: Vec<momo_core::HostMocModule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MocImportRequest {
    pub(crate) input_path: String,
    pub(crate) plan: momo_core::MocImportPlan,
    #[serde(default)]
    pub(crate) protection: momo_core::MocProtection,
    pub(crate) claim_unknown_to: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MocEncryptedQuery {
    pub(crate) input_path: String,
}

#[derive(Serialize)]
pub(crate) struct OkResponse {
    pub(crate) ok: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplacePromptSpaceRequest {
    pub(crate) content: String,
}
