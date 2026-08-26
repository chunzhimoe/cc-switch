use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

use crate::error::AppError;

const DEFAULT_API_SERVER_URL: &str = "https://server.codeium.com";
const AUTH_STATUS_KEY: &str = "windsurfAuthStatus";
const SESSIONS_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.sessions"}"#;
const API_SERVER_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.apiServerUrl"}"#;
const SELECTED_AUTH_KEY: &str = "codeium.windsurf-windsurf_auth";
const EXTENSION_STATE_KEY: &str = "codeium.windsurf";
const V10_PREFIX: &[u8] = b"v10";
const V11_PREFIX: &[u8] = b"v11";

pub fn default_api_server_url() -> &'static str {
    DEFAULT_API_SERVER_URL
}

pub fn write_windsurf_auth_data(
    conn: &Connection,
    profile_dir: &Path,
    auth_status: &Value,
    account_label: &str,
    access_token: &str,
    api_server_url: &str,
) -> Result<(), AppError> {
    let auth_status_content = serde_json::to_string(auth_status)
        .map_err(|error| AppError::JsonSerialize { source: error })?;
    upsert_item(conn, AUTH_STATUS_KEY, &auth_status_content)?;

    let sessions_prefix = query_existing_secret_prefix(conn, SESSIONS_SECRET_KEY)?;
    let sessions = serde_json::json!([{
        "id": Uuid::new_v4().to_string(),
        "accessToken": access_token,
        "account": {
            "label": account_label,
            "id": account_label,
        },
        "scopes": [],
    }]);
    let sessions_plain = serde_json::to_string(&sessions)
        .map_err(|error| AppError::JsonSerialize { source: error })?;
    let encrypted_sessions = encode_encrypted_buffer_json(
        sessions_plain.as_bytes(),
        sessions_prefix.as_deref(),
        profile_dir,
    )?;
    upsert_item(conn, SESSIONS_SECRET_KEY, &encrypted_sessions)?;

    let api_server_prefix = query_existing_secret_prefix(conn, API_SERVER_SECRET_KEY)?;
    let encrypted_api_server = encode_encrypted_buffer_json(
        api_server_url.as_bytes(),
        api_server_prefix.as_deref(),
        profile_dir,
    )?;
    upsert_item(conn, API_SERVER_SECRET_KEY, &encrypted_api_server)?;

    upsert_item(conn, SELECTED_AUTH_KEY, account_label)?;
    upsert_extension_state(conn, api_server_url, access_token)?;

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

fn upsert_extension_state(
    conn: &Connection,
    api_server_url: &str,
    access_token: &str,
) -> Result<(), AppError> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [EXTENSION_STATE_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| AppError::Database(error.to_string()))?;
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
        if is_supported_auth_token(access_token) {
            object.insert(
                "windsurf.pendingApiKeyMigration".to_string(),
                Value::String(access_token.to_string()),
            );
        } else {
            object.remove("windsurf.pendingApiKeyMigration");
        }
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

fn is_supported_auth_token(token: &str) -> bool {
    token.starts_with("sk-ws-")
        || token.starts_with("devin-session-token$")
        || token.starts_with("cog_")
}

fn upsert_item(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        (key, value),
    )
    .map_err(|error| AppError::Database(error.to_string()))?;
    Ok(())
}

fn query_existing_secret_prefix(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    let existing: Option<String> = conn
        .query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| AppError::Database(error.to_string()))?;
    let Some(existing) = existing else {
        return Ok(None);
    };
    let parsed: Value = match serde_json::from_str(&existing) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let bytes = match decode_buffer_data(&parsed) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    Ok(if bytes.starts_with(V10_PREFIX) {
        Some("v10".to_string())
    } else if bytes.starts_with(V11_PREFIX) {
        Some("v11".to_string())
    } else {
        None
    })
}

fn decode_buffer_data(value: &Value) -> Result<Vec<u8>, AppError> {
    value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::InvalidInput("Secret value is not a Buffer object".to_string()))?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .filter(|value| *value <= 255)
                .map(|value| value as u8)
                .ok_or_else(|| {
                    AppError::InvalidInput("Secret Buffer contains an invalid byte".to_string())
                })
        })
        .collect()
}

fn encode_encrypted_buffer_json(
    plaintext: &[u8],
    preferred_prefix: Option<&str>,
    profile_dir: &Path,
) -> Result<String, AppError> {
    let encrypted = encrypt_secret_payload(plaintext, preferred_prefix, profile_dir)?;
    serde_json::to_string(&serde_json::json!({
        "type": "Buffer",
        "data": encrypted,
    }))
    .map_err(|error| AppError::JsonSerialize { source: error })
}

#[cfg(target_os = "windows")]
fn encrypt_secret_payload(
    plaintext: &[u8],
    _preferred_prefix: Option<&str>,
    profile_dir: &Path,
) -> Result<Vec<u8>, AppError> {
    use aes_gcm::aead::generic_array::GenericArray;
    use aes_gcm::aead::{Aead, AeadCore, OsRng};
    use aes_gcm::{Aes256Gcm, KeyInit};
    use base64::{engine::general_purpose, Engine as _};
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let local_state_path = profile_dir.join("Local State");
    let content = std::fs::read_to_string(&local_state_path)
        .map_err(|error| AppError::io(&local_state_path, error))?;
    let local_state: Value =
        serde_json::from_str(&content).map_err(|error| AppError::json(&local_state_path, error))?;
    let encrypted_key = local_state
        .pointer("/os_crypt/encrypted_key")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::Config("Windsurf Local State is missing os_crypt.encrypted_key".to_string())
        })?;
    let encrypted_key = general_purpose::STANDARD
        .decode(encrypted_key)
        .map_err(|error| AppError::Config(format!("Invalid encrypted_key base64: {error}")))?;
    let protected = encrypted_key.strip_prefix(b"DPAPI").ok_or_else(|| {
        AppError::Config("Windsurf encrypted_key does not use the DPAPI prefix".to_string())
    })?;

    let key = unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: protected.len() as u32,
            pbData: protected.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        CryptUnprotectData(&input, None, None, None, None, 0, &mut output)
            .map_err(|error| AppError::Config(format!("DPAPI decrypt failed: {error}")))?;
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
        result
    };
    if key.len() != 32 {
        return Err(AppError::Config(format!(
            "Windsurf AES key has invalid length: {}",
            key.len()
        )));
    }

    let cipher = Aes256Gcm::new(GenericArray::from_slice(&key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|error| AppError::Config(format!("AES-GCM encryption failed: {error}")))?;
    let mut encrypted = Vec::with_capacity(3 + nonce.len() + ciphertext.len());
    encrypted.extend_from_slice(V10_PREFIX);
    encrypted.extend_from_slice(&nonce);
    encrypted.extend_from_slice(&ciphertext);
    Ok(encrypted)
}

#[cfg(not(target_os = "windows"))]
fn encrypt_secret_payload(
    _plaintext: &[u8],
    _preferred_prefix: Option<&str>,
    _profile_dir: &Path,
) -> Result<Vec<u8>, AppError> {
    Err(AppError::localized(
        "windsurf.secret_storage_platform_pending",
        "当前阶段仅支持 Windows Windsurf SecretStorage 写入",
        "This phase only supports Windsurf SecretStorage writes on Windows",
    ))
}
