//! Converter discovery (preview iPhone media TODO D1/D2, Office import TODO
//! §5.1, preview more formats TODO §3.1), in five groups, each optional:
//! `media` (ffmpeg + ffprobe), `office` (LibreOffice `soffice`), `image`
//! (ImageMagick 7 `magick`), `pdf` (poppler `pdftoppm` + `pdfinfo`) and
//! `markup` (pandoc). Lookup order is `$BUCKYOS_ROOT/libexec/buckyos-tool/bin/`,
//! then `PATH`, then fixed locations a service PATH lacks (macOS Homebrew
//! prefixes; LibreOffice's install directories on all three platforms), then
//! the extension packages in `$BUCKYOS_ROOT/tools/bin/`; Windows
//! names carry `.exe` and spawn without a console. Nothing is downloaded or
//! installed. Every `soffice` found is kept with what its install can load,
//! read from the install the binary starts (a launcher script is followed to
//! the `soffice` it runs): the applications (Writer / Calc / Impress / Draw, by
//! their module library and their registry `*.xcd`) and the import filters a
//! format needs on top (Visio: Draw's registration of the filter and the
//! library that implements it). A document goes to the first install that has
//! both, so a system LibreOffice without Calc or without the Visio filter
//! leaves those documents to the extension package's. `soffice` is part of
//! ext-core: none at all is a deployment defect, not a degradation. The probe
//! result is cached per identity (path, size, mtime) of the binaries and of
//! each LibreOffice install's libraries, registry and service files, and shown
//! in `hello` so a DV run can tell which path ran.

use super::convert::Profile;
use super::media::{filter_of, office_of, Filter, Kind, Office};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
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

/// One LibreOffice install, the applications and the import filters it can load.
#[derive(Debug, Clone, Serialize)]
pub struct Soffice {
    #[serde(flatten)]
    pub tool: Tool,
    pub components: Vec<Office>,
    pub filters: Vec<Filter>,
}

impl Soffice {
    pub fn has(&self, app: Office, filter: Option<Filter>) -> bool {
        self.components.contains(&app) && filter.is_none_or(|f| self.filters.contains(&f))
    }
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
    /// `soffice` binaries whose LibreOffice install could not be read (a launcher that names no `soffice`).
    pub unresolved: Vec<PathBuf>,
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

    /// The first install that has `app` (and `filter`).
    pub fn soffice_with(&self, app: Office, filter: Option<Filter>) -> Option<&Tool> {
        self.office.iter().find(|o| o.has(app, filter)).map(|o| &o.tool)
    }

    pub fn soffice_for(&self, app: Office) -> Option<&Tool> {
        self.soffice_with(app, None)
    }

    /// The install that converts a document of this import extension to PDF.
    pub fn soffice_for_document(&self, ext: &str) -> Option<&Tool> {
        self.soffice_with(office_of(ext), filter_of(ext))
    }

    pub fn scaler(&self) -> Option<Scaler> {
        self.media.clone().map(Scaler::Ffmpeg).or_else(|| self.magick.clone().map(Scaler::Magick))
    }

    fn searched_text(&self) -> String {
        self.searched.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    }

    /// Why no install takes a document, for the person looking at it: paths and versions are in `hello` and the log.
    fn soffice_detail(&self, app: Office, filter: Option<Filter>) -> String {
        if self.office.is_empty() {
            return format!("soffice is not available on this node ({SOFFICE_DEFECT})");
        }
        let what = match filter {
            Some(f) if self.soffice_for(app).is_some() => format!("{} with {}", app.name(), f.name()),
            _ => app.name().to_string(),
        };
        format!("the LibreOffice installs on this node have no {what} ({SOFFICE_DEFECT})")
    }

    /// The converter of `kind` for `profile`, or why this node has none.
    pub fn converter(&self, profile: Profile, kind: Kind) -> Result<Converter, String> {
        let missing = |group: &[&str]| self.detail(&[group], false).unwrap_or_default();
        match (profile, kind) {
            (Profile::PreviewDocument, Kind::Document(ext)) => {
                self.soffice_for_document(ext).cloned().map(Converter::Soffice).ok_or_else(|| self.soffice_detail(office_of(ext), filter_of(ext)))
            }
            (Profile::PreviewSheet, Kind::Document(_)) => self.soffice_for(Office::Calc).cloned().map(Converter::Soffice).ok_or_else(|| self.soffice_detail(Office::Calc, None)),
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

    /// What of `groups` is missing; `searched` adds the directories looked in (diagnostics, not what a viewer is told).
    fn detail(&self, groups: &[&[&str]], searched: bool) -> Option<String> {
        let sentences: Vec<String> = groups
            .iter()
            .map(|g| self.missing.iter().copied().filter(|m| g.contains(m)).collect::<Vec<_>>())
            .filter(|m| !m.is_empty())
            .map(|m| if m == OFFICE { format!("soffice not found on this node ({SOFFICE_DEFECT})") } else { format!("{} not found on this node", m.join(" and ")) })
            .collect();
        (!sentences.is_empty()).then(|| if searched { format!("{} (searched {})", sentences.join("; "), self.searched_text()) } else { sentences.join("; ") })
    }

    /// What is missing, e.g. "ffmpeg and ffprobe not found on this node; soffice not found on this node (…) (searched ...)".
    pub fn missing_detail(&self) -> Option<String> {
        self.detail(&[MEDIA, OFFICE, IMAGE, PDF, MARKUP], true)
    }

    /// The applications no install has (e.g. Calc when the only LibreOffice lacks it).
    pub fn missing_office(&self) -> Vec<Office> {
        if self.office.is_empty() {
            return Vec::new();
        }
        Office::ALL.into_iter().filter(|a| self.soffice_for(*a).is_none()).collect()
    }

    /// The import filters no install has together with the application they belong to.
    pub fn missing_filters(&self) -> Vec<Filter> {
        if self.office.is_empty() {
            return Vec::new();
        }
        Filter::ALL.into_iter().filter(|f| !self.office.iter().any(|o| o.filters.contains(f))).collect()
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
        let filters = self.missing_filters();
        if !filters.is_empty() {
            out["soffice_missing_filters"] = serde_json::json!(filters);
        }
        if !self.unresolved.is_empty() {
            out["soffice_unresolved"] = serde_json::json!(self.unresolved);
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
    // Last: a machine keeps the converter it had.
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

/// Everything `refresh` keys the cached probe on: the binaries, and what each LibreOffice install loads its applications and filters from.
pub fn installed(dirs: &[PathBuf]) -> Vec<PathBuf> {
    NAMES
        .iter()
        .flat_map(|n| if *n == "soffice" { find_all(n, dirs).into_iter().flat_map(|p| std::iter::once(p.clone()).chain(office_markers(&p))).collect() } else { find(n, dirs).into_iter().collect::<Vec<_>>() })
        .collect()
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

/// Working directory of every converter process. Inherited, it would be the
/// service's package under `$BUCKYOS_ROOT/bin`: a converter that never exits
/// (one wedged in an uncancellable kernel wait outlives a kill) then keeps that
/// directory open and an update cannot replace the package. The temporary
/// directory itself is never removed. Inputs and outputs are absolute paths.
pub(super) fn child_dir() -> Option<&'static Path> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| Some(std::env::temp_dir()).filter(|dir| dir.is_dir()))
        .as_deref()
}

fn run(path: &Path, args: &[&str]) -> Option<std::process::Output> {
    let mut cmd = Command::new(path);
    cmd.args(args).stdin(std::process::Stdio::null());
    if let Some(dir) = child_dir() {
        cmd.current_dir(dir);
    }
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

pub fn module_library(module: &str) -> String {
    if cfg!(windows) {
        format!("{module}lo.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{module}lo.dylib")
    } else {
        format!("lib{module}lo.so")
    }
}

/// Where a LibreOffice install keeps what the probe reads: the module libraries, the registry (`*.xcd`) and the UNO service registries.
struct Layout {
    libs: Vec<PathBuf>,
    registry: PathBuf,
    services: Vec<PathBuf>,
}

/// The layout around the real `soffice`: `program/` with `soffice.bin` (Linux, Windows) or `Contents/MacOS` (macOS).
fn layout_of(real: &Path) -> Option<Layout> {
    let program = real.parent()?;
    let root = program.parent()?;
    if program.join("soffice.bin").is_file() {
        return Some(Layout { libs: vec![program.to_path_buf()], registry: root.join("share").join("registry"), services: vec![program.join("services"), program.to_path_buf()] });
    }
    let frameworks = root.join("Frameworks");
    let resources = root.join("Resources");
    (frameworks.is_dir() && resources.is_dir()).then(|| Layout {
        libs: vec![program.to_path_buf(), frameworks],
        registry: resources.join("registry"),
        services: vec![resources.join("services"), resources],
    })
}

/// `$name` / `${name}` (sh) and `%NAME%` / `%~dp0` (cmd) of a launcher, from the variables it set before.
fn expand(text: &str, vars: &HashMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(['$', '%']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let (name, used) = if let Some(t) = tail.strip_prefix("${") {
            let end = t.find('}')?;
            (&t[..end], end + 3)
        } else if tail.starts_with("%~dp0") {
            ("~dp0", 5)
        } else if let Some(t) = tail.strip_prefix('%') {
            let end = t.find('%')?;
            (&t[..end], end + 2)
        } else {
            let t = &tail[1..];
            let end = t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(t.len());
            (&t[..end], end + 1)
        };
        out.push_str(vars.get(&name.to_ascii_lowercase())?);
        rest = &tail[used..];
    }
    out.push_str(rest);
    Some(out)
}

/// The first quoted (or bare) word of `text`.
fn first_word(text: &str) -> Option<&str> {
    let text = text.trim_start();
    match text.chars().next()? {
        q @ ('"' | '\'') => text[1..].split(q).next(),
        _ => text.split_whitespace().next(),
    }
}

/// The `soffice` a launcher script runs: the extension package's (`s="$here/../store/ext-core"`, then
/// `exec "$s/libreoffice/…/soffice" "$@"`; on Windows `set "S=%~dp0..\store\ext-core"`, then
/// `"%S%\libreoffice\program\soffice.com" %*`) or any script that `exec`s one.
fn launcher_target(launcher: &Path) -> Option<PathBuf> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(launcher).ok()?.take(16 * 1024).read_to_string(&mut text).ok()?;
    let dir = launcher.parent()?.to_string_lossy().into_owned();
    let mut vars: HashMap<String, String> = HashMap::from([("here".into(), dir.clone()), ("~dp0".into(), format!("{dir}{}", std::path::MAIN_SEPARATOR))]);
    for line in text.lines().map(str::trim) {
        let set = line.strip_prefix("set \"").and_then(|l| l.strip_suffix('"')).or_else(|| line.strip_prefix("set "));
        if let Some((name, value)) = set.and_then(|l| l.split_once('=')) {
            if let Some(v) = expand(value, &vars) {
                vars.insert(name.trim().to_ascii_lowercase(), v);
            }
            continue;
        }
        if let Some((name, value)) = line.split_once('=').filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
            if let Some(v) = first_word(value).filter(|v| !v.contains("$(")).and_then(|v| expand(v, &vars)) {
                vars.insert(name.to_ascii_lowercase(), v);
            }
            continue;
        }
        let command = line.strip_prefix("exec ").unwrap_or(line);
        let Some(target) = first_word(command).and_then(|w| expand(w, &vars)) else { continue };
        let target = PathBuf::from(target.replace(['\\', '/'], std::path::MAIN_SEPARATOR_STR));
        if target.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("soffice")) {
            return Some(lexical(&Path::new(&dir).join(target)));
        }
    }
    None
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The real `soffice` behind `path`: itself inside an install, else what its launcher runs (a few levels deep).
fn real_soffice(path: &Path) -> Option<(PathBuf, Layout)> {
    let mut at = path.to_path_buf();
    for _ in 0..4 {
        let real = std::fs::canonicalize(&at).ok()?;
        if let Some(layout) = layout_of(&real) {
            return Some((real, layout));
        }
        at = launcher_target(&real)?;
    }
    None
}

const APPLICATIONS: [(Office, &str, &str); 4] = [(Office::Writer, "sw", "writer.xcd"), (Office::Calc, "sc", "calc.xcd"), (Office::Impress, "sd", "impress.xcd"), (Office::Draw, "sd", "draw.xcd")];
const VISIO_TYPE: &str = "oor:name=\"draw_Visio_Document\"";
const VISIO_SERVICE: &str = "com.sun.star.comp.Draw.VisioImportFilter";

/// The library a service registry (`services.rdb`) names for an implementation.
fn implementation_library(rdb: &str, implementation: &str) -> Option<String> {
    let at = rdb.find(&format!("<implementation name=\"{implementation}\""))?;
    let component = &rdb[rdb[..at].rfind("<component ")?..at];
    let uri = component.split("uri=\"").nth(1)?.split('"').next()?;
    uri.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).map(String::from)
}

fn service_registries(layout: &Layout) -> Vec<PathBuf> {
    layout
        .services
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rdb") && p.is_file())
        .collect()
}

/// Draw registers the Visio type and filter (`draw.xcd`) and a service registry names the library that implements the filter, which is there.
fn visio_filter(layout: &Layout) -> bool {
    let registered = std::fs::read_to_string(layout.registry.join("draw.xcd")).is_ok_and(|x| x.contains(VISIO_TYPE) && x.contains(VISIO_SERVICE));
    registered
        && service_registries(layout).iter().any(|rdb| {
            std::fs::read_to_string(rdb).ok().and_then(|text| implementation_library(&text, VISIO_SERVICE)).is_some_and(|lib| layout.libs.iter().any(|d| d.join(&lib).is_file()))
        })
}

/// The applications and import filters of the install `soffice` starts; `None` when that install cannot be read.
pub fn office_components(soffice: &Path) -> Option<(Vec<Office>, Vec<Filter>)> {
    let (_, layout) = real_soffice(soffice)?;
    let apps: Vec<Office> = APPLICATIONS
        .iter()
        .filter(|(_, m, xcd)| layout.libs.iter().any(|d| d.join(module_library(m)).is_file()) && layout.registry.join(xcd).is_file())
        .map(|(a, _, _)| *a)
        .collect();
    let filters = if apps.contains(&Office::Draw) && visio_filter(&layout) { vec![Filter::Visio] } else { Vec::new() };
    Some((apps, filters))
}

/// What a change of the install behind `soffice` would show in: its real binary, launcher, module libraries, registry and service registries.
fn office_markers(soffice: &Path) -> Vec<PathBuf> {
    let Some((real, layout)) = real_soffice(soffice) else { return Vec::new() };
    let libs = APPLICATIONS.iter().map(|(_, m, _)| module_library(m)).flat_map(|lib| layout.libs.iter().map(move |d| d.join(&lib)).collect::<Vec<_>>());
    let registry = APPLICATIONS.iter().map(|(_, _, xcd)| layout.registry.join(xcd));
    std::iter::once(real).chain(libs).chain(registry).filter(|p| p.is_file()).chain(service_registries(&layout)).collect()
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
    let mut unresolved = Vec::new();
    let mut office = Vec::new();
    for path in find_all("soffice", dirs) {
        let Some(version) = soffice_version(&path) else { continue };
        match office_components(&path) {
            Some((components, filters)) if !components.is_empty() => office.push(Soffice { components, filters, tool: Tool::new(path, version) }),
            _ => unresolved.push(path),
        }
    }
    if office.is_empty() {
        missing.push("soffice");
    }
    let magick = tool("magick", dirs, &mut missing, |p| output_of(p, &["-version"]).and_then(|t| parse_magick_version(&t)));
    let pdftoppm = tool("pdftoppm", dirs, &mut missing, poppler_version("pdftoppm"));
    let pdfinfo = tool("pdfinfo", dirs, &mut missing, poppler_version("pdfinfo"));
    let pdf = pdftoppm.zip(pdfinfo).map(|(pdftoppm, pdfinfo)| PdfTools { pdftoppm, pdfinfo });
    let pandoc = tool("pandoc", dirs, &mut missing, |p| output_of(p, &["--version"]).and_then(|t| parse_pandoc_version(&t)));
    Probe { media, office, unresolved, magick, pdf, pandoc, missing, searched: dirs.to_vec() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_tool_wins_over_the_extension_package() {
        assert!(search_dirs().last().unwrap().ends_with("tools/bin"));
    }

    #[test]
    fn probes_run_outside_the_service_directory() {
        let (shell, args): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/C", "cd"]) } else { ("sh", &["-c", "pwd"]) };
        let reported = output_of(Path::new(shell), args).expect("shell runs");
        let reported = std::fs::canonicalize(reported.trim()).unwrap();
        let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(reported, temp);
        assert_ne!(reported, std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap());
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
        assert_eq!(why, "soffice is not available on this node (soffice is part of ext-core: this is a deployment defect)", "what a viewer is told names no paths");
        assert!(probe.missing_detail().unwrap().contains(&dir.path().display().to_string()), "the diagnostics do");
    }

    fn soffice(path: &str, components: &[Office], filters: &[Filter]) -> Soffice {
        Soffice { tool: Tool::new(path.into(), "25.2.3.2".into()), components: components.to_vec(), filters: filters.to_vec() }
    }

    const ALL_APPS: &[Office] = &[Office::Writer, Office::Calc, Office::Impress, Office::Draw];

    #[test]
    fn each_group_is_optional() {
        let probe = Probe { office: vec![soffice("soffice", ALL_APPS, &[Filter::Visio])], missing: vec!["ffmpeg", "ffprobe"], ..Default::default() };
        assert!(probe.available());
        assert_eq!(probe.profiles(), vec!["preview-document", "preview-sheet"]);
        assert!(matches!(probe.converter(Profile::PreviewDocument, Kind::Document("pptx")), Ok(Converter::Soffice(_))));
        assert!(matches!(probe.converter(Profile::PreviewDocument, Kind::Document("vsd")), Ok(Converter::Soffice(_))));
        assert!(probe.converter(Profile::Thumbnail, Kind::Video).is_err());
        assert!(matches!(probe.converter(Profile::Thumbnail, Kind::Document("odt")), Ok(Converter::Embedded(None))));
        assert!(matches!(probe.converter(Profile::Thumbnail, Kind::Document("vsdx")), Ok(Converter::Embedded(None))), "a thumbnail never starts soffice");
        let described = probe.describe();
        assert_eq!(described["soffice"][0]["version"], "25.2.3.2");
        assert_eq!(described["soffice"][0]["source"], "system");
        assert_eq!(described["soffice"][0]["components"], serde_json::json!(["writer", "calc", "impress", "draw"]));
        assert_eq!(described["soffice"][0]["filters"], serde_json::json!(["visio"]));
        assert!(described.get("soffice_missing_components").is_none() && described.get("soffice_missing_filters").is_none());
        assert!(described.get("ffmpeg").is_none());
        assert_eq!(described["missing"], serde_json::json!(["ffmpeg", "ffprobe"]));

        let tool = |name: &str| Tool::new(name.into(), "1".into());
        let probe = Probe { magick: Some(tool("magick")), pandoc: Some(tool("pandoc")), pdf: Some(PdfTools { pdftoppm: tool("pdftoppm"), pdfinfo: tool("pdfinfo") }), missing: vec!["ffmpeg", "ffprobe", "soffice"], ..Default::default() };
        assert_eq!(probe.profiles(), vec!["preview-image", "thumbnail", "preview-html"]);
        assert!(matches!(probe.converter(Profile::PreviewImage, Kind::Raster("dng")), Ok(Converter::Magick(_))));
        assert!(matches!(probe.converter(Profile::PreviewHtml, Kind::Markup("epub")), Ok(Converter::Pandoc(_))));
        assert!(matches!(probe.converter(Profile::Thumbnail, Kind::Pdf), Ok(Converter::Pdftoppm(_))));
        assert!(matches!(probe.converter(Profile::Thumbnail, Kind::Markup("epub")), Ok(Converter::Embedded(Some(Scaler::Magick(_))))));
        let why = probe.converter(Profile::PreviewImage, Kind::Image).unwrap_err();
        assert!(why.contains("ffmpeg and ffprobe not found") && !why.contains("searched"), "{why}");
    }

    #[test]
    fn a_document_goes_to_the_first_install_with_its_application() {
        let probe = Probe {
            office: vec![
                soffice("/usr/bin/soffice", &[Office::Writer, Office::Impress, Office::Draw], &[]),
                soffice("/opt/buckyos/tools/bin/soffice", ALL_APPS, &[Filter::Visio]),
            ],
            ..Default::default()
        };
        let path = |kind| match probe.converter(Profile::PreviewDocument, kind) {
            Ok(Converter::Soffice(t)) => t.path,
            other => panic!("{other:?}"),
        };
        let (system, ext) = (PathBuf::from("/usr/bin/soffice"), PathBuf::from("/opt/buckyos/tools/bin/soffice"));
        assert_eq!(path(Kind::Document("docx")), system);
        assert_eq!(path(Kind::Document("pptx")), system);
        assert_eq!(path(Kind::Document("odg")), system, "Draw without Visio still opens ODF drawings");
        assert_eq!(path(Kind::Document("pub")), system);
        assert_eq!(path(Kind::Document("xlsx")), ext);
        assert_eq!(path(Kind::Document("vsdx")), ext, "the Visio filter is only in the extension package's install");
        assert_eq!(path(Kind::Document("vsd")), ext);
        assert_eq!(probe.soffice_for_document("vsd").map(|t| t.path.clone()), Some(ext.clone()), "the cached PDF of a thumbnail is looked up under the same install");
        assert!(probe.missing_office().is_empty() && probe.missing_filters().is_empty());

        let probe = Probe { office: vec![soffice("/usr/bin/soffice", &[Office::Writer, Office::Impress], &[])], ..Default::default() };
        assert_eq!(probe.profiles(), vec!["preview-document"]);
        let why = probe.converter(Profile::PreviewSheet, Kind::Document("xls")).unwrap_err();
        assert_eq!(why, "the LibreOffice installs on this node have no Calc (soffice is part of ext-core: this is a deployment defect)");
        let why = probe.converter(Profile::PreviewDocument, Kind::Document("vsdx")).unwrap_err();
        assert!(why.starts_with("the LibreOffice installs on this node have no Draw ("), "Impress alone does not open drawings: {why}");
        let described = probe.describe();
        assert_eq!(described["soffice_missing_components"], serde_json::json!(["calc", "draw"]));
        assert_eq!(described["soffice_missing_filters"], serde_json::json!(["visio"]));

        let probe = Probe { office: vec![soffice("/usr/bin/soffice", ALL_APPS, &[])], ..Default::default() };
        let why = probe.converter(Profile::PreviewDocument, Kind::Document("vsd")).unwrap_err();
        assert!(why.starts_with("the LibreOffice installs on this node have no Draw with the Visio import filter ("), "{why}");
        assert!(matches!(probe.converter(Profile::PreviewDocument, Kind::Document("odg")), Ok(Converter::Soffice(_))));
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A Draw registry that registers the Visio type and filter, a service registry naming `lib` for the filter.
    fn visio_files(registry: &Path, services: &Path, lib: &str) {
        write(&registry.join("draw.xcd"), &format!("<oor:data><node oor:name=\"draw_Visio_Document\" oor:op=\"replace\"><prop oor:name=\"DetectService\"><value>{VISIO_SERVICE}</value></prop></node></oor:data>"));
        write(&services.join("services.rdb"), &format!("<components><component loader=\"com.sun.star.loader.SharedLibrary\" environment=\"gcc3\" uri=\"vnd.sun.star.expand:$LO_LIB_DIR/libsdlo.so\"><implementation name=\"com.sun.star.comp.Draw.DrawingDocument\"/></component><component loader=\"com.sun.star.loader.SharedLibrary\" environment=\"gcc3\" uri=\"vnd.sun.star.expand:$LO_LIB_DIR/{lib}\"><implementation name=\"{VISIO_SERVICE}\" constructor=\"x\"><service name=\"com.sun.star.document.ImportFilter\"/></implementation></component></components>"));
    }

    #[test]
    fn applications_and_filters_are_read_from_the_install() {
        let dir = tempfile::tempdir().unwrap();
        // Linux / Windows: program/ (soffice.bin, module libraries, services/), share/registry/.
        let root = dir.path().join("libreoffice");
        let program = root.join("program");
        let registry = root.join("share/registry");
        for f in ["soffice", &exe_name("soffice"), "soffice.bin", &module_library("sw"), &module_library("sd")] {
            write(&program.join(f), "");
        }
        assert_eq!(office_components(&program.join("soffice")), Some((vec![], vec![])), "libraries without their registry are no application");
        for x in ["writer.xcd", "impress.xcd"] {
            write(&registry.join(x), "");
        }
        assert_eq!(office_components(&program.join("soffice")), Some((vec![Office::Writer, Office::Impress], vec![])), "Impress and Draw share sd: the registry tells them apart");
        let before = tools_identity(&program);
        write(&registry.join("draw.xcd"), "<oor:data/>");
        assert_ne!(tools_identity(&program), before, "an application added to an install re-probes");
        assert_eq!(office_components(&program.join("soffice")), Some((vec![Office::Writer, Office::Impress, Office::Draw], vec![])), "Draw without the Visio registration");
        visio_files(&registry, &program.join("services"), "libwpftdrawlo.so");
        assert_eq!(office_components(&program.join("soffice")).unwrap().1, vec![], "the library the filter is in is missing");
        write(&program.join("libwpftdrawlo.so"), "");
        write(&program.join(module_library("sc")), "");
        write(&registry.join("calc.xcd"), "");
        assert_eq!(office_components(&program.join("soffice")), Some((vec![Office::Writer, Office::Calc, Office::Impress, Office::Draw], vec![Filter::Visio])));

        // macOS: Contents/MacOS/soffice, Contents/Frameworks, Contents/Resources/{registry,services}.
        let contents = dir.path().join("LibreOffice.app/Contents");
        write(&contents.join("MacOS/soffice"), "");
        write(&contents.join("Frameworks").join(module_library("sd")), "");
        write(&contents.join("Frameworks/libmergedlo.dylib"), "");
        write(&contents.join("Resources/registry/impress.xcd"), "");
        visio_files(&contents.join("Resources/registry"), &contents.join("Resources/services"), "libmergedlo.dylib");
        assert_eq!(office_components(&contents.join("MacOS/soffice")), Some((vec![Office::Impress, Office::Draw], vec![Filter::Visio])));

        // The extension package's launcher is followed to the install it runs; an unknown one is not taken as complete.
        let bin = dir.path().join("tools/bin");
        write(&dir.path().join("tools/store/ext-core/libreoffice"), "");
        std::fs::remove_file(dir.path().join("tools/store/ext-core/libreoffice")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, dir.path().join("tools/store/ext-core/libreoffice")).unwrap();
        write(&bin.join("soffice"), "#!/bin/sh\np=\"$0\"\nwhile [ -h \"$p\" ]; do l=\"$(readlink \"$p\")\"; done\nhere=\"$(cd \"${p%/*}\" && pwd -P)\"\ns=\"$here/../store/ext-core\"\n. \"$s/env/soffice.sh\"\nexec \"$s/libreoffice/program/soffice\" \"$@\"\n");
        #[cfg(unix)]
        assert_eq!(office_components(&bin.join("soffice")).map(|c| c.0.len()), Some(4), "ext-core's launcher");
        write(&bin.join("soffice.cmd"), "@echo off\r\nsetlocal\r\nset \"S=%~dp0..\\store\\ext-core\"\r\n\"%S%\\libreoffice\\program\\soffice.com\" %*\r\nexit /b %ERRORLEVEL%\r\n");
        let target = launcher_target(&bin.join("soffice.cmd")).unwrap();
        assert!(target.ends_with(Path::new("store/ext-core/libreoffice/program/soffice.com")) && target.starts_with(dir.path().join("tools")), "{}", target.display());
        #[cfg(windows)]
        {
            let ext_program = dir.path().join("tools/store/ext-core/libreoffice/program");
            for f in ["soffice.com", "soffice.bin"] {
                write(&ext_program.join(f), "");
            }
            assert_eq!(real_soffice(&bin.join("soffice.cmd")).map(|(real, _)| real), std::fs::canonicalize(ext_program.join("soffice.com")).ok(), "ext-core's Windows launcher");
        }
        write(&bin.join("other"), "#!/bin/sh\nexec \"$(dirname \"$0\")/real-office\" \"$@\"\n");
        assert_eq!(office_components(&bin.join("other")), None, "a launcher that names no soffice");
        write(&bin.join("plain"), "#!/bin/sh\n");
        assert_eq!(office_components(&bin.join("plain")), None);
        let bin_soffice = bin.join(if cfg!(windows) { "soffice.cmd" } else { "soffice" });
        assert_eq!(find_all("soffice", &[bin.clone(), program.clone(), bin.clone()]), vec![bin_soffice, program.join(exe_name("soffice"))]);
    }

    fn tools_identity(program: &Path) -> Identity {
        let found = installed(&[program.to_path_buf()]);
        identity(&found.iter().map(PathBuf::as_path).collect::<Vec<_>>())
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
