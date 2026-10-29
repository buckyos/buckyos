//! ffmpeg / ffprobe discovery (preview iPhone media TODO D1/D2): "有则使用，
//! 没有就失败". Lookup order is `$BUCKYOS_ROOT/libexec/buckyos-tool/bin/`, then
//! `PATH` (macOS also checks the Homebrew prefixes a launchd service PATH
//! lacks); Windows names carry `.exe` and spawn without a console. Nothing is
//! downloaded or installed. The probe result is cached per binary identity
//! (path, size, mtime) and shown in `hello` so a DV run can tell which path ran.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub path: PathBuf,
    /// First token after `<name> version ` of `-version` (e.g. `7.1.5-0+deb13u1`).
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Toolchain {
    pub ffmpeg: Tool,
    pub ffprobe: Tool,
    /// `zscale` + `tonemap` filters: HDR → SDR BT.709.
    pub tonemap: bool,
    /// `libx264` encoder: transcoding to H.264 (remux needs none).
    pub libx264: bool,
    /// ffmpeg ≥ 7.1 exposes HEIF tile grids as stream groups.
    pub heif: bool,
}

#[derive(Debug, Clone)]
pub enum Probe {
    Ready(Toolchain),
    Missing { missing: Vec<&'static str>, searched: Vec<PathBuf> },
}

impl Probe {
    pub fn toolchain(&self) -> Option<&Toolchain> {
        match self {
            Probe::Ready(t) => Some(t),
            Probe::Missing { .. } => None,
        }
    }

    /// Why `repr` is unavailable, e.g. "ffprobe not found (searched ...)".
    pub fn missing_detail(&self) -> Option<String> {
        match self {
            Probe::Ready(_) => None,
            Probe::Missing { missing, searched } => Some(format!(
                "{} not found on this node (searched {})",
                missing.join(" and "),
                searched.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            )),
        }
    }

    /// The `repr` capability block of `hello`.
    pub fn describe(&self) -> serde_json::Value {
        match self {
            Probe::Ready(t) => serde_json::json!({
                "available": true,
                "ffmpeg": t.ffmpeg,
                "ffprobe": t.ffprobe,
                "tonemap": t.tonemap,
                "h264_encoder": t.libx264,
                "heif": t.heif,
                "profiles": super::PROFILES,
            }),
            Probe::Missing { missing, .. } => serde_json::json!({
                "available": false,
                "missing": missing,
                "detail": self.missing_detail(),
            }),
        }
    }
}

pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Directories searched, in order.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![buckyos_kit::get_buckyos_root_dir().join("libexec").join("buckyos-tool").join("bin")];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path).filter(|p| !p.as_os_str().is_empty()));
    }
    if cfg!(target_os = "macos") {
        for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let extra = PathBuf::from(extra);
            if !dirs.contains(&extra) {
                dirs.push(extra);
            }
        }
    }
    dirs
}

pub fn find(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let exe = exe_name(name);
    dirs.iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// `(path, size, mtime)` of both binaries: a changed install re-probes.
pub type Identity = Vec<(PathBuf, u64, u64)>;

pub fn identity(paths: &[&Path]) -> Identity {
    paths
        .iter()
        .map(|p| {
            let meta = std::fs::metadata(p).ok();
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (p.to_path_buf(), meta.map(|m| m.len()).unwrap_or(0), mtime)
        })
        .collect()
}

fn output_of(path: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.args(args).stdin(std::process::Stdio::null());
    buckyos_api::hide_child_console(&mut cmd);
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn parse_version(name: &str, text: &str) -> Option<String> {
    let line = text.lines().next()?;
    let rest = line.trim().strip_prefix(name)?.trim_start().strip_prefix("version")?.trim_start();
    rest.split_whitespace().next().map(String::from)
}

/// `7.1.5-0+deb13u1` → (7, 1); git builds (`N-…`, dates) are taken as current.
pub fn at_least(version: &str, major: u32, minor: u32) -> bool {
    if version.starts_with("N-") || version.starts_with("git-") {
        return true;
    }
    let digits: Vec<u32> = version
        .trim_start_matches('n')
        .split(|c: char| !c.is_ascii_digit())
        .take(2)
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    match digits.as_slice() {
        [y, ..] if *y >= 2000 => true,
        [maj, min, ..] => (*maj, *min) >= (major, minor),
        [maj] => *maj > major,
        _ => true,
    }
}

fn listed(text: &str, name: &str) -> bool {
    text.lines().any(|l| l.split_whitespace().nth(1) == Some(name))
}

/// Finds and runs both binaries (blocking: call off the async runtime).
pub fn probe(dirs: &[PathBuf]) -> Probe {
    let ffmpeg = find("ffmpeg", dirs);
    let ffprobe = find("ffprobe", dirs);
    let (Some(ffmpeg), Some(ffprobe)) = (ffmpeg.clone(), ffprobe.clone()) else {
        let mut missing = Vec::new();
        if ffmpeg.is_none() {
            missing.push("ffmpeg");
        }
        if ffprobe.is_none() {
            missing.push("ffprobe");
        }
        return Probe::Missing { missing, searched: dirs.to_vec() };
    };
    let version = |path: &Path, name: &'static str| output_of(path, &["-hide_banner", "-version"]).and_then(|t| parse_version(name, &t));
    let (Some(ffmpeg_version), Some(ffprobe_version)) = (version(&ffmpeg, "ffmpeg"), version(&ffprobe, "ffprobe")) else {
        return Probe::Missing { missing: vec!["ffmpeg", "ffprobe"], searched: dirs.to_vec() };
    };
    let filters = output_of(&ffmpeg, &["-hide_banner", "-filters"]).unwrap_or_default();
    let encoders = output_of(&ffmpeg, &["-hide_banner", "-encoders"]).unwrap_or_default();
    Probe::Ready(Toolchain {
        heif: at_least(&ffmpeg_version, 7, 1) && at_least(&ffprobe_version, 7, 1),
        ffmpeg: Tool { path: ffmpeg, version: ffmpeg_version },
        ffprobe: Tool { path: ffprobe, version: ffprobe_version },
        tonemap: listed(&filters, "zscale") && listed(&filters, "tonemap"),
        libx264: listed(&encoders, "libx264"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("ffmpeg", "ffmpeg version 7.1.5-0+deb13u1 Copyright (c) 2000-2026").as_deref(), Some("7.1.5-0+deb13u1"));
        assert_eq!(parse_version("ffprobe", "ffprobe version n7.1 Copyright").as_deref(), Some("n7.1"));
        assert_eq!(parse_version("ffmpeg", "ffmpeg version 2024-10-21-git-baa23e40c1-full_build-www.gyan.dev Copyright").as_deref(), Some("2024-10-21-git-baa23e40c1-full_build-www.gyan.dev"));
        assert!(parse_version("ffmpeg", "something else").is_none());
        assert!(at_least("7.1.5-0+deb13u1", 7, 1));
        assert!(at_least("n7.1", 7, 1));
        assert!(!at_least("6.1.1-3ubuntu5", 7, 1));
        assert!(at_least("2024-10-21-git-baa23e40c1-full_build-www.gyan.dev", 7, 1));
        assert!(at_least("N-117000-gabcdef", 7, 1));
    }

    #[test]
    fn listing() {
        let text = " ... zscale            V->V       Apply resizing\n .S. tonemap           V->V       Conversion";
        assert!(listed(text, "zscale") && listed(text, "tonemap") && !listed(text, "libplacebo"));
    }

    #[test]
    fn missing_tools_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let probe = probe(&[dir.path().to_path_buf()]);
        assert!(probe.toolchain().is_none());
        let detail = probe.missing_detail().unwrap();
        assert!(detail.contains("ffmpeg and ffprobe not found"), "{detail}");
    }

    #[test]
    fn names_resolve_in_directories_with_spaces_and_non_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("工具 bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(exe_name("ffmpeg")), b"").unwrap();
        assert_eq!(find("ffmpeg", &[dir.path().to_path_buf(), bin.clone()]), Some(bin.join(exe_name("ffmpeg"))));
        assert_eq!(find("ffprobe", &[bin]), None);
    }
}
