# OpenDAN 内置 cmd_skills 编写与整合 TODO

日期：2026-10-17

状态：T0 已实施并通过单元/进程级测试，T1/T2 手册与工具已交付并在 DV 上用 CLI 直接验证；经 Agent 身份的真实验收（A07、G05/G07/G11、X04、D05/P03/E04、I08 端到端）等待部署（`start.py` 被权限策略拦下，需用户执行）。实施记录见 §10。（原计划状态：）2026-10-17 review 补入 TS + websdk 工具原则（S01/S04/M06 及各节落点）；容器内 `buckyos` 已定为脚本只读挂载、二进制用镜像（I07）。同日用户裁决：传统 agent-skills 已废弃，skill 安装时分为 cmd_skills / goal_skills，经 PIKG 与 opendan kRPC 安装（I08）；Work 在创建时选定最多 8 个 cmd_skill 并全部装入系统提示词，运行中不调整（I01–I04）；手册是捷径不是规定；AIWorkspace 模板开发暂缓（§7.2）；真实验收用 DV test 环境已有的模型，缺模型的项跳过并标未验证。随后确认：I03 的传递与继承规则、Agent 自行安装、模板升级不覆盖已安装 skill 按本文写法执行；`buckyos-agent` 改用挂载的宿主 `dist/`（I07）；agent-skills 代码已删除（§1）。

目标：基于已有 cmd_skills 机制，让 Agent 能完成 BuckyOS 系统设置与诊断、图片/语音/视频生成、xllm 调用，以及传统 App、Agent 扩展的开发和发布（AIWorkspace 模板暂缓）。skills 是随实现维护的产品内容，不能交付一版手册后任其过期。

关联：[cmd_skill 机制 TODO](opendan-cmd-skill-todo.md) 负责机制后续设计；本文负责首批能力手册及必要的接入闭环。扩展原则参考 [OpenDAN Agent 开发指南](../doc/sdk/OpenDAN_Agent_Dev_Guide.md)：复用 CLI、按需发现，不为每项业务新增模型 tool/action。访问 BuckyOS 的新能力优先用 TypeScript 基于 buckyos-websdk 实现，源码随 runtime 交给 Agent 阅读、派生和组合：Agent 读工具源码学会调用 BuckyOS，比只看手册或用管道串 CLI 更可控（出处 [buckyos-agent readme](../src/tools/buckyos-agent/readme.md)；开发指南尚未写入，见 M06）。

## 1. 当前基础与实际缺口

以下按当前工作树源码核对，表示已有实现入口，不表示已完成真实环境验收。路径默认相对 `buckyos/`；`../buckyos-websdk/`、`../buckyos-devkit/` 为同级仓库。

| 方面 | 当前实现 | 本计划需要补齐 |
| --- | --- | --- |
| 手册加载 | `src/frame/lib_opendan/src/state/behaviors.rs` 从 AgentRootFS 的 `cmd_skills/<name>.md` 按名读取，没有列表接口（kRPC 也只有 `behaviors.cmd_skill`）；`src/frame/lib_opendan/src/template.rs` 将 `default_cmd_skill` 写入配置，`freeze_config` 校验并冻结为单份 `{name, text}`，未知名称报配置错误。手册没有用途等元数据。 | 手册元数据与列表、按列表冻结、选择与错误反馈。 |
| Prompt 接入 | `src/frame/lib_opendan/src/runner/assembler.rs` 只在 UI 分支（`ui_sections`）渲染 `## Commands`，Work 即使冻结了手册也不显示。Jarvis 只有 `cmd_skills/ui_core.md`，`agent.toml` 的 ui/group 使用它，`[session.work]` 未配置。 | Work 的手册装配。只新增 Markdown 不会让 Work 使用它。 |
| 选择与创建 Work | `behaviors/analyze_intent.toml` 的探查步骤与结果 schema 都没有 cmd_skill；`src/frame/lib_opendan/src/bin/xagent.rs` 的 `create-worksession` 及 `src/frame/lib_opendan/src/api.rs` 的 `SubSessionSpec` 没有对应参数/字段。todo 的 `--skill`（`src/frame/agent_tool/src/todo_tools.rs`，上限 3）只作为文字渲染进 todo 正文。 | 发现命令、意图分析提示词与 schema、CLI/字段/协议、todo dispatch 与 Goal 的传递及测试。 |
| 安装 | cmd_skills 只随 Agent 包分发。AppDoc 已有 `pkg_list.agent_skills` / `agent_tools` 子包（`src/kernel/buckyos-api/src/app_doc.rs`），但 `src/frame/control_panel/src/app_installer.rs` 的发布路径拒绝二者；opendan 对外 kRPC（`src/frame/opendan/src/service.rs`）没有 skill 接口。Goal 的 `GoalSkill` 只是 `name=ref` 引用，没有安装落点。 | 见 I08：PIKG 安装与 opendan kRPC 安装入口。 |
| 命令环境 | `src/frame/lib_opendan/src/runtime/bin_overlay.rs` 已有 Session helper，`standard_helpers` 只包装 `llm_understand_media`；另有现行 PATH/tool-plan 机制。 | 逐项核验容器/宿主 runtime 的命令、依赖、身份和产物路径。尚无按 skill 自动挂载 `tools/<skill>/bin` 的能力。 |
| 系统管理 | `../buckyos-websdk/cli/modules/` 已有 `system`、`system_config`、`log`、`diagnostic`、`app`、`task` 模块；Control Panel 有对应服务入口。宿主分发在 `$BUCKYOS_ROOT/libexec/buckyos-tool`（含 `cli/*.ts` 源码），**aios 容器内没有 `buckyos` 命令**：镜像只带 `buckyos-agent` 和 websdk `dist`，node_daemon 也未挂载该目录。 | 按 I07 把宿主 Tool 脚本只读挂入容器；将实际命令组织为设置/诊断流程；未覆盖的管理动作补在 websdk `cli/modules`；验证 Agent 身份能否调用。 |
| AIGC | `src/tools/buckyos-agent/commands/` 已有图像、音频、视频 CLI；`build_aios` 分发 `aicc-tool` 和命令入口。已有 `behaviors/aicc_image_cli.inc` 可复用。 | 整理成专项 cmd_skills，核对参数、provider 能力、异步状态与媒体交付；旧 inc 不能原样视为当前已加载手册。 |
| xllm | `src/frame/agent_tool_cli_dev/src/lib.rs` 提供 `agent_tool xllm`，核心在 `src/frame/agent_tool/src/xllm.rs`。 | 独立使用手册、执行环境与边界说明；不能假定裸 `xllm` 已在 Session PATH。 |
| 开发与发布 | 已有 AIWorkspace RPC/程序宿主、PIKG 工具、Jarvis 模板和系统构建脚本。App CLI 尚无 `app publish`；后台存在 `app.publish` RPC。 | 开发场景手册、可执行样例，以及发布链路的 CLI 缺口。后台写入 Repo/NamedStore candidate 不等于完成 App DID 权威发布。 |

传统 agent-skills（`skills/ + skill.yaml + SKILL.md`）已废弃，代码已于 2026-10-17 删除：`src/frame/agent_tool/src/skills_mgr.rs`、`agent_tool_cli_dev` 的 `agent-skills` CLI、Jarvis 的 `try_create_new_skill` / `improve_skills` 行为；opendan 的 AgentRoot 骨架由 `skills/` 改为 `cmd_skills/`；旧 Agent Skill 设计已合并为 [历史摘要](../doc/pikg/history/agent-skills-design.md)。self-improve 的 `DiscoverSkillCoverageGap` 信号不依赖 skills_mgr，暂留（self-improve 模块本版本未启用）。新的 skill 在安装时分为两类：cmd_skills（命令手册，可带工具）和 goal_skills（Goal 使用）。本文负责 cmd_skills 的选择、装配与两类 skill 的安装入口；goal_skills 在 Goal 中如何引用属于 Goal 的 TODO。

## 2. 首批手册与交付约定

手册源码统一放在 `src/apps/jarvis_agent/agent/cmd_skills/`；Jarvis 的 `dapp_meta/pikg.json` 已将整个 `agent/` 作为子包打包。共享到其他 Agent 的方式沿用机制 TODO 设计，避免维护多份正文。

| 建议文件 | 负责的任务 | 最小交付 |
| --- | --- | --- |
| `buckyos_admin.md` | 系统设置、运行诊断、必要时结合运行版本源码定位问题。 | 一次设置变更闭环和一次有证据的故障诊断。 |
| `aigc_image.md` | 图片生成、编辑、局部重绘、背景处理等。 | 生成/编辑图片，检查结果并交付可打开的文件。 |
| `aigc_audio.md` | 文本生成语音；按已有能力扩展音乐生成、音频增强。 | 可播放语音、参数与文件信息。 |
| `aigc_video.md` | 文生/图生/视频转换，延长、增强及多段视频制作。 | 可播放视频；复杂任务有分镜、素材和中间产物。 |
| `xllm.md` | 一次性模型调用、结构化输出、有界工具执行与分析。 | 可复用调用示例和成功/失败判断。 |
| `buckyos_dev_release.md` | 共用环境准备、构建、打包、测试安装、发布和验证。 | 从源码到可下载、可安装发行物的证据链。 |
| `aiworkspace_template_dev.md`（暂缓，见 §7.2） | AIWorkspace 模板设计、生成、预览、导入导出与分发。 | 可在新 Workspace 复用的模板和示例。 |
| `buckyos_app_dev.md` | static-web/script/docker 等传统 App 开发。 | 构建、PIKG、测试安装和升级样例。 |
| `opendan_agent_dev.md` | Agent 模板、behavior、cmd_skills、CLI 工具扩展。 | 可创建实例并运行的模板/扩展及验证报告。 |

- [x] **S01 手册模板**：每份手册说明适用意图、前置条件、真实命令发现方式、最短成功示例、输入/输出、失败与恢复方式、验收方法、实现来源、最近核对版本/日期。版本记录用于核验当前能力，不要求新增旧版本兼容层。“实现来源”须写成 Agent runtime 内可读的路径：命令源码、可照抄的范本文件、SDK `.d.ts`；并说明 CLI 不覆盖时的做法：读源码，把派生脚本写到 Agent 或当前 Session 的 `tools/`，用 `--config` 复用安装态 `deno.json`，不改只读安装副本。手册是做成一类事的捷径，不是规定：写成“可以这样做、这样做更快更稳”，不写“必须”“只能”；Agent 有更合适的做法时可以不用。
- [x] **S02 可发现性**：每份手册在 front-matter 写名称、用途、前置条件，由 I01 的 `agent-session skill list` 输出，供意图分析按需读取；具体命令参数通过 `--help` 或 `buckyos command describe <module> <verb>` 获取。索引不能把未完成能力写成可用能力。
- [x] **S03 内容收敛**：`ui_core` 保持常用会话命令；专项长手册进入 Work。共用开发发布步骤只维护一处，由场景手册明确引用；保证引用文件随包分发且能在 runtime 读取。
- [x] **S04 工具边界**：复用已有 CLI；确有缺口才补命令。新命令默认用 TS 基于 websdk 编写：人也会用的系统、App、发布命令进 websdk `cli/modules`，保持 `buckyos` 命令只有一个源码真相源；Agent 专用的配方类工具（AIGC、媒体合成等）进 `src/tools/buckyos-agent`，沿用其 6 步配方结构。不优先新增 Rust `agent_tool` 子命令或 shell 薄包装。两类源码都必须在 Agent runtime 中可读。命令能被发现不代表当前身份有权限或 provider 支持，手册应说明实际拒绝/不支持结果的处理。

## 3. 机制与运行环境接入（先行）

- [x] **I01 Work 选择链路**（已定：创建 Work 时选好，最多 8 个）：打通“意图分析发现手册 → 创建 Work 传入名称 → 校验 → 冻结 → Work prompt 装配”。
  - 发现：`agent-session skill list` 输出已安装 cmd_skills 的名称、用途、前置条件（读 S02 的 front-matter，经 opendan kRPC `skills.list`）；结果在运行消息里，不进公共前缀。
  - 选择：`behaviors/analyze_intent.toml` 的探查步骤加入 `skill list`，“Creating”一节加入 `--cmd-skill`，结果 schema 的 `work` 加 `cmd_skills` 数组；按任务需要选，不选也是合法结果。
  - 协议：`create-worksession --cmd-skill <name>`（可重复）→ `SubSessionSpec.cmd_skills` → `prompt.cmd_skills: Vec<String>` → `frozen.cmd_skills: Vec<CmdSkill>`。UI 的 `default_cmd_skill` 一并改为列表 `default_cmd_skills`（开发期 breaking change，重生成 fixtures）。未知名称、重复名称、超过 8 个都明确失败。不设 Work 基础手册，`[session.work]` 不配默认值。
- [x] **I02 Work 装配**：冻结的 cmd_skills 按选择顺序全部装进 Work 的系统提示词（与 UI 一样放在系统段末尾的 `## Commands`）。段首用一句话说明这些是可选捷径，不要求必须使用。Work 运行中不调整手册：不提供切换/追加命令，提示词也不要求 Work 评估或更换手册。
- [x] **I03 任务传播**：Goal 与 PLAN/DO 创建 Work 走同一 `--cmd-skill` 入口，`behaviors/goal.toml`、`plan.toml` 的创建说明同步加入 `skill list` 与 `--cmd-skill`。todo 的 `--skill` 改为记录 cmd_skill 名，`todo dispatch` / `todo explore` 把它作为子 Work 的 cmd_skills 传入，校验与上限同 I01（todo 上限由 3 调为 8）。未显式指定时：父为 Work 则继承父 Work 冻结的选择，父为 UI 或 Goal 则为空；UI 的 `ui_core` 和 Goal 的 goal_skills 都不会自动注入 Work。
- [x] **I04 冻结语义**：新建 Session 读取当时已安装的手册，运行中 Session 保持冻结正文；安装、更新、删除 skill 只影响之后创建的 Session。`catalog_rev` 已把 `cmd_skills/` 计入，冻结记录保留每份手册的名称与正文。
- [ ] **I05 runtime 与打包**：核验系统 `buckyos`（容器接入见 I07）、项目 SDK CLI、`agent_tool xllm`、AIGC 命令、AIWorkspace 入口在容器/宿主环境的可用性；缺失时给出准确前置条件。同步 Jarvis 包与 `build_aios`/ExtTool 等实际分发来源，避免只改 rootfs 或生成产物。
- [x] **I06 最小回归**：扩展 `src/frame/lib_opendan/tests/{xagent_cli,xagent_lib}.rs`、`src/frame/opendan/tests/loader.rs` 和 `src/frame/lib_opendan/src/runtime/bin_overlay.rs` 的相关用例，覆盖名称选择、未知名称、超过 8 个、Work 实际装配、todo dispatch 与继承、冻结稳定性、安装/删除后新旧 Session 的差异、命令缺失；保留现有 UI 行为。
- [ ] **I07 容器内 `buckyos` 命令**（已定：脚本只读挂载，二进制用镜像里的，因为宿主可能不是 Linux）：
  - 挂载：node_daemon 在 `src/kernel/node_daemon/src/app_loader.rs` 的 `build_worker_volume_mounts` 中，把宿主 `$BUCKYOS_ROOT/libexec/buckyos-tool` 的脚本与清单（`cli/`、`dist/`、`package.json`、`distribution.json`）只读挂到容器 `/opt/buckyos/libexec/buckyos-tool`。不挂也不调用 `runtime/deno`。
  - 启动：aios 镜像提供 `buckyos` launcher（参照宿主 launcher `src/rootfs/bin/buckyos` 与镜像内 `publish/aios/aicc-tool`），用镜像自带的 Deno 运行挂载的 `cli/system_bootstrap.ts`；bootstrap 用 `Deno.execPath()` 派生 launcher，会沿用同一 runtime。挂载缺失时明确报错，不回退。
  - 连接与身份：容器内 CLI 的 endpoint 只认 `BUCKYOS_TOOL_ENDPOINT`，不读 `BUCKYOS_HOST_GATEWAY`；由 launcher 或 node_daemon 提供指向 `host.docker.internal` NodeGateway 的地址。身份用注入的 `BUCKYOS_APPCLIENT_SESSION_TOKEN`（`cli/core/auth.ts` 已优先使用），逐模块核对 Agent 身份的 RBAC 结果。
  - 版本：脚本随宿主系统升级，Deno 随镜像更新，两者各自漂移（当前镜像 Deno v2.9.2，本地 Tool 分发记录 2.9.4）。launcher 按 `distribution.json` 的 `deno_version` / `protocol_version` 检查兼容范围，不兼容时明确失败；`--version --verbose` 如实报告实际 runtime。
  - 收益：Agent 读到的 `cli/modules/*.ts` 与运行系统同版本，可直接作为 A05 的第一层证据。
  - `buckyos-agent` 改用挂载的 `dist/`（已定）：安装态 `deno.json` 的 `buckyos` 指向 `/opt/buckyos/libexec/buckyos-tool/dist/node.mjs`，与 `buckyos` 命令共用宿主同一版 SDK；`build_aios` 不再拉取、vendor websdk。镜像构建期没有挂载，Dockerfile 里的 `deno check`、`gen_image --help` 与 compose smoke 改为构建期用临时 SDK 只做检查（不打进镜像），或移到容器启动后的检查；`aicc-tool` launcher 在挂载缺失时明确报错。仓库内开发态 `deno.json` 改指 `src/rootfs/libexec/buckyos-tool/dist/node.mjs`，同步 `src/tools/buckyos-agent/readme.md` 与 `doc/arch/11_env_contract.md` 的相关说明。
  - 验证：扩展 `src/kernel/node_daemon/src/test_app_loader.rs` 的挂载参数断言；容器内以 Agent 身份执行 `buckyos --version --verbose`、`buckyos system status` 成功；宿主缺少 Tool 目录时报错可读。
- [x] **I08 skill 安装**（已定：cmd_skills 与 goal_skills 都可经 PIKG 安装，opendan runtime 提供明确的 kRPC 安装接口）：
  - kRPC：在 `src/frame/opendan/src/service.rs` 增加 `skills.list` / `skills.install` / `skills.remove`，按 kind（`cmd` / `goal`）分别落到 AgentRoot 的 `cmd_skills/<name>.md`、`goal_skills/<name>.md`；cmd_skill 附带的工具落到 `tools/<skill>/bin`（cmd_skill 机制 TODO 待设计项 1 的 PATH 层）。安装记录来源 PIKG、版本与 digest。
  - 入口：Agent 用 `agent-session skill list|install|remove` 调用该 kRPC；用户侧由 websdk `buckyos` CLI 调用同一接口（按 S04 用 TS 实现）。Agent 可以自行安装，按“先能力后安全”记为已知风险，不加审批。
  - 包格式：复用 AppDoc 的 `pkg_list.agent_skills` 子包与现有 PIKG 构建/校验流程，包内按 `cmd_skills/`、`goal_skills/`、`tools/` 分目录；`pikg init` 增加对应 kind，解除 `app_installer.rs` 对 `agent_skills` 的拒绝。包结构在机制 TODO 定稿后实施。
  - 与模板：Agent 模板自带的 skill 与安装的 skill 分开记录来源；模板升级不覆盖、不删除已安装的 skill；与已有名称冲突时拒绝，除非显式替换。
  - 验证：PIKG 构建 → 安装到测试 Agent → `skill list` 可见 → 新建 Work 可选中并装配 → 删除后新 Work 不可选、旧 Work 冻结正文不变。

## 4. BuckyOS 系统管理与诊断

依据：`../buckyos-websdk/cli/modules/{system,system_config,log,diagnostic,app,task}.ts`、`src/frame/control_panel/src/`、`src/check.py`、`doc/arch/system_config_reference.md`、`doc/path_usage.md`。

- [x] **A01 环境识别**：先识别目标 Zone、调用身份、runtime、系统/Tool/SDK 版本与实际可用命令。区分系统分发 `$BUCKYOS_ROOT/bin/buckyos`、容器内经 I07 挂载的同一份脚本和项目分发；不能把容器路径当宿主路径，也不能将 `check.py` 的“待激活”当成运行正常。
- [x] **A02 设置流程**：盘点用户/Agent、设备、应用、网络、存储、AI provider 等常见设置的真实管理入口，列明已支持范围和缺口。优先用领域 API/CLI，缺口命令按 S04 补在 websdk `cli/modules`；需改 system-config 时区分用户 `Settings` 和系统生成 `Config`，不能把直接改派生配置当通用方案。
- [x] **A03 变更闭环**：读取现状 → 按用户目标形成最小变更 → 执行 → 检查调度/服务实际生效 → 返回结果与必要恢复方法。遵守现有授权、RBAC 和 CLI 确认语义；已授权范围内不增加逐步反复确认。
- [x] **A04 诊断流程**：复用 `buckyos system status`、`system-config get/list`、`log query/tail/export`、`diagnostic collect/export`、App/Task 状态；按时间线整理现象、证据、假设、验证和建议。远程 CLI 不可用时，说明哪些本机检查需要宿主访问，不伪造已取得的日志。
- [x] **A05 对应版本源码分析**：第一层先读容器内挂载的 Tool 源码（I07，与运行系统同版本），理解客户端调用与 RPC 协议；需要追服务端实现时，再使用匹配运行版本的本地源码，否则下载对应发行物的只读源码快照到工作目录。`buckyos-agent` 与 `buckyos` 命令共用挂载的同一份 SDK（I07），版本以宿主 `distribution.json` 为准。记录 BuckyOS 版本、build ID/源码 revision、Tool/SDK 版本和来源；`src/tools/prepare_sdk_tool_distribution.py` 的 `build_id` 已含构建提交信息，可作为线索，但不能据此假定整个运行系统及本地未提交改动完全一致。
- [x] **A06 来源校验与降级**：建立发行物到源码快照的对应方式并验证摘要；无法确定运行源码时明确证据缺口，不能拿最新 `main` 直接断言旧版本行为。源码快照只用于诊断；开发依赖继续遵守 `AGENTS.md` 的显式 `main` 规则，不修改依赖来“匹配”故障版本。
- [ ] **A07 验收**：完成一项可恢复设置并验证实际效果；对一个可复现故障给出日志与源码位置支撑的结论；覆盖权限不足、服务不可达、源码版本不匹配。诊断输出不得包含私钥/token。

## 5. AIGC：图片、语音、视频

依据：`src/tools/buckyos-agent/aicc-tool.ts`、`src/tools/buckyos-agent/commands/`、`src/tools/buckyos-agent/lib/{aicc,io,result}.ts`、`src/frame/aicc/src/`、`src/kernel/buckyos-api/src/aicc_client.rs`。`llm_understand_media` 是媒体理解，不替代生成命令。

### 5.1 共用接入

- [x] **G01 迁移手册**：核对 `src/apps/jarvis_agent/agent/behaviors/aicc_image_cli.inc` 和工具 README，将有效说明拆入三份手册，包括 README 的“为什么源码长这样”：手册指向容器内 `/opt/buckyos/bin/opendan/buckyos-agent/` 的 `commands/*.ts` 范本（`gen_image.ts`）和 `lib/*.ts`，并给出派生工具的写法与运行方式；检查旧 include 是否仍有消费者，避免重复维护。旧 `<attachment .../>` 示例须按当前产物/消息协议改写。
- [x] **G02 能力与材料**：默认沿用逻辑模型和 AICC 路由，按实际能力使用尺寸、时长、格式等参数；不将 provider 健康检查设为每次调用的前置步骤。区分“有 CLI 入口”和“当前 provider 能执行”。保留输入的完整对象 ID/类型前缀；需要本地文件时复用 `materialize_resource`，不拿相似旧文件替代素材。
- [x] **G03 长任务与交付**：区分 shell 后台 task 与 AICC 生成 task，记录两层 ID；超时/断连不等于服务端失败。CLI 已有等待与幂等/预算选项，但按生成 task ID 续查并恢复下载的媒体命令仍需补齐（在 `buckyos-agent` 中用 TS 实现）。避免重复提交，保留中间产物。当前 stdout 始终是 `AgentToolResult`，不是加 `--json` 才结构化；检查成功状态与 `detail.files` 的实际文件，再通过 Work `report.artifacts` 及当前消息附件流程交付并验证可打开/播放。

### 5.2 图片

- [x] **G04 图片手册**：覆盖 `gen_image`、`edit_image`、`inpaint_image`、`upscale_image`、`remove_bg`；说明文本、参考图、mask、输出路径和生成后检查。模型不支持的编辑方式不写成通用保证。
- [ ] **G05 图片验收**：至少一次文生图和一次有参考素材的编辑；核对文件、格式、尺寸及内容；覆盖错误输入、provider 拒绝、结果下载失败。

### 5.3 语音与音频

- [x] **G06 音频手册**：以 `text_to_speech` 为最小闭环，核对音色、语言、格式、长文本等实际参数及限制；按能力补 `gen_music`、`enhance_audio`。语音识别属于辅助能力，不能代替语音生成验收。
- [ ] **G07 音频验收**：生成可播放语音，验证文本内容、格式与时长；覆盖不支持的音色/语言、过长输入和 provider 失败，明确是否支持分段生成与拼接。

### 5.4 视频（单独分期）

- [ ] **G08 V0 单段生成**：先完成 `gen_video` / `img2video` 的真实 provider 闭环；核对任务等待、下载、超时续查和播放。`video2video`、`extend_video`、`upscale_video` 按实际能力逐项接入；续写要验证 continuation handle 的有效/过期/不支持情形，末帧续做加拼接只能作为明确说明的替代方案，不能承诺原生无缝续写。
- [ ] **G09 V1 多素材制作**：流程覆盖需求与时长 → 分镜/脚本 → 图片/语音素材 → 分段生成 → 合成/字幕/音画同步 → 检查 → 交付。先核实 runtime 现有媒体工具（镜像已有 ffmpeg、imagemagick）；缺少合成/检查能力时，优先在 `buckyos-agent` 写 TS 配方工具来编排这些命令，列为独立工具工作，新增依赖按仓库规则处理。
- [ ] **G10 成本与恢复**：记录分镜、素材、生成参数、task ID、已完成片段和预算边界；失败只重做必要片段。让等待/恢复沿用 Session/Task 机制，不能仅靠长时间阻塞 shell 或无限轮询。
- [ ] **G11 视频验收**：V0 验证单段可播放、尺寸/时长符合目标；V1 验证至少两段画面与配音合成，检查音画和字幕（若有）、保留中间素材，并覆盖一次失败恢复。只有 provider mock 时标为未完成真实生成验收。

## 6. 使用 xllm

依据：`src/frame/agent_tool_cli_dev/src/lib.rs`、`src/frame/agent_tool/src/xllm.rs`、`doc/llm_context/`。推荐先使用已存在的 `agent_tool xllm`，若提供快捷入口，应纳入正式包装与分发。xllm 属于 runtime 层，保留 Rust 实现不违背 S04；websdk 的 TS 版尚未实现，手册不写成可用。

- [x] **X01 调用说明**：核对当前 `--help` 与参数解析，给出文本/文件/管道输入、模型选择、上下文目录和输出保存示例；区分 `--json` 的模型结果约束与 `--format json` 的 CLI 结果信封。覆盖现有 `list/status/result`、`--resume --run`，说明恢复沿用保存配置、如何重导出结果及各退出码，避免可恢复暂停被当成重新调用。
- [x] **X02 执行边界**：说明独立 xllm 默认不启用工具，以及启用工具、behavior/function_call 模式的真实配置限制；核对模型/provider 配置、cwd 和身份。不能把 xllm 当成接管活跃 xagent Session 的办法。
- [x] **X03 选择与约束**：一次性转换/分析可用 xllm；需要持久任务、PLAN/DO 或用户交互的工作使用既有 Work Session。配置调用预算/超时，避免 Agent 递归套用 xllm；正确处理失败、pending 和恢复边界。
- [ ] **X04 验收**：文本分析、结构化结果、文件输入与受限工具执行各一例；覆盖模型不可用、无效输出、超时与不允许的上下文复用，说明命令成功不等于任务验收成功。

## 7. BuckyOS 开发和发布

### 7.1 共用开发发布闭环

- [x] **D01 环境与构建**：手册引用当前 `AGENTS.md`、`src/buckyos-build.py`、`src/bucky_project.yaml`、devkit 和 SDK CLI。明确工作目录、平台依赖、系统构建与 App 自有构建的分工，复用 `start.py`/`check.py` 和镜像本地迭代方法。同步修正文档中的陈旧命令，尤其是 `doc/sdk/app-dev-quickstart.md`。
- [x] **D02 打包与安装**：复用 `buckyos pikg init/build/pack/info` 和 `app fetch/install/upgrade/status`；以当前 schema 核对参数。区分首次安装计划、本地新版本安装升级、Catalog 升级，保留版本、digest、任务状态与验证结果。
- [x] **D03 发布补齐**：盘点 `src/frame/control_panel/src/app_installer.rs` 的 `app.publish` 后台能力，在 websdk `cli/modules/app.ts` 补 `buckyos app publish`。分别验证 Owner 签名、PIKG/资源托管、BNS/App DID 权威发布及回读；上传 candidate 或本机安装成功均不能视为已发布。
- [x] **D04 系统发行**：区分 BuckyOS 系统/SDK/工具镜像发行与 App/模板发行，按 `publish/`、`.github/workflows/`、`build_aios` 和 release manifest 盘点构建、测试、产物上传、版本更新入口；手册不臆造统一发布命令。沿用实际发布目标的授权流程。
- [ ] **D05 发布验收**：在 DV test 环境上可下载且摘要一致，签名/权威版本可回读，新环境能安装和运行；记录失败恢复/撤回能力的实际边界，不承诺尚不存在的自动回滚。

### 7.2 AIWorkspace 模板开发（暂缓）

暂缓（2026-10-17 用户裁决）：AIWorkspace 的 kRPC SDK 化与 `aiworkspace_template_dev` 手册都等 AIWorkspace 部分设计稳定后再做；W01–W03 不进入当前阶段，下文保留为当时的起点。

依据：`src/frame/desktop/src/app/aiworkspace/api/{sample,demos,client}.ts`、`src/frame/aiworkspace/server/src/lib.rs`、`src/frame/aiworkspace/aiws/`。当前示例通过创建 Workspace 再提交内容构造，服务有 `ws.begin_import` / `ws.import`、`doc.export`；程序宿主 `aiws` 不等于现成模板发行 CLI。

- [ ] **W01 模板规范**：定义模板的参数、元数据、Record/Surface/Cell/Block/Wish 等对象、资源依赖及版本；区分可复用模板与某用户 Workspace 的数据快照，明确复制/重新绑定规则。
- [ ] **W02 制作入口**：复用已有 schema、RPC、示例和 SDK，补生成、校验、预览、导出、从模板新建 Workspace 的 CLI/脚本闭环。websdk 尚无 AIWorkspace 客户端：先把 `src/frame/desktop/src/app/aiworkspace/api/client.ts` 的 RPC 访问收敛进 websdk，再在其上用 TS 写模板命令，供 Desktop 与 Agent 共用；模板目录发现、打包与发布格式作为明确交付项。
- [ ] **W03 验收**：至少一个含真实资源和可执行内容的模板，可导出并在新 Workspace 导入；核对引用、渲染、权限与运行结果，覆盖缺失资源/无效 schema；默认不覆盖用户已有 Workspace。

### 7.3 传统 App 开发

依据：`../buckyos-websdk/cli/modules/{pikg,app}.ts`、`test/app_installer_test/pikg_samples/`。当前 `pikg init` 支持 `static-web` / `script` / `docker`；PIKG 消费构建输出，不替代 App 自身构建。

- [x] **P01 开发手册**：覆盖脚手架选择、SDK/runtime 身份接入、AppDoc/权限/服务与数据目录、构建输出、PIKG 配置和本地调试。每种运行形态标明前置条件，避免把宿主工具带进应用运行包。
- [x] **P02 安装与升级**：复用现有静态 Web 和 Docker 样例，增加新版本、测试 Zone 安装、日志与状态检查、升级后的持久数据验证；发布调用共用手册。
- [ ] **P03 验收**：至少静态 Web 与 Docker 两条实际闭环，页面/API 可用，首次安装与升版本升级均有证据；script 分支有可执行最小样例和限制说明。

### 7.4 Agent 开发与扩展

依据：`src/apps/jarvis_agent/{agent,dapp_meta}/`、`src/frame/control_panel/src/agent_mgr.rs`、`src/debug_jarvis.sh`、OpenDAN Agent 开发指南。

- [x] **E01 模板开发**：从最小 AgentRootFS 出发，讲清 `agent.toml`、behavior、cmd_skills、CLI 工具、Session/Workspace、产物报告和调试。CLI 工具以 TS + websdk 为主路径，以 `buckyos-agent` 为范本。使用 Jarvis 的 `agent` 子包结构，不能写不存在的 `pikg init --kind agent`。
- [x] **E02 创建和运行**：补齐模板打包、测试 Zone 登记、从模板创建独立 Agent、运行及更新验证。明确 AgentId、AppId、AppInstance binding 的关系，模板不携带某个实例的私钥或状态。
- [x] **E03 扩展分发**：覆盖增加/修改 behavior、cmd_skill、工具脚本及所需依赖的开发方式；工具以可读的 TS 脚本放进 cmd_skill 的 PATH 层（机制 TODO 待设计项 1 的 `tools/<skill>/bin`），与手册一起分发；核对工具镜像/PATH。cmd_skill / goal_skill 的打包、安装、更新与移除走 I08；`agent_tools` 子包仍被发布路径拒绝，未接通前不能仅靠文档声称可发布。
- [ ] **E04 验收**：一个模板可创建新 Agent，执行带专项 cmd_skill 的任务并产出报告；一个扩展有安装/更新后的真实调用验证，检查旧冻结 Session 与新 Session 的版本行为及实例数据保留。

## 8. 必须修改 buckyos/AGENTS.md：持续维护规则

本节是实施任务的一部分，落点为仓库根 [AGENTS.md](../AGENTS.md)，不是只在本文写一条提醒。

- [x] **M01 增加“cmd_skills 联动维护”小节**：明确手册源码目录、发现索引、Jarvis 配置、机制入口与共用样例位置。将 skill 视为对应功能的交付物；修改用户可见 CLI/API/配置/运行路径/发布流程时，必须在同一变更中检查并更新受影响手册和示例。未受影响要说明依据，不能仅因代码编译通过而跳过。
- [x] **M02 写入影响矩阵**：至少覆盖下表；跨仓库实现变化也要检查 BuckyOS 分发的手册，指向当前源码/命令 schema，不复制过期接口说明。

| 实现变更 | 必须检查的手册/验证 |
| --- | --- |
| System-config、Control Panel、身份/RBAC、scheduler/node-daemon、日志诊断、路径及版本信息、容器内 Tool 挂载与 launcher | `buckyos_admin`；设置生效、权限失败、诊断与源码版本核验、容器内 `buckyos` 可用。 |
| websdk SDK API、`buckyos-agent` 的 `commands/` 范本与 `lib/` | 引用这些源码的手册（`aigc_*`、`buckyos_admin`、`opendan_agent_dev`）；范本仍可照抄运行，派生工具示例可执行。 |
| AICC 协议、provider 能力、媒体 CLI、Task/结果/下载和消息附件 | 三份 `aigc_*`；真实生成、长任务恢复、交付可访问性。 |
| xllm 参数、模型路由、工具开关、上下文/behavior 协议 | `xllm`；示例调用、结构化结果、上下文与执行边界。 |
| SDK/Tool、PIKG/AppDoc、devkit、Installer/Repo/BNS、系统/镜像发布 | `buckyos_dev_release`、`buckyos_app_dev` 及相关场景；构建、包验证、测试安装、发布回读。 |
| AIWorkspace schema/RPC、对象/资源、renderer、Wish/Block 执行器、模板格式 | `aiworkspace_template_dev`（暂缓，恢复 §7.2 后生效）；新建、导出导入、引用和运行验证。 |
| skill 安装 kRPC、PIKG 的 `agent_skills` 子包、AgentRoot 的 `cmd_skills/` / `goal_skills/` / `tools/` 布局 | `opendan_agent_dev` 及所有内置手册的打包位置；安装、列表、选择、删除验证。 |
| AgentRootFS、Loader、Session/Behavior、cmd_skill 冻结/装配、PATH/helper、模板和扩展分发 | `opendan_agent_dev`、`ui_core` 及受影响专项手册；发现/选择/执行、冻结与升级验证。 |

- [x] **M03 加入完成条件**：提交说明列出受影响 skills、修改或无需修改的依据、验证命令与结果、尚未实测的 provider/平台。删除失效命令，不用兼容说明掩盖当前版本不可用；同步生成来源与分发产物验证，不能只改打包副本。
- [x] **M04 回归约定**：手册纯文字改动核对路径/命令/链接；CLI、协议、runtime 或打包变更运行相关用例和最小场景。真实生成/发布测试需要相应环境时如实标为未验证，不能用 mock 通过替代。
- [x] **M05 维护机制验证**：选一次真实 CLI/schema 变更，按新规则完成“找出受影响手册 → 更新示例 → 验证 → 记录结果”，确认规则能被日常开发执行；同步机制 TODO、开发指南与相关 README 中的状态。
- [x] **M06 写入工具实现原则**：把“访问 BuckyOS 的 Agent 工具优先用 TS 基于 websdk 实现、源码随 runtime 可读、Agent 可据此派生”写进 `doc/sdk/OpenDAN_Agent_Dev_Guide.md` §2 与 AGENTS.md 的 cmd_skills 小节，并给出 S04 的落点分工；目前它只写在 `src/tools/buckyos-agent/readme.md`，依据开发指南制定的计划容易漏掉。

## 9. 实施顺序与完成标准

| 阶段 | 范围 | 退出条件 |
| --- | --- | --- |
| T0 | S01–S04、I01–I08、M01–M04、M06 | 手册规范/索引和维护规则明确；意图分析能从已安装手册中选出多份并在 Work 中装配，冻结与 UI 无回归；经 PIKG 安装的 skill 可被列出和选中；容器内能以 Agent 身份运行并读取 `buckyos` 命令源码。 |
| T1 | A01–A07、X01–X04、G01–G07 | 系统管理、xllm、图片和语音各有真实任务闭环，命令缺失/失败有明确反馈。 |
| T2 | D01–D05、P01–P03、E01–E04（W01–W03 暂缓） | App 与 Agent 开发手册可复现；各类发布支持范围明确，所宣称的发布路径完成安装回验。 |
| T3 | G08–G11 | 先验收单段视频，再验收多素材制作、预算与失败恢复。 |
| 持续 | M05 及后续联动维护 | 每次功能变化同步 skill，发布前核对手册与镜像/包内实际工具。 |

- [ ] 每份 skill 都有入口、真实可执行命令、样例和验收证据，Agent 能完成用户任务；不能仅以 Markdown 文件存在判定完成。
- [ ] Work 接入、必要 CLI、Agent 包/工具分发、文档与 `AGENTS.md` 已同步；现有 UI 会话不被注入全部专项手册。
- [x] 实施后回写本 TODO：变更文件、运行环境/版本、测试命令、Session/task/产物证据、未完成项；未支持能力保持待办状态。

实施阶段按改动范围选择验证入口：`src/` 下的 `cargo test -p libopendan -- --test-threads=1`、`cargo test -p opendan --test loader -- --test-threads=1`、`cargo test -p agent_tool --lib xllm -- --test-threads=1`；SDK CLI 使用同级仓库 `cli/README.md` 的检查。AIGC 复用 `src/tools/buckyos-agent/lib/aicc_test.ts` 与 `test/jarvis_media_dv/`，其他方向复用各模块现有测试及上述真实场景。真实验收（生成、安装、发布）统一在 DV test 环境进行，使用其中已配置的模型与 provider；某项能力没有可用模型时跳过该项测试，在回写中标为未验证，不以 mock 代替。本次仅新增 TODO，不执行构建、在线生成、安装或发布。

## 10. 实施记录（2026-10-17）

### 10.1 落点

| 范围 | 文件 | 内容 |
| --- | --- | --- |
| 协议 | `lib_opendan/src/protocol/{behavior,config}.rs` | `prompt.cmd_skills: Vec<String>` → `frozen.cmd_skills: Vec<CmdSkill>`（按选择顺序）；`MAX_CMD_SKILLS = 8`；`SkillKind`（cmd/goal）、`SkillMeta`、`SkillInstallRecord`；`split_front_matter`（`---` 内 `key: value`）。开发期 breaking change，fixtures 与 schema 已重生成 |
| 冻结 | `state/behaviors.rs` | `freeze_config` 校验名称（合法、不重复、≤8）后逐个读手册、剥 front matter；未知名称报错并列出已安装的名字。`BehaviorCatalog` 加 `skills / install_skills / remove_skill`；Fs/Mem/Forwarding/Krpc 四个实现 |
| skill 存储 | `lib_opendan/src/skills.rs`（新） | 列表（`origin = installed / package / local`，`tools`）、安装（只收 `cmd_skills/<n>.md`、`goal_skills/<n>.md`、`tools/<cmd_skill>/…`；同名非本包拒绝，`replace` 替换；同包再装即更新；记录 `.meta/skills.json`：包、版本、内容 digest、`pikg_digest`、时间、安装者）、删除（只删已安装的）、`read_pikg`（读 `app_type = agent_skills` 的 PIKG，校验 payload sha256/size，拒绝非普通文件与越界路径） |
| kRPC | `state/krpc.rs` | `skills.list / install / remove`（install/remove 记为写）；opendan 服务经门面直接提供，`/kapi/<agent_id>` 经 NodeGateway 可达 |
| 装配 | `runner/assembler.rs` | `commands_section`：Work 等非 UI Session 在 `## objective` 后加 `## Commands`（段首一句“可选捷径”，每份 `### <name>`）；UI 一份时保持原样 |
| PATH | `runtime/mod.rs`、`runner/drive.rs` | `skill_tool_dirs`：Session 冻结的 cmd_skill 的 `tools/<n>/bin` 排在 `<agent_root>/tools` 前（绑定探测与执行环境两处） |
| 选择与传播 | `api.rs`、`todos.rs`、`template.rs`、`bin/xagent.rs`、`agent_tool/src/todo_tools.rs` | `SubSessionSpec.cmd_skills: Option<Vec<String>>`：显式即用；未指定时父 Work 继承其冻结选择（过滤掉已删除的），父 UI/Goal 用模板默认（Jarvis 为空）；有 cmd_skills 即在创建时冻结（错误当场返回）。`create-worksession --cmd-skill`（可重复或逗号分隔）、`xagent new --cmd-skill`、`skill list [--kind] [--json] / install <x.pikg|dir> [--replace] / remove <n> [--kind]`。todo `--skill` 作为探索子 Work 的 cmd_skills（`todo add/explore` 时校验已安装），上限 3→8；`agent.toml` 改为 `default_cmd_skills = [...]` |
| Loader | `opendan/src/{rootfs,loader,main}.rs` | 骨架加 `goal_skills/`；模板同步跳过 `.meta/skills.json` 记录的已安装文件；`BUCKYOS_AGENT_SKILLS_PKG_DIR`（模板的 `agent_skills` 子包）随模板同步（pinned 时不同步） |
| Jarvis | `agent/agent.toml`、`behaviors/{analyze_intent,goal,plan}.toml`、`cmd_skills/` | 意图分析探查 `skill list`，创建带 `--cmd-skill`，结果 schema `work.cmd_skills`（strict 全必填）；Goal 与 PLAN 的创建说明；`ui_core` 加 front matter 与 `skill` 命令；新增 8 份手册（见 10.2）；删除无消费者的 `aicc_image_cli.inc` |
| I07 | `kernel/node_daemon/src/app_loader.rs`、`test_app_loader.rs`、`publish/aios/{buckyos,aicc-tool,Dockerfile}`、`build_aios`、`src/tools/buckyos-agent/{deno.json,readme.md}`、`doc/arch/11_env_contract.md`、`doc/path_usage.md` | Agent 容器只读挂载宿主 `libexec/buckyos-tool/{cli,dist,package.json,distribution.json}`（缺项跳过并告警，不建目录，不挂 `runtime/`），注入 `BUCKYOS_TOOL_ENDPOINT=http://<host gateway>:3180`；模板带 `agent_skills` 子包时挂 `/mnt/buckyos/agent_skills` 并设 `BUCKYOS_AGENT_SKILLS_PKG_DIR`。镜像 launcher 用镜像 Deno、挂载缺失明确报错、校验 `protocol_version == "1"` 与 Deno 主版本；`buckyos-agent` 的 `buckyos` 映射到挂载的 `dist/node.mjs`，镜像不再 vendor SDK，构建期检查用 BuildKit bind mount 的临时 SDK |
| websdk | `cli/modules/{pikg,pikg_protocol,pikg_archive,skill,app,system,log,agent,ai}.ts`、`cli/core/app.ts`、`cli/runtime/host.ts`、tests、`doc/modules/{pikg,app,system,task}.md` | `pikg init --kind agent-skills`；`skill list|install|remove --agent`（install 用 `inspectPikg` 校验）；`app publish`（只是 Repo candidate，输出 `not-signed / not-published`）；A02 缺口：`system info`（`system status` 带版本）、`log services`、`--level warn → warning`、`agent list|get|profile-get|profile-set`、`ai providers|routing|usage`（只读） |
| control_panel | `app_installer.rs`、`sys_log_mgr.rs` | Agent 发布打包 `<local_dir>/agent_skills` 子包（`agent_tools` 仍拒绝）；日志级别过滤用 `normalize_log_level` |
| AIGC 工具 | `src/tools/buckyos-agent/{aicc-tool.ts,commands/*,lib/*}` | 新命令 `resume_aicc_task`（按 AICC task id 续查/等待/下载，exit 4 = 仍在运行）；等待超时返回 task id（exit 4）；`detail.files[].obj_id`；`ai_provider models`；裸 PCM（`audio/l16`）存为 WAV（`wrapRawPcm`） |
| xllm | `agent_tool/src/{run_local_llm,xllm}.rs` | `--json` 时关闭 web_search（AICC 给 llm.chat 开了联网搜索，OpenAI JSON mode 不兼容）；恢复提示改为 `agent_tool xllm --resume --run <id> --runs-dir <dir>` |
| 文档 | `AGENTS.md`（“cmd_skills 联动维护”：位置、工具原则、影响矩阵、完成条件、回归）、`doc/sdk/OpenDAN_Agent_Dev_Guide.md` §2.3/§2.4/§6.2/§7、`src/frame/opendan/README.md`（Skills）、Agent State Protocol、xAgent.md、`doc/sdk/app-dev-quickstart.md`（D01 陈旧命令） | |

### 10.2 手册（`src/apps/jarvis_agent/agent/cmd_skills/`）

模板见 `/home/lzc/project/cmd_skills_dv_runs/MANUAL_TEMPLATE.md`（front matter 三项、正文只用 `####`、60–140 行、英文、捷径口吻）。

| 手册 | 行数 | 真实核验（DV test.buckyos.io） |
| --- | --- | --- |
| `buckyos_admin` | 140 | 宿主设备身份跑通 `--version --verbose`、`system status/info`、`system-config get/list/set-file`、`log query/tail/export/services`、`diagnostic collect/export`（已脱敏）、`app/task/agent/ai` 各读命令；可恢复设置：desktop `iconSize` 改回、xiaobai 显示名改回；错误形态 3/4/5/6 |
| `aigc_image` / `aigc_audio` / `aigc_video` | 73 / 59 / 76 | 以 devtest 身份 5 次付费调用约 0.215 USD：文生图、按对象 id 编辑、TTS（含 `--format wav` 被拒）、4 s 720p 视频（超时 → `resume_aicc_task` 取回，同 key 重跑不重复提交）；ffmpeg 合成配方实跑 |
| `xllm` | ~120 | X04 各例（文本、schema、文件/stdin、受限工具、未知模型、无效输出、超时续跑、中断、禁止的复用）全部符合预期；以宿主/devtest 身份 |
| `buckyos_dev_release` / `buckyos_app_dev` / `opendan_agent_dev` | 75 / 72 / 107 | static-web、Docker、script 三类 App 经宿主 CLI 安装与升级（Docker 升级后数据保留）；`app publish` 得到 `repo_stored_candidate`；agent-skills PIKG 构建 |

原始证据与验收计划：`/home/lzc/project/cmd_skills_dv_runs/{buckyos_admin,aigc,xllm,dev}/{evidence,acceptance}.md`，缺口表 `buckyos_admin/gaps.md`、`gap_commands.md`。

### 10.3 验证

- `cargo test -p libopendan -p opendan --no-fail-fast -- --test-threads=1` 全过（新增 `xagent_lib::work_sessions_take_the_cmd_skills_chosen_at_creation`、`xagent_cli::skills_are_installed_listed_chosen_and_removed`、`skills::tests`、`rootfs::tests::package_sync_leaves_installed_skills_alone`；loader 断言 front matter 不进提示词）；`cargo test -p agent_tool --lib` 262 过；`cargo test -p node_daemon` 109 过；`cargo test -p control_panel` 135 过；buckyos-agent `deno test lib/` 18 过、`deno check aicc-tool.ts`；websdk `pnpm run check:cli`、`test:cli` 106 过。
- fixtures/schema：`cargo run -p libopendan --example fixtures -- /home/lzc/project/buckyos/doc/opendan/protocol/fixtures`。
- Rust `read_pikg` 读 websdk 打出的样例 `test/skills_pikg_sample/dapp_dist/hello-skills-0.1.0.pikg`：`pikg_digest` 一致，命令保留可执行位。
- 镜像：`./build_aios --local-test` 成功（`local/aios-test:latest-amd64`，含 `buckyos` launcher 与 `resume_aicc_task`）；手动挂载运行时 `buckyos --version --verbose` 报镜像 Deno 2.9.2、`system status` ok、挂载缺失与版本不兼容报错可读、cwd 下另有 `deno.json` 不受影响。
- 构建产物已就绪：`uv run buckyos-build.py -s node_daemon control_panel jarvis_agent`（Jarvis PIKG 含 9 份手册，SDK/Tool 含新模块）。

### 10.4 未完成 / 未验证

- **部署**：`uv run start.py`（更新安装并重启，重建 Agent 容器、触发 Jarvis 模板升级）被权限策略拦截，需用户执行。部署前 xiaobai 容器仍是旧镜像，没有 `skills.*`、容器内 `buckyos` 与新手册。
- 部署后待跑（计划见各 `acceptance.md`）：I05 容器内命令可用性；I07 以 Agent 注入 token 的 RBAC；I08 端到端（`buckyos skill install --agent` → `skill list` → Work 选中 → 删除后不可选、旧 Work 不变）；A07；G05/G07/G11（含 V1 多素材）；X04 经 Agent；P03/D05/E04 经 Agent。所有手册的“Agent 身份”结论目前来自代码与 RBAC 阅读。
- 已知缺陷（未修）：CLI 密码登录目标 `buckycli@<user>` 未登记 → `AppAccessDenied`（只能用 control-panel 登录绕过）；`system.dev_mode.get` 非公开导致开发者模式检测恒失败；`app status <短名>` 失败；`app uninstall --data delete` 不删数据；static-web 升级后网关仍服务旧目录；`app install` 默认 30 s 超时看似失败；`config set --profile-name` 新建失败；AICC 拼错的逻辑模型名静默回退默认模型；`extend_video` 无 continuation handle；镜像缺 CJK 字体（硬字幕中文）；xllm B3（失败时留下空 run 目录）、B4（会话 run 的 result 误报）、B5（`--resume` 静默忽略 `--model/--tools`）、B6（独立 xllm 不校验 schema）。
- 未实现：权威发布（Owner 签名、BNS/App DID 更新与回读，后端无路径）；A02 其余缺口（`user.*`、agent 创建/删除/模板、zone/网络/网关/容器、dev mode/locale/desktop、AI provider 写、system-config delete/create/CAS、按服务健康、容器日志）；`compose_video` TS 命令（launcher 无 `--allow-run`，以 ffmpeg 配方代替）。
- 已知风险：control-panel 把 Agent 的 App token 当作其 owner（同 user_type），管理员 owner 的 Agent 能调管理员 RPC；Agent 可自行安装 skill（按“先能力后安全”不加审批）。
- M05：本轮已按新规则执行两次（websdk 缺口命令 → 同步改 `buckyos_admin`；TTS 输出改 WAV → 同步改 `aigc_audio`），机制可日常执行；后续随功能变化继续。
- 子 agent 测试误在 DV 建了两个空目录（`/opt/buckyos/data/home/alice/.local/share/jarvis-agent.example`、`/opt/buckyos/logs/apps/56b9f1dc…`），删除被权限拦截，留待用户决定。
