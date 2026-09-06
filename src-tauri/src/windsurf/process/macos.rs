use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::AppError;

use super::{format_pid_list, is_helper_process, normalize_path_for_compare, WindsurfProcess};

const LAUNCH_ENV_REMOVALS: &[&str] = &[
    "__CFBundleIdentifier",
    "XPC_SERVICE_NAME",
    "NODE_OPTIONS",
    "NODE_PATH",
    "NODE_ENV",
    "npm_config_prefix",
    "npm_config_devdir",
    "ELECTRON_RUN_AS_NODE",
    "ELECTRON_NO_ASAR",
    "ELECTRON_FORCE_WINDOW_MENU_BAR",
    "ELECTRON_NO_ATTACH_CONSOLE",
];

fn bundle_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| {
            ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.eq_ignore_ascii_case("Windsurf.app")
                        || name.eq_ignore_ascii_case("Devin.app")
                })
        })
        .map(Path::to_path_buf)
}

fn bundle_brand(bundle: &Path) -> Option<&'static str> {
    let name = bundle.file_name()?.to_str()?;
    if name.eq_ignore_ascii_case("Windsurf.app") {
        Some("Windsurf")
    } else if name.eq_ignore_ascii_case("Devin.app") {
        Some("Devin")
    } else {
        None
    }
}

fn profile_brand(profile: &Path) -> Option<&'static str> {
    let name = profile.file_name()?.to_str()?.to_ascii_lowercase();
    if name.starts_with("windsurf") {
        Some("Windsurf")
    } else if name.starts_with("devin") {
        Some("Devin")
    } else {
        None
    }
}

fn resolve_bundle(path: &Path) -> Result<PathBuf, AppError> {
    let missing = || AppError::Message(format!("APP_PATH_NOT_FOUND:windsurf:{}", path.display()));
    if !path.exists() {
        return Err(missing());
    }
    let bundle = bundle_root(path).ok_or_else(missing)?;
    let plist = bundle.join("Contents/Info.plist");
    if !bundle.is_dir() || !plist.is_file() {
        return Err(missing());
    }
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleExecutable", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .map_err(|error| {
            AppError::Message(format!("Unable to inspect Windsurf app bundle: {error}"))
        })?;
    let name = String::from_utf8_lossy(&output.stdout);
    let name = name.trim();
    let mut components = Path::new(name).components();
    if !output.status.success()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || !bundle.join("Contents/MacOS").join(name).is_file()
    {
        return Err(missing());
    }
    Ok(bundle)
}

pub fn is_valid_launch_path(path: &Path) -> bool {
    resolve_bundle(path).is_ok()
}

pub fn validate_launch_profile(launch_path: &Path, profile_dir: &Path) -> Result<(), AppError> {
    let bundle = resolve_bundle(launch_path)?;
    if let (Some(app), Some(profile)) = (bundle_brand(&bundle), profile_brand(profile_dir)) {
        if app != profile {
            return Err(AppError::Message(format!(
                "Windsurf app/profile mismatch: {} uses the {app} Keychain, but {} is a {profile} profile. Select matching application and user-data paths.",
                bundle.display(), profile_dir.display()
            )));
        }
    }
    Ok(())
}

pub fn detect_and_save_launch_path(force: bool) -> Result<Option<PathBuf>, AppError> {
    let profile = crate::windsurf::paths::user_data_dir()?;
    if !force {
        if let Some(configured) = crate::settings::get_windsurf_app_path() {
            let bundle = resolve_bundle(&configured)?;
            validate_launch_profile(&bundle, &profile)?;
            crate::settings::set_windsurf_app_path(Some(&bundle.to_string_lossy()))?;
            return Ok(Some(bundle));
        }
    }

    let mut candidates = matching_processes(&profile)?
        .into_iter()
        .filter_map(|entry| entry.executable)
        .collect::<Vec<_>>();
    let brands: &[&str] = match profile_brand(&profile) {
        Some("Windsurf") => &["Windsurf"],
        Some("Devin") => &["Devin"],
        _ => &["Windsurf", "Devin"],
    };
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    for brand in brands {
        for root in &roots {
            candidates.push(root.join(format!("{brand}.app")));
        }
    }
    for candidate in candidates {
        if let Ok(bundle) = resolve_bundle(&candidate) {
            if validate_launch_profile(&bundle, &profile).is_ok() {
                crate::settings::set_windsurf_app_path(Some(&bundle.to_string_lossy()))?;
                return Ok(Some(bundle));
            }
        }
    }
    Ok(None)
}

fn is_main_executable(path: &Path) -> bool {
    let Some(bundle) = bundle_root(path) else {
        return false;
    };
    if path.parent() != Some(bundle.join("Contents/MacOS").as_path()) {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    matches!(
        name.to_ascii_lowercase().as_str(),
        "electron" | "windsurf" | "devin"
    )
}

fn parse_process_line(line: &str) -> Option<WindsurfProcess> {
    let (pid, rest) = line.trim().split_once(char::is_whitespace)?;
    let (state, executable) = rest.trim_start().split_once(char::is_whitespace)?;
    let pid = pid.parse::<u32>().ok()?;
    if state.starts_with('Z') {
        return None;
    }
    let executable = PathBuf::from(executable.trim());
    if !is_main_executable(&executable) {
        return None;
    }
    Some(WindsurfProcess {
        pid,
        executable: Some(executable),
        user_data_dir: None,
    })
}

fn profile_argument(command: &str) -> Result<Option<PathBuf>, AppError> {
    let flag = "--user-data-dir";
    for (index, _) in command.match_indices(flag) {
        if index > 0 && !command[..index].ends_with(char::is_whitespace) {
            continue;
        }
        let remainder = &command[index + flag.len()..];
        let value = if let Some(value) = remainder.strip_prefix('=') {
            value
        } else if remainder.starts_with(char::is_whitespace) || remainder.is_empty() {
            remainder.trim_start()
        } else {
            continue;
        };
        let value = if let Some(quote) = value.chars().next().filter(|ch| *ch == '\'' || *ch == '"')
        {
            let rest = &value[quote.len_utf8()..];
            let end = rest.find(quote).ok_or_else(|| {
                AppError::Message(
                    "Unable to parse Windsurf user-data directory: unterminated quote".to_string(),
                )
            })?;
            &rest[..end]
        } else {
            let end = value.find(" --").unwrap_or(value.len());
            &value[..end]
        };
        if value.trim().is_empty() {
            return Err(AppError::Message(
                "Windsurf process has an empty user-data directory".to_string(),
            ));
        }
        return Ok(Some(PathBuf::from(value.trim())));
    }
    Ok(None)
}

fn collect_main_processes() -> Result<Vec<WindsurfProcess>, AppError> {
    // comm contains only the executable, so a shell mentioning Windsurf in its
    // arguments cannot be mistaken for the application. ps also avoids TCC scans.
    let output = Command::new("/bin/ps")
        .args(["-axww", "-o", "pid=,stat=,comm="])
        .output()
        .map_err(|error| {
            AppError::Message(format!("Unable to inspect Windsurf processes: {error}"))
        })?;
    if !output.status.success() {
        return Err(AppError::Message(
            "Unable to inspect Windsurf processes with ps".to_string(),
        ));
    }
    let listing = String::from_utf8(output.stdout)
        .map_err(|_| AppError::Message("Invalid UTF-8 in macOS process listing".to_string()))?;
    let mut entries = Vec::new();
    for mut entry in listing.lines().filter_map(parse_process_line) {
        let output = Command::new("/bin/ps")
            .args(["-p", &entry.pid.to_string(), "-ww", "-o", "args="])
            .output()
            .map_err(|error| {
                AppError::Message(format!("Unable to inspect Windsurf arguments: {error}"))
            })?;
        if !output.status.success() {
            // ps exits 1 when a process from the earlier snapshot has exited.
            if output.status.code() == Some(1) && output.stdout.is_empty() {
                continue;
            }
            return Err(AppError::Message(format!(
                "Unable to inspect Windsurf PID {}",
                entry.pid
            )));
        }
        let arguments = String::from_utf8(output.stdout).map_err(|_| {
            AppError::Message("Invalid UTF-8 in Windsurf process arguments".to_string())
        })?;
        if arguments.trim().is_empty() {
            return Err(AppError::Message(format!(
                "Cannot read arguments for Windsurf PID {}",
                entry.pid
            )));
        }
        if is_helper_process("", &arguments) {
            continue;
        }
        entry.user_data_dir = profile_argument(&arguments)?;
        entries.push(entry);
    }
    entries.sort_unstable_by_key(|entry| entry.pid);
    Ok(entries)
}

fn matches_profile(entry: &WindsurfProcess, profile: &Path, default_base: &Path) -> bool {
    if let Some(explicit) = &entry.user_data_dir {
        return normalize_path_for_compare(explicit) == normalize_path_for_compare(profile);
    }
    let brand = entry
        .executable
        .as_deref()
        .and_then(bundle_root)
        .as_deref()
        .and_then(bundle_brand);
    brand.is_some_and(|brand| {
        normalize_path_for_compare(&default_base.join(brand)) == normalize_path_for_compare(profile)
    })
}

fn matching_processes(profile: &Path) -> Result<Vec<WindsurfProcess>, AppError> {
    let base = dirs::config_dir().ok_or_else(|| {
        AppError::Message("Cannot determine the default Windsurf user-data directory".to_string())
    })?;
    Ok(collect_main_processes()?
        .into_iter()
        .filter(|entry| matches_profile(entry, profile, &base))
        .collect())
}

pub fn is_running() -> bool {
    match collect_main_processes() {
        Ok(entries) => !entries.is_empty(),
        Err(error) => {
            log::warn!("Windsurf status inspection failed: {error}");
            false
        }
    }
}

pub fn is_running_for(profile: &Path) -> bool {
    match matching_processes(profile) {
        Ok(entries) => !entries.is_empty(),
        Err(error) => {
            log::warn!("Windsurf profile inspection failed: {error}");
            false
        }
    }
}

pub fn ensure_stopped_for(profile: &Path) -> Result<(), AppError> {
    let entries = matching_processes(profile)?;
    if entries.is_empty() {
        return Ok(());
    }
    let pids = entries.iter().map(|entry| entry.pid).collect::<Vec<_>>();
    Err(AppError::Message(format!(
        "Windsurf is still running for {} (PIDs: {}); close it before switching accounts",
        profile.display(),
        format_pid_list(&pids)
    )))
}

fn wait_for_exit(profile: &Path, timeout: Duration) -> Result<bool, AppError> {
    let started = Instant::now();
    loop {
        if matching_processes(profile)?.is_empty() {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn signal_processes(entries: &[WindsurfProcess], signal: &str) {
    for entry in entries {
        match Command::new("/bin/kill")
            .args([signal, &entry.pid.to_string()])
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => log::warn!(
                "Windsurf PID {} signal {signal} returned {status}",
                entry.pid
            ),
            Err(error) => log::warn!("Windsurf PID {} signal {signal} failed: {error}", entry.pid),
        }
    }
}

pub fn close_for(profile: &Path, timeout_secs: u64) -> Result<(), AppError> {
    let entries = matching_processes(profile)?;
    if entries.is_empty() {
        return Ok(());
    }
    signal_processes(&entries, "-15");
    if wait_for_exit(profile, Duration::from_secs(2))? {
        return Ok(());
    }
    signal_processes(&matching_processes(profile)?, "-9");
    if wait_for_exit(profile, Duration::from_secs(timeout_secs.min(10)))? {
        return Ok(());
    }
    ensure_stopped_for(profile)
}

fn launch_command(bundle: &Path, profile: &Path) -> Command {
    let mut command = Command::new("/usr/bin/open");
    command
        .arg("-n")
        .arg("-a")
        .arg(bundle)
        .arg("--args")
        .arg("--user-data-dir")
        .arg(profile)
        .arg("--new-window")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    for variable in LAUNCH_ENV_REMOVALS {
        command.env_remove(variable);
    }
    command
}

#[derive(Default)]
struct StartupProbe {
    candidate: Option<(u32, Duration)>,
}

impl StartupProbe {
    fn observe(&mut self, pids: &[u32], elapsed: Duration) -> Option<u32> {
        if let Some((pid, since)) = self.candidate {
            if pids.contains(&pid) {
                return (elapsed.saturating_sub(since) >= Duration::from_secs(1)).then_some(pid);
            }
        }
        self.candidate = pids.first().map(|pid| (*pid, elapsed));
        None
    }
}

pub fn start_with(launch_path: &Path, profile: &Path) -> Result<u32, AppError> {
    let bundle = resolve_bundle(launch_path)?;
    validate_launch_profile(&bundle, profile)?;
    ensure_stopped_for(profile)?;

    // A regular temporary file avoids both stderr pipe backpressure and waiting
    // for a descendant to close a pipe. Only a bounded diagnostic is read back.
    let mut diagnostics =
        tempfile::tempfile().map_err(|error| AppError::Message(error.to_string()))?;
    let stderr = diagnostics
        .try_clone()
        .map_err(|error| AppError::Message(error.to_string()))?;
    let mut child = launch_command(&bundle, profile)
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| {
            AppError::Message(format!("Failed to launch Windsurf via open: {error}"))
        })?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            result => {
                let reason = match result {
                    Err(error) => error.to_string(),
                    _ => "open did not finish within 10 seconds".to_string(),
                };
                if let Err(error) = child.kill().and_then(|_| child.wait()) {
                    log::warn!("Unable to reap Windsurf launcher: {error}");
                }
                return Err(AppError::Message(format!(
                    "Windsurf launcher failed: {reason}"
                )));
            }
        }
    };
    if !status.success() {
        diagnostics
            .seek(SeekFrom::Start(0))
            .map_err(|error| AppError::Message(error.to_string()))?;
        let mut bytes = Vec::new();
        diagnostics
            .take(8192)
            .read_to_end(&mut bytes)
            .map_err(|error| AppError::Message(error.to_string()))?;
        return Err(AppError::Message(format!(
            "Windsurf open failed ({status}): {}",
            String::from_utf8_lossy(&bytes).trim()
        )));
    }

    let target_bundle = normalize_path_for_compare(&bundle);
    let target_profile = normalize_path_for_compare(profile);
    let started = Instant::now();
    let mut probe = StartupProbe::default();
    while started.elapsed() < Duration::from_secs(10) {
        let pids = collect_main_processes()?
            .into_iter()
            .filter(|entry| {
                let actual_bundle = entry.executable.as_deref().and_then(bundle_root);
                actual_bundle
                    .as_deref()
                    .map(normalize_path_for_compare)
                    .as_deref()
                    == Some(target_bundle.as_str())
                    && entry
                        .user_data_dir
                        .as_deref()
                        .map(normalize_path_for_compare)
                        .as_deref()
                        == Some(target_profile.as_str())
            })
            .map(|entry| entry.pid)
            .collect::<Vec<_>>();
        if let Some(pid) = probe.observe(&pids, started.elapsed()) {
            return Ok(pid);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(AppError::Message(format!(
        "Windsurf did not remain running after launch: app={}, profile={}. Open it manually and check the account.",
        bundle.display(), profile.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bundle_roots_without_truncating_spaced_paths() {
        let bundle = Path::new("/Users/Test User/Applications/Windsurf.app");
        assert_eq!(
            bundle_root(&bundle.join("Contents/MacOS/Electron")),
            Some(bundle.to_path_buf())
        );
        assert_eq!(bundle_root(bundle), Some(bundle.to_path_buf()));
        assert!(
            bundle_root(Path::new("/Applications/Other.app/Contents/MacOS/Electron")).is_none()
        );
    }

    #[test]
    fn recognizes_macos_electron_but_not_other_apps_or_helpers() {
        assert!(is_main_executable(Path::new(
            "/Applications/Windsurf.app/Contents/MacOS/Electron"
        )));
        assert!(is_main_executable(Path::new(
            "/Applications/Devin.app/Contents/MacOS/Devin"
        )));
        assert!(!is_main_executable(Path::new(
            "/Applications/Other.app/Contents/MacOS/Electron"
        )));
        assert!(!is_main_executable(Path::new(
            "/Applications/Windsurf.app/Contents/MacOS/Windsurf Helper"
        )));
        assert!(!is_main_executable(Path::new("/Applications/Windsurf.app/Contents/Frameworks/Windsurf Helper.app/Contents/MacOS/Electron")));
    }

    #[test]
    fn parses_ps_executable_listing_with_spaces_and_excludes_zombies() {
        let entry = parse_process_line(
            "42 S /Users/测试 User/Applications/Windsurf.app/Contents/MacOS/Electron",
        )
        .unwrap();
        assert_eq!(entry.pid, 42);
        assert_eq!(
            entry.executable,
            Some(PathBuf::from(
                "/Users/测试 User/Applications/Windsurf.app/Contents/MacOS/Electron"
            ))
        );
        assert!(
            parse_process_line("42 Z /Applications/Windsurf.app/Contents/MacOS/Electron").is_none()
        );
        assert!(parse_process_line("42 S /bin/bash").is_none());
    }

    #[test]
    fn parses_profile_arguments_in_both_forms() {
        let expected = Some(PathBuf::from(
            "/Users/Test User/Library/Application Support/Windsurf",
        ));
        assert_eq!(profile_argument("Electron --user-data-dir /Users/Test User/Library/Application Support/Windsurf --new-window").unwrap(), expected);
        assert_eq!(profile_argument("Electron --user-data-dir=\"/Users/Test User/Library/Application Support/Windsurf\" --new-window").unwrap(), expected);
        assert_eq!(profile_argument("Electron --new-window").unwrap(), None);
        assert!(profile_argument("Electron --user-data-dir= --new-window").is_err());
        assert!(profile_argument("Electron --user-data-dir=\"broken").is_err());
    }

    #[test]
    fn implicit_profile_only_matches_its_brand_default() {
        let base = Path::new("/Users/test/Library/Application Support");
        let mut entry =
            parse_process_line("42 S /Applications/Windsurf.app/Contents/MacOS/Electron").unwrap();
        assert!(matches_profile(&entry, &base.join("Windsurf"), base));
        assert!(!matches_profile(&entry, &base.join("Devin"), base));
        assert!(!matches_profile(&entry, Path::new("/tmp/custom"), base));
        entry.user_data_dir = Some(PathBuf::from("/tmp/custom"));
        assert!(matches_profile(&entry, Path::new("/tmp/custom"), base));
        assert!(!matches_profile(&entry, &base.join("Windsurf"), base));
    }

    #[test]
    fn constructs_launchservices_command_and_removes_inherited_env() {
        let command = launch_command(
            Path::new("/Applications/Windsurf.app"),
            Path::new("/tmp/用户 Profile"),
        );
        assert_eq!(command.get_program(), "/usr/bin/open");
        let args = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            [
                "-n",
                "-a",
                "/Applications/Windsurf.app",
                "--args",
                "--user-data-dir",
                "/tmp/用户 Profile",
                "--new-window"
            ]
        );
        for variable in LAUNCH_ENV_REMOVALS {
            assert!(command
                .get_envs()
                .any(|(key, value)| key == *variable && value.is_none()));
        }
        assert!(!command.get_envs().any(|(key, _)| key == "HTTPS_PROXY"));
    }

    #[test]
    fn startup_requires_a_continuously_observed_real_pid() {
        let mut probe = StartupProbe::default();
        assert_eq!(probe.observe(&[], Duration::ZERO), None);
        assert_eq!(probe.observe(&[], Duration::from_secs(5)), None);
        assert_eq!(probe.observe(&[42], Duration::from_secs(5)), None);
        assert_eq!(probe.observe(&[], Duration::from_millis(5500)), None);
        assert_eq!(probe.observe(&[43], Duration::from_secs(6)), None);
        assert_eq!(probe.observe(&[43], Duration::from_millis(6500)), None);
        assert_eq!(probe.observe(&[43], Duration::from_secs(7)), Some(43));
    }

    #[test]
    fn validates_bundle_metadata_and_old_internal_executable_paths() {
        let temp = tempfile::TempDir::new().unwrap();
        let bundle = temp.path().join("Windsurf.app");
        let executable = bundle.join("Contents/MacOS/Electron");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, []).unwrap();
        assert!(resolve_bundle(&bundle).is_err());
        std::fs::write(bundle.join("Contents/Info.plist"), br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>Electron</string></dict></plist>"#).unwrap();
        assert_eq!(resolve_bundle(&bundle).unwrap(), bundle);
        assert_eq!(resolve_bundle(&executable).unwrap(), bundle);
        assert!(validate_launch_profile(&bundle, Path::new("/tmp/Windsurf")).is_ok());
        assert!(validate_launch_profile(&bundle, Path::new("/tmp/custom")).is_ok());
        assert!(validate_launch_profile(&bundle, Path::new("/tmp/Devin")).is_err());
    }
}
