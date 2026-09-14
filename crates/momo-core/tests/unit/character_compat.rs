use super::*;

fn ccv3() -> Value {
    let mut card = ccv2();
    card["spec"] = json!("chara_card_v3");
    card["spec_version"] = json!("3.0");
    card["data"]["group_only_greetings"] = json!([]);
    card["data"]["assets"] = json!([]);
    card
}

fn ccv1() -> Value {
    json!({
        "name": "Legacy Snowball",
        "description": "Legacy description",
        "personality": "Warm",
        "scenario": "At home",
        "first_mes": "Hello",
        "mes_example": "<BOT>: Hello"
    })
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut hasher = Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    output.extend_from_slice(&hasher.finalize().to_be_bytes());
    output
}

fn embedded_png(keyword: &[u8], card: &Value) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut text = keyword.to_vec();
    text.push(0);
    text.extend_from_slice(
        BASE64
            .encode(serde_json::to_vec(card).expect("card JSON"))
            .as_bytes(),
    );
    png.extend(png_chunk(b"tEXt", &text));
    png.extend(png_chunk(b"IEND", &[]));
    png
}

fn charx_bytes(card: &Value, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(cursor);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    archive
        .start_file("card.json", options)
        .expect("card entry");
    archive
        .write_all(&serde_json::to_vec(card).expect("CCv3 JSON"))
        .expect("card JSON");
    for (name, data) in entries {
        archive.start_file(*name, options).expect("CHARX entry");
        archive.write_all(data).expect("CHARX data");
    }
    archive.finish().expect("finish CHARX").into_inner()
}

fn ccv2() -> Value {
    json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "Snowball",
            "description": "A gentle cat.",
            "personality": "Warm",
            "scenario": "At home",
            "first_mes": "Welcome home.",
            "mes_example": "{{char}}: Hello",
            "creator_notes": "Visible note",
            "system_prompt": "runtime only",
            "post_history_instructions": "",
            "alternate_greetings": [],
            "character_book": {"entries": []},
            "tags": ["cat"],
            "creator": "Creator",
            "character_version": "1.2.3",
            "extensions": {"vendor": {"voice": "one"}}
        }
    })
}

#[test]
fn parses_ccv2_without_injecting_runtime_fields() {
    let parsed =
        parse_external_json(&serde_json::to_vec(&ccv2()).expect("JSON"), None).expect("CCv2");
    assert_eq!(parsed.name, "Snowball");
    assert_eq!(parsed.version, "1.2.3");
    assert!(parsed.character_markdown.contains("A gentle cat."));
    assert!(!parsed.character_markdown.contains("runtime only"));
    assert_eq!(parsed.opening_markdown.as_deref(), Some("Welcome home."));
    assert!(!parsed.warnings.is_empty());
}

#[test]
fn parses_legacy_ccv1_and_exports_v3() {
    let parsed =
        parse_external_json(&serde_json::to_vec(&ccv1()).expect("JSON"), None).expect("CCv1");
    assert_eq!(parsed.format, ExternalCharacterImportFormat::Ccv1Json);
    assert_eq!(parsed.author_name, "Unknown");
    let now = Utc::now();
    let character = CharacterCard {
        id: momo_domain::new_id(),
        scope_id: momo_domain::new_id(),
        name: parsed.name,
        version: parsed.version,
        author_name: parsed.author_name,
        author_url: None,
        character_markdown: parsed.character_markdown,
        user_markdown: String::new(),
        opening_markdown: parsed.opening_markdown,
        created_at: now,
        updated_at: now,
    };
    let exported = export_ccv3(&character, None).expect("CCv3 export");
    assert_eq!(exported["spec"], "chara_card_v3");
    assert_eq!(exported["data"]["name"], "Legacy Snowball");
    assert_eq!(exported["data"]["group_only_greetings"], json!([]));
}

#[test]
fn validates_external_spec_versions_without_prefix_guessing() {
    let mut future_v3 = ccv3();
    future_v3["spec_version"] = json!("3.1");
    let parsed = parse_external_json(
        &serde_json::to_vec(&future_v3).expect("future CCv3 JSON"),
        None,
    )
    .expect("compatible future CCv3 revision");
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.contains("newer than"))
    );

    for invalid in ["3bad", "30", "2.9"] {
        let mut card = ccv3();
        card["spec_version"] = json!(invalid);
        assert!(
            parse_external_json(&serde_json::to_vec(&card).expect("invalid CCv3 JSON"), None,)
                .is_err()
        );
    }

    let mut nonstandard_v2 = ccv2();
    nonstandard_v2["spec_version"] = json!("2.1");
    assert!(
        parse_external_json(
            &serde_json::to_vec(&nonstandard_v2).expect("nonstandard CCv2 JSON"),
            None,
        )
        .is_err()
    );
}

#[tokio::test]
async fn import_and_export_preserve_unknown_source_fields() {
    let directory = tempfile::tempdir().expect("data directory");
    let source = directory.path().join("source.json");
    fs::write(&source, serde_json::to_vec_pretty(&ccv2()).expect("JSON")).expect("source");
    let core = MomoCore::initialize(directory.path().join("core"))
        .await
        .expect("core");
    let scope_id = momo_domain::new_id();
    let imported = import_external_character(
        &core,
        scope_id,
        &source,
        ExternalCharacterImportFormat::Ccv2Json,
    )
    .await
    .expect("import");
    let preserved = directory.path().join("preserved.json");
    let preserved_report =
        export_preserved_character_source(&core, scope_id, imported.character.id, &preserved)
            .await
            .expect("preserved source export");
    assert_eq!(preserved_report.source_format, "ccv2_json");
    assert_eq!(
        fs::read(&preserved).expect("preserved bytes"),
        fs::read(&source).expect("source bytes")
    );
    let output = directory.path().join("output.json");
    let report = export_external_character(
        &core,
        scope_id,
        imported.character.id,
        &output,
        ExternalCharacterExportFormat::Ccv2Json,
    )
    .await
    .expect("export");
    assert_eq!(report["preserved_source_fields"], true);
    let exported: Value =
        serde_json::from_slice(&fs::read(output).expect("output")).expect("exported JSON");
    assert_eq!(exported["data"]["extensions"]["vendor"]["voice"], "one");
    assert_eq!(exported["data"]["system_prompt"], "runtime only");

    let moc = directory.path().join("character.moc");
    crate::export_moc(
        &core,
        &moc,
        &json!({}),
        &crate::MocExportPlan {
            include_config: false,
            characters: vec![crate::MocCharacterSelection {
                space_id: scope_id,
                character_ids: vec![imported.character.id],
            }],
            conversations: vec![],
            memory: vec![],
            semantic_graph: vec![],
            compatibility: crate::MocCompatibility::PreservedSource,
        },
    )
    .await
    .expect("MOC export");
    let destination = MomoCore::initialize(directory.path().join("destination"))
        .await
        .expect("destination");
    let destination_scope = momo_domain::new_id();
    crate::import_moc(
        &destination,
        &moc,
        &crate::MocImportPlan {
            apply_config: false,
            space_map: [(scope_id, destination_scope)].into_iter().collect(),
            conflict_mode: crate::ConflictMode::Replace,
        },
    )
    .await
    .expect("MOC import");
    let round_trip = directory.path().join("round-trip.json");
    let report = export_external_character(
        &destination,
        destination_scope,
        imported.character.id,
        &round_trip,
        ExternalCharacterExportFormat::Ccv2Json,
    )
    .await
    .expect("round-trip export");
    assert_eq!(report["preserved_source_fields"], true);
    let round_trip: Value =
        serde_json::from_slice(&fs::read(round_trip).expect("round-trip output"))
            .expect("round-trip JSON");
    assert_eq!(round_trip["data"]["extensions"]["vendor"]["voice"], "one");
}

#[test]
fn rejects_mismatched_png_chunk_and_spec() {
    let error = parse_external_json(
        &serde_json::to_vec(&ccv2()).expect("JSON"),
        Some(ExternalCharacterImportFormat::Ccv3Png),
    )
    .expect_err("mismatched format");
    assert!(error.to_string().contains("does not match"));
}

#[test]
fn extracts_ccv3_png_and_validates_crc() {
    let png = embedded_png(b"ccv3", &ccv3());
    let (json, chunk) = extract_png_card(&png).expect("embedded CCv3");
    assert!(matches!(chunk, PngCardChunk::Ccv3));
    let parsed =
        parse_external_json(&json, Some(ExternalCharacterImportFormat::Ccv3Png)).expect("CCv3 PNG");
    assert_eq!(parsed.name, "Snowball");

    let mut corrupted = png;
    let last = corrupted.len() - 1;
    corrupted[last] ^= 1;
    assert!(extract_png_card(&corrupted).is_err());
}

#[test]
fn rejects_apng_instead_of_selecting_a_frame() {
    let mut png = embedded_png(b"ccv3", &ccv3());
    let iend = png
        .windows(4)
        .position(|window| window == b"IEND")
        .expect("IEND")
        - 4;
    png.splice(iend..iend, png_chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
    let error = extract_png_card(&png).expect_err("APNG must be rejected");
    assert!(error.to_string().contains("APNG"));
}

#[test]
fn reads_charx_card_and_validates_container() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("card.charx");
    fs::write(&path, charx_bytes(&ccv3(), &[])).expect("CHARX output");

    let parsed =
        parse_external_path(&path, ExternalCharacterImportFormat::Ccv3Charx).expect("CHARX import");
    assert_eq!(parsed.format, ExternalCharacterImportFormat::Ccv3Charx);
    assert_eq!(parsed.name, "Snowball");
    let package = parsed.charx.expect("CHARX package");
    assert_eq!(package.info.entry_count, 1);
    assert_eq!(package.info.asset_count, 0);
}

#[test]
fn reads_jpeg_zip_hybrid_and_rejects_unsafe_entries() {
    let zip = charx_bytes(&ccv3(), &[]);
    let mut hybrid = b"\xFF\xD8\xFFpreview\xFF\xD9".to_vec();
    hybrid.extend_from_slice(&zip);
    let parsed = parse_charx_package(hybrid).expect("JPEG+ZIP CHARX");
    assert!(parsed.info.jpeg_zip_hybrid);

    let unsafe_archive = charx_bytes(&ccv3(), &[("../outside.txt", b"bad")]);
    assert!(parse_charx_package(unsafe_archive).is_err());
}

#[tokio::test]
async fn charx_assets_and_risu_extensions_round_trip_through_moc() {
    let directory = tempfile::tempdir().expect("data directory");
    let mut card = ccv3();
    card["data"]["assets"] = json!([{
        "type": "icon",
        "uri": "embeded://assets/icon/images/avatar.png",
        "name": "main",
        "ext": "png"
    }]);
    let input = directory.path().join("source.charx");
    fs::write(
        &input,
        charx_bytes(
            &card,
            &[
                ("assets/icon/images/avatar.png", b"image-bytes"),
                ("x_meta/0.json", br#"{"type":"PNG"}"#),
                ("module.risum", b"opaque-module"),
                ("app.json", br#"{"future":true}"#),
            ],
        ),
    )
    .expect("source CHARX");
    let core = MomoCore::initialize(directory.path().join("core"))
        .await
        .expect("core");
    let scope_id = momo_domain::new_id();
    let imported = import_external_character(
        &core,
        scope_id,
        &input,
        ExternalCharacterImportFormat::Ccv3Charx,
    )
    .await
    .expect("CHARX import");
    assert!(
        imported
            .warnings
            .iter()
            .all(|warning| !warning.contains("missing CHARX entry"))
    );
    let mut updated = imported.character.clone();
    updated.name = "Snowball 0.4".to_owned();
    updated.updated_at = Utc::now();
    core.store()
        .stage_character_update(&updated)
        .await
        .expect("character update");

    let output = directory.path().join("output.charx");
    let report = export_external_character(
        &core,
        scope_id,
        imported.character.id,
        &output,
        ExternalCharacterExportFormat::Ccv3Charx,
    )
    .await
    .expect("CHARX export");
    assert_eq!(report["preserved_source_assets"], true);
    assert_charx_entry(&output, "assets/icon/images/avatar.png", b"image-bytes");
    assert_charx_entry(&output, "module.risum", b"opaque-module");
    assert_charx_entry(&output, "app.json", br#"{"future":true}"#);
    assert_eq!(read_charx_card(&output)["data"]["name"], "Snowball 0.4");

    let moc = directory.path().join("character.moc");
    crate::export_moc(
        &core,
        &moc,
        &json!({}),
        &crate::MocExportPlan {
            include_config: false,
            characters: vec![crate::MocCharacterSelection {
                space_id: scope_id,
                character_ids: vec![imported.character.id],
            }],
            conversations: vec![],
            memory: vec![],
            semantic_graph: vec![],
            compatibility: crate::MocCompatibility::PreservedSource,
        },
    )
    .await
    .expect("MOC export");
    let destination = MomoCore::initialize(directory.path().join("destination"))
        .await
        .expect("destination");
    let destination_scope = momo_domain::new_id();
    crate::import_moc(
        &destination,
        &moc,
        &crate::MocImportPlan {
            apply_config: false,
            space_map: [(scope_id, destination_scope)].into_iter().collect(),
            conflict_mode: crate::ConflictMode::Replace,
        },
    )
    .await
    .expect("MOC import");
    let round_trip = directory.path().join("round-trip.charx");
    export_external_character(
        &destination,
        destination_scope,
        imported.character.id,
        &round_trip,
        ExternalCharacterExportFormat::Ccv3Charx,
    )
    .await
    .expect("round-trip CHARX export");
    assert_charx_entry(&round_trip, "assets/icon/images/avatar.png", b"image-bytes");
    assert_charx_entry(&round_trip, "x_meta/0.json", br#"{"type":"PNG"}"#);
    assert_eq!(read_charx_card(&round_trip)["data"]["name"], "Snowball 0.4");
}

#[tokio::test]
async fn native_character_exports_card_only_charx() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = MomoCore::initialize(directory.path().join("core"))
        .await
        .expect("core");
    let scope_id = momo_domain::new_id();
    let now = Utc::now();
    let character = CharacterCard {
        id: momo_domain::new_id(),
        scope_id,
        name: "Native".to_owned(),
        version: "1.0.0".to_owned(),
        author_name: "MOMO".to_owned(),
        author_url: None,
        character_markdown: "# Native".to_owned(),
        user_markdown: String::new(),
        opening_markdown: Some("Hello".to_owned()),
        created_at: now,
        updated_at: now,
    };
    core.store()
        .stage_character(&character)
        .await
        .expect("character");
    let output = directory.path().join("native.charx");
    let report = export_external_character(
        &core,
        scope_id,
        character.id,
        &output,
        ExternalCharacterExportFormat::Ccv3Charx,
    )
    .await
    .expect("CHARX export");
    assert_eq!(report["preserved_source_assets"], false);
    assert_eq!(read_charx_card(&output)["data"]["name"], "Native");
    let archive =
        zip::ZipArchive::new(fs::File::open(output).expect("CHARX file")).expect("CHARX archive");
    assert_eq!(archive.len(), 1);
}

fn assert_charx_entry(path: &Path, name: &str, expected: &[u8]) {
    let file = fs::File::open(path).expect("CHARX file");
    let mut archive = zip::ZipArchive::new(file).expect("CHARX archive");
    let mut entry = archive.by_name(name).expect("CHARX entry");
    let mut actual = Vec::new();
    entry.read_to_end(&mut actual).expect("entry data");
    assert_eq!(actual, expected);
}

fn read_charx_card(path: &Path) -> Value {
    let file = fs::File::open(path).expect("CHARX file");
    let mut archive = zip::ZipArchive::new(file).expect("CHARX archive");
    let mut entry = archive.by_name("card.json").expect("card.json");
    let mut data = Vec::new();
    entry.read_to_end(&mut data).expect("card JSON");
    serde_json::from_slice(&data).expect("CCv3 card")
}

#[test]
fn rejects_non_object_and_unknown_specs() {
    assert!(parse_external_json(b"[]", None).is_err());
    assert!(
        parse_external_json(
            br#"{"spec":"future_card","spec_version":"9.0","data":{}}"#,
            None
        )
        .is_err()
    );
}
