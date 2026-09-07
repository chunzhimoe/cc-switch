use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::AppError;

const DEFAULT_API_SERVER_URL: &str = "https://server.codeium.com";
const AUTH_STATUS_KEY: &str = "windsurfAuthStatus";
const SESSIONS_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.sessions"}"#;
const API_SERVER_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.apiServerUrl"}"#;
const SELECTED_AUTH_KEY: &str = "codeium.windsurf-windsurf_auth";
const EXTENSION_STATE_KEY: &str = "codeium.windsurf";
const PENDING_API_KEY_MIGRATION_KEY: &str = "windsurf.pendingApiKeyMigration";
const V10_PREFIX: &[u8] = b"v10";
const V11_PREFIX: &[u8] = b"v11";

pub fn default_api_server_url() -> &'static str {
    DEFAULT_API_SERVER_URL
}

/// Operation-scoped SecretStorage material. It deliberately has no Debug or
/// serialization implementation and is zeroized when the prepared switch drops.
pub(crate) enum EncryptionContext {
    #[cfg(any(target_os = "windows", test))]
    Windows(Zeroizing<Vec<u8>>),
    #[cfg(any(target_os = "macos", test))]
    Macos(Zeroizing<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExistingSecretHealth {
    Missing,
    Valid,
    Invalid,
}

/// Captures exactly the two authentication secrets inspected during prepare.
/// The encrypted values are retained only to reject a stale prepared write.
pub(crate) struct ExistingSecretsSnapshot {
    sessions_raw: Option<String>,
    api_server_raw: Option<String>,
    sessions_health: ExistingSecretHealth,
    api_server_health: ExistingSecretHealth,
}

impl ExistingSecretsSnapshot {
    pub(crate) fn ensure_unchanged(&self, conn: &Connection) -> Result<(), AppError> {
        let sessions = query_optional_item(conn, SESSIONS_SECRET_KEY)?;
        let api_server = query_optional_item(conn, API_SERVER_SECRET_KEY)?;
        if sessions != self.sessions_raw || api_server != self.api_server_raw {
            return Err(AppError::localized(
                "windsurf.secret_storage_changed",
                "Windsurf SecretStorage 在切换准备后发生变化，请重试",
                "Windsurf SecretStorage changed after the switch was prepared; please retry",
            ));
        }
        Ok(())
    }

    pub(crate) fn log_invalid_replacements(&self) {
        if self.sessions_health == ExistingSecretHealth::Invalid {
            log::warn!(
                "Existing Windsurf sessions secret failed strict validation; \
                 replacing it after creating a full database backup"
            );
        }
        if self.api_server_health == ExistingSecretHealth::Invalid {
            log::warn!(
                "Existing Windsurf API-server secret failed strict validation; \
                 replacing it after creating a full database backup"
            );
        }
    }
}

pub(crate) fn prepare_encryption_context(
    profile_dir: &Path,
    launch_path: &Path,
) -> Result<EncryptionContext, AppError> {
    #[cfg(target_os = "windows")]
    {
        let _ = launch_path;
        let key = get_windows_encryption_key(profile_dir)?;
        return Ok(EncryptionContext::Windows(Zeroizing::new(key)));
    }

    #[cfg(target_os = "macos")]
    {
        let target = macos_keychain_target(launch_path, profile_dir)?;
        let password = read_macos_safe_storage_password(target)?;
        return Ok(EncryptionContext::Macos(password));
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (profile_dir, launch_path);
        Err(unsupported_platform_error())
    }
}

pub(crate) fn inspect_existing_auth_secrets(
    conn: &Connection,
    context: &EncryptionContext,
) -> Result<ExistingSecretsSnapshot, AppError> {
    let sessions_raw = query_optional_item(conn, SESSIONS_SECRET_KEY)?;
    let api_server_raw = query_optional_item(conn, API_SERVER_SECRET_KEY)?;
    let sessions_health = classify_existing_secret(
        sessions_raw.as_deref(),
        ExistingSecretKind::Sessions,
        context,
    )?;
    let api_server_health = classify_existing_secret(
        api_server_raw.as_deref(),
        ExistingSecretKind::ApiServerUrl,
        context,
    )?;
    Ok(ExistingSecretsSnapshot {
        sessions_raw,
        api_server_raw,
        sessions_health,
        api_server_health,
    })
}

pub(crate) fn build_sessions_value(
    session_id: &str,
    access_token: &str,
    account_label: &str,
) -> Value {
    serde_json::json!([{
        "id": session_id,
        "accessToken": access_token,
        "account": {
            "label": account_label,
            "id": account_label,
        },
        "scopes": [],
    }])
}

pub(crate) fn write_windsurf_auth_data(
    conn: &Connection,
    auth_status: &Value,
    account_label: &str,
    access_token: &str,
    api_server_url: &str,
    session_id: &str,
    context: &EncryptionContext,
) -> Result<(), AppError> {
    let auth_status_content = serde_json::to_string(auth_status)
        .map_err(|error| AppError::JsonSerialize { source: error })?;
    upsert_item(conn, AUTH_STATUS_KEY, &auth_status_content)?;

    let sessions = build_sessions_value(session_id, access_token, account_label);
    let sessions_plain = serde_json::to_string(&sessions)
        .map_err(|error| AppError::JsonSerialize { source: error })?;
    let encrypted_sessions = encode_encrypted_buffer_json(sessions_plain.as_bytes(), context)?;
    upsert_item(conn, SESSIONS_SECRET_KEY, &encrypted_sessions)?;

    let encrypted_api_server = encode_encrypted_buffer_json(api_server_url.as_bytes(), context)?;
    upsert_item(conn, API_SERVER_SECRET_KEY, &encrypted_api_server)?;

    upsert_item(conn, SELECTED_AUTH_KEY, account_label)?;
    upsert_extension_state(conn, api_server_url)?;

    let onboarding = serde_json::json!({
        "completed": true,
        "version": 1,
        "timestamp": Utc::now().timestamp_millis(),
    });
    upsert_item(
        conn,
        "windsurfOnboarding",
        &serde_json::to_string(&onboarding)
            .map_err(|error| AppError::JsonSerialize { source: error })?,
    )?;

    conn.execute("DELETE FROM ItemTable WHERE key LIKE 'windsurf_auth-%'", [])
        .map_err(|error| AppError::Database(error.to_string()))?;
    let login_key = format!("windsurf_auth-{account_label}");
    let usage_key = format!("windsurf_auth-{account_label}-usages");
    let usage_value = serde_json::json!([{
        "extensionId": "codeium.windsurf",
        "extensionName": "Devin",
        "scopes": [],
        "lastUsed": Utc::now().timestamp_millis(),
    }]);
    upsert_item(conn, &login_key, "[]")?;
    upsert_item(
        conn,
        &usage_key,
        &serde_json::to_string(&usage_value)
            .map_err(|error| AppError::JsonSerialize { source: error })?,
    )?;

    Ok(())
}

fn upsert_extension_state(conn: &Connection, api_server_url: &str) -> Result<(), AppError> {
    let existing = query_optional_item(conn, EXTENSION_STATE_KEY)?;
    let mut state = existing
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(object) = state.as_object_mut() {
        object.insert(
            "apiServerUrl".to_string(),
            Value::String(api_server_url.to_string()),
        );
        object.remove(PENDING_API_KEY_MIGRATION_KEY);
        if object
            .get("codeium.installationId")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            object.insert(
                "codeium.installationId".to_string(),
                Value::String(Uuid::new_v4().to_string()),
            );
        }
    }
    let serialized =
        serde_json::to_string(&state).map_err(|error| AppError::JsonSerialize { source: error })?;
    upsert_item(conn, EXTENSION_STATE_KEY, &serialized)
}

fn upsert_item(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        (key, value),
    )
    .map_err(|error| AppError::Database(error.to_string()))?;
    Ok(())
}

fn query_optional_item(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .map_err(|error| AppError::Database(error.to_string()))
}

#[derive(Clone, Copy)]
enum ExistingSecretKind {
    Sessions,
    ApiServerUrl,
}

fn classify_existing_secret(
    raw: Option<&str>,
    kind: ExistingSecretKind,
    context: &EncryptionContext,
) -> Result<ExistingSecretHealth, AppError> {
    let Some(raw) = raw else {
        return Ok(ExistingSecretHealth::Missing);
    };
    let bytes = match parse_encrypted_buffer_json(raw) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(ExistingSecretHealth::Invalid),
    };
    if bytes.starts_with(V11_PREFIX) {
        return Err(unsupported_secret_version_error());
    }
    if !bytes.starts_with(V10_PREFIX) {
        return Ok(ExistingSecretHealth::Invalid);
    }
    let plaintext = match decrypt_secret_payload(&bytes, context) {
        Ok(plaintext) => plaintext,
        Err(_) => return Ok(ExistingSecretHealth::Invalid),
    };
    let valid = match kind {
        ExistingSecretKind::Sessions => {
            serde_json::from_slice::<Value>(&plaintext).is_ok_and(|value| value.is_array())
        }
        ExistingSecretKind::ApiServerUrl => {
            std::str::from_utf8(&plaintext).is_ok_and(|value| !value.trim().is_empty())
        }
    };
    Ok(if valid {
        ExistingSecretHealth::Valid
    } else {
        ExistingSecretHealth::Invalid
    })
}

fn decode_buffer_data(value: &Value) -> Result<Vec<u8>, AppError> {
    if value.get("type").and_then(Value::as_str) != Some("Buffer") {
        return Err(secret_format_error("Secret value is not a Buffer object"));
    }
    value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| secret_format_error("Secret Buffer data is missing"))?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .filter(|value| *value <= 255)
                .map(|value| value as u8)
                .ok_or_else(|| secret_format_error("Secret Buffer contains an invalid byte"))
        })
        .collect()
}

fn parse_encrypted_buffer_json(raw: &str) -> Result<Vec<u8>, AppError> {
    let value = serde_json::from_str::<Value>(raw)
        .map_err(|_| secret_format_error("Secret value is not valid JSON"))?;
    decode_buffer_data(&value)
}

fn encode_encrypted_buffer_json(
    plaintext: &[u8],
    context: &EncryptionContext,
) -> Result<String, AppError> {
    let encrypted = encrypt_secret_payload(plaintext, context)?;
    serde_json::to_string(&serde_json::json!({
        "type": "Buffer",
        "data": encrypted,
    }))
    .map_err(|error| AppError::JsonSerialize { source: error })
}

pub(crate) fn decrypt_encrypted_buffer_json(
    raw: &str,
    context: &EncryptionContext,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let encrypted = parse_encrypted_buffer_json(raw)?;
    if encrypted.starts_with(V11_PREFIX) {
        return Err(unsupported_secret_version_error());
    }
    if !encrypted.starts_with(V10_PREFIX) {
        return Err(secret_format_error(
            "Secret value does not use the supported v10 format",
        ));
    }
    decrypt_secret_payload(&encrypted, context)
}

fn encrypt_secret_payload(
    plaintext: &[u8],
    context: &EncryptionContext,
) -> Result<Vec<u8>, AppError> {
    #[cfg(not(any(target_os = "windows", target_os = "macos", test)))]
    let _ = plaintext;
    match context {
        #[cfg(any(target_os = "windows", test))]
        EncryptionContext::Windows(key) => encrypt_windows_gcm_v10(key, plaintext),
        #[cfg(any(target_os = "macos", test))]
        EncryptionContext::Macos(password) => encrypt_macos_secret(plaintext, password.as_bytes()),
    }
}

fn decrypt_secret_payload(
    encrypted: &[u8],
    context: &EncryptionContext,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    #[cfg(not(any(target_os = "windows", target_os = "macos", test)))]
    let _ = encrypted;
    match context {
        #[cfg(any(target_os = "windows", test))]
        EncryptionContext::Windows(key) => decrypt_windows_gcm_v10(key, encrypted),
        #[cfg(any(target_os = "macos", test))]
        EncryptionContext::Macos(password) => {
            decrypt_macos_secret(encrypted, password.as_bytes())
        }
    }
}

fn secret_format_error(reason: &str) -> AppError {
    AppError::localized(
        "windsurf.secret_storage_invalid",
        format!("Windsurf SecretStorage 数据无效: {reason}"),
        format!("Windsurf SecretStorage data is invalid: {reason}"),
    )
}

fn unsupported_secret_version_error() -> AppError {
    AppError::localized(
        "windsurf.secret_storage_v11_unsupported",
        "检测到不受支持的 Windsurf SecretStorage v11，已拒绝覆盖",
        "Unsupported Windsurf SecretStorage v11 was detected and \
         will not be overwritten",
    )
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn unsupported_platform_error() -> AppError {
    AppError::localized(
        "windsurf.secret_storage_platform_pending",
        "当前阶段仅支持 Windows/macOS Windsurf SecretStorage 写入",
        "This phase only supports Windsurf SecretStorage writes on Windows/macOS",
    )
}

#[cfg(target_os = "windows")]
fn get_local_state_path(profile_dir: &Path) -> Result<std::path::PathBuf, AppError> {
    let path = profile_dir.join("Local State");
    if path.is_file() {
        Ok(path)
    } else {
        Err(AppError::localized(
            "windsurf.windows.local_state_missing",
            format!("未找到 Windsurf Local State: {}", path.display()),
            format!("Windsurf Local State was not found: {}", path.display()),
        ))
    }
}

#[cfg(target_os = "windows")]
fn dpapi_decrypt(protected: &[u8]) -> Result<Vec<u8>, AppError> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: protected.len() as u32,
            pbData: protected.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        CryptUnprotectData(&input, None, None, None, None, 0, &mut output).map_err(|error| {
            AppError::localized(
                "windsurf.windows.dpapi_failed",
                format!(
                    "Windsurf DPAPI 解密失败，请使用创建该配置的同一 Windows 用户运行: {error}"
                ),
                format!(
                    "Windsurf DPAPI decryption failed; run as the same Windows \
                     user that created this profile: {error}"
                ),
            )
        })?;
        if output.pbData.is_null() || output.cbData == 0 {
            if !output.pbData.is_null() {
                let _ = LocalFree(HLOCAL(output.pbData as *mut _));
            }
            return Err(AppError::Config(
                "Windsurf DPAPI 解密返回空数据".to_string(),
            ));
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
        Ok(result)
    }
}

#[cfg(target_os = "windows")]
fn get_windows_encryption_key(profile_dir: &Path) -> Result<Vec<u8>, AppError> {
    use base64::{engine::general_purpose, Engine as _};

    let local_state_path = get_local_state_path(profile_dir)?;
    let content = std::fs::read_to_string(&local_state_path)
        .map_err(|error| AppError::io(&local_state_path, error))?;
    let local_state: Value =
        serde_json::from_str(&content).map_err(|error| AppError::json(&local_state_path, error))?;
    let encrypted_key = local_state
        .pointer("/os_crypt/encrypted_key")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::localized(
                "windsurf.windows.encrypted_key_missing",
                "Windsurf Local State 缺少 os_crypt.encrypted_key",
                "Windsurf Local State is missing os_crypt.encrypted_key",
            )
        })?;
    let encrypted_key = general_purpose::STANDARD
        .decode(encrypted_key)
        .map_err(|error| {
            AppError::Config(format!("Windsurf encrypted_key Base64 解码失败: {error}"))
        })?;
    let protected = encrypted_key
        .strip_prefix(b"DPAPI")
        .ok_or_else(|| AppError::Config("Windsurf encrypted_key 不包含 DPAPI 前缀".to_string()))?;
    if protected.is_empty() {
        return Err(AppError::Config(
            "Windsurf encrypted_key 的 DPAPI 数据为空".to_string(),
        ));
    }

    let key = dpapi_decrypt(protected)?;
    if key.len() != 32 {
        return Err(AppError::Config(format!(
            "Windsurf AES key 长度无效: {}（期望 32）",
            key.len()
        )));
    }
    Ok(key)
}

#[cfg(any(target_os = "windows", test))]
fn encrypt_windows_gcm_v10(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
    use aes_gcm::aead::generic_array::GenericArray;
    use aes_gcm::aead::{Aead, AeadCore, OsRng};
    use aes_gcm::{Aes256Gcm, KeyInit};

    if key.len() != 32 {
        return Err(secret_format_error(
            "Windows encryption key length is invalid",
        ));
    }
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| secret_format_error("AES-GCM encryption failed"))?;
    let mut encrypted = Vec::with_capacity(V10_PREFIX.len() + nonce.len() + ciphertext.len());
    encrypted.extend_from_slice(V10_PREFIX);
    encrypted.extend_from_slice(&nonce);
    encrypted.extend_from_slice(&ciphertext);
    Ok(encrypted)
}

#[cfg(any(target_os = "windows", test))]
fn decrypt_windows_gcm_v10(
    key: &[u8],
    encrypted: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    use aes_gcm::aead::generic_array::GenericArray;
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::Aes256Gcm;

    const NONCE_LEN: usize = 12;
    const TAG_LEN: usize = 16;
    if key.len() != 32 {
        return Err(secret_format_error(
            "Windows encryption key length is invalid",
        ));
    }
    let payload = encrypted
        .strip_prefix(V10_PREFIX)
        .ok_or_else(|| secret_format_error("Windows secret is missing the v10 prefix"))?;
    if payload.len() < NONCE_LEN + TAG_LEN {
        return Err(secret_format_error("Windows v10 secret is truncated"));
    }
    let (nonce, ciphertext) = payload.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let plaintext = cipher
        .decrypt(GenericArray::from_slice(nonce), ciphertext)
        .map_err(|_| secret_format_error("Windows v10 secret authentication failed"))?;
    Ok(Zeroizing::new(plaintext))
}

#[cfg(any(target_os = "macos", test))]
const MACOS_SAFE_STORAGE_IV: [u8; 16] = [b' '; 16];
#[cfg(any(target_os = "macos", test))]
const MACOS_SAFE_STORAGE_SALT: &[u8] = b"saltysalt";
#[cfg(any(target_os = "macos", test))]
const MACOS_SAFE_STORAGE_ITERATIONS: u32 = 1003;

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MacosKeychainTarget {
    application_name: &'static str,
    service: &'static str,
    account: &'static str,
}

#[cfg(any(target_os = "macos", test))]
const DEVIN_KEYCHAIN_TARGET: MacosKeychainTarget = MacosKeychainTarget {
    application_name: "Devin",
    service: "Devin Safe Storage",
    account: "Devin Key",
};

#[cfg(any(target_os = "macos", test))]
const WINDSURF_KEYCHAIN_TARGET: MacosKeychainTarget = MacosKeychainTarget {
    application_name: "Windsurf",
    service: "Windsurf Safe Storage",
    account: "Windsurf Key",
};

#[cfg(any(target_os = "macos", test))]
fn macos_keychain_target(
    launch_path: &Path,
    profile_dir: &Path,
) -> Result<MacosKeychainTarget, AppError> {
    let target = launch_path
        .ancestors()
        .find_map(|ancestor| {
            let name = ancestor.file_name()?.to_str()?;
            if name.eq_ignore_ascii_case("Devin.app") {
                Some(DEVIN_KEYCHAIN_TARGET)
            } else if name.eq_ignore_ascii_case("Windsurf.app") {
                Some(WINDSURF_KEYCHAIN_TARGET)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            AppError::localized(
                "windsurf.macos.app_identity_unknown",
                "无法从目标 .app 路径确定 Devin/Windsurf 身份，已拒绝查询 Keychain",
                "The Devin/Windsurf identity could not be determined from the \
                 target .app path; Keychain lookup was refused",
            )
        })?;

    let profile_name = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let profile_target = if profile_name.eq_ignore_ascii_case("Devin")
        || profile_name.to_ascii_lowercase().starts_with("devin-")
    {
        Some(DEVIN_KEYCHAIN_TARGET)
    } else if profile_name.eq_ignore_ascii_case("Windsurf")
        || profile_name.to_ascii_lowercase().starts_with("windsurf-")
    {
        Some(WINDSURF_KEYCHAIN_TARGET)
    } else {
        None
    };
    if profile_target.is_some_and(|profile_target| profile_target != target) {
        return Err(AppError::localized(
            "windsurf.macos.profile_app_mismatch",
            "目标 .app 与用户数据目录品牌不一致，已拒绝查询 Keychain",
            "The target .app and user-data directory brands do not match; \
             Keychain lookup was refused",
        ));
    }
    Ok(target)
}

#[cfg(any(target_os = "macos", test))]
fn strip_macos_command_line_ending(mut value: String) -> String {
    if value.ends_with("\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with('\r') || value.ends_with('\n') {
        value.pop();
    }
    value
}

#[cfg(target_os = "macos")]
fn read_macos_keychain_secret(target: MacosKeychainTarget) -> Result<String, AppError> {
    use std::process::Command;

    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            target.service,
            "-a",
            target.account,
        ])
        .output()
        .map_err(|error| {
            AppError::localized(
                "windsurf.secret_storage_keychain_command_failed",
                format!("执行 macOS Keychain 查询失败（阶段: spawn security）: {error}"),
                format!(
                    "Failed to execute the macOS Keychain query \
                     (stage: spawn security): {error}"
                ),
            )
        })?;
    if !output.status.success() {
        let exit = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        return Err(AppError::localized(
            "windsurf.secret_storage_keychain_query_failed",
            format!(
                "macOS Keychain 查询失败（阶段: find-generic-password，\
                 退出状态: {exit}）。请确认已在目标客户端初始化安全存储并允许访问"
            ),
            format!(
                "The macOS Keychain query failed \
                 (stage: find-generic-password, exit status: {exit}). \
                 Initialize secure storage in the target client and allow access"
            ),
        ));
    }
    let secret = String::from_utf8(output.stdout).map_err(|_| {
        AppError::localized(
            "windsurf.secret_storage_keychain_output_invalid",
            "macOS Keychain 查询返回了无效文本（阶段: decode stdout）",
            "The macOS Keychain query returned invalid text (stage: decode stdout)",
        )
    })?;
    Ok(strip_macos_command_line_ending(secret))
}

#[cfg(any(target_os = "macos", test))]
fn read_macos_safe_storage_password_with<F>(
    target: MacosKeychainTarget,
    mut reader: F,
) -> Result<Zeroizing<String>, AppError>
where
    F: FnMut(MacosKeychainTarget) -> Result<String, AppError>,
{
    let secret = reader(target)?;
    if secret.is_empty() {
        return Err(AppError::localized(
            "windsurf.secret_storage_keychain_missing",
            format!(
                "目标 {} 客户端尚未初始化 Safe Storage Keychain 条目，请先手动打开官方客户端完成初始化",
                target.application_name
            ),
            format!(
                "The target {} client has not initialized its Safe Storage \
                 Keychain item; open the official client first",
                target.application_name
            ),
        ));
    }
    Ok(Zeroizing::new(secret))
}

#[cfg(target_os = "macos")]
fn read_macos_safe_storage_password(
    target: MacosKeychainTarget,
) -> Result<Zeroizing<String>, AppError> {
    read_macos_safe_storage_password_with(target, read_macos_keychain_secret)
}

#[cfg(any(target_os = "macos", test))]
fn derive_macos_safe_storage_key(password: &[u8]) -> Zeroizing<[u8; 16]> {
    use pbkdf2::pbkdf2_hmac;
    use sha1::Sha1;

    let mut key = Zeroizing::new([0u8; 16]);
    pbkdf2_hmac::<Sha1>(
        password,
        MACOS_SAFE_STORAGE_SALT,
        MACOS_SAFE_STORAGE_ITERATIONS,
        &mut key[..],
    );
    key
}

#[cfg(any(target_os = "macos", test))]
fn encrypt_macos_secret(plaintext: &[u8], password: &[u8]) -> Result<Vec<u8>, AppError> {
    use aes::Aes128;
    use cbc::cipher::block_padding::Pkcs7;
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};

    type Aes128CbcEncryptor = cbc::Encryptor<Aes128>;

    let key = derive_macos_safe_storage_key(password);
    let cipher =
        Aes128CbcEncryptor::new_from_slices(&key[..], &MACOS_SAFE_STORAGE_IV).map_err(|_| {
            secret_format_error("Failed to initialize the macOS AES-CBC encryptor")
        })?;
    let message_len = plaintext.len();
    let padding_len = 16 - (message_len % 16);
    let mut buffer = Zeroizing::new(plaintext.to_vec());
    buffer.resize(message_len + padding_len, 0);
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(buffer.as_mut_slice(), message_len)
        .map_err(|_| secret_format_error("macOS AES-CBC encryption failed"))?
        .to_vec();

    let mut encrypted = Vec::with_capacity(V10_PREFIX.len() + ciphertext.len());
    encrypted.extend_from_slice(V10_PREFIX);
    encrypted.extend_from_slice(&ciphertext);
    Ok(encrypted)
}

#[cfg(any(target_os = "macos", test))]
fn decrypt_macos_secret(
    encrypted: &[u8],
    password: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    use aes::Aes128;
    use cbc::cipher::block_padding::Pkcs7;
    use cbc::cipher::{BlockDecryptMut, KeyIvInit};

    type Aes128CbcDecryptor = cbc::Decryptor<Aes128>;

    let ciphertext = encrypted
        .strip_prefix(V10_PREFIX)
        .ok_or_else(|| secret_format_error("macOS secret is missing the v10 prefix"))?;
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return Err(secret_format_error(
            "macOS v10 ciphertext length is invalid",
        ));
    }
    let key = derive_macos_safe_storage_key(password);
    let cipher =
        Aes128CbcDecryptor::new_from_slices(&key[..], &MACOS_SAFE_STORAGE_IV).map_err(|_| {
            secret_format_error("Failed to initialize the macOS AES-CBC decryptor")
        })?;
    let mut buffer = Zeroizing::new(ciphertext.to_vec());
    let plaintext_len = cipher
        .decrypt_padded_mut::<Pkcs7>(buffer.as_mut_slice())
        .map_err(|_| secret_format_error("macOS v10 padding validation failed"))?
        .len();
    buffer.truncate(plaintext_len);
    Ok(buffer)
}

#[cfg(test)]
pub(crate) fn test_macos_encryption_context(password: &str) -> EncryptionContext {
    EncryptionContext::Macos(Zeroizing::new(password.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const MACOS_HELLO_FIXTURE: [u8; 19] = [
        0x76, 0x31, 0x30, 0x95, 0x88, 0x15, 0xf4, 0x8a, 0x74, 0x22, 0x7a, 0x2a, 0x31, 0x53,
        0x50, 0x50, 0xc6, 0x88, 0x42,
    ];

    #[test]
    fn macos_fixture_encrypts_and_decrypts_strictly() {
        let encrypted = encrypt_macos_secret(b"hello", b"test-password").expect("encrypt");
        assert_eq!(encrypted, MACOS_HELLO_FIXTURE);
        let decrypted =
            decrypt_macos_secret(&MACOS_HELLO_FIXTURE, b"test-password").expect("decrypt");
        assert_eq!(decrypted.as_slice(), b"hello");
    }

    #[test]
    fn macos_decrypt_rejects_wrong_key_truncation_and_bad_padding() {
        assert!(decrypt_macos_secret(&MACOS_HELLO_FIXTURE, b"wrong-password").is_err());
        assert!(decrypt_macos_secret(
            &MACOS_HELLO_FIXTURE[..MACOS_HELLO_FIXTURE.len() - 1],
            b"test-password"
        )
        .is_err());
        let mut bad_padding = MACOS_HELLO_FIXTURE;
        *bad_padding.last_mut().expect("last byte") ^= 0xff;
        assert!(decrypt_macos_secret(&bad_padding, b"test-password").is_err());
    }

    #[test]
    fn buffer_parser_rejects_invalid_shape_and_bytes() {
        assert!(parse_encrypted_buffer_json(r#"{"data":[118,49,48]}"#).is_err());
        assert!(parse_encrypted_buffer_json(
            r#"{"type":"Buffer","data":[118,49,48,256]}"#
        )
        .is_err());
        assert!(parse_encrypted_buffer_json(
            r#"{"type":"Buffer","data":[118,49,"48"]}"#
        )
        .is_err());
    }

    #[test]
    fn decrypt_json_rejects_unknown_v11_and_v10_garbage() {
        let context = test_macos_encryption_context("test-password");
        let v11 = r#"{"type":"Buffer","data":[118,49,49,1,2,3]}"#;
        assert!(decrypt_encrypted_buffer_json(v11, &context).is_err());
        let garbage = r#"{"type":"Buffer","data":[118,49,48,1,2,3]}"#;
        assert!(decrypt_encrypted_buffer_json(garbage, &context).is_err());
    }

    #[test]
    fn windows_gcm_round_trip_validates_nonce_and_authentication() {
        let key = [7u8; 32];
        let encrypted = encrypt_windows_gcm_v10(&key, b"windsurf-secret").expect("encrypt");
        let decrypted = decrypt_windows_gcm_v10(&key, &encrypted).expect("decrypt");
        assert_eq!(decrypted.as_slice(), b"windsurf-secret");
        let mut tampered = encrypted;
        tampered[15] ^= 1;
        assert!(decrypt_windows_gcm_v10(&key, &tampered).is_err());
        assert!(decrypt_windows_gcm_v10(&key, b"v10short").is_err());
    }

    #[test]
    fn keychain_target_comes_only_from_exact_app_identity() {
        let devin = macos_keychain_target(
            Path::new("/Applications/Devin.app/Contents/MacOS/Devin"),
            Path::new("/tmp/Profiles/custom"),
        )
        .expect("Devin target");
        assert_eq!(devin, DEVIN_KEYCHAIN_TARGET);
        assert_eq!(devin.account, "Devin Key");

        let windsurf = macos_keychain_target(
            Path::new("/Applications/Windsurf.app"),
            Path::new("/tmp/Profiles/custom"),
        )
        .expect("Windsurf target");
        assert_eq!(windsurf, WINDSURF_KEYCHAIN_TARGET);
        assert_eq!(windsurf.account, "Windsurf Key");

        assert!(macos_keychain_target(
            Path::new("/Applications/Other.app/Contents/MacOS/Other"),
            Path::new("/tmp/Profiles/custom")
        )
        .is_err());
        assert!(macos_keychain_target(
            Path::new("/Applications/Windsurf.app"),
            Path::new("/tmp/Devin")
        )
        .is_err());
    }

    #[test]
    fn keychain_reader_is_called_exactly_once_without_fallback() {
        let calls = Cell::new(0);
        let password = read_macos_safe_storage_password_with(DEVIN_KEYCHAIN_TARGET, |target| {
            calls.set(calls.get() + 1);
            assert_eq!(target, DEVIN_KEYCHAIN_TARGET);
            Err(AppError::Message("denied".to_string()))
        });
        assert!(password.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn strips_only_security_command_line_ending() {
        assert_eq!(
            strip_macos_command_line_ending("  secret with spaces  \n".to_string()),
            "  secret with spaces  "
        );
        assert_eq!(
            strip_macos_command_line_ending("secret\n\n".to_string()),
            "secret\n"
        );
    }
}
