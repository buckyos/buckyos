# run_local_llm SDK 化：目录与命令行协议基线

> **2026-09-18 更新**：Rust `LocalLLMContext` 已按 [xllm PRD](../../product/xllm/PRD.md) 重写为 xllm SDK 核心（`src/frame/agent_tool/src/local_llm_context.rs`），命令行入口改为 `agent_tool xllm ...`（`run_local_llm` 仅作为别名保留），Run 目录格式、请求、结果与退出码均已重新定义，不兼容本文第 2–12 节描述的旧格式。新实现的协议摘要见 [xllm Rust SDK 参考](xllm_rust_sdk.md)。本文其余内容仍作为旧实现的基线记录保留。


本轮目标是以 `run_local_llm` 工具为起点，将 OpenDAN 的任务执行能力 SDK 化，并通过新的 TypeScript agent tools 提供命令行入口。SDK 提供可编程接口，CLI 封装 SDK；新设计可以重新定义目录协议、请求、结果及命令行参数。

本文记录当前 Rust 工具依赖的目录数据、命令行输入输出和运行生命周期，作为设计参考。第 2–12 节是现状说明，不是新 TS SDK 必须遵守的兼容规范；旧实现的缺陷和偶然行为不作为新设计约束。

基线日期：2026-09-17。核对仓库 HEAD：`ffbfa5fa79c77bb375a19eece28ab37fb34750e0`。文档依据当前源码的执行逻辑，源码注释与逻辑冲突时以逻辑为准。

本文的“当前行为”描述已经存在的 Rust 行为；“TS 建议”明确表示重实现时的修订建议，不代表 Rust 已经实现。按照仓库当前开发规则，不默认要求新实现兼容旧 CLI 或接管 Rust 历史运行目录。Rust `LocalLLMContext` 库的修改，以及已有调用方的迁移，是独立工作，不是本轮工具 SDK 化的前置条件。

## 1. 范围与入口

### 1.1 本轮 SDK 化的职责边界

| 层次 | 本轮设计职责 |
| --- | --- |
| TS SDK 调用接口 | 让程序直接提交结构化任务、配置执行策略、接收结构化结果和错误，无需拼接 shell 命令或解析 stdout |
| TS SDK 执行能力 | 组织任务生命周期、工作目录、持久化和恢复；由新的协议明确支持范围 |
| 依赖接入 | 在 SDK 边界表达 LLM 访问、工具执行和本地环境依赖，不把必要能力藏在 CLI 的全局初始化中 |
| TS agent tools CLI | 将 argv/stdin 转为 SDK 请求，将 SDK 结果映射为 stdout/stderr 和退出码，不另写一套执行逻辑 |

当前 Rust 库已被 `llm_understand_media` 和 `llm_explore` 复用，其中附件理解工具接入了 OpenDAN/Jarvis。这个事实只约束将来替换或修改 Rust 库时的影响范围，不要求本轮 TS SDK 迁移这些调用方，也不要求复刻 Rust 的内部 API。

本轮从单次本地 LLM 任务执行能力开始。完整 Agent Runtime、behavior 调度和现有 Rust 调用链的改造分别处理，不随这个工具自动纳入。

### 1.2 当前 Rust 实现的三个对象

以下对象用于理解现有实现：

| 对象 | 职责 | 对外入口 |
| --- | --- | --- |
| `LocalLLMContext` | 将本地目录、请求和模型客户端组合成一次可落盘的 LLM 任务；驱动 outcome，组织恢复 | Rust 库 API |
| `LLMContext` | 推理、工具调用、消息历史、预算与结果处理的底层循环 | 由 LocalLLMContext 调用 |
| `run_local_llm` | 组装请求、接入 AICC、选择启动方式、输出结果 | `agent_tool run_local_llm ...` |

这里的“local”指执行目录和工具在本地，**模型调用仍通过 AICC**，不表示本机部署模型。

一个 context 根目录可以保存多次 run，所有 run 共用 `workspace/` 和 `bin/`。一次 run 可以包含多次模型推理和工具调用；一次 CLI 进程通常驱动一个 run。`--append` 是从上一 run 的历史构造一个新 run。

该入口使用传统的 function/tool-call loop。`build_deps` 没有配置 behavior parser/renderer，因此不解析 XML behavior、不切换行为状态、不维护长期记忆，也不自动投递消息。

主要源码入口：

- [local_llm_context.rs](../../src/frame/agent_tool/src/local_llm_context.rs)：目录、状态、请求、恢复、工具装配。
- [run_local_llm.rs](../../src/frame/agent_tool/src/run_local_llm.rs)：CLI、AICC 适配、KeepTail 压缩。
- [agent_tool_cli_dev/src/lib.rs](../../src/frame/agent_tool_cli_dev/src/lib.rs)：子命令分发。
- [request.rs](../../src/frame/llm_context/src/request.rs)、[state.rs](../../src/frame/llm_context/src/state.rs)、[outcome.rs](../../src/frame/llm_context/src/outcome.rs)：落盘共享类型。
- [context_loop.rs](../../src/frame/llm_context/src/context_loop.rs)、[deps.rs](../../src/frame/llm_context/src/deps.rs)：字段实际执行语义。
- [aicc_client.rs](../../src/kernel/buckyos-api/src/aicc_client.rs)：消息、资源、模型响应类型。

## 2. 目录协议

### 2.1 布局与所有权

```text
<dir>/
├── .lock
├── workspace/
├── bin/
└── runs/
    └── <run_id>/
        ├── request.json
        ├── state.json
        ├── snapshots/
        │   ├── 0001.snap.json
        │   ├── 0002.snap.json
        │   └── ...
        └── outcomes/
            └── final.json
```

| 路径 | 内容与责任方 | 创建/更新时机 |
| --- | --- | --- |
| `<dir>` | 一组本地任务的持久化根目录 | 创建 context 时按需创建 |
| `workspace/` | 用户文件、输入材料和工具生成的文件；默认 shell cwd、文件工具 root | 初始化创建；跨 run 保留 |
| `bin/` | 用户放置的可执行脚本，作为 shell PATH overlay | 初始化仅创建空目录，不生成脚本、不自动 chmod |
| `.lock` | OS 排他锁的载体，无 PID/租约/JSON 内容协议 | 获取目录锁时创建或打开 |
| `runs/<run_id>/request.json` | 原始 `OneShotRequest`，审计和 follow-up 配置来源 | 新 run 写一次 |
| `runs/<run_id>/state.json` | `RunMetaState`，run 发现与恢复入口 | 初始化、outcome 边界、压缩恢复后更新 |
| `snapshots/<idx>.snap.json` | 完整 `LLMContextSnapshot` | 启动前、推理前、outcome 后、压缩 resume 前 |
| `outcomes/final.json` | 终态 `LLMContextOutcome` | `Done`、`Error`、`BudgetExhausted` 时写入 |

当前没有 root manifest、schema version、`latest` 链接、独立配置文件或目录清理策略。`worklog.jsonl` 只存在于旧注释中的布局示意，实际使用 `NoopWorklogSink`，**不会生成 worklog 文件**。文件写审计也是 no-op。

目录本身不保存模型客户端、认证 token、连接配置、工具进程或锁句柄。恢复时由当前进程重新建立这些依赖。

### 2.2 路径解释

- CLI 的 `--dir`、`--input-file`、`--output` 都直接构造路径，相对路径基于 CLI 启动 cwd；`--input-file` 和 `--output` 不相对于 `workspace/`。
- CLI 不执行 `~`、环境变量或 glob 展开；这些只有调用方 shell 在传参前展开才生效。
- 工具的相对文件路径基于 `<dir>/workspace`；`exec_bash` 默认 cwd 也是这个目录。
- `--dir` 不自动成为 prompt 内容；`bin/` 内的命令也不自动生成 tool spec 或使用说明。需要让模型知道的材料、命令用法由调用方写进 input。
- 当前代码允许使用已有目录、文件和符号链接，不会重置 workspace，也不会在新 run 时复制 workspace。

建议调用方传绝对 `--dir`。当前 `bin/` overlay 保存的是原始拼接路径：相对 `--dir` 会使 PATH 中的相对项按子 shell 的 cwd 再解析，可能无法命中预期的 `bin/`。文件工具的路径检查也未统一正规化 root。

### 2.3 run_id

当前格式为 `YYYYMMDD-HHMMSS-xxxx`，例如 `20260917-103045-a7f3`。

- 前半部分是创建时的**本地时间**，不是 UTC。
- `xxxx` 是四位小写十六进制，来自 Unix 毫秒时间的计算：`(now_ms & 0xffff) XOR ((now_ms >> 16) & 0xffff)`。
- 后缀不是随机数，不是四字节随机标识，也没有冲突重试；同一毫秒创建可能冲突。
- run 选择依据 `last_updated_unix_ms`，不按目录名或创建时间排序。

消费方应将 run_id 当作不透明字符串。TS 新实现应保证唯一性，不需要复刻当前时间后缀算法。

### 2.4 JSON 通则

- 正式文件是 UTF-8 JSON，Rust 使用 pretty serialization；键顺序和缩进不应作为消费者的判断依据。旧语义哈希涉及序列化字节，是特殊例外，见 §3.3。
- 没有统一的顶层 `version` 或 `agent_tool_protocol` 字段。
- `state.json.status` 使用 PascalCase 字符串；outcome 使用小写 `kind`；消息 content 使用小写 `type`。三者不能混用。
- `u32` 范围是 `0..4294967295`；`u64` 范围是 `0..18446744073709551615`。时间戳单位为 Unix 毫秒，token/次数为整数。
- 下文 `T?` 表示字段可以缺省或为 null；具体 Rust 写出时是否省略，按相应表说明。一个字段“有默认值”不意味着显式 null 对非 Option 类型也合法。
- 当前 serde 类型没有 `deny_unknown_fields`；普通结构体的未知字段会被忽略，重新写出时也不会保留。枚举的未知 variant 不能读取。

## 3. state.json 与请求身份

### 3.1 RunMetaState

| 字段 | JSON 类型 | 含义 |
| --- | --- | --- |
| `run_id` | string，必需 | 该 run 的标识，也是子目录名 |
| `created_at_unix_ms` | u64，必需 | run 创建时间 |
| `last_updated_unix_ms` | u64，必需 | 最近一次元数据刷新时间，不是心跳 |
| `request_semantic_hash` | u64，必需 | 对新传入请求进行 auto-resume 校验的旧 Rust 哈希 |
| `status` | string，必需 | `Running` / `Suspended` / `Completed` |
| `latest_snapshot_idx` | u32? | 恢复所读取的快照序号；初稿为 null |
| `last_suspend_kind` | string? | `PendingTool` / `ContextLimitReached` / `Interrupted` |

示例是字段形状示例，哈希和序号必须由实际运行生成：

```json
{
  "run_id": "20260917-103045-a7f3",
  "created_at_unix_ms": 1789666245000,
  "last_updated_unix_ms": 1789666245100,
  "request_semantic_hash": 12345678901234567890,
  "status": "Running",
  "latest_snapshot_idx": 1,
  "last_suspend_kind": null
}
```

Rust 正常写出全部七个字段，包括值为 null 的两个 Option 字段。`Completed` 表示已经到终态，不等于任务成功；成功必须看 `final.json.kind == "done"`。

### 3.2 run 发现规则

扫描 `runs/*/state.json`。不存在 state 的条目，以及读取失败或 JSON/类型解析失败的 state，会被静默跳过；目录枚举本身失败则报错。

- 自动恢复/新建检查：先筛选 `status == "Running"`，再取 `last_updated_unix_ms` 最大者。
- append：先在**所有状态**中取时间最大者，再要求它是 `Completed`；不会跳过最新的未完成 run 去找更早的 Completed。
- 多个 Running 只选择最新者；源码注释提到的 warning 实际未输出。
- 时间相同时没有额外的稳定排序规则，选择结果依赖目录枚举顺序。
- 当前不校验 state 内的 run_id 与被扫描目录名一致；后续读文件使用 state 内的 run_id。

### 3.3 request_semantic_hash

实际算法等价于：

```text
h = Rust DefaultHasher::new()
Hash(objective: String, h)
Hash(serde_json::to_vec(input): Vec<u8>, h)
return h.finish(): u64
```

只包含 `objective` 和 `input`。以下全部不参与：`model_policy`、`tool_policy`、`output`、`budget`、`human_policy`、`error_policy`。

因此改变 model、温度、工具开关或输出模式，仍可能命中旧 Running run；恢复后使用旧快照，**新传入的这些配置不会生效**。反之，仅修改不进入 prompt 的 objective，也会导致哈希不匹配。

这是 Rust 实现细节，不是稳定的跨语言摘要标准：

- 算法依赖 Rust `DefaultHasher` 和 `Hash` 编码，不能直接替换为“JSON 字符串 SHA-256”并声称兼容。
- `AiContent.tool_use.args` 是 HashMap，序列化字节顺序不是规范化 JSON；相同语义未必跨进程得到相同字节。
- 哈希是 JSON number，但可能超过 JS `Number.MAX_SAFE_INTEGER`。若读取旧目录，普通 `JSON.parse` 可能已经损失精度，之后转 BigInt 也无法补救。
- 比较的是 state 中的哈希和调用方新请求的哈希；不会另行验证 `request.json`、snapshot.request 与 state 彼此一致。

TS 新格式建议使用明确版本、规范化输入和字符串摘要；未增加迁移机制前，不用新算法直接接管 Rust Running run。

## 4. request.json 协议

### 4.1 OneShotRequest

| 字段 | 类型 | 缺省/null 时的 lowering 结果 |
| --- | --- | --- |
| `objective` | string，必需 | 无默认值；用于标识/审计，**不加入 prompt** |
| `input` | `AiMessage[]`，必需 | 无默认值；调用方已经准备好的完整初始消息历史 |
| `model_policy` | ModelPolicy? | 使用 §4.2 默认值 |
| `tool_policy` | ToolPolicy? | 使用 §4.3 默认值 |
| `output` | OutputSpec? | `{"kind":"text"}` |
| `budget` | BudgetSpec? | 使用 §4.4 默认值，并补 context 阈值 |
| `human_policy` | HumanPolicy? | `{"approval_required":[]}` |
| `error_policy` | ErrorPolicy? | `{"max_consecutive_errors":3}` |

Rust 写出时，六个未设置的覆盖字段会写为 null；文件读入时也允许省略这些 Option 字段。库构造函数不要求 objective/input 非空；CLI 会拒绝组装后消息数为零的 input。

```json
{
  "objective": "整理 workspace 中的文件",
  "input": [
    {
      "role": "user",
      "content": [{"type": "text", "text": "列出当前目录文件，并简要说明。"}]
    }
  ],
  "model_policy": {
    "preferred": "default-llm",
    "fallbacks": [],
    "temperature": null,
    "max_completion_tokens": null,
    "provider_options": null
  },
  "tool_policy": null,
  "output": null,
  "budget": null,
  "human_policy": null,
  "error_policy": null
}
```

`request.json` 是运行记录；CLI 没有“读取完整 OneShotRequest”的参数。`--input-file` 读取的仅是 AiMessage 数组。手工编辑 request.json 也不会覆盖恢复时 snapshot.request 中的策略。

### 4.2 ModelPolicy

| 字段 | 类型 | 默认值 |
| --- | --- | --- |
| `preferred` | string | `""` |
| `fallbacks` | string[] | `[]` |
| `temperature` | f32? | null |
| `max_completion_tokens` | u32? | null |
| `provider_options` | 任意 JSON? | null |

这些非 Option 字段允许缺省并填默认值。`fallbacks` 会保存到快照，但当前 AICC CLI 适配器不使用 fallback 列表。CLI 的 `--max-tokens` 对应此处的 `max_completion_tokens`。

### 4.3 ToolPolicy

| 字段 | 类型 | 默认值 | 当前本地路径的作用 |
| --- | --- | --- | --- |
| `mode` | `none` / `whitelist` / `all` | `all` | 控制工具声明；`none` 时不派发工具 |
| `whitelist` | string[] | `[]` | whitelist 模式下过滤向模型声明的工具 |
| `action_mode` | 同 mode | `all` | behavior action 策略；本入口无 behavior parser |
| `action_whitelist` | string[] | `[]` | 同上 |
| `max_rounds` | u32 | 8 | 工具轮数额度；不是模型调用总次数 |
| `max_calls_per_round` | u32 | 8 | 一轮模型返回的工具调用数上限 |
| `max_observation_bytes` | u32 | 32768 | 当前传统循环未按此值截断 observation |
| `disable_capabilities` | string[] | `[]` | 传给 AICC requirements.extra |
| `parallel` | boolean | false | 当前传统循环始终串行执行工具 |
| `allow_deferred` | boolean | false | Pending 路径见 §8.5；设 true 也尚未实现等待 |

当前装配的是 `AllowAllPolicy`。whitelist 的声明过滤不等于派发阶段权限校验，`human_policy.approval_required` 也不会触发审批流程。

### 4.4 BudgetSpec、HumanPolicy、ErrorPolicy

| BudgetSpec 字段 | 类型 | 默认值 |
| --- | --- | --- |
| `max_total_tokens` | u32? | null |
| `max_completion_tokens` | u32? | null |
| `max_wallclock_ms` | u64? | null |
| `max_cost_units` | u32? | null |
| `on_exhausted` | `fail` / `return_partial` / `escalate_human` | `fail` |
| `context_yield_threshold` | ContextThreshold? | request 原值为 null；lowering 后补 `{"kind":"ratio","value":0.75}` |

ContextThreshold 只有两种形状：`{"kind":"ratio","value":0.75}` 和 `{"kind":"absolute_tokens","value":32000}`。即便提供了自定义 budget，只要阈值为空，LocalLLMContext 仍补默认 ratio；无法通过 null 在这一层禁用阈值。

当前实际执行限制：

- wallclock 在每次推理前检查，已用时间 **大于** 上限时退出；起点在 LLMContext 创建时设定，恢复保留原起点，停机时间计入。它不是对 in-flight 推理的定时取消。
- total tokens 在收到响应并累计 usage 后检查，累计 **大于** 上限才退出；只用 provider 返回的 total_tokens，不由 input/output_tokens 自动推算。
- `budget.max_completion_tokens`、`max_cost_units`、`on_exhausted` 没有在当前传统循环中实现对应控制分支。单次输出 token 限制由 `model_policy.max_completion_tokens` 下发。
- **context 阈值目前仅被保存，没有执行检查；当前循环没有产生 ContextLimitReached 的路径。** 默认 75% 不构成已生效的自动压缩保证。

HumanPolicy 为 `{"approval_required": string[]}`，默认空数组。ErrorPolicy 只有 `max_consecutive_errors: u32`，默认 3，没有旧注释提到的 `mode: Suspend` 字段。只有 LLM 可纠正的错误（`output_parse`、`policy_rejected`、`tool_failed`）参与计数：按逻辑轮计数，同一轮多个工具错误只计 1，最多反馈 N 次，连续第 N+1 次终止，即默认第 4 次触发；0 关闭上限。整轮无可纠正错误才清零，推理请求成功本身不清零。输出协议错误回灌为 user 消息，工具 / Policy 错误按 call_id 回灌为 tool_result。Provider、运行时、快照、内部错误不计数，直接结束 run。

## 5. AiMessage 输入与历史格式

消息不是 OpenAI 风格的 `{"role":"user","content":"text"}`，而是：

```json
[
  {
    "role": "system",
    "content": [{"type": "text", "text": "你负责整理当前工作目录。"}]
  },
  {
    "role": "user",
    "content": [{"type": "text", "text": "先列出文件。"}]
  }
]
```

`role` 必需，枚举为 `system`、`user`、`assistant`、`tool`、`developer`；`content` 必需，是有序 block 数组。CLI 的文本参数各自构造一个 text block，不解析其中的 JSON、XML 或模板。

| block.type | 其它字段 | 含义 |
| --- | --- | --- |
| `text` | `text: string` | 文本 |
| `image` | `source: ResourceRef` | 图片引用 |
| `document` | `source: ResourceRef`，`title: string?` | 文档引用 |
| `tool_use` | `call_id: string`，`name: string`，`args: object`，args 默认 `{}` | assistant 工具调用 |
| `tool_result` | `call_id: string`，`content: AiToolResultContent[]`，`is_error: boolean` 默认 false | 工具响应；is_error=false 时省略 |
| `thinking` | `summary: string?`，`text: string?`，`provider_metadata: JSON?` | 推理/思考信息 |
| `provider_state` | `provider: string`，`value: JSON` | provider 特有状态，需保留其原始值 |

AiToolResultContent 只允许 `text`、`image`、`document` 三种 block，字段与上表对应，不允许嵌套 tool_use/tool_result/thinking。

ResourceRef 使用 `kind`：

| kind | 字段 |
| --- | --- |
| `url` | `url: string`，`mime_hint: string?` |
| `base64` | `mime: string`，`data_base64: string` |
| `named_object` | `obj_id: ObjId`，沿用公共 NDN ObjId 序列化 |

消息角色与 block 的组合还受公共 AiMessage 校验规则约束，例如 tool role 应携带恰好一个 tool_result。当前 `--input-file` 仅反序列化，没有单独调用消息语义 validate；不要把“能读入文件”视为 provider 一定接受。TS 应复用公共消息协议，不能丢弃非文本 block 或将所有 content 扁平化成字符串。

## 6. 快照协议

### 6.1 文件名与选择

`snapshots/<idx>.snap.json` 中 idx 为 u32，正式写出从 1 开始，**至少四位**十进制补零；10000 对应 `10000.snap.json`，不是截为四位。

- 分配下一序号：扫描所有文件名，接受能够去掉 `.snap.json` 后解析为 u32 的名称，取最大值 + 1。
- 扫描只看名称，不验证该项是否是有效快照文件，也不要求名称已经补零。
- 恢复和 append：读取 `state.latest_snapshot_idx` 对应的**规范补零文件名**，不扫描并选择最大的快照。
- 缺失、损坏、索引为空均不自动回退到其它快照。`.tmp` 文件不参与序号扫描。

### 6.2 LLMContextSnapshot

顶层恰为两个主要字段：`request: LLMContextRequest` 和 `state: LLMContextState`。没有序号、版本、校验和、外部依赖句柄。

snapshot.request 是补齐默认值的请求，与原始 request.json 不同：

| 字段 | 本地 lowering 结果 |
| --- | --- |
| `owner` | `{"kind":"one_shot","id":"<run_id>"}` |
| `trace` | run_id |
| `objective`、`input` | 原请求内容 |
| `model_policy`、`tool_policy`、`output`、`budget`、`human_policy`、`error_policy` | 全部补齐，使用 §4 的默认值 |
| `behavior_name` | 空字符串，序列化时省略 |
| `forbid_next_behavior` | false，正常写出 |

snapshot.state 字段：

| 字段 | 类型 | 初始值/说明 |
| --- | --- | --- |
| `accumulated` | AiMessage[] | 初始克隆 request.input；随后累积 assistant/tool/错误消息 |
| `usage` | AiUsage | 初始 `{}`；没有值的统计项省略 |
| `rounds_left` | u32 | 初始 tool_policy.max_rounds |
| `started_at_ms` | u64 | LLMContext 创建时的 Unix 毫秒时间 |
| `cost_units` | u32 | 初始 0；本路径未累计成本 |
| `consecutive_errors` | u32 | 初始 0 |
| `pending_tool_calls` | PendingToolCall[] | 缺省为空，空时省略 |
| `llm_task_ids` | string[] | provider_task_ref 集合；空时省略 |
| `steps` | StepRecord[] | behavior 历史；本地正常运行为空，空时省略 |
| `history_summaries` | HistorySummaryRecord[] | 本地正常运行为空，空时省略 |
| `history_inputs` | HistoryInputRecord[] | 本地正常运行为空，空时省略 |
| `last_step` | StepRecord? | 本地正常运行为空，空时省略 |
| `last_report` | string? | 本地正常运行为空，空时省略 |
| `next_step_index` | u32 | 缺省/初始 0，正常写出 |
| `next_action_id` | u32 | 缺省/初始 0，正常写出 |

共享 behavior 类型的精确定义见 [behavior_loop.rs](../../src/frame/llm_context/src/behavior_loop.rs)。这些字段是底层共享快照的组成部分，不表示本工具启用了 behavior 执行；TS 本地执行无需实现另一套 behavior scheduler。如果实现通用快照导入，应保留它们或明确拒绝不支持的快照。

AiUsage 的四个字段均为可省略 u64：`input_tokens`、`output_tokens`、`total_tokens`、`request_units`。本地累计使用饱和加法，不把未知值当成已测得的零。

PendingToolCall 形状为 `{call: {name, args, call_id}, eta_ms?: u64}`；args 是 JSON object。

### 6.3 实际写入顺序与原子性

| 时机 | 实际顺序 |
| --- | --- |
| 新 run | 写 request.json → 写 Running 且索引为空的 state → 构造依赖/上下文 → 写初始 snapshot → 写带索引的 state |
| 每次推理前 | hook 写下一 snapshot → 写带新索引的 state。任一步失败 ⇒ 不发起推理，`step()` 返回 `RuntimeFailure`，上下文留在内存 |
| 每次 outcome 返回后 | 写 ctx.snapshot → （终态）写 final.json → 按 outcome 写 state。任一步失败 ⇒ `CommitFailed{stage}`，outcome 与快照留在 `pending_outcome()`，`retry_commit()` 只补写未完成阶段 |
| 压缩 resume | 写压缩后的 snapshot 并提交索引 → 以 rewritten history resume → 清 last_suspend_kind 并写 state |

state 和 final 使用“临时文件 + 同目录 rename”：临时名分别是 `state.json.tmp`、`final.json.tmp`。snapshot 使用 `path.with_extension("snap.json.tmp")`，实际临时名为 **`0001.snap.snap.json.tmp`**。request.json 直接写入，没有临时文件。CLI `--output` 文件也直接覆盖写入。

rename 保证单文件替换的崩溃一致性；没有 fsync，不保证断电持久性；没有跨文件事务。仍可能出现：

- final.json 已写、state 仍为 Running：`resume_or_new` 识别后只补齐 state（索引指向最新快照、Completed），不重跑推理。
- 进程在工具副作用完成后、下一个推理前检查点提交前退出，恢复会重复该段推理或工具调用。

因此当前实现**不保证 exactly-once，也不保证不重复扣费**。检查点应解释为“可能重放的恢复位置”，不能作为外部副作用提交凭证；工具幂等性属于 adapter。

## 7. outcome 与 final.json 协议

### 7.1 顶层 union

CLI 输出和 `outcomes/final.json` 使用相同的 `LLMContextOutcome` JSON，没有 AgentToolResult 外层封装。

| kind | 字段 | 是否归档 final.json |
| --- | --- | --- |
| `done` | `output`、`usage`、`response`、`trace`；可选 `reason`、`behavior_result` | 是 |
| `error` | `error: LLMComputeError`、`usage`、`trace` | 是 |
| `budget_exhausted` | `which`、`usage`；可选 `partial: ContextOutput` | 是 |
| `pending_tool` | `pending: PendingToolCall[]`、`snapshot`；可选 `deadline_ms` | 否 |
| `context_limit_reached` | `which`、`usage`、`accumulated`、`snapshot`；可选 `deadline_ms` | 否，driver 收到后尝试压缩 |
| `interrupted` | `reason`、`usage`、`snapshot`、`abort` | 否 |

`budget_exhausted.which`：`tokens` / `wallclock` / `cost_units` / `tool_rounds`。

`context_limit_reached.which`：`approaching_window` / `hard_limit` / `provider_refused`。

完整 union 的存在不代表每个 variant 都可由当前 CLI 产生；实际支持边界见 §8.5。

### 7.2 ContextOutput、AiResponse、trace

ContextOutput：

```json
{"kind":"text","content":"完成"}
```

或：

```json
{"kind":"json","content":{"answer":"完成"}}
```

JSON output 的 content 可以是任意合法 JSON 值，不强制 object。`--json` 设置 `strict=false`：模型正文 JSON 解析失败时，仍返回 `done`，output 降级为 text，CLI 仍退出 0。它不会让 CLI 只输出模型正文。

AiResponse 的必需字段为 `message: AiMessage`；可省略字段为 `usage: AiUsage`、`cost: {amount: number, currency: string}`、`finish_reason: string`、`provider_task_ref: string`、`extra: JSON`。outcome.usage 是 run 累计统计，response.usage 通常是最后一次响应统计。

trace 字段：

- `trace_id: string`，通常为 run_id。
- `latency_ms: u64`，从 state.started_at_ms 到完成的耗时，恢复停机时间计入。
- `tool_trace?: ToolExecRecord[]`，非空时写出。每条为 `tool_name: string`、`call_id: string`、`status: "succeeded" | "failed" | "unknown" | "not_executed"`、`duration_ms: u64`、可选 `error: string`。`unknown` 表示派发基础设施在调用可能已开始后失败，`not_executed` 表示批次中止或被 Policy 拒绝而未派发。`error` outcome 同样携带 `trace`。
- `llm_task_ids?: string[]`，非空时写出。finish_done 会从 state 中取走此列表，因此 Done 后另存的 snapshot.state 不再带该列表。

恢复时 tool_trace 和 last_response 重新初始化，没有完整历史恢复；不能把最终 trace 当成所有恢复阶段的完整审计日志。

成功示例：

```json
{
  "kind": "done",
  "output": {"kind": "text", "content": "工作完成。"},
  "usage": {"input_tokens": 120, "output_tokens": 15, "total_tokens": 135},
  "response": {
    "message": {
      "role": "assistant",
      "content": [{"type": "text", "text": "工作完成。"}]
    },
    "usage": {"input_tokens": 120, "output_tokens": 15, "total_tokens": 135}
  },
  "trace": {"trace_id": "20260917-103045-a7f3", "latency_ms": 1200}
}
```

本入口的 `behavior_result` 为空并省略。其共享类型定义也在 behavior_loop.rs，不能将它误作 TS 本地工具的调度指令。

### 7.3 LLMComputeError

| error.kind | 附加字段 | 来源 | 是否曾喂回 LLM |
| --- | --- | --- | --- |
| `timeout` | 无 | provider | 否 |
| `cancelled` | 无 | provider | 否 |
| `provider` | `failure: "transient" \| "permanent" \| "unknown"`、`message: string` | provider | 否 |
| `output_parse` | `message: string` | llm_output | 是（自纠正上限耗尽） |
| `policy_rejected` | `message: string` | tool | 是 |
| `tool_failed` | `tool: string`、`call_id: string`、`message: string` | tool | 是 |
| `tool_runtime` | `tool`、`call_id`、`message`、`effect_unknown: boolean` | runtime | 否 |
| `checkpoint` | `stage: "before_inference" \| "outcome_boundary"`、`message: string` | runtime | 否 |
| `snapshot_corrupted` | `message: string` | snapshot | 否 |
| `internal` | `message: string` | internal | 否 |

`failure` 只有 `transient` 表示上层可以安全重跑；`unknown` 不等于可重试。`tool_runtime` / `checkpoint` 不会进入 final.json：`step()` 把它们转成 `RuntimeFailure` 并保留内存上下文（见 §8.4）。

```json
{
  "kind": "error",
  "error": {"kind": "provider", "failure": "transient", "message": "aicc llm.chat failed"},
  "usage": {},
  "trace": {"trace_id": "20260510-103045-a7f3", "latency_ms": 12}
}
```

目录 IO、锁失败、CLI 参数错误等 `LocalLLMContextError`/驱动异常不自动转换为这个 JSON。它们通过 stderr 和退出码报告，可能没有 outcome 输出。

### 7.4 Interrupted.abort

字段为 `reason: string`、`requested_at_ms: u64`、`observed_at_ms: u64`、`provider_cancel_supported: boolean`（读入缺省为 true）、可选 `provider_task_ref: string`。

Interrupted 的 snapshot 是推理开始前的状态，不包含部分 assistant 输出。但当前 CLI 没有中断接口，LocalLLMContext 也未公开底层 interrupt handle；不要将收到 OS 信号直接等同于已落盘的 interrupted outcome。当前 AICC 适配器没有远端 cancel 能力。

## 8. 生命周期、恢复与追加

### 8.1 目录锁

创建或恢复前，先确保 runs/workspace/bin 存在，再对 `.lock` 调用非阻塞排他 flock；被其它进程持有时立即报 LockFailed。

同进程用 canonicalized 根目录作为 registry key，重复获取直接成功；文件句柄保存在静态 registry，直到进程退出才释放。`drop(LocalLLMContext)` 不释放它，`.lock` 文件保留也不表示仍有活进程。

这不是同进程多 context 的并发调度器。同进程并发写同一目录没有额外保护；快照的“扫描最大值 + 1”也不是线程安全的序号分配。当前支持的使用方式是每个目录一个串行 driver。

### 8.2 三种启动方式

| 方式 | 选择逻辑 | 已有记录的处理 |
| --- | --- | --- |
| `new_run` / CLI `--new` | 发现任意 Running 即报错，否则新建 | 不覆盖、不删除旧 run；Suspended 不阻止新建 |
| `resume_or_new` / CLI 默认 | 没有 Running 则新建；有则比较 semantic_hash | 不匹配即拒绝；匹配进入 do_resume |
| `--append` | 从最近 run 生成 follow-up，再调用 new_run | 最近 run 必须 Completed；不复用其 run_id |

`--new` 的“force”仅表示不走自动恢复，**不表示能强制覆盖 Running run**。如果目录只有 Completed 或 Suspended，默认调用会新建 run。默认调用也不会自动复用 Completed 的历史，只有 append 会继承。

### 8.3 自动恢复算法

```text
ensure_layout(dir)
acquire_lock(dir)
meta = latest Running run by last_updated_unix_ms
if no meta:
    new_run(dir, incoming_request)
else:
    require incoming_request.semantic_hash == meta.request_semantic_hash
    stored_request = read request.json
    require meta.latest_snapshot_idx exists
    snapshot = read snapshots[meta.latest_snapshot_idx]
    require meta.last_suspend_kind == null
    rebuild tools/client/deps
    LLMContext.resume(snapshot, ResumeFromMidRun, deps)
```

read request.json 的结果供 LocalLLMContext 保留和压缩配置使用；实际恢复执行的 request/state 来自 snapshot。没有使用 incoming_request 重写它们，也没有重新 lower stored_request。

`ResumeFromMidRun` 要求 snapshot.state.pending_tool_calls 为空。挂起标记非空会返回 `CrashedInSuspended`。没有 `--resume <run_id>`、`--force-resume`、`--resume-fill` 或“忽略 hash”选项。

### 8.4 outcome 到元数据的状态转换

| outcome | state.status | last_suspend_kind | driver 后续动作 |
| --- | --- | --- | --- |
| Done | Completed | null | 写 final，返回 |
| Error | Completed | null | 写 final，返回 |
| BudgetExhausted | Completed | null | 写 final，返回 |
| PendingTool | Suspended | PendingTool | 返回给调用方 |
| ContextLimitReached | 保持原状态，通常 Running | ContextLimitReached | 调 compressor；成功后清标记并继续 |
| Interrupted | Suspended | Interrupted | 返回给调用方 |

`step()` 每次消耗持有的底层 ctx；返回后不能直接再次 step，除非 driver 内部已完成压缩 resume。再次误调得到 NoActiveContext。`drive_to_terminal` 名称虽然含 terminal，也可以返回挂起态。

`step()` 的两类失败返回不在上表内：

| 情况 | 返回 | 目录状态 | 恢复入口 |
| --- | --- | --- | --- |
| waist 返回 `error.kind ∈ {checkpoint, tool_runtime}` | `Err(RuntimeFailure { error })` | 不写任何文件，state 仍 Running | 上下文留在内存；修复后再次 `step()`。checkpoint 情况只重做保存再推理，不重放已执行工具 |
| outcome 已算出，snapshot / final / state 任一阶段写失败 | `Err(CommitFailed { stage })` | 已完成的阶段保留 | `pending_outcome()` 读取结果；`retry_commit()` 只补写剩余阶段，不重新 `run()`；期间 `step()` 返回 `CommitPending` |

压缩失败、目录 IO 失败不自动写 Error outcome，不自动将 run 标为 Completed。

### 8.5 已定义接口与实际支持边界

1. **PendingTool**：LocalLLMContext 有落盘分支，但底层传统循环尚未生成该 outcome。工具返回 Pending 时，allow_deferred=false 报内部错误；即使 true，也报 `deferred tool path not yet implemented`。
2. **ContextLimitReached**：类型、driver 分支和 Compressor 都存在，但当前循环没有检查 context 阈值，也没有把 provider 超窗错误专门转换为这一 outcome。因此 CLI 的自动压缩路径目前没有正常触发来源。
3. **Interrupted**：底层支持；本地 CLI 没有对外控制入口。即使获得了 Suspended/Interrupted 目录，resume_or_new 也不扫描 Suspended，因此不会自动续跑它。
4. **ResumeFill**：底层类型支持 `{"kind":"resume_from_mid_run"}`、`{"kind":"rewritten_history","history":[...]}`、`{"kind":"tool_results","results":[["call_id", observation], ...]}`。ToolResults 要求与 pending 的数量和顺序逐一一致。LocalLLMContext/CLI 未公开通用 fill 接口，不能把底层能力当作 CLI 已有参数。

### 8.6 append 算法与覆盖优先级

1. 不持锁地寻找最近 run；必须 Completed，且存在索引指向的有效 snapshot。
2. 读取该 run 的 request.json。
3. 新 input = snapshot.state.accumulated + 一条来自 `--append` 的 user/text 消息。
4. 继承 objective、model_policy、tool_policy、output、budget、human_policy、error_policy。
5. 应用下表 CLI 覆盖，然后初始化 AICC runtime，获取目录锁，执行 new_run。

| 配置 | append 时 CLI 行为 |
| --- | --- |
| objective | 未给 `--objective` 则继承；给了则覆盖 |
| model_policy | 未给 `--model` 则整体继承；给了则整体重建 |
| temperature / max_tokens | 只有同时给 `--model` 才应用；否则即使单独指定也被忽略 |
| fallbacks / provider_options | 给 `--model` 后分别重置为 `[]` / null |
| tool_policy | **总是整体重建**；默认 All、max_rounds=8，其它字段为默认值，不保留原自定义策略 |
| output | `--json` 覆盖为 JSON 非 strict；未给则继承，无法用 CLI 显式切回 text |
| budget / human_policy / error_policy | 继承，CLI 无对应覆盖参数 |

Completed 不限 Done：上一次 Error 或 BudgetExhausted 也允许 append。append 不读取 final.json，继承的是 snapshot 中实际存在的历史，不能假定失败时最后一份 provider response 已入历史。

新 run 重置 usage、rounds_left、started_at_ms 等运行计数；旧历史和 workspace 保留。没有 `parent_run_id` 字段或显式谱系记录。

当前 follow-up 的读取发生在 new_run 加锁之前，存在与其它进程更新目录竞争的窗口。新实现若要消除该窗口，应将“选前序、读取、创建新 run”置于同一锁内。

### 8.7 压缩策略

| 调用入口 | 策略 |
| --- | --- |
| CLI / `drive_to_terminal(KeepTailCompressor)` | 保留所有 system 消息，按原相对顺序放在前面，再保留最后 8 **条**非 system 消息；developer 属于非 system |
| 库 `drive_to_terminal(compressor)` | 使用调用方注入的策略 |
| 库 `drive_to_terminal_auto()` | 使用 LlmSummarizeCompressor，与主任务共享 llm client，模型取原请求 model_policy.preferred；为空则报错 |

auto 的 target token：优先取 budget.max_total_tokens 的 60%；否则取 AbsoluteTokens 阈值的 60%；两者均向下取整且最低 8192；没有绝对信号时为 32768。Ratio 不参与计算。小预算也会被抬到 8192，不能将这个公式理解为一定小于原预算。

KeepTail 不调用 LLM，不按 token 裁剪、不按完整 tool-use/result 对分组，也不保证压缩后一定更短。当前没有连续无效压缩次数上限。LlmSummarizeCompressor 细节见 [LLM Compress](../opendan/LLM%20Compress.md) 与 [llm_compress.rs](../../src/frame/agent_tool/src/llm_compress.rs)。

## 9. 命令行协议

### 9.1 命令形态

```text
agent_tool run_local_llm --dir <path> --model <alias> [input flags] [tuning flags] [--new] [--output <path>]
agent_tool run_local_llm --dir <path> --append <text> [--model <alias>] [tuning flags] [--output <path>]
agent_tool run_local_llm --help
```

当前可执行文件来自 `agent_tool_cli_dev` 包中的 `agent_tool` binary。分发依据 argv[1] 精确等于 `run_local_llm`；此命令不经普通 AgentTool dispatcher。

只支持下表参数，没有位置参数、短参数组合、`--flag=value` 或 `--` 终止选项语法。带值参数紧跟的一个 argv 被直接当作值，即使它以 `--` 开头。多次出现同一带值参数时最后一次覆盖，不会累积多条 system/user 消息；布尔 flag 重复仍为 true。

### 9.2 完整参数表

| 参数 | 值 | 必需/默认 | 作用 |
| --- | --- | --- | --- |
| `--dir` | path | 除 help 外必需 | context 根目录 |
| `--model` | alias | 非 append 必需 | AICC 模型别名；append 可继承 |
| `--objective` | text | 新请求默认 `run_local_llm dev test` | 设置审计目标，不进入 prompt |
| `--system` | text | 可选 | 一条 system 消息 |
| `--user` | text | 可选 | 一条 user 消息 |
| `--input-file` | path | 可选 | 从 UTF-8 JSON 文件读取 AiMessage[] |
| `--input-stdin` | 无 | false | 读取 stdin 至 EOF，作为一条 user/text |
| `--append` | text | 可选 | 从前一个 Completed run 派生新请求 |
| `--temperature` | f32 | 未设置 | 采样温度，无 0..2 范围验证 |
| `--max-tokens` | u32 | 未设置 | model_policy.max_completion_tokens |
| `--max-rounds` | u32 | 8 | 工具轮数，0 时不向模型提供可调用工具 |
| `--no-tools` | 无 | false | tool_policy.mode=none |
| `--json` | 无 | false | output=Json(schema=None, strict=false) |
| `--new` | 无 | false | 调用 new_run；已有 Running 仍报错 |
| `--output` | path | 未设置 | 将完整 outcome 写到指定文件，替代 stdout JSON |
| `-h` / `--help` | 无 | 无 | 打印帮助到 stdout，退出 0 |

u32 参数的 0 是有效输入，负数、溢出或非整数是解析错误。temperature 按 Rust f32 解析，没有有限值和业务范围的额外检查；新实现不应依赖 NaN/Infinity 等值的可用性。空字符串的 dir/model/objective/system/user/append 没有统一非空验证。

help 在解析器遇到时立即返回，忽略此前已收集的配置和之后的参数；如果更早遇到未知 flag 或无效数字则先报错，若 `--help` 被前一个带值参数消费则只是该参数的值。

### 9.3 输入组合顺序

非 append 路径始终按以下顺序组装，与 flag 的书写顺序无关：

```text
1. --system 的一条消息（如果有）
2. --input-file 数组中的全部消息，保留原顺序
3. --user 的一条消息（如果有）
4. --input-stdin 的一条消息（仅 stdin 字节内容非空）
```

stdin 是纯文本，不是 JSON；保留换行和空白，不 trim。零字节 stdin 不产生消息；全为空格或换行的 stdin 仍产生消息。`--system ""` 和 `--user ""` 也会产生空 text 消息，因此通过“至少一条消息”的检查。input-file 为 `[]` 且没有其它消息则失败。

`--append` 与 `--system`、`--user`、`--input-file`、`--input-stdin`、`--new` 互斥。该检查发生在执行阶段，退出码为 **1**，不是 2。`--objective` 和 tuning 参数可以与 append 同用。

### 9.4 输出、stderr 与退出码

| 情况 | stdout | stderr | 退出码 |
| --- | --- | --- | --- |
| help | 帮助文本 | 通常为空 | 0 |
| 参数解析失败 | 无 outcome | `error: ...` 加帮助文本 | 2 |
| 成功 Done，未指定 output | pretty outcome JSON，末尾换行 | run 信息 | 0 |
| 非 Done outcome，未指定 output | **仍先输出该 outcome JSON** | run 信息；`run_local_llm failed: non-done outcome: <kind>` | 1 |
| 指定 output 且写成功 | 不输出 outcome JSON | run 信息；`outcome written to <path>`；非 Done 时再输出失败行 | Done 为 0，其它为 1 |
| outcome 已算出但目录提交失败 | 仍输出 / 写出该 outcome JSON | `run_local_llm: outcome computed but not committed: ...` | 3 |
| 轮前 checkpoint 或工具派发基础设施失败 | 无 outcome | `run_local_llm: runtime failure, run kept resumable: ...`；run 仍为 Running，可 resume | 4 |
| 输入读取、初始化、锁、恢复、压缩或写文件失败 | 不保证有 outcome | `run_local_llm failed: ...` | 1 |

run 创建/恢复成功后，stderr 输出 `run_local_llm: dir=<path> run_id=<id>`。初始化失败可能发生在创建 run 前，因此不一定有这条日志。依赖自身也可能输出日志，stderr 不是机器结构化协议。

`--output` 直接覆盖目标，不创建父目录，不加结尾换行，不做原子 rename。归档 final.json 不受它是否指定影响；如果额外 output 写失败，run 可能已经 Completed 且 final.json 已存在，但进程仍返回 1。

读取结果的推荐条件为：**退出码 0 且 outcome.kind 为 done**。不能只判断文件存在，也不能把 Completed 当作成功。

目录/驱动层错误没有 JSON 错误码，Rust 类型及触发条件如下。除 `CommitFailed`（退出 3）和 `RuntimeFailure`（退出 4）外，CLI 将它们的 Display 文本放在 `run_local_llm failed: ` 后，退出 1；消费者不应依赖底层 OS 错误文本完全一致。

| Rust 错误类型 | 典型触发条件 |
| --- | --- |
| `Io` | 创建、读取、写入、枚举或 rename 失败 |
| `SnapshotMissing` | state 索引指向的快照文件不存在 |
| `RuntimeFailure` | waist 因轮前 checkpoint 或工具派发基础设施失败停下，上下文保留在内存 |
| `CommitFailed` | outcome 已算出，snapshot / final / state 提交失败；`retry_commit()` 补写 |
| `CommitPending` / `NoPendingCommit` | 有未提交 outcome 时调用 `step()`，或无未提交 outcome 时调用 `retry_commit()` |
| `Serialization` | request/state/snapshot/outcome 序列化或实际读取对象的反序列化失败；扫描 state 时的失败另按 §3.2 跳过 |
| `RunningRunExists` | new_run 发现已有 Running |
| `SemanticHashMismatch` | incoming_request 与选中 Running 的 state 哈希不同 |
| `CorruptedRun` | 缺少必需快照索引，或底层 resume 验证失败 |
| `CrashedInSuspended` | do_resume 发现 last_suspend_kind 非空 |
| `NoActiveContext` | 当前对象没有可执行底层 ctx 却再次 step |
| `CompressorFailed` | 默认 LLM 压缩适配器失败；自定义 compressor 也可返回其它本地错误 |
| `NoCompletedRunToAppend` | 没有前序 run，或最近 run 不是 Completed |
| `LockFailed` | 目录 flock 获取失败，或进程锁 registry 异常 |
| `ToolWiringFailed` | 工具注册失败，或自动压缩所需模型别名为空 |

### 9.5 与 AgentToolResult 的区别

`run_local_llm` 没有 `agent_tool_protocol`、`status`、`summary` 外层，也不使用通用工具的 pending 退出码。`--json` 控制模型输出契约，CLI 自身在普通执行时一直输出 JSON outcome。

新的 SDK/CLI 可以重新定义返回协议，包括是否接入统一 AgentToolResult 封装，但应在新协议中明确 SDK 结果和 CLI 输出的映射，不将这种变化描述为与旧入口兼容。当前 `bin/` 里被 exec_bash 调用的普通 AgentTool 则仍可使用 [AgentToolResult 协议](agent_tool_result_protocol.md)。

## 10. 工具与执行环境

### 10.1 注册工具

本地 ToolManager **只注册三个工具**，工具声明顺序不是协议：

| 名称 | 入参 | 核心行为 |
| --- | --- | --- |
| `write_file` | `path: string`，`content: string`，可选 `mode: string` | 写入 UTF-8 文本，创建父目录；默认覆盖 |
| `edit_file` | `path: string`，`old_string: string`，`new_string: string` | old_string 必须非空且精确匹配一次；new_string 可为空但必须不同；执行替换 |
| `exec_bash` | `command: string`，可选 `target`、`timeout_ms`、`cwd`、`env` | 本地启动 `/bin/bash -c <command>` |

没有独立注册 read/read_file/glob/grep，也不扫描 bin 并逐一注册函数；读文件、搜索文件通过 exec_bash 或 PATH 命令完成。

write_file.mode：`new`/`create` 要求目标不存在，`append` 追加原内容，`write`/`overwrite`/空字符串表示覆盖；不区分大小写，先 trim，缺省为 write。append 不自动补换行，不存在时允许创建。文件配置允许创建，没有有限的写入大小和 diff 行数上限。

文件工具读取旧文本使用 UTF-8 lossy 转换，再写回文本；不构成二进制编辑协议。文件写入不是事务写，也没有 OS 级沙箱。

### 10.2 exec_bash

| 项目 | 实际行为 |
| --- | --- |
| target | 缺省、空字符串、`local`、`localhost`、`.` 为本地，别名比较忽略大小写；其它值拒绝 |
| command | 必需非空字符串，trim 后执行 |
| cwd | 缺省 workspace；相对 cwd 基于 workspace，要求路径存在且词法上仍在 workspace 内 |
| timeout_ms | 默认 30000，上限 120000；接受正整数或可解析为 u64 的字符串；0/非法值拒绝，过大值 clamp |
| env | 继承父进程环境，再应用传入对象；键符合 `[A-Za-z_][A-Za-z0-9_]*`；值允许 string/number/bool/null，null 转空串 |
| stdin | null/EOF，不继承 CLI stdin |
| shell | `/bin/bash -c`，不是 login shell，不持久化 shell 会话 |
| PATH | 一般为 `<dir>/bin:<传入 env.PATH 或进程 PATH>` |
| 输出 | 收集 stdout/stderr 后，将 stdout + 必要的一个换行 + stderr 拼接；不是按发生时间交错合并 |
| 输出上限 | 拼接展示 output 截为 262144 字节，UTF-8 lossy 解码；detail 中原 stdout/stderr 未按此上限截断 |
| 退出 | 正常退出码；Unix 信号映射为 128+signal；超时返回工具错误 |

PATH 去重逻辑只要发现 `<dir>/bin` 已位于原 PATH 任意位置，就不再次 prepend，因此此时不保证它优先于系统命令。子 shell 的 `cd`、变量赋值不会成为下一次工具调用的持久状态；文件副作用会保留。

向模型展示的 exec_bash schema 目前只列 command、target、timeout_ms，未展示 cwd/env；schema timeout 上限还是通用的 600000，usage 中默认值也是通用的 60000。**本地真实运行限制以 30000/120000 为准。**

工具结果转发：对于通过现有简单命令检测、且 stdout 能整体解析为 AgentToolResult 的命令，exec_bash 直接转发该结构；Pending 必须带 task_id，否则回退普通 bash 结果。未引用的管道、分号、与号、重定向或换行会禁用转发。转发时内部 status 为准，必要时补非零 return_code；普通 bash 结果则以 exit_code 是否为 0 判 success/error。

这只描述结果识别，不是可靠的完整 shell AST 判定或命令权限检查。实现细节见 [llm_bash.rs](../../src/frame/agent_tool/src/llm_bash.rs)。

### 10.3 工具结果到 LLM observation

| AgentToolResult 状态 | Observation |
| --- | --- |
| Success | `kind=success`；content 为非空白 output，否则 summary；bytes 为所选文本 UTF-8 字节数；truncated=false |
| Error | `kind=error`；message 优先非空白 summary，其次 trim 后非空 output，否则 `tool error` |
| Pending | `kind=pending`；保留 call_id |
| manager 自身抛错 | `kind=error`；message 为错误文本，无 tool_result |

前三类均附上结构化 `tool_result` 视图。Success.content 不是旧注释所说的 detail JSON。传统 loop 再将 success/error 转为 tool role 的 tool_result 文本供模型读取。

session 模板：`agent_name=oneshot`、`behavior=oneshot`、`trace_id=session_id=run_id`、`wakeup_id=""`；每次工具调用增加 step_idx。恢复或重新 build_deps 会重置工具管理器的 step_idx，不是 run 内持久化序号。

### 10.4 路径限制的实际含义

文件工具使用词法路径前缀检查；并未全面解引用符号链接进行 root 限制。exec_bash 只校验初始 cwd，命令本身仍可 `cd`、访问绝对路径或通过符号链接访问其它位置。

因此 workspace 是工具的默认工作根，不是进程隔离边界。TS 重实现可以补充明确的执行隔离，但不能从当前注释推导出已经具备文件系统沙箱。

## 11. AICC 接入协议

CLI 先复用已初始化 BuckyOS API runtime；没有时调用 `init_buckyos_api_runtime("buckycli", None, AppClient)` 并设置全局 runtime。这里不是旧注释所说的 FrameService。

CLI 本身没有 `--api-key`、`--endpoint` 或专属环境变量协议，认证和服务发现沿用 BuckyOS runtime。具体环境配置不应从本地目录推断。

每次推理调用 `get_aicc_client().call_method(LLM_CHAT, request)`，method 为 `llm.chat`：

| 输入 | AICC 映射 |
| --- | --- |
| 模型 | `ModelSpec.alias = model_policy.preferred`，provider_model_hint 为空 |
| 能力 | capability=Llm |
| 历史 | payload.messages；text/input_json 为空，resources 为空 |
| 工具 | allow_tool_calls 为 true 才传 tool_specs；否则空数组；args_schema 作为 object，output_schema 为 `{}` |
| 温度/输出 token | payload.options.temperature / max_tokens |
| JSON schema | force_json 且 schema 存在时，options.response_schema |
| provider_options | 对象时覆盖合并进 options；其它 JSON 值放 options.provider_options |
| 必需特性 | 有工具时 `tool_calling`；force_json 时 `json_output` |
| 响应格式 | force_json 时 Json，否则 Text |
| disable_capabilities | 非空时写入 requirements.extra.disable_capabilities |

AICC `Succeeded` 必须带 result；`Failed` 转 `provider{failure=unknown}`；`Running` 转 `provider{failure=permanent}`，**不轮询 task**。kRPC 传输错误按变体归类：`S2sTransientError` ⇒ transient；token / 权限 / 服务无效等 ⇒ permanent；其余 ⇒ unknown。fallbacks 参数被忽略。任何 provider 错误都直接结束 run，不喂回模型，也不由 waist 重试。

上下文恢复重新连接 AICC，不恢复旧 HTTP/RPC 请求或远端生成任务；当前 adapter 没有远端取消实现。

## 12. 使用示例

示例模型别名需要在所用 AICC 环境中存在。

```bash
# 新目录：首次运行会创建 runs、workspace 和 bin
agent_tool run_local_llm \
  --dir /tmp/local-llm-demo \
  --model default-llm \
  --objective '整理本地工作目录' \
  --system '你负责检查和整理当前工作目录。' \
  --user '列出当前目录文件，并给出简短说明。'

# 崩溃恢复：重用相同 dir、objective 和最终组装后的 input
# 若目录已经没有 Running，这条相同命令会开启新 run
agent_tool run_local_llm \
  --dir /tmp/local-llm-demo \
  --model default-llm \
  --objective '整理本地工作目录' \
  --system '你负责检查和整理当前工作目录。' \
  --user '列出当前目录文件，并给出简短说明。'

# 追加对话：最新 run 必须 Completed；生成新的 run_id
agent_tool run_local_llm \
  --dir /tmp/local-llm-demo \
  --append '把上一步的结论整理成三条建议。'

# JSON 消息文件：文件内容是第 5 节所示的数组
agent_tool run_local_llm \
  --dir /tmp/local-llm-file-demo \
  --model default-llm \
  --input-file /tmp/messages.json \
  --output /tmp/local-llm-outcome.json

# stdin 作为文本消息；--json 不改变外层 outcome 格式
printf '%s\n' '返回一个 JSON 对象，其中 answer 为 42。' |
  agent_tool run_local_llm \
    --dir /tmp/local-llm-json-demo \
    --model default-llm --input-stdin --no-tools --json
```

用户脚本投放示例：

```bash
mkdir -p /tmp/local-llm-demo/bin
cat > /tmp/local-llm-demo/bin/hello-local <<'SH'
#!/bin/bash
printf '%s\n' 'hello from local bin'
SH
chmod +x /tmp/local-llm-demo/bin/hello-local

agent_tool run_local_llm \
  --dir /tmp/local-llm-demo \
  --model default-llm \
  --user '通过 exec_bash 执行 hello-local，报告输出。'
```

上例要求没有 Running run；若有，使用同一请求恢复或换一个目录。脚本可以输出普通文本，不要求 AgentToolResult JSON。

## 13. SDK 化范围与 TS 设计参考

### 13.1 可复用的能力与可重新定义的协议

可作为新 SDK 功能参考的能力包括：工作目录绑定、单次任务执行、模型和工具策略、结构化输入输出、持久化、恢复、追加对话，以及 CLI 调用。这些能力的具体接口和支持范围由新设计确定。

目录名、文件结构、状态命名、请求哈希、默认值、CLI 参数、覆盖优先级和退出码均可重新设计。Rust 的 OneShotRequest、LLMContextSnapshot 和 outcome 是理解现有功能的参考，不自动成为 TS 的公共类型。

本轮不要求 TS 使用 Rust crate，不要求 SDK 通过旧 CLI 子进程实现能力，也不默认提供旧目录或旧命令行兼容层。Rust 库及其现有调用方的迁移不列入本轮验收。

### 13.2 不应当成已实现保证的内容

| 项目 | Rust 当前事实 | TS 建议 |
| --- | --- | --- |
| 轮前恢复 | 轮前快照与索引一起提交，恢复定位到最近一次已提交的轮前快照；工具执行后到下一次提交前仍是重放窗口 | 保持同样的提交流程，明确工具幂等责任 |
| final 提交 | 顺序为 snapshot → final → state；Completed 必有 final；final 存在而 state 为 Running 的半提交由 resume_or_new 补齐 | 保持同样的顺序和半提交修复规则 |
| request hash | Rust 专用 u64，非规范化序列化 | 定义版本化、可跨语言计算的字符串摘要 |
| run_id | 时间派生后缀，无冲突检测 | 使用抗冲突 ID，并保证新建不能覆盖已有目录 |
| context limit | 保存阈值，没有实际触发分支 | 明确 token/window 来源，实现触发与压缩无进展退出 |
| Pending/resume | deferred 尚未实现，无通用 CLI fill | 需要长任务时设计完整回填入口；未支持时明确报错 |
| Interrupted/resume | CLI 无控制入口；Suspended 被自动恢复忽略 | 明确是否支持中断和显式恢复，不把 Ctrl-C 等价为保存成功 |
| 同进程锁 | registry 重入，未串行化多个 driver | 按根目录统一串行化，明确锁释放生命周期 |
| append 原子性 | 读取前序时无锁 | 在同一目录锁内读取前序并创建新 run |
| 路径/overlay | 相对路径和 PATH 去重有偏差 | 统一绝对 root；明确 overlay 优先级 |
| sandbox/approval | 词法检查、AllowAllPolicy | 如产品需要权限隔离，另行实现明确的执行策略 |
| 策略字段 | 部分字段只是持久化，未执行 | 标明支持能力；不把声明字段等同于已生效限制 |
| append tuning | tool_policy 总重建，单独温度参数可能被忽略 | 若改变覆盖逻辑，明确记录为 CLI 行为修订 |
| worklog | 未生成 | 需要审计时定义事件 schema 和写入保证，不继承虚构文件协议 |

这些建议面向新的 TS SDK，不要求同步修改 Rust，也不自动扩展为完整 Agent Runtime。新目录格式应能与旧格式明确区分；将来若需要旧目录迁移，应作为有边界的导入任务处理，不能靠猜测字段后直接续跑。

### 13.3 Rust 行为对照用例

以下用于复核现状或比较新旧设计，不是新 TS SDK 必须通过的兼容性验收，也不是已运行的 TS 测试：

| 场景 | 基线应观察到的结果 |
| --- | --- |
| 空目录运行 Done | 创建目录与 request/state/snapshot；Completed；final.kind=done；退出 0 |
| Error/BudgetExhausted | Completed 但 final.kind 非 done；先输出 outcome，再退出 1 |
| 同请求的 Running | 按 state 指定快照恢复，保留原 run_id、usage 和运行起点 |
| 仅改变 model 的 Running | 哈希仍可相同，恢复执行旧配置 |
| 改变 objective/input 的 Running | 拒绝自动恢复；不偷偷创建替代 run |
| `--new` 遇 Running | 报错，旧记录保留 |
| 只有 Completed/Suspended，普通调用 | 新建 run，不继承旧历史 |
| 最新 Completed 后 append | 新 run、新计数；继承 snapshot 历史并追加一条 user 消息 |
| 最新 run 非 Completed 后 append | 拒绝，不跳过最新 run |
| append 单独给 temperature | 原 model_policy 保持；给 model 才应用温度 |
| 输入 flags 换顺序/重复 | 消息按 system/file/user/stdin 排列；重复值取最后一个 |
| 缺必需参数/未知 flag/非法 u32 | stderr 加帮助，退出 2 |
| append 互斥/空消息数组 | 执行错误，退出 1 |
| `--json` 遇非 JSON 正文 | 非 strict 下 Done+Text，退出 0 |
| `--output` 成功 | stdout 无 outcome；文件为完整 outcome；内部 final 仍归档 |
| `--output` 写失败 | 退出 1；内部 run 可能已经 Completed |
| 另一进程占有目录锁 | 立即失败；不能以删除 .lock 文件规避 |
| state 索引之后另有快照 | 当前基线仍读取 state 索引；TS 若修正应另设修订验收 |
| bin 中有可执行脚本 | 在绝对 root、无重复 PATH 项的情况下可由 exec_bash 命中 |
| shell Pending/超时/非零退出 | 按工具及底层循环规则报告，不误报 CLI 成功 |
| 大于 JS 安全整数的旧哈希 | 读取旧格式时不能丢精度；不支持迁移时明确拒绝 |

### 13.4 SDK 化验收方向

- 程序能直接调用 SDK 执行任务，获得结构化结果和错误；不依赖 CLI 参数解析、stdout 或 process exit。
- CLI 调用同一 SDK 执行路径；同一请求的行为不因调用入口不同而分叉。
- SDK 请求、运行状态、持久化数据和 CLI 输出之间的关系有明确协议，分别说明哪些字段由调用方提供、哪些由运行时生成。
- LLM 客户端、工具执行和工作目录的接入方式适合在宿主程序中使用，不能只有 CLI main 才能初始化。
- 新设计明确承诺的恢复、压缩、锁和异步能力有相应的正常与故障场景验收；未纳入的能力明确说明，避免只声明字段却不执行。
- 旧格式兼容、Rust 库修改和现有调用方迁移不作为本轮完成条件。

具体 TS API、包组织、新目录格式和 CLI 协议应在新设计中定义；本现状文档不替代这些设计决定。
