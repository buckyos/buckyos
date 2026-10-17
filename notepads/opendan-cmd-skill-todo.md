# cmd_skill 机制 TODO（占位）

日期：2026-10-17

状态：v0.1 占位，未设计。来源：[Jarvis UI Session 提示词改造 TODO](jarvis-chat-prompt-todo.md) §8；review 原话“目前机制上还没有 cmd_skill，要先写个 todo”。

## 已有（v0，2026-10-17 实施）

- 一个 cmd_skill = Agent 包里的一份静态手册 `cmd_skills/<name>.md`。
- `agent.toml [session.<class>] default_cmd_skill = "<name>"` → `SessionTemplate.default_cmd_skill` → `SessionConfig.prompt.cmd_skill`（名字）→ 冻结时由 `BehaviorCatalog::cmd_skill(name)` 读出 → `prompt.frozen.cmd_skill = {name, text}`（随 Session 冻结，同一 Session 内前缀不变）。
- assembler 只对 UI Session 把它渲染成 system 段的 `## Commands`；Work / Goal 尚未使用。
- Jarvis 的 `ui_core` 只列今天存在、且在 Session PATH 上的命令（`llm_understand_media`、`agent-session read-session / sessions / recall / ctl perceive / notify / goal / ctl subscribe|unsubscribe / task`）。
- 冻结时找不到手册是配置错误（Session 不推进），不会静默降级。

## 待设计

1. **定义**：cmd_skill = 一组 CLI 的使用手册 + 可选的 PATH 层（`<agent_root>/tools/<skill>/bin`），作为 Agent 能力扩展的唯一形态（Dev Guide §5 原则）。v0 只有手册。
2. **运行时调整**：UI Session 有默认 cmd_skill，运行中可通过通用机制调整（例如 `agent-session skill use <name>`）：重新冻结 → 新 run 的前缀变化一次。要定：调整是否需要用户确认、是否跨 Session 记忆、与 `prompt.frozen` 的修订记录（`config_rev`）如何对应。
3. **延迟暴露**：手册只列核心命令；其余命令由背景行 / 事件行 / 工具结果里的一句 hint 带出（例如事件行后附 `manage with agent-session ctl subscribe …`），配合先行 TODO“以后”项“function result 里的背景行”。
4. **work session 的 cmd_skill**：用户可扩展的 cmd_skill 在 `try_analyze_user_intent` 里探索，在 `create-worksession` 时选定（`--skill`），work session 的默认 cmd_skill 由分析决定。
5. **待纳入的命令与现状**（实施时逐条核对）：
   - 计划任务：dcrontab，未实现（[Agent 计划任务 CLI 需求](<../doc/llm_context/agent_tool/Agent 计划任务cli工具需求.md>)）。
   - task：已实现 `agent-session task state|wait|cancel`（先行 P3）。
   - `llm_understand_media`：已接入（2026-10-17，主 TODO §13.5）——宿主的会话辅助命令 `SessionHelper` 在 `.runtime/bin` 放 `llm_understand_media` → `agent_tool llm_understand_media`，CLI 支持位置参数 locator，`ui_core` 已列出。辅助命令目前由宿主硬编码（`standard_helpers`），归入 cmd_skill 的 PATH 层是这里的待设计项。
   - 订阅 / 半订阅管理：`agent-session ctl subscribe|unsubscribe` 的会话内形式已存在（`$OPENDAN_SESSION_ID`）。
   - UI Session 命令，含群聊记录翻找：**没有** msg-center 会话历史的 CLI；`agent_tool` 的 `read --session-id` 读的是 Agent Session 的 worklog，不是群聊记录。
   - 切换 runtime：不做（xAgent §730：primary runtime 首次推进绑定、不可换）。
   - buckyos sdk 工具：只给 work session（主 TODO D6）。
