use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager, State};

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::services::ProviderService;
use crate::store::AppState;
use crate::windsurf::account::{
    self, new_account_from_auth1_refresh, new_token_account, resolve_api_key,
    resolve_session_token, WindsurfAccount, WindsurfAccountSummary,
};
use crate::windsurf::browser_oauth::{self, WindsurfOAuthStartResponse};
use crate::windsurf::{auth_write, devin_oauth, local_import, paths, process};

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
        new_account_from_auth1_refresh(None, label, &refresh.auth1_token, &refresh)
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
    let account =
        new_account_from_auth1_refresh(Some(email), label, &refresh.auth1_token, &refresh);
    let account = account::upsert_account(account).map_err(|error| error.to_string())?;
    save_provider_pointer(state.inner(), &account).map_err(|error| error.to_string())?;
    Ok(account.summary())
}

#[tauri::command]
pub async fn windsurf_oauth_login_start() -> Result<WindsurfOAuthStartResponse, String> {
    browser_oauth::start_login().await
}

#[tauri::command]
pub async fn windsurf_oauth_login_complete(
    state: State<'_, AppState>,
    login_id: String,
) -> Result<WindsurfAccountSummary, String> {
    let account = browser_oauth::complete_login(&login_id).await?;
    let account = account::upsert_account(account).map_err(|error| error.to_string())?;
    save_provider_pointer(state.inner(), &account).map_err(|error| error.to_string())?;
    Ok(account.summary())
}

#[tauri::command]
pub fn windsurf_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    browser_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn windsurf_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    browser_oauth::submit_callback_url(&login_id, &callback_url)
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
        let account = account::load_account(&account_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Windsurf account not found: {account_id}"))?;
        let access_token = resolve_session_token(&account)
            .ok_or_else(|| "Windsurf account does not contain a usable token".to_string())?;
        if !access_token.starts_with("devin-session-token$") && resolve_api_key(&account).is_none()
        {
            return Err("Windsurf account does not contain an apiKey".to_string());
        }

        let profile_dir = paths::user_data_dir().map_err(|error| error.to_string())?;
        let state_db_path = paths::state_db_under(&profile_dir);
        if !state_db_path.is_file() {
            return Err(format!(
                "Windsurf state.vscdb was not found: {}",
                state_db_path.display()
            ));
        }
        // Preflight before closing Windsurf or mutating state.vscdb. A failure
        // here is not a completed account switch, even with a launch warning.
        let launch_path = process::detect_and_save_launch_path(false)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "APP_PATH_NOT_FOUND:windsurf".to_string())?;
        process::validate_launch_profile(&launch_path, &profile_dir)
            .map_err(|error| error.to_string())?;
        auth_write::validate_profile_encryption(&profile_dir).map_err(|error| error.to_string())?;

        let was_running = process::is_running_for(&profile_dir);
        switch_with_restart(
            &account_id,
            was_running,
            || process::close_for(&profile_dir, 10).map_err(|error| error.to_string()),
            || {
                ProviderService::switch(state.inner(), AppType::Windsurf, &account_id)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
            || process::start_with(&launch_path, &profile_dir).map_err(|error| error.to_string()),
        )
    })
    .await
    .map_err(|error| format!("Windsurf switch task failed: {error}"))?
}

fn switch_with_restart(
    account_id: &str,
    was_running: bool,
    close: impl FnOnce() -> Result<(), String>,
    write: impl FnOnce() -> Result<(), String>,
    mut start: impl FnMut() -> Result<u32, String>,
) -> Result<WindsurfSwitchResult, String> {
    // Always rescan and confirm shutdown, even if the earlier snapshot was empty.
    close()?;
    if let Err(error) = write() {
        if was_running {
            if let Err(restart_error) = start() {
                log::warn!("Windsurf recovery launch failed after switch failure: {restart_error}");
                return Err(format!(
                    "{error}; Windsurf could not be restarted after the failed switch: {restart_error}"
                ));
            }
        }
        return Err(error);
    }

    match start() {
        Ok(process_id) => Ok(WindsurfSwitchResult {
            account_id: account_id.to_string(),
            restarted: true,
            process_id: Some(process_id),
            warning: None,
        }),
        Err(error) => Ok(WindsurfSwitchResult {
            account_id: account_id.to_string(),
            restarted: false,
            process_id: None,
            warning: Some(error),
        }),
    }
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
        if !process::is_valid_launch_path(candidate) {
            return Err(
                "Selected path is not a Windsurf/Devin executable or app bundle".to_string(),
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn switch_closes_before_writing_even_if_initially_not_running() {
        let events = RefCell::new(Vec::new());
        let result = switch_with_restart(
            "account-b",
            false,
            || {
                events.borrow_mut().push("close");
                Ok(())
            },
            || {
                events.borrow_mut().push("write");
                Ok(())
            },
            || {
                events.borrow_mut().push("start");
                Ok(42)
            },
        )
        .expect("switch succeeds");

        assert_eq!(*events.borrow(), vec!["close", "write", "start"]);
        assert_eq!(result.account_id, "account-b");
        assert!(result.restarted);
        assert_eq!(result.process_id, Some(42));
        assert!(result.warning.is_none());
    }

    #[test]
    fn close_failure_never_writes_or_starts() {
        let error = switch_with_restart(
            "account-b",
            true,
            || Err("still running".to_string()),
            || panic!("must not write while the old process is running"),
            || panic!("must not launch a second instance"),
        )
        .expect_err("close must fail");
        assert_eq!(error, "still running");
    }

    #[test]
    fn write_failure_recovers_previously_running_app_without_claiming_success() {
        let events = RefCell::new(Vec::new());
        let error = switch_with_restart(
            "account-b",
            true,
            || Ok(()),
            || Err("write failed".to_string()),
            || {
                events.borrow_mut().push("recover");
                Ok(42)
            },
        )
        .expect_err("write failure is not a completed switch");
        assert_eq!(error, "write failed");
        assert_eq!(*events.borrow(), vec!["recover"]);
    }

    #[test]
    fn reports_recovery_failure_alongside_write_error() {
        let error = switch_with_restart(
            "account-b",
            true,
            || Ok(()),
            || Err("write failed".to_string()),
            || Err("launch failed".to_string()),
        )
        .expect_err("write and recovery fail");
        assert!(error.contains("write failed"));
        assert!(error.contains("launch failed"));
    }

    #[test]
    fn write_failure_does_not_start_previously_stopped_app() {
        let error = switch_with_restart(
            "account-b",
            false,
            || Ok(()),
            || Err("write failed".to_string()),
            || panic!("previously stopped app should stay stopped"),
        )
        .expect_err("write fails");
        assert_eq!(error, "write failed");
    }

    #[test]
    fn launch_failure_after_write_is_partial_success() {
        let result = switch_with_restart(
            "account-b",
            true,
            || Ok(()),
            || Ok(()),
            || Err("no matching app process".to_string()),
        )
        .expect("auth was written");
        assert!(!result.restarted);
        assert!(result.process_id.is_none());
        assert_eq!(result.warning.as_deref(), Some("no matching app process"));
    }
}
