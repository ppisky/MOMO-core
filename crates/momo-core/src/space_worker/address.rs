//! Typed request/reply facade; inputs are owned before entering the mailbox.
use super::SpaceHandle;
use momo_domain::provenance::*;
use momo_memory::*;
use momo_memory::{lifecycle::*, nsg::*, provenance::*};
use std::path::PathBuf;
impl SpaceHandle {
    #[cfg(test)]
    pub(crate) fn read(&self, path: &str) -> Result<MemoryDocument, MemoryError> {
        let path = path.to_owned();
        self.call(move |workspace| workspace.read(path))
    }
    #[cfg(test)]
    pub(crate) fn record_revision_matches(
        &self,
        graph: bool,
        id: &str,
        path: &str,
    ) -> Result<bool, MemoryError> {
        let id = id.to_owned();
        let path = path.to_owned();
        self.call(move |workspace| workspace.record_revision_matches(graph, &id, &path))
    }

    pub(crate) fn export_snapshot(&self) -> Result<MemorySnapshot, MemoryError> {
        self.call(move |w| w.export_snapshot())
    }
    pub(crate) fn retrieve_in_scope(
        &self,
        query: &str,
        max_tokens: usize,
        counter: &(impl TokenCounter + Clone + Send + 'static),
        scope: Option<&MemoryRetrievalScope>,
    ) -> Result<Vec<RetrievedMemory>, MemoryError> {
        let query = query.to_owned();
        let counter = counter.clone();
        let scope = scope.cloned();
        self.call(move |w| w.retrieve_in_scope(&query, max_tokens, &counter, scope.as_ref()))
    }
    pub(crate) fn apply_patch(&self, yaml: &str) -> Result<(), MemoryError> {
        let yaml = yaml.to_owned();
        self.call(move |w| w.apply_patch(&yaml))
    }
    pub(crate) fn validate_patch(&self, yaml: &str) -> Result<(), MemoryError> {
        let yaml = yaml.to_owned();
        self.call(move |w| w.validate_patch(&yaml))
    }
    pub(crate) fn summarize_patch(&self, yaml: &str) -> Result<MemoryPatchSummary, MemoryError> {
        let yaml = yaml.to_owned();
        self.call(move |w| w.summarize_patch(&yaml))
    }
    pub(crate) fn restore_archived_authorized(&self, id: &str) -> Result<PathBuf, MemoryError> {
        let id = id.to_owned();
        self.call(move |w| w.restore_archived_authorized(&id))
    }
    pub(crate) fn delete_document_authorized(&self, id: &str) -> Result<(), MemoryError> {
        let id = id.to_owned();
        self.call(move |w| w.delete_document_authorized(&id))
    }
    pub(crate) fn list_documents(&self) -> Result<Vec<DocumentSummary>, MemoryError> {
        self.call(move |w| w.list_documents())
    }
    pub(crate) fn read_document_by_id(&self, id: &str) -> Result<MemoryDocument, MemoryError> {
        let id = id.to_owned();
        self.call(move |w| w.read_document_by_id(&id))
    }
    pub(crate) fn replace_document_body(&self, id: &str, body: &str) -> Result<(), MemoryError> {
        let id = id.to_owned();
        let body = body.to_owned();
        self.call(move |w| w.replace_document_body(&id, &body))
    }
    pub(crate) fn prepare_identity_patch_commit(
        &self,
        yaml: &str,
        identity: Option<&momo_domain::provenance::MemoryIdentity>,
    ) -> Result<PreparedMemoryCommit, MemoryError> {
        let yaml = yaml.to_owned();
        let identity = identity.cloned();
        self.call(move |w| w.prepare_identity_patch_commit(&yaml, identity.as_ref()))
    }
    pub(crate) fn prepare_patch_commit(
        &self,
        yaml: &str,
    ) -> Result<PreparedMemoryCommit, MemoryError> {
        let yaml = yaml.to_owned();
        self.call(move |w| w.prepare_patch_commit(&yaml))
    }
    pub(crate) fn apply_prepared_commit(
        &self,
        plan: &PreparedMemoryCommit,
    ) -> Result<(), MemoryError> {
        let plan = plan.clone();
        self.call(move |w| w.apply_prepared_commit(&plan))
    }
    pub(crate) fn mo_state_source_fingerprint(
        &self,
    ) -> Result<MoStateSourceFingerprint, MemoryError> {
        self.call(move |w| w.mo_state_source_fingerprint())
    }
    pub(crate) fn compile_mo_state_with_ddm(
        &self,
        retrieved_memory: &[RetrievedMemory],
        retrieved_nsg: &[RetrievedNsg],
        max_context_tokens: usize,
        counter: &(impl TokenCounter + Clone + Send + 'static),
        ddm_profile: Option<&DdmProfile>,
        ddm_runtime: Option<&DdmRuntimeInput>,
    ) -> Result<MoStateContext, MemoryError> {
        let retrieved_memory = retrieved_memory.to_vec();
        let retrieved_nsg = retrieved_nsg.to_vec();
        let counter = counter.clone();
        let ddm_profile = ddm_profile.cloned();
        let ddm_runtime = ddm_runtime.cloned();
        self.call(move |w| {
            w.compile_mo_state_with_ddm(
                &retrieved_memory,
                &retrieved_nsg,
                max_context_tokens,
                &counter,
                ddm_profile.as_ref(),
                ddm_runtime.as_ref(),
            )
        })
    }
    pub(crate) fn record_revision_matches_in_ledger(
        &self,
        ledger: &ProvenanceLedger,
        id: &str,
        path: &str,
    ) -> Result<bool, MemoryError> {
        let ledger = ledger.clone();
        let id = id.to_owned();
        let path = path.to_owned();
        self.call(move |w| w.record_revision_matches_in_ledger(&ledger, &id, &path))
    }
    pub(crate) fn provenance(&self, semantic_graph: bool) -> Result<ProvenanceLedger, MemoryError> {
        self.call(move |w| w.provenance(semantic_graph))
    }
    pub(crate) fn trace_commit(
        &self,
        plan: PreparedMemoryCommit,
        space: Option<uuid::Uuid>,
        context: &RevisionContext,
    ) -> Result<PreparedMemoryCommit, MemoryError> {
        let context = context.clone();
        self.call(move |w| w.trace_commit(plan, space, &context))
    }
    pub(crate) fn set_record_policy(
        &self,
        graph: bool,
        id: &str,
        policy: RecordPolicy,
    ) -> Result<(), MemoryError> {
        let id = id.to_owned();
        self.call(move |w| w.set_record_policy(graph, &id, policy))
    }
    pub(crate) fn scoped_current(
        &self,
        identity: &MemoryIdentity,
    ) -> Result<Vec<RetrievedMemory>, MemoryError> {
        let identity = identity.clone();
        self.call(move |w| w.scoped_current(&identity))
    }
    pub(crate) fn prepare_lifecycle_activity(
        &self,
        activity: &LifecycleActivity,
        now: i64,
    ) -> Result<(PreparedMemoryCommit, MaintenanceReport), MemoryError> {
        let activity = activity.clone();
        self.call(move |w| w.prepare_lifecycle_activity(&activity, now))
    }
    pub(crate) fn nsg(self: &std::sync::Arc<Self>) -> Result<NsgHandle, MemoryError> {
        Ok(NsgHandle(std::sync::Arc::clone(self)))
    }
}
pub(crate) struct NsgHandle(std::sync::Arc<SpaceHandle>);
impl NsgHandle {
    pub(crate) fn retrieve(
        &self,
        query: &str,
        vector_ranked_ids: &[String],
        max_tokens: usize,
        counter: &(impl TokenCounter + Clone + Send + 'static),
    ) -> Result<Vec<RetrievedNsg>, MemoryError> {
        let query = query.to_owned();
        let vector_ranked_ids = vector_ranked_ids.to_vec();
        let counter = counter.clone();
        self.0.call(move |w| {
            NsgWorkspace::initialize(w.root())?.retrieve(
                &query,
                &vector_ranked_ids,
                max_tokens,
                &counter,
            )
        })
    }

    pub(crate) fn apply_patch(&self, yaml: &str) -> Result<(), MemoryError> {
        let yaml = yaml.to_owned();
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.apply_patch(&yaml))
    }
    pub(crate) fn prepare_patch_commit(
        &self,
        yaml: &str,
    ) -> Result<PreparedMemoryCommit, MemoryError> {
        let yaml = yaml.to_owned();
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.prepare_patch_commit(&yaml))
    }
    pub(crate) fn apply_patch_authorized(&self, yaml: &str) -> Result<(), MemoryError> {
        let yaml = yaml.to_owned();
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.apply_patch_authorized(&yaml))
    }
    pub(crate) fn validate_patch(&self, yaml: &str) -> Result<(), MemoryError> {
        let yaml = yaml.to_owned();
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.validate_patch(&yaml))
    }
    pub(crate) fn list_nodes(
        &self,
        include_archived: bool,
    ) -> Result<Vec<ManagedNsgNode>, MemoryError> {
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.list_nodes(include_archived))
    }
    pub(crate) fn embedding_documents(&self) -> Result<Vec<NsgEmbeddingDocument>, MemoryError> {
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.embedding_documents())
    }
    pub(crate) fn list_pending_candidates(&self) -> Result<Vec<NsgPendingCandidate>, MemoryError> {
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.list_pending_candidates())
    }
    pub(crate) fn approve_pending_candidate(&self, pending_path: &str) -> Result<(), MemoryError> {
        let pending_path = pending_path.to_owned();
        self.0.call(move |w| {
            NsgWorkspace::initialize(w.root())?.approve_pending_candidate(&pending_path)
        })
    }
    pub(crate) fn reject_pending_candidate(&self, pending_path: &str) -> Result<(), MemoryError> {
        let pending_path = pending_path.to_owned();
        self.0.call(move |w| {
            NsgWorkspace::initialize(w.root())?.reject_pending_candidate(&pending_path)
        })
    }
    pub(crate) fn write_node(&self, target_file: &str, node: NsgNode) -> Result<(), MemoryError> {
        let target_file = target_file.to_owned();
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.write_node(&target_file, node))
    }
    pub(crate) fn archive_node(&self, target_file: &str) -> Result<(), MemoryError> {
        let target_file = target_file.to_owned();
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.archive_node(&target_file))
    }
    pub(crate) fn delete_node(&self, target_file: &str) -> Result<(), MemoryError> {
        let target_file = target_file.to_owned();
        self.0
            .call(move |w| NsgWorkspace::initialize(w.root())?.delete_node(&target_file))
    }
}
