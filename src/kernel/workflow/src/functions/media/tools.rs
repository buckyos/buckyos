//! Media tool discovery for the workflow host (copied from the nfs_server
//! preview probe so the kernel does not depend on a frame crate): poppler,
//! ffmpeg, LibreOffice and `page-render` looked up in `$BUCKYOS_ROOT/libexec/buckyos-tool/bin`,
//! `PATH`, fixed install locations, then the extension packages in `$BUCKYOS_ROOT/tools/bin`. The result is a [`ToolEnvironment`]
//! whose per-tool versions form function fingerprints.

use crate::native::{EnvironmentProvider, ToolEnvironment, ToolInfo};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub const PDFINFO: &str = "pdfinfo";
pub const PDFTOTEXT: &str = "pdftotext";
pub const PDFTOPPM: &str = "pdftoppm";
pub const FFMPEG: &str = "ffmpeg";
pub const FFPROBE: &str = "ffprobe";
pub const SOFFICE: &str = "soffice";
pub const TESSERACT: &str = "tesseract";
pub const PAGE_RENDER: &str = "page-render";

/// Every tool the built-in media functions may need.
pub const MEDIA_TOOLS: &[&str] = &[PDFINFO, PDFTOTEXT, PDFTOPPM, FFMPEG, FFPROBE, SOFFICE, TESSERACT, PAGE_RENDER];

pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Directories searched, in order.
pub fn search_dirs() -> Vec<PathBuf> {
    let root = buckyos_kit::get_buckyos_root_dir();
    let mut dirs = vec![root.join("libexec").join("buckyos-tool").join("bin")];
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
    // Last: a machine keeps the converter it had. ext-core's ffmpeg has no `zscale`, so taking it over a system one would lose HDR tone mapping.
    dirs.push(root.join("tools").join("bin"));
    dirs
}

pub fn find(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let exe = exe_name(name);
    // On Windows an extension package's launcher is a `.cmd`.
    let launcher = cfg!(windows).then(|| format!("{name}.cmd"));
    dirs.iter()
        .flat_map(|d| std::iter::once(d.join(&exe)).chain(launcher.as_ref().map(|l| d.join(l))))
        .find(|p| p.is_file())
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

/// `tesseract 5.5.3\n leptonica-1.87.0 ...` -> `5.5.3`.
pub fn parse_tesseract_version(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.trim_start().starts_with("tesseract"))?;
    line.split_whitespace()
        .nth(1)
        .map(|v| v.trim_start_matches('v').to_string())
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

/// The language data of `tesseract --list-langs`, sorted.
pub fn parse_tesseract_langs(text: &str) -> Vec<String> {
    let mut langs: Vec<String> = text
        .lines()
        .skip_while(|l| !l.starts_with("List of available languages"))
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .map(str::to_string)
        .collect();
    langs.sort();
    langs.dedup();
    langs
}

fn soffice_version(path: &Path) -> Option<String> {
    let version = |p: &Path| output_of(p, &["--version"]).and_then(|t| parse_soffice_version(&t));
    let com = path.with_extension("com");
    version(path).or_else(|| (cfg!(windows) && com.is_file()).then(|| version(&com)).flatten())
}

/// The line `page-render --version` prints: its own version, the browser's and
/// playwright-core's. A launcher that exits with an error is not a tool.
fn page_render_version(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version").stdin(std::process::Stdio::null());
    buckyos_api::hide_child_console(&mut cmd);
    let out = cmd.output().ok().filter(|out| out.status.success())?;
    parse_page_render_version(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_page_render_version(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|line| !line.is_empty()).map(String::from)
}

fn version_of(name: &str, path: &Path) -> Option<String> {
    match name {
        PAGE_RENDER => page_render_version(path),
        FFMPEG | FFPROBE => {
            output_of(path, &["-hide_banner", "-version"]).and_then(|t| parse_version(name, &t))
        }
        PDFINFO | PDFTOTEXT | PDFTOPPM => output_of(path, &["-v"]).and_then(|t| parse_poppler_version(&t)),
        SOFFICE => soffice_version(path),
        TESSERACT => output_of(path, &["--version"]).and_then(|t| parse_tesseract_version(&t)),
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
    if let Some(tesseract) = env.tools.get(TESSERACT) {
        env.ocr_langs = output_of(&tesseract.path, &["--list-langs"])
            .map(|t| parse_tesseract_langs(&t))
            .unwrap_or_default();
    }
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

pub struct MediaEnvironment(RwLock<Arc<ToolEnvironment>>);

impl EnvironmentProvider for MediaEnvironment {
    fn current(&self) -> Arc<ToolEnvironment> {
        self.0.read().unwrap().clone()
    }
}

impl MediaEnvironment {
    pub async fn start() -> Result<Arc<Self>, tokio::task::JoinError> {
        Self::start_with(None, Duration::from_secs(30)).await
    }

    async fn start_with(
        dirs: Option<Vec<PathBuf>>,
        interval: Duration,
    ) -> Result<Arc<Self>, tokio::task::JoinError> {
        let initial_dirs = dirs.clone();
        let initial = tokio::task::spawn_blocking(move || match initial_dirs {
            Some(dirs) => probe(&dirs),
            None => probe_default(),
        })
        .await?;
        Self::log_environment(&initial);
        let environment = Arc::new(Self(RwLock::new(Arc::new(initial))));
        let weak = Arc::downgrade(&environment);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if weak.strong_count() == 0 {
                    break;
                }
                let dirs = dirs.clone();
                let next = tokio::task::spawn_blocking(move || match dirs {
                    Some(dirs) => probe(&dirs),
                    None => probe_default(),
                })
                .await;
                let Some(environment) = weak.upgrade() else {
                    break;
                };
                match next {
                    Ok(next) => {
                        if environment.current().revision != next.revision {
                            Self::log_environment(&next);
                        }
                        *environment.0.write().unwrap() = Arc::new(next);
                    }
                    Err(err) => log::warn!("workflow media tool refresh failed: {err}"),
                }
            }
        });
        Ok(environment)
    }

    fn log_environment(environment: &ToolEnvironment) {
        log::info!(
            "workflow media tools revision={} present={:?} missing={:?}",
            environment.revision,
            environment.tools.keys().collect::<Vec<_>>(),
            environment.missing,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_tool_wins_over_the_extension_package() {
        assert!(search_dirs().last().unwrap().ends_with("tools/bin"));
    }

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("ffmpeg", "ffmpeg version 7.1.5-0+deb13u1 Copyright (c) 2000-2026").as_deref(),
            Some("7.1.5-0+deb13u1")
        );
        assert_eq!(parse_poppler_version("pdfinfo version 24.02.0\nCopyright").as_deref(), Some("24.02.0"));
        assert_eq!(parse_soffice_version("LibreOffice 25.2.3.2 520(Build:2)\n").as_deref(), Some("25.2.3.2"));
        assert!(parse_soffice_version("usage: soffice [options]").is_none());
        assert_eq!(parse_tesseract_version("tesseract 5.5.3\n leptonica-1.87.0\n").as_deref(), Some("5.5.3"));
        assert_eq!(parse_tesseract_version("tesseract v4.1.1\n").as_deref(), Some("4.1.1"));
        let langs = parse_tesseract_langs("List of available languages in \"/x/tessdata/\" (3):\nosd\neng\nchi_sim\n");
        assert_eq!(langs, vec!["chi_sim", "eng", "osd"]);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn running_environment_discovers_installs_upgrades_and_removals() {
        use std::os::unix::fs::PermissionsExt;

        async fn wait_for_version(environment: &MediaEnvironment, version: Option<&str>) -> Arc<ToolEnvironment> {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let current = environment.current();
                    if current.tool(PDFINFO).map(|tool| tool.version.as_str()) == version {
                        return current;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap()
        }

        let dir = tempfile::tempdir().unwrap();
        let environment = MediaEnvironment::start_with(Some(vec![dir.path().to_path_buf()]), Duration::from_millis(20)).await.unwrap();
        let missing = environment.current();
        assert_eq!(missing.missing_from(&[PDFINFO]), vec![PDFINFO]);
        let install = |version: &str| {
            let staging = dir.path().join("pdfinfo.new");
            std::fs::write(&staging, format!("#!/bin/sh\nprintf 'pdfinfo version {version}\\n'\n")).unwrap();
            std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::rename(staging, dir.path().join(PDFINFO)).unwrap();
        };

        install("24.02.0");
        let installed = wait_for_version(&environment, Some("24.02.0")).await;
        assert!(installed.missing_from(&[PDFINFO]).is_empty());
        assert_ne!(missing.revision, installed.revision);
        assert_ne!(missing.fingerprint_for(&[PDFINFO]), installed.fingerprint_for(&[PDFINFO]));
        assert!(missing.tool(PDFINFO).is_none());

        install("25.01.0");
        let upgraded = wait_for_version(&environment, Some("25.01.0")).await;
        assert_ne!(installed.revision, upgraded.revision);
        assert_ne!(installed.fingerprint_for(&[PDFINFO]), upgraded.fingerprint_for(&[PDFINFO]));
        assert_eq!(installed.tool(PDFINFO).unwrap().version, "24.02.0");

        std::fs::remove_file(dir.path().join(PDFINFO)).unwrap();
        let removed = wait_for_version(&environment, None).await;
        assert_eq!(removed.missing_from(&[PDFINFO]), vec![PDFINFO]);
        assert_eq!(missing.revision, removed.revision);
    }
}
