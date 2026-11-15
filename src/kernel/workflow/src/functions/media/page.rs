//! `media.page_to_pdf` / `media.page_snapshot`: one HTML file, or a zip with
//! `index.html` at its root, rendered by `page-render` (headless Chromium).
//! The page is third-party code: the tool runs through
//! [`NativeContext::run_tool_isolated`], without network.

use super::probe::sniff_format;
use super::{function_revised, resource_limit, tool_output_error, tools, IMPLEMENTATION_REVISION, PAGE_SNAPSHOT, PAGE_TO_PDF};
use crate::native::{
    internal_error, invalid_params, param_str, param_u64, CancelToken, ExecutionLane, NativeContext, NativeFunction,
    NativeOutcome, ToolEnvironment, ToolIsolation,
};
use async_trait::async_trait;
use buckyos_api::{error_codes, FunctionObject, ThunkExecutionError};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

/// Bump when what the page functions produce changes on their own.
const PAGE_REVISION: &str = "page-1";

fn revision() -> String {
    format!("{IMPLEMENTATION_REVISION}+{PAGE_REVISION}")
}

pub const PAPERS: [&str; 4] = ["A3", "A4", "A5", "Letter"];
const MAX_TEMPLATE_BYTES: usize = 64 * 1024;
const MAX_WAIT_READY_BYTES: usize = 2048;
const MAX_ZIP_ENTRIES: usize = 10_000;
/// How `page-render` exits when its own `--timeout` ran out.
const EXIT_TIMED_OUT: i32 = 3;
/// How `page-render` exits when the node has no browser for it to start: the
/// browser is the online item `chromium` of ext-core, downloaded by the node
/// after the package is installed.
const EXIT_NO_BROWSER: i32 = 6;

/// `is_pure` as for `media.to_pdf`: reusable, not byte-identical (Chromium
/// stamps the creation time into every PDF).
pub fn to_pdf_function_object() -> FunctionObject {
    function_revised(
        PAGE_TO_PDF,
        &revision(),
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "paper", "landscape", "margin_mm", "background"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "paper": {"type": "string"},
                "landscape": {"type": "boolean"},
                "margin_mm": {"type": "number", "minimum": 0, "maximum": 100},
                "background": {"type": "boolean"},
                "header_html": {"type": "string"},
                "footer_html": {"type": "string"},
                "wait_ready": {"type": "string"},
                "wait_ms": {"type": "integer", "minimum": 0, "maximum": 60000}
            }
        }),
        "media.derived_pdf",
        json!({
            "type": "object",
            "required": ["pdf_content_id", "pdf_obj_id", "pages"],
            "properties": {
                "pdf_content_id": {"type": "string"},
                "pdf_obj_id": {"type": "string"},
                "pages": {"type": "integer"}
            }
        }),
    )
}

pub fn snapshot_function_object() -> FunctionObject {
    function_revised(
        PAGE_SNAPSHOT,
        &revision(),
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "viewport_width", "viewport_height", "scale"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "viewport_width": {"type": "integer", "minimum": 320, "maximum": 3840},
                "viewport_height": {"type": "integer", "minimum": 240, "maximum": 4096},
                "scale": {"type": "number", "minimum": 1, "maximum": 3},
                "wait_ready": {"type": "string"},
                "wait_ms": {"type": "integer", "minimum": 0, "maximum": 60000}
            }
        }),
        "media.page_image",
        json!({
            "type": "object",
            "required": ["image_content_id", "image_obj_id", "width", "height"],
            "properties": {
                "image_content_id": {"type": "string"},
                "image_obj_id": {"type": "string"},
                "width": {"type": "integer"},
                "height": {"type": "integer"}
            }
        }),
    )
}

/// What the page looks like depends on the browser and on the installed fonts.
fn fingerprint(env: &ToolEnvironment) -> BTreeMap<String, String> {
    let mut fingerprint = env.fingerprint_for(&[tools::PAGE_RENDER]);
    fingerprint.insert(
        "env/fonts".to_string(),
        env.fonts.clone().unwrap_or_else(|| "unknown".to_string()),
    );
    fingerprint
}

fn optional_str<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params.get(name).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn number(params: &Value, name: &str, range: RangeInclusive<f64>) -> Result<f64, ThunkExecutionError> {
    params
        .get(name)
        .and_then(Value::as_f64)
        .filter(|value| range.contains(value))
        .ok_or_else(|| invalid_params(format!("param `{name}` must be a number from {} to {}", range.start(), range.end())))
}

fn flag(params: &Value, name: &str) -> Result<bool, ThunkExecutionError> {
    params
        .get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid_params(format!("param `{name}` must be a boolean")))
}

/// A named size, or `<width>x<height>mm`.
pub(crate) fn paper_arg(paper: &str) -> Result<String, ThunkExecutionError> {
    if PAPERS.contains(&paper) {
        return Ok(paper.to_string());
    }
    let side = |text: &str| text.parse::<f64>().ok().filter(|mm| (10.0..=5000.0).contains(mm));
    paper
        .strip_suffix("mm")
        .and_then(|size| size.split_once('x'))
        .and_then(|(width, height)| Some(format!("{}x{}mm", side(width)?, side(height)?)))
        .ok_or_else(|| invalid_params(format!("param `paper` must be one of {} or `<width>x<height>mm`", PAPERS.join(", "))))
}

fn wait_args(params: &Value, args: &mut Vec<String>) -> Result<(), ThunkExecutionError> {
    if let Some(ready) = optional_str(params, "wait_ready") {
        if ready.len() > MAX_WAIT_READY_BYTES || ready.starts_with('-') {
            return Err(invalid_params("param `wait_ready` must be a short expression that does not start with `-`"));
        }
        args.extend(["--wait-ready".to_string(), ready.to_string()]);
    }
    if let Some(ms) = params.get("wait_ms").and_then(Value::as_u64).filter(|ms| *ms > 0) {
        args.extend(["--wait-ms".to_string(), ms.to_string()]);
    }
    Ok(())
}

fn arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn io_error(what: &str, path: &Path, err: std::io::Error) -> ThunkExecutionError {
    internal_error(format!("{what} {}: {err}", path.display()))
}

/// The directory handed to the tool: `site/` holds the page, `out/` its output.
fn work_dir(ctx: &NativeContext) -> PathBuf {
    ctx.temp_dir.join("page")
}

/// Lays the page out as `site/index.html` plus whatever a zip brought with it.
async fn prepare(ctx: &NativeContext, content_id: &str) -> Result<PathBuf, ThunkExecutionError> {
    let work = work_dir(ctx);
    let site = work.join("site");
    for dir in [&site, &work.join("out")] {
        tokio::fs::create_dir_all(dir).await.map_err(|err| io_error("create", dir, err))?;
    }
    let (input, _) = ctx.materialize_input(content_id, "page.input").await?;
    let mut head = Vec::new();
    tokio::fs::File::open(&input)
        .await
        .map_err(|err| io_error("open", &input, err))?
        .take(8)
        .read_to_end(&mut head)
        .await
        .map_err(|err| io_error("read", &input, err))?;
    let index = site.join("index.html");
    if sniff_format(&head) == "zip" {
        let (target, max_bytes) = (site.clone(), ctx.limits.max_input_bytes);
        ctx.run_blocking(move |cancel| unpack(&input, &target, max_bytes, &cancel)).await?;
        if !tokio::fs::metadata(&index).await.is_ok_and(|meta| meta.is_file()) {
            return Err(invalid_params("the zip has no `index.html` at its root"));
        }
    } else {
        tokio::fs::rename(&input, &index).await.map_err(|err| io_error("move", &input, err))?;
    }
    Ok(work)
}

/// Every entry becomes a plain file under `site`; a link in the archive is
/// written as the text it holds.
fn unpack(archive: &Path, site: &Path, max_bytes: u64, cancel: &CancelToken) -> Result<(), ThunkExecutionError> {
    let file = std::fs::File::open(archive).map_err(|err| io_error("open", archive, err))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|err| invalid_params(format!("the input is not a readable zip: {err}")))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(resource_limit(
            format!("the zip has {} entries, more than {MAX_ZIP_ENTRIES}", zip.len()),
            json!({"entries": zip.len(), "max_entries": MAX_ZIP_ENTRIES}),
        ));
    }
    let mut total = 0u64;
    for i in 0..zip.len() {
        if cancel.is_cancelled() {
            return Err(ThunkExecutionError::new(error_codes::CANCELLED, "execution cancelled"));
        }
        let mut entry = zip.by_index(i).map_err(|err| invalid_params(format!("zip entry {i}: {err}")))?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(invalid_params(format!("zip entry `{}` leaves the archive", entry.name())));
        };
        let path = site.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|err| io_error("create", &path, err))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| io_error("create", parent, err))?;
        }
        let mut out = std::fs::File::create(&path).map_err(|err| io_error("create", &path, err))?;
        total += std::io::copy(&mut (&mut entry).take(max_bytes - total + 1), &mut out).map_err(|err| io_error("write", &path, err))?;
        if total > max_bytes {
            return Err(resource_limit(
                format!("the zip unpacks to more than {max_bytes} bytes"),
                json!({"max_input_bytes": max_bytes}),
            ));
        }
    }
    Ok(())
}

/// Runs `page-render` on the prepared page; returns the line it reports
/// (`{pages, bytes, sandbox, warnings}`).
async fn render(ctx: &NativeContext, work: &Path, mut args: Vec<String>, output: &Path) -> Result<Value, ThunkExecutionError> {
    let page_render = ctx.tool_path(tools::PAGE_RENDER)?;
    let secs = ctx
        .remaining()
        .map(|left| left.as_secs())
        .unwrap_or(ctx.limits.max_exec_secs)
        .saturating_sub(5)
        .max(5);
    args.extend(["--timeout".to_string(), secs.to_string()]);
    let out = ctx
        .run_tool_isolated(&page_render, &args, &ToolIsolation { dir: work, network: false }, 64 * 1024)
        .await?;
    // The tool owned the directory: only a plain file is taken from it.
    let produced = tokio::fs::symlink_metadata(output)
        .await
        .ok()
        .filter(|meta| meta.is_file() && meta.len() > 0);
    let report = out
        .stdout_string()
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok().filter(Value::is_object));
    let (Some(produced), Some(report)) = (produced.filter(|_| out.success), report) else {
        if out.exit_code == Some(EXIT_NO_BROWSER) {
            return Err(ThunkExecutionError::new(
                error_codes::TOOLS_MISSING,
                "this node has not downloaded the browser yet (the online item `chromium` of ext-core): download it in Settings › Extension tools",
            )
            .with_details(json!({
                "tools_missing": ["chromium"],
                "online_item": "chromium",
                "tool": out.tool,
                "stderr_tail": out.stderr_tail,
            })));
        }
        if out.exit_code == Some(EXIT_TIMED_OUT) {
            return Err(ThunkExecutionError::new(error_codes::TIMEOUT, "page-render: the page was not ready in time")
                .with_details(json!({"tool": out.tool, "stderr_tail": out.stderr_tail})));
        }
        return Err(tool_output_error(&out, "the page was not rendered"));
    };
    if produced.len() > ctx.limits.max_output_bytes {
        return Err(resource_limit(
            format!("the rendered page is {} bytes, more than {}", produced.len(), ctx.limits.max_output_bytes),
            json!({"bytes": produced.len(), "max_output_bytes": ctx.limits.max_output_bytes}),
        ));
    }
    if report.get("sandbox") == Some(&Value::Bool(false)) {
        log::warn!(
            "workflow: page-render ran without the browser sandbox execution_id={}",
            ctx.execution.execution_id
        );
    }
    Ok(report)
}

fn metrics(report: &Value) -> Value {
    json!({"sandbox": report.get("sandbox"), "warnings": report.get("warnings")})
}

/// A stand-in for `page-render` that keeps its contract without a browser: it
/// records its arguments and environment in its working directory, writes a
/// file of the kind asked for and prints the report line. While a file
/// `no-browser` is beside it, it is a `page-render` whose browser the node
/// has not downloaded.
#[cfg(all(test, unix))]
pub(crate) fn stub_page_render(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    const STUB: &str = r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "page-render 0.0.0-stub chromium 0 playwright-core 0"; exit 0; fi
if [ -e "${0%/*}/no-browser" ]; then echo "page-render: Chrome for Testing is not downloaded on this node yet" >&2; exit 6; fi
printf '%s\n' "$@" > args.txt
env > env.txt
while [ $# -gt 0 ]; do
  case "$1" in
    --html) [ -f "$2/index.html" ] || { echo "no index.html in $2" >&2; exit 3; }; shift 2 ;;
    --pdf) printf '%%PDF-1.4\n%%stub\n' > "$2"; shift 2 ;;
    --png) printf '\211PNG\r\n\032\n\000\000\000\015IHDR\000\000\005\000\000\000\003\040' > "$2"; shift 2 ;;
    *) shift ;;
  esac
done
echo "stub: rendering"
echo '{"pages": 3, "bytes": 15, "sandbox": false, "warnings": ["stub"]}'
"#;
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("page-render");
    std::fs::write(&path, STUB).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

pub struct PageToPdf;

impl PageToPdf {
    pub const TOOLS: &'static [&'static str] = &[tools::PAGE_RENDER];
}

#[async_trait]
impl NativeFunction for PageToPdf {
    fn tools(&self) -> &'static [&'static str] {
        Self::TOOLS
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Browser
    }

    fn fingerprint(&self, env: &ToolEnvironment) -> BTreeMap<String, String> {
        fingerprint(env)
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        ctx.require_tools(Self::TOOLS)?;
        let content_id = param_str(params, "content_id")?;
        let work = work_dir(ctx);
        let output = work.join("out").join("page.pdf");
        let mut args = vec![
            "--html".to_string(),
            arg(&work.join("site")),
            "--pdf".to_string(),
            arg(&output),
            "--paper".to_string(),
            paper_arg(param_str(params, "paper")?)?,
            "--margin".to_string(),
            number(params, "margin_mm", 0.0..=100.0)?.to_string(),
        ];
        if flag(params, "landscape")? {
            args.push("--landscape".to_string());
        }
        if flag(params, "background")? {
            args.push("--background".to_string());
        }
        let mut templates = Vec::new();
        for (param, option, file) in [("header_html", "--header-html", "header.html"), ("footer_html", "--footer-html", "footer.html")] {
            let Some(html) = optional_str(params, param) else {
                continue;
            };
            if html.len() > MAX_TEMPLATE_BYTES {
                return Err(invalid_params(format!("param `{param}` is larger than {MAX_TEMPLATE_BYTES} bytes")));
            }
            let path = work.join(file);
            args.extend([option.to_string(), arg(&path)]);
            templates.push((path, html));
        }
        wait_args(params, &mut args)?;
        prepare(ctx, content_id).await?;
        for (path, html) in templates {
            tokio::fs::write(&path, html).await.map_err(|err| io_error("write", &path, err))?;
        }
        let report = render(ctx, &work, args, &output).await?;
        let artifact = ctx
            .io
            .put_artifact_file(&ctx.artifact_key("page.pdf"), "page.pdf", "application/pdf", &output)
            .await?;
        let mut outcome = NativeOutcome::new(json!({
            "pdf_content_id": artifact.content_id,
            "pdf_obj_id": artifact.obj_id,
            "pages": report.get("pages").and_then(Value::as_u64).unwrap_or(0),
        }))
        .with_artifacts(vec![artifact.obj_id]);
        outcome.metrics = metrics(&report);
        Ok(outcome)
    }
}

pub struct PageSnapshot;

impl PageSnapshot {
    pub const TOOLS: &'static [&'static str] = &[tools::PAGE_RENDER];
}

/// Width and height of a PNG, from its header.
async fn png_size(path: &Path) -> Result<(u32, u32), ThunkExecutionError> {
    let mut head = Vec::new();
    tokio::fs::File::open(path)
        .await
        .map_err(|err| io_error("open", path, err))?
        .take(24)
        .read_to_end(&mut head)
        .await
        .map_err(|err| io_error("read", path, err))?;
    if head.len() < 24 || sniff_format(&head) != "png" || &head[12..16] != b"IHDR" {
        return Err(ThunkExecutionError::new(error_codes::TOOL_FAILED, "page-render did not write a PNG")
            .with_details(json!({"tool": tools::PAGE_RENDER})));
    }
    let side = |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    Ok((side(16), side(20)))
}

#[async_trait]
impl NativeFunction for PageSnapshot {
    fn tools(&self) -> &'static [&'static str] {
        Self::TOOLS
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Browser
    }

    fn fingerprint(&self, env: &ToolEnvironment) -> BTreeMap<String, String> {
        fingerprint(env)
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        ctx.require_tools(Self::TOOLS)?;
        let content_id = param_str(params, "content_id")?;
        let viewport = (param_u64(params, "viewport_width")?, param_u64(params, "viewport_height")?);
        let scale = number(params, "scale", 1.0..=3.0)?;
        if !(320..=3840).contains(&viewport.0) || !(240..=4096).contains(&viewport.1) {
            return Err(invalid_params("the viewport must be 320..3840 wide and 240..4096 high"));
        }
        let over_limit = |width: f64, height: f64| {
            (width * height > ctx.limits.max_page_pixels as f64).then(|| {
                resource_limit(
                    format!("a {width}x{height} image exceeds {} pixels", ctx.limits.max_page_pixels),
                    json!({"width": width, "height": height, "max_page_pixels": ctx.limits.max_page_pixels}),
                )
            })
        };
        if let Some(err) = over_limit(viewport.0 as f64 * scale, viewport.1 as f64 * scale) {
            return Err(err);
        }
        let work = work_dir(ctx);
        let output = work.join("out").join("page.png");
        let mut args = vec![
            "--html".to_string(),
            arg(&work.join("site")),
            "--png".to_string(),
            arg(&output),
            "--viewport".to_string(),
            format!("{}x{}", viewport.0, viewport.1),
            "--scale".to_string(),
            scale.to_string(),
        ];
        wait_args(params, &mut args)?;
        prepare(ctx, content_id).await?;
        let report = render(ctx, &work, args, &output).await?;
        let (width, height) = png_size(&output).await?;
        if let Some(err) = over_limit(width as f64, height as f64) {
            return Err(err);
        }
        let artifact = ctx
            .io
            .put_artifact_file(&ctx.artifact_key("page.png"), "page.png", "image/png", &output)
            .await?;
        let mut outcome = NativeOutcome::new(json!({
            "image_content_id": artifact.content_id,
            "image_obj_id": artifact.obj_id,
            "width": width,
            "height": height,
        }))
        .with_artifacts(vec![artifact.obj_id]);
        outcome.metrics = metrics(&report);
        Ok(outcome)
    }
}
