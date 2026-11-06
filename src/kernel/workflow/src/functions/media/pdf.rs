use super::{
    function, page_param, page_range_param, pdf_info, requested_pages, resource_limit,
    tool_output_error, tools, EXTRACT_PDF_TEXT, RENDER_PDF_PAGES, STDOUT_LIMIT,
};
use crate::native::{param_str, ExecutionLane, NativeContext, NativeFunction, NativeOutcome};
use async_trait::async_trait;
use buckyos_api::{error_codes, FunctionObject, ThunkExecutionError};
use serde_json::{json, Value};
use std::path::Path;

fn page_entry_error(err: &ThunkExecutionError) -> Value {
    json!({"code": err.code, "message": err.message, "retryable": err.retryable})
}

fn is_fatal(err: &ThunkExecutionError) -> bool {
    matches!(
        err.code.as_str(),
        error_codes::CANCELLED | error_codes::TIMEOUT | error_codes::TOOLS_MISSING | error_codes::INPUT_UNAVAILABLE
    )
}

pub fn extract_text_function_object() -> FunctionObject {
    function(
        EXTRACT_PDF_TEXT,
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "page_range", "max_pages"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "page_range": {"type": "array", "items": {"type": "integer", "minimum": 1}},
                "max_pages": {"type": "integer", "minimum": 1}
            }
        }),
        "media.pdf_text",
        json!({
            "type": "object",
            "required": ["pages", "total_pages"],
            "properties": {
                "pages": {"type": "array", "items": {
                    "type": "object",
                    "required": ["page", "status"],
                    "properties": {
                        "page": {"type": "integer"},
                        "status": {"type": "string", "enum": ["ok", "no_text", "failed"]},
                        "text_obj_id": {"type": "string"},
                        "chars": {"type": "integer"},
                        "error": {"type": "object"}
                    }
                }},
                "total_pages": {"type": "integer"}
            }
        }),
    )
}

pub struct ExtractPdfText;

impl ExtractPdfText {
    pub const TOOLS: &'static [&'static str] = &[tools::PDFINFO, tools::PDFTOTEXT];
}

#[async_trait]
impl NativeFunction for ExtractPdfText {
    fn tools(&self) -> &'static [&'static str] {
        Self::TOOLS
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Pdf
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        ctx.require_tools(Self::TOOLS)?;
        let content_id = param_str(params, "content_id")?;
        let page_range = page_range_param(params)?;
        let max_pages = page_param(params, "max_pages")?.min(ctx.limits.max_pages);
        let (path, _) = ctx.materialize_input(content_id, "input.pdf").await?;
        let info = pdf_info(ctx, &path).await?;
        let pages = requested_pages(&page_range, info.pages, max_pages);
        let pdftotext = ctx.tool_path(tools::PDFTOTEXT)?;
        let mut entries = Vec::with_capacity(pages.len());
        let mut cacheable = true;
        for page in pages {
            ctx.check_cancelled()?;
            match extract_page(ctx, &pdftotext, &path, page).await {
                Ok(text) => {
                    let chars = text.chars().count();
                    if text.trim().is_empty() {
                        entries.push(json!({"page": page, "status": "no_text", "chars": 0}));
                    } else {
                        let name = format!("page-{page}.txt");
                        let artifact = ctx
                            .io
                            .put_artifact(&ctx.artifact_key(&name), &name, "text/plain; charset=utf-8", text.into_bytes())
                            .await?;
                        entries.push(json!({"page": page, "status": "ok", "text_obj_id": artifact.obj_id, "chars": chars}));
                    }
                }
                Err(err) if is_fatal(&err) => return Err(err),
                Err(err) => {
                    if err.retryable {
                        cacheable = false;
                    }
                    entries.push(json!({"page": page, "status": "failed", "error": page_entry_error(&err)}));
                }
            }
        }
        let refs = entries
            .iter()
            .filter_map(|e| e.get("text_obj_id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let outcome = NativeOutcome::new(json!({"pages": entries, "total_pages": info.pages})).with_artifacts(refs);
        Ok(if cacheable { outcome } else { outcome.not_cacheable() })
    }
}

async fn extract_page(ctx: &NativeContext, pdftotext: &Path, path: &Path, page: u32) -> Result<String, ThunkExecutionError> {
    let args = vec![
        "-f".to_string(),
        page.to_string(),
        "-l".to_string(),
        page.to_string(),
        "-layout".to_string(),
        "-enc".to_string(),
        "UTF-8".to_string(),
        path.to_string_lossy().into_owned(),
        "-".to_string(),
    ];
    let out = ctx.run_tool(pdftotext, &args, STDOUT_LIMIT).await?;
    if !out.success {
        return Err(tool_output_error(&out, &format!("page {page} text extraction failed")).retryable());
    }
    Ok(String::from_utf8_lossy(&out.stdout).replace('\u{c}', ""))
}

pub fn render_pages_function_object() -> FunctionObject {
    function(
        RENDER_PDF_PAGES,
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "dpi", "max_long_edge", "page_range", "max_pages"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "dpi": {"type": "integer", "minimum": 1},
                "max_long_edge": {"type": "integer", "minimum": 1},
                "page_range": {"type": "array", "items": {"type": "integer", "minimum": 1}},
                "max_pages": {"type": "integer", "minimum": 1}
            }
        }),
        "media.page_images",
        json!({
            "type": "object",
            "required": ["pages", "total_pages"],
            "properties": {
                "pages": {"type": "array", "items": {
                    "type": "object",
                    "required": ["page", "status"],
                    "properties": {
                        "page": {"type": "integer"},
                        "status": {"type": "string", "enum": ["ok", "failed"]},
                        "image_obj_id": {"type": "string"},
                        "width": {"type": "integer"},
                        "height": {"type": "integer"},
                        "error": {"type": "object"}
                    }
                }},
                "total_pages": {"type": "integer"}
            }
        }),
    )
}

pub struct RenderPdfPages;

impl RenderPdfPages {
    pub const TOOLS: &'static [&'static str] = &[tools::PDFINFO, tools::PDFTOPPM];
}

/// Expected output pixels of a page rendered at `dpi` and clamped to
/// `max_long_edge` on its long side.
pub fn expected_page_dims(size_pt: (f64, f64), dpi: u32, max_long_edge: u32) -> (u32, u32) {
    let w = (size_pt.0 / 72.0 * f64::from(dpi)).round().max(1.0);
    let h = (size_pt.1 / 72.0 * f64::from(dpi)).round().max(1.0);
    let long = w.max(h);
    let scale = (f64::from(max_long_edge) / long).min(1.0);
    ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32)
}

#[async_trait]
impl NativeFunction for RenderPdfPages {
    fn tools(&self) -> &'static [&'static str] {
        Self::TOOLS
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Pdf
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        ctx.require_tools(Self::TOOLS)?;
        let content_id = param_str(params, "content_id")?;
        let dpi = page_param(params, "dpi")?;
        let max_long_edge = page_param(params, "max_long_edge")?;
        let page_range = page_range_param(params)?;
        let max_pages = page_param(params, "max_pages")?.min(ctx.limits.max_pages);
        let (path, _) = ctx.materialize_input(content_id, "input.pdf").await?;
        let info = pdf_info(ctx, &path).await?;
        let pages = requested_pages(&page_range, info.pages, max_pages);

        let mut total_pixels = 0u64;
        let mut plan = Vec::with_capacity(pages.len());
        for page in &pages {
            let size = info.page_sizes.get(*page as usize - 1).copied().unwrap_or((612.0, 792.0));
            let (w, h) = expected_page_dims(size, dpi, max_long_edge);
            let pixels = u64::from(w) * u64::from(h);
            if pixels > ctx.limits.max_page_pixels {
                return Err(resource_limit(
                    format!("page {page} would render to {w}x{h} pixels, above the limit of {}", ctx.limits.max_page_pixels),
                    json!({"page": page, "width": w, "height": h, "max_page_pixels": ctx.limits.max_page_pixels}),
                ));
            }
            total_pixels += pixels;
            let natural_long = (size.0.max(size.1) / 72.0 * f64::from(dpi)).round() as u32;
            plan.push((*page, natural_long > max_long_edge));
        }
        if total_pixels > ctx.limits.max_total_pixels {
            return Err(resource_limit(
                format!("{} pages would render to {total_pixels} pixels, above the limit of {}", pages.len(), ctx.limits.max_total_pixels),
                json!({"pages": pages.len(), "total_pixels": total_pixels, "max_total_pixels": ctx.limits.max_total_pixels}),
            ));
        }

        let pdftoppm = ctx.tool_path(tools::PDFTOPPM)?;
        let mut entries = Vec::with_capacity(plan.len());
        let mut cacheable = true;
        for (page, clamp) in plan {
            ctx.check_cancelled()?;
            match render_page(ctx, &pdftoppm, &path, page, dpi, max_long_edge, clamp).await {
                Ok((artifact_id, w, h)) => {
                    entries.push(json!({"page": page, "status": "ok", "image_obj_id": artifact_id, "width": w, "height": h}));
                }
                Err(err) if is_fatal(&err) => return Err(err),
                Err(err) => {
                    if err.retryable {
                        cacheable = false;
                    }
                    entries.push(json!({"page": page, "status": "failed", "error": page_entry_error(&err)}));
                }
            }
        }
        let refs = entries
            .iter()
            .filter_map(|e| e.get("image_obj_id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let outcome = NativeOutcome::new(json!({"pages": entries, "total_pages": info.pages})).with_artifacts(refs);
        Ok(if cacheable { outcome } else { outcome.not_cacheable() })
    }
}

async fn render_page(
    ctx: &NativeContext,
    pdftoppm: &Path,
    path: &Path,
    page: u32,
    dpi: u32,
    max_long_edge: u32,
    clamp: bool,
) -> Result<(String, u32, u32), ThunkExecutionError> {
    let prefix = ctx.temp_dir.join(format!("page-{page}"));
    let mut args = vec!["-f".to_string(), page.to_string(), "-l".to_string(), page.to_string(), "-png".to_string(), "-singlefile".to_string()];
    if clamp {
        args.extend(["-scale-to".to_string(), max_long_edge.to_string()]);
    } else {
        args.extend(["-r".to_string(), dpi.to_string()]);
    }
    args.push(path.to_string_lossy().into_owned());
    args.push(prefix.to_string_lossy().into_owned());
    let out = ctx.run_tool(pdftoppm, &args, 64 * 1024).await?;
    if !out.success {
        return Err(tool_output_error(&out, &format!("page {page} rendering failed")).retryable());
    }
    let png_path = ctx.temp_dir.join(format!("page-{page}.png"));
    let (w, h) = image::ImageReader::open(&png_path)
        .and_then(|r| r.into_dimensions().map_err(std::io::Error::other))
        .map_err(|err| ThunkExecutionError::new(error_codes::TOOL_FAILED, format!("read rendered page {page}: {err}")).retryable())?;
    let name = format!("page-{page}.png");
    let artifact = ctx.io.put_artifact_file(&ctx.artifact_key(&name), &name, "image/png", &png_path).await?;
    let _ = tokio::fs::remove_file(&png_path).await;
    Ok((artifact.obj_id, w, h))
}
