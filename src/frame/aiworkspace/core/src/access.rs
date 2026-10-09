//! Minimal permission model (design doc §2.9): capabilities granted on the
//! whole Workspace or on an entity subtree; effective set is the union along
//! the structural path. No deny rules, no field/record-level grants.

use crate::error::{WsError, WsResult};
use crate::model::{EntityRow, ReadCtx};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
];

impl CapSet {
    pub const NONE: CapSet = CapSet(0);
    pub const ALL: CapSet = CapSet(255);
    pub fn has(&self, c: Cap) -> bool {
        self.0 & c as u16 != 0
    }
    pub fn union(self, o: CapSet) -> CapSet {
        CapSet(self.0 | o.0)
    }
    pub fn any_write(&self) -> bool {
        self.has(Cap::Update) || self.has(Cap::Append) || self.has(Cap::Delete) || self.has(Cap::Structure)
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
    pub fn names(&self) -> Vec<&'static str> {
        ALL_CAPS.iter().filter(|(c, _)| self.has(*c)).map(|(_, s)| *s).collect()
    }
}

pub fn cap_name(c: Cap) -> &'static str {
    ALL_CAPS.iter().find(|(x, _)| *x == c).map(|(_, s)| *s).unwrap_or("?")
}

/// The effective grants of one trusted principal on one Workspace.
#[derive(Debug, Clone)]
pub struct Access {
    pub principal: String,
    pub ws_caps: CapSet,
    /// `scope_entity_id → caps` for subtree grants.
    pub scoped: BTreeMap<String, CapSet>,
}

impl Access {
    pub fn full(principal: &str) -> Access {
        Access { principal: principal.to_string(), ws_caps: CapSet::ALL, scoped: BTreeMap::new() }
    }

    /// True when the principal holds no grant at all on this Workspace.
    pub fn is_empty(&self) -> bool {
        self.ws_caps == CapSet::NONE && self.scoped.values().all(|c| *c == CapSet::NONE)
    }

    /// Workspace-level grant ∪ grants on the entity and each structural ancestor.
    pub fn caps(&self, ctx: &dyn ReadCtx, entity_id: &str) -> WsResult<CapSet> {
        let mut caps = self.ws_caps;
        if self.scoped.is_empty() {
            return Ok(caps);
        }
        let mut cur = entity_id.to_string();
        for _ in 0..4096 {
            if let Some(c) = self.scoped.get(&cur) {
                caps = caps.union(*c);
            }
            match ctx.edge(&cur)? {
                Some(e) => cur = e.parent_id,
                None => break,
            }
        }
        Ok(caps)
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

    /// Personal-scope entities exist only for their owner.
    pub fn sees_scope(&self, e: &EntityRow) -> bool {
        match e.owner() {
            Some(owner) => owner == self.principal,
            None => true,
        }
    }

    pub fn can_read(&self, ctx: &dyn ReadCtx, e: &EntityRow) -> WsResult<bool> {
        Ok(self.sees_scope(e) && self.can(ctx, &e.entity_id, Cap::Read)?)
    }
}
