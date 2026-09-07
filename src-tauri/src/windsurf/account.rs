use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{LazyLock, Mutex};

use crate::config::{get_app_config_dir, read_json_file, write_json_file};
use crate::error::AppError;

const ACCOUNTS_INDEX_FILE: &str = "windsurf_accounts.json";
const ACCOUNTS_DIR: &str = "windsurf_accounts";
static ACCOUNT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfAccount {
    pub id: String,
    #[serde(default)]
    pub github_login: String,
    #[serde(default)]
    pub github_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub github_access_token: String,
    #[serde(default)]
    pub copilot_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windsurf_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windsurf_api_server_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windsurf_auth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windsurf_user_status: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windsurf_plan_status: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windsurf_auth_status_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windsurf_token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devin_auth1_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devin_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devin_org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devin_session_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devin_user_status_proto_b64: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfAccountSummary {
    pub id: String,
    pub label: String,
    pub email: Option<String>,
    pub token_type: String,
    pub masked_token: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfAccountIndex {
    pub version: String,
    pub accounts: Vec<WindsurfAccountSummary>,
}

impl Default for WindsurfAccountIndex {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

fn accounts_dir() -> std::path::PathBuf {
    get_app_config_dir().join(ACCOUNTS_DIR)
}

pub fn accounts_index_path() -> std::path::PathBuf {
    get_app_config_dir().join(ACCOUNTS_INDEX_FILE)
}

fn account_path(account_id: &str) -> std::path::PathBuf {
    accounts_dir().join(format!("{account_id}.json"))
}

fn load_index() -> Result<WindsurfAccountIndex, AppError> {
    let path = accounts_index_path();
    if !path.exists() {
        return Ok(WindsurfAccountIndex::default());
    }
    read_json_file(&path)
}

fn save_index(index: &WindsurfAccountIndex) -> Result<(), AppError> {
    write_json_file(&accounts_index_path(), index)
}

pub fn load_account(account_id: &str) -> Result<Option<WindsurfAccount>, AppError> {
    let path = account_path(account_id);
    if !path.exists() {
        return Ok(None);
    }
    read_json_file(&path).map(Some)
}

fn save_account(account: &WindsurfAccount) -> Result<(), AppError> {
    write_json_file(&account_path(&account.id), account)
}

pub fn list_account_summaries() -> Result<Vec<WindsurfAccountSummary>, AppError> {
    Ok(load_index()?.accounts)
}

pub fn upsert_account(mut account: WindsurfAccount) -> Result<WindsurfAccount, AppError> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| AppError::Message("Windsurf account lock poisoned".to_string()))?;
    let now = Utc::now().timestamp();
    if account.id.trim().is_empty() {
        account.id = stable_account_id(&account);
    }
    if let Some(existing) = load_account(&account.id)? {
        account.created_at = existing.created_at;
        if account.tags.is_none() {
            account.tags = existing.tags;
        }
    } else if account.created_at <= 0 {
        account.created_at = now;
    }
    account.last_used = now;

    save_account(&account)?;
    let mut index = load_index()?;
    let summary = account.summary();
    if let Some(existing) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        *existing = summary;
    } else {
        index.accounts.push(summary);
    }
    index
        .accounts
        .sort_by(|a, b| b.last_used.cmp(&a.last_used).then_with(|| a.id.cmp(&b.id)));
    save_index(&index)?;
    Ok(account)
}

pub fn delete_account(account_id: &str) -> Result<bool, AppError> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| AppError::Message("Windsurf account lock poisoned".to_string()))?;
    let path = account_path(account_id);
    let existed = path.exists();
    if existed {
        std::fs::remove_file(&path).map_err(|error| AppError::io(&path, error))?;
    }
    let mut index = load_index()?;
    index.accounts.retain(|account| account.id != account_id);
    save_index(&index)?;
    Ok(existed)
}

pub fn mark_last_used(account_id: &str) -> Result<(), AppError> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| AppError::Message("Windsurf account lock poisoned".to_string()))?;
    let mut account = load_account(account_id)?.ok_or_else(refresh_target_changed)?;
    let mut index = load_index()?;
    let summary = index
        .accounts
        .iter_mut()
        .find(|item| item.id == account_id)
        .ok_or_else(refresh_target_changed)?;
    account.last_used = Utc::now().timestamp();
    *summary = account.summary();
    index
        .accounts
        .sort_by(|a, b| b.last_used.cmp(&a.last_used).then_with(|| a.id.cmp(&b.id)));
    save_account(&account)?;
    save_index(&index)
}

pub fn new_token_account(
    token: String,
    label: Option<String>,
) -> Result<WindsurfAccount, AppError> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(AppError::InvalidInput(
            "Windsurf token cannot be empty".to_string(),
        ));
    }
    if !is_supported_token(&token) {
        return Err(AppError::InvalidInput(
            "Unsupported Windsurf token format".to_string(),
        ));
    }

    let now = Utc::now().timestamp();
    let is_session = token.starts_with("devin-session-token$");
    let display = label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Windsurf Account")
        .to_string();
    let mut account = WindsurfAccount {
        id: String::new(),
        github_login: display.clone(),
        github_id: 0,
        github_name: Some(display.clone()),
        github_email: None,
        tags: None,
        github_access_token: if is_session {
            token.clone()
        } else {
            String::new()
        },
        copilot_token: String::new(),
        windsurf_api_key: Some(token.clone()),
        windsurf_api_server_url: Some(if is_session {
            "https://server.self-serve.windsurf.com".to_string()
        } else {
            "https://server.codeium.com".to_string()
        }),
        windsurf_auth_token: is_session.then(|| token.clone()),
        windsurf_user_status: None,
        windsurf_plan_status: None,
        windsurf_auth_status_raw: Some(serde_json::json!({
            "apiKey": token,
            "name": display,
            "authMethod": if is_session { "auth1" } else { "firebase" },
        })),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        windsurf_token_type: Some(if is_session {
            "devin-session".to_string()
        } else {
            "firebase".to_string()
        }),
        devin_auth1_token: None,
        devin_account_id: None,
        devin_org_id: None,
        devin_session_token: is_session.then(|| token.clone()),
        devin_user_status_proto_b64: None,
        created_at: now,
        last_used: now,
    };
    account.id = stable_account_id(&account);
    Ok(account)
}

pub fn new_account_from_oauth(
    api_key: String,
    api_server_url: String,
    name: Option<String>,
    email: Option<String>,
    user_status: Option<Value>,
) -> WindsurfAccount {
    let now = Utc::now().timestamp();
    let email = email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let display = name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| email.clone())
        .unwrap_or_else(|| "Windsurf Account".to_string());
    let login = email
        .as_deref()
        .and_then(|value| value.split('@').next())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| display.clone());

    let mut auth_status = serde_json::json!({
        "apiKey": api_key,
        "name": display,
        "authMethod": "firebase",
        "apiServerUrl": api_server_url,
        "status": "SignedIn",
    });
    if let Some(object) = auth_status.as_object_mut() {
        if let Some(email) = &email {
            object.insert("email".to_string(), Value::String(email.clone()));
            object.insert(
                "user".to_string(),
                serde_json::json!({
                    "name": display,
                    "email": email,
                }),
            );
        }
        if let Some(status) = &user_status {
            object.insert("userStatus".to_string(), status.clone());
            if let Some(plan_status) = status.get("planStatus") {
                object.insert("planStatus".to_string(), plan_status.clone());
            }
        }
    }

    let mut account = WindsurfAccount {
        id: String::new(),
        github_login: login,
        github_id: 0,
        github_name: Some(display),
        github_email: email,
        tags: None,
        github_access_token: String::new(),
        copilot_token: String::new(),
        windsurf_api_key: Some(api_key.clone()),
        windsurf_api_server_url: Some(api_server_url),
        windsurf_auth_token: Some(api_key),
        windsurf_user_status: user_status
            .as_ref()
            .and_then(|value| value.get("userStatus").cloned())
            .or_else(|| user_status.clone()),
        windsurf_plan_status: user_status
            .as_ref()
            .and_then(|value| value.get("planStatus").cloned())
            .or_else(|| {
                user_status
                    .as_ref()
                    .and_then(|value| value.pointer("/userStatus/planStatus").cloned())
            }),
        windsurf_auth_status_raw: Some(auth_status),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: Some(now),
        windsurf_token_type: Some("firebase".to_string()),
        devin_auth1_token: None,
        devin_account_id: None,
        devin_org_id: None,
        devin_session_token: None,
        devin_user_status_proto_b64: None,
        created_at: now,
        last_used: now,
    };
    account.id = stable_account_id(&account);
    account
}

pub fn new_account_from_auth1_refresh(
    email: Option<String>,
    label: Option<String>,
    auth1_token: &str,
    refresh: &super::devin_oauth::FullRefreshResult,
) -> WindsurfAccount {
    let now = Utc::now().timestamp();
    let email = email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let display = label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| email.clone())
        .unwrap_or_else(|| format!("devin_{}", refresh.account_id));
    let login = email
        .as_deref()
        .and_then(|value| value.split('@').next())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| display.clone());
    let mut auth_status = serde_json::json!({
        "apiKey": refresh.session_token,
        "sessionToken": refresh.session_token,
        "name": display,
        "authMethod": "auth1",
        "apiServerUrl": "https://server.self-serve.windsurf.com",
        "accountId": refresh.account_id,
        "primaryOrgId": refresh.org_id,
        "status": "SignedIn",
    });
    if let Some(object) = auth_status.as_object_mut() {
        if let Some(email) = &email {
            object.insert("email".to_string(), Value::String(email.clone()));
            object.insert(
                "user".to_string(),
                serde_json::json!({
                    "name": display,
                    "email": email,
                }),
            );
        }
        if let Some(proto) = &refresh.user_status_proto_b64 {
            object.insert(
                "userStatusProtoBinaryBase64".to_string(),
                Value::String(proto.clone()),
            );
        }
        if let Some(status) = &refresh.user_status {
            object.insert("userStatus".to_string(), status.clone());
            if let Some(plan_status) = status.get("planStatus") {
                object.insert("planStatus".to_string(), plan_status.clone());
            }
        }
    }

    let mut account = WindsurfAccount {
        id: String::new(),
        github_login: login,
        github_id: 0,
        github_name: Some(display),
        github_email: email,
        tags: None,
        github_access_token: refresh.ide_token.clone(),
        copilot_token: String::new(),
        windsurf_api_key: Some(refresh.ide_token.clone()),
        windsurf_api_server_url: Some("https://server.self-serve.windsurf.com".to_string()),
        windsurf_auth_token: Some(refresh.session_token.clone()),
        windsurf_user_status: refresh
            .user_status
            .as_ref()
            .and_then(|value| value.get("userStatus").cloned())
            .or_else(|| refresh.user_status.clone()),
        windsurf_plan_status: refresh
            .user_status
            .as_ref()
            .and_then(|value| value.get("planStatus").cloned())
            .or_else(|| {
                refresh
                    .user_status
                    .as_ref()
                    .and_then(|value| value.pointer("/userStatus/planStatus").cloned())
            }),
        windsurf_auth_status_raw: Some(auth_status),
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: Some(now),
        windsurf_token_type: Some("devin-session".to_string()),
        devin_auth1_token: Some(auth1_token.trim().to_string()),
        devin_account_id: Some(refresh.account_id.clone()),
        devin_org_id: Some(refresh.org_id.clone()),
        devin_session_token: Some(refresh.session_token.clone()),
        devin_user_status_proto_b64: refresh.user_status_proto_b64.clone(),
        created_at: now,
        last_used: now,
    };
    account.id = stable_account_id(&account);
    account
}

pub async fn refresh_account_for_switch(account_id: &str) -> Result<WindsurfAccount, AppError> {
    let snapshot = load_account(account_id)?.ok_or_else(refresh_target_changed)?;
    refresh_account_with(
        snapshot,
        |auth1| async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(90),
                super::devin_oauth::full_refresh_from_auth1(&auth1),
            )
            .await
            .map_err(|_| {
                AppError::localized(
                    "windsurf.refresh_timeout",
                    "刷新 Windsurf 登录凭据超时，客户端和登录态未修改",
                    "Windsurf credential refresh timed out; the client was not changed",
                )
            })?
            .map_err(|error| {
                AppError::localized(
                    "windsurf.refresh_failed",
                    format!("刷新 Windsurf 登录凭据失败：{error}"),
                    "Windsurf credential refresh failed; the client was not changed",
                )
            })
        },
        persist_auth1_refresh,
    )
    .await
}

async fn refresh_account_with<F, Fut, C>(
    snapshot: WindsurfAccount,
    refresh: F,
    commit: C,
) -> Result<WindsurfAccount, AppError>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<super::devin_oauth::FullRefreshResult, AppError>>,
    C: FnOnce(
        &WindsurfAccount,
        &super::devin_oauth::FullRefreshResult,
    ) -> Result<WindsurfAccount, AppError>,
{
    let Some(auth1) = non_empty(snapshot.devin_auth1_token.as_deref()) else {
        resolve_session_token(&snapshot).ok_or_else(|| {
            AppError::InvalidInput("Windsurf account has no usable login credential".to_string())
        })?;
        return Ok(snapshot);
    };
    if !auth1.starts_with("auth1_") {
        return Err(AppError::InvalidInput(
            "Invalid Windsurf refresh credential format".to_string(),
        ));
    }
    // The caller holds only an operation gate, not ACCOUNT_LOCK or a DB transaction.
    let refreshed = refresh(auth1).await?;
    commit(&snapshot, &refreshed)
}

fn merge_auth1_refresh(
    snapshot: &WindsurfAccount,
    mut current: WindsurfAccount,
    refresh: &super::devin_oauth::FullRefreshResult,
) -> Result<WindsurfAccount, AppError> {
    if current.id != snapshot.id
        || current.devin_auth1_token != snapshot.devin_auth1_token
        || current.devin_account_id != snapshot.devin_account_id
        || current
            .devin_account_id
            .as_deref()
            .is_some_and(|id| id != refresh.account_id.as_str())
    {
        return Err(refresh_target_changed());
    }
    if !refresh.auth1_token.starts_with("auth1_")
        || !refresh.session_token.starts_with("devin-session-token$")
        || refresh.ide_token.trim().is_empty()
        || refresh.account_id.trim().is_empty()
        || refresh.org_id.trim().is_empty()
    {
        return Err(AppError::InvalidInput(
            "Incomplete Windsurf refresh response".to_string(),
        ));
    }

    let org_changed = current.devin_org_id.as_deref() != Some(refresh.org_id.as_str());
    current.devin_auth1_token = Some(refresh.auth1_token.clone());
    current.devin_account_id = Some(refresh.account_id.clone());
    current.devin_org_id = Some(refresh.org_id.clone());
    current.devin_session_token = Some(refresh.session_token.clone());
    current.windsurf_auth_token = Some(refresh.session_token.clone());
    current.windsurf_api_key = Some(refresh.ide_token.clone());
    current.github_access_token = refresh.ide_token.clone();
    current.windsurf_token_type = Some("devin-session".to_string());

    if org_changed {
        current.windsurf_user_status = None;
        current.windsurf_plan_status = None;
        current.devin_user_status_proto_b64 = None;
        current.usage_updated_at = None;
    }
    if let Some(proto) = &refresh.user_status_proto_b64 {
        current.devin_user_status_proto_b64 = Some(proto.clone());
    }
    if let Some(status) = &refresh.user_status {
        current.windsurf_user_status = Some(status.get("userStatus").unwrap_or(status).clone());
        current.windsurf_plan_status = status
            .get("planStatus")
            .cloned()
            .or_else(|| status.pointer("/userStatus/planStatus").cloned());
        current.quota_query_last_error = None;
        current.quota_query_last_error_at = None;
    }
    if refresh.user_status.is_some() || refresh.user_status_proto_b64.is_some() {
        current.usage_updated_at = Some(Utc::now().timestamp());
    }

    let mut raw = current
        .windsurf_auth_status_raw
        .take()
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let root = raw.as_object_mut().expect("auth status is an object");
    root.insert(
        "apiKey".to_string(),
        Value::String(refresh.session_token.clone()),
    );
    root.insert(
        "sessionToken".to_string(),
        Value::String(refresh.session_token.clone()),
    );
    root.insert(
        "authMethod".to_string(),
        Value::String("auth1".to_string()),
    );
    root.insert(
        "accountId".to_string(),
        Value::String(refresh.account_id.clone()),
    );
    root.insert(
        "primaryOrgId".to_string(),
        Value::String(refresh.org_id.clone()),
    );
    if org_changed {
        for key in [
            "userStatus",
            "planStatus",
            "userStatusProtoBinaryBase64",
            "allowedCommandModelConfigsProtoBinaryBase64",
        ] {
            root.remove(key);
        }
    }
    if let Some(proto) = &refresh.user_status_proto_b64 {
        root.insert(
            "userStatusProtoBinaryBase64".to_string(),
            Value::String(proto.clone()),
        );
    }
    if let Some(status) = &refresh.user_status {
        root.insert("userStatus".to_string(), status.clone());
        if let Some(plan) = &current.windsurf_plan_status {
            root.insert("planStatus".to_string(), plan.clone());
        }
    }
    current.windsurf_auth_status_raw = Some(raw);
    Ok(current)
}

fn persist_auth1_refresh(
    snapshot: &WindsurfAccount,
    refresh: &super::devin_oauth::FullRefreshResult,
) -> Result<WindsurfAccount, AppError> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| AppError::Message("Windsurf account lock poisoned".to_string()))?;
    // Reload under the lock: deletion or credential replacement during the HTTP
    // request must never be undone by writing an old snapshot back to disk.
    let current = load_account(&snapshot.id)?.ok_or_else(refresh_target_changed)?;
    let current = merge_auth1_refresh(snapshot, current, refresh)?;
    let mut index = load_index()?;
    let summary = index
        .accounts
        .iter_mut()
        .find(|item| item.id == current.id)
        .ok_or_else(refresh_target_changed)?;
    *summary = current.summary();
    save_account(&current)?;
    save_index(&index)?;
    Ok(current)
}

fn refresh_target_changed() -> AppError {
    AppError::localized(
        "windsurf.refresh_target_changed",
        "账号已删除、被替换或登录身份不一致，请重新选择账号",
        "The account was deleted, replaced, or changed identity during refresh",
    )
}

pub fn resolve_api_key(account: &WindsurfAccount) -> Option<String> {
    non_empty(account.windsurf_api_key.as_deref())
        .or_else(|| {
            string_from_value(
                account.windsurf_auth_status_raw.as_ref(),
                &["apiKey", "api_key"],
            )
        })
        .or_else(|| {
            non_empty(Some(&account.github_access_token))
                .filter(|token| token.starts_with("sk-ws-") || token.starts_with("cog_"))
        })
}

pub fn resolve_session_token(account: &WindsurfAccount) -> Option<String> {
    for candidate in [
        account.devin_session_token.as_deref(),
        account.windsurf_auth_token.as_deref(),
        Some(account.github_access_token.as_str()),
    ] {
        if let Some(token) = non_empty(candidate) {
            if token.starts_with("devin-session-token$") {
                return Some(token);
            }
        }
    }
    if uses_auth1(account) {
        None
    } else {
        resolve_api_key(account)
    }
}

pub fn uses_auth1(account: &WindsurfAccount) -> bool {
    account.windsurf_token_type.as_deref() == Some("devin-session")
        || account
            .devin_auth1_token
            .as_deref()
            .is_some_and(|value| value.starts_with("auth1_"))
        || account
            .windsurf_auth_token
            .as_deref()
            .is_some_and(|value| value.starts_with("devin-session-token$"))
}

pub fn is_supported_token(token: &str) -> bool {
    token.starts_with("sk-ws-")
        || token.starts_with("devin-session-token$")
        || token.starts_with("auth1_")
        || token.starts_with("cog_")
}

pub fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 10 {
        return "••••••".to_string();
    }
    let prefix: String = chars.iter().take(6).collect();
    let suffix: String = chars.iter().rev().take(4).rev().collect();
    format!("{prefix}••••{suffix}")
}

fn stable_account_id(account: &WindsurfAccount) -> String {
    let identity = resolve_api_key(account)
        .or_else(|| resolve_session_token(account))
        .unwrap_or_else(|| format!("{}:{}", account.github_login, account.github_id));
    let digest = Sha256::digest(identity.as_bytes());
    let short = digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("windsurf_{short}")
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn string_from_value(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    let object = value.and_then(Value::as_object)?;
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .and_then(|value| non_empty(Some(value)))
}

impl WindsurfAccount {
    pub fn summary(&self) -> WindsurfAccountSummary {
        let token = resolve_session_token(self)
            .or_else(|| resolve_api_key(self))
            .unwrap_or_default();
        WindsurfAccountSummary {
            id: self.id.clone(),
            label: self
                .github_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| self.github_email.clone())
                .unwrap_or_else(|| self.github_login.clone()),
            email: self.github_email.clone(),
            token_type: self
                .windsurf_token_type
                .clone()
                .unwrap_or_else(|| "firebase".to_string()),
            masked_token: mask_token(&token),
            tags: self.tags.clone().unwrap_or_default(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::devin_oauth::FullRefreshResult;
    use super::*;
    use serde_json::json;

    fn refresh_result() -> FullRefreshResult {
        FullRefreshResult {
            ide_token: "devin-session-token$ide-fixture".to_string(),
            session_token: "devin-session-token$session-fixture".to_string(),
            auth1_token: "auth1_fixture".to_string(),
            account_id: "account-fixture".to_string(),
            org_id: "org-fixture".to_string(),
            user_status_proto_b64: None,
            user_status: None,
        }
    }

    fn password_account() -> WindsurfAccount {
        let result = refresh_result();
        new_account_from_auth1_refresh(
            Some("fixture@example.test".to_string()),
            Some("Fixture".to_string()),
            &result.auth1_token,
            &result,
        )
    }

    #[test]
    fn password_account_keeps_session_and_ide_credentials_distinct() {
        let account = password_account();
        let refresh = refresh_result();
        assert_eq!(
            resolve_session_token(&account),
            Some(refresh.session_token.clone())
        );
        assert_eq!(resolve_api_key(&account), Some(refresh.ide_token));
        assert_eq!(
            account.windsurf_auth_token,
            Some(refresh.session_token.clone())
        );
        assert_eq!(
            account.windsurf_auth_status_raw.unwrap()["sessionToken"],
            json!(refresh.session_token)
        );
    }

    #[test]
    fn legacy_wrong_alias_does_not_hide_the_canonical_session() {
        let mut account = password_account();
        account.windsurf_auth_token = account.windsurf_api_key.clone();
        assert_eq!(
            resolve_session_token(&account),
            Some(refresh_result().session_token)
        );
        account.devin_session_token = None;
        assert_eq!(resolve_session_token(&account), account.windsurf_auth_token);
    }

    #[test]
    fn refresh_merges_only_credentials_and_preserves_current_user_metadata() {
        let snapshot = password_account();
        let mut current = snapshot.clone();
        current.github_name = Some("Renamed while refreshing".to_string());
        current.tags = Some(vec!["keep".to_string()]);
        current.created_at = 12;
        current.last_used = 34;
        let mut refresh = refresh_result();
        refresh.auth1_token = "auth1_rotated".to_string();
        refresh.session_token = "devin-session-token$new-session".to_string();
        let merged = merge_auth1_refresh(&snapshot, current.clone(), &refresh).unwrap();
        assert_eq!(merged.id, snapshot.id);
        assert_eq!(merged.github_name, current.github_name);
        assert_eq!(merged.tags, current.tags);
        assert_eq!(merged.created_at, 12);
        assert_eq!(merged.last_used, 34);
        assert_eq!(merged.devin_auth1_token, Some(refresh.auth1_token));
        assert_eq!(resolve_session_token(&merged), Some(refresh.session_token));
    }

    #[test]
    fn refresh_rejects_replaced_credentials_or_another_server_identity() {
        let snapshot = password_account();
        let mut current = snapshot.clone();
        current.devin_auth1_token = Some("auth1_replaced".to_string());
        assert!(merge_auth1_refresh(&snapshot, current, &refresh_result()).is_err());
        let mut refresh = refresh_result();
        refresh.account_id = "another-account".to_string();
        assert!(merge_auth1_refresh(&snapshot, snapshot.clone(), &refresh).is_err());
    }

    #[test]
    fn organization_change_clears_stale_status_without_changing_account_id() {
        let snapshot = password_account();
        let mut current = snapshot.clone();
        current.windsurf_user_status = Some(json!({ "old": true }));
        current.windsurf_plan_status = Some(json!({ "oldPlan": true }));
        current.devin_user_status_proto_b64 = Some("old-proto".to_string());
        current.windsurf_auth_status_raw.as_mut().unwrap()["userStatusProtoBinaryBase64"] =
            json!("old-proto");
        let mut refresh = refresh_result();
        refresh.org_id = "new-org".to_string();
        let merged = merge_auth1_refresh(&snapshot, current, &refresh).unwrap();
        assert_eq!(merged.id, snapshot.id);
        assert!(merged.windsurf_user_status.is_none());
        assert!(merged.windsurf_plan_status.is_none());
        assert!(merged.devin_user_status_proto_b64.is_none());
        assert!(merged
            .windsurf_auth_status_raw
            .unwrap()
            .get("userStatusProtoBinaryBase64")
            .is_none());
    }

    #[test]
    fn equal_session_and_ide_values_are_not_rejected() {
        let snapshot = password_account();
        let mut refresh = refresh_result();
        refresh.ide_token = refresh.session_token.clone();
        assert!(merge_auth1_refresh(&snapshot, snapshot.clone(), &refresh).is_ok());
    }

    #[tokio::test]
    async fn session_only_and_legacy_accounts_do_not_attempt_auth1_refresh() {
        for token in ["devin-session-token$fixture", "sk-ws-fixture", "cog_fixture"] {
            let account = new_token_account(token.to_string(), None).unwrap();
            let original = serde_json::to_value(&account).unwrap();
            let result = refresh_account_with(
                account,
                |_| async { Err(AppError::Message("must not refresh".to_string())) },
                |_, _| panic!("must not persist a no-refresh account"),
            )
            .await
            .unwrap();
            assert_eq!(serde_json::to_value(result).unwrap(), original);
        }
    }

    #[tokio::test]
    async fn failed_refresh_does_not_persist_or_return_stale_credentials() {
        let result = refresh_account_with(
            password_account(),
            |_| async { Err(AppError::Message("offline".to_string())) },
            |_, _| panic!("failed refresh must not persist"),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn deleted_account_is_not_resurrected_after_refresh() {
        let result = refresh_account_with(
            password_account(),
            |_| async { Ok(refresh_result()) },
            |_, _| Err(refresh_target_changed()),
        )
        .await;
        assert!(result.is_err());
    }
}
