//! Reflow ([Agent Browser Protocol] §5): a page of a site shown from the
//! owner's reflow library — per site a package and the results made with it,
//! in the bucket `user/control-panel/<owner>/reflow` — instead of the site
//! itself. The Agent Service tells what the library has for an address
//! (`browser.reflow_state`) and makes a result in the agent's browser
//! (`browser.reflow_refresh`, `browser.reflow_action`): the package's
//! `refresh.mjs` runs on the page in a background tab (`agent-browser
//! reflow-run`) and what it returns is kept as the page's result.
//!
//! The library is read and written with the caller's session token: what a
//! caller may do there is what the Zone lets it do.
//!
//! [Agent Browser Protocol]: ../../../../../doc/opendan/protocol/Agent%20Browser%20Protocol.md

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use buckyos_api::bucket_client::BucketId;
use futures::FutureExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use super::{now_ms, BrowserError, BrowserModule};

pub const ENV_AGENT_BROWSER: &str = "OPENDAN_AGENT_BROWSER";
/// The group of the tab a refresh opens: the module's own, shown to nobody.
const GROUP: &str = "_reflow";
const MANIFEST_SCHEMA: &str = "buckyos.reflow/1";
const RESULT_SCHEMA: &str = "buckyos.reflow.result/1";
const MANIFEST_FILE: &str = "manifest.json";
const REFRESH_SCRIPT: &str = "refresh.mjs";
const DEFAULT_TIMEOUT_S: u64 = 60;
const MAX_TIMEOUT_S: u64 = 600;
/// What `agent-browser` gets beyond the script's own time, to start and to
/// close its tab.
const COMMAND_MARGIN: Duration = Duration::from_secs(10);
/// How much of the command's stderr is kept: its last line is why it failed.
const STDERR_TAIL: usize = 16 * 1024;
/// Failures are kept until the page is refreshed again; beyond this many the
/// oldest go.
const KEPT_FAILURES: usize = 200;
/// `encodeURIComponent`: everything but letters, digits and `-_.!~*'()`.
const COMPONENT: &AsciiSet =
    &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'!').remove(b'~').remove(b'*').remove(b'\'').remove(b'(').remove(b')');

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

// ---------------------------------------------------------------------------
// The rules (§5.1, §5.2), shared with the `reflow` command and checked
// against `doc/opendan/protocol/fixtures/reflow_vectors.json`
// ---------------------------------------------------------------------------

/// `www.example.com` → `www-example-com`; a name without a dash gets `-host`.
pub fn site_of_host(host: &str) -> Option<String> {
    let name = host.to_ascii_lowercase();
    let name = name.strip_suffix('.').unwrap_or(&name);
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')) {
        return None;
    }
    let site = name.replace('.', "-");
    Some(if site.contains('-') { site } else { format!("{site}-host") })
}

/// `*` matches any run of characters, `/` included; the rest is literal.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == path;
    }
    if !path.starts_with(parts[0]) {
        return false;
    }
    let mut at = parts[0].len();
    for part in &parts[1..parts.len() - 1] {
        match path[at..].find(part) {
            Some(found) => at += found + part.len(),
            None => return false,
        }
    }
    let last = parts[parts.len() - 1];
    path.len() - at >= last.len() && path.ends_with(last)
}

/// An address as the library names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Address {
    /// The normalized URL.
    pub url: String,
    /// Its host name: lower case, no port, no trailing dot.
    pub host: String,
    /// Its path, as the normalized URL spells it.
    pub path: String,
    pub site: String,
    pub page: String,
}

impl Address {
    /// `None`: nothing is reflowed for this address (not an http(s) URL, or a
    /// host the library has no name for).
    pub fn of(input: &str) -> Option<Self> {
        let mut url = url::Url::parse(input.trim()).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        url.set_fragment(None);
        let _ = url.set_username("");
        let _ = url.set_password(None);
        let host = url.host_str()?;
        let host = host.strip_suffix('.').unwrap_or(host).to_string();
        let site = site_of_host(&host)?;
        let page = hex::encode(&Sha256::digest(url.as_str().as_bytes())[..8]);
        Some(Self { path: url.path().to_string(), url: url.into(), host, site, page })
    }

    /// Whether one of a package's `match` rules (`(host, path pattern)`) is for this address.
    pub fn matches(&self, rules: &[(String, String)]) -> bool {
        rules.iter().any(|(host, path)| *host == self.host && path_matches(path, &self.path))
    }

    /// The page that shows it, relative to `reflow.<zone>`: the package's
    /// page when there is a result for it, else the fallback page.
    pub fn view(&self, result: bool, fallback: bool) -> Option<String> {
        if result {
            return Some(format!("{}/?url={}&page={}", self.site, utf8_percent_encode(&self.url, COMPONENT), self.page));
        }
        fallback.then(|| format!("{}/fallback/{}.html", self.site, self.page))
    }
}

/// What JS calls a safe integer, as a JSON number.
fn safe_integer(value: &Value) -> Option<u64> {
    const MAX: u64 = (1 << 53) - 1;
    let number = value.as_u64().or_else(|| value.as_f64().filter(|f| f.fract() == 0.0 && *f >= 0.0).map(|f| f as u64))?;
    (number <= MAX).then_some(number)
}

/// What a valid `manifest.json` declares.
#[derive(Debug, Clone, PartialEq)]
struct Package {
    /// `match[]`: `(host, path pattern)`.
    rules: Vec<(String, String)>,
    updated_at: u64,
    author: Value,
    preference: String,
    timeout_s: u64,
}

impl Package {
    /// `None`: the manifest breaks a rule of §5.2, which is as good as no package.
    fn of(manifest: &Value, site: &str) -> Option<Self> {
        if manifest.get("schema")?.as_str()? != MANIFEST_SCHEMA || manifest.get("site")?.as_str()? != site {
            return None;
        }
        let rules = manifest.get("match")?.as_array()?.iter().map(|rule| {
            let host = rule.get("host")?.as_str()?;
            let path = match rule.get("path") {
                None => "/*",
                Some(path) => path.as_str().filter(|p| p.starts_with('/'))?,
            };
            let own = site_of_host(host).as_deref() == Some(site) && host == host.to_lowercase() && !host.ends_with('.');
            own.then(|| (host.to_string(), path.to_string()))
        });
        let rules = rules.collect::<Option<Vec<_>>>().filter(|rules| !rules.is_empty())?;
        let preference = match manifest.get("preference") {
            None => "",
            Some(preference) => preference.as_str()?,
        };
        let author = manifest.get("author")?;
        if !matches!(author.get("kind")?.as_str()?, "agent" | "user") || author.get("id")?.as_str()?.is_empty() {
            return None;
        }
        let timeout_s = match manifest.get("refresh") {
            None => DEFAULT_TIMEOUT_S,
            Some(refresh) if refresh.get("script")?.as_str()? == REFRESH_SCRIPT => {
                safe_integer(refresh.get("timeout_s")?).filter(|t| (1..=MAX_TIMEOUT_S).contains(t))?
            }
            Some(_) => return None,
        };
        Some(Self {
            rules,
            updated_at: safe_integer(manifest.get("updated_at")?).filter(|at| *at > 0)?,
            author: author.clone(),
            preference: preference.to_string(),
            timeout_s,
        })
    }
}

/// When, by whom and of what a result or a fallback page was made.
fn made(meta: &Value) -> Option<Value> {
    let (made_at, made_by, source_url) = (meta.get("made_at")?, meta.get("made_by")?.as_str()?, meta.get("source_url")?.as_str()?);
    made_at.is_number().then(|| json!({ "made_at": made_at, "made_by": made_by, "source_url": source_url }))
}

// ---------------------------------------------------------------------------
// The library in the bucket
// ---------------------------------------------------------------------------

fn library_error(what: impl std::fmt::Display) -> BrowserError {
    BrowserError::new("other", format!("reflow library: {what}"))
}

/// The owner's library, and as whom it is read and written.
#[derive(Debug, Clone)]
pub struct Library {
    /// Where nfs-server's data plane (`/bucket/v1/`) is.
    pub origin: String,
    /// `user/control-panel/<owner>/reflow`.
    pub bucket_id: String,
    /// The session token requests carry: the caller's, or the service's own
    /// for a caller that presented none.
    pub token: Option<String>,
}

/// What the library has for an address.
struct Found {
    /// The site's package, when it is valid and for this address.
    package: Option<Package>,
    result: Option<Value>,
    fallback: Option<Value>,
}

impl Library {
    /// The library of this agent's owner in the Zone the service runs in.
    async fn of_runtime() -> Result<Self, BrowserError> {
        let runtime = buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| library_error(format!("this Agent Service is not in a Zone ({e})")))?;
        let owner = runtime.get_owner_user_id().ok_or_else(|| library_error("this agent has no owner"))?;
        let bucket = BucketId::user(buckyos_api::CONTROL_PANEL_SERVICE_NAME, &owner, buckyos_api::CONTROL_PANEL_REFLOW_BUCKET)
            .map_err(library_error)?;
        let token = runtime.get_session_token().await;
        // nfs-server's planes share an origin: the node's gateway.
        Ok(Self { origin: runtime.named_store_base_url(), bucket_id: bucket.to_string(), token: (!token.is_empty()).then_some(token) })
    }

    fn request(&self, http: &reqwest::Client, method: reqwest::Method, key: &str) -> reqwest::RequestBuilder {
        let request = http.request(method, format!("{}{}{}/{key}", self.origin, buckyos_api::BUCKET_DATA_PATH, self.bucket_id));
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    /// Send a request for `key`. `None`: the key is not there. A refusal is
    /// an error in the bucket's own words.
    async fn send(&self, request: reqwest::RequestBuilder, key: &str) -> Result<Option<reqwest::Response>, BrowserError> {
        let failed = |why: String| library_error(format!("{}/{key}: {why}", self.bucket_id));
        let response = request.send().await.map_err(|e| {
            let mut why = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(cause) = source {
                why = format!("{why}: {cause}");
                source = cause.source();
            }
            failed(why)
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(Some(response));
        }
        let refusal: Value = response.json().await.unwrap_or_default();
        match refusal["error"].as_str() {
            Some("no_such_key") => Ok(None),
            Some(code) => Err(failed(format!("{code}: {}", refusal["message"].as_str().unwrap_or_default()))),
            None => Err(failed(format!("HTTP {status}"))),
        }
    }

    async fn get(&self, http: &reqwest::Client, key: &str) -> Result<Option<Vec<u8>>, BrowserError> {
        let Some(response) = self.send(self.request(http, reqwest::Method::GET, key), key).await? else { return Ok(None) };
        let bytes = response.bytes().await.map_err(|e| library_error(format!("{}/{key}: {e}", self.bucket_id)))?;
        Ok(Some(bytes.to_vec()))
    }

    /// A key's content as JSON. `None`: the key is not there, or it is not JSON.
    async fn get_json(&self, http: &reqwest::Client, key: String) -> Result<Option<Value>, BrowserError> {
        Ok(self.get(http, &key).await?.and_then(|bytes| serde_json::from_slice(&bytes).ok()))
    }

    async fn put_json(&self, http: &reqwest::Client, key: &str, value: &Value) -> Result<(), BrowserError> {
        let request = self
            .request(http, reqwest::Method::PUT, key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(value.to_string());
        self.send(request, key).await?.map(|_| ()).ok_or_else(|| library_error(format!("{}/{key}: no_such_key", self.bucket_id)))
    }

    async fn find(&self, http: &reqwest::Client, address: &Address) -> Result<Found, BrowserError> {
        let Address { site, page, .. } = address;
        let (manifest, result, fallback) = tokio::try_join!(
            self.get_json(http, format!("{site}/{MANIFEST_FILE}")),
            self.get_json(http, format!("{site}/results/{page}.json")),
            self.get_json(http, format!("{site}/fallback/{page}.json")),
        )?;
        Ok(Found {
            package: manifest.and_then(|m| Package::of(&m, site)).filter(|p| address.matches(&p.rules)),
            result: result.as_ref().and_then(made),
            fallback: fallback.as_ref().and_then(made),
        })
    }
}

// ---------------------------------------------------------------------------
// Refreshes
// ---------------------------------------------------------------------------

/// One refresh: what `browser.reflow_refresh` or `browser.reflow_action` asked for.
struct Job {
    address: Address,
    /// The library as the caller that asked reads and writes it.
    library: Library,
    agent_browser: PathBuf,
    timeout_s: u64,
    /// `browser.reflow_action`: what is clicked on the page first.
    click: Option<String>,
}

struct Run {
    /// Not running: it failed.
    running: bool,
    error: Option<String>,
    at_ms: u64,
    /// An action asked for while this runs: done by one more run after it.
    next: Option<Job>,
}

/// The refreshes in progress and those that failed, by normalized URL.
#[derive(Default)]
pub(super) struct Reflow {
    http: OnceLock<reqwest::Client>,
    runs: Mutex<HashMap<String, Run>>,
    /// One refresh works in the browser at a time; the others wait their turn.
    turn: tokio::sync::Mutex<()>,
}

impl Reflow {
    fn http(&self) -> &reqwest::Client {
        self.http.get_or_init(|| {
            let builder = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60));
            builder.build().unwrap_or_default()
        })
    }

    fn runs(&self) -> std::sync::MutexGuard<'_, HashMap<String, Run>> {
        self.runs.lock().expect("reflow runs")
    }

    /// A refresh works in the browser: it is not idle.
    pub(super) fn working(&self) -> bool {
        self.turn.try_lock().is_err()
    }

    /// `refresh` of `browser.reflow_state`.
    fn status(&self, url: &str) -> Value {
        match self.runs().get(url) {
            None => json!({ "status": "idle", "error": null, "at_ms": null }),
            Some(run) => json!({ "status": if run.running { "running" } else { "failed" }, "error": run.error, "at_ms": run.at_ms }),
        }
    }
}

/// What the refresh command keeps of the service's environment: enough to
/// run. Nothing the service holds (its session tokens, where its kRPC is)
/// reaches `refresh.mjs`: a package is anyone's who may write the library.
const KEPT_ENV: &[&str] = if cfg!(windows) {
    &["PATH", "PATHEXT", "ComSpec", "SystemRoot", "SystemDrive", "windir", "ProgramData", "ProgramFiles", "ProgramFiles(x86)"]
} else {
    &["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ"]
};

/// The environment of the refresh command: [`KEPT_ENV`], and a home and a
/// temp directory of its own in `dir` (they go with it when the refresh ended).
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
            return Err(format!("the refresh did not end within {} seconds and was ended", limit.as_secs()));
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
            BrowserError::unavailable("no `agent-browser` on this node (it comes with the ext-core extension tools): it is what refreshes a reflow page")
        })
    }

    /// The owner's library as a caller with `token` reads and writes it.
    async fn reflow_library(&self, token: Option<&str>) -> Result<Library, BrowserError> {
        let mut library = match &self.tuning.reflow_library {
            Some(library) => library.clone(),
            None => Library::of_runtime().await?,
        };
        if let Some(token) = token {
            library.token = Some(token.to_string());
        }
        Ok(library)
    }

    fn reflow_address(url: &str) -> Result<Address, BrowserError> {
        Address::of(url).ok_or_else(|| BrowserError::invalid(format!("`{url}` has no reflow: only http(s) pages of a named host have")))
    }

    /// What `browser.reflow_state` returns (§5.3).
    fn reflow_view(address: &Address, found: &Found, refresh: Value) -> Value {
        json!({
            "url": address.url,
            "site": address.site,
            "page": address.page,
            "package": found.package.as_ref().map(|p| json!({ "updated_at": p.updated_at, "author": p.author, "preference": p.preference })),
            "result": found.result,
            "fallback": found.fallback,
            "refresh": refresh,
            "view": address.view(found.package.is_some() && found.result.is_some(), found.fallback.is_some()),
        })
    }

    /// `browser.reflow_state`: what the library has for `url`, and how its
    /// refresh is doing. Needs no browser: an agent whose browser is switched
    /// off or missing still tells which page shows what was made before.
    pub async fn reflow_state(&self, token: Option<&str>, url: &str) -> Result<Value, BrowserError> {
        let address = Self::reflow_address(url)?;
        let library = self.reflow_library(token).await?;
        // How the refresh is doing before the library is read: a refresh that
        // ends meanwhile is still "running" beside what the library had, never
        // "ended" beside a library without its result.
        let refresh = self.reflow.status(&address.url);
        let found = library.find(self.reflow.http(), &address).await?;
        Ok(Self::reflow_view(&address, &found, refresh))
    }

    /// `browser.reflow_refresh`, and with `click` `browser.reflow_action`:
    /// start making the page's result again and return at once. One refresh
    /// of an address runs at a time: asking again while it runs returns the
    /// same state, and an action asked for meanwhile is done right after it.
    pub async fn reflow_refresh(&self, token: Option<&str>, url: &str, click: Option<String>) -> Result<Value, BrowserError> {
        self.usable()?;
        let agent_browser = self.agent_browser()?;
        let address = Self::reflow_address(url)?;
        let library = self.reflow_library(token).await?;
        let found = library.find(self.reflow.http(), &address).await?;
        let Some(package) = &found.package else {
            return Err(BrowserError::new("reflow_no_package", format!("no package of the reflow library is for {}", address.url)));
        };
        let job = Job { address: address.clone(), library, agent_browser, timeout_s: package.timeout_s, click };
        let starts = {
            let mut runs = self.reflow.runs();
            match runs.get_mut(&address.url) {
                Some(run) if run.running => {
                    if job.click.is_some() {
                        run.next = Some(job);
                    }
                    None
                }
                _ => {
                    runs.insert(address.url.clone(), Run { running: true, error: None, at_ms: now_ms(), next: None });
                    Some(job)
                }
            }
        };
        if let Some(job) = starts {
            tokio::spawn(self.me().reflow_run(job));
        }
        Ok(Self::reflow_view(&address, &found, self.reflow.status(&address.url)))
    }

    /// Refresh, and again for every action asked for meanwhile; then the
    /// address is idle, or failed with the reason.
    async fn reflow_run(self: Arc<Self>, mut job: Job) {
        loop {
            let url = &job.address.url;
            let outcome = std::panic::AssertUnwindSafe(self.reflow_once(&job))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err("the refresh stopped unexpectedly".to_string()));
            match &outcome {
                Ok(()) => log::info!("browser: the reflow result of {url} was made again"),
                Err(e) => log::warn!("browser: the reflow result of {url} was not made: {e}"),
            }
            let next = {
                let mut runs = self.reflow.runs();
                let next = runs.get_mut(url).and_then(|run| run.next.take());
                match (&next, outcome) {
                    (Some(_), _) => {}
                    (None, Ok(())) => {
                        runs.remove(url);
                    }
                    (None, Err(error)) => {
                        runs.insert(url.clone(), Run { running: false, error: Some(error), at_ms: now_ms(), next: None });
                        if runs.len() > KEPT_FAILURES {
                            let oldest = runs.iter().filter(|(_, run)| !run.running).min_by_key(|(_, run)| run.at_ms).map(|(url, _)| url.clone());
                            oldest.and_then(|url| runs.remove(&url));
                        }
                    }
                }
                next
            };
            match next {
                Some(next) => job = next,
                None => break,
            }
        }
    }

    async fn reflow_once(&self, job: &Job) -> Result<(), String> {
        let _turn = self.reflow.turn.lock().await;
        let dir = self.dirs.root.join("reflow").join(&job.address.page);
        let made = self.reflow_in(job, &dir).await;
        let _ = tokio::fs::remove_dir_all(&dir).await;
        made
    }

    /// The package comes out of the library into `dir`, its script runs on
    /// the page, and what it returns goes into the library as the result.
    async fn reflow_in(&self, job: &Job, dir: &Path) -> Result<(), String> {
        let (http, Address { url, site, page, .. }) = (self.reflow.http(), &job.address);
        let package = dir.join("package");
        let _ = tokio::fs::remove_dir_all(dir).await;
        for part in [&package, &dir.join("home"), &dir.join("tmp")] {
            tokio::fs::create_dir_all(part).await.map_err(|e| format!("create {}: {e}", part.display()))?;
        }
        for file in [MANIFEST_FILE, REFRESH_SCRIPT] {
            let content = job.library.get(http, &format!("{site}/{file}")).await.map_err(|e| e.message)?;
            let content = content.ok_or_else(|| match file {
                REFRESH_SCRIPT => format!("the package of {site} has no {REFRESH_SCRIPT}: it cannot be refreshed, its results are put into the library"),
                _ => format!("the package of {site} is gone"),
            })?;
            tokio::fs::write(package.join(file), content).await.map_err(|e| format!("write {file}: {e}"))?;
        }
        let out = dir.join("result.json");

        self.ensure().await.map_err(|e| e.message)?;
        let port = self.shared().port.ok_or("the browser stopped")?;
        let mut cmd = tokio::process::Command::new(&job.agent_browser);
        cmd.arg("reflow-run").arg("--cdp").arg(format!("http://127.0.0.1:{port}")).arg("--url").arg(url);
        cmd.arg("--package").arg(&package).arg("--out").arg(&out).arg("--timeout").arg(job.timeout_s.to_string());
        if let Some(selector) = &job.click {
            cmd.arg(format!("--click={selector}"));
        }
        cmd.env_clear().envs(command_env(dir)).current_dir(dir);
        self.reflow_command(cmd, Duration::from_secs(job.timeout_s) + COMMAND_MARGIN).await?;

        let data = tokio::fs::read(&out).await.map_err(|e| format!("agent-browser left no result: {e}"))?;
        let data: Value = serde_json::from_slice(&data).map_err(|e| format!("what {REFRESH_SCRIPT} returned is not JSON: {e}"))?;
        let result = json!({
            "schema": RESULT_SCHEMA,
            "source_url": url,
            "made_at": now_ms(),
            "made_by": self.deps.agent.agent_did(),
            "data": data,
        });
        job.library.put_json(http, &format!("{site}/results/{page}.json"), &result).await.map_err(|e| e.message)
    }

    /// Run the refresh command. It opens one tab, right after it connected:
    /// until that tab is there, a tab without an opener belongs to `_reflow`.
    /// What is left in `_reflow` when it ended (it was ended, or did not
    /// close its tab) is closed.
    async fn reflow_command(&self, cmd: tokio::process::Command, limit: Duration) -> Result<(), String> {
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
}

#[cfg(all(test, unix))]
#[path = "../../tests/fake_browser/mod.rs"]
mod fake_browser;

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "did:bns:jarvis.bob";
    const SITE: &str = "www-example-com";
    const URL: &str = "https://www.example.com/news/1?a=1#top";
    const NORMALIZED: &str = "https://www.example.com/news/1?a=1";

    fn manifest(timeout_s: u64) -> Value {
        json!({
            "schema": "buckyos.reflow/1",
            "site": SITE,
            "match": [{ "host": "www.example.com", "path": "/news/*" }],
            "preference": "只看标题和正文",
            "author": { "kind": "agent", "id": AGENT },
            "updated_at": 1794600000000u64,
            "refresh": { "script": "refresh.mjs", "timeout_s": timeout_s },
        })
    }

    #[test]
    fn the_rules_pass_the_shared_vectors() {
        let vectors: Value = serde_json::from_str(include_str!("../../../../../doc/opendan/protocol/fixtures/reflow_vectors.json")).unwrap();
        let text = |value: &Value| value.as_str().unwrap().to_string();
        let (normalize, invalid, matching) =
            (vectors["normalize"].as_array().unwrap(), vectors["invalid"].as_array().unwrap(), vectors["match"].as_array().unwrap());
        assert!(normalize.len() >= 25 && invalid.len() >= 9 && matching.len() >= 27, "the vectors are all there");
        for case in normalize {
            let address = Address::of(case["url"].as_str().unwrap()).unwrap_or_else(|| panic!("{case} has a reflow page"));
            assert_eq!((&address.url, &address.site, &address.page), (&text(&case["normalized"]), &text(&case["site"]), &text(&case["page"])), "{case}");
            assert_eq!(address.view(true, false), Some(text(&case["view"])), "{case}");
            assert_eq!(address.view(true, true), Some(text(&case["view"])), "a result comes before a fallback page");
            assert_eq!(address.view(false, true), Some(format!("{}/fallback/{}.html", address.site, address.page)));
            assert_eq!(address.view(false, false), None);
        }
        for url in invalid {
            assert_eq!(Address::of(url.as_str().unwrap()), None, "{url}");
        }
        for case in matching {
            let rules: Vec<(String, String)> = case["manifest_match"]
                .as_array()
                .unwrap()
                .iter()
                .map(|rule| (text(&rule["host"]), rule["path"].as_str().unwrap_or("/*").to_string()))
                .collect();
            let address = Address::of(case["url"].as_str().unwrap()).unwrap();
            assert_eq!(address.matches(&rules), case["matches"].as_bool().unwrap(), "{case}");
        }
    }

    #[test]
    fn a_manifest_that_breaks_a_rule_is_no_package() {
        let package = Package::of(&manifest(5), SITE).unwrap();
        assert_eq!(package.rules, vec![("www.example.com".to_string(), "/news/*".to_string())]);
        assert_eq!((package.updated_at, package.timeout_s, package.preference.as_str()), (1794600000000, 5, "只看标题和正文"));
        assert_eq!(package.author, json!({ "kind": "agent", "id": AGENT }));

        // What a manifest may leave out.
        let mut bare = manifest(5);
        bare.as_object_mut().unwrap().retain(|key, _| !matches!(key.as_str(), "preference" | "refresh"));
        bare["match"] = json!([{ "host": "www.example.com" }, { "host": "www.example.com", "path": "/" }]);
        bare["author"]["kind"] = json!("user");
        let package = Package::of(&bare, SITE).unwrap();
        assert_eq!(package.rules[0].1, "/*");
        assert_eq!((package.timeout_s, package.preference.as_str()), (DEFAULT_TIMEOUT_S, ""));

        let changes: [(&str, fn(&mut Value)); 19] = [
            ("another schema", |m| m["schema"] = json!("buckyos.reflow/2")),
            ("another site", |m| m["site"] = json!("example-com")),
            ("no rule", |m| m["match"] = json!([])),
            ("rules that are no list", |m| m["match"] = json!({ "host": "www.example.com" })),
            ("a rule without a host", |m| m["match"][0] = json!({ "path": "/" })),
            ("the host of another site", |m| m["match"][0]["host"] = json!("example.com")),
            ("a host in upper case", |m| m["match"][0]["host"] = json!("WWW.example.com")),
            ("a host with a trailing dot", |m| m["match"][0]["host"] = json!("www.example.com.")),
            ("a path that does not start at the root", |m| m["match"][0]["path"] = json!("news/*")),
            ("a path that is no text", |m| m["match"][0]["path"] = json!(null)),
            ("a preference that is no text", |m| m["preference"] = json!(["short"])),
            ("no author", |m| m["author"] = json!(null)),
            ("an author of an unknown kind", |m| m["author"]["kind"] = json!("robot")),
            ("an author without an id", |m| m["author"]["id"] = json!("")),
            ("no time", |m| m["updated_at"] = json!(0)),
            ("a time that is no integer", |m| m["updated_at"] = json!(1794600000000.5)),
            ("another script", |m| m["refresh"]["script"] = json!("other.mjs")),
            ("a timeout out of range", |m| m["refresh"]["timeout_s"] = json!(MAX_TIMEOUT_S + 1)),
            ("refresh without a timeout", |m| m["refresh"] = json!({ "script": "refresh.mjs" })),
        ];
        for (what, change) in changes {
            let mut broken = manifest(5);
            change(&mut broken);
            assert_eq!(Package::of(&broken, SITE), None, "{what}");
        }
        assert_eq!(Package::of(&json!("manifest"), SITE), None);
        assert_eq!(Package::of(&manifest(5), "example-com"), None, "the manifest of another site");
        // A time a JS writer spelled as a float is the integer it is.
        let mut float = manifest(5);
        float["updated_at"] = json!(1794600000000.0);
        assert_eq!(Package::of(&float, SITE).unwrap().updated_at, 1794600000000);
    }

    #[test]
    fn the_reason_a_command_failed_is_its_last_line() {
        assert_eq!(last_line(b"Error: x\n    at refresh.mjs:1:1\nagent-browser: reflow failed: x\n\n").as_deref(), Some("agent-browser: reflow failed: x"));
        assert_eq!(last_line(b" \n"), None);
    }

    /// Against a fake browser (`tests/fake_browser`), a fake `agent-browser`
    /// and a fake data plane of the bucket; the library's wire also against
    /// a real nfs-server; and — ignored — the real `agent-browser` in a real
    /// Chromium.
    #[cfg(unix)]
    mod refreshing {
        use std::collections::BTreeMap;
        use std::time::Instant;

        use axum::body::Bytes;
        use axum::extract::{Path as UrlPath, State};
        use axum::http::{header, HeaderMap, Method, StatusCode};
        use axum::response::{IntoResponse, Response};

        use super::super::fake_browser::{self, FakeBrowser};
        use super::*;
        use crate::browser::{process, rpc, BrowserDeps, BrowserDirs, Tuning, ENV_BROWSER_RPC};
        use crate::config::BrowserSection;

        const BUCKET: &str = "user/control-panel/bob/reflow";

        /// Stands in for `agent-browser reflow-run`: what it "reads off the
        /// page" is its arguments and the package's script. Files beside it
        /// tell it to wait (`hold`), to fail (`fail`) or never to end (`hang`);
        /// it leaves its command line (`calls`) and environment (`env`) there.
        const AGENT_BROWSER: &str = r#"#!/bin/sh
here="$(dirname "$0")"
echo "$*" >> "$here/calls"
echo "$$" > "$here/pid"
env > "$here/env"
[ "$1" = reflow-run ] || { echo "agent-browser: unknown command: $1" >&2; exit 2; }
shift
click=
while [ $# -gt 0 ]; do
  case "$1" in
    --cdp) shift 2 ;;
    --url) url="$2"; shift 2 ;;
    --package) package="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --timeout) shift 2 ;;
    --click=*) click="${1#--click=}"; shift ;;
    *) echo "agent-browser: unknown option: $1" >&2; exit 2 ;;
  esac
done
[ -f "$package/refresh.mjs" ] && [ -f "$package/manifest.json" ] || { echo "agent-browser: no refresh script: $package/refresh.mjs" >&2; exit 1; }
while [ -e "$here/hold" ]; do sleep 0.05; done
if [ -e "$here/hang" ]; then sleep 300 & sleep 300; fi
if [ -e "$here/fail" ]; then
  echo "Error: 页面改版了" >&2
  echo "    at refresh.mjs:1:1" >&2
  echo "agent-browser: reflow failed: 页面改版了" >&2
  exit 1
fi
printf '{"headline":"%s","click":"%s","script":"%s"}' "$url" "$click" "$(cat "$package/refresh.mjs")" > "$out"
"#;

        async fn until(what: &str, mut ok: impl FnMut() -> bool) {
            let started = Instant::now();
            while !ok() {
                assert!(started.elapsed() < Duration::from_secs(30), "timed out: {what}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        #[cfg(target_os = "linux")]
        fn alive(pid: u32) -> bool {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| stat.rsplit_once(')').is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z')))
        }

        /// The bucket's data plane as far as the module uses it. `owner` and
        /// `writer` read and write, `reader` only reads, anyone else is
        /// refused, all in the words of the real one.
        #[derive(Default)]
        struct FakeLibrary {
            objects: Mutex<BTreeMap<String, Vec<u8>>>,
            /// `(method, key, token)` of every request.
            requests: Mutex<Vec<(String, String, String)>>,
            /// While true, what a read found is not answered yet.
            hold_reads: tokio::sync::watch::Sender<bool>,
        }

        async fn serve(State(library): State<Arc<FakeLibrary>>, method: Method, UrlPath(path): UrlPath<String>, headers: HeaderMap, body: Bytes) -> Response {
            let refuse = |status: StatusCode, code: &str, message: &str| (status, axum::Json(json!({ "error": code, "message": message }))).into_response();
            let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
            let token = bearer.unwrap_or_default().to_string();
            let Some(key) = path.strip_prefix(&format!("{BUCKET}/")) else {
                return refuse(StatusCode::NOT_FOUND, "bucket_not_found", &path);
            };
            let may_write = match token.as_str() {
                "owner" | "writer" => true,
                "reader" => false,
                _ => {
                    library.requests.lock().unwrap().push((method.to_string(), key.to_string(), token.clone()));
                    return refuse(StatusCode::UNAUTHORIZED, "token_invalid", "invalid or expired token");
                }
            };
            let found = library.objects.lock().unwrap().get(key).cloned();
            library.requests.lock().unwrap().push((method.to_string(), key.to_string(), token));
            match method {
                Method::GET => {
                    let _ = library.hold_reads.subscribe().wait_for(|held| !*held).await;
                    match found {
                        Some(content) => content.into_response(),
                        None => refuse(StatusCode::NOT_FOUND, "no_such_key", key),
                    }
                }
                Method::PUT if may_write => {
                    library.objects.lock().unwrap().insert(key.to_string(), body.to_vec());
                    (StatusCode::CREATED, axum::Json(json!({ "key": key, "created": true }))).into_response()
                }
                Method::PUT => refuse(StatusCode::FORBIDDEN, "permission_denied", &format!("app:reader@bob may not write {BUCKET}")),
                _ => refuse(StatusCode::BAD_REQUEST, "invalid_argument", "method"),
            }
        }

        impl FakeLibrary {
            /// Serve it; a caller without a token acts as `writer` (the service's own identity).
            async fn start() -> (Arc<Self>, Library) {
                let library = Arc::new(Self::default());
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let origin = format!("http://{}", listener.local_addr().unwrap());
                let app = axum::Router::new().route("/bucket/v1/{*path}", axum::routing::any(serve)).with_state(library.clone());
                tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
                (library, Library { origin, bucket_id: BUCKET.to_string(), token: Some("writer".to_string()) })
            }

            fn put(&self, key: &str, content: impl ToString) {
                self.objects.lock().unwrap().insert(key.to_string(), content.to_string().into_bytes());
            }

            /// The package of `www.example.com`, whose script is `script`.
            fn package(&self, script: &str, timeout_s: u64) {
                self.put(&format!("{SITE}/refresh.mjs"), script);
                self.put(&format!("{SITE}/manifest.json"), manifest(timeout_s));
            }

            fn json(&self, key: &str) -> Option<Value> {
                self.objects.lock().unwrap().get(key).map(|content| serde_json::from_slice(content).unwrap())
            }

            /// The tokens the requests of `method` came with.
            fn tokens(&self, method: &str) -> Vec<String> {
                self.requests.lock().unwrap().iter().filter(|(m, _, _)| m == method).map(|(_, _, token)| token.clone()).collect()
            }
        }

        fn deps(root: &Path) -> BrowserDeps {
            BrowserDeps {
                agent: Arc::new(libopendan::FsAgentStateClient::open(root, AGENT, None, None).unwrap()),
                supervisor: None,
                owner_did: Some("did:bns:bob".to_string()),
                agent_root: root.to_path_buf(),
            }
        }

        struct Bench {
            tmp: tempfile::TempDir,
            fake: Arc<FakeBrowser>,
            library: Arc<FakeLibrary>,
            module: Arc<BrowserModule>,
        }

        impl Bench {
            async fn new(idle_stop_secs: u64) -> Self {
                Self::with(BrowserSection { enabled: true, idle_stop_secs, max_tabs: 12 }, |_| {}).await
            }

            async fn with(cfg: BrowserSection, tune: impl FnOnce(&mut Tuning)) -> Self {
                use std::os::unix::fs::PermissionsExt;
                let tmp = tempfile::tempdir().unwrap();
                let dirs = BrowserDirs::at(tmp.path().join("browser"));
                let fake = FakeBrowser::start(&dirs.root).await;
                let (library, at) = FakeLibrary::start().await;
                let agent_browser = tmp.path().join("tool").join("agent-browser");
                std::fs::create_dir_all(agent_browser.parent().unwrap()).unwrap();
                std::fs::write(&agent_browser, AGENT_BROWSER).unwrap();
                std::fs::set_permissions(&agent_browser, std::fs::Permissions::from_mode(0o755)).unwrap();
                let mut tuning = Tuning {
                    chromium: Some(fake_browser::chromium()),
                    dirs: Some(dirs),
                    tick: Duration::from_millis(50),
                    agent_browser: Some(agent_browser),
                    reflow_library: Some(at),
                    ..Default::default()
                };
                tune(&mut tuning);
                let module = BrowserModule::with_tuning(cfg, deps(&tmp.path().join("agent_root")), tuning);
                Self { tmp, fake, library, module }
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

            async fn call(&self, token: &str, method: &str, params: Value) -> Result<Value, BrowserError> {
                rpc::call(&self.module, "did:bns:bob", Some(token), method, &params).await
            }

            async fn refresh(&self, token: &str) -> Value {
                self.call(token, "browser.reflow_refresh", json!({ "url": URL })).await.unwrap()
            }

            async fn state(&self) -> Value {
                self.call("owner", "browser.reflow_state", json!({ "url": URL })).await.unwrap()
            }

            /// The state once the refresh ended.
            async fn settled(&self) -> Value {
                let started = Instant::now();
                loop {
                    let state = self.state().await;
                    if state["refresh"]["status"] != "running" {
                        return state;
                    }
                    assert!(started.elapsed() < Duration::from_secs(30), "the refresh does not end");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }

        fn result_key() -> String {
            format!("{SITE}/results/{}.json", Address::of(URL).unwrap().page)
        }

        const IDLE: fn() -> Value = || json!({ "status": "idle", "error": null, "at_ms": null });

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn the_state_of_an_address_is_what_the_library_has() {
            let b = Bench::new(600).await;
            let page = Address::of(URL).unwrap().page;
            let made = |at: u64| json!({ "made_at": at, "made_by": AGENT, "source_url": NORMALIZED });
            let (fallback_view, package_view) = (
                format!("{SITE}/fallback/{page}.html"),
                format!("{SITE}/?url=https%3A%2F%2Fwww.example.com%2Fnews%2F1%3Fa%3D1&page={page}"),
            );

            // Nothing: no package, no result, no fallback page. The user's preference for a site without a package
            // (`preference.json`, the app's) is neither.
            let preference = json!({ "schema": "buckyos.reflow.preference/1", "preference": "只看正文", "updated_at": 1794600000000u64 });
            b.library.put(&format!("{SITE}/preference.json"), &preference);
            let nothing = json!({
                "url": NORMALIZED, "site": SITE, "page": page,
                "package": null, "result": null, "fallback": null, "refresh": IDLE(), "view": null,
            });
            assert_eq!(b.state().await, nothing);
            assert_eq!(b.call("owner", "browser.reflow_refresh", json!({ "url": URL })).await.unwrap_err().code, "reflow_no_package");

            // A package without a result: there is nothing to show yet.
            b.library.package("v1", 5);
            let state = b.state().await;
            assert_eq!(state["package"], json!({ "updated_at": 1794600000000u64, "author": { "kind": "agent", "id": AGENT }, "preference": "只看标题和正文" }));
            assert_eq!((&state["result"], &state["view"]), (&Value::Null, &Value::Null));

            // A package and its result: the package's page shows it.
            b.library.put(&result_key(), json!({ "schema": RESULT_SCHEMA, "source_url": NORMALIZED, "made_at": 1794600000001u64, "made_by": AGENT, "data": { "headline": "标题" } }));
            let state = b.state().await;
            assert_eq!(state["result"], made(1794600000001));
            assert_eq!(state["view"], package_view.as_str());

            // A fallback page beside them: the package's page still comes first.
            b.library.put(&format!("{SITE}/fallback/{page}.json"), json!({ "schema": "buckyos.reflow.fallback/1", "source_url": NORMALIZED, "made_at": 1794600000002u64, "made_by": AGENT }));
            let state = b.state().await;
            assert_eq!(state["fallback"], made(1794600000002));
            assert_eq!(state["view"], package_view.as_str());

            // No package for the address — it is for other paths, or its manifest breaks a rule: the fallback page shows.
            let elsewhere = b.call("owner", "browser.reflow_state", json!({ "url": "https://www.example.com/about" })).await.unwrap();
            assert_eq!((&elsewhere["package"], &elsewhere["view"]), (&Value::Null, &Value::Null));
            b.library.put(&format!("{SITE}/manifest.json"), json!({ "schema": "buckyos.reflow/1", "site": SITE }));
            let state = b.state().await;
            assert_eq!((&state["package"], &state["result"]), (&Value::Null, &made(1794600000001)));
            assert_eq!(state["view"], fallback_view.as_str());
            b.library.put(&format!("{SITE}/manifest.json"), "not json");
            assert_eq!(b.state().await["view"], fallback_view.as_str());

            // The library was read as the caller; a caller without a token reads as the service itself.
            assert!(b.library.tokens("GET").iter().all(|token| token == "owner"), "{:?}", b.library.tokens("GET"));
            rpc::call(&b.module, "did:bns:bob", None, "browser.reflow_state", &json!({ "url": URL })).await.unwrap();
            assert_eq!(b.library.tokens("GET").last().unwrap(), "writer");
            // What the Zone refuses is told in its words.
            let refused = b.call("stranger", "browser.reflow_state", json!({ "url": URL })).await.unwrap_err();
            assert_eq!(refused.code, "other");
            assert!(refused.message.contains(BUCKET) && refused.message.contains("token_invalid: invalid or expired token"), "{}", refused.message);

            // Addresses that have no reflow.
            for url in ["ftp://www.example.com/", "about:blank", "http://[::1]:8080/x", "www.example.com/news"] {
                let refused = b.call("owner", "browser.reflow_state", json!({ "url": url })).await.unwrap_err();
                assert_eq!(refused.code, "invalid_argument", "{url}");
            }
            assert_eq!(b.call("owner", "browser.reflow_state", json!({})).await.unwrap_err().code, "invalid_argument");
            assert!(b.fake.pid().is_none(), "the browser was not needed");
            let asked_for_preference = b.library.requests.lock().unwrap().iter().any(|(_, key, _)| key.ends_with("preference.json"));
            assert!(!asked_for_preference && b.library.json(&format!("{SITE}/preference.json")) == Some(preference));
        }

        /// What was made before shows whether or not the agent has a browser;
        /// making it again needs one.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn without_a_browser_the_library_is_still_told() {
            let off = BrowserSection { enabled: false, idle_stop_secs: 600, max_tabs: 12 };
            let on = BrowserSection { enabled: true, ..off.clone() };
            for (why, cfg, chromium) in [("switched off", off, None), ("no chromium on the node", on, Some("/nonexistent/chromium"))] {
                let b = Bench::with(cfg, |t| t.chromium = chromium.map(PathBuf::from).or(t.chromium.take())).await;
                assert_eq!(b.module.usable().unwrap_err().code, "browser_unavailable", "{why}");
                b.library.package("v1", 5);
                b.library.put(&result_key(), json!({ "schema": RESULT_SCHEMA, "source_url": NORMALIZED, "made_at": 1794600000001u64, "made_by": AGENT, "data": {} }));
                let state = b.state().await;
                assert!(state["package"].is_object() && state["result"].is_object(), "{why}: {state}");
                assert_eq!(state["refresh"], IDLE(), "{why}");
                assert_eq!(state["view"], Address::of(URL).unwrap().view(true, false).unwrap().as_str(), "{why}");
                for (method, params) in [
                    ("browser.reflow_refresh", json!({ "url": URL })),
                    ("browser.reflow_action", json!({ "url": URL, "selector": "#more" })),
                ] {
                    assert_eq!(b.call("owner", method, params).await.unwrap_err().code, "browser_unavailable", "{why}: {method}");
                }
                assert!(b.calls().is_empty() && b.fake.pid().is_none(), "{why}: nothing was started");
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_refresh_runs_the_packages_script_in_a_background_tab_and_keeps_what_it_returns() {
            let b = Bench::new(600).await;
            b.library.package("v1", 5);
            b.library.put(&format!("{SITE}/preference.json"), "left by the app");
            // What the service itself holds.
            std::env::set_var(ENV_BROWSER_RPC, "http://127.0.0.1:9/kapi/jarvis");
            std::env::set_var("BUCKYOS_APPCLIENT_SESSION_TOKEN", "the-services-own-token");
            b.flag("hold", true);
            let before = now_ms();
            let state = b.refresh("owner").await;
            assert_eq!((&state["refresh"]["status"], &state["refresh"]["error"]), (&json!("running"), &Value::Null));
            assert!(state["refresh"]["at_ms"].as_u64().unwrap() >= before);
            assert!(state["package"].is_object() && state["result"].is_null() && state["view"].is_null(), "{state}");

            // Asked again while it runs, by anyone and however the address is spelled: the same refresh.
            let again = b.call("writer", "browser.reflow_refresh", json!({ "url": " HTTPS://WWW.example.com:443/news/1?a=1#again " })).await.unwrap();
            assert_eq!(again["refresh"], state["refresh"]);
            assert_eq!(b.state().await["refresh"], state["refresh"]);
            until("the command runs", || b.calls().len() == 1).await;
            let pid = b.fake.pid().expect("the refresh started the browser");

            // The tab the command opens is the module's own; a tab somebody else opens meanwhile is theirs.
            let tab = b.fake.open("about:blank", None);
            until("the command's tab is seen", || b.module.tab(&tab).is_some()).await;
            assert_eq!(b.module.tab(&tab).unwrap().group_id, GROUP);
            until("only that tab was expected", || b.module.shared().expected.is_empty()).await;
            let others = b.fake.open("https://other.example/", None);
            until("the other tab is seen", || b.module.tab(&others).is_some()).await;
            assert_eq!(b.module.tab(&others).unwrap().group_id, "default");
            let status = b.module.status(None, None).await;
            assert_eq!(status["groups"].as_array().unwrap().iter().map(|g| g["group_id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["default"]);

            b.fake.close(&tab);
            b.flag("hold", false);
            let state = b.settled().await;
            assert_eq!(state["refresh"], IDLE());
            assert_eq!(b.calls().len(), 1, "one refresh ran: {:?}", b.calls());
            assert_eq!(b.fake.pid(), Some(pid));

            // What the script returned is the page's result, made now by this agent.
            let result = b.library.json(&result_key()).unwrap();
            let made_at = result["made_at"].as_u64().unwrap();
            assert!(made_at >= before && made_at <= now_ms(), "{result}");
            assert_eq!(
                result,
                json!({
                    "schema": "buckyos.reflow.result/1",
                    "source_url": NORMALIZED,
                    "made_at": made_at,
                    "made_by": AGENT,
                    "data": { "headline": NORMALIZED, "click": "", "script": "v1" },
                })
            );
            assert_eq!(state["result"], json!({ "made_at": made_at, "made_by": AGENT, "source_url": NORMALIZED }));
            assert_eq!(state["view"], format!("{SITE}/?url=https%3A%2F%2Fwww.example.com%2Fnews%2F1%3Fa%3D1&page={}", state["page"].as_str().unwrap()).as_str());
            // Written as the caller that asked for it; the refresh touched the package's two files and the result, nothing else.
            assert_eq!(b.library.tokens("PUT"), vec!["owner"]);
            let touched: std::collections::BTreeSet<String> = b.library.requests.lock().unwrap().iter().map(|(_, key, _)| key.clone()).collect();
            let expected = [format!("{SITE}/manifest.json"), format!("{SITE}/refresh.mjs"), result_key(), format!("{SITE}/fallback/{}.json", state["page"].as_str().unwrap())];
            assert_eq!(touched, expected.into_iter().collect());
            assert_eq!(b.library.objects.lock().unwrap()[&format!("{SITE}/preference.json")], b"left by the app");

            // The command got the browser's endpoint, the address and the package.
            let call = b.calls().remove(0);
            let work = b.module.dirs().root.join("reflow").join(state["page"].as_str().unwrap());
            assert_eq!(
                call,
                format!(
                    "reflow-run --cdp http://127.0.0.1:{} --url {NORMALIZED} --package {} --out {} --timeout 5",
                    b.fake.port,
                    work.join("package").display(),
                    work.join("result.json").display()
                )
            );
            // Of the service's environment it got what it needs to run: not the service's token, nor its address.
            let env = std::fs::read_to_string(b.tool("env")).unwrap();
            std::env::remove_var(ENV_BROWSER_RPC);
            std::env::remove_var("BUCKYOS_APPCLIENT_SESSION_TOKEN");
            assert!(!env.contains("the-services-own-token") && !env.contains("BUCKYOS_") && !env.contains("OPENDAN_"), "{env}");
            let names: Vec<&str> = env.lines().filter_map(|line| line.split_once('=')).map(|(name, _)| name).collect();
            // `PWD`, `OLDPWD`, `SHLVL` and `_` are the shell's own.
            let kept = ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "HOME", "TMPDIR", "PWD", "OLDPWD", "SHLVL", "_"];
            assert!(names.iter().all(|name| kept.contains(name)), "{names:?}");
            for line in [format!("PATH={}", std::env::var("PATH").unwrap()), format!("HOME={}", work.join("home").display()), format!("TMPDIR={}", work.join("tmp").display())] {
                assert!(env.lines().any(|l| l == line), "{line} in {env}");
            }
            assert!(!work.exists(), "what the refresh worked in is gone");
            b.module.shutdown().await;
        }

        /// A refresh that ends while a state is put together: the state must
        /// not say "ended" beside a library read before the result was there
        /// (the app would take that for a refresh that made nothing).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_state_read_while_the_refresh_ends_still_says_running() {
            let b = Bench::new(600).await;
            b.library.package("v1", 5);
            b.flag("hold", true);
            b.refresh("owner").await;
            until("the refresh runs", || b.calls().len() == 1).await;

            // The library is read: no result yet ...
            let reads = b.library.tokens("GET").len();
            b.library.hold_reads.send_replace(true);
            let reading = {
                let m = b.module.clone();
                tokio::spawn(async move { m.reflow_state(Some("owner"), URL).await.unwrap() })
            };
            until("the library was read", || b.library.tokens("GET").len() == reads + 3).await;
            // ... and the refresh ends before the answer is put together.
            b.flag("hold", false);
            until("the refresh ended", || b.library.json(&result_key()).is_some() && b.module.reflow.runs().is_empty()).await;
            b.library.hold_reads.send_replace(false);
            let state = reading.await.unwrap();
            assert_eq!((&state["refresh"]["status"], &state["result"]), (&json!("running"), &Value::Null), "{state}");
            // Asked again, it ended and the result is there.
            let state = b.state().await;
            assert!(state["refresh"] == IDLE() && state["result"].is_object() && state["view"].is_string(), "{state}");
            b.module.shutdown().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_refresh_that_fails_says_why() {
            let b = Bench::new(600).await;
            let refresh = |url: &'static str| b.call("owner", "browser.reflow_refresh", json!({ "url": url }));

            // Nothing in the library is for the address.
            assert_eq!(refresh(URL).await.unwrap_err().code, "reflow_no_package");
            b.library.package("v1", 5);
            let elsewhere = refresh("https://www.example.com/about").await.unwrap_err();
            assert_eq!(elsewhere.code, "reflow_no_package");
            assert!(elsewhere.message.contains("https://www.example.com/about"), "{}", elsewhere.message);
            assert_eq!(refresh("file:///etc/passwd").await.unwrap_err().code, "invalid_argument");
            assert!(b.calls().is_empty() && b.fake.pid().is_none(), "nothing was started");

            // The script fails: the state has the reason, the command's last line.
            b.flag("fail", true);
            assert_eq!(refresh(URL).await.unwrap()["refresh"]["status"], "running");
            let before = now_ms();
            let state = b.settled().await;
            assert_eq!((&state["refresh"]["status"], &state["refresh"]["error"]), (&json!("failed"), &json!("agent-browser: reflow failed: 页面改版了")));
            assert!(state["refresh"]["at_ms"].as_u64().unwrap() >= before - 1000);
            assert!(state["result"].is_null() && b.library.json(&result_key()).is_none());

            // The next refresh starts over.
            b.flag("fail", false);
            let state = refresh(URL).await.unwrap();
            assert_eq!((&state["refresh"]["status"], &state["refresh"]["error"]), (&json!("running"), &Value::Null));
            let state = b.settled().await;
            assert_eq!(state["refresh"], IDLE());
            assert!(state["result"].is_object());

            // A package of pages only has nothing to run.
            b.library.objects.lock().unwrap().remove(&format!("{SITE}/refresh.mjs"));
            refresh(URL).await.unwrap();
            let state = b.settled().await;
            assert_eq!(state["refresh"]["status"], "failed");
            assert!(state["refresh"]["error"].as_str().unwrap().contains("has no refresh.mjs"), "{state}");
            assert_eq!(b.calls().len(), 2);

            // An action names what to click.
            assert_eq!(b.call("owner", "browser.reflow_action", json!({ "url": URL })).await.unwrap_err().code, "invalid_argument");
            // A node without `agent-browser` refreshes nothing; what the library has is still told.
            std::fs::remove_file(b.tool("agent-browser")).unwrap();
            assert_eq!(refresh(URL).await.unwrap_err().code, "browser_unavailable");
            assert_eq!(b.state().await["refresh"]["status"], "failed");
            b.module.shutdown().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_caller_that_may_not_write_the_library_is_told() {
            let b = Bench::new(600).await;
            b.library.package("v1", 5);
            // It may read: the refresh starts.
            assert_eq!(b.refresh("reader").await["refresh"]["status"], "running");
            let state = b.settled().await;
            assert_eq!(state["refresh"]["status"], "failed");
            assert_eq!(
                state["refresh"]["error"],
                format!("reflow library: {BUCKET}/{}: permission_denied: app:reader@bob may not write {BUCKET}", result_key()).as_str()
            );
            assert_eq!(b.library.tokens("PUT"), vec!["reader"]);
            assert!(b.library.json(&result_key()).is_none() && state["result"].is_null());
            b.module.shutdown().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn an_action_clicks_on_the_page_and_refreshes_it() {
            let b = Bench::new(600).await;
            b.library.package("v1", 5);
            let action = |selector: &'static str| b.call("owner", "browser.reflow_action", json!({ "url": URL, "selector": selector }));

            // While the page is refreshed an action is not lost: it is done right after, the latest of them.
            b.flag("hold", true);
            let state = b.refresh("owner").await;
            until("the refresh runs", || b.calls().len() == 1).await;
            assert_eq!(action("#first").await.unwrap()["refresh"], state["refresh"]);
            assert_eq!(action("#more").await.unwrap()["refresh"], state["refresh"]);
            b.flag("hold", false);
            let state = b.settled().await;
            assert_eq!(state["refresh"], IDLE());
            let calls = b.calls();
            assert_eq!(calls.len(), 2, "{calls:?}");
            assert!(calls[0].ends_with("--timeout 5") && calls[1].ends_with("--timeout 5 --click=#more"), "{calls:?}");
            assert_eq!(b.library.json(&result_key()).unwrap()["data"]["click"], "#more");

            // By itself.
            assert_eq!(action("a.next").await.unwrap()["refresh"]["status"], "running");
            b.settled().await;
            assert!(b.calls()[2].ends_with("--click=a.next"));
            assert_eq!(b.library.json(&result_key()).unwrap()["data"]["click"], "a.next");
            b.module.shutdown().await;
        }

        #[cfg(target_os = "linux")]
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_refresh_that_does_not_end_is_ended_with_what_it_started() {
            let b = Bench::new(600).await;
            b.library.package("v1", 1);
            b.flag("hang", true);
            b.refresh("owner").await;
            until("the command started what it starts", || {
                std::fs::read_to_string(b.tool("pid")).is_ok_and(|pid| process::group_pids(pid.trim().parse().unwrap()).len() == 3)
            })
            .await;
            let pid: u32 = std::fs::read_to_string(b.tool("pid")).unwrap().trim().parse().unwrap();
            let tab = b.fake.open(NORMALIZED, None);
            until("the command's tab is seen", || b.module.tab(&tab).is_some()).await;

            let state = b.settled().await;
            assert_eq!(state["refresh"]["status"], "failed");
            assert_eq!(state["refresh"]["error"], "the refresh did not end within 11 seconds and was ended");
            assert!(!alive(pid) && process::group_pids(pid).is_empty(), "nothing of the command is left");
            // The tab it left behind was closed.
            assert!(b.module.tab(&tab).is_none() && b.fake.targets().is_empty(), "{:?}", b.fake.targets());
            assert!(b.library.json(&result_key()).is_none());
            b.module.shutdown().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn the_browser_is_not_idle_while_a_refresh_works_in_it() {
            let b = Bench::new(1).await;
            let housekeeping = tokio::spawn(b.module.clone().run());
            b.library.package("v1", 5);
            b.flag("hold", true);
            b.refresh("owner").await;
            until("the refresh runs", || b.calls().len() == 1).await;
            tokio::time::sleep(Duration::from_millis(1600)).await;
            assert!(b.module.running(), "a browser a refresh works in is not stopped");
            b.flag("hold", false);
            assert!(b.settled().await["result"].is_object());
            until("the idle browser is stopped", || !b.module.running()).await;
            housekeeping.abort();
            b.module.shutdown().await;
        }

        /// The data plane of a real nfs-server (no authentication) with the
        /// bucket as control-panel declares it.
        async fn nfs_server(dir: &Path) -> Library {
            let mut config = nfs_server::ServerConfig::new(dir.join("nfs"), vec![]);
            config.named_store = nfs_server::config::NamedStoreSource::Dir(dir.join("store"));
            config.bucket_specs = serde_json::from_value(json!({
                (buckyos_api::CONTROL_PANEL_SERVICE_NAME): {
                    "buckets": buckyos_api::control_panel_bucket_decls(),
                    "storage_quota_mib": buckyos_api::CONTROL_PANEL_STORAGE_QUOTA_MIB,
                },
            }))
            .unwrap();
            let state = nfs_server::AppState::new(config).unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let app = nfs_server::build_router(state);
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            Library { origin, bucket_id: BUCKET.to_string(), token: None }
        }

        #[tokio::test]
        async fn the_library_is_the_bucket_on_nfs_servers_data_plane() {
            let tmp = tempfile::tempdir().unwrap();
            let library = nfs_server(tmp.path()).await;
            let http = reqwest::Client::new();
            let address = Address::of(URL).unwrap();

            let found = library.find(&http, &address).await.unwrap();
            assert!(found.package.is_none() && found.result.is_none() && found.fallback.is_none(), "an empty library");
            assert_eq!(library.get(&http, &format!("{SITE}/refresh.mjs")).await.unwrap(), None);

            let result = json!({ "schema": RESULT_SCHEMA, "source_url": NORMALIZED, "made_at": 1794600000001u64, "made_by": AGENT, "data": { "headline": "标题" } });
            library.put_json(&http, &format!("{SITE}/{MANIFEST_FILE}"), &manifest(5)).await.unwrap();
            library.put_json(&http, &result_key(), &result).await.unwrap();
            let found = library.find(&http, &address).await.unwrap();
            assert_eq!(found.package, Package::of(&manifest(5), SITE));
            assert_eq!(found.result, Some(json!({ "made_at": 1794600000001u64, "made_by": AGENT, "source_url": NORMALIZED })));
            assert_eq!(library.get_json(&http, result_key()).await.unwrap(), Some(result));

            // A bucket the Zone does not have is an error in the bucket's words, not an empty library.
            let undeclared = Library { bucket_id: "user/control-panel/bob/nope".to_string(), ..library.clone() };
            let refused = undeclared.find(&http, &address).await.err().unwrap();
            assert_eq!(refused.code, "other");
            assert!(refused.message.starts_with("reflow library: user/control-panel/bob/nope/"), "{}", refused.message);
            // nfs-server that is not there.
            let gone = Library { origin: "http://127.0.0.1:9".to_string(), ..library };
            assert!(gone.find(&http, &address).await.err().unwrap().message.contains("127.0.0.1:9"));
        }

        /// The real `agent-browser reflow-run` in a real Chromium, on a page
        /// of a local site, with the library on a real nfs-server:
        ///
        /// ```sh
        /// OPENDAN_CHROMIUM=<chrome> OPENDAN_AGENT_BROWSER=<launcher> \
        ///   cargo test -p opendan --lib browser::reflow -- --ignored --test-threads=1
        /// ```
        ///
        /// `<launcher>` is `agent-browser` of an installed ext-core, or for
        /// the sources a script like
        /// `NODE_PATH=<dir with playwright-core>/node_modules exec node publish/exttool/ext-core/browser/agent-browser.mjs "$@"`.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        #[ignore = "needs OPENDAN_CHROMIUM and OPENDAN_AGENT_BROWSER"]
        async fn the_real_agent_browser_refreshes_a_page_in_a_real_browser() {
            const PAGE: &str = "<!doctype html><meta charset=utf-8><title>News</title><h1 id=headline>原标题</h1>\
                <button id=more onclick=\"document.querySelector('#headline').textContent = '点过之后的标题'\">more</button>";
            const SCRIPT: &str = "export default async ({ page, url }) => ({ url, title: await page.title(), headline: await page.locator('#headline').textContent(), \
                held: Object.keys(process.env).filter((name) => /^(BUCKYOS|OPENDAN)_/.test(name)) });\n";
            let chromium = std::env::var_os(process::ENV_CHROMIUM).map(PathBuf::from).expect("set OPENDAN_CHROMIUM to a Chromium executable");
            assert!(find_agent_browser().is_some(), "set OPENDAN_AGENT_BROWSER to an agent-browser launcher");

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/news/1", listener.local_addr().unwrap());
            let site = axum::Router::new().route("/news/{id}", axum::routing::get(|| async { axum::response::Html(PAGE) }));
            tokio::spawn(async move { axum::serve(listener, site).await.unwrap() });

            let tmp = tempfile::tempdir().unwrap();
            let library = nfs_server(tmp.path()).await;
            let http = reqwest::Client::new();
            let address = Address::of(&url).unwrap();
            assert_eq!(address.site, "127-0-0-1");
            let put_script = |script: &'static str| {
                let request = library.request(&http, reqwest::Method::PUT, "127-0-0-1/refresh.mjs").header("content-type", "text/javascript");
                async move { assert!(request.body(script).send().await.unwrap().status().is_success()) }
            };
            let mut package = manifest(20);
            package["site"] = json!("127-0-0-1");
            package["match"] = json!([{ "host": "127.0.0.1", "path": "/news/*" }]);
            library.put_json(&http, "127-0-0-1/manifest.json", &package).await.unwrap();
            put_script(SCRIPT).await;

            let tuning = Tuning {
                chromium: Some(chromium),
                dirs: Some(BrowserDirs::at(tmp.path().join("browser"))),
                reflow_library: Some(library.clone()),
                ..Default::default()
            };
            let cfg = BrowserSection { enabled: true, idle_stop_secs: 600, max_tabs: 12 };
            let m = BrowserModule::with_tuning(cfg, deps(&tmp.path().join("agent_root")), tuning);
            let settled = || async {
                let started = Instant::now();
                loop {
                    let state = m.reflow_state(None, &url).await.unwrap();
                    if state["refresh"]["status"] != "running" {
                        return state;
                    }
                    assert!(started.elapsed() < Duration::from_secs(60), "the refresh does not end");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            };
            let result = || async { library.get_json(&http, format!("127-0-0-1/results/{}.json", address.page)).await.unwrap().unwrap() };

            // What the service holds in its environment does not reach the package's script.
            std::env::set_var("BUCKYOS_APPCLIENT_SESSION_TOKEN", "the-services-own-token");
            // A user's tab is open meanwhile and stays as it is; the refresh's tab is the module's own.
            let users = m.open_tab("default", Some("data:text/html,<title>Users</title>")).await.unwrap();
            let tabs = Arc::new(Mutex::new(Vec::new()));
            let watching = {
                let (m, tabs) = (m.clone(), tabs.clone());
                tokio::spawn(async move {
                    let mut changes = m.changes();
                    while changes.changed().await.is_ok() {
                        tabs.lock().unwrap().push((m.group(GROUP).tabs.len(), m.group("default").tabs.len()));
                    }
                })
            };
            let before = now_ms();
            assert_eq!(m.reflow_refresh(None, &url, None).await.unwrap()["refresh"]["status"], "running");
            let state = settled().await;
            assert_eq!(state["refresh"], IDLE(), "{state}");
            watching.abort();
            let tabs = tabs.lock().unwrap().clone();
            assert!(tabs.contains(&(1, 1)) && tabs.iter().all(|(_, users)| *users == 1), "`(_reflow, default)` tabs while it ran: {tabs:?}");
            let made = result().await;
            assert_eq!(made["data"], json!({ "url": url, "title": "News", "headline": "原标题", "held": [] }));
            assert_eq!((&made["schema"], &made["source_url"], &made["made_by"]), (&json!(RESULT_SCHEMA), &json!(url), &json!(AGENT)));
            assert!(made["made_at"].as_u64().unwrap() >= before);
            assert_eq!(state["view"], address.view(true, false).unwrap().as_str());
            until("the refresh's tab is gone", || m.group(GROUP).tabs.is_empty()).await;
            assert_eq!(m.group("default").tabs.iter().map(|t| t.tab_id.clone()).collect::<Vec<_>>(), vec![users.tab_id.clone()]);

            // An action clicks on the page first.
            m.reflow_refresh(None, &url, Some("#more".to_string())).await.unwrap();
            assert_eq!(settled().await["refresh"], IDLE());
            assert_eq!(result().await["data"]["headline"], "点过之后的标题");

            // A script that fails: its reason is the state's.
            put_script("export default async () => { throw new Error('页面改版了'); };\n").await;
            m.reflow_refresh(None, &url, None).await.unwrap();
            let state = settled().await;
            assert_eq!((&state["refresh"]["status"], &state["refresh"]["error"]), (&json!("failed"), &json!("agent-browser: reflow failed: 页面改版了")));
            assert_eq!(result().await["data"]["headline"], "点过之后的标题", "the result that was there stays");

            std::env::remove_var("BUCKYOS_APPCLIENT_SESSION_TOKEN");
            assert!(m.group(GROUP).tabs.is_empty() && m.group("default").tabs.len() == 1);
            assert!(!m.dirs().root.join("reflow").join(&address.page).exists());
            m.shutdown().await;
        }
    }
}
