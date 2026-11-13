#![allow(dead_code)]
//! A wish test harness: the service in process, and a test executor playing the browser — it claims each run, drives
//! a scripted model over the stage's tools (`proc.tool`), runs the macro with the local Deno exactly as every executor
//! does (`lib/run.js --bridge`), answers the macro's bridge (`sql` through `proc.tool`, AI from the script with the
//! `llm_cache`), hands the run in (`proc.program`) and reports (see wish.rs, quality.rs, wish_control.rs).

#[path = "nfs.rs"]
pub mod nfs;

use aiworkspace_core::canonical::{canonical_json, sha256_hex};
use aiworkspace_server::auth::StaticTokens;
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{build_router, AppState, Limits};
use aiworkspace_store::Service;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// What the scripted model answers one turn: a tool call, or a final text.
pub enum Reply {
    Call(String, Value),
    Text(String),
}

pub type Script = Box<dyn Fn(&Turn) -> Reply + Send + Sync>;

/// What the scripted model sees of one request.
pub struct Turn {
    pub system: String,
    pub user: String,
    /// tool results so far, in order (text)
    pub results: Vec<String>,
}

pub struct ScriptedModel {
    pub analyze: Mutex<Script>,
    /// The coding stage.
    pub execute: Mutex<Script>,
    /// Turns of the stage loops (analyze, code).
    pub calls: AtomicUsize,
    /// Batches `llm.map` asked the model.
    pub map_calls: AtomicUsize,
    /// Inferences `ai.complete` asked the model (not the cached ones).
    pub complete_calls: AtomicUsize,
    pub delay_ms: AtomicU64,
    /// Every model call is refused by the AI service with this sub-code (AI_QUOTA_EXHAUSTED…).
    pub refuse: Mutex<Option<&'static str>>,
    /// `ai.complete`: (system, prompt) → the answer.
    pub complete: Mutex<Box<dyn Fn(&str, &str) -> String + Send + Sync>>,
    /// The budget of the last stage the executor claimed.
    pub budget: Mutex<Value>,
}

pub fn call(name: &str, args: Value) -> Reply {
    Reply::Call(name.into(), args)
}

pub fn text(t: &str) -> Reply {
    Reply::Text(t.into())
}

/// The per-item judge of `llm.map`: 正面 when the text says 好.
fn judge(item: &Value) -> Value {
    json!(if item.as_str().unwrap_or("").contains('好') { "正面" } else { "负面" })
}

pub struct Srv {
    pub base: String,
    pub http: reqwest::Client,
    pub model: Arc<ScriptedModel>,
    pub _dir: tempfile::TempDir,
    pub ws: String,
    pub nfs: nfs::Nfs,
}

/// The Deno a test executor runs macros with: `AIWS_DENO`, BuckyOS's own, then `PATH`.
pub fn find_deno() -> String {
    if let Ok(p) = std::env::var("AIWS_DENO") {
        return p;
    }
    let name = format!("deno{}", std::env::consts::EXE_SUFFIX);
    let bundled = buckyos_kit::get_buckyos_root_dir().join("libexec/buckyos-tool/runtime").join(&name);
    if bundled.is_file() {
        return bundled.display().to_string();
    }
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(&name)).find(|p| p.is_file())).expect("deno is needed for wish tests").display().to_string()
}

impl Srv {
    pub async fn start(dir: tempfile::TempDir, model: Arc<ScriptedModel>) -> Srv {
        Self::start_with(dir, model, json!({ "tok-alice": { "principal": "alice" } })).await
    }

    /// `tokens`: `{ token: { principal, class? } }`; `rpc` calls as `tok-alice`.
    pub async fn start_with(dir: tempfile::TempDir, model: Arc<ScriptedModel>, tokens: Value) -> Srv {
        Self::start_with_limits(dir, model, tokens, Limits::default()).await
    }

    pub async fn start_with_limits(dir: tempfile::TempDir, model: Arc<ScriptedModel>, tokens: Value, limits: Limits) -> Srv {
        let nfs = nfs::Nfs::start(dir.path()).await;
        let svc = Service::open(&dir.path().join("data"), nfs.objects().await).unwrap();
        let tokens: StaticTokens = serde_json::from_value(json!({ "tokens": tokens })).unwrap();
        let mut cfg = WishConfig::default();
        cfg.program_timeout_secs = 60;
        let state = AppState::new(svc, Arc::new(tokens), limits, None, cfg);
        let _ = state.ndm_gateway.set(nfs.base.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, build_router(state)).await.unwrap() });
        Srv { base: format!("http://{addr}/kapi/aiworkspace"), http: reqwest::Client::new(), model, _dir: dir, ws: String::new(), nfs }
    }

    /// The live records of a local table as `doc.read` gives a record, through SQL (`_record`).
    pub async fn records(&self, table: &str) -> Vec<Value> {
        let q = self.rpc("doc.query", json!({ "sql": "SELECT _record FROM t ORDER BY _id", "tables": { "t": table } })).await;
        q["rows"].as_array().unwrap_or_else(|| panic!("{q}")).iter().map(|r| serde_json::from_str::<Value>(r[0].as_str().unwrap()).unwrap()).collect()
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

    /// The raw response of `method` called as alice (errors included).
    pub async fn rpc_raw(&self, method: &str, params: Value) -> Value {
        self.call_as("tok-alice", method, params).await
    }

    /// The raw response of `method` called with `token` (errors included).
    pub async fn call_as(&self, token: &str, method: &str, mut params: Value) -> Value {
        static SEQ: AtomicU64 = AtomicU64::new(1_000_000);
        if !self.ws.is_empty() && params.get("workspace_id").is_none() {
            params["workspace_id"] = json!(self.ws);
        }
        self.http.post(&self.base).json(&json!({ "method": method, "params": params, "sys": [SEQ.fetch_add(1, Ordering::SeqCst), token] })).send().await.unwrap().json().await.unwrap()
    }

    pub async fn wait_as(&self, token: &str, run_id: &str) -> Value {
        for _ in 0..600 {
            let r = self.call_as(token, "proc.get", json!({ "run_id": run_id })).await["result"].clone();
            if !matches!(r["state"].as_str(), Some("queued" | "snapshotting" | "running" | "validating")) {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("run {run_id} did not settle");
    }

    pub async fn commit(&self, ops: Value) -> Value {
        static K: AtomicU64 = AtomicU64::new(0);
        let info = self.rpc("ws.get_info", json!({})).await;
        let r = self.rpc("doc.commit", json!({ "protocol_version": "1.0", "epoch": info["epoch"], "idempotency_key": format!("k{}", K.fetch_add(1, Ordering::SeqCst)), "session_id": "s1", "operations": ops })).await;
        assert_eq!(r["status"], "accepted", "{r}");
        r
    }

    /// The three-state result of a Commit, whatever it is.
    pub async fn commit_raw(&self, ops: Value) -> Value {
        static K: AtomicU64 = AtomicU64::new(0);
        let info = self.rpc("ws.get_info", json!({})).await;
        self.rpc("doc.commit", json!({ "protocol_version": "1.0", "epoch": info["epoch"], "idempotency_key": format!("raw{}", K.fetch_add(1, Ordering::SeqCst)), "session_id": "s1", "operations": ops })).await
    }

    /// `GET /asset/<ws>/<object_id>` as alice.
    pub async fn asset_bytes(&self, ws: &str, object_id: &str) -> Vec<u8> {
        let r = self.http.get(format!("{}/asset/{ws}/{object_id}", self.base)).bearer_auth("tok-alice").send().await.unwrap();
        assert!(r.status().is_success(), "asset {object_id}: {}", r.status());
        r.bytes().await.unwrap().to_vec()
    }

    /// Start a stage as alice, execute it as her browser would, and wait until it leaves the working states. A start
    /// the service refuses comes back as `{ state: "refused", error }`.
    pub async fn run(&self, program: &str, params: Value) -> Value {
        self.run_as("tok-alice", program, params).await
    }

    pub async fn run_as(&self, token: &str, program: &str, params: Value) -> Value {
        static K: AtomicU64 = AtomicU64::new(0);
        let start = self.call_as(token, "proc.start", json!({ "program": program, "params": params, "idempotency_key": format!("run{}", K.fetch_add(1, Ordering::SeqCst)) })).await;
        if let Some(e) = start["result"]["error"].as_object().or(start["error"].as_object()) {
            return json!({ "state": "refused", "error": e });
        }
        let start = start["result"].clone();
        assert_eq!(start["ok"], true, "{start}");
        let run_id = start["run_id"].as_str().unwrap().to_string();
        if let Some(claim) = start["claim_token"].as_str() {
            self.execute(token, &run_id, claim).await;
        }
        self.wait_as(token, &run_id).await
    }

    /// The test executor for one run (the browser's local executor): claim, the stage's work, report.
    pub async fn execute(&self, token: &str, run_id: &str, claim_token: &str) {
        let claimed = self.call_as(token, "proc.claim", json!({ "run_id": run_id, "claim_token": claim_token })).await;
        let claimed = claimed["result"].clone();
        if claimed.get("error").is_some() || claimed.get("run_token").is_none() {
            return;
        }
        let attempt = claimed["attempt"].as_u64().unwrap_or(1);
        let ex = Exec { srv: self, rt: claimed["run_token"].as_str().unwrap().to_string(), run_id: run_id.to_string(), attempt, manifest: claimed["stage"]["manifest"].clone() };
        let stage = claimed["stage"].clone();
        *self.model.budget.lock().unwrap() = stage["budget"].clone();
        let outcome = if stage["kind"] == json!("program") { ex.run_macro(stage["source"].as_str().unwrap_or("")).await.map(|_| ()) } else { ex.model_loop(&stage).await };
        match outcome {
            Ok(()) => {
                ex.call("proc.report", json!({ "finished": true })).await;
            }
            Err(Value::Null) => {}
            Err(e) => {
                ex.call("proc.report", json!({ "error": e })).await;
            }
        }
    }

    pub async fn wait(&self, run_id: &str) -> Value {
        self.wait_as("tok-alice", run_id).await
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

/// Scripts by default answer the stage with a plain sentence; `ai.complete` answers with a sentence about its prompt.
pub fn model() -> Arc<ScriptedModel> {
    Arc::new(ScriptedModel {
        analyze: Mutex::new(Box::new(|_| text("没有可做的。"))),
        execute: Mutex::new(Box::new(|_| text("没有可做的。"))),
        calls: AtomicUsize::new(0),
        map_calls: AtomicUsize::new(0),
        complete_calls: AtomicUsize::new(0),
        delay_ms: AtomicU64::new(0),
        refuse: Mutex::new(None),
        complete: Mutex::new(Box::new(|_, prompt| format!("解读：{prompt}"))),
        budget: Mutex::new(Value::Null),
    })
}

/// One claimed attempt of the test executor.
pub struct Exec<'a> {
    pub srv: &'a Srv,
    pub rt: String,
    pub run_id: String,
    pub attempt: u64,
    pub manifest: Value,
}

fn key_of(v: &Value) -> String {
    sha256_hex(canonical_json(v).unwrap_or_default().as_bytes())
}

impl Exec<'_> {
    /// A call of the executor protocol with the run token; the raw kRPC answer.
    pub async fn call(&self, method: &str, mut params: Value) -> Value {
        params["workspace_id"] = json!(self.srv.ws);
        params["run_id"] = json!(self.run_id);
        params["attempt"] = json!(self.attempt);
        self.srv.call_as(&self.rt, method, params).await
    }

    /// `proc.tool`; `None` when the service revoked the attempt (cancelled, superseded).
    pub async fn tool(&self, name: &str, args: Value) -> Option<Value> {
        let r = self.call("proc.tool", json!({ "name": name, "args": args })).await;
        if r["result"]["error"].is_object() || r["error"].is_object() {
            return None;
        }
        Some(r["result"].clone())
    }

    async fn model_loop(&self, stage: &Value) -> Result<(), Value> {
        let m = &self.srv.model;
        let analyze = stage["stage"] == json!("analyze");
        let mut turn = Turn { system: stage["system"].as_str().unwrap_or("").into(), user: stage["user"].as_str().unwrap_or("").into(), results: Vec::new() };
        let mut source: Option<String> = stage["source"].as_str().map(str::to_string);
        for _ in 0..stage["iterations"].as_u64().unwrap_or(60) {
            if let Some(code) = *m.refuse.lock().unwrap() {
                return Err(json!({ "code": code, "detail": "refused by the AI service" }));
            }
            let delay = m.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            m.calls.fetch_add(1, Ordering::SeqCst);
            let reply = if analyze { (m.analyze.lock().unwrap())(&turn) } else { (m.execute.lock().unwrap())(&turn) };
            let (name, args) = match reply {
                Reply::Text(_) => return Ok(()),
                Reply::Call(n, a) => (n, a),
            };
            let answer = match name.as_str() {
                // the executor's own file tool: what run_program runs next
                "write_file" => {
                    source = args["content"].as_str().map(str::to_string);
                    "program/main.js written".to_string()
                }
                "run_program" => {
                    let src = args["source"].as_str().map(str::to_string).or_else(|| source.clone()).unwrap_or_default();
                    source = Some(src.clone());
                    match self.run_macro(&src).await {
                        Ok(text) => text,
                        Err(Value::Null) => return Err(Value::Null),
                        Err(e) => return Err(e),
                    }
                }
                _ => match self.tool(&name, args).await {
                    Some(r) => r["text"].as_str().unwrap_or("").to_string(),
                    None => return Err(Value::Null),
                },
            };
            turn.results.push(answer);
        }
        Err(json!({ "code": "ITERATIONS_EXHAUSTED", "detail": "the model loop ran out of iterations" }))
    }

    /// Run `source` on the stage's snapshot with the local Deno and hand it in; the service's answer text.
    pub async fn run_macro(&self, source: &str) -> Result<String, Value> {
        let files = self.call("proc.files", json!({})).await;
        if let Some(e) = files["result"]["error"].as_object().or(files["error"].as_object()) {
            return Err(if e.get("code") == Some(&json!("RUN_REVOKED")) { Value::Null } else { Value::Null });
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (path, f) in files["result"]["files"].as_object().into_iter().flatten() {
            let target = root.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            if let Some(t) = f["text"].as_str() {
                std::fs::write(&target, t).unwrap();
            } else if let Some(b) = f["base64"].as_str() {
                use base64::Engine;
                std::fs::write(&target, base64::engine::general_purpose::STANDARD.decode(b).unwrap()).unwrap();
            }
        }
        std::fs::create_dir_all(root.join("program")).unwrap();
        std::fs::create_dir_all(root.join("output")).unwrap();
        std::fs::write(root.join("program/main.js"), source).unwrap();
        let deno_dir = tempfile::tempdir().unwrap();
        let mut child = tokio::process::Command::new(find_deno())
            .args(["run", "--no-prompt", "--quiet", "--no-remote", &format!("--allow-read={}", root.display()), &format!("--allow-write={}", root.join("output").display()), "lib/run.js", "--bridge"])
            .current_dir(root)
            .env_clear()
            .env("DENO_DIR", deno_dir.path())
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("deno");
        let mut stdin = child.stdin.take().unwrap();
        let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let mut stderr = child.stderr.take().unwrap();
        let err_task = tokio::spawn(async move {
            let mut s = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut s).await;
            s
        });
        while let Ok(Some(line)) = lines.next_line().await {
            let Some(msg) = line.strip_prefix("\u{1e}aiws ").and_then(|l| serde_json::from_str::<Value>(l).ok()) else { continue };
            let body = self.bridge(msg["op"].as_str().unwrap_or(""), &msg["req"]).await;
            let out = format!("{}\n", json!({ "id": msg["id"], "body": body }));
            let _ = stdin.write_all(out.as_bytes()).await;
        }
        drop(stdin);
        let _ = tokio::time::timeout(Duration::from_secs(60), child.wait()).await;
        let log = err_task.await.unwrap_or_default();
        let results: Value = std::fs::read_to_string(root.join("output/.aiws/results.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let mut out_files = Map::new();
        fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Map<String, Value>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                if rel.starts_with("output/.aiws") {
                    continue;
                }
                if p.is_dir() {
                    walk(base, &p, out);
                } else if let Ok(t) = std::fs::read_to_string(&p) {
                    out.insert(rel, json!(t));
                }
            }
        }
        walk(root, &root.join("output"), &mut out_files);
        let ran = self.call("proc.program", json!({ "source": source, "results": results, "log": log, "files": out_files })).await;
        if ran["result"]["error"].is_object() || ran["error"].is_object() {
            return Err(Value::Null);
        }
        Ok(ran["result"]["text"].as_str().unwrap_or("").to_string())
    }

    /// The executor's side of the macro's bridge (what host.js does in the browser and the hosted runner).
    async fn bridge(&self, op: &str, req: &Value) -> Value {
        let m = &self.srv.model;
        let declared = if self.manifest.is_object() { self.manifest.clone() } else { req["declare"].clone() };
        let refused = *m.refuse.lock().unwrap();
        match op {
            "sql" => {
                let mut args = json!({ "query": req["query"], "params": req["params"] });
                if !self.manifest.is_object() {
                    args["declare"] = req["declare"].clone();
                }
                match self.tool("sql", args).await {
                    Some(r) if r["status"] == json!("ok") => r["sql"].clone(),
                    Some(r) => json!({ "error": r["text"], "code": r["code"] }),
                    None => json!({ "error": "revoked" }),
                }
            }
            "ai.complete" => {
                if !declared["ai"]["complete"].is_object() {
                    return json!({ "error": "declare ai.complete", "code": "AI_NOT_DECLARED" });
                }
                let key = key_of(&json!({ "kind": "complete", "system": req["system"], "prompt": req["prompt"], "messages": req["messages"], "schema": req["schema"], "model": req["model"] }));
                if let Some(hit) = self.tool("llm_cache", json!({ "get": [key] })).await.and_then(|r| r["hits"].get(&key).cloned()) {
                    let mut v = hit;
                    v["cached"] = json!(true);
                    v["tokens"] = json!(0);
                    return v;
                }
                if let Some(code) = refused {
                    return json!({ "error": "refused by the AI service", "code": code });
                }
                m.complete_calls.fetch_add(1, Ordering::SeqCst);
                let answer = (m.complete.lock().unwrap())(req["system"].as_str().unwrap_or(""), req["prompt"].as_str().unwrap_or(""));
                let out = if req["schema"].is_object() { json!({ "value": serde_json::from_str::<Value>(&answer).unwrap_or(json!(answer)) }) } else { json!({ "text": answer }) };
                self.tool("llm_cache", json!({ "put": [{ "key": key, "value": out }] })).await;
                let mut v = out;
                v["cached"] = json!(false);
                v["tokens"] = json!(10);
                v
            }
            "llm.map" => {
                if !declared["ai"]["map"].is_object() {
                    return json!({ "error": "declare ai.map", "code": "AI_NOT_DECLARED" });
                }
                let items = req["items"].as_array().cloned().unwrap_or_default();
                let budget = m.budget.lock().unwrap()["map_model"].clone();
                let keys: Vec<String> = items.iter().map(|x| key_of(&json!({ "m": budget, "i": req["instruction"], "s": req["schema"], "x": x }))).collect();
                let hits = self.tool("llm_cache", json!({ "get": keys })).await.map(|r| r["hits"].clone()).unwrap_or(json!({}));
                let mut results = Vec::new();
                let (mut cached, mut put) = (0, Vec::new());
                let mut missed = false;
                for (i, x) in items.iter().enumerate() {
                    match hits.get(&keys[i]) {
                        Some(v) => {
                            cached += 1;
                            results.push(json!({ "value": v }));
                        }
                        None => {
                            missed = true;
                            let v = judge(x);
                            put.push(json!({ "key": keys[i], "value": v }));
                            results.push(json!({ "value": v }));
                        }
                    }
                }
                if missed {
                    if let Some(code) = refused {
                        return json!({ "error": "refused by the AI service", "code": code });
                    }
                    m.map_calls.fetch_add(1, Ordering::SeqCst);
                    self.tool("llm_cache", json!({ "put": put })).await;
                }
                json!({ "results": results, "calls": if missed { 1 } else { 0 }, "cached": cached })
            }
            "zone.fetch" => {
                let url = req["url"].as_str().unwrap_or("");
                if url.starts_with('/') {
                    json!({ "status": 200, "headers": {}, "text": "zone", "url": format!("https://zone.test{url}") })
                } else {
                    json!({ "error": format!("{url} is outside this Zone"), "code": "ZONE_ONLY" })
                }
            }
            _ => json!({ "error": format!("unknown bridge call {op}") }),
        }
    }
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
