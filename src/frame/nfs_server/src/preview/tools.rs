//! Converter discovery (preview iPhone media TODO D1/D2, Office import TODO
//! §5.1): ffmpeg + ffprobe and LibreOffice `soffice`, each optional (Q7) —
//! "有则使用，没有就失败". Lookup order is `$BUCKYOS_ROOT/libexec/buckyos-tool/bin/`,
//! then `PATH`, then fixed locations a service PATH lacks (macOS Homebrew
//! prefixes; LibreOffice's install directories on all three platforms); Windows
//! names carry `.exe` and spawn without a console. Nothing is downloaded or
//! installed. The probe result is cached per binary identity (path, size,
//! mtime) and shown in `hello` so a DV run can tell which path ran.

use super::convert::Profile;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub path: PathBuf,
    /// First token after `<name> version ` of `-version` (e.g. `7.1.5-0+deb13u1`),
    /// the first numeric token of `soffice --version` (e.g. `25.2.3.2`).
    pub version: String,
}

/// ffmpeg + ffprobe: images, video and audio.
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

/// Binaries `refresh` stats; a changed identity re-probes.
pub const NAMES: &[&str] = &["ffmpeg", "ffprobe", "soffice"];
const MEDIA: &[&str] = &["ffmpeg", "ffprobe"];
const DOCUMENT: &[&str] = &["soffice"];

/// What converts a plan: ffmpeg for media, LibreOffice for documents.
#[derive(Debug, Clone)]
pub enum Converter {
    Ffmpeg(Toolchain),
    Soffice(Tool),
}

/// The converters found; `repr` is advertised while any one is.
#[derive(Debug, Clone)]
pub struct Probe {
    pub media: Option<Toolchain>,
    pub soffice: Option<Tool>,
    /// Binaries not found (or not runnable), e.g. `["soffice"]`.
    pub missing: Vec<&'static str>,
    pub searched: Vec<PathBuf>,
}

impl Probe {
    pub fn toolchain(&self) -> Option<&Toolchain> {
        self.media.as_ref()
    }

    pub fn available(&self) -> bool {
        self.media.is_some() || self.soffice.is_some()
    }

    pub fn converter(&self, profile: Profile) -> Option<Converter> {
        match profile {
            Profile::PreviewDocument => self.soffice.clone().map(Converter::Soffice),
            _ => self.media.clone().map(Converter::Ffmpeg),
        }
    }

    /// The profiles this node can produce now.
    pub fn profiles(&self) -> Vec<&'static str> {
        super::PROFILES.iter().filter(|p| self.converter(**p).is_some()).map(Profile::as_str).collect()
    }

    fn detail(&self, groups: &[&[&str]]) -> Option<String> {
        let sentences: Vec<String> = groups
            .iter()
            .map(|g| self.missing.iter().copied().filter(|m| g.contains(m)).collect::<Vec<_>>())
            .filter(|m| !m.is_empty())
            .map(|m| format!("{} not found on this node", m.join(" and ")))
            .collect();
        (!sentences.is_empty()).then(|| {
            format!("{} (searched {})", sentences.join("; "), self.searched.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
        })
    }

    /// What is missing, e.g. "ffmpeg and ffprobe not found on this node; soffice not found on this node (searched ...)".
    pub fn missing_detail(&self) -> Option<String> {
        self.detail(&[MEDIA, DOCUMENT])
    }

    /// Why `profile` is unavailable, e.g. "soffice not found on this node (searched ...)".
    pub fn detail_for(&self, profile: Profile) -> String {
        self.detail(&[if profile == Profile::PreviewDocument { DOCUMENT } else { MEDIA }]).unwrap_or_default()
    }

    /// The `repr` capability block of `hello`.
    pub fn describe(&self) -> serde_json::Value {
        let mut out = serde_json::json!({"available": self.available(), "profiles": self.profiles()});
        if let Some(t) = &self.media {
            out["ffmpeg"] = serde_json::json!(t.ffmpeg);
            out["ffprobe"] = serde_json::json!(t.ffprobe);
            out["tonemap"] = t.tonemap.into();
            out["h264_encoder"] = t.libx264.into();
            out["heif"] = t.heif.into();
        }
        if let Some(t) = &self.soffice {
            out["soffice"] = serde_json::json!(t);
        }
        if !self.missing.is_empty() {
            out["missing"] = serde_json::json!(self.missing);
            out["detail"] = serde_json::json!(self.missing_detail());
        }
        out
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
    let mut extras: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        extras.extend(["/opt/homebrew/bin", "/usr/local/bin", "/Applications/LibreOffice.app/Contents/MacOS"].map(PathBuf::from));
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            extras.extend(std::env::var_os(var).map(|p| PathBuf::from(p).join("LibreOffice").join("program")));
        }
    } else {
        extras.extend(["/usr/bin", "/usr/lib/libreoffice/program"].map(PathBuf::from));
        // `/opt/libreoffice<version>/program` of the upstream packages, newest name first.
        let mut opt: Vec<PathBuf> = std::fs::read_dir("/opt")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("libreoffice"))
            .map(|e| e.path().join("program"))
            .collect();
        opt.sort_by(|a, b| b.cmp(a));
        extras.extend(opt);
    }
    for extra in extras {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

pub fn find(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let exe = exe_name(name);
    dirs.iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// `(path, size, mtime)` of the binaries found: a changed install re-probes.
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

/// `LibreOffice 25.2.3.2 520(Build:2)` → `25.2.3.2` (also `LibreOfficeDev`, Collabora builds).
pub fn parse_soffice_version(text: &str) -> Option<String> {
    let line = text.lines().next().filter(|l| l.contains("Office"))?;
    line.split_whitespace().find(|t| t.starts_with(|c: char| c.is_ascii_digit())).map(String::from)
}

/// `soffice --version`; on Windows `soffice.exe` is a GUI launcher, its console twin `soffice.com` prints.
fn soffice_version(path: &Path) -> Option<String> {
    let version = |p: &Path| output_of(p, &["--version"]).and_then(|t| parse_soffice_version(&t));
    let com = path.with_extension("com");
    version(path).or_else(|| (cfg!(windows) && com.is_file()).then(|| version(&com)).flatten())
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

/// ffmpeg + ffprobe, both or neither.
fn probe_media(dirs: &[PathBuf], missing: &mut Vec<&'static str>) -> Option<Toolchain> {
    let ffmpeg = find("ffmpeg", dirs);
    let ffprobe = find("ffprobe", dirs);
    let (Some(ffmpeg), Some(ffprobe)) = (ffmpeg.clone(), ffprobe.clone()) else {
        if ffmpeg.is_none() {
            missing.push("ffmpeg");
        }
        if ffprobe.is_none() {
            missing.push("ffprobe");
        }
        return None;
    };
    let version = |path: &Path, name: &'static str| output_of(path, &["-hide_banner", "-version"]).and_then(|t| parse_version(name, &t));
    let (Some(ffmpeg_version), Some(ffprobe_version)) = (version(&ffmpeg, "ffmpeg"), version(&ffprobe, "ffprobe")) else {
        missing.extend(MEDIA);
        return None;
    };
    let filters = output_of(&ffmpeg, &["-hide_banner", "-filters"]).unwrap_or_default();
    let encoders = output_of(&ffmpeg, &["-hide_banner", "-encoders"]).unwrap_or_default();
    Some(Toolchain {
        heif: at_least(&ffmpeg_version, 7, 1) && at_least(&ffprobe_version, 7, 1),
        ffmpeg: Tool { path: ffmpeg, version: ffmpeg_version },
        ffprobe: Tool { path: ffprobe, version: ffprobe_version },
        tonemap: listed(&filters, "zscale") && listed(&filters, "tonemap"),
        libx264: listed(&encoders, "libx264"),
    })
}

/// Finds and runs every binary (blocking: call off the async runtime).
pub fn probe(dirs: &[PathBuf]) -> Probe {
    let mut missing = Vec::new();
    let media = probe_media(dirs, &mut missing);
    let soffice = find("soffice", dirs).and_then(|path| soffice_version(&path).map(|version| Tool { path, version }));
    if soffice.is_none() {
        missing.push("soffice");
    }
    Probe { media, soffice, missing, searched: dirs.to_vec() }
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
        assert_eq!(parse_soffice_version("LibreOffice 25.2.3.2 520(Build:2)\n\n").as_deref(), Some("25.2.3.2"));
        assert_eq!(parse_soffice_version("LibreOfficeDev 24.8.0.0.alpha1 abc").as_deref(), Some("24.8.0.0.alpha1"));
        assert_eq!(parse_soffice_version("Collabora Office 24.04.5.2 build").as_deref(), Some("24.04.5.2"));
        assert!(parse_soffice_version("").is_none());
        assert!(parse_soffice_version("usage: soffice [options]").is_none());
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
        assert!(!probe.available() && probe.profiles().is_empty());
        assert_eq!(probe.detail_for(Profile::PreviewDocument), format!("soffice not found on this node (searched {})", dir.path().display()));
    }

    #[test]
    fn each_converter_is_optional() {
        let soffice = Tool { path: "soffice".into(), version: "25.2.3.2".into() };
        let probe = Probe { media: None, soffice: Some(soffice), missing: vec!["ffmpeg", "ffprobe"], searched: Vec::new() };
        assert!(probe.available());
        assert_eq!(probe.profiles(), vec!["preview-document"]);
        assert!(matches!(probe.converter(Profile::PreviewDocument), Some(Converter::Soffice(_))));
        assert!(probe.converter(Profile::Thumbnail).is_none());
        let described = probe.describe();
        assert_eq!(described["soffice"]["version"], "25.2.3.2");
        assert!(described.get("ffmpeg").is_none());
        assert_eq!(described["missing"], serde_json::json!(["ffmpeg", "ffprobe"]));
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
