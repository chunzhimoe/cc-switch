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

#[cfg(target_os = "macos")]
const MACOS_SAFE_STORAGE_IV: [u8; 16] = [b' '; 16];
#[cfg(target_os = "macos")]
const MACOS_SAFE_STORAGE_SALT: &[u8] = b"saltysalt";
#[cfg(target_os = "macos")]
const MACOS_SAFE_STORAGE_ITERATIONS: u32 = 1003;

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct MacosKeychainQuery {
    service: &'static str,
    account: Option<&'static str>,
}

#[cfg(target_os = "macos")]
const DEVIN_KEYCHAIN_QUERIES: [MacosKeychainQuery; 4] = [
    MacosKeychainQuery {
        service: "Devin Safe Storage",
        account: Some("Devin"),
    },
    MacosKeychainQuery {
        service: "Devin Safe Storage",
        account: Some("Devin Key"),
    },
    MacosKeychainQuery {
        service: "Devin Safe Storage",
        account: Some("devin"),
    },
    MacosKeychainQuery {
        service: "Devin Safe Storage",
        account: None,
    },
];

#[cfg(target_os = "macos")]
const WINDSURF_KEYCHAIN_QUERIES: [MacosKeychainQuery; 5] = [
    MacosKeychainQuery {
        service: "Windsurf Safe Storage",
        account: Some("Windsurf Key"),
    },
    MacosKeychainQuery {
        service: "Windsurf Safe Storage",
        account: Some("Windsurf"),
    },
    MacosKeychainQuery {
        service: "Windsurf Safe Storage",
        account: Some("windsurf"),
    },
    MacosKeychainQuery {
        service: "Windsurf Safe Storage",
        account: Some("Windsurf Safe Storage"),
    },
    MacosKeychainQuery {
        service: "Windsurf Safe Storage",
        account: None,
    },
];

#[cfg(target_os = "macos")]
fn macos_profile_prefers_windsurf(profile_dir: &Path) -> bool {
    profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("windsurf"))
}

#[cfg(target_os = "macos")]
fn macos_keychain_queries(profile_dir: &Path) -> impl Iterator<Item = MacosKeychainQuery> + '_ {
    let (primary, fallback): (&[MacosKeychainQuery], &[MacosKeychainQuery]) =
        if macos_profile_prefers_windsurf(profile_dir) {
            (&WINDSURF_KEYCHAIN_QUERIES, &DEVIN_KEYCHAIN_QUERIES)
        } else {
            (&DEVIN_KEYCHAIN_QUERIES, &WINDSURF_KEYCHAIN_QUERIES)
        };
    primary.iter().chain(fallback.iter()).copied()
}

#[cfg(target_os = "macos")]
fn strip_macos_command_line_ending(mut value: String) -> String {
    if value.ends_with("\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with('\r') || value.ends_with('\n') {
        value.pop();
    }
    value
}

#[cfg(target_os = "macos")]
fn read_macos_keychain_secret(query: MacosKeychainQuery) -> Option<String> {
    use std::process::Command;

    let mut command = Command::new("/usr/bin/security");
    command.args(["find-generic-password", "-w", "-s", query.service]);
    if let Some(account) = query.account {
        command.args(["-a", account]);
    }

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let secret = String::from_utf8(output.stdout).ok()?;
    let secret = strip_macos_command_line_ending(secret);
    (!secret.is_empty()).then_some(secret)
}

#[cfg(target_os = "macos")]
fn read_macos_safe_storage_password(
    profile_dir: &Path,
) -> Result<zeroize::Zeroizing<String>, AppError> {
    for query in macos_keychain_queries(profile_dir) {
        if let Some(secret) = read_macos_keychain_secret(query) {
            return Ok(zeroize::Zeroizing::new(secret));
        }
    }

    Err(AppError::localized(
        "windsurf.secret_storage_keychain_missing",
        "读取 Devin/Windsurf Safe Storage 密钥失败",
        "Failed to read the Devin/Windsurf Safe Storage key from Keychain",
    ))
}

#[cfg(target_os = "macos")]
fn encrypt_macos_secret(plaintext: &[u8], password: &[u8]) -> Result<Vec<u8>, AppError> {
    use aes::Aes128;
    use cbc::cipher::block_padding::Pkcs7;
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};
    use pbkdf2::pbkdf2_hmac;
    use sha1::Sha1;
    use zeroize::Zeroizing;

    type Aes128CbcEncryptor = cbc::Encryptor<Aes128>;

    let mut key = Zeroizing::new([0u8; 16]);
    pbkdf2_hmac::<Sha1>(
        password,
        MACOS_SAFE_STORAGE_SALT,
        MACOS_SAFE_STORAGE_ITERATIONS,
        &mut key[..],
    );

    let cipher = Aes128CbcEncryptor::new_from_slices(&key[..], &MACOS_SAFE_STORAGE_IV)
        .map_err(|error| {
            AppError::Config(format!("Failed to initialize AES-CBC encryptor: {error}"))
        })?;
    let message_len = plaintext.len();
    let padding_len = 16 - (message_len % 16);
    let mut buffer = Zeroizing::new(plaintext.to_vec());
    buffer.resize(message_len + padding_len, 0);
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(buffer.as_mut_slice(), message_len)
        .map_err(|error| AppError::Config(format!("AES-CBC encryption failed: {error}")))?
        .to_vec();

    let mut encrypted = Vec::with_capacity(V10_PREFIX.len() + ciphertext.len());
    encrypted.extend_from_slice(V10_PREFIX);
    encrypted.extend_from_slice(&ciphertext);
    Ok(encrypted)
}

#[cfg(target_os = "macos")]
fn encrypt_secret_payload(
    plaintext: &[u8],
    preferred_prefix: Option<&str>,
    profile_dir: &Path,
) -> Result<Vec<u8>, AppError> {
    if preferred_prefix == Some("v11") {
        return Err(AppError::Config(
            "Windsurf SecretStorage v11 writes are not supported on macOS".to_string(),
        ));
    }

    let password = read_macos_safe_storage_password(profile_dir)?;
    encrypt_macos_secret(plaintext, password.as_bytes())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn encrypt_secret_payload(
    _plaintext: &[u8],
    _preferred_prefix: Option<&str>,
    _profile_dir: &Path,
) -> Result<Vec<u8>, AppError> {
    Err(AppError::localized(
        "windsurf.secret_storage_platform_pending",
        "当前阶段仅支持 Windows/macOS Windsurf SecretStorage 写入",
        "This phase only supports Windsurf SecretStorage writes on Windows/macOS",
    ))
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

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

    #[test]
    fn orders_keychain_queries_by_profile_brand() {
        let windsurf = macos_keychain_queries(Path::new("/tmp/Windsurf"))
            .next()
            .expect("Windsurf query");
        assert_eq!(windsurf.service, "Windsurf Safe Storage");

        let devin = macos_keychain_queries(Path::new("/tmp/Devin"))
            .next()
            .expect("Devin query");
        assert_eq!(devin.service, "Devin Safe Storage");
    }

    #[test]
    fn encrypts_with_chromium_macos_secret_storage_format() {
        let encrypted = encrypt_macos_secret(b"hello", b"test-password").expect("encrypt");
        assert_eq!(
            encrypted,
            vec![
                0x76, 0x31, 0x30, 0x95, 0x88, 0x15, 0xf4, 0x8a, 0x74, 0x22, 0x7a, 0x2a,
                0x31, 0x53, 0x50, 0x50, 0xc6, 0x88, 0x42,
            ]
        );
    }
}
