use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};

use crate::error::AppError;

const AUTH_STATUS_KEY: &str = "windsurfAuthStatus";
const EXTENSION_STATE_KEY: &str = "codeium.windsurf";

pub fn user_data_candidates() -> Result<Vec<PathBuf>, AppError> {
    if let Some(path) = crate::settings::get_windsurf_user_data_dir() {
        return Ok(vec![path]);
    }

    default_user_data_candidates()
}

pub fn default_user_data_candidates() -> Result<Vec<PathBuf>, AppError> {
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
    if crate::settings::get_windsurf_user_data_dir().is_some() {
        return user_data_candidates().map(|candidates| {
            candidates
                .into_iter()
                .next()
                .unwrap_or_else(|| PathBuf::from("Windsurf"))
        });
    }

    default_user_data_dir()
}

pub fn default_user_data_dir() -> Result<PathBuf, AppError> {
    let candidates = default_user_data_candidates()?;
    Ok(pick_preferred_user_data_dir(&candidates))
}

fn pick_preferred_user_data_dir(candidates: &[PathBuf]) -> PathBuf {
    let Some(first) = candidates.first() else {
        return dirs::config_dir().unwrap_or_default().join("Windsurf");
    };

    let mut best = first.clone();
    let mut best_score = user_data_score(first);
    for candidate in candidates.iter().skip(1) {
        let score = user_data_score(candidate);
        if score > best_score {
            best = candidate.clone();
            best_score = score;
        }
    }
    best
}

pub fn skills_dir() -> Result<PathBuf, AppError> {
    Ok(crate::settings::get_windsurf_skills_dir().unwrap_or(default_skills_dir()))
}

pub fn default_skills_dir() -> PathBuf {
    crate::config::get_home_dir()
        .join(".codeium")
        .join("windsurf")
        .join("skills")
}

pub fn rules_path() -> Result<PathBuf, AppError> {
    Ok(rules_dir()?.join("global_rules.md"))
}

pub fn rules_dir() -> Result<PathBuf, AppError> {
    Ok(crate::settings::get_windsurf_rules_dir().unwrap_or_else(default_rules_dir))
}

pub fn default_rules_dir() -> PathBuf {
    crate::config::get_home_dir()
        .join(".codeium")
        .join("windsurf")
        .join("memories")
}

pub fn state_db_under(profile_dir: &Path) -> PathBuf {
    profile_dir
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

pub fn state_db_path() -> Result<PathBuf, AppError> {
    Ok(state_db_under(&user_data_dir()?))
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
    if let Ok(Some(raw)) = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [EXTENSION_STATE_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
    {
        score += 20;
        if serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|value| {
                value
                    .get("codeium.installationId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .is_some()
        {
            score += 100;
        }
    }
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
        score += 30;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_state_db(profile: &Path, extension_state: Option<&str>, has_auth: bool) {
        let db_path = state_db_under(profile);
        std::fs::create_dir_all(db_path.parent().expect("state db parent"))
            .expect("create state db parent");
        let conn = Connection::open(db_path).expect("open test state db");
        conn.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB)")
            .expect("create ItemTable");
        if let Some(extension_state) = extension_state {
            conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                (EXTENSION_STATE_KEY, extension_state),
            )
            .expect("insert extension state");
        }
        if has_auth {
            conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                (AUTH_STATUS_KEY, r#"{"status":"SignedIn"}"#),
            )
            .expect("insert auth status");
        }
    }

    #[test]
    fn prefers_profile_with_stable_installation_id() {
        let temp = TempDir::new().expect("temp dir");
        let devin = temp.path().join("Devin");
        let windsurf = temp.path().join("Windsurf");
        create_state_db(&devin, Some(r#"{"theme":"dark"}"#), true);
        create_state_db(
            &windsurf,
            Some(r#"{"codeium.installationId":"stable-id"}"#),
            false,
        );

        assert_eq!(
            pick_preferred_user_data_dir(&[devin, windsurf.clone()]),
            windsurf
        );
    }

    #[test]
    fn preserves_candidate_order_when_scores_tie() {
        let temp = TempDir::new().expect("temp dir");
        let devin = temp.path().join("Devin");
        let windsurf = temp.path().join("Windsurf");
        std::fs::create_dir_all(&devin).expect("create Devin");
        std::fs::create_dir_all(&windsurf).expect("create Windsurf");

        assert_eq!(
            pick_preferred_user_data_dir(&[devin.clone(), windsurf]),
            devin
        );
    }
}
