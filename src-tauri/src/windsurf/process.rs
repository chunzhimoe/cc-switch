use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::error::AppError;

pub fn detect_and_save_launch_path(force: bool) -> Result<Option<PathBuf>, AppError> {
    if !force {
        if let Some(configured) = crate::settings::get_windsurf_app_path() {
            if valid_launch_path(&configured) {
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

pub fn close(timeout_secs: u64) -> Result<(), AppError> {
    let pids = collect_main_processes()
        .into_iter()
        .map(|(pid, _)| pid)
        .collect::<Vec<_>>();
    if pids.is_empty() {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    for pid in &pids {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    for pid in &pids {
        let _ = Command::new("kill")
            .args(["-15", &pid.to_string()])
            .status();
    }

    if wait_for_exit(&pids, Duration::from_secs(timeout_secs.min(10))) {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    for pid in &pids {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    for pid in &pids {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }

    if wait_for_exit(&pids, Duration::from_secs(3)) {
        Ok(())
    } else {
        Err(AppError::Message(format!(
            "Unable to close Windsurf processes: {}",
            pids.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

pub fn start() -> Result<u32, AppError> {
    let executable = ensure_launch_path()?;
    let user_data_dir = super::paths::user_data_dir()?;
    let mut command = Command::new(&executable);
    command
        .arg("--user-data-dir")
        .arg(&user_data_dir)
        .arg("--reuse-window")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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
    for (_, path) in collect_main_processes() {
        if let Some(path) = path {
            if valid_launch_path(&path) {
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
                    if valid_launch_path(&candidate) {
                        return Some(candidate);
                    }
                }
            }
            if let Some(candidate) = scan_programs(&programs, 2) {
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
        if valid_launch_path(&candidate) {
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
        if valid_launch_path(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn collect_main_processes() -> Vec<(u32, Option<PathBuf>)> {
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
            let main = (name == "windsurf.exe"
                || name == "devin.exe"
                || name == "windsurf"
                || name == "devin"
                || (name == "electron.exe"
                    && (executable_text.contains("windsurf")
                        || executable_text.contains("devin"))))
                && !command.contains("--type=");
            main.then_some((pid, executable))
        })
        .collect()
}

fn wait_for_exit(pids: &[u32], timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        let running = collect_main_processes()
            .iter()
            .any(|(pid, _)| pids.contains(pid));
        if !running {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn valid_launch_path(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let text = path.to_string_lossy().to_ascii_lowercase();
    text.contains("windsurf") || text.contains("devin")
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
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(name.as_str(), "windsurf.exe" | "devin.exe" | "electron.exe")
            && valid_launch_path(&path)
        {
            return Some(path);
        }
    }
    None
}
