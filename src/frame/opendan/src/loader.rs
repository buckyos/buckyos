//! Start-up and shutdown of the Loader: AgentRoot identity → AgentRoot sync
//! (unless the template is pinned) → the instance role (its name and the
//! owner's supplement) →
//! `agent.toml` → Agent State (file implementation, registered for the
//! process) → HTTP (kRPC + WebUI) → Supervisor → modules → runtime info.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_tool::xllm::XllmDeps;
use anyhow::{anyhow, Context, Result};
use buckyos_api::{agent_short_name, AgentId, AgentRuntimeInfo, AgentSettings, KEventClient};
use buckyos_http_server::{DirHandlerOptions, Runner};
use libopendan::bridge::{EventBridge, KEventBridge, TimerBridge};
use libopendan::channel::{KmsgChannels, Waker};
use libopendan::host::goals::{GoalHost, GoalHostOptions};
use libopendan::host::{HostDeps, Supervisor};
use libopendan::protocol::{builtin_runtimes, RuntimeAvailability};
use libopendan::runner::{BehaviorAssembler, DefaultAssembler, ObjectStore, OutboundSink, RunnerOptions};
use libopendan::state::{
    register_in_process, unregister_in_process, AgentStateClient, FsAgentStateClient, WithNotify,
};
use name_lib::DID;
use ndn_lib::ObjId;

use crate::config::AgentConfig;
use crate::home::Home;
use crate::records::{self, AgentRecords};
use crate::rootfs::{self, RootIdentity};
use crate::service::{Access, LoaderInfo, ModuleStatus, StateService, SERVICE_PATH, WEB_API_PATH};
use crate::tasks::{owner_of, CancelBridge, TaskService, TurnTasks};
use crate::ui::{MailService, MsgCenterSink, OutboundTexts, UiModule};

/// The AgentSpec this process hosts the agent of.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecRef {
    pub agent_doc_object_id: ObjId,
    pub generation: u64,
}

/// Everything the Loader takes from its surroundings.
pub struct LoaderEnv {
    /// Driving identity of every session this process hosts: the app
    /// principal of the agent's runtime app (not the agent DID).
    pub who: String,
    pub agent_did: String,
    /// Name of the agent in kevent / kmsg names (default: from the DID).
    pub agent_id: Option<String>,
    /// The agent's owner (`AgentSpec.agent_doc.owner`): the only sender
    /// whose messages reach a session. `None`: messages reach none.
    pub owner_did: Option<DID>,
    /// `None` outside a zone: no AgentDocument the AgentRoot is bound to
    /// and no runtime info to write.
    pub spec: Option<SpecRef>,
    /// Read once at start-up; `allow_group` is read again while running.
    pub settings: AgentSettings,
    /// `settings`, `profile` and `info` outside the AgentRoot.
    pub records: Arc<dyn AgentRecords>,
    pub agent_root: PathBuf,
    pub package_root: Option<PathBuf>,
    /// The template's `agent_skills` sub package (`cmd_skills/`,
    /// `goal_skills/`, `tools/`), synced like the package itself.
    pub skills_package_root: Option<PathBuf>,
    /// Version of the package at `package_root` (the agent's template).
    pub package_version: Option<String>,
    pub channels: Arc<KmsgChannels>,
    pub waker: Arc<dyn Waker>,
    pub kevent: Option<Arc<KEventClient>>,
    /// `None`: no msg-center (development host); ui rules stay idle.
    pub mail: Option<Arc<dyn MailService>>,
    /// `None`: no TaskMgr (development host); Turns get no task.
    pub tasks: Option<Arc<dyn TaskService>>,
    /// `None`: no NamedStore (development host); report artifacts stay
    /// files and replies attach no local file.
    pub objects: Option<Arc<dyn ObjectStore>>,
    /// The owner's notification service (msg-center; files with `--dev`).
    /// `None`: a Work cannot ask, `agent-session notify` is not available.
    pub notify: Option<Arc<dyn libopendan::notify::NotifyHost>>,
    /// Where a finished user task is shared as the agent's own post (the
    /// agent's HomeStation; files with `--dev`). `None`: nothing is shared.
    pub share: Option<Arc<dyn libopendan::share::ShareHost>>,
    pub access: Access,
    pub port: u16,
    pub web_dir: Option<PathBuf>,
    pub xllm: XllmDeps,
    /// The `xagent` executable, wrapped as `agent-session` in every session.
    pub session_cli: Option<PathBuf>,
    /// Further commands every session gets (`llm_understand_media` →
    /// `agent_tool llm_understand_media`).
    pub session_helpers: Vec<libopendan::runtime::bin_overlay::SessionHelper>,
    pub options: RunnerOptions,
}

pub struct Loader {
    pub agent: Arc<dyn AgentStateClient>,
    pub supervisor: Arc<Supervisor>,
    pub info: Arc<LoaderInfo>,
    pub ui: Option<Arc<UiModule>>,
    pub config: AgentConfig,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// Where workspaces are best placed in the agent's own environment: the
/// AgentRoot's `workspaces/`, and the owner's home when the AgentRoot lives
/// under it (`<home>/agents/<agent>`).
fn container_roots(agent_root: &std::path::Path) -> Vec<PathBuf> {
    let mut roots = vec![agent_root.join("workspaces")];
    let home = agent_root
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "agents"))
        .and_then(|agents| agents.parent());
    if let Some(home) = home.filter(|h| h.is_dir()) {
        roots.push(home.to_path_buf());
    }
    roots
}

/// node_daemon runs this Agent Service natively on its node (the zone
/// owner's Agent on its host runtime): there is no agent container.
fn native_service() -> bool {
    std::env::var("OPENDAN_SERVICE_MODE").is_ok_and(|v| v == "native")
}

fn container_note() -> &'static str {
    if std::path::Path::new("/.dockerenv").exists() {
        "It is the agent's container: paths here are not the owner's host paths."
    } else {
        "The agent currently runs as a process on its host (development), so this is the host's own environment."
    }
}

impl Loader {
    pub async fn start(env: LoaderEnv) -> Result<Self> {
        rootfs::claim(
            &env.agent_root,
            &RootIdentity {
                agent_did: env.agent_did.clone(),
                agent_doc_object_id: env.spec.as_ref().map(|s| s.agent_doc_object_id.clone()),
            },
        )?;
        let pinned = !env.settings.template_auto_update && rootfs::initialized(&env.agent_root);
        let synced = match &env.package_root {
            Some(pkg) if pinned => {
                log::info!(
                    "loader: the template of {} is pinned; AgentRoot {} is not synced from {}",
                    env.agent_did,
                    env.agent_root.display(),
                    pkg.display()
                );
                false
            }
            Some(pkg) => {
                let r = rootfs::sync_from_package(pkg, &env.agent_root)
                    .with_context(|| format!("sync AgentRoot from {}", pkg.display()))?;
                log::info!(
                    "loader: AgentRoot {} synced from {} ({}): copied={} updated={} unchanged={} kept_local={}",
                    env.agent_root.display(),
                    pkg.display(),
                    env.package_version.as_deref().unwrap_or("unknown version"),
                    r.copied,
                    r.updated,
                    r.unchanged,
                    r.preserved
                );
                true
            }
            None => {
                log::warn!("loader: no agent package directory; AgentRoot is used as it is");
                false
            }
        };
        if let Some(skills) = env.skills_package_root.as_ref().filter(|_| !pinned) {
            let r = rootfs::sync_from_package(skills, &env.agent_root)
                .with_context(|| format!("sync AgentRoot from the skills package {}", skills.display()))?;
            log::info!(
                "loader: skills of the template synced from {}: copied={} updated={} unchanged={} kept={}",
                skills.display(),
                r.copied,
                r.updated,
                r.unchanged,
                r.preserved
            );
        }
        let display_name = match records::profile(env.records.as_ref()).await {
            Ok(profile) => DID::from_str(&env.agent_did)
                .ok()
                .and_then(|did| AgentId::from_agent_did(&did).ok())
                .map(|id| (profile.resolved_display_name(&id), agent_short_name(&id))),
            Err(e) => {
                log::warn!("loader: profile of {} not readable: {e}", env.agent_did);
                None
            }
        };
        // Who the agent is and whose: its name, account and DID, and its
        // owner's account and DID (the review of the UI prompt).
        let owner_line = env.owner_did.as_ref().map(|owner| {
            let owner = owner.to_string();
            format!(
                "Your owner is `{}` (DID `{owner}`).",
                libopendan::runner::assembler::did_account(&owner)
            )
        });
        let instance_role = display_name
            .map(|(name, account)| {
                let mut line = format!(
                    "Your name is {name} (account `{account}`, DID `{}`).",
                    env.agent_did
                );
                if let Some(o) = &owner_line {
                    line.push(' ');
                    line.push_str(o);
                }
                line
            })
            .into_iter()
            .chain(Some(env.settings.role_supplement.trim().to_string()))
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        rootfs::write_role_supplement(&env.agent_root, &instance_role)?;
        let config = AgentConfig::load(&env.agent_root)?;

        let mut fs = FsAgentStateClient::open(
            &env.agent_root,
            &env.agent_did,
            Some(env.channels.client()),
            Some(env.waker.clone()),
        )
        .map_err(|e| anyhow!("open Agent State at {}: {e}", env.agent_root.display()))?;
        if let Some(id) = &env.agent_id {
            fs = fs.with_agent_id(id);
        }
        let agent: Arc<dyn AgentStateClient> = match &env.notify {
            Some(notify) => Arc::new(WithNotify::new(Arc::new(fs), notify.clone())),
            None => Arc::new(fs),
        };
        register_in_process(agent.clone());
        // The runtimes the system configures for this agent (its container,
        // the owner's host, further enabled nodes), before anything can
        // choose one; followed while running (`watch_runtime_settings`).
        project_runtimes(agent.as_ref(), &env.agent_root, &env.settings, &env.who).await;
        tokio::spawn(check_runtimes(agent.clone()));

        let info = Arc::new(LoaderInfo {
            agent_did: env.agent_did.clone(),
            agent_id: agent.agent_id().to_string(),
            who: env.who.clone(),
            agent_root: env.agent_root.display().to_string(),
            started_at_ms: libopendan::now_ms(),
            modules: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
        });
        let module = |name: &str, enabled: bool, running: bool, note: Option<&str>| {
            info.modules.lock().expect("modules").push(ModuleStatus {
                name: name.to_string(),
                enabled,
                running,
                note: note.map(str::to_string),
            });
        };

        let mut bridges: Vec<Arc<dyn EventBridge>> = vec![Arc::new(TimerBridge {
            agent: agent.clone(),
            who: env.who.clone(),
        })];
        if let Some(client) = &env.kevent {
            bridges.push(Arc::new(KEventBridge {
                client: client.clone(),
                agent: agent.clone(),
                who: env.who.clone(),
            }));
        }
        let outbound: Option<Arc<dyn OutboundSink>> = env.mail.clone().map(|mail| {
            Arc::new(MsgCenterSink {
                mail,
                texts: OutboundTexts::load(&env.agent_root, &config.language),
                owner: env.owner_did.clone(),
            }) as Arc<dyn OutboundSink>
        });
        if env.session_cli.is_none() {
            log::warn!("loader: no xagent executable next to opendan; sessions get no `agent-session` command");
        }
        let mut options = env.options.clone();
        if let Some(ms) = config.loader.placeholder_delay_ms {
            options.placeholder_delay = Duration::from_millis(ms);
        }
        // Commands run by a session's shell reach the Agent State as the
        // same identity.
        std::env::set_var("LIBOPENDAN_WHO", &env.who);
        // ... and know how many sub sessions this host drives at once (a
        // Todo dispatch beyond it is refused, not queued).
        let max_child_concurrency = 4;
        std::env::set_var(
            libopendan::todos::ENV_MAX_CHILDREN,
            max_child_concurrency.to_string(),
        );
        let host = HostDeps {
            who: env.who.clone(),
            agent: agent.clone(),
            inputs: env.channels.clone(),
            waker: env.waker.clone(),
            xllm: {
                // `read` reaches other devices' files through the Agent's node runtimes.
                let mut xllm = env.xllm.clone();
                xllm.url_reader = Some(Arc::new(crate::urls::AgentUrlReader {
                    objects: agent_did_object_lib::ObjectUrlReader::builtin(),
                    agent: agent.clone(),
                    ops_dir: env.agent_root.join(".meta").join("node-reads"),
                }));
                xllm
            },
            assembler: Arc::new(BehaviorAssembler::new(DefaultAssembler {
                known_objects: true,
                ..Default::default()
            })),
            session_cli: env.session_cli.clone(),
            session_helpers: env.session_helpers.clone(),
            app_tools: Vec::new(),
            options,
            max_child_concurrency,
            bridges,
            outbound,
            objects: env.objects.clone(),
            notify: env.notify.clone(),
            share: env.share.clone(),
            turn_tasks: env.tasks.clone().map(|tasks| {
                Arc::new(TurnTasks {
                    tasks,
                    owner: owner_of(&env.who),
                }) as Arc<dyn libopendan::runner::TurnTaskSink>
            }),
            runtime_id: None,
            notifier: match (&env.kevent, DID::from_str(&env.agent_did)) {
                (Some(client), Ok(agent)) => Some(Arc::new(crate::obj_provider::SessionEvents::new(client.clone(), agent))
                    as Arc<dyn libopendan::channel::Notifier>),
                _ => None,
            },
        };
        let mut supervisor = Supervisor::new(
            host,
            Some(Duration::from_secs(config.loader.idle_unload_secs)),
        );
        for rule in &config.loader.ui {
            if let Some(secs) = rule.idle_unload_secs {
                supervisor =
                    supervisor.with_class_idle(&rule.session_class, Duration::from_secs(secs));
            }
        }
        // Goals: their Goal Sessions follow the Goal list, scheduled run
        // opportunities come from the local schedule (phase 1).
        if config.loader.goals.enabled {
            supervisor = supervisor.with_goals(GoalHost::new(GoalHostOptions::new(
                env.agent_root.join("sessions"),
                config.llm_context.clone(),
            )));
        }

        let mut tasks = Vec::new();
        let ui = match (&env.mail, config.loader.ui.is_empty()) {
            (_, true) => {
                module("ui", false, false, Some("no [[loader.ui]] rule"));
                None
            }
            (None, false) => {
                module("ui", true, false, Some("msg-center is not available to this host"));
                None
            }
            (Some(_), false) if env.owner_did.is_none() => {
                module("ui", true, false, Some("the agent has no owner"));
                None
            }
            (Some(mail), false) => {
                module("ui", true, true, None);
                Some(UiModule::new(
                    config.clone(),
                    agent.clone(),
                    &env.who,
                    env.owner_did.clone().expect("checked"),
                    env.settings.allow_group,
                    env.records.clone(),
                    mail.clone(),
                    env.channels.clone(),
                    supervisor.clone(),
                    env.agent_root.join("sessions"),
                ))
            }
        };
        module(
            "goals",
            config.loader.goals.enabled,
            config.loader.goals.enabled,
            config.loader.goals.enabled.then_some("schedule backend: local"),
        );
        module(
            "self_check",
            config.loader.self_check.enabled,
            false,
            config.loader.self_check.enabled.then_some("not available in this version"),
        );
        let maintenance = libopendan::goal::defaults(Some(&env.agent_root), libopendan::goal::GOAL_CLASS).maintenance;
        module(
            "memory_maintenance",
            maintenance.enabled,
            maintenance.enabled && config.loader.goals.enabled,
            Some(if maintenance.enabled {
                "built-in goal seeded by the goals module; decision pre-check off"
            } else {
                "[session.goal.maintenance] enabled = false"
            }),
        );

        if config.loader.agent_state_service || config.loader.webui {
            let runner = Runner::new(env.port);
            if config.loader.agent_state_service {
                let service = Arc::new(StateService {
                    agent: agent.clone(),
                    supervisor: supervisor.clone(),
                    info: info.clone(),
                    ui: ui.clone(),
                    access: env.access.clone(),
                    sessions: match libopendan::session_context::SessionService::new(
                        &env.agent_root,
                        env.session_cli.clone(),
                    ) {
                        Ok(s) => Some(Arc::new(s)),
                        Err(e) => {
                            log::warn!("loader: no session tokens ({e}); helpers on other runtimes cannot reach this agent");
                            None
                        }
                    },
                    objects: std::sync::OnceLock::new(),
                    home: Home::new(
                        env.records.clone(),
                        env.owner_did.as_ref().map(|d| d.to_string()),
                        match &env.access {
                            Access::Zone { zone_host, .. } => zone_host.as_ref().map(|h| format!("https://{h}")),
                            Access::Open { .. } => None,
                        },
                    ),
                });
                if let Ok(agent_did) = name_lib::DID::from_str(&info.agent_did) {
                    let provider = crate::obj_provider::provider(Arc::downgrade(&service), agent.agent_id(), agent_did);
                    let _ = service.objects.set(provider.into_http_server());
                }
                for path in [SERVICE_PATH, WEB_API_PATH] {
                    runner
                        .add_http_server(path.to_string(), service.clone())
                        .map_err(|e| anyhow!("mount {path}: {e:?}"))?;
                }
                let agent_path = format!("/kapi/{}", agent.agent_id());
                if agent_path != SERVICE_PATH {
                    runner.add_http_server(agent_path.clone(), service)
                        .map_err(|e| anyhow!("mount {agent_path}: {e:?}"))?;
                }
            }
            module("agent_state_service", config.loader.agent_state_service, config.loader.agent_state_service, None);
            match (&env.web_dir, config.loader.webui) {
                (Some(dir), true) if dir.join("index.html").is_file() => {
                    runner
                        .add_dir_handler_with_options(
                            "/".to_string(),
                            dir.clone(),
                            DirHandlerOptions {
                                index_file: Some("index.html".to_string()),
                                fallback_file: Some("index.html".to_string()),
                            },
                        )
                        .await
                        .map_err(|e| anyhow!("mount WebUI {}: {e:?}", dir.display()))?;
                    module("webui", true, true, None);
                }
                (_, true) => module("webui", true, false, Some("no built WebUI (web/index.html) next to opendan")),
                (_, false) => module("webui", false, false, None),
            }
            let port = env.port;
            // Another process on the service port is another host of this
            // agent (the app container): stop before any session is driven.
            drop(
                std::net::TcpListener::bind(("0.0.0.0", port))
                    .with_context(|| format!("service port {port} is not available"))?,
            );
            let errors = info.clone();
            tasks.push(tokio::spawn(async move {
                if let Err(e) = runner.run().await {
                    log::error!("loader: http server on port {port} ended: {e:?}");
                    errors.error("http", format!("server on port {port} ended: {e:?}"));
                }
            }));
        } else {
            module("agent_state_service", false, false, None);
            module("webui", false, false, None);
        }

        // Recovery does not wait for something to touch a session: the
        // first registry scan hosts everything left unfinished.
        tasks.push(tokio::spawn(supervisor.clone().run()));
        match &env.notify {
            Some(notify) => {
                module("notify", true, true, None);
                tasks.push(tokio::spawn(crate::notify::watch_answers(
                    notify.clone(),
                    env.kevent.clone(),
                    supervisor.clone(),
                    env.agent_did.clone(),
                    env.options.poll_interval.saturating_mul(6),
                )));
            }
            None => module("notify", false, false, Some("no notification service")),
        }
        if let Some(ui) = &ui {
            tasks.push(tokio::spawn(
                ui.clone().run(
                    env.waker.clone(),
                    env.options.poll_interval.min(Duration::from_secs(5)),
                ),
            ));
        }
        tasks.push(tokio::spawn(watch_runtime_settings(
            env.records.clone(),
            agent.clone(),
            env.agent_root.clone(),
            env.settings.clone(),
            env.who.clone(),
            env.options.poll_interval.saturating_mul(6).max(Duration::from_secs(15)),
        )));
        if let Some(shell_tasks) = env.xllm.shell_tasks.clone() {
            // The `shell` tasks a previous process left open are taken
            // back (or failed) before anything asks for them.
            tasks.push(tokio::spawn(async move { shell_tasks.recover().await }));
        }
        match &env.tasks {
            Some(service) => {
                module("task_mgr", true, true, None);
                let bridge = Arc::new(CancelBridge {
                    tasks: service.clone(),
                    agent: agent.clone(),
                    supervisor: supervisor.clone(),
                    who: env.who.clone(),
                });
                tasks.push(tokio::spawn(
                    bridge.run(env.options.poll_interval.min(Duration::from_secs(3))),
                ));
            }
            None => module("task_mgr", false, false, Some("TaskMgr is not available to this host")),
        }
        if let Some(spec) = &env.spec {
            let template_version = if synced {
                env.package_version.clone()
            } else {
                match records::info(env.records.as_ref()).await {
                    Ok(previous) => previous.and_then(|i| i.template_version),
                    Err(e) => {
                        log::warn!("loader: the previous runtime info is not readable ({e}); its template version is lost");
                        None
                    }
                }
            };
            tasks.push(tokio::spawn(report_loaded(
                env.records.clone(),
                AgentRuntimeInfo {
                    agent_doc_object_id: spec.agent_doc_object_id.clone(),
                    generation: spec.generation,
                    loaded_at: buckyos_kit::buckyos_get_unix_timestamp(),
                    template_version,
                },
                info.clone(),
            )));
        }
        log::info!(
            "loader: agent {} hosted by {} (root {})",
            env.agent_did,
            env.who,
            env.agent_root.display()
        );
        Ok(Self {
            agent,
            supervisor,
            info,
            ui,
            config,
            tasks,
        })
    }

    /// Stop taking new work, end the serving loops and wait for them (their
    /// leases are released). No session is stopped: each keeps its
    /// committed state and is recovered by the next start.
    pub async fn shutdown(self) {
        for t in &self.tasks {
            t.abort();
        }
        for t in self.tasks {
            let _ = t.await;
        }
        self.supervisor.shutdown().await;
        unregister_in_process(self.agent.agent_did());
    }
}

/// Register / refresh the agent's runtimes from its settings.
pub async fn project_runtimes(
    agent: &dyn AgentStateClient,
    agent_root: &std::path::Path,
    settings: &AgentSettings,
    who: &str,
) {
    let builtin = builtin_runtimes(
        container_roots(agent_root),
        container_note(),
        settings,
        agent.agent_did(),
        native_service(),
    );
    match agent.runtimes().ensure_builtin(&builtin, who).await {
        Ok(r) => log::info!(
            "loader: runtimes {}",
            r.iter()
                .map(|r| format!(
                    "{}={:?}{}",
                    r.runtime_id,
                    r.availability,
                    if r.default { " (default)" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(e) => log::warn!("loader: runtimes not registered: {e}"),
    }
}

/// Check every usable runtime once it is projected (start, upgrade,
/// settings change): whether it answers, and whether its standard tools are
/// there (`baseline_check`, a hint beside the declared baseline). In the
/// background: a node that does not answer never holds the loader up.
pub async fn check_runtimes(agent: Arc<dyn AgentStateClient>) {
    let runtimes = match agent.runtimes().list().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("loader: runtimes not checked: {e}");
            return;
        }
    };
    for r in runtimes.into_iter().filter(|r| r.availability == RuntimeAvailability::Available) {
        match agent.runtimes().check(&r.runtime_id).await {
            Ok(c) => match &c.baseline_check {
                Some(b) if !b.ok => log::warn!(
                    "loader: runtime {} ({:?} declared {}): {}",
                    c.runtime_id,
                    c.availability,
                    c.baseline.as_str(),
                    b.error.clone().unwrap_or_else(|| format!(
                        "standard tools missing: {} (install them on that machine, see `xagent runtime baseline`)",
                        b.missing.join(", ")
                    ))
                ),
                _ => log::info!("loader: runtime {} {:?}, standard tools complete", c.runtime_id, c.availability),
            },
            Err(e) => log::warn!("loader: runtime {} not checked: {e}", r.runtime_id),
        }
    }
}

/// The owner changes an agent's runtimes (enable the host, pick a default,
/// adjust ComputerUse) while the agent runs: the projection follows the
/// settings. Existing sessions keep what they froze.
async fn watch_runtime_settings(
    records: Arc<dyn AgentRecords>,
    agent: Arc<dyn AgentStateClient>,
    agent_root: PathBuf,
    mut last: AgentSettings,
    who: String,
    every: Duration,
) {
    loop {
        tokio::time::sleep(every).await;
        let current = match records::settings(records.as_ref()).await {
            Ok(s) => s,
            Err(e) => {
                log::debug!("loader: agent settings not readable ({e}); runtimes stay as projected");
                continue;
            }
        };
        if current.runtimes != last.runtimes || current.default_runtime_id != last.default_runtime_id {
            log::info!("loader: runtime settings changed; re-projecting runtimes");
            project_runtimes(agent.as_ref(), &agent_root, &current, &who).await;
            tokio::spawn(check_runtimes(agent.clone()));
            last = current;
        }
    }
}

/// The runtime info tells the zone the agent is loaded (and from which
/// template version); it is written until it is accepted.
async fn report_loaded(records: Arc<dyn AgentRecords>, info: AgentRuntimeInfo, status: Arc<LoaderInfo>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match records::set_info(records.as_ref(), &info).await {
            Ok(()) => {
                log::info!(
                    "loader: runtime info written (generation {}, template {})",
                    info.generation,
                    info.template_version.as_deref().unwrap_or("unknown")
                );
                return;
            }
            Err(e) => {
                log::warn!("loader: runtime info not written: {e}");
                status.error("info", e);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_workspace_roots_include_its_owners_home() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("agents/xiaobai.test.buckyos.io");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(container_roots(&root), vec![root.join("workspaces"), home.path().to_path_buf()]);
        let dev_root = home.path().join("dev-agent");
        assert_eq!(container_roots(&dev_root), vec![dev_root.join("workspaces")]);
    }
}
