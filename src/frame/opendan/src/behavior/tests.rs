use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{
    value_to_object_map, AiMethodRequest, AiMethodResponse, AiMethodStatus, AiResponseSummary,
    AiToolCall, AiUsage, AiccClient, Task, TaskManagerClient, TaskPermissions, TaskStatus,
};
use serde_json::{json, Value as Json};
use tempfile::tempdir;
use tokio::fs;

use super::*;
use crate::agent_environment::AgentEnvironment;
use crate::agent_tool::{
    AgentTool, AgentToolManager, AgentToolResult, DoAction, DoActions, ToolSpec,
};
use crate::test_utils::{MockAicc, MockTaskMgrHandler};

struct MockTokenizer;

impl Tokenizer for MockTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

struct MockWorklog;

#[async_trait]
impl WorklogSink for MockWorklog {
    async fn emit(&self, _event: AgentWorkEvent) {}
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
        calls: &[AiToolCall],
    ) -> Result<Vec<AiToolCall>, String> {
        Ok(calls.to_vec())
    }
}

struct EchoTool;

#[async_trait]
impl AgentTool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_string(),
            description: "echo".to_string(),
            args_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        false
    }
    fn support_action(&self) -> bool {
        true
    }
    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, crate::agent_tool::AgentToolError> {
        println!("[TEST][TOOL] echo called with args: {}", args);
        Ok(
            AgentToolResult::from_details(json!({"tool": "echo", "ok": true, "args": args}))
                .with_result("ok"),
        )
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

async fn build_test_environment() -> Arc<AgentEnvironment> {
    Arc::new(
        AgentEnvironment::new(std::env::temp_dir())
            .await
            .expect("create test agent environment"),
    )
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
            data: serde_json::to_value(AiMethodResponse::new(
                "".to_string(),
                AiMethodStatus::Succeeded,
                Some(AiResponseSummary {
                    text: None,
                    tool_calls: vec![AiToolCall {
                        name: "echo".to_string(),
                        args: value_to_object_map(json!({"msg": "hi"})),
                        call_id: "call-1".to_string(),
                    }],
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
        AiMethodResponse::new(
            aicc_async_task_id.to_string(),
            AiMethodStatus::Running,
            None,
            None,
        ),
        AiMethodResponse::new(
            "".to_string(),
            AiMethodStatus::Succeeded,
            Some(AiResponseSummary {
                text: Some(
                    "<response><next_behavior>END</next_behavior><reply>done</reply></response>"
                        .to_string(),
                ),
                tool_calls: vec![],
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
    let requests = Arc::new(Mutex::new(Vec::<AiMethodRequest>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));

    let tool_mgr = Arc::new(AgentToolManager::new());
    tool_mgr
        .register_tool(EchoTool)
        .expect("register echo should succeed");
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
system: test_rule
tools:
  mode: allow_list
  names:
    - echo
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
        memory: None,
        policy: Arc::new(MockPolicy {
            tools: behavior_cfg.tools.filter_tool_specs(&[ToolSpec {
                name: "echo".to_string(),
                description: "echo".to_string(),
                args_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
                usage: None,
            }]),
        }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
        environment: build_test_environment().await,
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-1".to_string(),
            session_id: "session-test".to_string(),
        },
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-test".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let (result, tracking) = behavior
        .run_step(&input)
        .await
        .expect("run_step should succeed");
    println!(
        "[TEST][RUN_STEP] tool_followup result: next_behavior={:?} is_sleep={} actions={} usage_total={}",
        result.next_behavior,
        result.is_sleep(),
        result.actions.cmds.len(),
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
                    "text": "<response><next_behavior>END</next_behavior><reply>mapped</reply></response>",
                    "usage": {"input_tokens": 5, "output_tokens": 4, "total_tokens": 9},
                    "extra": {"provider":"mock","model":"mock-1","latency_ms":6}
                }
            }),
            permissions: TaskPermissions::default(),
            created_at: 0,
            updated_at: 0,
        },
    );

    let responses = Arc::new(Mutex::new(VecDeque::from(vec![AiMethodResponse::new(
        external_aicc_task_id.to_string(),
        AiMethodStatus::Running,
        None,
        None,
    )])));
    let requests = Arc::new(Mutex::new(Vec::<AiMethodRequest>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
system: test_rule
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
        tools: Arc::new(AgentToolManager::new()),
        memory: None,
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
        environment: build_test_environment().await,
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: SessionRuntimeContext {
            trace_id: "trace-3".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-3".to_string(),
            session_id: "session-test".to_string(),
        },
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-test".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let (result, tracking) = behavior
        .run_step(&input)
        .await
        .expect("run_step should succeed");
    assert!(result.is_sleep());
    assert_eq!(tracking.token_usage.total, 9);
    assert_eq!(requests.lock().expect("requests lock").len(), 1);
}

#[tokio::test]
async fn run_step_accepts_succeeded_response_with_string_task_id() {
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![AiMethodResponse::new(
        "aicc-1770927904938-1".to_string(),
        AiMethodStatus::Succeeded,
        Some(AiResponseSummary {
            text: Some("<response><next_behavior>END</next_behavior></response>".to_string()),
            tool_calls: vec![],
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
    let requests = Arc::new(Mutex::new(Vec::<AiMethodRequest>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));

    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
system: test_rule
"#,
    )
    .await;

    let tool_mgr = Arc::new(AgentToolManager::new());
    let deps = LLMBehaviorDeps {
        taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: Arc::new(Mutex::new(HashMap::new())),
            },
        ))),
        aicc,
        tools: tool_mgr,
        memory: None,
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
        environment: build_test_environment().await,
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: SessionRuntimeContext {
            trace_id: "trace-2".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-2".to_string(),
            session_id: "session-test".to_string(),
        },
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-test".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let (result, tracking) = behavior
        .run_step(&input)
        .await
        .expect("run_step should succeed");
    assert!(result.is_sleep());
    assert_eq!(tracking.token_usage.total, 18);
    assert_eq!(requests.lock().expect("requests lock").len(), 1);
}

#[tokio::test]
async fn run_step_sets_behavior_task_as_parent_for_aicc_requests() {
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![AiMethodResponse::new(
        "aicc-1770927904938-42".to_string(),
        AiMethodStatus::Succeeded,
        Some(AiResponseSummary {
            text: Some("<response><next_behavior>END</next_behavior></response>".to_string()),
            tool_calls: vec![],
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
    let requests = Arc::new(Mutex::new(Vec::<AiMethodRequest>::new()));
    let tasks = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
system: test_rule
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
        tools: Arc::new(AgentToolManager::new()),
        memory: None,
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
        environment: build_test_environment().await,
    };
    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: SessionRuntimeContext {
            trace_id: "trace-parent-1".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-parent-1".to_string(),
            session_id: "session-test".to_string(),
        },
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-test".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let (_result, _tracking) = behavior
        .run_step(&input)
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
        None
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
        Some("session-test")
    );
    assert_eq!(
        behavior_task
            .data
            .get("session_id")
            .and_then(|value| value.as_str()),
        Some("session-test")
    );
    assert!(!tasks_guard
        .values()
        .any(|task| task.task_type == "llm_infer"));
}

#[tokio::test]
async fn run_step_uses_session_id_as_task_rootid_when_present() {
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![AiMethodResponse::new(
        "aicc-1770927904938-77".to_string(),
        AiMethodStatus::Succeeded,
        Some(AiResponseSummary {
            text: Some("<response><next_behavior>END</next_behavior></response>".to_string()),
            tool_calls: vec![],
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
    let requests = Arc::new(Mutex::new(Vec::<AiMethodRequest>::new()));
    let tasks = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
system: test_rule
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
        tools: Arc::new(AgentToolManager::new()),
        memory: None,
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
        environment: build_test_environment().await,
    };
    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: SessionRuntimeContext {
            trace_id: "trace-parent-2".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-parent-2".to_string(),
            session_id: "session-test".to_string(),
        },
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-user-1".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let (_result, _tracking) = behavior
        .run_step(&input)
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
        None
    );
    assert_eq!(
        requests_guard[0]
            .payload
            .options
            .as_ref()
            .and_then(|value| value.get("session_id"))
            .and_then(|value| value.as_str()),
        None
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
        Some("session-user-1"),
        "when session_id is present, rootid should be session_id"
    );
    assert_eq!(
        behavior_task
            .data
            .get("session_id")
            .and_then(|value| value.as_str()),
        Some("session-user-1"),
    );
}

#[tokio::test]
async fn run_step_returns_provider_error_when_summary_has_no_text_and_no_tools() {
    let responses = Arc::new(Mutex::new(VecDeque::from(vec![AiMethodResponse::new(
        "aicc-1770927904938-88".to_string(),
        AiMethodStatus::Succeeded,
        Some(AiResponseSummary {
            text: None,
            tool_calls: vec![],
            artifacts: vec![],
            usage: Some(AiUsage {
                input_tokens: Some(7),
                output_tokens: Some(3200),
                total_tokens: Some(3207),
            }),
            cost: None,
            finish_reason: Some("incomplete".to_string()),
            provider_task_ref: Some("provider-task-88".to_string()),
            extra: Some(
                json!({"provider":"openai","model":"gpt-5.2-codex","latency_ms":2000,"provider_io":{"output":{"incomplete_details":{"reason":"max_output_tokens"}}}}),
            ),
        }),
        None,
    )])));
    let requests = Arc::new(Mutex::new(Vec::<AiMethodRequest>::new()));
    let tasks = Arc::new(Mutex::new(HashMap::<i64, Task>::new()));

    let aicc = Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
        responses: responses.clone(),
        requests: requests.clone(),
    })));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
system: test_rule
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
        tools: Arc::new(AgentToolManager::new()),
        memory: None,
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
        environment: build_test_environment().await,
    };
    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);
    let input = BehaviorExecInput {
        trace: SessionRuntimeContext {
            trace_id: "trace-empty-1".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-empty-1".to_string(),
            session_id: "session-test".to_string(),
        },
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-user-1".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let err = behavior
        .run_step(&input)
        .await
        .expect_err("run_step should fail on empty response");
    assert!(
        err.to_string().contains("TOKEN_LIMIT_EXCEEDED"),
        "unexpected error: {}",
        err
    );
    assert!(
        err.to_string().contains("max_output_tokens"),
        "unexpected error: {}",
        err
    );
}

fn run_actions_for_test(actions: &DoActions) -> Vec<Observation> {
    actions
        .cmds
        .iter()
        .enumerate()
        .map(|(idx, action)| {
            let (name, command) = match action {
                DoAction::Exec(command) => (format!("exec-{idx}"), command.clone()),
                DoAction::Call(call) => {
                    let name = call.call_action_name.clone();
                    let command = serde_json::to_string(&call.call_params)
                        .unwrap_or_else(|_| "{}".to_string());
                    (name, command)
                }
            };
            println!(
                "[TEST][ACTION] running action name='{}' command='{}'",
                name, command
            );
            let content = json!({
                "command": command,
                "exit_code": 0,
                "stdout": "ok",
                "stderr": ""
            });
            println!("[TEST][ACTION] action observation: {}", content);
            Observation {
                source: ObservationSource::Action,
                name,
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
        AiMethodResponse::new(
            "".to_string(),
            AiMethodStatus::Succeeded,
            Some(AiResponseSummary {
                text: Some(
                    r#"<response><actions mode="failed_end"><command>echo hello</command></actions></response>"#
                        .to_string(),
                ),
                tool_calls: vec![],
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
        AiMethodResponse::new(
            "".to_string(),
            AiMethodStatus::Succeeded,
            Some(AiResponseSummary {
                text: Some(
                    r#"<response><next_behavior>END</next_behavior><actions mode="failed_end"></actions></response>"#
                        .to_string(),
                ),
                tool_calls: vec![],
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
    let requests = Arc::new(Mutex::new(Vec::<AiMethodRequest>::new()));
    let behavior_cfg = load_behavior_config_yaml_for_test(
        "on_wakeup",
        r#"
system: test_rule
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
        tools: Arc::new(AgentToolManager::new()),
        memory: None,
        policy: Arc::new(MockPolicy { tools: vec![] }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
        environment: build_test_environment().await,
    };

    let behavior = LLMBehavior::new(behavior_cfg.to_llm_behavior_config(), deps);

    let base_trace = SessionRuntimeContext {
        trace_id: "trace-actions".to_string(),
        agent_name: "did:example:agent".to_string(),
        behavior: "on_wakeup".to_string(),
        step_idx: 0,
        wakeup_id: "wakeup-actions".to_string(),
        session_id: "session-test".to_string(),
    };

    let first_input = BehaviorExecInput {
        trace: base_trace.clone(),
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-test".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let (first_result, first_tracking) = behavior
        .run_step(&first_input)
        .await
        .expect("first run_step should succeed");
    println!(
        "[TEST][RUN_STEP] first result: next_behavior={:?} is_sleep={} actions={} usage_total={}",
        first_result.next_behavior,
        first_result.is_sleep(),
        first_result.actions.cmds.len(),
        first_tracking.token_usage.total
    );
    assert_eq!(first_result.actions.cmds.len(), 1);
    assert!(!first_result.is_sleep());

    let _action_observations = run_actions_for_test(&first_result.actions);

    let second_input = BehaviorExecInput {
        trace: SessionRuntimeContext {
            step_idx: 1,
            ..base_trace
        },
        input_prompt: String::new(),
        last_step_prompt: String::new(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        session_id: "session-test".to_string(),
        behavior_prompt: behavior_cfg.system.clone(),
        limits: behavior_cfg.limits.clone(),
        behavior_cfg: behavior_cfg.clone(),
        session: None,
    };

    let (second_result, second_tracking) = behavior
        .run_step(&second_input)
        .await
        .expect("second run_step should succeed");
    println!(
        "[TEST][RUN_STEP] second result: next_behavior={:?} is_sleep={} actions={} usage_total={}",
        second_result.next_behavior,
        second_result.is_sleep(),
        second_result.actions.cmds.len(),
        second_tracking.token_usage.total
    );
    assert!(second_result.is_sleep());
    assert_eq!(second_result.next_behavior.as_deref(), Some("END"));

    let requests_guard = requests.lock().expect("requests lock");
    assert_eq!(requests_guard.len(), 2);
    // TODO: observations will be included when build_memory_prompt_text implements dynamic compression
    // let has_obs = requests_guard[1]
    //     .payload
    //     .messages
    //     .iter()
    //     .any(|m| m.content.contains("<<Observations>>"));
    // assert!(has_obs);
}
