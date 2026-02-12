use std::collections::HashMap;
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::{
    AiResponseSummary, AiUsage, AiccClient, AiccHandler, CompleteRequest, CompleteResponse,
    CompleteStatus, CreateTaskOptions, Task, TaskFilter, TaskManagerClient, TaskManagerHandler,
    TaskPermissions, TaskStatus,
};
use kRPC::{RPCContext, RPCErrors, Result as KRPCResult};
use serde_json::{json, Value as Json};

use super::*;
use crate::agent_tool::{AgentTool, ToolCall, ToolManager, ToolSpec};

struct MockTokenizer;

impl Tokenizer for MockTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

struct MockTaskMgrHandler {
    counter: Mutex<u64>,
    tasks: Arc<Mutex<HashMap<i64, Task>>>,
}

#[async_trait]
impl TaskManagerHandler for MockTaskMgrHandler {
    async fn handle_create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Json>,
        opts: CreateTaskOptions,
        user_id: &str,
        app_id: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<Task> {
        let mut guard = self.counter.lock().expect("counter lock");
        *guard += 1;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let task = Task {
            id: *guard as i64,
            user_id: user_id.to_string(),
            app_id: app_id.to_string(),
            parent_id: opts.parent_id,
            root_id: None,
            name: name.to_string(),
            task_type: task_type.to_string(),
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data: data.unwrap_or_else(|| json!({})),
            permissions: opts.permissions.unwrap_or(TaskPermissions::default()),
            created_at: now,
            updated_at: now,
        };
        self.tasks
            .lock()
            .expect("tasks lock")
            .insert(task.id, task.clone());
        Ok(task)
    }

    async fn handle_get_task(&self, id: i64, _ctx: RPCContext) -> KRPCResult<Task> {
        self.tasks
            .lock()
            .expect("tasks lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError(format!("mock task {} not found", id)))
    }

    async fn handle_list_tasks(
        &self,
        _filter: TaskFilter,
        _source_user_id: Option<&str>,
        _source_app_id: Option<&str>,
        _ctx: RPCContext,
    ) -> KRPCResult<Vec<Task>> {
        Ok(vec![])
    }

    async fn handle_list_tasks_by_time_range(
        &self,
        _app_id: Option<&str>,
        _task_type: Option<&str>,
        _source_user_id: Option<&str>,
        _source_app_id: Option<&str>,
        _time_range: Range<u64>,
        _ctx: RPCContext,
    ) -> KRPCResult<Vec<Task>> {
        Ok(vec![])
    }

    async fn handle_get_subtasks(
        &self,
        _parent_id: i64,
        _ctx: RPCContext,
    ) -> KRPCResult<Vec<Task>> {
        Ok(vec![])
    }

    async fn handle_update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data: Option<Json>,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            if let Some(s) = status {
                task.status = s;
            }
            if let Some(p) = progress {
                task.progress = p;
            }
            task.message = message;
            if let Some(patch) = data {
                task.data = patch;
            }
        }
        Ok(())
    }

    async fn handle_update_task_progress(
        &self,
        id: i64,
        completed_items: u64,
        total_items: u64,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            if total_items > 0 {
                task.progress = (completed_items as f32 / total_items as f32).clamp(0.0, 1.0);
            }
        }
        Ok(())
    }

    async fn handle_update_task_status(
        &self,
        id: i64,
        status: TaskStatus,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            task.status = status;
        }
        Ok(())
    }

    async fn handle_update_task_error(
        &self,
        id: i64,
        error_message: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            task.status = TaskStatus::Failed;
            task.message = Some(error_message.to_string());
        }
        Ok(())
    }

    async fn handle_update_task_data(
        &self,
        id: i64,
        data: Json,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            task.data = data;
        }
        Ok(())
    }

    async fn handle_cancel_task(
        &self,
        _id: i64,
        _recursive: bool,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        Ok(())
    }

    async fn handle_delete_task(&self, _id: i64, _ctx: RPCContext) -> KRPCResult<()> {
        Ok(())
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
    async fn allowed_tools(&self, _input: &ProcessInput) -> Result<Vec<ToolSpec>, String> {
        Ok(self.tools.clone())
    }

    async fn gate_tool_calls(
        &self,
        _input: &ProcessInput,
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
        _ctx: &crate::agent_tool::ToolCallContext,
        args: Json,
    ) -> Result<Json, crate::agent_tool::ToolError> {
        println!("[TEST][TOOL] tool.echo called with args: {}", args);
        Ok(json!({"tool": "tool.echo", "ok": true, "args": args}))
    }
}

struct MockAicc {
    responses: Arc<Mutex<VecDeque<CompleteResponse>>>,
    requests: Arc<Mutex<Vec<CompleteRequest>>>,
}

#[async_trait]
impl AiccHandler for MockAicc {
    async fn handle_complete(
        &self,
        request: CompleteRequest,
        _ctx: RPCContext,
    ) -> KRPCResult<CompleteResponse> {
        let prompt = request
            .payload
            .messages
            .iter()
            .map(|m| format!("role={}\n{}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        println!(
            "[TEST][AICC] complete request incoming prompt/messages:\n{}",
            prompt
        );
        if let Some(options) = &request.payload.options {
            println!("[TEST][AICC] complete request options: {}", options);
        }

        self.requests.lock().expect("requests lock").push(request);
        let resp = self
            .responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| RPCErrors::ReasonError("no response queued".to_string()))?;
        if let Some(result) = &resp.result {
            if let Some(output_json) = &result.json {
                println!("[TEST][AICC] llm output (json): {}", output_json);
            } else if let Some(output_text) = &result.text {
                println!("[TEST][AICC] llm output (text): {}", output_text);
            } else {
                println!("[TEST][AICC] llm output: <empty>");
            }
        } else if !resp.task_id.is_empty() {
            println!("[TEST][AICC] llm output: <pending task {}>", resp.task_id);
        } else {
            println!("[TEST][AICC] llm output: <none>");
        }
        Ok(resp)
    }

    async fn handle_cancel(
        &self,
        task_id: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<buckyos_api::CancelResponse> {
        Ok(buckyos_api::CancelResponse::new(task_id.to_string(), true))
    }
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

    let draft = OutputParser::parse_first(&raw, true).expect("parse should succeed");
    assert!(draft.is_sleep);
    assert!(matches!(draft.output, LLMOutput::Json(_)));
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
            root_id: None,
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
                    json!({"is_sleep":true,"next_behavior":"idle","output":{"answer":"done"}}),
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
            tools: vec![ToolSpec {
                name: "tool.echo".to_string(),
                description: "echo".to_string(),
                args_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
            }],
        }),
        worklog: Arc::new(MockWorklog),
        tokenizer: Arc::new(MockTokenizer),
    };

    let behavior = LLMBehavior::new(LLMBehaviorConfig::default(), deps);
    let input = ProcessInput {
        trace: TraceCtx {
            trace_id: "trace-1".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-1".to_string(),
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        behavior_prompt: "do work".to_string(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: StepLimits::default(),
    };

    let result = behavior.run_step(input).await;
    println!(
        "[TEST][RUN_STEP] tool_followup result: status={:?} next_behavior={:?} is_sleep={} actions={} usage_total={}",
        result.status,
        result.next_behavior,
        result.is_sleep,
        result.actions.len(),
        result.token_usage.total
    );
    assert!(matches!(result.status, LLMStatus::Ok));
    assert!(result.is_sleep);
    assert_eq!(result.next_behavior.as_deref(), Some("idle"));
    assert_eq!(result.tool_trace.len(), 1);
    assert_eq!(result.token_usage.total, 25);

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
                    "next_behavior": "idle",
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

    let behavior = LLMBehavior::new(LLMBehaviorConfig::default(), deps);

    let base_trace = TraceCtx {
        trace_id: "trace-actions".to_string(),
        agent_did: "did:example:agent".to_string(),
        behavior: "on_wakeup".to_string(),
        step_idx: 0,
        wakeup_id: "wakeup-actions".to_string(),
    };

    let first_input = ProcessInput {
        trace: base_trace.clone(),
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        behavior_prompt: "plan action first".to_string(),
        env_context: vec![],
        inbox: json!({"event":"wake"}),
        memory: json!({"facts":[]}),
        last_observations: vec![],
        limits: StepLimits::default(),
    };

    let first_result = behavior.run_step(first_input).await;
    println!(
        "[TEST][RUN_STEP] first result: status={:?} next_behavior={:?} is_sleep={} actions={} usage_total={}",
        first_result.status,
        first_result.next_behavior,
        first_result.is_sleep,
        first_result.actions.len(),
        first_result.token_usage.total
    );
    assert!(matches!(first_result.status, LLMStatus::Ok));
    assert_eq!(first_result.actions.len(), 1);
    assert!(!first_result.is_sleep);

    let action_observations = run_actions_for_test(&first_result.actions);

    let second_input = ProcessInput {
        trace: TraceCtx {
            step_idx: 1,
            ..base_trace
        },
        role_md: "role".to_string(),
        self_md: "self".to_string(),
        behavior_prompt: "summarize action result".to_string(),
        env_context: vec![],
        inbox: json!({"event":"action_done"}),
        memory: json!({"facts":[]}),
        last_observations: action_observations,
        limits: StepLimits::default(),
    };

    let second_result = behavior.run_step(second_input).await;
    println!(
        "[TEST][RUN_STEP] second result: status={:?} next_behavior={:?} is_sleep={} actions={} usage_total={}",
        second_result.status,
        second_result.next_behavior,
        second_result.is_sleep,
        second_result.actions.len(),
        second_result.token_usage.total
    );
    assert!(matches!(second_result.status, LLMStatus::Ok));
    assert!(second_result.is_sleep);
    assert_eq!(second_result.next_behavior.as_deref(), Some("idle"));

    let requests_guard = requests.lock().expect("requests lock");
    assert_eq!(requests_guard.len(), 2);
    let has_obs = requests_guard[1]
        .payload
        .messages
        .iter()
        .any(|m| m.content.contains("<<OBSERVATIONS (UNTRUSTED)>>"));
    assert!(has_obs);
}
