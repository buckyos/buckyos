//! node_daemon as the macOS Desktop Runtime (`BuckyOS Runtime.app`,
//! notepads/macos-desktop-install-runtime-todo.md): the `service` control
//! subcommand the installer and dev scripts use, the app entry for a
//! Finder/LaunchServices launch, the per-root instance guard and the desktop
//! readiness report node_control reads.

use std::path::{Path, PathBuf};
use std::process::Command;

use buckyos_api::node_control::{
    current_euid, current_os_user, detect_host_control_state_at, macos_register_runtime,
    macos_set_autostart, macos_unregister_runtime, node_daemon_local_dir, node_start, node_stop,
    read_desktop_binding, read_desktop_status, NodeStartRequest, NodeStopRequest,
    DESKTOP_STATUS_FILE, MACOS_RUNTIME_LABEL,
};

pub const SERVICE_ARG: &str = "service";
pub const REQUEST_PERMISSIONS_ARG: &str = "--request_permissions";

const SERVICE_HELP: &str = "usage: node_daemon service <command> [--root PATH]

Manage the macOS Desktop Runtime of a BuckyOS root (default: this Runtime's root).

commands:
  register --user NAME   bind the root to an OS user and install its login item
  unregister             remove the login item and the binding
  start                  start the Runtime in the bound user's GUI session
  stop                   stop the Runtime and this installation's processes
  restart                stop, then start
  autostart on|off       login autostart only; the running state is unchanged
  status                 host control state as JSON";

fn runtime_root() -> PathBuf {
    buckyos_kit::get_buckyos_root_dir()
}

fn start(root: &Path) -> Result<Vec<String>, String> {
    node_start(NodeStartRequest {
        buckyos_root: Some(root.to_path_buf()),
        reason: Some("node_daemon service start".into()),
        ..NodeStartRequest::default()
    })
    .map(|report| report.actions)
}

fn stop(root: &Path) -> Result<Vec<String>, String> {
    let report = node_stop(NodeStopRequest {
        buckyos_root: Some(root.to_path_buf()),
        reason: Some("node_daemon service stop".into()),
        ..NodeStopRequest::default()
    })?;
    let mut lines = report.host_service_actions;
    lines.extend(report.actions);
    lines.extend(
        report
            .stopped_processes
            .into_iter()
            .map(|p| format!("stopped {p}")),
    );
    lines.extend(
        report
            .stopped_containers
            .into_iter()
            .map(|c| format!("removed container {c}")),
    );
    lines.extend(
        report
            .remaining
            .into_iter()
            .map(|p| format!("still running {p}")),
    );
    Ok(lines)
}

fn status(root: &Path) -> String {
    let host = detect_host_control_state_at(root.to_path_buf());
    let desktop = read_desktop_status(root)
        .map(|(value, age)| serde_json::json!({"report": value, "age_secs": age}));
    serde_json::to_string_pretty(&serde_json::json!({"host_control": host, "desktop": desktop}))
        .unwrap_or_default()
}

/// `node_daemon service ...`; returns the process exit code.
pub fn service_main(args: Vec<String>) -> i32 {
    if !cfg!(target_os = "macos") {
        eprintln!("node_daemon service manages the macOS Desktop Runtime only");
        return 2;
    }
    let mut root = None;
    let mut user = None;
    let mut command = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--user" => user = args.next(),
            "-h" | "--help" => {
                println!("{SERVICE_HELP}");
                return 0;
            }
            _ => command.push(arg),
        }
    }
    let root = root.unwrap_or_else(runtime_root);
    let command: Vec<&str> = command.iter().map(String::as_str).collect();
    let result = match command.as_slice() {
        ["register"] => match user {
            Some(user) => macos_register_runtime(&root, &user),
            None => Err("register needs --user NAME".into()),
        },
        ["unregister"] => macos_unregister_runtime(&root),
        ["start"] => start(&root),
        ["stop"] => stop(&root),
        ["restart"] => stop(&root).and_then(|mut lines| {
            lines.extend(start(&root)?);
            Ok(lines)
        }),
        ["autostart", "on"] => macos_set_autostart(&root, true),
        ["autostart", "off"] => macos_set_autostart(&root, false),
        ["status"] => {
            println!("{}", status(&root));
            return 0;
        }
        _ => {
            eprintln!("{SERVICE_HELP}");
            return 2;
        }
    };
    match result {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            0
        }
        Err(err) => {
            eprintln!("node_daemon service: {err}");
            1
        }
    }
}

/// A LaunchServices launch of the bundle (Finder, `open`) carries no daemon
/// arguments: it is a user asking for the Runtime, never a reason to start a
/// system-mode node in the foreground.
pub fn is_app_entry(args: &[String]) -> bool {
    if args.iter().any(|arg| arg == REQUEST_PERMISSIONS_ARG) {
        return true;
    }
    let in_bundle = std::env::current_exe()
        .map(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
        .unwrap_or(false);
    in_bundle && args.iter().all(|arg| arg.starts_with("-psn_"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn applescript_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn alert(message: &str) {
    eprintln!("BuckyOS Runtime: {message}");
    let script = format!(
        "display alert {} message {} as warning",
        applescript_quote("BuckyOS Runtime"),
        applescript_quote(message)
    );
    let _ = Command::new("osascript").args(["-e", &script]).status();
}

/// First open after an installation without a GUI user: bind the root to the
/// person opening the app, with an administrator prompt (the root is still
/// installer-owned).
fn register_with_admin(root: &Path, user: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let command = format!(
        "{} {} register --user {} --root {}",
        shell_quote(&exe.to_string_lossy()),
        SERVICE_ARG,
        shell_quote(user),
        shell_quote(&root.to_string_lossy())
    );
    let script = format!(
        "do shell script {} with prompt {} with administrator privileges",
        applescript_quote(&command),
        applescript_quote("BuckyOS Runtime will be set up to run for your account on this Mac.")
    );
    match Command::new("osascript").args(["-e", &script]).output() {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            alert(&format!(
                "Setup was not completed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            false
        }
        Err(err) => {
            alert(&format!("Setup could not run: {err}"));
            false
        }
    }
}

/// Whether the running Runtime still reports a permission this process now
/// holds (Screen Recording only applies after a restart).
fn runtime_lacks(root: &Path, capture: bool, input: bool) -> bool {
    let Some((report, _)) = read_desktop_status(root) else {
        return false;
    };
    let caps = &report["capabilities"];
    let lacks = |name: &str, granted: bool| {
        granted
            && caps[name]["reason"]
                .as_str()
                .is_some_and(|reason| reason.ends_with("_permission_required"))
    };
    lacks("capture", capture) || lacks("input", input)
}

/// Finder/LaunchServices entry: finish setup if needed, request the privacy
/// permissions for this bundle identity, and make sure the managed Runtime
/// runs. `--request_permissions` only asks for the permissions.
#[cfg(target_os = "macos")]
pub fn app_entry_main(args: &[String]) -> i32 {
    let request_only = args.iter().any(|arg| arg == REQUEST_PERMISSIONS_ARG);
    let root = runtime_root();
    let euid = current_euid();
    let me = current_os_user();
    if !request_only {
        match read_desktop_binding(&root) {
            None => {
                let Some(me) = me else {
                    alert("Cannot determine the current macOS user.");
                    return 1;
                };
                if !register_with_admin(&root, &me.name) {
                    return 1;
                }
            }
            Some(binding) if binding.uid != euid => {
                alert(&format!(
                    "BuckyOS Runtime on this Mac belongs to the macOS user {}.",
                    binding.user
                ));
                return 1;
            }
            Some(_) => {}
        }
    }
    let (capture, input) = computer_driver::request_access();
    println!("screen_capture={capture} accessibility_input={input}");
    if request_only {
        return 0;
    }
    if runtime_lacks(&root, capture, input) {
        let domain = format!("gui/{euid}/{}", MACOS_RUNTIME_LABEL);
        let _ = Command::new("launchctl")
            .args(["kickstart", "-k", &domain])
            .status();
    }
    match start(&root) {
        Ok(_) => 0,
        Err(err) => {
            alert(&err);
            1
        }
    }
}

/// One controller per root: a second node_daemon for the same root (a
/// foreground debug run next to the LaunchAgent, another OS user) exits.
#[cfg(target_os = "macos")]
pub fn acquire_instance_guard() -> Result<std::fs::File, String> {
    use std::io::Write;
    use std::os::fd::AsRawFd;

    let root = runtime_root();
    let euid = current_euid();
    if let Some(binding) = read_desktop_binding(&root) {
        if binding.uid != euid {
            return Err(format!(
                "{} belongs to OS user {} (uid {}); refusing to run it as uid {euid}",
                root.display(),
                binding.user,
                binding.uid
            ));
        }
    }
    let dir = node_daemon_local_dir(&root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {} failed: {e}", dir.display()))?;
    let path = dir.join("node_daemon.lock");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("open {} failed: {e}", path.display()))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let holder = std::fs::read_to_string(&path).unwrap_or_default();
        return Err(format!(
            "another node_daemon (pid {}) already runs {}",
            holder.trim(),
            root.display()
        ));
    }
    let _ = file.set_len(0);
    let _ = write!(file, "{}", std::process::id());
    Ok(file)
}

/// The Runtime's own view of its GUI session and privacy permissions, for
/// node_control (tray, `check`) which cannot probe them for another process.
#[cfg(target_os = "macos")]
pub fn spawn_desktop_status_writer() {
    tokio::spawn(async {
        let path = buckyos_kit::get_buckyos_service_local_data_dir("node_daemon")
            .join(DESKTOP_STATUS_FILE);
        loop {
            if let Ok(capabilities) =
                tokio::task::spawn_blocking(computer_driver::capabilities).await
            {
                let body = serde_json::json!({
                    "updated_at": buckyos_kit::buckyos_get_unix_timestamp(),
                    "uid": current_euid(),
                    "pid": std::process::id(),
                    "capabilities": capabilities,
                });
                let temporary = path.with_extension("json.tmp");
                let written = std::fs::write(
                    &temporary,
                    serde_json::to_vec_pretty(&body).unwrap_or_default(),
                )
                .and_then(|_| std::fs::rename(&temporary, &path));
                if let Err(err) = written {
                    log::warn!("write desktop status {} failed: {err}", path.display());
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

/// SIGTERM (launchd bootout/kickstart -k, systemd stop) or Ctrl-C.
pub async fn termination_signal() -> &'static str {
    #[cfg(unix)]
    {
        let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        else {
            let _ = tokio::signal::ctrl_c().await;
            return "SIGINT";
        };
        tokio::select! {
            _ = term.recv() => "SIGTERM",
            _ = tokio::signal::ctrl_c() => "SIGINT",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "Ctrl-C"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_spaces_and_quotes() {
        assert_eq!(shell_quote("/opt/My Bucky's"), "'/opt/My Bucky'\\''s'");
        assert_eq!(
            applescript_quote("say \"hi\" \\"),
            "\"say \\\"hi\\\" \\\\\""
        );
    }

    #[test]
    fn daemon_arguments_are_not_an_app_entry() {
        assert!(is_app_entry(&[REQUEST_PERMISSIONS_ARG.to_string()]));
        assert!(!is_app_entry(&[
            "--enable_active".to_string(),
            "--desktop_daemon".to_string()
        ]));
    }
}
