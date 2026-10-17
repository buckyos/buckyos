# OpenDAN 内置 cmd_skills 编写与整合 TODO

日期：2026-10-17

状态：待实施。本次只记录开发计划；下列 skill 名称为建议名，新增命令、运行时接入和 `AGENTS.md` 修改均未实施。

目标：基于已有 cmd_skills 机制，让 Agent 能完成 BuckyOS 系统设置与诊断、图片/语音/视频生成、xllm 调用，以及 AIWorkspace 模板、传统 App、Agent 扩展的开发和发布。skills 是随实现维护的产品内容，不能交付一版手册后任其过期。

关联：[cmd_skill 机制 TODO](opendan-cmd-skill-todo.md) 负责机制后续设计；本文负责首批能力手册及必要的接入闭环。扩展原则参考 [OpenDAN Agent 开发指南](../doc/sdk/OpenDAN_Agent_Dev_Guide.md)：复用 CLI、按需发现，不为每项业务新增模型 tool/action。

## 1. 当前基础与实际缺口

以下按当前工作树源码核对，表示已有实现入口，不表示已完成真实环境验收。路径默认相对 `buckyos/`；`../buckyos-websdk/`、`../buckyos-devkit/` 为同级仓库。

| 方面 | 当前实现 | 本计划需要补齐 |
| --- | --- | --- |
| 手册加载 | `src/frame/lib_opendan/src/state/behaviors.rs` 从 AgentRootFS 的 `cmd_skills/<name>.md` 读取；`src/frame/lib_opendan/src/template.rs` 将 `default_cmd_skill` 写入配置，冻结为 `{name, text}`；未知名称报配置错误。 | 专项手册、名称/用途索引、选择与错误反馈。当前字段是单份 skill。 |
| Prompt 接入 | `src/frame/lib_opendan/src/runner/assembler.rs` 的 `ui_sections` 将冻结手册渲染为 `## Commands`。Jarvis 只有 `cmd_skills/ui_core.md`，`agent.toml` 的 ui/group 使用它。 | Work 的手册装配；`analyze_intent` 到 Work 创建的选择链路。只新增 Markdown 不会自动让 Work 使用它。 |
| 创建 Work | `src/frame/lib_opendan/src/bin/xagent.rs` 的 `create-worksession` 及 `src/frame/lib_opendan/src/api.rs` 的 `SubSessionSpec` 尚无 cmd_skill 选择参数。 | 补字段、CLI、状态协议、模板选择与测试；不能把旧 TODO 中设想的 `--skill` 写成现有命令。 |
| 命令环境 | `src/frame/lib_opendan/src/runtime/bin_overlay.rs` 已有 Session helper，`standard_helpers` 只包装 `llm_understand_media`；另有现行 PATH/tool-plan 机制。 | 逐项核验容器/宿主 runtime 的命令、依赖、身份和产物路径。尚无按 skill 自动挂载 `tools/<skill>/bin` 的能力。 |
| 系统管理 | `../buckyos-websdk/cli/modules/` 已有 `system`、`system_config`、`log`、`diagnostic`、`app`、`task` 模块；Control Panel 有对应服务入口。 | 将实际命令组织为设置/诊断流程；补未覆盖的管理动作，验证 Agent 身份能否调用。 |
| AIGC | `src/tools/buckyos-agent/commands/` 已有图像、音频、视频 CLI；`build_aios` 分发 `aicc-tool` 和命令入口。已有 `behaviors/aicc_image_cli.inc` 可复用。 | 整理成专项 cmd_skills，核对参数、provider 能力、异步状态与媒体交付；旧 inc 不能原样视为当前已加载手册。 |
| xllm | `src/frame/agent_tool_cli_dev/src/lib.rs` 提供 `agent_tool xllm`，核心在 `src/frame/agent_tool/src/xllm.rs`。 | 独立使用手册、执行环境与边界说明；不能假定裸 `xllm` 已在 Session PATH。 |
| 开发与发布 | 已有 AIWorkspace RPC/程序宿主、PIKG 工具、Jarvis 模板和系统构建脚本。App CLI 尚无 `app publish`；后台存在 `app.publish` RPC。 | 开发场景手册、可执行样例，以及发布链路的 CLI 缺口。后台写入 Repo/NamedStore candidate 不等于完成 App DID 权威发布。 |

`src/frame/agent_tool/src/skills_mgr.rs` 的 `skills/ + skill.yaml + SKILL.md`、Goal/Todo 的 skill 字段不是本文的 `prompt.cmd_skill`。本计划沿用 cmd_skills，不另建一套 skill manager，也不把这些字段视为已打通。

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
| `aiworkspace_template_dev.md` | AIWorkspace 模板设计、生成、预览、导入导出与分发。 | 可在新 Workspace 复用的模板和示例。 |
| `buckyos_app_dev.md` | static-web/script/docker 等传统 App 开发。 | 构建、PIKG、测试安装和升级样例。 |
| `opendan_agent_dev.md` | Agent 模板、behavior、cmd_skills、CLI 工具扩展。 | 可创建实例并运行的模板/扩展及验证报告。 |

- [ ] **S01 手册模板**：每份手册说明适用意图、前置条件、真实命令发现方式、最短成功示例、输入/输出、失败与恢复方式、验收方法、实现来源、最近核对版本/日期。版本记录用于核验当前能力，不要求新增旧版本兼容层。
- [ ] **S02 可发现性**：提供简短的 skill 名称、用途、前置条件索引，供意图分析按需读取；具体命令参数通过 `--help` 或 `buckyos command describe <module> <verb>` 获取。索引不能把未完成能力写成可用能力。
- [ ] **S03 内容收敛**：`ui_core` 保持常用会话命令；专项长手册进入 Work。共用开发发布步骤只维护一处，由场景手册明确引用；保证引用文件随包分发且能在 runtime 读取。
- [ ] **S04 工具边界**：复用已有 CLI；确有缺口才补命令/薄包装。命令能被发现不代表当前身份有权限或 provider 支持，手册应说明实际拒绝/不支持结果的处理。

## 3. 机制与运行环境接入（先行）

- [ ] **I01 Work 选择链路**：在机制 TODO 中落实最小选择方案，打通“意图分析发现手册 → 创建 Work 传入名称 → 校验 → 冻结 → Work prompt 装配”。同步 `SubSessionSpec`、CLI、协议/schema、Jarvis 提示词与文档。明确无匹配时的默认策略，未知名称明确失败。
- [ ] **I02 单份与组合**：首版复用单份 `cmd_skill`。跨能力任务通过主手册引用其他手册按需读取，明确路径与读取入口；只有验证单份入口不足时才扩展组合协议，不先发明动态依赖加载框架。
- [ ] **I03 任务传播**：明确 PLAN/DO 子 Work 如何选择或继承所需手册；Goal 发起 Work 时也走同一选择路径。不得默认父 UI 的手册、Goal/Todo 的 skill 字段会自动注入子 Session。
- [ ] **I04 冻结语义**：新建 Session 使用新手册，运行中 Session 保持冻结正文；若增加显式切换，需同步修订号和重新冻结规则。按需读取的补充手册要能识别版本不一致，不能静默替换已冻结正文。
- [ ] **I05 runtime 与打包**：核验系统 `buckyos`、项目 SDK CLI、`agent_tool xllm`、AIGC 命令、AIWorkspace 入口在容器/宿主环境的可用性；缺失时给出准确前置条件。同步 Jarvis 包与 `build_aios`/ExtTool 等实际分发来源，避免只改 rootfs 或生成产物。
- [ ] **I06 最小回归**：扩展 `src/frame/lib_opendan/tests/{xagent_cli,xagent_lib}.rs`、`src/frame/opendan/tests/loader.rs` 和 `src/frame/lib_opendan/src/runtime/bin_overlay.rs` 的相关用例，覆盖名称选择、未知名称、Work 实际装配、子任务、冻结稳定性、命令缺失；保留现有 UI 行为。

## 4. BuckyOS 系统管理与诊断

依据：`../buckyos-websdk/cli/modules/{system,system_config,log,diagnostic,app,task}.ts`、`src/frame/control_panel/src/`、`src/check.py`、`doc/arch/system_config_reference.md`、`doc/path_usage.md`。

- [ ] **A01 环境识别**：先识别目标 Zone、调用身份、runtime、系统/Tool/SDK 版本与实际可用命令。区分系统分发 `$BUCKYOS_ROOT/bin/buckyos` 和项目分发；不能把容器路径当宿主路径，也不能将 `check.py` 的“待激活”当成运行正常。
- [ ] **A02 设置流程**：盘点用户/Agent、设备、应用、网络、存储、AI provider 等常见设置的真实管理入口，列明已支持范围和缺口。优先用领域 API/CLI；需改 system-config 时区分用户 `Settings` 和系统生成 `Config`，不能把直接改派生配置当通用方案。
- [ ] **A03 变更闭环**：读取现状 → 按用户目标形成最小变更 → 执行 → 检查调度/服务实际生效 → 返回结果与必要恢复方法。遵守现有授权、RBAC 和 CLI 确认语义；已授权范围内不增加逐步反复确认。
- [ ] **A04 诊断流程**：复用 `buckyos system status`、`system-config get/list`、`log query/tail/export`、`diagnostic collect/export`、App/Task 状态；按时间线整理现象、证据、假设、验证和建议。远程 CLI 不可用时，说明哪些本机检查需要宿主访问，不伪造已取得的日志。
- [ ] **A05 对应版本源码分析**：优先使用匹配运行版本的本地源码，否则下载对应发行物的只读源码快照到工作目录。记录 BuckyOS 版本、build ID/源码 revision、Tool/SDK 版本和来源；`src/tools/prepare_sdk_tool_distribution.py` 的 `build_id` 已含构建提交信息，可作为线索，但不能据此假定整个运行系统及本地未提交改动完全一致。
- [ ] **A06 来源校验与降级**：建立发行物到源码快照的对应方式并验证摘要；无法确定运行源码时明确证据缺口，不能拿最新 `main` 直接断言旧版本行为。源码快照只用于诊断；开发依赖继续遵守 `AGENTS.md` 的显式 `main` 规则，不修改依赖来“匹配”故障版本。
- [ ] **A07 验收**：完成一项可恢复设置并验证实际效果；对一个可复现故障给出日志与源码位置支撑的结论；覆盖权限不足、服务不可达、源码版本不匹配。诊断输出不得包含私钥/token。

## 5. AIGC：图片、语音、视频

依据：`src/tools/buckyos-agent/aicc-tool.ts`、`src/tools/buckyos-agent/commands/`、`src/tools/buckyos-agent/lib/{aicc,io,result}.ts`、`src/frame/aicc/src/`、`src/kernel/buckyos-api/src/aicc_client.rs`。`llm_understand_media` 是媒体理解，不替代生成命令。

### 5.1 共用接入

- [ ] **G01 迁移手册**：核对 `src/apps/jarvis_agent/agent/behaviors/aicc_image_cli.inc` 和工具 README，将有效说明拆入三份手册；检查旧 include 是否仍有消费者，避免重复维护。旧 `<attachment .../>` 示例须按当前产物/消息协议改写。
- [ ] **G02 能力与材料**：默认沿用逻辑模型和 AICC 路由，按实际能力使用尺寸、时长、格式等参数；不将 provider 健康检查设为每次调用的前置步骤。区分“有 CLI 入口”和“当前 provider 能执行”。保留输入的完整对象 ID/类型前缀；需要本地文件时复用 `materialize_resource`，不拿相似旧文件替代素材。
- [ ] **G03 长任务与交付**：区分 shell 后台 task 与 AICC 生成 task，记录两层 ID；超时/断连不等于服务端失败。CLI 已有等待与幂等/预算选项，但按生成 task ID 续查并恢复下载的媒体命令仍需补齐。避免重复提交，保留中间产物。当前 stdout 始终是 `AgentToolResult`，不是加 `--json` 才结构化；检查成功状态与 `detail.files` 的实际文件，再通过 Work `report.artifacts` 及当前消息附件流程交付并验证可打开/播放。

### 5.2 图片

- [ ] **G04 图片手册**：覆盖 `gen_image`、`edit_image`、`inpaint_image`、`upscale_image`、`remove_bg`；说明文本、参考图、mask、输出路径和生成后检查。模型不支持的编辑方式不写成通用保证。
- [ ] **G05 图片验收**：至少一次文生图和一次有参考素材的编辑；核对文件、格式、尺寸及内容；覆盖错误输入、provider 拒绝、结果下载失败。

### 5.3 语音与音频

- [ ] **G06 音频手册**：以 `text_to_speech` 为最小闭环，核对音色、语言、格式、长文本等实际参数及限制；按能力补 `gen_music`、`enhance_audio`。语音识别属于辅助能力，不能代替语音生成验收。
- [ ] **G07 音频验收**：生成可播放语音，验证文本内容、格式与时长；覆盖不支持的音色/语言、过长输入和 provider 失败，明确是否支持分段生成与拼接。

### 5.4 视频（单独分期）

- [ ] **G08 V0 单段生成**：先完成 `gen_video` / `img2video` 的真实 provider 闭环；核对任务等待、下载、超时续查和播放。`video2video`、`extend_video`、`upscale_video` 按实际能力逐项接入；续写要验证 continuation handle 的有效/过期/不支持情形，末帧续做加拼接只能作为明确说明的替代方案，不能承诺原生无缝续写。
- [ ] **G09 V1 多素材制作**：流程覆盖需求与时长 → 分镜/脚本 → 图片/语音素材 → 分段生成 → 合成/字幕/音画同步 → 检查 → 交付。先核实 runtime 现有媒体工具；缺少合成/检查能力时列为独立工具工作，新增依赖按仓库规则处理。
- [ ] **G10 成本与恢复**：记录分镜、素材、生成参数、task ID、已完成片段和预算边界；失败只重做必要片段。让等待/恢复沿用 Session/Task 机制，不能仅靠长时间阻塞 shell 或无限轮询。
- [ ] **G11 视频验收**：V0 验证单段可播放、尺寸/时长符合目标；V1 验证至少两段画面与配音合成，检查音画和字幕（若有）、保留中间素材，并覆盖一次失败恢复。只有 provider mock 时标为未完成真实生成验收。

## 6. 使用 xllm

依据：`src/frame/agent_tool_cli_dev/src/lib.rs`、`src/frame/agent_tool/src/xllm.rs`、`doc/llm_context/`。推荐先使用已存在的 `agent_tool xllm`，若提供快捷入口，应纳入正式包装与分发。

- [ ] **X01 调用说明**：核对当前 `--help` 与参数解析，给出文本/文件/管道输入、模型选择、上下文目录和输出保存示例；区分 `--json` 的模型结果约束与 `--format json` 的 CLI 结果信封。覆盖现有 `list/status/result`、`--resume --run`，说明恢复沿用保存配置、如何重导出结果及各退出码，避免可恢复暂停被当成重新调用。
- [ ] **X02 执行边界**：说明独立 xllm 默认不启用工具，以及启用工具、behavior/function_call 模式的真实配置限制；核对模型/provider 配置、cwd 和身份。不能把 xllm 当成接管活跃 xagent Session 的办法。
- [ ] **X03 选择与约束**：一次性转换/分析可用 xllm；需要持久任务、PLAN/DO 或用户交互的工作使用既有 Work Session。配置调用预算/超时，避免 Agent 递归套用 xllm；正确处理失败、pending 和恢复边界。
- [ ] **X04 验收**：文本分析、结构化结果、文件输入与受限工具执行各一例；覆盖模型不可用、无效输出、超时与不允许的上下文复用，说明命令成功不等于任务验收成功。

## 7. BuckyOS 开发和发布

### 7.1 共用开发发布闭环

- [ ] **D01 环境与构建**：手册引用当前 `AGENTS.md`、`src/buckyos-build.py`、`src/bucky_project.yaml`、devkit 和 SDK CLI。明确工作目录、平台依赖、系统构建与 App 自有构建的分工，复用 `start.py`/`check.py` 和镜像本地迭代方法。同步修正文档中的陈旧命令，尤其是 `doc/sdk/app-dev-quickstart.md`。
- [ ] **D02 打包与安装**：复用 `buckyos pikg init/build/pack/info` 和 `app fetch/install/upgrade/status`；以当前 schema 核对参数。区分首次安装计划、本地新版本安装升级、Catalog 升级，保留版本、digest、任务状态与验证结果。
- [ ] **D03 发布补齐**：盘点 `src/frame/control_panel/src/app_installer.rs` 的 `app.publish` 后台能力，补齐适合 Agent 的 CLI。分别验证 Owner 签名、PIKG/资源托管、BNS/App DID 权威发布及回读；上传 candidate 或本机安装成功均不能视为已发布。
- [ ] **D04 系统发行**：区分 BuckyOS 系统/SDK/工具镜像发行与 App/模板发行，按 `publish/`、`.github/workflows/`、`build_aios` 和 release manifest 盘点构建、测试、产物上传、版本更新入口；手册不臆造统一发布命令。沿用实际发布目标的授权流程。
- [ ] **D05 发布验收**：隔离测试目标上可下载且摘要一致，签名/权威版本可回读，新环境能安装和运行；记录失败恢复/撤回能力的实际边界，不承诺尚不存在的自动回滚。

### 7.2 AIWorkspace 模板开发

依据：`src/frame/desktop/src/app/aiworkspace/api/{sample,demos,client}.ts`、`src/frame/aiworkspace/server/src/lib.rs`、`src/frame/aiworkspace/aiws/`。当前示例通过创建 Workspace 再提交内容构造，服务有 `ws.begin_import` / `ws.import`、`doc.export`；程序宿主 `aiws` 不等于现成模板发行 CLI。

- [ ] **W01 模板规范**：定义模板的参数、元数据、Record/Surface/Cell/Block/Wish 等对象、资源依赖及版本；区分可复用模板与某用户 Workspace 的数据快照，明确复制/重新绑定规则。
- [ ] **W02 制作入口**：复用已有 schema、RPC、示例和 SDK，补生成、校验、预览、导出、从模板新建 Workspace 的 CLI/脚本闭环；模板目录发现、打包与发布格式作为明确交付项。
- [ ] **W03 验收**：至少一个含真实资源和可执行内容的模板，可导出并在新 Workspace 导入；核对引用、渲染、权限与运行结果，覆盖缺失资源/无效 schema；默认不覆盖用户已有 Workspace。

### 7.3 传统 App 开发

依据：`../buckyos-websdk/cli/modules/{pikg,app}.ts`、`test/app_installer_test/pikg_samples/`。当前 `pikg init` 支持 `static-web` / `script` / `docker`；PIKG 消费构建输出，不替代 App 自身构建。

- [ ] **P01 开发手册**：覆盖脚手架选择、SDK/runtime 身份接入、AppDoc/权限/服务与数据目录、构建输出、PIKG 配置和本地调试。每种运行形态标明前置条件，避免把宿主工具带进应用运行包。
- [ ] **P02 安装与升级**：复用现有静态 Web 和 Docker 样例，增加新版本、测试 Zone 安装、日志与状态检查、升级后的持久数据验证；发布调用共用手册。
- [ ] **P03 验收**：至少静态 Web 与 Docker 两条实际闭环，页面/API 可用，首次安装与升版本升级均有证据；script 分支有可执行最小样例和限制说明。

### 7.4 Agent 开发与扩展

依据：`src/apps/jarvis_agent/{agent,dapp_meta}/`、`src/frame/control_panel/src/agent_mgr.rs`、`src/debug_jarvis.sh`、OpenDAN Agent 开发指南。

- [ ] **E01 模板开发**：从最小 AgentRootFS 出发，讲清 `agent.toml`、behavior、cmd_skills、CLI 工具、Session/Workspace、产物报告和调试。使用 Jarvis 的 `agent` 子包结构，不能写不存在的 `pikg init --kind agent`。
- [ ] **E02 创建和运行**：补齐模板打包、测试 Zone 登记、从模板创建独立 Agent、运行及更新验证。明确 AgentId、AppId、AppInstance binding 的关系，模板不携带某个实例的私钥或状态。
- [ ] **E03 扩展分发**：覆盖增加/修改 behavior、cmd_skill、工具脚本及所需依赖的开发方式；核对工具镜像/PATH。后台发布当前拒绝 `agent_skills` / `agent_tools`，扩展的打包、安装、更新与移除属于待补契约，不能仅靠文档声称可发布。
- [ ] **E04 验收**：一个模板可创建新 Agent，执行带专项 cmd_skill 的任务并产出报告；一个扩展有安装/更新后的真实调用验证，检查旧冻结 Session 与新 Session 的版本行为及实例数据保留。

## 8. 必须修改 buckyos/AGENTS.md：持续维护规则

本节是实施任务的一部分，落点为仓库根 [AGENTS.md](../AGENTS.md)，不是只在本文写一条提醒。

- [ ] **M01 增加“cmd_skills 联动维护”小节**：明确手册源码目录、发现索引、Jarvis 配置、机制入口与共用样例位置。将 skill 视为对应功能的交付物；修改用户可见 CLI/API/配置/运行路径/发布流程时，必须在同一变更中检查并更新受影响手册和示例。未受影响要说明依据，不能仅因代码编译通过而跳过。
- [ ] **M02 写入影响矩阵**：至少覆盖下表；跨仓库实现变化也要检查 BuckyOS 分发的手册，指向当前源码/命令 schema，不复制过期接口说明。

| 实现变更 | 必须检查的手册/验证 |
| --- | --- |
| System-config、Control Panel、身份/RBAC、scheduler/node-daemon、日志诊断、路径及版本信息 | `buckyos_admin`；设置生效、权限失败、诊断与源码版本核验。 |
| AICC 协议、provider 能力、媒体 CLI、Task/结果/下载和消息附件 | 三份 `aigc_*`；真实生成、长任务恢复、交付可访问性。 |
| xllm 参数、模型路由、工具开关、上下文/behavior 协议 | `xllm`；示例调用、结构化结果、上下文与执行边界。 |
| SDK/Tool、PIKG/AppDoc、devkit、Installer/Repo/BNS、系统/镜像发布 | `buckyos_dev_release`、`buckyos_app_dev` 及相关场景；构建、包验证、测试安装、发布回读。 |
| AIWorkspace schema/RPC、对象/资源、renderer、Wish/Block 执行器、模板格式 | `aiworkspace_template_dev`；新建、导出导入、引用和运行验证。 |
| AgentRootFS、Loader、Session/Behavior、cmd_skill 冻结/装配、PATH/helper、模板和扩展分发 | `opendan_agent_dev`、`ui_core` 及受影响专项手册；发现/选择/执行、冻结与升级验证。 |

- [ ] **M03 加入完成条件**：提交说明列出受影响 skills、修改或无需修改的依据、验证命令与结果、尚未实测的 provider/平台。删除失效命令，不用兼容说明掩盖当前版本不可用；同步生成来源与分发产物验证，不能只改打包副本。
- [ ] **M04 回归约定**：手册纯文字改动核对路径/命令/链接；CLI、协议、runtime 或打包变更运行相关用例和最小场景。真实生成/发布测试需要相应环境时如实标为未验证，不能用 mock 通过替代。
- [ ] **M05 维护机制验证**：选一次真实 CLI/schema 变更，按新规则完成“找出受影响手册 → 更新示例 → 验证 → 记录结果”，确认规则能被日常开发执行；同步机制 TODO、开发指南与相关 README 中的状态。

## 9. 实施顺序与完成标准

| 阶段 | 范围 | 退出条件 |
| --- | --- | --- |
| T0 | S01–S04、I01–I06、M01–M04 | 手册规范/索引和维护规则明确；至少一个专项手册可由意图分析选中并在 Work 中执行，冻结与 UI 无回归。 |
| T1 | A01–A07、X01–X04、G01–G07 | 系统管理、xllm、图片和语音各有真实任务闭环，命令缺失/失败有明确反馈。 |
| T2 | D01–D05、W01–W03、P01–P03、E01–E04 | 三类开发手册可复现；各类发布支持范围明确，所宣称的发布路径完成安装回验。 |
| T3 | G08–G11 | 先验收单段视频，再验收多素材制作、预算与失败恢复。 |
| 持续 | M05 及后续联动维护 | 每次功能变化同步 skill，发布前核对手册与镜像/包内实际工具。 |

- [ ] 每份 skill 都有入口、真实可执行命令、样例和验收证据，Agent 能完成用户任务；不能仅以 Markdown 文件存在判定完成。
- [ ] Work 接入、必要 CLI、Agent 包/工具分发、文档与 `AGENTS.md` 已同步；现有 UI 会话不被注入全部专项手册。
- [ ] 实施后回写本 TODO：变更文件、运行环境/版本、测试命令、Session/task/产物证据、未完成项；未支持能力保持待办状态。

实施阶段按改动范围选择验证入口：`src/` 下的 `cargo test -p libopendan -- --test-threads=1`、`cargo test -p opendan --test loader -- --test-threads=1`、`cargo test -p agent_tool --lib xllm -- --test-threads=1`；SDK CLI 使用同级仓库 `cli/README.md` 的检查。AIGC 复用 `src/tools/buckyos-agent/lib/aicc_test.ts` 与 `test/jarvis_media_dv/`，其他方向复用各模块现有测试及上述真实场景。本次仅新增 TODO，不执行构建、在线生成、安装或发布。
