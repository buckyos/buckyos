use std::collections::HashSet;
use std::path::Path;

use async_trait::async_trait;
use log::{info, warn};
use serde::Deserialize;
use serde_json::{json, Value as Json};

use crate::agent_tool::{tokenize_bash_command_line, AgentTool, AgentToolError, ToolSpec};
use crate::behavior::SessionRuntimeContext;

use super::*;

const TOOL_TODO: &str = "todo";

const TODO_USAGE: &str = "\
todo <command> [args...]

Plan:
  todo clear
  todo add \"title\" [--type=Task|Bench] [--priority=N] [--deps=T001,T003|--no-deps]

Do:
  todo start  T001 [\"reason\"]
  todo done   T001 \"reason\"
  todo fail   T001 \"reason\" [--error='{\"code\":\"...\",\"message\":\"...\"}']

Check:
  todo pass   T001 [\"reason\"]
  todo reject T001 \"reason\"

Notes:
  todo note   T001 \"content\" [--kind=note|result|error]

Query:
  todo ls     [--all] [--status=WAIT,IN_PROGRESS] [--type=Task|Bench] [-q \"keyword\"]
  todo show   T001
  todo next
  todo pending [--status=WAIT,IN_PROGRESS]

Prompt:
  todo prompt [--budget=N]
  todo current [T001]

Global flags:
  --ws=<workspace_id> --session=<session_id> --agent=<agent_id> --op-id=<op_id>";

impl TodoTool {
    fn resolve_workspace_id_for_exec(
        &self,
        explicit_ws: Option<String>,
        ctx: &SessionRuntimeContext,
        shell_cwd: Option<&Path>,
    ) -> Result<String, AgentToolError> {
        if let Some(workspace_id) = explicit_ws {
            return Ok(workspace_id);
        }
        if let Some(workspace_id) = self.lookup_bound_workspace_id(ctx.session_id.as_str()) {
            return Ok(workspace_id);
        }
        if let Some(workspace_id) = extract_workspace_id_from_cwd(shell_cwd) {
            return Ok(workspace_id);
        }
        Err(AgentToolError::InvalidArgs(
            "workspace_id is required; use `--ws=<workspace_id>` or bind a workspace for this session"
                .to_string(),
        ))
    }

    fn lookup_bound_workspace_id(&self, session_id: &str) -> Option<String> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return None;
        }
        let workshop_root = self.cfg.db_path.parent()?.parent()?;
        let bindings_path = workshop_root.join("sessions/local_workspace_bindings.json");
        let raw = std::fs::read_to_string(bindings_path).ok()?;
        let parsed = serde_json::from_str::<TodoSessionBindingsFile>(&raw).ok()?;
        parsed
            .bindings
            .into_iter()
            .find(|item| item.session_id.trim() == session_id)
            .map(|item| item.local_workspace_id.trim().to_string())
            .filter(|item| !item.is_empty())
    }

    fn build_cli_actor(agent_id: &str, session_id: &str) -> Json {
        json!({
            "kind": "root_agent",
            "did": agent_id,
            "session_id": session_id
        })
    }

    fn build_apply_delta_args(
        workspace_id: &str,
        session_id: &str,
        agent_id: &str,
        op_id: Option<String>,
        ops: Vec<Json>,
    ) -> Json {
        let mut delta = serde_json::Map::new();
        if let Some(op_id) = op_id {
            delta.insert("op_id".to_string(), Json::String(op_id));
        }
        delta.insert("ops".to_string(), Json::Array(ops));

        json!({
            "action": "apply_delta",
            "workspace_id": workspace_id,
            "session_id": session_id,
            "actor_ctx": Self::build_cli_actor(agent_id, session_id),
            "delta": Json::Object(delta)
        })
    }

    fn exec_clear(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
        session_id: &str,
        agent_id: &str,
        op_id: Option<String>,
    ) -> Result<Json, AgentToolError> {
        cli_args.expect_no_positionals("clear")?;
        cli_args.ensure_allowed_flags("clear", &["ws", "session", "agent", "op-id"])?;
        Ok(Self::build_apply_delta_args(
            workspace_id,
            session_id,
            agent_id,
            op_id,
            vec![json!({
                "op": "init",
                "mode": "replace",
                "items": []
            })],
        ))
    }

    async fn exec_add(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
        session_id: &str,
        agent_id: &str,
        op_id: Option<String>,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags(
            "add",
            &[
                "type",
                "priority",
                "deps",
                "no-deps",
                "labels",
                "skills",
                "desc",
                "description",
                "assignee",
                "ws",
                "session",
                "agent",
                "op-id",
            ],
        )?;
        let title = cli_args.require_positional("add", 0, "title")?;
        if cli_args.positionals.len() > 1 {
            return Err(AgentToolError::InvalidArgs(
                "todo add only accepts one positional argument: title".to_string(),
            ));
        }

        let todo_type = cli_args
            .flag_string("type")?
            .unwrap_or_else(|| TodoType::Task.as_str().to_string());
        let todo_type = TodoType::parse(&todo_type)?;

        let no_deps = cli_args.flag_switch("no-deps")?;
        let deps_flag = cli_args.flag_string("deps")?;
        if no_deps && deps_flag.is_some() {
            return Err(AgentToolError::InvalidArgs(
                "todo add cannot use both --deps and --no-deps".to_string(),
            ));
        }

        let priority = if let Some(raw_priority) = cli_args.flag_string("priority")? {
            Some(parse_i64_flag("priority", &raw_priority)?)
        } else {
            let ws_for_db = workspace_id.to_string();
            Some(
                self.run_db("todo add default priority", move |conn| {
                    read_next_default_priority(conn, &ws_for_db)
                })
                .await?,
            )
        };

        let has_existing_todos = if !no_deps && deps_flag.is_none() && todo_type == TodoType::Task {
            let ws_for_db = workspace_id.to_string();
            self.run_db("todo add has existing", move |conn| {
                workspace_has_todos(conn, &ws_for_db)
            })
            .await?
        } else {
            false
        };

        let deps = if no_deps {
            Some(Vec::<String>::new())
        } else if let Some(raw_deps) = deps_flag {
            Some(parse_csv_codes("deps", &raw_deps)?)
        } else if todo_type == TodoType::Task && has_existing_todos {
            Some(vec!["@prev".to_string()])
        } else {
            None
        };

        let labels = cli_args
            .flag_string("labels")?
            .map(|value| parse_csv_tokens("labels", &value))
            .transpose()?
            .unwrap_or_default();
        let skills = cli_args
            .flag_string("skills")?
            .map(|value| parse_csv_tokens("skills", &value))
            .transpose()?
            .unwrap_or_default();
        let description = cli_args
            .flag_string("desc")?
            .or(cli_args.flag_string("description")?);
        let assignee = cli_args.flag_string("assignee")?;

        let mut item = serde_json::Map::new();
        item.insert("title".to_string(), Json::String(title));
        item.insert(
            "type".to_string(),
            Json::String(todo_type.as_str().to_string()),
        );
        if let Some(priority) = priority {
            item.insert("priority".to_string(), Json::Number(priority.into()));
        }
        if let Some(description) = description {
            item.insert("description".to_string(), Json::String(description));
        }
        if !labels.is_empty() {
            item.insert(
                "labels".to_string(),
                Json::Array(labels.into_iter().map(Json::String).collect()),
            );
        }
        if !skills.is_empty() {
            item.insert(
                "skills".to_string(),
                Json::Array(skills.into_iter().map(Json::String).collect()),
            );
        }
        if let Some(assignee) = assignee {
            item.insert("assignee".to_string(), Json::String(assignee));
        }
        if let Some(deps) = deps {
            item.insert(
                "deps".to_string(),
                Json::Array(deps.into_iter().map(Json::String).collect()),
            );
        }

        Ok(Self::build_apply_delta_args(
            workspace_id,
            session_id,
            agent_id,
            op_id,
            vec![json!({
                "op": "init",
                "mode": "merge",
                "items": [Json::Object(item)]
            })],
        ))
    }

    fn exec_update(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
        session_id: &str,
        agent_id: &str,
        op_id: Option<String>,
        to_status: TodoStatus,
        reason_required: bool,
        default_reason: Option<&str>,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags("update", &["ws", "session", "agent", "op-id", "error"])?;
        let todo_code = cli_args.require_positional("update", 0, "todo_code")?;
        let todo_code = normalize_todo_code(&todo_code)?;
        let reason = cli_args.positionals.get(1).cloned();
        if cli_args.positionals.len() > 2 {
            return Err(AgentToolError::InvalidArgs(
                "status update only supports: <todo_code> [reason]".to_string(),
            ));
        }

        let reason = match reason {
            Some(reason) if !reason.trim().is_empty() => reason.trim().to_string(),
            _ if reason_required => {
                return Err(AgentToolError::InvalidArgs(
                    "reason is required for this command".to_string(),
                ));
            }
            _ => default_reason.unwrap_or("updated").to_string(),
        };

        let mut op = serde_json::Map::new();
        op.insert(
            "op".to_string(),
            Json::String(format!("update:{todo_code}")),
        );
        op.insert(
            "to_status".to_string(),
            Json::String(to_status.as_str().to_string()),
        );
        op.insert("reason".to_string(), Json::String(reason));

        if let Some(error) = cli_args.flag_string("error")? {
            let parsed: Json = serde_json::from_str(error.as_str()).map_err(|err| {
                AgentToolError::InvalidArgs(format!("invalid --error json payload: {err}"))
            })?;
            op.insert("last_error".to_string(), parsed);
        }

        Ok(Self::build_apply_delta_args(
            workspace_id,
            session_id,
            agent_id,
            op_id,
            vec![Json::Object(op)],
        ))
    }

    fn exec_fail(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
        session_id: &str,
        agent_id: &str,
        op_id: Option<String>,
    ) -> Result<Json, AgentToolError> {
        self.exec_update(
            cli_args,
            workspace_id,
            session_id,
            agent_id,
            op_id,
            TodoStatus::Failed,
            true,
            None,
        )
    }

    fn exec_note(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
        session_id: &str,
        agent_id: &str,
        op_id: Option<String>,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags("note", &["kind", "ws", "session", "agent", "op-id"])?;
        if cli_args.positionals.len() != 2 {
            return Err(AgentToolError::InvalidArgs(
                "todo note requires exactly: <todo_code> <content>".to_string(),
            ));
        }
        let todo_code = normalize_todo_code(
            cli_args
                .positionals
                .first()
                .map(String::as_str)
                .unwrap_or_default(),
        )?;
        let content = cli_args
            .positionals
            .get(1)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                AgentToolError::InvalidArgs("note content cannot be empty".to_string())
            })?;
        let kind = cli_args
            .flag_string("kind")?
            .unwrap_or_else(|| "note".to_string());

        Ok(Self::build_apply_delta_args(
            workspace_id,
            session_id,
            agent_id,
            op_id,
            vec![json!({
                "op": format!("note:{todo_code}"),
                "kind": kind,
                "content": content
            })],
        ))
    }

    fn exec_list(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags(
            "ls",
            &[
                "all", "status", "type", "assignee", "label", "q", "query", "limit", "offset",
                "sort", "sort_by", "asc", "ws", "session", "agent", "op-id",
            ],
        )?;
        cli_args.expect_max_positionals("ls", 0)?;
        let all = cli_args.flag_switch("all")?;
        let statuses = if all {
            Vec::new()
        } else if let Some(raw_status) = cli_args.flag_string("status")? {
            parse_csv_tokens("status", &raw_status)?
        } else {
            vec![
                TodoStatus::Wait.as_str().to_string(),
                TodoStatus::InProgress.as_str().to_string(),
                TodoStatus::Complete.as_str().to_string(),
                TodoStatus::Failed.as_str().to_string(),
                TodoStatus::CheckFailed.as_str().to_string(),
            ]
        };

        let mut filters = serde_json::Map::new();
        if !statuses.is_empty() {
            filters.insert(
                "status".to_string(),
                Json::Array(statuses.into_iter().map(Json::String).collect()),
            );
        }
        if let Some(todo_type) = cli_args.flag_string("type")? {
            filters.insert("type".to_string(), Json::String(todo_type));
        }
        if let Some(assignee) = cli_args.flag_string("assignee")? {
            filters.insert("assignee".to_string(), Json::String(assignee));
        }
        if let Some(label) = cli_args.flag_string("label")? {
            filters.insert("label".to_string(), Json::String(label));
        }
        if let Some(query) = cli_args
            .flag_string("q")?
            .or(cli_args.flag_string("query")?)
        {
            filters.insert("query".to_string(), Json::String(query));
        }
        if let Some(sort_by) = cli_args
            .flag_string("sort")?
            .or(cli_args.flag_string("sort_by")?)
        {
            filters.insert("sort_by".to_string(), Json::String(sort_by));
        }
        if cli_args.flag_switch("asc")? {
            filters.insert("asc".to_string(), Json::Bool(true));
        }

        let mut args = serde_json::Map::new();
        args.insert("action".to_string(), Json::String("list".to_string()));
        args.insert(
            "workspace_id".to_string(),
            Json::String(workspace_id.to_string()),
        );
        if !filters.is_empty() {
            args.insert("filters".to_string(), Json::Object(filters));
        }
        if let Some(limit) = cli_args.flag_string("limit")? {
            args.insert(
                "limit".to_string(),
                Json::Number(parse_u64_flag("limit", &limit)?.into()),
            );
        }
        if let Some(offset) = cli_args.flag_string("offset")? {
            args.insert(
                "offset".to_string(),
                Json::Number(parse_u64_flag("offset", &offset)?.into()),
            );
        }
        Ok(Json::Object(args))
    }

    fn exec_show(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags("show", &["ws", "session", "agent", "op-id"])?;
        let todo_ref = cli_args.require_positional("show", 0, "todo_code")?;
        cli_args.expect_max_positionals("show", 1)?;
        Ok(json!({
            "action": "get",
            "workspace_id": workspace_id,
            "todo_ref": todo_ref
        }))
    }

    fn exec_next(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags("next", &["ws", "session", "agent", "op-id"])?;
        cli_args.expect_no_positionals("next")?;
        Ok(json!({
            "action": "get_next_ready_todo",
            "workspace_id": workspace_id,
            "session_id": session_id,
            "agent_id": agent_id
        }))
    }

    fn exec_pending(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags("pending", &["status", "ws", "session", "agent", "op-id"])?;
        cli_args.expect_no_positionals("pending")?;
        let mut args = serde_json::Map::new();
        args.insert(
            "action".to_string(),
            Json::String("query_pending".to_string()),
        );
        args.insert(
            "workspace_id".to_string(),
            Json::String(workspace_id.to_string()),
        );
        if let Some(status) = cli_args.flag_string("status")? {
            args.insert(
                "states".to_string(),
                Json::Array(
                    parse_csv_tokens("status", &status)?
                        .into_iter()
                        .map(Json::String)
                        .collect(),
                ),
            );
        }
        Ok(Json::Object(args))
    }

    fn exec_prompt(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags("prompt", &["budget", "ws", "session", "agent", "op-id"])?;
        cli_args.expect_no_positionals("prompt")?;
        let mut args = serde_json::Map::new();
        args.insert(
            "action".to_string(),
            Json::String("render_for_prompt".to_string()),
        );
        args.insert(
            "workspace_id".to_string(),
            Json::String(workspace_id.to_string()),
        );
        if let Some(budget) = cli_args.flag_string("budget")? {
            args.insert(
                "token_budget".to_string(),
                Json::Number(parse_u64_flag("budget", &budget)?.into()),
            );
        }
        Ok(Json::Object(args))
    }

    fn exec_current(
        &self,
        cli_args: &TodoCliArgs,
        workspace_id: &str,
        session_id: &str,
    ) -> Result<Json, AgentToolError> {
        cli_args.ensure_allowed_flags("current", &["ws", "session", "agent", "op-id"])?;
        cli_args.expect_max_positionals("current", 1)?;
        let todo_ref = cli_args.positionals.first().cloned();
        Ok(json!({
            "action": "render_current_details",
            "workspace_id": workspace_id,
            "session_id": session_id,
            "todo_ref": todo_ref
        }))
    }
}

#[async_trait]
impl AgentTool for TodoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_TODO.to_string(),
            description:
                "Workspace todo CLI with sqlite/oplog persistence and PDCA state guardrails."
                    .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "list",
                            "get",
                            "apply_delta",
                            "query_pending",
                            "render_for_prompt",
                            "render_current_details",
                            "get_next_ready_todo"
                        ]
                    },
                    "workspace_id": { "type": "string" },
                    "todo_ref": { "type": "string", "description": "todo_code like T001 or item id" },
                    "agent_id": { "type": "string" },
                    "filters": { "type": "object" },
                    "limit": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "states": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "token_budget": { "type": "integer", "minimum": 1 },
                    "session_id": { "type": "string" },
                    "delta": {
                        "type": "object",
                        "properties": {
                            "op_id": { "type": "string" },
                            "ops": {
                                "type": "array",
                                "items": { "type": "object" }
                            }
                        }
                    },
                    "actor_ctx": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "enum": ["root_agent", "sub_agent", "user", "system"] },
                            "did": { "type": "string" },
                            "session_id": { "type": "string" },
                            "trace_id": { "type": "string" }
                        }
                    }
                },
                "required": ["action"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "action": { "type": "string" },
                    "items": { "type": "array", "items": { "type": "object" } },
                    "item": { "type": "object" },
                    "notes": { "type": "array", "items": { "type": "object" } },
                    "deps": { "type": "array", "items": { "type": "string" } },
                    "version": { "type": "integer" },
                    "new_version": { "type": "integer" },
                    "before_version": { "type": "integer" },
                    "op_id": { "type": "string" },
                    "errors": { "type": "array", "items": { "type": "object" } },
                    "counts_by_status": { "type": "object" },
                    "has_pending": { "type": "boolean" },
                    "text": { "type": "string" }
                }
            }),
            usage: Some(TODO_USAGE.to_string()),
        }
    }

    fn support_bash(&self) -> bool {
        true
    }
    fn support_action(&self) -> bool {
        false
    }
    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "empty bash command line".to_string(),
            ));
        }
        if tokens.len() < 2 {
            return Err(AgentToolError::InvalidArgs(
                "missing todo subcommand".to_string(),
            ));
        }

        let subcommand = tokens[1].trim().to_lowercase();
        let cli_args = TodoCliArgs::parse(&tokens[2..])?;
        let workspace_id =
            self.resolve_workspace_id_for_exec(cli_args.flag_string("ws")?, ctx, shell_cwd)?;
        let session_id = cli_args
            .flag_string("session")?
            .unwrap_or_else(|| ctx.session_id.clone());
        let agent_id = cli_args
            .flag_string("agent")?
            .unwrap_or_else(|| ctx.agent_name.clone());
        let op_id = cli_args.flag_string("op-id")?;

        let args = match subcommand.as_str() {
            "clear" => self.exec_clear(&cli_args, &workspace_id, &session_id, &agent_id, op_id)?,
            "add" => {
                self.exec_add(&cli_args, &workspace_id, &session_id, &agent_id, op_id)
                    .await?
            }
            "start" => self.exec_update(
                &cli_args,
                &workspace_id,
                &session_id,
                &agent_id,
                op_id,
                TodoStatus::InProgress,
                false,
                Some("started"),
            )?,
            "done" => self.exec_update(
                &cli_args,
                &workspace_id,
                &session_id,
                &agent_id,
                op_id,
                TodoStatus::Complete,
                true,
                None,
            )?,
            "fail" => self.exec_fail(&cli_args, &workspace_id, &session_id, &agent_id, op_id)?,
            "pass" => self.exec_update(
                &cli_args,
                &workspace_id,
                &session_id,
                &agent_id,
                op_id,
                TodoStatus::Done,
                false,
                Some("verified"),
            )?,
            "reject" => self.exec_update(
                &cli_args,
                &workspace_id,
                &session_id,
                &agent_id,
                op_id,
                TodoStatus::CheckFailed,
                true,
                None,
            )?,
            "note" => self.exec_note(&cli_args, &workspace_id, &session_id, &agent_id, op_id)?,
            "ls" => self.exec_list(&cli_args, &workspace_id)?,
            "show" => self.exec_show(&cli_args, &workspace_id)?,
            "next" => self.exec_next(&cli_args, &workspace_id, &session_id, &agent_id)?,
            "pending" => self.exec_pending(&cli_args, &workspace_id)?,
            "prompt" => self.exec_prompt(&cli_args, &workspace_id)?,
            "current" => self.exec_current(&cli_args, &workspace_id, &session_id)?,
            "help" => {
                return Ok(json!({
                    "ok": true,
                    "tool": TOOL_TODO,
                    "usage": TODO_USAGE
                }));
            }
            _ => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported todo subcommand `{}`",
                    tokens[1]
                )));
            }
        };

        self.call(ctx, args).await
    }

    async fn call(&self, ctx: &SessionRuntimeContext, args: Json) -> Result<Json, AgentToolError> {
        let action = require_string(&args, "action")?;
        let workspace_id = args
            .get("workspace_id")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_string();
        let result = match action.as_str() {
            "list" => self.call_list(args).await,
            "get" => self.call_get(args).await,
            "apply_delta" => self.call_apply_delta(ctx, args).await,
            "query_pending" => self.call_query_pending(args).await,
            "render_for_prompt" => self.call_render_for_prompt(args).await,
            "render_current_details" => self.call_render_current_details(args).await,
            "get_next_ready_todo" => self.call_get_next_ready_todo(args).await,
            _ => Err(AgentToolError::InvalidArgs(format!(
                "unsupported action `{action}`, expected list/get/apply_delta/query_pending/render_for_prompt/render_current_details/get_next_ready_todo"
            ))),
        };

        match &result {
            Ok(_) => {
                info!(
                    "opendan.tool_call: tool={} status=success trace_id={} action={} workspace_id={}",
                    TOOL_TODO, ctx.trace_id, action, workspace_id
                );
            }
            Err(err) => {
                warn!(
                    "opendan.tool_call: tool={} status=failed trace_id={} action={} workspace_id={} err={}",
                    TOOL_TODO, ctx.trace_id, action, workspace_id, err
                );
            }
        }

        result
    }
}

impl TodoTool {
    async fn call_list(&self, args: Json) -> Result<Json, AgentToolError> {
        let workspace_id = require_workspace_id(&args)?;
        let filters = TodoListFilters::from_args(&args)?;
        let limit = optional_u64(&args, "limit")?
            .map(|v| u64_to_usize(v, "limit"))
            .transpose()?
            .unwrap_or(self.cfg.default_list_limit)
            .clamp(1, self.cfg.max_list_limit);
        let offset = optional_u64(&args, "offset")?
            .map(|v| u64_to_usize(v, "offset"))
            .transpose()?
            .unwrap_or(0);

        let workspace_id_for_db = workspace_id.clone();
        let rows = self
            .run_db("todo list", move |conn| {
                list_todo_items(conn, &workspace_id_for_db, &filters, limit, offset)
            })
            .await?;

        let version_ws = workspace_id.clone();
        let version = self
            .run_db("todo read version", move |conn| {
                read_workspace_version(conn, &version_ws)
            })
            .await?;

        Ok(json!({
            "ok": true,
            "action": "list",
            "workspace_id": workspace_id,
            "items": rows,
            "total": rows.len(),
            "version": version
        }))
    }

    async fn call_get(&self, args: Json) -> Result<Json, AgentToolError> {
        let workspace_id = require_workspace_id(&args)?;
        let todo_ref = require_string(&args, "todo_ref")?;

        let ws_for_db = workspace_id.clone();
        let ref_for_db = todo_ref.clone();
        let detail = self
            .run_db("todo get", move |conn| {
                get_todo_detail(conn, &ws_for_db, &ref_for_db, MAX_NOTES_FETCH)
            })
            .await?;

        let Some(detail) = detail else {
            return Err(AgentToolError::InvalidArgs(format!(
                "todo `{todo_ref}` not found in workspace `{workspace_id}`"
            )));
        };

        let version_ws = workspace_id.clone();
        let version = self
            .run_db("todo read version", move |conn| {
                read_workspace_version(conn, &version_ws)
            })
            .await?;

        Ok(json!({
            "ok": true,
            "action": "get",
            "workspace_id": workspace_id,
            "item": detail.item,
            "notes": detail.notes,
            "deps": detail.dep_codes,
            "version": version
        }))
    }

    async fn call_apply_delta(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<Json, AgentToolError> {
        let input = ApplyDeltaInput::from_args(ctx, &args)?;
        let oplog_path = self.oplog_path.clone();
        let rsp = self
            .run_db("todo apply delta", move |conn| {
                apply_todo_delta(conn, &oplog_path, input)
            })
            .await?;

        self.publish_todo_status_events(&rsp.status_events).await;

        Ok(json!({
            "ok": rsp.ok,
            "action": "apply_delta",
            "workspace_id": rsp.workspace_id,
            "op_id": rsp.op_id,
            "before_version": rsp.before_version,
            "new_version": rsp.new_version,
            "idempotent": rsp.idempotent,
            "errors": rsp.errors,
            "applied_count": rsp.applied_count,
        }))
    }

    async fn publish_todo_status_events(&self, events: &[TodoStatusChangedEvent]) {
        for event in events {
            let event_id =
                build_todo_status_eventid(event.workspace_id.as_str(), event.todo_code.as_str());
            let payload = json!({
                "workspace_id": event.workspace_id,
                "todo_id": event.todo_id,
                "todo_code": event.todo_code,
                "from_status": event.from_status,
                "to_status": event.to_status,
                "updated_at": event.updated_at,
                "op_id": event.op_id,
                "actor_kind": event.actor_kind,
                "actor_did": event.actor_did,
                "session_id": event.session_id,
                "trace_id": event.trace_id,
            });

            if let Err(err) = self
                .kevent_client
                .pub_event(event_id.as_str(), payload)
                .await
            {
                warn!(
                    "todo.pub_status_event_failed: event_id={} workspace_id={} todo_id={} err={}",
                    event_id, event.workspace_id, event.todo_id, err
                );
            }
        }
    }

    async fn call_query_pending(&self, args: Json) -> Result<Json, AgentToolError> {
        let workspace_id = require_workspace_id(&args)?;
        let states = parse_status_set(args.get("states"))?;
        let ws_for_db = workspace_id.clone();
        let status_counts = self
            .run_db("todo query pending", move |conn| {
                query_pending_counts(conn, &ws_for_db)
            })
            .await?;

        let states_to_check = if states.is_empty() {
            vec![
                TodoStatus::Wait,
                TodoStatus::InProgress,
                TodoStatus::Complete,
                TodoStatus::CheckFailed,
            ]
        } else {
            states.into_iter().collect::<Vec<_>>()
        };

        let has_pending = states_to_check
            .iter()
            .any(|status| status_counts.get(status.as_str()).copied().unwrap_or(0) > 0);

        Ok(json!({
            "ok": true,
            "action": "query_pending",
            "workspace_id": workspace_id,
            "has_pending": has_pending,
            "counts_by_status": status_counts
        }))
    }

    async fn call_render_for_prompt(&self, args: Json) -> Result<Json, AgentToolError> {
        let workspace_id = require_workspace_id(&args)?;
        let token_budget = optional_u64(&args, "token_budget")?
            .map(|v| u64_to_usize(v, "token_budget"))
            .transpose()?
            .unwrap_or(DEFAULT_TOKEN_BUDGET);

        let ws_for_db = workspace_id.clone();
        let items = self
            .run_db("todo render for prompt", move |conn| {
                list_for_prompt(conn, &ws_for_db, RENDER_ITEM_LIMIT)
            })
            .await?;

        let version_ws = workspace_id.clone();
        let version = self
            .run_db("todo read version", move |conn| {
                read_workspace_version(conn, &version_ws)
            })
            .await?;

        let text = render_workspace_todo_text(&workspace_id, version, &items, token_budget);
        Ok(json!({
            "ok": true,
            "action": "render_for_prompt",
            "workspace_id": workspace_id,
            "version": version,
            "text": text
        }))
    }

    async fn call_render_current_details(&self, args: Json) -> Result<Json, AgentToolError> {
        let workspace_id = require_workspace_id(&args)?;
        let session_id = optional_string(&args, "session_id")?;
        let todo_ref = optional_string(&args, "todo_ref")?;

        let ws_for_db = workspace_id.clone();
        let sid_for_db = session_id.clone();
        let todo_ref_for_db = todo_ref.clone();
        let detail = self
            .run_db("todo render current details", move |conn| {
                select_current_todo_details(
                    conn,
                    &ws_for_db,
                    sid_for_db.as_deref(),
                    todo_ref_for_db.as_deref(),
                )
            })
            .await?;

        let text = if let Some(detail) = detail {
            render_current_todo_text(&detail)
        } else {
            "No active todo found for current context.".to_string()
        };

        Ok(json!({
            "ok": true,
            "action": "render_current_details",
            "workspace_id": workspace_id,
            "session_id": session_id,
            "todo_ref": todo_ref,
            "text": text
        }))
    }

    async fn call_get_next_ready_todo(&self, args: Json) -> Result<Json, AgentToolError> {
        let workspace_id = require_workspace_id(&args)?;
        let session_id = require_string(&args, "session_id")?;
        let agent_id = require_string(&args, "agent_id")?;

        let ws_for_db = workspace_id.clone();
        let sid_for_db = session_id.clone();
        let aid_for_db = agent_id.clone();
        let detail = self
            .run_db("todo get next ready", move |conn| {
                get_next_ready_todo(
                    conn,
                    ws_for_db.as_str(),
                    sid_for_db.as_str(),
                    aid_for_db.as_str(),
                )
            })
            .await?;

        let version_ws = workspace_id.clone();
        let version = self
            .run_db("todo read version", move |conn| {
                read_workspace_version(conn, &version_ws)
            })
            .await?;

        if let Some(detail) = detail {
            return Ok(json!({
                "ok": true,
                "action": "get_next_ready_todo",
                "workspace_id": workspace_id,
                "session_id": session_id,
                "agent_id": agent_id,
                "item": detail.item,
                "notes": detail.notes,
                "deps": detail.dep_codes,
                "version": version
            }));
        }

        Ok(json!({
            "ok": true,
            "action": "get_next_ready_todo",
            "workspace_id": workspace_id,
            "session_id": session_id,
            "agent_id": agent_id,
            "item": Json::Null,
            "notes": [],
            "deps": [],
            "version": version
        }))
    }
}

#[derive(Clone, Debug, Default)]
struct TodoCliArgs {
    positionals: Vec<String>,
    flags: HashMap<String, Option<String>>,
}

impl TodoCliArgs {
    fn parse(tokens: &[String]) -> Result<Self, AgentToolError> {
        let mut parsed = Self::default();
        let mut idx = 0usize;
        while idx < tokens.len() {
            let token = tokens[idx].trim().to_string();
            if token.is_empty() {
                idx += 1;
                continue;
            }
            if token == "-q" {
                let value = tokens.get(idx + 1).ok_or_else(|| {
                    AgentToolError::InvalidArgs("`-q` requires a query value".to_string())
                })?;
                parsed.insert_flag("q".to_string(), Some(value.trim().to_string()))?;
                idx += 2;
                continue;
            }
            if let Some(raw_query) = token.strip_prefix("-q=") {
                parsed.insert_flag("q".to_string(), Some(raw_query.trim().to_string()))?;
                idx += 1;
                continue;
            }
            if let Some(raw_flag) = token.strip_prefix("--") {
                if raw_flag.is_empty() {
                    return Err(AgentToolError::InvalidArgs(
                        "invalid empty flag `--`".to_string(),
                    ));
                }
                if let Some((raw_key, raw_value)) = raw_flag.split_once('=') {
                    let key = raw_key.trim().to_string();
                    if key.is_empty() {
                        return Err(AgentToolError::InvalidArgs(
                            "flag key cannot be empty".to_string(),
                        ));
                    }
                    parsed.insert_flag(key, Some(raw_value.trim().to_string()))?;
                    idx += 1;
                    continue;
                }

                let key = raw_flag.trim().to_string();
                if key.is_empty() {
                    return Err(AgentToolError::InvalidArgs(
                        "flag key cannot be empty".to_string(),
                    ));
                }
                if let Some(next) = tokens.get(idx + 1) {
                    if !next.starts_with('-') {
                        parsed.insert_flag(key, Some(next.trim().to_string()))?;
                        idx += 2;
                        continue;
                    }
                }
                parsed.insert_flag(key, None)?;
                idx += 1;
                continue;
            }
            if token.starts_with('-') {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported short flag `{token}`"
                )));
            }

            parsed.positionals.push(token);
            idx += 1;
        }
        Ok(parsed)
    }

    fn insert_flag(&mut self, key: String, value: Option<String>) -> Result<(), AgentToolError> {
        if self.flags.contains_key(key.as_str()) {
            return Err(AgentToolError::InvalidArgs(format!(
                "duplicated flag `--{key}`"
            )));
        }
        self.flags.insert(key, value);
        Ok(())
    }

    fn ensure_allowed_flags(&self, command: &str, allowed: &[&str]) -> Result<(), AgentToolError> {
        let allowed: HashSet<&str> = allowed.iter().copied().collect();
        for key in self.flags.keys() {
            if !allowed.contains(key.as_str()) {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported flag `--{}` for `{command}`",
                    key
                )));
            }
        }
        Ok(())
    }

    fn flag_string(&self, key: &str) -> Result<Option<String>, AgentToolError> {
        match self.flags.get(key) {
            None => Ok(None),
            Some(Some(value)) => {
                let value = value.trim();
                if value.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(value.to_string()))
                }
            }
            Some(None) => Err(AgentToolError::InvalidArgs(format!(
                "flag `--{key}` requires a value"
            ))),
        }
    }

    fn flag_switch(&self, key: &str) -> Result<bool, AgentToolError> {
        match self.flags.get(key) {
            None => Ok(false),
            Some(None) => Ok(true),
            Some(Some(_)) => Err(AgentToolError::InvalidArgs(format!(
                "flag `--{key}` does not take a value"
            ))),
        }
    }

    fn require_positional(
        &self,
        command: &str,
        index: usize,
        name: &str,
    ) -> Result<String, AgentToolError> {
        self.positionals
            .get(index)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(format!(
                    "`todo {command}` missing required argument `{name}`"
                ))
            })
    }

    fn expect_no_positionals(&self, command: &str) -> Result<(), AgentToolError> {
        self.expect_max_positionals(command, 0)
    }

    fn expect_max_positionals(&self, command: &str, max: usize) -> Result<(), AgentToolError> {
        if self.positionals.len() <= max {
            return Ok(());
        }
        Err(AgentToolError::InvalidArgs(format!(
            "`todo {command}` accepts at most {max} positional arguments"
        )))
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
struct TodoSessionBindingsFile {
    bindings: Vec<TodoSessionBinding>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
struct TodoSessionBinding {
    session_id: String,
    local_workspace_id: String,
}

fn parse_u64_flag(name: &str, raw: &str) -> Result<u64, AgentToolError> {
    raw.trim().parse::<u64>().map_err(|err| {
        AgentToolError::InvalidArgs(format!("invalid --{name} value `{}`: {err}", raw.trim()))
    })
}

fn parse_i64_flag(name: &str, raw: &str) -> Result<i64, AgentToolError> {
    raw.trim().parse::<i64>().map_err(|err| {
        AgentToolError::InvalidArgs(format!("invalid --{name} value `{}`: {err}", raw.trim()))
    })
}

fn parse_csv_tokens(name: &str, raw: &str) -> Result<Vec<String>, AgentToolError> {
    let mut out = Vec::new();
    for item in raw.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if !out.iter().any(|existing| existing == item) {
            out.push(item.to_string());
        }
    }
    if out.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "`--{name}` cannot be empty"
        )));
    }
    Ok(out)
}

fn parse_csv_codes(name: &str, raw: &str) -> Result<Vec<String>, AgentToolError> {
    let mut out = Vec::new();
    for item in parse_csv_tokens(name, raw)? {
        out.push(normalize_todo_code(item.as_str())?);
    }
    Ok(out)
}

fn extract_workspace_id_from_cwd(shell_cwd: Option<&Path>) -> Option<String> {
    let shell_cwd = shell_cwd?;
    let components: Vec<String> = shell_cwd
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => part.to_str().map(|v| v.to_string()),
            _ => None,
        })
        .collect();
    if components.len() < 3 {
        return None;
    }
    for idx in 0..(components.len() - 2) {
        if components[idx] == "workspaces" && components[idx + 1] == "local" {
            let workspace_id = components[idx + 2].trim().to_string();
            if !workspace_id.is_empty() {
                return Some(workspace_id);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn test_ctx(agent_did: &str) -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace-test".to_string(),
            agent_name: agent_did.to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-test".to_string(),
            session_id: "sess-demo".to_string(),
        }
    }

    async fn call(
        tool: &TodoTool,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<Json, AgentToolError> {
        tool.call(ctx, args).await
    }

    async fn exec(
        tool: &TodoTool,
        ctx: &SessionRuntimeContext,
        line: &str,
    ) -> Result<Json, AgentToolError> {
        tool.exec(ctx, line, None).await
    }

    fn tool_for_test() -> TodoTool {
        let root = std::env::temp_dir().join(format!("opendan-todo-{}", generate_id("test")));
        std::fs::create_dir_all(&root).expect("create test root");
        let db_path = root.join("todo").join("todo.db");
        TodoTool::new(TodoToolConfig::with_db_path(db_path)).expect("create todo tool")
    }

    fn write_session_binding(tool: &TodoTool, session_id: &str, workspace_id: &str) {
        let workshop_root = tool
            .cfg
            .db_path
            .parent()
            .and_then(|parent| parent.parent())
            .expect("workshop root from db path");
        let bindings_path = workshop_root.join("sessions/local_workspace_bindings.json");
        std::fs::create_dir_all(bindings_path.parent().expect("bindings parent"))
            .expect("create sessions dir");
        let payload = json!({
            "bindings": [
                {
                    "session_id": session_id,
                    "local_workspace_id": workspace_id,
                    "workspace_path": format!("/tmp/{workspace_id}"),
                    "bound_at_ms": 1
                }
            ]
        });
        std::fs::write(
            bindings_path,
            serde_json::to_vec(&payload).expect("serialize bindings"),
        )
        .expect("write bindings");
    }

    #[tokio::test]
    async fn apply_init_replace_assigns_codes_and_order() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");

        let result = call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-alpha",
                "actor_ctx": { "kind": "root_agent", "did": "did:od:jarvis", "session_id": "sess-a" },
                "delta": {
                    "ops": [
                        {
                            "op": "init",
                            "mode": "replace",
                            "items": [
                                { "title": "setup env", "type": "Task", "priority": 0 },
                                { "title": "integration bench", "type": "Bench", "priority": 10 }
                            ]
                        }
                    ]
                }
            }),
        )
        .await
        .expect("apply init");
        assert_eq!(result["ok"], true);

        let listed = call(
            &tool,
            &ctx,
            json!({
                "action": "list",
                "workspace_id": "ws-alpha",
                "filters": { "sort_by": "order", "asc": true }
            }),
        )
        .await
        .expect("list todos");

        let items = listed["items"].as_array().expect("items array");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["todo_code"], "T001");
        assert_eq!(items[1]["todo_code"], "T002");
        assert_eq!(items[0]["status"], "WAIT");
        assert_eq!(items[1]["status"], "WAIT");
    }

    #[tokio::test]
    async fn apply_update_supports_pdca_and_bench_special() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");

        call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-beta",
                "delta": {
                    "ops": [{
                        "op": "init",
                        "mode": "replace",
                        "items": [
                            { "title": "build", "type": "Task" },
                            { "title": "bench", "type": "Bench" }
                        ]
                    }]
                }
            }),
        )
        .await
        .expect("init");

        let invalid = call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-beta",
                "delta": {
                    "ops": [{
                        "op": "update:T001",
                        "to_status": "DONE",
                        "reason": "try check before complete"
                    }]
                }
            }),
        )
        .await
        .expect("apply should return domain error payload");
        assert_eq!(invalid["ok"], false);
        assert_eq!(invalid["errors"][0]["code"], "INVALID_TRANSITION");

        let bench_done = call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-beta",
                "delta": {
                    "ops": [{
                        "op": "update:T002",
                        "to_status": "DONE",
                        "reason": "bench check passed"
                    }]
                }
            }),
        )
        .await
        .expect("bench update");
        assert_eq!(bench_done["ok"], true);

        let got = call(
            &tool,
            &ctx,
            json!({
                "action": "get",
                "workspace_id": "ws-beta",
                "todo_ref": "T002"
            }),
        )
        .await
        .expect("get todo");
        assert_eq!(got["item"]["status"], "DONE");
    }

    #[tokio::test]
    async fn apply_note_appends_without_overwrite() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");

        call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-note",
                "delta": {
                    "ops": [{
                        "op": "init",
                        "mode": "replace",
                        "items": [{ "title": "write notes", "type": "Task" }]
                    }]
                }
            }),
        )
        .await
        .expect("init");

        call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-note",
                "delta": {
                    "ops": [
                        { "op": "note:T001", "kind": "result", "content": "first note" },
                        { "op": "note:T001", "kind": "note", "content": "second note" }
                    ]
                }
            }),
        )
        .await
        .expect("append notes");

        let got = call(
            &tool,
            &ctx,
            json!({
                "action": "get",
                "workspace_id": "ws-note",
                "todo_ref": "T001"
            }),
        )
        .await
        .expect("get todo");

        let notes = got["notes"].as_array().expect("notes array");
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().any(|n| n["content"] == "first note"));
        assert!(notes.iter().any(|n| n["content"] == "second note"));
    }

    #[tokio::test]
    async fn subagent_can_only_update_owned_todo() {
        let tool = tool_for_test();
        let root_ctx = test_ctx("did:od:jarvis");

        call(
            &tool,
            &root_ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-sub",
                "actor_ctx": { "kind": "root_agent", "did": "did:od:jarvis" },
                "delta": {
                    "ops": [{
                        "op": "init",
                        "mode": "replace",
                        "items": [
                            { "title": "task a", "type": "Task", "assignee": "did:od:alice" },
                            { "title": "task b", "type": "Task", "assignee": "did:od:bob" }
                        ]
                    }]
                }
            }),
        )
        .await
        .expect("init");

        let sub_ctx = test_ctx("did:od:bob");
        let forbidden = call(
            &tool,
            &sub_ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-sub",
                "actor_ctx": { "kind": "sub_agent", "did": "did:od:bob" },
                "delta": {
                    "ops": [{
                        "op": "update:T001",
                        "to_status": "COMPLETE",
                        "reason": "should fail"
                    }]
                }
            }),
        )
        .await
        .expect("apply should return forbidden payload");

        assert_eq!(forbidden["ok"], false);
        assert_eq!(forbidden["errors"][0]["code"], "FORBIDDEN");

        let allowed = call(
            &tool,
            &sub_ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-sub",
                "actor_ctx": { "kind": "sub_agent", "did": "did:od:bob" },
                "delta": {
                    "ops": [{
                        "op": "update:T002",
                        "to_status": "IN_PROGRESS",
                        "reason": "owned task"
                    }]
                }
            }),
        )
        .await
        .expect("owned update");
        assert_eq!(allowed["ok"], true);
    }

    #[tokio::test]
    async fn apply_op_id_is_idempotent() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");

        call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-idem",
                "delta": {
                    "ops": [{
                        "op": "init",
                        "mode": "replace",
                        "items": [{ "title": "idempotent task", "type": "Task" }]
                    }]
                }
            }),
        )
        .await
        .expect("init");

        let op_id = "op-fixed-001";
        let first = call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-idem",
                "delta": {
                    "op_id": op_id,
                    "ops": [{
                        "op": "update:T001",
                        "to_status": "IN_PROGRESS",
                        "reason": "start"
                    }]
                }
            }),
        )
        .await
        .expect("first apply");
        assert_eq!(first["ok"], true);
        let version_after_first = first["new_version"].as_i64().unwrap_or_default();

        let second = call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-idem",
                "delta": {
                    "op_id": op_id,
                    "ops": [{
                        "op": "update:T001",
                        "to_status": "IN_PROGRESS",
                        "reason": "start again"
                    }]
                }
            }),
        )
        .await
        .expect("second apply");
        assert_eq!(second["ok"], true);
        assert_eq!(second["idempotent"], true);
        assert_eq!(
            second["new_version"].as_i64().unwrap_or_default(),
            version_after_first
        );
    }

    #[tokio::test]
    async fn todo_cli_exec_supports_plan_do_check_with_implicit_workspace() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");
        write_session_binding(&tool, "sess-demo", "ws-cli");

        exec(&tool, &ctx, "todo clear").await.expect("todo clear");
        exec(&tool, &ctx, "todo add \"prepare env\"")
            .await
            .expect("todo add t001");
        exec(&tool, &ctx, "todo add \"implement api\"")
            .await
            .expect("todo add t002");

        let t002 = call(
            &tool,
            &ctx,
            json!({
                "action": "get",
                "workspace_id": "ws-cli",
                "todo_ref": "T002"
            }),
        )
        .await
        .expect("get T002");
        assert_eq!(t002["deps"], json!(["T001"]));
        assert_eq!(t002["item"]["priority"], 20);

        let next = exec(&tool, &ctx, "todo next").await.expect("todo next");
        assert_eq!(next["item"]["todo_code"], "T001");

        exec(&tool, &ctx, "todo start T001")
            .await
            .expect("todo start");
        exec(&tool, &ctx, "todo done T001 \"implemented\"")
            .await
            .expect("todo done");
        exec(&tool, &ctx, "todo pass T001")
            .await
            .expect("todo pass");

        let t001 = call(
            &tool,
            &ctx,
            json!({
                "action": "get",
                "workspace_id": "ws-cli",
                "todo_ref": "T001"
            }),
        )
        .await
        .expect("get T001");
        assert_eq!(t001["item"]["status"], "DONE");
    }

    #[tokio::test]
    async fn todo_cli_exec_no_deps_overrides_default_chain() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");
        write_session_binding(&tool, "sess-demo", "ws-nodeps");

        exec(&tool, &ctx, "todo clear").await.expect("clear");
        exec(&tool, &ctx, "todo add \"task A\"")
            .await
            .expect("add A");
        exec(&tool, &ctx, "todo add \"task B\" --no-deps")
            .await
            .expect("add B");

        let t002 = call(
            &tool,
            &ctx,
            json!({
                "action": "get",
                "workspace_id": "ws-nodeps",
                "todo_ref": "T002"
            }),
        )
        .await
        .expect("get T002");
        assert_eq!(t002["deps"], json!([]));
    }

    #[tokio::test]
    async fn render_for_prompt_complex_todos_with_deps_assignees_and_statuses() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");

        let init = call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-render-complex",
                "actor_ctx": { "kind": "root_agent", "did": "did:od:jarvis", "session_id": "sess-render" },
                "delta": {
                    "ops": [{
                        "op": "init",
                        "mode": "replace",
                        "items": [
                            { "title": "analysis design", "type": "Task", "assignee": "did:od:alice", "priority": 20 },
                            { "title": "implement feature", "type": "Task", "assignee": "did:od:bob", "priority": 30, "deps": ["T001"] },
                            { "title": "write docs", "type": "Task", "assignee": "did:od:alice", "priority": 5 },
                            { "title": "integration tests", "type": "Task", "assignee": "did:od:carol", "priority": 1, "deps": ["T001"] },
                            { "title": "fix flaky ci", "type": "Task", "assignee": "did:od:bob", "priority": 40 },
                            { "title": "benchmark happy path", "type": "Bench", "assignee": "did:od:alice", "priority": 10, "deps": ["T002"] },
                            { "title": "benchmark regression", "type": "Bench", "assignee": "did:od:dave", "priority": 15 },
                            { "title": "cleanup backlog", "type": "Task" }
                        ]
                    }]
                }
            }),
        )
        .await
        .expect("init complex todos");
        assert_eq!(init["ok"], true);

        let updates = call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-render-complex",
                "delta": {
                    "ops": [
                        { "op": "update:T001", "to_status": "IN_PROGRESS", "reason": "started analysis" },
                        { "op": "update:T002", "to_status": "COMPLETE", "reason": "implementation completed" },
                        { "op": "update:T005", "to_status": "FAILED", "reason": "ci check broken" },
                        { "op": "update:T006", "to_status": "DONE", "reason": "bench pass" },
                        { "op": "update:T007", "to_status": "CHECK_FAILED", "reason": "bench mismatch" }
                    ]
                }
            }),
        )
        .await
        .expect("update complex statuses");
        assert_eq!(updates["ok"], true);

        let t006 = call(
            &tool,
            &ctx,
            json!({
                "action": "get",
                "workspace_id": "ws-render-complex",
                "todo_ref": "T006"
            }),
        )
        .await
        .expect("get T006");
        assert_eq!(t006["deps"], json!(["T002"]));

        let rendered = call(
            &tool,
            &ctx,
            json!({
                "action": "render_for_prompt",
                "workspace_id": "ws-render-complex",
                "token_budget": 4096
            }),
        )
        .await
        .expect("render for prompt");
        assert_eq!(rendered["ok"], true);

        let text = rendered["text"].as_str().unwrap_or_default();
        assert!(text.starts_with("Workspace Todo (ws-render-complex, v"));
        assert!(text.contains("- T001 [IN_PROGRESS] assignee=did:od:alice p=20 analysis design"));
        assert!(text.contains("- T004 [WAIT] assignee=did:od:carol p=1 integration tests"));
        assert!(text.contains("- T003 [WAIT] assignee=did:od:alice p=5 write docs"));
        assert!(text.contains("- T008 [WAIT] assignee=did:od:jarvis p=- cleanup backlog"));
        assert!(text.contains("- T002 [COMPLETE] assignee=did:od:bob p=30 implement feature"));
        assert!(
            text.contains("- T007 [CHECK_FAILED] assignee=did:od:dave p=15 benchmark regression")
        );
        assert!(text.contains("- T005 [FAILED] assignee=did:od:bob p=40 fix flaky ci"));
        assert!(text.contains("- T006 [DONE] assignee=did:od:alice p=10 benchmark happy path"));

        let pos_t001 = text.find("- T001 [IN_PROGRESS]").expect("T001 position");
        let pos_t004 = text.find("- T004 [WAIT]").expect("T004 position");
        let pos_t003 = text.find("- T003 [WAIT]").expect("T003 position");
        let pos_t008 = text.find("- T008 [WAIT]").expect("T008 position");
        let pos_t002 = text.find("- T002 [COMPLETE]").expect("T002 position");
        let pos_t007 = text.find("- T007 [CHECK_FAILED]").expect("T007 position");
        let pos_t005 = text.find("- T005 [FAILED]").expect("T005 position");
        let pos_t006 = text.find("- T006 [DONE]").expect("T006 position");

        assert!(pos_t001 < pos_t004);
        assert!(pos_t004 < pos_t003);
        assert!(pos_t003 < pos_t008);
        assert!(pos_t008 < pos_t002);
        assert!(pos_t002 < pos_t007);
        assert!(pos_t007 < pos_t005);
        assert!(pos_t005 < pos_t006);
    }

    #[tokio::test]
    async fn get_next_ready_todo_respects_dep_and_newest_first() {
        let tool = tool_for_test();
        let ctx = test_ctx("did:od:jarvis");

        call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-ready",
                "actor_ctx": { "kind": "root_agent", "did": "did:od:jarvis", "session_id": "sess-ready" },
                "delta": {
                    "ops": [{
                        "op": "init",
                        "mode": "replace",
                        "items": [
                            { "title": "base task", "type": "Task", "assignee": "did:od:alice" },
                            { "title": "dep task", "type": "Task", "assignee": "did:od:alice", "deps": ["T001"] },
                            { "title": "newest task", "type": "Task", "assignee": "did:od:alice" }
                        ]
                    }]
                }
            }),
        )
        .await
        .expect("init");

        let first = call(
            &tool,
            &ctx,
            json!({
                "action": "get_next_ready_todo",
                "workspace_id": "ws-ready",
                "session_id": "sess-ready",
                "agent_id": "did:od:alice"
            }),
        )
        .await
        .expect("first next ready");
        assert_eq!(first["item"]["todo_code"], "T003");

        call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-ready",
                "delta": {
                    "ops": [{
                        "op": "update:T003",
                        "to_status": "IN_PROGRESS",
                        "reason": "start newest"
                    }]
                }
            }),
        )
        .await
        .expect("update t003");

        let second = call(
            &tool,
            &ctx,
            json!({
                "action": "get_next_ready_todo",
                "workspace_id": "ws-ready",
                "session_id": "sess-ready",
                "agent_id": "did:od:alice"
            }),
        )
        .await
        .expect("second next ready");
        assert_eq!(second["item"]["todo_code"], "T001");

        call(
            &tool,
            &ctx,
            json!({
                "action": "apply_delta",
                "workspace_id": "ws-ready",
                "delta": {
                    "ops": [{
                        "op": "update:T001",
                        "to_status": "COMPLETE",
                        "reason": "dep satisfied"
                    }]
                }
            }),
        )
        .await
        .expect("update t001");

        let third = call(
            &tool,
            &ctx,
            json!({
                "action": "get_next_ready_todo",
                "workspace_id": "ws-ready",
                "session_id": "sess-ready",
                "agent_id": "did:od:alice"
            }),
        )
        .await
        .expect("third next ready");
        assert_eq!(third["item"]["todo_code"], "T002");
    }
}
