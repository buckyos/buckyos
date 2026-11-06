use super::*;
use crate::io::MemoryNativeIo;
use crate::native::{CancelToken, ExecutionLimits, NativeIo, NativeOutcome, ToolEnvironment};
use crate::object_store::InMemoryObjectStore;
use buckyos_api::ThunkExecution;
use ::image::{ImageEncoder, RgbaImage};
use ndn_lib::ObjId;
use serde_json::json;
use std::collections::BTreeMap;

struct Env {
    io: Arc<MemoryNativeIo>,
    _tmp: tempfile::TempDir,
    ctx: NativeContext,
}

fn env_with(tool_env: ToolEnvironment) -> Env {
    let io = Arc::new(MemoryNativeIo::new(Arc::new(InMemoryObjectStore::new())));
    let tmp = tempfile::tempdir().unwrap();
    let ctx = NativeContext {
        execution: ThunkExecution {
            execution_id: "run-1/step/1".into(),
            thunk_obj_id: ObjId::new("thunk:00").unwrap(),
            run_id: "run-1".into(),
            node_id: "step".into(),
            attempt: 1,
            execution_epoch: 1,
            source_obj_id: None,
            requested_by: None,
            deadline_at: None,
            registration: None,
            fingerprint: BTreeMap::new(),
            cache_key: None,
        },
        temp_dir: tmp.path().to_path_buf(),
        cancel: CancelToken::new(),
        deadline: None,
        env: Arc::new(tool_env),
        io: io.clone(),
        limits: ExecutionLimits::default(),
        artifact_prefix: "media/test".into(),
    };
    Env { io, _tmp: tmp, ctx }
}

fn env() -> Env {
    env_with(ToolEnvironment::default())
}

fn png(width: u32, height: u32, alpha: bool) -> Vec<u8> {
    let img = RgbaImage::from_fn(width, height, |x, y| {
        let a = if alpha && (x + y) % 2 == 0 { 128 } else { 255 };
        ::image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, a])
    });
    let mut out = Vec::new();
    ::image::codecs::png::PngEncoder::new(&mut out)
        .write_image(img.as_raw(), width, height, ::image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let img = ::image::RgbImage::from_fn(width, height, |x, y| ::image::Rgb([(x % 256) as u8, (y % 256) as u8, 10]));
    let mut out = Vec::new();
    img.write_with_encoder(::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85))
        .unwrap();
    out
}

async fn invoke(f: &dyn NativeFunction, e: &Env, params: Value) -> Result<NativeOutcome, ThunkExecutionError> {
    f.invoke(&e.ctx, &params).await
}

#[test]
fn registry_publishes_nine_functions_and_baseline_lists_their_tools() {
    let mut registry = FunctionRegistry::new();
    register(&mut registry).unwrap();
    let names: Vec<String> = registry.list().iter().map(|f| f.registration.name.clone()).collect();
    assert_eq!(names.len(), 9);
    for name in [CAPABILITIES, PROBE, FIT_IMAGE, EXTRACT_PDF_TEXT, RENDER_PDF_PAGES, EXTRACT_FRAMES, EXTRACT_AUDIO, TO_PDF, BUILD_CATALOG] {
        let f = registry.resolve(name, "1").expect(name);
        f.object.validate().unwrap();
        assert!(f.native.is_some());
        assert_eq!(f.registration.requirements.get("runner/workflow"), Some(&0));
    }
    let render = registry.resolve(RENDER_PDF_PAGES, "1").unwrap();
    assert!(render.registration.requirements.contains_key("cmd/pdftoppm"));
    assert!(render.registration.requirements.contains_key("cmd/pdfinfo"));
    assert!(!registry.resolve(CAPABILITIES, "1").unwrap().object.is_pure);
    assert!(!registry.resolve(TO_PDF, "1").unwrap().object.is_pure);
    assert!(registry.resolve(FIT_IMAGE, "1").unwrap().object.is_pure);

    let baseline = media_tool_baseline();
    let tools: Vec<&str> = baseline.iter().map(|t| t.tool).collect();
    assert_eq!(tools, vec!["ffmpeg", "ffprobe", "pdfinfo", "pdftoppm", "pdftotext", "soffice"]);
    let ffmpeg = baseline.iter().find(|t| t.tool == "ffmpeg").unwrap();
    assert_eq!(ffmpeg.used_by, vec![EXTRACT_AUDIO, EXTRACT_FRAMES, FIT_IMAGE]);
    let soffice = baseline.iter().find(|t| t.tool == "soffice").unwrap();
    assert_eq!(soffice.used_by, vec![TO_PDF]);
}

#[tokio::test]
async fn capabilities_reports_environment_and_is_never_cached() {
    let mut tools = BTreeMap::new();
    tools.insert("pdfinfo".to_string(), crate::native::ToolInfo { path: "/usr/bin/pdfinfo".into(), version: "24.02".into() });
    let mut tool_env = ToolEnvironment::with_tools(tools);
    tool_env.missing = vec!["soffice".into()];
    let e = env_with(tool_env);
    let out = invoke(&capabilities::Capabilities, &e, json!({})).await.unwrap();
    assert!(!out.cacheable);
    assert_eq!(out.result["tools"]["pdfinfo"]["version"], "24.02");
    assert_eq!(out.result["missing"][0], "soffice");
    assert_eq!(out.result["revision"], e.ctx.env.revision);
    capabilities::function_object().validate_result(&out.result).unwrap();
}

#[tokio::test]
async fn probe_reads_image_headers_and_sniffs_documents() {
    let e = env();
    let (_, content_id) = e.io.add_file("a.png", &png(30, 20, false)).await;
    let out = invoke(&probe::Probe, &e, json!({"content_id": content_id, "family": "image"})).await.unwrap();
    assert_eq!(out.result["kind"], "image");
    assert_eq!(out.result["mime"], "image/png");
    assert_eq!(out.result["width"], 30);
    assert_eq!(out.result["height"], 20);
    assert_eq!(out.result["format"], "png");
    probe::function_object().validate_result(&out.result).unwrap();

    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&[64, 0, 48, 0, 0, 0, 0]);
    let (_, gif_id) = e.io.add_file("a.gif", &gif).await;
    let out = invoke(&probe::Probe, &e, json!({"content_id": gif_id, "family": "image"})).await.unwrap();
    assert_eq!(out.result["mime"], "image/gif");
    assert_eq!(out.result["width"], 64);

    let (_, doc_id) = e.io.add_file("a.docx", b"PK\x03\x04rest").await;
    let out = invoke(&probe::Probe, &e, json!({"content_id": doc_id, "family": "document"})).await.unwrap();
    assert_eq!(out.result["kind"], "document");
    assert_eq!(out.result["format"], "zip");
    assert_eq!(out.result["size"], 8);

    let err = invoke(&probe::Probe, &e, json!({"content_id": doc_id, "family": "pdf"})).await.unwrap_err();
    assert_eq!(err.code, error_codes::TOOLS_MISSING);
    assert_eq!(err.details["tools_missing"], json!(["pdfinfo"]));
    let err = invoke(&probe::Probe, &e, json!({"content_id": doc_id, "family": "video"})).await.unwrap_err();
    assert_eq!(err.details["tools_missing"], json!(["ffprobe"]));
}

#[tokio::test]
async fn fit_image_passes_through_downscales_and_keeps_alpha() {
    let e = env();
    let limits = json!({"max_long_edge": 64, "max_pixels": 64 * 64, "max_bytes": 1_000_000});
    let (_, small) = e.io.add_file("small.jpg", &jpeg(40, 30)).await;
    let mut p = limits.clone();
    p["content_id"] = json!(small);
    let out = invoke(&image::FitImage, &e, p).await.unwrap();
    assert_eq!(out.result["actions"], json!([]));
    assert_eq!(out.result["mime"], "image/jpeg");
    assert_eq!(out.result["sent"]["width"], 40);
    let id = out.result["model_media_obj_id"].as_str().unwrap().to_string();
    assert_eq!(out.artifact_refs, vec![id.clone()]);
    assert_eq!(e.io.artifact_bytes(&id).unwrap(), jpeg(40, 30));
    image::function_object().validate_result(&out.result).unwrap();

    let (_, big) = e.io.add_file("big.jpg", &jpeg(400, 200)).await;
    let mut p = limits.clone();
    p["content_id"] = json!(big);
    let out = invoke(&image::FitImage, &e, p).await.unwrap();
    assert!(out.result["actions"].as_array().unwrap().contains(&json!("downscaled")));
    assert_eq!(out.result["sent"]["width"], 64);
    assert_eq!(out.result["sent"]["height"], 32);
    assert_eq!(out.result["original"]["width"], 400);

    let (_, alpha) = e.io.add_file("alpha.png", &png(200, 100, true)).await;
    let mut p = limits.clone();
    p["content_id"] = json!(alpha);
    let out = invoke(&image::FitImage, &e, p).await.unwrap();
    assert_eq!(out.result["mime"], "image/png");
    let id = out.result["model_media_obj_id"].as_str().unwrap();
    let sent = ::image::load_from_memory(&e.io.artifact_bytes(id).unwrap()).unwrap();
    assert!(sent.color().has_alpha());
    assert_eq!(sent.width(), 64);

    // Same bytes -> same artifact id (deterministic FileObject).
    let out2 = invoke(&image::FitImage, &e, json!({"content_id": alpha, "max_long_edge": 64, "max_pixels": 4096, "max_bytes": 1_000_000})).await.unwrap();
    assert_eq!(out2.result["model_media_obj_id"], out.result["model_media_obj_id"]);

    // Decode bomb guard.
    let mut e2 = env();
    e2.ctx.limits.max_page_pixels = 1000;
    let (_, big) = e2.io.add_file("big.jpg", &jpeg(400, 200)).await;
    let err = invoke(&image::FitImage, &e2, json!({"content_id": big, "max_long_edge": 64, "max_pixels": 4096, "max_bytes": 1_000_000})).await.unwrap_err();
    assert_eq!(err.code, error_codes::RESOURCE_LIMIT);

    // Non-decodable without ffmpeg.
    let (_, weird) = e.io.add_file("x.bin", b"not an image at all").await;
    let err = invoke(&image::FitImage, &e, json!({"content_id": weird, "max_long_edge": 64, "max_pixels": 4096, "max_bytes": 1_000_000})).await.unwrap_err();
    assert_eq!(err.code, error_codes::TOOLS_MISSING);
    assert_eq!(err.details["tools_missing"], json!(["ffmpeg"]));
}

#[tokio::test]
async fn pdf_and_video_functions_report_every_missing_tool() {
    let e = env();
    let (_, id) = e.io.add_file("f.bin", b"%PDF-1.4").await;
    let err = invoke(&pdf::ExtractPdfText, &e, json!({"content_id": id, "page_range": [], "max_pages": 10})).await.unwrap_err();
    assert_eq!(err.code, error_codes::TOOLS_MISSING);
    assert_eq!(err.details["tools_missing"], json!(["pdfinfo", "pdftotext"]));
    let err = invoke(&pdf::RenderPdfPages, &e, json!({"content_id": id, "dpi": 72, "max_long_edge": 512, "page_range": [], "max_pages": 10})).await.unwrap_err();
    assert_eq!(err.details["tools_missing"], json!(["pdfinfo", "pdftoppm"]));
    let err = invoke(&video::ExtractFrames, &e, json!({"content_id": id, "strategy": "uniform", "max_frames": 4, "frame_max_edge": 320, "max_duration_ms": 60000})).await.unwrap_err();
    assert_eq!(err.details["tools_missing"], json!(["ffprobe", "ffmpeg"]));
    let err = invoke(&video::ExtractAudio, &e, json!({"content_id": id, "max_duration_ms": 60000})).await.unwrap_err();
    assert_eq!(err.details["tools_missing"], json!(["ffprobe", "ffmpeg"]));
    let err = invoke(&document::ToPdf, &e, json!({"content_id": id, "format": "docx"})).await.unwrap_err();
    assert_eq!(err.details["tools_missing"], json!(["soffice"]));
}

#[test]
fn page_helpers_are_deterministic() {
    assert_eq!(requested_pages(&[], 5, 10), vec![1, 2, 3, 4, 5]);
    assert_eq!(requested_pages(&[], 5, 3), vec![1, 2, 3]);
    assert_eq!(requested_pages(&[9, 3, 1, 3, 0], 5, 10), vec![1, 3]);
    let info = parse_pdfinfo("Title: x\nPages:          3\nPage    1 size: 612 x 792 pts (letter)\nPage    2 size: 595 x 842 pts\nPage    3 size: 612 x 792 pts\n");
    assert_eq!(info.pages, 3);
    assert_eq!(info.page_sizes[1], (595.0, 842.0));
    let info = parse_pdfinfo("Pages: 2\nPage size: 612 x 792 pts\n");
    assert_eq!(info.page_sizes, vec![(612.0, 792.0); 2]);
    assert_eq!(pdf::expected_page_dims((612.0, 792.0), 72, 2048), (612, 792));
    assert_eq!(pdf::expected_page_dims((612.0, 792.0), 300, 1024), (791, 1024));
    assert_eq!(video::uniform_timestamps(2000, 4), vec![250, 750, 1250, 1750]);
    assert_eq!(video::uniform_timestamps(0, 4), Vec::<u64>::new());
    let info = video::parse_ffprobe(r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":64,"height":48},{"codec_type":"audio","codec_name":"aac"}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"2.040000"}}"#).unwrap();
    assert_eq!(info.duration_ms, 2040);
    assert_eq!(info.width, Some(64));
    assert_eq!(info.audio_codec.as_deref(), Some("aac"));
    assert_eq!(info.mime(), "video/mp4");
    assert_eq!(probe::sniff_format(b"%PDF-1.7"), "pdf");
    assert_eq!(probe::sniff_format(&[0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm']), "mp4");
}

#[tokio::test]
async fn build_catalog_covers_pages_frames_and_absent_audio() {
    let e = env();
    let text = json!({"total_pages": 4, "pages": [
        {"page": 1, "status": "ok", "text_obj_id": "cyfile:a1", "chars": 10},
        {"page": 2, "status": "no_text", "chars": 0},
        {"page": 3, "status": "failed", "error": {"code": "tool_failed", "message": "x", "retryable": true}}
    ]});
    let images = json!({"total_pages": 4, "pages": [
        {"page": 1, "status": "ok", "image_obj_id": "cyfile:b1", "width": 10, "height": 10},
        {"page": 2, "status": "ok", "image_obj_id": "cyfile:b2", "width": 10, "height": 10},
        {"page": 3, "status": "ok", "image_obj_id": "cyfile:b3", "width": 10, "height": 10}
    ]});
    let out = invoke(&catalog::BuildCatalog, &e, json!({
        "family": "pdf", "source_content_id": "sha256:src", "derived_content_id": null,
        "probe": {"kind": "pdf", "pages": 4}, "text": text, "images": images
    })).await.unwrap();
    assert!(!out.cacheable);
    assert_eq!(out.result["complete"], false);
    let manifest_id = out.result["manifest_obj_id"].as_str().unwrap();
    assert!(out.artifact_refs.contains(&manifest_id.to_string()));
    assert!(out.artifact_refs.contains(&"cyfile:b3".to_string()));
    let stored = e.io.get_object(manifest_id).await.unwrap().unwrap();
    let manifest: buckyos_api::MediaManifest = serde_json::from_value(stored).unwrap();
    assert_eq!(manifest.entries.len(), 4);
    assert_eq!(manifest.entries[0].status, buckyos_api::MediaEntryStatus::Ok);
    assert_eq!(manifest.entries[1].status, buckyos_api::MediaEntryStatus::NoText);
    assert!(manifest.entries[1].text_obj_id.is_none());
    assert_eq!(manifest.entries[2].status, buckyos_api::MediaEntryStatus::Failed);
    assert_eq!(manifest.entries[3].status, buckyos_api::MediaEntryStatus::Unprocessed);
    assert_eq!(manifest.range.total, Some(4));
    assert_eq!(manifest.range.requested, vec![1, 2, 3]);
    assert_eq!(manifest.range.processed, vec![1, 2]);
    assert_eq!(manifest.range.failed, vec![3]);
    assert_eq!(manifest.entries[0].location.source, buckyos_api::MediaSourceRef::Original);
    assert_eq!(out.result["summary"]["failed"], 1);
    catalog::function_object().validate_result(&out.result).unwrap();

    // Complete, cacheable, no-text page keeps its image.
    let out = invoke(&catalog::BuildCatalog, &e, json!({
        "family": "document", "source_content_id": "sha256:src", "derived_content_id": "sha256:pdf",
        "probe": {}, "text": {"total_pages": 1, "pages": [{"page": 1, "status": "no_text", "chars": 0}]},
        "images": {"total_pages": 1, "pages": [{"page": 1, "status": "ok", "image_obj_id": "cyfile:p1"}]}
    })).await.unwrap();
    assert!(out.cacheable);
    assert_eq!(out.result["complete"], true);
    let manifest: buckyos_api::MediaManifest = serde_json::from_value(e.io.get_object(out.result["manifest_obj_id"].as_str().unwrap()).await.unwrap().unwrap()).unwrap();
    assert_eq!(manifest.entries[0].status, buckyos_api::MediaEntryStatus::NoText);
    assert_eq!(manifest.entries[0].image_obj_id.as_deref(), Some("cyfile:p1"));
    assert_eq!(manifest.entries[0].location.source, buckyos_api::MediaSourceRef::Derived);
    assert_eq!(manifest.derived_content_id.as_deref(), Some("sha256:pdf"));

    // Video: frames + absent audio is complete.
    let out = invoke(&catalog::BuildCatalog, &e, json!({
        "family": "video", "source_content_id": "sha256:v", "derived_content_id": null, "probe": {},
        "frames": {"duration_ms": 2000, "frames": [
            {"index": 0, "time_ms": 500, "image_obj_id": "cyfile:f0", "status": "ok"},
            {"index": 1, "time_ms": 1500, "image_obj_id": "cyfile:f1", "status": "ok"}
        ]},
        "audio": {"status": "absent", "duration_ms": 2000}
    })).await.unwrap();
    assert_eq!(out.result["complete"], true);
    assert_eq!(out.result["summary"]["absent"], 1);
    assert_eq!(out.result["summary"]["image_entries"], 2);
    let manifest: buckyos_api::MediaManifest = serde_json::from_value(e.io.get_object(out.result["manifest_obj_id"].as_str().unwrap()).await.unwrap().unwrap()).unwrap();
    assert_eq!(manifest.range.requested, vec![500, 1500]);
    assert_eq!(manifest.entries[2].kind, buckyos_api::MediaEntryKind::Audio);
}

// ----- tool-backed tests (run with `cargo test -p workflow -- --ignored`) -----

fn minimal_pdf(pages: &[&str]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let n = pages.len();
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());
    for text in pages {
        let page_num = objects.len() + 1;
        let content = if text.is_empty() {
            String::new()
        } else {
            format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET")
        };
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            page_num + 1
        ));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn real_env() -> Option<Env> {
    let tool_env = tools::probe_default();
    if tool_env.tools.is_empty() {
        eprintln!("skipped: no media tools found (searched {:?})", tool_env.searched);
        return None;
    }
    Some(env_with(tool_env))
}

fn has(e: &Env, names: &[&str]) -> bool {
    let missing = e.ctx.env.missing_from(names);
    if !missing.is_empty() {
        eprintln!("skipped: missing {missing:?}");
        return false;
    }
    true
}

#[tokio::test]
#[ignore]
async fn poppler_extracts_text_renders_pages_and_probes() {
    let Some(e) = real_env() else { return };
    if !has(&e, &[tools::PDFINFO, tools::PDFTOTEXT, tools::PDFTOPPM]) {
        return;
    }
    let pdf = minimal_pdf(&["First page", "", "ANSWER-42"]);
    let (_, id) = e.io.add_file("doc.pdf", &pdf).await;
    let out = invoke(&probe::Probe, &e, json!({"content_id": id, "family": "pdf"})).await.unwrap();
    assert_eq!(out.result["pages"], 3);
    assert_eq!(out.result["width"], 612);

    let out = invoke(&pdf::ExtractPdfText, &e, json!({"content_id": id, "page_range": [], "max_pages": 10})).await.unwrap();
    assert!(out.cacheable);
    let pages = out.result["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[1]["status"], "no_text");
    assert_eq!(pages[2]["status"], "ok");
    let text = e.io.artifact_bytes(pages[2]["text_obj_id"].as_str().unwrap()).unwrap();
    assert!(String::from_utf8(text).unwrap().contains("ANSWER-42"));
    pdf::extract_text_function_object().validate_result(&out.result).unwrap();

    let out = invoke(&pdf::ExtractPdfText, &e, json!({"content_id": id, "page_range": [3, 3, 9], "max_pages": 10})).await.unwrap();
    assert_eq!(out.result["pages"].as_array().unwrap().len(), 1);
    assert_eq!(out.result["pages"][0]["page"], 3);

    let out = invoke(&pdf::RenderPdfPages, &e, json!({"content_id": id, "dpi": 144, "max_long_edge": 400, "page_range": [1, 3], "max_pages": 10})).await.unwrap();
    let pages = out.result["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["status"], "ok");
    assert_eq!(pages[0]["height"], 400);
    let png = e.io.artifact_bytes(pages[0]["image_obj_id"].as_str().unwrap()).unwrap();
    let dims = ::image::load_from_memory(&png).unwrap();
    assert_eq!(dims.height(), 400);
    assert_eq!(out.artifact_refs.len(), 2);
    pdf::render_pages_function_object().validate_result(&out.result).unwrap();

    let out = invoke(&pdf::RenderPdfPages, &e, json!({"content_id": id, "dpi": 36, "max_long_edge": 4096, "page_range": [2], "max_pages": 10})).await.unwrap();
    assert_eq!(out.result["pages"][0]["width"], 306);

    let mut small = env_with((*e.ctx.env).clone());
    small.ctx.limits.max_total_pixels = 10;
    let (_, id2) = small.io.add_file("doc.pdf", &pdf).await;
    let err = invoke(&pdf::RenderPdfPages, &small, json!({"content_id": id2, "dpi": 72, "max_long_edge": 500, "page_range": [], "max_pages": 10})).await.unwrap_err();
    assert_eq!(err.code, error_codes::RESOURCE_LIMIT);
}

async fn make_video(e: &Env, with_audio: bool) -> Option<String> {
    let ffmpeg = e.ctx.env.tool(tools::FFMPEG)?.path.clone();
    let path = e.ctx.temp_dir.join(if with_audio { "a.mp4" } else { "v.mp4" });
    let mut args = vec!["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=64x64:rate=5"];
    if with_audio {
        args.extend(["-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-c:a", "aac"]);
    }
    args.extend(["-pix_fmt", "yuv420p", "-c:v", "libx264", "-t", "2"]);
    let args: Vec<String> = args.into_iter().map(String::from).chain([path.to_string_lossy().into_owned()]).collect();
    let out = e.ctx.run_tool(&ffmpeg, &args, 64 * 1024).await.unwrap();
    if !out.success {
        eprintln!("skipped: ffmpeg cannot generate the fixture: {}", out.stderr_tail);
        return None;
    }
    let bytes = tokio::fs::read(&path).await.unwrap();
    Some(e.io.add_file("video.mp4", &bytes).await.1)
}

#[tokio::test]
#[ignore]
async fn ffmpeg_extracts_frames_and_audio() {
    let Some(e) = real_env() else { return };
    if !has(&e, &[tools::FFPROBE, tools::FFMPEG]) {
        return;
    }
    let Some(silent) = make_video(&e, false).await else { return };
    let out = invoke(&probe::Probe, &e, json!({"content_id": silent, "family": "video"})).await.unwrap();
    assert_eq!(out.result["has_audio"], false);
    assert_eq!(out.result["width"], 64);
    assert!(out.result["duration_ms"].as_u64().unwrap() >= 1900);

    let out = invoke(&video::ExtractFrames, &e, json!({"content_id": silent, "strategy": "uniform", "max_frames": 4, "frame_max_edge": 32, "max_duration_ms": 60000})).await.unwrap();
    let frames = out.result["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 4);
    assert!(frames.iter().all(|f| f["status"] == "ok"));
    assert_eq!(frames[0]["width"], 32);
    assert_eq!(out.artifact_refs.len(), 4);
    video::extract_frames_function_object().validate_result(&out.result).unwrap();

    let out = invoke(&video::ExtractAudio, &e, json!({"content_id": silent, "max_duration_ms": 60000})).await.unwrap();
    assert_eq!(out.result["status"], "absent");
    assert!(out.artifact_refs.is_empty());

    let err = invoke(&video::ExtractFrames, &e, json!({"content_id": silent, "strategy": "uniform", "max_frames": 4, "frame_max_edge": 32, "max_duration_ms": 1000})).await.unwrap_err();
    assert_eq!(err.code, error_codes::RESOURCE_LIMIT);

    let Some(with_audio) = make_video(&e, true).await else { return };
    let out = invoke(&video::ExtractAudio, &e, json!({"content_id": with_audio, "max_duration_ms": 60000})).await.unwrap();
    assert_eq!(out.result["status"], "ok");
    assert_eq!(out.result["codec"], "aac");
    let mp3 = e.io.artifact_bytes(out.result["audio_obj_id"].as_str().unwrap()).unwrap();
    assert!(mp3.len() > 1000);
    video::extract_audio_function_object().validate_result(&out.result).unwrap();
}

#[tokio::test]
#[ignore]
async fn soffice_converts_documents_to_pdf() {
    let Some(e) = real_env() else { return };
    if !has(&e, &[tools::SOFFICE]) {
        return;
    }
    let (_, id) = e.io.add_file("note.txt", b"Hello from a text document\nANSWER-42\n").await;
    let out = invoke(&document::ToPdf, &e, json!({"content_id": id, "format": "txt"})).await.unwrap();
    assert!(!out.cacheable);
    let pdf = e.io.artifact_bytes(out.result["pdf_obj_id"].as_str().unwrap()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(out.result["pdf_content_id"].as_str().unwrap().contains(':'));
    if e.ctx.env.tool(tools::PDFINFO).is_some() {
        assert_eq!(out.result["pages"], 1);
    }
    document::function_object().validate_result(&out.result).unwrap();
}
