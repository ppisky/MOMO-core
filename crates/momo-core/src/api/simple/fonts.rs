//! Font import validation and web-font normalization.

use super::*;

pub async fn normalize_font_bytes(font_bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || normalize_font_bytes_inner(&font_bytes))
        .await
        .map_err(|error| format!("Font conversion task failed: {error}"))?
}

fn normalize_font_bytes_inner(font_bytes: &[u8]) -> Result<Vec<u8>, String> {
    if font_bytes.len() < 12 {
        return Err("The font file is too short.".to_owned());
    }
    if font_bytes.len() > MAX_IMPORTED_FONT_BYTES {
        return Err("The font file exceeds the 64 MiB import limit.".to_owned());
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
            .map_err(|_| "The WOFF font could not be decoded.".to_owned())?
        }
        _ => {
            return Err(
                "Unsupported font format. Choose a TTF, OTF, WOFF, or WOFF2 file.".to_owned(),
            );
        }
    };

    if normalized.len() < 12 {
        return Err("The decoded font is too short.".to_owned());
    }
    if normalized.len() > MAX_IMPORTED_FONT_BYTES {
        return Err("The decoded font exceeds the 64 MiB import limit.".to_owned());
    }
    if !matches!(&normalized[..4], [0x00, 0x01, 0x00, 0x00] | b"OTTO") {
        return Err("The decoded file is not a supported OpenType font.".to_owned());
    }
    Ok(normalized)
}

fn validate_web_font_header(font_bytes: &[u8]) -> Result<(), String> {
    let minimum_header_size = if &font_bytes[..4] == b"wOF2" { 48 } else { 44 };
    if font_bytes.len() < minimum_header_size {
        return Err("The WOFF header is incomplete.".to_owned());
    }
    let declared_length = u32::from_be_bytes(font_bytes[8..12].try_into().unwrap()) as usize;
    if declared_length != font_bytes.len() {
        return Err("The WOFF file length does not match its header.".to_owned());
    }
    let decoded_length = u32::from_be_bytes(font_bytes[16..20].try_into().unwrap()) as usize;
    if !(12..=MAX_IMPORTED_FONT_BYTES).contains(&decoded_length) {
        return Err("The decoded WOFF size is outside the supported range.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod font_tests {
    use super::*;

    #[test]
    fn plain_sfnt_font_bytes_pass_through() {
        let mut bytes = vec![0_u8; 12];
        bytes[..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        assert_eq!(normalize_font_bytes_inner(&bytes).unwrap(), bytes);
    }

    #[test]
    fn unsupported_font_signature_is_rejected() {
        let error = normalize_font_bytes_inner(b"not-a-font!!").unwrap_err();
        assert!(error.contains("Unsupported font format"));
    }

    #[test]
    fn web_font_header_must_match_file_length_and_size_limit() {
        let mut bytes = vec![0_u8; 48];
        bytes[..4].copy_from_slice(b"wOF2");
        bytes[8..12].copy_from_slice(&47_u32.to_be_bytes());
        bytes[16..20].copy_from_slice(&1024_u32.to_be_bytes());
        assert!(
            normalize_font_bytes_inner(&bytes)
                .unwrap_err()
                .contains("length")
        );

        bytes[8..12].copy_from_slice(&48_u32.to_be_bytes());
        bytes[16..20].copy_from_slice(&((MAX_IMPORTED_FONT_BYTES + 1) as u32).to_be_bytes());
        assert!(
            normalize_font_bytes_inner(&bytes)
                .unwrap_err()
                .contains("supported range")
        );
    }
}
