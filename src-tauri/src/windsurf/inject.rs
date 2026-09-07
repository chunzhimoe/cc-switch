use chrono::Utc;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

use crate::error::AppError;
use crate::provider::Provider;

use super::account::{resolve_session_token, WindsurfAccount};
use super::auth_write::{
    self, build_sessions_value, decrypt_encrypted_buffer_json, default_api_server_url,
    inspect_existing_auth_secrets, prepare_encryption_context, write_windsurf_auth_data,
    EncryptionContext, ExistingSecretsSnapshot,
};
use super::paths;

const AUTH_STATUS_KEY: &str = "windsurfAuthStatus";
const SESSIONS_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.sessions"}"#;
const API_SERVER_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.apiServerUrl"}"#;
const SELECTED_AUTH_KEY: &str = "codeium.windsurf-windsurf_auth";
const EXTENSION_STATE_KEY: &str = "codeium.windsurf";
const PENDING_API_KEY_MIGRATION_KEY: &str = "windsurf.pendingApiKeyMigration";

/// Fixed, operation-scoped input for one account injection. It deliberately has
/// no Debug or serialization implementation because it owns live credentials and
/// the one-time SecretStorage encryption context.
pub struct PreparedInjection {
    db_path: PathBuf,
    auth_status: Value,
    account_label: String,
    access_token: String,
    api_server_url: String,
    session_id: String,
    expected_sessions: Value,
    encryption_context: EncryptionContext,
    existing_secrets: ExistingSecretsSnapshot,
}

struct AccountInjectionPayload {
    auth_status: Value,
    account_label: String,
    access_token: String,
    api_server_url: String,
    session_id: String,
    expected_sessions: Value,
}

pub fn prepare_injection(
    account: &WindsurfAccount,
    profile_dir: &Path,
    launch_path: &Path,
) -> Result<PreparedInjection, AppError> {
    let db_path = ensure_state_db(profile_dir)?;
    let encryption_context = prepare_encryption_context(profile_dir, launch_path)?;
    prepare_injection_with_context(account, db_path, encryption_context)
}

fn prepare_injection_with_context(
    account: &WindsurfAccount,
    db_path: PathBuf,
    encryption_context: EncryptionContext,
) -> Result<PreparedInjection, AppError> {
    let payload = build_account_payload(account)?;
    let conn = Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| AppError::Database(error.to_string()))?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|error| AppError::Database(error.to_string()))?;
    let existing_secrets = inspect_existing_auth_secrets(&conn, &encryption_context)?;

    Ok(PreparedInjection {
        db_path,
        auth_status: payload.auth_status,
        account_label: payload.account_label,
        access_token: payload.access_token,
        api_server_url: payload.api_server_url,
        session_id: payload.session_id,
        expected_sessions: payload.expected_sessions,
        encryption_context,
        existing_secrets,
    })
}

/// The generic live-sync hook has no prepared account, target path, or one-time
/// encryption context. It must never rediscover globals or refresh credentials.
pub fn inject_provider(provider: &Provider) -> Result<(), AppError> {
    Err(AppError::localized(
        "windsurf.prepared_switch_required",
        format!(
            "Windsurf 供应商 '{}' 不能通过通用配置同步注入，\
             请使用专用账号切换",
            provider.id
        ),
        format!(
            "Windsurf provider '{}' cannot be injected by generic live sync; \
             use the dedicated account switch",
            provider.id
        ),
    ))
}

pub fn inject_prepared(prepared: &PreparedInjection) -> Result<(), AppError> {
    inject_prepared_with_verifier(prepared, verify_written_account)
}

fn inject_prepared_with_verifier<F>(
    prepared: &PreparedInjection,
    verifier: F,
) -> Result<(), AppError>
where
    F: FnOnce(&Connection, &PreparedInjection) -> Result<(), AppError>,
{
    let _backup_path = backup_state_db(&prepared.db_path)?;
    let conn = Connection::open(&prepared.db_path)
        .map_err(|error| AppError::Database(error.to_string()))?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|error| AppError::Database(error.to_string()))?;
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|error| {
        AppError::localized(
            "windsurf.database.busy",
            format!("Windsurf state.vscdb 仍被占用: {error}"),
            format!("Windsurf state.vscdb is still busy: {error}"),
        )
    })?;

    let write_result = (|| {
        prepared.existing_secrets.ensure_unchanged(&conn)?;
        prepared.existing_secrets.log_invalid_replacements();
        write_windsurf_auth_data(
            &conn,
            &prepared.auth_status,
            &prepared.account_label,
            &prepared.access_token,
            &prepared.api_server_url,
            &prepared.session_id,
            &prepared.encryption_context,
        )?;
        verifier(&conn, prepared)
    })();

    if let Err(error) = write_result {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(error);
    }
    if let Err(error) = conn.execute_batch("COMMIT") {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(AppError::Database(error.to_string()));
    }
    Ok(())
}

fn build_account_payload(account: &WindsurfAccount) -> Result<AccountInjectionPayload, AppError> {
    let mut auth_status = account
        .windsurf_auth_status_raw
        .clone()
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let access_token = resolve_session_token(account).ok_or_else(|| {
        AppError::InvalidInput(
            "Windsurf account does not contain a usable session token".to_string(),
        )
    })?;
    let api_server_url = non_empty(account.windsurf_api_server_url.as_deref())
        .or_else(|| string_field(&auth_status, &["apiServerUrl", "api_server_url"]))
        .unwrap_or_else(|| default_api_server_url().to_string());
    let account_label = account_label(account, &auth_status);
    mutate_auth_status(
        &mut auth_status,
        account,
        &account_label,
        &access_token,
        &api_server_url,
    );
    let session_id = Uuid::new_v4().to_string();
    let expected_sessions = build_sessions_value(&session_id, &access_token, &account_label);

    Ok(AccountInjectionPayload {
        auth_status,
        account_label,
        access_token,
        api_server_url,
        session_id,
        expected_sessions,
    })
}

fn ensure_state_db(profile_dir: &Path) -> Result<PathBuf, AppError> {
    let db_path = paths::state_db_under(profile_dir);
    if !db_path.is_file() {
        return Err(AppError::localized(
            "windsurf.state_db_missing",
            format!("未找到 Windsurf state.vscdb: {}", db_path.display()),
            format!("Windsurf state.vscdb was not found: {}", db_path.display()),
        ));
    }
    Ok(db_path)
}

fn backup_state_db(db_path: &Path) -> Result<PathBuf, AppError> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let file_name = db_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state.vscdb");
    let backup = db_path.with_file_name(format!(
        "{file_name}.cc-switch.bak.{timestamp}.{}",
        Uuid::new_v4()
    ));
    std::fs::copy(db_path, &backup).map_err(|error| AppError::io(&backup, error))?;
    Ok(backup)
}

fn mutate_auth_status(
    auth_status: &mut Value,
    account: &WindsurfAccount,
    account_label: &str,
    access_token: &str,
    api_server_url: &str,
) {
    let Some(object) = auth_status.as_object_mut() else {
        return;
    };
    let display_name =
        non_empty(account.github_name.as_deref()).unwrap_or_else(|| account_label.to_string());
    let display_email = non_empty(account.github_email.as_deref());
    object.insert(
        "apiKey".to_string(),
        Value::String(access_token.to_string()),
    );
    object.insert("name".to_string(), Value::String(display_name.clone()));
    if let Some(email) = &display_email {
        object.insert("email".to_string(), Value::String(email.clone()));
    } else {
        object.remove("email");
    }
    object.insert(
        "apiServerUrl".to_string(),
        Value::String(api_server_url.to_string()),
    );
    object.insert("status".to_string(), Value::String("SignedIn".to_string()));
    object.insert(
        "user".to_string(),
        serde_json::json!({
            "name": display_name,
            "email": display_email,
        }),
    );
    object.insert(
        "timestamp".to_string(),
        Value::Number(Utc::now().timestamp_millis().into()),
    );

    if access_token.starts_with("devin-session-token$") {
        object.insert(
            "sessionToken".to_string(),
            Value::String(access_token.to_string()),
        );
        object.insert("authMethod".to_string(), Value::String("auth1".to_string()));
        insert_optional_string(
            object,
            "userStatusProtoBinaryBase64",
            account.devin_user_status_proto_b64.as_deref(),
        );
        insert_optional_string(object, "accountId", account.devin_account_id.as_deref());
        insert_optional_string(object, "primaryOrgId", account.devin_org_id.as_deref());
    } else {
        object.remove("sessionToken");
        object.remove("accountId");
        object.remove("primaryOrgId");
    }
}

fn verify_written_account(
    conn: &Connection,
    prepared: &PreparedInjection,
) -> Result<(), AppError> {
    let auth_status = read_required_item(conn, AUTH_STATUS_KEY)?;
    let auth_status = serde_json::from_str::<Value>(&auth_status)
        .map_err(|_| verification_error("windsurfAuthStatus is not valid JSON"))?;
    let auth_object = auth_status
        .as_object()
        .ok_or_else(|| verification_error("windsurfAuthStatus is not a JSON object"))?;
    let written_api_key = auth_object.get("apiKey").and_then(Value::as_str);
    if written_api_key != Some(prepared.access_token.as_str()) {
        return Err(verification_error(
            "windsurfAuthStatus.apiKey does not match the prepared session",
        ));
    }
    if auth_object.get("status").and_then(Value::as_str) != Some("SignedIn") {
        return Err(verification_error(
            "windsurfAuthStatus.status is not SignedIn",
        ));
    }
    if auth_object.get("apiServerUrl").and_then(Value::as_str)
        != Some(prepared.api_server_url.as_str())
    {
        return Err(verification_error(
            "windsurfAuthStatus.apiServerUrl does not match",
        ));
    }

    let sessions_raw = read_required_item(conn, SESSIONS_SECRET_KEY)?;
    let sessions_plain =
        decrypt_encrypted_buffer_json(&sessions_raw, &prepared.encryption_context)?;
    let sessions = serde_json::from_slice::<Value>(&sessions_plain)
        .map_err(|_| verification_error("decrypted sessions is not valid JSON"))?;
    if sessions != prepared.expected_sessions {
        return Err(verification_error(
            "decrypted sessions does not match the prepared session structure",
        ));
    }

    let api_server_raw = read_required_item(conn, API_SERVER_SECRET_KEY)?;
    let api_server_plain =
        decrypt_encrypted_buffer_json(&api_server_raw, &prepared.encryption_context)?;
    let api_server = std::str::from_utf8(&api_server_plain)
        .map_err(|_| verification_error("decrypted apiServerUrl is not UTF-8"))?;
    if api_server != prepared.api_server_url {
        return Err(verification_error("decrypted apiServerUrl does not match"));
    }

    let selected_auth = read_required_item(conn, SELECTED_AUTH_KEY)?;
    if selected_auth != prepared.account_label {
        return Err(verification_error("selected auth label does not match"));
    }

    let extension_state = read_required_item(conn, EXTENSION_STATE_KEY)?;
    let extension_state = serde_json::from_str::<Value>(&extension_state)
        .map_err(|_| verification_error("codeium.windsurf is not valid JSON"))?;
    let extension_object = extension_state
        .as_object()
        .ok_or_else(|| verification_error("codeium.windsurf is not a JSON object"))?;
    if extension_object.get("apiServerUrl").and_then(Value::as_str)
        != Some(prepared.api_server_url.as_str())
    {
        return Err(verification_error(
            "codeium.windsurf.apiServerUrl does not match",
        ));
    }
    if extension_object.contains_key(PENDING_API_KEY_MIGRATION_KEY) {
        return Err(verification_error(
            "windsurf.pendingApiKeyMigration was not cleared",
        ));
    }
    if extension_object
        .get("codeium.installationId")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(verification_error(
            "codeium.installationId is missing or empty",
        ));
    }
    Ok(())
}

fn read_required_item(conn: &Connection, key: &str) -> Result<String, AppError> {
    conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .map_err(|error| AppError::Database(error.to_string()))?
    .ok_or_else(|| verification_error(&format!("required ItemTable key is missing: {key}")))
}

fn verification_error(reason: &str) -> AppError {
    AppError::localized(
        "windsurf.verification_failed",
        format!("Windsurf 登录态写入后校验失败: {reason}"),
        format!(
            "Windsurf login-state verification failed after writing: {reason}"
        ),
    )
}

fn account_label(account: &WindsurfAccount, auth_status: &Value) -> String {
    non_empty(account.github_name.as_deref())
        .or_else(|| string_field(auth_status, &["name"]))
        .or_else(|| non_empty(account.github_email.as_deref()))
        .or_else(|| string_field(auth_status, &["email"]))
        .or_else(|| non_empty(Some(&account.github_login)))
        .unwrap_or_else(|| "windsurf_user".to_string())
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .and_then(|value| non_empty(Some(value)))
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn insert_optional_string(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = non_empty(value) {
        object.insert(key.to_string(), Value::String(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db(profile: &Path, extension_state: Option<&str>) -> PathBuf {
        let db_path = paths::state_db_under(profile);
        std::fs::create_dir_all(db_path.parent().expect("state db parent"))
            .expect("create state db parent");
        let conn = Connection::open(&db_path).expect("open test state db");
        conn.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB)")
            .expect("create ItemTable");
        if let Some(extension_state) = extension_state {
            insert_item(&conn, EXTENSION_STATE_KEY, extension_state);
        }
        db_path
    }

    fn test_account() -> WindsurfAccount {
        let mut account = super::super::account::new_token_account(
            "devin-session-token$test-session".to_string(),
            Some("Test Account".to_string()),
        )
        .expect("test account");
        account.github_email = Some("test@example.com".to_string());
        account.devin_account_id = Some("account-1".to_string());
        account.devin_org_id = Some("org-1".to_string());
        account
    }

    fn test_prepared(account: &WindsurfAccount, db_path: PathBuf) -> PreparedInjection {
        prepare_injection_with_context(
            account,
            db_path,
            auth_write::test_macos_encryption_context("test-password"),
        )
        .expect("prepare injection")
    }

    fn verification_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB)")
            .expect("create ItemTable");
        conn
    }

    fn insert_item(conn: &Connection, key: &str, value: &str) {
        conn.execute(
            "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
            (key, value),
        )
        .expect("insert item");
    }

    #[test]
    fn account_payload_uses_session_for_client_api_key_and_session() {
        let mut account = test_account();
        account.windsurf_api_key = Some("sk-ws-ide-only".to_string());
        let payload = build_account_payload(&account).expect("payload");
        assert_eq!(
            payload.auth_status.get("apiKey").and_then(Value::as_str),
            Some("devin-session-token$test-session")
        );
        assert_eq!(
            payload
                .expected_sessions
                .pointer("/0/accessToken")
                .and_then(Value::as_str),
            Some("devin-session-token$test-session")
        );
        assert_eq!(account.windsurf_api_key.as_deref(), Some("sk-ws-ide-only"));
    }

    #[test]
    fn prepared_write_encrypts_and_verifies_complete_session() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = create_test_db(temp.path(), None);
        let prepared = test_prepared(&test_account(), db_path.clone());
        inject_prepared(&prepared).expect("inject prepared");

        let conn = Connection::open(db_path).expect("open written db");
        verify_written_account(&conn, &prepared).expect("strict verification");
        let sessions_raw = read_required_item(&conn, SESSIONS_SECRET_KEY).expect("sessions");
        let sessions =
            decrypt_encrypted_buffer_json(&sessions_raw, &prepared.encryption_context)
                .expect("decrypt sessions");
        assert_eq!(
            serde_json::from_slice::<Value>(&sessions).expect("session json"),
            prepared.expected_sessions
        );
    }

    #[test]
    fn verification_failure_rolls_back_transaction() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = create_test_db(temp.path(), Some(r#"{"unrelated":"keep"}"#));
        let prepared = test_prepared(&test_account(), db_path.clone());
        let error = inject_prepared_with_verifier(&prepared, |_conn, _prepared| {
            Err(verification_error("forced test failure"))
        })
        .expect_err("forced verification failure");
        assert!(error.to_string().contains("forced test failure"));

        let conn = Connection::open(&db_path).expect("open rolled-back db");
        assert!(conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = ?1",
                [AUTH_STATUS_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .expect("query auth")
            .is_none());
        assert_eq!(
            read_required_item(&conn, EXTENSION_STATE_KEY).expect("extension state"),
            r#"{"unrelated":"keep"}"#
        );
        let backup_count = std::fs::read_dir(db_path.parent().expect("db parent"))
            .expect("read db parent")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".cc-switch.bak.")
            })
            .count();
        assert_eq!(backup_count, 1);
    }

    #[test]
    fn clears_pending_migration_and_preserves_unrelated_state() {
        let temp = TempDir::new().expect("temp dir");
        let initial_state = serde_json::json!({
            "windsurf.pendingApiKeyMigration": "old-token",
            "codeium.installationId": "stable-installation",
            "unrelated": {"keep": true}
        })
        .to_string();
        let db_path = create_test_db(temp.path(), Some(&initial_state));
        let prepared = test_prepared(&test_account(), db_path.clone());
        inject_prepared(&prepared).expect("inject prepared");

        let conn = Connection::open(db_path).expect("open written db");
        let state = serde_json::from_str::<Value>(
            &read_required_item(&conn, EXTENSION_STATE_KEY).expect("extension state"),
        )
        .expect("state json");
        assert!(state.get(PENDING_API_KEY_MIGRATION_KEY).is_none());
        assert_eq!(
            state.get("codeium.installationId").and_then(Value::as_str),
            Some("stable-installation")
        );
        assert_eq!(state.pointer("/unrelated/keep"), Some(&Value::Bool(true)));
    }

    #[test]
    fn invalid_existing_auth_secrets_are_bounded_and_replaced() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = create_test_db(temp.path(), Some(r#"{"unrelated":true}"#));
        let conn = Connection::open(&db_path).expect("open test db");
        let garbage = r#"{"type":"Buffer","data":[118,49,48,1,2,3]}"#;
        insert_item(&conn, SESSIONS_SECRET_KEY, garbage);
        insert_item(&conn, API_SERVER_SECRET_KEY, garbage);
        insert_item(&conn, "secret://unrelated", "leave-me");
        drop(conn);

        let prepared = test_prepared(&test_account(), db_path.clone());
        inject_prepared(&prepared).expect("replace invalid auth secrets");

        let conn = Connection::open(db_path).expect("open written db");
        assert_ne!(
            read_required_item(&conn, SESSIONS_SECRET_KEY).expect("sessions"),
            garbage
        );
        assert_ne!(
            read_required_item(&conn, API_SERVER_SECRET_KEY).expect("api server"),
            garbage
        );
        assert_eq!(
            read_required_item(&conn, "secret://unrelated").expect("unrelated secret"),
            "leave-me"
        );
    }

    #[test]
    fn existing_v11_secret_is_refused_during_prepare() {
        let conn = verification_db();
        insert_item(
            &conn,
            SESSIONS_SECRET_KEY,
            r#"{"type":"Buffer","data":[118,49,49,1,2,3]}"#,
        );
        let context = auth_write::test_macos_encryption_context("test-password");
        assert!(inspect_existing_auth_secrets(&conn, &context).is_err());
    }

    #[test]
    fn stale_prepared_secret_snapshot_is_refused_without_overwrite() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = create_test_db(temp.path(), None);
        let prepared = test_prepared(&test_account(), db_path.clone());
        let conn = Connection::open(&db_path).expect("open test db");
        insert_item(&conn, SESSIONS_SECRET_KEY, "newer-value");
        drop(conn);

        assert!(inject_prepared(&prepared).is_err());
        let conn = Connection::open(db_path).expect("open unchanged db");
        assert_eq!(
            read_required_item(&conn, SESSIONS_SECRET_KEY).expect("sessions"),
            "newer-value"
        );
    }

    #[test]
    fn generic_provider_sync_requires_dedicated_switch() {
        let provider = Provider::with_id(
            "provider-one".to_string(),
            "Provider One".to_string(),
            serde_json::json!({"accountId":"account-one"}),
            None,
        );
        let error = inject_provider(&provider).expect_err("generic sync must fail");
        assert!(error.to_string().contains("dedicated account switch"));
    }
}
