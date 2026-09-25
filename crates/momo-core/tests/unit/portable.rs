use super::*;

#[test]
fn encrypted_moc_envelope_is_bounded_before_allocation() {
    validate_private_moc_envelope_size(PRIVATE_MOC_MAX_ENVELOPE_BYTES).expect("boundary envelope");
    assert!(matches!(
        validate_private_moc_envelope_size(PRIVATE_MOC_MAX_ENVELOPE_BYTES + 1),
        Err(PortableError::PrivateMocTooLarge)
    ));
}
use momo_domain::{MessageRole, new_id};

fn test_export_plan(
    space_id: Uuid,
    modules: &[MocModule],
    character_id: Option<Uuid>,
    compatibility: MocCompatibility,
) -> MocExportPlan {
    let selected = modules.iter().copied().collect::<HashSet<_>>();
    MocExportPlan {
        characters: selected
            .contains(&MocModule::Characters)
            .then(|| MocCharacterSelection {
                space_id,
                character_ids: character_id.into_iter().collect(),
            })
            .into_iter()
            .collect(),
        conversations: selected
            .contains(&MocModule::Conversations)
            .then_some(space_id)
            .into_iter()
            .collect(),
        memory: selected
            .contains(&MocModule::Memory)
            .then_some(space_id)
            .into_iter()
            .collect(),
        semantic_graph: selected
            .contains(&MocModule::SemanticGraph)
            .then_some(space_id)
            .into_iter()
            .collect(),
        compatibility,
    }
}

fn test_import_plan(source: Uuid, target: Uuid, conflict_mode: ConflictMode) -> MocImportPlan {
    MocImportPlan {
        space_map: [(source, target)].into_iter().collect(),
        conflict_mode,
    }
}

async fn seed_character_conversation(core: &MomoCore, scope_id: Uuid) -> (Uuid, Uuid) {
    let character_id = new_id();
    let conversation_id = new_id();
    let now = Utc::now();
    core.store()
        .save_character(&CharacterCard {
            id: character_id,
            scope_id,
            name: "Portable character".to_owned(),
            version: "2.0.0".to_owned(),
            author_name: "Tester".to_owned(),
            author_url: None,
            character_markdown: "# Character".to_owned(),
            user_markdown: String::new(),
            opening_markdown: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("character");
    core.store()
        .save_conversation(&Conversation {
            id: conversation_id,
            scope_id,
            character_id: Some(character_id),
            title: "Portable conversation".to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("conversation");
    (character_id, conversation_id)
}

#[tokio::test]
async fn round_trips_selected_moc_modules_and_rebinds_scope() {
    let source_directory = tempfile::tempdir().expect("source directory");
    let source = MomoCore::initialize(source_directory.path())
        .await
        .expect("source core");
    let original_scope = new_id();
    let character_id = new_id();
    let now = Utc::now();
    source
        .store()
        .save_character(&CharacterCard {
            id: character_id,
            scope_id: original_scope,
            name: "雪球".to_owned(),
            version: "2.0.0".to_owned(),
            author_name: "Tester".to_owned(),
            author_url: Some("https://example.com/creator".to_owned()),
            character_markdown: "# Character".to_owned(),
            user_markdown: "# User".to_owned(),
            opening_markdown: Some("Hello".to_owned()),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("character");
    source
        .store()
        .save_portable_metadata(
            "character",
            &character_id.to_string(),
            "future_field = \"preserve-me\"\n",
        )
        .await
        .expect("portable metadata");
    let conversation_id = new_id();
    source
        .store()
        .save_conversation(&Conversation {
            id: conversation_id,
            scope_id: original_scope,
            character_id: Some(character_id),
            title: "Portable".to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("conversation");
    source
        .store()
        .append_message(&Message {
            id: new_id(),
            conversation_id,
            role: MessageRole::User,
            content: "hello".to_owned(),
            created_at: now,
        })
        .await
        .expect("message");

    let ddm_profile = format!(
        "schema: momo.ddm/1\ncharacter_id: {character_id}\nrevision: 1\nprofile: logit_additive\ndispositions:\n  - id: patient_help\n    base_activation: 0.7\n    expression:\n      latent: Wait.\n      salient: Offer help.\n      dominant: Stay present.\n"
    );
    source
        .store()
        .save_portable_metadata(
            DDM_PROFILE_METADATA_KIND,
            &character_id.to_string(),
            &ddm_profile,
        )
        .await
        .expect("DDM profile");

    let output = source_directory.path().join("backup.moc");
    let manifest = export_moc(
        &source,
        &output,
        &test_export_plan(
            original_scope,
            &[
                MocModule::Characters,
                MocModule::Conversations,
                MocModule::Memory,
                MocModule::SemanticGraph,
            ],
            None,
            MocCompatibility::None,
        ),
    )
    .await
    .expect("export");
    assert!(
        manifest
            .modules
            .iter()
            .any(|entry| entry.module == "characters")
    );
    assert!(
        manifest.module_definitions.iter().any(|module| {
            module.id == "conversations" && module.dependencies == ["characters"]
        })
    );

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination core");
    let new_scope = new_id();
    let report = import_moc(
        &destination,
        &output,
        &test_import_plan(original_scope, new_scope, ConflictMode::Replace),
    )
    .await
    .expect("import");
    assert_eq!(report.characters_imported, 1);
    assert_eq!(report.conversations_imported, 1);
    assert_eq!(report.messages_imported, 1);
    assert!(report.memory_files_imported >= 3);
    assert_eq!(
        destination
            .store()
            .portable_metadata(DDM_PROFILE_METADATA_KIND, &character_id.to_string())
            .await
            .expect("profile metadata")
            .expect("imported DDM profile"),
        ddm_profile
    );
    let imported_cards = destination.store().list_characters().await.expect("cards");
    assert_eq!(imported_cards[0].scope_id, new_scope);
    assert_eq!(imported_cards[0].opening_markdown.as_deref(), Some("Hello"));

    let second_output = destination_directory.path().join("restored.moc");
    export_moc(
        &destination,
        &second_output,
        &test_export_plan(
            new_scope,
            &[MocModule::Characters],
            None,
            MocCompatibility::None,
        ),
    )
    .await
    .expect("re-export");
    let inspected = tempfile::tempdir().expect("inspect directory");
    momo_moc::extract(
        &second_output,
        inspected.path(),
        ExtractionLimits::default(),
    )
    .expect("inspect MOC");
    let metadata = fs::read_to_string(
        inspected
            .path()
            .join("characters")
            .join("spaces")
            .join(new_scope.to_string())
            .join(character_id.to_string())
            .join("character.toml"),
    )
    .expect("metadata");
    assert!(metadata.contains("future_field = \"preserve-me\""));
    assert!(metadata.contains("opening_file = \"opening.md\""));
    assert_eq!(
        fs::read_to_string(
            inspected
                .path()
                .join("characters")
                .join("spaces")
                .join(new_scope.to_string())
                .join(character_id.to_string())
                .join("opening.md")
        )
        .expect("opening"),
        "Hello"
    );
    assert_eq!(
        fs::read_to_string(
            inspected
                .path()
                .join("characters")
                .join("spaces")
                .join(new_scope.to_string())
                .join(character_id.to_string())
                .join(DDM_PROFILE_ASSET)
        )
        .expect("re-exported DDM profile"),
        ddm_profile
    );

    let private_output = source_directory.path().join("private.moc");
    export_private_moc(
        &source,
        &private_output,
        &test_export_plan(
            original_scope,
            &[MocModule::Characters],
            Some(character_id),
            MocCompatibility::None,
        ),
        "private-password",
    )
    .await
    .expect("private export");
    assert!(moc_is_encrypted(&private_output).expect("inspect private"));
    let private_import = test_import_plan(original_scope, new_scope, ConflictMode::KeepExisting);
    assert!(matches!(
        import_moc_with_passphrase(
            &destination,
            &private_output,
            &private_import,
            Some("wrong-password")
        )
        .await,
        Err(PortableError::Crypto(
            momo_crypto::CryptoError::AuthenticationFailed
        ))
    ));
    let private_report = import_moc_with_passphrase(
        &destination,
        &private_output,
        &private_import,
        Some("private-password"),
    )
    .await
    .expect("private import");
    assert_eq!(private_report.skipped_conflicts, 1);
    assert_eq!(
        destination
            .store()
            .list_conversations()
            .await
            .expect("conversations")[0]
            .scope_id,
        new_scope
    );
}

#[tokio::test]
async fn host_modules_are_explicitly_exported_reported_and_claimed() {
    let source_directory = tempfile::tempdir().expect("source directory");
    let source = MomoCore::initialize(source_directory.path())
        .await
        .expect("source core");
    let extension = source_directory.path().join("weather-module");
    fs::create_dir_all(&extension).expect("extension directory");
    fs::write(
        extension.join("module.json"),
        br#"{"provider":"local-weather"}"#,
    )
    .expect("extension payload");
    let output = source_directory.path().join("extension-only.moc");
    let host_module = HostMocModule {
        id: "weather".to_owned(),
        input_path: extension,
        dependencies: vec!["config".to_owned()],
        import_order: 900,
    };
    let manifest = export_moc_with_host_modules(
        &source,
        &output,
        &test_export_plan(new_id(), &[], None, MocCompatibility::None),
        std::slice::from_ref(&host_module),
    )
    .await
    .expect("export host module");
    assert_eq!(manifest.module_definitions[0].id, "weather");
    assert_eq!(manifest.module_definitions[0].dependencies, ["config"]);

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination core");
    let host_import = MocImportPlan {
        space_map: BTreeMap::new(),
        conflict_mode: ConflictMode::Replace,
    };
    let report = import_moc(&destination, &output, &host_import)
        .await
        .expect("report unknown module");
    assert_eq!(report.unknown_modules.len(), 1);
    assert_eq!(report.unknown_modules[0].id, "weather");
    assert_eq!(report.unknown_modules[0].claimed_path, None);

    let claims = destination_directory.path().join("claims");
    let report = import_moc_claiming_unknown_modules(&destination, &output, &host_import, &claims)
        .await
        .expect("claim unknown module");
    assert_eq!(
        report.unknown_modules[0].claimed_path.as_deref(),
        Some(claims.join("weather").to_string_lossy().as_ref())
    );
    assert_eq!(
        fs::read_to_string(claims.join("weather/module.json")).expect("claimed payload"),
        r#"{"provider":"local-weather"}"#
    );

    let reserved = HostMocModule {
        id: "characters".to_owned(),
        ..host_module
    };
    assert!(matches!(
        export_moc_with_host_modules(
            &source,
            source_directory.path().join("invalid.moc"),
            &test_export_plan(new_id(), &[], None, MocCompatibility::None),
            &[reserved],
        )
        .await,
        Err(PortableError::InvalidData(_))
    ));
}

#[tokio::test]
async fn imports_character_card_v3_without_user_file() {
    let root = tempfile::tempdir().expect("moc root");
    let character_id = new_id();
    let scope_id = new_id();
    let character_dir = root
        .path()
        .join("characters")
        .join("spaces")
        .join(scope_id.to_string())
        .join(character_id.to_string());
    fs::create_dir_all(&character_dir).expect("character directory");
    atomic_write(
        &character_dir.join("character.toml"),
        format!(
            r#"
id = "urn:uuid:{character_id}"
name = "No User Context"
version = "2.0.0"
character_file = "character.md"

[author]
name = "Creator"
"#
        )
        .as_bytes(),
    )
    .expect("metadata");
    atomic_write(&character_dir.join("character.md"), b"# Character").expect("character");
    atomic_write(
        &character_dir
            .parent()
            .expect("character Space")
            .join("index.json"),
        &serde_json::to_vec_pretty(&vec![character_id]).expect("index JSON"),
    )
    .expect("character index");

    let output = root.path().join("optional-user.moc");
    momo_moc::create_from_definitions_and_spaces(
        &output,
        root.path(),
        &[known_module_definition("characters")],
        &[space_module_definition("characters", scope_id)],
    )
    .expect("create moc");

    let target_directory = tempfile::tempdir().expect("target directory");
    let target = MomoCore::initialize(target_directory.path())
        .await
        .expect("target core");
    let report = import_moc(
        &target,
        &output,
        &test_import_plan(scope_id, scope_id, ConflictMode::Replace),
    )
    .await
    .expect("import moc");

    assert_eq!(report.characters_imported, 1);
    let characters = target
        .store()
        .list_characters()
        .await
        .expect("loaded characters");
    let imported = characters
        .iter()
        .find(|character| character.id == character_id)
        .expect("imported character");
    assert_eq!(imported.user_markdown, "");
}

#[tokio::test]
async fn preflight_rejects_late_invalid_payload_before_committing_earlier_modules() {
    let root = tempfile::tempdir().expect("MOC root");
    let space_id = new_id();
    let character_id = new_id();
    let character_space = root
        .path()
        .join("characters/spaces")
        .join(space_id.to_string());
    let character_directory = character_space.join(character_id.to_string());
    fs::create_dir_all(&character_directory).expect("character directory");
    atomic_write(
        &character_directory.join("character.toml"),
        format!(
            r#"id = "urn:uuid:{character_id}"
name = "Preflight"
version = "2.0.0"
character_file = "character.md"

[author]
name = "Tester"
"#
        )
        .as_bytes(),
    )
    .expect("character metadata");
    atomic_write(&character_directory.join("character.md"), b"# Character")
        .expect("character Markdown");
    atomic_write(
        &character_space.join("index.json"),
        &serde_json::to_vec_pretty(&vec![character_id]).expect("character index"),
    )
    .expect("character index");

    let conversation_space = root
        .path()
        .join("conversations/spaces")
        .join(space_id.to_string());
    fs::create_dir_all(&conversation_space).expect("conversation directory");
    let conversation_id = new_id();
    let now = Utc::now();
    atomic_write(
        &conversation_space.join("index.json"),
        &serde_json::to_vec_pretty(&vec![Conversation {
            id: conversation_id,
            scope_id: space_id,
            character_id: Some(character_id),
            title: "Preflight".to_owned(),
            created_at: now,
            updated_at: now,
        }])
        .expect("conversation index"),
    )
    .expect("conversation index");
    atomic_write(
        &conversation_space.join("messages.json"),
        &serde_json::to_vec_pretty(&vec![Message {
            id: new_id(),
            conversation_id: new_id(),
            role: MessageRole::User,
            content: "orphan".to_owned(),
            created_at: now,
        }])
        .expect("messages"),
    )
    .expect("messages");

    let output = root.path().join("invalid-late-module.moc");
    momo_moc::create_from_definitions_and_spaces(
        &output,
        root.path(),
        &[
            known_module_definition("characters"),
            known_module_definition("conversations"),
        ],
        &[
            space_module_definition("characters", space_id),
            space_module_definition("conversations", space_id),
        ],
    )
    .expect("create MOC");

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination Core");
    let error = import_moc(
        &destination,
        &output,
        &test_import_plan(space_id, space_id, ConflictMode::Replace),
    )
    .await
    .expect_err("orphan message must fail preflight");
    assert!(matches!(error, PortableError::InvalidData(_)));
    assert!(
        destination
            .store()
            .list_characters()
            .await
            .expect("characters")
            .is_empty(),
        "a later invalid module must not leave an earlier character committed"
    );
    assert!(
        destination
            .store()
            .list_conversations()
            .await
            .expect("conversations")
            .is_empty()
    );
}

#[tokio::test]
async fn import_uses_declared_module_order_when_space_declarations_are_reordered() {
    let source_directory = tempfile::tempdir().expect("source directory");
    let source = MomoCore::initialize(source_directory.path())
        .await
        .expect("source core");
    let source_space = new_id();
    let (character_id, conversation_id) = seed_character_conversation(&source, source_space).await;
    let canonical = source_directory.path().join("canonical.moc");
    export_moc(
        &source,
        &canonical,
        &test_export_plan(
            source_space,
            &[MocModule::Characters, MocModule::Conversations],
            None,
            MocCompatibility::None,
        ),
    )
    .await
    .expect("canonical export");
    let manifest = momo_moc::inspect(&canonical).expect("manifest");
    let extracted = tempfile::tempdir().expect("extracted payload");
    momo_moc::extract(&canonical, extracted.path(), ExtractionLimits::default())
        .expect("extract canonical MOC");
    let mut reordered_spaces = manifest.space_modules.clone();
    reordered_spaces.reverse();
    assert_eq!(reordered_spaces[0].module, "conversations");
    let reordered = source_directory.path().join("reordered.moc");
    momo_moc::create_from_definitions_and_spaces(
        &reordered,
        extracted.path(),
        &manifest.module_definitions,
        &reordered_spaces,
    )
    .expect("valid reordered MOC");

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination core");
    let target_space = new_id();
    import_moc(
        &destination,
        &reordered,
        &test_import_plan(source_space, target_space, ConflictMode::Replace),
    )
    .await
    .expect("import reordered MOC");

    let conversation = destination
        .store()
        .conversation_for_scope(target_space, conversation_id)
        .await
        .expect("conversation lookup")
        .expect("imported conversation");
    assert_eq!(conversation.character_id, Some(character_id));
}

#[tokio::test]
async fn preflight_rejects_cross_space_conversation_conflicts_before_importing_characters() {
    let source_directory = tempfile::tempdir().expect("source directory");
    let source = MomoCore::initialize(source_directory.path())
        .await
        .expect("source core");
    let source_space = new_id();
    let (character_id, conversation_id) = seed_character_conversation(&source, source_space).await;
    let package = source_directory.path().join("cross-space-conflict.moc");
    export_moc(
        &source,
        &package,
        &test_export_plan(
            source_space,
            &[MocModule::Characters, MocModule::Conversations],
            None,
            MocCompatibility::None,
        ),
    )
    .await
    .expect("export");

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination core");
    let existing_space = new_id();
    let target_space = new_id();
    let now = Utc::now();
    destination
        .store()
        .save_conversation(&Conversation {
            id: conversation_id,
            scope_id: existing_space,
            character_id: None,
            title: "Existing elsewhere".to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("existing conversation");

    let error = import_moc(
        &destination,
        &package,
        &test_import_plan(source_space, target_space, ConflictMode::Replace),
    )
    .await
    .expect_err("cross-Space conflict must fail preflight");
    assert!(matches!(error, PortableError::InvalidData(_)));
    assert!(
        destination
            .store()
            .list_characters()
            .await
            .expect("characters")
            .iter()
            .all(|character| character.id != character_id),
        "preflight failure must not commit the earlier character module"
    );
}

#[tokio::test]
async fn preflight_rejects_cross_space_character_conflicts() {
    let source_directory = tempfile::tempdir().expect("source directory");
    let source = MomoCore::initialize(source_directory.path())
        .await
        .expect("source core");
    let source_space = new_id();
    let (character_id, conversation_id) = seed_character_conversation(&source, source_space).await;
    let package = source_directory.path().join("character-conflict.moc");
    export_moc(
        &source,
        &package,
        &test_export_plan(
            source_space,
            &[MocModule::Characters, MocModule::Conversations],
            None,
            MocCompatibility::None,
        ),
    )
    .await
    .expect("export");

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination core");
    let existing_space = new_id();
    let target_space = new_id();
    let now = Utc::now();
    destination
        .store()
        .save_character(&CharacterCard {
            id: character_id,
            scope_id: existing_space,
            name: "Existing elsewhere".to_owned(),
            version: "2.0.0".to_owned(),
            author_name: "Tester".to_owned(),
            author_url: None,
            character_markdown: String::new(),
            user_markdown: String::new(),
            opening_markdown: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("existing character");

    let error = import_moc(
        &destination,
        &package,
        &test_import_plan(source_space, target_space, ConflictMode::Replace),
    )
    .await
    .expect_err("cross-Space character conflict must fail preflight");
    assert!(matches!(error, PortableError::InvalidData(_)));
    assert!(
        destination
            .store()
            .conversation_for_scope(target_space, conversation_id)
            .await
            .expect("conversation lookup")
            .is_none()
    );
    assert_eq!(
        destination
            .store()
            .character_by_id(character_id)
            .await
            .expect("character lookup")
            .expect("existing character")
            .scope_id,
        existing_space
    );
}

#[tokio::test]
async fn preflight_rejects_message_ids_owned_by_another_conversation() {
    let source_directory = tempfile::tempdir().expect("source directory");
    let source = MomoCore::initialize(source_directory.path())
        .await
        .expect("source core");
    let source_space = new_id();
    let (character_id, conversation_id) = seed_character_conversation(&source, source_space).await;
    let message_id = new_id();
    let now = Utc::now();
    source
        .store()
        .append_message(&Message {
            id: message_id,
            conversation_id,
            role: MessageRole::User,
            content: "Portable message".to_owned(),
            created_at: now,
        })
        .await
        .expect("source message");
    let package = source_directory.path().join("message-conflict.moc");
    export_moc(
        &source,
        &package,
        &test_export_plan(
            source_space,
            &[MocModule::Characters, MocModule::Conversations],
            None,
            MocCompatibility::None,
        ),
    )
    .await
    .expect("export");

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination core");
    let existing_space = new_id();
    let existing_conversation = new_id();
    destination
        .store()
        .save_conversation(&Conversation {
            id: existing_conversation,
            scope_id: existing_space,
            character_id: None,
            title: "Existing conversation".to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("existing conversation");
    destination
        .store()
        .append_message(&Message {
            id: message_id,
            conversation_id: existing_conversation,
            role: MessageRole::User,
            content: "Existing message".to_owned(),
            created_at: now,
        })
        .await
        .expect("existing message");
    let target_space = new_id();

    let error = import_moc(
        &destination,
        &package,
        &test_import_plan(source_space, target_space, ConflictMode::Replace),
    )
    .await
    .expect_err("cross-conversation message conflict must fail preflight");
    assert!(matches!(error, PortableError::InvalidData(_)));
    assert!(
        destination
            .store()
            .character_by_id(character_id)
            .await
            .expect("character lookup")
            .is_none(),
        "preflight failure must not commit the character module"
    );
}

#[tokio::test]
async fn imports_conversation_only_moc_without_missing_character_foreign_key() {
    let source_directory = tempfile::tempdir().expect("source directory");
    let source = MomoCore::initialize(source_directory.path())
        .await
        .expect("source core");
    let scope_id = new_id();
    let character_id = new_id();
    let conversation_id = new_id();
    let now = Utc::now();
    source
        .store()
        .save_character(&CharacterCard {
            id: character_id,
            scope_id,
            name: "Parent card".to_owned(),
            version: "2.0.0".to_owned(),
            author_name: "Tester".to_owned(),
            author_url: None,
            character_markdown: String::new(),
            user_markdown: String::new(),
            opening_markdown: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("character");
    source
        .store()
        .save_conversation(&Conversation {
            id: conversation_id,
            scope_id,
            character_id: Some(character_id),
            title: "Conversation without exported card".to_owned(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("conversation");

    let output = source_directory.path().join("conversation-only.moc");
    export_moc(
        &source,
        &output,
        &test_export_plan(
            scope_id,
            &[MocModule::Conversations],
            None,
            MocCompatibility::None,
        ),
    )
    .await
    .expect("conversation-only export");

    let destination_directory = tempfile::tempdir().expect("destination directory");
    let destination = MomoCore::initialize(destination_directory.path())
        .await
        .expect("destination core");
    let target_space = new_id();
    let report = import_moc(
        &destination,
        &output,
        &test_import_plan(scope_id, target_space, ConflictMode::Replace),
    )
    .await
    .expect("conversation-only import");
    assert_eq!(report.conversations_imported, 1);
    assert_eq!(report.messages_imported, 0);
    let imported = destination
        .store()
        .list_conversations()
        .await
        .expect("imported conversations");
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].character_id, None);
}

#[tokio::test]
async fn selected_character_export_and_generated_compatibility_are_explicit() {
    let directory = tempfile::tempdir().expect("directory");
    let core = MomoCore::initialize(directory.path()).await.expect("core");
    let scope_id = new_id();
    let selected_id = new_id();
    let now = Utc::now();
    for (id, name) in [(selected_id, "Selected"), (new_id(), "Other")] {
        core.store()
            .save_character(&CharacterCard {
                id,
                scope_id,
                name: name.to_owned(),
                version: "2.0.0".to_owned(),
                author_name: "Tester".to_owned(),
                author_url: None,
                character_markdown: "# Character".to_owned(),
                user_markdown: String::new(),
                opening_markdown: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("character");
    }
    let output = directory.path().join("selected.moc");
    export_moc(
        &core,
        &output,
        &test_export_plan(
            scope_id,
            &[MocModule::Characters],
            Some(selected_id),
            MocCompatibility::None,
        ),
    )
    .await
    .expect("shortcut export");
    let extracted = tempfile::tempdir().expect("extract directory");
    momo_moc::extract(&output, extracted.path(), ExtractionLimits::default()).expect("extract");
    let character_space = extracted
        .path()
        .join("characters/spaces")
        .join(scope_id.to_string());
    let card_directories = fs::read_dir(&character_space)
        .expect("characters")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    assert_eq!(card_directories.len(), 1);
    assert!(
        extracted
            .path()
            .join("characters/spaces")
            .join(scope_id.to_string())
            .join(selected_id.to_string())
            .exists()
    );

    let compatible_output = directory.path().join("selected-compatible.moc");
    export_moc(
        &core,
        &compatible_output,
        &test_export_plan(
            scope_id,
            &[MocModule::Characters],
            Some(selected_id),
            MocCompatibility::GeneratedCcv2Json,
        ),
    )
    .await
    .expect("generated compatibility export");
    let compatible = tempfile::tempdir().expect("compatible extract directory");
    momo_moc::extract(
        &compatible_output,
        compatible.path(),
        ExtractionLimits::default(),
    )
    .expect("extract compatible MOC");
    assert!(
        compatible
            .path()
            .join("tavern_compat")
            .join(selected_id.to_string())
            .join("generated.ccv2.json")
            .is_file()
    );
    let destination_dir = tempfile::tempdir().expect("destination");
    let destination = MomoCore::initialize(destination_dir.path())
        .await
        .expect("destination core");
    let report = import_moc(
        &destination,
        &compatible_output,
        &test_import_plan(scope_id, new_id(), ConflictMode::Replace),
    )
    .await
    .expect("generated compatibility does not become provenance");
    assert_eq!(report.characters_imported, 1);
}

fn valid_character_metadata() -> CharacterMetadata {
    CharacterMetadata {
        id: format!("urn:uuid:{}", new_id()),
        name: "Snowball".to_owned(),
        version: "1.2.3".to_owned(),
        author: CharacterAuthor {
            name: "Creator".to_owned(),
            url: Some("https://example.com".to_owned()),
        },
        character_file: "content/character.md".to_owned(),
        user_file: Some("content/user.md".to_owned()),
        opening_file: Some("content/opening.md".to_owned()),
    }
}

#[test]
fn character_v2_metadata_enforces_semver_author_and_safe_markdown_paths() {
    let mut metadata = valid_character_metadata();
    validate_character_metadata(&metadata).expect("valid metadata");

    metadata.version = "release-one".to_owned();
    assert!(validate_character_metadata(&metadata).is_err());
    metadata.version = "1.0.0".to_owned();
    metadata.author.url = Some("not a URL".to_owned());
    assert!(validate_character_metadata(&metadata).is_err());
    metadata.author.url = None;
    metadata.character_file = "../character.md".to_owned();
    assert!(validate_character_metadata(&metadata).is_err());
    metadata.character_file = "character.md".to_owned();
    metadata.opening_file = Some("opening.txt".to_owned());
    assert!(validate_character_metadata(&metadata).is_err());
}

#[test]
fn legacy_character_metadata_is_rejected_by_native_v2_import() {
    let values = ConfigDocument::parse(&format!(
        r#"
id = "urn:uuid:{}"
name = "Legacy"
version = "1.0.0"
description = "removed"
language = "en"
tags = ["removed"]
future_field = "preserved"
character_file = "character.md"
user_file = "user.md"

[author]
uid = "legacy_uid"
display_name = "Legacy Author"
"#,
        new_id()
    ))
    .expect("legacy metadata");
    assert!(parse_character_metadata(values.values()).is_err());
}

#[test]
fn markdown_asset_accepts_utf8_bom_but_rejects_frontmatter_and_links() {
    let directory = tempfile::tempdir().expect("directory");
    let content = directory.path().join("content");
    fs::create_dir_all(&content).expect("content directory");
    fs::write(content.join("character.md"), b"\xEF\xBB\xBF# Character").expect("write markdown");
    assert_eq!(
        read_markdown_asset(directory.path(), Path::new("content/character.md"))
            .expect("BOM accepted"),
        "# Character"
    );

    fs::write(
        content.join("frontmatter.md"),
        "---\nsecret: true\n---\nbody",
    )
    .expect("write frontmatter");
    assert!(read_markdown_asset(directory.path(), Path::new("content/frontmatter.md")).is_err());

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(content.join("character.md"), content.join("linked.md"))
            .expect("symlink");
        assert!(read_markdown_asset(directory.path(), Path::new("content/linked.md")).is_err());
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_file(
            content.join("character.md"),
            content.join("linked.md"),
        )
        .is_ok()
        {
            assert!(read_markdown_asset(directory.path(), Path::new("content/linked.md")).is_err());
        }
    }
}
