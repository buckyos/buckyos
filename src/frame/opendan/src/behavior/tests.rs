use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{
    AiResponseSummary, AiUsage, AiccClient, CompleteRequest, CompleteResponse, CompleteStatus,
    Task, TaskManagerClient, TaskPermissions, TaskStatus,
};
use rusqlite::Connection;
use serde_json::{json, Value as Json};
use tempfile::tempdir;
use tokio::fs;

use super::*;
use crate::agent_memory::{AgentMemory, AgentMemoryConfig, TOOL_LOAD_MEMORY, TOOL_LOAD_THINGS};
use crate::agent_tool::{AgentTool, ToolCall, ToolManager, ToolSpec};
use crate::test_utils::{MockAicc, MockTaskMgrHandler};
use crate::workspace::{AgentWorkshop, AgentWorkshopConfig, TOOL_EXEC_BASH};

struct MockTokenizer;

impl Tokenizer for MockTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

struct MockWorklog;

#[async_trait]
impl WorklogSink for MockWorklog {
    async fn emit(&self, _event: Event) {}
}

struct MockPolicy {
    tools: Vec<ToolSpec>,
}

#[async_trait]
impl PolicyEngine for MockPolicy {
    async fn allowed_tools(&self, _input: &BehaviorExecInput) -> Result<Vec<ToolSpec>, String> {
        Ok(self.tools.clone())
    }

    async fn gate_tool_calls(
        &self,
        _input: &BehaviorExecInput,
        calls: &[ToolCall],
    ) -> Result<Vec<ToolCall>, String> {
        Ok(calls.to_vec())
    }
}

struct EchoTool;

#[async_trait]
impl AgentTool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "tool.echo".to_string(),
            description: "echo".to_string(),
            args_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
        }
    }

    async fn call(
        &self,
        _ctx: &TraceCtx,
        args: Json,
    ) -> Result<Json, crate::agent_tool::ToolError> {
        println!("[TEST][TOOL] tool.echo called with args: {}", args);
        Ok(json!({"tool": "tool.echo", "ok": true, "args": args}))
    }
}

async fn load_behavior_config_yaml_for_test(behavior_name: &str, yaml: &str) -> BehaviorConfig {
    let tmp = tempdir().expect("create tempdir for behavior config");
    let behaviors_dir = tmp.path().join("behaviors");
    fs::create_dir_all(&behaviors_dir)
        .await
        .expect("create behaviors dir");
    fs::write(
        behaviors_dir.join(format!("{behavior_name}.yaml")),
        yaml.trim_start(),
    )
    .await
    .expect("write behavior config yaml");

    BehaviorConfig::load_from_dir(&behaviors_dir, behavior_name)
        .await
        .expect("load behavior config from yaml")
}

#[test]
fn parse_json_in_code_fence() {
    let raw = LLMRawResponse {
        content: "```json\n{\"is_sleep\":true,\"output\":{\"ok\":1}}\n```".to_string(),
        tool_calls: vec![],
        model: "m".to_string(),
        provider: "p".to_string(),
        latency_ms: 1,
    };

    let (parsed, output) =
        BehaviorResultParser::parse_first(&raw, true).expect("parse should succeed");
    assert_eq!(parsed.next_behavior.as_deref(), Some("END"));
    assert!(matches!(output, LLMOutput::Json(_)));
}

#[test]
fn parse_executor_result_payload() {
    let raw = LLMRawResponse {
        content: json!({
            "thinking": "collect evidence first",
            "reply": [{
                "audience": "user",
                "format": "markdown",
                "content": "Gather evidence first, then answer."
            }],
            "tool_calls": [{
                "name": "tool.echo",
                "args": {"msg":"hello"},
                "call_id": "call-1"
            }],
            "todo": [{"op":"add","item":{"title":"check"}}],
            "set_memory": [{"content":"x"}],
            "actions": [],
            "session_delta": [{"title":"T"}],
            "next_behavior": "END"
        })
        .to_string(),
        tool_calls: vec![],
        model: "m".to_string(),
        provider: "p".to_string(),
        latency_ms: 1,
    };

    let (parsed, _) =
        BehaviorResultParser::parse_first(&raw, true).expect("executor parse should succeed");
    assert_eq!(parsed.next_behavior.as_deref(), Some("END"));
    assert_eq!(parsed.tool_calls.len(), 1);
}

#[tokio::test]
async fn run_step_with_tool_followup() {
    let aicc_async_task_id = 9001_i64;
    let preloaded_tasks = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));
    preloaded_tasks.lock().expect("tasks lock").insert(
        aicc_async_task_id,
        Task {
            id: aicc_async_task_id,
            user_id: "did:example:agent".to_string(),
            app_id: "opendan-llm-behavior".to_string(),
            parent_id: None,
            root_id: String::new(),
            name: "aicc complete".to_string(),
            task_type: "aicc.complete".to_string(),
            status: TaskStatus::Completed,
            progress: 1.0,
            message: Some("done".to_string()),
            data: serde_json::to_value(CompleteResponse::new(
                "".to_string(),
                CompleteStatus::Succeeded,
                Some(AiResponseSummary {
                    text: None,
                    json: Some(json!({
                        "tool_calls": [{
                            "name": "tool.echo",
                            "args": {"msg": "hi"},
                            "call_id": "call-1"
                        }]
                    })),
                    artifacts: vec![],
                    usage: Some(AiUsage {
                        input_tokens: Some(10),
                        output_tokens: Some(5),
                        total_tokens: Some(15),
                    }),
                    cost: None,
                    finish_reason: Some("tool_calls".to_string()),
                    provider_task_ref: Some("provider-task-1".to_string()),
                    extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":10})),
                }),
                None,
            ))
            .expect("serialize complete response"),
            permissions: TaskPermissions::default(),
            created_at: 0,
            updated_at: 0,
        },
    );

    let responses = Arc::new(Mutex::new(VecDeque::from(vec![
        CompleteResponse::new(
            aicc_async_task_id.to_string(),
            CompleteStatus::Running,
            None,
            None,
        ),
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(
                    json!({"is_sleep":true,"next_behavior":"END","output":{"answer":"done"}}),
                ),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(4),
                    output_tokens: Some(6),
                    total_tokens: Some(10),
                }),
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: Some("provider-task-2".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":8})),
            }),
            None,
        ),
    ])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));

    let tool_mgr = Arc::new(ToolManager::new());
    tool_mgr
        .register_tool(EchoTool)
        .expect("register tool.echo should succeed");
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
tools:
  mode: allow_list
  names:
    - tool.echo
"#,
    )
    .await;

    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: preloaded_tasks.clone(),
            },
        ))),
        aicc,
        tools: tool_mgr,
        policy: Arc::new(MockPolicy {
            tools: behavior_cfg.tools.filter_tool_specs(&[ToolSpec {
                name: "tool.echo".to_string(),
                description: "echo".to_string(),
                args_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
            }]),
        }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: TraceCtx {
            trace_id: "trace-1".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-1".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (result, tracking) = behavior
        .run_step(input)
        .await
        .expect("run_step should succeed");
    println!(
        "[TEST][RUN_STEP] tool_followup result: next_behavior={:?} is_sleep={} actions={} usage_total={}",
        result.next_behavior,
        result.is_sleep(),
        result.actions.len(),
        tracking.token_usage.total
    );
    assert!(result.is_sleep());
    assert_eq!(result.next_behavior.as_deref(), Some("END"));
    assert_eq!(tracking.tool_trace.len(), 1);
    assert_eq!(tracking.token_usage.total, 25);

    let requests = requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 2);
    let tool_messages_len = requests[1]
        .payload
        .options
        .as_ref()
        .and_then(|v| v.get("tool_messages"))
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or(0);
    assert!(tool_messages_len >= 2);
}

#[tokio::test]
async fn run_step_resolves_prefixed_running_aicc_task_id_from_task_data() {
    let mapped_task_id = 9002_i64;
    let external_aicc_task_id = "aicc-1770927904938-99";
    let preloaded_tasks = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));
    preloaded_tasks.lock().expect("tasks lock").insert(
        mapped_task_id,
        Task {
            id: mapped_task_id,
            user_id: "did:example:agent".to_string(),
            app_id: "aicc".to_string(),
            parent_id: None,
            root_id: String::new(),
            name: "aicc complete".to_string(),
            task_type: "aicc.compute".to_string(),
            status: TaskStatus::Completed,
            progress: 1.0,
            message: Some("done".to_string()),
            data: json!({
                "aicc": {
                    "external_task_id": external_aicc_task_id
                },
                "result": {
                    "json": {"is_sleep":true,"output":{"answer":"mapped"}},
                    "usage": {"input_tokens": 5, "output_tokens": 4, "total_tokens": 9},
                    "extra": {"provider":"mock","model":"mock-1","latency_ms":6}
                }
            }),
            permissions: TaskPermissions::default(),
            created_at: 0,
            updated_at: 0,
        },
    );

    let responses = Arc::new(Mutex::new(VecDeque::from(vec![CompleteResponse::new(
        external_aicc_task_id.to_string(),
        CompleteStatus::Running,
        None,
        None,
    )])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
"#,
    )
    .await;

    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: preloaded_tasks.clone(),
            },
        ))),
        aicc,
        tools: Arc::new(ToolManager::new()),
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: TraceCtx {
            trace_id: "trace-3".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-3".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (result, tracking) = behavior
        .run_step(input)
        .await
        .expect("run_step should succeed");
    assert!(result.is_sleep());
    assert_eq!(tracking.token_usage.total, 9);
    assert_eq!(requests.lock().expect("requests lock").len(), 1);
}

#[tokio::test]
async fn run_step_accepts_succeeded_response_with_string_task_id() {
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![CompleteResponse::new(
        "aicc-1770927904938-1".to_string(),
        CompleteStatus::Succeeded,
        Some(AiResponseSummary {
            text: None,
            json: Some(json!({"is_sleep":true,"output":{"answer":"ok"}})),
            artifacts: vec![],
            usage: Some(AiUsage {
                input_tokens: Some(11),
                output_tokens: Some(7),
                total_tokens: Some(18),
            }),
            cost: None,
            finish_reason: Some("stop".to_string()),
            provider_task_ref: Some("provider-task-3".to_string()),
            extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":5})),
        }),
        None,
    )])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));

    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
"#,
    )
    .await;

    let tool_mgr = Arc::new(ToolManager::new());
    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: Arc::new(Mutex::new(HashMap::new())),
            },
        ))),
        aicc,
        tools: tool_mgr,
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: TraceCtx {
            trace_id: "trace-2".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-2".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (result, tracking) = behavior
        .run_step(input)
        .await
        .expect("run_step should succeed");
    assert!(result.is_sleep());
    assert_eq!(tracking.token_usage.total, 18);
    assert_eq!(requests.lock().expect("requests lock").len(), 1);
}

#[tokio::test]
async fn run_step_sets_behavior_task_as_parent_for_aicc_requests() {
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![CompleteResponse::new(
        "aicc-1770927904938-42".to_string(),
        CompleteStatus::Succeeded,
        Some(AiResponseSummary {
            text: None,
            json: Some(json!({"is_sleep":true,"output":{"answer":"ok"}})),
            artifacts: vec![],
            usage: Some(AiUsage {
                input_tokens: Some(6),
                output_tokens: Some(3),
                total_tokens: Some(9),
            }),
            cost: None,
            finish_reason: Some("stop".to_string()),
            provider_task_ref: Some("provider-task-42".to_string()),
            extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":5})),
        }),
        None,
    )])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
    let tasks = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
"#,
    )
    .await;
    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: tasks.clone(),
            },
        ))),
        aicc,
        tools: Arc::new(ToolManager::new()),
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };
    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: TraceCtx {
            trace_id: "trace-parent-1".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-parent-1".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (_result, _tracking) = behavior
        .run_step(input)
        .await
        .expect("run_step should succeed");

    let requests_guard = requests.lock().expect("requests lock");
    assert_eq!(requests_guard.len(), 1);
    let parent_id = requests_guard[0]
        .task_options
        .as_ref()
        .and_then(|opts| opts.parent_id)
        .expect("aicc request should carry parent task id");
    assert_eq!(
        requests_guard[0]
            .payload
            .options
            .as_ref()
            .and_then(|value| value.get("rootid"))
            .and_then(|value| value.as_str()),
        Some("agent#default")
    );
    assert!(requests_guard[0]
        .payload
        .options
        .as_ref()
        .and_then(|value| value.get("session_id"))
        .is_none());
    drop(requests_guard);

    let tasks_guard = tasks.lock().expect("tasks lock");
    let behavior_task = tasks_guard
        .get(&parent_id)
        .expect("parent behavior task should exist");
    assert_eq!(behavior_task.task_type, "llm_behavior");
    assert_eq!(behavior_task.parent_id, None);
    assert_eq!(behavior_task.status, TaskStatus::Completed);
    assert_eq!(
        behavior_task
            .data
            .get("rootid")
            .and_then(|value| value.as_str()),
        Some("agent#default")
    );
    assert_eq!(
        behavior_task
            .data
            .get("session_id")
            .and_then(|value| value.as_str()),
        None
    );
    assert!(!tasks_guard
        .values()
        .any(|task| task.task_type == "llm_infer"));
}

#[tokio::test]
async fn run_step_uses_session_id_as_task_rootid_when_present() {
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![CompleteResponse::new(
        "aicc-1770927904938-77".to_string(),
        CompleteStatus::Succeeded,
        Some(AiResponseSummary {
            text: None,
            json: Some(json!({"is_sleep":true,"output":{"answer":"ok"}})),
            artifacts: vec![],
            usage: Some(AiUsage {
                input_tokens: Some(7),
                output_tokens: Some(5),
                total_tokens: Some(12),
            }),
            cost: None,
            finish_reason: Some("stop".to_string()),
            provider_task_ref: Some("provider-task-77".to_string()),
            extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":6})),
        }),
        None,
    )])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
    let tasks = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
"#,
    )
    .await;
    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: tasks.clone(),
            },
        ))),
        aicc,
        tools: Arc::new(ToolManager::new()),
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };
    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: TraceCtx {
            trace_id: "trace-parent-2".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-parent-2".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![EnvKV {
            key: "loop.session_id".to_string(),
            value: "session-user-1".to_string(),
        }],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (_result, _tracking) = behavior
        .run_step(input)
        .await
        .expect("run_step should succeed");

    let requests_guard = requests.lock().expect("requests lock");
    assert_eq!(requests_guard.len(), 1);
    let parent_id = requests_guard[0]
        .task_options
        .as_ref()
        .and_then(|opts| opts.parent_id)
        .expect("aicc request should carry parent task id");
    assert_eq!(
        requests_guard[0]
            .payload
            .options
            .as_ref()
            .and_then(|value| value.get("rootid"))
            .and_then(|value| value.as_str()),
        Some("session-user-1")
    );
    assert_eq!(
        requests_guard[0]
            .payload
            .options
            .as_ref()
            .and_then(|value| value.get("session_id"))
            .and_then(|value| value.as_str()),
        Some("session-user-1")
    );
    drop(requests_guard);

    let tasks_guard = tasks.lock().expect("tasks lock");
    let behavior_task = tasks_guard
        .get(&parent_id)
        .expect("parent behavior task should exist");
    assert_eq!(
        behavior_task
            .data
            .get("rootid")
            .and_then(|value| value.as_str()),
        Some("session-user-1")
    );
    assert_eq!(
        behavior_task
            .data
            .get("session_id")
            .and_then(|value| value.as_str()),
        Some("session-user-1")
    );
}

fn run_actions_for_test(actions: &[ActionSpec]) -> Vec<Observation> {
    actions
        .iter()
        .map(|action| {
            println!(
                "[TEST][ACTION] running action title='{}' command='{}'",
                action.title, action.command
            );
            let content = json!({
                "command": action.command,
                "exit_code": 0,
                "stdout": "ok",
                "stderr": ""
            });
            println!("[TEST][ACTION] action observation: {}", content);
            Observation {
                source: ObservationSource::Action,
                name: action.title.clone(),
                bytes: serde_json::to_string(&content).unwrap_or_default().len(),
                content,
                ok: true,
                truncated: false,
            }
        })
        .collect()
}

#[tokio::test]
async fn run_step_then_run_actions_followup() {
    let task_store = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(json!({
                    "is_sleep": false,
                    "next_behavior": null,
                    "actions": [{
                        "kind": "bash",
                        "title": "echo action",
                        "command": "echo hello",
                        "cwd": null,
                        "timeout_ms": 1000,
                        "allow_network": false,
                        "fs_scope": {
                            "read_roots": [],
                            "write_roots": []
                        },
                        "rationale": "example action"
                    }],
                    "output": {"phase":"action_planned"}
                })),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(12),
                    output_tokens: Some(8),
                    total_tokens: Some(20),
                }),
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: Some("provider-action-1".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":12})),
            }),
            None,
        ),
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(json!({
                    "is_sleep": true,
                    "next_behavior": "END",
                    "actions": [],
                    "output": {"final":"after_action"}
                })),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(9),
                    output_tokens: Some(6),
                    total_tokens: Some(15),
                }),
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: Some("provider-action-2".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":9})),
            }),
            None,
        ),
    ])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
tools:
  mode: none
"#,
    )
    .await;

    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: task_store,
            },
        ))),
        aicc: Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
            responses: responses.clone(),
            requests: requests.clone(),
        }))),
        tools: Arc::new(ToolManager::new()),
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);

    let base_trace = TraceCtx {
        trace_id: "trace-actions".to_string(),
        agent_did: "did:example:agent".to_string(),
        behavior: "on_wakeup".to_string(),
        step_idx: 0,
        wakeup_id: "wakeup-actions".to_string(),
    };

    let first_input = BehaviorExecInput {
        trace: base_trace.clone(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (first_result, first_tracking) = behavior
        .run_step(first_input)
        .await
        .expect("first run_step should succeed");
    println!(
        "[TEST][RUN_STEP] first result: next_behavior={:?} is_sleep={} actions={} usage_total={}",
        first_result.next_behavior,
        first_result.is_sleep(),
        first_result.actions.len(),
        first_tracking.token_usage.total
    );
    assert_eq!(first_result.actions.len(), 1);
    assert!(!first_result.is_sleep());

    let action_observations = run_actions_for_test(&first_result.actions);

    let second_input = BehaviorExecInput {
        trace: TraceCtx {
            step_idx: 1,
            ..base_trace
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"action_done"}),
        memory: json!({"facts":[]}),
        last_observations: action_observations,
        limits: behavior_cfg.limits.clone(),
    };

    let (second_result, second_tracking) = behavior
        .run_step(second_input)
        .await
        .expect("second run_step should succeed");
    println!(
        "[TEST][RUN_STEP] second result: next_behavior={:?} is_sleep={} actions={} usage_total={}",
        second_result.next_behavior,
        second_result.is_sleep(),
        second_result.actions.len(),
        second_tracking.token_usage.total
    );
    assert!(second_result.is_sleep());
    assert_eq!(second_result.next_behavior.as_deref(), Some("END"));

    let requests_guard = requests.lock().expect("requests lock");
    assert_eq!(requests_guard.len(), 2);
    let has_obs = requests_guard[1]
        .payload
        .messages
        .iter()
        .any(|m| m.content.contains("<<OBSERVATIONS>>"));
    assert!(has_obs);
}

#[tokio::test]
async fn run_step_with_workshop_list_dir_then_plan_python_actions() {
    let tmp = tempdir().expect("create tempdir");
    let root = tmp.path().to_path_buf();
    let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
        .await
        .expect("create workshop");
    fs::write(root.join("todo/seed.txt"), "seed\n")
        .await
        .expect("write seed file");

    let tool_mgr = Arc::new(ToolManager::new());
    workshop
        .register_tools(tool_mgr.as_ref())
        .expect("register workshop tools");

    let responses = Arc::new(Mutex::new(VecDeque::from(vec![
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(json!({
                    "tool_calls": [{
                        "name": TOOL_EXEC_BASH,
                        "args": {
                            "command": "ls -1 todo"
                        },
                        "call_id": "call-list-todo"
                    }]
                })),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(11),
                    output_tokens: Some(7),
                    total_tokens: Some(18),
                }),
                cost: None,
                finish_reason: Some("tool_calls".to_string()),
                provider_task_ref: Some("provider-workshop-1".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":7})),
            }),
            None,
        ),
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(json!({
                    "is_sleep": false,
                    "next_behavior": "on_action",
                    "actions": [{
                        "kind": "bash",
                        "title": "write test.py",
                        "command": "cat > artifacts/test.py <<'PY'\nprint('hello workshop')\nPY",
                        "execution_mode": "serial",
                        "cwd": null,
                        "timeout_ms": 1000,
                        "allow_network": false,
                        "fs_scope": {
                            "read_roots": [],
                            "write_roots": ["artifacts"]
                        },
                        "rationale": "create python test script"
                    }, {
                        "kind": "bash",
                        "title": "chmod test.py executable",
                        "command": "chmod +x artifacts/test.py",
                        "execution_mode": "serial",
                        "cwd": null,
                        "timeout_ms": 1000,
                        "allow_network": false,
                        "fs_scope": {
                            "read_roots": [],
                            "write_roots": ["artifacts"]
                        },
                        "rationale": "make script executable"
                    }],
                    "output": {"phase":"actions_planned"}
                })),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(9),
                    output_tokens: Some(10),
                    total_tokens: Some(19),
                }),
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: Some("provider-workshop-2".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":8})),
            }),
            None,
        ),
    ])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
tools:
  mode: allow_list
  names:
    - exec_bash
"#,
    )
    .await;

    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: Arc::new(Mutex::new(HashMap::<i64, Task>::new())),
            },
        ))),
        aicc: Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
            responses,
            requests: requests.clone(),
        }))),
        tools: tool_mgr.clone(),
        policy: Arc::new(MockPolicy {
            tools: behavior_cfg
                .tools
                .filter_tool_specs(&tool_mgr.list_tool_specs()),
        }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: TraceCtx {
            trace_id: "trace-workshop-actions".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-workshop-actions".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (result, tracking) = behavior
        .run_step(input)
        .await
        .expect("run_step should succeed");
    assert_eq!(tracking.tool_trace.len(), 1);
    assert_eq!(tracking.tool_trace[0].tool_name, TOOL_EXEC_BASH);
    assert_eq!(result.actions.len(), 2);
    assert_eq!(
        result.actions[0].execution_mode,
        ActionExecutionMode::Serial
    );
    assert_eq!(
        result.actions[1].execution_mode,
        ActionExecutionMode::Serial
    );
    assert!(result.actions[0].command.contains("artifacts/test.py"));
    assert_eq!(result.actions[1].command, "chmod +x artifacts/test.py");

    let requests_guard = requests.lock().expect("requests lock");
    assert_eq!(requests_guard.len(), 2);
    let tool_messages = requests_guard[1]
        .payload
        .options
        .as_ref()
        .and_then(|v| v.get("tool_messages"))
        .cloned()
        .unwrap_or_else(|| json!([]))
        .to_string();
    assert!(tool_messages.contains(TOOL_EXEC_BASH));
    assert!(tool_messages.contains("seed.txt"));

    // Formally execute planned actions through workshop.exec_bash.
    let action_ctx = TraceCtx {
        trace_id: "trace-workshop-actions".to_string(),
        agent_did: "did:example:agent".to_string(),
        behavior: "on_action".to_string(),
        step_idx: 1,
        wakeup_id: "wakeup-workshop-actions".to_string(),
    };
    for (idx, action) in result.actions.iter().enumerate() {
        assert_eq!(
            action.execution_mode,
            ActionExecutionMode::Serial,
            "test fixture expects serial actions before executing sequentially"
        );

        let mut args = json!({
            "command": action.command,
            "timeout_ms": action.timeout_ms,
        });
        if let Some(cwd) = &action.cwd {
            args["cwd"] = json!(cwd);
        }

        let raw = tool_mgr
            .call_tool(
                &action_ctx,
                ToolCall {
                    name: TOOL_EXEC_BASH.to_string(),
                    args,
                    call_id: format!("action-exec-{idx}"),
                },
            )
            .await
            .expect("action command should run by workshop tool");
        assert_eq!(
            raw["ok"].as_bool(),
            Some(true),
            "action command returned non-zero: {}",
            raw
        );
    }

    // Verify workshop tool effect: file created and turned executable.
    let test_py_path = root.join("artifacts/test.py");
    let content = fs::read_to_string(&test_py_path)
        .await
        .expect("test.py should be created by executed action");
    assert!(content.contains("hello workshop"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&test_py_path)
            .expect("read test.py metadata")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "test.py should have executable bit");
    }
}

#[tokio::test]
async fn run_step_with_agent_memory_tool_chain_then_insert_thing_by_action() {
    let tmp = tempdir().expect("create tempdir");
    let root = tmp.path().to_path_buf();

    let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
        .await
        .expect("create workshop");
    let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
        .await
        .expect("create agent memory");

    fs::write(
        root.join("memory/memory.md"),
        "## Long-term context\n- project: opendan memory integration\n- user: prefers concise updates\n",
    )
    .await
    .expect("write memory.md");

    // Seed a thing so load_things has queryable baseline.
    let things_db = root.join("memory/things.db");
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(&things_db).expect("open things.db");
        conn.execute(
            "INSERT OR REPLACE INTO kv(key, value, updated_at, source, confidence)
             VALUES ('project.plan', 'integrate memory tools into behavior loop', 100, 'seed', 0.8)",
            [],
        )
        .expect("insert baseline kv");
    })
    .await
    .expect("join seed things");

    let tool_mgr = Arc::new(ToolManager::new());
    workshop
        .register_tools(tool_mgr.as_ref())
        .expect("register workshop tools");
    memory
        .register_tools(tool_mgr.as_ref())
        .expect("register memory tools");

    let responses = Arc::new(Mutex::new(VecDeque::from(vec![
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(json!({
                    "tool_calls": [{
                        "name": TOOL_LOAD_MEMORY,
                        "args": {
                            "token_limit": 128
                        },
                        "call_id": "call-load-memory"
                    }]
                })),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(6),
                    total_tokens: Some(16),
                }),
                cost: None,
                finish_reason: Some("tool_calls".to_string()),
                provider_task_ref: Some("provider-memory-step-1".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":8})),
            }),
            None,
        ),
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(json!({
                    "tool_calls": [{
                        "name": TOOL_LOAD_THINGS,
                        "args": {
                            "name": "project",
                            "limit": 8
                        },
                        "call_id": "call-load-things"
                    }]
                })),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(9),
                    output_tokens: Some(7),
                    total_tokens: Some(16),
                }),
                cost: None,
                finish_reason: Some("tool_calls".to_string()),
                provider_task_ref: Some("provider-memory-step-2".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":7})),
            }),
            None,
        ),
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(json!({
                    "is_sleep": false,
                    "next_behavior": "on_action",
                    "actions": [{
                        "kind": "bash",
                        "title": "insert project status into things",
                        "command": "python3 - <<'PY'\nimport sqlite3, time\nconn = sqlite3.connect('memory/things.db')\nconn.execute(\"INSERT OR REPLACE INTO kv(key, value, updated_at, source, confidence) VALUES (?, ?, ?, ?, ?)\", (\"project.status\", \"action_inserted\", int(time.time() * 1000), \"behavior-test\", 0.95))\nconn.commit()\nconn.close()\nPY",
                        "execution_mode": "serial",
                        "cwd": null,
                        "timeout_ms": 30000,
                        "allow_network": false,
                        "fs_scope": {
                            "read_roots": ["memory"],
                            "write_roots": ["memory"]
                        },
                        "rationale": "persist updated project status into structured memory"
                    }],
                    "output": {
                        "phase": "memory_tools_complete"
                    }
                })),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(11),
                    output_tokens: Some(10),
                    total_tokens: Some(21),
                }),
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: Some("provider-memory-step-3".to_string()),
                extra: Some(json!({"provider":"mock","model":"mock-1","latency_ms":10})),
            }),
            None,
        ),
    ])));
    let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
process_rule: test_rule
tools:
  mode: allow_list
  names:
    - load_memory
    - load_things
limits:
  max_tool_rounds: 3
"#,
    )
    .await;

    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: Arc::new(Mutex::new(HashMap::<i64, Task>::new())),
            },
        ))),
        aicc: Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
            responses,
            requests: requests.clone(),
        }))),
        tools: tool_mgr.clone(),
        policy: Arc::new(MockPolicy {
            tools: behavior_cfg
                .tools
                .filter_tool_specs(&tool_mgr.list_tool_specs()),
        }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: TraceCtx {
            trace_id: "trace-memory-action".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-memory-action".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: None,
        behavior_prompt: behavior_cfg.process_rule.clone(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({}),
        last_observations: vec![],
        limits: behavior_cfg.limits.clone(),
    };

    let (result, tracking) = behavior
        .run_step(input)
        .await
        .expect("run_step should succeed");
    assert_eq!(tracking.tool_trace.len(), 2);
    assert_eq!(tracking.tool_trace[0].tool_name, TOOL_LOAD_MEMORY);
    assert_eq!(tracking.tool_trace[1].tool_name, TOOL_LOAD_THINGS);
    assert_eq!(result.actions.len(), 1);
    assert!(result.actions[0].command.contains("memory/things.db"));

    let requests_guard = requests.lock().expect("requests lock");
    assert_eq!(requests_guard.len(), 3);
    let round_2_tool_messages = requests_guard[1]
        .payload
        .options
        .as_ref()
        .and_then(|v| v.get("tool_messages"))
        .cloned()
        .unwrap_or_else(|| json!([]))
        .to_string();
    assert!(round_2_tool_messages.contains(TOOL_LOAD_MEMORY));
    assert!(round_2_tool_messages.contains("Long-term context"));

    let round_3_tool_messages = requests_guard[2]
        .payload
        .options
        .as_ref()
        .and_then(|v| v.get("tool_messages"))
        .cloned()
        .unwrap_or_else(|| json!([]))
        .to_string();
    assert!(round_3_tool_messages.contains(TOOL_LOAD_THINGS));
    assert!(round_3_tool_messages.contains("project.plan"));

    let action_ctx = TraceCtx {
        trace_id: "trace-memory-action".to_string(),
        agent_did: "did:example:agent".to_string(),
        behavior: "on_action".to_string(),
        step_idx: 1,
        wakeup_id: "wakeup-memory-action".to_string(),
    };
    let exec_raw = tool_mgr
        .call_tool(
            &action_ctx,
            ToolCall {
                name: TOOL_EXEC_BASH.to_string(),
                args: json!({
                    "command": result.actions[0].command,
                    "timeout_ms": result.actions[0].timeout_ms
                }),
                call_id: "memory-action-exec-1".to_string(),
            },
        )
        .await
        .expect("execute action command");
    assert_eq!(
        exec_raw["ok"].as_bool(),
        Some(true),
        "action command returned non-zero: {}",
        exec_raw
    );

    let readback_db = root.join("memory/things.db");
    let inserted_value = tokio::task::spawn_blocking(move || {
        let conn = Connection::open(&readback_db).expect("open things db for readback");
        conn.query_row(
            "SELECT value FROM kv WHERE key = 'project.status'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read inserted kv value")
    })
    .await
    .expect("join readback");
    assert_eq!(inserted_value, "action_inserted");
}
