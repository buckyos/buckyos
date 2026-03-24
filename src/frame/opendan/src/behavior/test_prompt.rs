use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use buckyos_api::{MsgRecordWithObject, OpenDanAgentSessionRecord};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value as Json};
use tempfile::{tempdir, TempDir};
use tokio::fs;
use tokio::sync::Mutex;

use super::config::BehaviorMemoryBucketConfig;
use super::prompt::render_complete_request_prompt;
use super::{
    BehaviorConfig, BehaviorExecInput, BehaviorLLMResult, PromptBuilder, SessionRuntimeContext,
    StepLimits, Tokenizer,
};
use crate::agent_session::AgentSession;
use crate::agent_tool::{
    AgentTool, AgentToolResult, FileToolConfig, ReadFileTool,
    SessionRuntimeContext as ToolSessionRuntimeContext, TodoTool, TodoToolConfig,
};
use crate::step_record::LLMStepRecord;
use crate::workspace::{WorkshopIndex, WorkshopWorkspaceRecord, WorkspaceType};
use crate::workspace_path::WORKSHOP_INDEX_FILE_NAME;

struct MockTokenizer;

impl Tokenizer for MockTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

struct PromptFixture {
    _temp: TempDir,
    agent_env_root: PathBuf,
    sessions_root: PathBuf,
    workspace_id: String,
    workspace_dir: PathBuf,
    cwd_dir: PathBuf,
}

impl PromptFixture {
    async fn new() -> Self {
        let temp = tempdir().expect("create temp dir");
        let agent_env_root = temp.path().join("agent-env");
        let sessions_root = agent_env_root.join("sessions");
        let workspace_id = "ws-rich".to_string();
        let workspace_dir = agent_env_root.join("workspaces").join(&workspace_id);
        let cwd_dir = workspace_dir.join("active");

        fs::create_dir_all(&sessions_root)
            .await
            .expect("create sessions root");
        fs::create_dir_all(workspace_dir.join("notes"))
            .await
            .expect("create workspace note dir");
        fs::create_dir_all(cwd_dir.join("docs"))
            .await
            .expect("create cwd doc dir");
        fs::create_dir_all(agent_env_root.join("todo"))
            .await
            .expect("create todo dir");

        fs::write(
            agent_env_root.join("agent-guide.md"),
            "# Agent Guide\n\n- prefer explicit assertions\n- keep prompts inspectable\n",
        )
        .await
        .expect("write agent guide");
        fs::write(
            workspace_dir.join("notes").join("context.md"),
            "Workspace note: validate prompt rendering against realistic workspace state.\n",
        )
        .await
        .expect("write workspace note");
        fs::write(
            cwd_dir.join("docs").join("cwd-note.md"),
            "Cwd note: the active execution directory is scoped to this workspace.\n",
        )
        .await
        .expect("write cwd note");

        seed_recent_workspaces(&agent_env_root, &workspace_id).await;
        seed_todo_db(&agent_env_root, &workspace_id).await;

        Self {
            _temp: temp,
            agent_env_root,
            sessions_root,
            workspace_id,
            workspace_dir,
            cwd_dir,
        }
    }

    fn workspace_info(&self, current_todo: &str) -> Json {
        let binding = json!({
            "local_workspace_id": self.workspace_id,
            "workspace_path": self.workspace_dir,
            "workspace_rel_path": format!("workspaces/{}", self.workspace_id),
            "agent_env_root": self.agent_env_root,
        });
        let project = json!({
            "name": "BuckyOS Prompt Fixture",
            "repo": "buckyos",
            "lane": "integration-test",
        });
        json!({
            "binding": binding.clone(),
            "project": project.clone(),
            "current_todo": current_todo,
            "workspace": {
                "binding": binding,
                "project": project,
                "current_todo": current_todo,
            },
        })
    }

    async fn seed_recent_sessions(&self, current_session_id: &str) {
        let sessions = [
            (
                "work-alpha",
                "Alpha Session",
                "alpha summary",
                1_710_000_010_000_u64,
            ),
            (
                "work-beta",
                "Beta Session",
                "beta summary",
                1_710_000_020_000_u64,
            ),
            (
                "work-gamma",
                "Gamma Session",
                "gamma summary",
                1_710_000_030_000_u64,
            ),
            (
                current_session_id,
                "Current Session",
                "current summary",
                1_710_000_040_000_u64,
            ),
            ("ui-noise", "UI Noise", "hidden", 1_710_000_050_000_u64),
        ];

        for (session_id, title, summary, last_activity_ms) in sessions {
            write_session_record(
                &self.sessions_root,
                session_id,
                title,
                summary,
                last_activity_ms,
            )
            .await;
        }
    }
}

fn rich_behavior_config() -> BehaviorConfig {
    let mut cfg = BehaviorConfig {
        system: r#"
__OPENDAN_VAR(ctx_session_id, $session_id)
__OPENDAN_VAR(ctx_loop_session_id, $loop.session_id)
__OPENDAN_VAR(ctx_trace_step, $step.index)
__OPENDAN_VAR(session_title, $session_title)
__OPENDAN_VAR(session_step_index, $step_index)
__OPENDAN_VAR(session_step_num, $step_num)
__OPENDAN_VAR(current_behavior, $current_behavior)
__OPENDAN_VAR(last_step, $last_step)
__OPENDAN_VAR(workspace_id, $workspace.binding.local_workspace_id)
__OPENDAN_VAR(workspace_rel_path, $workspace.binding.workspace_rel_path)
__OPENDAN_VAR(project_name, $workspace.project.name)
__OPENDAN_VAR(current_todo_id, $workspace_current_todo_id)
__OPENDAN_VAR(current_todo_detail, $workspace_current_todo)
__OPENDAN_VAR(todo_t001, $workspace.todolist.T001)
__OPENDAN_VAR(session_list, $session_list.$3)
__OPENDAN_VAR(local_workspace_list, $local_workspace_list.$2)
__OPENDAN_VAR(last_steps, $last_steps.$3)
__OPENDAN_VAR(last_step_record, $step_record.last)
__OPENDAN_VAR(step_record_path, $step_record.path)
__OPENDAN_VAR(workspace_todolist, $workspace_todolist)
Context Session={{ctx_session_id}} Loop={{ctx_loop_session_id}} TraceStep={{ctx_trace_step}}
Session Title={{session_title}} StepIndex={{session_step_index}} StepNum={{session_step_num}} Behavior={{current_behavior}}
Last Step={{last_step}}
Workspace={{workspace_id}} Rel={{workspace_rel_path}} Project={{project_name}}
Current Todo Id={{current_todo_id}}
Current Todo:
{{current_todo_detail}}
Todo T001:
{{todo_t001}}
Recent Sessions:
{{session_list}}
Recent Workspaces:
{{local_workspace_list}}
Last Steps Snapshot:
{{last_steps}}
Last Step Record:
{{last_step_record}}
Step Record Path={{step_record_path}}
Agent Guide:
__OPENDAN_CONTENT($agent_root/agent-guide.md)__
Session Summary:
__OPENDAN_CONTENT($session_root/summary.md)__
Workspace Note:
__OPENDAN_CONTENT($workspace/notes/context.md)__
Cwd Note:
__OPENDAN_CONTENT($cwd/docs/cwd-note.md)__
Workspace Todo List:
{{workspace_todolist}}
"#
        .trim()
        .to_string(),
        ..Default::default()
    };

    cfg.memory.total_limit = 20_000;
    cfg.memory.history_messages = BehaviorMemoryBucketConfig {
        limit: 12_000,
        max_percent: Some(1.0),
        is_enable: true,
        skip_last_n: None,
    };
    cfg.memory.session_step_records = BehaviorMemoryBucketConfig {
        limit: 12_000,
        max_percent: Some(1.0),
        is_enable: true,
        skip_last_n: None,
    };
    cfg.memory.first_prompt = Some(
        "__OPENDAN_VAR(head_session, $session_id)\n__OPENDAN_VAR(head_todo, $workspace_current_todo_id)\nMEMORY_HEAD session={{head_session}} todo={{head_todo}}"
            .to_string(),
    );
    cfg.memory.last_prompt = Some(
        "__OPENDAN_VAR(tail_workspace, $workspace.binding.local_workspace_id)\nMEMORY_TAIL workspace={{tail_workspace}}"
            .to_string(),
    );
    cfg
}

fn compact_history_compression_config() -> BehaviorConfig {
    let mut cfg = BehaviorConfig {
        system: "compression demo: history".to_string(),
        ..Default::default()
    };
    cfg.limits.max_prompt_tokens = 4096;
    cfg.memory.total_limit = 48;
    cfg.memory.history_messages = BehaviorMemoryBucketConfig {
        limit: 48,
        max_percent: Some(1.0),
        is_enable: true,
        skip_last_n: None,
    };
    cfg.memory.first_prompt = Some("HISTORY_HEAD".to_string());
    cfg.memory.last_prompt = Some("HISTORY_TAIL".to_string());
    cfg
}

fn compact_step_record_compression_config() -> BehaviorConfig {
    let mut cfg = BehaviorConfig {
        system: "compression demo: step-record".to_string(),
        ..Default::default()
    };
    cfg.limits.max_prompt_tokens = 4096;
    cfg.memory.total_limit = 80;
    cfg.memory.session_step_records = BehaviorMemoryBucketConfig {
        limit: 80,
        max_percent: Some(1.0),
        is_enable: true,
        skip_last_n: None,
    };
    cfg.memory.first_prompt = Some("STEP_HEAD".to_string());
    cfg.memory.last_prompt = Some("STEP_TAIL".to_string());
    cfg
}

fn rich_role_md() -> String {
    r#"
Role Preamble
__OPENDAN_VAR(role_session_title, $session_title)
__OPENDAN_VAR(role_behavior, $behavior_name)
Role session={{role_session_title}} behavior={{role_behavior}}
"#
    .trim()
    .to_string()
}

fn rich_self_md() -> String {
    r#"
Self Preamble
__OPENDAN_VAR(self_workspace, $workspace.binding.local_workspace_id)
__OPENDAN_VAR(self_project, $workspace.project.name)
Self workspace={{self_workspace}} project={{self_project}}
"#
    .trim()
    .to_string()
}

fn build_input(
    session_id: &str,
    trace_step_idx: u32,
    behavior_cfg: BehaviorConfig,
    session: Arc<Mutex<AgentSession>>,
) -> BehaviorExecInput {
    BehaviorExecInput {
        session_id: session_id.to_string(),
        trace: SessionRuntimeContext {
            trace_id: format!("trace-{session_id}"),
            agent_name: "did:web:agent.example.com".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: trace_step_idx,
            wakeup_id: format!("wakeup-{session_id}"),
            session_id: session_id.to_string(),
        },
        input_prompt: format!("build prompt for {session_id}"),
        last_step_prompt: "last step prompt placeholder".to_string(),
        role_md: rich_role_md(),
        self_md: rich_self_md(),
        behavior_prompt: "rich behavior prompt".to_string(),
        limits: StepLimits::default(),
        behavior_cfg,
        session: Some(session),
    }
}

fn build_compact_input(
    session_id: &str,
    trace_step_idx: u32,
    behavior_cfg: BehaviorConfig,
    session: Arc<Mutex<AgentSession>>,
) -> BehaviorExecInput {
    BehaviorExecInput {
        session_id: session_id.to_string(),
        trace: SessionRuntimeContext {
            trace_id: format!("trace-{session_id}-compact"),
            agent_name: "did:web:agent.example.com".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: trace_step_idx,
            wakeup_id: format!("wakeup-{session_id}-compact"),
            session_id: session_id.to_string(),
        },
        input_prompt: "compress".to_string(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        behavior_prompt: "compact".to_string(),
        limits: StepLimits::default(),
        behavior_cfg,
        session: Some(session),
    }
}

async fn write_session_record(
    sessions_root: &Path,
    session_id: &str,
    title: &str,
    summary: &str,
    last_activity_ms: u64,
) {
    let session_dir = sessions_root.join(session_id);
    fs::create_dir_all(&session_dir)
        .await
        .expect("create session dir");

    let record = OpenDanAgentSessionRecord {
        session_id: session_id.to_string(),
        owner_agent: "did:web:agent.example.com".to_string(),
        title: title.to_string(),
        summary: summary.to_string(),
        status: "normal".to_string(),
        created_at_ms: last_activity_ms.saturating_sub(1000),
        updated_at_ms: last_activity_ms.saturating_sub(100),
        last_activity_ms,
        links: vec![],
        tags: vec![],
        meta: Json::Object(Map::new()),
    };
    let payload = serde_json::to_vec_pretty(&record).expect("serialize session record");
    fs::write(session_dir.join("session.json"), payload)
        .await
        .expect("write session record");
}

async fn seed_recent_workspaces(agent_env_root: &Path, current_workspace_id: &str) {
    let mut ws_alpha = WorkshopWorkspaceRecord::default();
    ws_alpha.workspace_id = "local-alpha".to_string();
    ws_alpha.workspace_type = WorkspaceType::Local;
    ws_alpha.name = "Alpha Workspace".to_string();
    ws_alpha.created_at_ms = 10;
    ws_alpha.updated_at_ms = 100;

    let mut ws_beta = WorkshopWorkspaceRecord::default();
    ws_beta.workspace_id = current_workspace_id.to_string();
    ws_beta.workspace_type = WorkspaceType::Local;
    ws_beta.name = "Rich Workspace".to_string();
    ws_beta.created_at_ms = 20;
    ws_beta.updated_at_ms = 400;

    let mut ws_gamma = WorkshopWorkspaceRecord::default();
    ws_gamma.workspace_id = "local-gamma".to_string();
    ws_gamma.workspace_type = WorkspaceType::Local;
    ws_gamma.name = "Gamma Workspace".to_string();
    ws_gamma.created_at_ms = 30;
    ws_gamma.updated_at_ms = 300;

    let index = WorkshopIndex {
        agent_did: "did:web:agent.example.com".to_string(),
        workspaces: vec![ws_alpha, ws_beta, ws_gamma],
        updated_at_ms: 400,
    };

    fs::write(
        agent_env_root.join(WORKSHOP_INDEX_FILE_NAME),
        serde_json::to_vec_pretty(&index).expect("serialize workshop index"),
    )
    .await
    .expect("write workshop index");
}

async fn seed_todo_db(agent_env_root: &Path, workspace_id: &str) {
    let todo_db_path = agent_env_root.join("todo").join("todo.db");
    let _tool = TodoTool::new(TodoToolConfig::with_db_path(todo_db_path.clone()))
        .expect("initialize todo schema");

    let conn = Connection::open(todo_db_path).expect("open todo db");
    conn.execute(
        "INSERT INTO todo_meta(key, value) VALUES (?1, ?2)",
        params![format!("version:{workspace_id}"), "7"],
    )
    .expect("insert todo version");

    let items = [
        (
            "todo-1",
            "ui-rich-history",
            "T001",
            "Audit prompt rendering",
            "WAIT",
            1_710_100_001_i64,
        ),
        (
            "todo-2",
            "ui-rich-history",
            "T002",
            "Summarize history timeline",
            "WAIT",
            1_710_100_002_i64,
        ),
        (
            "todo-3",
            "work-rich-step-records",
            "T101",
            "Review step records",
            "WAIT",
            1_710_100_003_i64,
        ),
        (
            "todo-4",
            "work-rich-step-records",
            "T102",
            "Compare last step snapshot",
            "WAIT",
            1_710_100_004_i64,
        ),
    ];

    for (id, session_id, todo_code, title, status, updated_at) in items {
        conn.execute(
            "INSERT INTO todo_items(
                id, workspace_id, session_id, todo_code, title, type, status,
                assignee_did, created_at, updated_at, created_by_kind, created_by_did
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                workspace_id,
                session_id,
                todo_code,
                title,
                "Task",
                status,
                "did:web:agent.example.com",
                updated_at - 100,
                updated_at,
                "root_agent",
                "did:web:agent.example.com",
            ],
        )
        .expect("insert todo item");
    }
}

async fn create_ui_session(fixture: &PromptFixture, session_id: &str) -> Arc<Mutex<AgentSession>> {
    fixture.seed_recent_sessions(session_id).await;
    let session_dir = fixture.sessions_root.join(session_id);
    fs::write(
        session_dir.join("summary.md"),
        "# UI Session Summary\n\n- collected dense chat history\n- prepared prompt fixture\n",
    )
    .await
    .expect("write ui summary");
    write_history_messages(&session_dir, 28).await;

    let mut session = AgentSession::new(
        session_id,
        "did:web:agent.example.com",
        Some("resolve_router"),
    );
    session.title = "Dense UI Session".to_string();
    session.current_behavior = "resolve_router".to_string();
    session.step_index = 9;
    session.step_num = 14;
    session.pwd = fixture.cwd_dir.clone();
    session.session_root_dir = fixture.sessions_root.clone();
    session.local_workspace_id = Some(fixture.workspace_id.clone());
    session.workspace_info = Some(fixture.workspace_info("T001"));

    Arc::new(Mutex::new(session))
}

async fn write_history_messages(session_dir: &Path, total: usize) {
    let mut lines = Vec::with_capacity(total);
    let base_ts = 1_772_578_920_000_u64;
    for idx in 0..total {
        let created_at_ms = base_ts + (idx as u64) * 60_000;
        let is_agent = idx % 3 == 2;
        let (from, box_kind, state, content) = if is_agent {
            (
                "did:web:agent.example.com",
                "OUTBOX",
                "SENT",
                format!(
                    "history item {:02} from agent\nfollow-up details {}\nclosing note {}",
                    idx + 1,
                    idx + 1,
                    idx + 1
                ),
            )
        } else {
            (
                "did:web:alice.example.com",
                "INBOX",
                "UNREAD",
                format!(
                    "history item {:02} from user requesting richer prompt coverage {}",
                    idx + 1,
                    idx + 1
                ),
            )
        };

        let line = json!({
            "record": {
                "record_id": format!("r{}", idx + 1),
                "box_kind": box_kind,
                "msg_id": format!("sha256:{:032x}", idx + 1),
                "msg_kind": "chat",
                "state": state,
                "from": from,
                "to": "did:web:agent.example.com",
                "created_at_ms": created_at_ms,
                "updated_at_ms": created_at_ms,
                "sort_key": created_at_ms,
                "tags": [],
            },
            "msg": {
                "from": from,
                "to": ["did:web:agent.example.com"],
                "kind": "chat",
                "created_at_ms": created_at_ms,
                "content": {
                    "content": content,
                },
            },
        });
        let raw = line.to_string();
        serde_json::from_str::<MsgRecordWithObject>(&raw).expect("history line should parse");
        lines.push(raw);
    }

    fs::write(
        session_dir.join("msg_record.jsonl"),
        lines.join("\n") + "\n",
    )
    .await
    .expect("write history jsonl");
}

async fn write_wordy_history_messages(session_dir: &Path, total: usize) {
    let mut lines = Vec::with_capacity(total);
    let base_ts = 1_772_578_920_000_u64;
    for idx in 0..total {
        let created_at_ms = base_ts + (idx as u64) * 60_000;
        let content = format!(
            "history item {:02} carries a deliberately long narrative for compression demonstration with token budget pressure repeated repeated repeated repeated repeated {}",
            idx + 1,
            idx + 1
        );
        let line = json!({
            "record": {
                "record_id": format!("rw{}", idx + 1),
                "box_kind": "INBOX",
                "msg_id": format!("sha256:{:032x}", 10_000 + idx + 1),
                "msg_kind": "chat",
                "state": "UNREAD",
                "from": "did:web:alice.example.com",
                "to": "did:web:agent.example.com",
                "created_at_ms": created_at_ms,
                "updated_at_ms": created_at_ms,
                "sort_key": created_at_ms,
                "tags": [],
            },
            "msg": {
                "from": "did:web:alice.example.com",
                "to": ["did:web:agent.example.com"],
                "kind": "chat",
                "created_at_ms": created_at_ms,
                "content": {
                    "content": content,
                },
            },
        });
        let raw = line.to_string();
        serde_json::from_str::<MsgRecordWithObject>(&raw).expect("wordy history line should parse");
        lines.push(raw);
    }

    fs::write(
        session_dir.join("msg_record.jsonl"),
        lines.join("\n") + "\n",
    )
    .await
    .expect("write wordy history jsonl");
}

async fn create_work_session(
    fixture: &PromptFixture,
    session_id: &str,
) -> Arc<Mutex<AgentSession>> {
    fixture.seed_recent_sessions(session_id).await;
    let session_dir = fixture.sessions_root.join(session_id);
    fs::write(
        session_dir.join("summary.md"),
        "# Work Session Summary\n\n- appended dense llm step records\n- prepared env-context heavy prompt\n",
    )
    .await
    .expect("write work summary");

    let mut session = AgentSession::new(session_id, "did:web:agent.example.com", Some("plan"));
    session.title = "Dense Work Session".to_string();
    session.current_behavior = "plan".to_string();
    session.step_index = 12;
    session.step_num = 18;
    session.pwd = fixture.cwd_dir.clone();
    session.session_root_dir = fixture.sessions_root.clone();
    session.local_workspace_id = Some(fixture.workspace_id.clone());
    session.workspace_info = Some(fixture.workspace_info("T101"));

    for idx in 1..=14_u32 {
        let behavior_name = match idx % 3 {
            0 => "check",
            1 => "plan",
            _ => "do",
        };
        let action_result = if idx >= 12 {
            build_large_read_file_results(fixture, idx).await
        } else {
            HashMap::new()
        };
        session
            .append_llm_step_record(LLMStepRecord {
                session_id: session_id.to_string(),
                step_num: idx,
                step_index: idx,
                behavior_name: behavior_name.to_string(),
                input: format!("step input {idx}: verify prompt build coverage"),
                llm_result: BehaviorLLMResult {
                    conclusion: Some(format!(
                        "step {idx} conclusion keeps the prompt builder on the happy path"
                    )),
                    thinking: Some(format!(
                        "step {idx} thinking enumerates env-context values and record density"
                    )),
                    reply: Some(format!("step {idx} reply placeholder")),
                    next_behavior: Some(if idx < 14 {
                        "do".to_string()
                    } else {
                        "END".to_string()
                    }),
                    ..Default::default()
                },
                action_result,
                ..Default::default()
            })
            .await
            .expect("append llm step record");
    }

    Arc::new(Mutex::new(session))
}

async fn build_large_read_file_results(
    fixture: &PromptFixture,
    step_idx: u32,
) -> HashMap<String, AgentToolResult> {
    let tool = ReadFileTool::new(FileToolConfig::new(fixture.cwd_dir.clone()));
    let ctx = ToolSessionRuntimeContext {
        trace_id: format!("prompt-read-trace-{step_idx}"),
        agent_name: "did:web:agent.example.com".to_string(),
        behavior: "test_prompt".to_string(),
        step_idx,
        wakeup_id: format!("prompt-read-wakeup-{step_idx}"),
        session_id: format!("prompt-read-session-{step_idx}"),
    };
    let mut results = HashMap::new();
    for file_idx in 0..6_u32 {
        let path = format!("docs/context-{step_idx:02}-{file_idx:02}.md");
        let mut body = String::new();
        for line_idx in 1..=40_u32 {
            body.push_str(
                format!(
                    "{path} line {line_idx:02}: large read_file payload for prompt compression walkthrough step {step_idx}\n"
                )
                .as_str(),
            );
        }
        fs::write(fixture.cwd_dir.join(&path), &body)
            .await
            .expect("write realistic read_file payload");

        let result = tool
            .call(
                &ctx,
                json!({
                    "path": path,
                    "range": "1-40"
                }),
            )
            .await
            .expect("call read_file tool");
        results.insert(format!("read_file_{file_idx:02}"), result);
    }
    results
}

#[tokio::test]
async fn build_prompt_for_ui_session_renders_dense_history_and_env_context() {
    let fixture = PromptFixture::new().await;
    let session_id = "ui-rich-history";
    let session = create_ui_session(&fixture, session_id).await;
    let behavior_cfg = rich_behavior_config();
    let input = build_input(session_id, 10, behavior_cfg.clone(), session);

    let req = PromptBuilder::build(
        &input,
        &behavior_cfg,
        &MockTokenizer,
        input.session.clone(),
        None,
    )
    .await
    .expect("build ui prompt");

    let rendered = render_complete_request_prompt(&req);
    println!("\n[ui rich prompt]\n{rendered}\n");

    assert!(rendered.contains("Context Session=ui-rich-history"));
    assert!(rendered.contains("Loop=ui-rich-history"));
    assert!(rendered.contains("TraceStep=10"));
    assert!(rendered.contains("Session Title=Dense UI Session"));
    assert!(rendered.contains("Behavior=resolve_router"));
    assert!(rendered.contains("Workspace=ws-rich"));
    assert!(rendered.contains("Project=BuckyOS Prompt Fixture"));
    assert!(rendered.contains("Current Todo Id=T002"));
    assert!(rendered.contains("Current Todo T001 [WAIT]"));
    assert!(rendered.contains("Workspace Todo (ws-rich, v7)"));
    assert!(rendered.contains("Recent Sessions:"));
    assert!(rendered.contains("- work-gamma : Gamma Session"));
    assert!(rendered.contains("- work-beta : Beta Session"));
    assert!(rendered.contains("Recent Workspaces:"));
    assert!(rendered.contains("$ws-rich"));
    assert!(rendered.contains("Agent Guide:"));
    assert!(rendered.contains("prefer explicit assertions"));
    assert!(rendered.contains("Session Summary:"));
    assert!(rendered.contains("collected dense chat history"));
    assert!(rendered.contains("Workspace Note:"));
    assert!(rendered.contains("Workspace note: validate prompt rendering"));
    assert!(rendered.contains("Cwd Note:"));
    assert!(rendered.contains("active execution directory is scoped"));
    assert!(rendered.contains("MEMORY_HEAD session=ui-rich-history todo=T002"));
    assert!(rendered.contains("MEMORY_TAIL workspace=ws-rich"));
    assert!(rendered.contains("## Timeline"));
    assert!(rendered.contains("history item 01 from user requesting richer prompt coverage 1"));
    assert!(rendered.contains("history item 15 from agent"));
    assert!(rendered.contains("history item 28 from user requesting richer prompt coverage 28"));
}

#[tokio::test]
async fn build_prompt_for_work_session_renders_dense_step_records_and_env_context() {
    let fixture = PromptFixture::new().await;
    let session_id = "work-rich-step-records";
    let session = create_work_session(&fixture, session_id).await;
    let behavior_cfg = rich_behavior_config();
    let input = build_input(session_id, 15, behavior_cfg.clone(), session);

    let req = PromptBuilder::build(
        &input,
        &behavior_cfg,
        &MockTokenizer,
        input.session.clone(),
        None,
    )
    .await
    .expect("build work prompt");

    let rendered = render_complete_request_prompt(&req);
    println!("\n[work rich prompt]\n{rendered}\n");

    assert!(rendered.contains("Context Session=work-rich-step-records"));
    assert!(rendered.contains("TraceStep=15"));
    assert!(rendered.contains("Session Title=Dense Work Session"));
    assert!(rendered.contains("Behavior=plan"));
    assert!(rendered.contains("Last Step="));
    assert!(rendered.contains("Current Todo Id=T102"));
    assert!(rendered.contains("Current Todo T102 [WAIT]"));
    assert!(rendered.contains("Step Record Path="));
    assert!(rendered.contains("llm_step_record.jsonl"));
    assert!(rendered.contains("Recent Sessions:"));
    assert!(rendered.contains("- work-rich-step-records : Current Session"));
    assert!(rendered.contains("Workspace Todo (ws-rich, v7)"));
    assert!(rendered.contains("Last Steps Snapshot:"));
    assert!(rendered.contains("<step behavior=\"check\" step_num=12 step_time=\""));
    assert!(rendered.contains("<step behavior=\"plan\" step_num=13 step_time=\""));
    assert!(rendered.contains("<step behavior=\"do\" step_num=14 step_time=\""));
    assert!(rendered.contains("Last Step Record:"));
    assert!(rendered.contains("step 14 conclusion keeps the prompt builder"));
    assert!(rendered.contains("<steps_summary>"));
    assert!(rendered.contains("<step behavior=\"plan\" step_num=1 step_time=\""));
    assert!(rendered.contains("<step behavior=\"plan\" step_num=7 step_time=\""));
    assert!(rendered.contains("<step behavior=\"do\" step_num=14 step_time=\""));
    assert!(rendered.contains("step 14 thinking enumerates env-context values"));
    assert!(rendered.contains("Agent Guide:"));
    assert!(rendered.contains("Session Summary:"));
    assert!(rendered.contains("dense llm step records"));
    assert!(rendered.contains("MEMORY_HEAD session=work-rich-step-records todo=T102"));
    assert!(rendered.contains("MEMORY_TAIL workspace=ws-rich"));
}

#[tokio::test]
async fn build_prompt_shows_history_compression_when_token_budget_is_small() {
    let fixture = PromptFixture::new().await;
    let session_id = "ui-rich-history";
    let session = create_ui_session(&fixture, session_id).await;
    write_wordy_history_messages(&fixture.sessions_root.join(session_id), 12).await;

    let behavior_cfg = compact_history_compression_config();
    let input = build_compact_input(session_id, 10, behavior_cfg.clone(), session);

    let req = PromptBuilder::build(
        &input,
        &behavior_cfg,
        &MockTokenizer,
        input.session.clone(),
        None,
    )
    .await
    .expect("build compressed history prompt");

    let rendered = render_complete_request_prompt(&req);
    println!("\n[history compressed prompt]\n{rendered}\n");

    assert!(rendered.contains("HISTORY_HEAD"));
    assert!(rendered.contains("HISTORY_TAIL"));
    assert!(rendered.contains("history item 12 carries a deliberately long narrative"));
    assert!(!rendered.contains("history item 11 carries a deliberately long narrative"));
    assert!(!rendered.contains("history item 01 carries a deliberately long narrative"));
}

#[tokio::test]
async fn build_prompt_shows_step_record_truncation_when_token_budget_is_small() {
    let fixture = PromptFixture::new().await;
    let session_id = "work-rich-step-records";
    let session = create_work_session(&fixture, session_id).await;

    let behavior_cfg = compact_step_record_compression_config();
    let input = build_compact_input(session_id, 15, behavior_cfg.clone(), session);

    let req = PromptBuilder::build(
        &input,
        &behavior_cfg,
        &MockTokenizer,
        input.session.clone(),
        None,
    )
    .await
    .expect("build compressed step-record prompt");

    let rendered = render_complete_request_prompt(&req);
    println!("\n[step-record compressed prompt]\n{rendered}\n");

    assert!(rendered.contains("STEP_HEAD"));
    assert!(rendered.contains("<steps_summary>"));
    assert!(rendered.contains("<step behavior=\"plan\" step_num=1 step_time=\""));
    assert!(rendered.contains("[TRUNCATED]"));
    assert!(!rendered.contains("<step behavior=\"do\" step_num=14 step_time=\""));
}
