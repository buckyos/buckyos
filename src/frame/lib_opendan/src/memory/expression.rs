//! Expression review of a consolidation plan: advice on how a cognition is
//! worded, never a rule. Whether a cognition is traceable, in range and
//! useful is the consolidation's judgment; this only names statements that
//! look like a log, run long, or repeat a cognition already held, so the
//! model rewrites them or keeps them on purpose.

use agent_tool::agent_memory::{claim_summary, fts_tokens, GraphOperation, GraphView, Scope};

/// A statement longer than this is asked to be looked at again.
pub const LONG_STATEMENT_CHARS: usize = 400;
/// Token overlap (Jaccard) from which a new statement repeats a known one.
const NEAR_DUPLICATE: f64 = 0.6;

fn is_session_id(word: &str) -> bool {
    let Some((prefix, rest)) = word.split_once('-') else {
        return false;
    };
    matches!(prefix, "ui" | "work" | "goal" | "sc")
        && rest.len() >= 6
        && rest.chars().any(|c| c.is_ascii_digit())
        && rest.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Session ids named in the text.
fn session_ids(text: &str) -> usize {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|w| is_session_id(w))
        .count()
}

/// Times of day (`hh:mm`) in the text: the trail of a log. A date, a
/// version or a period is not one.
fn clock_times(text: &str) -> usize {
    let b = text.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    (0..b.len())
        .filter(|&i| {
            b[i] == b':'
                && i >= 2
                && digit(i - 1)
                && digit(i - 2)
                && digit(i + 1)
                && digit(i + 2)
                && !(i >= 3 && b[i - 3] == b':')
        })
        .count()
}

fn overlap(a: &str, b: &str) -> f64 {
    let (a, b) = (fts_tokens(a), fts_tokens(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let both = a.intersection(&b).count();
    both as f64 / (a.len() + b.len() - both) as f64
}

/// Notes on the statements a plan writes (empty: nothing to look at).
pub fn review_expression(view: &GraphView, operations: &[GraphOperation]) -> Vec<String> {
    let mut notes = Vec::new();
    for op in operations {
        let GraphOperation::PutItem(p) = op else { continue };
        let statement = claim_summary(&p.claim);
        let name = p.item_id.as_deref().unwrap_or("a new item");
        let chars = statement.chars().count();
        if chars > LONG_STATEMENT_CHARS {
            notes.push(format!(
                "{name}: the statement has {chars} characters; keep the conclusion with the conditions that change the action, and move the course of events into the observation"
            ));
        }
        let (sids, times) = (session_ids(&statement), clock_times(&statement));
        if sids > 0 || times >= 2 {
            notes.push(format!(
                "{name}: the statement reads like a log ({sids} session id(s), {times} time(s) of day); session ids and event times belong in the observation, a date, version or period that changes the meaning stays"
            ));
        }
        let known = p.item_id.as_deref().and_then(|id| view.item(id));
        if known.is_some() {
            continue;
        }
        let scope: Option<&Scope> = p.meta.scope.as_ref();
        let twin = view.items().find(|i| {
            i.status.recallable()
                && !p.replaces.contains(&i.item_id)
                && match (scope, &i.scope) {
                    (Some(a), Some(b)) => a.overlaps(b),
                    _ => false,
                }
                && overlap(&statement, &i.statement()) >= NEAR_DUPLICATE
        });
        if let Some(t) = twin {
            notes.push(format!(
                "{name}: the statement is close to the known {}; when it is the same conclusion in the same range, revise that item (its item_id with expected_revision) instead of adding one",
                t.reference()
            ));
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_versions_are_not_a_log() {
        assert_eq!(clock_times("tool-x v1.2 on Linux, since 2026-11-08, ratio 16:9"), 0);
        assert_eq!(clock_times("valid 2026-11-01 to 2026-11-30"), 0);
        assert_eq!(clock_times("at 10:42 the run failed, at 10:57 it passed"), 2);
        assert_eq!(clock_times("2026-10-27T23:00:15Z then 2026-10-30T10:00:02Z"), 2);
        assert_eq!(session_ids("in work-20261027-3 and ui-3f9a2c1b77 it was reported"), 2);
        assert_eq!(session_ids("the follow-up work-around for tool-x in ui-design"), 0);
    }

    #[test]
    fn overlap_is_one_for_the_same_text_and_low_for_another() {
        assert!((overlap("用户要求项目 A 的周报一页以内", "用户要求项目 A 的周报一页以内") - 1.0).abs() < 1e-9);
        assert!(overlap("用户要求项目 A 的周报一页以内", "tool-x v1 needs an output directory") < 0.1);
    }
}
