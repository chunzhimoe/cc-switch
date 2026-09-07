//! Prepare one explicit account switch before closing the client or writing its database.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, TryLockError};

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::Provider;
use crate::services::mcp::McpService;
use crate::services::provider::SwitchResult;
use crate::store::AppState;

use super::account::{self, WindsurfAccount};
use super::inject::{self, PreparedInjection};
use super::{paths, process};

static SWITCH_LOCK: Mutex<()> = Mutex::new(());

// This context stays on the blocking worker. In particular, neither account
// credentials nor the operation's encryption key can be serialized or logged.
pub(crate) struct PreparedSwitch {
    provider_id: String,
    account: WindsurfAccount,
    profile_dir: PathBuf,
    launch_path: PathBuf,
    injection: PreparedInjection,
    was_running: bool,
    _guard: MutexGuard<'static, ()>,
}

fn acquire_switch_lock(lock: &Mutex<()>) -> Result<MutexGuard<'_, ()>, AppError> {
    match lock.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(AppError::localized(
            "windsurf.switch_in_progress",
            "另一个 Windsurf 账号切换正在进行，请等待完成后重试",
            "Another Windsurf account switch is already in progress",
        )),
        Err(TryLockError::Poisoned(_)) => Err(AppError::Lock(
            "Windsurf switch lock is unavailable".to_string(),
        )),
    }
}

fn provider_account_id(provider: &Provider) -> Result<&str, AppError> {
    provider
        .settings_config
        .get("accountId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::InvalidInput("Windsurf provider is missing accountId".to_string()))
}

fn same_credentials(expected: &WindsurfAccount, actual: &WindsurfAccount) -> bool {
    expected.id == actual.id
        && account::resolve_session_token(expected) == account::resolve_session_token(actual)
        && account::resolve_api_key(expected) == account::resolve_api_key(actual)
        && expected.devin_account_id == actual.devin_account_id
        && expected.devin_org_id == actual.devin_org_id
        && expected.windsurf_api_server_url == actual.windsurf_api_server_url
}

/// Called on a blocking worker, never while holding an account-file lock or a
/// SQLite transaction. The runtime drives the existing asynchronous auth flow.
pub(crate) fn prepare_switch(
    state: &AppState,
    provider_id: &str,
    will_close_client: bool,
) -> Result<PreparedSwitch, AppError> {
    let guard = acquire_switch_lock(&SWITCH_LOCK)?;
    let provider = state
        .db
        .get_provider_by_id(provider_id, AppType::Windsurf.as_str())?
        .ok_or_else(|| AppError::InvalidInput("Windsurf provider no longer exists".to_string()))?;
    let account_id = provider_account_id(&provider)?;
    let profile_dir = paths::user_data_dir()?;
    let db_path = paths::state_db_under(&profile_dir);
    if !db_path.is_file() {
        return Err(AppError::localized(
            "windsurf.state_db_missing",
            "目标 Devin/Windsurf 尚未初始化用户数据目录，请先手动打开客户端",
            "The target Devin/Windsurf user-data directory has not been initialized",
        ));
    }
    let launch_path = process::detect_and_save_launch_path(false)?
        .ok_or_else(|| AppError::Message("APP_PATH_NOT_FOUND:windsurf".to_string()))?;
    process::validate_launch_profile(&launch_path, &profile_dir)?;

    // Generic provider/tray switches never silently gain permission to close
    // the client. The account panel supplies its existing close/restart wrapper.
    #[cfg(target_os = "macos")]
    if !will_close_client {
        process::ensure_stopped_for(&profile_dir)?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = will_close_client;

    let account = tauri::async_runtime::block_on(account::refresh_account_for_switch(account_id))?;
    let injection = inject::prepare_injection(&account, &profile_dir, &launch_path)?;
    let was_running = process::is_running_for(&profile_dir);
    let prepared = PreparedSwitch {
        provider_id: provider_id.to_string(),
        account,
        profile_dir,
        launch_path,
        injection,
        was_running,
        _guard: guard,
    };
    prepared.validate_target(state)?;
    Ok(prepared)
}

impl PreparedSwitch {
    pub(crate) fn account_id(&self) -> &str {
        &self.account.id
    }

    pub(crate) fn was_running(&self) -> bool {
        self.was_running
    }

    fn validate_target(&self, state: &AppState) -> Result<(), AppError> {
        let provider = state
            .db
            .get_provider_by_id(&self.provider_id, AppType::Windsurf.as_str())?
            .ok_or_else(target_changed)?;
        if provider_account_id(&provider)? != self.account.id
            || paths::user_data_dir()? != self.profile_dir
            || crate::settings::get_windsurf_app_path().as_deref()
                != Some(self.launch_path.as_path())
        {
            return Err(target_changed());
        }
        let current = account::load_account(&self.account.id)?.ok_or_else(target_changed)?;
        if !same_credentials(&self.account, &current) {
            return Err(target_changed());
        }
        Ok(())
    }

    pub(crate) fn close(&self, state: &AppState) -> Result<(), AppError> {
        self.validate_target(state)?;
        process::close_for(&self.profile_dir, 10)
    }

    pub(crate) fn start(&self) -> Result<u32, AppError> {
        process::start_with(&self.launch_path, &self.profile_dir)
    }

    pub(crate) fn commit(&self, state: &AppState) -> Result<SwitchResult, AppError> {
        self.validate_target(state)?;
        inject::inject_prepared(&self.injection)?;

        // From here on the client DB has committed; report bookkeeping errors
        // honestly instead of claiming the authentication write was rolled back.
        crate::settings::set_current_provider(&AppType::Windsurf, Some(&self.provider_id))
            .map_err(registration_error)?;
        state
            .db
            .set_current_provider(AppType::Windsurf.as_str(), &self.provider_id)
            .map_err(registration_error)?;
        account::mark_last_used(&self.account.id).map_err(registration_error)?;

        let mut result = SwitchResult::default();
        if let Err(error) = McpService::sync_enabled_for_app(state, &AppType::Windsurf) {
            log::warn!("Windsurf MCP sync failed after account switch: {error}");
            result.warnings.push("windsurf_mcp_sync_failed".to_string());
        }
        Ok(result)
    }
}

fn target_changed() -> AppError {
    AppError::localized(
        "windsurf.switch_target_changed",
        "账号或目标路径在切换准备期间发生变化，请重试",
        "The account or target paths changed while preparing the switch; please retry",
    )
}

fn registration_error(error: AppError) -> AppError {
    AppError::localized(
        "windsurf.switch_registration_failed",
        format!("登录态已写入，但保存当前账号记录失败：{error}"),
        "Login state was written, but current-account bookkeeping failed",
    )
}

pub(crate) fn switch_provider(
    state: &AppState,
    provider_id: &str,
) -> Result<SwitchResult, AppError> {
    prepare_switch(state, provider_id, false)?.commit(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_pointer_is_not_assumed_to_equal_provider_id() {
        let provider = Provider::with_id(
            "provider-alias".to_string(),
            "Alias".to_string(),
            json!({ "accountId": " account-one " }),
            None,
        );
        assert_eq!(provider_account_id(&provider).unwrap(), "account-one");
        let mut invalid = provider;
        invalid.settings_config = json!({ "accountId": "  " });
        assert!(provider_account_id(&invalid).is_err());
    }

    #[test]
    fn repeated_switch_fails_instead_of_queuing_a_second_refresh() {
        let lock = Mutex::new(());
        let first = acquire_switch_lock(&lock).expect("first switch");
        assert!(acquire_switch_lock(&lock).is_err());
        drop(first);
        assert!(acquire_switch_lock(&lock).is_ok());
    }

    #[test]
    fn changed_credentials_cannot_be_written_from_stale_preparation() {
        let mut account = account::new_token_account(
            "devin-session-token$test-session".to_string(),
            Some("Test".to_string()),
        )
        .expect("test account");
        let original = account.clone();
        account.github_name = Some("Renamed".to_string());
        assert!(same_credentials(&original, &account));
        account.devin_session_token = Some("devin-session-token$new-session".to_string());
        assert!(!same_credentials(&original, &account));
    }

    #[test]
    fn post_commit_error_does_not_claim_rollback() {
        let error = registration_error(AppError::Database("unavailable".to_string()));
        assert!(error.to_string().contains("Login state was written"));
    }
}
