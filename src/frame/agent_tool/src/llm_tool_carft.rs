//根据 session传递下来的context,完成 command not found的构造
//1）先判断是否需要构造，如可以开始构造，则创建 taskid记录构造状态
//2）组合大于制造，优先在可见bin的范围内进行查找，争取通过脚本组合，让失败的命令正确工作
//3）判断是apt install合适，还是手写py / ts 脚本合适.写的脚本工具有一定通用性，放在agentRootFS/tools目录下，方便下次再走到第二步的时候可以复用
//4）构建成功，用命令行调用拿到结果
//5）构建失败，保持command not found ...
//
// ## 触发路径
//
// 当 behavior cfg 在某条 exec_bash 上打开了意图旁路：
// 1. `opendan::agent_bash::build_exec_script` 注入 `command_not_found_handle`
//    并由 runtime 按路径规则直接写入 agent_tool 绝对路径。
// 2. tmux pane 里跑 `missing-cmd args...` 触发 hook,bash 改调
//    `agent_tool __command_not_found__ missing-cmd args...`。
// 3. agent_tool_cli_dev 的 `ParsedCommand::CommandNotFound` 分支把
//    `(command, argv)` 打包成 [`CommandNotFoundRequest`],转手交给
//    [`run_subcommand`] —— 也就是本模块。
// 4. 本模块按上述 5 步流水线决定要不要构造工具、走哪种构造、跑构造结果。
// 5. 完全失败时回退到 127,让 bash 打印原生 `command not found`,对 LLM
//    看到的信号和"没开旁路"等价。
//
// ## 当前实现状态
//
// 仅搭架子:所有步骤的 stub 返回 `NotImplemented`,整条流水线直落 step 5,
// 对外行为等价于旧 placeholder(exit 127 + 一条日志),但 envelope 已经是
// 结构化 `AgentToolResult`,后续填实不再改 dispatcher。
//
// 文件骨架参考 `llm_explore.rs`,但入口签名不同:意图旁路的输入是上游
// dispatcher 已经分拣过的 `(command, argv)`,而不是 CLI argv 字符串列表,
// 所以省掉 `CliOpts`,直接收一个 [`CommandNotFoundRequest`] 结构体。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::{
    AgentToolResult, AgentToolStatus, AGENT_TOOL_PROTOCOL_VERSION, CLI_EXIT_COMMAND_NOT_FOUND,
};

/// 本子命令对外暴露的工具名 —— 写进 `AgentToolResult.tool` 字段,让
/// 日志 / worklog / metric 都能按 "llm_tool_carft" 聚合,而不是按 shell
/// hook 那个内部子命令名 `__command_not_found__`。
const TOOL_NAME: &str = "llm_tool_carft";

// =========================================================================
// 输入 / 输出
// =========================================================================

/// 由 dispatcher (`agent_tool_cli_dev::execute`) 在 `ParsedCommand::
/// CommandNotFound` 分支构造,作为 [`run_subcommand`] 的唯一入参。
#[derive(Debug, Clone)]
pub struct CommandNotFoundRequest {
    /// `argv[0]` —— 未找到的命令名。`None` 表示 shell hook 没传命令
    /// (理论上不该出现,保留这条分支是为了和 dispatcher 已有的
    /// `CommandNotFound { command: None, argv: vec![] }` 兼容)。
    pub command: Option<String>,
    /// 完整 argv (含 argv[0])。直接传给 step 4 的执行器。
    pub argv: Vec<String>,
    /// shell hook 当时的 PWD。step 2 搜 PATH / step 4 跑构造产物时
    /// 需要它做 cwd。
    pub current_dir: PathBuf,
    /// RuntimeContext 解析到的 agent state 根。step 1 读 behavior
    /// cfg、step 3 把新造工具写到 `<agent_root>/tools/` 都靠它。
    /// `None` 表示 CLI 在裸进程模式下被调起 (dev / test),意图旁路
    /// 此时按 "skip" 处理 —— 没有 agent context 谈不上"造工具"。
    pub agent_env_root: Option<PathBuf>,
}

/// 把流水线的中间状态做成 enum,让各 stub 之间的契约可见。也方便
/// 单元测试单独构造某一步的结果断言下游行为。
#[derive(Debug, Clone)]
enum Decision {
    /// step 1 决定进入构造。`task_id` 用于持久化构造进度 (step 2/3
    /// 跨进程时 resume)。stub 阶段没人构造它,容忍 dead_code。
    #[allow(dead_code)]
    Construct { task_id: String },
    /// step 1 拒绝构造。`reason` 写进 result.details,供调试。
    Skip { reason: SkipReason },
}

#[derive(Debug, Clone)]
enum SkipReason {
    /// behavior cfg 没开旁路,或者 agent_env_root 缺失。
    BypassDisabled,
    /// 命令在拒绝名单 (避免对 rm/dd 这种危险命令做"自动救援")。
    #[allow(dead_code)] // 等 step 1 接 behavior cfg 时被产出
    Denylisted,
    /// 历史上对这条命令的构造已经失败过 N 次,避免重复 LLM 调用。
    #[allow(dead_code)]
    PreviousFailures,
    /// shell hook 没传命令 (argv 为空)。
    EmptyCommand,
    /// 占位 —— 未来添加新的拒绝原因时补这里,外部不依赖具体 variant。
    #[allow(dead_code)]
    Other(String),
}

/// step 2/3 的产出:一段可执行的脚本 / 已落盘的工具路径,加上调用方
/// 应该用什么命令行去 invoke 它。
#[derive(Debug, Clone)]
#[allow(dead_code)] // 字段都在 stub 阶段,等 step 2/3 真造时被读
struct ConstructedArtifact {
    /// 人类可读的标签 (e.g. "compose:cat+jq", "crafted:py:weather.py"),
    /// 写进 result.summary 让 LLM 看到走了哪条路。
    label: String,
    /// 实际要 exec 的 argv0。组合脚本写文件后这里是 `/bin/bash` +
    /// argv = [script_path, ...orig_argv[1..]]; 新造的脚本就是脚本自己
    /// 的绝对路径。
    exec_program: PathBuf,
    /// 跟在 exec_program 后面的参数。
    exec_argv: Vec<String>,
    /// step 4 跑完后写到 result.details,供 worklog 复盘。
    construction_notes: Value,
}

/// step 4 的产物。透传给 LLM 的字段。
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ExecResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

// =========================================================================
// 入口
// =========================================================================

/// dispatcher 调用点。返回 `(result, exit_code)`:
/// - `result` 是结构化的 `AgentToolResult`,dispatcher 用
///   `render_cli_output(&result, exit_code)` 包成最终 CLI 输出。
/// - `exit_code` 是回给 bash 的退出码。任何走到 step 5 fallback 的情况
///   都返回 `CLI_EXIT_COMMAND_NOT_FOUND` (127),shell hook 据此决定要不
///   要再补一条 bash 原生的 `command not found` 错误。
pub async fn run_subcommand(req: CommandNotFoundRequest) -> (AgentToolResult, i32) {
    // step 1
    let task_id = match decide_should_construct(&req) {
        Decision::Skip { reason } => {
            return surface_fallthrough(&req, FallthroughCause::Skipped(reason));
        }
        Decision::Construct { task_id } => task_id,
    };

    // step 2
    if let Some(artifact) = try_compose_from_visible_bins(&req, &task_id).await {
        return run_artifact(&req, &task_id, artifact).await;
    }

    // step 3
    match craft_new_tool(&req, &task_id).await {
        Ok(artifact) => run_artifact(&req, &task_id, artifact).await,
        Err(err) => surface_fallthrough(&req, FallthroughCause::CraftFailed { task_id, err }),
    }
}

// =========================================================================
// step 1:决策
// =========================================================================

/// 决定要不要进入构造流水线。stub 实现:
/// - argv 为空 → Skip(EmptyCommand)
/// - agent_env_root 缺失 → Skip(BypassDisabled)
/// - 其余 → 暂时一律 Skip(BypassDisabled),等真接 behavior cfg + 拒绝名
///   单 + 失败计数后改。
///
/// 待实现:
/// - 读取 `<agent_env_root>/behavior/<current>.toml` 的旁路开关
/// - 读取拒绝名单 (rm/dd/sudo 等高危命令)
/// - 读取持久化的失败计数 (同命令短时间内失败 ≥ N 次就 Skip)
/// - 创建并持久化 task_id (写进
///   `<agent_env_root>/tasks/llm_tool_carft/<task_id>.json`)
fn decide_should_construct(req: &CommandNotFoundRequest) -> Decision {
    if req
        .command
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return Decision::Skip {
            reason: SkipReason::EmptyCommand,
        };
    }
    if req.agent_env_root.is_none() {
        return Decision::Skip {
            reason: SkipReason::BypassDisabled,
        };
    }
    // 真接 behavior cfg 之前一律 Skip,对外行为 = 旧 placeholder。
    Decision::Skip {
        reason: SkipReason::BypassDisabled,
    }
}

// =========================================================================
// step 2:组合优先
// =========================================================================

/// 在当前 PATH 可见的 bin 范围里找现成命令的组合,尝试让 `req.argv`
/// 等价跑通。stub 实现:直接返回 None。
///
/// 待实现:
/// - 列出 4-layer overlay PATH 的全部 bin 文件名 + (来自 agent tools 的)
///   usage
/// - 用一个轻量 LLM (`llm.summary` 级别) 问"用这些命令能不能拼出
///   `<missing-cmd> <argv>` 的等价语义"
/// - 命中:生成一段 bash 片段写到临时目录,返回 ConstructedArtifact
async fn try_compose_from_visible_bins(
    _req: &CommandNotFoundRequest,
    _task_id: &str,
) -> Option<ConstructedArtifact> {
    None
}

// =========================================================================
// step 3:真造工具
// =========================================================================

#[derive(Debug, Clone)]
enum CraftError {
    /// 还没接 LLM,所有调用直接落这条。
    NotImplemented,
    /// LLM 决定造不出来 (命令语义太特化,或要求权限太大)。
    #[allow(dead_code)]
    Refused { reason: String },
}

impl std::fmt::Display for CraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CraftError::NotImplemented => write!(f, "llm_tool_carft step 3 not implemented yet"),
            CraftError::Refused { reason } => write!(f, "refused: {reason}"),
        }
    }
}

/// 让 LLM 决定走 (a) `apt install <pkg>` 还是 (b) 手写 py/ts 脚本,把
/// 产出物落盘到 `<agent_env_root>/tools/`。stub 实现:返回
/// `NotImplemented`。
///
/// 待实现:
/// - 起一个 `LocalLLMContext`,system prompt 描述 step 3 的两个分支 + 当
///   前 agent 的 tools 目录布局 + 已安装 apt 包列表
/// - 把 (missing-cmd, argv, 失败原因) 喂进去,让模型选分支并出方案
/// - apt 分支:直接 exec_bash `apt install -y <pkg>`,成功后 retry 原 argv
/// - script 分支:把生成的脚本写进 `<agent_env_root>/tools/<name>`,加
///   可执行位,返回 ConstructedArtifact 指向新脚本
async fn craft_new_tool(
    _req: &CommandNotFoundRequest,
    _task_id: &str,
) -> Result<ConstructedArtifact, CraftError> {
    Err(CraftError::NotImplemented)
}

// =========================================================================
// step 4:执行构造产物
// =========================================================================

/// 拿组合脚本 / 新工具跑原 argv,stdout/stderr/exit 透传成
/// `AgentToolResult`。stub:因为 step 2/3 都没实现,这条路径暂时不可达;
/// 保留实现框架,未来 step 2/3 接通后第一时间能跑端到端。
async fn run_artifact(
    req: &CommandNotFoundRequest,
    task_id: &str,
    artifact: ConstructedArtifact,
) -> (AgentToolResult, i32) {
    match execute_artifact(req, &artifact).await {
        Ok(exec) => {
            let details = json!({
                "task_id": task_id,
                "command": req.command,
                "argv": req.argv,
                "outcome": "executed",
                "artifact_label": artifact.label,
                "exec_program": artifact.exec_program.display().to_string(),
                "exec_argv": artifact.exec_argv,
                "construction_notes": artifact.construction_notes,
                "exit_code": exec.exit_code,
            });
            let result = AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_NAME.to_string()),
                cmd_name: req.command.clone(),
                status: if exec.exit_code == 0 {
                    AgentToolStatus::Success
                } else {
                    AgentToolStatus::Error
                },
                task_id: Some(task_id.to_string()),
                pending_reason: None,
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_NAME} => executed ({})", artifact.label),
                summary: format!("ran {} → exit {}", artifact.label, exec.exit_code),
                details,
                cmd_args: Some(req.argv.join(" ")),
                return_code: Some(exec.exit_code),
                partial_output: if exec.stderr.is_empty() {
                    None
                } else {
                    Some(exec.stderr.clone())
                },
                output: Some(exec.stdout),
            };
            (result, exec.exit_code)
        }
        Err(err) => surface_fallthrough(
            req,
            FallthroughCause::ExecFailed {
                task_id: task_id.to_string(),
                label: artifact.label,
                error: err,
            },
        ),
    }
}

/// 真正起子进程跑构造产物。stub:直接返回错误。等 step 2/3 落地后这里
/// 可以复用 `llm_bash::LocalProcessBashRunner` 把 `exec_program +
/// exec_argv` 跑起来。
async fn execute_artifact(
    _req: &CommandNotFoundRequest,
    _artifact: &ConstructedArtifact,
) -> Result<ExecResult, String> {
    Err("execute_artifact stub: step 4 not implemented".to_string())
}

// =========================================================================
// step 5:回落 127
// =========================================================================

enum FallthroughCause {
    Skipped(SkipReason),
    CraftFailed {
        task_id: String,
        err: CraftError,
    },
    #[allow(dead_code)] // 等 step 4 真造时被产出
    ExecFailed {
        task_id: String,
        label: String,
        error: String,
    },
}

/// 构造跑不通时统一出口:组装一份说明性 `AgentToolResult`,exit code
/// 固定 127,让上层 shell hook 看到 127 后补打 bash 原生 `command not
/// found` 错误。这一致性是设计契约:旁路开 vs 关,LLM 拿到的 stderr 至少
/// 包含 `command not found` 字样,否则模型容易把"旁路静悄悄吞了错误"
/// 误判成"命令成功了"。
fn surface_fallthrough(
    req: &CommandNotFoundRequest,
    cause: FallthroughCause,
) -> (AgentToolResult, i32) {
    let (outcome_tag, summary, task_id, detail_extras) = match &cause {
        FallthroughCause::Skipped(reason) => (
            "skipped",
            format!(
                "skip llm_tool_carft for `{}`: {}",
                req.command.as_deref().unwrap_or(""),
                describe_skip(reason)
            ),
            None,
            json!({ "skip_reason": describe_skip(reason) }),
        ),
        FallthroughCause::CraftFailed { task_id, err } => (
            "craft_failed",
            format!(
                "llm_tool_carft could not craft a tool for `{}`: {err}",
                req.command.as_deref().unwrap_or("")
            ),
            Some(task_id.clone()),
            json!({ "craft_error": err.to_string() }),
        ),
        FallthroughCause::ExecFailed {
            task_id,
            label,
            error,
        } => (
            "exec_failed",
            format!(
                "llm_tool_carft built `{label}` for `{}` but exec failed: {error}",
                req.command.as_deref().unwrap_or("")
            ),
            Some(task_id.clone()),
            json!({
                "artifact_label": label,
                "exec_error": error,
            }),
        ),
    };

    let mut details = json!({
        "command": req.command,
        "argv": req.argv,
        "outcome": outcome_tag,
        "fallback_exit_code": CLI_EXIT_COMMAND_NOT_FOUND,
    });
    if let Value::Object(ref mut map) = details {
        if let Value::Object(extra_map) = detail_extras {
            for (k, v) in extra_map {
                map.insert(k, v);
            }
        }
    }

    let status = match cause {
        // Skipped 是设计内的正常出口 (旁路没开 / 命令在拒绝名单),用
        // Success 而非 Error 表示"流水线本身没出问题";构造或执行失败才
        // 是 Error。两者 exit code 都是 127,区别只在 envelope。
        FallthroughCause::Skipped(_) => AgentToolStatus::Success,
        FallthroughCause::CraftFailed { .. } | FallthroughCause::ExecFailed { .. } => {
            AgentToolStatus::Error
        }
    };

    let result = AgentToolResult {
        agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
        tool: Some(TOOL_NAME.to_string()),
        cmd_name: req.command.clone(),
        status,
        task_id,
        pending_reason: None,
        check_after: None,
        estimated_wait: None,
        title: format!("{TOOL_NAME} => {outcome_tag}"),
        summary,
        details,
        cmd_args: Some(req.argv.join(" ")),
        return_code: Some(CLI_EXIT_COMMAND_NOT_FOUND),
        partial_output: None,
        output: None,
    };
    (result, CLI_EXIT_COMMAND_NOT_FOUND)
}

fn describe_skip(reason: &SkipReason) -> String {
    match reason {
        SkipReason::BypassDisabled => "intent bypass disabled in behavior cfg".to_string(),
        SkipReason::Denylisted => "command on denylist".to_string(),
        SkipReason::PreviousFailures => {
            "too many recent craft failures for this command".to_string()
        }
        SkipReason::EmptyCommand => "shell hook delivered empty command".to_string(),
        SkipReason::Other(msg) => msg.clone(),
    }
}

// =========================================================================
// tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn req(
        command: Option<&str>,
        argv: &[&str],
        agent_env_root: Option<&str>,
    ) -> CommandNotFoundRequest {
        CommandNotFoundRequest {
            command: command.map(str::to_string),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            current_dir: PathBuf::from("/tmp"),
            agent_env_root: agent_env_root.map(PathBuf::from),
        }
    }

    #[tokio::test]
    async fn empty_command_skips_with_dedicated_reason() {
        let (result, exit) = run_subcommand(req(None, &[], Some("/agent"))).await;
        assert_eq!(exit, CLI_EXIT_COMMAND_NOT_FOUND);
        assert_eq!(result.tool.as_deref(), Some(TOOL_NAME));
        assert_eq!(result.status, AgentToolStatus::Success); // skip = 设计内出口
        let reason = result
            .details
            .get("skip_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(reason.contains("empty command"), "got skip_reason={reason}");
    }

    #[tokio::test]
    async fn missing_agent_env_skips_with_bypass_disabled() {
        let (result, exit) = run_subcommand(req(Some("foo"), &["foo"], None)).await;
        assert_eq!(exit, CLI_EXIT_COMMAND_NOT_FOUND);
        let reason = result
            .details
            .get("skip_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            reason.contains("intent bypass disabled"),
            "got skip_reason={reason}"
        );
    }

    #[tokio::test]
    async fn current_stub_always_skips_when_agent_env_present() {
        // 真接 behavior cfg 之前所有命令都 skip,行为 = 旧 placeholder。
        // 这条测试在 step 1 真接后会失败 —— 那时该改成断言进入 step 2/3。
        let (result, exit) = run_subcommand(req(
            Some("missing-cmd"),
            &["missing-cmd", "--flag"],
            Some("/agent"),
        ))
        .await;
        assert_eq!(exit, CLI_EXIT_COMMAND_NOT_FOUND);
        assert_eq!(
            result.details.get("outcome").and_then(|v| v.as_str()),
            Some("skipped")
        );
        assert_eq!(result.cmd_args.as_deref(), Some("missing-cmd --flag"));
    }
}
