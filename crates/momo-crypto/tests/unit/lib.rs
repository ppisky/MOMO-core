use super::*;

fn fast_parameters() -> KdfParameters {
    KdfParameters::FAST
}

#[test]
fn adaptive_kdf_parameters_follow_hardware_budget() {
    assert_eq!(
        KdfParameters::for_hardware(2 * 1024 * 1024, 4),
        KdfParameters::FAST
    );
    assert_eq!(
        KdfParameters::for_hardware(8 * 1024 * 1024, 4),
        KdfParameters::STANDARD
    );
    assert_eq!(
        KdfParameters::for_hardware(32 * 1024 * 1024, 16),
        KdfParameters::HARD
    );
    KdfParameters::adaptive_default()
        .validate()
        .expect("detected profile is valid");
}

#[test]
fn round_trips_binary_payload() {
    let plaintext = b"MOMO\0private\xffpayload";
    let envelope =
        encrypt(plaintext, "correct horse", b"backup.moc", fast_parameters()).expect("encrypt");
    let encoded = encode(&envelope).expect("encode");
    let decoded = decode_envelope(&encoded).expect("decode");
    assert_eq!(
        decrypt(&decoded, "correct horse", b"backup.moc").expect("decrypt"),
        plaintext
    );
}

#[test]
fn rejects_wrong_password_and_associated_data() {
    let envelope = encrypt(b"secret", "correct", b"object-1", fast_parameters()).expect("encrypt");
    assert!(matches!(
        decrypt(&envelope, "wrong", b"object-1"),
        Err(CryptoError::AuthenticationFailed)
    ));
    assert!(matches!(
        decrypt(&envelope, "correct", b"object-2"),
        Err(CryptoError::AuthenticationFailed)
    ));
}

#[test]
fn rejects_tampering() {
    let mut envelope = encrypt(b"secret", "password", b"", fast_parameters()).expect("encrypt");
    let mut ciphertext = decode(&envelope.ciphertext).expect("ciphertext");
    ciphertext[0] ^= 0x80;
    envelope.ciphertext = STANDARD_NO_PAD.encode(ciphertext);
    assert!(matches!(
        decrypt(&envelope, "password", b""),
        Err(CryptoError::AuthenticationFailed)
    ));
}

#[test]
fn never_reuses_salt_or_nonces() {
    let first = encrypt(b"same", "password", b"", fast_parameters()).expect("first");
    let second = encrypt(b"same", "password", b"", fast_parameters()).expect("second");
    assert_ne!(first.salt, second.salt);
    assert_ne!(first.wrapped_key_nonce, second.wrapped_key_nonce);
    assert_ne!(first.payload_nonce, second.payload_nonce);
    assert_ne!(first.ciphertext, second.ciphertext);
}

#[test]
fn bounds_untrusted_kdf_parameters() {
    let mut envelope = encrypt(b"x", "password", b"", fast_parameters()).expect("encrypt");
    envelope.kdf_parameters.memory_kib = u32::MAX;
    assert!(matches!(
        decrypt(&envelope, "password", b""),
        Err(CryptoError::InvalidParameters)
    ));
}
