use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::AppError;
use crate::provider::Provider;

use super::account::{
    load_account, mark_last_used, resolve_api_key, resolve_session_token, WindsurfAccount,
};
use super::auth_write::{default_api_server_url, write_windsurf_auth_data};
use super::paths;

const AUTH_STATUS_KEY: &str = "windsurfAuthStatus";
const SESSIONS_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.sessions"}"#;
const API_SERVER_SECRET_KEY: &str =
    r#"secret://{"extensionId":"codeium.windsurf","key":"windsurf_auth.apiServerUrl"}"#;
const SELECTED_AUTH_KEY: &str = "codeium.windsurf-windsurf_auth";
const EXTENSION_STATE_KEY: &str = "codeium.windsurf";

pub fn inject_provider(provider: &Provider) -> Result<(), AppError> {
    let account_id = provider
        .settings_config
        .get("accountId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::InvalidInput(format!(
                "Windsurf provider '{}' is missing accountId",
                provider.id
            ))
        })?;
    inject_account(account_id)
}

pub fn inject_account(account_id: &str) -> Result<(), AppError> {
    let account = load_account(account_id)?
        .ok_or_else(|| AppError::Message(format!("Windsurf account not found: {account_id}")))?;
    let mut auth_status = account
        .windsurf_auth_status_raw
        .clone()
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let access_token = resolve_session_token(&account).ok_or_else(|| {
        AppError::InvalidInput("Windsurf account does not contain a usable token".to_string())
    })?;
    let auth1 = access_token.starts_with("devin-session-token$");
    let api_key = if auth1 {
        access_token.clone()
    } else {
        resolve_api_key(&account).ok_or_else(|| {
            AppError::InvalidInput("Windsurf account does not contain an apiKey".to_string())
        })?
    };
    let api_server_url = non_empty(account.windsurf_api_server_url.as_deref())
        .or_else(|| string_field(&auth_status, &["apiServerUrl", "api_server_url"]))
        .unwrap_or_else(|| default_api_server_url().to_string());
    let account_label = account_label(&account, &auth_status);
    mutate_auth_status(
        &mut auth_status,
        &account,
        &account_label,
        &api_key,
        &access_token,
        &api_server_url,
        auth1,
    );

    let profile_dir = paths::user_data_dir()?;
    let db_path = ensure_state_db(&profile_dir)?;
    backup_state_db(&db_path)?;
    let conn = Connection::open(&db_path).map_err(|error| AppError::Database(error.to_string()))?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|error| AppError::Database(error.to_string()))?;
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|error| {
        AppError::localized(
            "windsurf.database.busy",
            format!("Windsurf state.vscdb 仍被占用: {error}"),
            format!("Windsurf state.vscdb is still busy: {error}"),
        )
    })?;
    let write_result = write_windsurf_auth_data(
        &conn,
        &profile_dir,
        &auth_status,
        &account_label,
        &access_token,
        &api_server_url,
    );
    if let Err(error) = write_result {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(error);
    }
    if let Err(error) = verify_written_account(&conn, &api_key, &account_label) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(error);
    }
    conn.execute_batch("COMMIT")
        .map_err(|error| AppError::Database(error.to_string()))?;
    mark_last_used(account_id)?;
    Ok(())
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
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let file_name = db_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state.vscdb");
    let backup = db_path.with_file_name(format!("{file_name}.cc-switch.bak.{timestamp}"));
    std::fs::copy(db_path, &backup).map_err(|error| AppError::io(&backup, error))?;
    Ok(backup)
}

fn mutate_auth_status(
    auth_status: &mut Value,
    account: &WindsurfAccount,
    account_label: &str,
    api_key: &str,
    access_token: &str,
    api_server_url: &str,
    auth1: bool,
) {
    let Some(object) = auth_status.as_object_mut() else {
        return;
    };
    let display_name =
        non_empty(account.github_name.as_deref()).unwrap_or_else(|| account_label.to_string());
    let display_email = non_empty(account.github_email.as_deref());
    object.insert("apiKey".to_string(), Value::String(api_key.to_string()));
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

    if auth1 {
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
        object.remove("authMethod");
        object.remove("accountId");
        object.remove("primaryOrgId");
    }
}

fn verify_written_account(
    conn: &Connection,
    expected_api_key: &str,
    expected_label: &str,
) -> Result<(), AppError> {
    let auth_status = read_required_item(conn, AUTH_STATUS_KEY)?;
    let actual = serde_json::from_str::<Value>(&auth_status)
        .ok()
        .and_then(|value| {
            value
                .get("apiKey")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if actual.as_deref() != Some(expected_api_key) {
        return Err(verification_error("windsurfAuthStatus.apiKey does not match"));
    }

    verify_secret_buffer(&read_required_item(conn, SESSIONS_SECRET_KEY)?, "sessions")?;
    verify_secret_buffer(
        &read_required_item(conn, API_SERVER_SECRET_KEY)?,
        "apiServerUrl",
    )?;

    let selected_auth = read_required_item(conn, SELECTED_AUTH_KEY)?;
    if selected_auth != expected_label {
        return Err(verification_error("selected auth label does not match"));
    }

    let extension_state = read_required_item(conn, EXTENSION_STATE_KEY)?;
    if !serde_json::from_str::<Value>(&extension_state).is_ok_and(|value| value.is_object()) {
        return Err(verification_error("codeium.windsurf is not a JSON object"));
    }
    Ok(())
}

fn read_required_item(conn: &Connection, key: &str) -> Result<String, AppError> {
    conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        [key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| AppError::Database(error.to_string()))?
    .ok_or_else(|| verification_error(&format!("required ItemTable key is missing: {key}")))
}

fn verify_secret_buffer(raw: &str, name: &str) -> Result<(), AppError> {
    let value = serde_json::from_str::<Value>(raw)
        .map_err(|_| verification_error(&format!("{name} secret is not valid JSON")))?;
    if value.get("type").and_then(Value::as_str) != Some("Buffer") {
        return Err(verification_error(&format!(
            "{name} secret is not a Buffer object"
        )));
    }
    let bytes = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| verification_error(&format!("{name} secret data is missing")))?
        .iter()
        .map(|value| value.as_u64().filter(|byte| *byte <= 255).map(|byte| byte as u8))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| verification_error(&format!("{name} secret contains invalid bytes")))?;
    if !bytes.starts_with(b"v10") {
        return Err(verification_error(&format!(
            "{name} secret does not use the expected v10 prefix"
        )));
    }
    Ok(())
}

fn verification_error(reason: &str) -> AppError {
    AppError::localized(
        "windsurf.verification_failed",
        format!("Windsurf 登录态写入后校验失败: {reason}"),
        format!("Windsurf login-state verification failed after writing: {reason}"),
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

    fn verification_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB)")
            .expect("create ItemTable");
        conn
    }

    fn insert_item(conn: &Connection, key: &str, value: &str) {
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            (key, value),
        )
        .expect("insert item");
    }

    #[test]
    fn verifies_complete_written_account() {
        let conn = verification_db();
        insert_item(&conn, AUTH_STATUS_KEY, r#"{"apiKey":"sk-ws-test"}"#);
        let secret = r#"{"type":"Buffer","data":[118,49,48,1,2,3]}"#;
        insert_item(&conn, SESSIONS_SECRET_KEY, secret);
        insert_item(&conn, API_SERVER_SECRET_KEY, secret);
        insert_item(&conn, SELECTED_AUTH_KEY, "test-user");
        insert_item(&conn, EXTENSION_STATE_KEY, r#"{"apiServerUrl":"test"}"#);

        verify_written_account(&conn, "sk-ws-test", "test-user")
            .expect("verification should pass");
    }

    #[test]
    fn rejects_missing_or_corrupt_secret() {
        let conn = verification_db();
        insert_item(&conn, AUTH_STATUS_KEY, r#"{"apiKey":"sk-ws-test"}"#);
        insert_item(
            &conn,
            SESSIONS_SECRET_KEY,
            r#"{"type":"Buffer","data":[1,2,3]}"#,
        );
        insert_item(
            &conn,
            API_SERVER_SECRET_KEY,
            r#"{"type":"Buffer","data":[118,49,48,1]}"#,
        );
        insert_item(&conn, SELECTED_AUTH_KEY, "test-user");
        insert_item(&conn, EXTENSION_STATE_KEY, "{}");

        assert!(verify_written_account(&conn, "sk-ws-test", "test-user").is_err());
    }
}
