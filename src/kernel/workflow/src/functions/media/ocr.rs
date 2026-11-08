//! `media.ocr_pages`: re-reads page images with tesseract where the text layer
//! is missing or does not match the page (doc/llm_context/LLM Understand
//! Media.md §4.6.5). The function produces facts only: the OCR text, its
//! confidence and how it compares with the text layer. Which text a reader
//! uses is the reader's rule, so changing that rule never invalidates this
//! cache.
//!
//! Per page window:
//! 1. Pages without a text layer are always read.
//! 2. When some text-layer pages look suspicious (few clean words), a few of
//!    them are read first; if they disagree with the layer, the layer of the
//!    whole window is judged garbled and every page is read.
//! 3. The language is chosen per window from a few sample pages: the
//!    representative model of each common script is tried and the most
//!    confident wins; OSD is asked when none of them fits. Latin text is then
//!    told apart by common words, Han text by simplified / traditional, and
//!    CJK pages whose characters stack in columns are read as vertical text.

use super::{function_revised, tool_output_error, tools, IMPLEMENTATION_REVISION, OCR_PAGES, STDOUT_LIMIT};
use crate::native::{param_str, ExecutionLane, NativeContext, NativeFunction, NativeOutcome, ToolEnvironment};
use async_trait::async_trait;
use buckyos_api::{error_codes, FunctionObject, ThunkExecutionError};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Mutex;
use std::task::Poll;

pub const LANG_AUTO: &str = "auto";
/// Text-layer pages below this clean-word percentage are suspicious.
pub const SUSPICIOUS_CLEAN: u8 = 60;
/// Probed pages agreeing less than this (median, percent) mark the layer garbled:
/// native text layers agree 77% or more, the garbled one of dk10000.pdf at most 65%.
pub const GARBLED_AGREEMENT: u8 = 70;
/// Bump when what this function decides changes; only its own results go stale.
const OCR_REVISION: &str = "ocr-2";
/// Words tesseract is less sure of are left out of the text.
const WORD_CONF_MIN: f64 = 60.0;
/// A trial below this confidence did not find its script.
const SCRIPT_CONF_MIN: f64 = 60.0;
/// A chosen model this unsure is the wrong one: its language data is missing.
const MODEL_CONF_MIN: f64 = 40.0;
const LANG_SAMPLE_PAGES: usize = 3;
const PROBE_PAGES: usize = 4;
const MAX_PAGE_TEXT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PAGE_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// One representative model per common script, tried on the sample pages.
const SCRIPT_TRIALS: [(&str, &str); 5] = [
    ("eng", "Latin"),
    ("chi_sim", "Han"),
    ("jpn", "Japanese"),
    ("kor", "Hangul"),
    ("rus", "Cyrillic"),
];

pub fn function_object() -> FunctionObject {
    function_revised(
        OCR_PAGES,
        &format!("{IMPLEMENTATION_REVISION}+{OCR_REVISION}"),
        true,
        &[],
        json!({
            "type": "object",
            "required": ["text", "images", "lang"],
            "additionalProperties": false,
            "properties": {
                "text": {"type": "object"},
                "images": {"type": "object"},
                "lang": {"type": "string", "pattern": "^(auto|[a-z][a-z_]*(\\+[a-z][a-z_]*)*)$"}
            }
        }),
        "media.ocr",
        json!({
            "type": "object",
            "required": ["status", "pages"],
            "properties": {
                "status": {"type": "string", "enum": ["ok", "not_needed", "lang_unavailable", "tools_missing"]},
                "lang": {"type": ["string", "null"]},
                "script": {"type": ["string", "null"]},
                "vertical": {"type": "boolean"},
                "garbled_layer": {"type": "boolean"},
                "basis": {"type": "object"},
                "pages": {"type": "array", "items": {
                    "type": "object",
                    "required": ["page", "status"],
                    "properties": {
                        "page": {"type": "integer"},
                        "status": {"type": "string", "enum": ["ok", "empty", "failed", "skipped", "lang_unavailable", "tools_missing"]},
                        "ocr_text_obj_id": {"type": "string"},
                        "chars": {"type": "integer"},
                        "confidence": {"type": "integer"},
                        "layer_clean": {"type": "integer"},
                        "ocr_clean": {"type": "integer"},
                        "agreement": {"type": "integer"},
                        "error": {"type": "object"}
                    }
                }}
            }
        }),
    )
}

pub struct OcrPages;

#[async_trait]
impl NativeFunction for OcrPages {
    /// None required: without tesseract the pages keep their text layer.
    fn tools(&self) -> &'static [&'static str] {
        &[]
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Pdf
    }

    /// The result depends on tesseract and the language data it has.
    fn fingerprint(&self, env: &ToolEnvironment) -> BTreeMap<String, String> {
        let mut fingerprint = env.fingerprint_for(&[tools::TESSERACT]);
        fingerprint.insert("env/ocr_langs".to_string(), env.ocr_langs.join(","));
        fingerprint
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let lang = param_str(params, "lang")?;
        if !valid_lang(lang) {
            return Err(crate::native::invalid_params(format!("param `lang` `{lang}` is neither `auto` nor tesseract language names joined by `+`")));
        }
        let pages = window_pages(ctx, params).await?;
        let mut out = OcrRun::new(pages);
        if out.candidates_hint().is_empty() {
            return Ok(out.finish("not_needed"));
        }
        let Some(tool) = ctx.env.tool(tools::TESSERACT).map(|t| t.path.clone()) else {
            out.mark_unread("tools_missing");
            return Ok(out.finish("tools_missing").not_cacheable());
        };
        let tess = Tesseract { ctx, path: tool, workers: workers() };
        let available: BTreeSet<String> = ctx.env.ocr_langs.iter().cloned().collect();
        let choice = if lang == LANG_AUTO {
            let sample: Vec<u32> = spread(&out.candidates_hint(), LANG_SAMPLE_PAGES);
            choose_language(&tess, &out, &sample, &available).await?
        } else {
            LangChoice::given(lang, &available)
        };
        out.basis = choice.basis.clone();
        out.script = choice.script.clone();
        out.vertical = choice.vertical;
        let Some(model) = choice.lang.clone() else {
            out.mark_unread("lang_unavailable");
            return Ok(out.finish("lang_unavailable"));
        };
        out.lang = Some(model.clone());
        let psm = if choice.vertical { "5" } else { "11" };

        // Suspicious layers first: are they garbled?
        let suspicious = out.suspicious();
        if !suspicious.is_empty() {
            let probe = spread(&suspicious, PROBE_PAGES);
            tess.read_pages(&mut out, &probe, &model, psm).await?;
            out.garbled_layer = median(probe.iter().filter_map(|p| out.pages[p].agreement)).is_some_and(|m| m < GARBLED_AGREEMENT);
        }
        let rest: Vec<u32> = out
            .pages
            .iter()
            .filter(|(_, p)| p.ocr.is_none() && (p.layer.is_none() || out.garbled_layer))
            .map(|(n, _)| *n)
            .collect();
        tess.read_pages(&mut out, &rest, &model, psm).await?;
        let retry = out.has_retryable_failure();
        let outcome = out.finish("ok");
        Ok(if retry { outcome.not_cacheable() } else { outcome })
    }
}

fn workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(1)
        .clamp(1, 4)
}

// ----- the pages of a window ------------------------------------------------

struct PageState {
    image_obj_id: String,
    layer: Option<String>,
    layer_clean: Option<u8>,
    ocr: Option<PageOcr>,
    agreement: Option<u8>,
    unread: Option<&'static str>,
}

enum PageOcr {
    Text { obj_id: String, chars: usize, confidence: u8, clean: Option<u8> },
    Empty { confidence: u8 },
    Failed(ThunkExecutionError),
}

struct OcrRun {
    pages: BTreeMap<u32, PageState>,
    lang: Option<String>,
    script: Option<String>,
    vertical: bool,
    garbled_layer: bool,
    basis: Value,
}

impl OcrRun {
    fn new(pages: BTreeMap<u32, PageState>) -> Self {
        Self { pages, lang: None, script: None, vertical: false, garbled_layer: false, basis: json!({}) }
    }

    fn suspicious(&self) -> Vec<u32> {
        self.pages
            .iter()
            .filter(|(_, p)| p.layer_clean.is_some_and(|c| c < SUSPICIOUS_CLEAN))
            .map(|(n, _)| *n)
            .collect()
    }

    /// Pages that will be read whatever the probe finds, else the suspicious ones.
    fn candidates_hint(&self) -> Vec<u32> {
        let no_layer: Vec<u32> = self.pages.iter().filter(|(_, p)| p.layer.is_none()).map(|(n, _)| *n).collect();
        let mut all = no_layer;
        all.extend(self.suspicious());
        all.sort_unstable();
        all
    }

    fn mark_unread(&mut self, why: &'static str) {
        for page in self.pages.values_mut() {
            if page.ocr.is_none() {
                page.unread = Some(why);
            }
        }
    }

    fn has_retryable_failure(&self) -> bool {
        self.pages.values().any(|p| matches!(&p.ocr, Some(PageOcr::Failed(e)) if e.retryable))
    }

    fn finish(self, status: &str) -> NativeOutcome {
        let mut refs = Vec::new();
        let pages: Vec<Value> = self
            .pages
            .iter()
            .map(|(n, p)| {
                let mut entry = Map::new();
                entry.insert("page".into(), json!(n));
                let status = match &p.ocr {
                    Some(PageOcr::Text { obj_id, chars, confidence, clean }) => {
                        refs.push(obj_id.clone());
                        entry.insert("ocr_text_obj_id".into(), json!(obj_id));
                        entry.insert("chars".into(), json!(chars));
                        entry.insert("confidence".into(), json!(confidence));
                        if let Some(clean) = clean {
                            entry.insert("ocr_clean".into(), json!(clean));
                        }
                        "ok"
                    }
                    Some(PageOcr::Empty { confidence }) => {
                        entry.insert("confidence".into(), json!(confidence));
                        "empty"
                    }
                    Some(PageOcr::Failed(err)) => {
                        entry.insert("error".into(), json!({"code": err.code, "message": err.message, "retryable": err.retryable}));
                        "failed"
                    }
                    None => p.unread.unwrap_or("skipped"),
                };
                entry.insert("status".into(), json!(status));
                if let Some(clean) = p.layer_clean {
                    entry.insert("layer_clean".into(), json!(clean));
                }
                if let Some(agreement) = p.agreement {
                    entry.insert("agreement".into(), json!(agreement));
                }
                Value::Object(entry)
            })
            .collect();
        NativeOutcome::new(json!({
            "status": status,
            "lang": self.lang,
            "script": self.script,
            "vertical": self.vertical,
            "garbled_layer": self.garbled_layer,
            "basis": self.basis,
            "pages": pages,
        }))
        .with_artifacts(refs)
    }
}

async fn file_content_id(ctx: &NativeContext, obj_id: &str) -> Result<String, ThunkExecutionError> {
    let object = ctx.io.get_object(obj_id).await?.ok_or_else(|| {
        ThunkExecutionError::new(error_codes::INPUT_UNAVAILABLE, format!("artifact `{obj_id}` is not available"))
            .with_details(json!({"obj_id": obj_id}))
    })?;
    object
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ThunkExecutionError::new(error_codes::INPUT_UNAVAILABLE, format!("`{obj_id}` is not a file object")))
}

/// Every page with an image, with its text layer and that layer's quality.
async fn window_pages(ctx: &NativeContext, params: &Value) -> Result<BTreeMap<u32, PageState>, ThunkExecutionError> {
    let entries = |key: &str| -> Vec<Value> {
        params.get(key).and_then(|v| v.get("pages")).and_then(Value::as_array).cloned().unwrap_or_default()
    };
    let page_of = |e: &Value| e.get("page").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok());
    let texts: BTreeMap<u32, String> = entries("text")
        .iter()
        .filter(|e| e.get("status").and_then(Value::as_str) == Some("ok"))
        .filter_map(|e| Some((page_of(e)?, e.get("text_obj_id")?.as_str()?.to_string())))
        .collect();
    let mut pages = BTreeMap::new();
    for entry in entries("images") {
        let (Some(page), Some(image)) = (page_of(&entry), entry.get("image_obj_id").and_then(Value::as_str)) else {
            continue;
        };
        if entry.get("status").and_then(Value::as_str) != Some("ok") {
            continue;
        }
        ctx.check_cancelled()?;
        let layer = match texts.get(&page) {
            Some(id) => {
                let content = file_content_id(ctx, id).await?;
                let bytes = ctx.io.read_content(&content, MAX_PAGE_TEXT_BYTES).await?;
                Some(String::from_utf8_lossy(&bytes).into_owned()).filter(|t| !t.trim().is_empty())
            }
            None => None,
        };
        pages.insert(
            page,
            PageState {
                image_obj_id: image.to_string(),
                layer_clean: layer.as_deref().and_then(clean_percent),
                layer,
                ocr: None,
                agreement: None,
                unread: None,
            },
        );
    }
    Ok(pages)
}

// ----- running tesseract ----------------------------------------------------

struct Tesseract<'a> {
    ctx: &'a NativeContext,
    path: PathBuf,
    workers: usize,
}

/// One tesseract reading of a page.
struct Reading {
    words: Vec<Word>,
}

impl Reading {
    fn confidence(&self) -> f64 {
        mean_confidence(&self.words)
    }
}

impl Tesseract<'_> {
    async fn image(&self, page: u32, image_obj_id: &str) -> Result<PathBuf, ThunkExecutionError> {
        let path = self.ctx.temp_dir.join(format!("ocr-page-{page}.png"));
        if !path.is_file() {
            let content = file_content_id(self.ctx, image_obj_id).await?;
            self.ctx.io.materialize(&content, &path, MAX_PAGE_IMAGE_BYTES).await?;
        }
        Ok(path)
    }

    async fn run(&self, image: &Path, lang: &str, psm: &str) -> Result<Reading, ThunkExecutionError> {
        let args = vec![
            image.to_string_lossy().into_owned(),
            "-".to_string(),
            "--psm".to_string(),
            psm.to_string(),
            "-l".to_string(),
            lang.to_string(),
            "tsv".to_string(),
        ];
        // One thread each: several processes in parallel beat OpenMP inside one.
        let out = self.ctx.run_tool_env(&self.path, &args, &[("OMP_THREAD_LIMIT", "1")], STDOUT_LIMIT).await?;
        if !out.success {
            return Err(tool_output_error(&out, &format!("ocr of {} failed", image.display())).retryable());
        }
        Ok(Reading { words: parse_tsv(&out.stdout_string()) })
    }

    async fn osd(&self, image: &Path) -> Option<(String, f64)> {
        let args = vec![image.to_string_lossy().into_owned(), "-".to_string(), "--psm".to_string(), "0".to_string()];
        let out = self.ctx.run_tool_env(&self.path, &args, &[("OMP_THREAD_LIMIT", "1")], 64 * 1024).await.ok()?;
        parse_osd(&format!("{}\n{}", out.stdout_string(), out.stderr_tail))
    }

    /// Reads `pages` with `lang` and records text, confidence and agreement.
    async fn read_pages(&self, run: &mut OcrRun, pages: &[u32], lang: &str, psm: &str) -> Result<(), ThunkExecutionError> {
        let jobs: Vec<(u32, String)> = pages
            .iter()
            .filter(|p| run.pages.get(p).is_some_and(|s| s.ocr.is_none()))
            .map(|p| (*p, run.pages[p].image_obj_id.clone()))
            .collect();
        let results = self
            .each(jobs, |(page, image)| async move {
                let read = async {
                    let path = self.image(page, &image).await?;
                    let reading = self.run(&path, lang, psm).await;
                    let _ = tokio::fs::remove_file(&path).await;
                    reading
                }
                .await;
                (page, read)
            })
            .await;
        for (page, read) in results {
            let reading = match read {
                Ok(reading) => reading,
                Err(err) if is_fatal(&err) => return Err(err),
                Err(err) => {
                    run.pages.get_mut(&page).expect("page").ocr = Some(PageOcr::Failed(err));
                    continue;
                }
            };
            let text = assemble(&reading.words, WORD_CONF_MIN);
            let confidence = reading.confidence().round().clamp(0.0, 100.0) as u8;
            let state = run.pages.get_mut(&page).expect("page");
            if text.trim().is_empty() {
                state.ocr = Some(PageOcr::Empty { confidence });
                continue;
            }
            state.agreement = state.layer.as_deref().and_then(|layer| agreement_percent(&text, layer));
            let clean = clean_percent(&text);
            let chars = text.chars().count();
            let name = format!("ocr-page-{page}.txt");
            let artifact = self
                .ctx
                .io
                .put_artifact(&self.ctx.artifact_key(&name), &name, "text/plain; charset=utf-8", text.into_bytes())
                .await?;
            state.ocr = Some(PageOcr::Text { obj_id: artifact.obj_id, chars, confidence, clean });
        }
        Ok(())
    }

    /// Runs `job` over `items` with at most `self.workers` in flight.
    async fn each<T, F, Fut, R>(&self, items: Vec<T>, job: F) -> Vec<R>
    where
        F: Fn(T) -> Fut,
        Fut: Future<Output = R>,
    {
        let queue = Mutex::new(items.into_iter());
        let results = Mutex::new(Vec::new());
        let worker = || async {
            loop {
                let next = queue.lock().unwrap().next();
                let Some(item) = next else { break };
                let result = job(item).await;
                results.lock().unwrap().push(result);
            }
        };
        join_all((0..self.workers).map(|_| worker()).collect()).await;
        results.into_inner().unwrap()
    }
}

fn is_fatal(err: &ThunkExecutionError) -> bool {
    matches!(err.code.as_str(), error_codes::CANCELLED | error_codes::TIMEOUT | error_codes::INPUT_UNAVAILABLE)
}

async fn join_all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut futures: Vec<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut out: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (slot, future) in out.iter_mut().zip(futures.iter_mut()) {
            if slot.is_none() {
                match future.as_mut().poll(cx) {
                    Poll::Ready(value) => *slot = Some(value),
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    out.into_iter().map(|v| v.expect("joined")).collect()
}

// ----- choosing the language ------------------------------------------------

struct LangChoice {
    lang: Option<String>,
    script: Option<String>,
    vertical: bool,
    basis: Value,
}

impl LangChoice {
    fn given(lang: &str, available: &BTreeSet<String>) -> Self {
        let missing: Vec<&str> = lang.split('+').filter(|l| !available.contains(*l)).collect();
        Self {
            lang: missing.is_empty().then(|| lang.to_string()),
            script: None,
            vertical: lang.split('+').any(|l| l.ends_with("_vert")),
            basis: json!({"given": lang, "missing": missing}),
        }
    }
}

/// tesseract model -> writing system, for scripts OSD may report.
fn osd_script_models(script: &str) -> &'static [&'static str] {
    match script {
        "Latin" => &["eng"],
        "Han" => &["chi_sim", "chi_tra"],
        "Japanese" | "Hiragana" | "Katakana" => &["jpn"],
        "Hangul" | "Korean" => &["kor"],
        "Cyrillic" => &["rus", "ukr"],
        "Arabic" => &["ara", "fas"],
        "Hebrew" => &["heb"],
        "Devanagari" => &["hin"],
        "Bengali" => &["ben"],
        "Thai" => &["tha"],
        "Greek" => &["ell"],
        _ => &[],
    }
}

fn script_of_model(model: &str) -> &'static str {
    match model {
        "eng" | "spa" | "fra" | "deu" | "por" | "ita" | "nld" | "pol" | "tur" | "vie" | "ind" | "msa" => "Latin",
        "chi_sim" | "chi_tra" => "Han",
        "jpn" => "Japanese",
        "kor" => "Hangul",
        "rus" | "ukr" => "Cyrillic",
        "ara" | "fas" => "Arabic",
        "heb" => "Hebrew",
        "hin" => "Devanagari",
        "ben" => "Bengali",
        "tha" => "Thai",
        "ell" => "Greek",
        _ => "Other",
    }
}

/// Alternatives of a script tried against the winner (Han: simplified /
/// traditional; Cyrillic: Russian / Ukrainian; Arabic: Arabic / Persian).
fn sibling_models(model: &str) -> &'static [&'static str] {
    match model {
        "chi_sim" => &["chi_tra"],
        "rus" => &["ukr"],
        "ara" => &["fas"],
        _ => &[],
    }
}

/// Confidence of a model over the sample pages, weighted by word count.
fn weighted_confidence(readings: &[Reading]) -> (f64, usize) {
    let mut sum = 0.0;
    let mut n = 0usize;
    for reading in readings {
        for word in reading.words.iter().filter(|w| w.conf >= 0.0) {
            sum += word.conf;
            n += 1;
        }
    }
    (if n == 0 { 0.0 } else { sum / n as f64 }, n)
}

async fn choose_language(
    tess: &Tesseract<'_>,
    run: &OcrRun,
    sample: &[u32],
    available: &BTreeSet<String>,
) -> Result<LangChoice, ThunkExecutionError> {
    let mut images = Vec::new();
    for page in sample {
        images.push(tess.image(*page, &run.pages[page].image_obj_id).await?);
    }
    let mut trials: BTreeMap<String, (f64, usize, Vec<Reading>)> = BTreeMap::new();
    let try_models = |models: Vec<String>| {
        let images = &images;
        async move {
            let jobs: Vec<(String, PathBuf)> =
                models.iter().flat_map(|m| images.iter().map(move |i| (m.clone(), i.clone()))).collect();
            tess.each(jobs, |(model, image)| async move {
                let reading = tess.run(&image, &model, "11").await;
                (model, reading)
            })
            .await
        }
    };
    let first: Vec<String> = SCRIPT_TRIALS.iter().map(|(m, _)| m.to_string()).filter(|m| available.contains(m)).collect();
    for (model, reading) in try_models(first).await {
        let reading = match reading {
            Ok(r) => r,
            Err(err) if is_fatal(&err) => return Err(err),
            Err(_) => continue,
        };
        trials.entry(model).or_insert_with(|| (0.0, 0, Vec::new())).2.push(reading);
    }
    let score = |trials: &mut BTreeMap<String, (f64, usize, Vec<Reading>)>| {
        for value in trials.values_mut() {
            let (conf, words) = weighted_confidence(&value.2);
            value.0 = conf;
            value.1 = words;
        }
    };
    let best_of = |trials: &BTreeMap<String, (f64, usize, Vec<Reading>)>| {
        trials
            .iter()
            .filter(|(_, v)| v.1 > 0)
            .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .map(|(m, v)| (m.clone(), v.0))
    };
    score(&mut trials);
    let mut basis = Map::new();
    let mut best = best_of(&trials);
    let mut osd_script = None;
    if best.as_ref().map_or(true, |(_, conf)| *conf < SCRIPT_CONF_MIN) {
        // None of the common scripts: OSD names the writing system.
        let mut votes: BTreeMap<String, f64> = BTreeMap::new();
        for image in &images {
            if let Some((script, conf)) = tess.osd(image).await {
                *votes.entry(script).or_default() += conf.max(0.1);
            }
        }
        basis.insert("osd".into(), json!(votes));
        osd_script = votes.iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|(s, _)| s.clone());
        if let Some(script) = osd_script.as_deref() {
            let extra: Vec<String> = osd_script_models(script)
                .iter()
                .map(|m| m.to_string())
                .filter(|m| available.contains(m) && !trials.contains_key(m))
                .collect();
            for (model, reading) in try_models(extra).await {
                if let Ok(reading) = reading {
                    trials.entry(model).or_insert_with(|| (0.0, 0, Vec::new())).2.push(reading);
                }
            }
            score(&mut trials);
            best = best_of(&trials);
        }
    }
    basis.insert(
        "trial".into(),
        Value::Object(trials.iter().map(|(m, v)| (m.clone(), json!([v.0.round() as i64, v.1]))).collect()),
    );
    let total_words: usize = trials.values().map(|v| v.1).max().unwrap_or(0);
    let Some((mut model, conf)) = best else {
        basis.insert("reason".into(), json!("no text in the sample pages"));
        let lang = available.contains("eng").then(|| "eng".to_string());
        return Ok(LangChoice { lang, script: Some("Latin".into()), vertical: false, basis: Value::Object(basis) });
    };
    if conf < MODEL_CONF_MIN && total_words >= 10 {
        let script = osd_script.unwrap_or_else(|| "unknown".to_string());
        basis.insert("reason".into(), json!(format!("no installed language data reads {script} text")));
        return Ok(LangChoice { lang: None, script: Some(script), vertical: false, basis: Value::Object(basis) });
    }
    // Within the script.
    let siblings: Vec<String> = sibling_models(&model)
        .iter()
        .map(|m| m.to_string())
        .filter(|m| available.contains(m) && !trials.contains_key(m))
        .collect();
    if !siblings.is_empty() {
        for (sibling, reading) in try_models(siblings).await {
            if let Ok(reading) = reading {
                trials.entry(sibling).or_insert_with(|| (0.0, 0, Vec::new())).2.push(reading);
            }
        }
        score(&mut trials);
        for sibling in sibling_models(&model) {
            if let Some(v) = trials.get(*sibling) {
                basis.insert(format!("sibling_{sibling}"), json!(v.0.round() as i64));
            }
        }
        let family: Vec<&str> = std::iter::once(model.as_str()).chain(sibling_models(&model).iter().copied()).collect();
        if let Some(winner) = family
            .iter()
            .filter_map(|m| trials.get(*m).map(|v| (*m, v.0)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
        {
            model = winner.0.to_string();
        }
    }
    let script = script_of_model(&model).to_string();
    let mut vertical = false;
    if model == "eng" {
        let text: String = trials["eng"].2.iter().map(|r| assemble(&r.words, WORD_CONF_MIN)).collect::<Vec<_>>().join("\n");
        let (language, scores) = latin_language(&text);
        basis.insert("words".into(), json!(scores));
        if language != "eng" {
            if available.contains(language) {
                model = language.to_string();
            } else {
                basis.insert("wanted".into(), json!(language));
            }
        }
    } else if matches!(script.as_str(), "Han" | "Japanese" | "Hangul") {
        let votes: Vec<bool> = trials[&model].2.iter().filter_map(|r| is_vertical(&r.words)).collect();
        let vertical_votes = votes.iter().filter(|v| **v).count();
        basis.insert("vertical_votes".into(), json!([vertical_votes, votes.len()]));
        let vert_model = format!("{model}_vert");
        if vertical_votes * 2 > votes.len() && available.contains(&vert_model) {
            vertical = true;
            model = vert_model;
        }
    }
    let lang = if vertical || model == "eng" || !available.contains("eng") {
        model
    } else {
        format!("{model}+eng")
    };
    Ok(LangChoice { lang: Some(lang), script: Some(script), vertical, basis: Value::Object(basis) })
}

/// `auto`, or language data names joined by `+` (`chi_sim+eng`).
pub fn valid_lang(lang: &str) -> bool {
    lang == LANG_AUTO
        || (!lang.is_empty()
            && lang.split('+').all(|part| {
                part.starts_with(|c: char| c.is_ascii_lowercase())
                    && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            }))
}

pub fn parse_osd(text: &str) -> Option<(String, f64)> {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string()))
    };
    let script = field("Script:")?;
    let conf = field("Script confidence:").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    Some((script, conf))
}

/// Common words of the Latin-script languages with language data in ext-core.
const LATIN_WORDS: [(&str, &str); 12] = [
    ("eng", "the and of to in is that for it with as was on are by this be from or an which at have not"),
    ("spa", "de la que el en y los del se las por un para con una es al lo como mas pero sus le"),
    ("fra", "de la le et les des en un du une que est pour qui dans par pas sur au avec ce il"),
    ("deu", "der die und in den von zu das mit sich des auf fur ist im dem nicht ein eine als auch"),
    ("por", "de a o que e do da em um para com nao uma os no se na por mais as dos"),
    ("ita", "di e il la che per un in del non una sono le si con della al da gli"),
    ("nld", "de en van het een in is dat op te zijn met voor niet aan er die worden ook als"),
    ("pol", "i w na nie z sie do to ze jest o jak ale po co tak za od przez oraz"),
    ("tur", "ve bir bu da de icin ile olarak cok daha gibi olan en kadar ama sonra her ki"),
    ("vie", "va cua la co trong duoc cac cho khong nhung mot nguoi voi nay da khi de den tu"),
    ("ind", "yang dan di ini itu dengan untuk dari dalam tidak ke akan pada juga ada adalah oleh atau bisa karena"),
    ("msa", "yang dan di ini itu dengan untuk dari dalam tidak ke akan pada juga ada adalah oleh atau boleh kerana"),
];

/// Latin letters with diacritics -> base letter, enough for the word lists.
fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ạ' | 'ả' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ' | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'đ' | 'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' | 'ẹ' | 'ẻ' | 'ẽ' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ı' | 'ị' | 'ỉ' | 'ĩ' => 'i',
        'ł' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ő' | 'ơ' | 'ọ' | 'ỏ' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' => 'o',
        'ś' | 'š' | 'ş' => 's',
        'ù' | 'ú' | 'û' | 'ü' | 'ű' | 'ư' | 'ụ' | 'ủ' | 'ũ' | 'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự' => 'u',
        'ý' | 'ÿ' | 'ỳ' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        'ğ' => 'g',
        other => other,
    }
}

/// The language of Latin-script text by its common words; `eng` unless
/// another language is clearly ahead.
pub fn latin_language(text: &str) -> (&'static str, BTreeMap<&'static str, u32>) {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase().chars().map(fold).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    let mut scores = BTreeMap::new();
    if words.len() < 20 {
        return ("eng", scores);
    }
    for (lang, list) in LATIN_WORDS {
        let set: BTreeSet<&str> = list.split_whitespace().collect();
        let hits = words.iter().filter(|w| set.contains(w.as_str())).count();
        scores.insert(lang, (hits * 1000 / words.len()) as u32);
    }
    let eng = scores["eng"];
    let (best, score) = scores
        .iter()
        .filter(|(l, _)| **l != "eng")
        .max_by_key(|(l, s)| (**s, std::cmp::Reverse(**l == "msa")))
        .map(|(l, s)| (*l, *s))
        .unwrap_or(("eng", 0));
    if score >= 80 && score * 2 > eng * 3 {
        (best, scores)
    } else {
        ("eng", scores)
    }
}

// ----- TSV, text and metrics -------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub block: u32,
    pub par: u32,
    pub line: u32,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub conf: f64,
    pub text: String,
}

/// Word rows (level 5) of `tesseract ... tsv`.
pub fn parse_tsv(tsv: &str) -> Vec<Word> {
    tsv.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 12 || f[0] != "5" || f[11].trim().is_empty() {
                return None;
            }
            let n = |i: usize| f[i].trim().parse::<i64>().ok();
            Some(Word {
                block: n(2)? as u32,
                par: n(3)? as u32,
                line: n(4)? as u32,
                left: n(6)? as i32,
                top: n(7)? as i32,
                width: n(8)? as i32,
                height: n(9)? as i32,
                conf: f[10].trim().parse().ok()?,
                text: f[11].trim().to_string(),
            })
        })
        .collect()
}

pub fn mean_confidence(words: &[Word]) -> f64 {
    let confs: Vec<f64> = words.iter().map(|w| w.conf).filter(|c| *c >= 0.0).collect();
    if confs.is_empty() {
        0.0
    } else {
        confs.iter().sum::<f64>() / confs.len() as f64
    }
}

pub fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'
        | '\u{ac00}'..='\u{d7af}' | '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}')
}

/// CJK punctuation stands where Latin text has a space.
fn is_cjk_punct(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{303f}' | '\u{ff01}'..='\u{ff0f}' | '\u{ff1a}'..='\u{ff20}' | '\u{ff3b}'..='\u{ff40}' | '\u{ff5b}'..='\u{ff65}')
}

fn tight(c: char) -> bool {
    is_cjk(c) || is_cjk_punct(c)
}

/// Lines in reading order (blocks / paragraphs separated by a blank line),
/// leaving out words below `conf_min`; CJK runs are joined without spaces.
pub fn assemble(words: &[Word], conf_min: f64) -> String {
    let mut out = String::new();
    let mut key: Option<(u32, u32, u32)> = None;
    for word in words.iter().filter(|w| w.conf >= conf_min) {
        let k = (word.block, word.par, word.line);
        match key {
            Some(prev) if prev == k => {
                let joins = out.chars().last().is_some_and(tight) && word.text.chars().next().is_some_and(tight);
                if !joins {
                    out.push(' ');
                }
            }
            Some(prev) => out.push_str(if (prev.0, prev.1) == (k.0, k.1) { "\n" } else { "\n\n" }),
            None => {}
        }
        out.push_str(&word.text);
        key = Some(k);
    }
    out
}

fn split_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || is_cjk_punct(c)).filter(|w| !w.is_empty())
}

fn is_clean(word: &str) -> bool {
    let mut core: &str = word;
    if let Some(first) = core.chars().next().filter(|c| !c.is_alphanumeric()) {
        core = &core[first.len_utf8()..];
    }
    if let Some(last) = core.chars().last().filter(|c| !c.is_alphanumeric()) {
        core = &core[..core.len() - last.len_utf8()];
    }
    !core.is_empty() && core.chars().all(char::is_alphabetic)
}

/// Percentage of words made of letters only; `None` without words.
pub fn clean_percent(text: &str) -> Option<u8> {
    let (mut clean, mut total) = (0usize, 0usize);
    for word in split_words(text) {
        total += 1;
        clean += usize::from(is_clean(word));
    }
    (total > 0).then(|| (clean * 100 / total) as u8)
}

/// Distinct words of two letters or more, and CJK character pairs.
fn tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for word in split_words(&text.to_lowercase()) {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        if word.is_empty() {
            continue;
        }
        if word.chars().any(is_cjk) {
            let mut run: Vec<char> = Vec::new();
            for c in word.chars().chain(std::iter::once(' ')) {
                if is_cjk(c) {
                    run.push(c);
                    continue;
                }
                match run.len() {
                    0 => {}
                    1 => {
                        out.insert(run[0].to_string());
                    }
                    _ => out.extend(run.windows(2).map(|p| p.iter().collect::<String>())),
                }
                run.clear();
            }
        } else if word.chars().count() >= 2 {
            out.insert(word.to_string());
        }
    }
    out
}

/// Percentage of the OCR words that the text layer also has.
pub fn agreement_percent(ocr: &str, layer: &str) -> Option<u8> {
    let ocr = tokens(ocr);
    if ocr.is_empty() {
        return None;
    }
    let layer = tokens(layer);
    Some((ocr.iter().filter(|t| layer.contains(*t)).count() * 100 / ocr.len()) as u8)
}

fn median(values: impl Iterator<Item = u8>) -> Option<u8> {
    let mut v: Vec<u8> = values.collect();
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// Whether the single CJK characters of a page stack in columns: the nearest
/// glyph below is as close as the nearest one to the right. Horizontal text
/// has its next line several glyph heights down. `None`: too few glyphs.
pub fn is_vertical(words: &[Word]) -> Option<bool> {
    let glyphs: Vec<&Word> = words
        .iter()
        .filter(|w| w.conf >= 30.0 && w.text.chars().count() == 1 && w.text.chars().all(is_cjk) && w.height > 0)
        .collect();
    if glyphs.len() < 20 {
        return None;
    }
    let mut heights: Vec<i32> = glyphs.iter().map(|g| g.height).collect();
    heights.sort_unstable();
    let h = f64::from(heights[heights.len() / 2]);
    let overlap = |a0: i32, a1: i32, b0: i32, b1: i32| f64::from(a1.min(b1) - a0.max(b0));
    let mut right = Vec::new();
    let mut down = Vec::new();
    for g in &glyphs {
        let (gr, gb) = (g.left + g.width, g.top + g.height);
        let r = glyphs
            .iter()
            .filter(|o| f64::from(o.left) >= f64::from(gr) - 0.2 * h && overlap(g.top, gb, o.top, o.top + o.height) > 0.5 * h)
            .map(|o| f64::from(o.left - gr) / h)
            .fold(f64::INFINITY, f64::min);
        let d = glyphs
            .iter()
            .filter(|o| f64::from(o.top) >= f64::from(gb) - 0.2 * h && overlap(g.left, gr, o.left, o.left + o.width) > 0.5 * h)
            .map(|o| f64::from(o.top - gb) / h)
            .fold(f64::INFINITY, f64::min);
        if r.is_finite() {
            right.push(r);
        }
        if d.is_finite() {
            down.push(d);
        }
    }
    if right.len() < 10 || down.len() < 10 {
        return None;
    }
    let mid = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    Some(mid(&mut down) < 2.0 * mid(&mut right).max(0.05))
}

/// `n` items spread over `items` (first, last and evenly between).
pub fn spread<T: Copy>(items: &[T], n: usize) -> Vec<T> {
    let n = n.min(items.len());
    (0..n).map(|i| items[i * items.len() / n]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(block: u32, line: u32, conf: f64, text: &str) -> Word {
        Word { block, par: 1, line, left: 0, top: 0, width: 10, height: 10, conf, text: text.to_string() }
    }

    #[test]
    fn tsv_rows_become_lines_without_unsure_words() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t10\t10\t40\t12\t96.5\tFAMOUS\n\
                   5\t1\t1\t1\t1\t2\t60\t10\t40\t12\t91.0\tBUILDINGS\n\
                   5\t1\t1\t1\t2\t1\t10\t30\t40\t12\t12.0\t~~\n\
                   5\t1\t2\t1\t1\t1\t10\t80\t20\t12\t88.0\t故宫\n\
                   5\t1\t2\t1\t1\t2\t30\t80\t20\t12\t90.0\t博物院\n";
        let words = parse_tsv(tsv);
        assert_eq!(words.len(), 5);
        assert_eq!(assemble(&words, WORD_CONF_MIN), "FAMOUS BUILDINGS\n\n故宫博物院");
        assert!((mean_confidence(&words) - 75.5).abs() < 0.01);
    }

    #[test]
    fn a_garbled_layer_disagrees_with_what_the_page_shows() {
        // dk10000.pdf page 21: a text layer from an OCR of the wrong language.
        let layer = "See al,so: 01 Parts o 「 the body 03 Musel 巳 a 「1 d s.keleto 同 19 1 llness and inju叩201 \
                     Visitl 们 g the doctor ,21 丁he 七 ospitai 4.3 HEAD 4功 ：＼之二 。！ brain ①s,inus ①palate \
                     ① p,harynx l 二①t,01ngue ①叩iglottis. 二 一①t叫阳 ①throat @.) Adam's app1le";
        let ocr = "See also 01 Parts of the body 03 Muscles and skeleton 19 Illness and injury 20 Visiting the doctor \
                   21 The hospital HEAD brain sinus palate pharynx tongue epiglottis throat Adam's apple";
        assert!(clean_percent(layer).unwrap() < SUSPICIOUS_CLEAN);
        assert!(clean_percent(ocr).unwrap() > clean_percent(layer).unwrap() + 20);
        assert!(agreement_percent(ocr, layer).unwrap() < GARBLED_AGREEMENT);
        let native = "Proxmox VE uses a REST like API. The concept is to use resources as objects.";
        assert!(clean_percent(native).unwrap() >= 80);
        assert_eq!(agreement_percent(native, native), Some(100));
        let zh = "为 BuckyOS 提供统一的系统级内容查看能力。";
        assert_eq!(agreement_percent("提供统一的系统级内容查看能力 。", zh), Some(100));
        assert_eq!(clean_percent("   "), None);
    }

    #[test]
    fn latin_text_is_told_apart_by_its_common_words() {
        let fr = "La cuisine française est célèbre dans le monde entier pour la qualité de ses produits et la diversité de ses recettes régionales. Chaque région possède ses spécialités et le repas se compose d'une entrée.";
        assert_eq!(latin_language(fr).0, "fra");
        let de = "Die erste deutsche Eisenbahnstrecke wurde im Dezember zwischen Nürnberg und Fürth eröffnet. Die Lokomotive mit dem Namen Adler zog die Wagen über eine Strecke von etwa sechs Kilometern und das war neu.";
        assert_eq!(latin_language(de).0, "deu");
        let en = "Before the invention of the printing press, books in Europe were copied by hand, usually by monks working in monasteries. A single book could take months to produce, which made written knowledge rare.";
        assert_eq!(latin_language(en).0, "eng");
        assert_eq!(latin_language("too short").0, "eng");
    }

    #[test]
    fn stacked_glyphs_read_as_vertical_text() {
        let grid = |step_x: i32, step_y: i32| -> Vec<Word> {
            let mut words = Vec::new();
            for col in 0..6 {
                for row in 0..8 {
                    words.push(Word {
                        block: 1,
                        par: 1,
                        line: row,
                        left: 1000 - col * step_x,
                        top: 100 + row as i32 * step_y,
                        width: 40,
                        height: 40,
                        conf: 90.0,
                        text: "字".to_string(),
                    });
                }
            }
            words
        };
        // Columns 70 px apart, glyphs 44 px apart inside a column.
        assert_eq!(is_vertical(&grid(70, 44)), Some(true));
        // Lines: glyphs 42 px apart in a line, lines 120 px apart.
        let horizontal: Vec<Word> = grid(42, 120);
        assert_eq!(is_vertical(&horizontal), Some(false));
        assert_eq!(is_vertical(&[word(1, 1, 90.0, "字")]), None);
    }

    #[test]
    fn osd_and_spread() {
        let osd = "Page number: 0\nOrientation in degrees: 0\nRotate: 0\nOrientation confidence: 2.1\nScript: Arabic\nScript confidence: 268.30\n";
        assert_eq!(parse_osd(osd), Some(("Arabic".to_string(), 268.3)));
        assert_eq!(parse_osd("Too few characters. Skipping this page"), None);
        assert_eq!(spread(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 3), vec![1, 4, 7]);
        assert_eq!(spread(&[1, 2], 5), vec![1, 2]);
        assert_eq!(median([70u8, 20, 40].into_iter()), Some(40));
        assert!(valid_lang("auto") && valid_lang("chi_sim+eng") && valid_lang("jpn_vert"));
        assert!(!valid_lang("-c") && !valid_lang("eng+") && !valid_lang("") && !valid_lang("../x"));
    }
}
