//! `media.*` native functions (TODO T2a / T2b): capabilities, probe,
//! fit_image, extract_pdf_text, render_pdf_pages, extract_frames,
//! extract_audio, to_pdf, build_catalog. Tool requirements derived from the
//! registrations are the host / paios baseline (TODO T2c, §2.5).

pub mod capabilities;
pub mod catalog;
pub mod document;
pub mod image;
pub mod pdf;
pub mod probe;
pub mod tools;
pub mod video;

#[cfg(test)]
mod tests;

use crate::error::WorkflowResult;
use crate::function_registry::{FunctionRegistry, RegistrationSpec};
use crate::native::{NativeContext, NativeFunction, ToolOutput};
use buckyos_api::{
    error_codes, FunctionObject, FunctionParamType, FunctionResultType, FunctionType,
    ThunkExecutionError,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

pub const CAPABILITIES: &str = "media.capabilities";
pub const PROBE: &str = "media.probe";
pub const FIT_IMAGE: &str = "media.fit_image";
pub const EXTRACT_PDF_TEXT: &str = "media.extract_pdf_text";
pub const RENDER_PDF_PAGES: &str = "media.render_pdf_pages";
pub const EXTRACT_FRAMES: &str = "media.extract_frames";
pub const EXTRACT_AUDIO: &str = "media.extract_audio";
pub const TO_PDF: &str = "media.to_pdf";
pub const BUILD_CATALOG: &str = "media.build_catalog";

pub const STDOUT_LIMIT: usize = 16 * 1024 * 1024;

/// Bump when what any media function produces changes: the revision is part of
/// `fun_id`, so results of the old implementation stop matching. impl-3: artifact
/// FileObjects carry the common meta under `mime`.
const IMPLEMENTATION_REVISION: &str = "impl-3";

pub fn content_param() -> FunctionParamType {
    FunctionParamType::CheckByRunner("file_content".to_string())
}

pub fn function(
    name: &str,
    is_pure: bool,
    content_params: &[&str],
    params_schema: Value,
    result_name: &str,
    result_schema: Value,
) -> FunctionObject {
    FunctionObject {
        func_type: FunctionType::Native,
        content: format!("{name}/{IMPLEMENTATION_REVISION}"),
        is_pure,
        params_type: content_params
            .iter()
            .map(|p| (p.to_string(), content_param()))
            .collect::<BTreeMap<_, _>>(),
        params_schema,
        result_type: FunctionResultType::Object(result_name.to_string()),
        result_schema,
    }
}

struct Registration {
    name: &'static str,
    description: &'static str,
    timeout: u64,
    tools: &'static [&'static str],
    object: FunctionObject,
    native: Arc<dyn NativeFunction>,
}

fn registrations() -> Vec<Registration> {
    vec![
        Registration {
            name: CAPABILITIES,
            description: "Reports the media tool environment of this runner",
            timeout: 30,
            tools: &[],
            object: capabilities::function_object(),
            native: Arc::new(capabilities::Capabilities),
        },
        Registration {
            name: PROBE,
            description: "Reads the type, size and dimensions of an image, PDF, video or document",
            timeout: 60,
            tools: &[],
            object: probe::function_object(),
            native: Arc::new(probe::Probe),
        },
        Registration {
            name: FIT_IMAGE,
            description: "Fits an image into a vision model's edge, pixel and byte limits",
            timeout: 120,
            tools: &[],
            object: image::function_object(),
            native: Arc::new(image::FitImage),
        },
        Registration {
            name: EXTRACT_PDF_TEXT,
            description: "Extracts per-page text from a PDF",
            timeout: 300,
            tools: pdf::ExtractPdfText::TOOLS,
            object: pdf::extract_text_function_object(),
            native: Arc::new(pdf::ExtractPdfText),
        },
        Registration {
            name: RENDER_PDF_PAGES,
            description: "Renders PDF pages to page images",
            timeout: 600,
            tools: pdf::RenderPdfPages::TOOLS,
            object: pdf::render_pages_function_object(),
            native: Arc::new(pdf::RenderPdfPages),
        },
        Registration {
            name: EXTRACT_FRAMES,
            description: "Samples uniformly spaced frames of a video",
            timeout: 600,
            tools: video::ExtractFrames::TOOLS,
            object: video::extract_frames_function_object(),
            native: Arc::new(video::ExtractFrames),
        },
        Registration {
            name: EXTRACT_AUDIO,
            description: "Extracts the audio track of a video",
            timeout: 600,
            tools: video::ExtractAudio::TOOLS,
            object: video::extract_audio_function_object(),
            native: Arc::new(video::ExtractAudio),
        },
        Registration {
            name: TO_PDF,
            description: "Converts an office document to PDF with LibreOffice",
            timeout: 600,
            tools: document::ToPdf::TOOLS,
            object: document::function_object(),
            native: Arc::new(document::ToPdf),
        },
        Registration {
            name: BUILD_CATALOG,
            description: "Builds the media manifest of a conversion",
            timeout: 60,
            tools: &[],
            object: catalog::function_object(),
            native: Arc::new(catalog::BuildCatalog),
        },
    ]
}

pub fn register(registry: &mut FunctionRegistry) -> WorkflowResult<()> {
    for reg in registrations() {
        let mut spec = RegistrationSpec::new(reg.name, "1")
            .description(reg.description)
            .timeout(reg.timeout)
            .requirement("runner/workflow", 0);
        for tool in reg.tools {
            spec = spec.requirement(&format!("cmd/{tool}"), 0);
        }
        registry.register_native(spec, reg.object, reg.native)?;
    }
    Ok(())
}

/// One entry of the media tool baseline the workflow host and the paios
/// image must both satisfy (TODO §2.5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolRequirement {
    pub tool: &'static str,
    pub used_by: Vec<&'static str>,
    pub min_version: Option<&'static str>,
    pub purpose: &'static str,
}

/// Derives the baseline from the registrations: a tool appears once with
/// every function that hard-requires it. `ffmpeg` is additionally an optional
/// dependency of `media.fit_image` (non-decodable formats) and `pdfinfo` of
/// `media.to_pdf` (page count); those are listed under `used_by` too.
pub fn media_tool_baseline() -> Vec<ToolRequirement> {
    let mut by_tool: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    for reg in registrations() {
        for tool in reg.tools {
            by_tool.entry(tool).or_default().push(reg.name);
        }
    }
    by_tool.entry(tools::FFMPEG).or_default().push(FIT_IMAGE);
    by_tool.entry(tools::PDFINFO).or_default().push(TO_PDF);
    by_tool
        .into_iter()
        .map(|(tool, mut used_by)| {
            used_by.sort();
            used_by.dedup();
            let (min_version, purpose) = match tool {
                tools::PDFINFO => (Some("22.0"), "PDF page count and page sizes (poppler)"),
                tools::PDFTOTEXT => (Some("22.0"), "per-page PDF text extraction (poppler)"),
                tools::PDFTOPPM => (Some("22.0"), "PDF page rendering to PNG (poppler)"),
                tools::FFPROBE => (Some("6.0"), "video / audio stream probing"),
                tools::FFMPEG => (Some("6.0"), "frame sampling, audio extraction, exotic image decoding (needs libmp3lame)"),
                tools::SOFFICE => (Some("7.0"), "office document to PDF conversion (headless LibreOffice with fonts and locale)"),
                _ => (None, ""),
            };
            ToolRequirement {
                tool,
                used_by,
                min_version,
                purpose,
            }
        })
        .collect()
}

pub(crate) fn tool_output_error(out: &ToolOutput, message: &str) -> ThunkExecutionError {
    ThunkExecutionError::new(error_codes::TOOL_FAILED, format!("{}: {message}", out.tool)).with_details(json!({
        "tool": out.tool,
        "exit_code": out.exit_code,
        "stderr_tail": out.stderr_tail,
    }))
}

pub(crate) fn resource_limit(message: String, details: Value) -> ThunkExecutionError {
    ThunkExecutionError::new(error_codes::RESOURCE_LIMIT, message).with_details(details)
}

pub(crate) fn page_param(params: &Value, name: &str) -> Result<u32, ThunkExecutionError> {
    let value = crate::native::param_u64(params, name)?;
    u32::try_from(value)
        .map_err(|_| crate::native::invalid_params(format!("param `{name}` is out of range")))
}

/// Expands `page_range` (empty = every page) into the ordered, deduplicated
/// list of existing pages the function will process, truncated to
/// `max_pages`. The truncation is part of the params, so the result stays
/// deterministic and cacheable.
pub(crate) fn requested_pages(page_range: &[u64], total_pages: u32, max_pages: u32) -> Vec<u32> {
    let mut pages: Vec<u32> = if page_range.is_empty() {
        (1..=total_pages).collect()
    } else {
        page_range
            .iter()
            .filter_map(|p| u32::try_from(*p).ok())
            .filter(|p| *p >= 1 && *p <= total_pages)
            .collect()
    };
    pages.sort_unstable();
    pages.dedup();
    pages.truncate(max_pages as usize);
    pages
}

pub(crate) fn page_range_param(params: &Value) -> Result<Vec<u64>, ThunkExecutionError> {
    match params.get("page_range") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_u64()
                    .ok_or_else(|| crate::native::invalid_params("page_range items must be positive integers"))
            })
            .collect(),
        Some(_) => Err(crate::native::invalid_params("page_range must be an array")),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PdfInfo {
    pub pages: u32,
    /// Per page `(width_pt, height_pt)` in document order, when known.
    pub page_sizes: Vec<(f64, f64)>,
}

pub(crate) fn parse_pdfinfo(text: &str) -> PdfInfo {
    let mut pages = 0u32;
    let mut sizes: Vec<(u32, (f64, f64))> = Vec::new();
    let mut default_size = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "Pages" {
            pages = value.parse().unwrap_or(0);
        } else if key == "Page size" {
            default_size = parse_size(value);
        } else if let Some(rest) = key.strip_prefix("Page").and_then(|r| r.strip_suffix(" size")) {
            if let (Ok(n), Some(size)) = (rest.trim().parse::<u32>(), parse_size(value)) {
                sizes.push((n, size));
            }
        }
    }
    sizes.sort_by_key(|(n, _)| *n);
    let page_sizes = if sizes.len() as u32 == pages && pages > 0 {
        sizes.into_iter().map(|(_, s)| s).collect()
    } else if let Some(size) = default_size {
        vec![size; pages as usize]
    } else {
        Vec::new()
    };
    PdfInfo { pages, page_sizes }
}

fn parse_size(value: &str) -> Option<(f64, f64)> {
    let mut it = value.split_whitespace();
    let w = it.next()?.parse::<f64>().ok()?;
    if it.next()? != "x" {
        return None;
    }
    let h = it.next()?.parse::<f64>().ok()?;
    Some((w, h))
}

pub(crate) async fn pdf_info(ctx: &NativeContext, path: &Path) -> Result<PdfInfo, ThunkExecutionError> {
    let pdfinfo = ctx.tool_path(tools::PDFINFO)?;
    let out = ctx
        .run_tool(
            &pdfinfo,
            &["-f".to_string(), "1".to_string(), "-l".to_string(), "1000000".to_string(), path.to_string_lossy().into_owned()],
            STDOUT_LIMIT,
        )
        .await?
        .ok()?;
    let info = parse_pdfinfo(&out.stdout_string());
    if info.pages == 0 {
        return Err(tool_output_error(&out, "could not read the page count"));
    }
    Ok(info)
}

pub(crate) fn mime_of_format(format: &str) -> &'static str {
    match format {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "doc" => "application/msword",
        "ppt" => "application/vnd.ms-powerpoint",
        "rtf" => "application/rtf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}
