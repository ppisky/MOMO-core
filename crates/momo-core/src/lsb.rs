//! Codec-independent MOMO LSB Carrier v1 support.
//!
//! Callers decode an image to interleaved RGB or RGBA pixels, use this module,
//! then encode those exact pixels with a lossless image codec. Container
//! metadata and bytes after an image terminator are deliberately irrelevant.

use std::io::Cursor;

use crc32fast::hash;
use image::{DynamicImage, ImageEncoder, ImageFormat};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const LSB_CARRIER_MAGIC: &[u8; 8] = b"MOMOLSB1";
pub const LSB_CARRIER_VERSION: u8 = 1;
pub const MOMO_LSB_CHARACTER_SCHEMA: &str = "momo.character/1.0";
pub const LSB_HEADER_BYTES: usize = 24;
pub const MAX_LSB_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_LSB_IMAGE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_LSB_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;

const FLAG_ZSTD: u8 = 1;
const KNOWN_FLAGS: u8 = FLAG_ZSTD;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum LsbPayloadType {
    CharacterData = 1,
    Moc = 2,
    Charx = 3,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LsbImageFormat {
    Png,
    WebpLossless,
}

impl TryFrom<u8> for LsbPayloadType {
    type Error = LsbCarrierError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::CharacterData),
            2 => Ok(Self::Moc),
            3 => Ok(Self::Charx),
            _ => Err(LsbCarrierError::UnknownPayloadType(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsbPayload {
    pub payload_type: LsbPayloadType,
    pub compressed: bool,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LsbCarrierInfo {
    pub payload_type: LsbPayloadType,
    pub compressed: bool,
    pub stored_bytes: usize,
    pub original_bytes: usize,
    pub available_bytes: usize,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LsbCarrierError {
    #[error("pixel channel count must be 3 (RGB) or 4 (RGBA)")]
    InvalidChannels,
    #[error("pixel buffer length is not divisible by the channel count")]
    InvalidPixelBuffer,
    #[error("payload must not be empty")]
    EmptyPayload,
    #[error("payload exceeds the {MAX_LSB_PAYLOAD_BYTES} byte limit")]
    PayloadTooLarge,
    #[error("carrier capacity is {available} bytes but {required} bytes are required")]
    InsufficientCapacity { required: usize, available: usize },
    #[error("image does not contain a MOMO LSB Carrier v1 header")]
    MissingCarrier,
    #[error("unsupported LSB carrier version {0}")]
    UnsupportedVersion(u8),
    #[error("unsupported LSB carrier flags 0x{0:02x}")]
    UnknownFlags(u8),
    #[error("unknown LSB payload type {0}")]
    UnknownPayloadType(u8),
    #[error("LSB payload length is invalid")]
    InvalidLength,
    #[error("LSB payload integrity check failed")]
    ChecksumMismatch,
    #[error("LSB payload compression failed: {0}")]
    Compression(String),
    #[error("LSB payload decompression failed: {0}")]
    Decompression(String),
    #[error("image carrier exceeds the image size limit")]
    ImageTooLarge,
    #[error("animated image carriers are not supported")]
    AnimatedImage,
    #[error("invalid lossless image carrier: {0}")]
    InvalidImage(String),
}

#[must_use]
pub fn lsb_capacity(pixels: &[u8], channels: u8) -> usize {
    if !matches!(channels, 3 | 4) || !pixels.len().is_multiple_of(usize::from(channels)) {
        return 0;
    }
    let rgb_samples = pixels.len() / usize::from(channels) * 3;
    rgb_samples / 8
}

pub fn embed_lsb_carrier(
    pixels: &mut [u8],
    channels: u8,
    payload_type: LsbPayloadType,
    payload: &[u8],
    compress: bool,
) -> Result<LsbCarrierInfo, LsbCarrierError> {
    validate_pixels(pixels, channels)?;
    if payload.is_empty() {
        return Err(LsbCarrierError::EmptyPayload);
    }
    if payload.len() > MAX_LSB_PAYLOAD_BYTES {
        return Err(LsbCarrierError::PayloadTooLarge);
    }
    let stored = if compress {
        zstd::bulk::compress(payload, 9)
            .map_err(|error| LsbCarrierError::Compression(error.to_string()))?
    } else {
        payload.to_vec()
    };
    let stored_len = u32::try_from(stored.len()).map_err(|_| LsbCarrierError::PayloadTooLarge)?;
    let original_len =
        u32::try_from(payload.len()).map_err(|_| LsbCarrierError::PayloadTooLarge)?;
    let mut carrier = Vec::with_capacity(LSB_HEADER_BYTES + stored.len());
    carrier.extend_from_slice(LSB_CARRIER_MAGIC);
    carrier.push(LSB_CARRIER_VERSION);
    carrier.push(if compress { FLAG_ZSTD } else { 0 });
    carrier.push(payload_type as u8);
    carrier.push(0);
    carrier.extend_from_slice(&stored_len.to_be_bytes());
    carrier.extend_from_slice(&original_len.to_be_bytes());
    carrier.extend_from_slice(&hash(payload).to_be_bytes());
    carrier.extend_from_slice(&stored);

    let available = lsb_capacity(pixels, channels);
    if carrier.len() > available {
        return Err(LsbCarrierError::InsufficientCapacity {
            required: carrier.len(),
            available,
        });
    }
    write_bits(pixels, channels, &carrier);
    Ok(LsbCarrierInfo {
        payload_type,
        compressed: compress,
        stored_bytes: stored.len(),
        original_bytes: payload.len(),
        available_bytes: available,
    })
}

pub fn extract_lsb_carrier(pixels: &[u8], channels: u8) -> Result<LsbPayload, LsbCarrierError> {
    validate_pixels(pixels, channels)?;
    if lsb_capacity(pixels, channels) < LSB_HEADER_BYTES {
        return Err(LsbCarrierError::MissingCarrier);
    }
    let header = read_bytes(pixels, channels, LSB_HEADER_BYTES);
    if &header[..8] != LSB_CARRIER_MAGIC {
        return Err(LsbCarrierError::MissingCarrier);
    }
    if header[8] != LSB_CARRIER_VERSION {
        return Err(LsbCarrierError::UnsupportedVersion(header[8]));
    }
    let flags = header[9];
    if flags & !KNOWN_FLAGS != 0 {
        return Err(LsbCarrierError::UnknownFlags(flags));
    }
    let payload_type = LsbPayloadType::try_from(header[10])?;
    let stored_len = u32::from_be_bytes(header[12..16].try_into().expect("fixed header")) as usize;
    let original_len =
        u32::from_be_bytes(header[16..20].try_into().expect("fixed header")) as usize;
    let checksum = u32::from_be_bytes(header[20..24].try_into().expect("fixed header"));
    if stored_len == 0
        || original_len == 0
        || original_len > MAX_LSB_PAYLOAD_BYTES
        || stored_len > MAX_LSB_PAYLOAD_BYTES
        || LSB_HEADER_BYTES.saturating_add(stored_len) > lsb_capacity(pixels, channels)
    {
        return Err(LsbCarrierError::InvalidLength);
    }
    let carrier = read_bytes(pixels, channels, LSB_HEADER_BYTES + stored_len);
    let stored = &carrier[LSB_HEADER_BYTES..];
    let compressed = flags & FLAG_ZSTD != 0;
    let bytes = if compressed {
        zstd::bulk::decompress(stored, original_len)
            .map_err(|error| LsbCarrierError::Decompression(error.to_string()))?
    } else {
        if stored_len != original_len {
            return Err(LsbCarrierError::InvalidLength);
        }
        stored.to_vec()
    };
    if bytes.len() != original_len || hash(&bytes) != checksum {
        return Err(LsbCarrierError::ChecksumMismatch);
    }
    Ok(LsbPayload {
        payload_type,
        compressed,
        bytes,
    })
}

pub fn embed_lsb_png(
    png: &[u8],
    payload_type: LsbPayloadType,
    payload: &[u8],
    compress: bool,
) -> Result<Vec<u8>, LsbCarrierError> {
    embed_lsb_image(png, LsbImageFormat::Png, payload_type, payload, compress)
}

pub fn extract_lsb_png(png: &[u8]) -> Result<LsbPayload, LsbCarrierError> {
    extract_lsb_image(png, LsbImageFormat::Png)
}

pub fn embed_lsb_webp(
    webp: &[u8],
    payload_type: LsbPayloadType,
    payload: &[u8],
    compress: bool,
) -> Result<Vec<u8>, LsbCarrierError> {
    embed_lsb_image(
        webp,
        LsbImageFormat::WebpLossless,
        payload_type,
        payload,
        compress,
    )
}

pub fn extract_lsb_webp(webp: &[u8]) -> Result<LsbPayload, LsbCarrierError> {
    extract_lsb_image(webp, LsbImageFormat::WebpLossless)
}

pub fn embed_lsb_image(
    image_bytes: &[u8],
    format: LsbImageFormat,
    payload_type: LsbPayloadType,
    payload: &[u8],
    compress: bool,
) -> Result<Vec<u8>, LsbCarrierError> {
    let image = decode_bounded_image(image_bytes, format)?;
    let mut rgba = image.to_rgba8();
    embed_lsb_carrier(rgba.as_mut(), 4, payload_type, payload, compress)?;
    encode_rgba_image(rgba, format)
}

pub fn extract_lsb_image(
    image_bytes: &[u8],
    format: LsbImageFormat,
) -> Result<LsbPayload, LsbCarrierError> {
    let image = decode_bounded_image(image_bytes, format)?;
    let rgba = image.to_rgba8();
    extract_lsb_carrier(rgba.as_raw(), 4)
}

fn decode_bounded_image(
    bytes: &[u8],
    format: LsbImageFormat,
) -> Result<DynamicImage, LsbCarrierError> {
    if bytes.len() > MAX_LSB_IMAGE_BYTES {
        return Err(LsbCarrierError::ImageTooLarge);
    }
    reject_animation(bytes, format)?;
    let image_format = match format {
        LsbImageFormat::Png => ImageFormat::Png,
        LsbImageFormat::WebpLossless => ImageFormat::WebP,
    };
    let (width, height) = match format {
        LsbImageFormat::Png => png_dimensions(bytes)?,
        LsbImageFormat::WebpLossless => {
            image::ImageReader::with_format(Cursor::new(bytes), image_format)
                .into_dimensions()
                .map_err(|error| LsbCarrierError::InvalidImage(error.to_string()))?
        }
    };
    if width == 0
        || height == 0
        || u64::from(width).saturating_mul(u64::from(height)) > MAX_LSB_IMAGE_PIXELS
    {
        return Err(LsbCarrierError::ImageTooLarge);
    }
    image::load_from_memory_with_format(bytes, image_format)
        .map_err(|error| LsbCarrierError::InvalidImage(error.to_string()))
}

fn reject_animation(bytes: &[u8], format: LsbImageFormat) -> Result<(), LsbCarrierError> {
    match format {
        LsbImageFormat::Png if png_contains_chunk(bytes, b"acTL")? => {
            Err(LsbCarrierError::AnimatedImage)
        }
        LsbImageFormat::WebpLossless if webp_is_animated(bytes)? => {
            Err(LsbCarrierError::AnimatedImage)
        }
        _ => Ok(()),
    }
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), LsbCarrierError> {
    if bytes.len() < 24
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[12..16] != b"IHDR"
        || u32::from_be_bytes(bytes[8..12].try_into().expect("checked PNG header")) != 13
    {
        return Err(LsbCarrierError::InvalidImage(
            "missing canonical PNG signature and IHDR".to_owned(),
        ));
    }
    Ok((
        u32::from_be_bytes(bytes[16..20].try_into().expect("checked PNG header")),
        u32::from_be_bytes(bytes[20..24].try_into().expect("checked PNG header")),
    ))
}

fn png_contains_chunk(bytes: &[u8], wanted: &[u8; 4]) -> Result<bool, LsbCarrierError> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(LsbCarrierError::InvalidImage(
            "invalid PNG signature".to_owned(),
        ));
    }
    let mut cursor = 8_usize;
    while cursor < bytes.len() {
        if bytes.len().saturating_sub(cursor) < 12 {
            return Err(LsbCarrierError::InvalidImage(
                "truncated PNG chunk".to_owned(),
            ));
        }
        let length = u32::from_be_bytes(
            bytes[cursor..cursor + 4]
                .try_into()
                .expect("four-byte PNG length"),
        ) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(|| LsbCarrierError::InvalidImage("PNG chunk overflow".to_owned()))?;
        if end > bytes.len() {
            return Err(LsbCarrierError::InvalidImage(
                "truncated PNG chunk".to_owned(),
            ));
        }
        let kind = &bytes[cursor + 4..cursor + 8];
        if kind == wanted {
            return Ok(true);
        }
        cursor = end;
        if kind == b"IEND" {
            return Ok(false);
        }
    }
    Err(LsbCarrierError::InvalidImage(
        "PNG has no IEND chunk".to_owned(),
    ))
}

fn webp_is_animated(bytes: &[u8]) -> Result<bool, LsbCarrierError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(LsbCarrierError::InvalidImage(
            "invalid WebP signature".to_owned(),
        ));
    }
    let mut cursor = 12_usize;
    while cursor < bytes.len() {
        if bytes.len().saturating_sub(cursor) < 8 {
            return Err(LsbCarrierError::InvalidImage(
                "truncated WebP chunk".to_owned(),
            ));
        }
        let kind = &bytes[cursor..cursor + 4];
        let length = u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8]
                .try_into()
                .expect("four-byte WebP length"),
        ) as usize;
        let data_start = cursor + 8;
        let end = data_start
            .checked_add(length)
            .ok_or_else(|| LsbCarrierError::InvalidImage("WebP chunk overflow".to_owned()))?;
        if end > bytes.len() {
            return Err(LsbCarrierError::InvalidImage(
                "truncated WebP chunk".to_owned(),
            ));
        }
        if kind == b"ANIM" || kind == b"ANMF" {
            return Ok(true);
        }
        if kind == b"VP8X" && length >= 1 && bytes[data_start] & 0x02 != 0 {
            return Ok(true);
        }
        cursor = end + (length & 1);
    }
    Ok(false)
}

fn encode_rgba_image(
    image: image::RgbaImage,
    format: LsbImageFormat,
) -> Result<Vec<u8>, LsbCarrierError> {
    let mut output = Cursor::new(Vec::new());
    match format {
        LsbImageFormat::Png => DynamicImage::ImageRgba8(image)
            .write_to(&mut output, ImageFormat::Png)
            .map_err(|error| LsbCarrierError::InvalidImage(error.to_string()))?,
        LsbImageFormat::WebpLossless => {
            let (width, height) = image.dimensions();
            image::codecs::webp::WebPEncoder::new_lossless(&mut output)
                .write_image(
                    image.as_raw(),
                    width,
                    height,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|error| LsbCarrierError::InvalidImage(error.to_string()))?;
        }
    }
    Ok(output.into_inner())
}

fn validate_pixels(pixels: &[u8], channels: u8) -> Result<(), LsbCarrierError> {
    if !matches!(channels, 3 | 4) {
        return Err(LsbCarrierError::InvalidChannels);
    }
    if !pixels.len().is_multiple_of(usize::from(channels)) {
        return Err(LsbCarrierError::InvalidPixelBuffer);
    }
    Ok(())
}

fn rgb_indices(len: usize, channels: u8) -> impl Iterator<Item = usize> {
    let channels = usize::from(channels);
    (0..len).filter(move |index| index % channels < 3)
}

fn write_bits(pixels: &mut [u8], channels: u8, bytes: &[u8]) {
    for (index, bit) in rgb_indices(pixels.len(), channels).zip(
        bytes
            .iter()
            .flat_map(|byte| (0..8).rev().map(move |shift| (byte >> shift) & 1)),
    ) {
        pixels[index] = (pixels[index] & 0xfe) | bit;
    }
}

fn read_bytes(pixels: &[u8], channels: u8, count: usize) -> Vec<u8> {
    let mut output = vec![0_u8; count];
    for (bit_index, pixel_index) in rgb_indices(pixels.len(), channels)
        .take(count.saturating_mul(8))
        .enumerate()
    {
        output[bit_index / 8] |= (pixels[pixel_index] & 1) << (7 - bit_index % 8);
    }
    output
}

#[cfg(test)]
mod tests {
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
        let carrier = embed_lsb_png(&tailed, LsbPayloadType::CharacterData, payload, true)
            .expect("embed PNG");
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
}
