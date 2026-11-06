use super::{function, parse_pdfinfo, tool_output_error, tools, STDOUT_LIMIT, TO_PDF};
use crate::native::{param_str, ExecutionLane, NativeContext, NativeFunction, NativeOutcome, ToolEnvironment};
use async_trait::async_trait;
use buckyos_api::{error_codes, FunctionObject, ThunkExecutionError};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const FORMATS: [&str; 8] = ["docx", "pptx", "odt", "odp", "doc", "ppt", "rtf", "txt"];

/// `is_pure` here means the result is reusable, not byte-identical: LibreOffice
/// stamps `CreationDate`, `/ID` and `DocChecksum` into every PDF. The first
/// conversion is cached, which keeps `pdf_content_id` stable for the page steps.
pub fn function_object() -> FunctionObject {
    function(
        TO_PDF,
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "format"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "format": {"type": "string", "enum": FORMATS}
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

pub struct ToPdf;

impl ToPdf {
    pub const TOOLS: &'static [&'static str] = &[tools::SOFFICE];
}

#[async_trait]
impl NativeFunction for ToPdf {
    fn tools(&self) -> &'static [&'static str] {
        Self::TOOLS
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Office
    }

    /// The rendered pages depend on the installed fonts; the page count on pdfinfo.
    fn fingerprint(&self, env: &ToolEnvironment) -> BTreeMap<String, String> {
        let mut fingerprint = env.fingerprint_for(&[tools::SOFFICE, tools::PDFINFO]);
        fingerprint.insert(
            "env/fonts".to_string(),
            env.fonts.clone().unwrap_or_else(|| "unknown".to_string()),
        );
        fingerprint
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        ctx.require_tools(Self::TOOLS)?;
        let content_id = param_str(params, "content_id")?;
        let format = param_str(params, "format")?;
        if !FORMATS.contains(&format) {
            return Err(crate::native::invalid_params(format!("unsupported document format `{format}`")));
        }
        let (input, _) = ctx.materialize_input(content_id, &format!("input.{format}")).await?;
        let out_dir = ctx.temp_dir.join("out");
        let profile = ctx.temp_dir.join("profile");
        for dir in [&out_dir, &profile] {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|err| ThunkExecutionError::new(error_codes::INTERNAL, format!("create {}: {err}", dir.display())))?;
        }
        let soffice = ctx.tool_path(tools::SOFFICE)?;
        let args = vec![
            format!("-env:UserInstallation=file://{}", profile.to_string_lossy()),
            "--headless".to_string(),
            "--norestore".to_string(),
            "--nologo".to_string(),
            "--convert-to".to_string(),
            "pdf".to_string(),
            "--outdir".to_string(),
            out_dir.to_string_lossy().into_owned(),
            input.to_string_lossy().into_owned(),
        ];
        let out = ctx.run_tool(&soffice, &args, 64 * 1024).await?;
        let pdf_path = out_dir.join("input.pdf");
        if !out.success || !pdf_path.is_file() {
            return Err(tool_output_error(&out, "document conversion produced no PDF"));
        }
        let pages = match ctx.env.tool(tools::PDFINFO) {
            Some(info) => {
                let pdfinfo_out = ctx
                    .run_tool(&info.path, &[pdf_path.to_string_lossy().into_owned()], STDOUT_LIMIT)
                    .await?;
                if pdfinfo_out.success {
                    parse_pdfinfo(&pdfinfo_out.stdout_string()).pages
                } else {
                    0
                }
            }
            None => 0,
        };
        let artifact = ctx
            .io
            .put_artifact_file(&ctx.artifact_key("derived.pdf"), "derived.pdf", "application/pdf", &pdf_path)
            .await?;
        Ok(NativeOutcome::new(json!({
            "pdf_content_id": artifact.content_id,
            "pdf_obj_id": artifact.obj_id,
            "pages": pages,
        }))
        .with_artifacts(vec![artifact.obj_id]))
    }
}
