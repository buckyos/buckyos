//! The tabs of the agent's browser and who operates them. The agent has one
//! group, [`DEFAULT_GROUP`]: every session of the agent works in it and the
//! user sees it; a group whose id starts with `_` is one the module keeps for
//! itself. All of it is memory; the tabs (and whether the user holds them)
//! are also kept in `<AgentRoot>/state/browser/groups.json`, from which they
//! are opened again after the browser was stopped.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::rpc::BrowserError;

/// The agent's one group.
pub const DEFAULT_GROUP: &str = "default";
pub const LEASE_TTL_MS: u64 = 60_000;
const SCHEMA: &str = "opendan.browser.groups/1";
const MAX_SAVED_URL: usize = 8 * 1024;

/// A group the module keeps for itself (tabs no conversation shows): not
/// listed, not saved, not counted against `max_tabs`.
pub fn is_internal(group_id: &str) -> bool {
    group_id.starts_with('_')
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tab {
    pub tab_id: String,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Controller {
    User,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    WaitingAnswer,
    WaitingTakeover,
    Done,
    Stuck,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handover {
    pub reason: String,
    pub note: Option<String>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeaseInfo {
    pub session_id: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    pub group_id: String,
    pub controller: Controller,
    pub user_hold: bool,
    pub agent_state: AgentState,
    pub note: Option<String>,
    pub handover: Option<Handover>,
    pub lease: Option<LeaseInfo>,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub lease_id: String,
    pub group_id: String,
    pub session_id: String,
    pub expires_at_ms: u64,
}

/// What a tab is to the module besides what [`Tab`] shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TabRef {
    pub tab_id: String,
    pub group_id: String,
    /// The module's flat CDP session of the tab (`None` until attached).
    pub session: Option<String>,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedGroup {
    group_id: String,
    #[serde(default)]
    tabs: Vec<SavedTab>,
    #[serde(default)]
    active: Option<usize>,
    #[serde(default)]
    user_hold: bool,
    #[serde(default)]
    handover: Option<Handover>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedFile {
    schema: String,
    /// The agent's browser conversation (`browser.bind_default`).
    #[serde(default)]
    default_session_id: Option<String>,
    #[serde(default)]
    groups: Vec<SavedGroup>,
}

#[derive(Debug, Default)]
struct TabState {
    group_id: String,
    url: String,
    title: String,
    loading: bool,
    session: Option<String>,
}

#[derive(Debug, Default)]
struct GroupState {
    tabs: Vec<String>,
    active: Option<String>,
    /// Tabs to open again at the next start (the browser is not running).
    saved: Vec<SavedTab>,
    saved_active: Option<usize>,
    user_hold: bool,
    handover: Option<Handover>,
    /// The agent's last `release {state}`: `(done | stuck, note)`.
    report: Option<(AgentState, Option<String>)>,
    leases: Vec<Lease>,
    waiting_answer: bool,
}

impl GroupState {
    fn leased(&self, now_ms: u64) -> Option<&Lease> {
        self.leases.iter().filter(|l| l.expires_at_ms > now_ms).max_by_key(|l| l.expires_at_ms)
    }

    fn idle(&self) -> bool {
        self.tabs.is_empty()
            && self.saved.is_empty()
            && self.leases.is_empty()
            && !self.user_hold
            && self.handover.is_none()
            && self.report.is_none()
            && !self.waiting_answer
    }
}

pub struct Groups {
    groups: BTreeMap<String, GroupState>,
    tabs: HashMap<String, TabState>,
    /// The agent's browser conversation (`browser.bind_default`).
    default_session_id: Option<String>,
    max_tabs: usize,
    lease_ttl_ms: u64,
    lease_seq: u64,
}

impl Groups {
    pub fn new(max_tabs: usize) -> Self {
        Self {
            groups: BTreeMap::new(),
            tabs: HashMap::new(),
            default_session_id: None,
            max_tabs,
            lease_ttl_ms: LEASE_TTL_MS,
            lease_seq: 0,
        }
    }

    pub fn with_lease_ttl(mut self, ttl_ms: u64) -> Self {
        self.lease_ttl_ms = ttl_ms;
        self
    }

    pub fn lease_ttl_ms(&self) -> u64 {
        self.lease_ttl_ms
    }

    fn entry(&mut self, group_id: &str) -> &mut GroupState {
        self.groups.entry(group_id.to_string()).or_default()
    }

    // ---- the browser conversation ----

    pub fn default_session(&self) -> Option<&str> {
        self.default_session_id.as_deref()
    }

    /// The MessageHub conversation that is the agent's browser conversation
    /// from now on; the tabs and who holds them stay as they are. Returns
    /// whether it changed.
    pub fn bind_default(&mut self, session_id: &str) -> bool {
        if self.default_session_id.as_deref() == Some(session_id) {
            return false;
        }
        self.default_session_id = Some(session_id.to_string());
        true
    }

    // ---- tabs ----

    /// Tabs conversations show (internal groups do not count).
    pub fn tab_count(&self) -> usize {
        self.tabs.values().filter(|t| !is_internal(&t.group_id)).count()
    }

    pub fn has_room(&self) -> bool {
        self.tab_count() < self.max_tabs
    }

    pub fn max_tabs(&self) -> usize {
        self.max_tabs
    }

    /// The group a tab that just appeared belongs to: its opener's; else the
    /// one a tab is being opened for (`expected`); else the default group.
    pub fn place(&self, opener: Option<&str>, expected: Option<&str>) -> String {
        if let Some(group) = opener.and_then(|o| self.tabs.get(o)) {
            return group.group_id.clone();
        }
        expected.unwrap_or(DEFAULT_GROUP).to_string()
    }

    /// A tab exists (seen as a target, or just opened): it is in `group_id`
    /// from now on and is that group's current tab when it is new. Returns
    /// whether it is new.
    pub fn tab_seen(&mut self, tab_id: &str, group_id: &str, url: &str, title: &str) -> bool {
        match self.tabs.get_mut(tab_id) {
            Some(tab) if tab.group_id == group_id => {
                if !url.is_empty() {
                    tab.url = url.to_string();
                }
                if !title.is_empty() {
                    tab.title = title.to_string();
                }
                false
            }
            Some(_) => {
                self.move_tab(tab_id, group_id);
                false
            }
            None => {
                self.tabs.insert(
                    tab_id.to_string(),
                    TabState {
                        group_id: group_id.to_string(),
                        url: url.to_string(),
                        title: title.to_string(),
                        ..Default::default()
                    },
                );
                let group = self.entry(group_id);
                group.tabs.push(tab_id.to_string());
                group.active = Some(tab_id.to_string());
                true
            }
        }
    }

    pub fn move_tab(&mut self, tab_id: &str, group_id: &str) {
        let Some(from) = self.tabs.get(tab_id).map(|t| t.group_id.clone()) else { return };
        if from == group_id {
            return;
        }
        self.detach(&from, tab_id);
        if let Some(tab) = self.tabs.get_mut(tab_id) {
            tab.group_id = group_id.to_string();
        }
        let group = self.entry(group_id);
        group.tabs.push(tab_id.to_string());
        group.active = Some(tab_id.to_string());
    }

    fn detach(&mut self, group_id: &str, tab_id: &str) {
        if let Some(group) = self.groups.get_mut(group_id) {
            group.tabs.retain(|t| t != tab_id);
            if group.active.as_deref() == Some(tab_id) {
                group.active = group.tabs.last().cloned();
            }
        }
    }

    /// The browser says what a tab shows now. A tab always has an address:
    /// a report without one (a target on its way in or out) changes nothing.
    pub fn tab_changed(&mut self, tab_id: &str, url: &str, title: &str) -> bool {
        match self.tabs.get_mut(tab_id) {
            Some(tab) if !url.is_empty() && (tab.url != url || tab.title != title) => {
                tab.url = url.to_string();
                tab.title = title.to_string();
                true
            }
            _ => false,
        }
    }

    pub fn tab_removed(&mut self, tab_id: &str) -> bool {
        let Some(tab) = self.tabs.remove(tab_id) else { return false };
        self.detach(&tab.group_id, tab_id);
        true
    }

    pub fn tab_attached(&mut self, tab_id: &str, session: Option<&str>) {
        if let Some(tab) = self.tabs.get_mut(tab_id) {
            tab.session = session.map(str::to_string);
        }
    }

    pub fn tab_of_session(&self, session: &str) -> Option<String> {
        self.tabs.iter().find(|(_, t)| t.session.as_deref() == Some(session)).map(|(id, _)| id.clone())
    }

    pub fn set_loading(&mut self, tab_id: &str, loading: bool) -> bool {
        match self.tabs.get_mut(tab_id) {
            Some(tab) if tab.loading != loading => {
                tab.loading = loading;
                true
            }
            _ => false,
        }
    }

    /// Make a tab its group's current one.
    pub fn activate(&mut self, tab_id: &str) -> Result<String, BrowserError> {
        let group_id = self.tabs.get(tab_id).map(|t| t.group_id.clone()).ok_or_else(|| BrowserError::no_such_tab(tab_id))?;
        self.entry(&group_id).active = Some(tab_id.to_string());
        Ok(group_id)
    }

    pub fn tab(&self, tab_id: &str) -> Option<TabRef> {
        self.tabs.get(tab_id).map(|t| TabRef {
            tab_id: tab_id.to_string(),
            group_id: t.group_id.clone(),
            session: t.session.clone(),
            url: t.url.clone(),
            title: t.title.clone(),
        })
    }

    pub fn tab_ids(&self) -> Vec<String> {
        self.tabs.keys().cloned().collect()
    }

    pub fn active_tab(&self, group_id: &str) -> Option<String> {
        self.groups.get(group_id).and_then(|g| g.active.clone())
    }

    // ---- who operates ----

    /// Drop the leases that ran out and the groups nothing is left of.
    /// Returns whether a lease ended (the operating party may have changed).
    pub fn expire(&mut self, now_ms: u64) -> bool {
        let mut changed = false;
        for group in self.groups.values_mut() {
            let before = group.leases.len();
            group.leases.retain(|l| l.expires_at_ms > now_ms);
            changed |= group.leases.len() != before;
        }
        self.groups.retain(|id, g| id == DEFAULT_GROUP || !g.idle());
        changed
    }

    /// Whether `lease_id` is a lease that has not ended.
    pub fn lease_valid(&self, lease_id: &str, now_ms: u64) -> bool {
        self.groups.values().any(|g| !g.user_hold && g.leases.iter().any(|l| l.lease_id == lease_id && l.expires_at_ms > now_ms))
    }

    pub fn has_lease(&self, now_ms: u64) -> bool {
        self.groups.values().any(|g| g.leased(now_ms).is_some())
    }

    pub fn controller(&self, group_id: &str, now_ms: u64) -> Controller {
        match self.groups.get(group_id) {
            Some(g) if !g.user_hold && g.leased(now_ms).is_some() => Controller::Agent,
            _ => Controller::User,
        }
    }

    /// A lease of `session_id` on its group. Refused while the user holds
    /// the group. A group may be leased more than once at a time (a
    /// conversation and a Work of it each run a command).
    pub fn acquire(&mut self, group_id: &str, session_id: &str, now_ms: u64) -> Result<Lease, BrowserError> {
        self.lease_seq += 1;
        let lease = Lease {
            lease_id: format!("lease-{now_ms:x}-{}", self.lease_seq),
            group_id: group_id.to_string(),
            session_id: session_id.to_string(),
            expires_at_ms: now_ms + self.lease_ttl_ms,
        };
        let group = self.entry(group_id);
        if group.user_hold {
            return Err(BrowserError::user_in_control());
        }
        group.leases.retain(|l| l.expires_at_ms > now_ms);
        group.leases.push(lease.clone());
        group.report = None;
        Ok(lease)
    }

    fn lease_mut(&mut self, lease_id: &str, now_ms: u64) -> Result<(&mut GroupState, usize), BrowserError> {
        self.groups
            .values_mut()
            .find_map(|g| {
                let at = g.leases.iter().position(|l| l.lease_id == lease_id && l.expires_at_ms > now_ms)?;
                Some((g, at))
            })
            .ok_or_else(|| BrowserError::lease_lost(lease_id))
    }

    pub fn renew(&mut self, lease_id: &str, now_ms: u64) -> Result<u64, BrowserError> {
        let expires_at_ms = now_ms + self.lease_ttl_ms;
        let (group, at) = self.lease_mut(lease_id, now_ms)?;
        group.leases[at].expires_at_ms = expires_at_ms;
        Ok(expires_at_ms)
    }

    /// End a lease; `report` is what the agent says about the thing
    /// (`done | stuck`, with a note).
    pub fn release(&mut self, lease_id: &str, report: Option<(AgentState, Option<String>)>, now_ms: u64) -> Result<(), BrowserError> {
        let (group, at) = self.lease_mut(lease_id, now_ms)?;
        group.leases.remove(at);
        if report.is_some() {
            group.report = report;
        }
        Ok(())
    }

    /// The agent asks the user to take the group over: its leases end and
    /// the user holds the group until they hand it back.
    pub fn request_handover(&mut self, group_id: &str, reason: &str, note: Option<String>, now_ms: u64) {
        let group = self.entry(group_id);
        group.leases.clear();
        group.user_hold = true;
        group.report = None;
        group.handover = Some(Handover { reason: reason.to_string(), note, at_ms: now_ms });
    }

    pub fn take_over(&mut self, group_id: &str) {
        let group = self.entry(group_id);
        group.leases.clear();
        group.user_hold = true;
        group.report = None;
    }

    /// The user gives the group back (`hand_back`, `hand_to_agent`).
    pub fn hand_back(&mut self, group_id: &str) {
        let group = self.entry(group_id);
        group.user_hold = false;
        group.handover = None;
        group.report = None;
    }

    /// Whether the browser conversation waits for an answer of the user.
    /// Returns whether that changed.
    pub fn set_waiting_answer(&mut self, waiting: bool) -> bool {
        let group = self.entry(DEFAULT_GROUP);
        std::mem::replace(&mut group.waiting_answer, waiting) != waiting
    }

    // ---- views ----

    pub fn group(&self, group_id: &str, now_ms: u64) -> Group {
        let empty = GroupState::default();
        let g = self.groups.get(group_id).unwrap_or(&empty);
        let lease = g.leased(now_ms);
        let agent_state = if g.user_hold && g.handover.is_some() {
            AgentState::WaitingTakeover
        } else if lease.is_some() {
            AgentState::Working
        } else if g.waiting_answer {
            AgentState::WaitingAnswer
        } else {
            g.report.as_ref().map(|(state, _)| *state).unwrap_or(AgentState::Idle)
        };
        Group {
            group_id: group_id.to_string(),
            controller: if !g.user_hold && lease.is_some() { Controller::Agent } else { Controller::User },
            user_hold: g.user_hold,
            agent_state,
            note: match &g.handover {
                Some(h) => h.note.clone(),
                None => g.report.as_ref().and_then(|(_, note)| note.clone()),
            },
            handover: g.handover.clone(),
            lease: lease.map(|l| LeaseInfo { session_id: l.session_id.clone(), expires_at_ms: l.expires_at_ms }),
            tabs: g
                .tabs
                .iter()
                .filter_map(|id| {
                    let t = self.tabs.get(id)?;
                    Some(Tab {
                        tab_id: id.clone(),
                        url: t.url.clone(),
                        title: t.title.clone(),
                        loading: t.loading,
                        active: g.active.as_deref() == Some(id.as_str()),
                    })
                })
                .collect(),
        }
    }

    // ---- stop, start, the file ----

    /// The browser stopped: its tabs are gone; what they showed is kept to
    /// be opened again. Leases end with it.
    pub fn suspend(&mut self) {
        let tabs = std::mem::take(&mut self.tabs);
        for (id, group) in self.groups.iter_mut() {
            if !group.tabs.is_empty() && !is_internal(id) {
                group.saved_active = group.active.as_ref().and_then(|a| group.tabs.iter().position(|t| t == a));
                group.saved = group.tabs.iter().filter_map(|t| tabs.get(t)).filter_map(saved_tab).collect();
            }
            group.tabs.clear();
            group.active = None;
            group.leases.clear();
        }
    }

    /// The tabs to open again, per group with the index of its current one;
    /// they are not kept any more.
    pub fn take_saved(&mut self) -> Vec<(String, Vec<SavedTab>, Option<usize>)> {
        self.groups
            .iter_mut()
            .filter(|(_, g)| !g.saved.is_empty())
            .map(|(id, g)| (id.clone(), std::mem::take(&mut g.saved), g.saved_active.take()))
            .collect()
    }

    fn file(&self) -> SavedFile {
        SavedFile {
            schema: SCHEMA.to_string(),
            default_session_id: self.default_session_id.clone(),
            groups: self
                .groups
                .iter()
                .filter(|(id, _)| !is_internal(id))
                .map(|(id, g)| {
                    let live: Vec<SavedTab> = g.tabs.iter().filter_map(|t| self.tabs.get(t)).filter_map(saved_tab).collect();
                    let running = !g.tabs.is_empty();
                    SavedGroup {
                        group_id: id.clone(),
                        active: if running {
                            g.active.as_ref().and_then(|a| g.tabs.iter().position(|t| t == a))
                        } else {
                            g.saved_active
                        },
                        tabs: if running { live } else { g.saved.clone() },
                        user_hold: g.user_hold,
                        handover: g.handover.clone(),
                    }
                })
                .filter(|g| !g.tabs.is_empty() || g.user_hold || g.handover.is_some())
                .collect(),
        }
    }

    /// The file's content for the state as it is (compared by the caller to
    /// write only what changed).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.file()).expect("groups serialize")
    }

    pub fn load(&mut self, path: &Path) {
        let Ok(bytes) = std::fs::read(path) else { return };
        let file = match serde_json::from_slice::<SavedFile>(&bytes) {
            Ok(file) if file.schema == SCHEMA => file,
            Ok(file) => {
                log::warn!("browser: {} has schema {}; its tabs are not restored", path.display(), file.schema);
                return;
            }
            Err(e) => {
                log::warn!("browser: {} is not readable ({e}); its tabs are not restored", path.display());
                return;
            }
        };
        self.default_session_id = file.default_session_id.filter(|s| !s.is_empty());
        for saved in file.groups.into_iter().filter(|g| g.group_id == DEFAULT_GROUP) {
            let group = self.entry(&saved.group_id);
            group.saved = saved.tabs;
            group.saved_active = saved.active;
            group.user_hold = saved.user_hold;
            group.handover = saved.handover;
        }
    }
}

fn saved_tab(tab: &TabState) -> Option<SavedTab> {
    (!tab.url.is_empty() && tab.url.len() <= MAX_SAVED_URL).then(|| SavedTab { url: tab.url.clone(), title: tab.title.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;

    fn code<T: std::fmt::Debug>(r: Result<T, BrowserError>) -> &'static str {
        r.unwrap_err().code
    }

    #[test]
    fn one_party_operates_a_group_and_hands_it_over_explicitly() {
        let mut g = Groups::new(12);
        assert_eq!(g.controller("default", T0), Controller::User);
        assert_eq!(g.group("default", T0).agent_state, AgentState::Idle);

        // The agent holds a lease: it operates.
        let lease = g.acquire("default", "ui-1", T0).unwrap();
        assert_eq!(lease.expires_at_ms, T0 + LEASE_TTL_MS);
        let view = g.group("default", T0 + 1);
        assert_eq!((view.controller, view.agent_state), (Controller::Agent, AgentState::Working));
        assert_eq!(view.lease.unwrap().session_id, "ui-1");

        // The user takes over: the lease is void and no new one is given.
        g.take_over("default");
        assert_eq!(g.controller("default", T0 + 2), Controller::User);
        assert_eq!(code(g.renew(&lease.lease_id, T0 + 2)), "lease_lost");
        assert_eq!(code(g.acquire("default", "ui-1", T0 + 3)), "user_in_control");
        let view = g.group("default", T0 + 3);
        assert!(view.user_hold && view.lease.is_none());
        assert_eq!(view.agent_state, AgentState::Idle, "the user took it without being asked");

        // Handed back: the agent may operate again.
        g.hand_back("default");
        let lease = g.acquire("default", "ui-1", T0 + 4).unwrap();
        assert_eq!(g.controller("default", T0 + 5), Controller::Agent);
        assert_eq!(g.renew(&lease.lease_id, T0 + 1_000).unwrap(), T0 + 1_000 + LEASE_TTL_MS);

        // A lease that is not renewed runs out: the user operates.
        let late = T0 + 1_000 + LEASE_TTL_MS;
        assert_eq!(g.controller("default", late - 1), Controller::Agent);
        assert_eq!(g.controller("default", late), Controller::User);
        assert_eq!(g.group("default", late).agent_state, AgentState::Idle);
        assert!(g.expire(late));
        assert_eq!(code(g.renew(&lease.lease_id, late)), "lease_lost");
        assert_eq!(code(g.release(&lease.lease_id, None, late)), "lease_lost");
    }

    #[test]
    fn the_agent_asks_the_user_to_take_over_and_reports_how_it_ended() {
        let mut g = Groups::new(12);
        let lease = g.acquire(DEFAULT_GROUP, "work-1", T0).unwrap();
        g.request_handover(DEFAULT_GROUP, "password", Some("the bank's login".into()), T0 + 10);
        let view = g.group(DEFAULT_GROUP, T0 + 11);
        assert_eq!((view.controller, view.agent_state), (Controller::User, AgentState::WaitingTakeover));
        assert!(view.user_hold && view.lease.is_none());
        assert_eq!(view.note.as_deref(), Some("the bank's login"));
        assert_eq!(view.handover.as_ref().unwrap().reason, "password");
        assert_eq!(code(g.renew(&lease.lease_id, T0 + 12)), "lease_lost");
        assert_eq!(code(g.acquire(DEFAULT_GROUP, "ui-2", T0 + 12)), "user_in_control", "whichever session asks");
        // Taking over what was asked for keeps the request shown.
        g.take_over(DEFAULT_GROUP);
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 13).agent_state, AgentState::WaitingTakeover);
        g.hand_back(DEFAULT_GROUP);
        let view = g.group(DEFAULT_GROUP, T0 + 14);
        assert_eq!((view.agent_state, view.user_hold, view.handover, view.note), (AgentState::Idle, false, None, None));

        // done / stuck stay until the browser is leased, taken or handed back again.
        let lease = g.acquire(DEFAULT_GROUP, "work-1", T0 + 20).unwrap();
        g.release(&lease.lease_id, Some((AgentState::Stuck, Some("needs a code".into()))), T0 + 21).unwrap();
        let view = g.group(DEFAULT_GROUP, T0 + 22);
        assert_eq!((view.controller, view.agent_state), (Controller::User, AgentState::Stuck));
        assert_eq!(view.note.as_deref(), Some("needs a code"));
        let lease = g.acquire(DEFAULT_GROUP, "work-1", T0 + 30).unwrap();
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 31).agent_state, AgentState::Working);
        g.release(&lease.lease_id, Some((AgentState::Done, None)), T0 + 32).unwrap();
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 33).agent_state, AgentState::Done);
        g.take_over(DEFAULT_GROUP);
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 34).agent_state, AgentState::Idle);

        // A question the browser conversation waits on shows when no lease is held.
        g.hand_back(DEFAULT_GROUP);
        let lease = g.acquire(DEFAULT_GROUP, "work-1", T0 + 40).unwrap();
        assert!(g.set_waiting_answer(true));
        assert!(!g.set_waiting_answer(true));
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 41).agent_state, AgentState::Working);
        g.release(&lease.lease_id, None, T0 + 42).unwrap();
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 43).agent_state, AgentState::WaitingAnswer);
        assert!(g.set_waiting_answer(false));
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 44).agent_state, AgentState::Idle);
    }

    #[test]
    fn sessions_hold_leases_side_by_side() {
        let mut g = Groups::new(12);
        let a = g.acquire(DEFAULT_GROUP, "ui-1", T0).unwrap();
        let b = g.acquire(DEFAULT_GROUP, "work-1", T0 + 1_000).unwrap();
        assert_ne!(a.lease_id, b.lease_id);
        assert_eq!(g.group(DEFAULT_GROUP, T0 + 1_001).lease.unwrap().session_id, "work-1", "the one that ends last");
        g.release(&a.lease_id, None, T0 + 1_002).unwrap();
        assert_eq!(g.controller(DEFAULT_GROUP, T0 + 1_003), Controller::Agent);
        g.release(&b.lease_id, None, T0 + 1_004).unwrap();
        assert_eq!(g.controller(DEFAULT_GROUP, T0 + 1_005), Controller::User);
    }

    #[test]
    fn a_new_tab_goes_to_its_openers_group_then_the_one_it_is_opened_for_then_the_default_one() {
        let mut g = Groups::new(3);
        assert_eq!(g.place(None, None), DEFAULT_GROUP);
        assert!(g.tab_seen("r1", "_reflow", "https://r.example/", "R"));
        assert!(g.tab_seen("t1", DEFAULT_GROUP, "https://a.example/", "A"));
        assert!(!g.tab_seen("t1", DEFAULT_GROUP, "https://a.example/x", ""));
        assert_eq!(g.tab("t1").unwrap().url, "https://a.example/x");
        // A page opened by a page.
        assert_eq!(g.place(Some("r1"), None), "_reflow");
        assert_eq!(g.place(Some("t1"), Some("_reflow")), DEFAULT_GROUP);
        // A tab the module opens for one of its own groups.
        assert_eq!(g.place(None, Some("_reflow")), "_reflow");
        // A lease changes nothing: every session works in the one group.
        g.acquire(DEFAULT_GROUP, "ui-9", T0).unwrap();
        assert_eq!(g.place(None, None), DEFAULT_GROUP);

        // The newest tab is the group's current one; closing it falls back.
        assert!(g.tab_seen("t2", DEFAULT_GROUP, "https://b.example/", "B"));
        assert_eq!(g.active_tab(DEFAULT_GROUP).as_deref(), Some("t2"));
        assert_eq!(g.activate("t1").unwrap(), DEFAULT_GROUP);
        let tabs = g.group(DEFAULT_GROUP, T0).tabs;
        assert_eq!(tabs.iter().map(|t| (t.tab_id.as_str(), t.active)).collect::<Vec<_>>(), vec![("t1", true), ("t2", false)]);
        assert!(g.tab_removed("t1"));
        assert_eq!(g.active_tab(DEFAULT_GROUP).as_deref(), Some("t2"));
        assert_eq!(code(g.activate("t1")), "no_such_tab");

        // The limit counts the tabs the user sees.
        assert!(g.tab_seen("t3", DEFAULT_GROUP, "about:blank", ""));
        assert!(g.has_room());
        assert!(g.tab_seen("t4", "_reflow", "about:blank", ""));
        assert!(g.has_room());
        assert!(g.tab_seen("t5", DEFAULT_GROUP, "about:blank", ""));
        assert!(!g.has_room());
        // A tab the module opened for another group than the one it was first seen in moves there.
        assert!(!g.tab_seen("t5", "_reflow", "", ""));
        assert_eq!(g.tab("t5").unwrap().group_id, "_reflow");
        assert_eq!(g.group(DEFAULT_GROUP, T0).tabs.len(), 2);
    }

    #[test]
    fn tabs_are_kept_across_a_stop_and_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("groups.json");
        let mut g = Groups::new(12);
        g.tab_seen("t1", DEFAULT_GROUP, "https://a.example/", "A");
        g.tab_seen("t2", DEFAULT_GROUP, "https://b.example/", "B");
        g.tab_seen("t4", "_reflow", "https://d.example/", "D");
        g.activate("t1").unwrap();
        g.take_over(DEFAULT_GROUP);
        let running = g.to_json();
        g.suspend();
        assert_eq!(g.to_json(), running, "what is saved does not depend on whether the browser runs");
        assert!(g.tab_ids().is_empty() && !g.has_lease(T0));
        assert!(g.group(DEFAULT_GROUP, T0).tabs.is_empty());
        std::fs::write(&path, g.to_json()).unwrap();

        let mut again = Groups::new(12);
        again.load(&path);
        assert!(again.group(DEFAULT_GROUP, T0).user_hold, "the browser the user holds stays held");
        let saved = again.take_saved();
        assert_eq!(saved.len(), 1, "the module's own tabs are not kept");
        assert_eq!(saved[0].0, DEFAULT_GROUP);
        assert_eq!(saved[0].1.iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), vec!["https://a.example/", "https://b.example/"]);
        assert_eq!(saved[0].2, Some(0));
        assert!(again.take_saved().is_empty());

        // A file from when every conversation had a group of its own: only the default group's tabs come back.
        let old = serde_json::json!({ "schema": SCHEMA, "default_session_id": "conv-1", "groups": [
            { "group_id": "default", "tabs": [{ "url": "https://a.example/" }], "active": 0 },
            { "group_id": "conv-2", "tabs": [{ "url": "https://c.example/" }], "user_hold": true },
        ] });
        std::fs::write(&path, old.to_string()).unwrap();
        let mut again = Groups::new(12);
        again.load(&path);
        assert_eq!(again.default_session(), Some("conv-1"));
        let saved = again.take_saved();
        assert_eq!(saved.iter().map(|(id, tabs, _)| (id.as_str(), tabs.len())).collect::<Vec<_>>(), vec![(DEFAULT_GROUP, 1)]);
        assert!(!again.group(DEFAULT_GROUP, T0).user_hold);
    }

    #[test]
    fn the_browser_conversation_is_bound_and_kept_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("groups.json");
        let mut g = Groups::new(12);
        assert_eq!(g.default_session(), None);
        g.tab_seen("t1", DEFAULT_GROUP, "https://a.example/", "A");
        let lease = g.acquire(DEFAULT_GROUP, "ui-1", T0).unwrap();

        // Binding moves nothing: the tabs and the lease stay.
        assert!(g.bind_default("conv-1"));
        assert_eq!(g.default_session(), Some("conv-1"));
        assert!(!g.bind_default("conv-1"), "again: nothing changes");
        let view = g.group(DEFAULT_GROUP, T0 + 1);
        assert_eq!((view.tabs.len(), view.controller), (1, Controller::Agent));
        assert!(g.renew(&lease.lease_id, T0 + 2).is_ok());
        assert!(g.bind_default("conv-2"));

        g.suspend();
        std::fs::write(&path, g.to_json()).unwrap();
        let mut again = Groups::new(12);
        again.load(&path);
        assert_eq!(again.default_session(), Some("conv-2"));
        assert_eq!(again.take_saved()[0].1.len(), 1);
    }
}
