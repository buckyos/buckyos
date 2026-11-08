//! The catalog of a long document as a local directory (doc/llm_context/LLM
//! Understand Media.md §4.7, "Agent 侧目录"). The long-lived copy is the
//! workflow step cache in the Named Store; this directory unpacks it where the
//! Agent's own tools reach (`ls`, `grep`, `cat`, cropping a page image) and can
//! be deleted at any time: a path into it names the source object, so the
//! next call rebuilds it from the cache.
//!
//! ```text
//! <root>/<obj_id with ':' as '-'>/
//!   README.md       source, page count, where the text comes from, how to use
//!   index.tsv       page  status  text_source  chars  image_wh  first_line
//!   manifest.json   the merged MediaManifest and the manifest id of each window
//!   pages/0001.txt  the page's text as read (normalized; OCR where chosen)
//!   pages/0001.png  the page image
//!   .complete       what was written: manifest ids, text rule
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use buckyos_api::{MediaEntryStatus, MediaManifest};
use serde::{Deserialize, Serialize};

use super::pipeline::{
    image_dimensions, page_file_name, progress, read_object_bytes, PageText, TextSource, TEXT_RULE_VERSION,
};

pub const CATALOG_DIR_ENV: &str = "LLM_UNDERSTAND_MEDIA_CATALOG_DIR";
pub const CATALOG_MAX_BYTES_ENV: &str = "LLM_UNDERSTAND_MEDIA_CATALOG_MAX_BYTES";
const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const IMAGE_READ_CONCURRENCY: usize = 8;
/// A half-written catalog older than this is from a call that died.
const STALE_TMP: Duration = Duration::from_secs(3600);
const MANIFEST_FILE: &str = "manifest.json";
const COMPLETE_FILE: &str = ".complete";

/// Where catalogs go: the Agent's tool cache, else the system temp directory.
pub fn catalog_root() -> PathBuf {
    std::env::var(CATALOG_DIR_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("llm_understand_media-catalog"))
}

pub fn dir_name(obj_id: &str) -> String {
    obj_id.replacen(':', "-", 1)
}

/// The object a catalog directory is named after.
pub fn obj_id_of_dir(name: &str) -> Option<String> {
    let (kind, rest) = name.split_once('-')?;
    let id = format!("{kind}:{rest}");
    ndn_lib::ObjId::new(&id).ok().map(|_| id)
}

pub fn dir_of(root: &Path, obj_id: &str) -> PathBuf {
    root.join(dir_name(obj_id))
}

/// Where a catalog came from.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CatalogSource {
    pub obj_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub mime: String,
    /// `media.pdf` / `media.document`.
    pub family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// The path or URL the caller gave, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given_as: Option<String>,
}

/// `manifest.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CatalogFile {
    pub source: CatalogSource,
    pub manifest_ids: Vec<String>,
    pub manifest: MediaManifest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Complete {
    manifest_ids: Vec<String>,
    text_rule: u32,
    written_at: u64,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_complete(dir: &Path) -> Option<Complete> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(COMPLETE_FILE)).ok()?).ok()
}

/// When the catalog in `dir` was written; `None` when it is not complete.
pub fn written_at(dir: &Path) -> Option<u64> {
    read_complete(dir).map(|c| c.written_at)
}

pub fn load(dir: &Path) -> Option<CatalogFile> {
    read_complete(dir)?;
    serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?).ok()
}

/// The page texts a catalog holds, as written.
pub fn load_texts(dir: &Path, file: &CatalogFile) -> BTreeMap<u64, PageText> {
    let total = file.manifest.range.total.unwrap_or(0);
    let sources = index_sources(dir);
    let mut texts = BTreeMap::new();
    for entry in &file.manifest.entries {
        let Some(page) = entry.location.page.map(u64::from) else { continue };
        let Some(source) = sources.get(&page).copied() else { continue };
        if let Ok(text) = std::fs::read_to_string(dir.join("pages").join(page_file_name(page, total, "txt"))) {
            texts.insert(page, PageText { text, source });
        }
    }
    texts
}

fn index_sources(dir: &Path) -> BTreeMap<u64, TextSource> {
    let Ok(index) = std::fs::read_to_string(dir.join("index.tsv")) else {
        return BTreeMap::new();
    };
    index
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let page = cols.next()?.parse().ok()?;
            let source = match cols.nth(1)? {
                "layer" => TextSource::Layer,
                "ocr" => TextSource::Ocr,
                _ => return None,
            };
            Some((page, source))
        })
        .collect()
}

/// A path into a catalog.
#[derive(Clone, Debug, PartialEq)]
pub enum CatalogTarget {
    Dir { dir: PathBuf, obj_id: String },
    Page { dir: PathBuf, obj_id: String, page: u64 },
}

impl CatalogTarget {
    pub fn dir(&self) -> &Path {
        match self {
            CatalogTarget::Dir { dir, .. } | CatalogTarget::Page { dir, .. } => dir,
        }
    }

    pub fn obj_id(&self) -> &str {
        match self {
            CatalogTarget::Dir { obj_id, .. } | CatalogTarget::Page { obj_id, .. } => obj_id,
        }
    }
}

/// Whether `path` is a catalog directory or one of its page images. A path
/// that no longer exists counts when it is under the catalog root.
pub fn recognize(path: &Path, root: &Path) -> Option<CatalogTarget> {
    let path = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().ok()?.join(path) };
    let is_catalog = |dir: &Path| dir.join(MANIFEST_FILE).is_file() || dir.parent().is_some_and(|p| p == root);
    let named = |dir: &Path| dir.file_name().and_then(|n| obj_id_of_dir(&n.to_string_lossy()));
    if path.extension().is_some_and(|e| e == "png") {
        let pages = path.parent()?;
        if pages.file_name()? != "pages" {
            return None;
        }
        let dir = pages.parent()?;
        let obj_id = named(dir)?;
        let page = path.file_stem()?.to_str()?.parse::<u64>().ok().filter(|p| *p > 0)?;
        return is_catalog(dir).then(|| CatalogTarget::Page { dir: dir.to_path_buf(), obj_id, page });
    }
    if path.is_file() {
        return None;
    }
    let dir = path.as_path();
    let obj_id = named(dir)?;
    is_catalog(dir).then(|| CatalogTarget::Dir { dir: dir.to_path_buf(), obj_id })
}

/// A catalog page image the Agent changed after it was written (cropped in
/// place, say) is just an image.
pub fn page_image_changed(dir: &Path, image: &Path) -> bool {
    let (Some(written), Ok(meta)) = (written_at(dir), std::fs::metadata(image)) else {
        return false;
    };
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .is_some_and(|m| m.as_secs() > written + 1)
}

/// One row of `index.tsv`.
fn index_row(page: u64, status: &str, text: Option<&PageText>, image_wh: Option<(u32, u32)>) -> String {
    let first = text
        .and_then(|t| t.text.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("")
        .replace('\t', " ")
        .chars()
        .take(80)
        .collect::<String>();
    format!(
        "{page}\t{status}\t{}\t{}\t{}\t{first}\n",
        text.map(|t| t.source.label()).unwrap_or("-"),
        text.map(|t| t.text.chars().count()).unwrap_or(0),
        image_wh.map(|(w, h)| format!("{w}x{h}")).unwrap_or_else(|| "-".to_string()),
    )
}

fn status_label(status: MediaEntryStatus) -> &'static str {
    match status {
        MediaEntryStatus::Ok => "ok",
        MediaEntryStatus::NoText => "no_text",
        MediaEntryStatus::Absent => "absent",
        MediaEntryStatus::Unprocessed => "unprocessed",
        MediaEntryStatus::Failed => "failed",
    }
}

fn readme(dir: &Path, file: &CatalogFile, texts: &BTreeMap<u64, PageText>) -> String {
    let manifest = &file.manifest;
    let total = manifest.range.total.unwrap_or(manifest.entries.len() as u64);
    let layer = texts.values().filter(|t| t.source == TextSource::Layer).count();
    let ocr = texts.values().filter(|t| t.source == TextSource::Ocr).count();
    let none = total as usize - (layer + ocr).min(total as usize);
    let name = file.source.name.as_deref().unwrap_or("the document");
    let lang = manifest
        .ocr
        .as_ref()
        .and_then(|o| o.lang.as_deref())
        .map(|l| format!(" ({l})"))
        .unwrap_or_default();
    let unprocessed = manifest.entries.iter().filter(|e| e.status == MediaEntryStatus::Unprocessed).count();
    let failed = manifest.range.failed.len();
    let example = |ext| page_file_name(1, total, ext);
    let d = dir.display();
    format!(
        "# Catalog of {name}\n\n\
         Source: {}object `{}` ({}).\n\
         Pages: {total}{}. Text from the PDF text layer on {layer} pages, from OCR{lang} on {ocr} pages, none on {none}.\n\n\
         This directory is a cache of the conversion, which the system keeps: it may be deleted at any time, and \
         `llm_understand_media` on any path in it rebuilds it without converting again.\n\n\
         ## Files\n\n\
         - `index.tsv`: one row a page: page, status, text_source (`layer` | `ocr` | `-`), chars, image_wh, first_line.\n\
         - `pages/{}`: the text of the page as the tool reads it (layout whitespace removed; OCR where the text layer is missing or does not match the page).\n\
         - `pages/{}`: the page image; its long edge is at most 2048 pixels, so cropping shows what is there but adds no detail.\n\
         - `manifest.json`: the media manifest of the conversion.\n\n\
         ## Use\n\n\
         ```sh\n\
         grep -il <word> {d}/pages/*.txt                       # which pages mention it\n\
         llm_understand_media {d}/pages/{} --goal \"...\"   # one page: its image and its text\n\
         llm_understand_media {d} --pages 14,50,70 --goal \"...\"  # several pages together\n\
         ```\n\n\
         To look at part of a page closely, crop the page image (ImageMagick `magick in.png -crop WxH+X+Y out.png`) \
         and pass the crop as an ordinary image.\n",
        file.source.given_as.as_deref().map(|g| format!("`{g}`, ")).unwrap_or_default(),
        file.source.obj_id,
        file.source.mime,
        if unprocessed + failed > 0 { format!(" ({unprocessed} not converted, {failed} failed)") } else { " (all converted)".to_string() },
        example("txt"),
        example("png"),
        example("png"),
    )
}

/// Writes `pages/*.txt`, `index.tsv` and `README.md`; images come from `images`
/// (page -> dimensions of the file written).
fn write_texts(dir: &Path, file: &CatalogFile, texts: &BTreeMap<u64, PageText>, images: &BTreeMap<u64, (u32, u32)>) -> Result<(), String> {
    let total = file.manifest.range.total.unwrap_or(0);
    let pages_dir = dir.join("pages");
    std::fs::create_dir_all(&pages_dir).map_err(|e| format!("create {}: {e}", pages_dir.display()))?;
    let mut index = String::from("page\tstatus\ttext_source\tchars\timage_wh\tfirst_line\n");
    for entry in &file.manifest.entries {
        let Some(page) = entry.location.page.map(u64::from) else { continue };
        let txt = pages_dir.join(page_file_name(page, total, "txt"));
        match texts.get(&page) {
            Some(text) => std::fs::write(&txt, &text.text).map_err(|e| format!("write {}: {e}", txt.display()))?,
            None => {
                let _ = std::fs::remove_file(&txt);
            }
        }
        index.push_str(&index_row(page, status_label(entry.status), texts.get(&page), images.get(&page).copied()));
    }
    std::fs::write(dir.join("index.tsv"), index).map_err(|e| format!("write index.tsv: {e}"))?;
    std::fs::write(dir.join("README.md"), readme(dir, file, texts)).map_err(|e| format!("write README.md: {e}"))?;
    Ok(())
}

fn png_dims_on_disk(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut head = [0u8; 32];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut head).ok()?;
    image_dimensions(&head[..n])
}

async fn write_images(dir: &Path, manifest: &MediaManifest) -> Result<BTreeMap<u64, (u32, u32)>, String> {
    let total = manifest.range.total.unwrap_or(0);
    let pages_dir = dir.join("pages");
    std::fs::create_dir_all(&pages_dir).map_err(|e| format!("create {}: {e}", pages_dir.display()))?;
    let jobs: Vec<(u64, String)> = manifest
        .entries
        .iter()
        .filter_map(|e| Some((u64::from(e.location.page?), e.image_obj_id.clone()?)))
        .collect();
    let mut dims = BTreeMap::new();
    for batch in jobs.chunks(IMAGE_READ_CONCURRENCY) {
        let mut reads = tokio::task::JoinSet::new();
        for (page, id) in batch.iter().cloned() {
            reads.spawn(async move { (page, read_object_bytes(&id).await) });
        }
        while let Some(read) = reads.join_next().await {
            let (page, bytes) = read.map_err(|e| format!("page image read aborted: {e}"))?;
            let bytes = bytes.map_err(|e| format!("read page {page} image: {e}"))?;
            let path = pages_dir.join(page_file_name(page, total, "png"));
            tokio::fs::write(&path, &bytes).await.map_err(|e| format!("write {}: {e}", path.display()))?;
            if let Some(wh) = image_dimensions(&bytes) {
                dims.insert(page, wh);
            }
        }
    }
    Ok(dims)
}

fn write_complete(dir: &Path, manifest_ids: &[String]) -> Result<(), String> {
    let complete = Complete { manifest_ids: manifest_ids.to_vec(), text_rule: TEXT_RULE_VERSION, written_at: now_secs() };
    std::fs::write(dir.join(COMPLETE_FILE), serde_json::to_string(&complete).unwrap_or_default())
        .map_err(|e| format!("write {COMPLETE_FILE}: {e}"))
}

fn touch(path: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Unpacks a catalog into `<root>/<obj_id>`. The same conversion already there
/// is only touched; a newer text rule rewrites only the texts; anything else
/// is written beside it and renamed into place.
pub async fn write(root: &Path, file: &CatalogFile, texts: &BTreeMap<u64, PageText>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("create {}: {e}", root.display()))?;
    let dir = dir_of(root, &file.source.obj_id);
    let total = file.manifest.range.total.unwrap_or(0);
    if let Some(done) = read_complete(&dir) {
        if done.manifest_ids == file.manifest_ids {
            if done.text_rule != TEXT_RULE_VERSION {
                let dims: BTreeMap<u64, (u32, u32)> = file
                    .manifest
                    .entries
                    .iter()
                    .filter_map(|e| {
                        let page = u64::from(e.location.page?);
                        Some((page, png_dims_on_disk(&dir.join("pages").join(page_file_name(page, total, "png")))?))
                    })
                    .collect();
                write_texts(&dir, file, texts, &dims)?;
                write_complete(&dir, &file.manifest_ids)?;
            } else {
                touch(&dir.join(COMPLETE_FILE));
            }
            return Ok(dir);
        }
    }
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let tmp = root.join(format!(".tmp-{}-{}-{nanos}", dir_name(&file.source.obj_id), std::process::id()));
    let result = async {
        std::fs::create_dir_all(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        std::fs::write(tmp.join(MANIFEST_FILE), serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?)
            .map_err(|e| format!("write {MANIFEST_FILE}: {e}"))?;
        let dims = write_images(&tmp, &file.manifest).await?;
        write_texts(&tmp, file, texts, &dims)?;
        write_complete(&tmp, &file.manifest_ids)?;
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("replace {}: {e}", dir.display()))?;
        }
        std::fs::rename(&tmp, &dir).map_err(|e| format!("move {} into place: {e}", tmp.display()))
    }
    .await;
    if let Err(err) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(err);
    }
    // README and the usage lines name the final directory.
    let _ = std::fs::write(dir.join("README.md"), readme(&dir, file, texts));
    clean(root, &dir, max_bytes());
    Ok(dir)
}

fn max_bytes() -> u64 {
    std::env::var(CATALOG_MAX_BYTES_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_BYTES)
}

fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Keeps the catalogs under `max` bytes by deleting the least recently used
/// (by `.complete` mtime), never `keep`; leftovers of dead calls go too.
pub fn clean(root: &Path, keep: &Path, max: u64) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mut catalogs: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_dir() {
            continue;
        }
        if name.starts_with(".tmp-") {
            let old = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|m| m.elapsed().ok());
            if old.is_some_and(|age| age > STALE_TMP) {
                let _ = std::fs::remove_dir_all(&path);
            }
            continue;
        }
        if obj_id_of_dir(&name).is_none() {
            continue;
        }
        let used = std::fs::metadata(path.join(COMPLETE_FILE)).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        catalogs.push((used, dir_bytes(&path), path));
    }
    let mut total: u64 = catalogs.iter().map(|c| c.1).sum();
    catalogs.sort_by_key(|c| c.0);
    for (_, bytes, path) in catalogs {
        if total <= max {
            break;
        }
        if path == keep {
            continue;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            progress(format!("removed the catalog {} (catalogs above {max} bytes)", path.display()));
            total = total.saturating_sub(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{MediaEntry, MediaEntryKind, MediaSourceRef, MediaWhere};

    const OBJ: &str = "cyfile:25b7a541c0ffee25b7a541c0ffee25b7a541c0ffee25b7a541c0ffee0011";

    fn file(pages: u32) -> CatalogFile {
        let mut manifest = MediaManifest::new("pdf", "sha256:x");
        manifest.range.total = Some(u64::from(pages));
        manifest.range.requested = (1..=u64::from(pages)).collect();
        manifest.entries = (1..=pages)
            .map(|p| MediaEntry {
                kind: MediaEntryKind::Page,
                location: MediaWhere { source: MediaSourceRef::Original, page: Some(p), time_ms: None, index: None },
                text_obj_id: Some(format!("cyfile:t{p}")),
                image_obj_id: None,
                audio_obj_id: None,
                ocr_text_obj_id: None,
                text_quality: None,
                status: MediaEntryStatus::Ok,
                error: None,
            })
            .collect();
        manifest.recompute_coverage();
        CatalogFile {
            source: CatalogSource { obj_id: OBJ.into(), name: Some("dk.pdf".into()), mime: "application/pdf".into(), family: "media.pdf".into(), format: None, given_as: Some("/home/devtest/dk.pdf".into()) },
            manifest_ids: vec!["mmanifest:a".into()],
            manifest,
        }
    }

    fn texts(pages: u64) -> BTreeMap<u64, PageText> {
        (1..=pages).map(|p| (p, PageText { text: format!("page {p} the Colosseum\nmore"), source: if p == 2 { TextSource::Ocr } else { TextSource::Layer } })).collect()
    }

    #[tokio::test]
    async fn a_catalog_is_written_reused_and_recognized() {
        let root = tempfile::tempdir().unwrap();
        let f = file(3);
        let dir = write(root.path(), &f, &texts(3)).await.unwrap();
        assert_eq!(dir, root.path().join(dir_name(OBJ)));
        assert_eq!(obj_id_of_dir(&dir_name(OBJ)).as_deref(), Some(OBJ));
        let index = std::fs::read_to_string(dir.join("index.tsv")).unwrap();
        assert!(index.starts_with("page\tstatus\ttext_source\tchars\timage_wh\tfirst_line\n"));
        assert!(index.contains("2\tok\tocr\t"), "{index}");
        assert!(std::fs::read_to_string(dir.join("pages/0002.txt")).unwrap().contains("Colosseum"));
        let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
        assert!(readme.contains("from OCR on 1 pages") && readme.contains(&format!("{}/pages/0001.png", dir.display())), "{readme}");
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded, f);
        assert_eq!(load_texts(&dir, &loaded), texts(3));

        // Same conversion: kept. A new text rule would rewrite only texts; a new conversion replaces it.
        let before = written_at(&dir).unwrap();
        write(root.path(), &f, &texts(3)).await.unwrap();
        assert_eq!(written_at(&dir), Some(before));
        // An older text rule: only the texts are written again.
        let old = Complete { manifest_ids: f.manifest_ids.clone(), text_rule: TEXT_RULE_VERSION - 1, written_at: 1 };
        std::fs::write(dir.join(COMPLETE_FILE), serde_json::to_string(&old).unwrap()).unwrap();
        std::fs::write(dir.join("pages/0001.txt"), "stale").unwrap();
        write(root.path(), &f, &texts(3)).await.unwrap();
        assert!(std::fs::read_to_string(dir.join("pages/0001.txt")).unwrap().contains("Colosseum"));
        assert!(written_at(&dir).unwrap() > 1);
        let mut g = f.clone();
        g.manifest_ids = vec!["mmanifest:b".into()];
        write(root.path(), &g, &texts(2)).await.unwrap();
        assert_eq!(load(&dir).unwrap().manifest_ids, vec!["mmanifest:b".to_string()]);
        assert!(!dir.join("pages/0003.txt").exists());

        assert_eq!(recognize(&dir, root.path()), Some(CatalogTarget::Dir { dir: dir.clone(), obj_id: OBJ.into() }));
        let page = dir.join("pages/0101.png");
        assert_eq!(recognize(&page, root.path()), Some(CatalogTarget::Page { dir: dir.clone(), obj_id: OBJ.into(), page: 101 }));
        std::fs::remove_dir_all(&dir).unwrap();
        // Deleted: the path still names the object.
        assert_eq!(recognize(&page, root.path()).map(|t| t.obj_id().to_string()), Some(OBJ.to_string()));
        let elsewhere = tempfile::tempdir().unwrap();
        let lookalike = elsewhere.path().join(dir_name(OBJ)).join("pages/0001.png");
        assert_eq!(recognize(&lookalike, root.path()), None);
        assert_eq!(recognize(Path::new("/tmp/report.png"), root.path()), None);
    }

    #[tokio::test]
    async fn the_least_recently_used_catalogs_go_above_the_limit() {
        let root = tempfile::tempdir().unwrap();
        let mut dirs = Vec::new();
        for i in 0..3 {
            let mut f = file(2);
            f.source.obj_id = format!("cyfile:{i:064x}");
            dirs.push(write(root.path(), &f, &texts(2)).await.unwrap());
            std::thread::sleep(Duration::from_millis(20));
        }
        touch(&dirs[0].join(COMPLETE_FILE));
        let stale = root.path().join(".tmp-x-1-2");
        std::fs::create_dir_all(&stale).unwrap();
        let one = dir_bytes(&dirs[0]);
        clean(root.path(), &dirs[2], one * 2);
        assert!(dirs[0].exists(), "used last");
        assert!(!dirs[1].exists(), "least recently used");
        assert!(dirs[2].exists(), "just written");
        assert!(stale.exists(), "a fresh temp dir may belong to a call at work");
    }
}
