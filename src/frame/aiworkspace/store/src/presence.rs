//! Presence (UI 框架 PRD §4.2, §12, K2): what the list of online members shows of one person to one reader. The
//! connections themselves live in the server's memory and never enter the document's history; this module judges,
//! against the Workspace's permissions as they are now, on which Surface a person may be shown and whose groups a
//! reader may see (Q6: a manager sees everyone's, anyone else only its own — listing others' groups would disclose
//! memberships a non-manager may not list).

use crate::acl::judge;
use crate::workspace::{Caller, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::model::{ReadCtx, TYPE_CONTAINER};
use aiworkspace_core::WsResult;
use serde_json::{json, Value};

/// Longest entity id a presence report may name.
const MAX_ID_LEN: usize = 128;

/// One online person as the server knows it: its caller at its latest heartbeat (memberships settled then) and the
/// Surface its most recently active window reported.
pub struct PresenceInput<'a> {
    pub caller: &'a Caller,
    pub surface_id: Option<&'a str>,
}

impl Workspace {
    /// `surface_id` when it names a live Surface `caller` may read; `None` otherwise (a deleted, unreadable or
    /// unknown one is not shown).
    pub fn readable_surface(&self, caller: &Caller, surface_id: &str) -> WsResult<Option<String>> {
        if surface_id.is_empty() || surface_id.len() > MAX_ID_LEN {
            return Ok(None);
        }
        let access = self.require_ws_any(caller)?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let Some(e) = ctx.entity(surface_id)? else { return Ok(None) };
        let surface = e.alive() && e.type_id == TYPE_CONTAINER && e.payload.get("kind").and_then(Value::as_str) == Some("surface");
        Ok((surface && access.can_read(&ctx, &e)?).then(|| surface_id.to_string()))
    }

    /// Each person as `viewer` sees it, in the given order: `principal`, `self`, `surface_id` (only a Surface the viewer
    /// may read too) and — to a manager, or about the viewer itself — `owner` and `groups` (`[{ group_id, title }]`, the
    /// logical groups it belongs to now). A membership that cannot be settled leaves both out.
    pub fn presence_view(&self, viewer: &Caller, people: &[PresenceInput]) -> WsResult<Vec<Value>> {
        let access = self.require_ws_any(viewer)?;
        let manages = access.ws_caps.has(Cap::Manage);
        let data = self.acl_data()?;
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::with_capacity(people.len());
        for p in people {
            let me = p.caller.principal == viewer.principal;
            let surface = match p.surface_id {
                Some(id) if me => Some(id.to_string()),
                Some(id) => match ctx.entity(id)? {
                    Some(e) if e.alive() && access.can_read(&ctx, &e)? => Some(id.to_string()),
                    _ => None,
                },
                None => None,
            };
            let mut v = json!({ "principal": p.caller.principal, "self": me, "guest": p.caller.is_guest(), "surface_id": surface });
            if manages || me {
                if let Ok(j) = judge(&data, p.caller) {
                    let groups: Vec<Value> = j
                        .groups
                        .keys()
                        .filter_map(|g| data.groups.get(g))
                        .map(|g| json!({ "group_id": g.group_id, "title": if g.title.is_empty() { &g.group_id } else { &g.title } }))
                        .collect();
                    v["owner"] = json!(j.owner);
                    v["groups"] = json!(groups);
                }
            }
            out.push(v);
        }
        Ok(out)
    }
}
