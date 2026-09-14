use super::*;

#[test]
fn compressed_rgba_round_trip_preserves_alpha() {
    let mut pixels = vec![0xaa; 512 * 4];
    for alpha in pixels.iter_mut().skip(3).step_by(4) {
        *alpha = 0x37;
    }
    let original_alpha = pixels
        .iter()
        .skip(3)
        .step_by(4)
        .copied()
        .collect::<Vec<_>>();
    let payload = br#"{"name":"MOMO","description":"portable character"}"#;
    let info = embed_lsb_carrier(&mut pixels, 4, LsbPayloadType::CharacterData, payload, true)
        .expect("embed");
    assert!(info.stored_bytes > 0);
    assert_eq!(
        pixels
            .iter()
            .skip(3)
            .step_by(4)
            .copied()
            .collect::<Vec<_>>(),
        original_alpha
    );
    assert_eq!(
        extract_lsb_carrier(&pixels, 4).expect("extract"),
        LsbPayload {
            payload_type: LsbPayloadType::CharacterData,
            compressed: true,
            bytes: payload.to_vec(),
        }
    );
}

#[test]
fn capacity_failure_does_not_modify_pixels() {
    let mut pixels = vec![0x54; 32 * 3];
    let original = pixels.clone();
    let error = embed_lsb_carrier(&mut pixels, 3, LsbPayloadType::Moc, &[7; 100], false)
        .expect_err("capacity rejection");
    assert!(matches!(
        error,
        LsbCarrierError::InsufficientCapacity { .. }
    ));
    assert_eq!(pixels, original);
}

#[test]
fn decoded_pixels_survive_lossless_container_and_tail_changes() {
    let payload = b"the carrier depends on pixels, not a PNG chunk or file tail";
    let mut decoded_pixels = vec![0x80; 384 * 3];
    embed_lsb_carrier(
        &mut decoded_pixels,
        3,
        LsbPayloadType::CharacterData,
        payload,
        false,
    )
    .expect("embed");
    // A lossless decoder produces the same sample sequence regardless of
    // metadata ordering or whether unrelated tail bytes were stripped.
    let redecoded_pixels = decoded_pixels.clone();
    let raw = read_bytes(&redecoded_pixels, 3, LSB_HEADER_BYTES + payload.len());
    assert_eq!(&raw[LSB_HEADER_BYTES..], payload);
    assert_eq!(
        extract_lsb_carrier(&redecoded_pixels, 3)
            .expect("extract after rewrap")
            .bytes,
        payload
    );
}

#[test]
fn corruption_is_detected() {
    let mut pixels = vec![0; 384 * 3];
    embed_lsb_carrier(&mut pixels, 3, LsbPayloadType::Charx, b"archive", false).expect("embed");
    let payload_bit = LSB_HEADER_BYTES * 8 + 2;
    let pixel_index = rgb_indices(pixels.len(), 3)
        .nth(payload_bit)
        .expect("payload bit");
    pixels[pixel_index] ^= 1;
    assert_eq!(
        extract_lsb_carrier(&pixels, 3),
        Err(LsbCarrierError::ChecksumMismatch)
    );
}

fn png_fixture() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(128, 128, |x, y| {
        image::Rgba([
            (x % 251) as u8,
            (y % 241) as u8,
            ((x + y) % 239) as u8,
            ((x * 3 + y * 5) % 256) as u8,
        ])
    });
    encode_rgba_image(image, LsbImageFormat::Png).expect("fixture PNG")
}

fn webp_fixture() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(128, 128, |x, y| {
        image::Rgba([
            (x % 251) as u8,
            (y % 241) as u8,
            ((x + y) % 239) as u8,
            ((x * 3 + y * 5) % 256) as u8,
        ])
    });
    encode_rgba_image(image, LsbImageFormat::WebpLossless).expect("fixture WebP")
}

#[test]
fn png_tail_stripping_and_lossless_rewrap_preserve_carrier() {
    let original = png_fixture();
    let original_pixels = decode_bounded_image(&original, LsbImageFormat::Png)
        .expect("original PNG")
        .to_rgba8();
    let mut tailed = original.clone();
    tailed.extend_from_slice(b"UNRELATED-TRAILING-ARCHIVE");
    let payload = br#"{"schema":"momo.character/1.0","name":"MOMO"}"#;
    let carrier =
        embed_lsb_png(&tailed, LsbPayloadType::CharacterData, payload, true).expect("embed PNG");
    assert!(!carrier.ends_with(b"UNRELATED-TRAILING-ARCHIVE"));
    assert_eq!(extract_lsb_png(&carrier).expect("extract").bytes, payload);

    let carrier_pixels = decode_bounded_image(&carrier, LsbImageFormat::Png)
        .expect("carrier PNG")
        .to_rgba8();
    assert_eq!(carrier_pixels.dimensions(), original_pixels.dimensions());
    for (before, after) in original_pixels.pixels().zip(carrier_pixels.pixels()) {
        assert_eq!(before.0[3], after.0[3]);
        for channel in 0..3 {
            assert!(before.0[channel].abs_diff(after.0[channel]) <= 1);
        }
    }

    let rewrapped =
        encode_rgba_image(carrier_pixels, LsbImageFormat::Png).expect("lossless rewrap");
    assert_eq!(
        extract_lsb_png(&rewrapped)
            .expect("extract after rewrap")
            .bytes,
        payload
    );
}

#[test]
fn png_dimensions_are_rejected_before_decode() {
    let mut png = png_fixture();
    png[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    png[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(extract_lsb_png(&png), Err(LsbCarrierError::ImageTooLarge));
}

#[test]
fn lossless_webp_round_trip_preserves_carrier() {
    let payload = b"a complete MOC would be one opaque payload here";
    let carrier = embed_lsb_webp(&webp_fixture(), LsbPayloadType::Moc, payload, true)
        .expect("embed lossless WebP");
    let extracted = extract_lsb_webp(&carrier).expect("extract lossless WebP");
    assert_eq!(extracted.payload_type, LsbPayloadType::Moc);
    assert_eq!(extracted.bytes, payload);
}

#[test]
fn rejects_apng_before_decoding_a_frame() {
    let mut png = png_fixture();
    let iend = png
        .windows(4)
        .position(|window| window == b"IEND")
        .expect("IEND")
        - 4;
    let mut animation_control = Vec::new();
    animation_control.extend_from_slice(&8_u32.to_be_bytes());
    animation_control.extend_from_slice(b"acTL");
    animation_control.extend_from_slice(&1_u32.to_be_bytes());
    animation_control.extend_from_slice(&0_u32.to_be_bytes());
    animation_control.extend_from_slice(&0_u32.to_be_bytes());
    png.splice(iend..iend, animation_control);
    assert_eq!(extract_lsb_png(&png), Err(LsbCarrierError::AnimatedImage));
}
