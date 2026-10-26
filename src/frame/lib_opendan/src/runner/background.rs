//! The background context of the chat input format (UI sessions): one line
//! per fact the conversation runs against — the time, the runtime, the
//! work this session started, other conversations' topics, notifications
//! shown, hints — and only the lines that changed since they were last
//! shown (`SessionState.background_shown`, committed with the batch).
//!
//! ```text
//! <<background_context>>
//! - current_time 2026-10-16T10:45:45-07:00 (America/Los_Angeles)
//! - work work-1248…: running — reviewing PR #667
//! <</background_context>>
//! ```

use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::protocol::*;
use crate::state::{ActiveSession, Hint, Relation};

use super::input_view::EventView;

/// One line: a stable key (what it is about) and its text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BackgroundLine {
    pub key: String,
    pub text: String,
}

impl BackgroundLine {
    pub(super) fn new(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            text: text.into(),
        }
    }
}

/// Key of the time line (its digest is `<epoch ms>|<time zone>`).
pub const KEY_TIME: &str = "time";

/// What one batch's background is made of.
pub struct BackgroundInput<'a> {
    pub sid: &'a str,
    pub now_ms: u64,
    /// The user's time zone (IANA name), `None`: UTC.
    pub timezone: Option<&'a str>,
    pub runtime_status: &'a Value,
    pub active: &'a [ActiveSession],
    /// Sub sessions whose end (or need) this batch delivers, as registered.
    pub children: &'a [RegistryEntry],
    /// Semi-subscription state versions shown with this batch.
    pub events: &'a [EventView],
    pub hints: &'a [Hint],
    /// DID of the tunnel the batch's last message came through.
    pub tunnel: Option<&'a str>,
}

fn one_line(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `2026-10-16T10:45:45-07:00` in `tz` (UTC with `Z` when there is none or
/// it is unknown), and the zone's name.
pub fn local_time(ms: u64, tz: Option<&str>) -> (String, String) {
    let utc = chrono::DateTime::from_timestamp_millis(ms as i64).unwrap_or_default();
    match tz.and_then(|t| t.parse::<chrono_tz::Tz>().ok()) {
        Some(zone) => (
            utc.with_timezone(&zone)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
            zone.name().to_string(),
        ),
        None => (
            utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "UTC".to_string(),
        ),
    }
}

fn outcome_name(o: Option<Outcome>) -> String {
    o.map(|o| format!("{o:?}").to_lowercase())
        .unwrap_or_else(|| "no outcome".into())
}

pub(super) fn child_line(e: &RegistryEntry) -> BackgroundLine {
    let st = &e.status;
    let sid = &e.session_id;
    let status = one_line(&st.one_line_status, 200);
    let text = if st.run_state == RunState::Finished {
        let mut t = format!("work {sid}: finished ({})", outcome_name(st.outcome));
        let status = status
            .strip_prefix("finished:")
            .unwrap_or(&status)
            .trim()
            .trim_end_matches(['.', '。', '!', '！', ';', '；']);
        if !status.is_empty() {
            t.push_str(&format!(" — {status}"));
        }
        t.push_str(&format!(". Read: agent-session read-session {sid} --report"));
        t
    } else if st.pending_decision.is_some() {
        format!("work {sid}: waits for a decision")
    } else if st.waiting_for == Some(WaitingKind::Input) {
        format!("work {sid}: waits for input — answer with agent-session post {sid} --msg <text>")
    } else {
        let mut t = format!("work {sid}: {}", st.run_state.as_str());
        if !status.is_empty() {
            t.push_str(&format!(" — {status}"));
        }
        t
    };
    BackgroundLine::new(format!("work:{sid}"), text)
}

/// Every candidate line of a batch, unfiltered.
pub fn lines(i: &BackgroundInput) -> Vec<BackgroundLine> {
    let mut out = Vec::new();
    let (time, zone) = local_time(i.now_ms, i.timezone);
    out.push(BackgroundLine::new(KEY_TIME, format!("current_time {time} ({zone})")));
    let field = |k: &str| {
        i.runtime_status
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let kind = field("kind");
    if !kind.is_empty() {
        let mut t = format!("runtime {kind}");
        let host = field("hostname");
        if !host.is_empty() {
            t.push_str(&format!(" on {host}"));
        }
        let cwd = field("cwd");
        if !cwd.is_empty() {
            t.push_str(&format!(", cwd {cwd}"));
        }
        let shell = field("shell_kind");
        if !shell.is_empty() {
            t.push_str(&format!(", shell {shell}"));
        }
        let baseline = field("baseline");
        if !baseline.is_empty() {
            t.push_str(&format!(", {baseline} environment"));
        }
        out.push(BackgroundLine::new("runtime", t));
    }
    if let Some(w) = i.runtime_status.get("workspace").filter(|w| w.is_object()) {
        let s = |k: &str| w.get(k).and_then(Value::as_str).unwrap_or_default();
        let dir = w.get("directory").and_then(Value::as_str).unwrap_or_default();
        let reach = if w.get("same_runtime").and_then(Value::as_bool) == Some(true) {
            "on this runtime: cd there or use absolute paths"
        } else {
            "on another runtime: copy files with agent-session copy"
        };
        out.push(BackgroundLine::new(
            "workspace",
            format!("workspace {} at {dir} on runtime {} ({reach})", s("workspace_id"), s("runtime_id")),
        ));
    }
    if let Some(t) = i.tunnel.filter(|t| !t.is_empty()) {
        out.push(BackgroundLine::new("channel", format!("channel: the user writes through {t}")));
    }
    // The work this session started: what the registry says now; an end
    // delivered in this batch wins over the active list.
    let mut work: BTreeMap<String, BackgroundLine> = BTreeMap::new();
    for a in i.active {
        if a.parent.as_deref() == Some(i.sid) {
            let summary = if a.activity.summary.is_empty() {
                &a.objective
            } else {
                &a.activity.summary
            };
            let mut t = format!("work {}: {}", a.session_id, a.run_state.as_str());
            let summary = one_line(summary, 200);
            if !summary.is_empty() {
                t.push_str(&format!(" — {summary}"));
            }
            work.insert(
                a.session_id.clone(),
                BackgroundLine::new(format!("work:{}", a.session_id), t),
            );
        }
    }
    for e in i.children {
        work.insert(e.session_id.clone(), child_line(e));
    }
    out.extend(work.into_values());
    for a in i.active {
        if a.parent.as_deref() == Some(i.sid) {
            continue;
        }
        if a.kind == SessionKind::Ui {
            if let Some(title) = a
                .topic
                .as_ref()
                .map(|t| one_line(&t.title, 120))
                .filter(|t| !t.is_empty())
            {
                out.push(BackgroundLine::new(
                    format!("ui:{}", a.session_id),
                    format!("conversation {} is about: {title}", a.session_id),
                ));
            }
        } else if a.relation != Relation::Other {
            let summary = if a.activity.summary.is_empty() {
                &a.objective
            } else {
                &a.activity.summary
            };
            let mut t = format!(
                "session {} ({}) is {} — {}",
                a.session_id,
                a.kind.as_str(),
                a.run_state.as_str(),
                one_line(summary, 160)
            );
            if !a.overlap.is_empty() {
                t.push_str(&format!("; it touches {}", a.overlap.join(", ")));
            }
            out.push(BackgroundLine::new(format!("session:{}", a.session_id), t));
        }
    }
    for e in i.events {
        // The time line carries the user's time zone.
        if e.source.kind == "system" && e.source.id == USER_TIMEZONE_SOURCE_ID {
            continue;
        }
        let notify = e.source.kind == "system"
            && (e.source.id == NOTIFY_SOURCE_ID
                || e.source.id.starts_with(&format!("{NOTIFY_SOURCE_ID}:")));
        let text = if notify {
            one_line(&e.summary, 600)
        } else {
            format!(
                "{}:{} {}: {}",
                e.source.kind,
                e.source.id,
                e.event,
                one_line(&e.summary, 400)
            )
        };
        out.push(BackgroundLine::new(format!("event:{}", e.key), text));
    }
    for h in i.hints {
        out.push(BackgroundLine::new(
            format!("hint:{}", h.id),
            format!("hint: {} (noticed {})", one_line(&h.sentence, 300), h.time),
        ));
    }
    out
}

pub(super) fn digest(text: &str) -> String {
    hex::encode(&Sha256::digest(text.as_bytes())[..8])
}

/// The lines to show now, and the `background_shown` map after showing
/// them: a line whose text changed since it was last shown (the time line:
/// a new time zone, or [`BACKGROUND_TIME_STEP_MS`] passed). Keys no longer
/// among the candidates are dropped from the map (shown again if they come
/// back).
pub fn select(
    candidates: Vec<BackgroundLine>,
    shown: &BTreeMap<String, String>,
    now_ms: u64,
) -> (Vec<BackgroundLine>, BTreeMap<String, String>) {
    let mut out = Vec::new();
    let mut next = BTreeMap::new();
    for line in candidates {
        if line.key == KEY_TIME {
            let zone = line
                .text
                .rsplit_once('(')
                .map(|(_, z)| z.trim_end_matches(')').to_string())
                .unwrap_or_default();
            let previous = shown.get(KEY_TIME).and_then(|v| {
                let (ms, z) = v.split_once('|')?;
                Some((ms.parse::<u64>().ok()?, z.to_string()))
            });
            let due = match &previous {
                Some((ms, z)) => *z != zone || now_ms.saturating_sub(*ms) >= BACKGROUND_TIME_STEP_MS,
                None => true,
            };
            if due {
                next.insert(KEY_TIME.to_string(), format!("{now_ms}|{zone}"));
                out.push(line);
            } else if let Some(v) = shown.get(KEY_TIME) {
                next.insert(KEY_TIME.to_string(), v.clone());
            }
            continue;
        }
        let d = digest(&line.text);
        if shown.get(&line.key) != Some(&d) {
            out.push(line.clone());
        }
        next.insert(line.key, d);
    }
    (out, next)
}

/// The `<<background_context>>` block (empty without lines).
pub fn render(lines: &[BackgroundLine]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut s = String::from("<<background_context>>\n");
    for l in lines {
        s.push_str("- ");
        s.push_str(&l.text);
        s.push('\n');
    }
    s.push_str("<</background_context>>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(now_ms: u64, hints: &'a [Hint]) -> BackgroundInput<'a> {
        BackgroundInput {
            sid: "ui-1",
            now_ms,
            timezone: Some("America/Los_Angeles"),
            runtime_status: Box::leak(Box::new(serde_json::json!({"kind": "native", "hostname": "pve", "cwd": "/s"}))),
            active: &[],
            children: &[],
            events: &[],
            hints,
            tunnel: None,
        }
    }

    #[test]
    fn local_time_uses_the_users_zone() {
        // 2026-10-16T17:45:45Z
        let ms = 1_792_172_745_000;
        assert_eq!(
            local_time(ms, Some("America/Los_Angeles")),
            ("2026-10-16T10:45:45-07:00".to_string(), "America/Los_Angeles".to_string())
        );
        assert_eq!(local_time(ms, None).0, "2026-10-16T17:45:45Z");
        assert_eq!(local_time(ms, Some("Mars/Base")).1, "UTC");
    }

    #[test]
    fn only_changed_lines_are_shown_and_time_every_five_seconds() {
        let t0 = 1_792_172_745_000;
        let hints = [Hint {
            id: "h1".into(),
            time: "2026-10-02".into(),
            sentence: "owner prefers Chinese".into(),
            kind: String::new(),
        }];
        let (shown, map) = select(lines(&input(t0, &hints)), &BTreeMap::new(), t0);
        let keys: Vec<&str> = shown.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(keys, ["time", "runtime", "hint:h1"]);
        assert!(render(&shown).contains("- current_time 2026-10-16T10:45:45-07:00 (America/Los_Angeles)\n"));
        // 2 s later nothing changed: nothing at all.
        let (shown, map2) = select(lines(&input(t0 + 2_000, &hints)), &map, t0 + 2_000);
        assert!(shown.is_empty(), "{shown:?}");
        assert_eq!(render(&shown), "");
        // 6 s after the first: the time line only.
        let (shown, map3) = select(lines(&input(t0 + 6_000, &hints)), &map2, t0 + 6_000);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].key, "time");
        // A hint that is gone is forgotten; it is shown again when back.
        let (_, map4) = select(lines(&input(t0 + 7_000, &[])), &map3, t0 + 7_000);
        assert!(!map4.contains_key("hint:h1"));
        let (shown, _) = select(lines(&input(t0 + 8_000, &hints)), &map4, t0 + 8_000);
        assert_eq!(shown.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(), ["hint:h1"]);
    }
}
