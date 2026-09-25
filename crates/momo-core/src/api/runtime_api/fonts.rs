//! Font import validation and web-font normalization.

use super::*;

pub async fn normalize_font_bytes(font_bytes: Vec<u8>) -> Result<Vec<u8>, RuntimeApiError> {
    tokio::task::spawn_blocking(move || normalize_font_bytes_inner(&font_bytes))
        .await
        .map_err(|error| {
            RuntimeApiError::internal(format!("Font conversion task failed: {error}"))
        })?
}

fn normalize_font_bytes_inner(font_bytes: &[u8]) -> Result<Vec<u8>, RuntimeApiError> {
    if font_bytes.len() < 12 {
        return Err(RuntimeApiError::invalid("The font file is too short."));
    }
    if font_bytes.len() > MAX_IMPORTED_FONT_BYTES {
        return Err(RuntimeApiError::invalid(
            "The font file exceeds the 64 MiB import limit.",
        ));
    }

    let signature = &font_bytes[..4];
    let normalized = match signature {
        [0x00, 0x01, 0x00, 0x00] | b"OTTO" => font_bytes.to_vec(),
        b"wOFF" | b"wOF2" => {
            validate_web_font_header(font_bytes)?;
            if signature == b"wOFF" {
                wuff::decompress_woff1(font_bytes)
            } else {
                wuff::decompress_woff2(font_bytes)
            }
            .map_err(|_| RuntimeApiError::invalid("The WOFF font could not be decoded."))?
        }
        _ => {
            return Err(RuntimeApiError::invalid(
                "Unsupported font format. Choose a TTF, OTF, WOFF, or WOFF2 file.",
            ));
        }
    };

    if normalized.len() < 12 {
        return Err(RuntimeApiError::invalid("The decoded font is too short."));
    }
    if normalized.len() > MAX_IMPORTED_FONT_BYTES {
        return Err(RuntimeApiError::invalid(
            "The decoded font exceeds the 64 MiB import limit.",
        ));
    }
    if !matches!(&normalized[..4], [0x00, 0x01, 0x00, 0x00] | b"OTTO") {
        return Err(RuntimeApiError::invalid(
            "The decoded file is not a supported OpenType font.",
        ));
    }
    Ok(normalized)
}

fn validate_web_font_header(font_bytes: &[u8]) -> Result<(), RuntimeApiError> {
    let minimum_header_size = if &font_bytes[..4] == b"wOF2" { 48 } else { 44 };
    if font_bytes.len() < minimum_header_size {
        return Err(RuntimeApiError::invalid("The WOFF header is incomplete."));
    }
    let declared_length = u32::from_be_bytes(font_bytes[8..12].try_into().unwrap()) as usize;
    if declared_length != font_bytes.len() {
        return Err(RuntimeApiError::invalid(
            "The WOFF file length does not match its header.",
        ));
    }
    let decoded_length = u32::from_be_bytes(font_bytes[16..20].try_into().unwrap()) as usize;
    if !(12..=MAX_IMPORTED_FONT_BYTES).contains(&decoded_length) {
        return Err(RuntimeApiError::invalid(
            "The decoded WOFF size is outside the supported range.",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/api_simple_fonts.rs"]
mod font_tests;
