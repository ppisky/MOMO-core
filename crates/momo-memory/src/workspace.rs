use super::*;

impl MemoryWorkspace {
    pub fn initialize(root: impl AsRef<Path>) -> Result<Self, MemoryError> {
        fs::create_dir_all(root.as_ref())?;
        let root = fs::canonicalize(root.as_ref())?;
        for directory in MEMORY_DIRECTORIES {
            create_managed_directory(&root, Path::new(directory))?;
        }
        write_if_missing(
            &root.join("config/access.yaml"),
            "version: 1\nread: [current, character, relationship, event, world]\nwrite: [current, character, relationship, event, world]\nallow_archive_restore: false\n",
        )?;
        write_if_missing(
            &root.join("indexes/memory_index.yaml"),
            "version: 1\nentries: {}\n",
        )?;
        write_if_missing(
            &root.join("indexes/memory_activity.yaml"),
            "version: 1\nentries: {}\n",
        )?;
        write_if_missing(&root.join("tombstones/forgotten.yaml"), "{}\n")?;
        write_if_missing(&root.join("audit/memory.log"), "")?;
        let now = Utc::now().timestamp();
        for (name, id, title) in [
            ("scene.md", "current_scene", "当前场景"),
            ("active_threads.md", "current_active_threads", "活跃剧情线"),
        ] {
            let path = root.join("current").join(name);
            if regular_file_exists(&path)? {
                continue;
            }
            let document = MemoryDocument {
                metadata: Metadata {
                    id: id.to_owned(),
                    kind: "current".to_owned(),
                    importance: None,
                    weight: None,
                    touch_at: now,
                    decay_at: None,
                    archived_at: None,
                    relations: BTreeMap::new(),
                    injection_scope: None,
                    injection_conversation_id: None,
                    injection_character_id: None,
                    tags: Vec::new(),
                    aliases: Vec::new(),
                    status: "active".to_owned(),
                },
                body: format!("# {title}\n\n"),
            };
            atomic_write(&path, &document.encode()?)?;
        }
        Ok(Self { root })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn export_snapshot(&self) -> Result<MemorySnapshot, MemoryError> {
        let mut files = BTreeMap::new();
        collect_snapshot_files(&self.root, &self.root, &mut files)?;
        Ok(MemorySnapshot { version: 1, files })
    }

    /// DMW and NSG share a local workspace for filesystem convenience, but
    /// they are independent MOC modules. These snapshots preserve that module
    /// boundary without changing the portable full-workspace format.
    pub fn export_memory_partition_snapshot(&self) -> Result<MemorySnapshot, MemoryError> {
        let mut snapshot = self.export_snapshot()?;
        snapshot
            .files
            .retain(|path, _| !is_semantic_graph_snapshot_path(path));
        Ok(snapshot)
    }

    pub fn export_semantic_graph_partition_snapshot(&self) -> Result<MemorySnapshot, MemoryError> {
        let mut snapshot = self.export_snapshot()?;
        snapshot
            .files
            .retain(|path, _| is_semantic_graph_snapshot_path(path));
        Ok(snapshot)
    }

    pub fn import_snapshot(&self, snapshot: &MemorySnapshot) -> Result<(), MemoryError> {
        if snapshot.version != 1 {
            return Err(MemoryError::InvalidPatch(format!(
                "unsupported memory snapshot version: {}",
                snapshot.version
            )));
        }
        for path in snapshot.files.keys() {
            validate_relative(Path::new(path))?;
        }
        remove_workspace_files(&self.root)?;
        for directory in MEMORY_DIRECTORIES {
            create_managed_directory(&self.root, Path::new(directory))?;
        }
        for (path, content) in &snapshot.files {
            let relative = Path::new(path);
            validate_relative(relative)?;
            let target = self.root.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&target, content)?;
        }
        Ok(())
    }

    pub fn import_memory_partition_snapshot(
        &self,
        snapshot: &MemorySnapshot,
    ) -> Result<(), MemoryError> {
        let memory_snapshot =
            snapshot_partition(snapshot, |path| !is_semantic_graph_snapshot_path(path));
        self.replace_snapshot_partition(&memory_snapshot, |path| {
            !is_semantic_graph_snapshot_path(path)
        })
    }

    pub fn import_semantic_graph_partition_snapshot(
        &self,
        snapshot: &MemorySnapshot,
    ) -> Result<(), MemoryError> {
        self.replace_snapshot_partition(snapshot, is_semantic_graph_snapshot_path)
    }

    fn replace_snapshot_partition(
        &self,
        snapshot: &MemorySnapshot,
        includes: fn(&str) -> bool,
    ) -> Result<(), MemoryError> {
        if snapshot.version != 1 {
            return Err(MemoryError::InvalidPatch(format!(
                "unsupported memory snapshot version: {}",
                snapshot.version
            )));
        }
        for path in snapshot.files.keys() {
            validate_relative(Path::new(path))?;
            if !includes(path) {
                return Err(MemoryError::InvalidPatch(
                    "sync snapshot contains files outside its category".to_owned(),
                ));
            }
        }
        // Export first so a missing incoming file means deletion only inside
        // this category. The other category remains untouched.
        let current = self.export_snapshot()?;
        for path in current.files.keys().filter(|path| includes(path)) {
            let target = self.root.join(path);
            if target.exists() {
                fs::remove_file(target)?;
            }
        }
        for (path, content) in &snapshot.files {
            let target = self.root.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&target, content)?;
        }
        Ok(())
    }

    pub fn read(&self, relative: impl AsRef<Path>) -> Result<MemoryDocument, MemoryError> {
        let relative = relative.as_ref();
        let kind = expected_kind_for_path(relative)
            .ok_or_else(|| MemoryError::UnsafePath(relative.to_path_buf()))?;
        self.load_access()?.require_read(kind)?;
        self.read_unchecked(relative)
    }

    pub fn retrieve(
        &self,
        query: &str,
        max_tokens: usize,
        counter: &impl TokenCounter,
    ) -> Result<Vec<RetrievedMemory>, MemoryError> {
        let access = self.load_access()?;
        access.require_read("current")?;
        let index = self.load_index()?;
        let now = Utc::now().timestamp();
        let mut used = 0_usize;
        let mut result = Vec::new();
        let mut mutations = Vec::new();
        let mut loaded_ids = Vec::new();
        let mut touch_ids = HashSet::new();

        let mut hot_documents = Vec::new();
        for relative in ["current/scene.md", "current/active_threads.md"] {
            let document = self.read_unchecked(Path::new(relative))?;
            hot_documents.push((relative.to_owned(), document));
        }
        let hot_text = hot_documents
            .iter()
            .map(|(_, document)| document.body.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let hot_reference_ids = explicit_memory_references(&hot_text);
        let hot_tokens = hot_documents
            .iter()
            .map(|(_, document)| counter.count(&document.body))
            .fold(0_usize, usize::saturating_add);
        let hot_over_budget = hot_tokens > max_tokens;
        for (path, document) in hot_documents {
            let remaining = max_tokens.saturating_sub(used);
            let body = if hot_over_budget {
                markdown_prefix_within_budget(&document.body, remaining, counter)
            } else {
                document.body.clone()
            };
            if body.is_empty() {
                continue;
            }
            let tokens = counter.count(&body);
            if tokens > remaining {
                continue;
            }
            used += tokens;
            let id = document.metadata.id.clone();
            let source_character_ids = document
                .metadata
                .relations
                .get("characters")
                .cloned()
                .unwrap_or_default();
            loaded_ids.push(id.clone());
            result.push(RetrievedMemory {
                id,
                path: PathBuf::from(path),
                body,
                estimated_tokens: tokens,
                source_character_ids,
                injection_scope: document.metadata.injection_scope.clone(),
                injection_conversation_id: document.metadata.injection_conversation_id.clone(),
                injection_character_id: document.metadata.injection_character_id.clone(),
            });
        }
        if hot_over_budget {
            self.record_memory_activity(now, &loaded_ids, &mut mutations)?;
            if !mutations.is_empty() {
                commit_mutations(&mutations)?;
            }
            return Ok(result);
        }

        let normalized_query = normalize(query);
        let mut direct_pool = Vec::new();
        let mut by_id = HashMap::new();

        for (id, entry) in &index.entries {
            by_id.insert(id.clone(), entry);
            if !access.can_read(&entry.kind) {
                continue;
            }
            let query_hit = query_hit(entry, id, &normalized_query);
            let hot_reference_hit = hot_reference_ids.contains(id);
            if query_hit.candidate || hot_reference_hit {
                let document = self.read_unchecked(Path::new(&entry.path))?;
                if document.metadata.id != *id {
                    return Err(MemoryError::InvalidIndex(format!(
                        "entry {id} points to document {}",
                        document.metadata.id
                    )));
                }
                if document.metadata.status == "active" {
                    if query_hit.substantive || hot_reference_hit {
                        touch_ids.insert(id.clone());
                    }
                    direct_pool.push((entry.path.clone(), document));
                }
            }
        }
        direct_pool.sort_by(compare_documents);
        let related_ids = direct_pool
            .iter()
            .take(EXPANSION_SOURCE_LIMIT)
            .flat_map(|(_, document)| {
                let is_hub = relation_degree(document, &by_id, &access) > HUB_THRESHOLD;
                let limit = if is_hub {
                    HUB_EXPANSION_PER_SOURCE
                } else {
                    NORMAL_EXPANSION_PER_SOURCE
                };
                ranked_relation_ids(
                    document,
                    &by_id,
                    &access,
                    &normalized_query,
                    &hot_reference_ids,
                )
                .into_iter()
                .take(limit)
            })
            .take(MAX_EXPANSION_TOTAL)
            .collect::<Vec<_>>();
        let mut candidate_ids = direct_pool
            .iter()
            .map(|(_, document)| document.metadata.id.clone())
            .collect::<HashSet<_>>();
        let mut expansion_pool = Vec::new();
        for related_id in related_ids {
            if candidate_ids.insert(related_id.clone())
                && let Some(entry) = by_id.get(&related_id)
                && access.can_read(&entry.kind)
            {
                let document = self.read_unchecked(Path::new(&entry.path))?;
                if document.metadata.id != related_id {
                    return Err(MemoryError::InvalidIndex(format!(
                        "entry {related_id} points to document {}",
                        document.metadata.id
                    )));
                }
                if document.metadata.status == "active" {
                    expansion_pool.push((entry.path.clone(), document));
                }
            }
        }
        direct_pool.sort_by(compare_documents);
        expansion_pool.sort_by(compare_documents);

        let remaining_memory_budget = max_tokens.saturating_sub(used);
        let direct_budget =
            remaining_memory_budget.saturating_mul(DIRECT_RESERVE_RATIO_NUMERATOR) / 100;
        let expansion_budget =
            remaining_memory_budget.saturating_mul(EXPANSION_MAX_RATIO_NUMERATOR) / 100;
        let mut direct_used = 0_usize;
        let mut expansion_used = 0_usize;

        for (path, document) in &direct_pool {
            let tokens = counter.count(&document.body);
            if used.saturating_add(tokens) > max_tokens
                || (!expansion_pool.is_empty()
                    && direct_used.saturating_add(tokens) > direct_budget
                    && direct_used > 0)
            {
                continue;
            }
            used += tokens;
            direct_used += tokens;
            let id = document.metadata.id.clone();
            let body = document.body.clone();
            let source_character_ids = document
                .metadata
                .relations
                .get("characters")
                .cloned()
                .unwrap_or_default();
            loaded_ids.push(id.clone());
            result.push(RetrievedMemory {
                id,
                path: PathBuf::from(path.clone()),
                body,
                estimated_tokens: tokens,
                source_character_ids,
                injection_scope: document.metadata.injection_scope.clone(),
                injection_conversation_id: document.metadata.injection_conversation_id.clone(),
                injection_character_id: document.metadata.injection_character_id.clone(),
            });
        }
        for (path, document) in &expansion_pool {
            let tokens = counter.count(&document.body);
            if used.saturating_add(tokens) > max_tokens
                || expansion_used.saturating_add(tokens) > expansion_budget
            {
                continue;
            }
            used += tokens;
            expansion_used += tokens;
            let id = document.metadata.id.clone();
            loaded_ids.push(id.clone());
            result.push(RetrievedMemory {
                id,
                path: PathBuf::from(path.clone()),
                body: document.body.clone(),
                estimated_tokens: tokens,
                source_character_ids: document
                    .metadata
                    .relations
                    .get("characters")
                    .cloned()
                    .unwrap_or_default(),
                injection_scope: document.metadata.injection_scope.clone(),
                injection_conversation_id: document.metadata.injection_conversation_id.clone(),
                injection_character_id: document.metadata.injection_character_id.clone(),
            });
        }
        let loaded_long_term = result
            .iter()
            .map(|memory| memory.id.as_str())
            .collect::<HashSet<_>>();
        let mut queued_touch_ids = HashSet::new();
        for (_, document) in direct_pool.iter().take(HIT_REFRESH_LIMIT) {
            if touch_ids.contains(&document.metadata.id) {
                queued_touch_ids.insert(document.metadata.id.clone());
                self.queue_touch(now, document, &mut mutations)?;
            }
        }
        for (_, document) in &direct_pool {
            if loaded_long_term.contains(document.metadata.id.as_str())
                && touch_ids.contains(&document.metadata.id)
                && queued_touch_ids.insert(document.metadata.id.clone())
            {
                self.queue_touch(now, document, &mut mutations)?;
            }
        }
        self.record_memory_activity(now, &loaded_ids, &mut mutations)?;
        if !mutations.is_empty() {
            commit_mutations(&mutations)?;
        }
        Ok(result)
    }

    pub fn apply_patch(&self, yaml: &str) -> Result<(), MemoryError> {
        let mut writer = atomic_write_bytes;
        self.apply_patch_with_writer(yaml, &mut writer)
    }

    /// Performs the same complete parse, path, access and operation validation
    /// as [`Self::apply_patch`] without changing a memory file or its index.
    pub fn validate_patch(&self, yaml: &str) -> Result<(), MemoryError> {
        self.prepare_patch(yaml).map(|_| ())
    }

    pub fn summarize_patch(&self, yaml: &str) -> Result<MemoryPatchSummary, MemoryError> {
        self.validate_patch(yaml)?;
        let patch = parse_patch_document(yaml)?;
        Ok(MemoryPatchSummary {
            operation_count: patch.patches.iter().map(|item| item.operations.len()).sum(),
            targets: patch
                .patches
                .into_iter()
                .map(|item| item.target_file)
                .collect(),
        })
    }

    pub fn rebuild_index(&self) -> Result<usize, MemoryError> {
        let previous = self.load_index_for_rebuild()?;
        let index = self.build_index_data(Some(&previous))?;
        let count = index.entries.len();
        commit_mutations(&[FileMutation::Write {
            path: self.checked_index_path()?,
            content: encode_index(&index)?.into_bytes(),
        }])?;
        Ok(count)
    }

    pub fn run_maintenance(&self) -> Result<MaintenanceReport, MemoryError> {
        self.run_maintenance_at(Utc::now().timestamp())
    }

    pub fn run_maintenance_at(&self, now: i64) -> Result<MaintenanceReport, MemoryError> {
        let access = self.load_access()?;
        let existing_index = self.load_index_for_rebuild()?;
        let mut index = self.build_index_data(Some(&existing_index))?;
        let all_paths = self.all_long_term_memory_paths()?;
        let mut referenced_ids = HashSet::new();
        for relative in &all_paths {
            let document = self.read_unchecked(relative)?;
            referenced_ids.extend(document.metadata.relations.values().flatten().cloned());
        }
        let hot_text = ["current/scene.md", "current/active_threads.md"]
            .into_iter()
            .map(|path| {
                self.read_unchecked(Path::new(path))
                    .map(|document| document.body)
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        let mut paths = self.active_memory_paths()?;
        paths.sort();

        let mut report = MaintenanceReport::default();
        let mut mutations = Vec::new();
        let mut audit_events = Vec::new();
        for relative in paths {
            let kind = expected_kind_for_path(&relative)
                .ok_or_else(|| MemoryError::UnsafePath(relative.clone()))?;
            if !access.can_read(kind) || !access.can_write(kind) {
                continue;
            }
            let source = self.resolve(&relative)?;
            let mut document = MemoryDocument::parse(&fs::read_to_string(&source)?)?;
            validate_document_location(&relative, &document.metadata)?;

            let mut changed = false;
            let clock_skew_guarded = clock_skew_exceeds_tolerance(&document.metadata, now);
            if clock_skew_guarded {
                report
                    .clock_skew_guarded_ids
                    .push(document.metadata.id.clone());
                audit_events.push(format!(
                    "{now}\tclock_skew_guard\t{}\t{}",
                    document.metadata.id,
                    relative.display()
                ));
            }
            if !clock_skew_guarded
                && document.metadata.status == "active"
                && now.saturating_sub(document.metadata.touch_at) > DECAY_INTERVAL_SECONDS
                && now.saturating_sub(document.metadata.decay_at.unwrap_or_default())
                    >= DECAY_INTERVAL_SECONDS
            {
                let weight = document
                    .metadata
                    .weight
                    .ok_or(MemoryError::InvalidFrontmatter)?;
                document.metadata.weight = Some((weight * 0.9).clamp(0.0, 1.0));
                document.metadata.decay_at = Some(now);
                report.decayed_ids.push(document.metadata.id.clone());
                changed = true;
            }

            let should_archive = document.metadata.status == "archived"
                || (document.metadata.status == "active"
                    && document.metadata.weight.unwrap_or_default() < 0.2
                    && document.metadata.importance.unwrap_or_default() < 0.8);
            if should_archive {
                document.metadata.status = "archived".to_owned();
                document.metadata.archived_at.get_or_insert(now);
                let destination_relative = archive_path_for(&relative, &document.metadata.kind)?;
                let destination = self.resolve(&destination_relative)?;
                if regular_file_exists(&destination)? {
                    return Err(MemoryError::InvalidPatch(format!(
                        "archive destination exists: {}",
                        destination_relative.display()
                    )));
                }
                mutations.push(FileMutation::Write {
                    path: destination,
                    content: document.encode()?.into_bytes(),
                });
                mutations.push(FileMutation::Delete {
                    path: source.clone(),
                });
                update_index_entry(&mut index, &destination_relative, &document)?;
                report.archived_ids.push(document.metadata.id.clone());
                audit_events.push(format!(
                    "{now}\tarchive\t{}\t{}",
                    document.metadata.id,
                    destination_relative.display()
                ));
            } else if changed {
                mutations.push(FileMutation::Write {
                    path: source,
                    content: document.encode()?.into_bytes(),
                });
                update_index_entry(&mut index, &relative, &document)?;
            }
        }

        let mut tombstones = self.load_tombstones()?;
        let mut tombstones_changed = false;
        let mut archived_paths = all_paths
            .into_iter()
            .filter(|path| path.starts_with("archive"))
            .collect::<Vec<_>>();
        archived_paths.sort();
        for relative in archived_paths {
            let kind = expected_kind_for_path(&relative)
                .ok_or_else(|| MemoryError::UnsafePath(relative.clone()))?;
            if !access.can_read(kind) || !access.can_write(kind) {
                continue;
            }
            let path = self.resolve(&relative)?;
            let mut document = MemoryDocument::parse(&fs::read_to_string(&path)?)?;
            let mut changed = false;
            let clock_skew_guarded = clock_skew_exceeds_tolerance(&document.metadata, now);
            if clock_skew_guarded {
                report
                    .clock_skew_guarded_ids
                    .push(document.metadata.id.clone());
                audit_events.push(format!(
                    "{now}\tclock_skew_guard\t{}\t{}",
                    document.metadata.id,
                    relative.display()
                ));
            }
            if document.metadata.archived_at.is_none() {
                document.metadata.archived_at = Some(now);
                changed = true;
            }
            if !clock_skew_guarded
                && now.saturating_sub(document.metadata.touch_at) > DECAY_INTERVAL_SECONDS
                && now.saturating_sub(document.metadata.decay_at.unwrap_or_default())
                    >= DECAY_INTERVAL_SECONDS
            {
                let weight = document
                    .metadata
                    .weight
                    .ok_or(MemoryError::InvalidFrontmatter)?;
                document.metadata.weight = Some((weight * 0.9).clamp(0.0, 1.0));
                document.metadata.decay_at = Some(now);
                report.decayed_ids.push(document.metadata.id.clone());
                changed = true;
            }
            let should_forget = document.metadata.kind == "event"
                && document.metadata.importance.unwrap_or_default() < 0.2
                && document.metadata.weight.unwrap_or_default() < 0.05
                && now.saturating_sub(document.metadata.archived_at.unwrap_or(now))
                    >= FORGET_AFTER_SECONDS
                && !referenced_ids.contains(&document.metadata.id)
                && !hot_text.contains(&document.metadata.id);
            if should_forget {
                let id = document.metadata.id.clone();
                if tombstones.contains_key(&id) {
                    return Err(MemoryError::InvalidPatch(format!(
                        "forgotten memory id already has a tombstone: {id}"
                    )));
                }
                tombstones.insert(
                    id.clone(),
                    ForgottenTombstone {
                        kind: document.metadata.kind,
                        forgotten_at: now,
                        reason: "low_narrative_value".to_owned(),
                    },
                );
                tombstones_changed = true;
                index.entries.remove(&id);
                mutations.push(FileMutation::Delete { path });
                report.forgotten_ids.push(id.clone());
                audit_events.push(format!("{now}\tforget\t{id}\t{}", relative.display()));
            } else if changed {
                mutations.push(FileMutation::Write {
                    path,
                    content: document.encode()?.into_bytes(),
                });
            }
        }

        if tombstones_changed {
            mutations.push(FileMutation::Write {
                path: self.resolve(Path::new("tombstones/forgotten.yaml"))?,
                content: yaml_serde::to_string(&tombstones)?.into_bytes(),
            });
        }
        if !audit_events.is_empty() {
            let audit_path = self.resolve(Path::new("audit/memory.log"))?;
            let mut audit = fs::read_to_string(&audit_path).unwrap_or_default();
            for event in audit_events {
                audit.push_str(&event);
                audit.push('\n');
            }
            mutations.push(FileMutation::Write {
                path: audit_path,
                content: audit.into_bytes(),
            });
        }
        if !mutations.is_empty() {
            mutations.push(FileMutation::Write {
                path: self.checked_index_path()?,
                content: encode_index(&index)?.into_bytes(),
            });
            commit_mutations(&mutations)?;
        }
        Ok(report)
    }

    pub fn restore_archived(&self, id: &str) -> Result<PathBuf, MemoryError> {
        self.restore_archived_inner(id, false)
    }

    /// Restores memory after the host application has received an explicit
    /// user action. Automated callers must use `restore_archived`.
    pub fn restore_archived_authorized(&self, id: &str) -> Result<PathBuf, MemoryError> {
        self.restore_archived_inner(id, true)
    }

    /// Permanently removes one long-term memory document after an explicit user
    /// action. This bypasses archive retention and updates the local index in
    /// the same filesystem transaction.
    pub fn delete_document_authorized(&self, id: &str) -> Result<(), MemoryError> {
        let access = self.load_access()?;
        let mut index = self.load_index()?;
        let entry = index
            .entries
            .get(id)
            .cloned()
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))?;
        if entry.kind == "current" {
            return Err(MemoryError::InvalidPatch(
                "current memory documents cannot be deleted".to_owned(),
            ));
        }
        access.require_write(&entry.kind)?;
        let relative = PathBuf::from(&entry.path);
        let kind = expected_kind_for_path(&relative)
            .ok_or_else(|| MemoryError::UnsafePath(relative.clone()))?;
        if kind != entry.kind {
            return Err(MemoryError::InvalidIndex(format!(
                "index kind does not match document path: {id}"
            )));
        }
        let path = self.resolve(&relative)?;
        let document = MemoryDocument::parse(&fs::read_to_string(&path)?)?;
        if document.metadata.id != id {
            return Err(MemoryError::InvalidIndex(format!(
                "document does not match index entry: {id}"
            )));
        }
        index.entries.remove(id);
        commit_mutations(&[
            FileMutation::Delete { path },
            FileMutation::Write {
                path: self.checked_index_path()?,
                content: encode_index(&index)?.into_bytes(),
            },
        ])?;
        self.append_audit_event(&format!(
            "{}\tdelete\t{id}\t{}",
            Utc::now().timestamp(),
            relative.display()
        ))?;
        Ok(())
    }

    fn restore_archived_inner(
        &self,
        id: &str,
        explicitly_authorized: bool,
    ) -> Result<PathBuf, MemoryError> {
        let access = self.load_access()?;
        let mut index = self.load_index()?;
        let entry = index
            .entries
            .get(id)
            .cloned()
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))?;
        access.require_read(&entry.kind)?;
        access.require_write(&entry.kind)?;
        if !access.allow_archive_restore && !explicitly_authorized {
            return Err(MemoryError::AccessDenied {
                operation: "archive_restore",
                kind: entry.kind,
            });
        }
        let source_relative = PathBuf::from(&entry.path);
        if source_relative.components().next().and_then(component_name) != Some("archive") {
            return Err(MemoryError::InvalidPatch(format!(
                "memory is not archived: {id}"
            )));
        }
        let source = self.resolve(&source_relative)?;
        let mut document = MemoryDocument::parse(&fs::read_to_string(&source)?)?;
        if document.metadata.id != id || document.metadata.status != "archived" {
            return Err(MemoryError::InvalidIndex(format!(
                "archive entry does not match {id}"
            )));
        }
        let destination_relative = active_path_for(&source_relative, &document.metadata.kind)?;
        let destination = self.resolve(&destination_relative)?;
        if regular_file_exists(&destination)? {
            return Err(MemoryError::InvalidPatch(format!(
                "restore destination exists: {}",
                destination_relative.display()
            )));
        }
        document.metadata.status = "active".to_owned();
        document.metadata.touch_at = Utc::now().timestamp();
        document.metadata.archived_at = None;
        update_index_entry(&mut index, &destination_relative, &document)?;
        commit_mutations(&[
            FileMutation::Write {
                path: destination,
                content: document.encode()?.into_bytes(),
            },
            FileMutation::Delete { path: source },
            FileMutation::Write {
                path: self.checked_index_path()?,
                content: encode_index(&index)?.into_bytes(),
            },
        ])?;
        self.append_audit_event(&format!(
            "{}\trestore\t{id}\t{}",
            Utc::now().timestamp(),
            destination_relative.display()
        ))?;
        Ok(destination_relative)
    }

    pub(super) fn load_tombstones(
        &self,
    ) -> Result<BTreeMap<String, ForgottenTombstone>, MemoryError> {
        let path = self.resolve(Path::new("tombstones/forgotten.yaml"))?;
        let text = fs::read_to_string(path)?;
        Ok(yaml_serde::from_str(&text)?)
    }

    fn append_audit_event(&self, event: &str) -> Result<(), MemoryError> {
        let path = self.resolve(Path::new("audit/memory.log"))?;
        let mut text = fs::read_to_string(&path).unwrap_or_default();
        text.push_str(event);
        text.push('\n');
        atomic_write(&path, &text)
    }

    pub fn list_documents(&self) -> Result<Vec<DocumentSummary>, MemoryError> {
        let index = self.load_index()?;
        let mut summaries = Vec::new();
        for (id, entry) in &index.entries {
            let relative = PathBuf::from(&entry.path);
            let document = self.read_unchecked(&relative).ok();
            summaries.push(DocumentSummary {
                id: id.clone(),
                path: entry.path.clone(),
                kind: entry.kind.clone(),
                aliases: entry.aliases.clone(),
                tags: entry.tags.clone(),
                status: document
                    .as_ref()
                    .map(|d| d.metadata.status.clone())
                    .unwrap_or_default(),
                importance: document.as_ref().and_then(|d| d.metadata.importance),
                weight: document.as_ref().and_then(|d| d.metadata.weight),
                touch_at: document
                    .as_ref()
                    .map(|d| d.metadata.touch_at)
                    .unwrap_or_default(),
                source_character_ids: document
                    .as_ref()
                    .and_then(|document| document.metadata.relations.get("characters"))
                    .cloned()
                    .unwrap_or_default(),
                injection_scope: document
                    .as_ref()
                    .and_then(|document| document.metadata.injection_scope.clone()),
                injection_conversation_id: document
                    .as_ref()
                    .and_then(|document| document.metadata.injection_conversation_id.clone()),
                injection_character_id: document
                    .as_ref()
                    .and_then(|document| document.metadata.injection_character_id.clone()),
            });
        }
        Ok(summaries)
    }

    pub fn read_document_by_id(&self, id: &str) -> Result<MemoryDocument, MemoryError> {
        let index = self.load_index()?;
        let entry = index
            .entries
            .get(id)
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))?;
        self.read(PathBuf::from(&entry.path))
    }

    /// Replaces the complete Markdown body of one existing document. Manual
    /// editors operate on the whole body, so routing this through a synthetic
    /// section patch can accidentally target the document title or fail when
    /// an older file has no `##` section yet.
    pub fn replace_document_body(&self, id: &str, body: &str) -> Result<(), MemoryError> {
        let mut index = self.load_index()?;
        let entry = index
            .entries
            .get(id)
            .cloned()
            .ok_or_else(|| MemoryError::NotFound(id.to_owned()))?;
        let relative = PathBuf::from(&entry.path);
        let kind = expected_kind_for_path(&relative)
            .ok_or_else(|| MemoryError::UnsafePath(relative.clone()))?;
        self.load_access()?.require_write(kind)?;
        let mut document = self.read_unchecked(&relative)?;
        document.body = body.replace("\r\n", "\n");
        if !document.body.ends_with('\n') {
            document.body.push('\n');
        }
        document.metadata.touch_at = Utc::now().timestamp();
        validate_metadata(&document.metadata)?;
        update_index_entry(&mut index, &relative, &document)?;
        commit_mutations(&[
            FileMutation::Write {
                path: self.resolve(&relative)?,
                content: document.encode()?.into_bytes(),
            },
            FileMutation::Write {
                path: self.checked_index_path()?,
                content: encode_index(&index)?.into_bytes(),
            },
        ])
    }

    pub(super) fn apply_patch_with_writer<F>(
        &self,
        yaml: &str,
        writer: &mut F,
    ) -> Result<(), MemoryError>
    where
        F: FnMut(&Path, &[u8]) -> Result<(), MemoryError>,
    {
        let mutations = self.prepare_patch(yaml)?;
        commit_mutations_with(&mutations, writer)
    }

    fn prepare_patch(&self, yaml: &str) -> Result<Vec<FileMutation>, MemoryError> {
        let access = self.load_access()?;
        let patch = parse_patch_document(yaml)?;
        if patch.patches.is_empty() {
            return Ok(Vec::new());
        }
        let existing_index = self.load_index_for_rebuild()?;
        let mut index = self.build_index_data(Some(&existing_index))?;
        let mut mutations = Vec::new();
        let mut touched = HashSet::new();
        let now = Utc::now().timestamp();
        for item in patch.patches {
            if !touched.insert(item.target_file.clone()) {
                return Err(MemoryError::InvalidPatch(format!(
                    "duplicate target_file: {}",
                    item.target_file
                )));
            }
            let relative = PathBuf::from(&item.target_file);
            validate_patch_target(&relative)?;
            let kind = expected_kind_for_path(&relative)
                .ok_or_else(|| MemoryError::UnsafePath(relative.clone()))?;
            access.require_write(kind)?;
            let path = self.resolve(&relative)?;
            let creates = item
                .operations
                .iter()
                .filter(|operation| matches!(operation, PatchOperation::Create { .. }))
                .count();
            let mut document = if creates == 1 && item.operations.len() == 1 {
                if regular_file_exists(&path)? {
                    return Err(MemoryError::InvalidPatch(format!(
                        "create target exists: {}",
                        item.target_file
                    )));
                }
                match item.operations.into_iter().next() {
                    Some(PatchOperation::Create {
                        frontmatter,
                        content,
                    }) => MemoryDocument {
                        metadata: frontmatter.into_metadata(),
                        body: content,
                    },
                    _ => unreachable!(),
                }
            } else {
                if creates != 0 || item.operations.is_empty() {
                    return Err(MemoryError::InvalidPatch(
                        "create must be the only operation".to_owned(),
                    ));
                }
                let mut document = self.read_unchecked(&relative)?;
                let original_id = document.metadata.id.clone();
                for operation in item.operations {
                    apply_operation(&mut document, operation)?;
                }
                if document.metadata.id != original_id {
                    return Err(MemoryError::InvalidPatch("id cannot be changed".to_owned()));
                }
                document
            };
            document.metadata.touch_at = now;
            validate_document_location(&relative, &document.metadata)?;
            if creates == 1
                && (document.metadata.status != "active" || document.metadata.kind == "current")
            {
                return Err(MemoryError::InvalidPatch(
                    "created memories must be active long-term memories".to_owned(),
                ));
            }
            if creates == 1 && index.entries.contains_key(&document.metadata.id) {
                return Err(MemoryError::InvalidPatch(format!(
                    "duplicate memory id: {}",
                    document.metadata.id
                )));
            }
            update_index_entry(&mut index, &relative, &document)?;
            mutations.push(FileMutation::Write {
                path,
                content: document.encode()?.into_bytes(),
            });
        }
        mutations.push(FileMutation::Write {
            path: self.checked_index_path()?,
            content: encode_index(&index)?.into_bytes(),
        });
        Ok(mutations)
    }

    fn queue_touch(
        &self,
        now: i64,
        document: &MemoryDocument,
        mutations: &mut Vec<FileMutation>,
    ) -> Result<(), MemoryError> {
        if document.metadata.kind == "current" {
            return Ok(());
        }
        let index = self.load_index_for_rebuild()?;
        let Some(entry) = index.entries.get(&document.metadata.id) else {
            return Ok(());
        };
        let relative = PathBuf::from(&entry.path);
        let mut touched = document.clone();
        touched.metadata.touch_at = now;
        mutations.push(FileMutation::Write {
            path: self.resolve(&relative)?,
            content: touched.encode()?.into_bytes(),
        });
        Ok(())
    }

    fn record_memory_activity(
        &self,
        now: i64,
        ids: &[String],
        mutations: &mut Vec<FileMutation>,
    ) -> Result<(), MemoryError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut activity = self.load_activity()?;
        let mut changed = false;
        for id in ids {
            let entry = activity.entries.entry(id.clone()).or_default();
            entry.last_injected_at = now;
            entry.injection_count = entry.injection_count.saturating_add(1);
            changed = true;
        }
        if changed {
            mutations.push(FileMutation::Write {
                path: self.checked_activity_path()?,
                content: encode_activity(&activity)?.into_bytes(),
            });
        }
        Ok(())
    }

    fn resolve(&self, relative: &Path) -> Result<PathBuf, MemoryError> {
        validate_relative(relative)?;
        let mut resolved = self.root.clone();
        let component_count = relative.components().count();
        for (index, component) in relative.components().enumerate() {
            let Component::Normal(name) = component else {
                return Err(MemoryError::UnsafePath(relative.to_path_buf()));
            };
            resolved.push(name);
            match fs::symlink_metadata(&resolved) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink()
                        || (index + 1 < component_count && !metadata.is_dir())
                    {
                        return Err(MemoryError::UnsafePath(relative.to_path_buf()));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if index + 1 < component_count {
                        return Err(MemoryError::UnsafePath(relative.to_path_buf()));
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(resolved)
    }

    #[cfg(test)]
    pub(super) fn index_path(&self) -> PathBuf {
        self.root.join("indexes/memory_index.yaml")
    }

    fn checked_index_path(&self) -> Result<PathBuf, MemoryError> {
        self.resolve(Path::new("indexes/memory_index.yaml"))
    }

    fn checked_activity_path(&self) -> Result<PathBuf, MemoryError> {
        self.resolve(Path::new("indexes/memory_activity.yaml"))
    }

    fn load_activity(&self) -> Result<MemoryActivity, MemoryError> {
        let path = self.checked_activity_path()?;
        let parsed = match fs::read_to_string(path) {
            Ok(text) => yaml_serde::from_str::<MemoryActivity>(&text)
                .ok()
                .filter(|activity| activity.version == 1),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        Ok(parsed.unwrap_or(MemoryActivity {
            version: 1,
            entries: BTreeMap::new(),
        }))
    }

    pub(super) fn load_index(&self) -> Result<MemoryIndex, MemoryError> {
        let path = self.checked_index_path()?;
        let parsed = match fs::read_to_string(&path) {
            Ok(text) => yaml_serde::from_str::<MemoryIndex>(&text)
                .ok()
                .filter(|index| index.version == 1),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let rebuilt = self.build_index_data(parsed.as_ref())?;
        if parsed.as_ref() != Some(&rebuilt) {
            commit_mutations(&[FileMutation::Write {
                path,
                content: encode_index(&rebuilt)?.into_bytes(),
            }])?;
        }
        Ok(rebuilt)
    }

    fn load_index_for_rebuild(&self) -> Result<MemoryIndex, MemoryError> {
        let path = self.checked_index_path()?;
        if !regular_file_exists(&path)? {
            return Ok(MemoryIndex {
                version: 1,
                entries: BTreeMap::new(),
            });
        }
        let text = fs::read_to_string(path)?;
        Ok(yaml_serde::from_str::<MemoryIndex>(&text)
            .ok()
            .filter(|index| index.version == 1)
            .unwrap_or(MemoryIndex {
                version: 1,
                entries: BTreeMap::new(),
            }))
    }

    fn build_index_data(&self, previous: Option<&MemoryIndex>) -> Result<MemoryIndex, MemoryError> {
        let mut paths = self.all_long_term_memory_paths()?;
        paths.sort();
        let mut entries = BTreeMap::new();
        for relative in paths {
            let document = self.read_unchecked(&relative)?;
            validate_document_location(&relative, &document.metadata)?;
            let id = document.metadata.id.clone();
            let mut aliases = if document.metadata.aliases.is_empty() {
                previous
                    .and_then(|index| index.entries.get(&id))
                    .map(|entry| entry.aliases.clone())
                    .unwrap_or_default()
            } else {
                document.metadata.aliases.clone()
            };
            add_title_alias(&mut aliases, &id, &document.body);
            let entry = IndexEntry {
                path: portable_path(&relative),
                kind: document.metadata.kind.clone(),
                aliases,
                tags: document.metadata.tags.clone(),
            };
            if entries.insert(id.clone(), entry).is_some() {
                return Err(MemoryError::InvalidIndex(format!(
                    "duplicate memory id: {id}"
                )));
            }
        }
        Ok(MemoryIndex {
            version: 1,
            entries,
        })
    }

    fn read_unchecked(&self, relative: &Path) -> Result<MemoryDocument, MemoryError> {
        let path = self.resolve(relative)?;
        MemoryDocument::parse(&fs::read_to_string(path)?)
    }

    fn load_access(&self) -> Result<AccessConfig, MemoryError> {
        let path = self.resolve(Path::new("config/access.yaml"))?;
        let text = fs::read_to_string(path)?;
        let access: AccessConfig = yaml_serde::from_str(&text)?;
        access.validate()?;
        Ok(access)
    }

    fn active_memory_paths(&self) -> Result<Vec<PathBuf>, MemoryError> {
        let mut paths = Vec::new();
        for (directory, _) in ACTIVE_MEMORY_DIRECTORIES {
            let directory = self.resolve(Path::new(directory))?;
            collect_markdown_paths(&self.root, &directory, &mut paths)?;
        }
        Ok(paths)
    }

    fn all_long_term_memory_paths(&self) -> Result<Vec<PathBuf>, MemoryError> {
        let mut paths = self.active_memory_paths()?;
        for (_, kind) in ACTIVE_MEMORY_DIRECTORIES {
            let relative = Path::new("archive").join(kind);
            let directory = self.resolve(&relative)?;
            collect_markdown_paths(&self.root, &directory, &mut paths)?;
        }
        Ok(paths)
    }
}
