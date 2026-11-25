//! The pages the owner looked at in the agent's browser ([Agent Browser
//! Protocol] §7.3): what the agent that keeps the owner's personal site
//! learns from about what the owner still reads. A page counts while someone
//! looks at the screen and the user operates the browser, never while the
//! user enters a password the agent asked for. Pages are kept by their
//! address without query and fragment — neither ever reaches the disk — in
//! `<AgentRoot>/state/browser/visits.jsonl`, one line each; whether they are
//! recorded at all is the owner's switch, kept beside it in `visits.json`.
//!
//! [Agent Browser Protocol]: ../../../../../doc/opendan/protocol/Agent%20Browser%20Protocol.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::groups::{is_internal, Controller, Group, DEFAULT_GROUP};
use super::{now_ms, BrowserError, BrowserModule};

const KEPT_MS: u64 = 90 * 24 * 3600 * 1000;
const MAX_KEPT: usize = 5000;
const MAX_LISTED: usize = 500;
const WRITE_EVERY_MS: u64 = 30_000;
/// The page is looked at again every tick of the module: a longer step is the clock or the service
/// having stopped, not the owner reading.
const MAX_STEP_MS: u64 = 30_000;
const MAX_URL: usize = 2048;
const MAX_TITLE: usize = 300;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Visit {
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub first_at_ms: u64,
    pub last_at_ms: u64,
    /// How long it was the current tab while someone looked.
    pub dwell_ms: u64,
    /// How often it was arrived at.
    pub count: u64,
}

/// The address a page is kept under: an http(s) address without query,
/// fragment and credentials. `None`: the page is not recorded.
pub fn address(url: &str) -> Option<String> {
    let mut url = url::Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let address = String::from(url);
    (address.len() <= MAX_URL).then_some(address)
}

/// The letters and digits of `text` once it is unescaped: what stays of an
/// address however a browser displays it.
fn letters(text: &str) -> String {
    percent_encoding::percent_decode_str(text).decode_utf8_lossy().chars().filter(char::is_ascii_alphanumeric).collect()
}

/// The title a page is kept with. A tab that has no title yet, or whose
/// document has none, shows its address instead — query and all, some of it
/// unescaped: that is not kept.
fn kept_title(url: &str, title: &str) -> String {
    let title = title.trim();
    let Ok(url) = url::Url::parse(url) else { return String::new() };
    let shown = title.strip_prefix("https://").or_else(|| title.strip_prefix("http://")).unwrap_or(title);
    let host = url.host_str().unwrap_or_default();
    let starts_as_address = shown.get(..host.len()).is_some_and(|start| start.eq_ignore_ascii_case(host))
        && matches!(shown[host.len()..].chars().next(), None | Some('/' | ':' | '?' | '#'));
    let more = [url.query(), url.fragment()].into_iter().flatten().any(|part| !part.is_empty());
    let ends_as_address = more && letters(title).ends_with(&letters(&url[url::Position::BeforePath..]));
    if starts_as_address || ends_as_address {
        return String::new();
    }
    title.chars().take(MAX_TITLE).collect()
}

/// Whether the current tab of `group` is recorded: someone looks at it, the
/// user operates the browser, and it is not the time they enter a password.
fn recorded(watched: bool, group: &Group) -> bool {
    watched
        && !is_internal(&group.group_id)
        && group.controller == Controller::User
        && group.handover.as_ref().is_none_or(|handover| handover.reason != "password")
}

pub(super) struct Visits {
    file: PathBuf,
    switch: PathBuf,
    enabled: bool,
    /// By address.
    visits: HashMap<String, Visit>,
    /// The page being looked at, and up to when its time is counted.
    watching: Option<(String, u64)>,
    unwritten: bool,
    flushed_at_ms: u64,
}

impl Visits {
    /// What `dir` holds; `enabled` until the owner switched it.
    pub(super) fn open(dir: &Path, enabled: bool) -> Self {
        let (file, switch) = (dir.join("visits.jsonl"), dir.join("visits.json"));
        let switched = std::fs::read(&switch).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()).and_then(|v| v["enabled"].as_bool());
        let visits = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Visit>(line).ok())
            .filter(|visit| address(&visit.url).as_deref() == Some(visit.url.as_str()))
            .map(|visit| (visit.url.clone(), visit))
            .collect();
        Self { file, switch, enabled: switched.unwrap_or(enabled), visits, watching: None, unwritten: false, flushed_at_ms: 0 }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    /// At `now_ms` the owner looks at `page` (`(url, title)`), or at nothing
    /// that is recorded: the page looked at until now gets its time, a page
    /// that was not the one looked at is arrived at.
    pub(super) fn observe(&mut self, now_ms: u64, page: Option<(&str, &str)>) {
        let page = page.filter(|_| self.enabled).and_then(|(url, title)| Some((address(url)?, kept_title(url, title))));
        if let Some((at, since)) = &mut self.watching {
            if let Some(visit) = self.visits.get_mut(at) {
                visit.dwell_ms += now_ms.saturating_sub(*since).min(MAX_STEP_MS);
                visit.last_at_ms = visit.last_at_ms.max(now_ms);
                self.unwritten = true;
            }
            *since = now_ms;
        }
        let Some((at, title)) = page else {
            self.watching = None;
            return;
        };
        if self.watching.as_ref().is_none_or(|(watching, _)| *watching != at) {
            let visit = self.visits.entry(at.clone()).or_insert_with(|| Visit { url: at.clone(), title: String::new(), first_at_ms: now_ms, last_at_ms: now_ms, dwell_ms: 0, count: 0 });
            visit.count += 1;
            visit.last_at_ms = visit.last_at_ms.max(now_ms);
            self.watching = Some((at.clone(), now_ms));
            self.unwritten = true;
        }
        if let Some(visit) = self.visits.get_mut(&at).filter(|visit| !title.is_empty() && visit.title != title) {
            visit.title = title;
            self.unwritten = true;
        }
    }

    /// From the one looked at last to the one looked at longest ago.
    fn sorted(&self) -> Vec<&Visit> {
        let watched = |visit: &Visit| self.watching.as_ref().is_some_and(|(at, _)| *at == visit.url);
        let mut visits: Vec<&Visit> = self.visits.values().collect();
        visits.sort_by(|a, b| b.last_at_ms.cmp(&a.last_at_ms).then_with(|| watched(b).cmp(&watched(a))).then_with(|| a.url.cmp(&b.url)));
        visits
    }

    /// `browser.visits`: those looked at since `since_ms`.
    pub(super) fn list(&self, since_ms: Option<u64>) -> Vec<Visit> {
        self.sorted().into_iter().filter(|visit| visit.last_at_ms >= since_ms.unwrap_or(0)).take(MAX_LISTED).cloned().collect()
    }

    /// When it is due (`force`: whether or not it is), drop what is older
    /// than is kept and what is more than is kept, and write the file when
    /// what it holds changed.
    pub(super) fn flush(&mut self, now_ms: u64, force: bool) {
        if !force && now_ms.abs_diff(self.flushed_at_ms) < WRITE_EVERY_MS {
            return;
        }
        self.flushed_at_ms = now_ms;
        let before = self.visits.len();
        self.visits.retain(|_, visit| visit.last_at_ms.saturating_add(KEPT_MS) > now_ms);
        if self.visits.len() > MAX_KEPT {
            let oldest: Vec<String> = self.sorted().into_iter().skip(MAX_KEPT).map(|visit| visit.url.clone()).collect();
            for url in &oldest {
                self.visits.remove(url);
            }
        }
        if !self.unwritten && self.visits.len() == before {
            return;
        }
        self.unwritten = false;
        let lines: String = self.sorted().into_iter().map(|visit| serde_json::to_string(visit).expect("a visit serializes") + "\n").collect();
        if let Err(e) = write(&self.file, lines.as_bytes()) {
            log::warn!("browser: {} not written: {e}", self.file.display());
        }
    }

    /// The owner's switch. Switched off, nothing more is recorded; what was
    /// stays until it is cleared.
    pub(super) fn set_enabled(&mut self, enabled: bool, now_ms: u64) -> std::io::Result<()> {
        write(&self.switch, json!({ "enabled": enabled }).to_string().as_bytes())?;
        if !enabled {
            self.observe(now_ms, None);
        }
        self.enabled = enabled;
        self.flush(now_ms, true);
        Ok(())
    }

    pub(super) fn clear(&mut self) -> std::io::Result<()> {
        self.visits.clear();
        self.watching = None;
        self.unwritten = false;
        match std::fs::remove_file(&self.file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn write(file: &Path, content: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(file.parent().expect("the file has a directory"))?;
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, file)
}

impl BrowserModule {
    pub(super) fn visits(&self) -> MutexGuard<'_, Visits> {
        self.visits.lock().expect("browser visits")
    }

    /// Note what the owner looks at now.
    pub(super) fn note_visit(&self) {
        let now = now_ms();
        let page = {
            let s = self.shared();
            let group = s.groups.group(DEFAULT_GROUP, now);
            let watched = s.viewers.iter().any(|(_, viewed)| *viewed == group.group_id);
            match recorded(watched, &group) {
                true => group.tabs.into_iter().find(|tab| tab.active).map(|tab| (tab.url, tab.title)),
                false => None,
            }
        };
        self.visits().observe(now, page.as_ref().map(|(url, title)| (url.as_str(), title.as_str())));
    }

    /// `browser.visits`.
    pub fn visited(&self, since_ms: Option<u64>) -> Value {
        self.note_visit();
        let visits = self.visits();
        json!({ "enabled": visits.enabled(), "visits": visits.list(since_ms) })
    }

    /// `browser.visits_set_enabled`.
    pub fn visits_set_enabled(&self, enabled: bool) -> Result<(), BrowserError> {
        self.note_visit();
        let set = self.visits().set_enabled(enabled, now_ms());
        set.map_err(|e| BrowserError::new("other", format!("the switch of the visits was not written: {e}")))
    }

    /// `browser.visits_clear`.
    pub fn visits_clear(&self) -> Result<(), BrowserError> {
        self.visits().clear().map_err(|e| BrowserError::new("other", format!("the visits were not cleared: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::groups::Handover;

    const T0: u64 = 1_794_600_000_000;

    fn group(group_id: &str, controller: Controller, handover: Option<&str>) -> Group {
        Group {
            group_id: group_id.to_string(),
            controller,
            user_hold: handover.is_some(),
            agent_state: crate::browser::AgentState::Idle,
            note: None,
            handover: handover.map(|reason| Handover { reason: reason.to_string(), note: None, at_ms: T0 }),
            lease: None,
            tabs: Vec::new(),
        }
    }

    #[test]
    fn a_page_counts_while_its_user_looks_at_it_and_not_while_they_enter_a_password() {
        assert!(recorded(true, &group("default", Controller::User, None)));
        assert!(!recorded(false, &group("default", Controller::User, None)), "nobody looks");
        assert!(!recorded(true, &group("default", Controller::Agent, None)), "the agent operates");
        assert!(!recorded(true, &group("default", Controller::User, Some("password"))), "the user enters a password");
        assert!(recorded(true, &group("default", Controller::User, Some("captcha"))));
        assert!(recorded(true, &group("default", Controller::User, Some("other"))));
        assert!(!recorded(true, &group("_job", Controller::User, None)), "a tab of the module's own");
    }

    #[test]
    fn a_page_is_kept_by_its_address_without_query_and_fragment() {
        for (url, kept) in [
            ("https://www.example.com/news/1?utm=a&token=SECRET#top", Some("https://www.example.com/news/1")),
            ("https://www.example.com", Some("https://www.example.com/")),
            ("HTTP://Example.com:80/a/../b?x=1", Some("http://example.com/b")),
            ("https://user:pass@example.com:8443/p?q#f", Some("https://example.com:8443/p")),
            ("http://127.0.0.1:8080/?q=1", Some("http://127.0.0.1:8080/")),
            ("about:blank", None),
            ("chrome://settings/passwords", None),
            ("file:///etc/passwd", None),
            ("data:text/html,<p>x</p>", None),
            ("", None),
            ("not an address", None),
        ] {
            assert_eq!(address(url).as_deref(), kept, "{url}");
        }
        assert_eq!(address(&format!("https://example.com/{}", "a".repeat(MAX_URL))), None, "an address too long to keep");

        // The title a tab shows before its document has one is its address: no title.
        let url = "https://www.example.com/search?q=secret+words#results";
        for shown in ["www.example.com/search?q=secret+words#results", url, "www.example.com", "WWW.Example.com/search"] {
            assert_eq!(kept_title(url, shown), "", "{shown}");
        }
        // However the browser displays it: unescaped, the host in its own script, without `www.`.
        for (url, shown) in [
            ("https://xn--fsqu00a.example/s?q=secret%20words&x=%E4%B8%AD", "例子.example/s?q=secret words&x=中"),
            ("https://www.example.com/search?q=secret+words", "example.com/search?q=secret+words"),
            ("http://example.com/#access_token=SECRET", "example.com/#access_token=SECRET"),
            ("https://xn--fsqu00a.example/?%E4%B8%AD", "例子.example/?中"),
        ] {
            assert_eq!(kept_title(url, shown), "", "{shown}");
        }
        assert_eq!(kept_title(url, "  Search results  "), "Search results");
        assert_eq!(kept_title(url, "www.example.com.cn is another site"), "www.example.com.cn is another site");
        assert_eq!(kept_title("https://example.com/a", "Banana"), "Banana");
        assert_eq!(kept_title("https://example.com/", &"题".repeat(MAX_TITLE + 5)), "题".repeat(MAX_TITLE));
    }

    #[test]
    fn visits_merge_by_address_and_count_arrivals_and_time() {
        let tmp = tempfile::tempdir().unwrap();
        let mut v = Visits::open(tmp.path(), true);
        assert!(v.enabled() && v.list(None).is_empty());

        // Arrived at a page; its title comes later.
        v.observe(T0, Some(("https://a.example/read?id=1", "a.example/read?id=1")));
        v.observe(T0 + 1000, Some(("https://a.example/read?id=1", "Reading")));
        // The same page under another query: the same visit, not a new arrival.
        v.observe(T0 + 3000, Some(("https://a.example/read?id=2#more", "Reading on")));
        // Another page, then nothing that is recorded, then the first again.
        v.observe(T0 + 4000, Some(("https://b.example/", "B")));
        v.observe(T0 + 4500, None);
        v.observe(T0 + 9000, Some(("https://a.example/read", "Reading on")));
        v.observe(T0 + 9100, Some(("about:blank", "")));
        // A step no tick is apart: the clock jumped, or the service stood still.
        v.observe(T0 + 10_000, Some(("https://b.example/", "B")));
        v.observe(T0 + 10_000 + 3_600_000, Some(("https://b.example/", "B")));
        v.observe(T0 + 10_000 + 3_600_000, None);

        let b = Visit { url: "https://b.example/".into(), title: "B".into(), first_at_ms: T0 + 4000, last_at_ms: T0 + 3_610_000, dwell_ms: 500 + MAX_STEP_MS, count: 2 };
        let a = Visit { url: "https://a.example/read".into(), title: "Reading on".into(), first_at_ms: T0, last_at_ms: T0 + 9100, dwell_ms: 4000 + 100, count: 2 };
        assert_eq!(v.list(None), vec![b.clone(), a.clone()], "the newest first");
        assert_eq!(v.list(Some(T0 + 9101)), vec![b.clone()]);
        assert_eq!(v.list(Some(T0 + 3_610_001)), vec![]);

        // Written when it is due, read by the next service; no query reached the file.
        v.flush(T0 + 3_700_000, false);
        let file = std::fs::read_to_string(tmp.path().join("visits.jsonl")).unwrap();
        assert_eq!(file.lines().map(|line| serde_json::from_str::<Visit>(line).unwrap()).collect::<Vec<_>>(), vec![b.clone(), a.clone()]);
        assert!(!file.contains('?') && !file.contains("id=") && !file.contains("more"), "{file}");
        v.observe(T0 + 3_700_000, Some(("https://c.example/", "C")));
        v.flush(T0 + 3_700_000 + WRITE_EVERY_MS - 1, false);
        assert!(!std::fs::read_to_string(tmp.path().join("visits.jsonl")).unwrap().contains("c.example"), "not written again so soon");
        v.flush(T0 + 3_700_000 + WRITE_EVERY_MS - 1, true);
        let again = Visits::open(tmp.path(), true);
        assert_eq!(again.list(None).iter().map(|visit| visit.url.as_str()).collect::<Vec<_>>(), vec!["https://c.example/", "https://b.example/", "https://a.example/read"]);

        // A line somebody else wrote with a query in it is not taken.
        std::fs::write(tmp.path().join("visits.jsonl"), format!("{}\nnot json\n{}\n", serde_json::to_string(&a).unwrap(), json!({ "url": "https://a.example/?q=1", "first_at_ms": 1, "last_at_ms": 1, "dwell_ms": 0, "count": 1 }))).unwrap();
        assert_eq!(Visits::open(tmp.path(), true).list(None), vec![a]);
    }

    #[test]
    fn the_owner_switches_the_visits_off_and_clears_them() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state").join("browser");
        // Off by the agent's configuration: nothing is recorded, no file is written.
        let mut v = Visits::open(&dir, false);
        assert!(!v.enabled());
        v.observe(T0, Some(("https://a.example/", "A")));
        v.flush(T0, true);
        assert!(v.list(None).is_empty() && !dir.exists());

        // The owner switches it on: that is kept, whatever the configuration says.
        v.set_enabled(true, T0).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("visits.json")).unwrap(), r#"{"enabled":true}"#);
        assert!(Visits::open(&dir, false).enabled());
        v.observe(T0 + 1000, Some(("https://a.example/", "A")));
        v.observe(T0 + 3000, Some(("https://a.example/", "A")));

        // Switched off: the page looked at gets its time up to now, nothing more is recorded, what was stays.
        v.set_enabled(false, T0 + 4000).unwrap();
        assert!(!Visits::open(&dir, true).enabled());
        v.observe(T0 + 9000, Some(("https://a.example/", "A")));
        v.observe(T0 + 9500, Some(("https://b.example/", "B")));
        let kept = v.list(None);
        assert_eq!((kept.len(), kept[0].dwell_ms, kept[0].count, kept[0].last_at_ms), (1, 3000, 1, T0 + 4000));
        assert_eq!(Visits::open(&dir, true).list(None), kept, "written when it was switched off");

        // Cleared: nothing is left, in the service and in the file; the switch stays.
        v.clear().unwrap();
        assert!(v.list(None).is_empty() && !dir.join("visits.jsonl").exists());
        v.clear().unwrap();
        assert!(!Visits::open(&dir, true).enabled());
        // Switched on again, the page looked at is arrived at anew.
        v.set_enabled(true, T0 + 10_000).unwrap();
        v.observe(T0 + 10_000, Some(("https://a.example/", "A")));
        assert_eq!(v.list(None)[0].first_at_ms, T0 + 10_000);
    }

    #[test]
    fn visits_are_kept_ninety_days_and_five_thousand_at_most() {
        let tmp = tempfile::tempdir().unwrap();
        let mut v = Visits::open(tmp.path(), true);
        let day = 24 * 3600 * 1000;
        v.observe(T0, Some(("https://old.example/", "Old")));
        v.observe(T0 + 1, None);
        v.observe(T0 + day, Some(("https://newer.example/", "Newer")));
        v.observe(T0 + day + 1, None);
        v.flush(T0 + 90 * day - 1, true);
        assert_eq!(v.list(None).len(), 2);
        // Ninety days after it was last looked at a visit goes, from the file too.
        v.flush(T0 + 90 * day + 1, true);
        assert_eq!(v.list(None).iter().map(|visit| visit.url.as_str()).collect::<Vec<_>>(), vec!["https://newer.example/"]);
        assert!(!std::fs::read_to_string(tmp.path().join("visits.jsonl")).unwrap().contains("old.example"));
        // A service that starts later finds what is too old in the file and drops it.
        let mut later = Visits::open(tmp.path(), true);
        later.flush(T0 + 92 * day, false);
        assert!(later.list(None).is_empty());
        assert_eq!(std::fs::read_to_string(tmp.path().join("visits.jsonl")).unwrap(), "");

        // More than are kept: the ones looked at longest ago go.
        let start = T0 + 100 * day;
        for n in 0..MAX_KEPT as u64 + 3 {
            v.observe(start + n, Some((&format!("https://many.example/{n}"), "")));
        }
        v.observe(start + MAX_KEPT as u64 + 3, None);
        v.flush(start + day, true);
        assert_eq!(std::fs::read_to_string(tmp.path().join("visits.jsonl")).unwrap().lines().count(), MAX_KEPT);
        let kept = Visits::open(tmp.path(), true);
        assert_eq!(kept.visits.len(), MAX_KEPT);
        assert!(!kept.visits.contains_key("https://many.example/2") && kept.visits.contains_key("https://many.example/3"));
        assert_eq!(kept.list(None).len(), MAX_LISTED, "a call returns the newest of them");
        assert_eq!(kept.list(None)[0].url, format!("https://many.example/{}", MAX_KEPT + 2));
    }
}
