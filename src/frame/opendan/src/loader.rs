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
use libopendan::host::{HostDeps, Supervisor};
use libopendan::runner::{BehaviorAssembler, OutboundSink, RunnerOptions};
use libopendan::state::{register_in_process, unregister_in_process, AgentStateClient, FsAgentStateClient};
use name_lib::DID;
use ndn_lib::ObjId;

use crate::config::AgentConfig;
use crate::home::Home;
use crate::records::{self, AgentRecords};
use crate::rootfs::{self, RootIdentity};
use crate::service::{Access, LoaderInfo, ModuleStatus, StateService, SERVICE_PATH};
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
    /// Version of the package at `package_root` (the agent's template).
    pub package_version: Option<String>,
    pub channels: Arc<KmsgChannels>,
    pub waker: Arc<dyn Waker>,
    pub kevent: Option<Arc<KEventClient>>,
    /// `None`: no msg-center (development host); ui rules stay idle.
    pub mail: Option<Arc<dyn MailService>>,
    /// `None`: no TaskMgr (development host); Turns get no task.
    pub tasks: Option<Arc<dyn TaskService>>,
    pub access: Access,
    pub port: u16,
    pub web_dir: Option<PathBuf>,
    pub xllm: XllmDeps,
    /// The `xagent` executable, wrapped as `agent-session` in every session.
    pub session_cli: Option<PathBuf>,
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
        let instance_role = display_name
            .map(|(name, account)| format!("Your name is {name} (account `{account}`)."))
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
        let agent: Arc<dyn AgentStateClient> = Arc::new(fs);
        register_in_process(agent.clone());

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
        let host = HostDeps {
            who: env.who.clone(),
            agent: agent.clone(),
            inputs: env.channels.clone(),
            waker: env.waker.clone(),
            xllm: env.xllm.clone(),
            assembler: Arc::new(BehaviorAssembler::default()),
            session_cli: env.session_cli.clone(),
            app_tools: Vec::new(),
            options,
            max_child_concurrency: 4,
            bridges,
            outbound,
            turn_tasks: env.tasks.clone().map(|tasks| {
                Arc::new(TurnTasks {
                    tasks,
                    owner: owner_of(&env.who),
                }) as Arc<dyn libopendan::runner::TurnTaskSink>
            }),
            runtime_id: None,
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
        for (name, switch) in [
            ("self_check", &config.loader.self_check),
            ("self_improve", &config.loader.self_improve),
        ] {
            module(
                name,
                switch.enabled,
                false,
                switch.enabled.then_some("not available in this version"),
            );
        }

        if config.loader.agent_state_service || config.loader.webui {
            let runner = Runner::new(env.port);
            if config.loader.agent_state_service {
                let service = Arc::new(StateService {
                    agent: agent.clone(),
                    supervisor: supervisor.clone(),
                    info: info.clone(),
                    ui: ui.clone(),
                    access: env.access.clone(),
                    home: Home::new(
                        env.records.clone(),
                        env.owner_did.as_ref().map(|d| d.to_string()),
                        match &env.access {
                            Access::Zone { zone_host, .. } => zone_host.as_ref().map(|h| format!("https://{h}")),
                            Access::Open { .. } => None,
                        },
                    ),
                });
                runner
                    .add_http_server(SERVICE_PATH.to_string(), service.clone())
                    .map_err(|e| anyhow!("mount {SERVICE_PATH}: {e:?}"))?;
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
        if let Some(ui) = &ui {
            tasks.push(tokio::spawn(
                ui.clone().run(
                    env.waker.clone(),
                    env.options.poll_interval.min(Duration::from_secs(5)),
                ),
            ));
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
