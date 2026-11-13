//! Every kRPC method, its level and how it is authorized (权限需求 R21). Dispatch enforces the generic checks of this
//! table before a handler runs (the handler keeps its own, finer ones); the README's authorization table is generated
//! from it (`server/tests/contract.rs`). Levels (内核架构改进 D11): `stable` methods change only with the protocol version
//! and each has a contract test; `internal` serve the Desktop and diagnostics; `experimental` are not frozen yet.

use aiworkspace_core::access::{cap_name, Cap};

/// Where a method is judged, and by what.
#[derive(Debug, Clone, Copy)]
pub enum Check {
    /// No session: what anyone may know.
    Public,
    /// Any verified session; nothing of a Workspace.
    Session,
    /// Unrestricted accounts and services: those that may own Workspaces (PRE-03).
    MayOwn,
    /// The Workspace's owner (R17).
    Owner,
    /// May enter the Workspace (any member of it; otherwise it does not exist for the caller).
    Member,
    /// The capability on the Workspace itself (`/`).
    Ws(Cap),
    /// The capability on the entity the parameter names (an id or a path).
    Entity(Cap, &'static str),
    /// May enter; each entity the answer carries is filtered by `read` on it.
    Reads,
    /// Judged by the handler, for the reason given.
    Handler(&'static str),
}

impl Check {
    pub fn describe(&self) -> String {
        match self {
            Check::Public => "none (no session)".into(),
            Check::Session => "a session".into(),
            Check::MayOwn => "may own (unrestricted account or service)".into(),
            Check::Owner => "the owner".into(),
            Check::Member => "may enter".into(),
            Check::Ws(c) => format!("`{}` on `/`", cap_name(*c)),
            Check::Entity(c, p) => format!("`{}` on `{p}`", cap_name(*c)),
            Check::Reads => "may enter; `read` on each entity".into(),
            Check::Handler(why) => (*why).into(),
        }
    }
}

pub struct MethodSpec {
    pub method: &'static str,
    pub level: &'static str,
    /// All of these hold.
    pub checks: &'static [Check],
}

const fn m(method: &'static str, level: &'static str, checks: &'static [Check]) -> MethodSpec {
    MethodSpec { method, level, checks }
}

use Check::*;

const OPS: Check = Handler("each operation's capability on its target (`schemas/operations.json`)");
const RUN_OWN: Check = Handler("one's own run, or `update` / `manage` on its wish");
const RUN_TOKEN: Check = Handler("the run's token, while its attempt is current and running");
const SESSION_ROLE: Check = Handler("the session's roles and links");
const SHOW_ROLE: Check = Handler("the presenter's read; a prompter token");

pub const METHODS: &[MethodSpec] = &[
    m("ws.create", "stable", &[MayOwn]),
    m("ws.ensure", "stable", &[MayOwn]),
    m("ws.time", "stable", &[Session]),
    m("ws.list", "stable", &[Session]),
    m("ws.get_info", "stable", &[Member]),
    m("ws.delete", "stable", &[Owner]),
    m("ws.clone", "stable", &[MayOwn, Ws(Cap::Export)]),
    m("acl.set", "stable", &[Handler("`manage` on each resource; the owner when `manage` is touched")]),
    m("acl.query", "stable", &[Handler("may enter; anyone about itself, `manage` about others")]),
    m("acl.groups", "stable", &[Member]),
    m("acl.set_group", "stable", &[Ws(Cap::Manage)]),
    m("acl.delete_group", "stable", &[Ws(Cap::Manage)]),
    m("acl.members", "stable", &[Ws(Cap::Manage)]),
    m("acl.set_anonymous", "stable", &[Ws(Cap::Manage)]),
    m("acl.bind_macro", "stable", &[Ws(Cap::Manage)]),
    m("acl.audit", "stable", &[Ws(Cap::Manage)]),
    m("ws.list_subjects", "stable", &[MayOwn]),
    m("ws.create_entry", "stable", &[Ws(Cap::Manage)]),
    m("ws.list_entries", "stable", &[Ws(Cap::Manage)]),
    m("ws.close_entry", "stable", &[Ws(Cap::Manage)]),
    m("ws.join_entry", "stable", &[Handler("a person presenting an active entry the Zone switches admit")]),
    m("entry.describe", "stable", &[Public]),
    m("ws.get_user_state", "stable", &[Member]),
    m("ws.set_user_state", "stable", &[Member]),
    m("ws.import", "stable", &[MayOwn, Handler("a restore over a Workspace: its owner")]),
    m("ws.release_retention", "stable", &[Ws(Cap::Manage)]),
    m("ws.delete_checkpoint", "stable", &[Ws(Cap::Manage)]),
    m("ws.purge_deleted", "stable", &[Ws(Cap::Manage)]),
    m("ws.clear_staged", "stable", &[Ws(Cap::Manage)]),
    m("doc.resolve", "stable", &[Reads]),
    m("doc.read", "stable", &[Reads]),
    m("doc.list", "stable", &[Reads]),
    m("doc.find", "stable", &[Reads]),
    m("doc.query", "stable", &[Reads]),
    m("doc.prepare", "stable", &[OPS]),
    m("doc.commit", "stable", &[OPS]),
    m("doc.stage", "stable", &[Member]),
    m("doc.get_submission", "stable", &[Member]),
    m("doc.undo", "stable", &[OPS]),
    m("doc.get_changes", "stable", &[Reads]),
    m("doc.wait_changes", "stable", &[Member]),
    m("asset.import_object", "stable", &[Handler("a write right anywhere (the Commit that references it is judged where it writes)")]),
    m("lock.acquire", "stable", &[Handler("a write right on each entity")]),
    m("lock.renew", "stable", &[Member]),
    m("lock.release", "stable", &[Member]),
    m("lock.break", "stable", &[Entity(Cap::Manage, "entity_id")]),
    m("lock.list", "stable", &[Reads]),
    m("doc.outline", "internal", &[Reads]),
    m("doc.source_capabilities", "internal", &[Reads]),
    m("doc.source_query", "internal", &[Reads]),
    m("doc.index", "internal", &[Reads]),
    m("doc.get_collab_state", "internal", &[Entity(Cap::Read, "entity_id")]),
    m("replica.bootstrap", "internal", &[Reads]),
    m("show.start", "internal", &[Ws(Cap::Read), Handler("a Workspace-level write right locks it; anyone else presents read-only")]),
    m("show.heartbeat", "internal", &[SHOW_ROLE]),
    m("show.publish", "internal", &[SHOW_ROLE]),
    m("show.command", "internal", &[SHOW_ROLE]),
    m("show.watch", "internal", &[SHOW_ROLE]),
    m("show.notes", "internal", &[SHOW_ROLE]),
    m("show.end", "internal", &[Handler("the presenter, or `manage`")]),
    m("diag.list_unretained", "internal", &[Ws(Cap::Manage)]),
    m("diag.verify_refs", "internal", &[Ws(Cap::Manage)]),
    m("doc.list_annotations", "experimental", &[Reads]),
    m("doc.freshness", "experimental", &[Reads]),
    m("doc.relations", "experimental", &[Entity(Cap::Read, "entity_id")]),
    m("doc.list_versions", "experimental", &[Entity(Cap::Read, "entity_id")]),
    m("doc.restore_version", "experimental", &[Entity(Cap::Read, "entity_id")]),
    m("doc.checkpoint", "experimental", &[Ws(Cap::Export)]),
    m("doc.export", "experimental", &[Ws(Cap::Export)]),
    m("proc.start", "stable", &[Handler("a Source: `update` on it; a wish: `update` for analyze / code, `run` or a write right for run, on the wish")]),
    m("proc.get", "stable", &[RUN_OWN]),
    m("proc.apply", "stable", &[RUN_OWN, Handler("written as the macro identity ∩ the declared outputs")]),
    m("proc.cancel", "stable", &[RUN_OWN]),
    m("proc.list", "stable", &[Member]),
    m("proc.claim", "stable", &[Handler("the executor the run's context names, with its claim token")]),
    m("proc.heartbeat", "stable", &[RUN_TOKEN]),
    m("proc.report", "stable", &[RUN_TOKEN]),
    m("proc.tool", "stable", &[RUN_TOKEN]),
    m("proc.program", "stable", &[RUN_TOKEN]),
    m("proc.files", "stable", &[RUN_TOKEN]),
    m("ws.set_hosted_exec", "experimental", &[Owner]),
    m("ws.publish", "experimental", &[Entity(Cap::Manage, "entity_id")]),
    m("ws.unpublish", "experimental", &[Handler("`manage` on the entity (on `/` once it is deleted)")]),
    m("ws.list_publications", "experimental", &[Reads]),
    m("ws.list_referrers", "experimental", &[Entity(Cap::Read, "entity_id")]),
    m("control.list", "experimental", &[Session]),
    m("control.describe", "experimental", &[Session]),
    m("control.import", "experimental", &[OPS]),
    m("control.export", "experimental", &[Reads]),
    m("ext.build", "experimental", &[OPS]),
    m("ext.export", "experimental", &[Reads]),
    m("ext.install", "experimental", &[Handler("into the caller's own library (the control panel installs into anyone's)")]),
    m("ext.list_library", "experimental", &[MayOwn]),
    m("ext.uninstall", "experimental", &[MayOwn]),
    m("ext.library_bundle", "experimental", &[MayOwn]),
    m("ext.import", "experimental", &[MayOwn, OPS]),
    m("ext.pack", "experimental", &[MayOwn, Reads]),
    m("ext.upgrade", "experimental", &[MayOwn, OPS]),
    m("ext.export_template", "experimental", &[Handler("may enter; `export` on each Surface; the permission design with `manage`")]),
    m("ext.create_from_template", "experimental", &[MayOwn]),
    m("asset.stream_url", "stable", &[SESSION_ROLE]),
    m("session.create", "experimental", &[SESSION_ROLE]),
    m("session.list", "experimental", &[SESSION_ROLE]),
    m("session.list_mine", "experimental", &[SESSION_ROLE]),
    m("session.get", "experimental", &[SESSION_ROLE]),
    m("session.join", "experimental", &[SESSION_ROLE]),
    m("session.start", "experimental", &[SESSION_ROLE]),
    m("session.end", "experimental", &[SESSION_ROLE]),
    m("session.save", "experimental", &[SESSION_ROLE]),
    m("session.delete", "experimental", &[SESSION_ROLE]),
    m("session.set_access", "experimental", &[SESSION_ROLE]),
    m("session.invite", "experimental", &[SESSION_ROLE]),
    m("session.list_invites", "experimental", &[SESSION_ROLE]),
    m("session.revoke_invite", "experimental", &[SESSION_ROLE]),
    m("session.takeover", "experimental", &[SESSION_ROLE]),
    m("session.command", "experimental", &[SESSION_ROLE]),
    m("session.sample", "experimental", &[SESSION_ROLE]),
    m("session.task", "experimental", &[SESSION_ROLE]),
    m("session.claim", "experimental", &[SESSION_ROLE]),
    m("session.respond", "experimental", &[SESSION_ROLE]),
    m("session.review", "experimental", &[SESSION_ROLE]),
    m("session.publish", "experimental", &[SESSION_ROLE]),
    m("session.react", "experimental", &[SESSION_ROLE]),
    m("session.events", "experimental", &[SESSION_ROLE]),
    m("session.watch", "experimental", &[SESSION_ROLE]),
    m("session.replay", "experimental", &[SESSION_ROLE]),
    m("session.notes", "experimental", &[SESSION_ROLE]),
    m("session.feed_sample", "experimental", &[SESSION_ROLE]),
    m("session.post", "experimental", &[SESSION_ROLE]),
    m("session.list_posts", "experimental", &[SESSION_ROLE]),
    m("session.delete_post", "experimental", &[SESSION_ROLE]),
    m("session.rtc", "experimental", &[SESSION_ROLE]),
];

pub fn spec(method: &str) -> Option<&'static MethodSpec> {
    METHODS.iter().find(|s| s.method == method)
}

/// The README's authorization table (`| Method | Level | Authorization |`), generated from [`METHODS`].
pub fn authorization_table() -> String {
    let mut out = String::from("| Method | Level | Authorization |\n| --- | --- | --- |\n");
    for s in METHODS {
        let checks: Vec<String> = s.checks.iter().map(Check::describe).collect();
        out.push_str(&format!("| `{}` | {} | {} |\n", s.method, s.level, checks.join("; ")));
    }
    out
}
