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

pub fn list_accounts() -> Result<Vec<WindsurfAccount>, AppError> {
    let index = load_index()?;
    index
        .accounts
        .iter()
        .filter_map(|summary| match load_account(&summary.id) {
            Ok(Some(account)) => Some(Ok(account)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
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
    let Some(mut account) = load_account(account_id)? else {
        return Err(AppError::Message(format!(
            "Windsurf account not found: {account_id}"
        )));
    };
    account.last_used = Utc::now().timestamp();
    upsert_account(account).map(|_| ())
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
        "apiKey": refresh.ide_token,
        "sessionToken": refresh.ide_token,
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
        windsurf_auth_token: Some(refresh.ide_token.clone()),
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
        account.windsurf_auth_token.as_deref(),
        account.devin_session_token.as_deref(),
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
