# xllm：本地任务、Run 目录与命令行协议

本文描述当前 Rust xllm 的已实现行为，供 CLI 使用者、SDK 调用方及需要读写 Run 目录的宿主参考。

- 核对日期：2026-10-01；仓库 HEAD `93eafea5` 之上的工作区（Round / Step / Turn 术语统一之后）。
- CLI 入口：`agent_tool xllm ...`。源码模块仍名为 `run_local_llm.rs`，旧的 `agent_tool run_local_llm` 命令已移除。
- 当前版本：`RunRecord.version = 6`、`prompt.protocol_version = "xllm/2"`、快照 `state.snapshot_version = 5`。三个版本分别管理 Run 记录、提示词运行协议和底层上下文快照；恢复只接受当前版本，见 §7.2。
- 术语：Round = 一次推理（一次宿主 `LlmClient::infer`），Step = behavior 的一次决策记录（`StepRecord`），定义见 [LLM Context readme](readme.md)。本文的工具预算单位是“工具迭代”（§9.2），不是 Round。
- 本文替换原 2026-09-17 的旧工具基线，不再把旧目录格式、请求哈希或 TS SDK 设计建议描述为现行协议。产品目标见 [xllm PRD](../../product/xllm/PRD.md)，SDK 概览见 [xllm Rust SDK 参考](xllm_rust_sdk.md)；实现细节以本文列出的源码为准。

## 1. 范围与 SDK 入口

xllm 执行一次独立任务（Run）：准备配置和输入，调用模型，按需执行工具，保存进度并交付最终结果。普通调用始终建立新 Run，不自动继承前一次历史；恢复必须显式使用 `--resume`。本地指工作目录和工具执行位置，模型既可以通过 BuckyOS AICC 调用，也可以使用 OpenAI 兼容接口。

| 层次 | 当前入口 | 职责 |
| --- | --- | --- |
| 配置 | `discover_config_files` / `load_config_layers` / `merge_config_layers` | 查找、解析、合并 `.llm_context`，记录字段来源 |
| 准备 | `XllmTask::prepare(workdir, TaskInput, TaskOverrides, &XllmDeps)` | 展开工具、读取附件、组装提示词，返回 `PreparedTask`；不建立 Run |
| 执行 | `XllmRun::start(prepared, deps)` → `execute()` | 分配 run_id、加锁、建立记录，驱动到底层上下文停止点 |
| 恢复 | `XllmRun::resume(store, run_id?, workdir_filter?, ResumeLimits, deps)` | 返回 `ResumeStart::Terminal(record)` 或可执行的 `ResumeStart::Run` |
| 查询 / 导出 | `list_runs` / `latest_run` / `load_run` / `export_result` / `build_result_view` | 读取记录、提取已保存结果，不调用模型 |
| 宿主装配 | `XllmTask::prepare_hosted` / `HostedTask` | 接收宿主配置和 system 文本，供宿主直接驱动底层 `LLMContext` |

`XllmDeps` 注入 `LlmClientFactory`、`host_tools`、`RunObserver`、锁目录及可选 `BashRunner`。SDK 可用 `with_llm` 注入模型客户端，不必经过 CLI 初始化。`skip_workdir_lock` 供自行协调共享工作目录的宿主使用。

`RunOutcome` 包含 `Completed`、`Paused`、`Interrupted`、`Failed`、`LimitReached`，每个分支携带 `RunRecord`。配置、输入、存储或恢复错误也可能通过 `Result::Err(XllmError)` 返回；调用方必须同时处理两层结果。

主要源码：

- [xllm.rs](../../src/frame/agent_tool/src/xllm.rs)：配置、提示词、Provider、工具装配、RunStore 和生命周期。
- [run_local_llm.rs](../../src/frame/agent_tool/src/run_local_llm.rs)：CLI 参数、stdin、查询、结果交付和退出码。
- [agent_tool_cli_dev/src/lib.rs](../../src/frame/agent_tool_cli_dev/src/lib.rs)：`agent_tool xllm` 子命令分发。
- [request.rs](../../src/frame/llm_context/src/request.rs)、[state.rs](../../src/frame/llm_context/src/state.rs)、[outcome.rs](../../src/frame/llm_context/src/outcome.rs)：底层请求、快照、outcome 与恢复输入。
- [context_loop.rs](../../src/frame/llm_context/src/context_loop.rs)、[context_window.rs](../../src/frame/llm_context/src/context_window.rs)、[suspension.rs](../../src/frame/llm_context/src/suspension.rs)：循环、容量检查、挂起和续跑校验。
- [exec_tracking.rs](../../src/frame/agent_tool/src/exec_tracking.rs)：宿主执行跟踪与未确认工具结果处理。
- [aicc_client.rs](../../src/kernel/buckyos-api/src/aicc_client.rs)：`AiMessage`、资源和 AICC 请求类型。

## 2. `.llm_context` 配置协议

### 2.1 查找与合并

配置文件为 YAML。从规范化后的工作目录向上查找到文件系统根目录，按“最远祖先 → 工作目录”合并。不是只取最近一份，也不以 Git 仓库根目录作为停止点。

- 普通标量：后层显式声明覆盖前层；CLI / `TaskOverrides` 的显式值再覆盖文件配置。
- `provider`：按字段合并；切换 Provider 类型时清理前一种类型的接入配置。
- `tools` 对象：按字段合并；其中 `tools`、`actions`、`bash_tools` 列表整体替换，显式空列表清空继承值。
- `prompt.groups`：按组名合并，组内 section 按行号覆盖；`prompt.sections` 同样按行号覆盖。
- 配置解析检查未知键、类型、section 别名与行号冲突，错误包含文件、字段和原因。不要把任意扩展字段写进 `.llm_context`。
- 配置中的相对路径相对于声明文件；`~` / `~/...` 展开为主目录。`prompt.groups.<name>` 为字符串时表示外部组文件，其根节点直接是组对象。

### 2.2 顶层字段与默认值

| 字段 | 默认值 / 形状 | 含义 |
| --- | --- | --- |
| `provider` | `type: buckyos` | 见 §4；另一类型为 `openai` |
| `model` | buckyos 为 `llm.chat`；openai 必须提供 | 主模型选择器 |
| `file_model` | 未设置 | 有显式图片附件时，先由此模型分析图片 |
| `max_tokens` | 未设置 | 单次模型请求的最大输出 token 数，不是 Run 总 token 预算 |
| `max_tool_iterations` | `8` | 工具迭代上限：原生工具批次与派发 action 的 behavior Step 共用的工具预算，不是推理次数，见 §9.2；旧键 `max_rounds` 已移除，按未知键报错 |
| `timeout` | `3600`，秒 | 执行时长额度；恢复时重新计时，具体边界见 §9 |
| `llm_timeout` | `600`，秒 | 单次模型请求超时；客户端实际至少使用 1 秒 |
| `context_window` | 未设置 | 模型上下文窗口 token 数；启用本地容量检查与 75% 提前压缩阈值 |
| `runs_dir` | `~/.xllm/runs` | Run 存储目录；YAML `none` 或 `null` 表示内存模式 |
| `loop_model` | `function_call` | 另一值为 `behavior` |
| `run_logs` | `info` | `debug` / `info` / `warn` / `result` |
| `result_format` | `raw` | 原文或 `result.<path>` 提取路径 |
| `tools` | 默认关闭 | 布尔开关或工具配置对象 |
| `prompt` | standard 模式 | `mode`、`select`、`groups`、`sections`、`tools`、`system` |

`context_window` 当前没有 CLI flag，也不是 `TaskOverrides` 字段。`--json` 属于本次任务覆盖项，不能写成顶层 YAML `json`。SDK 另可设置 `json_schema` 和 `disable_capabilities`；CLI 没有对应参数。

### 2.3 工具配置优先级

工具配置按以下顺序覆盖：

```text
默认值 → 合并后的顶层 tools → 选中组的 tools → prompt.tools → --tools / --no-tools
```

custom 模式和结构化输入不选组，因此不应用组内工具配置。工具对象支持：

| 字段 | 含义 |
| --- | --- |
| `enabled` | 是否启用，默认 false |
| `filesystem_policy` | `workspace`（默认）或 `unrestricted` |
| `tools2actions` | behavior 模式把原生 tools 转为 actions，默认 false |
| `tools` / `actions` | 来源列表，每项恰好包含 `groupname`、`mcp`、`name` 中的一个 |
| `bash_tools` | 命令手册条目，含 `name`、`description`、`command`、可选 `usage` |

来源例子：`{groupname: bash}`、`{mcp: "http://localhost:8080/mcp"}`、`{name: host_tool}`。具名工具必须由 SDK 宿主注入；普通 CLI 默认没有宿主工具。MCP 当前通过 HTTP JSON-RPC `tools/list` 和 `tools/call` 接入，发现超时参数为 30 秒、调用为 120 秒。工具名称冲突、未知组或发现失败在准备阶段报错。

仅启用工具但未声明 `tools` 列表时，使用内置 `bash` 组；`tools: []` 不补回默认组。function_call 模式拒绝 `tools2actions: true` 和非空 `actions`。behavior 模式可以同时使用原生 tools 与 actions；启用 `tools2actions` 后原生列表置空，转换结果与显式 actions 合并。

## 3. 输入与提示词

### 3.1 `TaskInput` 与路径

`TaskInput` 包含 `user`、有序 `attachments`、`stdin`、`structured` 和可选 `base_dir`。CLI 的映射及路径规则如下：

| 输入 | 当前语义 |
| --- | --- |
| 位置参数 / `--user` | 单个任务要求，互斥；问题中有空格时作为一个 argv 传入 |
| `--dir` | 已存在的工作目录，默认进程 cwd；不会创建旧协议的 `workspace/` |
| `--file` | UTF-8 文本材料，每个最多 8 MiB |
| `--image` | PNG / JPEG / WebP 本地文件，每张最多 20 MiB；或 HTTP(S) URL |
| `--input-file` | `Vec<AiMessage>` 的 JSON 文件 |
| stdin | 新任务自动检测 FIFO / 普通文件重定向并读到 EOF |
| `--runs-dir` | 覆盖 Run 存储目录；这是路径参数，字符串 `none` 不等于 YAML 的内存模式 |
| `--output` | 结果输出路径，见 §10 |

相对 `--dir`、附件、`--input-file`、`--runs-dir`、`--output` 均以 CLI 启动 cwd 为基准；`--dir` 不改变其余 argv 路径的基准。SDK 附件及 runs_dir 覆盖值使用 `TaskInput.base_dir`，缺省取进程 cwd。配置路径解析会展开 `~`，附件、`--dir` 和直接读写的输入/输出路径不统一提供这种展开；示例优先使用绝对路径或让 shell 展开。

本地图片以 base64 进入消息；URL 图片保存为资源引用，准备阶段不下载、不计算摘要。附件记录保留顺序、标签、类型及可获得的路径、MIME、字节数、SHA-256。

### 3.2 任务要求与材料顺序

任务要求优先级是：显式问题 / `--user` → 选中组的 `default_user` → stdin。已有任务要求时，stdin 是补充材料。只有附件且没有任何任务要求会报错。

主模型 user 消息顺序：任务要求 → 按命令行顺序排列的文本 / 图片附件 → stdin 材料。文本包装为 `<material name="...">`；图片带编号标签和图片块；补充 stdin 包装为 `<stdin>`。

配置 `file_model` 且存在显式图片附件时，先进行无工具的图片分析。主模型收到文本材料及 `<image_analysis model="...">`，不再收到原图片；分析成功后结果保存在 `file_model_stage`，恢复不重复已完成的分析阶段。

Unix 下终端、socket、`/dev/null` 不自动读取。空白 FIFO 报输入错误；空白普通文件重定向视为无 stdin。`--input-file` 路径分支直接读取 JSON，不再读取 stdin。

### 3.3 结构化消息

结构化输入与问题、附件、SDK stdin、`--select`、`--system` 互斥。它跳过组默认输入与业务 section 组装，但仍在消息前增加 xllm 的 capabilities 和 runtime_protocol system 消息。

最小输入文件：

```json
[
  {
    "role": "user",
    "content": [{"type": "text", "text": "用一句话解释这个任务。"}]
  }
]
```

`AiMessage` 只有 `role` 和 `content`；role 为 `system`、`user`、`assistant`、`tool` 或 `developer`。内容块包括 `text`、`image`、`document`、`tool_use`、`tool_result`、`thinking`、`provider_state`，精确形状见 `aicc_client.rs`。工具调用与回执放在 content 块中，通过 `call_id` 配对，不能使用旧的顶层 `tool_calls` / `tool_call_id`。消息与内容块会拒绝未知字段；这与普通 Run 记录结构的 serde 行为不同。

### 3.4 section、custom 与模板

standard 模式按“系统默认 → 选中组 section → `prompt.sections`”组装，再按行号升序输出非空段落。section 可使用数字键或固定别名：

| 别名 | 行号 | 系统追加内容 |
| --- | --- | --- |
| `role` | 10 | 无 |
| `contexts` / `env` | 20 | 未在模板中引用的时间、时区、OS、工作目录 |
| `rules` | 30 | 实际可用 tools / actions，或无工具说明 |
| `cmd_manual` | 40 | exec 和 `bash_tools` 手册；exec 未启用时整段省略 |
| `output_format` | 100 | 无 |

section 可用字符串或 `{name, text}` 对象；`text: ""` 清空用户文本，不移除必要的系统说明。同层同一行号不能同时使用数字键和别名；`name` 不能冒用其它固定行号的别名。

`--system` 或 `prompt.mode: custom` 使用整段业务 system，保留自动追加的能力、命令说明和 runtime_protocol。`--system` 与 `--select` 互斥；文件同层 `mode: custom` 与 `select` / `sections` 冲突。进入 custom 模式会清掉继承的选择与 section 覆盖；子层显式选择组或声明 section 可切回 standard。

模板用于业务 system、section 文本和组的 `default_user`：支持 `{{runtime.current_time}}`、`{{runtime.timezone}}`、`{{runtime.os}}`、`{{runtime.cwd}}`、`{{env.NAME}}`。`\{{` 输出字面 `{{`；替换值不递归展开；缺失变量报错。显式任务正文、附件和 stdin 不做同样的模板渲染。渲染结果及引用变量值随 Run 保存，恢复直接复用。

## 4. Provider 与模型选择

| Provider | 接入方式 | 配置 |
| --- | --- | --- |
| `buckyos` | 复用 / 初始化 BuckyOS runtime，通过 AICC SDK 调用 | 可选 `provider.session_token`；默认当前身份 |
| `openai` | OpenAI 兼容 Chat Completions，POST `<base_url>/chat/completions` | `base_url`、`api_key` 或 `api_key_env`、`headers`；必须给出 model |

`openai` 默认 base_url 为 `https://api.openai.com/v1`，未配置凭据引用时尝试 `OPENAI_API_KEY`。这里描述的是仓库 adapter 的实现，不代表所有兼容服务都支持同一能力。

buckyos 的 `model` / `file_model` 支持两类选择器：

- `llm.chat`、`llm.vision` 等逻辑名：走 `helper.llm_chat`，带工具 / JSON 能力需求。
- `model@provider` 或 `model:variant@provider`：走 `chat.completions.create`，精确保留模型、variant 和 Provider，不做逻辑路由或 fallback。

AICC 请求采用 `execution_mode: immediate`，传入消息、实际允许的工具、response_format 和 max_output_tokens。返回 `Succeeded` 必须有 message；`Failed` 转 Provider Unknown 错误；`Running` 视为不支持的异步任务，不轮询。当前 adapter 不使用 fallbacks / provider_options，也不实现远端任务取消；`disable_capabilities` 经逻辑 helper 的 `disable` 传递，精确选择器不走此字段。

`ensure_buckyos_runtime` 优先复用现有 runtime。新初始化时，按环境中的 AppClient session token、OOD 本机设备私钥、开发目录用户私钥选择登录方式；默认 app id 为 `buckycli`，可用 `BUCKYOS_APP_ID` 覆盖。具体登录流程见 [SDK 参考 §8](xllm_rust_sdk.md#8-buckyos-provider-的登录方式)。

`api_key` / `session_token` 保存为 `SecretRef` 的文件字段引用，`api_key_env` 保存为环境变量引用；恢复时重新解析，所以修复同一个凭据来源后可以重试。`headers` 及提示词引用的 `env.*` 值按普通配置 / 文本保存，不属于凭据引用机制。

## 5. 目录、锁与持久化

### 5.1 布局

```text
<workdir>/                       已有工作目录，工具直接在这里工作
└── .llm_context                 可选；也可继承祖先目录配置

<runs_dir>/                     默认 ~/.xllm/runs
└── <run_id>/
    ├── run.json                 RunRecord
    ├── snapshots/
    │   ├── 0001.json            LLMContextSnapshot
    │   ├── 0002.json
    │   └── ...
    └── .lock                   该 Run 的执行锁

<lock_dir>/                     默认 ~/.xllm/locks
└── <hash(workdir)>.lock         启用工具的任务按工作目录互斥
```

不再生成旧协议的 `request.json`、`state.json`、`outcomes/final.json` 或 `*.snap.json`。xllm 不创建共享 `workspace/` / `bin/`，也不自动给 PATH 增加工作目录中的 bin。进度事件由 `RunObserver` 接收，CLI 输出到 stderr；默认不落成 `worklog.jsonl`。

YAML `runs_dir: none` / `null` 使用内存 RunStore。本次 SDK 仍可读取同一个内存 store 的记录，但新 CLI 进程无法查询或恢复；没有磁盘 Run 锁。工具工作目录锁仍生效，除非宿主设置 `skip_workdir_lock`。

### 5.2 标识与锁

run_id 格式为 `YYYYMMDD-HHMMSS-<6hex>`，前缀使用本地时间；后缀由纳秒时间、PID、进程内计数经 BLAKE3 生成，不是旧的四位时间后缀。磁盘创建使用 `create_dir`，冲突最多尝试 8 次。消费方应将 ID 当作不透明字符串。

Run 锁和工作目录锁均为非阻塞 OS 排他锁，句柄释放后解锁。工作目录锁文件名取规范化 workdir 字符串的 BLAKE3 前 24 个十六进制字符，与 runs_dir 无关。启用工具的任务即使使用不同 runs_dir，在同一默认锁目录下仍互斥。锁文件内容用于诊断，是否活跃以持锁情况为准，不能靠 PID 或删除锁文件判断 / 解除占用。

### 5.3 提交边界

`start` 在首次模型调用前写 `run.json`，其中 `pending_input` 保存完整初始输入与文件模型计划。进入主模型阶段后先提交初始快照，再清除 `pending_input`。每次推理（Round）前（xllm 的 `InferenceHook`；behavior 模式包括 Step 内层的推理）、outcome 边界和压缩续跑前也提交快照。

快照提交顺序为：写新的 `snapshots/NNNN.json` → 更新 `run.json.latest_snapshot_idx`；终态结果随后写入 `run.json.result` 和状态。单个文件使用临时文件、fsync、rename，并同步目录；多个文件之间不构成一个事务。恢复读取记录指向的编号，不扫描更大编号来代替它；未被记录引用的快照不代表已提交结果。

`run.json` 为 UTF-8 pretty JSON，快照为 UTF-8 JSON；缩进和键序不是协议。时间戳单位均为 Unix 毫秒，计数为非负整数。普通结构体没有通用未知字段透传能力；跨语言执行者应保留已定义的宿主字段，不能靠 serde 忽略未知字段来实现无损读写。

## 6. `run.json` 与查询视图

### 6.1 `RunRecord`

| 字段 | 含义 |
| --- | --- |
| `version` / `run_id` | Run 记录版本与目录标识 |
| `status` | snake_case 状态，见 §8 |
| `workdir` / `runs_dir` | 原工作目录和存储位置；内存模式省略 runs_dir |
| `created_at_ms` / `updated_at_ms` | 创建 / 最近记录更新时间；不是心跳 |
| `summary` | 任务要求首行摘要 |
| `input` | request、request_source、附件记录、stdin_role / stdin_chars、structured_messages |
| `config` | 实际 Provider / 模型 / loop / limits / 工具 / 输出策略，以及 config_files / sources |
| `prompt` | mode、group、渲染后的 sections / custom_system、最终 system_prompt、runtime_protocol / protocol_version、任务要求、模板及 runtime 变量 |
| `file_model_stage` | 成功的图片分析、模型、usage、response_model 和完成时间，可省略 |
| `pending_input` | 首次快照前的 initial_messages、main_user_parts、file_stage，可省略 |
| `latest_snapshot_idx` | 恢复入口的快照编号，可省略 |
| `last_error` | phase、kind、message、recoverable、condition、at_ms，可省略 |
| `result` | raw、extracted、extract_error、json_valid、json_error、response_model，可省略 |
| `artifacts` | 已追踪的 write_file / edit_file 成功写入路径；不是工作目录全量变化清单 |
| `usage` | main、file_model、compaction、llm_requests；各阶段 usage 可缺省。`llm_requests` 是本 Run 经宿主 `LlmClient::infer` 发起的推理尝试数：主循环 Round（含失败的尝试）加文件模型阶段；xllm 自己执行时，上下文压缩的摘要请求经同一模型客户端发出，也计入。每个执行段（xllm，或 libOpenDAN 等宿主）只在已有值上累加，从不重置或覆盖。它不是工具迭代数，也不是 `run()` 调用次数 |
| `limit_reason` / `interrupt_reason` | 停止原因，可省略 |
| `compactions` / `pid` | 已完成压缩次数 / 最近执行者 PID |
| `host` / `host_commit_pending` / `inflight` / `executions` | 宿主装配、输入提交门槛及执行跟踪，见 §11 |

`EffectiveConfig` 中的 `limits`（`RunLimits`）使用 `max_tokens`、`max_tool_iterations`、`timeout_secs`、`llm_timeout_secs`、`context_window_tokens`；后三项不同于 YAML 的 `timeout`、`llm_timeout`、`context_window`。Provider 的已解析类型字段为 `kind`，不同于 YAML `type`。`result_format` 在记录里是 `{"kind":"raw"}` 或 `{"kind":"path","segments":[...]}`，在 CLI 视图中才是字符串。

`config.tools` 保存 `enabled`、`filesystem_policy`、`tools2actions`、原始来源列表 `tool_sources` / `action_sources`、展开的 `native` / `actions`、`bash_tools`、`exec_enabled` 和来源信息。恢复按保存的来源重新构建工具，MCP 仍需可连接；记录不保存工具进程或模型连接。

### 6.2 查询选择

`list_runs` 按 `updated_at_ms` 倒序排列，CLI 默认只列当前工作目录，最多 20 条。`status` / `result` 不给 `--run` 时选择当前工作目录最近一条；`--resume` 不给 `--run` 时选择最近的非终态记录。显式 run_id 在所选 RunStore 中直接查找，不受当前 workdir 过滤，但真正恢复使用记录中的原工作目录。

扫描列表时跳过缺失 / 不可解析的 run.json；按 ID 读取时返回明确错误。`run.json.status = running` 但没有进程持 Run 锁时，`RunSummary` / `XllmResult` 显示 `interrupted`，且 `RunSummary.stale_running = true`。查询不会为此重写持久状态；锁探测可能创建锁文件。

查询的 `resumable` 只根据状态与活跃锁推导，不代表版本、宿主提交、工具依赖和快照检查已经通过。

## 7. 快照 v5 与底层恢复

### 7.1 快照字段

快照顶层只有 `request` 和 `state`。request 为 `LLMContextRequest`，包括 owner、trace、objective、behavior_name、input、model_policy、tool_policy、output、budget、human_policy、error_policy、forbid_next_behavior；这是已编译的底层请求，不是 `.llm_context` 或旧 `OneShotRequest`。

| `state` 字段 | 含义 |
| --- | --- |
| `snapshot_version` | 当前为 5，缺省读取为 0；恢复只接受 5 |
| `accumulated` | 当前消息历史；behavior 模式下 `request.input` 之后的部分是进行中 Step 的 inner transcript（内层原生工具循环消息，尚未折入 StepRecord） |
| `usage` / `tool_iterations_left` | 累积模型用量 / 剩余工具迭代额度 |
| `started_at_ms` / `cost_units` / `consecutive_errors` | 计时起点、成本计数、连续可纠正错误计数 |
| `suspended` | 挂起原因：`pending_tool` 或 `context_limit`；无挂起时省略 |
| `tool_batch` | 尚未派发的原生工具调用 `remaining` 及本批 `batch_error`（批次完成时只计一次失败） |
| `action_step` | 尚未完成的 behavior Step 及其模型 response |
| `llm_task_ids` | Provider 任务追踪 ID |
| `steps` / `history_summaries` / `history_inputs` / `last_step` / `last_report` | behavior 历史、最新 step 与 report |
| `next_step_index` / `next_action_id` | 后续编号 |
| `host` | 不透明宿主 JSON；底层循环原样携带 |

v2 起用 `suspended` / `tool_batch` / `action_step` 取代旧 `pending_tool_calls`，不得只保存一个 pending call 列表后猜测如何续跑；v3 把工具预算字段改为 `tool_iterations_left` / `tool_batch.batch_error`。

### 7.2 `ResumeFill` 匹配规则

| 快照状态 | 底层允许的恢复输入 | xllm 行为 |
| --- | --- | --- |
| 未挂起 | `ResumeFromMidRun` | 从提交点续跑 |
| `suspended.kind = context_limit`，function_call | `RewrittenHistory` | 先压缩消息，再恢复 |
| `suspended.kind = context_limit`，behavior | `RewrittenSteps` | 物化历史折入 input，再恢复；底层保留进行中 Step 的 inner transcript、挂起状态及编号 |
| `suspended.kind = pending_tool` | `ToolResults` | 底层支持回填；xllm 无回填入口，拒绝接手 |

挂起时间不计入底层 wallclock。恢复会校验 fill 与挂起态匹配、工具调用 / 回执配对和 continuation 状态；不合法的组合返回 `SnapshotCorrupted`。宿主自行回填后，应先持久化新的快照再继续，因为 continuation 可能先执行尚未派发的工具。

当前 Rust 对非终态的恢复检查要求 `RunRecord.version` 精确等于 6、运行协议精确等于 `xllm/2`；`LLMContext::resume` 只接受 `state.snapshot_version = 5`，更旧（含缺省的 0）或更新的版本都以 `SnapshotCorrupted` 拒绝。不做旧版本迁移，也不提供旧字段别名：`version = 1` / 快照 v2 的 Run（`config.limits.max_rounds`、`rounds_left`）不能恢复，其 run.json 按当前结构也无法解析（列表跳过，按 ID 读取报错）。`LLMContext::snapshot()` 写出时使用当前快照版本。

## 8. 生命周期、中断与恢复

### 8.1 状态映射

| `status` | 终态 | 触发条件 |
| --- | --- | --- |
| `running` | 否 | 持锁执行中；遗留 running 的展示规则见 §6.2 |
| `interrupted` | 否 | 用户中断，或查询发现执行进程已退出 |
| `paused` | 否 | Provider 超时 / transient / 可识别的凭据错误、checkpoint 失败、工具基础设施失败 |
| `completed` | 是 | 底层 `Done`，最终原文已保存；提取或 JSON 校验可能仍失败 |
| `failed` | 是 | 不可恢复 Provider / 内部错误、连续可纠正错误超限、压缩失败或重复超限等 |
| `limit_reached` | 是 | 工具迭代、wallclock 等底层预算耗尽；工具预算的 `limit_reason` 为 `tool iteration limit (N) reached`（底层 `BudgetKind::ToolIterations`，序列化为 `tool_iterations`） |

Provider Permanent 错误中包含 token、401、403、auth、expired、permission 等线索时，当前实现将其归为可恢复的 `credentials`；其它 Permanent 和 Unknown 为失败。此处是错误文本分类，不是自动刷新任意凭据的保证。可恢复故障停止为 paused，由调用方显式 resume；不在同一次 CLI 中无限重试。

### 8.2 恢复流程

1. 按显式 ID 或当前工作目录最新非终态记录选择 Run。若指定终态，返回保存的记录，不再次执行；同时给执行额度参数会报 `RunTerminal`。
2. 非终态检查运行锁、记录版本、`host_commit_pending`、宿主 runtime_kind、运行协议及原工作目录。
3. 取得 Run / 工作目录锁后重读记录，确认宿主输入已提交；核对旧受管执行并处理 inflight，见 §11。
4. 使用保存的配置与工具来源重建工具 / Provider，重新读取凭据引用；不重新合并 `.llm_context`、重选组或重渲染提示词。
5. 使用 `latest_snapshot_idx` 指向的快照恢复；尚未有快照时使用 `pending_input`。两者都没有则拒绝恢复。
6. 重置本次 wallclock 起点；保留用量、`usage.llm_requests` 和已消费的工具迭代。`ResumeLimits` 可覆盖 `max_tokens`、`max_tool_iterations`、`timeout_secs`、`llm_timeout_secs`；调整 `max_tool_iterations` 时以新总额度减去已消费额度（原总额度 − `tool_iterations_left`）计算剩余值，已消费的额度不退还。
7. 未挂起快照直接续跑；上下文上限快照先压缩；等待 deferred 工具结果的快照拒绝接手。

CLI resume 只将四种额度参数传入 `ResumeLimits`。模型、Provider、工具、loop 等使用原记录；不要用 `--resume --model ...` 期望切换模型。`--run-logs` 和交付格式可影响本次展示，`--result-format` 可影响导出，不改变原任务语义。

没有请求语义哈希、自动匹配相同输入或 `--append`。新任务使用新的上下文和 run_id；要把前次输出作为新材料，可使用管道或结构化消息。

### 8.3 中断与故障边界

CLI 第一次 Ctrl-C 请求协作中断并保存进度；第二次 Ctrl-C 杀掉仍受管理的本地 bash 进程组并立即以 4 退出，不等待新的提交。SDK 使用 `XllmInterrupter::interrupt(reason)`。

保存点之间仍可能发生模型重试或工具副作用重放。普通 xllm 默认不自动填充宿主的 inflight / executions，也没有 exactly-once 执行保证。宿主提供的执行跟踪可以将已登记而结果未提交的动作变成“结果未知”，但不能恢复丢失的外部结果。强制杀进程不等价于完成一次 checkpoint。

若 outcome 已算出但提交失败，xllm 尝试记录 paused/storage；磁盘完全不可写时，这条暂停记录也可能无法落盘。恢复基于最后成功提交的记录，不能把内存里的 outcome 当成已经持久化。

## 9. 工具与限制的执行语义

### 9.1 内置 `bash` 组

| 名称 | 入参 | 行为 |
| --- | --- | --- |
| `read_file` | `path`，可选 `range`、`first_chunk` | 读取文件内容 / 指定范围，具体选择规则由文件工具定义 |
| `write_file` | `path`、`content`，可选 `mode` | 写 UTF-8 文本；默认覆盖，支持 create/new、append 等模式 |
| `edit_file` | `path`、`old_string`、`new_string` | old_string 非空且精确匹配一次，执行文本替换 |
| `exec` | `command`，可选 `target`、`timeout_ms`、`cwd`、`env` | 本地 `/bin/bash -c`；env 由执行器接受，当前 schema 未展示该字段 |

这里的工具名是 `exec`，不是旧目录工具的 `shell`。默认 cwd 为 workdir；相对 cwd 和文件路径基于 workdir。exec 只支持本地 target，继承进程环境，可覆盖 env；stdin 为 EOF，每次调用启动新的 shell，不保留上次 shell 的 cwd / 变量。`bash_tools` 只生成命令手册，不安装程序、不注册函数、不修改 PATH。

exec 默认超时 1,800,000 ms（30 分钟），最大 3,600,000 ms（1 小时），过大的有效值 clamp 到上限。输出限制为 64 KiB，保留头部 1/4 和尾部 3/4。命令在独立进程组中运行，超时、取消或总时长限制触发时杀掉受管理的进程组；超时 Error 含已有输出、`timed_out` 和重试提示。非零退出的错误 observation 同时包含摘要及输出。

`filesystem_policy: workspace` 限制内置文件工具的路径和 exec 的初始 cwd；`unrestricted` 清空文件读写 root 白名单，允许 cwd 指向其它目录，相对路径基准不变。策略随 Run 保存并在恢复时复用，仅作用于内置组。workspace 的路径约束不是 OS 沙箱，不能隔离 shell 命令自身的绝对路径、cd 或符号链接访问；MCP / 宿主工具自行决定执行策略。

Success observation 优先取非空 output，其次 summary，最后 details JSON；Error observation 包含 summary 和可用的 output。普通工具业务错误供模型纠正；连续可纠正错误上限为 3。`artifacts` 只追踪已观察到成功的 write_file / edit_file 路径，exec 产生的文件不会自动枚举登记。

### 9.2 工具迭代、时间与 deferred

xllm 设置串行工具策略（`parallel: false`）和 `max_calls_per_round = 16`：一次模型 response（一个 Round）最多 16 个原生工具调用；它不限制一个 behavior Step 的 action 数。

`max_tool_iterations` 是工具预算（底层 `ToolPolicy.max_tool_iterations`，剩余值为快照的 `tool_iterations_left`）。一次工具迭代 = 一批实际派发的原生工具调用（同一 response 的全部调用），或一个派发 action 的 behavior Step（同一 Step 的多个 action 只计一次）；业务失败也消耗额度，behavior Step 内层的原生工具批次与外层 action 共用同一额度。额度为零时仍允许模型给出无工具的最终答案；再次要求工具或 action 则进入 `limit_reached`，不执行超额调用。behavior actions 按顺序执行，首个业务失败后本 Step 的后续动作跳过并反馈给模型。

推理次数与工具预算分别计算：无工具的最终回答、解析纠错后的重试同样是 Round，却不消耗工具迭代，所以不能用剩余额度推算推理次数。xllm 没有推理次数上限，实际推理尝试数只记在 `usage.llm_requests`（§6.1）。

`timeout` 映射到底层 `max_wallclock_ms`，同时在 `execute()` 为正在运行的工具设置 deadline。`llm_timeout` 包装单次模型请求。文件模型阶段和压缩请求也使用请求超时，但总时长检查不是覆盖所有准备 / 网络阶段的统一硬 deadline；运行中的模型请求仍受自己的超时边界约束。恢复重新获得本次时长额度。

`max_tokens` 传入模型输出上限；xllm 构造的普通请求没有 `max_total_tokens` / `max_cost_units` 总额度。底层支持的预算种类不能都当成现行 CLI 开关。

底层（自快照 v2 起）已能产出 `PendingTool` 并接受 `ToolResults`，但 xllm 配置 `allow_deferred: false`。工具返回 Pending 会按违约 / 未知副作用处理，不应把它当作可继续轮询的 CLI pending 状态；如果执行路径收到 `PendingTool` outcome，xllm 将其记为 `failed` / `deferred_tool`。没有 `--fill` 或通用异步工具回填命令。

### 9.3 上下文容量与压缩

设置 `context_window` 后，xllm 设置 `context_yield_threshold = Ratio(0.75)`。底层估算将要发送的消息、工具声明和输出 schema：

- 估算输入达到窗口的 75% 时，产出 `ContextLimitReached`（`approaching_window`）。
- 估算输入加 `max_tokens` 的输出预留超过窗口时，产出 `hard_limit`，该请求不发送。
- 未配置窗口时，不自动查询模型窗口；依赖 Provider 的结构化 `ContextLimit` 拒绝。OpenAI 兼容 adapter 识别 `/error/code = context_length_exceeded`。

Ratio 必须有窗口；窗口必须大于零，输出预留必须小于窗口。估算是本地近似值，媒体使用固定费用，并非 Provider 精确 tokenizer 的容量保证；不是按累计 usage 触发压缩。

xllm 使用本次主模型和 `LlmSummarizeCompressor`，目标 token 参数为 32768。function_call 使用 `RewrittenHistory`，behavior 使用 `RewrittenSteps` 把物化历史折入 input；压缩后的上下文先提交快照再继续。`compactions` 随 Run 保存，最多完成 3 次；再次超限进入 failed/context_capacity，压缩调用失败进入 failed/context_compaction。

## 10. CLI、结果与退出码

### 10.1 参数面

```text
agent_tool xllm "任务要求" [options]
agent_tool xllm --select <group> ["任务要求"]
agent_tool xllm --resume [--run <id>] [limits/output options]
agent_tool xllm list [--limit N]
agent_tool xllm status [--run <id>]
agent_tool xllm result [--run <id>]
```

| 分类 | 参数 |
| --- | --- |
| 输入 | `--user`、`--system`、`--select`、可重复 `--file` / `--image`、`--input-file` |
| 目录 | `--dir`、`--runs-dir` |
| 模型 / 工具 | `--provider buckyos\|openai`、`--model`、`--file-model`、`--loop-model function_call\|behavior`、`--tools` / `--no-tools` |
| 额度 | `--max-tokens`、`--max-tool-iterations`、`--timeout`、`--llm-timeout` |
| 交付 | `--result-format raw\|result.<path>`、`--json`、`--format text\|json`、`--output` |
| 查询 / 日志 | `--run`、`--limit`、`--run-logs debug\|info\|warn\|result`、`--help`、`--version` |

未知 flag、缺值、无效数字或互斥输入返回 2；大多数单值参数重复也报错，附件按出现顺序保留。`--resume` 不接受新问题、system、select、structured input 或附件。`--run` 不能用于创建新任务。旧 `--objective`、`--new`、`--append`、`--input-stdin` 等不属于当前入口。

CLI 帮助、状态标签、进度和诊断使用英文；用户输入、模型输出和工具正文按原文保留。info/debug 在 stderr 列出参与合并的配置文件；resume 列出原 Run 保存的来源。exec 日志显示实际 command，控制字符转义为单行；warn/result 隐藏常规进度。stdout 应按所选交付格式读取，stderr 不是结构化 API。

### 10.2 三种结果控制

| 选项 | 控制对象 |
| --- | --- |
| `--result-format` | 从最终原文提取什么 |
| `--json` | 提取结果必须能解析成合法 JSON |
| `--format json` | CLI 输出 `XllmResult` 包装，而非单独答案 |

`raw` 保留最终原文。`result.<path>` 先剥离代码围栏，再按 JSON 对象字段 / 数组下标，或 XML 根节点之下的唯一子元素路径提取。`result` 是路径前缀，不是要求模型输出一个名为 result 的外层字段。例如 `result.answer` 从 `{"answer":42}` 提取 42，`result.report` 从 `<response><report>...</report></response>` 提取 report。

JSON 字符串和 XML 文本叶节点导出为 Text；JSON 对象、数组、数值等导出为 Json；含子元素的 XML 节点导出其内部 XML。路径缺失、重复 XML 匹配或格式不符报提取错误，不自动回退 raw。`ExtractedValue` 的保存形状分别为 `{"kind":"text","text":...}`、`{"kind":"json","value":...}`、`{"kind":"xml","xml":...}`。

function_call 的 `--json` 会向模型提出 JSON 输出要求；本地仍对提取结果做 JSON 语法校验。SDK `json_schema` 转发给 Provider，不进行本地 schema 校验。behavior 的原始响应是 XML，因此 `behavior + raw + --json` 在准备阶段拒绝，通常应配置 `result.report`。behavior 完成要求无动作的非空 `<report end="true">`；普通 report-only 继续，END / done 拒绝；默认 `raw` 仍交付完整最终 XML，不自动只交付 report。

`completed` 表示模型阶段完成且原文已保存。即使提取 / JSON 校验失败，记录仍为 completed，CLI 返回 6；之后用 `result` 重新导出，不需要再次调用模型，也不改写原结果记录。

### 10.3 结构化视图与输出文件

`XllmResult` 包含：

| 字段组 | 字段 |
| --- | --- |
| 身份 / 状态 | `run_id`、`status`、`status_label`、`is_terminal`、`resumable` |
| 答案 / 失败 | 可选 `answer`、`answer_kind`、`extract_error`、`json_error`、`error`、`limit_reason`、`interrupt_reason` |
| 产物 / 用量 | `artifacts`、可选 `usage`、`usage_detail` |
| 执行配置 | `provider`、`model`、可选 `file_model` / `response_model`、`loop_model`、字符串 `result_format` |
| 定位 / 来源 | `workdir`、可选 `runs_dir` / `resume_command`、`config_files`、`created_at_ms`、`updated_at_ms` |

`list --format json` 输出 `RunSummary[]`；`status --format json` 输出 `{result: XllmResult, input, config, prompt}`；新任务和 `result --format json` 输出 `XllmResult`。它们不使用通用 `AgentToolResult` 的 status/summary 外层，也不直接输出底层 `LLMContextOutcome`。

默认 text 格式输出提取后的答案；stdout 不以换行结尾时会补一个换行。已完成结果的 `--output` 直接覆盖指定文件，不创建父目录、不使用 RunStore 的原子提交，文件内容不自动补换行，成功时 stdout 不再输出答案。输出文件写失败不撤销 completed 状态，可稍后重新导出。

当前实现的分支差异：paused/interrupted/failed/limit_reached 的 `--format json` 直接写 stdout，不经 `--output`；list/status 也直接输出。对已失败 / 达到限制的终态显式 resume，只展示保存状态并返回 0，且当前 `--format json` 分支先打印文本状态再打印 JSON；需要纯结构化查询应使用 `status --format json`。`resume_command` 保存为 `xllm --resume --run <id>`，通过 agent_tool 调用时需补入口前缀，并按原存储位置提供 `--runs-dir`。

### 10.4 退出码

| 退出码 | 含义 |
| --- | --- |
| 0 | 完成且交付有效，或查询 / 终态展示成功 |
| 1 | 本次执行 failed / limit_reached，目标不存在、不可恢复 / 损坏记录，或一般存储 / 执行错误 |
| 2 | 参数、配置、输入、能力、模板、工具准备错误，或 Run / 工作目录正被占用 |
| 3 | 可恢复暂停 paused |
| 4 | 用户中断 interrupted |
| 5 | 已完成结果的 `--output` 写入失败 |
| 6 | 结果提取或 JSON 校验失败，原文已保留 |

脚本应同时检查命令类型、退出码和结构化状态 / 错误，不能只判断记录存在或 status 为 completed。退出 2 通常发生在可执行 Run 建立前，但 `start` 的目录分配早于加锁及 Provider 初始化，预检失败不保证没有留下空目录。

## 11. 宿主装配的 Run 与接手边界

libOpenDAN 的 Agent Session 直接使用 xllm Run 目录。目录关系及宿主提交规则见 [Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md) 和 [LLM Context 设计](LLM%20Context%20设计.md)。

`prepare_hosted(workdir, llm_context_json, origin, host_system, deps)` 解析宿主提供的单份 `.llm_context` JSON，使用同一严格 schema，不向上发现目录配置。xllm 在宿主 system 后追加能力、命令手册和 runtime_protocol；`HostedTask::new_record` / `build_request` 供宿主建立记录与底层请求，`hosted_waist_deps` 为 behavior 装配 xllm parser 和不带时间戳的 step renderer。`rebuild_toolset` / `create_run_llm` 根据保存记录重建依赖。

| Run 字段 | 协议意义 |
| --- | --- |
| `host` | `assembled_by`、session_id、runtime_kind、runtime_id、env_check、extra |
| `host_commit_pending` | 已写入快照但宿主尚未提交消费状态的输入批次序号；非空时 xllm 拒绝执行 |
| `inflight[]` | 已派发但结果尚未持久化的动作：call_id、tool、args、effect、idempotency_key、execution_ids、step_index、started_at_ms |
| `executions[]` | 待确认停止的受管执行：execution_id、kind、call_id、runtime_id、host、boot_id、pgid、leader_start_ticks、command、started_at_ms |

宿主字段不是普通 CLI 自动生成的执行审计。`TrackedBashRunner` 可通过 `XllmDeps.bash_runner` 注入：执行标识先由 `ExecutionRegistrar` 持久化，再放行命令，子进程继承 `OPENDAN_EXECUTION_ID`。探测 / 停止通过环境标记等执行事实核对，不能仅信任可复用的 PID。

接手时仅允许 `runtime_kind` 缺省或为 `native`。取得 Run 锁后，先通过 `stop_execution` 确认 executions 中的旧执行已停止，无法确认则拒绝；再把 inflight 中没有持久结果的动作经 `materialize_unresolved` 写入快照为“结果未知”，提交新索引后清除 inflight，不自动重放这些已登记动作。

宿主装配的 Run 使用同一记录与快照版本（`version = 6`、快照 v5）；libOpenDAN 读到其它 `RunRecord.version` 或不支持的快照版本时进入 RecoveryBlocked，同样不迁移。宿主驱动时的快照边界由宿主决定（libOpenDAN 用 `CheckpointHook`：function_call 在每次推理前，behavior 只在外层 Step 边界），xllm 接手后用自己的 `InferenceHook` 在每次推理前提交。`usage.llm_requests` 由每个执行段累加：libOpenDAN 在每个 outcome 后加上本段经其 `LlmClient::infer` 发起的 Round 数，xllm 接手后加上自己的请求数，任何一方都不覆盖已有值；xllm 接手后的推理只出现在 run.json 中，不进入 session 的 `static.json`。

快照的 `state.host` 与 Run 的 `host` 是不同层的元数据，续跑必须保留。`host.env_check` 当前只是保存的数据，`XllmRun::resume` 没有通用 PATH / 环境检查器；不能把声明的环境要求描述为已经自动验证。工作目录并发协调、输入消费提交以及其它宿主恢复条件仍由宿主负责。

## 12. 使用示例

### 12.1 最小目录配置

以下 `.llm_context` 使用当前 BuckyOS 身份，模型逻辑名需在 AICC 环境可用。先创建工作目录，将文件放在该目录下：

```yaml
provider:
  type: buckyos
model: llm.chat
runs_dir: ./runs
loop_model: function_call
max_tool_iterations: 8
timeout: 3600
llm_timeout: 600
result_format: raw
run_logs: info
tools:
  enabled: false
  filesystem_policy: workspace
prompt:
  select: default
  groups:
    default:
      default_user: 请概括本次任务的目标。
      sections:
        role: 你负责执行一次独立任务，并给出简短、可核对的结果。
        contexts: '工作目录：{{runtime.cwd}}'
```

```bash
# 全新独立任务；--tools 覆盖文件中的 enabled: false，启用内置 bash 组
agent_tool xllm --dir /tmp/xllm-demo --tools '列出当前目录文件，并给出简短说明。'

# 查询当前工作目录的 Run
agent_tool xllm list --dir /tmp/xllm-demo --limit 10
agent_tool xllm status --dir /tmp/xllm-demo --format json

# 恢复该目录最近一个非终态 Run，不传入新任务正文
agent_tool xllm --dir /tmp/xllm-demo --resume

# 指定 Run 与实际存储位置；提高非终态 Run 的总工具迭代额度（已消费的不退还）
agent_tool xllm --resume --run '<run_id>' --runs-dir /tmp/xllm-demo/runs --max-tool-iterations 16

# 重新导出已保存原文，不调用模型
agent_tool xllm result --run '<run_id>' --runs-dir /tmp/xllm-demo/runs \
  --result-format raw --output /tmp/xllm-result.txt
```

### 12.2 管道、附件与结构化结果

```bash
# stdin 在已有任务要求时作为补充材料
printf '%s\n' '第一条记录：完成接口调整。' |
  agent_tool xllm --no-tools '把输入材料整理成一条摘要。'

# 附件使用启动 cwd 的相对路径；与 --dir 指向的位置无关
agent_tool xllm --file ./notes.txt --image ./diagram.png '结合材料解释这张图。'

# 原生 function_call 的 JSON 输出要求与 CLI JSON 包装是两个开关
agent_tool xllm --loop-model function_call --no-tools \
  --result-format raw --json --format json '返回 JSON 对象，其中 answer 为 42。'

# /tmp/messages.json 的内容为 §3.3 所示的 AiMessage 数组
agent_tool xllm --input-file /tmp/messages.json --no-tools --format json
```

如需 behavior，把配置调整为：

```yaml
loop_model: behavior
result_format: result.report
tools:
  enabled: true
  tools2actions: true
  tools:
    - groupname: bash
```

模型动作示例为 `<read_file path="notes.txt"/>` 或 `<read_file><![CDATA[notes.txt]]></read_file>`，正文直接承载参数值，不加 `path:` 前缀。最终回复使用 `<response><report end="true"><![CDATA[最终结果]]></report></response>`；上面的 result_format 只导出 report 文本。

### 12.3 Rust SDK

在已有 tokio async 上下文中调用：

```rust
use std::path::Path;
use agent_tool::xllm::{TaskInput, TaskOverrides, XllmDeps, XllmRun, XllmTask};

async fn run_task() -> Result<(), Box<dyn std::error::Error>> {
    let deps = XllmDeps::default();
    let prepared = XllmTask::prepare(
        Path::new("/tmp/xllm-demo"),
        TaskInput::question("用一句话说明本次任务目标。"),
        TaskOverrides { tools: Some(false), ..Default::default() },
        &deps,
    ).await?;
    let mut run = XllmRun::start(prepared, deps).await?;
    let outcome = run.execute().await?;
    println!("{}: {}", outcome.record().run_id, outcome.status());
    Ok(())
}
```

调用方按 `RunOutcome` 分支处理暂停、中断与失败；成功交付还需检查 result 的提取 / JSON 校验结果。SDK 不需要解析 stdout 或进程退出码。

## 13. 核对与实现边界

与本协议直接相关的已有测试可在 `src/` 目录运行：

```bash
cargo test -p agent_tool --lib xllm -- --test-threads=1
cargo test -p agent_tool --lib run_local_llm -- --test-threads=1
cargo test -p llm_context --lib suspension -- --test-threads=1
```

这些测试覆盖配置继承、组与模板、工具优先级、输入顺序、Run 落盘、Provider 分类、暂停 / 中断 / 恢复、工具迭代额度与 `llm_requests` 累加、工具取消、文件模型、结果提取、上下文压缩及快照挂起回填。它们使用脚本化模型或本地 mock，不替代真实 AICC / MCP 环境的联调。

跨语言或宿主实现接手 Run 时，应对齐已支持的记录版本、运行协议、快照语义和锁 / 提交边界，并保留宿主元数据。当前没有旧 run_local_llm 目录自动迁移、跨 Run 记忆 / append、通用 deferred 回填、MCP 完整会话管理或 exactly-once 外部副作用保证；不得从共享类型中存在某个字段推导出 CLI 已实现对应能力。
