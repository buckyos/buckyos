//! What the Agent Service itself runs in the agent's browser
//! (`browser.background_run`, [Agent Browser Protocol] §7.1): a script in a
//! tab of its own, started as an internal subcommand of `agent-browser`.
//! Such a run takes no lease and is nobody's turn at the browser: the user
//! and the agent go on in their tabs. One run works in the browser at a
//! time; the others wait their turn.
//!
//! [Agent Browser Protocol]: ../../../../../doc/opendan/protocol/Agent%20Browser%20Protocol.md

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;

use super::{BrowserError, BrowserModule};

pub const ENV_AGENT_BROWSER: &str = "OPENDAN_AGENT_BROWSER";
/// The group of the tab a run opens: the module's own, shown to nobody.
const GROUP: &str = "_job";
/// What `agent-browser` gets beyond the script's own time, to start and to
/// close its tab.
const COMMAND_MARGIN: Duration = Duration::from_secs(10);
/// How much of the command's stderr is kept: its last line is why it failed.
const STDERR_TAIL: usize = 16 * 1024;
/// What a script may return, as JSON.
const MAX_RESULT: u64 = 8 << 20;

/// The `agent-browser` command: `OPENDAN_AGENT_BROWSER`, else the one of the
/// node's tool directories (where `chromium` is).
pub fn find_agent_browser() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(ENV_AGENT_BROWSER).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let dirs = std::env::join_paths(libopendan::baseline::managed_tool_dirs()).ok()?;
    libopendan::baseline::resolve_locally(&["agent-browser"], &[], Some(&dirs), &[])
        .remove("agent-browser")
        .map(PathBuf::from)
}

#[derive(Default)]
pub(super) struct Background {
    /// One run works in the browser at a time; the others wait their turn.
    turn: tokio::sync::Mutex<()>,
}

impl Background {
    /// A run works in the browser: it is not idle.
    pub(super) fn working(&self) -> bool {
        self.turn.try_lock().is_err()
    }
}

/// What the command keeps of the service's environment: enough to run.
/// Nothing the service holds (its session tokens, where its kRPC is) reaches
/// the script: it is whatever the agent was led to write.
const KEPT_ENV: &[&str] = if cfg!(windows) {
    &["PATH", "PATHEXT", "ComSpec", "SystemRoot", "SystemDrive", "windir", "ProgramData", "ProgramFiles", "ProgramFiles(x86)"]
} else {
    &["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ"]
};

/// The environment of the command: [`KEPT_ENV`], and a home and a temp
/// directory of its own in `dir` (they go with it when the run ended).
fn command_env(dir: &Path) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> =
        KEPT_ENV.iter().filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value))).collect();
    let (home, tmp) = (dir.join("home"), dir.join("tmp"));
    let mut set = |name: &str, value: &Path| env.push((name.into(), value.into()));
    if cfg!(windows) {
        set("USERPROFILE", &home);
        set("APPDATA", &home.join("AppData").join("Roaming"));
        set("LOCALAPPDATA", &home.join("AppData").join("Local"));
        set("TEMP", &tmp);
        set("TMP", &tmp);
    } else {
        set("HOME", &home);
        set("TMPDIR", &tmp);
    }
    env
}

/// The reason a command gave for failing: the last line it wrote to stderr.
fn last_line(said: &[u8]) -> Option<String> {
    String::from_utf8_lossy(said).lines().rev().map(str::trim).find(|line| !line.is_empty()).map(str::to_string)
}

/// Run `cmd` to its end, or end it — with whatever it started — when it
/// takes longer than `limit`.
async fn run_command(mut cmd: tokio::process::Command, limit: Duration) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(buckyos_api::windows_background_creation_flags());
    let mut child = cmd.spawn().map_err(|e| format!("agent-browser was not started: {e}"))?;
    #[cfg(windows)]
    let job = super::process::job::Job::of(&child).ok();
    #[cfg(unix)]
    let pid = child.id();
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let mut said = tokio::spawn(async move {
        let (mut tail, mut chunk) = (Vec::new(), [0u8; 4096]);
        while let Ok(read @ 1..) = stderr.read(&mut chunk).await {
            tail.extend_from_slice(&chunk[..read]);
            if tail.len() > 2 * STDERR_TAIL {
                tail.drain(..tail.len() - STDERR_TAIL);
            }
        }
        tail
    });
    let ended = tokio::time::timeout(limit, child.wait()).await;
    // Nothing it started outlives it.
    #[cfg(unix)]
    if let Some(pid) = pid {
        let _ = tokio::task::spawn_blocking(move || super::process::reap_group(pid)).await;
    }
    #[cfg(windows)]
    if let Some(job) = &job {
        job.terminate();
    }
    let status = match ended {
        Ok(status) => status.map_err(|e| format!("agent-browser: {e}"))?,
        Err(_) => {
            let _ = child.kill().await;
            said.abort();
            return Err(format!("the script did not end within {} seconds and was ended", limit.as_secs()));
        }
    };
    if status.success() {
        return Ok(());
    }
    // Whoever still holds the pipe is not waited for.
    let said = match tokio::time::timeout(Duration::from_secs(2), &mut said).await {
        Ok(said) => said.unwrap_or_default(),
        Err(_) => {
            said.abort();
            Vec::new()
        }
    };
    Err(last_line(&said).unwrap_or_else(|| format!("agent-browser ended with {status}")))
}

impl BrowserModule {
    fn agent_browser(&self) -> Result<PathBuf, BrowserError> {
        let found = match &self.tuning.agent_browser {
            Some(path) => path.is_file().then(|| path.clone()),
            None => find_agent_browser(),
        };
        found.ok_or_else(|| {
            BrowserError::unavailable("no `agent-browser` on this node (it comes with the ext-core extension tools): it is what runs a script in a tab of the browser")
        })
    }

    /// Run the command. It opens one tab, right after it connected: until
    /// that tab is there, a tab without an opener belongs to `_job`. What is
    /// left in `_job` when it ended (it was ended, or did not close its
    /// tab) is closed.
    async fn run_in_tab(&self, cmd: tokio::process::Command, limit: Duration) -> Result<(), String> {
        let known: Vec<String> = self.group(GROUP).tabs.into_iter().map(|tab| tab.tab_id).collect();
        let mut changes = self.changes();
        let mut expected = Some(self.expect_tabs(GROUP));
        let running = run_command(cmd, limit);
        tokio::pin!(running);
        let ran = loop {
            tokio::select! {
                ran = &mut running => break ran,
                changed = changes.changed(), if expected.is_some() => {
                    if changed.is_err() || self.group(GROUP).tabs.iter().any(|tab| !known.contains(&tab.tab_id)) {
                        expected = None;
                    }
                }
            }
        };
        drop(expected);
        for tab in self.group(GROUP).tabs {
            let _ = self.close_tab(&tab.tab_id).await;
        }
        ran
    }

    /// `browser.background_run`: run the script `script_path` in a
    /// background tab of the browser (opened at `url`) and return what it
    /// returns. The browser is started for it when it does not run; started
    /// for this only, it does not open its saved tabs and is stopped soon
    /// after ([`BrowserModule::ensure_for`]).
    pub async fn background_run(&self, script_path: &str, url: Option<String>, args: Option<Value>, timeout_s: u64) -> Result<Value, BrowserError> {
        let script = PathBuf::from(script_path);
        if !script.is_absolute() || script.extension().and_then(|e| e.to_str()) != Some("mjs") || !script.is_file() {
            return Err(BrowserError::invalid(format!("`{script_path}` is no script to run: it is the absolute path of an existing .mjs file")));
        }
        self.usable()?;
        let agent_browser = self.agent_browser()?;
        // A caller that goes away does not leave the run half done: it ends by itself, its tab closed.
        let run = tokio::spawn(self.me().background_turn(agent_browser, script, url, args, timeout_s));
        run.await.unwrap_or_else(|e| Err(BrowserError::new("other", format!("the background run stopped unexpectedly: {e}"))))
    }

    async fn background_turn(self: Arc<Self>, agent_browser: PathBuf, script: PathBuf, url: Option<String>, args: Option<Value>, timeout_s: u64) -> Result<Value, BrowserError> {
        let wait = self.tuning.background_queue_wait;
        let Ok(_turn) = tokio::time::timeout(wait, self.background.turn.lock()).await else {
            return Err(BrowserError::new("other", format!("the browser was busy with other background work for {} seconds", wait.as_secs())));
        };
        let dir = self.dirs.root.join("job");
        let ran = self.background_in(&dir, &agent_browser, &script, url.as_deref(), args.as_ref(), timeout_s).await;
        let _ = tokio::fs::remove_dir_all(&dir).await;
        match &ran {
            Ok(_) => log::info!("browser: the background run of {} ended", script.display()),
            Err(e) => log::warn!("browser: the background run of {} failed: {e}", script.display()),
        }
        ran
    }

    async fn background_in(&self, dir: &Path, agent_browser: &Path, script: &Path, url: Option<&str>, args: Option<&Value>, timeout_s: u64) -> Result<Value, BrowserError> {
        let other = |why: String| BrowserError::new("other", why);
        let _ = tokio::fs::remove_dir_all(dir).await;
        for part in [&dir.join("home"), &dir.join("tmp")] {
            tokio::fs::create_dir_all(part).await.map_err(|e| other(format!("create {}: {e}", part.display())))?;
        }
        let (out, args_file) = (dir.join("result.json"), dir.join("args.json"));
        if let Some(args) = args {
            tokio::fs::write(&args_file, args.to_string()).await.map_err(|e| other(format!("write {}: {e}", args_file.display())))?;
        }

        self.ensure_for(true).await?;
        let port = self.shared().port.ok_or_else(|| BrowserError::unavailable("the browser stopped"))?;
        let mut cmd = tokio::process::Command::new(agent_browser);
        cmd.arg("background-run").arg("--cdp").arg(format!("http://127.0.0.1:{port}")).arg("--script").arg(script).arg("--out").arg(&out);
        if let Some(url) = url {
            cmd.arg(format!("--url={url}"));
        }
        if args.is_some() {
            cmd.arg("--args").arg(&args_file);
        }
        cmd.arg("--timeout").arg(timeout_s.to_string());
        cmd.env_clear().envs(command_env(dir)).current_dir(dir);
        self.run_in_tab(cmd, Duration::from_secs(timeout_s) + COMMAND_MARGIN).await.map_err(other)?;

        let size = tokio::fs::metadata(&out).await.map_err(|e| other(format!("agent-browser left no result: {e}")))?.len();
        if size > MAX_RESULT {
            return Err(other(format!("the script returned {size} bytes of JSON: more than the {MAX_RESULT} a background run returns")));
        }
        let data = tokio::fs::read(&out).await.map_err(|e| other(format!("agent-browser left no result: {e}")))?;
        serde_json::from_slice(&data).map_err(|e| other(format!("what the script returned is not JSON: {e}")))
    }
}

#[cfg(all(test, unix))]
#[path = "../../tests/fake_browser/mod.rs"]
mod fake_browser;

#[cfg(all(test, unix))]
mod tests {
    use std::time::Instant;

    use serde_json::json;

    use super::fake_browser::{self, FakeBrowser};
    use super::*;
    use crate::browser::{rpc, BrowserDeps, BrowserDirs, Tuning, DEFAULT_GROUP, ENV_BROWSER_RPC};
    use crate::config::BrowserSection;

    const AGENT: &str = "did:bns:jarvis.bob";
    const OWNER: &str = "did:bns:bob";

    /// Stands in for `agent-browser background-run`: what it "reads off the
    /// page" is its arguments and the script. Files beside it tell it to
    /// wait (`hold`), to fail (`fail`, `auth`), never to end (`hang`) or to
    /// return too much (`big`); it leaves its command line (`calls`) and
    /// environment (`env`) there.
    const AGENT_BROWSER: &str = r#"#!/bin/sh
here="$(dirname "$0")"
echo "$*" >> "$here/calls"
echo "$$" > "$here/pid"
env > "$here/env"
[ "$1" = background-run ] || { echo "agent-browser: unknown command: $1" >&2; exit 2; }
shift
url=
args=null
while [ $# -gt 0 ]; do
  case "$1" in
    --cdp) shift 2 ;;
    --url=*) url="${1#--url=}"; shift ;;
    --script) script="$2"; shift 2 ;;
    --args) args="$(cat "$2")"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --timeout) shift 2 ;;
    *) echo "agent-browser: unknown option: $1" >&2; exit 2 ;;
  esac
done
[ -f "$script" ] || { echo "agent-browser: no such script: $script" >&2; exit 1; }
while [ -e "$here/hold" ]; do sleep 0.05; done
if [ -e "$here/hang" ]; then sleep 300 & sleep 300; fi
if [ -e "$here/fail" ]; then
  echo "Error: 页面改版了" >&2
  echo "    at fetch.mjs:1:1" >&2
  echo "agent-browser: background run failed: 页面改版了" >&2
  exit 1
fi
if [ -e "$here/auth" ]; then
  echo "Error: the session ran out" >&2
  echo "    at fetch.mjs:1:1" >&2
  echo "auth: the session ran out" >&2
  exit 1
fi
if [ -e "$here/big" ]; then head -c 9000000 /dev/zero | tr '\0' 1 > "$out"; exit 0; fi
printf '{"url":"%s","script":"%s","args":%s}' "$url" "$(cat "$script")" "$args" > "$out"
"#;

    async fn until(what: &str, mut ok: impl FnMut() -> bool) {
        let started = Instant::now();
        while !ok() {
            assert!(started.elapsed() < Duration::from_secs(30), "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn deps(root: &Path) -> BrowserDeps {
        BrowserDeps {
            agent: Arc::new(libopendan::FsAgentStateClient::open(root, AGENT, None, None).unwrap()),
            supervisor: None,
            owner_did: Some(OWNER.to_string()),
            agent_root: root.to_path_buf(),
        }
    }

    fn section(idle_stop_secs: u64, background_idle_stop_secs: u64) -> BrowserSection {
        BrowserSection { enabled: true, idle_stop_secs, background_idle_stop_secs, display: crate::browser::display::DisplayMode::Headless, ..Default::default() }
    }

    struct Bench {
        tmp: tempfile::TempDir,
        fake: Arc<FakeBrowser>,
        module: Arc<BrowserModule>,
    }

    impl Bench {
        async fn new(cfg: BrowserSection, tune: impl FnOnce(&mut Tuning)) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let tmp = tempfile::tempdir().unwrap();
            let fake = FakeBrowser::start(&tmp.path().join("browser")).await;
            let agent_browser = tmp.path().join("tool").join("agent-browser");
            std::fs::create_dir_all(agent_browser.parent().unwrap()).unwrap();
            std::fs::write(&agent_browser, AGENT_BROWSER).unwrap();
            std::fs::set_permissions(&agent_browser, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(tmp.path().join("fetch.mjs"), "v1").unwrap();
            Self { module: Self::service(tmp.path(), cfg, tune), tmp, fake }
        }

        /// A service on the bench's directories (another one: the service was started again).
        fn service(tmp: &Path, cfg: BrowserSection, tune: impl FnOnce(&mut Tuning)) -> Arc<BrowserModule> {
            let mut tuning = Tuning {
                chromium: Some(fake_browser::chromium()),
                dirs: Some(BrowserDirs::at(tmp.join("browser"))),
                tick: Duration::from_millis(50),
                agent_browser: Some(tmp.join("tool").join("agent-browser")),
                ..Default::default()
            };
            tune(&mut tuning);
            BrowserModule::with_tuning(cfg, deps(&tmp.join("agent_root")), tuning)
        }

        fn script(&self) -> String {
            self.tmp.path().join("fetch.mjs").display().to_string()
        }

        fn tool(&self, name: &str) -> PathBuf {
            self.tmp.path().join("tool").join(name)
        }

        /// Tell the fake `agent-browser` how to behave.
        fn flag(&self, name: &str, on: bool) {
            if on {
                std::fs::write(self.tool(name), "").unwrap();
            } else {
                let _ = std::fs::remove_file(self.tool(name));
            }
        }

        /// The command lines `agent-browser` was started with.
        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.tool("calls")).unwrap_or_default().lines().map(str::to_string).collect()
        }

        fn groups_file(&self) -> String {
            std::fs::read_to_string(self.tmp.path().join("agent_root/state/browser/groups.json")).unwrap_or_default()
        }
    }

    async fn run(module: &Arc<BrowserModule>, params: Value) -> Result<Value, BrowserError> {
        rpc::call(module, OWNER, Some("the-callers-token"), "browser.background_run", &params).await
    }

    /// A run that is asked for and goes on while the test looks.
    fn running(module: &Arc<BrowserModule>, params: Value) -> tokio::task::JoinHandle<Result<Value, BrowserError>> {
        let module = module.clone();
        tokio::spawn(async move { run(&module, params).await })
    }

    #[test]
    fn the_reason_a_command_failed_is_its_last_line() {
        assert_eq!(last_line(b"Error: x\n    at fetch.mjs:1:1\nagent-browser: background run failed: x\n\n").as_deref(), Some("agent-browser: background run failed: x"));
        assert_eq!(last_line(b" \n"), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_background_run_runs_the_script_in_a_tab_of_its_own_and_returns_what_it_returns() {
        let b = Bench::new(section(600, 600), |_| {}).await;
        let m = &b.module;
        // What the service itself holds.
        std::env::set_var(ENV_BROWSER_RPC, "http://127.0.0.1:9/kapi/jarvis");
        std::env::set_var("BUCKYOS_APPCLIENT_SESSION_TOKEN", "the-services-own-token");
        b.flag("hold", true);
        let asked = running(m, json!({ "script_path": b.script(), "url": "https://list.example/?page=2", "args": { "limit": 3, "tags": ["新闻"] }, "timeout_s": 5 }));
        until("the command runs", || b.calls().len() == 1).await;
        let pid = b.fake.pid().expect("the run started the browser");

        // The tab the command opens is the module's own; a tab somebody else opens meanwhile is theirs.
        let tab = b.fake.open("about:blank", None);
        until("the command's tab is seen", || m.tab(&tab).is_some()).await;
        assert_eq!(m.tab(&tab).unwrap().group_id, GROUP);
        until("only that tab was expected", || m.shared().expected.is_empty()).await;
        let others = b.fake.open("https://other.example/", None);
        until("the other tab is seen", || m.tab(&others).is_some()).await;
        assert_eq!(m.tab(&others).unwrap().group_id, DEFAULT_GROUP);
        // It is nobody's turn at the browser: no lease, the user operates, and the status shows the user's tabs only.
        let status = m.status().await;
        assert_eq!(status["groups"].as_array().unwrap().len(), 1);
        let group = &status["groups"][0];
        assert_eq!((&group["group_id"], &group["controller"], &group["lease"], group["tabs"].as_array().unwrap().len()), (&json!("default"), &json!("user"), &Value::Null, 1));

        b.fake.close(&tab);
        b.flag("hold", false);
        let answered = asked.await.unwrap().unwrap();
        assert_eq!(answered, json!({ "data": { "url": "https://list.example/?page=2", "script": "v1", "args": { "limit": 3, "tags": ["新闻"] } } }));
        assert_eq!(b.fake.pid(), Some(pid));

        // The command got the browser's endpoint, the script, the address and the arguments.
        let work = m.dirs().root.join("job");
        assert_eq!(
            b.calls(),
            vec![format!(
                "background-run --cdp http://127.0.0.1:{} --script {} --out {} --url=https://list.example/?page=2 --args {} --timeout 5",
                b.fake.port,
                b.script(),
                work.join("result.json").display(),
                work.join("args.json").display()
            )]
        );
        // Of the service's environment it got what it needs to run: not the service's token, nor its address.
        let env = std::fs::read_to_string(b.tool("env")).unwrap();
        std::env::remove_var(ENV_BROWSER_RPC);
        std::env::remove_var("BUCKYOS_APPCLIENT_SESSION_TOKEN");
        assert!(!env.contains("the-services-own-token") && !env.contains("the-callers-token") && !env.contains("BUCKYOS_") && !env.contains("OPENDAN_"), "{env}");
        let names: Vec<&str> = env.lines().filter_map(|line| line.split_once('=')).map(|(name, _)| name).collect();
        // `PWD`, `OLDPWD`, `SHLVL` and `_` are the shell's own.
        let kept = ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "HOME", "TMPDIR", "PWD", "OLDPWD", "SHLVL", "_"];
        assert!(names.iter().all(|name| kept.contains(name)), "{names:?}");
        for line in [format!("PATH={}", std::env::var("PATH").unwrap()), format!("HOME={}", work.join("home").display()), format!("TMPDIR={}", work.join("tmp").display())] {
            assert!(env.lines().any(|l| l == line), "{line} in {env}");
        }
        assert!(!work.exists(), "what the run worked in is gone");

        // Without an address and arguments, for the time a run gets when none is given.
        let answered = run(m, json!({ "script_path": b.script(), "url": null, "args": null })).await.unwrap();
        assert_eq!(answered, json!({ "data": { "url": "", "script": "v1", "args": null } }));
        assert!(b.calls()[1].ends_with(&format!("--script {} --out {} --timeout 60", b.script(), work.join("result.json").display())), "{:?}", b.calls());
        m.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_background_run_that_fails_says_why() {
        let b = Bench::new(section(600, 600), |_| {}).await;
        let m = &b.module;

        // What is no script to run.
        std::fs::write(b.tmp.path().join("fetch.js"), "v1").unwrap();
        std::fs::create_dir_all(b.tmp.path().join("dir.mjs")).unwrap();
        let near = |name: &str| b.tmp.path().join(name).display().to_string();
        for (why, script_path) in [
            ("a path that is not absolute", "fetch.mjs".to_string()),
            ("a script that is not there", near("nowhere.mjs")),
            ("a script that is no .mjs", near("fetch.js")),
            ("a directory", near("dir.mjs")),
        ] {
            let refused = run(m, json!({ "script_path": script_path })).await.unwrap_err();
            assert_eq!(refused.code, "invalid_argument", "{why}");
            assert!(refused.message.contains(&script_path), "{}", refused.message);
        }
        for params in [json!({}), json!({ "script_path": "" }), json!({ "script_path": b.script(), "timeout_s": 0 }), json!({ "script_path": b.script(), "timeout_s": 301 }), json!({ "script_path": b.script(), "timeout_s": "soon" }), json!({ "script_path": b.script(), "url": 7 })] {
            assert_eq!(run(m, params.clone()).await.unwrap_err().code, "invalid_argument", "{params}");
        }
        assert!(b.calls().is_empty() && b.fake.pid().is_none(), "nothing was started");

        // The script fails: the reason is the command's last line.
        b.flag("fail", true);
        let failed = run(m, json!({ "script_path": b.script() })).await.unwrap_err();
        assert_eq!((failed.code, failed.message.as_str()), ("other", "agent-browser: background run failed: 页面改版了"));
        b.flag("fail", false);
        // The script finds the user has to sign in: the caller is told in the form it knows.
        b.flag("auth", true);
        let failed = run(m, json!({ "script_path": b.script(), "url": "https://list.example/" })).await.unwrap_err();
        assert_eq!((failed.code, failed.message.as_str()), ("other", "auth: the session ran out"));
        b.flag("auth", false);
        // The script returns more than a run returns.
        b.flag("big", true);
        let failed = run(m, json!({ "script_path": b.script() })).await.unwrap_err();
        assert_eq!(failed.code, "other");
        assert!(failed.message.starts_with("the script returned 9000000 bytes of JSON"), "{}", failed.message);
        b.flag("big", false);
        // The next run is none the worse for them.
        assert_eq!(run(m, json!({ "script_path": b.script() })).await.unwrap()["data"]["script"], "v1");
        assert!(!m.dirs().root.join("job").exists());

        // A node without `agent-browser` runs nothing.
        std::fs::remove_file(b.tool("agent-browser")).unwrap();
        let none = run(m, json!({ "script_path": b.script() })).await.unwrap_err();
        assert_eq!((none.code, none.unavailable_code), ("browser_unavailable", None));
        m.shutdown().await;

        // Nor does an agent whose browser is switched off.
        let off = Bench::new(BrowserSection { enabled: false, ..section(600, 600) }, |_| {}).await;
        let refused = run(&off.module, json!({ "script_path": off.script() })).await.unwrap_err();
        assert_eq!((refused.code, refused.unavailable_code), ("browser_unavailable", Some("disabled")));
        assert!(off.calls().is_empty() && off.fake.pid().is_none());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_background_run_that_does_not_end_is_ended_with_what_it_started() {
        use crate::browser::process;
        let b = Bench::new(section(600, 600), |_| {}).await;
        let m = &b.module;
        b.flag("hang", true);
        let asked = running(m, json!({ "script_path": b.script(), "timeout_s": 1 }));
        until("the command started what it starts", || {
            std::fs::read_to_string(b.tool("pid")).is_ok_and(|pid| process::group_pids(pid.trim().parse().unwrap()).len() == 3)
        })
        .await;
        let pid: u32 = std::fs::read_to_string(b.tool("pid")).unwrap().trim().parse().unwrap();
        let tab = b.fake.open("https://list.example/", None);
        until("the command's tab is seen", || m.tab(&tab).is_some()).await;

        let failed = asked.await.unwrap().unwrap_err();
        assert_eq!((failed.code, failed.message.as_str()), ("other", "the script did not end within 11 seconds and was ended"));
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists() && process::group_pids(pid).is_empty(), "nothing of the command is left");
        // The tab it left behind was closed.
        assert!(m.tab(&tab).is_none() && b.fake.targets().is_empty(), "{:?}", b.fake.targets());
        assert!(!m.dirs().root.join("job").exists());
        m.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn background_runs_take_turns_and_do_not_wait_forever() {
        let b = Bench::new(section(600, 600), |tuning| tuning.background_queue_wait = Duration::from_secs(1)).await;
        let m = &b.module;
        b.flag("hold", true);
        let first = running(m, json!({ "script_path": b.script(), "url": "https://one.example/" }));
        until("the first runs", || b.calls().len() == 1).await;
        // The second waits its turn; when that takes too long it gives up and says so.
        let started = Instant::now();
        let late = run(m, json!({ "script_path": b.script(), "url": "https://two.example/" })).await.unwrap_err();
        assert_eq!(late.code, "other");
        assert!(late.message.starts_with("the browser was busy with other background work"), "{}", late.message);
        assert!(started.elapsed() >= Duration::from_secs(1) && b.calls().len() == 1, "it was not started");
        // One that waits less long than the first takes runs right after it.
        let second = running(m, json!({ "script_path": b.script(), "url": "https://two.example/" }));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(b.calls().len(), 1, "one at a time");
        b.flag("hold", false);
        assert_eq!(first.await.unwrap().unwrap()["data"]["url"], "https://one.example/");
        assert_eq!(second.await.unwrap().unwrap()["data"]["url"], "https://two.example/");
        assert_eq!(b.calls().len(), 2);
        m.shutdown().await;
    }

    /// A browser that is started for a background run only is not the
    /// user's browser coming back: the tabs it had stay closed, and it is
    /// stopped soon after the run. They are there as soon as someone uses it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_browser_started_for_a_background_run_only_keeps_its_tabs_closed_and_stops_soon() {
        let b = Bench::new(section(600, 1), |_| {}).await;
        // What the user had open when the browser stopped.
        b.module.open_tab(DEFAULT_GROUP, Some("https://a.example/kept")).await.unwrap();
        let second = b.module.open_tab(DEFAULT_GROUP, Some("https://b.example/kept")).await.unwrap();
        b.module.open_tab(DEFAULT_GROUP, Some("https://c.example/kept")).await.unwrap();
        b.module.activate_tab(&second.tab_id).unwrap();
        b.module.shutdown().await;
        b.fake.forget_tabs();
        let kept = b.groups_file();
        assert!(kept.contains("https://b.example/kept"), "{kept}");

        let m = Bench::service(b.tmp.path(), section(600, 1), |_| {});
        let housekeeping = tokio::spawn(m.clone().run());
        let urls = |m: &BrowserModule| m.group(DEFAULT_GROUP).tabs.into_iter().map(|tab| (tab.url, tab.active)).collect::<Vec<_>>();
        let all = vec![("https://a.example/kept".to_string(), false), ("https://b.example/kept".to_string(), true), ("https://c.example/kept".to_string(), false)];

        // A run starts the browser, without the tabs. It is not idle while the run works in it.
        b.flag("hold", true);
        let asked = running(&m, json!({ "script_path": b.script() }));
        until("the command runs", || b.calls().len() == 1).await;
        let pid = b.fake.pid().expect("the run started the browser");
        tokio::time::sleep(Duration::from_millis(1600)).await;
        assert!(m.running() && b.fake.pid() == Some(pid), "a browser a run works in is not stopped");
        assert!(urls(&m).is_empty() && b.fake.targets().is_empty(), "no tab was opened again: {:?}", b.fake.targets());
        let status = m.status().await;
        assert_eq!((&status["running"], &status["groups"][0]["tabs"]), (&json!(true), &json!([])));
        // Soon after the run it is stopped, long before a browser someone used would be.
        b.flag("hold", false);
        asked.await.unwrap().unwrap();
        let ended = Instant::now();
        until("the browser is stopped", || !m.running()).await;
        assert!(ended.elapsed() >= Duration::from_millis(900) && ended.elapsed() < Duration::from_secs(10), "{:?}", ended.elapsed());
        assert_eq!(b.groups_file(), kept, "the tabs it had are still what is kept");

        // While a run works someone comes to look: the tabs are there before they see the browser, and the
        // browser stays as long as any browser someone used.
        b.fake.forget_tabs();
        b.flag("hold", true);
        let asked = running(&m, json!({ "script_path": b.script() }));
        until("the command runs", || b.calls().len() == 2).await;
        assert!(urls(&m).is_empty());
        m.ensure().await.unwrap();
        assert_eq!(urls(&m), all);
        let viewer = m.viewer(DEFAULT_GROUP);
        b.flag("hold", false);
        asked.await.unwrap().unwrap();
        drop(viewer);
        tokio::time::sleep(Duration::from_millis(1600)).await;
        assert!(m.running() && urls(&m) == all, "it is the user's browser now");
        // A run in a browser that runs for someone changes nothing about it.
        run(&m, json!({ "script_path": b.script() })).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1600)).await;
        assert!(m.running() && urls(&m) == all);
        housekeeping.abort();
        m.shutdown().await;
        let tabs = |file: &str| {
            let group = serde_json::from_str::<Value>(file).unwrap()["groups"][0].clone();
            (group["tabs"].as_array().unwrap().iter().map(|tab| tab["url"].clone()).collect::<Vec<_>>(), group["active"].clone())
        };
        assert_eq!(tabs(&b.groups_file()), tabs(&kept));

        // It dies while it runs for a run only: it comes back as it was, without the tabs.
        b.fake.forget_tabs();
        let m = Bench::service(b.tmp.path(), section(600, 600), |_| {});
        b.flag("hold", true);
        let asked = running(&m, json!({ "script_path": b.script() }));
        until("the command runs", || b.calls().len() == 4).await;
        let pid = b.fake.pid().unwrap();
        b.fake.crash();
        until("the browser runs again", || m.running() && b.fake.pid().is_some_and(|again| again != pid)).await;
        assert!(urls(&m).is_empty() && b.fake.targets().is_empty(), "{:?}", b.fake.targets());
        b.flag("hold", false);
        asked.await.unwrap().unwrap();
        m.shutdown().await;
        assert_eq!(tabs(&b.groups_file()), tabs(&kept));

        // The agent taking a lease is someone using it too: it gets the tabs.
        b.fake.forget_tabs();
        let m = Bench::service(b.tmp.path(), section(600, 1), |_| {});
        b.flag("hold", true);
        let asked = running(&m, json!({ "script_path": b.script() }));
        until("the command runs", || b.calls().len() == 5).await;
        assert!(urls(&m).is_empty());
        let lease = m.acquire("work-1").await.unwrap();
        assert_eq!(lease.tabs.iter().map(|tab| tab.url.as_str()).collect::<Vec<_>>(), vec!["https://a.example/kept", "https://b.example/kept", "https://c.example/kept"]);
        assert_eq!(lease.active_tab_id, lease.tabs.get(1).map(|tab| tab.tab_id.clone()));
        b.flag("hold", false);
        asked.await.unwrap().unwrap();
        assert!(m.renew(&lease.lease_id).is_ok(), "the run did not end the agent's lease");
        m.shutdown().await;
    }

    /// The real `agent-browser background-run` in a real Chromium, on the
    /// pages of a local site:
    ///
    /// ```sh
    /// OPENDAN_CHROMIUM=<chrome> OPENDAN_AGENT_BROWSER=<launcher> \
    ///   cargo test -p opendan --lib browser::background -- --ignored --test-threads=1
    /// ```
    ///
    /// `<launcher>` is `agent-browser` of an installed ext-core, or for the
    /// sources a script like
    /// `NODE_PATH=<dir with playwright-core>/node_modules exec node publish/exttool/ext-core/browser/agent-browser.mjs "$@"`.
    mod real {
        use std::sync::Mutex;

        use super::*;
        use crate::browser::{process, AgentState, Controller, Handed};

        const SCRIPT: &str = "export default async ({ page, request, save, url, args }) => ({\n\
              url, at: page.url(), title: await page.title(), items: await page.locator('.item').allTextContents(),\n\
              cookies: await (await request.get(new URL('/cookies', url).href)).text(), limit: args.limit, save: typeof save.image,\n\
              held: Object.keys(process.env).filter((name) => /^(BUCKYOS|OPENDAN)_/.test(name)) });\n";
        const NEEDS_LOGIN: &str = "export default async ({ page }) => { throw Object.assign(new Error('sign in at ' + new URL(page.url()).pathname), { code: 'auth' }); };\n";

        /// `/list` is what a script reads, `/cookies` tells the cookies a request came with, anything else is the user's page.
        async fn site() -> String {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    tokio::spawn(async move {
                        let mut request = [0u8; 8192];
                        let read = stream.read(&mut request).await.unwrap_or(0);
                        let request = String::from_utf8_lossy(&request[..read]).to_string();
                        let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                        let cookies = request.lines().find_map(|l| l.strip_prefix("Cookie: ").or_else(|| l.strip_prefix("cookie: "))).unwrap_or_default().trim().to_string();
                        let body = match path.split('?').next().unwrap_or_default() {
                            "/list" => "<!doctype html><meta charset=utf-8><title>List</title><ul><li class=item>一</li><li class=item>二</li></ul>".to_string(),
                            "/cookies" => cookies,
                            _ => "<!doctype html><title>Users</title><body>the user's page</body>".to_string(),
                        };
                        let _ = stream
                            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                            .await;
                    });
                }
            });
            format!("http://127.0.0.1:{port}")
        }

        async fn eval(m: &BrowserModule, tab_id: &str, expression: &str) -> Value {
            let answer = m.tab_send(tab_id, "Runtime.evaluate", json!({ "expression": expression, "returnByValue": true })).await.unwrap();
            answer["result"]["value"].clone()
        }

        /// What the browser's tabs were at every change while `work` went on: `(tabs of the run, the user's tabs, the user's current tab, who operates)`.
        async fn watching<T>(m: &Arc<BrowserModule>, work: impl std::future::Future<Output = T>) -> (T, Vec<(usize, usize, Option<String>, Controller)>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let watcher = {
                let (m, seen) = (m.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut changes = m.changes();
                    while changes.changed().await.is_ok() {
                        let users = m.group(DEFAULT_GROUP);
                        let current = users.tabs.iter().find(|tab| tab.active).map(|tab| tab.tab_id.clone());
                        seen.lock().unwrap().push((m.group(GROUP).tabs.len(), users.tabs.len(), current, users.controller));
                    }
                })
            };
            let done = work.await;
            watcher.abort();
            let seen = seen.lock().unwrap().clone();
            (done, seen)
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        #[ignore = "needs OPENDAN_CHROMIUM and OPENDAN_AGENT_BROWSER"]
        async fn a_background_run_works_beside_the_user_beside_the_agent_and_in_a_stopped_browser() {
            let chromium = std::env::var_os(process::ENV_CHROMIUM).map(PathBuf::from).expect("set OPENDAN_CHROMIUM to a Chromium executable");
            assert!(find_agent_browser().is_some(), "set OPENDAN_AGENT_BROWSER to an agent-browser launcher");
            let tmp = tempfile::tempdir().unwrap();
            let (script, needs_login) = (tmp.path().join("fetch.mjs"), tmp.path().join("login.mjs"));
            std::fs::write(&script, SCRIPT).unwrap();
            std::fs::write(&needs_login, NEEDS_LOGIN).unwrap();
            let site = site().await;
            let service = || {
                let tuning = Tuning { chromium: Some(chromium.clone()), dirs: Some(BrowserDirs::at(tmp.path().join("browser"))), tick: Duration::from_millis(100), ..Default::default() };
                BrowserModule::with_tuning(section(600, 2), deps(&tmp.path().join("agent_root")), tuning)
            };
            let list = format!("{site}/list?page=1");
            let params = json!({ "script_path": script.display().to_string(), "url": list, "args": { "limit": 2 }, "timeout_s": 30 });
            let read = |cookies: &str| json!({ "data": { "url": list, "at": list, "title": "List", "items": ["一", "二"], "cookies": cookies, "limit": 2, "save": "function", "held": [] } });
            // What the service holds in its environment does not reach the script.
            std::env::set_var("BUCKYOS_APPCLIENT_SESSION_TOKEN", "the-services-own-token");
            let m = service();
            let housekeeping = tokio::spawn(m.clone().run());

            // ---- the user operates the browser, signed in to the site
            let users = m.open_tab(DEFAULT_GROUP, Some(&format!("{site}/user"))).await.unwrap();
            let viewer = m.viewer(DEFAULT_GROUP);
            until("the user's page is loaded", || m.group(DEFAULT_GROUP).tabs.first().is_some_and(|tab| tab.title == "Users" && !tab.loading)).await;
            eval(&m, &users.tab_id, "document.cookie = 'login=alice; path=/; max-age=3600'").await;
            let (answered, seen) = watching(&m, run(&m, params.clone())).await;
            // The run read the page in a tab of its own, as the user the browser is signed in as.
            assert_eq!(answered.unwrap(), read("login=alice"));
            assert!(seen.iter().any(|(job, ..)| *job == 1), "the run had a tab of its own: {seen:?}");
            assert!(seen.iter().all(|(_, tabs, current, controller)| *tabs == 1 && current.as_deref() == Some(users.tab_id.as_str()) && *controller == Controller::User), "{seen:?}");
            until("the run's tab is gone", || m.group(GROUP).tabs.is_empty()).await;
            assert_eq!(eval(&m, &users.tab_id, "document.title + ' ' + location.pathname + ' ' + document.visibilityState").await, "Users /user visible");
            // What the run read is not what the owner read.
            let visits = m.visited(None)["visits"].as_array().unwrap().iter().map(|visit| visit["url"].clone()).collect::<Vec<_>>();
            assert_eq!(visits, vec![json!(format!("{site}/user"))]);
            // A script that finds it has to sign in says so in the form its caller knows.
            let refused = run(&m, json!({ "script_path": needs_login.display().to_string(), "url": list })).await.unwrap_err();
            assert_eq!((refused.code, refused.message.as_str()), ("other", "auth: sign in at /list"));
            assert!(m.group(GROUP).tabs.is_empty());
            drop(viewer);

            // ---- the agent operates the browser
            let lease = m.acquire("work-1").await.unwrap();
            let (answered, seen) = watching(&m, run(&m, params.clone())).await;
            assert_eq!(answered.unwrap(), read("login=alice"));
            assert!(seen.iter().all(|(_, tabs, current, controller)| *tabs == 1 && current.as_deref() == Some(users.tab_id.as_str()) && *controller == Controller::Agent), "{seen:?}");
            assert!(m.renew(&lease.lease_id).is_ok(), "the agent's lease is as it was");
            // ... and the user who took it over.
            m.hand(OWNER, Handed::TakeOver).await.unwrap();
            assert_eq!(run(&m, params.clone()).await.unwrap(), read("login=alice"));
            let group = m.group(DEFAULT_GROUP);
            assert!(group.user_hold && group.tabs.len() == 1 && group.agent_state == AgentState::Idle);
            m.hand(OWNER, Handed::HandBack).await.unwrap();
            housekeeping.abort();
            m.shutdown().await;

            // ---- the browser is stopped: it is started for the run, without the user's tab, and stopped soon after
            let kept = std::fs::read_to_string(tmp.path().join("agent_root/state/browser/groups.json")).unwrap();
            assert!(kept.contains("/user"), "{kept}");
            let m = service();
            let housekeeping = tokio::spawn(m.clone().run());
            let (answered, seen) = watching(&m, run(&m, params.clone())).await;
            assert_eq!(answered.unwrap(), read("login=alice"), "the browser's sign-ins are there");
            assert!(seen.iter().any(|(job, ..)| *job == 1) && seen.iter().all(|(_, tabs, ..)| *tabs == 0), "{seen:?}");
            let ended = Instant::now();
            assert!(m.running() && m.group(DEFAULT_GROUP).tabs.is_empty());
            until("the browser is stopped", || !m.running()).await;
            assert!(ended.elapsed() >= Duration::from_millis(1500) && ended.elapsed() < Duration::from_secs(20), "{:?}", ended.elapsed());
            assert_eq!(std::fs::read_to_string(tmp.path().join("agent_root/state/browser/groups.json")).unwrap(), kept, "the user's tab is still what is kept");
            // The user comes back: their tab is there.
            m.ensure().await.unwrap();
            until("the user's tab is back", || m.group(DEFAULT_GROUP).tabs.first().is_some_and(|tab| tab.title == "Users" && tab.url == format!("{site}/user"))).await;
            std::env::remove_var("BUCKYOS_APPCLIENT_SESSION_TOKEN");
            housekeeping.abort();
            m.shutdown().await;
        }
    }
}
