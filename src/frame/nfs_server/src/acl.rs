//! Path ACLs: who may traverse / read / write / manage which part of `cyfs:///`.
//!
//! Policy protects paths (NFSP §7.3), so every decision is a pure function of
//! the (export root, relative path) string and the actor — it never touches the
//! filesystem, which keeps "exists but forbidden" indistinguishable from
//! "forbidden" for paths the actor cannot see.
//!
//! - Admin actors (zone owner, trusted services, auth-disabled standalone) have
//!   `manage` everywhere.
//! - A user always has `manage` on `home/<user_id>/**`.
//! - Explicit records grant `read` / `write` / `manage` to `user:<id>`,
//!   `zone_users` (any signed-in user) or `public` (anyone, read only). They
//!   inherit down the tree; a record with `inherit=false` stops collecting
//!   grants from its ancestors (built-in rules above still apply).
//! - Ancestors of anything an actor can read are `traverse`: listable, but only
//!   the children leading to readable items are shown.

use crate::error::*;
use crate::filedb::FileDb;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

pub const PUBLIC: &str = "public";
pub const ZONE_USERS: &str = "zone_users";
pub const USER_PREFIX: &str = "user:";
pub const HOME_ROOT: &str = "home";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    None,
    Traverse,
    Read,
    Write,
    Manage,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::None => "none",
            Level::Traverse => "traverse",
            Level::Read => "read",
            Level::Write => "write",
            Level::Manage => "manage",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// Zone owner, a trusted system service, or standalone mode without auth.
    Admin(Option<String>),
    User(String),
    /// An App or Agent session acting for `user` (D13): only `share.*`
    /// accepts it, as that user's file actor; path ACLs never grant it.
    Delegated { user: String, app: String, admin: bool },
    Anonymous,
}

impl Actor {
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Actor::Admin(u) => u.as_deref(),
            Actor::User(u) | Actor::Delegated { user: u, .. } => Some(u),
            Actor::Anonymous => None,
        }
    }

    /// The app behind a delegated session.
    pub fn app_id(&self) -> Option<&str> {
        match self {
            Actor::Delegated { app, .. } => Some(app),
            _ => None,
        }
    }

    /// The actor whose file access a delegated session borrows.
    pub fn file_actor(&self) -> Actor {
        match self {
            Actor::Delegated { user, admin: true, .. } => Actor::Admin(Some(user.clone())),
            Actor::Delegated { user, .. } => Actor::User(user.clone()),
            other => other.clone(),
        }
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, Actor::Admin(_))
    }

    pub fn label(&self) -> String {
        match self {
            Actor::Admin(Some(u)) | Actor::User(u) => format!("{}{}", USER_PREFIX, u),
            Actor::Delegated { user, app, .. } => format!("app:{}@{}", app, user),
            Actor::Admin(None) => "system".to_string(),
            Actor::Anonymous => PUBLIC.to_string(),
        }
    }

    fn matches(&self, principal: &str) -> bool {
        if matches!(self, Actor::Delegated { .. }) {
            return false;
        }
        match principal {
            PUBLIC => true,
            ZONE_USERS => !matches!(self, Actor::Anonymous),
            p => match (p.strip_prefix(USER_PREFIX), self.user_id()) {
                (Some(want), Some(me)) => want == me,
                _ => false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclEntry {
    pub principal: String,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclRecord {
    pub root: String,
    pub path: String,
    pub inherit: bool,
    pub entries: Vec<AclEntry>,
    pub updated_by: String,
    pub updated_at: i64,
}

impl AclRecord {
    fn grant_for(&self, actor: &Actor) -> Level {
        self.entries
            .iter()
            .filter(|e| actor.matches(&e.principal))
            .map(|e| e.level)
            .max()
            .unwrap_or(Level::None)
    }
}

/// Validates a principal string and the level it may hold.
pub fn validate_entry(entry: &AclEntry) -> NfsResult<()> {
    match entry.principal.as_str() {
        PUBLIC if entry.level != Level::Read => {
            return Err(invalid("public access can only be read"));
        }
        PUBLIC | ZONE_USERS => {}
        p => match p.strip_prefix(USER_PREFIX) {
            Some(u) if !u.is_empty() && u.len() <= 128 && !u.contains('/') => {}
            _ => return Err(invalid(format!("unknown principal '{}'", p))),
        },
    }
    if !matches!(entry.level, Level::Read | Level::Write | Level::Manage) {
        return Err(invalid("level must be read, write or manage"));
    }
    Ok(())
}

/// The user owning `home/<user>/**`, if `(root, rel)` lies in a home dir.
pub fn home_owner<'a>(root: &str, rel: &'a str) -> Option<&'a str> {
    if root != HOME_ROOT || rel.is_empty() {
        return None;
    }
    Some(rel.split('/').next().unwrap_or(rel))
}

fn parent_rel(rel: &str) -> Option<&str> {
    if rel.is_empty() {
        None
    } else {
        Some(rel.rsplit_once('/').map(|(p, _)| p).unwrap_or(""))
    }
}

fn is_under(path: &str, base: &str) -> bool {
    base.is_empty() && !path.is_empty() || path.starts_with(base) && path[base.len()..].starts_with('/')
}

type Key = (String, String);

/// In-memory mirror of the filedb `acls` table (write-through).
pub struct AclTable {
    map: RwLock<BTreeMap<Key, AclRecord>>,
}

impl AclTable {
    pub fn load(db: &FileDb) -> NfsResult<Self> {
        let map = db
            .acl_all()?
            .into_iter()
            .map(|r| ((r.root.clone(), r.path.clone()), r))
            .collect();
        Ok(AclTable { map: RwLock::new(map) })
    }

    pub fn get(&self, root: &str, rel: &str) -> Option<AclRecord> {
        self.map.read().unwrap().get(&(root.to_string(), rel.to_string())).cloned()
    }

    pub fn records(&self) -> Vec<AclRecord> {
        self.map.read().unwrap().values().cloned().collect()
    }

    pub fn level(&self, actor: &Actor, root: &str, rel: &str) -> Level {
        if actor.is_admin() {
            return Level::Manage;
        }
        if matches!(actor, Actor::Delegated { .. }) {
            return Level::None;
        }
        if let (Some(owner), Some(me)) = (home_owner(root, rel), actor.user_id()) {
            if owner == me {
                return Level::Manage;
            }
        }
        let map = self.map.read().unwrap();
        let granted = Self::granted(&map, actor, root, rel);
        if granted >= Level::Read {
            return granted;
        }
        if Self::traversable(&map, actor, root, rel) {
            Level::Traverse
        } else {
            Level::None
        }
    }

    /// Level of the namespace root `cyfs:///` (read-only, listing filtered).
    pub fn root_level(&self, actor: &Actor) -> Level {
        match actor {
            Actor::Admin(_) => Level::Read,
            Actor::User(_) => Level::Traverse,
            Actor::Delegated { .. } => Level::None,
            Actor::Anonymous => {
                let map = self.map.read().unwrap();
                if map.values().any(|r| r.grant_for(actor) >= Level::Read) {
                    Level::Traverse
                } else {
                    Level::None
                }
            }
        }
    }

    /// Explicit + inherited grants, walking up until a non-inheriting record.
    fn granted(map: &BTreeMap<Key, AclRecord>, actor: &Actor, root: &str, rel: &str) -> Level {
        let mut level = Level::None;
        let mut cur = Some(rel);
        while let Some(p) = cur {
            if let Some(r) = map.get(&(root.to_string(), p.to_string())) {
                level = level.max(r.grant_for(actor));
                if !r.inherit {
                    break;
                }
            }
            cur = parent_rel(p);
        }
        level
    }

    fn traversable(map: &BTreeMap<Key, AclRecord>, actor: &Actor, root: &str, rel: &str) -> bool {
        if root == HOME_ROOT && rel.is_empty() && matches!(actor, Actor::User(_)) {
            return true;
        }
        Self::under(map, root, rel).any(|r| r.grant_for(actor) >= Level::Read)
    }

    fn under<'a>(
        map: &'a BTreeMap<Key, AclRecord>,
        root: &'a str,
        rel: &'a str,
    ) -> impl Iterator<Item = &'a AclRecord> + 'a {
        map.range((root.to_string(), rel.to_string())..)
            .take_while(move |((r, _), _)| r == root)
            .map(|(_, rec)| rec)
            .filter(move |rec| is_under(&rec.path, rel))
    }

    /// True when every non-inheriting record strictly below `(root, rel)` still
    /// grants the actor `min` — recursive delete/move/copy must not reach into
    /// subtrees that were deliberately protected.
    pub fn subtree_allows(&self, actor: &Actor, root: &str, rel: &str, min: Level) -> bool {
        if actor.is_admin() {
            return true;
        }
        let protected: Vec<String> = {
            let map = self.map.read().unwrap();
            Self::under(&map, root, rel).filter(|r| !r.inherit).map(|r| r.path.clone()).collect()
        };
        protected.iter().all(|p| self.level(actor, root, p) >= min)
    }

    /// Records below or at `(root, rel)` (for listings scoped to a subtree).
    pub fn records_at_or_under(&self, root: &str, rel: &str) -> Vec<AclRecord> {
        let map = self.map.read().unwrap();
        let mut out: Vec<AclRecord> = map.get(&(root.to_string(), rel.to_string())).cloned().into_iter().collect();
        out.extend(Self::under(&map, root, rel).cloned());
        out
    }

    pub fn set(&self, db: &FileDb, record: AclRecord) -> NfsResult<()> {
        let key = (record.root.clone(), record.path.clone());
        let mut map = self.map.write().unwrap();
        if record.entries.is_empty() && record.inherit {
            db.acl_delete(&record.root, &record.path)?;
            map.remove(&key);
        } else {
            db.acl_put(&record)?;
            map.insert(key, record);
        }
        Ok(())
    }

    /// Records follow protocol moves (bypass renames lose them, like anchors
    /// that the reconciler cannot follow).
    pub fn move_prefix(&self, db: &FileDb, root: &str, rel: &str, new_root: &str, new_rel: &str) -> NfsResult<()> {
        let mut map = self.map.write().unwrap();
        let moved: Vec<Key> = map
            .keys()
            .filter(|(r, p)| r == root && (p == rel || is_under(p, rel)))
            .cloned()
            .collect();
        if moved.is_empty() {
            return Ok(());
        }
        db.acl_move_prefix(root, rel, new_root, new_rel)?;
        for key in moved {
            if let Some(mut rec) = map.remove(&key) {
                rec.root = new_root.to_string();
                rec.path = format!("{}{}", new_rel, &rec.path[rel.len()..]);
                map.insert((rec.root.clone(), rec.path.clone()), rec);
            }
        }
        Ok(())
    }

    pub fn delete_subtree(&self, db: &FileDb, root: &str, rel: &str) -> NfsResult<()> {
        let mut map = self.map.write().unwrap();
        let gone: Vec<Key> = map
            .keys()
            .filter(|(r, p)| r == root && (p == rel || is_under(p, rel)))
            .cloned()
            .collect();
        if gone.is_empty() {
            return Ok(());
        }
        db.acl_delete_subtree(root, rel)?;
        for key in gone {
            map.remove(&key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(path: &str, inherit: bool, entries: &[(&str, Level)]) -> AclRecord {
        AclRecord {
            root: "home".into(),
            path: path.into(),
            inherit,
            entries: entries
                .iter()
                .map(|(p, l)| AclEntry { principal: p.to_string(), level: *l })
                .collect(),
            updated_by: "user:alice".into(),
            updated_at: 0,
        }
    }

    fn table(records: Vec<AclRecord>) -> (FileDb, AclTable) {
        let db = FileDb::open_memory().unwrap();
        let t = AclTable::load(&db).unwrap();
        for r in records {
            t.set(&db, r).unwrap();
        }
        (db, t)
    }

    #[test]
    fn home_and_admin_rules() {
        let (_db, t) = table(vec![]);
        let alice = Actor::User("alice".into());
        assert_eq!(t.level(&alice, "home", "alice"), Level::Manage);
        assert_eq!(t.level(&alice, "home", "alice/docs/a.txt"), Level::Manage);
        assert_eq!(t.level(&alice, "home", "bob"), Level::None);
        assert_eq!(t.level(&alice, "home", ""), Level::Traverse);
        assert_eq!(t.level(&alice, "srv", ""), Level::None);
        assert_eq!(t.level(&Actor::Admin(None), "srv", "x"), Level::Manage);
        assert_eq!(t.level(&Actor::Anonymous, "home", ""), Level::None);
        assert_eq!(t.root_level(&Actor::Anonymous), Level::None);
        assert_eq!(t.root_level(&alice), Level::Traverse);
    }

    #[test]
    fn inheritance_and_breaking() {
        let (_db, t) = table(vec![
            rec("bob/share", true, &[("user:alice", Level::Write)]),
            rec("bob/share/private", false, &[]),
            rec("bob/share/private/open", true, &[("zone_users", Level::Read)]),
        ]);
        let alice = Actor::User("alice".into());
        assert_eq!(t.level(&alice, "home", "bob/share"), Level::Write);
        assert_eq!(t.level(&alice, "home", "bob/share/x/y.txt"), Level::Write);
        assert_eq!(t.level(&alice, "home", "bob/share/private"), Level::Traverse);
        assert_eq!(t.level(&alice, "home", "bob/share/private/z"), Level::None);
        assert_eq!(t.level(&alice, "home", "bob/share/private/open/f"), Level::Read);
        assert_eq!(t.level(&alice, "home", "bob"), Level::Traverse);
        assert_eq!(t.level(&alice, "home", "bob/other"), Level::None);
        // The home owner is never locked out by inheritance breaking.
        assert_eq!(t.level(&Actor::User("bob".into()), "home", "bob/share/private/z"), Level::Manage);
        assert!(!t.subtree_allows(&alice, "home", "bob/share", Level::Write));
        assert!(t.subtree_allows(&alice, "home", "bob/share/x", Level::Write));
        assert!(!t.subtree_allows(&alice, "home", "bob/share", Level::Read));
    }

    #[test]
    fn public_and_prefix_boundaries() {
        let (_db, t) = table(vec![rec("bob/pub", true, &[("public", Level::Read)])]);
        let anon = Actor::Anonymous;
        assert_eq!(t.level(&anon, "home", "bob/pub/a.jpg"), Level::Read);
        assert_eq!(t.level(&anon, "home", "bob/public"), Level::None);
        assert_eq!(t.level(&anon, "home", "bob"), Level::Traverse);
        assert_eq!(t.level(&anon, "home", ""), Level::Traverse);
        assert_eq!(t.root_level(&anon), Level::Traverse);
        assert!(validate_entry(&AclEntry { principal: "public".into(), level: Level::Write }).is_err());
        assert!(validate_entry(&AclEntry { principal: "user:".into(), level: Level::Read }).is_err());
        assert!(validate_entry(&AclEntry { principal: "user:bob".into(), level: Level::Traverse }).is_err());
    }

    #[test]
    fn move_and_delete_follow_paths() {
        let (db, t) = table(vec![
            rec("bob/a", true, &[("user:alice", Level::Read)]),
            rec("bob/a/b", false, &[]),
            rec("bob/ab", true, &[("user:carol", Level::Read)]),
        ]);
        t.move_prefix(&db, "home", "bob/a", "home", "bob/z").unwrap();
        assert!(t.get("home", "bob/a").is_none());
        assert!(t.get("home", "bob/z").is_some());
        assert!(t.get("home", "bob/z/b").is_some());
        assert!(t.get("home", "bob/ab").is_some());
        let reloaded = AclTable::load(&db).unwrap();
        assert_eq!(reloaded.records(), t.records());
        t.delete_subtree(&db, "home", "bob/z").unwrap();
        assert_eq!(t.records().len(), 1);
        assert_eq!(AclTable::load(&db).unwrap().records().len(), 1);
    }
}
