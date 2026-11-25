//! Reflow ([Agent Browser Protocol] §5): a page of a site shown from the
//! owner's reflow library — the bucket `user/control-panel/<owner>/reflow` —
//! instead of the site itself. The Agent Service tells what the library has
//! for an address (`browser.reflow_state`): the fallback page an agent made
//! of it. That needs no browser.
//!
//! The library is read with the caller's session token: what a caller may
//! read there is what the Zone lets it read.
//!
//! [Agent Browser Protocol]: ../../../../../doc/opendan/protocol/Agent%20Browser%20Protocol.md

use std::sync::OnceLock;
use std::time::Duration;

use buckyos_api::bucket_client::BucketId;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{BrowserError, BrowserModule};

// ---------------------------------------------------------------------------
// The rules (§5.1), shared with the `reflow` command and checked against
// `doc/opendan/protocol/fixtures/reflow_vectors.json`
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

/// An address as the library names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Address {
    /// The normalized URL.
    pub url: String,
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
        let site = site_of_host(url.host_str()?)?;
        let page = hex::encode(&Sha256::digest(url.as_str().as_bytes())[..8]);
        Some(Self { url: url.into(), site, page })
    }

    /// Its fallback page, relative to `reflow.<zone>`.
    pub fn fallback_view(&self) -> String {
        format!("{}/fallback/{}.html", self.site, self.page)
    }
}

/// When, by whom and of what a fallback page was made.
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

/// The owner's library, and as whom it is read.
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

    /// A key's content as JSON. `None`: the key is not there, or it is not JSON.
    async fn get_json(&self, http: &reqwest::Client, key: &str) -> Result<Option<Value>, BrowserError> {
        let Some(response) = self.send(self.request(http, reqwest::Method::GET, key), key).await? else { return Ok(None) };
        let bytes = response.bytes().await.map_err(|e| library_error(format!("{}/{key}: {e}", self.bucket_id)))?;
        Ok(serde_json::from_slice(&bytes).ok())
    }
}

/// What the library is read with.
#[derive(Default)]
pub(super) struct Reflow {
    http: OnceLock<reqwest::Client>,
}

impl Reflow {
    fn http(&self) -> &reqwest::Client {
        self.http.get_or_init(|| {
            let builder = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60));
            builder.build().unwrap_or_default()
        })
    }
}

impl BrowserModule {
    /// The owner's library as a caller with `token` reads it.
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

    /// `browser.reflow_state`: what the library has for `url`. Needs no
    /// browser: an agent whose browser is switched off or missing still
    /// tells which page shows what was made before.
    pub async fn reflow_state(&self, token: Option<&str>, url: &str) -> Result<Value, BrowserError> {
        let address = Address::of(url).ok_or_else(|| BrowserError::invalid(format!("`{url}` has no reflow: only http(s) pages of a named host have")))?;
        let library = self.reflow_library(token).await?;
        let meta = library.get_json(self.reflow.http(), &format!("{}/fallback/{}.json", address.site, address.page)).await?;
        let fallback = meta.as_ref().and_then(made);
        Ok(json!({
            "view": fallback.is_some().then(|| address.fallback_view()),
            "fallback": fallback,
            "url": address.url,
            "site": address.site,
            "page": address.page,
        }))
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

    #[test]
    fn the_rules_pass_the_shared_vectors() {
        let vectors: Value = serde_json::from_str(include_str!("../../../../../doc/opendan/protocol/fixtures/reflow_vectors.json")).unwrap();
        let text = |value: &Value| value.as_str().unwrap().to_string();
        let (normalize, invalid) = (vectors["normalize"].as_array().unwrap(), vectors["invalid"].as_array().unwrap());
        assert!(normalize.len() >= 25 && invalid.len() >= 9, "the vectors are all there");
        for case in normalize {
            let address = Address::of(case["url"].as_str().unwrap()).unwrap_or_else(|| panic!("{case} has a reflow page"));
            assert_eq!((&address.url, &address.site, &address.page), (&text(&case["normalized"]), &text(&case["site"]), &text(&case["page"])), "{case}");
            assert_eq!(address.fallback_view(), format!("{}/fallback/{}.html", address.site, address.page));
        }
        for url in invalid {
            assert_eq!(Address::of(url.as_str().unwrap()), None, "{url}");
        }
    }

    /// Against a fake browser (`tests/fake_browser`) and a fake data plane
    /// of the bucket; the library's wire also against a real nfs-server.
    #[cfg(unix)]
    mod telling {
        use std::collections::BTreeMap;
        use std::path::{Path, PathBuf};
        use std::sync::{Arc, Mutex};

        use axum::extract::{Path as UrlPath, State};
        use axum::http::{header, HeaderMap, StatusCode};
        use axum::response::{IntoResponse, Response};

        use super::super::fake_browser::{self, FakeBrowser};
        use super::*;
        use crate::browser::{rpc, BrowserDeps, BrowserDirs, Tuning};
        use crate::config::BrowserSection;

        const BUCKET: &str = "user/control-panel/bob/reflow";

        /// The bucket's data plane as far as the module uses it: `owner`
        /// and `agent` read, anyone else is refused, in the words of the
        /// real one.
        #[derive(Default)]
        struct FakeLibrary {
            objects: Mutex<BTreeMap<String, Vec<u8>>>,
            /// `(key, token)` of every request.
            requests: Mutex<Vec<(String, String)>>,
        }

        async fn serve(State(library): State<Arc<FakeLibrary>>, UrlPath(path): UrlPath<String>, headers: HeaderMap) -> Response {
            let refuse = |status: StatusCode, code: &str, message: &str| (status, axum::Json(json!({ "error": code, "message": message }))).into_response();
            let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
            let token = bearer.unwrap_or_default().to_string();
            let Some(key) = path.strip_prefix(&format!("{BUCKET}/")) else {
                return refuse(StatusCode::NOT_FOUND, "bucket_not_found", &path);
            };
            library.requests.lock().unwrap().push((key.to_string(), token.clone()));
            if !matches!(token.as_str(), "owner" | "agent") {
                return refuse(StatusCode::UNAUTHORIZED, "token_invalid", "invalid or expired token");
            }
            match library.objects.lock().unwrap().get(key).cloned() {
                Some(content) => content.into_response(),
                None => refuse(StatusCode::NOT_FOUND, "no_such_key", key),
            }
        }

        impl FakeLibrary {
            /// Serve it; a caller without a token acts as `agent` (the service's own identity).
            async fn start() -> (Arc<Self>, Library) {
                let library = Arc::new(Self::default());
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let origin = format!("http://{}", listener.local_addr().unwrap());
                let app = axum::Router::new().route("/bucket/v1/{*path}", axum::routing::get(serve)).with_state(library.clone());
                tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
                (library, Library { origin, bucket_id: BUCKET.to_string(), token: Some("agent".to_string()) })
            }

            fn put(&self, key: &str, content: impl ToString) {
                self.objects.lock().unwrap().insert(key.to_string(), content.to_string().into_bytes());
            }

            /// The tokens the requests came with.
            fn tokens(&self) -> Vec<String> {
                self.requests.lock().unwrap().iter().map(|(_, token)| token.clone()).collect()
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
            _tmp: tempfile::TempDir,
            fake: Arc<FakeBrowser>,
            library: Arc<FakeLibrary>,
            module: Arc<BrowserModule>,
        }

        impl Bench {
            async fn new(cfg: BrowserSection, tune: impl FnOnce(&mut Tuning)) -> Self {
                let tmp = tempfile::tempdir().unwrap();
                let dirs = BrowserDirs::at(tmp.path().join("browser"));
                let fake = FakeBrowser::start(&dirs.root).await;
                let (library, at) = FakeLibrary::start().await;
                let mut tuning = Tuning { chromium: Some(fake_browser::chromium()), dirs: Some(dirs), reflow_library: Some(at), ..Default::default() };
                tune(&mut tuning);
                let module = BrowserModule::with_tuning(cfg, deps(&tmp.path().join("agent_root")), tuning);
                Self { _tmp: tmp, fake, library, module }
            }

            async fn state(&self, token: Option<&str>, url: &str) -> Result<Value, BrowserError> {
                rpc::call(&self.module, "did:bns:bob", token, "browser.reflow_state", &json!({ "url": url })).await
            }
        }

        fn section(enabled: bool) -> BrowserSection {
            BrowserSection { enabled, display: crate::browser::display::DisplayMode::Headless, ..Default::default() }
        }

        fn fallback(made_at: u64) -> Value {
            json!({ "schema": "buckyos.reflow.fallback/1", "source_url": NORMALIZED, "made_at": made_at, "made_by": AGENT })
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn the_state_of_an_address_is_what_the_library_has() {
            let b = Bench::new(section(true), |_| {}).await;
            let page = Address::of(URL).unwrap().page;
            let meta = format!("{SITE}/fallback/{page}.json");
            let nothing = json!({ "url": NORMALIZED, "site": SITE, "page": page, "fallback": null, "view": null });

            // Nothing was made of the page. What a library may still hold of the packages there were is not looked at.
            b.library.put(&format!("{SITE}/manifest.json"), json!({ "schema": "buckyos.reflow/1", "site": SITE }));
            b.library.put(&format!("{SITE}/results/{page}.json"), json!({ "source_url": NORMALIZED, "made_at": 1794600000001u64, "made_by": AGENT, "data": {} }));
            b.library.put(&format!("{SITE}/preference.json"), "left by the app");
            assert_eq!(b.state(Some("owner"), URL).await.unwrap(), nothing);

            // A fallback page: it is what shows, however the address is spelled.
            b.library.put(&meta, fallback(1794600000002));
            let made = json!({
                "url": NORMALIZED, "site": SITE, "page": page,
                "fallback": { "made_at": 1794600000002u64, "made_by": AGENT, "source_url": NORMALIZED },
                "view": format!("{SITE}/fallback/{page}.html"),
            });
            assert_eq!(b.state(Some("owner"), URL).await.unwrap(), made);
            assert_eq!(b.state(Some("owner"), " HTTPS://WWW.example.com:443/news/1?a=1#again ").await.unwrap(), made);
            assert_eq!(b.state(Some("owner"), "https://www.example.com/about").await.unwrap()["view"], Value::Null, "another page");
            // What says nothing of how the page was made is no fallback page.
            for broken in ["not json".to_string(), json!({ "schema": "buckyos.reflow.fallback/1", "made_at": 1 }).to_string()] {
                b.library.put(&meta, broken);
                assert_eq!(b.state(Some("owner"), URL).await.unwrap(), nothing);
            }
            b.library.put(&meta, fallback(1794600000002));

            // The one key of the page was read, as the caller; a caller without a token reads as the service itself.
            let asked = b.library.requests.lock().unwrap().clone();
            assert!(asked.iter().all(|(key, token)| token == "owner" && key.starts_with(&format!("{SITE}/fallback/")) && key.ends_with(".json")), "{asked:?}");
            assert_eq!(b.state(None, URL).await.unwrap(), made);
            assert_eq!(b.library.tokens().last().unwrap(), "agent");
            // What the Zone refuses is told in its words.
            let refused = b.state(Some("stranger"), URL).await.unwrap_err();
            assert_eq!(refused.code, "other");
            assert!(refused.message.contains(&format!("{BUCKET}/{meta}")) && refused.message.contains("token_invalid: invalid or expired token"), "{}", refused.message);

            // Addresses that have no reflow.
            for url in ["ftp://www.example.com/", "about:blank", "http://[::1]:8080/x", "www.example.com/news"] {
                assert_eq!(b.state(Some("owner"), url).await.unwrap_err().code, "invalid_argument", "{url}");
            }
            let missing = rpc::call(&b.module, "did:bns:bob", Some("owner"), "browser.reflow_state", &json!({})).await.unwrap_err();
            assert_eq!(missing.code, "invalid_argument");
            assert!(b.fake.pid().is_none(), "the browser was not needed");
        }

        /// What was made before shows whether or not the agent has a browser.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn without_a_browser_the_library_is_still_told() {
            let tools = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(tools.path().join("bin")).unwrap();
            std::fs::copy(fake_browser::chromium(), tools.path().join("bin/chromium")).unwrap();
            let tool_dirs = vec![tools.path().join("bin")];
            for (why, enabled, chromium, code) in [
                ("switched off", false, None, "disabled"),
                ("no chromium on the node", true, Some("/nonexistent/chromium"), "no_package"),
                ("the browser is not downloaded", true, None, "not_downloaded"),
            ] {
                let tool_dirs = tool_dirs.clone();
                let b = Bench::new(section(enabled), |t| match code {
                    "not_downloaded" => (t.chromium, t.tool_dirs) = (None, Some(tool_dirs)),
                    _ => t.chromium = chromium.map(PathBuf::from).or(t.chromium.take()),
                })
                .await;
                let unusable = b.module.usable().unwrap_err();
                assert_eq!((unusable.code, unusable.unavailable_code), ("browser_unavailable", Some(code)), "{why}");
                let address = Address::of(URL).unwrap();
                b.library.put(&format!("{SITE}/fallback/{}.json", address.page), fallback(1794600000001));
                let state = b.state(Some("owner"), URL).await.unwrap();
                assert!(state["fallback"].is_object(), "{why}: {state}");
                assert_eq!(state["view"], address.fallback_view().as_str(), "{why}");
                assert!(b.fake.pid().is_none(), "{why}: nothing was started");
            }
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
            let key = format!("{SITE}/fallback/{}.json", Address::of(URL).unwrap().page);

            assert_eq!(library.get_json(&http, &key).await.unwrap(), None, "an empty library");
            let put = library.request(&http, reqwest::Method::PUT, &key).header(reqwest::header::CONTENT_TYPE, "application/json").body(fallback(1794600000001).to_string());
            assert!(put.send().await.unwrap().status().is_success());
            assert_eq!(library.get_json(&http, &key).await.unwrap(), Some(fallback(1794600000001)));

            // A bucket the Zone does not have is an error in the bucket's words, not an empty library.
            let undeclared = Library { bucket_id: "user/control-panel/bob/nope".to_string(), ..library.clone() };
            let refused = undeclared.get_json(&http, &key).await.unwrap_err();
            assert_eq!(refused.code, "other");
            assert!(refused.message.starts_with("reflow library: user/control-panel/bob/nope/"), "{}", refused.message);
            // nfs-server that is not there.
            let gone = Library { origin: "http://127.0.0.1:9".to_string(), ..library };
            assert!(gone.get_json(&http, &key).await.unwrap_err().message.contains("127.0.0.1:9"));
        }
    }
}
