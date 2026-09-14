use super::*;

fn write_raw_container(output: &Path, manifest: &Manifest, payloads: &[(&str, &[u8])]) {
    let encoder =
        zstd::Encoder::new(File::create(output).expect("create archive"), 1).expect("encoder");
    let mut archive = tar::Builder::new(encoder.auto_finish());
    let manifest_text = toml::to_string_pretty(manifest).expect("manifest");
    append_bytes(&mut archive, "manifest.toml", manifest_text.as_bytes()).expect("append manifest");
    for (path, bytes) in payloads {
        append_bytes(&mut archive, path, bytes).expect("append payload");
    }
    archive.finish().expect("finish archive");
}

#[test]
fn creates_and_extracts_verified_container() {
    let root = tempfile::tempdir().expect("source directory");
    fs::create_dir(root.path().join("config")).expect("config directory");
    fs::write(root.path().join("config/user.toml"), "stream = true\n").expect("fixture");
    let output = root.path().join("backup.moc");
    let manifest = create(
        &output,
        root.path(),
        &[("config".to_owned(), PathBuf::from("config"))],
    )
    .expect("create MOC");
    assert_eq!(manifest.format_version, FORMAT_VERSION);
    assert_eq!(manifest.modules.len(), 1);
    assert_eq!(
        manifest.module_definitions,
        vec![ModuleDefinition {
            id: "config".to_owned(),
            path: "config".to_owned(),
            dependencies: Vec::new(),
            import_order: 10,
        }]
    );
    assert!(manifest.encryption.is_none());

    let extracted = tempfile::tempdir().expect("destination");
    let decoded =
        extract(&output, extracted.path(), ExtractionLimits::default()).expect("extract MOC");
    assert_eq!(decoded.modules, manifest.modules);
    assert_eq!(
        fs::read_to_string(extracted.path().join("config/user.toml")).expect("extracted file"),
        "stream = true\n"
    );
}

#[test]
fn creates_host_extension_from_explicit_module_definition() {
    let root = tempfile::tempdir().expect("source directory");
    let module_root = root.path().join("extensions/weather");
    fs::create_dir_all(&module_root).expect("module directory");
    fs::write(module_root.join("module.json"), br#"{"unit":"celsius"}"#).expect("module payload");
    let output = root.path().join("extension.moc");
    let definition = ModuleDefinition {
        id: "weather".to_owned(),
        path: "extensions/weather".to_owned(),
        dependencies: vec!["config".to_owned()],
        import_order: 900,
    };

    let manifest = create_from_definitions(&output, root.path(), std::slice::from_ref(&definition))
        .expect("create extension MOC");
    assert_eq!(manifest.module_definitions, [definition]);
    let extracted = tempfile::tempdir().expect("extracted directory");
    extract(&output, extracted.path(), ExtractionLimits::default()).expect("extract");
    assert_eq!(
        fs::read_to_string(extracted.path().join("extensions/weather/module.json"))
            .expect("claimed payload"),
        r#"{"unit":"celsius"}"#
    );
}

#[test]
fn creates_verified_space_module_and_rejects_undeclared_space_payload() {
    let root = tempfile::tempdir().expect("root");
    let space_id = uuid::Uuid::now_v7();
    let relative = format!("memory/spaces/{space_id}");
    fs::create_dir_all(root.path().join(&relative)).expect("Space directory");
    fs::write(root.path().join(&relative).join("profile.md"), "memory").expect("Space payload");
    let module = ModuleDefinition {
        id: "memory".to_owned(),
        path: "memory".to_owned(),
        dependencies: vec![],
        import_order: 40,
    };
    let space = SpaceModuleDefinition {
        space_id: space_id.to_string(),
        module: "memory".to_owned(),
        path: relative.clone(),
    };
    let output = root.path().join("space.moc");
    let manifest = create_from_definitions_and_spaces(
        &output,
        root.path(),
        std::slice::from_ref(&module),
        std::slice::from_ref(&space),
    )
    .expect("create Space MOC");
    assert_eq!(manifest.space_modules, [space]);
    assert_eq!(
        manifest.modules[0].space_id.as_deref(),
        Some(space_id.to_string().as_str())
    );

    assert!(matches!(
        create_from_definitions_and_spaces(
            root.path().join("undeclared.moc"),
            root.path(),
            &[module],
            &[],
        ),
        Err(MocError::InvalidManifest(message)) if message.contains("not declared")
    ));
}

#[test]
fn rejects_parent_path() {
    assert!(matches!(
        validate_relative(Path::new("../secret")),
        Err(MocError::UnsafePath(_))
    ));
}

#[test]
fn inspects_encrypted_wrapper_metadata() {
    let root = tempfile::tempdir().expect("root");
    fs::create_dir(root.path().join("private")).expect("private");
    fs::write(root.path().join("private/payload.enc"), "ciphertext").expect("payload");
    let output = root.path().join("private.moc");
    let encryption = EncryptionMetadata {
        profile: "momo-envelope-v1".to_owned(),
        payload_path: "private/payload.enc".to_owned(),
        associated_data: "momo-private-moc-v1".to_owned(),
    };
    create_with_encryption(
        &output,
        root.path(),
        &[("encrypted-container".to_owned(), PathBuf::from("private"))],
        Some(encryption.clone()),
    )
    .expect("create wrapper");
    assert_eq!(
        inspect(output).expect("inspect").encryption,
        Some(encryption)
    );
}

#[test]
fn rejects_duplicate_manifest_paths_that_hide_an_unlisted_payload() {
    let root = tempfile::tempdir().expect("root");
    let output = root.path().join("malicious.moc");
    let declared = b"declared";
    let module = ModuleEntry {
        module: "config".to_owned(),
        space_id: None,
        path: "config/momo.toml".to_owned(),
        size: declared.len() as u64,
        sha256: hex::encode(Sha256::digest(declared)),
    };
    let manifest = Manifest {
        format: FORMAT_NAME.to_owned(),
        format_version: FORMAT_VERSION,
        created_at: Utc::now(),
        module_definitions: vec![ModuleDefinition {
            id: "config".to_owned(),
            path: "config".to_owned(),
            dependencies: Vec::new(),
            import_order: 10,
        }],
        space_modules: Vec::new(),
        modules: vec![module.clone(), module],
        encryption: None,
    };
    write_raw_container(
        &output,
        &manifest,
        &[
            ("config/momo.toml", declared),
            ("private/unlisted.txt", b"must not be accepted"),
        ],
    );

    let destination = tempfile::tempdir().expect("destination");
    assert!(matches!(
        extract(&output, destination.path(), ExtractionLimits::default()),
        Err(MocError::DuplicatePath(path)) if path == "config/momo.toml"
    ));
    assert!(matches!(
        inspect(&output),
        Err(MocError::DuplicatePath(path)) if path == "config/momo.toml"
    ));
}

#[test]
fn v1_and_singular_module_ids_are_rejected() {
    let root = tempfile::tempdir().expect("root");
    let output = root.path().join("legacy.moc");
    let character = b"# Character";
    let legacy = Manifest {
        format: FORMAT_NAME.to_owned(),
        format_version: 1,
        created_at: Utc::now(),
        module_definitions: Vec::new(),
        space_modules: Vec::new(),
        modules: vec![ModuleEntry {
            module: "character".to_owned(),
            space_id: None,
            path: "characters/card/character.md".to_owned(),
            size: character.len() as u64,
            sha256: hex::encode(Sha256::digest(character)),
        }],
        encryption: None,
    };
    write_raw_container(
        &output,
        &legacy,
        &[("characters/card/character.md", character)],
    );

    let rejected = tempfile::tempdir().expect("rejected destination");
    assert!(matches!(
        extract(&output, rejected.path(), ExtractionLimits::default()),
        Err(MocError::UnsupportedFormatVersion { found: 1, .. })
    ));
    assert!(matches!(
        inspect(&output),
        Err(MocError::UnsupportedFormatVersion { found: 1, .. })
    ));

    fs::create_dir_all(root.path().join("characters")).expect("characters directory");
    fs::write(root.path().join("characters/card.md"), character).expect("character payload");
    assert!(matches!(
        create(
            root.path().join("singular.moc"),
            root.path(),
            &[("character".to_owned(), PathBuf::from("characters"))],
        ),
        Err(MocError::InvalidManifest(message))
            if message.contains("legacy module id character")
    ));
}

#[test]
fn removed_incremental_manifest_fields_are_not_silently_accepted() {
    let text = format!(
        "format = \"{FORMAT_NAME}\"\nformat_version = {FORMAT_VERSION}\ncreated_at = \"{}\"\npackage_type = \"incremental\"\nmodules = []\n",
        Utc::now().to_rfc3339()
    );
    assert!(toml::from_str::<Manifest>(&text).is_err());
}

#[test]
fn rejects_higher_versions() {
    let manifest = Manifest {
        format: FORMAT_NAME.to_owned(),
        format_version: FORMAT_VERSION + 1,
        created_at: Utc::now(),
        module_definitions: Vec::new(),
        space_modules: Vec::new(),
        modules: Vec::new(),
        encryption: None,
    };
    assert!(matches!(
        validate_manifest(&manifest),
        Err(MocError::UnsupportedFormatVersion { .. })
    ));
}
