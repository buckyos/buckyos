//! Converter discovery (preview iPhone media TODO D1/D2, Office import TODO
//! §5.1, preview more formats TODO §3.1), in five groups, each optional:
//! `media` (ffmpeg + ffprobe), `office` (LibreOffice `soffice`), `image`
//! (ImageMagick 7 `magick`), `pdf` (poppler `pdftoppm` + `pdfinfo`) and
//! `markup` (pandoc). Lookup order is `$BUCKYOS_ROOT/libexec/buckyos-tool/bin/`,
//! then `PATH`, then fixed locations a service PATH lacks (macOS Homebrew
//! prefixes; LibreOffice's install directories on all three platforms), then
//! the extension packages in `$BUCKYOS_ROOT/tools/bin/`; Windows
//! names carry `.exe` and spawn without a console. Nothing is downloaded or
//! installed. Every `soffice` found is kept with the applications it has
//! (Writer / Calc / Impress, by their module libraries): a document goes to
//! the first install that has its application, so a system LibreOffice
//! without Calc leaves spreadsheets to the extension package's. `soffice` is
//! part of ext-core: none at all is a deployment defect, not a degradation.
//! The probe result is cached per binary identity (path, size, mtime) and
//! shown in `hello` so a DV run can tell which path ran.

use super::convert::Profile;
use super::media::{Kind, Office};
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
    /// `ext-tools` for `$BUCKYOS_ROOT/tools/bin`, otherwise `system`.
    pub source: &'static str,
}

impl Tool {
    pub fn new(path: PathBuf, version: String) -> Tool {
        let source = if path.parent() == Some(ext_tools_dir().as_path()) { "ext-tools" } else { "system" };
        Tool { path, version, source }
    }
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

/// One LibreOffice install and the applications it can load.
#[derive(Debug, Clone, Serialize)]
pub struct Soffice {
    #[serde(flatten)]
    pub tool: Tool,
    pub components: Vec<Office>,
}

/// poppler: the first page of a PDF as a thumbnail.
#[derive(Debug, Clone, Serialize)]
pub struct PdfTools {
    pub pdftoppm: Tool,
    pub pdfinfo: Tool,
}

/// Binaries `refresh` stats; a changed identity re-probes.
pub const NAMES: &[&str] = &["ffmpeg", "ffprobe", "soffice", "magick", "pdftoppm", "pdfinfo", "pandoc"];
const MEDIA: &[&str] = &["ffmpeg", "ffprobe"];
const OFFICE: &[&str] = &["soffice"];
const IMAGE: &[&str] = &["magick"];
const PDF: &[&str] = &["pdftoppm", "pdfinfo"];
const MARKUP: &[&str] = &["pandoc"];
const SOFFICE_DEFECT: &str = "soffice is part of ext-core: this is a deployment defect";

/// Shrinks an embedded thumbnail to the thumbnail size.
#[derive(Debug, Clone)]
pub enum Scaler {
    Ffmpeg(Toolchain),
    Magick(Tool),
}

/// What converts a plan.
#[derive(Debug, Clone)]
pub enum Converter {
    Ffmpeg(Toolchain),
    Soffice(Tool),
    Magick(Tool),
    Pandoc(Tool),
    /// The first page of a PDF (the source, or a cached `preview-document` result).
    Pdftoppm(PdfTools),
    /// The thumbnail a document carries inside itself; no tool but an optional scaler.
    Embedded(Option<Scaler>),
}

/// The converters found; `repr` is advertised while any one is.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub media: Option<Toolchain>,
    /// In lookup order.
    pub office: Vec<Soffice>,
    pub magick: Option<Tool>,
    pub pdf: Option<PdfTools>,
    pub pandoc: Option<Tool>,
    /// Binaries not found (or not runnable), e.g. `["soffice"]`.
    pub missing: Vec<&'static str>,
    pub searched: Vec<PathBuf>,
}

impl Probe {
    pub fn toolchain(&self) -> Option<&Toolchain> {
        self.media.as_ref()
    }

    pub fn available(&self) -> bool {
        self.media.is_some() || !self.office.is_empty() || self.magick.is_some() || self.pdf.is_some() || self.pandoc.is_some()
    }

    /// The first install that has `app`.
    pub fn soffice_for(&self, app: Office) -> Option<&Tool> {
        self.office.iter().find(|o| o.components.contains(&app)).map(|o| &o.tool)
    }

    pub fn scaler(&self) -> Option<Scaler> {
        self.media.clone().map(Scaler::Ffmpeg).or_else(|| self.magick.clone().map(Scaler::Magick))
    }

    fn searched_text(&self) -> String {
        self.searched.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    }

    fn soffice_detail(&self, app: Office) -> String {
        match self.office.first() {
            None => format!("soffice is not available on this node ({SOFFICE_DEFECT}; searched {})", self.searched_text()),
            Some(o) => format!(
                "the LibreOffice at {} has no {} and no other soffice has it ({SOFFICE_DEFECT}; searched {})",
                o.tool.path.display(),
                app.name(),
                self.searched_text()
            ),
        }
    }

    /// The converter of `kind` for `profile`, or why this node has none.
    pub fn converter(&self, profile: Profile, kind: Kind) -> Result<Converter, String> {
        let missing = |group: &[&str]| self.detail(&[group]).unwrap_or_default();
        match (profile, kind) {
            (Profile::PreviewDocument, Kind::Document(ext)) => {
                let app = super::media::office_of(ext);
                self.soffice_for(app).cloned().map(Converter::Soffice).ok_or_else(|| self.soffice_detail(app))
            }
            (Profile::PreviewSheet, Kind::Document(_)) => self.soffice_for(Office::Calc).cloned().map(Converter::Soffice).ok_or_else(|| self.soffice_detail(Office::Calc)),
            (Profile::PreviewHtml, Kind::Markup(_)) => self.pandoc.clone().map(Converter::Pandoc).ok_or_else(|| missing(MARKUP)),
            (Profile::PreviewImage | Profile::Thumbnail, Kind::Raster(_)) => self.magick.clone().map(Converter::Magick).ok_or_else(|| missing(IMAGE)),
            (Profile::Thumbnail, Kind::Pdf) => self.pdf.clone().map(Converter::Pdftoppm).ok_or_else(|| missing(PDF)),
            (Profile::Thumbnail, Kind::Document(_) | Kind::Markup(_)) => Ok(Converter::Embedded(self.scaler())),
            (Profile::PreviewImage | Profile::PreviewVideo | Profile::PreviewAudio | Profile::Thumbnail, Kind::Heif | Kind::Image | Kind::Video | Kind::Audio) => {
                self.media.clone().map(Converter::Ffmpeg).ok_or_else(|| missing(MEDIA))
            }
            _ => Err(format!("no {} conversion for this content", profile.as_str())),
        }
    }

    fn advertised(&self, profile: Profile) -> bool {
        match profile {
            Profile::PreviewImage => self.media.is_some() || self.magick.is_some(),
            Profile::PreviewVideo | Profile::PreviewAudio => self.media.is_some(),
            Profile::Thumbnail => self.media.is_some() || self.magick.is_some() || self.pdf.is_some(),
            Profile::PreviewDocument => !self.office.is_empty(),
            Profile::PreviewSheet => self.soffice_for(Office::Calc).is_some(),
            Profile::PreviewHtml => self.pandoc.is_some(),
        }
    }

    /// The profiles this node can produce now.
    pub fn profiles(&self) -> Vec<&'static str> {
        super::PROFILES.iter().filter(|p| self.advertised(**p)).map(Profile::as_str).collect()
    }

    fn detail(&self, groups: &[&[&str]]) -> Option<String> {
        let sentences: Vec<String> = groups
            .iter()
            .map(|g| self.missing.iter().copied().filter(|m| g.contains(m)).collect::<Vec<_>>())
            .filter(|m| !m.is_empty())
            .map(|m| if m == OFFICE { format!("soffice not found on this node ({SOFFICE_DEFECT})") } else { format!("{} not found on this node", m.join(" and ")) })
            .collect();
        (!sentences.is_empty()).then(|| format!("{} (searched {})", sentences.join("; "), self.searched_text()))
    }

    /// What is missing, e.g. "ffmpeg and ffprobe not found on this node; soffice not found on this node (…) (searched ...)".
    pub fn missing_detail(&self) -> Option<String> {
        self.detail(&[MEDIA, OFFICE, IMAGE, PDF, MARKUP])
    }

    /// The applications no install has (e.g. Calc when the only LibreOffice lacks it).
    pub fn missing_office(&self) -> Vec<Office> {
        if self.office.is_empty() {
            return Vec::new();
        }
        [Office::Writer, Office::Calc, Office::Impress].into_iter().filter(|a| self.soffice_for(*a).is_none()).collect()
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
        if !self.office.is_empty() {
            out["soffice"] = serde_json::json!(self.office);
        }
        if let Some(t) = &self.magick {
            out["magick"] = serde_json::json!(t);
        }
        if let Some(t) = &self.pdf {
            out["pdftoppm"] = serde_json::json!(t.pdftoppm);
            out["pdfinfo"] = serde_json::json!(t.pdfinfo);
        }
        if let Some(t) = &self.pandoc {
            out["pandoc"] = serde_json::json!(t);
        }
        let apps = self.missing_office();
        if !apps.is_empty() {
            out["soffice_missing_components"] = serde_json::json!(apps);
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
    let root = buckyos_kit::get_buckyos_root_dir();
    let mut dirs = vec![root.join("libexec").join("buckyos-tool").join("bin")];
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
    // Last: a machine keeps the converter it had. ext-core's ffmpeg has no `zscale`, so taking it over a system one would lose HDR tone mapping.
    dirs.push(ext_tools_dir());
    dirs
}

pub fn ext_tools_dir() -> PathBuf {
    buckyos_kit::get_buckyos_root_dir().join("tools").join("bin")
}

fn candidates(name: &str, dir: &Path) -> impl Iterator<Item = PathBuf> {
    // On Windows an extension package's launcher is a `.cmd`.
    let launcher = cfg!(windows).then(|| dir.join(format!("{name}.cmd")));
    std::iter::once(dir.join(exe_name(name))).chain(launcher)
}

/// Every `name` in `dirs`, in order, each real file once.
pub fn find_all(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    dirs.iter()
        .flat_map(|d| candidates(name, d))
        .filter(|p| p.is_file())
        .filter(|p| {
            let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            let new = !seen.contains(&real);
            seen.push(real);
            new
        })
        .collect()
}

/// Everything `refresh` keys the cached probe on.
pub fn installed(dirs: &[PathBuf]) -> Vec<PathBuf> {
    NAMES.iter().flat_map(|n| if *n == "soffice" { find_all(n, dirs) } else { find(n, dirs).into_iter().collect() }).collect()
}

pub fn find(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().flat_map(|d| candidates(name, d)).find(|p| p.is_file())
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

fn run(path: &Path, args: &[&str]) -> Option<std::process::Output> {
    let mut cmd = Command::new(path);
    cmd.args(args).stdin(std::process::Stdio::null());
    buckyos_api::hide_child_console(&mut cmd);
    cmd.output().ok().filter(|o| o.status.success())
}

fn output_of(path: &Path, args: &[&str]) -> Option<String> {
    run(path, args).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `Version: ImageMagick 7.1.2-31 Q16-HDRI …` → `7.1.2-31`; ImageMagick 6 is not taken (its default policy blocks formats).
pub fn parse_magick_version(text: &str) -> Option<String> {
    let rest = text.lines().next()?.trim().strip_prefix("Version: ImageMagick")?;
    rest.split_whitespace().next().filter(|v| v.starts_with("7.")).map(String::from)
}

/// `pandoc 3.12.1` (also `pandoc.exe 3.12.1`).
pub fn parse_pandoc_version(text: &str) -> Option<String> {
    let mut words = text.lines().next()?.split_whitespace();
    words.next().filter(|w| w.starts_with("pandoc"))?;
    words.next().filter(|v| v.starts_with(|c: char| c.is_ascii_digit())).map(String::from)
}

fn tool(name: &'static str, dirs: &[PathBuf], missing: &mut Vec<&'static str>, version: impl Fn(&Path) -> Option<String>) -> Option<Tool> {
    let found = find(name, dirs).and_then(|path| version(&path).map(|v| Tool::new(path, v)));
    if found.is_none() {
        missing.push(name);
    }
    found
}

/// poppler prints `pdftoppm version 24.02.0` to stderr.
fn poppler_version(name: &'static str) -> impl Fn(&Path) -> Option<String> {
    move |path| run(path, &["-v"]).and_then(|o| parse_version(name, &String::from_utf8_lossy(&[o.stderr, o.stdout].concat())))
}

fn module_library(module: &str) -> String {
    if cfg!(windows) {
        format!("{module}lo.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{module}lo.dylib")
    } else {
        format!("lib{module}lo.so")
    }
}

/// The applications of the install `soffice` starts: the module libraries next
/// to the real binary (`../Frameworks` on macOS). A launcher that is not inside
/// a LibreOffice program directory (an extension package's) is taken as complete.
pub fn office_components(soffice: &Path) -> Vec<Office> {
    const ALL: [(Office, &str); 3] = [(Office::Writer, "sw"), (Office::Calc, "sc"), (Office::Impress, "sd")];
    let real = std::fs::canonicalize(soffice).unwrap_or_else(|_| soffice.to_path_buf());
    let Some(program) = real.parent() else { return ALL.iter().map(|(a, _)| *a).collect() };
    let frameworks = program.parent().map(|p| p.join("Frameworks")).filter(|p| p.is_dir());
    if !program.join("soffice.bin").is_file() && frameworks.is_none() {
        return ALL.iter().map(|(a, _)| *a).collect();
    }
    let dirs: Vec<&Path> = std::iter::once(program).chain(frameworks.as_deref()).collect();
    ALL.iter().filter(|(_, m)| dirs.iter().any(|d| d.join(module_library(m)).is_file())).map(|(a, _)| *a).collect()
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
        ffmpeg: Tool::new(ffmpeg, ffmpeg_version),
        ffprobe: Tool::new(ffprobe, ffprobe_version),
        tonemap: listed(&filters, "zscale") && listed(&filters, "tonemap"),
        libx264: listed(&encoders, "libx264"),
    })
}

/// Finds and runs every binary (blocking: call off the async runtime).
pub fn probe(dirs: &[PathBuf]) -> Probe {
    let mut missing = Vec::new();
    let media = probe_media(dirs, &mut missing);
    let office: Vec<Soffice> = find_all("soffice", dirs)
        .into_iter()
        .filter_map(|path| soffice_version(&path).map(|version| Soffice { components: office_components(&path), tool: Tool::new(path, version) }))
        .filter(|o| !o.components.is_empty())
        .collect();
    if office.is_empty() {
        missing.push("soffice");
    }
    let magick = tool("magick", dirs, &mut missing, |p| output_of(p, &["-version"]).and_then(|t| parse_magick_version(&t)));
    let pdftoppm = tool("pdftoppm", dirs, &mut missing, poppler_version("pdftoppm"));
    let pdfinfo = tool("pdfinfo", dirs, &mut missing, poppler_version("pdfinfo"));
    let pdf = pdftoppm.zip(pdfinfo).map(|(pdftoppm, pdfinfo)| PdfTools { pdftoppm, pdfinfo });
    let pandoc = tool("pandoc", dirs, &mut missing, |p| output_of(p, &["--version"]).and_then(|t| parse_pandoc_version(&t)));
    Probe { media, office, magick, pdf, pandoc, missing, searched: dirs.to_vec() }
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
        assert_eq!(parse_magick_version("Version: ImageMagick 7.1.2-31 Q16-HDRI x86_64 23490 https://imagemagick.org").as_deref(), Some("7.1.2-31"));
        assert!(parse_magick_version("Version: ImageMagick 6.9.12-98 Q16 x86_64").is_none());
        assert_eq!(parse_pandoc_version("pandoc 3.12.1\nFeatures: +server +lua").as_deref(), Some("3.12.1"));
        assert_eq!(parse_version("pdftoppm", "pdftoppm version 24.02.0\nCopyright").as_deref(), Some("24.02.0"));
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
        assert!(detail.contains("magick not found") && detail.contains("pdftoppm and pdfinfo not found") && detail.contains("pandoc not found"), "{detail}");
        assert!(!probe.available() && probe.profiles().is_empty());
        let why = probe.converter(Profile::PreviewDocument, Kind::Document("docx")).unwrap_err();
        assert_eq!(why, format!("soffice is not available on this node (soffice is part of ext-core: this is a deployment defect; searched {})", dir.path().display()));
    }

    fn soffice(path: &str, components: &[Office]) -> Soffice {
        Soffice { tool: Tool::new(path.into(), "25.2.3.2".into()), components: components.to_vec() }
    }

    #[test]
    fn each_group_is_optional() {
        let probe = Probe { office: vec![soffice("soffice", &[Office::Writer, Office::Calc, Office::Impress])], missing: vec!["ffmpeg", "ffprobe"], ..Default::default() };
        assert!(probe.available());
        assert_eq!(probe.profiles(), vec!["preview-document", "preview-sheet"]);
        assert!(matches!(probe.converter(Profile::PreviewDocument, Kind::Document("pptx")), Ok(Converter::Soffice(_))));
        assert!(probe.converter(Profile::Thumbnail, Kind::Video).is_err());
        assert!(matches!(probe.converter(Profile::Thumbnail, Kind::Document("odt")), Ok(Converter::Embedded(None))));
        let described = probe.describe();
        assert_eq!(described["soffice"][0]["version"], "25.2.3.2");
        assert_eq!(described["soffice"][0]["source"], "system");
        assert_eq!(described["soffice"][0]["components"], serde_json::json!(["writer", "calc", "impress"]));
        assert!(described.get("ffmpeg").is_none());
        assert_eq!(described["missing"], serde_json::json!(["ffmpeg", "ffprobe"]));

        let tool = |name: &str| Tool::new(name.into(), "1".into());
        let probe = Probe { magick: Some(tool("magick")), pandoc: Some(tool("pandoc")), pdf: Some(PdfTools { pdftoppm: tool("pdftoppm"), pdfinfo: tool("pdfinfo") }), missing: vec!["ffmpeg", "ffprobe", "soffice"], ..Default::default() };
        assert_eq!(probe.profiles(), vec!["preview-image", "thumbnail", "preview-html"]);
        assert!(matches!(probe.converter(Profile::PreviewImage, Kind::Raster("dng")), Ok(Converter::Magick(_))));
        assert!(matches!(probe.converter(Profile::PreviewHtml, Kind::Markup("epub")), Ok(Converter::Pandoc(_))));
        assert!(matches!(probe.converter(Profile::Thumbnail, Kind::Pdf), Ok(Converter::Pdftoppm(_))));
        assert!(matches!(probe.converter(Profile::Thumbnail, Kind::Markup("epub")), Ok(Converter::Embedded(Some(Scaler::Magick(_))))));
        assert!(probe.converter(Profile::PreviewImage, Kind::Image).unwrap_err().contains("ffmpeg and ffprobe not found"));
    }

    #[test]
    fn a_document_goes_to_the_first_install_with_its_application() {
        let probe = Probe { office: vec![soffice("/usr/bin/soffice", &[Office::Writer, Office::Impress]), soffice("/opt/buckyos/tools/bin/soffice", &[Office::Writer, Office::Calc, Office::Impress])], ..Default::default() };
        let path = |kind| match probe.converter(Profile::PreviewDocument, kind) {
            Ok(Converter::Soffice(t)) => t.path,
            other => panic!("{other:?}"),
        };
        assert_eq!(path(Kind::Document("docx")), PathBuf::from("/usr/bin/soffice"));
        assert_eq!(path(Kind::Document("xlsx")), PathBuf::from("/opt/buckyos/tools/bin/soffice"));
        assert!(probe.missing_office().is_empty());

        let probe = Probe { office: vec![soffice("/usr/bin/soffice", &[Office::Writer, Office::Impress])], ..Default::default() };
        assert_eq!(probe.profiles(), vec!["preview-document"]);
        let why = probe.converter(Profile::PreviewSheet, Kind::Document("xls")).unwrap_err();
        assert!(why.starts_with("the LibreOffice at /usr/bin/soffice has no Calc and no other soffice has it"), "{why}");
        assert_eq!(probe.describe()["soffice_missing_components"], serde_json::json!(["calc"]));
    }

    #[test]
    fn applications_are_read_from_the_program_directory() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("lib/program");
        std::fs::create_dir_all(&program).unwrap();
        let launcher = dir.path().join("bin");
        std::fs::create_dir_all(&launcher).unwrap();
        std::fs::write(launcher.join("soffice"), b"#!/bin/sh\n").unwrap();
        assert_eq!(office_components(&launcher.join("soffice")), vec![Office::Writer, Office::Calc, Office::Impress], "a launcher outside a program directory is complete");
        for f in ["soffice", "soffice.bin", &module_library("sw"), &module_library("sd")] {
            std::fs::write(program.join(f), b"").unwrap();
        }
        assert_eq!(office_components(&program.join("soffice")), vec![Office::Writer, Office::Impress]);
        std::fs::write(program.join(module_library("sc")), b"").unwrap();
        assert_eq!(office_components(&program.join("soffice")), vec![Office::Writer, Office::Calc, Office::Impress]);
        assert_eq!(find_all("soffice", &[launcher.clone(), program.clone(), launcher.clone()]), vec![launcher.join("soffice"), program.join("soffice")]);
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
