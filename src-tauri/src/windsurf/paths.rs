use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};

use crate::error::AppError;

const AUTH_STATUS_KEY: &str = "windsurfAuthStatus";
const EXTENSION_STATE_KEY: &str = "codeium.windsurf";

pub fn user_data_candidates() -> Result<Vec<PathBuf>, AppError> {
    if let Some(path) = crate::settings::get_windsurf_user_data_dir() {
        return Ok(vec![path]);
    }

    let base = dirs::config_dir().ok_or_else(|| {
        AppError::localized(
            "config_dir_not_found",
            "无法确定 Windsurf 用户数据目录",
            "Cannot determine the Windsurf user-data directory",
        )
    })?;
    Ok(vec![base.join("Devin"), base.join("Windsurf")])
}

pub fn user_data_dir() -> Result<PathBuf, AppError> {
    let candidates = user_data_candidates()?;
    Ok(candidates
        .iter()
        .max_by_key(|path| user_data_score(path))
        .cloned()
        .unwrap_or_else(|| dirs::config_dir().unwrap_or_default().join("Windsurf")))
}

pub fn state_db_under(user_data_dir: &Path) -> PathBuf {
    user_data_dir
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

pub fn state_db_path() -> Result<PathBuf, AppError> {
    Ok(state_db_under(&user_data_dir()?))
}

pub fn rules_path() -> Result<PathBuf, AppError> {
    dirs::home_dir()
        .map(|home| {
            home.join(".codeium")
                .join("windsurf")
                .join("memories")
                .join("global_rules.md")
        })
        .ok_or_else(|| {
            AppError::localized(
                "home_dir_not_found",
                "无法确定 Windsurf Rules 目录：用户主目录不存在",
                "Cannot determine Windsurf Rules directory: user home not found",
            )
        })
}

pub fn rules_dir() -> Result<PathBuf, AppError> {
    rules_path()?
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| AppError::Config("Invalid Windsurf Rules path".to_string()))
}

fn user_data_score(path: &Path) -> i32 {
    let db_path = state_db_under(path);
    if !db_path.is_file() {
        return if path.is_dir() { 1 } else { 0 };
    }

    let Ok(conn) = Connection::open(&db_path) else {
        return 10;
    };
    let mut score = 10;
    if conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [AUTH_STATUS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
    {
        score += 100;
    }
    if conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [EXTENSION_STATE_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
    {
        score += 20;
    }
    score
}
