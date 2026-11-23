//! The print package (打印 TODO §3.2, §3.4, §4.2, T5, T7–T9; contract `print/package.ts`): the rows one print of a
//! canvas needs, taken at one head and cut to what the caller may read and export, zipped with the Desktop's print
//! renderer and the assets the pages show, for Workflow's `media.page`.

use crate::docdb::SqlCtx;
use crate::export::TempFile;
use crate::objects::ObjectStore;
use crate::session::query_rows;
use crate::workspace::{random_id, Caller, Workspace};
use aiworkspace_core::access::{Access, Cap};
use aiworkspace_core::canonical::obj_id_to_filename;
use aiworkspace_core::materialize::{without_notes, ObjectSource};
use aiworkspace_core::model::*;
use aiworkspace_core::value::{reference_entity_id, reference_is_local};
use aiworkspace_core::{richtext, Code, WsError, WsResult};
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

pub const PRINT_FORMAT: &str = "buckyos.aiws-print";
pub const PRINT_FORMAT_VERSION: u32 = 1;
/// The print renderer's folder in the Desktop build.
pub const RENDERER_DIR: &str = "aiws-print";
/// The FileObject name of every print package.
const PACKAGE_NAME: &str = "aiws-print.zip";
const PAPER_NAMES: &[&str] = &["A3", "A4", "A5", "Letter", "16:9", "4:3", "custom"];
const MAX_PAPER_MM: f64 = 2000.0;
/// `media.page` unpacks at most 512 MiB and 10 000 files: the assets leave room for the renderer and the data.
const MAX_ASSET_BYTES: u64 = 384 << 20;
const MAX_ASSET_FILES: usize = 5000;
const MAX_RENDERER_FILES: usize = 1000;
const MAX_RENDERER_BYTES: u64 = 96 << 20;
/// References whose targets a printed page shows: the data a Block binds or created, what a rich text embeds, object
/// references, record bodies, Block definitions and what they build on, child directories.
const SHOWN: &[&str] = &["bind", "owns", "embed", "value", "value_link", "body", "def", "def_dep", "member"];

fn io(e: std::io::Error) -> WsError {
    let code = if e.raw_os_error() == Some(28) { Code::StorageFull } else { Code::StorageIoError };
    WsError::new(code, e.to_string())
}

fn path_unusable(reason: &str, detail: &str) -> WsError {
    WsError::sub(Code::InvalidOperation, "PRINT_PATH_UNUSABLE", detail).with_data(json!({ "reason": reason }))
}

/// `print.package`'s parameters (`PrintPackageParams`), checked.
#[derive(Debug, Clone)]
pub struct PrintRequest {
    pub surface_id: String,
    pub path_id: Option<String>,
    /// `{ name, width_mm, height_mm }`, as printed.
    pub paper: Value,
    pub frame_id: Option<String>,
    pub date: String,
    pub locale: String,
}

impl PrintRequest {
    pub fn parse(p: &Value) -> WsResult<PrintRequest> {
        let text = |k: &str, max: usize| -> WsResult<String> {
            p.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.chars().count() <= max)
                .map(str::to_string)
                .ok_or_else(|| WsError::invalid_op(format!("{k}: a string of 1…{max} characters")))
        };
        let id = |k: &str| -> WsResult<Option<String>> {
            match p.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
                Some(_) => Err(WsError::invalid_op(format!("{k} must be an entity id"))),
            }
        };
        let paper = p.get("paper").and_then(Value::as_object).ok_or_else(|| WsError::invalid_op("paper: { name, width_mm, height_mm }"))?;
        let name = paper
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| PAPER_NAMES.contains(n))
            .ok_or_else(|| WsError::invalid_op(format!("paper.name: one of {}", PAPER_NAMES.join(", "))))?;
        let mm = |k: &str| -> WsResult<f64> {
            paper
                .get(k)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_PAPER_MM)
                .ok_or_else(|| WsError::invalid_op(format!("paper.{k}: millimetres in (0, {MAX_PAPER_MM}]")))
        };
        Ok(PrintRequest {
            surface_id: text("surface_id", 256)?,
            path_id: id("path_id")?,
            paper: json!({ "name": name, "width_mm": mm("width_mm")?, "height_mm": mm("height_mm")? }),
            frame_id: id("frame_id")?,
            date: text("date", 64)?,
            locale: text("locale", 64)?,
        })
    }
}

/// The renderer files of a Desktop build's `aiws-print/` by their path in the package (the package's own `data.js`
/// and `assets/` win over anything of that name there).
pub fn renderer_files(dir: Option<&Path>) -> WsResult<Vec<(String, Vec<u8>)>> {
    let missing = || {
        WsError::sub(Code::DependencyUnavailable, "PRINT_RENDERER_MISSING", format!("the print renderer (the Desktop build's {RENDERER_DIR}/) is not installed with this service"))
    };
    let dir = dir.filter(|d| d.join("index.html").is_file()).ok_or_else(missing)?;
    let (mut out, mut total) = (Vec::new(), 0u64);
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        for entry in std::fs::read_dir(&folder).map_err(io)? {
            let entry = entry.map_err(io)?;
            let kind = entry.file_type().map_err(io)?;
            let rel = match (prefix.is_empty(), entry.file_name().to_string_lossy()) {
                (true, n) => n.into_owned(),
                (false, n) => format!("{prefix}/{n}"),
            };
            if kind.is_dir() {
                stack.push((entry.path(), rel));
            } else if kind.is_file() && rel != "data.js" && !rel.starts_with("assets/") {
                let bytes = std::fs::read(entry.path()).map_err(io)?;
                total += bytes.len() as u64;
                if out.len() >= MAX_RENDERER_FILES || total > MAX_RENDERER_BYTES {
                    return Err(WsError::limit(format!("the print renderer exceeds {MAX_RENDERER_FILES} files or {MAX_RENDERER_BYTES} bytes")));
                }
                out.push((rel, bytes));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// One print before it is zipped: the package (`window.__AIWS_PRINT__`) and the assets it brings, `(object id, path)`,
/// and where it goes (the Workspace owner's `exports` bucket).
pub struct PrintContents {
    pub package: Value,
    pub assets: Vec<(String, String)>,
    pub workspace_id: String,
    pub exports: String,
}

impl PrintContents {
    /// Zip the package with `renderer` at its root and bind it under `<workspace_id>/print/<id>.zip` in `exports`
    /// (nothing is bound when anything fails). Needs no Workspace writer: the rows are fixed and the assets are content.
    pub fn store(&self, objects: &ObjectStore, renderer: &[(String, Vec<u8>)]) -> WsResult<PrintBuilt> {
        let print_id = random_id("pr_");
        let tmp = TempFile::new(&format!("{print_id}.zip"))?;
        let file = std::fs::File::create(&tmp.0).map_err(io)?;
        let mut zip = zip::ZipWriter::new(file);
        // the same head and job give the same bytes: `media.page` answers a reprint from its cache (keyed on content)
        let fixed = zip::write::SimpleFileOptions::default().last_modified_time(zip::DateTime::default());
        let deflated = fixed.compression_method(zip::CompressionMethod::Deflated);
        let stored = fixed.compression_method(zip::CompressionMethod::Stored);
        let zerr = |e: zip::result::ZipError| WsError::io(format!("print package: {e}"));
        for (name, bytes) in renderer {
            zip.start_file(name.as_str(), deflated).map_err(zerr)?;
            zip.write_all(bytes).map_err(io)?;
        }
        let data = serde_json::to_string(&self.package).map_err(|e| WsError::io(e.to_string()))?;
        zip.start_file("data.js", deflated).map_err(zerr)?;
        zip.write_all(format!("window.__AIWS_PRINT__ = {data};\n").as_bytes()).map_err(io)?;
        for (obj, path) in &self.assets {
            zip.start_file(path.as_str(), stored).map_err(zerr)?;
            zip.write_all(&objects.get_file(obj)?).map_err(io)?;
        }
        let file = zip.finish().map_err(zerr)?;
        file.sync_all().map_err(io)?;
        let size = file.metadata().map_err(io)?.len();
        drop(file);
        let key = format!("{}/print/{print_id}.zip", self.workspace_id);
        let obj_id = objects.store_file(&tmp.0, PACKAGE_NAME)?;
        objects.bind(&self.exports, &key, &obj_id)?;
        Ok(PrintBuilt { obj_id, size, key, job: self.package["job"].clone(), issues: self.package["issues"].clone() })
    }
}

/// A built print package: the FileObject in the owner's `exports` bucket.
pub struct PrintBuilt {
    pub obj_id: String,
    pub size: u64,
    pub key: String,
    pub job: Value,
    pub issues: Value,
}

fn issue(code: &str, fields: &[(&str, &str)]) -> Value {
    let mut v = Map::new();
    v.insert("code".into(), json!(code));
    for (k, val) in fields {
        v.insert(k.to_string(), json!(val));
    }
    Value::Object(v)
}

fn container_kind(e: &EntityRow) -> &str {
    if e.type_id == TYPE_CONTAINER {
        e.payload.get("kind").and_then(Value::as_str).unwrap_or("")
    } else {
        ""
    }
}

fn is_free(surface: &EntityRow) -> bool {
    surface.payload.get("layout").and_then(|l| l["mode"].as_str()) == Some("free")
}

/// A comment or a highlight (Q7: not printed): an anchored annotation, and a Block showing one. A sticky note is an
/// annotation without a target and is canvas content.
fn is_comment(ctx: &SqlCtx, e: &EntityRow) -> WsResult<bool> {
    let anchored = |a: &EntityRow| a.type_id == TYPE_ANNOTATION && a.payload.get("target").is_some_and(|t| !t.is_null());
    Ok(match e.type_id.as_str() {
        TYPE_ANNOTATION => anchored(e),
        TYPE_CELL => match e.payload.get("source_ref").filter(|r| reference_is_local(r)).and_then(reference_entity_id) {
            Some(id) => ctx.entity(id)?.is_some_and(|a| anchored(&a)),
            None => false,
        },
        _ => false,
    })
}

/// The shared, alive entity `id` when `access` may read it.
fn readable(ctx: &SqlCtx, access: &Access, id: &str) -> WsResult<Option<EntityRow>> {
    match ctx.entity(id)? {
        Some(e) if e.alive() && e.scope == SCOPE_SHARED && access.can_read(ctx, &e)? => Ok(Some(e)),
        _ => Ok(None),
    }
}

/// Where a step of a presentation path is, as a show resolves it (`presentation/model.ts` `resolveStep`, `playable`):
/// its target and the free Surface it lies on, or why it is not played.
fn resolve_step(ctx: &SqlCtx, step: &Value) -> WsResult<Result<(EntityRow, EntityRow), &'static str>> {
    if step.get("enabled") == Some(&json!(false)) {
        return Ok(Err("disabled"));
    }
    let kind = step["target"]["kind"].as_str().unwrap_or("");
    let Some(target) = ctx.entity(step["target"]["entity_id"].as_str().unwrap_or(""))?.filter(EntityRow::alive) else { return Ok(Err("missing")) };
    let surface_id = match (kind, target.type_id.as_str()) {
        ("viewport", TYPE_VIEWPORT) => match target.payload.get("surface_ref").and_then(reference_entity_id) {
            Some(s) => s.to_string(),
            None => return Ok(Err("missing")),
        },
        ("frame", TYPE_CELL) => {
            // a Frame laid out by a control has no position of its own; groups shift it
            let Some(edge) = ctx.edge(&target.entity_id)? else { return Ok(Err("surface")) };
            let placed = edge.placement.as_ref().is_some_and(|p| ["x", "y", "w", "h"].iter().all(|k| p[*k].as_f64().is_some_and(f64::is_finite)));
            if !placed {
                return Ok(Err("surface"));
            }
            let mut parent = edge.parent_id;
            let mut found = None;
            for _ in 0..64 {
                let Some(p) = ctx.entity(&parent)? else { break };
                match container_kind(&p) {
                    "surface" => {
                        found = Some(parent);
                        break;
                    }
                    "group" => match ctx.edge(&parent)? {
                        Some(e) => parent = e.parent_id,
                        None => break,
                    },
                    _ => break,
                }
            }
            match found {
                Some(s) => s,
                None => return Ok(Err("surface")),
            }
        }
        _ => return Ok(Err("missing")),
    };
    let Some(surface) = ctx.entity(&surface_id)?.filter(|s| s.alive() && container_kind(s) == "surface") else { return Ok(Err("surface")) };
    if !is_free(&surface) {
        return Ok(Err("flow"));
    }
    Ok(Ok((target, surface)))
}

/// What the package holds: entities walked (subtrees of what is printed, and what they show), each counted once when
/// it may not be read.
struct Scope<'a> {
    ctx: &'a SqlCtx<'a>,
    access: &'a Access,
    content: BTreeSet<String>,
    expanded: BTreeSet<String>,
    denied: BTreeSet<String>,
    /// Unreadable entities by where they were met (`surface_id` or `entity_id`, the id).
    unreadable: BTreeMap<(&'static str, String), u64>,
}

impl<'a> Scope<'a> {
    fn walk(&mut self, root: &str, subtree: bool, at: (&'static str, &str)) -> WsResult<()> {
        let mut queue = vec![(root.to_string(), subtree)];
        while let Some((id, subtree)) = queue.pop() {
            if !self.content.contains(&id) {
                let Some(e) = self.ctx.entity(&id)?.filter(EntityRow::alive) else { continue };
                if e.scope != SCOPE_SHARED || is_comment(self.ctx, &e)? {
                    continue;
                }
                if !self.access.can_read(self.ctx, &e)? {
                    if self.denied.insert(id.clone()) {
                        *self.unreadable.entry((at.0, at.1.to_string())).or_default() += 1;
                    }
                    continue;
                }
                self.content.insert(id.clone());
                for r in self.ctx.refs_from(&id)? {
                    if SHOWN.contains(&r.kind.as_str()) && r.dst_workspace_id.is_empty() && !r.dst_entity_id.is_empty() {
                        queue.push((r.dst_entity_id.clone(), false));
                    }
                }
            }
            if subtree && self.expanded.insert(id.clone()) {
                for edge in self.ctx.children(&id)? {
                    queue.push((edge.child_id, true));
                }
            }
        }
        Ok(())
    }
}

/// The extension an asset's file needs in the package: Chromium types `file://` by the extension (an SVG is never sniffed).
fn extension(media_type: &str) -> Option<&'static str> {
    Some(match media_type {
        "image/svg+xml" => "svg",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        "application/pdf" => "pdf",
        _ => return None,
    })
}

/// readFontSettings (`print/fonts.ts`): the three generic families the Workspace maps, non-empty names only.
fn font_settings(config: &Value) -> Value {
    let mut out = Map::new();
    for g in ["sans", "serif", "mono"] {
        if let Some(f) = config["fonts"][g].as_str().map(str::trim).filter(|f| !f.is_empty() && f.chars().count() <= 128) {
            out.insert(g.into(), json!(f));
        }
    }
    Value::Object(out)
}

/// A table's deleted fields leave values behind in its records: they never travel.
fn strip_fields(row: &mut Value, dead: &[String]) {
    for col in ["values_json", "revs_json", "meta_json"] {
        if let Some(Ok(Value::Object(mut m))) = row[col].as_str().map(serde_json::from_str::<Value>) {
            if dead.iter().any(|f| m.remove(f).is_some()) {
                row[col] = json!(Value::Object(m).to_string());
            }
        }
    }
}

impl Workspace {
    /// What one print of `req.surface_id` holds (README “Printing”), at the current head: nothing is written.
    pub fn print_contents(&self, caller: &Caller, req: &PrintRequest) -> WsResult<PrintContents> {
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let surface = readable(&ctx, &access, &req.surface_id)?.ok_or_else(|| WsError::not_found(format!("{} not found", req.surface_id)))?;
        if container_kind(&surface) != "surface" {
            return Err(WsError::invalid_op(format!("{} is not a canvas", req.surface_id)));
        }
        if !access.can(&ctx, &surface.entity_id, Cap::Export)? {
            return Err(WsError::denied("export capability required on the canvas"));
        }
        let mode = if is_free(&surface) { "free" } else { "flow" };
        let mut issues: Vec<Value> = Vec::new();
        // the Surfaces printed, in path order, and the Viewports of the steps played
        let mut surfaces = vec![surface.entity_id.clone()];
        let mut viewports = Vec::new();
        match (mode, req.path_id.as_deref()) {
            ("flow", Some(_)) => return Err(WsError::invalid_op("a flow canvas prints without a presentation path")),
            ("flow", None) => {}
            (_, None) => return Err(WsError::sub(Code::InvalidOperation, "PRINT_PATH_REQUIRED", "a free canvas prints along one of its presentation paths")),
            (_, Some(path_id)) => {
                let path = readable(&ctx, &access, path_id)?.ok_or_else(|| WsError::not_found(format!("{path_id} not found")))?;
                if path.type_id != TYPE_SHOW_PATH {
                    return Err(path_unusable("not_a_path", &format!("{path_id} is not a presentation path")));
                }
                if path.payload.get("purpose").and_then(Value::as_str) != Some("presentation") {
                    return Err(path_unusable("guide", "a guide is not printed: only a presentation path is"));
                }
                surfaces.clear();
                let mut here = false;
                for step in path.payload.get("steps").and_then(Value::as_array).into_iter().flatten() {
                    let step_id = step["id"].as_str().unwrap_or("");
                    let (target, s) = match resolve_step(&ctx, step)? {
                        Ok(found) => found,
                        Err(why) => {
                            issues.push(issue("step_skipped", &[("step_id", step_id), ("detail", why)]));
                            continue;
                        }
                    };
                    if !(s.scope == SCOPE_SHARED && access.can_read(&ctx, &s)?) {
                        issues.push(issue("surface_unreadable", &[("step_id", step_id)]));
                    } else if !access.can(&ctx, &s.entity_id, Cap::Export)? {
                        issues.push(issue("surface_not_exportable", &[("step_id", step_id), ("surface_id", &s.entity_id)]));
                    } else if !(target.scope == SCOPE_SHARED && access.can_read(&ctx, &target)?) {
                        issues.push(issue("content_unreadable", &[("step_id", step_id)]));
                    } else {
                        here |= s.entity_id == surface.entity_id;
                        if !surfaces.contains(&s.entity_id) {
                            surfaces.push(s.entity_id.clone());
                        }
                        if target.type_id == TYPE_VIEWPORT {
                            viewports.push(target.entity_id.clone());
                        }
                    }
                }
                if !here {
                    return Err(path_unusable("not_on_surface", "the path has no step printed on this canvas"));
                }
            }
        }
        let frame = match req.frame_id.as_deref() {
            None => None,
            Some(id) => {
                let f = readable(&ctx, &access, id)?.ok_or_else(|| WsError::not_found(format!("{id} not found")))?;
                if container_kind(&f) != aiworkspace_core::types::PDF_FRAME {
                    return Err(WsError::sub(Code::InvalidOperation, "PRINT_FRAME_UNUSABLE", format!("{id} is not a PDF Frame")));
                }
                Some(f.entity_id)
            }
        };
        let mut scope = Scope { ctx: &ctx, access: &access, content: BTreeSet::new(), expanded: BTreeSet::new(), denied: BTreeSet::new(), unreadable: BTreeMap::new() };
        for s in &surfaces {
            scope.walk(s, true, ("surface_id", s))?;
        }
        if let Some(path_id) = req.path_id.as_deref() {
            scope.walk(path_id, false, ("entity_id", path_id))?;
        }
        for v in &viewports {
            scope.walk(v, false, ("entity_id", v))?;
        }
        if let Some(f) = &frame {
            scope.walk(f, true, ("entity_id", f))?;
        }
        for ((key, id), n) in &scope.unreadable {
            issues.push(issue("content_unreadable", &[(key, id), ("detail", &n.to_string())]));
        }
        let content = scope.content;
        // every ancestor, so the tree holds together: whole when readable, else its id and kind only (as the outline
        // shows a path node)
        let (mut ancestors, mut path_only) = (BTreeSet::new(), BTreeSet::new());
        for id in &content {
            let mut cur = id.clone();
            while let Some(edge) = ctx.edge(&cur)? {
                let p = edge.parent_id;
                if content.contains(&p) || ancestors.contains(&p) || path_only.contains(&p) {
                    break;
                }
                match ctx.entity(&p)? {
                    Some(e) if e.scope == SCOPE_SHARED && access.can_read(&ctx, &e)? => ancestors.insert(p.clone()),
                    Some(_) => path_only.insert(p.clone()),
                    None => break,
                };
                cur = p;
            }
        }
        // the bytes the pages show: pictures, snapshots and posters; audio and video travel as their poster
        let (mut assets, mut objects, mut total) = (Vec::new(), BTreeSet::new(), 0u64);
        for id in &content {
            let Some(e) = ctx.entity(id)? else { continue };
            if e.type_id != TYPE_ASSET && e.type_id != TYPE_CELL {
                continue;
            }
            let main = e.payload.get("object_id").and_then(Value::as_str).filter(|_| e.type_id == TYPE_ASSET);
            for obj in aiworkspace_core::types::asset_object_ids(&e.type_id, &e.payload) {
                if !objects.insert(obj.to_string()) {
                    continue;
                }
                let info = ctx.asset(obj)?;
                let media = info.as_ref().map(|i| i.media_type.as_str()).unwrap_or("");
                if main == Some(obj) && (media.starts_with("audio/") || media.starts_with("video/")) {
                    issues.push(issue("asset_omitted", &[("entity_id", id), ("detail", media)]));
                    continue;
                }
                let Some(info) = info.filter(|_| self.objects.availability(obj) == crate::objects::Availability::Available) else {
                    issues.push(issue("asset_missing", &[("entity_id", id), ("detail", obj)]));
                    continue;
                };
                if assets.len() >= MAX_ASSET_FILES || total + info.size > MAX_ASSET_BYTES {
                    issues.push(issue("asset_omitted", &[("entity_id", id), ("detail", "package_limit")]));
                    continue;
                }
                let Some(file) = obj_id_to_filename(obj) else { continue };
                total += info.size;
                let ext = extension(&info.media_type).map(|e| format!(".{e}")).unwrap_or_default();
                assets.push((obj.to_string(), format!("assets/{file}{ext}")));
            }
        }
        let rows = self.print_rows(&ctx, &content, &ancestors, &path_only, &objects)?;
        let fonts = font_settings(&self.settings()?.0);
        let title = surface.payload.get("title").and_then(Value::as_str).or(surface.name.as_deref()).unwrap_or("");
        let job = json!({ "surface_id": surface.entity_id, "mode": mode, "path_id": req.path_id, "paper": req.paper, "frame_id": frame,
                          "fonts": fonts, "title": title, "date": req.date, "locale": req.locale });
        let package = json!({
            "format": PRINT_FORMAT, "format_version": PRINT_FORMAT_VERSION, "workspace_id": self.workspace_id, "title": self.title(),
            "epoch": self.epoch, "head_seq": self.head_seq, "principal": caller.principal, "printed_at": self.changed_at()?.unwrap_or_default(),
            "job": job, "rows": rows,
            "assets": assets.iter().map(|(o, p)| (o.clone(), json!(p))).collect::<Map<_, _>>(),
            "issues": issues,
        });
        Ok(PrintContents { package, assets, workspace_id: self.workspace_id.clone(), exports: self.bucket(crate::objects::EXPORTS)? })
    }

    /// The Replica rows (`tables_json`) of `content` (notes left out), its readable `ancestors` and the bare
    /// `path_only` ones.
    fn print_rows(&self, ctx: &SqlCtx, content: &BTreeSet<String>, ancestors: &BTreeSet<String>, path_only: &BTreeSet<String>, objects: &BTreeSet<String>) -> WsResult<Value> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let one = |sql: &str, id: &str| query_rows(&self.doc, sql, [id]);
        let all: BTreeSet<&String> = content.iter().chain(ancestors).chain(path_only).collect();
        let mut entities = Vec::new();
        for id in content.iter().chain(ancestors) {
            for mut row in one("SELECT * FROM entities WHERE entity_id = ?1", id)? {
                if let Some(stripped) = ctx.entity(id)?.as_ref().and_then(without_notes) {
                    row["payload_json"] = json!(Value::Object(stripped.payload).to_string());
                }
                entities.push(row);
            }
        }
        for id in path_only {
            for mut row in one("SELECT * FROM entities WHERE entity_id = ?1", id)? {
                let kind = if row["type_id"] == json!(TYPE_CONTAINER) {
                    ctx.entity(id)?.and_then(|e| e.payload.get("kind").cloned()).map(|k| json!({ "kind": k })).unwrap_or_else(|| json!({}))
                } else {
                    json!({})
                };
                row["payload_json"] = json!(kind.to_string());
                row["key_revs_json"] = json!("{}");
                for k in ["name", "derived_json", "created_by"] {
                    row[k] = Value::Null;
                }
                entities.push(row);
            }
        }
        let mut edges = Vec::new();
        for id in &all {
            edges.extend(one("SELECT * FROM tree_edges WHERE child_id = ?1", id)?);
        }
        let mut refs = Vec::new();
        for id in content {
            for r in one("SELECT * FROM refs WHERE src_entity_id = ?1", id)? {
                let dst = r["dst_entity_id"].as_str().unwrap_or("");
                if dst.is_empty() || r["dst_workspace_id"].as_str().is_some_and(|w| !w.is_empty()) || all.contains(&dst.to_string()) {
                    refs.push(r);
                }
            }
        }
        let (mut fields, mut records, mut richtexts) = (Vec::new(), Vec::new(), Vec::new());
        for id in content {
            let Some(e) = ctx.entity(id)? else { continue };
            if e.type_id == TYPE_TABLE {
                fields.extend(one("SELECT * FROM table_fields WHERE source_id = ?1 AND deleted_seq IS NULL", id)?);
                let dead: Vec<String> = one("SELECT field_id FROM table_fields WHERE source_id = ?1 AND deleted_seq IS NOT NULL", id)?
                    .iter()
                    .filter_map(|r| r["field_id"].as_str().map(str::to_string))
                    .collect();
                for mut r in one("SELECT * FROM table_records WHERE source_id = ?1 AND deleted_seq IS NULL", id)? {
                    strip_fields(&mut r, &dead);
                    records.push(r);
                }
            } else if e.type_id == TYPE_RICHTEXT {
                // pending updates folded into the snapshot: the package needs no update rows
                let rt = ctx.richtext(id)?;
                for mut r in one("SELECT * FROM richtext_states WHERE entity_id = ?1", id)? {
                    if let Some(rt) = &rt {
                        r["snapshot"] = json!(b64.encode(richtext::export_snapshot(&rt.doc)?));
                        r["snapshot_seq"] = json!(self.head_seq);
                    }
                    richtexts.push(r);
                }
            }
        }
        let mut assets = Vec::new();
        for obj in objects {
            assets.extend(one("SELECT object_id, media_type, size FROM assets WHERE object_id = ?1", obj)?);
        }
        Ok(json!({ "entities": entities, "tree_edges": edges, "table_fields": fields, "table_records": records,
                   "richtext_states": richtexts, "refs": refs, "assets": assets }))
    }

    /// `print.package` in one go: [`Self::print_contents`] stored with [`PrintContents::store`].
    pub fn print_package(&mut self, caller: &Caller, req: &PrintRequest, renderer: &[(String, Vec<u8>)]) -> WsResult<PrintBuilt> {
        self.check_quota(0, false)?;
        self.print_contents(caller, req)?.store(&self.objects, renderer)
    }
}
