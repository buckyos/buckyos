//! The permission model (权限需求 §4.6): entries grant (`allow`) and refuse (`deny`) capabilities on the whole
//! Workspace (`""`, the path `/`) or on an entity subtree. A capability is decided by the nearest layer — the entity,
//! then each structural ancestor, then the Workspace — that has an entry mentioning it; within that layer by the most
//! specific tier that mentions it (single caller, then logical groups and `@creator`, then `@guest` and `@all`), and
//! within the tier a refusal wins. Nothing mentioning it anywhere: refused. The owner holds everything. No field or
//! record level grants.

use crate::error::{WsError, WsResult};
use crate::model::{EntityRow, JsonMap, ReadCtx, ROOT_ID, SCOPE_PRESENTER, TYPE_ANNOTATION, TYPE_CELL};
use crate::types::annotation_author;
use crate::value::{reference_entity_id, reference_is_local};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapSet(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Cap {
    Read = 1,
    Append = 2,
    Update = 4,
    Delete = 8,
    Structure = 16,
    Comment = 32,
    Export = 64,
    Manage = 128,
    /// Trigger the predefined tasks of a wish without editing its data (内核架构改进 D16): `proc.start`.
    Run = 256,
}

pub const ALL_CAPS: &[(Cap, &str)] = &[
    (Cap::Read, "read"),
    (Cap::Append, "append"),
    (Cap::Update, "update"),
    (Cap::Delete, "delete"),
    (Cap::Structure, "structure"),
    (Cap::Comment, "comment"),
    (Cap::Export, "export"),
    (Cap::Manage, "manage"),
    (Cap::Run, "run"),
];

impl CapSet {
    pub const NONE: CapSet = CapSet(0);
    pub const ALL: CapSet = CapSet(511);
    pub fn of(c: Cap) -> CapSet {
        CapSet(c as u16)
    }
    pub fn has(&self, c: Cap) -> bool {
        self.0 & c as u16 != 0
    }
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
    pub fn union(self, o: CapSet) -> CapSet {
        CapSet(self.0 | o.0)
    }
    pub fn intersect(self, o: CapSet) -> CapSet {
        CapSet(self.0 & o.0)
    }
    pub fn minus(self, o: CapSet) -> CapSet {
        CapSet(self.0 & !o.0)
    }
    pub fn any_write(&self) -> bool {
        self.has(Cap::Update) || self.has(Cap::Append) || self.has(Cap::Delete) || self.has(Cap::Structure)
    }
    /// May start a run: `run`, or any editing right (an editor runs what it could write by hand).
    pub fn may_run(&self) -> bool {
        self.has(Cap::Run) || self.any_write()
    }
    pub fn parse(names: &[String]) -> WsResult<CapSet> {
        let mut out = 0;
        for n in names {
            let c = ALL_CAPS
                .iter()
                .find(|(_, s)| s == n)
                .ok_or_else(|| WsError::invalid_op(format!("unknown capability {n}")))?;
            out |= c.0 as u16;
        }
        Ok(CapSet(out))
    }
    /// A JSON array of capability names (absent or null: none).
    pub fn parse_json(v: Option<&Value>, key: &str) -> WsResult<CapSet> {
        match v {
            None | Some(Value::Null) => Ok(CapSet::NONE),
            Some(Value::Array(a)) => {
                let names: Vec<String> = a.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| WsError::invalid_op(format!("{key} must be capability names")))).collect::<WsResult<_>>()?;
                CapSet::parse(&names)
            }
            Some(_) => Err(WsError::invalid_op(format!("{key} must be an array of capability names"))),
        }
    }
    pub fn names(&self) -> Vec<&'static str> {
        ALL_CAPS.iter().filter(|(c, _)| self.has(*c)).map(|(_, s)| *s).collect()
    }
}

pub fn cap_name(c: Cap) -> &'static str {
    ALL_CAPS.iter().find(|(x, _)| *x == c).map(|(_, s)| *s).unwrap_or("?")
}

/// The tiers of one layer, most specific first (Q25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// A single caller (or an inner-path publication's own subject).
    Single = 0,
    /// A logical group, or `@creator`.
    Group = 1,
    /// `@guest` and `@all`.
    Everyone = 2,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Single => "single",
            Tier::Group => "group",
            Tier::Everyone => "everyone",
        }
    }
}

/// One entry that applies to the caller on one layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub subject: String,
    pub tier: Tier,
    /// A `@creator` entry: it applies to an entity only when the caller created it.
    pub creator: bool,
    pub allow: CapSet,
    pub deny: CapSet,
}

/// What decided one capability on one entity.
#[derive(Debug, Clone)]
pub struct Decision {
    pub cap: Cap,
    pub allowed: bool,
    /// The deciding entry and the layer it sits on (`""` = the Workspace); `None`: the owner, or nothing mentions it.
    pub by: Option<(String, Applied)>,
    pub owner: bool,
}

impl Decision {
    pub fn to_json(&self, path_of: &dyn Fn(&str) -> Value) -> Value {
        let by = match &self.by {
            Some((scope, a)) => json!({ "subject": a.subject, "tier": a.tier.as_str(), "kind": if self.allowed { "allow" } else { "deny" },
                                        "resource": if scope.is_empty() { json!({ "entity_id": Value::Null, "path": "/" }) } else { json!({ "entity_id": scope, "path": path_of(scope) }) } }),
            None => Value::Null,
        };
        json!({ "capability": cap_name(self.cap), "allowed": self.allowed, "owner": self.owner, "decided_by": by })
    }
}

/// The effective permissions of one judged caller on one Workspace.
#[derive(Debug, Clone)]
pub struct Access {
    /// Who the caller is judged as: its own principal, or — an Agent nobody configured here — its owner (R16).
    pub principal: String,
    /// The capabilities on the Workspace itself (`/`).
    pub ws_caps: CapSet,
    full: bool,
    member: bool,
    /// Entries that apply to the caller, by layer (`""` = the Workspace).
    layers: BTreeMap<String, Vec<Applied>>,
    has_creator: bool,
    /// A macro bound to a group with `intersection` (§4.5): never more than this.
    limit: Option<Box<Access>>,
}

impl Access {
    /// Everything: the owner, or the service's own reads.
    pub fn full(principal: &str) -> Access {
        Access { principal: principal.to_string(), ws_caps: CapSet::ALL, full: true, member: true, layers: BTreeMap::new(), has_creator: false, limit: None }
    }

    /// The same capabilities on everything (a session role's synthesized access).
    pub fn uniform(principal: &str, caps: CapSet) -> Access {
        let layers = BTreeMap::from([(String::new(), vec![Applied { subject: principal.to_string(), tier: Tier::Single, creator: false, allow: caps, deny: CapSet::NONE }])]);
        Access { principal: principal.to_string(), ws_caps: caps, full: false, member: true, layers, has_creator: false, limit: None }
    }

    /// Allowances only, on the Workspace and on subtrees (`scope → caps`): the caller's own entries.
    pub fn granted(principal: &str, ws_caps: CapSet, scoped: impl IntoIterator<Item = (String, CapSet)>) -> Access {
        let single = |caps: CapSet| vec![Applied { subject: principal.to_string(), tier: Tier::Single, creator: false, allow: caps, deny: CapSet::NONE }];
        let mut layers: BTreeMap<String, Vec<Applied>> = scoped.into_iter().map(|(k, c)| (k, single(c))).collect();
        layers.insert(String::new(), single(ws_caps));
        Access::from_layers(principal, true, layers)
    }

    /// From the entries that apply to the caller. `member`: the caller may enter the Workspace at all.
    pub fn from_layers(principal: &str, member: bool, layers: BTreeMap<String, Vec<Applied>>) -> Access {
        let has_creator = layers.values().flatten().any(|a| a.creator);
        let mut a = Access { principal: principal.to_string(), ws_caps: CapSet::NONE, full: false, member, layers, has_creator, limit: None };
        a.ws_caps = CapSet(a.walk_scopes(&[String::new()], false, None));
        a
    }

    /// Never more than `limit` allows (a macro bound with `intersection`).
    pub fn bounded_by(mut self, limit: Access) -> Access {
        self.ws_caps = self.ws_caps.intersect(limit.ws_caps);
        self.limit = Some(Box::new(limit));
        self
    }

    pub fn is_owner(&self) -> bool {
        self.full && self.limit.is_none()
    }

    /// True when the caller may not enter this Workspace at all.
    pub fn is_empty(&self) -> bool {
        !self.full && !self.member
    }

    /// Whether `cap` is allowed by some entry that applies to the caller, anywhere (an approximation used to decide
    /// whether a caller manages or reads something here at all).
    pub fn anywhere(&self, cap: Cap) -> bool {
        if self.full {
            return self.limit.as_ref().is_none_or(|l| l.anywhere(cap));
        }
        let mine = self.ws_caps.has(cap) || self.layers.values().flatten().any(|a| a.allow.has(cap));
        mine && self.limit.as_ref().is_none_or(|l| l.anywhere(cap))
    }

    /// The layers from `entity_id` up to the Workspace: the entity, each ancestor, `""`. The root entity is the
    /// Workspace itself.
    fn chain(&self, ctx: &dyn ReadCtx, entity_id: &str) -> WsResult<Vec<String>> {
        let mut out = Vec::new();
        let scoped = self.layers.keys().any(|k| !k.is_empty());
        if scoped {
            let mut cur = entity_id.to_string();
            for _ in 0..4096 {
                if cur != ROOT_ID {
                    out.push(cur.clone());
                }
                match ctx.edge(&cur)? {
                    Some(e) => cur = e.parent_id,
                    None => break,
                }
            }
        }
        out.push(String::new());
        Ok(out)
    }

    /// Decide every capability along `scopes` (nearest first). `explain` collects the deciding entries.
    fn walk_scopes(&self, scopes: &[String], creator: bool, mut explain: Option<&mut [Option<(String, Applied, bool)>; 9]>) -> u16 {
        let (mut decided, mut allowed) = (0u16, 0u16);
        for scope in scopes {
            let Some(entries) = self.layers.get(scope) else { continue };
            for tier in [Tier::Single, Tier::Group, Tier::Everyone] {
                let applying = || entries.iter().filter(|a| a.tier == tier && (!a.creator || creator));
                let (mut a_bits, mut d_bits) = (0u16, 0u16);
                for a in applying() {
                    a_bits |= a.allow.0;
                    d_bits |= a.deny.0;
                }
                let mentioned = (a_bits | d_bits) & !decided;
                if mentioned == 0 {
                    continue;
                }
                allowed |= a_bits & mentioned & !d_bits;
                decided |= mentioned;
                if let Some(ex) = explain.as_deref_mut() {
                    for (i, (c, _)) in ALL_CAPS.iter().enumerate() {
                        let bit = *c as u16;
                        if mentioned & bit == 0 {
                            continue;
                        }
                        let denied = d_bits & bit != 0;
                        let who = applying().find(|a| if denied { a.deny.0 & bit != 0 } else { a.allow.0 & bit != 0 });
                        ex[i] = who.map(|w| (scope.clone(), w.clone(), !denied));
                    }
                }
            }
            if decided == CapSet::ALL.0 {
                break;
            }
        }
        allowed
    }

    fn own_caps(&self, ctx: &dyn ReadCtx, entity_id: &str, creator: Option<&Option<String>>) -> WsResult<CapSet> {
        if self.full {
            return Ok(CapSet::ALL);
        }
        let scoped = self.layers.keys().any(|k| !k.is_empty());
        if !scoped && !self.has_creator {
            return Ok(self.ws_caps);
        }
        let creator_match = if self.has_creator {
            match creator {
                Some(c) => c.as_deref() == Some(self.principal.as_str()),
                None => ctx.entity(entity_id)?.is_some_and(|e| e.created_by.as_deref() == Some(self.principal.as_str())),
            }
        } else {
            false
        };
        let chain = self.chain(ctx, entity_id)?;
        Ok(CapSet(self.walk_scopes(&chain, creator_match, None)))
    }

    fn caps_with(&self, ctx: &dyn ReadCtx, entity_id: &str, creator: Option<&Option<String>>) -> WsResult<CapSet> {
        let caps = self.own_caps(ctx, entity_id, creator)?;
        match &self.limit {
            Some(l) => Ok(caps.intersect(l.caps_with(ctx, entity_id, creator)?)),
            None => Ok(caps),
        }
    }

    /// What these entries decide along an explicit chain of layers (nearest first, ending with `""`): a move compares
    /// the chain before and after (权限需求 R23).
    pub fn caps_along(&self, chain: &[String]) -> CapSet {
        if self.full && self.limit.is_none() {
            return CapSet::ALL;
        }
        CapSet(self.walk_scopes(chain, false, None))
    }

    /// The caller's capabilities on an entity (or on a would-be entity: its ancestors decide).
    pub fn caps(&self, ctx: &dyn ReadCtx, entity_id: &str) -> WsResult<CapSet> {
        self.caps_with(ctx, entity_id, None)
    }

    /// [`Self::caps`] for a row in hand (its creator need not be read again).
    pub fn caps_of(&self, ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<CapSet> {
        self.caps_with(ctx, &e.entity_id, Some(&e.created_by))
    }

    pub fn can(&self, ctx: &dyn ReadCtx, entity_id: &str, cap: Cap) -> WsResult<bool> {
        Ok(self.caps(ctx, entity_id)?.has(cap))
    }

    pub fn require(&self, ctx: &dyn ReadCtx, entity_id: &str, cap: Cap) -> WsResult<()> {
        if self.can(ctx, entity_id, cap)? {
            Ok(())
        } else {
            Err(WsError::denied(format!("{} capability required", cap_name(cap))))
        }
    }

    /// Why each capability is allowed or refused on `entity_id` (`None`: the Workspace itself).
    pub fn explain(&self, ctx: &dyn ReadCtx, entity_id: Option<&str>) -> WsResult<Vec<Decision>> {
        if self.full && self.limit.is_none() {
            return Ok(ALL_CAPS.iter().map(|(c, _)| Decision { cap: *c, allowed: true, by: None, owner: true }).collect());
        }
        let (chain, creator_match) = match entity_id {
            Some(id) => {
                let creator = self.has_creator && ctx.entity(id)?.is_some_and(|e| e.created_by.as_deref() == Some(self.principal.as_str()));
                let mut chain = Vec::new();
                let mut cur = id.to_string();
                for _ in 0..4096 {
                    if cur != ROOT_ID {
                        chain.push(cur.clone());
                    }
                    match ctx.edge(&cur)? {
                        Some(e) => cur = e.parent_id,
                        None => break,
                    }
                }
                chain.push(String::new());
                (chain, creator)
            }
            None => (vec![String::new()], false),
        };
        let mut ex: [Option<(String, Applied, bool)>; 9] = Default::default();
        let allowed = self.walk_scopes(&chain, creator_match, Some(&mut ex));
        let limited = match (&self.limit, entity_id) {
            (Some(l), Some(id)) => l.caps(ctx, id)?.0,
            (Some(l), None) => l.ws_caps.0,
            (None, _) => CapSet::ALL.0,
        };
        Ok(ALL_CAPS
            .iter()
            .enumerate()
            .map(|(i, (c, _))| {
                let bit = *c as u16;
                Decision { cap: *c, allowed: allowed & bit != 0 && limited & bit != 0, by: ex[i].take().map(|(s, a, _)| (s, a)), owner: false }
            })
            .collect())
    }

    /// Presenters and managers of a session Workspace (第四期规划 §4, Q07): any Workspace-level write right. They read
    /// the `presenter` scope and every participant's data.
    pub fn reviewer(&self) -> bool {
        self.ws_caps.any_write() || self.ws_caps.has(Cap::Manage)
    }

    /// Personal-scope entities exist only for their owner; `presenter` ones only for reviewers; a participant's
    /// (`participant:<p>`) for that participant, reviewers and the principals its `readers` list names.
    pub fn sees_scope(&self, e: &EntityRow) -> bool {
        if let Some(owner) = e.owner() {
            return owner == self.principal;
        }
        if e.scope == SCOPE_PRESENTER {
            return self.reviewer();
        }
        if let Some(p) = e.participant() {
            return p == self.principal
                || self.reviewer()
                || e.payload.get("readers").and_then(Value::as_array).is_some_and(|r| r.iter().any(|x| x.as_str() == Some(self.principal.as_str())));
        }
        true
    }

    pub fn can_read(&self, ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<bool> {
        Ok(self.sees_scope(e) && self.caps_of(ctx, e)?.has(Cap::Read))
    }

    /// What the caller may do to `e`, as envelopes report it: the grants, and — with `comment` — what the
    /// caller's own writing needs: editing and deleting its annotations, and also moving its note Blocks
    /// (see [`own_note_block`]). The planner applies the same rules where those writes are checked.
    pub fn entity_caps(&self, ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<CapSet> {
        let caps = self.caps_of(ctx, e)?;
        if !caps.has(Cap::Comment) {
            return Ok(caps);
        }
        let mine = if e.type_id == TYPE_ANNOTATION {
            annotation_author(e) == Some(self.principal.as_str())
        } else {
            own_note_block(ctx, &self.principal, e)?
        };
        if !mine {
            return Ok(caps);
        }
        let mut extra = Cap::Update as u16 | Cap::Delete as u16;
        if e.type_id == TYPE_CELL {
            extra |= Cap::Structure as u16;
        }
        Ok(caps.union(CapSet(extra)))
    }
}

/// A note Block `principal` wrote: a Cell showing (`view.type` note) a shared annotation by `principal`.
/// Where it sits on a canvas belongs to the note as much as its text does, so `comment` is enough to put
/// it there, move, resize or remove it (a public link's guest adds notes this way); it takes no lock, like
/// the annotation itself. Other people's note Blocks need the usual capabilities.
pub fn own_note_block(ctx: &dyn ReadCtx, principal: &str, e: &EntityRow) -> WsResult<bool> {
    if e.type_id != TYPE_CELL {
        return Ok(false);
    }
    own_note_payload(ctx, principal, &e.payload)
}

/// [`own_note_block`] for a Cell payload about to be created or written.
pub fn own_note_payload(ctx: &dyn ReadCtx, principal: &str, payload: &JsonMap) -> WsResult<bool> {
    if payload.get("view").and_then(|v| v.get("type")).and_then(Value::as_str) != Some("note") {
        return Ok(false);
    }
    let Some(id) = payload.get("source_ref").filter(|r| reference_is_local(r)).and_then(reference_entity_id) else {
        return Ok(false);
    };
    Ok(ctx.entity(id)?.is_some_and(|a| a.type_id == TYPE_ANNOTATION && a.owner().is_none() && annotation_author(&a) == Some(principal)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MemStore;

    fn rule(subject: &str, tier: Tier, allow: &[Cap], deny: &[Cap]) -> Applied {
        let bits = |cs: &[Cap]| CapSet(cs.iter().fold(0, |b, c| b | *c as u16));
        Applied { subject: subject.into(), tier, creator: false, allow: bits(allow), deny: bits(deny) }
    }

    #[test]
    fn nearest_layer_then_tier_then_deny() {
        let ctx = MemStore::default();
        // two groups on the same layer: the refusal wins
        let a = Access::from_layers("bob", true, BTreeMap::from([(String::new(), vec![rule("@teacher", Tier::Group, &[Cap::Read, Cap::Update], &[]), rule("@sub", Tier::Group, &[], &[Cap::Update])])]));
        assert_eq!(a.ws_caps, CapSet(Cap::Read as u16));
        // a single-caller entry beats a group refusal on the same layer
        let b = Access::from_layers("bob", true, BTreeMap::from([(String::new(), vec![rule("@all", Tier::Everyone, &[], &[Cap::Read]), rule("bob", Tier::Single, &[Cap::Read], &[])])]));
        assert!(b.ws_caps.has(Cap::Read));
        assert!(b.caps(&ctx, "x").unwrap().has(Cap::Read));
        // nothing mentions it: refused
        assert!(!b.ws_caps.has(Cap::Update));
    }
}
