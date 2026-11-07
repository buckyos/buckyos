# xllm behavior 协议：只有原生工具时的说明自相矛盾

日期：2026-10-11

状态：已修复（2026-10-11，用户确认按建议修）。见文末“实施”。

来源：Goal Session 第一段 DV（[Goal Session TODO](opendan-goal-session-todo.md) §11.5 第 1 条）。

## 现象

DV 中 Jarvis 的 Work Session（入口 behavior `plan`，`mode = "behavior"`，基础 `[llm_context.tools] enabled = true`，没有 `tools2actions`）有时第一步就交 `<report end="true">`，内容是“本轮运行协议要求直接返回最终报告，不能执行文件检查或修改”，一个工具都不调用。6 个 Work 中有 2 个如此。UI 创建的 Work 同样受影响。

## 原因

`src/frame/agent_tool/src/xllm.rs` 生成 behavior 循环的运行协议时（约 2678 行），没有 XML action（`tools.actions` 为空）就写：

```text
- No actions are available in this run: reply with a single <response> whose <report end="true"> contains the complete final result. Do not include <actions>.
```

紧接着在有原生工具时又写：

```text
- Native function-call tools are also declared for this run; a reply that uses them is treated as a step with actions.
```

前一句是一条明确的指令，模型常按它直接结束。该文本由 2026-10-10 的提交 `upgrade opendan report` 引入。

## 建议修法（待确认）

`has_actions == false && has_native == true` 时不写 “No actions are available …”，改为说明用原生工具行动、观察结果后再交 `<report end="true">`；只有既没有 action 也没有原生工具时才写现在这句。改后补一个 `agent_tool` 单元测试（三种组合的协议文本），并在 DV 重跑一次 Work。

## 实施（2026-10-11）

- `src/frame/agent_tool/src/xllm.rs::build_runtime_protocol_for`（behavior 分支）：三种组合分开写。
  - 有 XML action：原样；有原生工具时另写 “Native function-call tools are also declared …”。
  - 只有原生工具：“To act, call one or more of the native function-call tools declared for this run (never inside `<actions>`) …”、失败后调整继续、“A reply without tool calls is the `<response>` document of the step: without `<report end="true">` the work continues with the next step. Submit `<report end="true">` … only after you have done the work and observed the results.”（与 behavior 循环的实际执行一致：一个 step 内原生调用在内层循环里执行，step 的 `<response>` 是第一条不带工具调用的回复）。
  - 两者都没有：“No actions or tools are available in this run: …”。
- 单测 `xllm::behavior_protocol_tests`（三种组合）。`cargo test -p agent_tool --lib` 247 通过、`cargo test -p llm_context` 222 通过。
- 已有 Session 的 run 不重新生成系统提示：修复对新建的 run 生效。


## 配置侧根因（2026-10-11）

触发上面问题的是 Jarvis 配置：`plan.toml`、`do.toml` 都没有 `[capabilities]`，只继承了 `agent.toml` 里的 `[llm_context.tools] enabled = true`。xllm 在只启用工具、没有列出工具时，会默认提供 `group:bash`；`tools2actions` 为 false 时，这组工具作为原生 function-call 工具提供，XML action 为空，于是落入“只有原生工具”这种组合。

约定：XML behavior（`mode = "behavior"`）只配置 actions，不配置 tools，并且要明确写出来。已给 plan/do 补上：

```toml
[capabilities]
tool_whitelist = []
action_whitelist = ["group:bash"]
```

两项都要写：只写 `action_whitelist` 时，tools 仍会默认用 bash 组，同名工具会重复注册（`tool name ... conflicts`）。内置工具不能逐个列名（具名工具只从宿主的 `host_tools` 里找，目前只有 `call_behavior`、`report`），所以最精确只能写到 `group:bash`。验证方法：用 `parse_behavior` + `overlay_llm_context` + `build_toolset` 跑一遍，plan/do 得到 `native=[]`，actions 为 read_file/write_file/edit_file/shell/wait_task/get_task_state/cancel_task。已有 Session 冻结的是旧配置，只有新建的 Work 会用到新配置。
