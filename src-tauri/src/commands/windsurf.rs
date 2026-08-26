use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager, State};

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::services::ProviderService;
use crate::store::AppState;
use crate::windsurf::account::{
    self, new_account_from_auth1_refresh, new_token_account, WindsurfAccount,
    WindsurfAccountSummary,
};
use crate::windsurf::{devin_oauth, local_import, paths, process};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfSwitchResult {
    pub account_id: String,
    pub restarted: bool,
    pub process_id: Option<u32>,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfStatus {
    pub current_account_id: Option<String>,
    pub running: bool,
    pub app_path: Option<String>,
    pub user_data_dir: String,
    pub state_db_path: String,
    pub mcp_config_path: Option<String>,
    pub rules_path: String,
}

#[tauri::command]
pub fn list_windsurf_accounts() -> Result<Vec<WindsurfAccountSummary>, String> {
    account::list_account_summaries().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn import_windsurf_from_local(
    state: State<'_, AppState>,
) -> Result<WindsurfAccountSummary, String> {
    let account = local_import::import_local_account().map_err(|error| error.to_string())?;
    save_provider_pointer(state.inner(), &account).map_err(|error| error.to_string())?;
    state
        .db
        .set_current_provider(AppType::Windsurf.as_str(), &account.id)
        .map_err(|error| error.to_string())?;
    crate::settings::set_current_provider(&AppType::Windsurf, Some(&account.id))
        .map_err(|error| error.to_string())?;
    Ok(account.summary())
}

#[tauri::command]
pub async fn add_windsurf_account_with_token(
    state: State<'_, AppState>,
    token: String,
    label: Option<String>,
) -> Result<WindsurfAccountSummary, String> {
    let trimmed = token.trim().to_string();
    let account = if trimmed.starts_with("auth1_") {
        let refresh = devin_oauth::full_refresh_from_auth1(&trimmed).await?;
        // Best-effort quota enrichment; failures should not block login.
        let mut refresh = refresh;
        if refresh.user_status.is_none() {
            match devin_oauth::fetch_user_status(&refresh.ide_token).await {
                Ok(status) => refresh.user_status = Some(status),
                Err(error) => {
                    log::warn!("Windsurf GetUserStatus failed after auth1 refresh: {error}")
                }
            }
        }
        new_account_from_auth1_refresh(None, label, &trimmed, &refresh)
    } else {
        new_token_account(trimmed, label).map_err(|error| error.to_string())?
    };
    let account = account::upsert_account(account).map_err(|error| error.to_string())?;
    save_provider_pointer(state.inner(), &account).map_err(|error| error.to_string())?;
    Ok(account.summary())
}

#[tauri::command]
pub async fn add_windsurf_account_with_password(
    state: State<'_, AppState>,
    email: String,
    password: String,
    label: Option<String>,
) -> Result<WindsurfAccountSummary, String> {
    let login = devin_oauth::login_with_password(&email, &password).await?;
    let mut refresh = devin_oauth::full_refresh_from_auth1(&login.auth1_token).await?;
    if refresh.user_status.is_none() {
        match devin_oauth::fetch_user_status(&refresh.ide_token).await {
            Ok(status) => refresh.user_status = Some(status),
            Err(error) => log::warn!("Windsurf GetUserStatus failed after password login: {error}"),
        }
    }
    let email = login
        .email
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| email.trim().to_string());
    let account = new_account_from_auth1_refresh(Some(email), label, &login.auth1_token, &refresh);
    let account = account::upsert_account(account).map_err(|error| error.to_string())?;
    save_provider_pointer(state.inner(), &account).map_err(|error| error.to_string())?;
    Ok(account.summary())
}

#[tauri::command]
pub fn delete_windsurf_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<bool, String> {
    let current = crate::settings::get_current_provider(&AppType::Windsurf);
    state
        .db
        .delete_provider(AppType::Windsurf.as_str(), &account_id)
        .map_err(|error| error.to_string())?;
    let deleted = account::delete_account(&account_id).map_err(|error| error.to_string())?;
    if current.as_deref() == Some(account_id.as_str()) {
        crate::settings::set_current_provider(&AppType::Windsurf, None)
            .map_err(|error| error.to_string())?;
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn switch_windsurf_account(
    app: AppHandle,
    account_id: String,
) -> Result<WindsurfSwitchResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| "Application state is unavailable".to_string())?;
        let _account = account::load_account(&account_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Windsurf account not found: {account_id}"))?;

        // Detect while the process is still running; its executable path is the
        // strongest discovery signal and is lost after shutdown.
        let launch_path =
            process::detect_and_save_launch_path(false).map_err(|error| error.to_string())?;
        let was_running = process::is_running();
        if was_running {
            process::close(10).map_err(|error| error.to_string())?;
        }

        if let Err(error) = ProviderService::switch(state.inner(), AppType::Windsurf, &account_id) {
            if was_running && launch_path.is_some() {
                let _ = process::start();
            }
            return Err(error.to_string());
        }

        if launch_path.is_none() {
            return Ok(WindsurfSwitchResult {
                account_id,
                restarted: false,
                process_id: None,
                warning: Some("APP_PATH_NOT_FOUND:windsurf".to_string()),
            });
        }

        match process::start() {
            Ok(process_id) => Ok(WindsurfSwitchResult {
                account_id,
                restarted: true,
                process_id: Some(process_id),
                warning: None,
            }),
            Err(error) => Ok(WindsurfSwitchResult {
                account_id,
                restarted: false,
                process_id: None,
                warning: Some(error.to_string()),
            }),
        }
    })
    .await
    .map_err(|error| format!("Windsurf switch task failed: {error}"))?
}

#[tauri::command]
pub fn detect_windsurf_app_path(force: Option<bool>) -> Result<Option<String>, String> {
    process::detect_and_save_launch_path(force.unwrap_or(false))
        .map(|path| path.map(|path| path.to_string_lossy().to_string()))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_windsurf_app_path(path: Option<String>) -> Result<(), String> {
    if let Some(path) = path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let candidate = std::path::Path::new(path);
        let lower = path.to_ascii_lowercase();
        if !candidate.is_file() || (!lower.contains("windsurf") && !lower.contains("devin")) {
            return Err("Selected file is not a Windsurf/Devin executable".to_string());
        }
    }
    crate::settings::set_windsurf_app_path(path.as_deref()).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_windsurf_user_data_dir(path: Option<String>) -> Result<(), String> {
    crate::settings::set_windsurf_user_data_dir(path.as_deref()).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_windsurf_status(state: State<'_, AppState>) -> Result<WindsurfStatus, String> {
    let user_data_dir = paths::user_data_dir().map_err(|error| error.to_string())?;
    let state_db_path = paths::state_db_path().map_err(|error| error.to_string())?;
    let rules_path = paths::rules_path().map_err(|error| error.to_string())?;
    let mcp_config_path = crate::mcp::get_windsurf_mcp_config_path()
        .ok()
        .map(|path| path.to_string_lossy().to_string());
    let current_account_id =
        crate::settings::get_effective_current_provider(state.db.as_ref(), &AppType::Windsurf)
            .map_err(|error| error.to_string())?;

    Ok(WindsurfStatus {
        current_account_id,
        running: process::is_running(),
        app_path: crate::settings::get_windsurf_app_path()
            .map(|path| path.to_string_lossy().to_string()),
        user_data_dir: user_data_dir.to_string_lossy().to_string(),
        state_db_path: state_db_path.to_string_lossy().to_string(),
        mcp_config_path,
        rules_path: rules_path.to_string_lossy().to_string(),
    })
}

fn save_provider_pointer(
    state: &AppState,
    account: &WindsurfAccount,
) -> Result<(), crate::error::AppError> {
    let summary = account.summary();
    let mut provider = Provider::with_id(
        account.id.clone(),
        summary.label,
        json!({
            "accountId": account.id,
            "tokenType": summary.token_type,
            "email": summary.email,
            "maskedToken": summary.masked_token,
        }),
        None,
    );
    provider.category = Some("windsurf-account".to_string());
    provider.icon = Some("windsurf".to_string());
    state
        .db
        .save_provider(AppType::Windsurf.as_str(), &provider)
}
