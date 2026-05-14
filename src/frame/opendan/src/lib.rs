// opendan crate — see notepads/NewOpenDANRuntime.md
// Module layout follows §9 checklist order: bottom-up dependencies first.

// §9 step 2 — LLMContextDeps assembly (LlmClient / ToolManager / PolicyEngine /
//             WorklogSink / TurnHook adapters over aicc + agent_tool).
pub mod ai_runtime;

// §9 step 3 — config layer.
//   agent_config: agent-level config (agent.toml, default behavior, subscribed
//                 event types, etc.).
//   behavior_cfg: single-behavior TOML parsing (BehaviorCfg, tool whitelist,
//                 parser/renderer choice, switch_mode, ...).
pub mod agent_config;
pub mod behavior_cfg;

// §9 step 4 — AgentSession worker loop, build_or_resume_context, handle_outcome,
//             switch_behavior (normal / fork / independent).
pub mod agent_session;

// §9 step 5 — UI-session default tool wiring; exec_bash + session /bin scripts.
pub mod agent_bash;

// §9 step 6 — AIAgent::run, msg/event dispatch, session restoration, subscriptions.
pub mod agent;

// §9 step 6 — msg-center / kevent inbound pump that feeds AIAgent::inbox().
pub mod msg_center_pump;

// §9 step 7 — workspace data model (BehaviorLoop deps stripped; session binding
//             owned by AgentSession).
pub mod local_workspace;

// Worklog SQLite service (unchanged from beta2.x — consumed by ai_runtime's
// OpenDanWorklogSink).
pub mod worklog;

// Placeholder for future builtin-tool wiring (Agent/Session bin layers in the
// 4-layer AgentToolManager). Currently empty; will be populated alongside
// agent_bash / ai_runtime.
pub mod buildin_tool;
