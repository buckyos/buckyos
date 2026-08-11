//! Centralized BuckyOS path layout helpers.
//!
//! Single source of truth for "where do the 4 bin layers live on disk?" so
//! the rest of the runtime never has to deal with the
//! `<buckyos_root>/tools/...` prefix directly. When the BuckyOS team
//! publishes a canonical path API this module is the only place that has
//! to switch over.
//!
//! Resolution order (see §9.2 of NewOpenDANRuntime.md):
//!   1. `BUCKYOS_ROOT` env var — single override for both production
//!      containers (`export BUCKYOS_ROOT=/opt/buckyos`) and ad-hoc dev runs.
//!   2. Linux default: `/opt/buckyos`
//!   3. macOS default: `$HOME/.buckyos` (falls back to `/tmp/buckyos` if
//!      `HOME` is unset).
//!   4. Other OSes: `/tmp/buckyos` — best-effort, dev-only.

use std::path::PathBuf;

const BUCKYOS_INSTANCE_VOLUME_ENV: &str = "BUCKYOS_INSTANCE_VOLUME";

/// Resolve the BuckyOS root directory. See module docs for fallback order.
pub fn buckyos_root() -> PathBuf {
    if let Ok(v) = std::env::var("BUCKYOS_ROOT") {
        if !v.trim().is_empty() {
            return PathBuf::from(v);
        }
    }
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/opt/buckyos")
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".buckyos"))
            .unwrap_or_else(|| PathBuf::from("/tmp/buckyos"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp/buckyos")
    }
}

/// `<buckyos_root>/tools/` — parent dir of all 4 bin layers.
pub fn buckyos_tools_root() -> PathBuf {
    buckyos_root().join("tools")
}

/// Writable tool root for layers rendered by OpenDAN itself.
pub fn writable_tools_root() -> PathBuf {
    if let Ok(v) = std::env::var(BUCKYOS_INSTANCE_VOLUME_ENV) {
        if !v.trim().is_empty() {
            return PathBuf::from(v).join("tools");
        }
    }
    buckyos_tools_root()
}

/// System Bin layer — rx, shared across every Agent on the host.
pub fn system_bin_dir() -> PathBuf {
    buckyos_tools_root().join("store")
}

/// Runtime Bin layer — rx, App-scoped symlink view rendered from System Bin
/// + ExtTool Volume. First-version is an empty directory placeholder; future
/// Crafter / volume integrations write into it.
pub fn runtime_bin_dir() -> PathBuf {
    writable_tools_root().join("bin")
}

/// Session Exec Bin layer — rwx, per-Agent + per-Session. The runtime
/// renders Agent tools + tool-plan tombstones into this directory at
/// session boot, and on every `exec_bash` invocation re-checks Agent
/// tools mtime to pick up live changes.
pub fn session_exec_bin_dir(agent_id: &str, session_id: &str) -> PathBuf {
    writable_tools_root()
        .join(sanitize_path_segment(agent_id))
        .join(sanitize_path_segment(session_id))
}

/// Restrict an identifier to `[A-Za-z0-9_-]`. Used wherever we splice an
/// agent_id / session_id into a filesystem path; matches the Linux /
/// macOS / Windows-safe subset and keeps the path stable across hosts.
/// Non-conforming chars collapse to `_`. Empty input yields `_`.
pub fn sanitize_path_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_path_segment_replaces_unsafe_chars() {
        assert_eq!(sanitize_path_segment("did:dev:alice"), "did_dev_alice");
        assert_eq!(sanitize_path_segment("ok-name_42"), "ok-name_42");
        assert_eq!(sanitize_path_segment(""), "_");
    }

    // Single test for env-driven paths: tokio test framework runs cases in
    // parallel within a binary, so we serialize all BUCKYOS_ROOT mutation
    // into one test to avoid cross-test interference.
    #[test]
    fn buckyos_root_layout_honors_env_override() {
        let prev = std::env::var("BUCKYOS_ROOT").ok();
        let prev_instance = std::env::var(BUCKYOS_INSTANCE_VOLUME_ENV).ok();
        std::env::set_var("BUCKYOS_ROOT", "/tmp/buckyos_test_layout");
        std::env::remove_var(BUCKYOS_INSTANCE_VOLUME_ENV);
        assert_eq!(buckyos_root(), PathBuf::from("/tmp/buckyos_test_layout"));
        assert_eq!(
            buckyos_tools_root(),
            PathBuf::from("/tmp/buckyos_test_layout/tools")
        );
        assert_eq!(
            writable_tools_root(),
            PathBuf::from("/tmp/buckyos_test_layout/tools")
        );
        assert_eq!(
            session_exec_bin_dir("agent-1", "ses/01"),
            PathBuf::from("/tmp/buckyos_test_layout/tools/agent-1/ses_01")
        );
        std::env::set_var(BUCKYOS_INSTANCE_VOLUME_ENV, "/tmp/buckyos_instance");
        assert_eq!(
            system_bin_dir(),
            PathBuf::from("/tmp/buckyos_test_layout/tools/store")
        );
        assert_eq!(
            runtime_bin_dir(),
            PathBuf::from("/tmp/buckyos_instance/tools/bin")
        );
        assert_eq!(
            session_exec_bin_dir("agent-1", "ses/01"),
            PathBuf::from("/tmp/buckyos_instance/tools/agent-1/ses_01")
        );
        if let Some(p) = prev {
            std::env::set_var("BUCKYOS_ROOT", p);
        } else {
            std::env::remove_var("BUCKYOS_ROOT");
        }
        if let Some(p) = prev_instance {
            std::env::set_var(BUCKYOS_INSTANCE_VOLUME_ENV, p);
        } else {
            std::env::remove_var(BUCKYOS_INSTANCE_VOLUME_ENV);
        }
    }
}
