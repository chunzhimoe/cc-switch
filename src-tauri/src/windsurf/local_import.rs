use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use crate::error::AppError;

use super::account::{upsert_account, WindsurfAccount};
use super::paths;

const AUTH_STATUS_KEY: &str = "windsurfAuthStatus";
const EXTENSION_STATE_KEY: &str = "codeium.windsurf";
const MAX_AUTH_STATUS_BYTES: usize = 512 * 1024;

pub fn read_local_auth_status() -> Result<Option<Value>, AppError> {
    let db_path = paths::state_db_path()?;
    if !db_path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(&db_path).map_err(|error| AppError::Database(error.to_string()))?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [AUTH_STATUS_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| AppError::Database(error.to_string()))?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > MAX_AUTH_STATUS_BYTES {
        return Err(AppError::InvalidInput(
            "Windsurf auth status exceeds the supported size".to_string(),
        ));
    }
    let value: Value = serde_json::from_str(&raw).map_err(|error| AppError::json(&db_path, error))?;
    if !value.is_object() {
        return Err(AppError::InvalidInput(
            "Windsurf auth status must be a JSON object".to_string(),
        ));
    }
    Ok(Some(value))
}

pub fn import_local_account() -> Result<WindsurfAccount, AppError> {
    let auth_status = read_local_auth_status()?.ok_or_else(|| {
        AppError::localized(
            "windsurf.local_auth_missing",
            "未在本机 Windsurf/Devin 客户端中找到登录信息",
            "No local Windsurf/Devin authentication was found",
        )
    })?;
    let object = auth_status.as_object().ok_or_else(|| {
        AppError::InvalidInput("Windsurf auth status must be an object".to_string())
    })?;
    let api_key = first_string(object, &["apiKey", "api_key"]).ok_or_else(|| {
        AppError::localized(
            "windsurf.local_token_missing",
            "本机 Windsurf 登录信息缺少 apiKey",
            "The local Windsurf authentication does not contain an apiKey",
        )
    })?;
    let session_token = first_string(object, &["sessionToken", "session_token"])
        .filter(|token| token.starts_with("devin-session-token$"))
        .or_else(|| api_key.starts_with("devin-session-token$").then(|| api_key.clone()));
    let auth_method = first_string(object, &["authMethod", "auth_method"]);
    let is_auth1 = session_token.is_some()
        || auth_method
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("auth1"));
    let extension_hint = read_extension_state().unwrap_or_default();
    let user = object.get("user").and_then(Value::as_object);
    let email = first_string(object, &["email"])
        .or_else(|| user.and_then(|value| first_string(value, &["email"])))
        .or_else(|| first_string(&extension_hint, &["lastLoginEmail"]));
    let name = first_string(object, &["name"])
        .or_else(|| user.and_then(|value| first_string(value, &["name"])))
        .or_else(|| email.clone());
    let label = name
        .clone()
        .or_else(|| email.clone())
        .unwrap_or_else(|| "Windsurf Account".to_string());
    let now = chrono::Utc::now().timestamp();

    let account = WindsurfAccount {
        id: String::new(),
        github_login: label,
        github_id: 0,
        github_name: name,
        github_email: email,
        tags: None,
        github_access_token: session_token.clone().unwrap_or_default(),
        copilot_token: String::new(),
        windsurf_api_key: Some(api_key),
        windsurf_api_server_url: first_string(object, &["apiServerUrl", "api_server_url"]),
        windsurf_auth_token: session_token.clone(),
        windsurf_user_status: object.get("userStatus").cloned(),
        windsurf_plan_status: object.get("planStatus").cloned(),
        windsurf_auth_status_raw: Some(auth_status),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        windsurf_token_type: Some(if is_auth1 {
            "devin-session".to_string()
        } else {
            "firebase".to_string()
        }),
        devin_auth1_token: first_string(object, &["auth1Token", "auth1_token"]),
        devin_account_id: first_string(object, &["accountId", "account_id"]),
        devin_org_id: first_string(object, &["primaryOrgId", "orgId", "org_id"]),
        devin_session_token: session_token,
        devin_user_status_proto_b64: first_string(
            object,
            &["userStatusProtoBinaryBase64"],
        ),
        created_at: now,
        last_used: now,
    };
    upsert_account(account)
}

fn read_extension_state() -> Result<serde_json::Map<String, Value>, AppError> {
    let db_path = paths::state_db_path()?;
    let conn = Connection::open(&db_path).map_err(|error| AppError::Database(error.to_string()))?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [EXTENSION_STATE_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| AppError::Database(error.to_string()))?;
    Ok(raw
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default())
}

fn first_string(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
