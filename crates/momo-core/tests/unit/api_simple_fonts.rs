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
    assert!(error.message.contains("Unsupported font format"));
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
            .message
            .contains("length")
    );

    bytes[8..12].copy_from_slice(&48_u32.to_be_bytes());
    bytes[16..20].copy_from_slice(&((MAX_IMPORTED_FONT_BYTES + 1) as u32).to_be_bytes());
    assert!(
        normalize_font_bytes_inner(&bytes)
            .unwrap_err()
            .message
            .contains("supported range")
    );
}
