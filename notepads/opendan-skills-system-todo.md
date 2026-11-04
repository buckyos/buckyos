# OpenDAN Skills 体系改进计划 TODO

日期：2026-11-03

状态：v0 草案，待裁决（§6）。来源：脑图《整理 skills 体系》（ui_skills / work_skills / plan skills / goal skills / skills 的基础结构），对照 2026-11-03 工作树源码整理。关联：[cmd_skill 机制 TODO](opendan-cmd-skill-todo.md)、[内置 cmd_skills TODO](opendan-builtin-cmd-skills-todo.md)、[Goal Session TODO](opendan-goal-session-todo.md)、[OpenDAN 开发指南 §2.4](../doc/sdk/OpenDAN_Agent_Dev_Guide.md)、[Session 架构设计 §9.3 / §10](<../doc/opendan/OpenDAN Agent Session架构设计.md>)。

目标：把现有的 cmd_skills / goal_skills 两类机制整理成脑图中的四类（ui / work / plan / goal），补齐“谁选、何时选、选了之后如何反馈”的闭环，并把安装、禁用/卸载、导入、发行和 self-improve 的入口做实。原则沿用已定裁决：skill 是捷径不是规定；Work 的手册创建时选定、运行中不调；同一 Session 内系统提示词前缀不变（provider prompt cache）；新增能力不改公共 tool schema。

## 1. 现状核对（2026-11-03 工作树）

| 方面 | 当前实现 | 与脑图的差距 |
| --- | --- | --- |
| 分类与格式 | 两类：`cmd_skills/<name>.md`（+ 可选 `tools/<name>/bin`）与 `goal_skills/<name>.md`；front matter `name/description/requires/baseline/platforms`（`protocol/behavior.rs` `skill_meta`）。`SkillKind` 只有 `Cmd`/`Goal`。 | 没有 ui / work 的区分（`ui_core` 与 `share`、`aigc_*` 同属 cmd）；没有 plan skill。 |
| 存放与来源 | AgentRoot；`.meta/skills.json` 记录已安装；`origin` = package / installed / local；模板同步不覆盖已安装 skill（`opendan/src/rootfs.rs`）。 | 没有“禁用”状态；package 来源的 skill 只能改文件，不能停用。 |
| UI Session | `agent.toml [session.ui] default_cmd_skills = ["ui_core"]`（`template.rs`），创建时冻结；安装新 skill 不影响已有 Session（cache 安全），但也没有任何途径把已安装 skill 加进 UI 默认集，除了改模板里的 `agent.toml`。 | 脑图：ui_skills 默认加载、谨慎扩展、只能命令行强制安装。缺“Agent 级 UI skill 集合”设置与 CLI。 |
| Work Session | 创建者选：`analyze_intent`（`create-worksession --cmd-skill`）、Goal（同上）、plan 的 Todo（`todo add --skill`）；≤ 8；按 session-runtime 的 baseline/platform 过滤；未指定时子 Work 继承父 Work 冻结的手册（`api.rs` ~L1000）。 | 选择机制已齐。缺：选了之后的效果反馈（report 没有 skill 字段，`doc/opendan/todo.md` 的“两种信号”未实现）；plan 的手册被 Todo 继承，与“plan skill 务虚”冲突。 |
| Plan Session | `plan_simple` 是 class `work` 的一种行为，自身也通过 `--cmd-skill` 拿到手册并冻结进系统提示词（只读的规划者用不上），再被 Todo 继承。没有“分析阶段”。 | 脑图：plan_simple 有专门阶段，分析任务目标并选出 plan skill 与未来 Todo 的 work skill；plan skill 务虚、不依赖 work_skills。 |
| Goal Session | `GoalSkill {name, ref, version, note}` 存在 goal state，`goal skills add <name>=<ref>` 维护；`goal_skills/` 目录只被 list/install/remove 处理，运行时没有任何路径读取它（`runner/goal.rs` 只渲染 `<current_skills>` 名字）。 | 脑图：暂不做。保持现状，记入 Goal TODO。 |
| 安装入口 | `buckyos skill install <x.pikg> --agent`（kRPC `skills.install`，TS 侧离线校验）；`agent-session skill install <pikg|dir>`（只能在 Agent 自己的 runtime 上）；只接受本地文件。 | 缺“任意文本 → LLM 拆成本机专用 skill”的导入安装；缺按 DID / URL 从 Repo 安装。 |
| 卸载 | `skill remove` 只删 installed 来源；无 disable；opendan WebUI（`src/frame/opendan/web/src/pages/`：Home/Goals/Sessions/Workspaces/AgentState/Loader）没有 skill 页面。 | 脑图：Agent home page 提供 skill 管理，可 disable 或卸载。 |
| Self-improve | 架构 §9.3 定义了“操作经验结晶 Goal”（修正 / 清理 / 提炼）；`todo.md` 标注本版本暂不实现；旧 self_improve 行为与 attention signal 已退役（2026-11-01）。 | 没有任何从 worklog / report / memory 到 skill 的通路；连输入信号都没有。 |
| 发行 | `buckyos pikg init --kind agent-skills` → `pikg build/pack` → `app publish`（Repo candidate，未签名、未发布 DID）；样例 `test/skills_pikg_sample/`。 | 路径存在但只到 candidate；消费端 `skill install` 不认 DID/URL；Desktop 应用商店不展示 `agent_skills`。 |

## 2. 目标模型

### 2.1 四类 skill 与物理格式

| 类别 | 消费者 | 内容性质 | 物理落点（建议） | 注入方式 |
| --- | --- | --- | --- | --- |
| ui | UI / group Session | 对话里可用的命令，短 | `cmd_skills/<name>.md`，front matter `scope: ui` | Session 创建时冻结进 `## Commands`；集合由 Agent 级设置决定 |
| work | Work Session（work_simple / explore） | 一类事的命令手册 + 可选 `tools/<name>/bin` | `cmd_skills/<name>.md`，`scope: work`（缺省） | 创建者 `--cmd-skill` / Todo `--skill`，冻结 |
| plan | plan_simple | 如何拆解、排序、验收一类目标的方法论；不写具体命令 | 新目录 `plan_skills/<name>.md`，新 `SkillKind::Plan` | 第一轮“分析阶段”按需 `read`，不进系统前缀（§2.3） |
| goal | Goal Session | 判断距离、推进方式的方法论 | `goal_skills/<name>.md`（已有） | 暂不做；`GoalSkill.ref` 指向它仍是 Goal TODO |

- ui 与 work 共用 `cmd_skills/` 与 tools PATH 机制，只加一个 `scope` 字段（`ui` / `work` / `ui,work`，缺省 `work`）：安装、冻结、PATH 代码不动，已发布的 skill 包不用改目录。`ui_core` 标 `scope: ui`。
- plan 单独一个目录：它不是命令手册，不带 tools，不能被 `--cmd-skill` 点名（冻结时拒绝）。lint 规则：plan skill 正文不得出现 `agent-session create-worksession`、`todo add --skill <具体名>` 之类对 work skill 的硬依赖，允许写“这类步骤交给带 X 类手册的 Todo”。
- `skill list` 增加 `scope` / `kind=plan` 列；`--kind cmd|plan|goal`，`--scope ui|work`。

### 2.2 Agent 级 skill 状态文件

新增 `.meta/skills_state.json`（schema `opendan.skills_state/1`），与 `.meta/skills.json`（安装记录）分开，模板同步不碰：

```json
{
  "schema": "opendan.skills_state/1",
  "disabled": {"cmd:aigc_video": {"at_ms": 0, "by": "did:bns:owner", "reason": "no provider"}},
  "ui_cmd_skills": ["ui_core", "share"],
  "usage": {"cmd:share": {"frozen": 12, "reported_used": 7, "reported_unhelpful": 1, "last_ms": 0}}
}
```

- `disabled`：任何来源（含 package）都可禁用；禁用后 `skill list` 缺省不显示（`--all` 显示并标 `disabled`），选择提示词看不到它，显式点名冻结时拒绝并说明“disabled by owner”，继承链里被过滤，tools 不进 PATH。已运行的 Session 不受影响。
- `ui_cmd_skills`：覆盖 `agent.toml default_cmd_skills`；为空键时用模板缺省。只能由 owner 身份通过 kRPC `skills.ui_set` 改（§3.1）。
- `usage`：§3.4 的反馈信号累计。

## 3. 脑图逐项对照与设计

### 3.1 ui_skills：默认加载、谨慎扩展、只能命令行强制安装

- 保持“UI Session 创建时冻结、运行中不改”的规则：安装 / 启用 / 禁用都只影响之后创建的 UI Session。UI Session `idle_unload_secs = 900`，重建很频繁，“下次生效”可接受；不做 `skill use` 式的运行中重冻结（[cmd_skill 机制 TODO](opendan-cmd-skill-todo.md) 待设计项 2 据此关闭）。
- 新 CLI（owner 身份，走 opendan kRPC，`access: write`）：
  `buckyos skill ui list|add <name>|remove <name> --agent <id>`（kRPC `skills.ui_get` / `skills.ui_set`）。`add` 只接受 `scope` 含 `ui` 且未禁用的 cmd_skill。
- Agent 自己没有这个入口：`agent-session skill` 不提供 `ui` 子命令，kRPC 只授权 owner App；`ui_core` 手册写明“用户要把某手册加进对话时，告诉用户命令，不代为执行”。这就是脑图的“只能通过命令行强制安装”。
- 谨慎扩展的硬约束：`scope: ui` 的手册正文上限（建议 6 KB，`skill lint` 警告、安装不拒绝）；`skill ui add` 输出当前 UI 集合的总字节数；`skill list` 显示每份手册大小。
- 文档：`ui_core` 继续只列对话里真会用的命令；需要在对话里直接用的新能力（例如 `share`）走 `scope: ui,work`，由 owner 决定是否 `ui add`。

### 3.2 work_skills：谁创建 work session，谁选择

现状已满足“创建者选”：`analyze_intent` → `create-worksession --cmd-skill`；Goal 激活 → 同上；plan 的 Todo → `todo add --skill`。要补的是三点：

1. **plan 不再把自己的手册传给 Todo**：`api.rs` 的继承规则改为“父 Session 行为是 work_simple / explore 时继承；父是 plan_simple 时不继承”。plan session 创建时传入的 `--cmd-skill` 不再冻结进它的系统提示词，而是记为 `prompt.work_skill_candidates`，在它第一轮输入里以 `<skill_candidates>` 列出（名字 + description），供分析阶段参考。Todo 不带 `--skill` 就没有手册——plan 必须为每个 Todo 明确选择（§3.3）。
2. **三处选择提示词统一**：`analyze_intent.toml`、`goal.toml`、`plan_simple.toml` 里关于“看 `skill list`、匹配 runtime baseline/platform、≤ 8、是捷径不是规定”的段落抽成 `behaviors/skill_select_rule.inc`（`__INCLUDE`），避免三份各自漂移。
3. **效果反馈进 report**（§3.4）。

### 3.3 plan skills：plan_simple 的分析阶段

- `plan_simple.toml` 第一轮改为两步：**分析** → **建表**。分析步要做：读目标与 runtime 上下文；`agent-session skill list --kind plan`，挑最多 2 份 plan skill 并 `read $OPENDAN_AGENT_ROOT/plan_skills/<name>.md`；`agent-session skill list --kind cmd --scope work`（加上 `<skill_candidates>`）；输出一段结构化的“计划简报”——目标分解方式、验收方式、每个 Todo 预计用的 work skill。简报用 `report`（非 end）的 `summary` 记下，Session 页面能看到。
- plan skill 按需 `read`，内容进对话历史而不是系统前缀：plan session 寿命长、tick 稀少，重冻结一次前缀的收益不大，按需读最简单，也符合“务虚”——它只在建表时用一次。不做 `agent-session skill use`。
- 冻结规则：`--class plan_simple` 的 Session 不冻结任何 cmd_skill（`freeze` 时 `cmd_skills` 置空、候选另存）；`--behavior work_simple` 的照旧。
- 首批 plan skill（随 Jarvis 包）：`plan_software_change`（改代码类：探查 → 改动 → 验证 Todo 的拆法与验收标准写法）、`plan_content_production`（AIGC / 文档类：素材 → 生成 → 审核）、`plan_research`（只读调研类：并行 explore 的切分与汇总）。每份 ≤ 3 KB，不含命令。
- 关联修改：`analyze_intent` 创建 plan session 时，`--cmd-skill` 语义改为“候选”，结果 schema 的 `work.cmd_skills` 改名 `work.skill_candidates`（plan）/ 保持 `cmd_skills`（simple）。

### 3.4 效果反馈（self-improve 的输入，也是 work_skills 的闭环）

- `report` 工具的 schema（`runner/reports.rs`）增加可选 `skills_feedback: [{"name", "used": bool, "note"}]`；`work_simple.toml` / `explore.toml` 的完成段写明：带了手册就填（用了没用、哪一步不准）。宿主在 `is_end` 时把它写进 report 对象，并累计到 `.meta/skills_state.json` 的 `usage`。
- 两种信号（`doc/opendan/todo.md`）的落地：① 冻结了某 skill 的 Session 结束 → 看它的 `skills_feedback`；② 创建者看过 `skill list` 却没选任何 skill → `analyze_intent` / plan 的结果里带 `skills_considered: []` 与 `no_skill_reason`（可选字段），宿主记一条 `usage.none_chosen`。
- `agent-session skill list --stats` / `buckyos skill list --stats` 显示 frozen / used / unhelpful / last。

### 3.5 goal skills：暂不做

保持 `goal_skills/` 目录与 `GoalSkill` 引用现状；在 [Goal Session TODO](opendan-goal-session-todo.md) 记一条：`GoalSkill.ref` 指向 `goal_skills/<name>.md` 时由激活渲染读入 `<current_skills>` 正文。本计划不改 Goal 的选择逻辑，只把 §3.2 第 2 条的 include 用上。

### 3.6 基础结构：如何安装

**常规导入安装（任意文本 → LLM installer → 本机专用 skills）**

- 入口：`buckyos skill import (<file> | <url> | -) --agent <id> [--name <slug>] [--scope work|ui|plan] [--dry-run]` → kRPC `skills.import`（owner）；对话里用户贴文本说“装成 skill”时，`analyze_intent` 可创建同样的 work（`--behavior skill_import`）。
- 实现为 Jarvis 内置行为 `behaviors/skill_import.toml`（work_simple 变体，cmd_skill `skill_authoring`）：读文本 → 判断拆成几份、各自 kind/scope → 写 front matter（`name/description/requires/scope/baseline/platforms`）与正文 → 在当前 runtime 上逐条核验正文提到的命令是否存在（`command -v`、`--help`），不存在的写进 `requires` 并降为 `baseline: extended` → 把结果放到临时目录 `cmd_skills/`、`plan_skills/` → `agent-session skill install <dir>`，包名 `imported-<slug>`，安装记录加 `origin: imported`、`imported_from: {digest, runtime_id, platform}` → `report`。
- “本机专用”的含义：`platforms` 固定为导入时的平台，`baseline` 按核验结果；导入的 skill 不进发行路径（`pikg init` 不认 `origin: imported` 的目录；要发行先人工整理）。
- `--dry-run` 只产出草稿到 `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/`，用户看过再 `skill install <dir>`。缺省直接安装、通知 owner 结果；是否改为缺省需确认见 §6。

**强制安装（ui_skills）**：§3.1 的 `buckyos skill ui add`。

**从发行渠道安装**：`buckyos skill install <source>` 的 `<source>` 对齐 `app install` 的来源解析（本地 `.pikg` / HTTP(S) URL / Catalog DID，`--from` 消歧）；非本地来源由 CLI 下载到本机后走现有离线校验与 `skills.install`。

### 3.7 基础结构：如何卸载 / 禁用

- 协议：kRPC `skills.set_enabled {kind, name, enabled, reason?}`、`skills.ui_get/ui_set`、`skills.list` 返回 `enabled` 与 `scope`；CLI `buckyos skill disable|enable <name> --agent`；`agent-session skill` 不提供 disable（能力边界由 owner 定）。
- opendan WebUI 新页面 `pages/Skills.tsx`（路由与 Goals 同级），Home 卡片加“Skills：N 可用 / M 禁用”：列表（kind、scope、来源、runtime 要求、大小、usage）、启用 / 禁用、卸载（仅 installed/imported，二次确认）、UI 集合编辑（§3.1 同一 kRPC）、从本地 `.pikg` 安装（浏览器里复用 websdk 的 `pikg_archive.ts` 解析后调 `skills.install`，与 CLI 同一份校验代码）。
- Desktop 侧不做；WebUI 就是脑图里的 agent home page。

### 3.8 基础结构：self improve——按 worklog 与 memory 改进 skills

按架构 §9.3 做成内置 maintain Goal，分两步，不与价值观层绑定：

- **A. 信号**：§3.4 的 `skills_feedback` 与 `usage`；Memory 整理 Goal 产出的“能力缺口 / 操作经验”认知保持不变（《Agent Memory 认知管理需求》§… 已把经验认知定义为结晶输入）。
- **B. `skill_crystallize` Goal**（`[session.goal.crystallize]`，缺省关闭，owner 在设置里打开；行为 `behaviors/skill_crystallize.toml`，预算与 memory_maintenance 同级）。每次激活：读最近 N 个带 `skills_feedback` 的 report 与相关认知 → 三类决定：
  - 修正：对 `origin: imported|local` 的手册直接改写（保留 `revisions/` 备份与修订记录）；对 `origin: package` 的手册只写草稿到 `skills_drafts/<name>.md` 并通知 owner（模板同步会保留本地改动，但自动改模板手册风险大，先人工）。
  - 提炼：同类 Work 反复出现、`no_skill_reason` 相同 → 产出候选 `skills_drafts/<slug>/`，通知 owner `skill install <dir>`。
  - 清理：`usage` 显示 60 天未冻结或 unhelpful 占比高 → 建议禁用（通知，不自动禁用）。
- 边界：不改 ui 集合、不改 package 手册正文、不自动安装新 skill；所有自动改动在 `skills_state.json` 留审计记录。

### 3.9 skill 作者如何发行：走 PIKG 路径

路径已存在：`buckyos pikg init --kind agent-skills --source ./skills` → `pikg doctor/build/pack` → `app publish`（Repo candidate）→ Owner 签名 / BNS 发布（现有 App 流程）。要补：

- `pikg doctor` 对 `agent_skills` 子包做 skill lint（front matter 固定词、`scope`、`plan_skills/` 允许、plan 正文禁依赖、大小警告）；`skills.install` 复用同一套校验（Rust 侧 `skills.rs` `install` 已有 `skill_requirements`，加 scope/plan 规则）。
- 消费端 §3.6 的 `skill install <did|url>`。
- 文档：新增 `doc/sdk/skill-dev-quickstart.md`（写手册 → 本地 `agent-session skill install <dir>` 试用 → 打包 → 发布 → 安装），`opendan_agent_dev.md` 手册只保留指引。Desktop 商店展示 `agent_skills` 类别不在本计划内。

## 4. 实施任务清单

路径默认相对 `buckyos/`；`../buckyos-websdk/` 为同级仓库。

### P0 格式与状态（其余阶段的基础）

| # | 任务 | 落点 |
| --- | --- | --- |
| S01 | `SkillKind::Plan` 与 `plan_skills/` 目录；`SkillMeta.scope`；front matter `scope` 解析与校验（固定词）；`install`/`place`/`read_pikg`/`read_package_dir` 认新目录；lint 规则（plan 禁依赖、ui 大小警告） | `src/frame/lib_opendan/src/protocol/behavior.rs`、`skills.rs`、`opendan/src/rootfs.rs`（骨架目录） |
| S02 | `.meta/skills_state.json`：`disabled` / `ui_cmd_skills` / `usage`；读写与原子替换；`list` 合并 `enabled` | `skills.rs`、`state/behaviors.rs` |
| S03 | 冻结规则：禁用的拒绝 / 过滤；`scope: ui` 不能被 `--cmd-skill` 点名；plan kind 不能冻结；`ui_cmd_skills` 覆盖 `default_cmd_skills` | `state/behaviors.rs` `freeze_config`、`template.rs`、`api.rs` |
| S04 | kRPC `skills.set_enabled`、`skills.ui_get/ui_set`、`skills.list` 新字段；权限表 | `src/frame/opendan/src/service.rs` |
| S05 | CLI：`buckyos skill list --kind/--scope/--all/--stats`、`disable/enable`、`ui list/add/remove`；`agent-session skill list` 同步（无 ui/disable 子命令） | `../buckyos-websdk/cli/modules/skill.ts`、`src/frame/lib_opendan/src/xagent_cli.rs` |
| S06 | `ui_core` 标 `scope: ui`；现有 `cmd_skills/*.md` 补 `scope`；`opendan_agent_dev.md` / 开发指南 §2.4 / AGENTS.md 联动表更新 | `src/apps/jarvis_agent/agent/cmd_skills/`、`doc/sdk/OpenDAN_Agent_Dev_Guide.md`、`AGENTS.md` |

### P1 选择闭环

| # | 任务 | 落点 |
| --- | --- | --- |
| S07 | plan_simple 不冻结手册；`--cmd-skill` → `prompt.work_skill_candidates`；首轮输入渲染 `<skill_candidates>`；Todo 不继承 plan 的手册 | `api.rs`、`protocol/config.rs`、`runner/inputs.rs` |
| S08 | `plan_simple.toml` 分析阶段（读 plan skill、选 Todo skill、计划简报）；三份 plan skill 随 Jarvis 包 | `src/apps/jarvis_agent/agent/behaviors/plan_simple.toml`、`agent/plan_skills/` |
| S09 | `skill_select_rule.inc` 抽取并 include 进 analyze_intent / goal / plan_simple；analyze_intent 结果 schema 的 `skill_candidates` | `agent/behaviors/*.toml` |
| S10 | `report.skills_feedback`；宿主落 report 与累计 `usage`；`skills_considered` / `no_skill_reason`；`--stats` 输出 | `runner/reports.rs`、`runner/drive.rs`、`skills.rs`、`work_simple.toml`、`explore.toml` |
| S11 | 测试：冻结规则单测（禁用 / scope / plan）、继承规则、report 累计；DV：创建 plan → Todo 带 `--skill` → report 反馈可见 | `src/frame/lib_opendan/src/**/tests`、`test/` |

### P2 安装与管理体验

| # | 任务 | 落点 |
| --- | --- | --- |
| S12 | `skills.import` kRPC + `buckyos skill import`；`behaviors/skill_import.toml` + `cmd_skills/skill_authoring.md`；`origin: imported` 记录 | `service.rs`、`skill.ts`、Jarvis 包、`skills.rs` |
| S13 | `buckyos skill install <source>` 来源解析对齐 `app install`（pikg / url / catalog） | `../buckyos-websdk/cli/modules/skill.ts`（复用 `app.ts` 的 source 解析） |
| S14 | opendan WebUI `pages/Skills.tsx` + Home 卡片；浏览器内 `.pikg` 解析复用 websdk `pikg_archive.ts` | `src/frame/opendan/web/src/` |
| S15 | `pikg doctor` skill lint；`doc/sdk/skill-dev-quickstart.md`；样例包加 `plan_skills/` | `../buckyos-websdk/cli/modules/pikg.ts`、`doc/sdk/`、`test/skills_pikg_sample/` |

### P3 Self-improve

| # | 任务 | 落点 |
| --- | --- | --- |
| S16 | `skill_crystallize` 内置 Goal（缺省关闭）：行为、预算、`skills_drafts/` 与 `revisions/` 约定、通知文案 | `agent.toml [session.goal.crystallize]`、`behaviors/skill_crystallize.toml`、Loader 的内置 Goal 种子（参照 memory_maintenance） |
| S17 | 审计：自动改动写入 `skills_state.json`；WebUI Skills 页显示草稿与修订 | `skills.rs`、`pages/Skills.tsx` |

顺序：P0 → P1 可以合并为一个里程碑（格式一旦定下就同时改选择提示词）；P2 的 S13/S15 独立，可并行；P3 依赖 S10 的信号积累，最后做。

## 5. 验收

- 装一个 `scope: work` 的 skill 包后：新 UI Session 的系统提示词不变；`skill ui add` 后新 UI Session 多出该手册，旧 Session 不变。
- `skill disable aigc_video` 后：`analyze_intent` 看不到它；`create-worksession --cmd-skill aigc_video` 被拒且原因是 disabled；已运行 Session 不受影响。
- 创建 plan session：它的系统提示词没有 `## Commands`；第一轮能看到候选与 plan skill 列表；每个 Todo 的 work session 只带 plan 明确给的 `--skill`。
- Work 结束的 report 带 `skills_feedback`，`skill list --stats` 计数变化。
- `buckyos skill import ./howto.md --agent x` 产出 1 个以上 `origin: imported` 的 skill，`platforms` 为本机平台，正文提到的命令都在本机核验过。
- `skill install did:bns:xxx.owner` 与本地 `.pikg` 结果一致。
- WebUI Skills 页能列出、禁用、卸载、改 UI 集合。

## 6. 待裁决

1. ui / work 用 `scope` 字段区分（建议）还是拆成 `ui_skills/` 目录？拆目录要改 PIKG 子包布局与样例。
2. plan skill 按需 `read`（建议）还是进系统前缀一次性重冻结？
3. Todo 不写 `--skill` 就没有手册（建议，逼 plan 明确选择）还是缺省继承 `<skill_candidates>` 全部？
4. `skill import` 缺省直接安装并通知（建议）还是缺省 `--dry-run` 等 owner 确认？
5. 结晶 Goal 对 `origin: imported|local` 手册自动改写（建议，留备份）还是一律走草稿 + 人工确认？
6. `report.skills_feedback` 对带手册的 Work 是必填（宿主校验缺失则 nudge 一次）还是可选？
