use std::env;
use std::path::Path;

use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, BuckyOSRuntimeType,
    WorkflowCreateScheduledTaskReq, WorkflowGetScheduledTaskHistoryReq,
    WorkflowListScheduledTasksReq, WorkflowOwner, WorkflowRunScheduledTaskNowReq,
    WorkflowScheduledTask, WorkflowScheduledTaskFireRecord, WorkflowScheduledTaskMisfirePolicy,
    WorkflowScheduledTaskPolicy, WorkflowScheduledTaskSchedule, WorkflowScheduledTaskStatus,
    WorkflowScheduledTaskTarget, WorkflowServiceClient, WorkflowValidateScheduledTaskReq,
    WorkflowValidateScheduledTaskResult,
};
use chrono::{DateTime, Utc};
use kRPC::kRPC;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as Json};
use sha2::{Digest, Sha256};

use crate::{
    AgentToolError, CallingConventions, CliInvocation, RuntimeContext, ToolCtx, TypedTool,
};

pub const TOOL_DCRONTAB: &str = "dcrontab";

#[derive(Clone, Debug)]
pub struct DcrontabTool;

impl DcrontabTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DcrontabTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DcrontabArgs {
    command: DcrontabCommand,
    #[serde(default)]
    cmd_line: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum DcrontabCommand {
    Add(AddArgs),
    List(ListArgs),
    Show(IdArgs),
    Pause(IdArgs),
    Resume(IdArgs),
    Remove(IdArgs),
    RunNow(RunNowArgs),
    Validate(ValidateArgs),
    History(HistoryArgs),
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct AddArgs {
    trigger: TriggerArgs,
    target: TargetArgs,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    misfire: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct ValidateArgs {
    trigger: TriggerArgs,
    #[serde(default)]
    target: Option<TargetArgs>,
    #[serde(default)]
    misfire: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct ListArgs {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct IdArgs {
    id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct RunNowArgs {
    id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct HistoryArgs {
    id: String,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum TriggerArgs {
    Cron {
        expr: String,
        #[serde(default)]
        timezone: Option<String>,
    },
    Once {
        run_at: i64,
        #[serde(default)]
        timezone: Option<String>,
    },
    RunEvery {
        every_sec: u64,
        #[serde(default)]
        start_at: Option<i64>,
        #[serde(default)]
        end_at: Option<i64>,
        #[serde(default)]
        timezone: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum TargetArgs {
    Remind {
        text: String,
        #[serde(default)]
        to: Option<String>,
    },
    Task {
        title: String,
        objective: String,
        workspace_id: String,
        #[serde(default)]
        behavior: Option<String>,
        #[serde(default)]
        agent: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DcrontabOutput {
    #[serde(skip_serializing)]
    command: String,
    #[serde(skip_serializing)]
    summary: String,
    #[serde(flatten)]
    detail: JsonMap<String, Json>,
}

#[async_trait]
impl TypedTool for DcrontabTool {
    type Args = DcrontabArgs;
    type Output = DcrontabOutput;

    fn name(&self) -> &str {
        TOOL_DCRONTAB
    }

    fn description(&self) -> &str {
        "Manage OpenDAN delegated schedules. Common forms: dcrontab \"0 9 * * 1-5\" \"standup\"; dcrontab --every 5m \"drink water\"; dcrontab --run-at 2026-06-01T20:00:00+08:00 --to owner \"call\"."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::BASH
    }

    fn usage(&self) -> Option<String> {
        Some(
            "dcrontab [add] (<cron> | --run-at <RFC3339> | --every <duration>) [--to <target>] <text>\n\
             dcrontab [add] (<cron> | --run-at <RFC3339> | --every <duration>) task --title <title> --objective <text> --workspace <id>\n\
             dcrontab list [--status enabled|paused|archived|error] [--target remind|task]\n\
             dcrontab show|pause|resume|remove|run-now <schedule_id|name>\n\
             dcrontab validate (<cron> | --run-at <RFC3339> | --every <duration>)"
                .to_string(),
        )
    }

    fn parse_bash_args(
        &self,
        tokens: &[String],
        shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        self.parse_cli_args(tokens, shell_cwd)
            .and_then(|invocation| match invocation {
                CliInvocation::Json { args, .. } => Ok(args),
                CliInvocation::Bash { .. } => Err(AgentToolError::InvalidArgs(
                    "dcrontab parser produced unexpected bash invocation".to_string(),
                )),
            })
    }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> {
        let parsed = parse_dcrontab_tokens(tokens)?;
        Ok(CliInvocation::Json {
            args: serde_json::to_value(parsed).map_err(|err| {
                AgentToolError::InvalidArgs(format!("serialize dcrontab args failed: {err}"))
            })?,
            content_input: None,
        })
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        if args.cmd_line.trim().is_empty() {
            Some(TOOL_DCRONTAB.to_string())
        } else {
            Some(args.cmd_line.clone())
        }
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        output.summary.clone()
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        Some(format!("dcrontab {} => success", output.command))
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        execute_dcrontab(ctx, args).await
    }
}

async fn execute_dcrontab(
    ctx: &ToolCtx<'_>,
    args: DcrontabArgs,
) -> Result<DcrontabOutput, AgentToolError> {
    match args.command {
        DcrontabCommand::Add(add) => add_schedule(ctx, add).await,
        DcrontabCommand::List(list) => list_schedules(ctx, list).await,
        DcrontabCommand::Show(id) => show_schedule(ctx, id).await,
        DcrontabCommand::Pause(id) => set_schedule_state(ctx, "pause", id).await,
        DcrontabCommand::Resume(id) => set_schedule_state(ctx, "resume", id).await,
        DcrontabCommand::Remove(id) => set_schedule_state(ctx, "remove", id).await,
        DcrontabCommand::RunNow(run_now) => run_schedule_now(ctx, run_now).await,
        DcrontabCommand::Validate(validate) => validate_schedule(validate).await,
        DcrontabCommand::History(history) => history_schedule(ctx, history).await,
    }
}

async fn add_schedule(ctx: &ToolCtx<'_>, args: AddArgs) -> Result<DcrontabOutput, AgentToolError> {
    let client = workflow_client().await?;
    let owner = resolve_owner(ctx, args.owner.as_deref(), args.agent.as_deref());
    let schedule = to_workflow_schedule(&args.trigger);
    let target = to_workflow_target(&args.target);
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| default_schedule_name(&args.target));
    let policy = args.misfire.as_deref().map(build_policy).transpose()?;
    let record = client
        .create_scheduled_task(WorkflowCreateScheduledTaskReq {
            owner,
            name,
            description: args.description,
            schedule,
            target,
            policy,
            status: None,
        })
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("workflow.create_scheduled_task failed: {err}"))
        })?;
    let detail = schedule_detail(&record);
    let summary = format!(
        "created schedule {}{}",
        record.name,
        record
            .state
            .next_fire_at
            .map(|ts| format!("; next fire at {}", rfc3339(ts)))
            .unwrap_or_default()
    );
    Ok(output("add", summary, detail))
}

async fn list_schedules(
    ctx: &ToolCtx<'_>,
    args: ListArgs,
) -> Result<DcrontabOutput, AgentToolError> {
    let client = workflow_client().await?;
    let owner = if args.owner.is_some() || args.agent.is_some() {
        Some(resolve_owner(
            ctx,
            args.owner.as_deref(),
            args.agent.as_deref(),
        ))
    } else {
        None
    };
    let status = args.status.as_deref().map(parse_status).transpose()?;
    let mut records = client
        .list_scheduled_tasks(WorkflowListScheduledTasksReq {
            owner,
            status,
            workflow_id: None,
            name: None,
        })
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("workflow.list_scheduled_tasks failed: {err}"))
        })?;
    if let Some(target) = args.target.as_deref() {
        records.retain(|record| target_kind(&record.target) == target);
    }
    let enabled = records
        .iter()
        .filter(|record| record.status == WorkflowScheduledTaskStatus::Enabled)
        .count();
    let paused = records
        .iter()
        .filter(|record| record.status == WorkflowScheduledTaskStatus::Paused)
        .count();
    let summary = format!(
        "{} schedules: {} enabled, {} paused",
        records.len(),
        enabled,
        paused
    );
    let schedules = records.iter().map(schedule_summary).collect::<Vec<_>>();
    Ok(output("list", summary, json!({ "schedules": schedules })))
}

async fn show_schedule(ctx: &ToolCtx<'_>, args: IdArgs) -> Result<DcrontabOutput, AgentToolError> {
    let client = workflow_client().await?;
    let record = resolve_schedule(&client, ctx, &args.id).await?;
    let history = client
        .get_scheduled_task_history(WorkflowGetScheduledTaskHistoryReq {
            schedule_id: record.schedule_id.clone(),
            limit: Some(5),
        })
        .await
        .unwrap_or_default();
    Ok(output(
        "show",
        format!("schedule {} is {}", record.name, status_text(record.status)),
        json!({
            "schedule": schedule_detail(&record),
            "recent_fires": history.iter().map(fire_detail).collect::<Vec<_>>(),
        }),
    ))
}

async fn set_schedule_state(
    ctx: &ToolCtx<'_>,
    command: &str,
    args: IdArgs,
) -> Result<DcrontabOutput, AgentToolError> {
    let client = workflow_client().await?;
    let record = resolve_schedule(&client, ctx, &args.id).await?;
    let updated = match command {
        "pause" => client.pause_scheduled_task(&record.schedule_id).await,
        "resume" => client.resume_scheduled_task(&record.schedule_id).await,
        "remove" => client.archive_scheduled_task(&record.schedule_id).await,
        _ => unreachable!(),
    }
    .map_err(|err| {
        AgentToolError::ExecFailed(format!("workflow.{command}_scheduled_task failed: {err}"))
    })?;
    Ok(output(
        command,
        format!(
            "{} schedule {}",
            command_summary_verb(command),
            updated.name
        ),
        schedule_detail(&updated),
    ))
}

async fn run_schedule_now(
    ctx: &ToolCtx<'_>,
    args: RunNowArgs,
) -> Result<DcrontabOutput, AgentToolError> {
    let client = workflow_client().await?;
    let record = resolve_schedule(&client, ctx, &args.id).await?;
    let fire = client
        .run_scheduled_task_now(WorkflowRunScheduledTaskNowReq {
            schedule_id: record.schedule_id.clone(),
            fire_time: None,
        })
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("workflow.run_scheduled_task_now failed: {err}"))
        })?;
    Ok(output(
        "run-now",
        format!("created manual fire {} for {}", fire.fire_id, record.name),
        json!({
            "schedule_id": record.schedule_id,
            "fire": fire_detail(&fire),
            "reason": args.reason,
        }),
    ))
}

async fn validate_schedule(args: ValidateArgs) -> Result<DcrontabOutput, AgentToolError> {
    let client = workflow_client().await?;
    let result = client
        .validate_scheduled_task(WorkflowValidateScheduledTaskReq {
            schedule: to_workflow_schedule(&args.trigger),
            target: args.target.as_ref().map(to_workflow_target),
        })
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("workflow.validate_scheduled_task failed: {err}"))
        })?;
    Ok(output(
        "validate",
        "schedule is valid".to_string(),
        validate_detail(&args.trigger, result),
    ))
}

async fn history_schedule(
    ctx: &ToolCtx<'_>,
    args: HistoryArgs,
) -> Result<DcrontabOutput, AgentToolError> {
    let client = workflow_client().await?;
    let record = resolve_schedule(&client, ctx, &args.id).await?;
    let fires = client
        .get_scheduled_task_history(WorkflowGetScheduledTaskHistoryReq {
            schedule_id: record.schedule_id.clone(),
            limit: args.limit,
        })
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("workflow.get_scheduled_task_history failed: {err}"))
        })?;
    Ok(output(
        "history",
        format!("{} fire records for {}", fires.len(), record.name),
        json!({
            "schedule_id": record.schedule_id,
            "fires": fires.iter().map(fire_detail).collect::<Vec<_>>(),
        }),
    ))
}

async fn workflow_client() -> Result<WorkflowServiceClient, AgentToolError> {
    if let Ok(runtime) = get_buckyos_api_runtime() {
        return runtime.get_workflow_service_client().await.map_err(|err| {
            AgentToolError::ExecFailed(format!("init workflow client from runtime failed: {err}"))
        });
    }

    let runtime_context = process_runtime_context()?;

    if runtime_context.is_dev_fallback() {
        if let Some(url) = first_env(&[
            "OPENDAN_WORKFLOW_URL",
            "OPENDAN_WORKFLOW_RPC",
            "WORKFLOW_SERVICE_URL",
            "WORKFLOW_SERVICE_RPC",
        ]) {
            let session_token = first_env(&["OPENDAN_SESSION_TOKEN", "SESSION_TOKEN"]);
            return Ok(WorkflowServiceClient::new(kRPC::new(
                url.as_str(),
                session_token,
            )));
        }
    }

    runtime_context.require_appclient_session_token()?;
    let runtime = init_buckyos_api_runtime("agent_tool", None, BuckyOSRuntimeType::AppClient)
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("init runtime for workflow access failed: {err}"))
        })?;
    runtime
        .get_workflow_service_client()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("init workflow client failed: {err}")))
}

async fn resolve_schedule(
    client: &WorkflowServiceClient,
    ctx: &ToolCtx<'_>,
    id_or_name: &str,
) -> Result<WorkflowScheduledTask, AgentToolError> {
    if id_or_name.starts_with("sch-") {
        return client.get_scheduled_task(id_or_name).await.map_err(|err| {
            AgentToolError::ExecFailed(format!("workflow.get_scheduled_task failed: {err}"))
        });
    }
    let owner = resolve_owner(ctx, None, None);
    let records = client
        .list_scheduled_tasks(WorkflowListScheduledTasksReq {
            owner: Some(owner),
            status: None,
            workflow_id: None,
            name: Some(id_or_name.to_string()),
        })
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("workflow.list_scheduled_tasks failed: {err}"))
        })?;
    let mut exact = records
        .iter()
        .filter(|record| record.name == id_or_name)
        .cloned()
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Ok(exact.remove(0));
    }
    if records.len() == 1 {
        return Ok(records[0].clone());
    }
    if records.is_empty() {
        return Err(AgentToolError::NotFound(id_or_name.to_string()));
    }
    Err(AgentToolError::InvalidArgs(format!(
        "schedule name `{id_or_name}` is ambiguous; use schedule_id"
    )))
}

fn parse_dcrontab_tokens(tokens: &[String]) -> Result<DcrontabArgs, AgentToolError> {
    let cmd_line = if tokens.is_empty() {
        TOOL_DCRONTAB.to_string()
    } else {
        format!("{} {}", TOOL_DCRONTAB, shell_join(tokens))
    };
    let (verb, rest) = match tokens.first().map(String::as_str) {
        Some(
            "add" | "list" | "show" | "pause" | "resume" | "remove" | "run-now" | "validate"
            | "history",
        ) => (tokens[0].as_str(), &tokens[1..]),
        _ => ("add", tokens),
    };
    let command = match verb {
        "add" => DcrontabCommand::Add(parse_add(rest)?),
        "list" => DcrontabCommand::List(parse_list(rest)?),
        "show" => DcrontabCommand::Show(parse_id(rest, "show")?),
        "pause" => DcrontabCommand::Pause(parse_id(rest, "pause")?),
        "resume" => DcrontabCommand::Resume(parse_id(rest, "resume")?),
        "remove" => DcrontabCommand::Remove(parse_id(rest, "remove")?),
        "run-now" => DcrontabCommand::RunNow(parse_run_now(rest)?),
        "validate" => DcrontabCommand::Validate(parse_validate(rest)?),
        "history" => DcrontabCommand::History(parse_history(rest)?),
        _ => unreachable!(),
    };
    Ok(DcrontabArgs { command, cmd_line })
}

fn parse_add(tokens: &[String]) -> Result<AddArgs, AgentToolError> {
    let (common, target_tokens) = parse_common(tokens, true)?;
    let target = parse_target(&target_tokens, common.to.clone())?;
    Ok(AddArgs {
        trigger: common.trigger()?,
        target,
        name: common.name,
        description: common.description,
        owner: common.owner,
        agent: common.agent,
        misfire: common.misfire,
    })
}

fn parse_validate(tokens: &[String]) -> Result<ValidateArgs, AgentToolError> {
    let (common, target_tokens) = parse_common(tokens, false)?;
    let target = if target_tokens.is_empty() {
        None
    } else {
        Some(parse_target(&target_tokens, common.to.clone())?)
    };
    Ok(ValidateArgs {
        trigger: common.trigger()?,
        target,
        misfire: common.misfire,
    })
}

#[derive(Default)]
struct CommonParsed {
    cron: Option<String>,
    run_at: Option<i64>,
    every_sec: Option<u64>,
    start_at: Option<i64>,
    end_at: Option<i64>,
    timezone: Option<String>,
    misfire: Option<String>,
    name: Option<String>,
    description: Option<String>,
    owner: Option<String>,
    agent: Option<String>,
    to: Option<String>,
}

impl CommonParsed {
    fn trigger(&self) -> Result<TriggerArgs, AgentToolError> {
        let trigger_count = self.cron.is_some() as u8
            + self.run_at.is_some() as u8
            + self.every_sec.is_some() as u8;
        if trigger_count == 0 {
            return Err(AgentToolError::InvalidArgs(
                "MISSING_TRIGGER: provide one of <cron>, --run-at, or --every".to_string(),
            ));
        }
        if trigger_count > 1 {
            return Err(AgentToolError::InvalidArgs(
                "MULTIPLE_TRIGGERS: <cron>, --run-at and --every are mutually exclusive"
                    .to_string(),
            ));
        }
        if let Some(expr) = self.cron.clone() {
            return Ok(TriggerArgs::Cron {
                expr,
                timezone: self.timezone.clone(),
            });
        }
        if let Some(run_at) = self.run_at {
            return Ok(TriggerArgs::Once {
                run_at,
                timezone: self.timezone.clone(),
            });
        }
        Ok(TriggerArgs::RunEvery {
            every_sec: self.every_sec.unwrap(),
            start_at: self.start_at,
            end_at: self.end_at,
            timezone: self.timezone.clone(),
        })
    }
}

fn parse_common(
    tokens: &[String],
    allow_target_marker: bool,
) -> Result<(CommonParsed, Vec<String>), AgentToolError> {
    let mut common = CommonParsed::default();
    let mut target_tokens = Vec::new();
    let mut in_target = false;
    let mut i = 0usize;
    while i < tokens.len() {
        let token = &tokens[i];
        if allow_target_marker && !in_target && matches!(token.as_str(), "remind" | "task") {
            in_target = true;
            target_tokens.push(token.clone());
            i += 1;
            continue;
        }
        if !in_target {
            match token.as_str() {
                "--run-at" => {
                    common.run_at = Some(parse_rfc3339_arg(
                        next_value(tokens, &mut i, "--run-at")?,
                        "INVALID_RUN_AT",
                    )?)
                }
                "--every" => {
                    common.every_sec = Some(parse_duration(next_value(tokens, &mut i, "--every")?)?)
                }
                "--start-at" => {
                    common.start_at = Some(parse_time_arg(
                        next_value(tokens, &mut i, "--start-at")?,
                        "INVALID_START_AT",
                    )?)
                }
                "--end-at" => {
                    common.end_at = Some(parse_time_arg(
                        next_value(tokens, &mut i, "--end-at")?,
                        "INVALID_END_AT",
                    )?)
                }
                "--timezone" | "--tz" => {
                    common.timezone = Some(next_value(tokens, &mut i, token)?.to_string())
                }
                "--misfire" => {
                    common.misfire = Some(next_value(tokens, &mut i, "--misfire")?.to_string())
                }
                "--name" => common.name = Some(next_value(tokens, &mut i, "--name")?.to_string()),
                "--description" => {
                    common.description =
                        Some(next_value(tokens, &mut i, "--description")?.to_string())
                }
                "--owner" => {
                    common.owner = Some(next_value(tokens, &mut i, "--owner")?.to_string())
                }
                "--agent" => {
                    common.agent = Some(next_value(tokens, &mut i, "--agent")?.to_string())
                }
                "--to" => common.to = Some(next_value(tokens, &mut i, "--to")?.to_string()),
                value if value.starts_with('-') => {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "unknown dcrontab option `{value}`"
                    )));
                }
                value => {
                    if common.cron.is_none() && looks_like_cron(value) {
                        common.cron = Some(value.to_string());
                    } else {
                        in_target = true;
                        target_tokens.push(token.clone());
                    }
                }
            }
            i += 1;
        } else {
            target_tokens.push(token.clone());
            i += 1;
        }
    }
    Ok((common, target_tokens))
}

fn parse_target(
    tokens: &[String],
    default_to: Option<String>,
) -> Result<TargetArgs, AgentToolError> {
    if tokens.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "remind text or task target is required".to_string(),
        ));
    }
    if tokens.first().map(String::as_str) == Some("task") {
        return parse_task_target(&tokens[1..]);
    }
    let remind_tokens = if tokens.first().map(String::as_str) == Some("remind") {
        &tokens[1..]
    } else {
        tokens
    };
    parse_remind_target(remind_tokens, default_to)
}

fn parse_remind_target(
    tokens: &[String],
    default_to: Option<String>,
) -> Result<TargetArgs, AgentToolError> {
    let mut to = default_to;
    let mut text = None;
    let mut trailing = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--to" => to = Some(next_value(tokens, &mut i, "--to")?.to_string()),
            "--text" => text = Some(next_value(tokens, &mut i, "--text")?.to_string()),
            value if value.starts_with('-') => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unknown remind option `{value}`"
                )));
            }
            _ => trailing.push(tokens[i].clone()),
        }
        i += 1;
    }
    let text = text.unwrap_or_else(|| trailing.join(" "));
    if text.trim().is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "remind text is required".to_string(),
        ));
    }
    Ok(TargetArgs::Remind {
        text,
        to: normalize_self_recipient(to),
    })
}

fn parse_task_target(tokens: &[String]) -> Result<TargetArgs, AgentToolError> {
    let mut title = None;
    let mut objective = None;
    let mut workspace_id = None;
    let mut behavior = None;
    let mut agent = None;
    let mut i = 0usize;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--title" => title = Some(next_value(tokens, &mut i, "--title")?.to_string()),
            "--objective" => {
                objective = Some(next_value(tokens, &mut i, "--objective")?.to_string())
            }
            "--workspace" | "--workspace-id" => {
                workspace_id = Some(next_value(tokens, &mut i, "--workspace")?.to_string())
            }
            "--behavior" => behavior = Some(next_value(tokens, &mut i, "--behavior")?.to_string()),
            "--agent" => agent = Some(next_value(tokens, &mut i, "--agent")?.to_string()),
            value => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unknown task option `{value}`"
                )))
            }
        }
        i += 1;
    }
    let title = required(title, "--title")?;
    let objective = required(objective, "--objective")?;
    let workspace_id = required(workspace_id, "--workspace")?;
    Ok(TargetArgs::Task {
        title,
        objective,
        workspace_id,
        behavior,
        agent,
    })
}

fn parse_list(tokens: &[String]) -> Result<ListArgs, AgentToolError> {
    let mut out = ListArgs {
        status: None,
        target: None,
        owner: None,
        agent: None,
    };
    let mut i = 0usize;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--status" => out.status = Some(next_value(tokens, &mut i, "--status")?.to_string()),
            "--target" => out.target = Some(next_value(tokens, &mut i, "--target")?.to_string()),
            "--owner" => out.owner = Some(next_value(tokens, &mut i, "--owner")?.to_string()),
            "--agent" => out.agent = Some(next_value(tokens, &mut i, "--agent")?.to_string()),
            value => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unknown list option `{value}`"
                )))
            }
        }
        i += 1;
    }
    Ok(out)
}

fn parse_id(tokens: &[String], command: &str) -> Result<IdArgs, AgentToolError> {
    if tokens.len() != 1 {
        return Err(AgentToolError::InvalidArgs(format!(
            "{command} requires exactly one schedule_id or name"
        )));
    }
    Ok(IdArgs {
        id: tokens[0].clone(),
    })
}

fn parse_run_now(tokens: &[String]) -> Result<RunNowArgs, AgentToolError> {
    if tokens.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "run-now requires schedule_id or name".to_string(),
        ));
    }
    let id = tokens[0].clone();
    let mut reason = None;
    let mut i = 1usize;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--reason" => reason = Some(next_value(tokens, &mut i, "--reason")?.to_string()),
            value => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unknown run-now option `{value}`"
                )))
            }
        }
        i += 1;
    }
    Ok(RunNowArgs { id, reason })
}

fn parse_history(tokens: &[String]) -> Result<HistoryArgs, AgentToolError> {
    if tokens.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "history requires schedule_id or name".to_string(),
        ));
    }
    let id = tokens[0].clone();
    let mut limit = None;
    let mut i = 1usize;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--limit" => {
                limit = Some(
                    next_value(tokens, &mut i, "--limit")?
                        .parse::<u32>()
                        .map_err(|_| AgentToolError::InvalidArgs("invalid --limit".to_string()))?,
                )
            }
            value => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unknown history option `{value}`"
                )))
            }
        }
        i += 1;
    }
    Ok(HistoryArgs { id, limit })
}

fn next_value<'a>(
    tokens: &'a [String],
    index: &mut usize,
    flag: &str,
) -> Result<&'a str, AgentToolError> {
    *index += 1;
    tokens
        .get(*index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("{flag} requires a value")))
}

fn required(value: Option<String>, flag: &str) -> Result<String, AgentToolError> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("{flag} is required")))
}

fn looks_like_cron(value: &str) -> bool {
    value.starts_with('@') || value.split_whitespace().count() == 5
}

fn parse_duration(raw: &str) -> Result<u64, AgentToolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "INVALID_DURATION: duration is empty".to_string(),
        ));
    }
    let (number, multiplier) = match trimmed.chars().last().unwrap() {
        's' | 'S' => (&trimmed[..trimmed.len() - 1], 1),
        'm' | 'M' => (&trimmed[..trimmed.len() - 1], 60),
        'h' | 'H' => (&trimmed[..trimmed.len() - 1], 60 * 60),
        'd' | 'D' => (&trimmed[..trimmed.len() - 1], 24 * 60 * 60),
        c if c.is_ascii_digit() => (trimmed, 1),
        _ => {
            return Err(AgentToolError::InvalidArgs(format!(
                "INVALID_DURATION: unsupported duration `{raw}`"
            )))
        }
    };
    let value = number.parse::<u64>().map_err(|_| {
        AgentToolError::InvalidArgs(format!("INVALID_DURATION: unsupported duration `{raw}`"))
    })?;
    let seconds = value.checked_mul(multiplier).ok_or_else(|| {
        AgentToolError::InvalidArgs("INVALID_DURATION: duration overflow".to_string())
    })?;
    if seconds == 0 {
        return Err(AgentToolError::InvalidArgs(
            "INVALID_DURATION: duration must be greater than zero".to_string(),
        ));
    }
    Ok(seconds)
}

fn parse_rfc3339_arg(raw: &str, code: &str) -> Result<i64, AgentToolError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp())
        .map_err(|_| AgentToolError::InvalidArgs(format!("{code}: expected RFC3339 timestamp")))
}

fn parse_time_arg(raw: &str, code: &str) -> Result<i64, AgentToolError> {
    raw.parse::<i64>()
        .or_else(|_| DateTime::parse_from_rfc3339(raw).map(|dt| dt.timestamp()))
        .map_err(|_| {
            AgentToolError::InvalidArgs(format!(
                "{code}: expected unix seconds or RFC3339 timestamp"
            ))
        })
}

fn normalize_self_recipient(to: Option<String>) -> Option<String> {
    match to.as_deref().map(str::trim) {
        None | Some("") | Some("self") => None,
        _ => to,
    }
}

fn default_schedule_name(target: &TargetArgs) -> String {
    match target {
        TargetArgs::Task { title, .. } => title.trim().to_string(),
        TargetArgs::Remind { text, .. } => {
            let mut prefix = text
                .chars()
                .filter(|ch| ch.is_alphanumeric())
                .take(16)
                .collect::<String>();
            if prefix.is_empty() {
                prefix = "reminder".to_string();
            }
            format!("{}-{}", prefix, short_hash(text))
        }
    }
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..2])
}

fn to_workflow_schedule(trigger: &TriggerArgs) -> WorkflowScheduledTaskSchedule {
    match trigger {
        TriggerArgs::Cron { expr, timezone } => WorkflowScheduledTaskSchedule::Cron {
            expr: expr.clone(),
            timezone: timezone.clone().unwrap_or_else(|| "UTC".to_string()),
            calendar: None,
            start_at: None,
            end_at: None,
        },
        TriggerArgs::Once { run_at, timezone } => WorkflowScheduledTaskSchedule::Once {
            run_at: *run_at,
            timezone: timezone.clone(),
        },
        TriggerArgs::RunEvery {
            every_sec,
            start_at,
            end_at,
            timezone,
        } => WorkflowScheduledTaskSchedule::RunEvery {
            every_sec: *every_sec,
            start_at: *start_at,
            end_at: *end_at,
            timezone: timezone.clone(),
        },
    }
}

fn to_workflow_target(target: &TargetArgs) -> WorkflowScheduledTaskTarget {
    match target {
        TargetArgs::Remind { text, to } => WorkflowScheduledTaskTarget {
            task_type: "workflow.send_message".to_string(),
            name_template: "remind: ${schedule.name} [${fire.fire_id}]".to_string(),
            data_template: json!({
                "send_message": {
                    "to": to.clone().unwrap_or_else(|| "self".to_string()),
                    "text": text,
                    "trigger": trigger_template()
                }
            }),
        },
        TargetArgs::Task {
            title,
            objective,
            workspace_id,
            behavior,
            agent,
        } => {
            // The executing agent rides inside the task payload
            // (`execution.runner`, OpenDAN's business schema); TaskMgr no
            // longer has a dispatch-level runner field.
            let agent = agent
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "${schedule.owner.app_id}".to_string());
            WorkflowScheduledTaskTarget {
                task_type: "agent.delegate".to_string(),
                name_template: title.clone(),
                data_template: json!({
                    "agent_delegate": {
                        "version": 1,
                        "title": title,
                        "purpose": objective,
                        "requester_agent_id": "${schedule.owner.app_id}",
                        "owner_session_id": "schedule-${schedule.schedule_id}",
                        "input": {
                            "text": objective
                        },
                        "workspace_hints": [{
                            "workspace_id": workspace_id
                        }],
                        "trigger": trigger_template(),
                        "execution": {
                            "workspace_id": workspace_id,
                            "behavior": behavior,
                            "runner": agent,
                            "status": "pending"
                        }
                    }
                }),
            }
        }
    }
}

fn trigger_template() -> Json {
    json!({
        "schedule_id": "${schedule.schedule_id}",
        "fire_id": "${fire.fire_id}",
        "fire_time": "${fire.fire_time}",
        "manual": "${fire.manual}"
    })
}

fn build_policy(raw: &str) -> Result<WorkflowScheduledTaskPolicy, AgentToolError> {
    let misfire = match raw {
        "skip" => WorkflowScheduledTaskMisfirePolicy::Skip,
        "run_once" => WorkflowScheduledTaskMisfirePolicy::RunOnce,
        "catch_up" => WorkflowScheduledTaskMisfirePolicy::CatchUp,
        "manual" => WorkflowScheduledTaskMisfirePolicy::Manual,
        _ => {
            return Err(AgentToolError::InvalidArgs(format!(
                "invalid --misfire `{raw}`"
            )))
        }
    };
    Ok(WorkflowScheduledTaskPolicy {
        misfire,
        max_parallel_runs: 1,
        catch_up_limit: 1,
        jitter_sec: 0,
    })
}

fn parse_status(raw: &str) -> Result<WorkflowScheduledTaskStatus, AgentToolError> {
    match raw {
        "enabled" => Ok(WorkflowScheduledTaskStatus::Enabled),
        "paused" => Ok(WorkflowScheduledTaskStatus::Paused),
        "archived" => Ok(WorkflowScheduledTaskStatus::Archived),
        "error" => Ok(WorkflowScheduledTaskStatus::Error),
        _ => Err(AgentToolError::InvalidArgs(format!(
            "invalid status `{raw}`"
        ))),
    }
}

fn resolve_owner(ctx: &ToolCtx<'_>, owner: Option<&str>, agent: Option<&str>) -> WorkflowOwner {
    let runtime = get_buckyos_api_runtime().ok();
    let user_id = owner
        .map(str::to_string)
        .or_else(|| runtime.and_then(|rt| rt.get_owner_user_id().or_else(|| rt.user_id.clone())))
        .unwrap_or_else(|| "devtest".to_string());
    let app_id = agent
        .map(str::to_string)
        .or_else(|| get_buckyos_api_runtime().ok().map(|rt| rt.get_app_id()))
        .unwrap_or_else(|| ctx.session().agent_name.clone());
    WorkflowOwner { user_id, app_id }
}

/// Build the unified `RuntimeContext` from process env. dev fallback is allowed
/// here: when `OPENDAN_AGENT_ROOT` is absent the context is marked
/// `DevFallback`, which is what gates the dev-only workflow-service overrides.
fn process_runtime_context() -> Result<RuntimeContext, AgentToolError> {
    let current_dir = env::current_dir().ok();
    let current_dir = current_dir.as_deref().unwrap_or_else(|| Path::new("."));
    RuntimeContext::from_process_env(current_dir, true)
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn target_kind(target: &WorkflowScheduledTaskTarget) -> &'static str {
    match target.task_type.as_str() {
        "workflow.send_message" => "remind",
        "agent.delegate" => "task",
        "workflow.run" => "workflow",
        "opendan.command" => "opendan",
        "service.rpc" => "service",
        _ => "subtask",
    }
}

fn schedule_summary(record: &WorkflowScheduledTask) -> Json {
    json!({
        "schedule_id": record.schedule_id,
        "name": record.name,
        "status": status_text(record.status),
        "trigger": trigger_summary(&record.schedule),
        "target": target_summary(&record.target),
        "next_fire_at": record.state.next_fire_at.map(rfc3339),
    })
}

fn schedule_detail(record: &WorkflowScheduledTask) -> Json {
    json!({
        "schedule_id": record.schedule_id,
        "name": record.name,
        "description": record.description,
        "status": status_text(record.status),
        "trigger": trigger_detail(&record.schedule),
        "target": target_detail(&record.target),
        "state": {
            "next_fire_at": record.state.next_fire_at.map(rfc3339),
            "last_fire_at": record.state.last_fire_at.map(rfc3339),
            "last_task_id": record.state.last_task_id,
            "last_run_id": record.state.last_run_id,
            "last_error": record.state.last_error,
        },
        "task_mirror": record.task_mirror,
        "created_at": rfc3339(record.created_at),
        "updated_at": rfc3339(record.updated_at),
    })
}

fn trigger_detail(schedule: &WorkflowScheduledTaskSchedule) -> Json {
    match schedule {
        WorkflowScheduledTaskSchedule::Cron { expr, timezone, .. } => {
            json!({ "kind": "cron", "expr": expr, "timezone": timezone })
        }
        WorkflowScheduledTaskSchedule::Once { run_at, timezone } => {
            json!({ "kind": "once", "run_at": rfc3339(*run_at), "timezone": timezone })
        }
        WorkflowScheduledTaskSchedule::RunEvery {
            every_sec,
            start_at,
            end_at,
            timezone,
        } => json!({
            "kind": "every",
            "every_sec": every_sec,
            "start_at": start_at.map(rfc3339),
            "end_at": end_at.map(rfc3339),
            "timezone": timezone,
        }),
    }
}

fn trigger_summary(schedule: &WorkflowScheduledTaskSchedule) -> Json {
    match schedule {
        WorkflowScheduledTaskSchedule::Cron { expr, timezone, .. } => {
            json!({ "kind": "cron", "expr": expr, "timezone": timezone })
        }
        WorkflowScheduledTaskSchedule::Once { run_at, timezone } => {
            json!({ "kind": "once", "run_at": rfc3339(*run_at), "timezone": timezone })
        }
        WorkflowScheduledTaskSchedule::RunEvery {
            every_sec,
            timezone,
            ..
        } => json!({
            "kind": "every",
            "every_sec": every_sec,
            "timezone": timezone,
        }),
    }
}

fn target_detail(target: &WorkflowScheduledTaskTarget) -> Json {
    match target.task_type.as_str() {
        "workflow.send_message" => {
            let send_message = target.data_template.get("send_message");
            json!({
                "kind": "remind",
                "task_type": target.task_type,
                "to": send_message.and_then(|value| value.get("to")).and_then(Json::as_str).unwrap_or("self"),
                "text": send_message.and_then(|value| value.get("text")).and_then(Json::as_str).unwrap_or_default()
            })
        }
        "agent.delegate" => {
            let delegate = target.data_template.get("agent_delegate");
            let workspace_id = delegate
                .and_then(|value| value.get("workspace_hints"))
                .and_then(Json::as_array)
                .and_then(|items| items.first())
                .and_then(|value| value.get("workspace_id"))
                .and_then(Json::as_str);
            json!({
                "kind": "task",
                "task_type": target.task_type,
                "agent": delegate.and_then(|value| value.pointer("/execution/runner")).cloned().unwrap_or(Json::Null),
                "title": delegate.and_then(|value| value.get("title")).and_then(Json::as_str).unwrap_or_default(),
                "objective": delegate.and_then(|value| value.get("purpose")).and_then(Json::as_str).unwrap_or_default(),
                "workspace_id": workspace_id,
                "behavior": delegate
                    .and_then(|value| value.pointer("/execution/behavior"))
                    .cloned()
                    .unwrap_or(Json::Null),
            })
        }
        _ => json!(target),
    }
}

fn target_summary(target: &WorkflowScheduledTaskTarget) -> Json {
    match target.task_type.as_str() {
        "workflow.send_message" => {
            let send_message = target.data_template.get("send_message");
            json!({
                "kind": "remind",
                "to": send_message
                    .and_then(|value| value.get("to"))
                    .and_then(Json::as_str)
                    .unwrap_or("self"),
                "text": send_message
                    .and_then(|value| value.get("text"))
                    .and_then(Json::as_str)
                    .map(|text| truncate_summary_text(text, 80))
                    .unwrap_or_default(),
            })
        }
        "agent.delegate" => {
            let delegate = target.data_template.get("agent_delegate");
            let workspace_id = delegate
                .and_then(|value| value.get("workspace_hints"))
                .and_then(Json::as_array)
                .and_then(|items| items.first())
                .and_then(|value| value.get("workspace_id"))
                .and_then(Json::as_str);
            json!({
                "kind": "task",
                "title": delegate
                    .and_then(|value| value.get("title"))
                    .and_then(Json::as_str)
                    .map(|text| truncate_summary_text(text, 80))
                    .unwrap_or_default(),
                "objective": delegate
                    .and_then(|value| value.get("purpose"))
                    .and_then(Json::as_str)
                    .map(|text| truncate_summary_text(text, 120))
                    .unwrap_or_default(),
                "workspace_id": workspace_id,
            })
        }
        _ => json!({
            "kind": target_kind(target),
        }),
    }
}

fn truncate_summary_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn validate_detail(trigger: &TriggerArgs, result: WorkflowValidateScheduledTaskResult) -> Json {
    json!({
        "valid": result.valid,
        "trigger": {
            "kind": trigger_kind(trigger),
            "normalized_expr": result.normalized_expr,
            "timezone": result.timezone,
        },
        "next_fire_times": result.next_fire_times,
        "warnings": result.warnings,
    })
}

fn fire_detail(fire: &WorkflowScheduledTaskFireRecord) -> Json {
    json!({
        "fire_id": fire.fire_id,
        "schedule_id": fire.schedule_id,
        "fire_time": rfc3339(fire.fire_time),
        "manual": fire.manual,
        "status": fire.status,
        "task_id": fire.task_id,
        "run_id": fire.run_id,
        "error": fire.error,
    })
}

fn trigger_kind(trigger: &TriggerArgs) -> &'static str {
    match trigger {
        TriggerArgs::Cron { .. } => "cron",
        TriggerArgs::Once { .. } => "once",
        TriggerArgs::RunEvery { .. } => "every",
    }
}

fn status_text(status: WorkflowScheduledTaskStatus) -> &'static str {
    match status {
        WorkflowScheduledTaskStatus::Enabled => "enabled",
        WorkflowScheduledTaskStatus::Paused => "paused",
        WorkflowScheduledTaskStatus::Archived => "archived",
        WorkflowScheduledTaskStatus::Error => "error",
    }
}

fn command_summary_verb(command: &str) -> &'static str {
    match command {
        "pause" => "paused",
        "resume" => "resumed",
        "remove" => "archived",
        _ => "updated",
    }
}

fn output(command: &str, summary: String, detail: Json) -> DcrontabOutput {
    let detail = match detail {
        Json::Object(map) => map,
        other => {
            let mut map = JsonMap::new();
            map.insert("value".to_string(), other);
            map
        }
    };
    DcrontabOutput {
        command: command.to_string(),
        summary,
        detail,
    }
}

fn rfc3339(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

fn shell_join(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| {
            if token.chars().all(|ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@')
            }) {
                token.clone()
            } else {
                format!("'{}'", token.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_record(target: WorkflowScheduledTaskTarget) -> WorkflowScheduledTask {
        WorkflowScheduledTask {
            schedule_id: "sch-1".to_string(),
            owner: WorkflowOwner {
                user_id: "devtest".to_string(),
                app_id: "buckyos_jarvis".to_string(),
            },
            name: "standup".to_string(),
            description: Some("description should stay out of list".to_string()),
            status: WorkflowScheduledTaskStatus::Enabled,
            schedule: WorkflowScheduledTaskSchedule::Cron {
                expr: "*/30 9-17 * * 1-6".to_string(),
                timezone: "America/Los_Angeles".to_string(),
                calendar: None,
                start_at: None,
                end_at: None,
            },
            target,
            state: buckyos_api::WorkflowScheduledTaskState {
                next_fire_at: Some(1_780_000_000),
                ..Default::default()
            },
            policy: WorkflowScheduledTaskPolicy {
                misfire: WorkflowScheduledTaskMisfirePolicy::Skip,
                max_parallel_runs: 1,
                catch_up_limit: 1,
                jitter_sec: 0,
            },
            task_mirror: Default::default(),
            created_at: 1,
            updated_at: 2,
        }
    }

    fn parse(tokens: &[&str]) -> DcrontabArgs {
        parse_dcrontab_tokens(
            &tokens
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn short_cron_remind_defaults_add_and_target() {
        let parsed = parse(&["0 9 * * 1-5", "standup"]);
        match parsed.command {
            DcrontabCommand::Add(args) => {
                assert!(matches!(args.trigger, TriggerArgs::Cron { .. }));
                assert!(matches!(args.target, TargetArgs::Remind { .. }));
            }
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn parses_every_duration() {
        let parsed = parse(&["--every", "5m", "drink"]);
        match parsed.command {
            DcrontabCommand::Add(args) => {
                assert!(matches!(
                    args.trigger,
                    TriggerArgs::RunEvery { every_sec: 300, .. }
                ));
            }
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn rejects_multiple_triggers() {
        let err = parse_dcrontab_tokens(&[
            "--every".to_string(),
            "5m".to_string(),
            "0 9 * * *".to_string(),
            "drink".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("MULTIPLE_TRIGGERS"));
    }

    #[test]
    fn list_summary_omits_show_level_target_fields() {
        let record = test_record(WorkflowScheduledTaskTarget {
            task_type: "agent.delegate".to_string(),
            name_template: "scheduled task".to_string(),
            data_template: json!({
                "agent_delegate": {
                    "title": "Daily mail scan",
                    "purpose": "This long objective starts with duplicate-check context and then keeps adding verbose implementation details that are useful in show but should not be repeated in list output.",
                    "workspace_hints": [{"workspace_id": "mail"}],
                    "execution": {
                        "behavior": "work_default"
                    }
                }
            }),
        });

        let summary = schedule_summary(&record);
        let text = serde_json::to_string(&summary).unwrap();
        assert!(text.contains("Daily mail scan"));
        assert!(text.contains("mail"));
        assert!(text.contains("long objective"));
        assert!(!text.contains("agent.delegate"));
        assert!(!text.contains("buckyos_jarvis"));
        assert!(!text.contains("repeated in list output"));
        assert!(!text.contains("work_default"));
    }
}
