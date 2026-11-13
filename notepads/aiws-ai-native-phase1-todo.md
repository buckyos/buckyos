# AIWS AI Native 第一期实施 TODO：许愿格三步与宏的边界

状态：2026-11-12 立项；同日按本单实施（未提交），前置的浏览器优先 M1–M2 一并实施（见其 §7）。§5 的六项按“建议”列执行，Q6 未定所以 T6-2 未做。实施记录与偏差在 §7。

依据：

- [AI Native 内核升级 PRD](aiws-ai-native-kernel-upgrade-prd.md) 的 §5、§6、§11.5，以及 owner 2026-11-12 的裁决。
- 第二期（可扩展数据源，PRD §7）不在本单。
- 按裁决 5，实现排在[浏览器优先规划](aiworkspace-wish-browser-first-todo.md) M1–M2 之后，M0 可以先做。

现状是 2026-11-12 对 buckyos `ee426e1e9` 的源码静态核对，没有运行测试。

日期：2026-11-12

## 0. 目标与范围

本单交付：

1. **三个阶段。** 许愿格分为分析、编码、运行。编码产出宏版本和试运行候选；运行只执行宏，没有模型循环。
2. **宏的边界。** 字段引用按 ID 固定；集合计算写成 SQL；宏可以直接调用 AICC；不访问 Zone 外资源；产物里的代码在编码时固定；运行时不再提供整棵数据树。
3. **状态。** 数据上能区分 PRD §5.8 的主状态。界面只做到能区分，交互在实现后再调（PRD §3）。
4. **宏编写指南。** 编码提示词、托管手册和人工编写宏共用这一份。
5. **分析交接。** Jarvis 分析的产物与 xllm 分析的产物是同一类。
6. **对象入口的第一批动作（xcall）。** 前提是先定审批门。

不做：

- 第二期的全部内容：同步程序、数据源定义接口、“尚未取得数据”的状态。
- 自动运行与自动应用。
- 收紧 run token（裁决 4）。
- 浏览器执行器接入 xllm TS（浏览器优先 M3）。
- 界面交互细节。
- 宏的受限语言（PRD §6.7 的 B）。

前置：

- 浏览器优先 M1–M2（裁决 5）。M1 默认本地执行，加上 `llm_cache` 和预算下发；M2 下线服务内执行，删除服务内的 Deno 运行器。
- 核心 API 首批（已提交）。

### 0.1 已定的裁决

| # | 裁决 | 来源 | 落点 |
| --- | --- | --- | --- |
| D1 | 分析、编码、运行三步，不合并 | owner 2026-11-12，PRD §5.7 | §2.1 |
| D2 | 宏用 JS，集合计算写成 SQL | 裁决 1，PRD §6.7 | §2.3 |
| D3 | 宏可以直接调用 AICC，由程序构造提示词、取得分析结果；要有宏编写指南 | 裁决 2，PRD §6.4 | §2.4、§2.8 |
| D4 | 不是不联网，而是不访问 Zone 外资源 | 裁决 3，PRD §6.6 | §2.6 |
| D5 | run token 先能力再安全，暂不收紧；风险列在 PRD §6.5 | 裁决 4 | §2.6 |
| D6 | 先做浏览器优先 M1–M2，再做本单 | 裁决 5 | §0 |
| D7 | 首版只做手动运行和预览 | PRD §10 | — |
| D8 | 试运行直通首次运行；契约失效用确定性规则判定；引用按 ID 固定；产物代码固定 | PRD §11.5，按建议执行 | §2.1–§2.5 |

## 1. 现状与缺口

路径相对 `src/frame/aiworkspace/`，Desktop 指 `src/frame/desktop/src/app/aiworkspace/`。

| 范围 | 现状 | 本单 |
| --- | --- | --- |
| 阶段 | `proc.start { program: "wish.xllm@1", params.stage }` 有四种阶段：`analyze`、`execute`、`rerun_program`、`repair_program`（`server/src/wish/mod.rs` `start`，`stages.rs` `open_stage` / `drive_inner`）。阶段记在共用 `runs` 表的 `params_json.stage`；没有 `wish.*` 方法，`proc.*` 都是 experimental | 改为三种：`analyze`、`code`、`run`（§2.1） |
| 分析的确认 | 分析就绪后，Desktop 自动写回（`ui/wish/WishService.ts` `finishAnalysis`），没有单独的确认步骤。执行前，`ready_to_execute` 检查分析是否过期、引用的文档是否变化、状态是否 `ready` | 保留自动写回；用户发起编码即视为确认（§2.1） |
| 程序 | 许愿格的 `program { language, api_version: 2, source, digest, produces }`（`core/src/wish.rs` `check_program`），在候选被应用时写入（`store/src/wish/plan.rs`）。只重跑时不检查输入结构（`stages.rs` `rerun_prepare`） | 宏版本 = 程序 + 声明清单（§2.2） |
| 直接书写 | 执行阶段的工具 `put_result` 写 `approach: direct` 的结果（`server/src/wish/tools.rs`）；分析的输出契约区分 program 和 direct（`aiws/prompts/analyze.md`）。只重跑时，plan.rs 保留旧的直接结果，freshness 给出 `direct_stale`，界面提示“文字部分需要重新生成” | 去掉 direct，所有结果都由宏产出（§2.1、§2.4） |
| 字段引用 | `rows.jsonl` 按 `field_id` 存值，字段名取自物化当时的 `schema.json`（`store/src/wish/context.rs` `materialize_entity`，`aiws/aiws.js` `#allRows`）。字段改名后取到 `undefined`，不报错 | 按清单物化，用编码时的名字（§2.2） |
| 集合计算 | 程序只能在 JS 里过滤、排序（`Input.rows`）；SQL 只有模型用的阶段工具 `ws_query` | `aiws.sql`（§2.3） |
| 宏内 AI | 只有 `aiws.llm.map`。服务端 `server/src/wish/llm.rs` 调用 AICC：缓存是 `<ws>/cache/llm_map/<key>.json`；项数上限 2000，缓存命中也计入；`max_completion_tokens` 写死 4096。托管位置没有桥。浏览器优先 M1 会删掉服务端的 `llm_map`，改由执行器直接推理并使用 `llm_cache`，浏览器一侧的实现在它的 M3 T3-1 | `aiws.ai.complete`（§2.4） |
| 数字检查 | `uncited_numbers`（`store/src/wish/results.rs`）只检查直接书写的结果，只给警告 | 改为检查宏产出的文字（§2.4） |
| 网络 | 服务内 Deno 用 `--allow-net`，不限地址（`server/src/wish/runner.rs`，M2 删除）。浏览器沙箱的 CSP 是 `default-src 'none'`，Worker 覆盖了 `fetch`（Desktop `executor/programHost.ts`、`program.worker.ts`）。浏览器里 `#trackFetch` 把每个失败的 fetch 也记成外部数据 | 以 Zone 为界（§2.6） |
| 托管程序 | CLI 的 `run-program` 不运行程序，只上报。程序由 Agent 用 `deno run -A` 运行（`aiworkspace_run` 手册），能读到上一级目录的 `claim.json`（含 run token），`llm.map` 不可用 | 受限运行器（§2.6） |
| HTML 结果 | 由程序的 `result.html` 或 `put_result` 产出。`plan_html` 在代码变化时执行 `set_keys html` 并把版本加 1；预览 iframe 不执行脚本 | 代码在编码时固定（§2.5） |
| 运行时的树 | 快照目录里有 `context/data-tree.json`、`block-tree.json`，程序 API 有 `aiws.resolve` / `find` / `outline` | 去掉（§2.6） |
| 状态 | `core/src/freshness.rs` `wish_freshness` 给出 `status`、`needs_analysis`、`config_changed`、`direct_stale` 等；程序版本只经 `config_digest` 间接起作用 | 增加“需要编码”和主状态（§2.7） |
| 界面 | 按钮有分析、执行、只重跑程序、让 AI 修程序（Desktop `ui/wish/WishPanel.tsx`）；画布上的“运行”只会分析或执行，从不只重跑 | 改为分析、编码、运行 |
| 分析交接 | 托管 Agent 可以领取分析运行，用 `proc.report { analysis }` 交付（`stages.rs` `provided_analysis`） | §2.9 |
| 对象动作 | 没有（核心 API R15） | §2.10 |

## 2. 设计

### 2.1 三个阶段

| 阶段 | 起点 | 模型循环 | 工具 | 产物 | 应用后写入 |
| --- | --- | --- | --- | --- | --- |
| `analyze` | 原始需求 | 有 | 读工具、`submit_analysis` | 任务契约候选 | `analysis`、`inputs`（不变） |
| `code` | 已写回且就绪的任务契约；可以带 `parent_run_id`（失败的运行或上一轮候选）和 `feedback` | 有 | 读工具、`run_program`、`check_results`、`finish`；没有 `put_result` | 宏版本和试运行候选 | 一个 Commit 同时写宏版本和结果 |
| `run` | 已保存的宏 | 无 | 无；程序经桥调用 SQL 和 AICC | 候选 | 结果 |

- **阶段名。** 改为 `analyze`、`code`、`run`；删除 `execute`、`rerun_program`、`repair_program`，不留兼容（开发阶段，见 AGENTS.md）。修程序用 `code` 加指向失败运行的 `parent_run_id`；反馈轮用 `code` 加 `parent_run_id` 和 `feedback`。这两种情况都从现有程序出发。
- **确认。** 分析就绪后照旧自动写回，用户发起编码就是确认任务契约。以后要不要合并成一次点击，属于交互，后调（§5 Q3）。
- **试运行直通。** 编码的候选就是首次运行的候选，应用它时宏和结果一起写入，不必再跑一次。用户不应用，宏也不保存，这与今天程序随候选写入的做法一致（§5 Q4）。
- **就绪检查。**
  - `code`：分析存在、没有过期、状态是 `ready`，与现在的 `ready_to_execute` 相同。
  - `run`：上述三条，加上有宏、`api_version` 匹配、结构检查通过（§2.2）。不满足时返回 `NEEDS_CODING`（新子码），并带上原因。
- **去掉直接书写。** 输出契约不再有 `approach`。删除 `put_result`、plan.rs 里保留旧直接结果的逻辑、`Collector.program_only`、`direct_stale` 及其界面提示。分析的 schema 升到 `wish.analysis.v3`，分析提示词同步修改。

### 2.2 宏版本与声明清单

- **存放。** 沿用许愿格的 `program` 键，`api_version` 升到 3，增加 `manifest`。不新起键，`GUARDED_KEYS`、导出导入和 Fork 都不用改。`digest` 改为覆盖源码和清单；`config_digest` 本来就包含它，所以宏一变，结果就过期。
- **清单从哪里来。** 由程序自己在开头声明（`aiws.declare({...})`，名字 M0 定），aiws.js 把声明写进 `results.json`。试运行时，宿主按编码时的 schema 把字段名解析成 `field_id`，写进候选的宏版本。声明和程序在同一个文件里，模型改程序时不会漏改清单；运行时由 aiws.js 和宿主按清单强制。
- **清单内容**（PRD §6.6）：

  ```json
  { "inputs": { "sales": { "entity_id": "…", "fields": [{ "field_id": "f3", "name": "月份", "type": "date" }] } },
    "results": { "chart": { "type": "html", "code_digest": "…" } },
    "ai": { "complete": { "max_calls": 20, "max_tokens": 20000 }, "map": { "max_items": 500 } },
    "zone": ["…"], "params": ["today", "timezone"], "resources": { "timeout_secs": 120 } }
  ```

- **运行阶段的物化。**
  - 只物化清单里的输入和字段。
  - `schema.json` 里的名字用编码时的名字；现在 `materialize_entity` 用的是当前名字。
  - `rows.jsonl` 照旧按 `field_id` 存，不变。
- **编码阶段的物化。** 物化全部字段，因为模型需要看到全貌。程序里的声明照样生效，所以试运行和运行的行为一致。
- **未声明的字段。** aiws.js 返回的行对象被读取未声明的字段时抛错（用 Proxy 实现）；SQL 逻辑表里也没有这一列。所以编码时的试运行就会暴露问题，运行时不会静默出错。
- **结构检查。** 在运行前做，状态计算也要用到，放在 core，服务和 WASM 共用。检查内容：声明的每个 `field_id` 仍然存在，类型没变；输入实体仍然存在。改名不算变化。不通过时列出具体字段。
- **纯文本呈现。** 清单的文字形式写进许愿格的 `body`（core `body.rs`），Agent 和读屏读到的是同一份。

### 2.3 `aiws.sql`

- **接口。** `await aiws.sql(query, params?)` 返回 `{ columns, rows, truncated }`。
  - 表名用输入名。
  - 列是清单里的字段，列名用编码时的名字；另有 `_id`，原生值用 `"<名字>#raw"`，与核心 API 的约定一致。
- **执行。** 新增宿主工具 `sql`（`proc.tool`，返回 JSON）；给模型看文本的 `ws_query` 保留。服务端在阶段快照上调用 `store/src/sql.rs` 的 `execute`，`tables` 为 `{ 输入名: entity_id }`。列由 core `sqlview.rs` 的 `logical_table_ddl` 新加的列映射参数生成：只含清单里的字段，名字用编码时的。编码阶段的试运行走同一条路径。
- **视图输入。** `table_view` / `table_query` 输入在 SQL 里是这个视图显示的行（外面包一层 `compile_view` 生成的 SQL），列仍按清单。细节在 M0 定。
- **预算。** 单条语句沿用执行器的 2 秒和行数、字节上限；每次运行另有调用次数上限，经 `brief.budget` 下发。
- **读集。** 执行器返回的 `read` 记入读集，`ws_query` 现在已经这样做。

### 2.4 宏调用 AICC

- **接口。** `await aiws.ai.complete({ system?, prompt | messages, schema?, model?, max_tokens? })`：没有 `schema` 时返回 `{ text }`，有 `schema` 时返回 `{ value }`。名字在 M0 定。`llm.map` 保留，两者共用预算和缓存。宏里只做单次推理，不开放工具调用和 Agent 循环（PRD §6.4）。
- **在执行器侧调用。** 与浏览器优先 Q1 方向一致，服务端不调用模型：
  - 浏览器用发起者的 AICC 会话（`buckyos/xllm` 的 `LlmClient.infer`，与浏览器优先 T3-1 用同一个客户端）。
  - 托管位置用 Agent 自己的 AICC。
- **缓存。** 经 `proc.tool llm_cache`（浏览器优先 T1-5）。键是 `sha256(model, system, prompt/messages, schema, max_tokens)`，只缓存成功并且通过 schema 校验的结果。输入不变时重跑会命中缓存，得到同一段文字。
- **预算。** 在浏览器优先 T1-4 的 `brief.budget` 上增加 complete 的调用次数和 token 上限；取清单声明值与工作区上限中较小的一个。缓存命中是否计入预算，在 M0 定。现在 `llm.map` 是计入的。
- **记录。** `results.json` 增加 AI 调用的统计（调用次数、命中缓存次数、token），写入候选和运行记录。结果的 `derived.model_judgment` 在宏调用过 AICC 时置位，`llm.map` 现在已经这样做。
- **数字检查。** `uncited_numbers` 改为检查宏产出的文字结果（`richtext`、`record`）。已知数字来自 `facts` 和程序产出的表格结果。仍然只给警告。
- **错误。** 额度拒绝沿用 `AI_QUOTA_*` 子码。aiws.js 不再丢弃服务端给的 `code`（现在 `llm.map` 会丢掉）。

### 2.5 产物里的代码在编码时固定

- **编码。** `result.html` 的 html、css、js 按结果名记录 `code_digest`，写入清单。
- **运行。**
  - 程序再次产出的 HTML 代码，digest 与清单不一致时，这次运行失败，返回 `MACRO_CODE_CHANGED`，要求重新编码。
  - 一致时，`plan_html` 只更新 `bindings` 指向的数据，不再 `set_keys html`。
  - 数据要经 `bindings` 交给页面，不能拼进代码。宏编写指南写明这一点。
- **确认。** 首次写入的确认就是应用编码候选。预览本来就不执行脚本（`WishPreviews.tsx`）。
- **不受影响的结果。** 组合控件和内置控件的结果（`control`）只带数据和配置，不受这条约束。

### 2.6 执行边界

| 位置 | 运行方式 | 网络 | AICC | SQL |
| --- | --- | --- | --- | --- |
| 浏览器 | 现有沙箱：sandbox iframe 加 Worker | 只能访问 Zone：经宿主桥（§5 Q2） | 宿主桥，用发起者的 AICC | 宿主桥 → `proc.tool sql` |
| 托管 | CLI 起受限的 Deno 子进程：`--allow-read=<work>`、`--allow-write=<work>/output`、`--allow-net=<Zone 主机>`、清空环境变量。`claim.json` 在 work 目录之外，程序读不到 | 同左 | stdio 桥，用 Agent 的 AICC | stdio 桥 → `proc.tool sql` |
| 服务内 Deno | 浏览器优先 M2 删除 | — | — | — |

- **Zone 内资源的身份与机制**（§5 Q2）。
  - 建议经宿主桥（`aiws.zone.fetch`，名字 M0 定），用发起者（浏览器）或 Agent（托管）的会话；地址限定在本 Zone 的主机；访问过的地址记入运行记录，沿用今天 `external` 的记录路径。
  - 不建议直接放开沙箱的 CSP：沙箱是不透明源，直接发的请求带不上身份，只能读到公开资源。
- **修正 `#trackFetch`。** 浏览器里失败的 fetch 不再记成外部数据。
- **运行时收窄。** 去掉 `aiws.resolve` / `find` / `outline`，快照里不再带 `data-tree.json` 和 `block-tree.json`。模型用的阶段工具不受影响。
- **run token。** 不收紧（D5）。受限运行器不把 token 交给程序，这项风险因此限定在执行者（Agent）一侧。

### 2.7 状态

- **需要编码。** core freshness 增加 `needs_coding { reasons }`，原因包括：没有宏、`api_version` 不匹配、声明的字段被删除、字段类型变化、输入不存在、上次运行 `PROGRAM_FAILED`。同时去掉 `direct_stale`。
- **主状态函数。** 放在 core，服务和 WASM 共用。按 PRD §5.8 的顺序给出 `{ main, next_action }`。“等待数据”在首版只用现有的状态（输入 `unavailable`、Source 刷新失败）；“尚未取得有效数据”随第二期加入。
- **Desktop。** 主状态和下一步动作各一处，详情里保留现有的各个维度。画布上的“运行”改为执行下一步动作。

### 2.8 宏编写指南

- **一份来源。** 放在 aiws.js 旁边，位置在 M0 定。
  - 编码提示词把它整段嵌入。现在 `execute_system` 嵌入的是 `AIWS_API`，改成嵌入这份指南。
  - `aiworkspace_run` 手册引用它，不复制内容。
  - `aiws/README.md` 的程序 API 部分并入指南。
- **内容。**
  - 程序 API v3 和 `declare`。
  - SQL 写法：输入名作表名，显示值与 `#raw`，预算，大表 JOIN 把小的一侧写成物化 CTE，不支持递归 CTE。
  - AICC 调用：文字里的数字来自 `facts`；输入数据与指令分开；输出用 schema 约束；缓存与预算；拒绝时的子码。
  - Zone 内资源怎么访问。
  - HTML 结果：代码固定，数据经 `bindings` 传给页面。
  - 不能做的事：访问 Zone 外资源、自己取时钟（用运行参数）、写工作区。

### 2.9 分析交接（PRD §9“分析交接”、§8.3）

- **已有路径。** 托管 Agent 领取分析运行，用 `proc.report { analysis }` 交付；编码只读写回的任务契约，不关心分析者是谁。
- **缺口。** Jarvis 在对话里分析时，需要由 Agent 自己发起一次分析运行并执行。浏览器优先 T1-3 之后，本地模式要求发起者是 Account，所以 Agent 只能经托管派发（§5 Q5）。
- **建议。** 给 CLI 加 `run-start`，允许 Agent 以本地执行者的身份发起分析运行，AI 用它自己的额度。分析结果仍走 `submit_analysis`，产物与 xllm 分析的相同。

### 2.10 对象入口的第一批动作

- **范围。** 启动许愿格的分析、编码、运行，即 `proc.start` 的三种阶段。触发数据源刷新放在第二期。
- **前置。** 先由[权限优化 TODO](aiws-permission-optimization-todo.md) B-6 定审批门。没定之前不做本节，其余部分不受影响。

## 3. 实施任务

### M0：契约（只改文档和 Schema）

- [ ] **T0-1** 阶段契约：
  - `proc.start` 的三种阶段及参数，各阶段的工具表，`host_open` 的 brief（`code` 阶段不再有 `put_result`）。
  - 新子码 `NEEDS_CODING`、`MACRO_CODE_CHANGED`，写进 `schemas/error-codes.json`。
  - README 的 Wish 一节。
- [ ] **T0-2** 宏版本：
  - `program` 的 v3 格式与清单 Schema。
  - `digest` 的覆盖范围；`check_program` 的规则。
  - 现有 v2 程序怎样处理：建议直接要求重新编码（§6）。
- [ ] **T0-3** 去掉 direct：
  - `wish.analysis.v3` 的输出契约。
  - 现有直接结果的去向（§6）。
- [ ] **T0-4** 程序 API v3：
  - `declare`、`sql`、`ai.complete`、`zone.fetch` 的签名和错误。
  - 删除的接口。
  - 同步 `aiws.d.ts`。
- [ ] **T0-5** 预算字段（在浏览器优先 T1-4 的 `budget` 上扩展）、`llm_cache` 的键格式、默认值。
- [ ] **T0-6** 主状态函数的输入输出；`needs_coding` 的原因枚举。
- [ ] **T0-7** 托管受限运行器：CLI 动词、Deno 参数、stdio 桥的消息格式。
- [ ] **T0-8** 宏编写指南的位置和目录。
- [ ] **T0-9** 确认 §5 的各项。

### M1：core 与 store

- [ ] **T1-1** core：
  - `check_program` v3 与清单校验。
  - `wish.analysis.v3`，去掉 `approach`。
  - `basis_digest` / `config_digest` 同步。
  - 阶段名常量。
- [ ] **T1-2** core `sqlview.rs`：`logical_table_ddl` 增加列映射（字段子集，用指定的名字），视图输入包一层；对照测试。
- [ ] **T1-3** core：结构检查 `macro_check` 和主状态函数；freshness 加 `needs_coding`，删 `direct_stale`。改完重建 Desktop 的 WASM。
- [ ] **T1-4** store `context.rs`：
  - 运行阶段按清单物化，字段名用编码时的名字。
  - 快照里不再写 `data-tree.json` 和 `block-tree.json`。
  - 清单写进许愿格的 `body`。
- [ ] **T1-5** store：宿主工具 `sql`，在阶段快照上按清单建表，返回 JSON，记读集，有调用次数上限。
- [ ] **T1-6** store `plan.rs` / `results.rs`：
  - 删除保留直接结果的逻辑和 `program_only`。
  - `plan_html` 校验 `code_digest`，运行时只更新 `bindings`。
  - `uncited_numbers` 改为检查宏产出的文字。
  - 收集清单（`declare` 的输出），把字段名解析成 `field_id`。

### M2：服务端阶段

- [ ] **T2-1** `mod.rs` `start` 和 `stages.rs`：
  - 三种阶段的分派与就绪检查（`code` 用现在的 `ready_to_execute`；`run` 另加 `macro_check`）。
  - `host_open` 的 brief：`code` 是模型阶段，`run` 是 `{ kind: "program", source, manifest, budget }`。
- [ ] **T2-2** `tools.rs`：
  - 删除 `put_result`。
  - `finish` 要求至少一次成功的试运行，并且有清单。
  - 候选带上宏版本，`mode` 为 `code` 或 `run`。
- [ ] **T2-3** `budget` 增加 SQL 和 AICC 的上限；`llm_cache` 支持 complete 的键。
- [ ] **T2-4** 契约测试：`fixtures/protocol` 重录；README 的方法表和错误码。

### M3：执行器

- [ ] **T3-1** `aiws/aiws.js` v3：
  - `declare`，未声明的字段抛错。
  - `sql`、`ai.complete`、`zone.fetch` 经桥调用。
  - 删除 `resolve` / `find` / `outline`。
  - 修正 `#trackFetch`；保留拒绝的 `code`。
  - `aiws_test.js`、`aiws.d.ts` 同步。
- [ ] **T3-2** 浏览器：
  - `program.worker.ts` 和 `programHost.ts` 增加三个桥及消息校验。
  - `localExecutor.ts`：`sql` 走 `proc.tool`；`ai.complete` 和 `llm.map` 在浏览器里直接推理，并读写 `llm_cache`（依赖 §5 Q1）。
  - `zone.fetch` 用发起者的会话，只允许本 Zone 的地址。
- [ ] **T3-3** 托管（websdk `cli/modules/aiws.ts`）：
  - 新动词（例如 `run-exec`）以受限 Deno 运行程序，stdio 桥接 `sql`（`proc.tool`）、`ai.complete` / `llm.map`（Agent 的 AICC 加 `llm_cache`）和 `zone.fetch`。
  - `run-program` 改为调用它之后再上报。
  - 补 CLI 测试。

### M4：Desktop

- [ ] **T4-1** `WishService.ts`：三种阶段；修程序和反馈轮改走 `code`；画布上的“运行”执行下一步动作。
- [ ] **T4-2** `WishPanel.tsx` / `WishCandidate.tsx` / `wishBlock.tsx`：
  - 主状态和下一步动作。
  - 按钮改为分析、编码、运行。
  - 删除 `direct_stale` 的提示和 approach 标签。
  - 中英文 i18n 同步。
- [ ] **T4-3** `pnpm check`、lint、build。

### M5：指南、提示词、手册、文档

- [ ] **T5-1** 宏编写指南（§2.8）；`aiws/prompts/execute.md` 改为编码提示词并嵌入指南；`analyze.md` 去掉 direct。
- [ ] **T5-2** 手册：
  - `aiworkspace_run`：三种阶段；运行程序用新的 CLI 动词，不再 `-A`；`llm.map` 和 AICC 可用。
  - `aiworkspace_template_dev`：`proc.start` 的阶段名。
- [ ] **T5-3** 文档：
  - 内核 [08](../doc/workspace/kernel/08-任务执行与AI.md)：程序隔离一节改为“以 Zone 为界”；许愿格的三个阶段。
  - [12](../doc/workspace/kernel/12-接口治理与验收.md)：方法、错误码、版本。
  - 服务 README。
  - [许愿格详细设计](<../doc/workspace/BuckyOS AI  Workspace 许愿格详细设计.md>) 按 PRD §2.2 回写：§8.2、§9.2、§9.3、§18。

### M6：分析交接与对象动作

- [ ] **T6-1** Agent 发起分析运行（§2.9，依赖 §5 Q5），更新手册，做真实 Zone DV。
- [ ] **T6-2** 对象入口声明许愿格三个阶段的启动动作（§2.10，依赖 B-6）。

### M7：测试与验收

- [ ] **T7-1** 改写受影响的测试：
  - `server/tests/wish.rs`、`quality.rs`：q09 改为经 `code` 修程序；q10 的 HTML 结果改由程序产出。
  - `wish_control.rs`、`store/tests/wish.rs`。
  - Desktop `tests/aiworkspace/wish.spec.ts`，以及 `objects.spec.ts` 的 OX03 / OX13。
- [ ] **T7-2** 新增 §4 的用例。
- [ ] **T7-3** 真实 Zone DV：浏览器和托管两个位置，各走一遍分析 → 编码 → 应用 → 数据更新 → 运行；手机上走一遍运行。

## 4. 验收场景

| 场景 | 通过条件 |
| --- | --- |
| 三步 | 分析写回后才能编码；编码的候选应用后宏和结果都已写入；运行不调用模型循环，只有声明过的 AI 调用 |
| 试运行直通 | 应用编码候选后不需要再运行；宏和输入都没变时，运行得到与试运行相同的数据结果 |
| 字段或表改名 | 运行结果不变，不需要重新编码 |
| 字段删除或类型变化 | 运行前返回 `NEEDS_CODING` 并列出字段；状态显示“需要编码” |
| 需求或口径变化 | 状态显示“需要分析”；`code` 被拒绝 |
| 程序读取未声明的字段 | 编码时的试运行报错，信息里有字段名；不会静默得到空值 |
| `aiws.sql` | 只能查声明的输入表，列只有清单里的字段；越权的表、内核表、写语句都被拒绝；读到的表记入读集 |
| 宏调用 AICC | 输入不变时重跑命中缓存，文字相同；超出预算被拒绝；额度拒绝给出 `AI_QUOTA_*`；文字里有无来源的数字时出现警告 |
| 解读随数据更新 | 数据更新后运行，表格、图表和解读对应同一份输入快照，不再出现“文字部分需要重新生成” |
| Zone 外资源 | 浏览器和托管位置访问 Zone 外的地址都被拒绝；访问 Zone 内的地址成功，并记入运行记录 |
| 托管受限运行 | 程序读不到 work 目录以外的文件（包括 `claim.json`），拿不到 run token，环境变量为空 |
| HTML 代码 | 运行时代码与清单不一致，返回 `MACRO_CODE_CHANGED`；数据变化只更新 `bindings`，不改控件定义的版本 |
| 运行时收窄 | 程序 API 里没有 `resolve` / `find` / `outline`，快照里没有整棵树 |
| 状态顺序 | PRD §5.8 的九个主状态按顺序取第一个成立的；运行失败不遮住已应用结果的过期 |
| 分析交接 | Agent 发起并完成的分析，与 xllm 的分析写回同一种 `analysis`，编码不区分来源 |

## 5. 需要 owner 确认

| # | 事项 | 建议 |
| --- | --- | --- |
| Q1 | 浏览器优先 M1 的 T1-5 删掉服务端的 `llm_map`；浏览器一侧的直接推理（T3-1 `llmMap.ts`）在 M3，而 M3 要等 xllm TS 的 M6。照原计划，M1 之后浏览器里的 `llm.map` 会一直不可用，直到 M3 | 把 T3-1 提到 M1 一起做。直接推理只要 `buckyos/xllm` 的 `LlmClient`，不需要 Worker 宿主。本单的 `ai.complete` 复用它。需要同步修改浏览器优先 TODO |
| Q2 | 宏以什么身份、经什么机制访问 Zone 内资源 | 经宿主桥，用发起者或 Agent 的会话，地址限定在本 Zone，访问记入运行记录（§2.6） |
| Q3 | 分析就绪后自动写回，用户发起编码即视为确认 | 照此执行；以后是否合并为一次点击，属于交互，后调 |
| Q4 | 编码候选不应用，宏也不保存 | 照此执行（与今天一致）；首版不做“只保存宏、不要结果” |
| Q5 | Jarvis 在对话里分析时怎样交付 | CLI 加 `run-start`，允许 Agent 以本地执行者身份发起分析运行（§2.9） |
| Q6 | 对象入口动作的审批门（权限优化 TODO B-6，选项 A / B / C） | 定下来之前 M6 的 T6-2 不做 |

## 6. M0 要定的细节

1. `declare`、`ai.complete`、`zone.fetch` 的名字和签名，清单的 JSON Schema。
2. 已有许愿格的处理：v2 程序直接要求重新编码；已应用的直接结果（文字）不再由宏产出，第一次编码后列为 `missing`，保留实体，不再管理。不做数据迁移，开发阶段允许 breaking change。
3. 预算的默认值和上限；缓存命中是否计入预算。
4. 视图输入在 SQL 里的表示。
5. 宏编写指南放在哪里、用什么语言（提示词现在是英文）。
6. 托管运行器的 CLI 动词名和 stdio 消息格式；Zone 主机列表从哪里取。
7. `needs_coding` 的原因枚举和主状态的取值名。

## 7. 实施回写

2026-11-12 实施，未提交。§5 Q1–Q5 按建议执行；Q6（B-6 审批门）未定，T6-2 未做。

### 7.1 落点

| 范围 | 文件 |
| --- | --- |
| 设置 | `kernel/buckyos-api/src/aiworkspace_client.rs`：删 `deno`、`stage_timeout_secs`、`program_memory_mb`、`allow_service_execution`；加 `complete_model`、`ai_max_calls`、`ai_max_tokens`、`sql_max_calls`、`snapshot_download_max_bytes`；`default_execution` 默认 `local` |
| core | `wish.rs`（v3 分析、`check_program` v3 + `check_manifest`、`macro_digest`、`macro_check`、阶段常量）；`freshness.rs`（`needs_coding`、删 `direct_stale`、`main_status`）；`sqlview.rs`（`manifest_table_ddl`）；`body.rs`（`manifest_text`）；`wasm/src/lib.rs`（`wish_main_status`），Desktop 的 WASM 已重建 |
| store | `wish/context.rs`（`StageKind` = Analyze / Code / Run；运行按清单物化、用编码时的名字；不再写 data-tree / block-tree，运行阶段不写 WORKSPACE.md；`macro_sql`、`input_fields`）；`sql.rs`（`execute_views`）；`wish/results.rs`（去 direct / program_only；`resolve_manifest`、`code_changed`、AI 统计、宏文字的数字检查）；`wish/plan.rs`（去 keep_direct；程序存 v3 + manifest；运行时 `plan_html` 不改定义）；`workspace.rs`（删 `wish_aicc_mode`） |
| 服务 | `server/src/wish/mod.rs`（三阶段、启动时就绪检查 `ready`、`run_attribution` 按执行位置）；`exec.rs`（Mode 只剩 local / hosted；`proc.report` 不再收结果；`proc.files` 超限置 failed）；`stages.rs`（整文件改为阶段宿主：brief 带 `budget`、`sql` / `llm_cache` 工具、运行阶段的判定 `judge_run`）；`tools.rs`（删 `put_result` 与 AgentTool 实现）；删 `llm.rs`、`runner.rs`、`wish-host` 路由、`ws.set_wish_aicc`；`proc.*` 升 stable；`Cargo.toml` 去 `agent_tool` / `llm_context` |
| 程序宿主 | `aiws/aiws.js` v3（`declare`、Proxy 拦截未声明字段、`sql`、`ai.complete`、`llm.map`、`zone.fetch` 都经 `__aiwsBridge`；删 `resolve` / `find` / `outline`；`#trackFetch` 只记成功的请求）；`aiws/run.js`（`--bridge` stdio 桥）；新增 `aiws/host.js`（执行器侧：预算、按内容缓存、schema 校验、`llm.map` 分批与重试、Zone 边界）与 `host.d.ts`；`aiws_test.js` 8 例 |
| 指南与提示词 | `aiws/README.md` 改为宏编写指南；`prompts/execute.md` → `prompts/code.md`（嵌入指南 `{AIWS_GUIDE}`）；`prompts/analyze.md` 去 approach |
| Desktop | `executor/infer.ts`（新：`AiccLlmClient` / dev 用 `OpenAiLlmClient`，拒绝按子码分类）；`localExecutor.ts`（三阶段、host.js 应答桥、预算取自 brief）；`programHost.ts` / `program.worker.ts`（通用桥）；`chat.ts`（AICC 错误分类）；`WishService.ts`（`code` / `runMacro` / `next` / `mainStatus`）；界面见 §7.3 |
| CLI（websdk） | `cli/modules/aiws.ts`：`run-exec`（受限 Deno + stdio 桥，宿主逻辑 import 自 stage 的 `lib/host.js`）、`run-program`（先跑再交）、`run-start`、`run-files` 写 `lib/stage.json`、`run-report` 去 `--results`；`runtime/host*.ts` 加 `spawn`，策略只给 `aiws run-exec / run-program` 放开 CLI 自身的 Deno；`deno.json` 映射 `buckyos/xllm` |
| 手册与文档 | `cmd_skills/aiworkspace_run`、`aiworkspace_template_dev`；服务 README（Wish 一节、方法表、HTTP 表）；`schemas/error-codes.json`；内核 08、12；许愿格详细设计 §8.2 / §9.2 / §9.3 / §18 的修订注 |

### 7.2 M0 细节的取定（§6）

1. `aiws.declare({ inputs: { 名: [字段名] }, ai?: { complete?: { max_calls?, max_tokens?, models? }, map?: { max_items? } }, zone?, params?, resources?: { timeout_secs? } })`，宏开头调用一次；非表格输入写 `[]`。`aiws.ai.complete({ system?, prompt | messages, schema?, model?, max_tokens? })` → `{ text }` / `{ value }`。`aiws.zone.fetch(url, init?)` → `{ ok, status, headers, text(), json(), bytes() }`。清单 Schema 见 core `check_manifest` 的文档注释。
2. v2 程序：`macro_check` 给出 `api_version`，要求重新编码；不迁移。已应用的直接结果在第一次编码后按 `missing` 列出（plan 去掉了 keep_direct）。
3. 默认值：`ai_max_calls` 50、`ai_max_tokens` 100 000、`sql_max_calls` 200、`llm_map_max_items` 2000、`snapshot_download_max_bytes` 64 MiB。缓存命中不计入 complete 的次数和 token；`llm.map` 的项数仍含缓存命中（与原来一致）。
4. 视图输入：`manifest_table_ddl` 在逻辑表外包一层视图的过滤（只取 filter，不排序）。
5. 指南放 `aiws/README.md`，英文（提示词是英文）。
6. 托管动词 `run-exec` / `run-program`；桥消息：程序 → 宿主是 stdout 上以 `\x1eaiws ` 开头的一行 `{"id","op","req"}`，宿主 → 程序是 stdin 一行 `{"id","body"}`；程序自己的输出走 stderr。Zone 主机：浏览器取页面的 hostname（及其子域），CLI 取连接端点的主机。
7. `needs_coding.reasons[].code`：`no_macro`、`api_version`、`input_unbound`、`input_changed`、`input_missing`、`field_deleted`、`field_type_changed`；主状态另有 `program_failed`（来自运行记录）。主状态取值：`needs_input`、`needs_analysis`、`needs_coding`、`waiting_data`、`cannot_run`、`in_progress`、`pending_confirmation`、`stale`、`current`；下一步：`answer`、`analyze`、`code`、`view_inputs`、`fix`、`cancel`、`apply`、`run`、`none`；另带 `stale` 布尔，失败或运行中不遮住过期。

### 7.3 与设计不同或自定的地方

- **AI 预算在执行器侧执行。** `host.js` 计调用次数和 token；服务只执行 SQL 的次数上限，并下发上限。宏可以绕开 `aiws.js` 直接调桥，但桥的另一端仍是 `host.js`，所以上限仍然有效。
- **`llm.map` 的逻辑在 `host.js`**（浏览器和 CLI 共用一份），没有按浏览器优先 T3-1 写 `llmMap.ts`。Rust 测试执行器另有一份简化实现。
- **`proc.report { results }` 删除**：执行器直接交结果就是 direct 写入，与“所有结果都由宏产出”矛盾。Mock 仍经 `params.provided`，阶段名改为 `run`，执行记录的 mode 记 `mock`。
- **手写宏**（Desktop 的“编辑程序”）保存时沿用原清单、`api_version` 3；新清单要经编码试运行得到。
- **运行阶段只物化清单里的输入**：分析后来加的、清单里没有的输入不物化；清单里的输入在 wish 的输入里换了实体，算 `input_changed`。
- **未声明字段的拦截**只对表里确有的字段名抛错，其他键（`toJSON`、`then` 等）照常返回 `undefined`。
- **T6-1** 不是只开分析：`run-start` 允许 Agent 以本地执行者身份发起三种阶段；服务端 `run_attribution` 对 kind 为 `agent` 的 Service 主体放行本地模式，归属记 Agent 自己。
- **浏览器优先 M3 未做**：浏览器里的模型循环仍是手写的 `localExecutor.loop` + `chat.ts`，只补了拒绝的子码分类（G4 的最小部分）；Q5 的前置提示未做。

- **主状态的一处补充**：分析或编码运行进行中、或其候选待确认时，它就是“需要分析”/“需要编码”这一步的进展，主状态给 `in_progress` / `pending_confirmation`（首个宏要等候选应用后才存在，否则会一直显示“需要编码”）。
- **测试时发现并修掉的问题**：
  - 文字结果整段换掉时，`richtext::diff_blocks` 先删后插，删空的中间态被拒（运行每次都重写文字，会常碰到）。改为同 id 的叶子块即使换了类型也原地替换，整篇都不保留时先插后删。
  - 运行阶段按清单收窄字段后，视图过滤和排序用到的未声明字段没进读集，数据移入视图后结果不显示过期。改为从全部字段里找决定成员和顺序的字段。
  - SQL 里写错或未声明的列名加了双引号时，SQLite 会把它当字符串（得到错值而不报错）。执行用户 SQL 期间关闭 `SQLITE_DBCONFIG_DQS_DML`（`doc.query` 同样受益；store 的 rusqlite 加了 `modern_sqlite` 特性）。
  - `main_status` 对 `sub_code: null` 不回落到 `code`。
- **已知风险（原有写法）**：`stage_files` 在工作区锁内对 NFS 发 HTTP；测试里进程内 NFS 会因此被同时等锁的请求卡住约 15 秒。生产上 NFS 是独立进程，只是延迟。

### 7.4 验证

| 项 | 结果 |
| --- | --- |
| `cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1` | 61 个测试目标全部通过（317 例，2 例原有忽略的性能测试）；服务端测试经 `wishkit.rs` 的测试侧执行器（本机 Deno `--bridge`）；契约 fixture 已重录，`proc.*` 全部有契约步骤 |
| `aiws/aiws_test.js`（`deno test -A`） | 8 例全过（含 stdio 桥、未声明字段、host.js 的缓存 / 预算 / Zone 边界） |
| websdk CLI `deno test … tests/` | 169 过、3 失败；3 个失败（模板 `manifest_version`）在改动前就失败；新增 `aiws_run_exec_test.ts`（受限运行读不到 `claim.json`、拿不到环境、写不出 `output/`）与策略用例 |
| Desktop `pnpm check`、eslint、`pnpm build`；WASM 已重建 | 通过 |
| Desktop e2e `tests/aiworkspace/wish.spec.ts` | 8 例全过 |
| i18n 守卫 | 1 项失败，只剩本次未改的 `core/src/naming.rs`、`search.rs`、`store/src/wish/outline.rs` 的中文字面量（基线就有） |
| 真实 Zone DV（本机 test.buckyos.io，服务与 Desktop 已部署） | `tests/dv/aiworkspace.macro.spec.ts` 两例通过：浏览器用真实模型走分析 → 编码 → 应用 → 改数据 → 运行（解读随数据更新）→ 字段改名照常运行 → 删声明字段显示需要编码；托管位置用 websdk CLI 领取托管 run，宏以受限 Deno 运行（读不到 `claim.json` 与环境变量），宏内 AI 用 CLI 会话调真实模型，数据更新后再运行。`aiworkspace.mindmap.spec.ts` 的真实模型脑图通过。`aiworkspace.journeys.spec.ts` 的销售旅程失败在数据树“展开”按钮（未走到许愿格，未在基线复核） |

DV 的坑：Zone 现在只在根域发放系统 token，DV 要设 `BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io`（默认的 `sys.` 子域登录失败）；双击许愿格块不再打开面板，用工具栏 `aiws-near-wish-open`。

### 7.5 未做与未验证

- T6-2（对象入口动作）：等权限优化 B-6。
- 浏览器优先 M3（xllm TS Worker 接入）与 Q5 的前置提示。
- 手机上走一遍运行（T7-3 的最后一项）未做。
- 托管 DV 用 CLI 扮演 Agent，没有让真实 Agent 按手册执行：Zone 里的 CLI 发行版（`/opt/buckyos/bin/buckyos`、容器里的 `agent-tools/buckyos`）没有重新打包，仍是旧版，没有 `run-exec`；要 `buckyos-build.py` 打包 SDK/CLI 并更新安装后才能让 Agent 用。
- websdk CLI 的 Node 发行版跑宏用 PATH 上的 `deno`；系统发行版用 CLI 自身的 Deno。
