//! The background context of an input batch: the semi-subscription state
//! the session runs against, one line per fact, and only the lines that
//! changed since they were last shown (`SessionState.background_shown`,
//! committed with the batch).
//!
//! It is never an input of its own: it is material of the batch's input
//! message (`background` / `background_context` of the input templates),
//! and a line counts as shown only when the rendered message contains it.
//! A template that does not place it leaves the session blind to it.
//!
//! A UI session's chat format has the time, the runtime, the work it
//! started, other conversations' topics, state versions (notifications),
//! hints; every session has its state versions and the new output of the
//! tasks it follows. The end or need of a sub session is its event, never
//! a line.
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
    /// What is recorded once the line is shown (default: its text's
    /// digest). A line without text only records it.
    #[serde(skip)]
    pub mark: Option<String>,
}

impl BackgroundLine {
    pub(super) fn new(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            text: text.into(),
            mark: None,
        }
    }

    pub(super) fn marked(key: impl Into<String>, text: impl Into<String>, mark: String) -> Self {
        Self {
            mark: Some(mark),
            ..Self::new(key, text)
        }
    }
}

/// Key of the time line (its digest is `<epoch ms>|<time zone>`).
pub const KEY_TIME: &str = "time";

/// What one UI batch's background is made of.
pub struct BackgroundInput<'a> {
    pub sid: &'a str,
    pub now_ms: u64,
    /// The user's time zone (IANA name), `None`: UTC.
    pub timezone: Option<&'a str>,
    pub runtime_status: &'a Value,
    pub active: &'a [ActiveSession],
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

/// Every candidate line of a UI batch, unfiltered.
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
    // The work this session started and that still runs (its end or need
    // is its event).
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
            out.push(BackgroundLine::new(format!("work:{}", a.session_id), t));
        }
    }
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
    // The time line carries the user's time zone.
    out.extend(event_lines(
        i.events
            .iter()
            .filter(|e| !(e.source.kind == "system" && e.source.id == USER_TIMEZONE_SOURCE_ID)),
    ));
    for h in i.hints {
        out.push(BackgroundLine::new(
            format!("hint:{}", h.id),
            match &h.object {
                Some(object) => format!("hint: {} (noticed {}; about {object})", one_line(&h.sentence, 300), h.time),
                None => format!("hint: {} (noticed {})", one_line(&h.sentence, 300), h.time),
            },
        ));
    }
    out
}

/// One line per semi-subscription state version (`event:<key>`): a
/// notification as it is, anything else `<source> <event>: <summary>`.
pub fn event_lines<'a>(events: impl IntoIterator<Item = &'a EventView>) -> Vec<BackgroundLine> {
    events
        .into_iter()
        .map(|e| {
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
            BackgroundLine::new(format!("event:{}", e.key), text)
        })
        .collect()
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
        let d = line.mark.clone().unwrap_or_else(|| digest(&line.text));
        if !line.text.is_empty() && shown.get(&line.key) != Some(&d) {
            out.push(line.clone());
        }
        next.insert(line.key, d);
    }
    (out, next)
}

/// After the input is rendered: the keys of the selected lines its text
/// really shows. The others are recorded as they were before (`next` from
/// [`select`]), so a later input that places them shows them.
pub fn settle(
    next: &mut BTreeMap<String, String>,
    before: &BTreeMap<String, String>,
    selected: &[BackgroundLine],
    text: &str,
) -> Vec<String> {
    let mut shown = Vec::new();
    for l in selected {
        if text.contains(&l.text) {
            shown.push(l.key.clone());
            continue;
        }
        match before.get(&l.key) {
            Some(v) => next.insert(l.key.clone(), v.clone()),
            None => next.remove(&l.key),
        };
    }
    shown
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
            object: None,
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

    #[test]
    fn a_line_counts_as_shown_only_when_the_input_contains_it() {
        let before = BTreeMap::from([("task:t1".to_string(), "3,0".to_string())]);
        let candidates = vec![
            BackgroundLine::new("hint:h1", "hint: a"),
            BackgroundLine::marked("task:t1", "task t1 · new output:\nb", "5,0".into()),
            // Recorded, never shown.
            BackgroundLine::marked("task:t2", "", "7,0".into()),
        ];
        let (selected, mut next) = select(candidates, &before, 0);
        assert_eq!(selected.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(), ["hint:h1", "task:t1"]);
        assert_eq!(next["task:t2"], "7,0");
        // A template that places only the hint.
        let shown = settle(&mut next, &before, &selected, "- hint: a\nhello");
        assert_eq!(shown, ["hint:h1"]);
        assert_eq!(next["task:t1"], "3,0", "the new output comes again");
        assert_eq!(next["hint:h1"], digest("hint: a"));
        // A template that places none of it.
        let (selected, mut next) = select(vec![BackgroundLine::new("hint:h2", "hint: b")], &BTreeMap::new(), 0);
        assert!(settle(&mut next, &BTreeMap::new(), &selected, "hello").is_empty());
        assert!(next.is_empty());
    }
}
