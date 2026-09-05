use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::error::AppError;

#[derive(Clone, Debug)]
struct WindsurfProcess {
    pid: u32,
    executable: Option<PathBuf>,
    user_data_dir: Option<PathBuf>,
}

pub fn detect_and_save_launch_path(force: bool) -> Result<Option<PathBuf>, AppError> {
    if !force {
        if let Some(configured) = crate::settings::get_windsurf_app_path() {
            if is_valid_launch_path(&configured) {
                return Ok(Some(configured));
            }
        }
    }

    let detected = detect_launch_path();
    if let Some(path) = &detected {
        let path_string = path.to_string_lossy().to_string();
        crate::settings::set_windsurf_app_path(Some(&path_string))?;
    }
    Ok(detected)
}

pub fn ensure_launch_path() -> Result<PathBuf, AppError> {
    detect_and_save_launch_path(false)?
        .ok_or_else(|| AppError::Message("APP_PATH_NOT_FOUND:windsurf".to_string()))
}

pub fn is_running() -> bool {
    !collect_main_processes().is_empty()
}

pub fn is_running_for(user_data_dir: &Path) -> bool {
    !matching_processes(user_data_dir).is_empty()
}

pub fn close(timeout_secs: u64) -> Result<(), AppError> {
    let user_data_dir = super::paths::user_data_dir()?;
    close_for(&user_data_dir, timeout_secs)
}

pub fn close_for(user_data_dir: &Path, timeout_secs: u64) -> Result<(), AppError> {
    let mut pids = matching_processes(user_data_dir)
        .into_iter()
        .map(|entry| entry.pid)
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    if pids.is_empty() {
        return Ok(());
    }

    for pid in &pids {
        request_graceful_close(*pid);
    }
    if wait_for_profile_exit(user_data_dir, Duration::from_secs(2)) {
        return Ok(());
    }

    for pid in &pids {
        force_close(*pid);
    }
    if wait_for_profile_exit(user_data_dir, Duration::from_secs(timeout_secs.min(10))) {
        return Ok(());
    }

    let mut remaining = matching_processes(user_data_dir)
        .into_iter()
        .map(|entry| entry.pid)
        .collect::<Vec<_>>();
    remaining.sort_unstable();
    remaining.dedup();
    for pid in &remaining {
        force_close(*pid);
    }
    let _ = wait_for_profile_exit(user_data_dir, Duration::from_secs(6));

    let mut remaining = matching_processes(user_data_dir)
        .into_iter()
        .map(|entry| entry.pid)
        .collect::<Vec<_>>();
    remaining.sort_unstable();
    remaining.dedup();
    if remaining.is_empty() {
        Ok(())
    } else {
        Err(AppError::localized(
            "windsurf.process.close_failed",
            format!(
                "无法关闭 Windsurf 进程，请手动关闭后重试: {}",
                format_pid_list(&remaining)
            ),
            format!(
                "Unable to close Windsurf processes; close them manually and retry: {}",
                format_pid_list(&remaining)
            ),
        ))
    }
}

pub fn start() -> Result<u32, AppError> {
    let executable = ensure_launch_path()?;
    let user_data_dir = super::paths::user_data_dir()?;
    start_with(&executable, &user_data_dir)
}

pub fn start_with(executable: &Path, user_data_dir: &Path) -> Result<u32, AppError> {
    if !is_valid_launch_path(executable) {
        return Err(AppError::Message(format!(
            "APP_PATH_NOT_FOUND:windsurf:{}",
            executable.display()
        )));
    }

    let mut command = Command::new(executable);
    command
        .arg("--user-data-dir")
        .arg(user_data_dir)
        .arg("--reuse-window")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(parent) = executable.parent() {
        command.current_dir(parent);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn().map(|child| child.id()).map_err(|error| {
        AppError::Message(format!(
            "Failed to start Windsurf from {}: {error}",
            executable.display()
        ))
    })
}

fn detect_launch_path() -> Option<PathBuf> {
    for entry in collect_main_processes() {
        if let Some(path) = entry.executable {
            if is_valid_launch_path(&path) {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let programs = PathBuf::from(local_appdata).join("Programs");
            for folder in ["Devin", "Windsurf"] {
                for executable in ["Devin.exe", "Windsurf.exe", "Electron.exe"] {
                    let candidate = programs.join(folder).join(executable);
                    if is_valid_launch_path(&candidate) {
                        return Some(candidate);
                    }
                }
            }
            if let Some(candidate) = scan_programs(&programs, 3) {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "macos")]
    for candidate in [
        "/Applications/Devin.app/Contents/MacOS/Devin",
        "/Applications/Windsurf.app/Contents/MacOS/Electron",
    ] {
        let candidate = PathBuf::from(candidate);
        if is_valid_launch_path(&candidate) {
            return Some(candidate);
        }
    }

    #[cfg(target_os = "linux")]
    for candidate in [
        "/usr/bin/devin",
        "/opt/devin/devin",
        "/usr/bin/windsurf",
        "/opt/windsurf/windsurf",
    ] {
        let candidate = PathBuf::from(candidate);
        if is_valid_launch_path(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn collect_main_processes() -> Vec<WindsurfProcess> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let pid = pid.as_u32();
            if pid == current_pid {
                return None;
            }
            let name = process.name().to_string_lossy().to_ascii_lowercase();
            let executable = process.exe().map(Path::to_path_buf);
            let executable_text = executable
                .as_ref()
                .map(|path| path.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let command = process
                .cmd()
                .iter()
                .map(|value| value.to_string_lossy().to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let supported_name = matches!(
                name.as_str(),
                "windsurf.exe" | "devin.exe" | "windsurf" | "devin"
            ) || (name == "electron.exe"
                && (executable_text.contains("windsurf") || executable_text.contains("devin")));
            if !supported_name || is_helper_process(&name, &command) {
                return None;
            }
            Some(WindsurfProcess {
                pid,
                executable,
                user_data_dir: extract_user_data_dir(process.cmd()),
            })
        })
        .collect()
}

fn matching_processes(user_data_dir: &Path) -> Vec<WindsurfProcess> {
    let target = normalize_path_for_compare(user_data_dir);
    collect_main_processes()
        .into_iter()
        .filter(|entry| process_matches_profile(entry, &target))
        .collect()
}

fn process_matches_profile(entry: &WindsurfProcess, target: &str) -> bool {
    entry
        .user_data_dir
        .as_deref()
        .map(normalize_path_for_compare)
        .map(|path| path == target)
        // A normal single-profile launch may omit --user-data-dir entirely.
        // Treat it as managed so switching cannot write while Windsurf is open.
        .unwrap_or(true)
}

fn extract_user_data_dir(args: &[OsString]) -> Option<PathBuf> {
    let tokens = args
        .iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(value) = token.strip_prefix("--user-data-dir=") {
            return parse_user_data_dir_value(value).map(PathBuf::from);
        }
        if token == "--user-data-dir" {
            return tokens
                .get(index + 1)
                .and_then(|value| parse_user_data_dir_value(value))
                .map(PathBuf::from);
        }
        index += 1;
    }
    None
}

fn parse_user_data_dir_value(raw: &str) -> Option<String> {
    let value = raw.trim().trim_matches(|ch| ch == '"' || ch == '\'');
    (!value.is_empty()).then(|| value.to_string())
}

fn is_helper_process(name: &str, command: &str) -> bool {
    command.contains("--type=")
        || name.contains("helper")
        || name.contains("renderer")
        || name.contains("gpu")
        || name.contains("utility")
        || name.contains("crashpad")
        || name.contains("sandbox")
}

fn normalize_path_for_compare(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = resolved.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    {
        value.to_ascii_lowercase()
    }
    #[cfg(not(target_os = "windows"))]
    {
        value
    }
}

fn request_graceful_close(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill")
            .args(["-15", &pid.to_string()])
            .status();
    }
}

fn force_close(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

fn wait_for_profile_exit(user_data_dir: &Path, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if matching_processes(user_data_dir).is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    matching_processes(user_data_dir).is_empty()
}

fn format_pid_list(pids: &[u32]) -> String {
    pids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn is_valid_launch_path(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let text = path.to_string_lossy().to_ascii_lowercase();
    if !text.contains("windsurf") && !text.contains("devin") {
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        matches!(name.as_str(), "windsurf.exe" | "devin.exe" | "electron.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

#[cfg(target_os = "windows")]
fn scan_programs(root: &Path, depth: usize) -> Option<PathBuf> {
    if depth == 0 || !root.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = scan_programs(&path, depth - 1) {
                return Some(found);
            }
            continue;
        }
        if is_valid_launch_path(&path) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_data_dir_arguments() {
        assert_eq!(
            extract_user_data_dir(&[
                OsString::from("windsurf"),
                OsString::from("--user-data-dir"),
                OsString::from("C:\\Users\\Test User\\AppData\\Roaming\\Windsurf"),
                OsString::from("--reuse-window"),
            ]),
            Some(PathBuf::from(
                "C:\\Users\\Test User\\AppData\\Roaming\\Windsurf"
            ))
        );
        assert_eq!(
            extract_user_data_dir(&[OsString::from("--user-data-dir=\"C:\\Profiles\\Devin\"")]),
            Some(PathBuf::from("C:\\Profiles\\Devin"))
        );
    }

    #[test]
    fn excludes_helper_processes() {
        assert!(is_helper_process("windsurf.exe", "--type=renderer"));
        assert!(is_helper_process("windsurf helper", ""));
        assert!(!is_helper_process("windsurf.exe", "--reuse-window"));
    }

    #[test]
    fn matches_only_target_profile() {
        let entry = WindsurfProcess {
            pid: 42,
            executable: None,
            user_data_dir: Some(PathBuf::from("profile-a")),
        };
        assert!(process_matches_profile(
            &entry,
            &normalize_path_for_compare(Path::new("profile-a"))
        ));
        assert!(!process_matches_profile(
            &entry,
            &normalize_path_for_compare(Path::new("profile-b"))
        ));

        let implicit_default = WindsurfProcess {
            pid: 43,
            executable: None,
            user_data_dir: None,
        };
        assert!(process_matches_profile(
            &implicit_default,
            &normalize_path_for_compare(Path::new("profile-b"))
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn validates_supported_windows_executables() {
        use tempfile::TempDir;

        let temp = TempDir::new().expect("temp dir");
        let windsurf_dir = temp.path().join("Windsurf");
        std::fs::create_dir_all(&windsurf_dir).expect("create app dir");
        let windsurf = windsurf_dir.join("Windsurf.exe");
        let electron = windsurf_dir.join("Electron.exe");
        let other = windsurf_dir.join("Other.exe");
        std::fs::write(&windsurf, []).expect("create Windsurf.exe");
        std::fs::write(&electron, []).expect("create Electron.exe");
        std::fs::write(&other, []).expect("create Other.exe");

        assert!(is_valid_launch_path(&windsurf));
        assert!(is_valid_launch_path(&electron));
        assert!(!is_valid_launch_path(&other));
    }
}
