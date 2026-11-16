# AIWS AI Native 余下任务 TODO：浏览器循环改用 xllm、可扩展数据源、对象动作

状态：2026-11-15 立项；§6 的 Q1–Q14 owner 已于同日全部答复。2026-11-15/16 第一轮实施，记录在 §7；owner 2026-11-16 答复 §7.12 后做了第二轮（§8）：Agent 宿主里的同步任务、托管执行改为发起者自己的 Agent、自增编号、xllm 连续错误上限可配置、`run` 动作 destructive。全部未提交。真实 Zone DV 见 §7.10、§8.5。B5-2 的版本保留另行立项（§8.1 第 2 项）。§8.6 的五项 owner 已按现状接受。

依据：

- [AI Native 内核升级 PRD](aiws-ai-native-kernel-upgrade-prd.md)（下称 PRD）中还没有完成的部分。
- [第一期实施 TODO](aiws-ai-native-phase1-todo.md) §7.5 列出的未做项。
- [许愿格浏览器优先 TODO](aiworkspace-wish-browser-first-todo.md) 的 M3（浏览器执行器接入 xllm TS）和 M5 的遗留。
- [许愿格交互升级 TODO](aiws-wish-card-interaction-todo.md)：2026-11-14 以后的交互和契约以它为准，包括不再确认候选、编码只保存宏、按运行记录回滚。
- [xllm TS 运行时 TODO](xllm-ts-runtime-todo.md) 的 Q5、Q10、Q11。

现状来自 2026-11-15 对 buckyos `c64998a6a` 和 buckyos-websdk `main` 的源码静态核对，没有运行测试。

日期：2026-11-15

## 0. 目标与范围

本单分四部分：

- **A. 浏览器里的分析、编码模型循环改用 xllm TS。** 本来以为第一期会顺带切过来，实际没有切：浏览器优先 M3 把 xllm TS 的 Worker 宿主（M6）列为前置，而 M6 没有做。
- **B. 第二期：可扩展数据源。** 对应 PRD §7，以及 §11.4 的第 1 步和第 3 步。
- **C. 共用契约的余项。** 对象入口的动作（PRD §8.1 xcall），以及 Jarvis 接入（PRD §8.3）。
- **D. 收尾。** 验收对账、未核对的风险、文档。

建议的顺序：

1. A 先做。它不依赖其他部分，服务端契约也不变。
2. C1 随时可以做。B-6 审批门已定为维持现状（§6 Q11）。
3. B 先做 B0（接口冻结）和 B1（执行设施核对），其余部分按 §6 的裁决推进。B 的不确定性最大，依赖 AIWS 之外的设施，所以不放在 A 前面。

不做：

- 下游自动运行与自动应用（PRD §10）。将来纳入时需要的东西列在 PRD §11.3。
- 收紧 run token（权限优化 TODO B-2）。owner 2026-11-12 裁决暂不收紧。
- 宏的受限语言（PRD §6.7 的方案 B）。
- 向外部系统写入的数据源（PRD §10）。
- 循环 Worker（xllm TS M6）。A 先在页面主线程接入，Worker 等 M6 做出来以后再挪（§6 Q1）。
- 许愿格的界面交互，由交互升级 TODO 负责。

## 1. 现状

### 1.1 已完成，不重做

| 项 | 落点 |
| --- | --- |
| 分析、编码、运行三阶段；宏 v3 加声明清单；`aiws.sql`、`ai.complete`、`llm.map`、`zone.fetch`；执行器侧的 `aiws/host.js`；托管的 `run-exec`、`run-program`、`run-start` | 第一期，buckyos `aee37a21a` |
| 服务不再调用模型、不再运行程序（浏览器优先 M1–M2） | 同上 |
| 阶段结束时由服务直接写入，删除 `proc.apply`，新增 `proc.rollback`，许愿格数据里有 `history` | 交互升级，`3cc5823a5`、`0be0e1b49` 及后续提交 |
| 权限系统第 0–5 步；运行写出的 HTML 只有在宏身份能管理时才算 trusted | `8b0e1c16a` 及后续 |
| xllm TS 运行时 M1–M5、MW、M7：`XllmTask.prepareHosted`、`XllmRun`、`createSiteRuntime`、`HOST_PROMPT_SECTIONS_NONE`、`exportRun` / `importRun`、`IndexedDbRunStore`、`AiccLlmClient`（逻辑模型走 `helper.llm_chat`，精确模型走 `chat.completions.create`，与 `chat.ts` 相同） | websdk `0502a60`；导出在 `src/xllm/index.ts` |
| 宿主上的 `buckyos` CLI 已有 `run-exec` | 本机 Zone `/opt/buckyos/libexec/buckyos-tool`，2026-11-15 更新 |

### 1.2 缺口

| 范围 | 现状 | 本单 |
| --- | --- | --- |
| 浏览器模型循环 | [localExecutor.ts](../src/frame/desktop/src/app/aiworkspace/executor/localExecutor.ts) 的 `loop()` 加 [chat.ts](../src/frame/desktop/src/app/aiworkspace/executor/chat.ts)，是手写的 function_call 循环。只有直接推理（`infer.ts`）和错误分类（`providerErrorFromRpc`）用了 `buckyos/xllm`。缺预算枚举与 `limit_reached`、快照与续跑、上下文压缩、运行记录 | A |
| 续跑 | 重新领取后，新的 attempt 从头跑循环 | A3 |
| 没有 AICC 时的前置提示 | 未做（浏览器优先 Q5） | A4 |
| 循环 Worker | websdk 没有 `src/xllm/worker/`，也没有 `./xllm/worker` 导出 | 不做（§6 Q1） |
| 数据源的状态 | `meta.data` 只有 `local`、`synced`、`url_query` 三种（[meta.rs](../src/frame/aiworkspace/core/src/meta.rs)） | B0 |
| 数据源定义 | 内核只认两个内置 schema：`buckyos.todo@1`、`buckyos.calendar@1`（[standard.rs](../src/frame/aiworkspace/core/src/standard.rs)） | B0 |
| 表格的记录身份 | 没有 `id` 列时用行号（[source.rs](../src/frame/aiworkspace/server/src/source.rs) 的 `table_from_rows`） | B5 |
| 用户编写的同步程序 | 没有执行路径。`auth_mode: hosted` 只是一个声明值，服务内的定时器跳过它 | B3、B6 |
| 对象动作 | 对象入口没有声明动作（核心 API L-3）。审批门（权限优化 B-6）2026-11-15 定为维持现状（§6 Q11） | C1 |
| Jarvis 接入 | AIWS Desktop 里没有把当前选择和位置交给 Jarvis 的入口 | C2 |
| 文档 | 《内核架构改进》§12.3 的 T3-9 仍写“未做”；PRD 正文（§3、§5.2、§5.7、§10、§10.1、§12）里还有“候选”“试运行直通”的旧说法，文首的修订已声明以交互升级为准 | D3 |

## 2. A：浏览器循环改用 xllm TS

### 2.1 映射

起点是 xllm TS TODO 的 Q10。服务下发的 brief 见 [stages.rs](../src/frame/aiworkspace/server/src/wish/stages.rs) 的 `host_open`，形状是 `kind: "model"`，带 `system`、`user`、`tools`、`model`、`iterations`、`budget`。

| 现在 | 改用 xllm TS 后 |
| --- | --- |
| `brief.system` | `prepareHosted` 的 hostSystem，提示词段落用 `HOST_PROMPT_SECTIONS_NONE`（AIWS 的提示词自带约束和回复契约） |
| `brief.user` | `XllmRun.start` 的 `input` |
| `brief.model` | `llm_context.model`，provider 为 `buckyos`（`AiccLlmClient`，用发起者的会话） |
| `budget.iterations` | `max_tool_iterations` |
| `brief.tools`，每个调用转给 `proc.tool` | `createSiteRuntime({ scene: "aiws.wish", tools })`：每个工具是一个页面工具，调用转给 `proc.tool` |
| `run_program` | 一个页面工具，走 `programHost`（MW 沙箱）。程序不进循环 |
| 取消、`RUN_REVOKED`、页面离开时停止 | `XllmInterrupter` |
| `proc.report.usage`（调用次数、token、最后一轮的模型） | 从运行结果或运行记录取，字段不变 |
| 失败码 `AI_*`、`SNAPSHOT_TOO_LARGE`、`EXECUTOR_FAILED` | 由 `classifyError` 加现有的子码表得出，失败码不变 |
| 停下时说的话（`summary`） | 最后一轮的回复文本 |
| 进度 `phase`（`waiting_model`、工具名、`running_program`） | 由运行事件更新心跳里的 `progress`。状态栏要的字段（工具调用次数、试运行次数、模型名、最后一条命令）都保留 |
| 开发覆盖 `localStorage['aiworkspace.dev'].llm.baseUrl` | `buckyos/xllm/dev` 的 `OpenAiLlmClient` |

以下几处不变：

- 服务的 `proc.*` 契约。服务不知道执行器换了实现，契约 fixture 不重录。
- 运行阶段（执行宏）。它本来就没有模型循环。
- 托管位置。分析和编码由 Agent 自己完成，不走这个循环，所以手册 `aiworkspace_run` 不受影响。提交说明里要写明这条依据。
- Rust 的测试侧执行器 `server/tests/common/wishkit.rs`。

### 2.2 A0 要定的细节

1. **迭代用满时的失败码。** 现在抛出 `aiworkspace.wish.error.iterations`，上报为 `EXECUTOR_FAILED`。建议继续用 `EXECUTOR_FAILED`，在 `detail` 里写明用满的是哪项预算。如果要新增码，就要同步 `schemas/error-codes.json`、i18n 和 `api/session.ts` 的错误文案。
2. **阶段时长。** brief 里没有阶段时长：浏览器优先 Q3 删掉了 `stage_timeout_secs`，现在的循环只靠迭代数兜底。建议不新增上限，保持现状。如果 xllm 必须给 `timeout_secs`，就取一个大值，并记进 §7。
3. **上下文压缩。** 分析和编码最多各有几十轮工具调用。建议按模型窗口开启 xllm 的压缩，并在 DV 里观察有没有触发。
4. **续跑能否接着原来的循环。** 要核对 `proc.claim` 给新 attempt 打开阶段时（`exec.rs` 的 `claim` → `stages::host_open`）是否保留上一个 attempt 的阶段状态，比如已暂存的试运行结果和工具轮数。保留，就用 `importRun` 接着跑；不保留，就从头跑（§6 Q2）。

### 2.3 任务

- [x] **A0** 定下 §2.2 的四项，写进 §7。
- [x] **A1** 新增 `executor/wishRuntime.ts`：由 `brief.tools` 构造 `createSiteRuntime`，每个工具转给 `procTool`，`run_program` 调 `LocalExecutor.program()`。
- [x] **A2** 把 `localExecutor.ts` 的 `loop()` 改为 `XllmTask.prepareHosted` → `XllmRun.start` → `execute`，用量、失败码、进度按 §2.1。删除 `chat.ts`，以及 `LocalExecutor` 构造参数里的 `ChatClient`；构造它的地方（`WishService`）同步改。
- [x] **A3** 续跑：运行记录存进 `IndexedDbRunStore`，xllm 的 run id 记在 `localStorage['aiworkspace.claims']` 旁边。重新领取时，按 A0 第 4 项决定是接着跑还是从头跑。私密窗口里 IndexedDB 不可用时从头跑。
- [x] **A4** 浏览器优先 Q5：当前身份没有可用的 AICC 时（guest、公共端），在卡片的执行按钮旁写明“本浏览器无法调用模型”；`hosted_exec.enabled` 时引导改用托管执行。按手机布局检查。i18n 补 en 和 zh。
- [x] **A5** 测试：
  - `tests/aiworkspace/wish.spec.ts` 的 14 条，以及“T3-4: the local executor”一例，在新循环下通过。`mock-llm.mjs` 的报文要与 `OpenAiLlmClient` 一致，先确认。
  - 新增用例：迭代用满时的失败码；刷新后续跑，不重复已经做过的工具调用（按 A3 的取法）；取消。
  - 完成条件：`grep -rn "chat.completions.create\|helper.llm_chat" src/frame/desktop/src/app/aiworkspace` 为空，模型调用全部经过 `buckyos/xllm`。
- [ ] **A6** 真实 Zone DV（2026-11-16 除手机真机外都已通过，见 §7.10）：
  - `tests/dv/aiworkspace.{macro,templates,mindmap}.spec.ts` 用真实模型通过。
  - AICC 用量全部记在发起者名下。
  - 关掉标签页后能续跑。
  - 用手机真机走一遍分析 → 编码 → 运行。这是第一期 T7-3 的最后一项。
- [x] **A7** 托管 DV（第一期 §7.5 的遗留）：让真实的 Jarvis 照 `aiworkspace_run` 手册领取托管运行，用 `run-exec` 跑宏。先确认容器里 `agent-tools` 的 `buckyos` 也有 `run-exec`；宿主上的 CLI 已经有。
- [x] **A8** 文档：
  - 浏览器优先 TODO：改 M3 的状态和前置条件，§7 加实施记录。
  - xllm TS TODO：在 Q10 注明 AIWS 已接入，并写明是在主线程接入的。
  - 服务 README 的“Execution modes”：如果写到手写循环，同步修改。

## 3. B：可扩展数据源（PRD 第二期）

需求在 PRD §7；待决事项在 §10、§10.1，以及 §11.5 的“留到第二期”。下面按 PRD §11.4 的顺序拆分。B2 以后的实现细节要等 §6 的裁决，本单只拆到任务粒度。

### B0 两期之间的接口（PRD §11.4 第 1 步）

- [x] **B0-1** 数据源定义：数据含义、schema、主键、时效要求（PRD §7.2、§7.7），以及可选的条数上限（只要前 N 条，§6 Q9）。在实体已有的 `schema_id` 上扩展，加上用户自定义 schema 的登记方式；现在内核只认两个内置 schema。
- [x] **B0-2** 可读的数据版本，带三个时间：业务数据时间、最近一次成功同步、最近一次尝试（§7.3）。这三个时间都放进公共属性 `meta.data`。
- [x] **B0-3** `meta.data` 增加“定义已有、还没有有效数据”这一状态（§7.8；PRD §11.0 表中的落点是“第二期”）。未满足的数据源不是有效依赖：下游不能读它，它也不显示为空表。
- [x] **B0-4** 许愿格的输出契约可以声明 `schema_id`，让标准控件按 on_bind 直接绑定产出的表（§11.4 第 1 步）。
- [x] **B0-5** 契约文档：服务 README、`schemas/`、内核文档 [02](../doc/workspace/kernel/02-数据模型与引用.md) 和 [04](../doc/workspace/kernel/04-数据源发布与同步.md)。

### B1 执行设施核对（PRD §12 的未核对项）

- [x] **B1-1** 同步程序在后端运行，以工作区 owner 的身份（§6 Q4，已定）。这一项核对后端哪个设施能承担这种“owner 计算空间里的普通定时程序”（PRD §7.5、§7.6），从调度、隔离、资源归属、凭据保管四方面看。候选都未核对：workflow 服务、node executor、扩展工具容器、Goal 的检查脚本。凭据放在哪里（§6 Q6）一起调研。产出一页对比，交 owner 选设施并裁决 Q6。

### B2 定义先于数据，交给 Agent（PRD §7.8）

- [x] **B2-1** 分析发现缺少外部数据时，按执行器分（§6 Q3）：
  - 浏览器里的 xllm 执行器：分析失败，与交互升级的“找不到输入算失败”一致，因为页面没有建数据源的权限。失败原因要说明缺的是外部数据，并引导改用 Agent 执行。
  - Agent 做执行器：Agent 写入带同步脚本的数据源，分析的输入指向它。运行的发起者不是 owner 时，Agent 不代建，改为给 owner 提需求（B4-6，§6 Q14）。许愿格的设计目的是让所有人都知道复杂的流程是怎么来的、哪些东西可以改；协作者主要通过运行宏来使用许愿格。
- [x] **B2-2** 首次同步完成之前，Agent 建的数据源处于 B0-3 的“定义已有、还没有有效数据”状态。许愿格按定义的 schema 绑定输入，编码可以完成，主状态是“等待数据”（PRD §5.8 第 3 行）。
- [x] **B2-3** 交给 Agent。Agent 是执行器时由它自己完成（B2-1）。协作者提的需求（B4-6）由 owner 交给 Jarvis。Agent 的工作是：编写同步程序、申请授权、完成首次同步。入口随 C2。

### B3 结果怎样进入工作区（PRD §7.6）

- [x] **B3-1** 方式一“拉”（先做，§6 Q5）：同步程序把结果写到 Zone 内的一个位置，新增一种以 owner 身份读取 Zone 路径的来源。产出里带业务时间、程序版本和状态。
- [ ] **B3-2** 方式二“推”（目标，在“拉”之后做）：同步运行是一种新的运行类型，复用外部执行器协议（领取、租约、心跳、报告）。内核按数据源契约校验后提交，每次尝试都有运行记录。
- [x] **B3-3** 全量与增量（§7.3）：
  - 全量刷新要说明记录被移除意味着什么。
  - 增量同步要有稳定的记录身份、重复更新的处理和删除的表达。
  - 重试不重复发布，也不发布半成品。
- [x] **B3-4** schema 变化（§7.3）：破坏下游契约的变化不能悄悄发布。字段被删除或改了类型时，接上第一期的 `needs_coding` 判定。

### B4 授权、凭据、隔离（PRD §7.4、§7.9；权限 PRD R39、R40）

- [x] **B4-1**（2026-11-16：权限按 §7.5；能力范围由同步任务的执行环境强制：只放行声明的域名、清空环境变量、不给读写，流量不计量，§8.4）授权绑定的是能力范围：可访问的域名、可用的凭据、流量与频率预算、写入的数据源、可读的配置输入。
  - 程序在范围内的新版本（包括 Agent 修复后的版本）直接生效；范围扩大时，授权者要重新确认。
  - 范围由执行环境强制，不靠程序自觉。
  - 权限（§6 Q7、Q13）：写草稿、测试、安装、升级、授予能力、修改同步程序的代码、修改定义，都只有 owner 能做（含替 owner 工作的 Agent）。触发同步不限 owner，按数据源上的 `run`；查看同步记录按 `read`。改定义一项比 R39 严（R39 只要 `update`），R39 的修改见 B8-1。
- [x] **B4-2**（2026-11-16：Agent 的 settings，owner 本人在数据源属性里填，结果和记录里都脱敏，§8.4）凭据保管：凭据不进宏的输入、数据、聊天、模板和运行记录，只交给被授权的执行环境。PRD 没有写放在哪里（§12），见 §6 Q6。
- [ ] **B4-3** 同步数据的可见范围：用拒绝条目表达（R40，已定）。DV 验证不该看到的成员读不到。
- [x] **B4-4**（2026-11-16：同步程序没有环境变量和文件权限，读不到工作区，`[sync]` DV 验证）不变量（§6.6）：同步程序读不到工作区的其他数据，声明的配置输入除外。
- [x] **B4-5**（2026-11-16：停用的任务不再运行，刷新报 `SYNC_UNAVAILABLE`）停用或撤销授权后，后续运行不再执行，未完成的运行失去提交资格（§7.4）。
- [x] **B4-6** 协作者的需求入口（§6 Q7）：协作者描述要一个什么样的数据源（数据含义、来源、时效），作为需求提给工作区 owner；owner 可以自己处理，或交给 Agent（B2-3）。Q14 的情形（协作者发起、Agent 执行）也走这里。形态待定，可以先用发给 owner 的通知或消息。

### B5 记录身份、频率、数据量（PRD §7.7）

- [x] **B5-1** 表格数据源必须声明主键。现有 URL 来源在没有 `id` 列时用行号作身份，一并改掉。
- [ ] **B5-2**（间隔下限已实施；版本保留按 owner 2026-11-16 的答复另行立项：用 named store 保存历史，大数据源要自动分片，按大小决定是否保留历史）最小同步间隔 15 分钟；数据源 Commit 保留最近 50 个版本，再加被固定引用的版本。这是 §6 Q8 的占位值，B8-2 的 DV 之后再调。
- [x] **B5-3** 同步程序只带回工作区需要的部分，确定单次同步的数据上限。现有抓取的上限是 32 MiB、30 秒。

### B6 稳态运行与修复（PRD §7.5）

- [x] **B6-1**（2026-11-16：Agent 宿主的同步任务时间表，§8.2；按周期自动运行没在 DV 里等）按调度直接运行，没有模型在环。
- [x] **B6-2**（2026-11-16：§8.2，单测覆盖）失败分三类处理：
  - 暂时性失败：按策略重试，不打扰用户。
  - 程序失效：通知 owner，可以交给 Agent 修复，修复产生新的程序版本。
  - 授权失效：只能由授权者处理。
- [x] **B6-3** 新增 Agent 手册：编写、测试、安装、修复同步程序，按 AGENTS.md 的 cmd_skills 规则写。

### B7 与许愿格的衔接

- [x] **B7-1** 数据源更新后，下游许愿格显示结果过期，不自动运行（§7.3、§10）。
- [x] **B7-2** 运行期间数据源输入有更新时（PRD §5.7）：建议照常写入，如实记下所依据的数据版本，并立即标为过期。工作区内由人编辑的输入维持现在的处理：交互升级之后是运行失败、提示重跑。
- [x] **B7-3** PRD §4.3 的“截取前 100”分两层做（§6 Q9）：数据源定义里的条数上限（B0-1），同步程序只带回这么多；表格视图的显示行数上限，由 UI 选择显示前 100 条还是全部。
- [x] **B7-4** 复制与模板（§7.4）：数据源定义和同步程序的引用可以复用；授权、凭据、执行环境要重新检查，不继承原 owner 的。`ws.clone` 同步处理。

### B8 文档与验收

- [x] **B8-1** 回写 PRD §2.2 里标为“修订”的已有决定：冻结思路 §4.4（定义先于数据）、§8.5（许愿格不新建数据源）。PRD §7.8 按 §6 Q3 改为“按执行器分”。[权限 PRD](aiws-permission-system-prd.md) R39 按 §6 Q7、Q13 改：同步程序上的一切写操作（含修改代码）和修改定义都只有 owner 能做，协作者只能提需求；触发同步维持 `run`。
- [ ] **B8-2**（“拉”路径的同步、失败重试、复制、同步任务已 DV，§7.10、§8.5；真实 Agent 在对话里自己写程序装任务、按周期自动运行没在 DV 里验）在真实 Zone 上 DV PRD §9 的四行：“外部数据缺口”“自定义同步”“同步失败与重试”“复制与模板”。

## 4. C：共用契约的余项

### C1 对象入口的动作（PRD §8.1；核心 API L-3；权限 PRD R43）

权限优化 TODO B-6 的审批门已定为维持现状（§6 Q11）。动作的权限检查与对应的 kRPC 方法相同，结果带 `risk`，SDK 和核心 API 不设审批门；Agent 调用非只读动作前先问用户，这一条写在手册里。

- [x] **C1-1** 声明首批动作：启动许愿格的三个阶段（第一期 T6-2）、触发数据源同步（随 B）。权限检查与对应的 kRPC 方法相同（权限 PRD R21 的表）。
- [x] **C1-2** 走同一个审批门的还有：Agent 修改权限（权限 PRD R42 的 Agent 部分，权限 TODO D3）。这一项属于权限 TODO，这里只标出：它原本也在等 B-6，现在 B-6 已定，可以推进。
- [ ] **C1-3**（手册已更新，Agent 在对话里先问再 xcall 的真实 Zone DV 没做）更新手册 `aiworkspace_template_dev`、`aiworkspace_run`：写明动作的 `risk`，以及非只读动作先问用户。做真实 Zone DV。

### C2 Jarvis 接入（PRD §8.3）

- [x] **C2-1** AIWS 里开侧栏或浮窗（§6 Q12），把当前选择、位置和对象引用交给 Jarvis。接口已经具备（路径、选择器、`meta`）。照 Agent 浏览器的对话面板做（[ThingConversation.tsx](../src/frame/desktop/src/app/agent-browser/conversation/ThingConversation.tsx)，会话公共组件 `SessionConversation`），会话的绑定参照它的 D35：每个 Agent 一个带 `ui.source` 标记的专用会话，用 `bind_default` 绑定（见 [Agent 浏览器 TODO](agent-runtime-browser-todo.md)）。AIWS 要不要也用一个专用会话，实施时对照 Agent 浏览器再定，记进 §7。按手机布局检查。
- [x] **C2-2** Jarvis 在对话中完成分析后，用 `run-start` 交付同一种 `analysis`（第一期 T6-1 已经有这个 CLI 动词）。补一条真实 Zone DV：Jarvis 分析 → 浏览器编码 → 运行。
- [x] **C2-3** 在 C1 之后，验收 PRD §9 的“统一发现”：AI 能发现可读对象、可用控件和获准的动作。

## 5. D：收尾

- [x] **D1** 把 PRD §9 的验收表逐行对账。第一期 TODO §4 中被交互升级作废的几行另外标出：“试运行直通”，以及“编码的候选应用后”。
- [x] **D2** 测 PRD §12 的未核对项：运行时间长、提交又频繁时，阶段快照的读事务一直不结束，WAL 文件会不会持续增长。
- [x] **D3** 文档：
  - 《内核架构改进》§12.3：T3-9 标为已实施，再核对浏览器优先 M5 的其余几项。
  - 第一期 TODO：状态行注明哪些内容被交互升级取代，§3 的勾选框按实际状态勾上。
  - PRD 正文：整理“候选”“试运行直通”等旧说法（可选，文首的修订已经声明以交互升级为准）。

## 6. 需要 owner 确认

2026-11-15 owner 答复了 Q1–Q12，以及由此引出的 Q13、Q14。“owner 原话”一列照录；“落到”一列是据此改过的任务。

| # | 事项 | owner 原话 | 裁决 | 落到 |
| --- | --- | --- | --- | --- |
| Q1 | A 是否不等 xllm M6，先在页面主线程接入 | 按你的建议 | 是。A 在页面主线程接入，不等 M6。理由是 xllm TS 的 Q5、Q11：core 不知道桥，以后加 Worker 不用改 core；Worker 只是为了不卡 UI，不是安全边界 | A；xllm TS TODO 的 M6 |
| Q2 | 续跑时是接着原来的循环，还是从头跑 | 按你的建议 | 由 A0 第 4 项的核对决定：新 attempt 保留阶段状态，就接着原来的循环跑；不保留，就从头跑 | A0、A3 |
| Q3 | 分析发现缺少外部数据时，结果是什么 | 看执行器，xllm 失败（因为页面内没权限），用 agent 做执行器可以，agent 有权限写带同步脚本的数据源 | 按执行器分。浏览器里的 xllm 执行器：分析失败，因为页面没有建数据源的权限。Agent 做执行器：Agent 写入带同步脚本的数据源，许愿格的输入指向它 | B2-1、B2-3 |
| Q4 | 同步程序在哪里运行 | 在后端运行，跑在工作区 owner 的身份下 | 后端，以工作区 owner 的身份运行。B1-1 只在后端设施里选 | B1-1、B3、B6 |
| Q5 | “拉”先行，还是直接做“推” | 拉 | 先做“拉”（B3-1），“推”（B3-2）后做 | B3 |
| Q6 | 凭据放在哪里、怎样只交给被授权的执行环境 | 按建议 | 随 B1-1 一起调研，调研后再定。B4-2 仍等这一条 | B1-1、B4-2 |
| Q7 | 协作者在同步程序上能做什么（草稿、测试、升级、管理） | 只有 Owner 能做，协作者只能提带同步脚本的数据源的需求给 Owner | 写草稿、测试、安装、升级、授予能力、管理都只有 owner 能做（含替 owner 工作的 Agent）。协作者只能向 owner 提“带同步脚本的数据源”的需求。例外是触发同步，见 Q13 | B4-1、B4-6、B8-1 |
| Q8 | 最小同步间隔和数据源 Commit 的保留策略 | 先拍个脑门 | 先用占位值：最小间隔 15 分钟；保留最近 50 个版本，再加被固定引用的版本。B8-2 的 DV 之后再调 | B5-2 |
| Q9 | “截取前 100”由谁做（PRD §4.3） | 数据源里定义就是 100 条那就只有 100 条，数据源里是 1000 条则 UI 可以选择显示 100 还是 1000 | 分两层。数据源定义可以只要前 N 条，那么同步程序只带回 N 条，数据源里也只有 N 条。表格视图另有行数上限，数据源有 1000 条时，UI 可以选显示 100 条还是全部 | B0-1、B7-3 |
| Q10 | 列组级 Source（同步列只读、手动列可写；PRD §2.2 D12 的 Review 补充）是否放进本期 | 按你的建议 | 不放进本期，等主键（B5-1）落地后再看 | — |
| Q11 | B-6 审批门选哪个选项（Session 内审批、Provider 侧确认协议、维持现状） | SDK 不设审批门 | 维持现状，同 `xcall` 2026-10-29 选的方案 C（[对象 URL TODO](done/agent-object-url-todo.md)）和 xllm TS TODO §2.1。动作的授权就是对应 kRPC 方法的权限检查（Provider 的 RBAC），结果带 `risk`。SDK 和核心 API 不设审批门。非只读动作要不要先让用户确认，由调用方决定：页面在实现里定，Agent 照手册先问用户 | C1、C1-2 |
| Q12 | Jarvis 在 AIWS 里的入口 | 在 AIWS 里开一个侧栏或浮窗。现在可以参考 Agent Browser 的类似实现了 | AIWS 里开侧栏或浮窗，打开时带上当前选择和位置。参照 Agent 浏览器的对话面板 | C2-1 |

由上面的答复引出的两个新问题，owner 同日也答复了：

| # | 事项 | owner 原话 | 裁决 | 落到 |
| --- | --- | --- | --- | --- |
| Q13 | Q7 的“只有 owner”是否也包括修改数据源定义和触发同步 | 触发同步谁都可以做，但修改代码只有 owner 可以 | 触发同步不限 owner，仍按 R39 要数据源上的 `run`。同步程序的代码只有 owner 能改。修改定义没有单独答复，按 Q7 的“协作者只能提需求”归为只有 owner | B4-1、B8-1 |
| Q14 | 协作者发起分析，执行器是 Agent，分析发现缺外部数据：Agent 能不能替他建带同步脚本的数据源 | 不能。从许愿格设计的角度来看，核心设计目的是让所有人都知道“这个复杂的要死的流程是怎么来的，哪些东西可以改”。协作者主要使用许愿格的方式是运行宏 | 不能。运行的发起者不是 owner 时，Agent 不代建，改为给 owner 提需求（B4-6）。要按运行的发起者判断，不按 Agent 判断：托管执行时只有配置的 Agent 拿得到 claim token | B2-1、B4-6 |
## 7. 实施回写

2026-11-15 开始实施。A 由主会话做；B 的内核部分、B1-1 调研、C1、C2 各交给一个并行子任务，按 §7.2 的契约各改各的文件。

### 7.1 A0 的四项（2026-11-15 定）

1. **迭代用满**：继续用 `EXECUTOR_FAILED`，`detail` 用 xllm 的 `limit_reason`（如 `tool iteration limit (60) reached`），说明用满的是哪项预算。不新增错误码。
2. **阶段时长**：brief 里仍没有阶段时长，靠迭代数和租约兜底。xllm 必须有 `timeout_secs`，取 21600（6 小时）；单次模型调用 `llm_timeout` 取 900 秒（编码用 xhigh 时单轮可能要几分钟）。
3. **上下文压缩**：brief 不带模型窗口。`context_window` 取 200000，按 xllm 默认的 0.8 让出比例压缩；模型实际窗口更小时，provider 拒绝也会触发压缩（xllm 的 `context_limit_reached`）。DV 里看有没有触发。
4. **续跑**：核对结果是**不保留**。`proc.claim` → `stages::host_open` 先 `state.wish.drop_hosted(run_id)`，再 `open_stage` 打开一个新的阶段状态：上一个 attempt 暂存的分析、试运行结果、工具轮数都在内存里，随旧 attempt 丢掉（服务重启也一样）。所以按 Q2，新 attempt 从头跑循环；上一个 attempt 的 xllm 运行记录留在 `IndexedDbRunStore` 里，标为中断，不 `resume`。

### 7.2 B0 契约（2026-11-15 拟，实施按它做）

**数据源定义 `source.def`**（B0-1）。挂在实体已有的 `source` 上，`session` 以外的来源都可以带：

```json
"def": {
  "meaning": "数据含义，必填，≤2000 字",
  "origin": "来源说明（从哪里、怎么取），可选",
  "primary_key": ["field_id"],
  "freshness": { "max_age_s": 3600 },
  "limit": 100
}
```

- `primary_key`：1–4 个字段 ID。`kind: zone` 的表格必填；`url` 的表格没写时用 `id` 列，也没有 `id` 列就刷新失败（`PRIMARY_KEY_REQUIRED`），不再用行号（B5-1）。`workspace` 来源的记录自带 `record_id`，不需要。行的 `record_id` 由主键值经 `wish::record_id_for_key` 得出；一次取回里主键重复，刷新失败（`DUPLICATE_KEY`），什么都不写。
- `freshness.max_age_s`：时效要求。业务时间（没有就用同步时间）超过它，`meta.data.expired` 为真。只用于显示，不触发运行。
- `limit`：只要前 N 条（1–100000）。取回的行按文件顺序截到 N 条，`data_version.truncated` 记下（§6 Q9 第一层）。

**用户自定义 schema 的登记**（B0-1）。`schema_id` 的写法是 `<ns>.<name>@<major>`。`buckyos.` 命名空间只有内置的两个，其他 `buckyos.*` 拒绝。用户 schema 由带 `source.def` 的表登记：工作区里最早创建、还活着、带这个 `schema_id` 和 `source.def` 的那张表就是它的定义，字段（ID 和类型）加 `def.primary_key` 就是这个 schema。别的表带同一个 `schema_id`（比如许愿格的产出）时，必须含这些字段且类型相同。这是 schema 类校验，只管准入。

**Zone 路径来源 `kind: zone`**（B3-1，“拉”）：

```json
"source": { "kind": "zone", "path": "/home/<owner>/…/quotes.json", "refresh": { "interval_s": 900 }, "def": { … } }
```

- `path` 是工作区 owner 家目录下的绝对 Zone 路径，不许有 `..`。刷新时服务以**当前**工作区 owner 的身份读：路径不在当前 owner 的家目录下就失败（复制给别人的工作区因此要重新配置，B7-4）。读法与 `agent_tool` 的 `zone_file.rs` 相同：节点上读 `$BUCKYOS_ROOT/data/home/<owner>/<rest>`；独立后端用启动参数给根目录（测试用）。大小与时间上限沿用抓取的 32 MiB、30 秒。
- 文件内容可以是普通 JSON / CSV / Markdown（与 `url` 相同的解析），也可以是同步产出的信封：

```json
{
  "format": "aiws.sync.v1",
  "status": "ok",
  "error": "status 为 failed 时程序自己写的原因",
  "program": { "name": "quotes", "version": "3" },
  "business_time": "2026-11-15T08:00:00Z",
  "produced_at": "2026-11-15T08:01:12Z",
  "fields": [ { "field_id": "symbol", "name": "Symbol", "type": "text" } ],
  "records": [ { "symbol": "AAPL", "price": 1.0 } ],
  "truncated": false
}
```

  `status: failed` 时这次刷新失败，原因取 `error`，保留上一个有效版本。`fields` 缺省时沿用表现有的字段。

**可读的数据版本 `payload.data_version`**（B0-2）。只由刷新的 Commit 写，其他写入照 `SOURCE_READ_ONLY` 拒绝：

```json
"data_version": { "synced_at": "…", "business_time": "…", "produced_at": "…", "program": { "name": "…", "version": "…" }, "records": 100, "truncated": false, "content_root": "…" }
```

**公共属性 `meta.data`**（B0-2、B0-3）。在现有的 `mode`、`write`、`source` 之外增加：

| 字段 | 含义 |
| --- | --- |
| `state` | `synced` 才有：`valid`，或 `pending`（定义已有、还没有有效数据，即没有 `data_version`）。`session` 来源恒为 `valid` |
| `version` | 就是 `data_version`：业务数据时间、最近一次成功同步（`synced_at`）、程序版本 |
| `attempt` | 最近一次尝试：`{ at, status: running / succeeded / failed, error? }`，取自本服务的刷新 run（本地状态，不随导出） |
| `expired` | 超过 `def.freshness.max_age_s` |
| `def` | `source.def` 原样 |

`data.source` 摘要加上 `path`；之前列出却从未写入的 `revision`、`fetched_at`、`refreshed_at`、`error` 去掉。

**未满足的数据源不是有效依赖**（B0-3、B2-2）：

- `doc.read` 的 `body.state` 为 `pending`：给字段定义，不给记录；文本查询只覆盖名字和定义；SQL 和行读取返回 `DATA_PENDING`。
- 许愿格的 `input_problems` 加 `{ input, entity_id, reason: "pending" }`，主状态是已有的 `waiting_data`。
- 编码可以做：快照里待同步的输入是一张只有定义字段、没有行的空表，编码的 brief 写明它还没有数据、照定义编写。运行阶段（`proc.start` 的 `run`）拒绝，用现有的输入未就绪的码。

**许愿格输出声明 `schema_id`**（B0-4）。输出契约里 `type: "table"` 的结果可以带 `schema_id`，产出的表带上它：内置 schema 要有标准字段（`standard::check_table`），用户 schema 要有登记的字段。标准控件据此按 on_bind 直接绑定。

**分析发现缺外部数据**（B2-1）。分析的 blocker 增加 `code: "external_data"`，可带 `wanted: { meaning, origin, freshness, fields }`。有这种 blocker 时运行失败，子码 `NEEDS_EXTERNAL_DATA`（不是 `NEEDS_INPUT`）。分析的系统提示写明什么时候用它。浏览器执行器到此为止，卡片上说明缺的是外部数据，并引导改用 Agent 执行；Agent 执行器照手册处理（发起者是 owner 时建数据源，否则给 owner 提需求）。

**权限**（B4-1，§6 Q7、Q13）：

- 改一张表的 `source`（含 `def`，增加或去掉来源）只有工作区 owner 能做（含替 owner 工作的 Agent），其他人 `PERMISSION_DENIED`。
- 触发刷新（`proc.start source.refresh@1`）要数据源上的 `run`（原来要 `update`）；查看刷新记录要 `read`。

**频率与保留**（B5-2）：`refresh.interval_s` 至少 900。数据源的刷新 Commit 保留最近 50 个版本，再加被固定引用的版本。

**表格视图的显示行数**（B7-3，§6 Q9 第二层）：表格视图的配置加 `row_limit`（正整数或不设），UI 可以选“显示前 100 条”或“全部”。

### 7.3 分工

| 包 | 内容 | 主要文件 |
| --- | --- | --- |
| A（主会话） | A0–A5、A8，随后 A6、A7；B2-1 在卡片上的引导文字 | Desktop `app/aiworkspace/executor/`、`api/`、`ui/wish/` |
| B-内核 | B0、B2-1（内核）、B2-2、B3-1、B3-3、B3-4、B4-1、B5、B6-3（手册里写编写、测试、首次同步）、B7-1–B7-4、B8-1 的文档回写，以及表格视图和属性里数据源状态的显示 | `aiworkspace/{core,store,server,schemas}`、内核文档 02 / 04、Desktop 表格视图与属性面板 |
| B-调研 | B1-1（含 Q6 凭据）、D2 | 新的调研页、`store` 的 WAL 实验 |
| C1 | C1-1、C1-3、C2-3 | `server/src/obj_provider.rs`、手册 `aiworkspace_run`、`aiworkspace_template_dev` |
| C2 | C2-1、C2-2、B4-6（协作者向 owner 提数据源需求） | Desktop 新的 Jarvis 面板、AIWS 外壳接入 |

### 7.4 A：浏览器循环（主会话）

- `executor/wishRuntime.ts`（新）：brief 的工具成为 xllm site runtime `aiws.wish` 的页面工具，调用转 `proc.tool`；`run_program` 走 `programHost`（MW 沙箱），程序不进循环。服务答 `status: error` 是模型看得到的失败调用（与原来一样带 `is_error`）；调用根本到不了服务（撤销、取消、离线、`SNAPSHOT_TOO_LARGE`）时不给模型看，直接结束这次运行，按原来的码上报。
- `executor/localExecutor.ts`：`loop()` 改为 `XllmTask.prepareHosted`（`hostPromptSections: 'NONE'`；阶段的系统提示把 `{{` 转义，原样交给模型，不当 xllm 模板）→ `XllmRun.start({ input: brief.user })` → `execute`。限额按 §7.1。用量由包一层的 `LlmClient` 计数（每次成功的模型调用，含压缩），之前 attempt 的用量存在 `localStorage['aiworkspace.claims.usage']`，随最后的 `usage` 一起上报。xllm 停在 `paused` 且原因是 provider 暂时故障或超时时，隔 2、4 秒各重试一次。`limit_reached` 报 `EXECUTOR_FAILED`，`detail` 是 `limit_reason`；拒绝按原子码表报 `AI_*`。`chat.ts` 删除；`infer.ts` 导出 `stageLlm()`、`refusalCode()`，阶段循环和宏的直接推理共用一个模型客户端。
- A3：按 §7.1 第 4 项从头跑，所以用 `MemoryRunStore`，不用 `IndexedDbRunStore`：存一份永远不会 `resume` 的快照只会占页面存储。
- A4：`WishService.noModelReason()`：guest（没有 Zone 会话）在本地执行时，卡片上分析、编码、续跑、调 AI 的宏的按钮置灰并写明原因；开了托管执行时提示到属性里改用托管。dev 覆盖带模型端点时不提示。
- B2-1 的卡片部分：`NEEDS_EXTERNAL_DATA` 的文案说明缺的是工作区以外的数据、浏览器不能建数据源、改用 Agent 执行或请 owner 准备；卡片多一个“准备这个数据源…”按钮，打开 C2 的数据源需求对话框（owner 发给自己的 Agent，别人发给 owner）。
- 与 TODO 原文不同的地方：
  - xllm 的连续错误上限（3 次）现在也是一个停止条件：连续 4 轮工具调用都失败（比如试运行一直失败）时，阶段以 `EXECUTOR_FAILED` 结束，原来的手写循环只受迭代数限制。xllm 的这个上限不能从配置改（Rust 也一样），需要的话再议。
  - 完成条件的 grep 不为空：`infer.ts` 里还有 `chat.completions.create` / `helper.llm_chat` 两个方法名，是交给 `AiccLlmClient` 的传输适配（用用户的会话令牌调 kRPC），模型调用本身都经过 `buckyos/xllm`。
- 测试：`wish.spec.ts` 17 条全过（原 14 条 + T3-4 + 新增两条：迭代用满报 `EXECUTOR_FAILED` 并写明限额；`NEEDS_EXTERNAL_DATA` 的卡片文案与需求对话框）。T3-4 加了断言：第一次 attempt 的模型用量计入续跑后的 `usage.calls`。`mock-llm.mjs` 与 `OpenAiLlmClient` 的报文一致，新增标记 `#external`。DV 用例 `[macro-resume]`（关标签页后续跑）加在 `tests/dv/aiworkspace.macro.spec.ts`。

### 7.5 B-内核

- B0：`check_source` 支持 `def`、`kind: zone`，刷新间隔下限 900 秒；用户 schema 登记在 `core/src/standard.rs`（`user_schema_definer`、`check_user_table`）；`data_version` 只由刷新写（`check_data_version`）；`meta.data` 加 `state`、`version`、`def`，`attempt`、`expired` 由服务补（`store/src/reads.rs`、`source_attempt`）；输出契约可带 `schema_id`（`store/src/wish/plan.rs`、`analysis.rs`）。
- B0-3 / B2-2：`body.state: pending` 只给定义；SQL、行读取返回 `DATA_PENDING`；宏的视图执行不拦，所以编码时是一张空表；`input_problems` 加 `reason: pending`；编码 brief 写明输入还没有数据；运行阶段在 `ready()` 拒绝（`NEEDS_INPUT`，`missing_inputs[].reason: pending`）；分析看到的地图标出 `schema_id` 和数据源状态。
- B2-1：blocker 可带 `wanted`（`wish::check_wanted`）；有 `external_data` blocker 时运行失败，子码 `NEEDS_EXTERNAL_DATA`，记录的 `missing_inputs[].wanted` 带上需要的数据；`aiws/prompts/analyze.md` 加了第 5 条。
- B3-1：`server/src/source.rs` 的 `zone_file`：以当前 owner 读家目录下的文件（canonicalize，不跟随指出家目录的链接），32 MiB / 30 秒；节点上根是 `$BUCKYOS_ROOT/data`，独立后端用 `--zone-root`；支持 `aiws.sync.v1`（`schemas/sync-envelope.schema.json`）。
- B3-3：全量刷新，输出里没有的记录删除；内容和版本都没变就记 `unchanged`，不产生 Commit。B3-4：刷新结果带 `schema_changes`，读到这些字段的许愿格进入 `needs_coding`。
- B4-1：增删改 `source` 只有 owner（`require_source_owner`，覆盖所有能带来源的类型）；触发刷新要 `run`（或写权限），查看记录要 `read`。能力范围由执行环境强制一项等 B1-1。
- B5-1：`keyed()`：`def.primary_key`，否则 `id` 列，否则 `PRIMARY_KEY_REQUIRED`；重复键 `DUPLICATE_KEY`；工作区发布来的行保留对方的记录 ID。B5-3：`def.limit` 加 32 MiB / 30 秒。
- B7：B7-1 已有行为满足（加了端到端测试）；B7-2：数据源输入不再是写入的前置条件（`readset::guarded`），照常写、记下所依据的版本；B7-3：Cell 的 `row_limit`（表格视图“只显示前 100 条 / 显示全部”）；B7-4：Clone 保留数据和定义，最近一次尝试不带过去，新 owner 读不到原 owner 的路径。
- B6-3：新手册 `cmd_skills/aiworkspace_sync/SKILL.md`（定义、编写、测试、首次同步、修复；写明程序还不会被定时运行）。`aiworkspace_run` 写明缺外部数据时的规则（发起者是 owner 才建，否则报 `external_data` blocker），`aiworkspace_template_dev` 写明主键要求和 owner 才能设来源。
- Desktop：`SourcePanel.tsx` 显示待同步或有效、三个时间、最近一次尝试与错误、是否过期、定义，以及“只有 owner 能改来源”；`TableEditor.tsx` 待同步时显示“还没有数据”，不显示空表。新 i18n 区 `datasource`。WASM 已重建。
- 文档：服务 README（Sources、分析、Validator 表、视图 SQL）、`schemas/error-codes.json`（`PRIMARY_KEY_REQUIRED`、`DUPLICATE_KEY`、`DATA_PENDING`、`NEEDS_EXTERNAL_DATA`）、内核文档 02、04。
- 与 §7.2 不同的地方：
  - 数据源 Commit 保留最近 50 个版本没有做：历史只有一个整体的下限 `base_seq`，没有按实体压缩的机制，现在全部保留（README 和 04 标为未做）。
  - `synced_at` 是“这个版本同步的时间”：没变化的刷新不写 Commit（否则每次同步一个 Commit，富文本和资源的结果也会误标过期）。`expired` 没有业务时间时，取它和本地最近一次成功检查中较晚的那个。
  - 没登记的用户 `schema_id` 允许存在、不带规则（已有测试依赖）；只有许愿格的产出要求 schema 已登记。
- 行为变化：一张 CSV 链接没有 `id` 列、又没写 `def.primary_key`，刷新会失败；Desktop 现在没有选主键的界面（§7.10 待确认）。

### 7.6 B1-1 调研与 D2

- 调研页：[aiws-sync-facility-survey.md](aiws-sync-facility-survey.md)。建议在 Agent 宿主里做一种独立于 Goal 的“同步任务”，复用 Goal 检查脚本的执行器（Deno 子进程）和时间表；按能力范围收窄 Deno 权限，B4-1 五项能强制四项（流量计量做不到）；产出写进 AgentRoot（在 owner 家目录下），`kind: zone` 直接读。workflow 现在不能运行用户代码、用户定义只在内存里；node executor 没有调度；扩展工具不是执行设施。Q6：系统没有密钥库，建议第一步放在 Agent 自己的 settings 里，“Agent 自己读得到”记为已知风险。页面 §6 有 7 个待 owner 决定的问题。
- D2：WAL 会持续增长。阶段快照单开一个只读连接并 `BEGIN DEFERRED`（`store/src/wish/snapshot.rs`），从 `proc.claim` 一直持有到报告、取消、失败或重新领取。实测（`store/tests/wal_growth.rs`，`#[ignore]`）：持快照时每次小提交约 51 KiB，2000 次后 WAL 99.5 MiB（库本身 4.3 MiB）；不持时稳定在约 4 MiB。还发现一处泄漏：租约过期后，只有有人对这次运行调 `proc.get` 才释放阶段。
  - 已修（主会话）：`open_db` 设 `journal_size_limit` 16 MiB，读者结束后 WAL 缩回；60 秒定时器调 `wish::release_lapsed`，租约过期的阶段即使没人读也释放。
  - 没做：根治办法是开阶段时 `VACUUM INTO` 一份快照副本、活库上不挂读事务（实测 10 万条记录 62 ms）。

### 7.7 C1：对象入口的动作

- `buckyos-api` `did_object_provider.rs` 加 `ObjectKind::only_if(...)`：动作只在实体的 `meta` 满足条件时列进 Profile（用调用者的凭据读一次 `meta`）；调用时不另加检查，仍由 kRPC 方法判定。
- `server/src/obj_provider.rs`：许愿格实体上 `analyze` / `code` / `run`（`proc.start { program: "wish.xllm@1" }`，risk `write`）；由来源写入的实体上 `refresh`（`proc.start { program: "source.refresh@1" }`，risk `idempotent_write`）。说明引用 `methods.rs` 里 `proc.start` 的授权文字。根对象加属性 `controls`（控件目录）。
- 手册：`aiworkspace_run` 新增 “Actions on workspace objects”（发现、risk、非只读动作先问用户、`agent_tool xcall`）；`aiworkspace_template_dev` 加简版。
- C2-3：`server/tests/objects.rs` 的 `actions_are_declared_where_they_apply_and_judged_as_their_krpc_method`：Profile 列出动作和 risk；只读成员看得到动作但没有权限；被拒时对象入口与 kRPC 的错误逐字段相同，且没有产生运行；同一个 key 返回同一个运行。仍不能经对象入口发现的：renderer 目录、运行本身、机器可读的“动作要什么权限”、单个控件的自述。
- C1-2：B-6 已定为维持现状，权限 TODO 的 D3（Agent 修改权限）不再阻塞；现在这层保护只在手册里。

### 7.8 C2：Agent 面板与数据源需求

- 面板：`ui/jarvis/`（`JarvisPanel.tsx`、`open.ts`、`context.ts`、`session.ts`），挂在 `ui/framework/FrameworkHost.tsx`。入口：协作区按钮、主菜单“协作 ▸ 与 Agent 对话”、手机主菜单同名项；访客和受限会话不显示。宽窗口是右侧栏，手机是底部浮窗。对话复用 `SessionConversation`。
- 会话：每个 Agent 一个带 `ui.source = aiworkspace` 的专用会话，所有工作区共用，不调 `bind_default`。理由：和 Agent 浏览器 D35 一致，不按工作区拆；不进主对话，免得引用塞满主对话；Agent 不需要往这个会话投递，也不需要按会话挂手册。
- 上下文：用户消息后面跟一个纯文本块 `[aiworkspace context]`（工作区、页面、画布、视图、打开的对象、选中的 Block 及它显示的数据，带 `obj://` 引用，最多 12 项）；同一位置和选择只发一次，面板顶部提示下一条会带什么、可以不发。用纯文本是因为 libopendan 只把消息正文给模型。
- B4-6：`ui/sources/DataSourceRequestDialog.tsx`、`sourceRequest.ts`（`openDataSourceRequest`）：协作者填要什么数据、来源、时效、字段，作为 MessageHub 消息发给 owner（带许愿格和工作区的引用与链接）；owner 自己打开时发进和 Agent 的工作区会话并打开面板（B2-3 的“交给 Jarvis”）。主菜单“协作 ▸ 提数据源需求…”。
- 共享文件的小改：`canvasFocus` 加 `current`（单独选中的许愿格也能拿到）；`SessionConversation` 加可选 `outgoing`（发送前改写）；会话后端加 `addPerson`（给不在联系人里的 owner 发消息）；`aiworkspace.css` 的基础样式不再作用于 `.aiws-system-host` 内部。
- C2-2：`aiworkspace_run` 新增 “Asked from the AI Workspace panel”。DV 用例 `tests/dv/aiworkspace.jarvis.spec.ts`。

### 7.9 集成与 DV 中修的问题（主会话）

- **托管派发在真实 Zone 上从来没有成功过**（A7 的 DV 发现）：`dispatch_goal` 按 AgentSpec 的 `binding.service_name` 当内核服务查找，Agent 不是内核服务，报 `service info not found`，运行一直停在 `queued`。另外，即使找到了，服务自己的会话主体是设备（`ood1`），OpenDAN 只接受 owner 或 `root` 的调用，也会被拒。改为：经节点网关调 `/kapi/<agent_id>`（scheduler 按 Agent 的 ID 配了这条路由），用**发起这次运行的人自己的会话**（`proc.start` 的 kRPC 令牌，像 `asset.import_object` 一样传进来）。这样 Agent 只从它本来就接受的人那里收到工作：owner 发起的托管运行能派发；配置里的其他 `triggerers` 发起时 Agent 会拒绝，记成 `unavailable`（§7.10 待确认）。真实 Zone 上已验证派发成功（`status: dispatched`，Agent 收到 Goal）。
- **真实 Agent 不被认作 Agent**（A7、C2-2 的 DV 发现）：Agent 容器里的运行时用 App 会话登录（`appid` 是 Agent 的 ID，主体是 owner，`principal_kind: app`），AIWS 把它当成主体为 owner 的服务，没有 `kind: agent`。结果是：托管运行它领取不了（`run-claim` 报 “this run is executed by another principal”），在对话里分析完用 `run-start` 交付时报 `AI_ATTRIBUTION_UNAVAILABLE`。服务端（`server/src/auth.rs` 的 `app_agent`）改为：App 会话的 `appid` 是主体名下某个 Agent 时，就是这个 Agent（`Caller::agent_of(<Agent DID>, <owner>)`，与 Agent DID 会话一样）。工作区没有为它单独配置时按 owner 判定（R16），所以权限不变；记录里的 `by` 和 AI 归属从 owner 变成了 Agent。
- **领取以后报告被拒**（同一次 DV）：`check_executor` 拿调用者的主体（Agent 的 DID）去比上下文里按 AgentId 记的执行者，`run-files`、`run-heartbeat`、`run-report` 都报 “only the run's executor reports on it”，租约过期后运行成了 `interrupted`。改为与 `proc.claim` 用同一个判定 `is_executor`。
- **同样的数据重新产出一次也写一个 Commit**（sources DV 发现）：`data_version` 比较时带上了 `produced_at`，程序每跑一次都会产生一个新版本。改为比较时去掉 `produced_at`（与 `synced_at` 一样），数据、业务时间、程序版本都没变就记 `unchanged`；`zone_source.rs` 加了对应断言。
- **D2 的小修**：见 §7.6。
- `proc.start` 的授权文字（`methods.rs`）改成刷新要 `run`，README 授权表按 `AIWS_REGEN=1` 重新生成。
- e2e 的坑：`start.py` 会调 `stop.py`，按进程名杀掉 e2e 自己起的 `aiworkspace` / `nfs_server`，所以重启本机 Zone 时不能同时跑 e2e。

### 7.10 验证

| 项 | 结果 |
| --- | --- |
| `cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -p buckyos-api -- --test-threads=1` | 825 例全过（`activity`、`show` 两例在机器满载时超时，单独重跑通过） |
| `cargo test -p libopendan --lib skills`、`--test xagent_cli skills_are` | 通过（新手册 `aiworkspace_sync` 与改过的两本） |
| `aiws/aiws_test.js` | 10 例全过 |
| Desktop `pnpm check`、改动文件的 eslint、`pnpm build` | 通过 |
| Desktop e2e 全量（`playwright.aiworkspace.config.ts`） | 403 过、6 跳过、1 失败（手写 H03，单独重跑通过）；`wish.spec.ts` 17 条全过 |
| i18n 守卫 | 只剩基线就失败的“后端说英文”（`naming.rs`、`search.rs`、`acl.rs` 等未改动文件） |
| 真实 Zone DV（本机 test.buckyos.io，部署了本单的服务和 Desktop） | 见下 |

真实 Zone DV（真实模型）：

- `[macro]`、`[macro-cli]`、`[macro-sandbox]`、`[macro-permissions]` 通过：浏览器里的 xllm 循环走完分析 → 编码 → 运行 → 改数据 → 再运行。
- `[macro-resume]`（新增）通过：编码中途离开页面，租约过期成 `interrupted`，回到页面续跑成 attempt 2，编码记录正常，用量和 AI 归属记在发起者名下（`current_user`）。
- `[template-quarterly]`、`[template-film]` 通过；`[mindmap] a wish answered by the real model` 通过；`[mindmap] fixture` 因调用已删除的 `ws.grant` 失败（基线就坏）。
- `[sources]`（新增）通过：Zone 路径来源先是 `pending`、SQL 报 `DATA_PENDING`；首次同步后 `valid`，带业务时间和程序版本；程序报失败时保留上一个版本、`attempt` 记下原因；同样的数据再同步不产生 Commit、不重复记录；Clone 保留数据和定义。
- `[jarvis]`（C2 的用例）通过：在 AIWS 面板里让真实 Agent 分析选中的许愿格，Agent 照手册用 `run-start` 交付分析（记录的 `by` 是 Agent），浏览器编码并运行。
- `[macro-hosted-agent]`（新增，A7）通过（修了 §7.9 的三处之后）：浏览器分析、编码后，owner 发起托管运行，服务把 Goal 派给真实 Agent（browsy），它照 `aiworkspace_run` 手册 `run-claim` → `run-files` → `run-exec` 试跑 → `run-program` 交运行 → `run-report --finished`，运行 `succeeded`，汇总表和解读写入工作区。容器里 `agent-tools` 的 `buckyos` 已有 `run-exec`。DV 前把改过的三本手册用 `docker cp` 放进了 browsy 的 AgentRoot（loader 保留本地修改）；正式生效要重新打包 Jarvis。

没跑的 DV：B4-3（拒绝条目挡住同步数据，机制随权限系统已有，没有专门跑）、C1-3 的 Agent 在对话里先问用户再 xcall（手册已写，只有服务端测试）、手机真机（A6 最后一项）、B8-2 的“外部数据缺口”里由真实 Agent 写同步程序并建数据源（手册已写；浏览器一侧的 `NEEDS_EXTERNAL_DATA` 有 e2e）。

### 7.11 D1：PRD §9 验收表对账

| 场景 | 状态 | 依据 |
| --- | --- | --- |
| 分析交接 | 通过 | 浏览器 xllm 与 Jarvis（面板里 `run-start`）写回同一种 `analysis`；`[macro]`、`[jarvis]` DV |
| 准备与运行条件 | 通过 | 主状态函数；待同步的数据源可以编码、不能运行（`NEEDS_INPUT`，`reason: pending`）；`zone_source.rs`、`[sources]` |
| 数据更新后重跑 | 通过 | `[macro]`、`[macro-cli]` |
| 需求或结构变化 | 通过 | `[macro]`（改名照常、删字段要重新编码）；刷新带来的字段删除接上 `needs_coding`（B3-4） |
| 宏越界 | 通过 | `[macro-sandbox]`、`[macro-cli]`、第一期测试 |
| 宏内 AI | 通过 | 第一期测试与 DV |
| 外部数据缺口 | 部分 | 浏览器执行器报 `NEEDS_EXTERNAL_DATA` 并引导（e2e）；Agent 建数据源有手册 `aiworkspace_sync`，真实 Agent 写程序没有 DV；“数据就绪后绑定回原任务”按 PRD §7.8 不再需要（定义先于数据，绑定不断开） |
| 自定义同步 | 部分 | “拉”的路径通过（`[sources]`）；程序在画布关闭时按时间表运行，要等 B1-1 选定执行设施 |
| 同步失败与重试 | 通过（拉） | 失败保留上一个版本、状态准确、重试不重复（`[sources]`、`zone_source.rs`） |
| 并发与人工修改 | 通过 | 交互升级的运行写入规则；B7-2 数据源输入在运行中更新时照常写、记下所依据的版本 |
| 复制与模板 | 部分 | 定义和数据随 Clone，新 owner 读不到原路径（B7-4）；凭据、执行授权不随复制的部分等 B1-1、Q6 |
| 统一发现 | 通过 | 对象入口列出可读对象、控件目录和获准的动作（C2-3，`objects.rs`） |

第一期 TODO §4 中被交互升级作废的两行：“试运行直通”（编码只保存宏，不写结果），以及“三步”一行里的“编码的候选应用后”（没有候选，阶段结束直接写）。

### 7.12 需要 owner 确认

1. B1-1：执行设施选哪个、Q6 凭据放哪里，见 [调研页](aiws-sync-facility-survey.md) §6 的 7 个问题。B3-2、B4-2、B4-4、B4-5、B6-1、B6-2 等它。
> 按建议，做成独立于 Goal 的“同步任务”
2. 数据源 Commit 的保留（B5-2 的“最近 50 个版本”）：现在没有按实体压缩历史的机制，要不要立项？
> 从设计上，应该用namedstore的机制来保存历史版本，目前这个对小数据容器直接支持（named object）,单大的数据源，要做类似DirObject的自动分片。这个需要单独立项。 --> 根据数据源的大小决定是否支持历史记录
3. CSV / JSON 链接没有 `id` 列也没写主键时刷新失败（B5-1）：Desktop 要不要加选主键的界面？
> 按你的建议，没有合适的主键可以自动生成一个 （传统的自增ID）
4. 托管运行由配置里的其他 `triggerers`（非 owner）发起时，Agent 不接受他们的会话，派发记为 `unavailable`。是维持（Agent 只听 owner 的），还是让 OpenDAN 接受 AIWS 服务替 owner 派发？
>  托管运行是指什么？ 如果是spider脚本，Agent编写好后，有权限的用户都可以触发执行，但只有owner和owner的spider可以修改这个脚本。许愿格的执行器选“使用Agent”的时候，用的是当前用户的agent,不是owner的。权限等同当前用户的Agent
5. xllm 的连续错误上限：连续 4 轮工具调用失败，分析或编码就结束（原来的手写循环只受迭代数限制）。可以接受吗？
> 可以配置。我记得默认是6轮
6. C1：`run` 动作的 risk 定为 `write`（它会覆盖别人编辑过的结果，但可以回滚），要不要改成 `destructive`？
> 按你的建议
7. C1-2：Agent 修改权限现在只靠手册里“先问用户”，R42 原来担心的“被注入的内容驱使 Agent 公开工作区”这层保护只在手册里。按“先能力再安全”记为已知风险，请确认。
> 了解
8. C2：受限会话（External DID、Limited 账号）现在看不到 Agent 面板和需求入口；面板上下文只到 Block 一级（不含表格里选中的行）。
> 好
## 8. 第二轮：owner 对 §7.12 的答复（2026-11-16）

### 8.1 裁决与落点

| # | owner 答复（摘要） | 落点 |
| --- | --- | --- |
| 1 | 按建议，做成独立于 Goal 的“同步任务” | §8.2：Agent 宿主里的同步任务（复用 Goal 检查脚本的执行器和时间表），AIWS 经它“立即同步”。调研页 §6 的其余各问按页内建议：以 owner 的 Agent 运行（资源算在 Agent 上）；Agent 不在时数据源停在最近一个有效版本并说明执行环境不可用；凭据用方案 1（Agent 的 settings，“Agent 自己读得到”为已知风险）；流量不计量；只支持单节点。程序失效时的入口（调研页 §6 第 4 问没有建议）：通知 owner，由 owner 交给 Agent 修，不自动唤醒 Agent（Goal 是珍贵的资源） |
| 2 | 历史版本应该用 named store 保存：小的数据容器直接是 named object，大的数据源要做类似 DirObject 的自动分片，单独立项；根据数据源的大小决定是否支持历史记录 | B5-2 的保留策略不在本单做，另行立项 |
| 3 | 没有合适的主键时自动生成一个（传统的自增 ID） | 已实施：没有 `def.primary_key` 也没有 `id` 列时，记录按自增编号（`1`、`2`…，删掉的号不再用）；值没变的行保留原编号，新行或改过的行取下一个号。Zone 表格不再必须写主键 |
| 4 | 许愿格选“使用 Agent”执行时，用的是**当前用户自己的 Agent**，不是 owner 的；权限等同当前用户的 Agent。spider 脚本写好后，有权限的用户都可以触发执行，只有 owner 和 owner 的 Agent 能改 | §8.3：托管执行改为发起者自己的 Agent，去掉 owner 的托管配置。spider 一条与 §6 Q13 一致，落在 §8.2 的 `sync.run` |
| 5 | 可以配置，记得默认 6 轮 | xllm 的 `.llm_context` 现在没有这一项（Rust 与 TS 都固定为 3；OpenDAN behavior 的 budget 可以配，Jarvis 用 4–5）。加配置项 `max_consecutive_errors`（Rust 与 TS 同步），默认仍为 3；AIWS 的阶段循环设为 6 |
| 6 | `run` 动作的 risk 按建议改 | 已实施：`run` 为 `destructive`，`analyze` / `code` 仍为 `write` |
| 7 | 了解 | 记为已知风险 |
| 8 | 好 | 维持 |

### 8.2 同步任务契约（Agent 宿主）

**存放**：AgentRoot 下 `sync/<task_id>/`：`task.json`（定义）、`program.ts`（当前版本）、`state.json`（程序自己的状态）、`runs.jsonl`（运行记录）、`out.json`（产出）。产出的 Zone 路径是 `/home/<owner>/agents/<agent_id>/sync/<task_id>/out.json`，就是数据源 `kind: zone` 的 `path`。

**程序**：一个 Deno 脚本。从 stdin 读 `{ task_id, inputs, secrets, state }`，向 stdout 写一个 JSON：`{ status: "ok" | "failed", error?, error_kind?: "transient" | "program" | "auth", business_time?, fields?, records, truncated?, state? }`。宿主用 `deno run --no-prompt --allow-net=<task.hosts>` 运行它：清空环境变量，不给读写文件的权限（只运行脚本本身），超时 120 秒，stdout 上限 32 MiB。宿主把结果包成 `aiws.sync.v1` 信封（`program: { name: task_id, version }`、`produced_at`）原子写入 `out.json`；失败时写 `status: failed` 的信封（重试都失败以后才写）。

**时间表**：`schedule.every_secs` 至少 900；宿主自己的时间表，服务重启后错过的槽合并成一次。

**失败**（B6-2）：`transient`（以及超时、网络错误）隔 30 秒、120 秒各重试一次；`program`（脚本异常、输出不合规）连续 3 次失败后通知 owner 一次（`notify.send`），由 owner 交给 Agent 修；`auth` 立即通知 owner。任务状态 `ok` / `failing` / `disabled`。

**Agent Service kRPC**（`/kapi/<agent_id>`）：

| 方法 | 参数 → 结果 | 谁能调 |
| --- | --- | --- |
| `sync.list` | `{}` → `{ tasks: [Task] }` | owner、owner 的 Agent |
| `sync.get` | `{ task_id }` → `Task` | owner、owner 的 Agent、AIWS 服务 |
| `sync.put` | `{ task_id, title, program, hosts, schedule, inputs?, secrets? }` → `Task`；改程序时 `version` 加一 | owner、owner 的 Agent |
| `sync.set_enabled` | `{ task_id, enabled }` → `Task` | owner、owner 的 Agent |
| `sync.delete` | `{ task_id }` | owner、owner 的 Agent |
| `sync.set_secret` | `{ task_id, name, value: string \| null }` → `Task` | 只有 owner 本人（不是 Agent 的会话） |
| `sync.try` | `{ task_id }` → `{ run, preview: { status, records, sample, error? } }`，不写产出 | owner、owner 的 Agent |
| `sync.run` | `{ task_id, wait_ms? }`（最多 120000）→ `{ run }`；停用的任务报错 | owner、owner 的 Agent、AIWS 服务（`appid` 为 `aiworkspace` 的系统会话，只放开 `sync.run` 和 `sync.get`） |
| `sync.runs` | `{ task_id, limit? }` → `{ runs: [Run] }` | owner、owner 的 Agent |

`Task`：`{ task_id, title, version, hosts, schedule, inputs, secrets: [{ name, set }], enabled, state, output: { path }, last_run?, failures }`。`Run`：`{ run_id, at, ended_at?, status: running | succeeded | failed, kind?, error?, records?, trigger }`。凭据值永远不出现在结果、日志和运行记录里。

**Agent 的命令**：`agent-session sync list|get|put|try|run|enable|disable|delete`，照 `agent-session skill` 的做法。手册 `aiworkspace_sync` 改写为：定义数据源 → 写程序 → `sync try` → `sync put`（装上即按时间表运行）→ 数据源加 `source.sync` → 首次同步 → 修复。凭据由 owner 在数据源的属性里填。

**AIWS 侧**：`kind: zone` 的来源可以带 `source.sync = { agent_id, task_id }`（只有 owner 能设，同其他来源字段）。有 `sync` 的数据源触发刷新（要 `run`）时，服务先经节点网关调这个 Agent 的 `sync.run`（等最多 120 秒），再读文件；Agent 不可达或任务停用时，这次刷新失败并说明执行环境不可用，数据停在最近一个有效版本。AIWS 自己的定时器照旧只重读文件。`meta.data.sync` 原样给出。Desktop 数据源属性：owner 看到任务（版本、时间表、域名、启用、最近一次运行、失败次数），能启用停用、立即同步、填写凭据（只写不读）；其他人看到 AIWS 记下的尝试，有 `run` 时能立即同步。

### 8.3 托管执行改为发起者自己的 Agent

- `params.execution = { mode: "hosted", agent? }`：`agent` 是发起者的一个 Agent（DID 或 AgentId）；不给时取发起者唯一的 Agent，没有或不止一个时报错。服务核对这个 Agent 属于发起者（system-config 里 `users/<发起者>/agents/<agent>/spec`），发起者必须是账号。
- 执行上下文：执行者是这个 Agent；AI 资源是发起者的（`aicc: { mode: current_user, user_id: <发起者> }`，Agent 用自己的会话调 AICC，记在它的 owner 即发起者名下）。权限按这个 Agent 判定：工作区没有单独配置它时等同发起者（R16）。
- 派发：照现在的做法，用发起者的会话在这个 Agent 上建 Goal。
- 去掉 owner 的托管配置：`ws.set_hosted_exec`、`ws.get_info.hosted_exec`、`allowed_wishes` / `triggerers` / `limits` 和对应的存储与检查；Desktop 的托管设置、权限审计里的对应部分。Desktop 的执行方式：“本浏览器”总有；查看者有 Agent 时加“使用我的 Agent”，用 MessageHub 的默认 Agent（与 Agent 面板相同）。
- 文档与手册：服务 README 的 Execution modes、内核文档 08、权限 PRD R30、《内核架构改进》D22 的实施注；手册 `aiworkspace_run`（托管运行就是发起者自己的 Agent 被派来执行，没有 owner 配置）。

### 8.4 第二轮实施记录（2026-11-16，未提交）

**同步任务（S1，Agent 宿主）**
- `lib_opendan/src/sync.rs`（新）：复用 Goal 检查脚本的 Deno 查找和时间表槽计算。`deno run --quiet --no-prompt --no-config [--allow-net=<hosts>]`，清空环境变量（只留 `NO_COLOR`、`DENO_DIR`），不给读写和子进程权限，120 秒、32 MiB。原子写 `out.json`；`runs.jsonl`、`state.json`，另有 `schedule.json` 存宿主自己的进度（上一个槽、连续失败数、是否已通知）。重试、通知、状态、900 秒下限、重启后槽合并、同一任务同时只有一个运行，都按 §8.2。凭据值在结果、日志、运行记录、错误、`try` 的样例里都换成 `[secret 名字]`。
- kRPC：`opendan/src/service.rs` 的 `sync.*`，按调用者种类鉴权；AIWS 服务（target 是系统服务 `aiworkspace`，主体 System 或 Device）只能 `sync.run` / `sync.get`；`sync.set_secret` 只认 owner 本人的人的会话（root、App、Agent 都拒绝）。宿主在 `loader.rs` 启动，`loader.status` 多一个 `sync` 模块。
- 凭据：Agent 的 settings（`buckyos-api` `AgentSettings.sync_secrets`），经 Loader 的 AgentRecords 读写（Loader 已经在写自己的 settings）。因为加了字段，control_panel 也要随之重建部署，否则它读改写 settings 时会丢掉这个字段。
- Agent 的命令：`agent-session sync list|get|put|try|run|enable|disable|delete`（`xagent_cli.rs`）。`sync.try` 可以带草稿（`program`、`hosts`、`inputs`、`secrets`），只跑一次、不写不记，给出样例、`kind`、脱敏的 stderr。
- 手册 `aiworkspace_sync` 按完整流程重写；`doc/opendan/protocol/Agent State Protocol.md` §11、`opendan/README.md`。

**AIWS 侧（S2）**
- `source.sync = { agent_id, task_id }` 只用于 `kind: zone`，带它时 `path` 必须正好是 `/home/<user>/agents/<agent_id>/sync/<task_id>/out.json`（内核校验时不知道 owner，没法自动补）；`meta.data.sync` 原样给出。
- 触发的刷新先调 owner 名下这个 Agent 的 `sync.run`（经 `Limits.agent_gateway`，节点上是本机网关 `/kapi`，独立后端 `--agent-gateway`），再读文件；只对当前 owner 名下的 Agent 调用（否则 owner 能借服务的会话触发别人的 Agent）。`sync.run` 的结果是失败时这次刷新直接失败，原因取运行的错误，不去读还没更新的旧文件；Agent 不可达、不属于 owner、任务停用等是 `SYNC_UNAVAILABLE`。定时器只重读文件。
- Desktop `ui/sources/SyncTaskSection.tsx`：owner 用自己的会话看任务（版本、时间表、域名、启用、最近一次运行、失败次数），能启用停用、立即同步、填写凭据（只写不读）；其他人看到 AIWS 的最近一次尝试，有 `run` 时能立即同步。`api/transport.ts` 加 `agentCall`。

**托管执行改为发起者自己的 Agent（H）**
- `wish/exec.rs` `starters_agent`：`execution.agent` 是 DID 或 AgentId，不给时取唯一的 Agent；没有、多个未指定、不是账号 → `EXECUTION_CONTEXT_MISSING`；别人的 Agent → `PERMISSION_DENIED`。Agent 列表来自 `auth.rs` 的 `agents_of`（Zone 上读 system-config，独立后端读静态令牌表里 `class: agent` 的条目）。
- 执行上下文的 executor 是这个 Agent，`compute` 与 AI 归属都是发起者（`current_user`）；运行以这个 Agent 的身份读写（`acting_as`），`MACRO_CANNOT_WRITE` 预检也按它判定。派发用发起者的会话。
- 删除 owner 的托管配置：`ws.set_hosted_exec`、`ws.get_info.hosted_exec`、`allowed_wishes` / `triggerers` / `limits`、`max_tool_rounds`、每日次数、`config_digest` 与对应的 `DELEGATION_STALE`；Desktop 的托管设置及其在权限编辑器、审计、worklog 里的引用。
- `store/acl.rs` 的 `judge`：Agent 按 owner 判定的条件从 `is_agent()` 改为 `acting_for.is_some()`，否则托管运行的 run token 什么都读不到；顺带修了 Agent 自己发起的本地运行的 run token。
- Desktop：执行方式“本浏览器”总有，查看者有 Agent（MessageHub 的默认 Agent，与 Agent 面板相同）时加“使用我的 Agent”，请求带 `execution.agent`。
- 记录的 `by` 和 Commit 的 `as_principal` 仍是发起者，授权按 Agent，`attribution.executed_by` 记 AgentId。派发细节给发起者和管理者看。
- 手册 `aiworkspace_run`、服务 README、内核文档 08 和 12、权限 PRD R30、《内核架构改进》D22。

**主会话**
- §8.1 第 3、5、6 项（见上表）。第 3 项实施时发现：CSV 里的整数解析成浮点（`1.0`），库里存的是整数（`1`），不只是按内容配编号失败，带主键的表每次刷新也会把这些行当作改过、写一个 Commit；已在比较前把整数形式的浮点数换成整数。
- xllm：Rust `agent_tool/src/xllm.rs`、TS `src/xllm/core/{config,types,run,prepare}.ts` 加 `max_consecutive_errors`（至少 1，记入 `RunLimits`，未配置不写），各加一个测试；`doc/llm_context/local_llm_context_protocol.md`、`xllm_rust_sdk.md`；websdk `dist/` 已重建。AIWS 的阶段循环设为 6。

### 8.5 第二轮验证

| 项 | 结果 |
| --- | --- |
| `cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -p buckyos-api -- --test-threads=1` | 828 例，只有 `activity` 两例在满载时启动超时，单独重跑通过 |
| `cargo test -p libopendan --test sync_tasks`、`-p opendan`、`-p libopendan --lib skills`、`--test xagent_cli skills_are`、`-p agent_tool --lib xllm` | 7、124、14、93 例全过；`cargo check -p control_panel -p scheduler --tests` 通过 |
| websdk `pnpm test:xllm` | 290 过（含新增 `config_limits.test.ts`） |
| Desktop `tsc -b`；e2e 全量 | 通过；406 过、6 跳过、1 失败。失败的 P3-12 是用例里的竞态：放映锁一过期 `ws.get_info` 就不显示，克隆要等清理器下一次运行才删，用例却立即检查；改为轮询（同一用例后面已经这样写），连跑 3 次通过 |
| 部署 | `uv run buckyos-build.py`、`./build_aios --local-test`、`uv run start.py`（要更新 control_panel：`AgentSettings` 加了字段）；三本手册 `docker cp` 进 browsy |
| 真实 Zone DV | `[sync]`（新增）、`[sources]`、`[jarvis]`、`[macro]` 系列 6 条全部通过 |

`[sync]`：owner 在 browsy 上装一个探测任务；声明了凭据却没填时运行失败（`kind: auth`）；owner 填入凭据后，`sync.get` 只显示“已填”；数据源带 `source.sync`，刷新经 AIWS 服务调 `sync.run`，变为 `valid`，程序版本是这个任务；程序读不到环境变量和文件、连不上没声明的主机（都是 `NotCapable`），声明的主机返回 200，凭据送到了程序；任务目录里的文件都不含凭据值；停用任务后刷新报 `SYNC_UNAVAILABLE`，数据停在上一个版本。`[macro-hosted-agent]` 按新形式（发起者自己的 Agent）通过。`[macro]` 第一次因宏里调真实模型超过 120 秒的程序时限失败，重跑通过。

没做的：时间表按周期自动运行（等一个周期，没在 DV 里等）、Agent 在对话里用 `agent-session sync` 自己写程序装任务（手册已写，命令有单测）、协作者用自己的 Agent 跑托管运行、手机真机。

### 8.6 第二轮新出现、需要 owner 确认的

2026-11-16 owner 答复“可以”：下面五项都按现状接受。

1. 凭据明文存在 system-config 的 Agent settings 里：Agent 自己读得到（已定的已知风险）；另外现有 RBAC 的 `agent_read_write`（`obj://*`）让 Zone 里任何读写模式的 Agent 都读得到，这比“Agent 自己读得到”更宽。是否接受，还是要先收紧 `agent_read_write`？
2. 托管运行的历史记录 `by` 记发起者，Commit 按 Agent 授权、`attribution.executed_by` 记 Agent。`by` 要不要改成 Agent？
3. 查看者有多个 Agent 时，“使用我的 Agent”固定用 MessageHub 的默认 Agent，没有选择器。要不要加？
4. Limited 账号如果有 Agent，现在也能发起托管运行。要不要禁止？
5. 同步任务的时间表按纪元对齐（同 Goal），装上以后第一次按计划运行最晚在一个周期之后，首次数据靠数据源的“立即同步”。遇到暂时性失败时一次运行最长约 5 分钟（两次重试），AIWS 只等 120 秒，之后读到的是旧文件。可以吗？
