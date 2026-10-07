//! The program runner (许愿格 §9.3): Deno runs `lib/run.js` → `program/main.js` on the stage's
//! snapshot directory. Reads only the run directory, writes only `output/`, may use the network
//! (recorded as external data); bounded in time and memory. The model's `run_program`, a feedback
//! round and "re-run the program" all run through here.

use super::StageState;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

pub struct ProgramOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub results: Option<Value>,
    pub timed_out: bool,
}

impl ProgramOutput {
    /// The end of the program's own output (what a person debugging would look at).
    pub fn tail(&self) -> String {
        let take = |s: &str| {
            let chars: Vec<char> = s.chars().collect();
            chars[chars.len().saturating_sub(3000)..].iter().collect::<String>()
        };
        let mut out = String::new();
        if !self.stdout.trim().is_empty() {
            out.push_str(&format!("stdout:\n{}\n", take(&self.stdout)));
        }
        if !self.stderr.trim().is_empty() {
            out.push_str(&format!("stderr:\n{}\n", take(&self.stderr)));
        }
        out
    }
}

/// The Deno to use: the configured one, BuckyOS's own (`libexec/buckyos-tool/runtime/deno`), then `PATH`.
pub fn find_deno(configured: &str) -> Option<PathBuf> {
    if !configured.trim().is_empty() {
        let p = PathBuf::from(configured);
        return p.is_file().then_some(p);
    }
    if let Ok(p) = std::env::var("AIWS_DENO") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let bundled = buckyos_kit::get_buckyos_root_dir().join("libexec/buckyos-tool/runtime/deno");
    if bundled.is_file() {
        return Some(bundled);
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join("deno")).find(|p| p.is_file()))
}

async fn read_all(r: Option<impl tokio::io::AsyncRead + Unpin>) -> String {
    let mut buf = Vec::new();
    if let Some(mut r) = r {
        let _ = r.read_to_end(&mut buf).await;
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Run the program in `state.workdir`.
pub async fn run(state: &Arc<StageState>) -> Result<ProgramOutput, String> {
    let cfg = &state.runtime.config;
    let deno = find_deno(&cfg.deno).ok_or_else(|| "宿主没有可用的 Deno（配置 wish.deno、AIWS_DENO，或安装 buckyos-tool 运行时）".to_string())?;
    run_in(&deno, &state.workdir, state.program_env(), cfg.program_timeout_secs, cfg.program_memory_mb, Some(state)).await
}

pub async fn run_in(deno: &Path, workdir: &Path, env: Vec<(String, String)>, timeout_secs: u64, memory_mb: u32, state: Option<&Arc<StageState>>) -> Result<ProgramOutput, String> {
    if !workdir.join("program/main.js").is_file() {
        return Err("program/main.js 不存在：先用 write_file 写出程序".into());
    }
    let results = workdir.join("output/.aiws/results.json");
    let _ = std::fs::remove_file(&results);
    std::fs::create_dir_all(workdir.join("output")).map_err(|e| e.to_string())?;
    let mut cmd = tokio::process::Command::new(deno);
    cmd.arg("run")
        .arg("--no-prompt")
        .arg("--quiet")
        .arg("--no-remote")
        .arg(format!("--allow-read={}", workdir.display()))
        .arg(format!("--allow-write={}", workdir.join("output").display()))
        .arg("--allow-net")
        .arg("--allow-env=AIWS_HOST,AIWS_TOKEN")
        .arg(format!("--v8-flags=--max-old-space-size={memory_mb}"))
        .arg("lib/run.js")
        .current_dir(workdir)
        .env_clear()
        .env("NO_COLOR", "1")
        .env("DENO_NO_UPDATE_CHECK", "1")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_else(|_| workdir.display().to_string()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("启动 Deno 失败：{e}"))?;
    if let Some(s) = state {
        s.set_child(child.id());
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_task = tokio::spawn(read_all(stdout));
    let err_task = tokio::spawn(read_all(stderr));
    let waited = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs.max(1)), child.wait()).await;
    let (code, timed_out) = match waited {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(format!("等待程序失败：{e}")),
        Err(_) => {
            let _ = child.kill().await;
            (None, true)
        }
    };
    if let Some(s) = state {
        s.set_child(None);
    }
    let stdout = out_task.await.unwrap_or_default();
    let stderr = err_task.await.unwrap_or_default();
    let results = std::fs::read_to_string(&results).ok().and_then(|t| serde_json::from_str(&t).ok());
    Ok(ProgramOutput { code, stdout, stderr, results, timed_out })
}
