//! What a headless browser is to the sites it visits. Headless Chromium
//! names itself `HeadlessChrome` in its user agent, which sites take for a
//! robot; a user agent given on its command line instead leaves its client
//! hints (`navigator.userAgentData`, `Sec-CH-UA-*`) empty, which no browser
//! does. So the browser's own identity is read once, from a page of its own,
//! and every page, frame and worker it starts gets it with that one word
//! changed (`Chrome`), before its first request: a connection of its own
//! attaches to each of them as it starts, and lets it go on once that is set.
//!
//! The same connection makes passkey requests fail at once, as they do when
//! the user cancels them: a headless browser has no passkey dialog, so a
//! page that asks for one would wait for good; refused, a site offers
//! another way to sign in.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::watch;

use super::cdp::{CdpClient, Scope};

/// A page of the browser itself: a secure context, which client hints need.
const PROBE_PAGE: &str = "chrome://version";
const HINTS: &[&str] = &["architecture", "bitness", "formFactors", "fullVersionList", "model", "platformVersion", "uaFullVersion", "wow64"];
const WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub user_agent: String,
    /// `userAgentMetadata`; `None`: the client hints could not be read.
    pub metadata: Option<Value>,
}

impl Identity {
    /// The identity of a browser whose user agent is `user_agent` and whose
    /// pages read `hints` (`getHighEntropyValues`), without `HeadlessChrome`.
    pub fn of(user_agent: &str, hints: Option<&Value>) -> Self {
        let metadata = hints.and_then(|hints| {
            let text = |name: &str| hints[name].as_str().unwrap_or_default().to_string();
            let brands = |name: &str| hints[name].as_array().filter(|list| !list.is_empty()).cloned();
            Some(json!({
                "brands": brands("brands")?,
                "fullVersionList": brands("fullVersionList")?,
                "fullVersion": text("uaFullVersion"),
                "platform": hints["platform"].as_str()?,
                "platformVersion": text("platformVersion"),
                "architecture": text("architecture"),
                "model": text("model"),
                "mobile": hints["mobile"].as_bool().unwrap_or(false),
                "bitness": text("bitness"),
                "wow64": hints["wow64"].as_bool().unwrap_or(false),
                "formFactors": hints["formFactors"].as_array().cloned().unwrap_or_default(),
            }))
        });
        Self { user_agent: user_agent.replace("HeadlessChrome/", "Chrome/"), metadata }
    }

    /// Read it from the browser `cdp` is connected to, on a page of a
    /// browsing context of its own (gone again before this returns).
    pub async fn read(cdp: &CdpClient) -> Result<Self, String> {
        let version = cdp.send_within(None, "Browser.getVersion", json!({}), WAIT).await.map_err(|e| e.to_string())?;
        let user_agent = version["userAgent"].as_str().ok_or("the browser names no user agent")?;
        let hints = match read_hints(cdp).await {
            Ok(hints) => Some(hints),
            Err(e) => {
                log::warn!("browser: its client hints were not read ({e}); pages get its user agent without them");
                None
            }
        };
        Ok(Self::of(user_agent, hints.as_ref()))
    }

    fn override_params(&self) -> Value {
        let mut params = json!({ "userAgent": self.user_agent });
        if let Some(metadata) = &self.metadata {
            params["userAgentMetadata"] = metadata.clone();
        }
        params
    }
}

async fn read_hints(cdp: &CdpClient) -> Result<Value, String> {
    let fail = |e: super::cdp::CdpError| e.to_string();
    let context = cdp.send_within(None, "Target.createBrowserContext", json!({}), WAIT).await.map_err(fail)?;
    let context = context["browserContextId"].as_str().ok_or("no browsing context")?.to_string();
    let read = async {
        let target = cdp.send_within(None, "Target.createTarget", json!({ "url": PROBE_PAGE, "browserContextId": context }), WAIT).await.map_err(fail)?;
        let target = target["targetId"].as_str().ok_or("no page")?;
        let session = cdp.send_within(None, "Target.attachToTarget", json!({ "targetId": target, "flatten": true }), WAIT).await.map_err(fail)?;
        let session = session["sessionId"].as_str().ok_or("no session")?;
        let expression = format!("navigator.userAgentData.getHighEntropyValues({}).then(JSON.stringify)", json!(HINTS));
        let mut answer = Value::Null;
        // The page may not be loaded yet when it is first asked.
        for _ in 0..20 {
            answer = cdp
                .send_within(Some(session), "Runtime.evaluate", json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }), WAIT)
                .await
                .map_err(fail)?;
            if answer["result"]["value"].is_string() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let text = answer["result"]["value"].as_str().ok_or_else(|| format!("the page did not tell: {answer}"))?;
        serde_json::from_str::<Value>(text).map_err(|e| e.to_string())
    }
    .await;
    if let Err(e) = cdp.send_within(None, "Target.disposeBrowserContext", json!({ "browserContextId": context }), WAIT).await {
        log::warn!("browser: the browsing context that read the client hints was not closed: {e}");
    }
    read
}

/// The pages that were dressed as they started.
pub struct Dressed {
    pages: Mutex<HashSet<String>>,
    changed: watch::Sender<u64>,
}

impl Dressed {
    fn new() -> Self {
        Self { pages: Mutex::new(HashSet::new()), changed: watch::channel(0).0 }
    }

    fn add(&self, page: &str) {
        self.pages.lock().expect("dressed").insert(page.to_string());
        self.changed.send_modify(|n| *n += 1);
    }

    pub fn forget(&self, page: &str) {
        self.pages.lock().expect("dressed").remove(page);
    }

    /// Returns once `page` is dressed, or `within` is over: whether it is.
    pub async fn wait(&self, page: &str, within: Duration) -> bool {
        let mut changed = self.changed.subscribe();
        let late = tokio::time::sleep(within);
        tokio::pin!(late);
        loop {
            if self.pages.lock().expect("dressed").contains(page) {
                return true;
            }
            tokio::select! {
                gone = changed.changed() => if gone.is_err() { return false },
                _ = &mut late => return false,
            }
        }
    }
}

/// Dress every target of the browser at `endpoint` as it starts: `identity`
/// (if any), passkeys refused. The task runs until the browser's connection closes.
pub async fn dress_all(endpoint: &str, identity: Option<Identity>) -> Result<(tokio::task::JoinHandle<()>, Arc<Dressed>), String> {
    let cdp = CdpClient::connect(endpoint).await.map_err(|e| e.to_string())?;
    let mut attached = cdp.subscribe(Scope::Any, &["Target.attachedToTarget"]);
    let identity = identity.map(Arc::new);
    let dressed = Arc::new(Dressed::new());
    let (dresser, done) = (cdp.clone(), dressed.clone());
    let task = tokio::spawn(async move {
        let mut lost = 0;
        while let Some(event) = attached.next().await {
            if attached.lagged > lost {
                lost = attached.lagged;
                log::warn!("browser: {lost} events of the dressing connection were lost: targets that started then may wait to start");
            }
            tokio::spawn(dress(dresser.clone(), event.params.clone(), identity.clone(), done.clone()));
        }
    });
    let auto = json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true });
    if let Err(e) = cdp.send_within(None, "Target.setAutoAttach", auto, WAIT).await {
        task.abort();
        return Err(e.to_string());
    }
    Ok((task, dressed))
}

async fn dress(cdp: Arc<CdpClient>, params: Value, identity: Option<Arc<Identity>>, dressed: Arc<Dressed>) {
    let Some(session) = params["sessionId"].as_str() else { return };
    let kind = params["targetInfo"]["type"].as_str().unwrap_or_default();
    let mut steps = Vec::new();
    if let Some(identity) = &identity {
        match kind {
            "page" | "iframe" => steps.push(("Emulation.setUserAgentOverride", identity.override_params())),
            "worker" | "shared_worker" | "service_worker" => steps.push(("Network.setUserAgentOverride", identity.override_params())),
            _ => {}
        }
    }
    // Its frames of other sites and its workers start as targets of their own.
    if matches!(kind, "page" | "iframe") {
        steps.push(("Target.setAutoAttach", json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true })));
    }
    for (method, params) in steps {
        if let Err(e) = cdp.send_within(Some(session), method, params, WAIT).await {
            log::debug!("browser: {method} of a {kind} as it started: {e}");
        }
    }
    if kind == "page" {
        if let Err(e) = refuse_passkeys(&cdp, session).await {
            log::debug!("browser: passkeys of a page are not refused: {e}");
        }
    }
    if params["waitingForDebugger"] == true {
        if let Err(e) = cdp.send_within(Some(session), "Runtime.runIfWaitingForDebugger", json!({}), WAIT).await {
            log::warn!("browser: a {kind} that started was not let go on: {e}");
        }
    }
    if kind == "page" {
        if let Some(page) = params["targetInfo"]["targetId"].as_str() {
            dressed.add(page);
        }
    }
}

/// The page's only authenticator is one that fails whatever it is asked:
/// a passkey request is refused at once, as when the user cancels it.
async fn refuse_passkeys(cdp: &CdpClient, session: &str) -> Result<(), super::cdp::CdpError> {
    cdp.send_within(Some(session), "WebAuthn.enable", json!({ "enableUI": false }), WAIT).await?;
    let options = json!({
        "protocol": "ctap2",
        "transport": "internal",
        "hasResidentKey": true,
        "hasUserVerification": true,
        "isUserVerified": false,
        "automaticPresenceSimulation": true,
    });
    let added = cdp.send_within(Some(session), "WebAuthn.addVirtualAuthenticator", json!({ "options": options }), WAIT).await?;
    let authenticator = added["authenticatorId"].as_str().unwrap_or_default();
    cdp.send_within(Some(session), "WebAuthn.setResponseOverrideBits", json!({ "authenticatorId": authenticator, "isBadUP": true, "isBadUV": true }), WAIT).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_is_the_browsers_own_without_headless() {
        let hints = json!({
            "architecture": "arm", "bitness": "64", "brands": [{ "brand": "Chromium", "version": "159" }, { "brand": "Not/A)Brand", "version": "8" }],
            "formFactors": ["Desktop"], "fullVersionList": [{ "brand": "Chromium", "version": "159.0.8154.6" }, { "brand": "Not/A)Brand", "version": "8.0.0.0" }],
            "mobile": false, "model": "", "platform": "macOS", "platformVersion": "15.8.1", "uaFullVersion": "159.0.8154.6", "wow64": false,
        });
        let agent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/159.0.0.0 Safari/537.36";
        let identity = Identity::of(agent, Some(&hints));
        assert_eq!(identity.user_agent, "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/159.0.0.0 Safari/537.36");
        let metadata = identity.metadata.clone().unwrap();
        assert_eq!(metadata["brands"], hints["brands"]);
        assert_eq!(metadata["fullVersionList"], hints["fullVersionList"]);
        assert_eq!((metadata["platform"].clone(), metadata["platformVersion"].clone(), metadata["architecture"].clone()), (json!("macOS"), json!("15.8.1"), json!("arm")));
        assert_eq!((metadata["fullVersion"].clone(), metadata["formFactors"].clone()), (json!("159.0.8154.6"), json!(["Desktop"])));
        assert_eq!(identity.override_params()["userAgentMetadata"], metadata);
        // Hints that were not read, or name no brands: the user agent alone.
        for hints in [None, Some(&json!({ "platform": "macOS", "brands": [] }))] {
            let identity = Identity::of(agent, hints);
            assert_eq!(identity.metadata, None);
            assert_eq!(identity.override_params(), json!({ "userAgent": identity.user_agent }));
        }
    }
}
