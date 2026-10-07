#![allow(dead_code)]
//! A wish test harness: the service in process with a scripted model (see wish.rs, quality.rs).

use aiworkspace_server::auth::StaticTokens;
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{build_router, AppState, Limits};
use aiworkspace_store::Service;
use async_trait::async_trait;
use buckyos_api::{AiContent, AiResponse, AiToolCall, AiToolResultContent};
use llm_context::{LLMComputeError, LlmClient, LlmInferenceRequest};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Script = Box<dyn Fn(&Turn) -> AiResponse + Send + Sync>;

/// What the scripted model sees of one request.
pub struct Turn {
    pub system: String,
    pub user: String,
    /// tool results so far, in order (text)
    pub results: Vec<String>,
}

pub struct ScriptedModel {
    pub analyze: Mutex<Script>,
    pub execute: Mutex<Script>,
    pub calls: AtomicUsize,
    pub map_calls: AtomicUsize,
    pub delay_ms: AtomicU64,
}

pub fn call(name: &str, args: Value) -> AiResponse {
    static N: AtomicU64 = AtomicU64::new(0);
    let args: HashMap<String, Value> = args.as_object().cloned().unwrap_or_default().into_iter().collect();
    AiResponse::from_parts(None, vec![AiToolCall { name: name.into(), args, call_id: format!("c{}", N.fetch_add(1, Ordering::SeqCst)) }], vec![])
}

pub fn text(t: &str) -> AiResponse {
    AiResponse::text(t)
}

#[async_trait]
impl LlmClient for ScriptedModel {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let delay = self.delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let mut turn = Turn { system: String::new(), user: String::new(), results: Vec::new() };
        for m in &req.messages {
            for c in &m.content {
                match c {
                    AiContent::Text { text } if format!("{:?}", m.role).contains("System") => turn.system.push_str(text),
                    AiContent::Text { text } if format!("{:?}", m.role).contains("User") => turn.user.push_str(text),
                    AiContent::ToolResult { content, .. } => turn.results.push(
                        content.iter().filter_map(|c| match c { AiToolResultContent::Text { text } => Some(text.clone()), _ => None }).collect::<Vec<_>>().join("\n"),
                    ),
                    _ => {}
                }
            }
        }
        if turn.system.contains("逐项判断器") {
            self.map_calls.fetch_add(1, Ordering::SeqCst);
            // per-item judgement: 正面 when the text says 好
            let items: Vec<Value> = turn.user.split("输入项（JSON 数组，共").nth(1).and_then(|t| t.split_once('\n')).and_then(|(_, rest)| rest.split("\n\n").next()).and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
            let out: Vec<Value> = items.iter().map(|i| json!(if i.as_str().unwrap_or("").contains('好') { "正面" } else { "负面" })).collect();
            return Ok(text(&serde_json::to_string(&out).unwrap()));
        }
        let script = if turn.system.contains("的**分析阶段**") { self.analyze.lock().unwrap() } else { self.execute.lock().unwrap() };
        Ok(script(&turn))
    }
}

pub struct Srv {
    pub base: String,
    pub http: reqwest::Client,
    pub model: Arc<ScriptedModel>,
    pub _dir: tempfile::TempDir,
    pub ws: String,
}

pub fn find_deno() -> String {
    aiworkspace_server::wish::runner::find_deno("").expect("deno is needed for wish tests").display().to_string()
}

impl Srv {
    pub async fn start(dir: tempfile::TempDir, model: Arc<ScriptedModel>) -> Srv {
        let svc = Service::open(&dir.path().join("data")).unwrap();
        let tokens: StaticTokens = serde_json::from_value(json!({ "tokens": { "tok-alice": { "principal": "alice" } } })).unwrap();
        let mut cfg = WishConfig::default();
        cfg.llm_override = Some(model.clone());
        cfg.deno = find_deno();
        cfg.program_timeout_secs = 60;
        let state = AppState::new(svc, Arc::new(tokens), Limits::default(), None, cfg);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        *state.wish.host_base.lock().unwrap() = Some(format!("http://{addr}/kapi/aiworkspace"));
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await.unwrap() });
        Srv { base: format!("http://{addr}/kapi/aiworkspace"), http: reqwest::Client::new(), model, _dir: dir, ws: String::new() }
    }

    pub async fn rpc(&self, method: &str, mut params: Value) -> Value {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        if !self.ws.is_empty() && params.get("workspace_id").is_none() {
            params["workspace_id"] = json!(self.ws);
        }
        let v: Value = self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [SEQ.fetch_add(1, Ordering::SeqCst), "tok-alice"] })).send().await.unwrap().json().await.unwrap();
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    pub async fn commit(&self, ops: Value) -> Value {
        static K: AtomicU64 = AtomicU64::new(0);
        let info = self.rpc("ws.get_info", json!({})).await;
        let r = self.rpc("doc.commit", json!({ "protocol_version": "0.1", "epoch": info["epoch"], "idempotency_key": format!("k{}", K.fetch_add(1, Ordering::SeqCst)), "session_id": "s1", "operations": ops })).await;
        assert_eq!(r["status"], "accepted", "{r}");
        r
    }

    /// Start a stage and wait until it leaves the working states.
    pub async fn run(&self, program: &str, params: Value) -> Value {
        static K: AtomicU64 = AtomicU64::new(0);
        let start = self.rpc("proc.start", json!({ "program": program, "params": params, "idempotency_key": format!("run{}", K.fetch_add(1, Ordering::SeqCst)) })).await;
        assert_eq!(start["ok"], true, "{start}");
        self.wait(start["run_id"].as_str().unwrap()).await
    }

    pub async fn wait(&self, run_id: &str) -> Value {
        for _ in 0..600 {
            let r = self.rpc("proc.get", json!({ "run_id": run_id })).await;
            if !matches!(r["state"].as_str(), Some("queued" | "snapshotting" | "running" | "validating")) {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("run {run_id} did not settle");
    }

    pub async fn apply(&self, run: &Value, choices: Value) -> Value {
        let id = run["run_id"].as_str().unwrap();
        let preview = self.rpc("proc.get", json!({ "run_id": id, "choices": choices })).await;
        assert_eq!(preview["preview"]["ready"], true, "{}", preview["preview"]);
        let r = self.rpc("proc.apply", json!({ "run_id": id, "plan_digest": preview["preview"]["plan_digest"], "session_id": "s1" })).await;
        assert_eq!(r["applied"]["status"], "accepted", "{r}");
        r
    }

    pub async fn wish(&self) -> Value {
        self.wish_of("wish-q3").await
    }

    pub async fn wish_of(&self, id: &str) -> Value {
        self.rpc("doc.read", json!({ "entity_id": id })).await["content"]["payload"].clone()
    }
}

pub fn handle(map: &str, needle: &str) -> String {
    let line = map.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle} not in map:\n{map}"));
    let i = line.find('@').unwrap();
    line[i..].split(|c: char| !(c.is_ascii_alphanumeric() || c == '@')).next().unwrap().to_string()
}

/// Scripts by default answer the stage with a plain sentence.
pub fn model() -> Arc<ScriptedModel> {
    Arc::new(ScriptedModel { analyze: Mutex::new(Box::new(|_| text("没有可做的。"))), execute: Mutex::new(Box::new(|_| text("没有可做的。"))), calls: AtomicUsize::new(0), map_calls: AtomicUsize::new(0), delay_ms: AtomicU64::new(0) })
}

/// The tool results so far contain `needle`.
pub fn seen(t: &Turn, needle: &str) -> bool {
    t.results.iter().any(|r| r.contains(needle))
}

/// Run an async test body on a runtime with large stacks (debug builds of the xllm loop and of big
/// `json!` test bodies need more than the default 2 MiB).
pub fn run_test<F>(f: impl FnOnce() -> F + Send + 'static)
where
    F: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).thread_stack_size(16 * 1024 * 1024).enable_all().build().unwrap();
            rt.block_on(f());
        })
        .unwrap()
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}
