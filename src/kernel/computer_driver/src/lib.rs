//! The desktop ComputerUse Driver (macOS 14+, Windows) shared by the
//! standalone Agent Daemon (form B) and node-daemon's desktop mode (form
//! A-desktop, doc/arch/node_exector.md §4.2): one implementation, one
//! input-coordination lock per user session, so both daemons on the same
//! machine exclude each other instead of each driving the desktop.
//! Linux has no desktop Driver: `capabilities()` reports unsupported.

use std::path::Path;

#[cfg(windows)]
pub mod windows;

#[cfg(any(target_os = "macos", windows))]
mod desktop;

#[cfg(any(target_os = "macos", windows))]
pub use desktop::{capabilities, driver};
#[cfg(target_os = "macos")]
pub use desktop::request_access;

/// Whether this build carries a desktop Driver.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

#[cfg(not(any(target_os = "macos", windows)))]
pub fn capabilities() -> serde_json::Value {
    let unsupported =
        serde_json::json!({"supported": false, "ready": false, "state": "unsupported"});
    serde_json::json!({
        "ui": unsupported,
        "capture": unsupported,
        "input": unsupported,
        "accessibility": unsupported
    })
}

/// Milliseconds since boot (monotonic across sleep where the OS offers it).
pub fn boot_ms() -> u64 {
    #[cfg(target_os = "linux")]
    let clock = libc::CLOCK_BOOTTIME;
    #[cfg(target_os = "macos")]
    let clock = libc::CLOCK_MONOTONIC_RAW;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(clock, &mut ts) } == 0 {
            return (ts.tv_sec as u64).saturating_mul(1000) + ts.tv_nsec as u64 / 1_000_000;
        }
        return u64::MAX;
    }
    #[cfg(windows)]
    {
        windows::boot_ms()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    node_executor::now_ms()
}

/// A directory only the current OS user can use (created when missing).
pub fn private_dir(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    return windows::private_dir(path);
    #[cfg(not(windows))]
    {
        std::fs::create_dir_all(path).map_err(|_| "cannot create daemon data directory")?;
        let metadata =
            std::fs::symlink_metadata(path).map_err(|_| "cannot inspect daemon data directory")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("daemon data directory must be a real directory".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err("daemon data directory belongs to another OS user".into());
            }
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "cannot secure daemon data directory")?;
        }
        Ok(())
    }
}
