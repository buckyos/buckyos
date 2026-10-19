//! Permission checks at the protocol boundary and the ACL methods
//! (`get_acl` / `set_acl` / `list_acls`, implementation extensions).
//!
//! Path-form locators are checked before any filesystem access so an actor
//! cannot probe for names it is not allowed to see. Ref-form locators are
//! checked right after resolution.

use crate::acl::*;
use crate::error::*;
use crate::filedb::CollectionRow;
use crate::fsutil::{join_root, lstat, normalize_rel, unix_now};
use crate::namespace::{canonical_path, current_zone_cyfs_path, Node};
use crate::state::AppState;
use crate::types::{Locator, WireRef};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn denied(op: &str, actor: &Actor) -> NfsError {
    let e = NfsError::new(ErrorCode::PermissionDenied, format!("{} permission required", op))
        .with("required_op", json!(op));
    if matches!(actor, Actor::Anonymous) {
        e.with("reason", json!("unauthenticated"))
    } else {
        e
    }
}

fn collection_level(actor: &Actor, c: &CollectionRow) -> Level {
    match (actor, &c.owner) {
        (Actor::Admin(_), _) => Level::Manage,
        (Actor::User(u), Some(owner)) if u == owner => Level::Manage,
        _ => Level::None,
    }
}

fn record_revision(r: Option<&AclRecord>) -> String {
    match r {
        None => "acl-0".to_string(),
        Some(r) => {
            let body = json!([r.inherit, r.entries, r.updated_by, r.updated_at]).to_string();
            format!("acl-{}", &hex::encode(Sha256::digest(body.as_bytes()))[..16])
        }
    }
}

fn entry_json(e: &AclEntry) -> Value {
    json!({"principal": e.principal, "level": e.level})
}

#[derive(Deserialize)]
struct SetAclArgs {
    #[serde(default)]
    r#ref: Option<WireRef>,
    #[serde(default = "default_true")]
    inherit: bool,
    #[serde(default)]
    entries: Vec<AclEntry>,
    #[serde(default)]
    expected_revision: Option<String>,
}

fn default_true() -> bool {
    true
}

impl AppState {
    pub fn node_level(&self, actor: &Actor, node: &Node) -> Level {
        match node {
            Node::Root => self.acls.root_level(actor),
            Node::Native { root, rel, .. } => self.acls.level(actor, root, rel),
            Node::Collection(c) => collection_level(actor, c),
            Node::CollectionGroup { collection, .. } => collection_level(actor, collection),
            Node::View(_) | Node::ViewGroup { .. } => match actor {
                Actor::Anonymous | Actor::Delegated { .. } => Level::None,
                _ => Level::Read,
            },
        }
    }

    /// Containers are visible from `traverse`, content needs `read`.
    pub fn is_visible(node: &Node, level: Level) -> bool {
        if node.is_container() {
            level >= Level::Traverse
        } else {
            level >= Level::Read
        }
    }

    pub fn require(&self, actor: &Actor, node: &Node, min: Level, op: &str) -> NfsResult<Level> {
        let level = self.node_level(actor, node);
        let ok = if min == Level::Traverse { Self::is_visible(node, level) } else { level >= min };
        if !ok {
            return Err(denied(op, actor));
        }
        if let (false, Node::Native { root, rel, .. }) = (actor.is_admin(), node) {
            self.require_no_symlink_ancestor(actor, root, rel, op)?;
        }
        Ok(level)
    }

    /// ACLs are judged on the path as written, so a symlinked directory on the
    /// way would carry one path's grants onto another location: non-admin
    /// access never passes through one.
    fn require_no_symlink_ancestor(&self, actor: &Actor, root: &str, rel: &str, op: &str) -> NfsResult<()> {
        let cfg = self.config.root(root).ok_or_else(|| stale("root gone"))?;
        let mut cur = cfg.path.clone();
        let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        for seg in parts.iter().take(parts.len().saturating_sub(1)) {
            cur.push(seg);
            if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(denied(op, actor).with("reason", json!("symlink")));
            }
        }
        Ok(())
    }

    /// Data-plane reads follow a final symlink: authorize the real target,
    /// which must lie inside an export root.
    pub fn require_symlink_target(&self, actor: &Actor, full: &std::path::Path) -> NfsResult<()> {
        if actor.is_admin() {
            return Ok(());
        }
        let real = std::fs::canonicalize(full).map_err(NfsError::from)?;
        for export in &self.config.exports {
            let Ok(base) = std::fs::canonicalize(&export.path) else { continue };
            if let Ok(rest) = real.strip_prefix(&base) {
                let rel: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                if self.acls.level(actor, &export.id, &rel.join("/")) >= Level::Read {
                    return Ok(());
                }
            }
        }
        Err(denied("read", actor).with("reason", json!("symlink")))
    }

    /// Level of a `cyfs:///` path string, without touching the filesystem.
    pub fn path_level(&self, actor: &Actor, path: &str) -> NfsResult<Level> {
        let rel_all = normalize_rel(path)?;
        if rel_all.is_empty() {
            return Ok(self.acls.root_level(actor));
        }
        let (root, rel) = rel_all.split_once('/').unwrap_or((&rel_all, ""));
        Ok(self.acls.level(actor, root, rel))
    }

    fn precheck_path(&self, actor: &Actor, path: &str, op: &str) -> NfsResult<()> {
        if self.path_level(actor, path)? < Level::Traverse {
            return Err(denied(op, actor));
        }
        Ok(())
    }

    /// The DFS path a path-form locator names (None for refs and virtual URIs).
    fn locator_dfs_path(at: &Locator) -> NfsResult<Option<String>> {
        if at.r#ref.is_some() {
            return Ok(None);
        }
        if let Some(uri) = &at.uri {
            return current_zone_cyfs_path(uri);
        }
        match (&at.path, at.realm.as_deref().unwrap_or("dfs")) {
            (Some(p), "dfs") => Ok(Some(p.clone())),
            _ => Ok(None),
        }
    }

    pub fn resolve_for(&self, actor: &Actor, at: &Locator, min: Level, op: &str) -> NfsResult<Node> {
        if let Some(path) = Self::locator_dfs_path(at)? {
            self.precheck_path(actor, &path, op)?;
        }
        let node = self.resolve_locator(at)?;
        self.require(actor, &node, min, op)?;
        Ok(node)
    }

    pub fn resolve_ref_for(&self, actor: &Actor, r: &WireRef, min: Level, op: &str) -> NfsResult<Node> {
        let node = self.resolve_ref(r)?;
        self.require(actor, &node, min, op)?;
        Ok(node)
    }

    pub fn walk_child_for(&self, actor: &Actor, node: &Node, name: &str, op: &str) -> NfsResult<Node> {
        let parent_level = self.node_level(actor, node);
        let child_path = match node {
            Node::Root => Some(name.to_string()),
            Node::Native { root, rel, .. } => {
                Some(if rel.is_empty() { format!("{}/{}", root, name) } else { format!("{}/{}/{}", root, rel, name) })
            }
            _ => None,
        };
        if let Some(p) = &child_path {
            self.precheck_path(actor, p, op)?;
        }
        let child = self.walk_child(node, name)?;
        // A reference binding is only visible inside a readable directory.
        if let (Node::Native { .. }, Some(p)) = (node, &child_path) {
            let native = matches!(&child, Node::Native { root, rel, .. } if canonical_path(root, rel) == format!("cyfs:///{}", p));
            if !native && parent_level < Level::Read {
                return Err(denied(op, actor));
            }
        }
        if !matches!(node, Node::Root | Node::Native { .. }) && parent_level < Level::Read {
            return Err(denied(op, actor));
        }
        self.require(actor, &child, Level::Traverse, op)?;
        Ok(child)
    }

    /// Accounts get no home directory when they are created; a signed-in user's
    /// `home/<user_id>` is created on first use (as smb_service does), so
    /// the home they fully control is always there to see.
    pub fn ensure_home(&self, actor: &Actor) -> NfsResult<()> {
        let (Some(user), Some(home)) = (actor.user_id(), self.config.root(HOME_ROOT)) else {
            return Ok(());
        };
        if user.starts_with('.') || crate::fsutil::validate_name(user).is_err() {
            return Ok(());
        }
        let dir = home.path.join(user);
        if std::fs::symlink_metadata(&dir).is_ok() {
            return Ok(());
        }
        std::fs::create_dir(&dir)?;
        self.acls.delete_subtree(&self.db, HOME_ROOT, user)?;
        if let Ok(node) = self.resolve_dfs_path(HOME_ROOT) {
            self.bump_dir(&node, "entries_changed", Some(json!({"added": [user]})));
        }
        Ok(())
    }

    /// Recursive operation guard: `min` on the target and on every protected
    /// (non-inheriting) record below it.
    pub fn require_subtree(&self, actor: &Actor, root: &str, rel: &str, min: Level, op: &str) -> NfsResult<()> {
        if self.acls.level(actor, root, rel) < min || !self.acls.subtree_allows(actor, root, rel, min) {
            return Err(denied(op, actor));
        }
        Ok(())
    }

    /// Whether an event about a container may reach this actor.
    pub fn event_visible(&self, actor: &Actor, container: Option<&crate::watch::ContainerKey>, data: &Value) -> bool {
        use crate::watch::ContainerKey;
        if actor.is_admin() {
            return true;
        }
        match container {
            Some(ContainerKey::Root) => self.acls.root_level(actor) >= Level::Traverse,
            // Pass-through watchers only get hint-less refreshes (e.g. a new share
            // below); entry names in hints stay with readers.
            Some(ContainerKey::Dir { root, rel }) => {
                let level = self.acls.level(actor, root, rel);
                level >= Level::Read || level >= Level::Traverse && data.get("hint").is_none()
            }
            Some(ContainerKey::Node(id)) => self
                .resolve_node_id(*id, None)
                .map(|n| self.node_level(actor, &n) >= Level::Read)
                .unwrap_or(false),
            None => match data["anchor"].as_str().and_then(|a| a.strip_prefix("live:n_")) {
                Some(id) => id
                    .parse::<i64>()
                    .ok()
                    .and_then(|id| self.resolve_node_id(id, None).ok())
                    .map(|n| self.node_level(actor, &n) >= Level::Read)
                    .unwrap_or(false),
                None => true,
            },
        }
    }

    // ---------- ACL methods ----------

    fn acl_target(&self, actor: &Actor, at: Option<&Locator>, r: Option<&WireRef>) -> NfsResult<(String, String)> {
        let node = match (r, at) {
            (Some(r), _) => self.resolve_ref_for(actor, r, Level::Traverse, "read")?,
            (None, Some(at)) => self.resolve_for(actor, at, Level::Traverse, "read")?,
            _ => return Err(invalid("missing target (at or ref)")),
        };
        match node {
            Node::Native { root, rel, .. } => Ok((root, rel)),
            Node::Root => Err(invalid("the namespace root has no ACL; set it on export roots or below")),
            _ => Err(invalid("ACLs apply to files and directories")),
        }
    }

    fn builtin_entries(&self, root: &str, rel: &str) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(owner) = self.zone_owner_id() {
            out.push(json!({"principal": format!("{}{}", USER_PREFIX, owner), "level": Level::Manage, "reason": "zone_owner"}));
        }
        if let Some(home) = home_owner(root, rel) {
            if self.zone_owner_id().as_deref() != Some(home) {
                out.push(json!({"principal": format!("{}{}", USER_PREFIX, home), "level": Level::Manage, "reason": "home_owner"}));
            }
        }
        out
    }

    pub fn zone_owner_id(&self) -> Option<String> {
        match &self.config.auth {
            crate::config::AuthMode::TestKey { zone_owner, .. } => Some(zone_owner.clone()),
            _ => self.zone_owner.read().unwrap().clone(),
        }
    }

    /// Grants a record inherits from its ancestors (stops at a non-inheriting one).
    fn inherited_entries(&self, root: &str, rel: &str) -> Vec<Value> {
        let mut out = Vec::new();
        if self.acls.get(root, rel).is_some_and(|r| !r.inherit) {
            return out;
        }
        let mut cur = rel.to_string();
        while !cur.is_empty() {
            cur = cur.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
            if let Some(r) = self.acls.get(root, &cur) {
                for e in &r.entries {
                    let mut v = entry_json(e);
                    v["from"] = json!(canonical_path(root, &cur));
                    out.push(v);
                }
                if !r.inherit {
                    break;
                }
            }
        }
        out
    }

    fn record_json(&self, r: &AclRecord) -> Value {
        let full = self.config.root(&r.root).map(|c| join_root(&c.path, &r.path));
        json!({
            "path": canonical_path(&r.root, &r.path),
            "inherit": r.inherit,
            "entries": r.entries.iter().map(entry_json).collect::<Vec<_>>(),
            "home_owner": home_owner(&r.root, &r.path),
            "updated_by": r.updated_by,
            "updated_at": r.updated_at,
            "revision": record_revision(Some(r)),
            "missing": full.map(|p| lstat(&p).is_err()).unwrap_or(true),
        })
    }

    pub fn op_get_acl(&self, actor: &Actor, at: Option<&Locator>, args: &Value) -> NfsResult<Value> {
        let r: Option<WireRef> = match args.get("ref").filter(|v| !v.is_null()) {
            Some(v) => Some(serde_json::from_value(v.clone()).map_err(|_| invalid("bad ref"))?),
            None => None,
        };
        let (root, rel) = self.acl_target(actor, at, r.as_ref())?;
        let level = self.acls.level(actor, &root, &rel);
        let record = self.acls.get(&root, &rel);
        let mut out = json!({
            "path": canonical_path(&root, &rel),
            "access_level": level,
            "can_manage": level >= Level::Manage,
            "revision": record_revision(record.as_ref()),
        });
        if level >= Level::Manage {
            out["inherit"] = json!(record.as_ref().map(|r| r.inherit).unwrap_or(true));
            out["entries"] = json!(record.as_ref().map(|r| r.entries.iter().map(entry_json).collect::<Vec<_>>()).unwrap_or_default());
            out["inherited"] = json!(self.inherited_entries(&root, &rel));
            out["builtin"] = json!(self.builtin_entries(&root, &rel));
            if let Some(r) = &record {
                out["updated_by"] = json!(r.updated_by);
                out["updated_at"] = json!(r.updated_at);
            }
        }
        Ok(out)
    }

    pub fn op_set_acl(&self, actor: &Actor, at: Option<&Locator>, args: &Value) -> NfsResult<Value> {
        let a: SetAclArgs = serde_json::from_value(args.clone()).map_err(|e| invalid(format!("bad set_acl args: {}", e)))?;
        let clearing = a.inherit && a.entries.is_empty();
        let (root, rel) = match self.acl_target(actor, at, a.r#ref.as_ref()) {
            Ok(t) => t,
            // Orphaned records (target removed outside the protocol) can still be cleared.
            Err(e) if e.code == ErrorCode::NotFound && clearing => {
                let path = at.and_then(|l| Self::locator_dfs_path(l).ok().flatten()).ok_or(e)?;
                let rel_all = normalize_rel(&path)?;
                let (root, rel) = rel_all.split_once('/').unwrap_or((&rel_all, ""));
                (root.to_string(), rel.to_string())
            }
            Err(e) => return Err(e),
        };
        if self.acls.level(actor, &root, &rel) < Level::Manage {
            return Err(denied("manage", actor));
        }
        let current = self.acls.get(&root, &rel);
        if let Some(expected) = &a.expected_revision {
            let cur = record_revision(current.as_ref());
            if expected != &cur {
                return Err(rev_mismatch(expected, &cur));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for e in &a.entries {
            validate_entry(e)?;
            if !seen.insert(e.principal.clone()) {
                return Err(invalid(format!("duplicate principal '{}'", e.principal)));
            }
        }
        let record = AclRecord {
            root: root.clone(),
            path: rel.clone(),
            inherit: a.inherit,
            entries: a.entries,
            updated_by: actor.label(),
            updated_at: unix_now(),
        };
        self.acls.set(&self.db, record)?;
        // Visibility of this node changed for others: let watchers re-list its parent.
        let parent = match crate::fsutil::rel_split(&rel) {
            Some((p, _)) => self.resolve_dfs_path(&format!("{}/{}", root, p)).ok(),
            None => Some(Node::Root),
        };
        if let Some(node) = parent {
            self.bump_dir(&node, "acl_changed", None);
        }
        let saved = self.acls.get(&root, &rel);
        Ok(json!({"path": canonical_path(&root, &rel), "revision": record_revision(saved.as_ref())}))
    }

    /// Every record the actor can manage — for the zone owner, all of them.
    pub fn op_list_acls(&self, actor: &Actor) -> NfsResult<Value> {
        if matches!(actor, Actor::Anonymous) {
            return Err(denied("manage", actor));
        }
        let records: Vec<Value> = self
            .acls
            .records()
            .iter()
            .filter(|r| self.acls.level(actor, &r.root, &r.path) >= Level::Manage)
            .map(|r| self.record_json(r))
            .collect();
        Ok(json!({"records": records, "zone_owner": self.zone_owner_id(), "is_admin": actor.is_admin()}))
    }
}
