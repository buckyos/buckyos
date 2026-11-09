//根据 session传递下来的context,完成 command not found的构造
//1）先判断是否需要构造，如可以开始构造，则创建 taskid记录构造状态
//2）组合大于制造，优先在可见bin的范围内进行查找，争取通过脚本组合，让失败的命令正确工作
//3）判断是apt install合适，还是手写py / ts 脚本合适.写的脚本工具有一定通用性，放在agentRootFS/tools目录下，方便下次再走到第二步的时候可以复用
//4）构建成功，用命令行调用拿到结果
//5）构建失败，保持command not found ...
//
// ## 触发路径
//
// 当 behavior cfg 在某条 shell 上打开了意图旁路：
// 1. `opendan::agent_bash::build_exec_script` 注入 `command_not_found_handle`
//    并由 runtime 按路径规则直接写入 agent_tool 绝对路径。
// 2. shell 里跑 `missing-cmd args...` 触发 hook,bash 改调
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
// 只实现了 step 1 的拒绝分支和 step 5:流水线一律直落 step 5,对外行为
// 等价于旧 placeholder(exit 127 + 一条日志),但 envelope 已经是结构化
// `AgentToolResult`,后续实现 step 2–4 不再改 dispatcher。
//
// 文件骨架参考 `llm_explore.rs`,但入口签名不同:意图旁路的输入是上游
// dispatcher 已经分拣过的 `(command, argv)`,而不是 CLI argv 字符串列表,
// 所以省掉 `CliOpts`,直接收一个 [`CommandNotFoundRequest`] 结构体。

use std::path::PathBuf;

use serde_json::json;

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

#[derive(Debug, Clone)]
enum SkipReason {
    /// behavior cfg 没开旁路,或者 agent_env_root 缺失。
    BypassDisabled,
    /// shell hook 没传命令 (argv 为空)。
    EmptyCommand,
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
    let reason = decide_should_construct(&req);
    surface_fallthrough(&req, &reason)
}

// =========================================================================
// step 1:决策
// =========================================================================

/// 决定要不要进入构造流水线。目前一律拒绝:
/// - argv 为空 → EmptyCommand
/// - 其余 → BypassDisabled,等真接 behavior cfg + 拒绝名单 + 失败计数后改。
///
/// 待实现:
/// - 读取 `<agent_env_root>/behavior/<current>.toml` 的旁路开关
/// - 读取拒绝名单 (rm/dd/sudo 等高危命令)
/// - 读取持久化的失败计数 (同命令短时间内失败 ≥ N 次就 Skip)
/// - 创建并持久化 task_id (写进
///   `<agent_env_root>/tasks/llm_tool_carft/<task_id>.json`)
fn decide_should_construct(req: &CommandNotFoundRequest) -> SkipReason {
    if req
        .command
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return SkipReason::EmptyCommand;
    }
    // 真接 behavior cfg 之前一律 Skip,对外行为 = 旧 placeholder。
    SkipReason::BypassDisabled
}

// =========================================================================
// step 5:回落 127
// =========================================================================

/// 构造跑不通时统一出口:组装一份说明性 `AgentToolResult`,exit code
/// 固定 127,让上层 shell hook 看到 127 后补打 bash 原生 `command not
/// found` 错误。这一致性是设计契约:旁路开 vs 关,LLM 拿到的 stderr 至少
/// 包含 `command not found` 字样,否则模型容易把"旁路静悄悄吞了错误"
/// 误判成"命令成功了"。
fn surface_fallthrough(req: &CommandNotFoundRequest, reason: &SkipReason) -> (AgentToolResult, i32) {
    let outcome_tag = "skipped";
    let summary = format!(
        "skip llm_tool_carft for `{}`: {}",
        req.command.as_deref().unwrap_or(""),
        describe_skip(reason)
    );
    let details = json!({
        "command": req.command,
        "argv": req.argv,
        "outcome": outcome_tag,
        "fallback_exit_code": CLI_EXIT_COMMAND_NOT_FOUND,
        "skip_reason": describe_skip(reason),
    });
    // Skipped 是设计内的正常出口 (旁路没开),用 Success 而非 Error 表示
    // "流水线本身没出问题";exit code 仍是 127。
    let status = AgentToolStatus::Success;

    let result = AgentToolResult {
        agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
        hints: Vec::new(),
        dedup: None,
        tool: Some(TOOL_NAME.to_string()),
        cmd_name: req.command.clone(),
        status,
        task_id: None,
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
        images: Vec::new(),
    };
    (result, CLI_EXIT_COMMAND_NOT_FOUND)
}

fn describe_skip(reason: &SkipReason) -> String {
    match reason {
        SkipReason::BypassDisabled => "intent bypass disabled in behavior cfg".to_string(),
        SkipReason::EmptyCommand => "shell hook delivered empty command".to_string(),
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
