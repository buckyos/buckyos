use super::*;
use buckyos_api::ext_tools_install::{ExtToolsInstall, EXT_TOOLS_INSTALL_SCHEMA};

pub(super) fn component(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', '\0', ':'])
}

pub(super) fn zones(ctx: &mut Context) -> Result<(), Failure> {
    let etc = ctx.root.join("etc");
    for path in ctx.entries(&etc)? {
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(".zone_lkgs.json"))
        {
            let v = ctx.json(&path)?;
            if v["schema"] != "buckyos.zone_lkgs.v1"
                || !v["accepted_at"].is_u64()
                || serde_json::from_value::<name_lib::ZoneDocument>(v["zone_document"].clone())
                    .is_err()
            {
                return Err(ctx.unsupported(
                    path.display(),
                    "unknown last-known-zone-state layout (target buckyos.zone_lkgs.v1)",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn account(ctx: &mut Context) -> Result<(), Failure> {
    use buckyos_api::host_account::{HostAccountFile, HOST_ACCOUNT_FILE};
    let account = ctx.root.join("etc").join(HOST_ACCOUNT_FILE);
    if ctx.exists(&account)? {
        let bytes = fs::read(&account).map_err(|e| ctx.error(account.display(), e))?;
        serde_json::from_slice::<HostAccountFile>(&bytes)
            .map_err(|e| ctx.unsupported(account.display(), e))?;
    }
    Ok(())
}

pub(super) fn receipts(ctx: &mut Context) -> Result<(), Failure> {
    for path in ctx.entries(&ctx.root.join("tools/installs"))? {
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let receipt: ExtToolsInstall = serde_json::from_value(ctx.json(&path)?)
                .map_err(|e| ctx.unsupported(path.display(), e))?;
            if receipt.schema != EXT_TOOLS_INSTALL_SCHEMA {
                return Err(ctx.unsupported(
                    path.display(),
                    format!(
                        "actual {}, target {EXT_TOOLS_INSTALL_SCHEMA}",
                        receipt.schema
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn startup(ctx: &mut Context) -> Result<(), Failure> {
    let old = ctx.root.join("etc/start_config.json");
    let new = ctx.security.join("start_config.json");
    let active_exists = ctx.exists(&new)?;
    if active_exists {
        buckyos_api::validate_start_config_schema(&ctx.json(&new)?)
            .map_err(|e| ctx.unsupported(new.display(), e))?;
    }
    if old == new || !ctx.exists(&old)? {
        return Ok(());
    }
    buckyos_api::validate_start_config_schema(&ctx.json(&old)?)
        .map_err(|e| ctx.unsupported(old.display(), e))?;
    let read = |p: &Path| fs::read(p).map_err(|e| ctx.error(p.display(), e));
    let new = if active_exists && read(&new)? != read(&old)? {
        let archive = ctx.security.join("start_config.legacy.json");
        if ctx.exists(&archive)? && read(&archive)? != read(&old)? {
            return Err(ctx.error(
                archive.display(),
                "existing legacy archive differs from the source",
            ));
        }
        let message = format!(
            "{} and {} differ. Keep the active security configuration and archive the older copy at {}?",
            old.display(), new.display(), archive.display()
        );
        let accepted = (ctx.confirm)(&message).map_err(|e| ctx.error(old.display(), e))?;
        if !accepted {
            return Err(ctx.error(old.display(), message));
        }
        archive
    } else {
        new
    };
    let private = ctx.copy_path();
    fs::copy(&old, &private).map_err(|e| ctx.error(old.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&private, fs::Permissions::from_mode(0o600))
            .map_err(|e| ctx.error(old.display(), e))?;
    }
    ctx.changes.push(Change {
        permissions_from: Some(old.clone()),
        target: new,
        staged: Some(private),
    });
    ctx.changes.push(Change {
        permissions_from: None,
        target: old,
        staged: None,
    });
    Ok(())
}

pub(super) fn executor(ctx: &mut Context) -> Result<(), Failure> {
    let dir = ctx.root.join("local/node_daemon/executor");
    if !ctx.exists(&dir)? {
        return Ok(());
    }
    let record = |path: &Path, kind: &str| -> Result<(), Failure> {
        if !ctx.exists(path)? {
            return Err(ctx.unsupported(path.display(), "required executor record missing"));
        }
        let bytes = fs::read(path).map_err(|e| ctx.error(path.display(), e))?;
        node_executor::validate_offline_record(kind, &bytes)
            .map_err(|e| ctx.unsupported(path.display(), e))
    };
    record(&dir.join("meta.json"), "meta")?;
    for op in ctx.entries(&dir.join("operations"))? {
        record(&op.join("request.json"), "request")?;
        record(&op.join("state.json"), "state")?;
        if ctx.exists(&op.join("result.json"))? {
            record(&op.join("result.json"), "result")?;
        }
    }
    for p in ctx.entries(&dir.join("tombstones"))? {
        record(&p, "tombstone")?;
    }
    Ok(())
}

pub(super) fn kmsg(ctx: &mut Context) -> Result<(), Failure> {
    let path = ctx.root.join("local/kmsg");
    if !ctx.exists(&path)? {
        return Ok(());
    }
    for required in ["conf", "db"] {
        if !ctx.exists(&path.join(required))? {
            return Err(ctx.unsupported(path.display(), "incomplete sled store"));
        }
    }
    let staged = ctx.copy_path();
    copy_tree(&path, &staged).map_err(|e| ctx.error(path.display(), e))?;
    let db = sled::open(&staged).map_err(|e| ctx.error(path.display(), e))?;
    kmsg::sled_msg_queue::validate_offline_copy(&db).map_err(|e| ctx.unsupported(path.display(), e))
}

fn validate_goal_file<T: serde::de::DeserializeOwned>(
    ctx: &Context,
    path: &Path,
    schema: &str,
) -> Result<(), Failure> {
    let json = ctx.json(path)?;
    if json["schema"].as_str() != Some(schema) {
        return Err(ctx.unsupported(
            path.display(),
            format!("unsupported goal record, expected {schema}"),
        ));
    }
    serde_json::from_value::<T>(json).map_err(|e| ctx.unsupported(path.display(), e))?;
    Ok(())
}

fn validate_goals(ctx: &Context, agent: &Path) -> Result<(), Failure> {
    use libopendan::protocol::*;
    use std::io::BufRead;
    for goal in ctx.entries(&agent.join("state/goals"))? {
        if !goal.is_dir() {
            continue;
        }
        validate_goal_file::<GoalDefinition>(ctx, &goal.join("goal.json"), GOAL_SCHEMA)?;
        for p in ctx.entries(&goal.join("revisions"))? {
            if p.extension().is_some_and(|s| s == "json") {
                validate_goal_file::<GoalDefinition>(ctx, &p, GOAL_SCHEMA)?;
            }
        }
        let state = goal.join("state.json");
        if ctx.exists(&state)? {
            validate_goal_file::<GoalState>(ctx, &state, GOAL_STATE_SCHEMA)?;
        }
        for epoch in ctx.entries(&goal.join("epochs"))? {
            let state = epoch.join("state.json");
            if ctx.exists(&state)? {
                validate_goal_file::<GoalState>(ctx, &state, GOAL_STATE_SCHEMA)?;
            }
        }
        let schedule = goal.join("schedule.json");
        if ctx.exists(&schedule)? {
            validate_goal_file::<GoalSchedule>(ctx, &schedule, GOAL_SCHEDULE_SCHEMA)?;
        }
        for name in ["changes.jsonl", "activations.jsonl"] {
            let path = goal.join(name);
            if !ctx.exists(&path)? {
                continue;
            }
            let file = fs::File::open(&path).map_err(|e| ctx.error(path.display(), e))?;
            for line in io::BufReader::new(file).lines() {
                let line = line.map_err(|e| ctx.error(path.display(), e))?;
                if name == "changes.jsonl" {
                    serde_json::from_str::<GoalChange>(&line)
                        .map_err(|e| ctx.unsupported(path.display(), e))?;
                } else {
                    let a: GoalActivation = serde_json::from_str(&line)
                        .map_err(|e| ctx.unsupported(path.display(), e))?;
                    if a.schema != GOAL_ACTIVATION_SCHEMA {
                        return Err(
                            ctx.unsupported(path.display(), "unknown goal activation schema")
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// Only fixed protocol files reached through an Agent's persisted session
/// registry may be read outside the installation. No transaction target is added.
fn session_record_exists(ctx: &Context, path: &Path) -> Result<bool, Failure> {
    crate::transaction::validate_path(path).map_err(|e| ctx.error(path.display(), e))?;
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !linked(&meta) => Ok(true),
        Ok(_) => Err(ctx.unsupported(path.display(), "expected a regular session record")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ctx.error(path.display(), e)),
    }
}

pub(super) fn agents(ctx: &mut Context) -> Result<(), Failure> {
    use agent_tool::agent_memory::{AgentMemory, AgentMemoryConfig};
    use libopendan::protocol::*;
    for agent in &ctx.agents {
        use libopendan::skills;
        if ctx.exists(&agent.join(skills::SKILLS_RECORD))? {
            let record =
                skills::load_record(agent).map_err(|e| ctx.unsupported(agent.display(), e))?;
            if record.schema != skills::SKILLS_RECORD_SCHEMA {
                return Err(
                    ctx.unsupported(agent.display(), "unsupported installed skill manifest")
                );
            }
        }
        if ctx.exists(&agent.join(skills::SKILLS_STATE))? {
            let state =
                skills::load_state(agent).map_err(|e| ctx.unsupported(agent.display(), e))?;
            if state.schema != skills::SKILLS_STATE_SCHEMA {
                return Err(ctx.unsupported(agent.display(), "unsupported installed skill state"));
            }
        }
        validate_goals(ctx, agent)?;

        let memory = agent.join("memory");
        if ctx.exists(&memory)? {
            let meta = memory.join(".meta/meta.json");
            if ctx.exists(&meta)?
                && ctx.json(&meta)?["schema_version"] != agent_tool::agent_memory::SCHEMA_VERSION
            {
                return Err(ctx.unsupported(meta.display(), "unsupported Agent Memory schema"));
            }
            ctx.exists(&memory.join(".meta/occasions.jsonl"))?;
            AgentMemory::open_read(AgentMemoryConfig::new(&memory))
                .map_err(|e| ctx.unsupported(memory.display(), e))?;
        }
        let layout = libopendan::state::AgentLayout::new(agent);
        for entry in ctx.entries(&layout.sessions_dir())? {
            let Some(id) = entry.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if id.starts_with('.') || entry.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if !component(id) || !session_record_exists(ctx, &entry)? {
                return Err(ctx.unsupported(entry.display(), "invalid session registry entry"));
            }
            let reg: RegistryEntry = serde_json::from_value(ctx.json(&entry)?)
                .map_err(|e| ctx.unsupported(entry.display(), e))?;
            if id != reg.session_id {
                return Err(ctx.unsupported(entry.display(), "session identity mismatch"));
            }
            let dir = Path::new(&reg.location);
            #[cfg(windows)]
            {
                use std::path::Prefix;
                if !matches!(dir.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
                {
                    return Err(
                        ctx.unsupported(dir.display(), "expected a local session directory")
                    );
                }
            }
            crate::transaction::validate_path(dir).map_err(|e| ctx.error(dir.display(), e))?;
            let area = dir.join(STATE_DIR);
            let config_exists = session_record_exists(ctx, &area.join(SESSION_CONFIG_FILE))?;
            let mut other_records = false;
            for name in [STATE_FILE, SUMMARY_FILE, WORKLOG_FILE, BINDING_FILE] {
                other_records |= session_record_exists(ctx, &area.join(name))?;
            }
            // FsRegistry persists an unreachable marker separately. Its read path
            // ignores the value of 'unreachable' in the JSON registry entry.
            let mark = layout.unreachable_mark(id);
            let marked = session_record_exists(ctx, &mark)?;
            if !config_exists && marked && !other_records {
                continue;
            }
            if !config_exists {
                return Err(
                    ctx.unsupported(area.display(), "registered session configuration missing")
                );
            }
            // SessionDir::open / config / state / summary_opt / binding_opt and
            // Worklog::len / reverse perform only metadata, reads and seeks.
            let sd = libopendan::session::SessionDir::open(dir)
                .map_err(|e| ctx.unsupported(dir.display(), e))?;
            let cfg = sd.config().map_err(|e| ctx.unsupported(dir.display(), e))?;
            if cfg.session_id() != id {
                return Err(ctx.unsupported(
                    dir.display(),
                    "session configuration differs from its registry identity",
                ));
            }
            let state = sd.state().map_err(|e| ctx.unsupported(dir.display(), e))?;
            let summary = sd
                .summary_opt()
                .map_err(|e| ctx.unsupported(dir.display(), e))?;
            if cfg.schema != SESSION_CONFIG_SCHEMA
                || state.schema != SESSION_STATE_SCHEMA
                || summary.is_some_and(|s| s.schema != SESSION_SUMMARY_SCHEMA)
            {
                return Err(ctx.unsupported(dir.display(),format!("expected {SESSION_CONFIG_SCHEMA}, {SESSION_STATE_SCHEMA}, {SESSION_SUMMARY_SCHEMA}")));
            }
            sd.binding_opt()
                .map_err(|e| ctx.unsupported(dir.display(), e))?;
            let len = sd
                .worklog()
                .len()
                .map_err(|e| ctx.error(area.display(), e))?;
            if state.worklog.committed_bytes > 0 && len >= state.worklog.committed_bytes {
                let mut rows = sd
                    .worklog()
                    .reverse(state.worklog.committed_bytes, 0)
                    .map_err(|e| ctx.error(area.display(), e))?;
                let mut seq = state.worklog.committed_seq;
                while let Some((_, entry)) = rows
                    .next_json::<WorklogEntry>()
                    .map_err(|e| ctx.unsupported(area.display(), e))?
                {
                    if seq != entry.seq || seq == 0 {
                        return Err(ctx.unsupported(area.display(), "worklog sequence mismatch"));
                    }
                    seq -= 1;
                }
                if seq != 0 {
                    return Err(
                        ctx.unsupported(area.display(), "committed worklog entries missing")
                    );
                }
            }
            if len < state.worklog.committed_bytes {
                return Err(ctx.error(
                    area.display(),
                    "worklog truncated before committed boundary",
                ));
            }
        }
    }
    Ok(())
}
