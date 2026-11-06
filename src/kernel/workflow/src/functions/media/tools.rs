//! Media tool discovery for the workflow host (copied from the nfs_server
//! preview probe so the kernel does not depend on a frame crate): poppler,
//! ffmpeg and LibreOffice looked up in `$BUCKYOS_ROOT/libexec/buckyos-tool/bin`,
//! `PATH`, then fixed install locations. The result is a [`ToolEnvironment`]
//! whose per-tool versions form function fingerprints.

use crate::native::{ToolEnvironment, ToolInfo};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PDFINFO: &str = "pdfinfo";
pub const PDFTOTEXT: &str = "pdftotext";
pub const PDFTOPPM: &str = "pdftoppm";
pub const FFMPEG: &str = "ffmpeg";
pub const FFPROBE: &str = "ffprobe";
pub const SOFFICE: &str = "soffice";

/// Every tool the built-in media functions may need.
pub const MEDIA_TOOLS: &[&str] = &[PDFINFO, PDFTOTEXT, PDFTOPPM, FFMPEG, FFPROBE, SOFFICE];

pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Directories searched, in order.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![buckyos_kit::get_buckyos_root_dir()
        .join("libexec")
        .join("buckyos-tool")
        .join("bin")];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    let mut extras: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        extras.push("/opt/homebrew/bin".into());
        extras.push("/usr/local/bin".into());
        extras.push("/Applications/LibreOffice.app/Contents/MacOS".into());
    } else if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(base) {
                extras.push(PathBuf::from(dir).join("LibreOffice").join("program"));
            }
        }
    } else {
        extras.push("/usr/lib/libreoffice/program".into());
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

fn output_of(path: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.args(args).stdin(std::process::Stdio::null());
    buckyos_api::hide_child_console(&mut cmd);
    let out = cmd.output().ok()?;
    let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
    Some(String::from_utf8_lossy(&text).into_owned())
}

/// `ffmpeg version 7.1.5-0+deb13u1 ...` -> `7.1.5-0+deb13u1`.
pub fn parse_version(name: &str, text: &str) -> Option<String> {
    let line = text.lines().next()?;
    let rest = line
        .trim()
        .strip_prefix(name)?
        .trim_start()
        .strip_prefix("version")?
        .trim_start();
    rest.split_whitespace().next().map(String::from)
}

/// `pdfinfo version 24.02.0` / `pdftoppm version 22.12.0` -> the version token.
pub fn parse_poppler_version(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.contains("version"))
        .and_then(|l| l.split("version").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .map(String::from)
}

/// `LibreOffice 25.2.3.2 520(Build:2)` -> `25.2.3.2`.
pub fn parse_soffice_version(text: &str) -> Option<String> {
    let line = text.lines().next().filter(|l| l.contains("Office"))?;
    line.split_whitespace()
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
        .map(String::from)
}

fn soffice_version(path: &Path) -> Option<String> {
    let version = |p: &Path| output_of(p, &["--version"]).and_then(|t| parse_soffice_version(&t));
    let com = path.with_extension("com");
    version(path).or_else(|| (cfg!(windows) && com.is_file()).then(|| version(&com)).flatten())
}

fn version_of(name: &str, path: &Path) -> Option<String> {
    match name {
        FFMPEG | FFPROBE => {
            output_of(path, &["-hide_banner", "-version"]).and_then(|t| parse_version(name, &t))
        }
        PDFINFO | PDFTOTEXT | PDFTOPPM => output_of(path, &["-v"]).and_then(|t| parse_poppler_version(&t)),
        SOFFICE => soffice_version(path),
        _ => None,
    }
}

/// Finds and runs every media tool (blocking: call off the async runtime).
pub fn probe(dirs: &[PathBuf]) -> ToolEnvironment {
    let mut tools = BTreeMap::new();
    let mut missing = Vec::new();
    for name in MEDIA_TOOLS {
        match find(name, dirs).and_then(|path| version_of(name, &path).map(|version| ToolInfo { path, version })) {
            Some(info) => {
                tools.insert(name.to_string(), info);
            }
            None => missing.push(name.to_string()),
        }
    }
    let mut env = ToolEnvironment::with_tools(tools);
    env.missing = missing;
    env.searched = dirs.to_vec();
    env.fonts = font_set_digest(dirs);
    env.revision = env.compute_revision();
    env
}

/// Digest of the fonts fontconfig sees; `None` where `fc-list` is not available.
fn font_set_digest(dirs: &[PathBuf]) -> Option<String> {
    let fc_list = find("fc-list", dirs)?;
    let listed = output_of(&fc_list, &["--format", "%{file}|%{family}|%{style}\n"])?;
    Some(digest_of_font_list(&listed))
}

fn digest_of_font_list(listed: &str) -> String {
    let mut fonts: Vec<&str> = listed.lines().filter(|l| !l.trim().is_empty()).collect();
    fonts.sort_unstable();
    let (id, _) = ndn_lib::build_named_object_by_json("fontset", &serde_json::json!(fonts));
    hex::encode(&id.obj_hash)
}

pub fn probe_default() -> ToolEnvironment {
    probe(&search_dirs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("ffmpeg", "ffmpeg version 7.1.5-0+deb13u1 Copyright (c) 2000-2026").as_deref(),
            Some("7.1.5-0+deb13u1")
        );
        assert_eq!(parse_poppler_version("pdfinfo version 24.02.0\nCopyright").as_deref(), Some("24.02.0"));
        assert_eq!(parse_soffice_version("LibreOffice 25.2.3.2 520(Build:2)\n").as_deref(), Some("25.2.3.2"));
        assert!(parse_soffice_version("usage: soffice [options]").is_none());
    }

    #[test]
    fn the_font_digest_ignores_order_and_follows_the_set() {
        let a = digest_of_font_list("/f/a.ttf|A|Regular\n/f/b.ttf|B|Bold\n");
        assert_eq!(a, digest_of_font_list("/f/b.ttf|B|Bold\n/f/a.ttf|A|Regular\n"));
        assert_ne!(a, digest_of_font_list("/f/a.ttf|A|Regular\n"));
    }

    #[test]
    fn to_pdf_is_fingerprinted_by_its_fonts() {
        use crate::functions::media::document::ToPdf;
        use crate::native::NativeFunction;
        let mut env = ToolEnvironment::default();
        env.fonts = Some("set-a".to_string());
        let first = ToPdf.fingerprint(&env);
        env.fonts = Some("set-b".to_string());
        assert_ne!(first, ToPdf.fingerprint(&env));
        assert!(first.contains_key("cmd/soffice") && first.contains_key("cmd/pdfinfo"));
    }

    #[test]
    fn missing_tools_are_reported_and_fingerprinted() {
        let dir = tempfile::tempdir().unwrap();
        let env = probe(&[dir.path().to_path_buf()]);
        assert_eq!(env.missing.len(), MEDIA_TOOLS.len());
        assert!(env.tools.is_empty());
        let fp = env.fingerprint_for(&[PDFTOPPM]);
        assert_eq!(fp.get("cmd/pdftoppm").map(String::as_str), Some("missing"));
        assert_eq!(env.missing_from(&[PDFTOPPM, FFMPEG]), vec!["pdftoppm", "ffmpeg"]);
    }
}
