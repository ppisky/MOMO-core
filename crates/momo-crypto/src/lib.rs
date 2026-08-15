//! Versioned envelope encryption for private MOMO objects and containers.

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sysinfo::System;
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

pub const ENVELOPE_FORMAT: &str = "momo-encrypted-envelope";
pub const ENVELOPE_VERSION: u32 = 1;
pub const KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 12;
pub const SALT_BYTES: usize = 16;
const WRAP_AAD: &[u8] = b"momo-envelope-v1-dek";
const PAYLOAD_AAD_PREFIX: &[u8] = b"momo-envelope-v1-payload\0";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfParameters {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParameters {
    pub const FAST: Self = Self {
        memory_kib: 32 * 1024,
        iterations: 2,
        parallelism: 1,
    };

    pub const STANDARD: Self = Self {
        memory_kib: 64 * 1024,
        iterations: 3,
        parallelism: 1,
    };

    pub const HARD: Self = Self {
        memory_kib: 256 * 1024,
        iterations: 4,
        parallelism: 1,
    };

    pub const DESKTOP_PROTOTYPE: Self = Self::STANDARD;

    #[must_use]
    pub fn adaptive_default() -> Self {
        let mut system = System::new();
        system.refresh_memory();
        let total_memory_kib = system.total_memory() / 1024;
        let logical_parallelism = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        Self::for_hardware(total_memory_kib, logical_parallelism)
    }

    #[must_use]
    pub const fn for_hardware(total_memory_kib: u64, logical_parallelism: usize) -> Self {
        if total_memory_kib >= 16 * 1024 * 1024 && logical_parallelism >= 8 {
            Self::HARD
        } else if total_memory_kib >= 4 * 1024 * 1024 && logical_parallelism >= 2 {
            Self::STANDARD
        } else {
            Self::FAST
        }
    }

    fn validate(self) -> Result<(), CryptoError> {
        if !(8 * 1024..=1024 * 1024).contains(&self.memory_kib)
            || !(1..=10).contains(&self.iterations)
            || !(1..=8).contains(&self.parallelism)
        {
            return Err(CryptoError::InvalidParameters);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedEnvelope {
    pub format: String,
    pub format_version: u32,
    pub cipher: String,
    pub kdf: String,
    pub kdf_parameters: KdfParameters,
    pub salt: String,
    pub wrapped_key_nonce: String,
    pub wrapped_key: String,
    pub payload_nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("unsupported encrypted envelope version")]
    UnsupportedVersion,
    #[error("invalid or unsafe encryption parameters")]
    InvalidParameters,
    #[error("invalid encrypted envelope encoding")]
    InvalidEncoding,
    #[error("encrypted envelope authentication failed")]
    AuthenticationFailed,
    #[error("encrypted envelope serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub fn encrypt(
    plaintext: &[u8],
    passphrase: &str,
    associated_data: &[u8],
    parameters: KdfParameters,
) -> Result<EncryptedEnvelope, CryptoError> {
    parameters.validate()?;
    let mut salt = [0_u8; SALT_BYTES];
    let mut wrapped_key_nonce = [0_u8; NONCE_BYTES];
    let mut payload_nonce = [0_u8; NONCE_BYTES];
    let mut dek = Zeroizing::new([0_u8; KEY_BYTES]);
    let mut rng = rand::rng();
    rng.fill_bytes(&mut salt);
    rng.fill_bytes(&mut wrapped_key_nonce);
    rng.fill_bytes(&mut payload_nonce);
    rng.fill_bytes(dek.as_mut());

    let kek = derive_key(passphrase, &salt, parameters)?;
    let key_cipher =
        Aes256Gcm::new_from_slice(kek.as_ref()).map_err(|_| CryptoError::InvalidParameters)?;
    let wrapped_key = key_cipher
        .encrypt(
            &Nonce::from(wrapped_key_nonce),
            Payload {
                msg: dek.as_ref(),
                aad: WRAP_AAD,
            },
        )
        .map_err(|_| CryptoError::AuthenticationFailed)?;

    let data_cipher =
        Aes256Gcm::new_from_slice(dek.as_ref()).map_err(|_| CryptoError::InvalidParameters)?;
    let payload_aad = payload_aad(associated_data);
    let ciphertext = data_cipher
        .encrypt(
            &Nonce::from(payload_nonce),
            Payload {
                msg: plaintext,
                aad: &payload_aad,
            },
        )
        .map_err(|_| CryptoError::AuthenticationFailed)?;

    Ok(EncryptedEnvelope {
        format: ENVELOPE_FORMAT.to_owned(),
        format_version: ENVELOPE_VERSION,
        cipher: "AES-256-GCM".to_owned(),
        kdf: "Argon2id".to_owned(),
        kdf_parameters: parameters,
        salt: STANDARD_NO_PAD.encode(salt),
        wrapped_key_nonce: STANDARD_NO_PAD.encode(wrapped_key_nonce),
        wrapped_key: STANDARD_NO_PAD.encode(wrapped_key),
        payload_nonce: STANDARD_NO_PAD.encode(payload_nonce),
        ciphertext: STANDARD_NO_PAD.encode(ciphertext),
    })
}

pub fn decrypt(
    envelope: &EncryptedEnvelope,
    passphrase: &str,
    associated_data: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    validate_envelope(envelope)?;
    let salt = decode_array::<SALT_BYTES>(&envelope.salt)?;
    let wrapped_key_nonce = decode_array::<NONCE_BYTES>(&envelope.wrapped_key_nonce)?;
    let payload_nonce = decode_array::<NONCE_BYTES>(&envelope.payload_nonce)?;
    let wrapped_key = decode(&envelope.wrapped_key)?;
    let ciphertext = decode(&envelope.ciphertext)?;

    let kek = derive_key(passphrase, &salt, envelope.kdf_parameters)?;
    let key_cipher =
        Aes256Gcm::new_from_slice(kek.as_ref()).map_err(|_| CryptoError::InvalidParameters)?;
    let mut decoded_dek = key_cipher
        .decrypt(
            &Nonce::from(wrapped_key_nonce),
            Payload {
                msg: &wrapped_key,
                aad: WRAP_AAD,
            },
        )
        .map_err(|_| CryptoError::AuthenticationFailed)?;
    if decoded_dek.len() != KEY_BYTES {
        decoded_dek.zeroize();
        return Err(CryptoError::AuthenticationFailed);
    }
    let mut dek = Zeroizing::new([0_u8; KEY_BYTES]);
    dek.copy_from_slice(&decoded_dek);
    decoded_dek.zeroize();

    let data_cipher =
        Aes256Gcm::new_from_slice(dek.as_ref()).map_err(|_| CryptoError::InvalidParameters)?;
    let payload_aad = payload_aad(associated_data);
    data_cipher
        .decrypt(
            &Nonce::from(payload_nonce),
            Payload {
                msg: &ciphertext,
                aad: &payload_aad,
            },
        )
        .map_err(|_| CryptoError::AuthenticationFailed)
}

pub fn encode(envelope: &EncryptedEnvelope) -> Result<Vec<u8>, CryptoError> {
    Ok(serde_json::to_vec(envelope)?)
}

pub fn decode_envelope(bytes: &[u8]) -> Result<EncryptedEnvelope, CryptoError> {
    let envelope: EncryptedEnvelope = serde_json::from_slice(bytes)?;
    validate_envelope(&envelope)?;
    Ok(envelope)
}

fn derive_key(
    passphrase: &str,
    salt: &[u8; SALT_BYTES],
    parameters: KdfParameters,
) -> Result<Zeroizing<[u8; KEY_BYTES]>, CryptoError> {
    parameters.validate()?;
    let params = Params::new(
        parameters.memory_kib,
        parameters.iterations,
        parameters.parallelism,
        Some(KEY_BYTES),
    )
    .map_err(|_| CryptoError::InvalidParameters)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|_| CryptoError::InvalidParameters)?;
    Ok(key)
}

fn validate_envelope(envelope: &EncryptedEnvelope) -> Result<(), CryptoError> {
    if envelope.format != ENVELOPE_FORMAT
        || envelope.format_version != ENVELOPE_VERSION
        || envelope.cipher != "AES-256-GCM"
        || envelope.kdf != "Argon2id"
    {
        return Err(CryptoError::UnsupportedVersion);
    }
    envelope.kdf_parameters.validate()
}

fn decode(value: &str) -> Result<Vec<u8>, CryptoError> {
    STANDARD_NO_PAD
        .decode(value)
        .map_err(|_| CryptoError::InvalidEncoding)
}

fn decode_array<const N: usize>(value: &str) -> Result<[u8; N], CryptoError> {
    decode(value)?
        .try_into()
        .map_err(|_| CryptoError::InvalidEncoding)
}

fn payload_aad(associated_data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(PAYLOAD_AAD_PREFIX.len() + associated_data.len());
    result.extend_from_slice(PAYLOAD_AAD_PREFIX);
    result.extend_from_slice(associated_data);
    result
}

#[cfg(test)]
mod tests {
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
        let envelope =
            encrypt(b"secret", "correct", b"object-1", fast_parameters()).expect("encrypt");
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
}
