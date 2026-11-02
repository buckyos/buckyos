//! The session feeds (第四期规划 §5.3, Q19, TODO I6): seven fixed Tables a Block binds to. The fields are part of the
//! protocol: a courseware's sample entity and the live feed of a session have the same ones, so a Block bound at
//! design time keeps working when the session writes real rows.

use aiworkspace_core::canonical::sha256_hex;
use serde_json::{json, Value};

/// `(field_id, name, type)` of each feed, in column order.
pub fn columns(feed: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    match feed {
        "participants" => &[
            ("name", "Name", "text"),
            ("principal", "Principal", "text"),
            ("role", "Role", "text"),
            ("level", "Identity level", "text"),
            ("online", "Online", "boolean"),
            ("last_seen", "Last seen", "text"),
            ("joined_at", "Joined", "text"),
        ],
        "stage" => &[
            ("step_id", "Step", "text"),
            ("step_index", "Step number", "number"),
            ("step_count", "Steps", "number"),
            ("step_title", "Step title", "text"),
            ("black", "Black screen", "boolean"),
            ("free", "Free camera", "boolean"),
            ("transition", "Transition", "text"),
            ("media", "Media", "text"),
            ("followers", "Followers", "number"),
            ("updated_at", "Updated", "text"),
        ],
        "tasks" => &[
            ("task_id", "Task", "text"),
            ("title", "Title", "text"),
            ("mode", "Mode", "text"),
            ("state", "State", "text"),
            ("slots", "Slots", "number"),
            ("claimed", "Claimed", "number"),
            ("answered", "Answered", "number"),
            ("picked", "Picked", "text"),
            ("block_id", "Block", "text"),
            ("created_at", "Created", "text"),
        ],
        "my_responses" => &[
            ("task_id", "Task", "text"),
            ("state", "State", "text"),
            ("content", "Answer", "text"),
            ("version", "Version", "number"),
            ("submitted_at", "Submitted", "text"),
            ("verdict", "Verdict", "text"),
            ("score", "Score", "number"),
            ("comment", "Comment", "text"),
        ],
        "responses" => &[
            ("response_id", "Response", "text"),
            ("task_id", "Task", "text"),
            ("participant", "Participant", "text"),
            ("name", "Name", "text"),
            ("level", "Identity level", "text"),
            ("state", "State", "text"),
            ("content", "Answer", "text"),
            ("submitted_at", "Submitted", "text"),
            ("late", "Late", "boolean"),
        ],
        "results" => &[
            ("task_id", "Task", "text"),
            ("title", "Title", "text"),
            ("content", "Content", "text"),
            ("published_at", "Published", "text"),
        ],
        "reactions" => &[
            ("kind", "Kind", "text"),
            ("text", "Text", "text"),
            ("count", "Count", "number"),
            ("x", "X", "number"),
            ("y", "Y", "number"),
            ("target_id", "Target", "text"),
            ("at", "At", "text"),
        ],
        _ => &[],
    }
}

/// The FieldDef values of a feed.
pub fn fields(feed: &str) -> Vec<Value> {
    columns(feed).iter().map(|(id, name, ty)| json!({ "field_id": id, "name": name, "type": ty })).collect()
}

pub fn title(feed: &str) -> &'static str {
    match feed {
        "participants" => "Session · participants",
        "stage" => "Session · stage",
        "tasks" => "Session · tasks",
        "my_responses" => "Session · my answers",
        "responses" => "Session · all answers",
        "results" => "Session · published results",
        "reactions" => "Session · reactions",
        _ => "Session feed",
    }
}

/// Who reads a feed by default (§5.3 table): `presenter` scope for the private ones, a participant's own scope for
/// `my_responses`, shared for the rest.
pub fn default_scope(feed: &str) -> &'static str {
    match feed {
        "participants" | "responses" => aiworkspace_core::model::SCOPE_PRESENTER,
        _ => aiworkspace_core::model::SCOPE_SHARED,
    }
}

/// A short stable record id for a principal (principals are DIDs, user ids or `guest:` ids, not ids themselves).
pub fn principal_key(principal: &str) -> String {
    format!("p{}", &sha256_hex(principal.as_bytes())[..30])
}

/// The id of a participant's own `my_responses` table, derived from the courseware's sample entity.
pub fn my_feed_id(sample_id: &str, principal: &str) -> String {
    format!("mf{}", &sha256_hex(format!("{sample_id}\n{principal}").as_bytes())[..30])
}

/// The id of a participant's response to a task: one per task and participant, so claiming twice is the same entity.
pub fn response_id(task_id: &str, principal: &str) -> String {
    format!("rs{}", &sha256_hex(format!("{task_id}\n{principal}").as_bytes())[..30])
}

/// Example rows of a courseware's sample entity (Q20): the author sees what the Block will show; a session clears them.
pub fn sample_records(feed: &str) -> Vec<Value> {
    let rec = |id: &str, values: Value| json!({ "record_id": id, "values": values });
    match feed {
        "participants" => vec![
            rec("sample-a", json!({ "name": "Student A", "principal": "did:example:a", "role": "audience", "level": "verified", "online": true, "joined_at": "" })),
            rec("sample-b", json!({ "name": "Student B", "principal": "did:example:b", "role": "audience", "level": "verified", "online": false, "joined_at": "" })),
        ],
        "stage" => vec![rec("stage", json!({ "step_id": "", "step_index": 1, "step_count": 1, "step_title": "Step 1", "black": false, "free": false, "followers": 0 }))],
        "tasks" => vec![rec("sample-task", json!({ "task_id": "sample-task", "title": "Question 1", "mode": "open", "state": "open", "answered": 0, "claimed": 0 }))],
        "responses" => vec![rec("sample-response", json!({ "response_id": "sample-response", "task_id": "sample-task", "participant": "did:example:a", "name": "Student A", "level": "verified", "state": "submitted", "content": "\"A\"" }))],
        _ => vec![],
    }
}
