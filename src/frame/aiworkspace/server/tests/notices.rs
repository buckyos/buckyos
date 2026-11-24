//! System Notify notices (README “Notices”): who is told of a comment and of a due to-do or event, when, once, and
//! where the notice leads.

use aiworkspace_server::auth::StaticTokens;
use aiworkspace_server::notices::{tick, Notifier};
use aiworkspace_server::wish::WishConfig;
use aiworkspace_server::{AppState, Limits};
use aiworkspace_store::objects::ObjectStore;
use aiworkspace_store::workspace::{Caller, CommitOpts};
use aiworkspace_store::Service;
use buckyos_api::{NotifyCategory, NotifyLevel, NotifySendReq};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const T0: i64 = 1_795_420_800_000; // 2026-11-23T08:00:00Z
const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

/// msg-center: one notification per key.
#[derive(Default)]
struct Center {
    sent: Mutex<BTreeMap<String, NotifySendReq>>,
    down: AtomicBool,
}

#[async_trait::async_trait]
impl Notifier for Center {
    fn available(&self) -> bool {
        true
    }
    async fn send(&self, req: NotifySendReq) -> Result<(), String> {
        if self.down.load(Ordering::SeqCst) {
            return Err("unreachable".into());
        }
        self.sent.lock().unwrap().entry(req.idempotency_key.clone()).or_insert(req);
        Ok(())
    }
}

impl Center {
    /// What was sent since the last look.
    fn take(&self) -> Vec<NotifySendReq> {
        let mut out: Vec<NotifySendReq> = std::mem::take(&mut *self.sent.lock().unwrap()).into_values().collect();
        out.sort_by(|a, b| (&a.to_user, title(a)).cmp(&(&b.to_user, title(b))));
        out
    }
}

fn title(req: &NotifySendReq) -> String {
    req.prompt.title.clone().unwrap_or_default()
}

fn message(req: &NotifySendReq) -> String {
    req.prompt.message.clone().unwrap_or_default()
}

fn url(req: &NotifySendReq) -> String {
    req.source.clone().unwrap().link.unwrap_or_default()
}

struct Env {
    state: Arc<AppState>,
    center: Arc<Center>,
    clock: Arc<AtomicI64>,
    ws: String,
    dir: tempfile::TempDir,
}

fn open(dir: &std::path::Path, clock: &Arc<AtomicI64>, center: &Arc<Center>) -> Arc<AppState> {
    let objects = ObjectStore::embedded(&dir.join("store"), buckyos_api::aiworkspace_bucket_decls(), Some(buckyos_api::AIWORKSPACE_STORAGE_QUOTA_MIB)).unwrap();
    let c = clock.clone();
    let svc = Service::open_with_clock(&dir.join("data"), objects, Arc::new(move || c.load(Ordering::SeqCst))).unwrap();
    let tokens: StaticTokens = serde_json::from_value(json!({ "tokens": {
        "a": { "principal": "alice" }, "b": { "principal": "bob" }, "c": { "principal": "carol" }, "d": { "principal": "dave" },
        "x": { "principal": "did:web:bot.example", "class": "external" } } }))
    .unwrap();
    let limits = Limits { public_base: Some("https://aiws-{workspace_id}.test.example".into()), ..Default::default() };
    let mut state = AppState::new(svc, Arc::new(tokens), limits, None, WishConfig::default());
    Arc::get_mut(&mut state).unwrap().notifier = center.clone();
    state
}

impl Env {
    /// alice's Workspace: a canvas `s1` with a Block `todo` showing the to-do table, a calendar `cal` shown by `calendar`,
    /// and a table `plain`; bob and carol may read and comment, carol not on `plain`; dave has no access.
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicI64::new(T0));
        let center = Arc::new(Center::default());
        let state = open(dir.path(), &clock, &center);
        let ws = state.svc.create_workspace(&Caller::user("alice"), "Team plan", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
        let env = Env { state, center, clock, ws, dir };
        env.commit("alice", json!([
            table("plain", None, json!([{ "field_id": "title", "name": "Title", "type": "text" }]), "a"),
            table("todos", Some("buckyos.todo@1"), json!([
                { "field_id": "title", "name": "Title", "type": "text", "required": true },
                { "field_id": "status", "name": "Status", "type": "select", "required": true, "default": "todo", "options": [
                    { "option_id": "todo", "label": "To do" }, { "option_id": "done", "label": "Done" }] },
                { "field_id": "due", "name": "Due", "type": "date", "nullable": true },
                { "field_id": "order", "name": "Order", "type": "text", "nullable": true },
                { "field_id": "done_at", "name": "Done at", "type": "datetime", "nullable": true },
                { "field_id": "ext", "name": "Ext", "type": "json", "nullable": true }]), "b"),
            table("cal", Some("buckyos.calendar@1"), json!([
                { "field_id": "title", "name": "Title", "type": "text", "required": true },
                { "field_id": "all_day", "name": "All day", "type": "boolean", "nullable": true },
                { "field_id": "start", "name": "Start", "type": "datetime", "nullable": true },
                { "field_id": "end", "name": "End", "type": "datetime", "nullable": true },
                { "field_id": "start_date", "name": "Start date", "type": "date", "nullable": true },
                { "field_id": "end_date", "name": "End date", "type": "date", "nullable": true },
                { "field_id": "status", "name": "Status", "type": "select", "nullable": true, "options": [
                    { "option_id": "confirmed", "label": "Confirmed" }, { "option_id": "tentative", "label": "Tentative" }, { "option_id": "cancelled", "label": "Cancelled" }] },
                { "field_id": "ext", "name": "Ext", "type": "json", "nullable": true }]), "c"),
            { "op": "entity.create", "entity_id": "s1-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "u", "name": "Week", "payload": { "kind": "folder", "title": "Week", "system": "surface_content", "surface_id": "s1" } },
            { "op": "entity.create", "entity_id": "s1", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "Week", "content_folder_id": "s1-content" } },
            { "op": "entity.create", "entity_id": "calendar", "type_id": "buckyos.container", "parent_id": "s1", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 800, "h": 600 },
              "payload": { "kind": "calendar", "layout": { "mode": "data" }, "title": "Calendar", "bindings": { "events": [{ "entity_id": "cal" }] }, "config": { "view": "week" } } },
        ]));
        env.state.with_ws(&env.ws, |ws| {
            let caps = json!(["read", "comment", "append", "update"]);
            ws.acl_set(&Caller::user("alice"), &json!([
                { "subject": "bob", "resource": "/", "allow": caps }, { "subject": "carol", "resource": "/", "allow": caps },
                { "subject": "carol", "resource": "plain", "deny": ["read"] }]))
        })
        .unwrap();
        env
    }

    fn commit(&self, who: &str, ops: Value) -> Value {
        self.commit_in(&self.ws, who, ops)
    }

    fn commit_in(&self, ws_id: &str, who: &str, ops: Value) -> Value {
        static KEY: AtomicU64 = AtomicU64::new(0);
        let caller = Caller::user(who);
        self.state
            .with_ws(ws_id, |ws| {
                let req = json!({ "protocol_version": "1.0", "workspace_id": ws.workspace_id, "epoch": ws.epoch,
                                  "idempotency_key": format!("t/{}", KEY.fetch_add(1, Ordering::SeqCst)), "session_id": "s", "operations": ops });
                let r = ws.commit(&req, &caller, &CommitOpts::default());
                assert_eq!(r["status"], "accepted", "{r}");
                Ok(r)
            })
            .unwrap()
    }

    fn comment(&self, who: &str, target: Value, body: &str) {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        self.commit(who, json!([{ "op": "entity.create", "entity_id": format!("note-{n}"), "type_id": "buckyos.annotation", "parent_id": "data",
                                  "order_key": format!("n{n}z"), "payload": { "target": target, "kind": "note", "body": body } }]));
    }

    fn record(&self, who: &str, source: &str, id: &str, values: Value) {
        self.commit(who, json!([{ "op": "table.insert_records", "source_id": source, "records": [{ "record_id": id, "values": values }] }]));
    }

    async fn after(&self, ms: i64) -> Vec<NotifySendReq> {
        self.clock.fetch_add(ms, Ordering::SeqCst);
        tick(&self.state).await;
        self.center.take()
    }
}

fn table(id: &str, schema: Option<&str>, fields: Value, key: &str) -> Value {
    let mut op = json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.table-source", "parent_id": "data", "order_key": key, "name": id,
                         "payload": { "title_field_id": "title", "fields": fields } });
    if let Some(schema) = schema {
        op["schema_id"] = json!(schema);
    }
    op
}

fn on(id: &str) -> Value {
    json!({ "entity_id": id })
}

fn people(sent: &[NotifySendReq]) -> Vec<&str> {
    sent.iter().map(|r| r.to_user.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_comment_tells_whom_it_concerns_and_nobody_else() {
    let env = Env::new();
    // what was there before the service looked is no news
    env.comment("bob", on("plain"), "old news");
    assert!(env.after(0).await.is_empty());

    env.comment("bob", on("calendar"), "Is Friday fixed?");
    let sent = env.after(MIN).await;
    assert_eq!(people(&sent), ["alice"], "who made it, not who wrote the comment");
    assert_eq!(title(&sent[0]), "bob commented on “Calendar”");
    assert_eq!(message(&sent[0]), "Is Friday fixed?");
    assert_eq!(sent[0].level, Some(NotifyLevel::Normal));
    assert_eq!(url(&sent[0]), format!("https://aiws-{}.test.example/team-plan/week?id=s1&block=calendar", env.ws.trim_start_matches("ws_")));
    assert!(env.after(MIN).await.is_empty(), "told once");

    // the next ones wait and go out together: the thread, and who is named — never what may not be read
    env.comment("alice", on("calendar"), "Yes.");
    env.comment("carol", on("calendar"), "@bob, @dave and @nobody: see you there. mail me@alice");
    env.comment("alice", on("plain"), "@carol cannot read this");
    env.comment("bob", on("plain"), "noted");
    assert!(env.after(MIN).await.is_empty(), "not more than once in a while");
    let sent = env.after(4 * MIN).await;
    assert_eq!(people(&sent), ["alice", "bob"], "dave has no access, carol not to the table: {sent:?}");
    assert_eq!(title(&sent[0]), "2 new comments in “Team plan”");
    assert_eq!(message(&sent[0]), "carol on “Calendar”: @bob, @dave and @nobody: see you there. mail me@alice\nbob on “plain”: noted");
    assert_eq!(sent[0].level, Some(NotifyLevel::Normal), "an address is no mention");
    assert_eq!(title(&sent[1]), "3 new comments in “Team plan”");
    assert_eq!(sent[1].level, Some(NotifyLevel::Timely), "bob was named");

    // a note on the canvas and a deleted comment are none
    env.commit("bob", json!([{ "op": "entity.create", "entity_id": "sticky", "type_id": "buckyos.annotation", "parent_id": "data", "order_key": "q", "payload": { "kind": "note", "body": "@alice" } }]));
    env.comment("bob", on("plain"), "gone");
    let id = env.state.with_ws(&env.ws, |ws| ws.list_annotations(&Caller::user("alice"), &["plain".into()], None)).unwrap()["annotations"]
        .as_array().unwrap().last().unwrap().clone();
    env.commit("bob", json!([{ "op": "entity.delete", "entity_id": id["entity_id"], "expect": { "rev": id["life_rev"] } }]));
    assert!(env.after(10 * MIN).await.is_empty());

    // msg-center was away: sent when it is back, also after a restart
    env.comment("bob", on("calendar"), "Moved to Monday");
    env.center.down.store(true, Ordering::SeqCst);
    assert!(env.after(10 * MIN).await.is_empty());
    env.center.down.store(false, Ordering::SeqCst);
    let Env { state, center, clock, ws, dir } = env;
    drop(state);
    let state = open(dir.path(), &clock, &center);
    let env = Env { state, center, clock, ws, dir };
    let sent = env.after(MIN).await;
    assert_eq!(people(&sent), ["alice", "carol"], "the maker, and who commented before");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_comment_on_a_record_concerns_who_wrote_it() {
    let env = Env::new();
    env.record("bob", "plain", "r1", json!({ "title": "Budget" }));
    env.after(0).await;
    env.comment("alice", json!({ "entity_id": "plain", "selector": { "kind": "table_record", "record_id": "r1" } }), "Too high?");
    let sent = env.after(MIN).await;
    assert_eq!(people(&sent), ["bob"]);
    assert_eq!(url(&sent[0]), format!("https://aiws-{}.test.example/team-plan", env.ws.trim_start_matches("ws_")), "the table is on no canvas");
}

#[tokio::test(flavor = "multi_thread")]
async fn reminders_come_when_they_are_due() {
    let env = Env::new();
    assert!(env.after(0).await.is_empty());
    env.record("bob", "cal", "standup", json!({ "title": "Stand-up", "start": "2026-11-23T09:00:00Z", "ext": { "reminders": [{ "before_min": 10 }, { "before_min": 0 }], "location": "Room 1" } }));
    env.record("alice", "cal", "past", json!({ "title": "Past", "start": "2026-11-23T07:59:00Z", "ext": { "reminders": [{ "before_min": 0 }] } }));
    env.record("alice", "cal", "silent", json!({ "title": "Silent", "start": "2026-11-23T09:00:00Z" }));
    env.record("alice", "cal", "agents", json!({ "title": "Kept by an Agent", "start": "2026-11-23T09:00:00Z", "ext": { "reminders": [{ "before_min": 0 }], "maintenance": { "maintainer": "jarvis" } } }));
    env.record("alice", "cal", "off", json!({ "title": "Cancelled", "start": "2026-11-23T09:00:00Z", "status": "cancelled", "ext": { "reminders": [{ "before_min": 0 }] } }));
    assert!(env.after(49 * MIN).await.is_empty());
    let sent = env.after(MIN).await;
    assert_eq!(people(&sent), ["bob"], "who wrote the event");
    assert_eq!((title(&sent[0]), message(&sent[0])), ("Stand-up".into(), "Starts 2026-11-23 09:00 (UTC) · Room 1".into()));
    assert_eq!((sent[0].level, sent[0].category), (Some(NotifyLevel::Timely), None));
    assert_eq!(url(&sent[0]), format!("https://aiws-{}.test.example/team-plan/week?id=s1&block=calendar&record=standup", env.ws.trim_start_matches("ws_")));
    assert_eq!(sent[0].expires_at_ms, Some((T0 + 25 * HOUR) as u64));
    assert!(env.after(MIN).await.is_empty());
    let sent = env.after(9 * MIN).await;
    assert_eq!(people(&sent), ["bob"], "when it starts, and the tick after the service was away: {sent:?}");

    // moved: the new time reminds; a tick that comes late still sends, one that comes far too late does not
    let rev = |field: &str| {
        let v = env.state.with_ws(&env.ws, |ws| ws.read(&Caller::user("alice"), "cal", Some(&json!({ "kind": "table_record", "record_id": "standup" })))).unwrap();
        v["content"]["revs"][field].as_u64().unwrap_or_else(|| panic!("{v}"))
    };
    let move_to = |at: &str| {
        env.commit("bob", json!([{ "op": "table.set_values", "source_id": "cal", "values": [{ "record_id": "standup", "field_id": "start", "value": at, "expect": { "rev": rev("start") } }] }]));
    };
    move_to("2026-11-23T10:00:00Z");
    assert_eq!(env.after(110 * MIN).await.len(), 1, "ten minutes before is more than an hour ago, the start is not");
    move_to("2026-11-23T13:00:00Z");
    assert!(env.after(5 * HOUR).await.is_empty());

    // repeating in its own zone, without the excluded and the moved occurrence
    env.record("alice", "cal", "weekly", json!({ "title": "Review", "start": "2026-11-23T15:00:00Z", "ext": { "rrule": "FREQ=DAILY", "tz": "America/New_York",
        "reminders": [{ "before_min": 30 }], "exdate": ["2026-11-25T15:00:00.000Z"] } }));
    env.record("alice", "cal", "moved", json!({ "title": "Review (moved)", "start": "2026-11-26T18:00:00Z", "ext": { "recurrence_id": { "master": "weekly", "original": "2026-11-26T15:00:00Z" }, "reminders": [{ "before_min": 30 }] } }));
    env.after(0).await;
    let mut seen = Vec::new();
    while env.clock.load(Ordering::SeqCst) < T0 + 5 * 24 * HOUR {
        for r in env.after(30 * MIN).await {
            seen.push((title(&r), message(&r), url(&r).contains("&occ=")));
        }
    }
    let expected = [
        ("Review", "Starts 2026-11-24 15:00 (UTC)", true),
        ("Review (moved)", "Starts 2026-11-26 18:00 (UTC)", false),
        ("Review", "Starts 2026-11-27 15:00 (UTC)", true),
    ]
    .map(|(t, m, o)| (t.to_string(), m.to_string(), o));
    assert_eq!(seen, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_to_do_reminds_its_assignee_in_their_zone() {
    let env = Env::new();
    env.state.svc.users.set_prefs(&Caller::user("bob"), &json!({ "merge": { "timezone": "Asia/Shanghai" } }), T0).unwrap();
    env.after(0).await;
    let due = |who: &str| json!({ "title": format!("For {who}"), "due": "2026-11-24", "ext": { "assignee": who, "reminders": [{ "before_min": 0 }] } });
    for who in ["bob", "dave", "somebody else"] {
        env.record("alice", "todos", &format!("t-{}", who.replace(' ', "-")), due(who));
    }
    env.record("alice", "todos", "t-done", json!({ "title": "Done", "status": "done", "due": "2026-11-24", "ext": { "reminders": [{ "before_min": 0 }] } }));
    // nine o'clock of the day in Shanghai, then in UTC for alice, who wrote those nobody else may be told of
    let mut seen = Vec::new();
    for _ in 0..52 {
        for r in env.after(30 * MIN).await {
            assert_eq!((message(&r), r.category), ("Due 2026-11-24".into(), Some(NotifyCategory::Todo)));
            seen.push(((env.clock.load(Ordering::SeqCst) - T0) / HOUR, r.to_user.clone(), title(&r)));
        }
    }
    assert_eq!(seen, [(17, "bob".into(), "For bob".into()), (25, "alice".into(), "For dave".into()), (25, "alice".into(), "For somebody else".into())]
        .map(|(h, u, t): (i64, String, String)| (h, u, t)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_does_not_remind_until_it_is_worked_on() {
    let env = Env::new();
    env.record("alice", "cal", "e1", json!({ "title": "Launch", "start": "2026-11-23T09:00:00Z", "ext": { "reminders": [{ "before_min": 0 }] } }));
    let copy = env.state.svc.clone_workspace(&Caller::user("alice"), &env.ws).unwrap()["workspace_id"].as_str().unwrap().to_string();
    env.after(0).await;
    env.commit_in(&copy, "alice", json!([{ "op": "table.insert_records", "source_id": "cal", "records": [{ "record_id": "e2", "values": { "title": "Copy's own", "start": "2026-11-23T09:00:00Z", "ext": { "reminders": [{ "before_min": 0 }] } } }] }]));
    let sent = env.after(HOUR).await;
    assert_eq!(sent.iter().map(title).collect::<Vec<_>>(), ["Copy's own", "Launch"]);
    assert!(sent[1].idempotency_key.contains(&env.ws));
}
