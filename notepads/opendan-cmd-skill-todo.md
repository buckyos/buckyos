# cmd_skill 机制 TODO（占位）

日期：2026-10-17

状态：v0.3，“已定”各项与待设计项 1 已实施（2026-10-17，见内置 cmd_skills TODO §10），其余待设计。来源：[Jarvis UI Session 提示词改造 TODO](jarvis-chat-prompt-todo.md) §8；review 原话“目前机制上还没有 cmd_skill，要先写个 todo”。实施任务在 [内置 cmd_skills TODO](opendan-builtin-cmd-skills-todo.md) §3（I01–I04、I08）。

## 已有（v0，2026-10-17 实施；同日 v1 已改为列表，见下）

- 一个 cmd_skill = Agent 包里的一份静态手册 `cmd_skills/<name>.md`。
- v1：`agent.toml [session.<class>] default_cmd_skills = ["<name>"]` 或 `create-worksession --cmd-skill` → `prompt.cmd_skills` → 冻结为 `prompt.frozen.cmd_skills = [{name, text}]`（随 Session 冻结，同一 Session 内前缀不变）。
- assembler 对 UI 与 Work 都渲染 `## Commands`（Work 段首说明是可选捷径，每份一个 `### <name>`）。
- Jarvis 的 `ui_core` 只列今天存在、且在 Session PATH 上的命令（`llm_understand_media`、`agent-session read-session / sessions / recall / ctl perceive / notify / goal / ctl subscribe|unsubscribe / task`）。
- 冻结时找不到手册是配置错误（Session 不推进），不会静默降级。

## 已定（2026-10-17 用户裁决）

- **skill 的分类**：传统 agent-skills（`agent_tool` 的 `skills_mgr`，`skills/ + skill.yaml + SKILL.md`）已废弃，代码已删除（2026-10-17）。skill 安装时分为两类：cmd_skills（命令手册，可带 PATH 层工具）和 goal_skills（Goal 使用）。
- **安装**：cmd_skills 与 goal_skills 都可经 PIKG 安装；opendan runtime 为 Agent 提供明确的 kRPC 安装接口（`skills.list/install/remove`）。
- **Work 的 cmd_skills**：在 `create-worksession` 时选好（`--cmd-skill`，可重复），上限 8 个；选中的手册全部装进 Work 的系统提示词。Work 运行中不花注意力调整手册，没有切换机制。选择由 `try_analyze_user_intent`、Goal、PLAN/DO 在创建时完成。
- **手册的口吻**：cmd_skill 是捷径，提示词写成可用的做法，不写成“必须这么做”。

## 待设计

1. **定义**（已实施，2026-10-17）：cmd_skill = 一组 CLI 的使用手册 + 可选的 PATH 层（`<agent_root>/tools/<skill>/bin`，只在选了它的 Session PATH 上）。front matter 为 `name` / `description` / `requires`（冻结时剥掉）；skill 包是 `app_type = agent_skills` 的 PIKG，唯一子包 `agent_skills` 内分 `cmd_skills/`、`goal_skills/`、`tools/`；安装记录 `.meta/skills.json`。仍待定：goal_skills 在 Goal 里如何引用（`GoalSkill.ref` 指向 `goal_skills/<name>.md`），属于 Goal 的 TODO。
2. **运行时调整（只对 UI）**：UI Session 有默认 cmd_skill，运行中可通过通用机制调整（例如 `agent-session skill use <name>`）：重新冻结 → 新 run 的前缀变化一次。要定：调整是否需要用户确认、是否跨 Session 记忆、与 `prompt.frozen` 的修订记录（`config_rev`）如何对应。Work 不做（见“已定”）。
3. **延迟暴露**：手册只列核心命令；其余命令由背景行 / 事件行 / 工具结果里的一句 hint 带出（例如事件行后附 `manage with agent-session ctl subscribe …`），配合先行 TODO“以后”项“function result 里的背景行”。
4. ~~**work session 的 cmd_skill**~~：已定，见上。
5. **待纳入的命令与现状**（实施时逐条核对）：
   - 计划任务：dcrontab，未实现（[Agent 计划任务 CLI 需求](<../doc/llm_context/agent_tool/Agent 计划任务cli工具需求.md>)）。
   - task：已实现 `agent-session task state|wait|cancel`（先行 P3）。
   - `llm_understand_media`：已接入（2026-10-17，主 TODO §13.5）——宿主的会话辅助命令 `SessionHelper` 在 `.runtime/bin` 放 `llm_understand_media` → `agent_tool llm_understand_media`，CLI 支持位置参数 locator，`ui_core` 已列出。辅助命令目前由宿主硬编码（`standard_helpers`），归入 cmd_skill 的 PATH 层是这里的待设计项。
   - 订阅 / 半订阅管理：`agent-session ctl subscribe|unsubscribe` 的会话内形式已存在（`$OPENDAN_SESSION_ID`）。
   - UI Session 命令，含群聊记录翻找：**没有** msg-center 会话历史的 CLI；`agent_tool` 的 `read --session-id` 读的是 Agent Session 的 worklog，不是群聊记录。
   - 切换 runtime：不做（xAgent §730：primary runtime 首次推进绑定、不可换）。
   - buckyos sdk 工具：只给 work session（主 TODO D6）。
