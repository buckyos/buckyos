# OpenDAN Skills 体系改进计划 TODO

日期：2026-11-03

状态：v0.3。**2026-11-09 已按本文实施（未提交），逐项状态、与正文的偏差和未验证项见 §0**。v0.2：**2026-11-09 owner 答复 §6 / §7.3 的全部待裁决，已回写正文**：删 `skills_feedback`（§3.4）、只有 owner 能装（§3.6 / §3.6.1）、`--yes` 覆盖全部风险等级、目录布局与对外 kind 改名随 S01（§2.1）、plan skill 只给 plan_simple（§3.3）；新增 §3.6.2（`commands` 声明与可用性判定）与任务 S22 / S23；回写时的推定（待确认）与新问题 15–17 的答复见 §6.1。v0 为 2026-11-03 草案；**2026-11-05 按 [runtime 工作模型与生命周期 TODO](opendan-agent-runtime-lifecycle-todo.md) 第一期（P0 工作模型 + P1 记录模型，提交 c88404aaf）复核**：§1.1 记 runtime 侧的变化；§2.2、§3.6、§3.6.1 A–D、§4 P2 按“私有 runtime 永远在、skill 自装命令缺省装私有、connector + caps + grant、`runtime_exec / runtime_put`”改写；§6 的 1、10、12、13 据 runtime 侧裁决关闭，新增 14。来源：脑图《整理 skills 体系》（ui_skills / work_skills / plan skills / goal skills / skills 的基础结构），对照 2026-11-03 工作树源码整理。关联：[cmd_skill 机制 TODO](opendan-cmd-skill-todo.md)、[内置 cmd_skills TODO](opendan-builtin-cmd-skills-todo.md)、[Goal Session TODO](done/opendan-goal-session-todo.md)、[OpenDAN 开发指南 §2.4](../doc/sdk/OpenDAN_Agent_Dev_Guide.md)、[Session 架构设计 §9.3 / §10](<../doc/opendan/OpenDAN Agent Session架构设计.md>)。

目标：把现有的 cmd_skills / goal_skills 两类机制整理成脑图中的四类（ui / work / plan / goal），补齐“谁选、何时选、选了之后如何反馈”的闭环，并把安装、禁用/卸载、导入、发行和 self-improve 的入口做实。原则沿用已定裁决：skill 是捷径不是规定；Work 的手册创建时选定、运行中不调；同一 Session 内系统提示词前缀不变（provider prompt cache）；新增能力不改公共 tool schema。

## 0. 实施状态（2026-11-09；已提交 `174dc06f7`，websdk `1052876`）

> **2026-11-09 新裁决（实施之后；[runtime TODO](opendan-agent-runtime-lifecycle-todo.md) 裁决 28 与 §7.2，owner）：skill 不能带二进制，只是纯文本；这是分发的安全边界。安装脚本也不保留：安装 skill 就是确定性的复制。带命令的东西做成扩展包；skill 包依赖扩展包，做 ext tool 的人顺手再发一个 skills pikg。**
>
> 这推翻了本文“work skill 可以带 `tools/<name>/bin` 和 `install.toml`”的设计（§2.1、§3.6.1 C / D、裁决 10 / 12 / 13）。owner 的结论是删除：S18–S20 里已经实现的 cmd 安装拆掉，S21 取消，下面的偏差 2、5 随之失效。**本文正文还没有按它回写**，任务先记在这里。
>
> **2026-11-09：S24 已实施（未提交），与 runtime TODO 的 R17、R19 一起，记录在那份文档的 §11。本文正文仍未回写。**
>
> **S24 拆除 skill 自带命令。** skill 包只剩 `SKILL.md` 与 `references/`，安装 = 校验 + 复制。要动的地方：`skills.rs`（`tools/` 布局、`install.toml`、安装计划里的脚本风险、`install_cmd` / `check_commands`、按 runtime 的安装状态）、`runtime/mod.rs` 的 `skill_tool_dirs`、`api.rs` 的 `skill_runtime`、`home.rs` / `service.rs` 的 `skills.install_cmd` 等方法、websdk `skill.ts` 的 `install-cmd` / `check`、lint（改为拒绝 `tools/` 和非文本文件）、Skills 页的落点与确认文案、`test/skills_pikg_sample/skills/tools/`、`skill_authoring` / `opendan_agent_dev` 手册、`doc/sdk/skill-dev-quickstart.md`、`AGENTS.md`。拆完以后，安装计划里与脚本有关的风险项、按 runtime 的安装状态、落点选择都没有了；手册依赖的命令仍由 `commands` 加扩展包目录的 `provides` 给出推荐（扩展包 X5，不变）。runtime 一侧同步做那份文档的 R17、R19。

> **2026-11-10：Skills 页与 [Agent UI 升级规划](<BuckyOS Agent UI 升级规划.md>) §3 在界面形态上的差距（手机样式、单个 skill 的详情页与正文、四组计数、折叠的“已停用”小节、安装入口合并）不在本文，也不在 [Agent UI 升级 TODO](agent-ui-upgrade-todo.md)，另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)（那份 TODO 的 D19）。**

§1 / §1.1 记的是实施前的现状，留作对照；当前行为以代码为准。落点：`src/frame/lib_opendan/src/skills.rs`（布局、状态文件、lint、安装计划与风险、`install.toml`、草稿、改写）、`protocol/behavior.rs`（`SkillKind` / `SkillScope` / 状态类型、`[capabilities] cmd_skills`）、`state/behaviors.rs`（冻结规则）、`api.rs`（点名校验、使用记录、skill 落点级）、`src/frame/opendan/src/{home.rs,service.rs}`（owner 方法与权限）、`../buckyos-websdk/cli/modules/{skill.ts,skill_lint.ts,pikg.ts}`、`src/frame/opendan/web/src/pages/Skills.tsx`、`src/apps/jarvis_agent/agent/`（手册迁移、三份 plan skill、`skill_select_rule.inc`、`skill_import` / `skill_crystallize` / `skill_authoring`）。kRPC 一览在 `src/frame/opendan/README.md`「Skills」。

| 任务 | 状态 | 说明 |
| --- | --- | --- |
| S01–S06、S23 | 已做 | CLI 没有嵌套动词，UI 集合是 `buckyos skill ui-list / ui-add / ui-remove`；`agent-session skill` 只有 `list / status / revise` |
| S07–S10 | 已做 | plan 由行为声明 `[capabilities] cmd_skills = "candidates"` 识别，不写死行为名；`usage` 只在创建时已冻结的 Work 上计数 |
| S11 | 单测 / 集成测试已做，DV 未跑 | 见下「未验证」 |
| S12 | 已做 | `skills.import` 把文本写到 `skills_drafts/.inbox/<id>/source.md` 并起 `skill_import` Work |
| S13 | 部分 | 本地 `.pikg` / 目录 / URL / `--draft` 已做；**Catalog DID 来源未做**：解析 DID 得到的是 AppDoc，control-panel / repo 没有把包字节交给 CLI 的接口，`--from did` 报 `SKILL_SOURCE_UNSUPPORTED` |
| S14、S17 | 已做 | 浏览器内读 `.pikg` 用页面自带的 `src/pikg.ts`（websdk 的 `pikg_archive.ts` 在 CLI 树里、按文件路径读，浏览器构建不带它） |
| S15 | 已做 | `pikg doctor` 的 `skill_lint` 与 Rust 同一套规则（`skill_lint.ts` 是移植，两处的 `commands_in_manual` 要一起改）；样例包已重建 |
| S16 | 已做 | `[session.goal.crystallize]` 缺省关闭；种子 key `builtin:skill_crystallize`，每天一次，`max_works = 0` |
| S18–S20 | 已做 | 含 skill 落点级（runtime R4 的这一半：`api.rs` `skill_runtime`）。节点上的 `tools/<name>/bin` PATH 前置（R3）仍未做，装到非私有 runtime 的路径没有在真实节点上跑过 |
| S21 | 未做 | 按正文，等 runtime R6 |
| S22 | 已做 | 每个 work / ui skill 一条，带 `unmet`、`commands`、`install` |

与正文的偏差（实施时定的，待 owner 过目）：

1. **ui 手册大小警告定在 16 KiB**，不是 §3.1 的 6 / 8 KB：`ui_core` 实施前已是 13.9 KB（§3.1 的 6.7 KB 是旧数），按“以它为基准”取 16 KiB。`ui_core` 本身没有压缩。
2. **ExtTools 命令拒装只拦安装脚本**：`install.toml` 有 `[install.<os>]` 且核验清单里有 ExtTools 命令时拒绝。包在 `tools/<name>/bin` 里自带同名命令仍允许（现有用例 `a_skill_cannot_shadow_a_standard_command_but_may_shadow_an_extension_one` 明确要这个行为），与 R-U9“一律拒绝”的字面不同。
3. **状态机不落 `disabled`**：禁用只记在 `disabled` 表，`install[…].status` 只有 `installing / enabled / failed`；`enabled` 是两者合出来的。
4. **`plan_digest` 只盖包名、版本和内容**，不含落点 runtime：确认页换 runtime 重新要 plan 时 digest 不变。
5. **冻结时的落点说明**：命令不在本 Session runtime 上的手册仍注入正文，末尾说明命令装在哪（私有 Session 提示用 `agent-session run --runtime <id>`）；没有做成“不注入”。
6. **plan skill 的 lint 是警告**（`plan_depends_on_commands`），带 `commands` / `scope` 才是错误；§7.2 第 4 点“对导入产物生效”靠 `skill_authoring` 手册要求草稿过 lint。
7. `skills.status` 每条多了 `off`（禁用或命令未装好的原因）、`installed_on`；顶层多了 `baseline`、`platform`。
8. analyze_intent 的结果 schema：`work.cmd_skills` 与 `work.skill_candidates` 都是必填数组（不适用的给 `[]`），因为 strict JSON 输出要求每个属性都在 `required` 里。

未验证：

- 没有在真实 Zone 上跑过任何一项：WebUI 对真实服务、CLI 对真实 Agent、`skill_import` / `skill_crystallize` / 改写后的 `plan_simple` 对真实模型，§5 的验收条目都只有单测 / 集成测试 / mock e2e 覆盖。
- 容器内 OpenDAN 要重建 aios 镜像后才有这些改动。
- 既有 AgentRoot 上的平铺手册和 `.meta/skills.json` 里的 `cmd:<name>` 记录不再被识别（breaking change，没有迁移）。

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

### 1.1 runtime 第一期落地后的变化（2026-11-05 复核，提交 c88404aaf）

skills 侧代码未动（`SkillMeta` 仍只有 `requires / baseline / platforms`，没有 `scope`、`skills_state.json`、plan skill；**11-09 更正**：`commands` 与 `skills.status` 已随 ExtTools X5 加上，提交 `7d0b359c7`，见 §3.6.2；plan_simple 仍冻结 `--cmd-skill` 并被 Todo 继承，`api.rs` 的继承规则只看父 Session 是否 `kind == Work`），但本计划依赖的 runtime 面已经换了形状：

| 方面 | 2026-11-03 的假设 | 现在 | 对本计划的影响 |
| --- | --- | --- | --- |
| 命令跑在哪 | work 缺省落 owner 的 `default_runtime_id`（Desktop 上是 host）；skill 命令要跟着 work 的 runtime 走平台矩阵 | **私有 runtime 永远在**（`private`，别名 `container` / `local`，原生运行时也 `available`，grant 为 `install = system`、`workspace = true`）；chat 恒私有；work 不写 `--runtime` 就是私有；owner 默认 runtime 只回答“用户说‘在我电脑上’指哪台”。`agent-session` 只在工作 runtime 为私有的 session 里有，其它 runtime 上的 session 是单机 session | **skill 自装命令缺省装私有**（runtime TODO 裁决 15），§3.6.1 C 从“平台矩阵”收窄成“私有 + 例外”；`ui_core`、`skill_import` work 天然都在私有侧（`agent-session skill install` 随裁决 4 删除） |
| runtime 记录 | `state/runtimes/<id>.json` v3：`kind / platform / baseline / availability / default` | v4：`connector {native \| node_executor \| ssh}` + `caps {os, arch, shell, desktop, baseline, baseline_check, tools_check}` + `grant {ops, roots, computer, install: none \| agent_bin \| system, workspace, expires_at_ms}` + `desc / enabled / lifecycle: active \| retired / usage {sessions_total, …, skill_commands}` | 候选 runtime 改读 `usable()` 与 `grant.install`（§3.6.1 C）；安装记录的 key 用规范 id（`private`，不是 `container`）；`usage.skill_commands` 就是 plan 全表“skill 命令”一列，由安装写入（kRPC `runtimes.note_usage` 与 `RuntimeUsageEvent::SkillCommand` 已有，`skills.rs` 还没调） |
| Agent 级执行原语 | 只有 Session 驱动路径与宿主探测 `run_probe` | `runtime::runtime_exec(runtime_id, command, cwd, timeout)` / `runtime_put(runtime_id, local, remote_dir)` / `runtime_op(op_id)`，输出存 `state/runtime_ops/<op_id>.json`；私有侧 PATH 自动前置 `<agent_root>/tools`；`runtimes.check {runtime_id, commands?}` 写 `caps.tools_check` | §3.6.1 C 的 install / check 直接用它们，不再“复用 run_probe 那条路” |
| 节点上的 skill 命令 | 安装时把 `bin/` 复制到 `<root>/.opendan/<agent>/tools/<name>/bin`，“补 §6.1 欠的 helper” | 节点**不装 helper**（R-W10 已裁决，已写进 Session Directory Protocol §9 与 primary-runtime TODO §6.1）；节点 Agent 目录定为 `<账户 home>/.opendan/<agent_id>/`，PATH 前置走每次 exec 的 `env_overrides.PATH`（R-U4，**降为 P2**：多数 skill 命令来自 ExtTools） | S19 的节点部分 = runtime TODO R3，不在本计划里先做 |
| 冻结过滤 | 按 session-runtime 的 `baseline / platform` | 不变，读 `runtime.frozen.caps`；cmd_skill 只声明 `baseline / platforms`，**不加新字段**（R-W6，`runs_on` 否决） | 待裁决 14 已关闭并实现（§3.6.2）：`commands` 是依赖声明，不是运行位置；冻结仍只按 `baseline / platforms` 过滤，缺命令只在 `## Commands` 末尾提示 |
| 选 runtime 的级别 | `freeze_runtime`：显式 > 继承 > owner 默认 | 显式 > 继承 > skill 落点（`RuntimeSelection::Skill` 枚举已有、逻辑未做 = runtime TODO R4）> 私有；owner 默认不再参与 | §3.6.1 D 的“自动取 `pinned_runtime`”就是第三级，待裁决 12 关闭 |
| 提示词里的 runtime | 三份手册都教“`runtime list` 后自己挑” | chat 两行、work 工作 runtime + 私有一行、plan 首个输入带 `<runtimes>` 全表（`runtime_view = all`）；`analyze_intent` / `plan_simple` / `goal` / `work_simple` / `ui_core` / `aigc_*` / `opendan_agent_dev` 于 11-04 重写 | S09 的 `skill_select_rule.inc` 要从重写后的文本抽：三处“匹配手册与 runtime”的段落（`analyze_intent.toml` 的 runtime 段、`goal.toml` 的 create-worksession 说明、`plan_simple.toml` 的 working runtime 段）措辞已各不相同，正是要收口的漂移 |
| 安装与持久性 | 未提 | `system` 安装在容器上必须落持久卷，`install.toml` 声明 `writes`，否则记 `volatile`、重启后重核验；ExtTools 清单里的命令拒绝由 skill 自装（R-U9，裁决 11）；ExtTools 全平台包与安装器并入安装包改造（裁决 22，R16）；`host` 要满足 Agent runtime 的 baseline，`agent-session` 留在 `STANDARD_TOOLS` | 待裁决 13 关闭；S19 加 `writes / volatile` |

## 2. 目标模型

### 2.1 四类 skill 与物理格式

| 类别 | 消费者 | 内容性质 | 物理落点（建议） | 注入方式 |
| --- | --- | --- | --- | --- |
| ui | UI / group Session | 对话里可用的命令，短 | `cmd_skills/<name>/SKILL.md`，front matter `scope: ui` | Session 创建时冻结进 `## Commands`；集合由 Agent 级设置决定 |
| work | Work Session（work_simple / explore） | 一类事的命令手册或范式 + 可选 `references/`、`tools/<name>/bin` | `cmd_skills/<name>/SKILL.md`，`scope: work`（缺省） | 创建者 `--cmd-skill` / Todo `--skill`，冻结 SKILL.md 正文；`references/` 按需 `read` |
| plan | plan_simple | 如何拆解、排序、验收一类目标的方法论；不写具体命令 | 新目录 `plan_skills/<name>/SKILL.md`，新 `SkillKind::Plan` | 第一轮“分析阶段”按需 `read`，不进系统前缀（§2.3） |
| goal | Goal Session | 判断距离、推进方式的方法论 | `goal_skills/<name>/SKILL.md`（目录已有，布局随 S01 迁） | 暂不做；`GoalSkill.ref` 指向它仍是 Goal TODO |

- ui 与 work 共用 `cmd_skills/` 与 tools PATH 机制，只加一个 `scope` 字段（`ui` / `work` / `ui,work`，缺省 `work`）；`scope` 本身不动安装、冻结、PATH 代码。`ui_core` 标 `scope: ui`。
- **布局（裁决 9，随 S01）**：`cmd_skills/<name>/SKILL.md` + 可选 `references/`（来由见 §7.2 第 1 点）。冻结只进 SKILL.md 正文；正文用相对路径指 `references/x.md`，Session 用 `read $OPENDAN_AGENT_ROOT/cmd_skills/<name>/references/x.md` 按需读；ui 大小 lint 只算 SKILL.md 正文。`tools/<name>/bin` 的位置与 PATH 机制不动。平铺的 `cmd_skills/<name>.md` 不再认（§6.1 推定 1）：内置手册、`test/skills_pikg_sample/`、手册之间互相引用的路径（如 `read $OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release.md`）同一变更迁完。`plan_skills/`、`goal_skills/` 同形：`<name>/SKILL.md` + 可选 `references/`，安装与 lint 一条规则（裁决 17）。
- **对外名（裁决 8）**：`SkillKind::Cmd` 的对外字符串由 `cmd` 改为 `work`：`skill list` 的 kind 列、`--kind work|plan|goal`、kRPC 的 `kind` 字段、`skills_state.json` 的 key 前缀（`work:<name>`）。目录名 `cmd_skills/` 保留；`--cmd-skill` / `todo add --skill` 参数与 `default_cmd_skills` 配置键不在这次改名范围。`scope: ui` 的手册 kind 也是 `work`，靠 `scope` 区分；文档与提示词统一称 work skill / ui skill。
- plan 单独一个目录：它不是命令手册，不带 tools，不能被 `--cmd-skill` 点名（冻结时拒绝）。lint 规则：plan skill 正文不得出现 `agent-session create-worksession`、`todo add --skill <具体名>` 之类对 work skill 的硬依赖，允许写“这类步骤交给带 X 类手册的 Todo”。
- `skill list` 增加 `scope` / `kind=plan` 列；`--kind work|plan|goal`，`--scope ui|work`。

### 2.2 Agent 级 skill 状态文件

新增 `.meta/skills_state.json`（schema `opendan.skills_state/1`），与 `.meta/skills.json`（安装记录）分开，模板同步不碰：

```json
{
  "schema": "opendan.skills_state/1",
  "disabled": {"work:aigc_video": {"at_ms": 0, "by": "did:bns:owner", "reason": "no provider"}},
  "ui_cmd_skills": ["ui_core", "share"],
  "usage": {"work:share": {"frozen": 12, "last_ms": 0, "recent_sessions": ["…"]}, "none": {"frozen": 3, "last_ms": 0, "recent_sessions": ["…"]}},
  "install": {
    "work:aigc_video": {
      "status": "enabled",
      "risk": {"level": "medium", "accepted_by": "did:bns:owner", "accepted_at_ms": 0, "plan_digest": "sha256:…"},
      "runtimes": {
        "private": {"status": "ok", "at_ms": 0, "commands": ["ffmpeg", "ffprobe"], "op_id": "op-…", "volatile": false},
        "host": {"status": "failed", "at_ms": 0, "error": "grant.install is none on host", "op_id": "op-…"}
      },
      "pinned_runtime": "private"
    }
  }
}
```

- `disabled`：任何来源（含 package）都可禁用；禁用后 `skill list` 缺省不显示（`--all` 显示并标 `disabled`），选择提示词看不到它，显式点名冻结时拒绝并说明“disabled by owner”，继承链里被过滤，tools 不进 PATH。已运行的 Session 不受影响。
- `ui_cmd_skills`：覆盖 `agent.toml default_cmd_skills`；为空键时用模板缺省。只能由 owner 身份通过 kRPC `skills.ui_set` 改（§3.1）。
- `usage`：§3.4 的使用记录，全部由宿主在冻结时记，没有模型填报的字段（裁决 6）。
- `install`：§3.6.1 的安装状态机。`status` 取 `installing | enabled | failed | disabled`（原 `pending_confirm` 随裁决 4 取消：等确认的是 `skills_drafts/` 里的草稿，不进状态机）；带命令的 cmd skill 只有在至少一个 runtime 的 `runtimes[id].status == ok` 后才会是 `enabled`。`runtimes` 永远按实际 runtime_id 记录，key 用规范 id（`private`，不是别名 `container`）；`pinned_runtime` 只是创建 Session 时的优先落点，只对带自装命令的 skill 有意义，缺省 `private`（2026-11-04 修正：默认 runtime 可被 owner 改，不能用 null 表示“默认”，见 [runtime 工作模型与生命周期 TODO §3](opendan-agent-runtime-lifecycle-todo.md)）。`op_id` 指向 `state/runtime_ops/<op_id>.json`（安装脚本与 check 的输出，runtime R2），不再内嵌 `log`；`volatile` 见 §3.6.1 C。冻结与选择提示词都读这里，不读手册正文。`runtimes[id].commands` 只记安装流程自己核验过的自装命令名（`install.toml [check]`）；front matter `commands` 在各 runtime 上的可用性不存这里，由 `skills.status` 现算（§3.6.2，§6.1 推定 4）。

## 3. 脑图逐项对照与设计

### 3.1 ui_skills：默认加载、谨慎扩展、只能命令行强制安装

- 保持“UI Session 创建时冻结、运行中不改”的规则：安装 / 启用 / 禁用都只影响之后创建的 UI Session。UI Session `idle_unload_secs = 900`，重建很频繁，“下次生效”可接受；不做 `skill use` 式的运行中重冻结（[cmd_skill 机制 TODO](opendan-cmd-skill-todo.md) 待设计项 2 据此关闭）。
- 新 CLI（owner 身份，走 opendan kRPC，`access: write`）：
  `buckyos skill ui list|add <name>|remove <name> --agent <id>`（kRPC `skills.ui_get` / `skills.ui_set`）。`add` 只接受 `scope` 含 `ui` 且未禁用的 cmd_skill。
- Agent 自己没有这个入口：`agent-session skill` 不提供 `ui` 子命令，kRPC 只授权 owner App；`ui_core` 手册写明“用户要把某手册加进对话时，告诉用户命令，不代为执行”。这就是脑图的“只能通过命令行强制安装”。
- 谨慎扩展的硬约束：`scope: ui` 的手册正文上限（建议 6 KB，`skill lint` 警告、安装不拒绝；11-04 重写后的 `ui_core.md` 已是 6.7 KB，多出来的是 M7 路径规则与 `agent-session run` 的轻量用法，上限以它为基准定在 8 KB 或把 runtime 一段挪进 `<runtime>` 背景块，S06 时定）；`skill ui add` 输出当前 UI 集合的总字节数；`skill list` 显示每份手册大小。
- 文档：`ui_core` 继续只列对话里真会用的命令；需要在对话里直接用的新能力（例如 `share`）走 `scope: ui,work`，由 owner 决定是否 `ui add`。

### 3.2 work_skills：谁创建 work session，谁选择

现状已满足“创建者选”：`analyze_intent` → `create-worksession --cmd-skill`；Goal 激活 → 同上；plan 的 Todo → `todo add --skill`。要补的是三点：

1. **plan 不再把自己的手册传给 Todo**：`api.rs` 的继承规则改为“父 Session 行为是 work_simple / explore 时继承；父是 plan_simple 时不继承”。plan session 创建时传入的 `--cmd-skill` 不再冻结进它的系统提示词，而是记为 `prompt.work_skill_candidates`，在它第一轮输入里以 `<skill_candidates>` 列出（名字 + description），供分析阶段参考。Todo 不带 `--skill` 就没有手册——plan 必须为每个 Todo 明确选择（§3.3，裁决 3）。
2. **三处选择提示词统一**：`analyze_intent.toml`、`goal.toml`、`plan_simple.toml` 里关于“看 `skill list`、匹配 runtime baseline/platform、≤ 8、是捷径不是规定”的段落抽成 `behaviors/skill_select_rule.inc`（`__INCLUDE`），避免三份各自漂移。
3. **使用记录**（§3.4）：宿主在冻结时计数；原拟的 `report.skills_feedback` 按裁决 6 不做。

### 3.3 plan skills：plan_simple 的分析阶段

- `plan_simple.toml` 第一轮改为两步：**分析** → **建表**。分析步要做：读目标与首个输入里的 `<runtime>` / `<runtimes>`（11-04 起 plan 已拿到全表，runtime R-W7；plan 恒私有，`agent-session` 与 `read` 都在）；`agent-session skill list --kind plan`，挑最多 2 份 plan skill 并 `read $OPENDAN_AGENT_ROOT/plan_skills/<name>/SKILL.md`；`agent-session skill list --kind work --scope work`（加上 `<skill_candidates>`）；输出一段结构化的“计划简报”——目标分解方式、验收方式、每个 Todo 预计用的 work skill。简报用 `report`（非 end）的 `summary` 记下，Session 页面能看到。
- plan skill 按需 `read`（裁决 2），内容进对话历史而不是系统前缀：plan session 寿命长、tick 稀少，重冻结一次前缀的收益不大，按需读最简单，也符合“务虚”——它只在建表时用一次。不做 `agent-session skill use`。
- plan skill 只给 plan_simple 读，`analyze_intent` 不读（裁决 7）。plan_simple 已开 Input queue：用户消息作为 input 进来，也有 `ask_user`（`plan_simple.toml`「Asking the user」）。访谈式 plan skill（grill-me、brainstorming 一类，§7.2 第 3 点）要问用户的部分由它在分析步自己问，问完再建表。
- 冻结规则：`--class plan_simple` 的 Session 不冻结任何 cmd_skill（`freeze` 时 `cmd_skills` 置空、候选另存）；`--behavior work_simple` 的照旧。
- 首批 plan skill（随 Jarvis 包）：`plan_software_change`（改代码类：探查 → 改动 → 验证 Todo 的拆法与验收标准写法）、`plan_content_production`（AIGC / 文档类：素材 → 生成 → 审核）、`plan_research`（只读调研类：并行 explore 的切分与汇总）。每份 ≤ 3 KB，不含命令。
- 关联修改：`analyze_intent` 创建 plan session 时，`--cmd-skill` 语义改为“候选”，结果 schema 的 `work.cmd_skills` 改名 `work.skill_candidates`（plan）/ 保持 `cmd_skills`（simple）。

### 3.4 使用记录（self-improve 的输入）

裁决 6（2026-11-09）：**不加 `report.skills_feedback`**。结晶（§3.8）本来就要读整个 Work Session，让执行者在 report 里自评是重复信息。原拟的 `skills_considered / no_skill_reason` 同理去掉（§6.1 推定 2）。留下的记录都由宿主确定性地记，不经模型：

- 冻结时：对每份冻结的手册累计 `usage[work:<name>].frozen`、`last_ms`，并把 session_id 记进 `recent_sessions`（留最近 20 个）。结晶靠它找到“用过这份手册的 Work”，再读这些 Session 的 worklog / report 全文。
- 创建了不带任何手册的 Work（work_simple / explore）：同样累计到 `usage.none`。这是 `doc/opendan/todo.md` 两种信号里的第二种（没有手册可选的工作）；为什么没选，由结晶读创建者一侧的 Session 判断。
- `agent-session skill list --stats` / `buckyos skill list --stats` 显示 frozen / last。

### 3.5 goal skills：暂不做

保持 `goal_skills/` 目录与 `GoalSkill` 引用现状；在 [Goal Session TODO](done/opendan-goal-session-todo.md) 记一条：`GoalSkill.ref` 指向 `goal_skills/<name>/SKILL.md` 时由激活渲染读入 `<current_skills>` 正文。本计划不改 Goal 的选择逻辑，只把 §3.2 第 2 条的 include 用上。

### 3.6 基础结构：如何安装

**谁能装（裁决 4，2026-11-09）**：乱装 skill 是核心风险之一，**只有 owner 能装，Agent 自己装的入口暂不提供**。

- owner 的入口：opendan WebUI 的 Skills 页（§3.7）与 `buckyos skill install`。
- 删除 `agent-session skill install`（`xagent_cli.rs`，连同“runtime 不是 Agent 自己的”那条报错）；`ui_core` 里“用户要求时安装或删除”、`opendan_agent_dev` 里的安装示例改成“告诉用户到 Skills 页或用 `buckyos skill install`”。`agent-session skill remove` 一并删除（裁决 16：卸载与禁用同属 owner 定的能力边界），Agent 侧只留 `skill list`。
- kRPC `skills.install / skills.remove` 现在只要调用方是 Agent 的 App 身份就放行（`service.rs` `authorize_agent_method`：read_write 的 Agent 不限方法，只读的 Agent 也能装自己，见用例 `read_only_agent_writes_only_its_own_state`），改成与 `skills.ui_set / skills.set_enabled` 同一类：仅 owner。
- Agent 一侧只能**产出草稿**：`skill_import`（下）与结晶（§3.8）把结果写到 `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/` 并通知 owner；owner 在 Skills 页或 `buckyos skill install --draft <slug> --agent <id>` 走 §3.6.1 的确认后安装。
- 去掉入口只挡住正常路径：Agent 直接往 `cmd_skills/` 写文件就得到一个 `origin: local` 的 skill，冻结不看来源。owner 2026-11-09：这个先不管，记为已知缺口（§6.1 问题 15）。

**导入（任意文本 → LLM → 本机专用 skill 的草稿；安装由 owner 确认）**

- 入口：`buckyos skill import (<file> | <url> | -) --agent <id> [--name <slug>] [--scope work|ui|plan]` → kRPC `skills.import`（owner），以及 Skills 页的“导入”；对话里用户贴文本说“装成 skill”时，`analyze_intent` 可创建同样的 work（`--behavior skill_import`），产物同样只是草稿。
- 实现为 Jarvis 内置行为 `behaviors/skill_import.toml`（work_simple 变体，cmd_skill `skill_authoring`）：读文本 → 判断拆成几份、各自 kind/scope → 写 front matter（`name/description/requires/commands/scope/baseline/platforms`）与正文（正文用到的命令写进 `commands`，生态 skill 的 `allowed-tools: Bash(<cmd>:*)` 直接映射，§3.6.2） → 在私有 runtime 上逐条核验正文提到的命令是否存在（`command -v`、`--help`；`skill_import` work 不写 `--runtime`，它要 `agent-session`），不存在的写进 `requires` 并降为 `baseline: extended`；要为 owner 的机器定制时 `--runtime <id>` 只改核验目标，经 `agent-session run --runtime <id> -- command -v <cmd>`（已有）核验，owner 之后安装时落点仍缺省私有 → 把结果放到 `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/`（`cmd_skills/<name>/SKILL.md`、`plan_skills/<name>/SKILL.md`，附 `draft.json`：`imported_from: {digest, runtime_id, platform}`、拆分说明、核验结果、风险说明）→ `report` 并通知 owner“草稿在 Skills 页，确认后安装”。**不调用安装**（裁决 4）；owner 确认安装后包名 `imported-<slug>`，安装记录 `origin: imported`，`imported_from` 取自草稿。
- “本机专用”的含义：`platforms` 固定为核验所在 runtime 的 `caps.os`，`baseline` 按核验结果，`imported_from.runtime_id` 写实际 id；导入的 skill 不进发行路径（`pikg init` 不认 `origin: imported` 的目录；要发行先人工整理）。
- 原 `--dry-run` 取消：import 的产物永远是草稿，owner 在 Skills 页或 `buckyos skill install --draft <slug> --agent <id>` 看过再装（待裁决 4 原问的“缺省直接安装还是缺省 dry-run”随之消失）。

**强制安装（ui_skills）**：§3.1 的 `buckyos skill ui add`。

**从发行渠道安装**：`buckyos skill install <source>` 的 `<source>` 对齐 `app install` 的来源解析（本地 `.pikg` / HTTP(S) URL / Catalog DID，`--from` 消歧）；非本地来源由 CLI 下载到本机后走现有离线校验与 `skills.install`。另加两种本地来源：目录（skill 作者试用，取代原来的 `agent-session skill install <dir>`；现在 `buckyos skill install` 只收本地 `.pikg`）与 `--draft <slug>`（AgentRoot 里的草稿）。

### 3.6.1 安装流程：风险分析 → 人工确认 → cmd 落到 runtime → 才启用

所有来源（`.pikg` / 目录 / URL / DID / 草稿 / WebUI 上传）共用一条流程，发起者都是 owner（§3.6，裁决 4），四个阶段，前两个阶段没有副作用：

**A. 解析与风险分析（无副作用）**

- 输入解析成 `install_plan`：skills 列表（kind / scope / name / 覆盖谁）、包自带的命令（`tools/<name>/bin`）、手册要求的外部命令（front matter `commands: ffmpeg, ffprobe`，只写命令名；`requires` 继续给人读；`install.toml [check] commands` 缺省取它。2026-11-05 owner 定：skill 只说明自己依赖的命令，系统凭扩展包目录反推该装哪个包，见 [ExtTools TODO §3.9.3](opendan-exttools-cross-platform-todo.md)）、`baseline` / `platforms`、安装脚本（下）。
- 风险分析分两层。确定性 lint（Rust，`skills.rs`）：脚本是否可读（二进制无法审计记 high）、脚本和正文里的网络下载（`curl | sh`、包管理器 `apt/brew/npm -g/pip`）、提权与落点（`sudo`、包管理器；`install.toml` 的 `writes` 声明到 `<agent_root>` / 持久卷之外的，容器上记 `volatile`；脚本需要 `system` 而目标 runtime 的 `grant.install` 不是 `system` 的直接列为阻断）、需要的 secret / 环境变量、`scope: ui`（进 UI 系统提示）、正文用了但没声明的命令（`undeclared_commands`，§3.6.2 第 3 点，只警告）、同名覆盖已装 skill、来源（签名的 package vs imported 文本）。LLM 层只在 import 路径跑（`skill_import` 行为本来就要读全文；结果存进草稿的 `draft.json`，owner 安装草稿时并入 plan），输出“这份手册会让 Agent 做什么、哪些步骤不可逆”的一段话。两层合成 `risk: {level: low|medium|high, findings: [{code, where, note}]}`，`plan_digest` 盖住整个 plan。
- 协议：kRPC `skills.install_plan {package | source | draft}` 返回 `install_plan`，不落盘；`buckyos skill install` 与 Skills 页都先调它。现状：`skills.install_plan {package, runtime_id}` 已有（ExtTools X5），只答命令依赖；风险 lint 与 `draft` 来源未做。

**B. 人工确认**

- CLI：打印 plan 与风险清单后 `[y/N]`；`--yes` 覆盖全部风险等级（裁决 11，不设 `--accept-risk`）；非交互且没给 `--yes` 就以 `CONFIRMATION_REQUIRED` 退出（与 `skill remove` 一致）。WebUI Skills 页弹同一份清单。对话入口（`analyze_intent` 建的 `skill_import` work）不确认也不安装（裁决 4，§6.1 推定 5）：产出草稿后通知 owner 去 Skills 页；未安装的草稿在 Skills 页的草稿区与 `buckyos skill list --all` 里可见。
- 确认页同时定 cmd 的落点 runtime：缺省 `private`，可改为别的 `grant.install ≠ none` 的候选（C）；落点是 owner 共用的机器（`host`）且脚本要 `system` 时，文案点明“会装到你的电脑上”（runtime TODO R-M8）。确认提交 `skills.install {plan_digest, runtime_id?}`；digest 不符（包换了）拒绝。
- 没有命令的 skill（范式类 work skill、plan skill、ui skill）确认后直接 `enabled`。

**C. cmd 安装：装到哪个 runtime**（2026-11-05 按 runtime TODO §0 / §3 / R-U1 / R-U3 / R-U9 改写）

- 记录：Agent 的 runtimes 是 `state/runtimes/<id>.json`（v4）：`private`（别名 `container` / `local`，永远可用）、`host`、`node-*`、`exec-*`、ssh 登记；每条有 `caps.os`、`caps.baseline`、`caps.tools_check`、`grant.install`（`none | agent_bin | system`）、`availability`、`enabled`、`lifecycle`、`grant.expires_at_ms`。owner 的 `default_runtime_id` 只是“用户说‘我电脑’指哪台”，**不再决定安装落点**。
- 候选 = `usable()`（可用、启用、未到期、未 retire）且 `grant.install ≠ none` 且 `caps.os ∈ skill.platforms`（空则全部）且 `caps.baseline.meets(skill.baseline)`。只带 `tools/<name>/bin` 的包要 `agent_bin` 就够；带 `install.<os>` 脚本的包要 `system`。
- 选择：**缺省 `private`**（runtime TODO 裁决 15：skill 自装命令缺省装私有；私有的 grant 是 `install = system`，落持久卷）。私有不在候选（只有 `platforms` 把它的 os 排除时会发生，例如只有 Windows 版的工具）才取候选第一个，B 里用户可改；两种情况都把实际 runtime_id 写进 `install.runtimes`，`pinned_runtime` 记当前优先落点。没有候选 → cmd 部分拒绝，skill 以 `failed` 入库并给原因（“需要 windows，没有 `grant.install` 允许安装的 windows runtime；请 owner 在 runtime 目录里开一台”，与 R-C5 的拒绝文案同形），手册文件保留以便 owner 登记新 runtime 后重试。
- ExtTools 清单（runtime R-S3 的 `EXTENDED_TOOLS`）里已有的命令**拒绝由 skill 自己装**：报“由 ExtTools 提供，手册改声明 `baseline: extended`”（R-U9，裁决 11）。官方 skill 只用 `standard / extended` 清单里的命令（R-S2），自装命令是第三方 skill 的例外路径；`EXTENDED_TOOLS` 落地（R16）前只拒 `STANDARD_TOOLS` 里的。
- 执行：包内 `tools/<name>/install.toml`（可选）描述核验、安装、落点与卸载：

  ```toml
  [check]
  commands = ["ffmpeg", "ffprobe"]      # 核验清单；没有 install.toml 的 skill 不核验命令
  [install.linux]
  script = "setup/linux.sh"             # 相对 tools/<name>/；需要目标 runtime grant.install = system
  writes = ["<instance_volume>/tools/bin"]   # 脚本写到哪；容器上不在 <instance_volume> / <agent_root> 下的记 volatile
  [install.windows]
  script = "setup/windows.ps1"
  [uninstall.linux]
  script = "setup/uninstall.sh"         # 可选；没有就只删 bin 副本
  ```

  没有 `install.*` 只有 `check` 时只核验不安装。流程全部走 Agent 级原语（runtime R2，已有）：`runtime_put(runtime_id, tools/<name>/, <目标 tools 目录>)` → `runtime_exec(runtime_id, script, cwd, timeout)` → `runtimes.check {runtime_id, commands}`（写 `caps.tools_check`，复用 `baseline::missing_probe`）；每步的输出在 `state/runtime_ops/<op_id>.json`，`install.runtimes[id].op_id` 指过去。`check` 全部存在才 `ok`；`writes` 有落在持久位置之外的记 `volatile: true`（容器重启后由 runtime R6 / R-M2 重核验）。
- 包自带的 `tools/<name>/bin`：私有 runtime 就是现在的做法（复制到 `<agent_root>/tools/<name>/bin`；Session 的 bin overlay 与 `runtime_exec` 都已前置 `<agent_root>/tools`）。节点 runtime：放到 `<账户 home>/.opendan/<agent_id>/tools/<name>/bin`，PATH 前置由 libopendan 在每次 exec 的 `env_overrides.PATH` 里做（runtime R-U4 / R3，**P2**）；节点上不装 helper（R-W10），原文“补 §6.1 欠的工作”作废。R3 之前装到节点的 skill 只记 `install.runtimes[node].status`，单机 work 要用它得在 Todo 正文写绝对路径。
- 装好后 `runtimes.note_usage {runtime_id, event: SkillCommand {name}}`（kRPC 已有，`skills.rs` 未调）写 `usage.skill_commands`，这就是 plan `<runtimes>` 全表“skill 命令”一列的来源；卸载时去掉。
- 一个 skill 可以装到多个 runtime：`buckyos skill install-cmd <name> --runtime <id> --agent` 追加；`skill check <name> [--runtime <id>]` 重跑核验（= `runtimes.check --commands`）。该条标 `stale` 的四种情形（runtime R-M2 / R-M5，都不自动删）：runtime retire（`unregister` 的语义即 retire）、`check` 失败、节点账户 home 变了（视为新机器）、容器重启后 `volatile` 的记录重核验失败。

**D. 启用与冻结**

- `enabled` 只在 C 的任一 runtime `ok` 后置位；之前 `skill list` 缺省不显示，选择提示词看不到，显式 `--cmd-skill` 点名拒绝并说明状态。
- 冻结时 `SkillMeta::unmet_on` 之外再查 `install.runtimes[runtime.frozen.runtime_id]`（规范 id）：不是 `ok` 的不注入，`## Commands` 末尾写“commands of `<name>` are installed on `private` only; this work runs on `host`”。典型情形是单机 work（`--runtime host`）点名了只装在私有的 skill；反过来（装在 host、work 在私有）经 `agent-session run --runtime host -- <cmd>` 也能用，手册末尾同样说明。
- 选 runtime 的第三级（✅ 待裁决 12 随 runtime TODO R-U1 / 裁决 15 关闭）：`create-worksession` / Todo 没写 `--runtime`、也没有继承时，点名的带自装命令的 skill 都装在同一 runtime → 自动取它（`selected_by: skill`，枚举已有）；装在多台且都可用 → 取 `pinned_runtime`；多个 skill 的落点互相冲突 → 拒绝并列出；都没有 → 私有。因为缺省装私有，这一级很少触发。实现是 runtime TODO **R4**（`api.rs freeze_runtime`），S20 依赖它。`analyze_intent` 的“二选一”手册不用改：它不写 `--runtime` 时由宿主按这一级决定。
- 卸载反向走：先按 `install.runtimes` 在每个 runtime 经 `runtime_exec` 跑 `install.toml` 的 `[uninstall.<os>]`（可选，没有就只删 bin 副本），再删手册与记录，并从 `usage.skill_commands` 去掉。

### 3.6.2 命令依赖声明与可用性判定（2026-11-09）

问题：skill 要不要列出自己引用的命令，让 UI 能检查它在目标 runtime 上能否工作？**要，而且声明与检查已有**（待裁决 14 + ExtTools X5，提交 `7d0b359c7`）；要补的是把它做成 UI 能直接用的完整判定。

已有：

- 声明：front matter `commands: ffmpeg, ffprobe`，只写命令名（`protocol/behavior.rs` `skill_commands_of`），8 份官方手册已补。标准层命令（`agent-session`、`buckyos`、`gen_image` 等）不写，由 `baseline` 覆盖；包自带的 `tools/<name>/bin` 不写，由安装记录覆盖。
- 检查：kRPC `skills.status {runtime_id}`（`opendan/src/home.rs`）当场在目标 runtime 上探测，逐 skill 返回 `ready | ready_system | partial`、逐命令 `present / provided_by`、`recommended_packages`、`no_package`；CLI `buckyos skill status --agent <id> --runtime <id>`。结果不落盘：事实在 runtime 记录的 `caps.tools_check.resolved`，skill 状态是它的派生。
- 安装前：`skills.install_plan {package, runtime_id}` 给同一判定。冻结时：缺的命令写在 `## Commands` 末尾，不拦（ExtTools TODO §3.9.4）。

要补：

1. **`skills.status` 给完整判定**。现在只答命令一项，没写 `commands` 的 skill 被过滤掉（`share`、`ui_core`、`buckyos_admin`、`xllm` 不在结果里），真正会让冻结被拒的 `baseline / platforms` 不满足（`SkillMeta::unmet_on`）也不在。改为每个 work / ui skill 一条：`{name, scope, unmet?: "<原因>", commands: <现有 status>, install?: <install.runtimes[id]，S20 之后>}`，UI 只调这一个接口（S22）。
2. **UI 的三档与措辞**。“命令齐”不等于“能正常工作”：命令在但能力不够（扩展包二期遇到过 `ffmpeg` 没有 zscale），或缺的是 AICC provider、read_write 权限（只写在 `requires` 里）；反过来命令缺但主流程能跑（`aigc_audio` 在没装 ext-core 的 standard host 上永远 `partial`，TTS 不需要 ffmpeg）。不加字段：不做版本约束，不分必选 / 可选。Skills 页带 runtime 选择器（缺省 `private`），按所选 runtime 显示三档：**不可用**（`unmet`，冻结会拒）、**部分步骤缺命令**（`partial`，列缺什么与推荐包）、**命令齐全**（`ready`；`ready_system` 注明来自用户自备）；`requires` 原文放在旁边（S14）。
3. **未声明命令的 lint**。现在没有任何检查保证 `commands` 与正文一致，第三方 skill 不写这一行时看起来和“无依赖”一样。`pikg doctor`（S15）与 `skills.install_plan`（S18）加一条警告 `undeclared_commands`：SKILL.md 与 `references/` 里 shell 代码块命令位置的词，去掉 shell 内建、标准层命令（`STANDARD_TOOLS`）和包自带的 `tools/<name>/bin`，剩下不在 `commands` 里的列出来；声明了但正文没出现的也警告。只警告不拒绝；官方手册用同一规则核一遍（S06）。
4. **生态 skill 的映射**：`allowed-tools: Bash(<cmd>:*)` → `commands`，其余由 `skill_import` 从正文抽取后核验（§3.6）。§7.2 第 2 点原写“忽略 `allowed-tools`”，已据此改（S12）。
5. plan skill 不写具体命令（§2.1），带 `commands` 的 lint 报错（S01）。

### 3.7 基础结构：如何卸载 / 禁用

- 协议：kRPC `skills.set_enabled {kind, name, enabled, reason?}`、`skills.ui_get/ui_set`、`skills.list` 返回 `enabled` 与 `scope`；CLI `buckyos skill disable|enable <name> --agent`；`agent-session skill` 不提供 install / remove / disable（能力边界由 owner 定，裁决 4 / 16）。
- opendan WebUI 新页面 `pages/Skills.tsx`（路由与 Goals 同级），Home 卡片加“Skills：N 可用 / M 禁用”：列表（kind、scope、来源、大小、usage；带 runtime 选择器，按所选 runtime 显示可用性三档与 `requires` 原文，§3.6.2）、启用 / 禁用、卸载（仅 installed/imported，二次确认）、UI 集合编辑（§3.1 同一 kRPC）、从本地 `.pikg` 安装（浏览器里复用 websdk 的 `pikg_archive.ts` 解析，走 §3.6.1 的 plan → 确认 → `skills.install`，与 CLI 同一份校验代码）、草稿区（`skills_drafts/`：import 与结晶的产物，查看 → 确认安装 / 丢弃）。裁决 4 之后，skill 只从这一页和 `buckyos skill install` 进来。
- Desktop 侧不做；WebUI 就是脑图里的 agent home page。

### 3.8 基础结构：self improve——按 worklog 与 memory 改进 skills

按架构 §9.3 做成内置 maintain Goal，分两步，不与价值观层绑定：

- **A. 信号**：§3.4 的 `usage`（哪些 Work 冻结了哪份手册、哪些 Work 没有手册）；评价不由执行者填报，由结晶读 Session 全文得出（裁决 6）；Memory 整理 Goal 产出的“能力缺口 / 操作经验”认知保持不变（《Agent Memory 认知管理需求》§… 已把经验认知定义为结晶输入）。
- **B. `skill_crystallize` Goal**（`[session.goal.crystallize]`，缺省关闭，owner 在设置里打开；行为 `behaviors/skill_crystallize.toml`，预算与 memory_maintenance 同级）。每次激活：按 `usage.*.recent_sessions` 取最近 N 个 Work Session，读它们的 worklog / report 全文与相关认知 → 三类决定：
  - 修正：对 `origin: imported|local` 的手册直接改写（裁决 5；保留 `revisions/` 备份与修订记录）。自动改写只动正文与 `description`；`scope / commands / baseline / platforms` 的变化与新增 `tools/` 走草稿，由 owner 装（§6.1 推定 3）；对 `origin: package` 的手册只写草稿到 `skills_drafts/<name>.md` 并通知 owner（模板同步会保留本地改动，但自动改模板手册风险大，先人工）。
  - 提炼：`usage.none` 里同类 Work 反复出现 → 产出候选 `skills_drafts/<slug>/`，通知 owner 到 Skills 页确认安装（§3.6）。
  - 清理：60 天未冻结，或读过的 Session 里多次没用上、把执行带偏 → 建议禁用（通知，不自动禁用）。
- 边界：不改 ui 集合、不改 package 手册正文、不自动安装新 skill；所有自动改动在 `skills_state.json` 留审计记录。

### 3.9 skill 作者如何发行：走 PIKG 路径

路径已存在：`buckyos pikg init --kind agent-skills --source ./skills` → `pikg doctor/build/pack` → `app publish`（Repo candidate）→ Owner 签名 / BNS 发布（现有 App 流程）。要补：

- `pikg doctor` 对 `agent_skills` 子包做 skill lint（front matter 固定词、`scope`、目录布局、`plan_skills/` 允许、plan 正文禁依赖、大小警告、`undeclared_commands`（§3.6.2））；`skills.install` 复用同一套校验（Rust 侧 `skills.rs` `install` 已有 `skill_requirements`，加 scope/plan 规则）。
- 消费端 §3.6 的 `skill install <did|url>`。
- 文档：新增 `doc/sdk/skill-dev-quickstart.md`（写手册 → `buckyos skill install <dir> --agent <id>` 试用 → 打包 → 发布 → 安装），`opendan_agent_dev.md` 手册只保留指引。Desktop 商店展示 `agent_skills` 类别不在本计划内。

## 4. 实施任务清单

路径默认相对 `buckyos/`；`../buckyos-websdk/` 为同级仓库。

### P0 格式与状态（其余阶段的基础）

| # | 任务 | 落点 |
| --- | --- | --- |
| S01 | `SkillKind::Plan` 与 `plan_skills/` 目录；`SkillMeta.scope`；front matter `scope` 解析与校验（固定词）；**目录布局** `<name>/SKILL.md` + 可选 `references/`，`cmd_skills/`、`plan_skills/`、`goal_skills/` 三个目录同形，平铺形式不再认（裁决 9 / 17）；**对外 kind 字符串** `cmd` → `work`（裁决 8）；`install`/`place`/`read_pikg`/`read_package_dir` 认新目录与新布局；lint 规则（plan 禁依赖、plan 不带 `commands`、ui 大小警告只算 SKILL.md） | `src/frame/lib_opendan/src/protocol/behavior.rs`、`skills.rs`、`state/behaviors.rs`（读手册正文）、`opendan/src/rootfs.rs`（骨架目录）、`opendan/src/home.rs`（`skills_install_plan` 写死的 `cmd_skills/<name>.md` 匹配）、`../buckyos-websdk/cli/modules/skill.ts`（离线校验） |
| S02 | `.meta/skills_state.json`：`disabled` / `ui_cmd_skills` / `usage`（key 前缀 `work:`；`usage` 的写入在 S10）；读写与原子替换；`list` 合并 `enabled` | `skills.rs`、`state/behaviors.rs` |
| S03 | 冻结规则：禁用的拒绝 / 过滤；`scope: ui` 不能被 `--cmd-skill` 点名；plan kind 不能冻结；`ui_cmd_skills` 覆盖 `default_cmd_skills` | `state/behaviors.rs` `freeze_config`、`template.rs`、`api.rs` |
| S04 | kRPC `skills.set_enabled`、`skills.ui_get/ui_set`、`skills.list` 新字段；权限表 | `src/frame/opendan/src/service.rs` |
| S05 | CLI：`buckyos skill list --kind/--scope/--all/--stats`、`disable/enable`、`ui list/add/remove`；`--kind` 取 work / plan / goal；`agent-session skill list` 同步（Agent 侧只留 list，S23） | `../buckyos-websdk/cli/modules/skill.ts`、`src/frame/lib_opendan/src/xagent_cli.rs` |
| S06 | `ui_core` 标 `scope: ui`；现有手册迁到 `cmd_skills/<name>/SKILL.md` 并补 `scope`，手册之间互相引用的路径跟着改；用 §3.6.2 的 `undeclared_commands` 规则核一遍官方手册的 `commands`；`opendan_agent_dev` / 开发指南 §2.4 / AGENTS.md 联动表更新 | `src/apps/jarvis_agent/agent/cmd_skills/`、`doc/sdk/OpenDAN_Agent_Dev_Guide.md`、`AGENTS.md` |
| S23 | **只有 owner 能装**（裁决 4 / 16，§3.6）：删除 `agent-session skill install` 与 `skill remove`（子命令、帮助文本、“runtime 不是 Agent 自己的”报错）；kRPC `skills.install / skills.remove` 由“Agent 的 App 身份可调”改为仅 owner（`authorize_agent_method` 与用例 `read_only_agent_writes_only_its_own_state`）；`ui_core`、`opendan_agent_dev` 与开发指南 §2.4 里教 Agent 安装 / 删除的句子改成“告诉用户到 Skills 页或用 `buckyos skill install`”；`--test xagent_cli skills_are…` 的安装与删除步骤改走 owner 路径。不依赖格式改动，可最先做 | `src/frame/lib_opendan/src/xagent_cli.rs`、`src/frame/opendan/src/service.rs`、`src/apps/jarvis_agent/agent/cmd_skills/`（`ui_core`、`opendan_agent_dev`）、`doc/sdk/OpenDAN_Agent_Dev_Guide.md`、`src/frame/lib_opendan/tests/xagent_cli.rs` |

### P1 选择闭环

| # | 任务 | 落点 |
| --- | --- | --- |
| S07 | plan_simple 不冻结手册；`--cmd-skill` → `prompt.work_skill_candidates`；首轮输入渲染 `<skill_candidates>`；Todo 不继承 plan 的手册 | `api.rs`、`protocol/config.rs`、`runner/inputs.rs` |
| S08 | `plan_simple.toml` 分析阶段（读 `<runtimes>`（已有）与 plan skill、选 Todo skill、计划简报）；三份 plan skill 随 Jarvis 包 | `src/apps/jarvis_agent/agent/behaviors/plan_simple.toml`、`agent/plan_skills/` |
| S09 | `skill_select_rule.inc` 抽取并 include 进 analyze_intent / goal / plan_simple（以 11-04 重写后的文本为底，三处“匹配手册与 runtime”段落收成一份：私有 `extended` 满足全部官方手册、单机 work 只有那台机器的基线、点名不满足的手册被拒）；analyze_intent 结果 schema 的 `skill_candidates` | `agent/behaviors/*.toml` |
| S10 | 宿主在冻结时累计 `usage`（`frozen / last_ms / recent_sessions`，没有手册的 Work 记 `usage.none`）；`--stats` 输出。`report.skills_feedback`、`skills_considered / no_skill_reason` 不做（裁决 6） | `state/behaviors.rs`（冻结）、`skills.rs`、`xagent_cli.rs`、`skill.ts` |
| S11 | 测试：冻结规则单测（禁用 / scope / plan）、继承规则、usage 累计；DV：创建 plan → Todo 带 `--skill` → `skill list --stats` 的 frozen 变化 | `src/frame/lib_opendan/src/**/tests`、`test/` |

### P2 安装与管理体验

| # | 任务 | 落点 |
| --- | --- | --- |
| S12 | `skills.import` kRPC + `buckyos skill import`，产物是 `skills_drafts/<slug>/` 草稿，不安装（裁决 4）；`behaviors/skill_import.toml` + `cmd_skills/skill_authoring`；`draft.json`（来源、拆分、核验、LLM 风险段，owner 安装草稿时并入 S18 的 plan）；agentskills.io front matter 映射表（含 `allowed-tools` → `commands`，§7.2 第 2 点）；owner 安装草稿后记 `origin: imported` | `service.rs`、`skill.ts`、Jarvis 包、`skills.rs` |
| S13 | `buckyos skill install <source>` 来源解析对齐 `app install`（pikg / url / catalog），另加本地目录（作者试用）与 `--draft <slug>` | `../buckyos-websdk/cli/modules/skill.ts`（复用 `app.ts` 的 source 解析） |
| S14 | opendan WebUI `pages/Skills.tsx` + Home 卡片；runtime 选择器与可用性三档（调 S22 的 `skills.status`，§3.6.2）；安装确认页（S18 的 plan）；草稿区（查看 / 确认安装 / 丢弃）；浏览器内 `.pikg` 解析复用 websdk `pikg_archive.ts` | `src/frame/opendan/web/src/` |
| S15 | `pikg doctor` skill lint（含目录布局、`undeclared_commands`）；`doc/sdk/skill-dev-quickstart.md`；样例包迁目录布局并加 `plan_skills/` | `../buckyos-websdk/cli/modules/pikg.ts`、`doc/sdk/`、`test/skills_pikg_sample/` |
| S18（命令部分已做：`skills.install_plan`、`recommended_packages`，见 ExtTools TODO §4.2；风险 lint 未做） | `skills.install_plan` kRPC + 确定性风险 lint（含 `grant.install` 与 `writes` 的落点检查）；front matter `commands` 解析，缺的命令查扩展包目录给 `recommended_packages`（不 block，ExtTools TODO X5）；`undeclared_commands` 警告（§3.6.2）；`buckyos skill install` 改为 plan → 确认 → `skills.install {plan_digest, runtime_id}`（`runtime_id` 缺省 `private`）；`--yes` 覆盖全部风险等级（裁决 11）；`draft` 来源 | `skills.rs`、`service.rs`、`opendan/src/home.rs`、`skill.ts` |
| S19 | `tools/<name>/install.toml`（`check / install.<os> / writes / uninstall.<os>`）；经 `runtime_put / runtime_exec / runtimes.check` 在目标 runtime 装与核验（runtime R2 已有）；`volatile` 标记（= runtime R13）；ExtTools 清单命令拒装（R-U9；`EXTENDED_TOOLS` 已落地，依赖已解除，2026-11-06）；`note_usage SkillCommand` 记账；`install-cmd`（只在 `buckyos skill`，owner）/ `check` 子命令。节点 `tools/<name>/bin` + PATH 前置 = runtime R3（P2），**不在本计划里做** | `skills.rs`、`runtime/mod.rs`（`runtime_exec` / `runtime_put` / `check_commands` 已有）、`skill.ts`、`xagent_cli.rs` |
| S20 | `.meta/skills_state.json` 的 `install` 状态机；冻结按 `runtime.frozen.runtime_id` 查安装状态；`## Commands` 末尾的落点说明；`create-worksession` 的 skill 落点级 = runtime R4（`RuntimeSelection::Skill` 枚举已有，逻辑在 runtime TODO 排期） | `api.rs`（冻结）、`runner/assembler.rs`、`skills.rs` |
| S21 | runtime 事件 → `install.runtimes[id]` 标 `stale` / 恢复与容器重启后重核验（= runtime R6，R-M2 / R-M5）；同一个任务，在 runtime TODO 里排期，这里只记依赖 | `skills.rs`、`loader.rs` |
| S22 | `skills.status` 给完整判定（§3.6.2 第 1 点）：每个 work / ui skill 一条，带 `unmet`（`baseline / platforms`，`SkillMeta::unmet_on`）与现有的命令 status，S20 之后并入 `install.runtimes[id]`；`buckyos skill status` 输出跟着改 | `src/frame/opendan/src/home.rs`、`skills.rs`、`skill.ts` |

### P3 Self-improve

| # | 任务 | 落点 |
| --- | --- | --- |
| S16 | `skill_crystallize` 内置 Goal（缺省关闭）：行为、预算、`skills_drafts/` 与 `revisions/` 约定、通知文案 | `agent.toml [session.goal.crystallize]`、`behaviors/skill_crystallize.toml`、Loader 的内置 Goal 种子（参照 memory_maintenance） |
| S17 | 审计：自动改动写入 `skills_state.json`；WebUI Skills 页显示草稿与修订 | `skills.rs`、`pages/Skills.tsx` |

顺序：S23 不依赖格式改动，最先做；P0 → P1 可以合并为一个里程碑（格式一旦定下就同时改选择提示词）；P2 的 S13/S15/S22 独立，可并行；S18–S21 依赖 runtime TODO 的 R2（已有）与 R4 / R6 / R13 / R16（未做），其中 R16 的扩展包按 [扩展包 TODO](opendan-exttools-cross-platform-todo.md) 分期，S19 的 ExtTools 拒装直接用 `baseline::EXTENDED_TOOLS`；P3 依赖 S10 的信号积累，最后做。

## 5. 验收

- 装一个 `scope: work` 的 skill 包后：新 UI Session 的系统提示词不变；`skill ui add` 后新 UI Session 多出该手册，旧 Session 不变。
- `skill disable aigc_video` 后：`analyze_intent` 看不到它；`create-worksession --cmd-skill aigc_video` 被拒且原因是 disabled；已运行 Session 不受影响。
- 创建 plan session：它的系统提示词没有 `## Commands`；第一轮能看到候选与 plan skill 列表；每个 Todo 的 work session 只带 plan 明确给的 `--skill`。
- 建一个带手册的 Work 后 `skill list --stats` 的 frozen / last 变化；report schema 没有 skill 字段。
- `buckyos skill import ./howto.md --agent x` 只在 `skills_drafts/` 产出草稿；owner 在 Skills 页或 `buckyos skill install --draft <slug>` 确认后才出现在 `skill list`，`origin: imported`，`platforms` 为私有 runtime 的平台，`commands` 列出正文用到的命令且都在私有 runtime 上核验过。
- `agent-session skill install` / `skill remove` 不存在；用 Agent 的 App 身份调 kRPC `skills.install` / `skills.remove` 被拒；`buckyos skill install x.pikg --agent x --yes` 对 high 风险的包也直接装。
- `buckyos skill status --agent x --runtime host` 对每个 skill 都有一条：`share` 这类没写 `commands` 的也在；`baseline: extended` 的手册在 standard host 上是 `unmet`；Skills 页切 runtime 后三档跟着变。
- 手册代码块里用了 `jq` 而 `commands` 没写：`pikg doctor` 与安装确认页都列出 `undeclared_commands: jq`，安装不被拒。
- 装一个带 `tools/<name>/bin` 的包：不写 `--runtime` 时落 `private`，`install.runtimes.private.status == ok`，`runtime list` 的该条 `usage.skill_commands` 含它；不带 `--cmd-skill` 以外任何 runtime 参数建的 work 落私有且命令可用；`--runtime host` 的单机 work 点名它时 `## Commands` 末尾说明只装在 `private`。
- 带 `install.linux` 脚本的包装到 `grant.install = agent_bin` 的 runtime 被拒，原因写明需要 `system`；装到私有时 `writes` 不在持久卷下的记 `volatile`。
- `skill install did:bns:xxx.owner` 与本地 `.pikg` 结果一致。
- WebUI Skills 页能列出、禁用、卸载、改 UI 集合、确认安装草稿。

## 6. 待裁决

1. ✅ 已裁决（间接，2026-11-05）：用 `scope` 字段。runtime TODO §3 已按 `ui_core`（`scope: ui`，UI session 恒私有）行文并经 owner review；拆目录要动 PIKG 子包布局与样例，没有人提出。反对请在 S01 前说。
2. ✅ 已裁决（2026-11-09，owner）：plan skill 按需 `read`，不进系统前缀（§3.3）。原问：按需 `read`（建议）还是进系统前缀一次性重冻结？
> 按你的建议
3. ✅ 已裁决（2026-11-09，owner）：Todo 不写 `--skill` 就没有手册，不继承 `<skill_candidates>`（§3.2）。原问：没有手册（建议，逼 plan 明确选择）还是缺省继承 `<skill_candidates>` 全部？
> 按建议
4. ✅ 已裁决（2026-11-09，owner）：**只有 owner 能装，Agent 自己装的入口暂不提供**。删除 `agent-session skill install`（`remove` 随裁决 16 一并删），kRPC `skills.install / skills.remove` 仅 owner；`skill import` 与结晶只产出草稿，owner 在 Skills 页或 `buckyos skill install` 确认安装（§3.6，S23）。原问：`skill import` 缺省直接安装并通知（建议）还是缺省 `--dry-run` 等 owner 确认？
> 先只通过ui装skill吧（讨论了一下，乱装skill是核心风险之一)
5. ✅ 已裁决（2026-11-09，owner）：结晶 Goal 对 `origin: imported|local` 的手册自动改写，留 `revisions/` 备份（§3.8）。原问：自动改写（建议，留备份）还是一律走草稿 + 人工确认？
> 按建议
6. ✅ 已裁决（2026-11-09，owner）：删除 `skills_feedback`，使用记录只留宿主计数（§3.4）。原问：`report.skills_feedback` 对带手册的 Work 是必填（宿主校验缺失则 nudge 一次）还是可选？
> 我觉得删除 skills_feedback 比较好，结晶的过程肯定会看整个worksession的
10. ✅ 已裁决（2026-11-05，随 runtime TODO §3）：记在 `.meta/skills_state.json` 的 `install`，key 用规范 runtime_id（“安装记录永远写实际 runtime_id”）；手册文件保持内容寻址，front matter 不加字段（R-W6）。
11. ✅ 已裁决（2026-11-09，owner）：`--yes` 覆盖全部风险等级，不设 `--accept-risk`（§3.6.1 B）。原问：只覆盖 `low` 风险（建议）还是覆盖全部？影响脚本化安装。
12. ✅ 已裁决（2026-11-05，随 runtime TODO R-U1 / 裁决 15）：自动取。显式 `--runtime` > 继承 > skill 落点 > 私有；都装在同一 runtime 才自动取，装在多台且都可用取 `pinned_runtime`，多个 skill 落点冲突拒绝并列出。缺省装私有所以很少触发；实现是 runtime R4。
13. ✅ 已裁决（2026-11-05，随 runtime TODO 裁决 11 / R-U9 / `grant.install`）：两种都允许，由目标 runtime 的 `grant.install` 门控。用户态安装（`tools/<name>/bin` 进 `<agent_root>/tools` 或节点 `.opendan/<agent>/tools`）对应 `agent_bin`；包管理器 / 系统位置对应 `system`，只在 `grant.install = system` 的 runtime 上允许（私有缺省就是 `system`），容器上必须落持久卷（`writes` 声明，否则 `volatile`）。风险等级：需要 `system` 的脚本记 `high`（确认页标出；`--yes` 同样覆盖，裁决 11），落点是 owner 共用机器时文案点明“会装到你的电脑上”（R-M8）。ExtTools 清单里的命令一律拒绝由 skill 自装。
14. ✅ 已裁决（2026-11-05，owner，见 ExtTools TODO §0.3 / §3.9.3）：skill **加机器可读的 `commands` front matter**（只写命令名），`requires` 继续给人读；`install.toml [check] commands` 缺省取它。系统凭扩展包目录的 `provides` 反推“装哪个包能让这个 skill 可用”，作为安装时的推荐，不 block；用户在 host 上自备的命令按 `provided_by: system` 计可用。runtime R-W6 的“不加字段”只针对运行位置（`runs_on`），不针对依赖声明。官方手册（`aigc_*`、`aiworkspace_run` 等）补 `commands`，随 S06 做。

### 6.1 2026-11-09 回写时的推定与新问题

推定（已按此写进正文，owner 已经确认）：

1. 平铺的 `cmd_skills/<name>.md` 不再认。§7.2 第 1 点原写“旧的继续认”，与仓库“不做向下兼容”的规则冲突，按规则写（§2.1）。
2. `skills_considered / no_skill_reason` 随 `skills_feedback` 一并去掉，理由相同；“没有手册的 Work”改由宿主计数。`usage.*.recent_sessions` 是为了让结晶找得到要读的 Session 而加的（§3.4）。
3. 结晶自动改写只动正文与 `description`；`scope / commands / baseline / platforms` 的变化与新增 `tools/` 走草稿（§3.8）。
4. front matter `commands` 的可用性不落盘：现实现是 `skills.status` 现算，[ExtTools TODO §3.9.3](opendan-exttools-cross-platform-todo.md) 写的 `install.runtimes[id].commands: {name: {present, provided_by}}` 形状不做；`pending_confirm` 状态取消（§2.2）。
5. 对话入口的 `skill_import` 只产出草稿，不用 `notify.ask` 让 owner 在对话里确认安装（按“先只通过 UI 装”读，§3.6.1 B）。

新问题（接 §6 / §7.3 的编号；2026-11-09 owner 已答复）：

15. ⏸ 暂不处理（2026-11-09，owner：这个先不管）。已知缺口，记录保留，重提时再定。原问：`origin: local` 算不算“Agent 自己装”？入口去掉后，Agent 仍能直接往 `$OPENDAN_AGENT_ROOT/cmd_skills/` 写文件：`skills::list` 把没有安装记录、也不在包清单里的手册列为 `local`，冻结不看来源，也没找到对这个目录的写保护（只在 `lib_opendan` 里按关键词查过）。当时的建议：选择与冻结只认 `package` 和有 owner 安装记录的；`local` 只列出并标 `unconfirmed`，owner 在 Skills 页确认后才可选；开发态 root 没有包清单、全部是 `local`，给 `--dev` 豁免。要防伪造还得让安装记录（`.meta/skills.json`）不在 Agent 可写处，这一步做不做一并定。
16. ✅ 已裁决（2026-11-09，owner）：一并去掉，Agent 侧只留 `skill list`（§3.6，S23）。原问：`agent-session skill remove` 是否一并去掉？
17. ✅ 已裁决（2026-11-09，owner）：同形，三个目录都是 `<name>/SKILL.md`（§2.1，S01）。原问：`plan_skills/`、`goal_skills/` 是否也改成 `<name>/SKILL.md`？

## 7. 生态热门 skills 按本方案归类（2026-11-03 抽样）

样本来源：skills.sh 全站榜（Vercel，2026-01 上线，榜首 `find-skills` 4.2M 安装）、anthropics/skills、obra/superpowers、mattpocock/skills、vercel-labs、heygen hyperframes、Microsoft Playwright（SKILL.md 直接随 `playwright-core` npm 包分发，本仓库 `node_modules/playwright-core/lib/tools/skills/` 可读）。生态格式是 agentskills.io 规范：`SKILL.md`（front matter `name` / `description` 必填，`license` / `compatibility` / `metadata` / `allowed-tools` 可选）+ `scripts/` + `references/` + `assets/`；`description` 同时写“做什么”和“何时用”，是唯一的触发索引；SKILL.md 建议 < 500 行，细节放 `references/` 按需读。

### 7.1 归类表

| 热门 skill | 内容形态 | 本方案归类 | 备注 |
| --- | --- | --- | --- |
| playwright-cli（Microsoft，15 KB + 10 份 references 共 55 KB，`allowed-tools: Bash(playwright-cli:*)`） | 纯 CLI 手册：quick start、按功能分组的命令、示例、`references/` 索引 | **cmd / work**，`requires: playwright-cli`，`baseline: extended` | 随工具的 npm 包分发，不单独发布；references 体量是正文 3.5 倍，我们没有按需读的落点 |
| playwright-trace（5 KB） | workflow 6 步 + 命令 | **cmd / work** | 与我们的 `aigc_*` 手册同形 |
| hyperframes-cli（heygen，榜 3，~1300 词） | 按任务组织的 CLI 手册 + 故障表；依赖 Node 22、ffmpeg、Chrome | **cmd / work**，`baseline: extended`，`platforms` | 与同包的 `hyperframes`（格式与动效指南）配对：一个 PIKG 里多份 cmd_skill |
| agent-browser（vercel-labs，榜 5，40 行） | 发现桩：`agent-browser skills get core` 由 CLI 自己吐手册，`skills list` 列专项 | **cmd / work** | “手册由工具按安装版本提供”，与开发指南 §2.3“读工具源码按需发现”同一思路 |
| anthropics/pdf（~190 行 + REFERENCE.md / FORMS.md） | 库与 CLI 速查（pypdf、qpdf、pdftotext…）+ 任务→工具表 | **cmd / work**，`baseline: extended` | 典型“交付类 / 信息获取类” |
| anthropics/webapp-testing（~450 词 + `scripts/with_server.py` + `examples/`） | 决策规则 + 自带脚本 | **cmd / work** + `tools/webapp_testing/bin` | `scripts/` 就是我们的 `tools/<name>/bin`；“把脚本当黑盒、先 `--help`”与我们一致 |
| lark-*（飞书，十余份）、azure-*（Microsoft）、prisma-*（Prisma） | 厂商 CLI / API 手册族 | **cmd / work**，一个包多份 | 消费端靠 description 选；与我们 `skill list` → 创建者选的流程吻合 |
| find-skills（vercel-labs，榜 1，~900 词） | `npx skills find/add/update` 的流程 + 甄别规则 | **cmd / ui**（对话里“有没有 skill 能做 X”） | 对应我们 §3.6 的 `skill install <source>`；是生态里少见的“对话级”skill |
| grill-me / grilling / triage / handoff / to-prd（mattpocock，榜 2、16、18、14） | 访谈式方法论；`grill-me` 只是一行桩，`disable-model-invocation: true`（只能用户显式调用） | **plan**（方法论）；“用户显式调用”对应我们的 owner 强制安装 / 不自动选 | 访谈要和用户对话，在我们的分层里属于 `analyze_intent` 的提问阶段，不是 plan_simple（见 7.2 第 3 点） |
| superpowers/brainstorming（~2000 词，无命令，HARD-GATE，产出 design doc 后交给 writing-plans） | 先问后建的方法论 | **plan**，但提问部分归 analyze_intent | 同上 |
| superpowers/writing-plans（~1500 词） | 任务粒度、每任务文件路径 / 接口 / 测试 / 验证命令 / commit、自审、交接执行 | **plan**，几乎就是 §3.3 的“分析阶段 + 建表” | 正文里有 `pytest` / `git commit` 示例，触我们的“plan 禁命令”lint：导入时应把命令段拆到 cmd（tdd）里，plan 只说“加验证 Todo” |
| superpowers/systematic-debugging（~1100 词，四阶段，Iron Law） | 方法论 + 少量示例命令 + 3 份 references | **plan + cmd 各一份** | 典型混合体：阶段划分给 plan（explore → 假设 → 修 → 验证的 Todo 拆法），命令与 references 给 work |
| anthropics/mcp-builder（四阶段 + 按阶段读 reference + `npm run build` / inspector 命令） | 流程 + 手册 + 参考库 | **plan（mcp_builder）+ cmd（mcp_python / mcp_typescript）** | “第 N 阶段读哪份 reference”正是 progressive disclosure；我们只能整篇冻结 |
| anthropics/frontend-design（榜 15，~1300 词，无命令） | 设计原则与自我审视流程 | **cmd / work（无 tools）** | 是 `todo.md` 说的“范式类”：工作时的知识，不是命令也不是计划。说明 `cmd_skills` 这个名字偏窄，语义上应叫 work_skills |
| vercel-react-best-practices（榜 31，70 条规则，`rules/` 目录，无命令） | 规则索引 + 每条规则一个文件 | **cmd / work（无 tools）** | 同上；`rules/` 需要按需读 |
| coreyhaines/copywriting（~2300 词 + 3 份 references，无命令） | 写作范式 + 禁用词表 | **cmd / work（无 tools）** | 同上 |
| anthropics/skill-creator（4–5k 词，带 eval 脚本与 grader 子 agent） | 造 skill 的流程：意图 → 访谈 → 草稿 → eval → 对比基线 → 迭代 | 对应我们的 **skill_import 行为 + 结晶 Goal** | 它的“description 是触发机制、Claude 倾向欠触发所以 description 要写得激进”对我们的 `skill list` 选择同样成立 |

### 7.2 对本方案的修正

1. **要有按需读的落点**：热门 skill 里 references 普遍比正文大（playwright 3.5 倍、react 70 条规则、pdf 的 REFERENCE.md）。我们整篇冻结、最多 8 份的模型装不下这种 skill。建议 S01 同时把布局改成目录形式 `cmd_skills/<name>/SKILL.md`（+ `references/`、`tools/`；旧的 `<name>.md` 继续认），冻结只进 SKILL.md 正文，正文里用相对路径指 `references/`，Session 用 `read $OPENDAN_AGENT_ROOT/cmd_skills/<name>/references/x.md` 按需读；ui 大小 lint 改为对 SKILL.md 正文。（✅ 裁决 9：S01 一并做，见 §2.1；“旧的 `<name>.md` 继续认”按仓库“不做向下兼容”的规则改为不再认，§6.1 推定 1。）
2. **front matter 对齐 agentskills.io**：`description` 按生态写法同时写“何时用”（我们的选择提示词只看 description，正好）；`compatibility` → `requires`；`metadata.version` → 版本；`allowed-tools: Bash(<cmd>:*)` 里的命令名映射成 `commands`（§3.6.2 第 4 点；它的权限语义我们没有对应物，权限在 runtime 侧，报告里说明）；`disable-model-invocation` → 标为 `scope: ui` 且不自动选（只能 owner `skill ui add`）。`skill_import` 的映射表写进 S12。
3. **访谈式 plan skill 的归属**：生态里最热的方法论 skill（grill-me、brainstorming）是“先把用户问清”，在我们的分层里这是 `analyze_intent` 的 `ask_user` 阶段，plan_simple 没有对话权。建议 plan skill 分两种读者：`analyze_intent` 在“Ready”判断前可读 `plan_skills` 的 description 决定该问什么（不读全文）；plan_simple 读全文建表。记入 §6 待裁决 7。（✅ 裁决 7：只给 plan_simple。它已开 Input queue、有 `ask_user`，访谈式的提问由它在分析步自己做，上面“两种读者”的建议不采纳，见 §3.3。）
4. **混合体是常态**：writing-plans 带 pytest/git、mcp-builder 带 build 命令、systematic-debugging 带诊断命令。`skill_import` 的核心工作就是脑图说的“拆分成特定 skills”：方法论段 → `plan_skills/`，命令段 → `cmd_skills/`，两者同名互指。plan 的 lint 对导入产物生效，对手写的只警告。
5. **工具自带手册是导入源**：playwright 把 SKILL.md 放进 npm 包，agent-browser 由 CLI 吐手册。`skill import` 应支持扫描 runtime 上已安装工具的 skills 目录（`node_modules/*/lib/tools/skills`、`<cli> skills list`）作为来源，不只接受用户贴的文本。
6. **命名**：`cmd_skills` 里会装 frontend-design 这类无命令的范式手册。目录名为兼容可以不改，但文档、`skill list` 与提示词里统一称 work skill / ui skill；`SkillKind::Cmd` 的对外字符串是否改为 `work` 记入 §6 待裁决 8。（✅ 裁决 8：改，见 §2.1。）
7. **plan skill 首批可直接改写自生态**：`plan_software_change` ← writing-plans + systematic-debugging 的阶段部分；`plan_research` ← superpowers 的 dispatching-parallel-agents；`plan_content_production` ← mcp-builder 式的“按阶段读参考”结构 + hyperframes 的 lint → check → preview → render 门禁。许可证：superpowers、mattpocock 为 MIT，anthropics/skills 的 frontend-design / mcp-builder 为自有许可（LICENSE.txt），改写前核对。

### 7.3 待裁决补充

7. ✅ 已裁决（2026-11-09，owner）：只给 plan_simple（§3.3）。原问：plan skill 是否也给 `analyze_intent` 读 description（决定提问），还是只给 plan_simple？
> 只给 plan_simple，现在plan_simple 开Input queue了，拥有对话能力
8. ✅ 已裁决（2026-11-09，owner）：改，目录名保留（§2.1）。原问：对外是否把 `cmd` 改名 `work`（`skill list` 的 kind、`--kind`、kRPC 字段），目录名保留？
> 好
9. ✅ 已裁决（2026-11-09，owner）：S01 一并做（§2.1）。原问：布局改目录形式（`cmd_skills/<name>/SKILL.md` + `references/`）是否在 S01 一并做，还是先只加 `scope` 字段？
> 一并做吧
