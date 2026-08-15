//! Local account-key and sync-payload cryptography facade.

const LOCAL_ONLY_CRYPTO_MESSAGE: &str =
    "账号级同步加密已禁用：当前 MOMO Core 只保留 .moc 私有容器加密";

pub async fn crypto_setup_account_keys(passphrase: String) -> Result<String, String> {
    let _ = passphrase;
    Err(LOCAL_ONLY_CRYPTO_MESSAGE.to_owned())
}

pub async fn crypto_unlock_account_keys(
    bundle_json: String,
    passphrase: String,
) -> Result<(), String> {
    let _ = (bundle_json, passphrase);
    Err(LOCAL_ONLY_CRYPTO_MESSAGE.to_owned())
}

pub async fn crypto_unlock_with_recovery(
    bundle_json: String,
    recovery_key: String,
) -> Result<(), String> {
    let _ = (bundle_json, recovery_key);
    Err(LOCAL_ONLY_CRYPTO_MESSAGE.to_owned())
}

pub async fn crypto_rewrap_account_keys(
    bundle_json: String,
    old_passphrase: String,
    new_passphrase: String,
) -> Result<String, String> {
    let _ = (bundle_json, old_passphrase, new_passphrase);
    Err(LOCAL_ONLY_CRYPTO_MESSAGE.to_owned())
}

pub async fn crypto_reset_with_recovery(
    bundle_json: String,
    recovery_key: String,
    new_passphrase: String,
) -> Result<String, String> {
    let _ = (bundle_json, recovery_key, new_passphrase);
    Err(LOCAL_ONLY_CRYPTO_MESSAGE.to_owned())
}

pub fn crypto_recovery_verifier(recovery_key: String) -> Result<String, String> {
    let _ = recovery_key;
    Err(LOCAL_ONLY_CRYPTO_MESSAGE.to_owned())
}

pub fn crypto_lock() {}

pub fn crypto_is_unlocked() -> bool {
    false
}

pub fn crypto_payload_is_encrypted(payload_json: String) -> Result<bool, String> {
    let _ = payload_json;
    Ok(false)
}

pub async fn crypto_encrypt_sync_payload(
    object_id: String,
    category: String,
    schema_version: i32,
    revision: i64,
    plaintext_json: String,
) -> Result<String, String> {
    let _ = (
        object_id,
        category,
        schema_version,
        revision,
        plaintext_json,
    );
    Err(LOCAL_ONLY_CRYPTO_MESSAGE.to_owned())
}
