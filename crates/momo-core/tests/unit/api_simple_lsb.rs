use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use crc32fast::Hasher;
use image::ImageEncoder;
use serde_json::{Value, json};

fn ccv2(name: &str) -> Value {
    json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": name,
            "description": "Format fidelity fixture",
            "personality": "Precise",
            "scenario": "Tests",
            "first_mes": "Hello",
            "mes_example": "{{char}}: Hello",
            "creator_notes": "",
            "system_prompt": "",
            "post_history_instructions": "",
            "alternate_greetings": [],
            "character_book": {"entries": []},
            "tags": [],
            "creator": "MOMO",
            "character_version": "1.0.0",
            "extensions": {"fixture": {"keep": true}}
        }
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

fn character_png(card: &Value) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(256, 256, image::Rgba([96, 144, 192, 255]));
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .expect("encode PNG");
    let iend = png
        .windows(4)
        .position(|window| window == b"IEND")
        .expect("IEND")
        - 4;
    let mut text = b"chara\0".to_vec();
    text.extend_from_slice(
        BASE64
            .encode(serde_json::to_vec(card).expect("card JSON"))
            .as_bytes(),
    );
    png.splice(iend..iend, png_chunk(b"tEXt", &text));
    png
}

#[tokio::test]
async fn preserved_sources_can_be_selected_as_both_lsb_carrier_and_payload() {
    let directory = tempfile::tempdir().expect("data directory");
    let core = crate::MomoCore::initialize(directory.path().join("core"))
        .await
        .expect("core");
    let scope_id = crate::momo_domain::new_id();

    let carrier_bytes = character_png(&ccv2("Carrier"));
    let carrier_path = directory.path().join("carrier.png");
    fs::write(&carrier_path, &carrier_bytes).expect("carrier source");
    let carrier = crate::import_external_character(
        &core,
        scope_id,
        &carrier_path,
        crate::ExternalCharacterImportFormat::Ccv2Png,
    )
    .await
    .expect("carrier import");

    let payload_bytes = serde_json::to_vec_pretty(&ccv2("Payload")).expect("payload JSON");
    let payload_path = directory.path().join("payload.json");
    fs::write(&payload_path, &payload_bytes).expect("payload source");
    let payload = crate::import_external_character(
        &core,
        scope_id,
        &payload_path,
        crate::ExternalCharacterImportFormat::Ccv2Json,
    )
    .await
    .expect("payload import");

    let output_path = directory.path().join("output.png");
    let report: Value = serde_json::from_str(
        &embed_lsb_image_json_with_core(
            &core,
            json!({
                "carrier": {
                    "type": "preserved_character_source",
                    "owner_space_id": scope_id,
                    "character_id": carrier.character.id,
                },
                "output_path": output_path,
                "format": "png",
                "payload": {
                    "type": "preserved_character_source",
                    "owner_space_id": scope_id,
                    "character_id": payload.character.id,
                },
                "compress": true,
            })
            .to_string(),
        )
        .await
        .expect("embed preserved source"),
    )
    .expect("report JSON");
    assert_eq!(report["payload_type"], "external_character_json");
    assert_eq!(report["source_format"], "ccv2_json");
    let extracted =
        crate::extract_lsb_png(&fs::read(output_path).expect("LSB output")).expect("extract LSB");
    assert_eq!(
        extracted.payload_type,
        crate::LsbPayloadType::ExternalCharacterJson
    );
    assert_eq!(extracted.bytes, payload_bytes);

    let original_export = directory.path().join("original.png");
    crate::export_preserved_character_source(
        &core,
        scope_id,
        carrier.character.id,
        &original_export,
    )
    .await
    .expect("original source export");
    assert_eq!(
        fs::read(original_export).expect("original bytes"),
        carrier_bytes
    );

    let png_payload_output = directory.path().join("png-payload.png");
    embed_lsb_image_json_with_core(
        &core,
        json!({
            "carrier": {
                "type": "preserved_character_source",
                "owner_space_id": scope_id,
                "character_id": carrier.character.id,
            },
            "output_path": png_payload_output,
            "format": "png",
            "payload": {
                "type": "preserved_character_source",
                "owner_space_id": scope_id,
                "character_id": carrier.character.id,
            },
            "compress": true,
        })
        .to_string(),
    )
    .await
    .expect("embed preserved PNG source");
    let extracted_png =
        crate::extract_lsb_png(&fs::read(png_payload_output).expect("PNG payload output"))
            .expect("extract PNG payload");
    assert_eq!(
        extracted_png.payload_type,
        crate::LsbPayloadType::ExternalCharacterPng
    );
    assert_eq!(extracted_png.bytes, carrier_bytes);
}
