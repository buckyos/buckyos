# llm_context / agent_tool 先行项 TODO：UI Session 提示词改造所需

日期：2026-10-17

状态：v0.3，**P1–P5 已实施（2026-10-17）**，实施记录见 §实施记录。这是 [jarvis-chat-prompt-todo.md](jarvis-chat-prompt-todo.md)（下称“主 TODO”）§9 列出的 llm_context / agent_tool 层改动，按约定单列并**先于** libopendan / xagent 侧实施（[feedback-llm-context-first]）。五项彼此独立，可并行；每项自带单测与 fixtures 更新。

修订记录：2026-10-17 v0.2 按用户答复（主 TODO §10）：P2 以 provider 原生 JSON Schema 为主路径、校验纠正只兜底；P3 的 `bash_core` 用 `read` 取代 `read_file`；新增 P5。2026-10-17 v0.3 实施完成。

依据：主 TODO §3.3、§5、§6；[OpenDAN Agent Dev Guide](../doc/sdk/OpenDAN_Agent_Dev_Guide.md) §3.3（task CLI 化目标、“不能先移除 task 工具”）、§6.2–6.3；[xllm_rust_sdk](../doc/llm_context/xllm_rust_sdk.md) §36、§112（`prepare_hosted` 追加 `capabilities / cmd_manual / runtime_protocol`）；[LLM Context 设计](<../doc/llm_context/LLM Context 设计.md>) §324、§412（`OutputSpec::Json` 的 schema 只透传不校验）；[local_llm_context_protocol](../doc/llm_context/local_llm_context_protocol.md) §151–155、§411、§451。

读者：实施本文的 code agent。路径相对 `buckyos/`。行号是 2026-10-17 的源码。

---

## P1｜hosted run 可关闭 xllm 追加的三段

现状：`agent_tool/src/xllm.rs:7036-7050` 在宿主 system 后无条件追加 `## capabilities`（`build_rules_system_text`）、`## cmd_manual`（`build_cmd_manual_system_text`，shell 启用且有 bash_tools 时）、`## runtime_protocol`（`build_runtime_protocol_for`，开场白由 `HostProtocolFlavor::Session { intro }` 给）。宿主无法单独去掉。

改为：

- `XllmDeps` 新增 `host_prompt_sections: HostPromptSections { capabilities: bool, cmd_manual: bool, runtime_protocol: bool }`，`Default` 全 `true`（xllm 自己的 run 与现有宿主行为不变）。
- `prepare_hosted` 按它决定是否追加；`PromptPlan.runtime_protocol` 仍计算并记录在 run.json（`prompt.runtime_protocol`），只是不进 `system_prompt`，便于 `xagent xllm <sid>` 排查。
- `runtime_protocol = false` 只允许 `LoopModel::FunctionCall`：behavior 循环的 XML 形状由这段定义，关掉就无法解析；传入时返回 `XllmError::Capability` 说明原因。
- 关掉 `capabilities` 后，“Builtin file tools and shell's cwd are restricted to the working directory. Shell commands are not sandboxed…” 这两句的信息由 P4 的 description 承担。

验证：`xllm.rs` 测试 `hosted_sections_can_be_disabled`（三段任意组合）；`protocol()` 相关既有测试不变；`doc/llm_context/xllm_rust_sdk.md` §112 补一句。

## P2｜hosted run 的 JSON 回复契约与 fork 子 context 的输出 schema

现状：原生路径已打通——`force_json + json_schema` 时 xllm 构造 `LlmResponseFormat::json_schema`（`xllm.rs:4918-4927`），AICC 映射为 Anthropic `output_config.format`（`aicc/src/protocol/claude_messages.rs:419-449`）与 OpenAI `response_format.json_schema`（`openai_chat_completions.rs:689-725`）；MiniMax 等适配器没有映射（`minimax_messages.rs:364`）。缺的是：xllm 侧对返回文本**不校验、不重试**（runtime_protocol 只写一句“必须是一个 JSON 值”，`xllm.rs:2649`）；`force_json / json_schema` 在 lib_opendan 的 `LlmSettings` 里恒为 false / None（`lib_opendan/src/runner/rounds.rs:201-202`）。fork 继承父 context 的全部配置（`lib_opendan/src/protocol/config.rs:616-626` 禁止换工具集 / system）。

改为：

1. **回复契约**：`.llm_context` 的 `result_format = "json"` + `json_schema` 对 hosted run 生效：
   - **主路径**：照现状把 schema 作为 `LlmResponseFormat::json_schema` 交给 AICC，由 provider 原生约束输出（Claude / OpenAI 已映射）。
   - **兜底**：AICC 对该 provider 不支持结构化输出（适配器返回 unsupported / 忽略 + warn，见 [feedback-provider-optional-param-defaults]）时，在 runtime_protocol 末尾（或 P1 关掉时作为 user 侧一句约束）写 `Your final reply must be exactly one JSON value matching: <schema>`。实施前核对 AICC 对“不支持的 response_format”的实际行为（报错还是忽略），决定 xllm 如何得知走了兜底。
   - 两条路径都做：最终 assistant 文本（无工具调用）用 `jsonschema`（或现有依赖）校验；失败时追加一条 user 纠正消息（`Your reply was not valid JSON for the required schema: <error>. Reply again with only the JSON value.`），**最多一次**；再失败把原文交给宿主并在 `RunRecord.result.validation` 记录错误，宿主决定怎么出站。
   - 纠正回合算一次 llm_request，受 `max_tool_iterations` 约束之外的独立计数（`json_repairs: 1`），记入 usage。
2. **fork 可换输出 schema**：`llm_context/src/context_derive.rs` 的 `fork_snapshot`新增 `output: Option<OutputSpec>` 覆盖项；只影响回复约束与校验，不改 system / tools / model，前缀缓存不变。`protocol/config.rs:616-626` 的禁改清单明确排除 `result_format / json_schema`。
3. 宿主侧（主 TODO §5）才做：解析 `reply / topic / artifacts`、`analyze_intent` 的结果 schema。

验证：`llm_context` 单测：合法 JSON 直接通过；非法一次纠正后通过；两次失败返回原文 + validation 错误；fork 子 context 用不同 schema 且父 schema 不受影响；`history` 渲染里纠正消息可见。fixtures（`doc/llm_context` 的 behavior / function_call 样例）按需更新。

## P3｜task CLI 桥 + 工具组 `bash_core`

现状：`group:bash` = `read_file / write_file / edit_file / shell` + `ShellMode::Auto` 下的 `wait_task / get_task_state / cancel_task`（`agent_tool/src/xllm.rs:4415-4435`，`task_tools(manager.resolver())`）。DEVG §3.3：目标是 `get_task_state <id>` / `wait_task <id> --wait-ms` 这样的 shell 命令，接同一个 task resolver；“不能先移除 task 工具而让长命令失去跟进入口”。

改为：

1. **CLI 桥**：新命令 `xllm-task state <task_id> | wait <task_id> [--wait-ms <ms>] | cancel <task_id>`（名字可议，不与查 BuckyOS task-mgr 的既有 `check_task / cancel_task` CLI 混淆）。实现方式：`shell` 工具已把本次调用 id 以 `XLLM_CALL_ID` 传给命令；同样传 `XLLM_TASK_ENDPOINT`（run 目录下的 unix socket 或文件协议），CLI 经它调用进程内 `RunningTaskResolver`（`llm_context/src/tasks.rs`），语义（等待上限、取消不支持的返回、输出尾部）与三个工具一致。等待超过工具内 30 分钟规则时按同一规则截断。
2. **工具组**：`BUILTIN_TOOL_GROUP_BASH_CORE = "bash_core"` = `read`（P5 之后的超集版）/ `write_file` / `edit_file` / `shell`，不含 `read_file`、不含 task 三件；P5 未完成前临时含 `read_file` 而非 `read`；`group:bash` 不变。`cmd_manual`（若宿主没关）在 `bash_core` 下列出 `xllm-task` 的一行用法。
3. 把桥命令放进 run 的 bin 层（与 `agent-session` 同机制：lib_opendan `runtime/bin_overlay.rs` 由宿主写入；xllm 独立运行时写自己的 `.runtime/bin`）。

验证：`agent_tool` 单测：长命令转 task 后用 CLI `state / wait / cancel` 与工具结果逐字段一致；`bash_core` 组展开只有 4 个工具且含 `read` 不含 `read_file`；DEVG §3.3 的“目标用法”示例改成实际命令名并去掉“当前尚未提供”。

## P4｜`shell` 与文件工具的 description

现状：`file_tools.rs:609 / 388 / 153` 的 description 分别是 `Read file.` / `Write file.` / `Edit file.`；`shell` 的 description（`xllm.rs` / `llm_bash.rs`）讲了 task 与等待，没讲 runtime、cwd 约束、PATH。

改为（文本是 schema 的一部分，之后不再随能力增减改动，DEVG §6.3）：

- `read_file`：`Read a file in the working directory. range selects lines (e.g. "120-200"); first_chunk returns only the beginning of a large file. Paths are relative to the working directory or absolute within it.`
- `write_file`：`Create or overwrite a file in the working directory (mode: "overwrite" default | "append"). Parent directories are created.`
- `edit_file`：`Replace one exact occurrence of old_string with new_string in a file; fails when old_string is absent or ambiguous.`
- `shell`：现文本前加 `Runs in this session's runtime ({kind}); cwd is the working directory unless cwd is given; PATH provides agent-session and the agent's tools. Commands are not sandboxed and run as the current user.`（`{kind}` 用 runtime kind，不用 id，保持跨 Session 稳定。）

验证：fixtures 中记录 tool spec 的样例重生成；`xllm.rs` 的 `protocol()` 测试不受影响。

## P5｜`read` 补成 `read_file` 的超集

现状：`agent_tool/src/read_tool.rs:4-7`：`read` 是“按 URI 方案读取”的占位，v1 只实现 `file`（显式 `file://` 或裸路径），参数 `uri / offset(行) / limit(行)`；其他方案返回 `InvalidArgs`。`read_file`（`file_tools.rs:609`）的参数是 `path / range / first_chunk`，受工作目录限制。

改为（用户决定 D3：保留 `read`，它是 `read_file` 的超集）：

1. 文件能力对齐：`read` 的 `file` 方案复用 `read_file` 的实现与工作目录策略（`FilesystemPolicy`），支持 `range`（与 `offset/limit` 互为别名，保留两种写法）与 `first_chunk` 等价能力；大文件截断、二进制判定、结果格式与 `read_file` 一致。
2. 对象方案：`cyfile:<obj_id>`（work 报告附件）→ 经 `XllmDeps` 注入的对象读取器（宿主 lib_opendan 提供 `ObjectStore` / `attachment_path` 映射）取回文本或落到临时路径再读；`session:<sid>`（读 session 报告 / worklog）可留到 cmd_skill 的 CLI 承担，本项不强求。
3. description（并入 P4）：`Read a file or an object and return LLM-readable text. uri is a path (relative to the working directory or absolute within it), file://…, or cyfile:<id>. range selects lines ("120-200"); first_chunk returns only the beginning of a large file.`
4. `read_file` 保留给 Work / 既有配置，不删；`bash_core` 用 `read`。

验证：`read_tool` 单测覆盖裸路径 / `file://` / `range` / `first_chunk` / 越界路径拒绝 / `cyfile:` 经注入读取器；与 `read_file` 对同一文件的输出逐字节一致的对照测试。

## 以后（不在本文）

- **宿主提供的 background 行随 tool result 附加**（REQ S-16、ARCH §371）：推广 `llm_context/src/tasks.rs:223-245` 的 `render_background_env`（只随请求附加、不进历史）为 `BackgroundProvider` 钩子，宿主在每次装配边界给一组 `{key, text}` 行；渲染为 `<<background_context>>` 块与用户输入里的一致。等主 TODO §4 落地、看到真实需要再做。
- `report` 的 CLI 等价物（DEVG §4）：主 TODO §5 走结构化回复后 UI 不再需要；Work 继续用工具。

## 验证总览

```
cargo test -p agent_tool
cargo test -p llm_context
cargo run -p libopendan --example fixtures -- doc/opendan/protocol/fixtures   # 若 tool spec 文本进了 fixtures
```

完成后在主 TODO §9 表格标注状态，再开始主 TODO §3.3 / §5 / §6。P5 完成前主 TODO §6 的 `bash_core` 临时用 `read_file`。

---

## 实施记录（2026-10-17）

### 落点

| 项 | 代码 | 测试 |
|---|---|---|
| P1 | `agent_tool/src/xllm.rs`：`HostPromptSections { capabilities, cmd_manual, runtime_protocol }`（`Default` 全开，`NONE`）、`XllmDeps.host_prompt_sections`；`prepare_hosted` 按它追加各段，`prompt.runtime_protocol` 照算照记；`runtime_protocol = false` + behavior 循环 → `XllmError::Capability` | `xllm::tests::hosted_sections_can_be_disabled`（8 种组合 + behavior 拒绝） |
| P2 | llm_context：`OutputSpec::Json` 加 `max_repairs: Option<u32>`；`ContextOutput::Invalid { content, error }` + `ContextOutput::text()`；`json_check.rs`（JSON Schema 子集校验 + 容忍一层代码围栏的 `parse_reply`）；`LLMContextDeps.output_validator: Option<Arc<dyn OutputValidator>>`；`LLMContextState.output_repairs`；`finish_done` 的回复契约分支（失败回复留在历史 + user 纠正消息，不计连续错误）；`ForkOptions.output`。xllm：`.llm_context` 新键 `json` / `json_schema`（文件、合并、CLI run 也认）；`prepare_hosted` 设 `config.json/json_schema`，JSON + behavior 循环报错；`hosted_request` 生成 `Json { schema, strict: true, max_repairs: Some(1) }` | `llm_context/src/tests/reply_contract.rs`（直接通过 / 一次纠正 / 两次失败返回 Invalid / 宿主校验参与 / fork 换契约且父不变）；`json_check` 单测；`xllm::tests::a_hosted_json_schema_is_a_reply_contract` |
| P3 | `agent_tool/src/task_bridge.rs`（Linux 抽象 Unix socket，其它平台 temp 目录 socket 文件；一行 JSON 请求 / 应答；`answer()` 用 `task_state_observation_via`）；`ENV_TASK_ENDPOINT = "XLLM_TASK_ENDPOINT"`；llm_context `TaskFollowup { Tools, Command(cli) }` + `RunningTaskResolver::followup()`、`render_background_env_via` / `task_state_observation_via`；`CompositeTaskResolver::set_followup`；`XllmToolManager::serve_task_bridge`（桥随 manager 生命周期）；`BUILTIN_TOOL_GROUP_BASH_CORE = "bash_core"`；`XllmDeps.task_cli`；shell 的说明 / 转 task 提示 / 环境变量按 followup；CLI `agent-session task state|wait|cancel <id> [--wait-ms]`（`lib_opendan/src/bin/xagent.rs`） | `xllm::tests::bash_core_follows_tasks_through_the_cli_bridge`（两种组展开、shell 说明、环境变量、`state` / `wait` / 未知 task、与工具文本逐字一致、manager 释放后桥关闭） |
| P4 | `file_tools.rs` 三个 description 与参数文档（schemars 由字段注释生成 description）；shell 说明加“从工作目录开始、不沙箱、当前用户权限” + `XllmDeps.shell_note` | 由 prompt review 基线覆盖 |
| P5 | `agent_tool/src/read_tool.rs` 重写并**注册为模块**（原文件是孤儿文件，从未编译）：文件目标直接交给 `ReadFileTool`（同实现、同策略、同结果）；`cyfile:` 等对象 id 经 `ObjectReader`（缺省 `NamedStoreObjectReader`，NamedStore）+ 内存 `FileBackend` 复用同一窗口逻辑，非文本对象拒绝并指向 `llm_understand_media`；`offset/limit` 是 `range` 的另一种写法；`path` 是 `uri` 的别名 | `read_tool::tests`（与 `read_file` 序列化结果逐字节一致、offset/limit 等价、越界拒绝、对象读取与二进制拒绝、目标解析） |

### 与本文不同的决定

1. **`first_chunk` 的含义**：本文 P4/P5 草案说“first_chunk returns only the beginning of a large file”，源码里它是**锚点文本**（从第一次出现处开始读）。description 按源码写。`range` 的真实语法是 `120-200` / `120:$` / `120:+20` / `-50:$`，description 也按源码写。
2. **`.llm_context` 键名**：主 TODO 写的 `result_format = "json"` 与 xllm 已有的 `result_format`（`raw` / `result.<path>`，提取路径）冲突，改为新键 `json` / `json_schema`；在 behavior 包里由 `[reply] schema` 表达（主 TODO 实施记录）。
3. **校验器**：没有现成的 `jsonschema` crate 可离线使用，写了 JSON Schema 子集（`json_check.rs`）；provider 原生结构化输出是主路径，子集校验只兜底，不比 schema 更严。另加了宿主语义校验钩子 `OutputValidator`（文件存在、`clarifications` 计数），与 schema 失败走同一次纠正。
4. **兜底提示**：无法得知 AICC 对某 provider 是否真的应用了结构化输出（MiniMax 适配器忽略），所以不在 xllm 里追加“必须输出 JSON”的提示；关掉 runtime_protocol 的宿主自己在提示词里说明回复格式（Jarvis 的 `## Replying`），纠正消息里带完整 schema。
5. **task 命令名**：用 `agent-session task …`（Session 的 PATH 上本来就有），没有新造 `xllm-task` 二进制；独立 xllm 没有这个命令，`bash_core` 在没有 `task_cli` 或桥起不来时仍带三个 task 工具（DEVG §3.3：不能先删 task 工具）。CLI 的 `wait` 缺省 25s（低于 shell 缺省 30s，避免命令自己转成 task）。
6. **桥的可达性**：抽象 socket 只在执行器所在主机 / 网络命名空间可达；remote_ssh runtime 的命令到不了，返回“endpoint not reachable”。
7. `cmd_manual` 在 `bash_core` 下列出 task 命令一行：未做（UI 关掉了 cmd_manual，命令由 cmd_skill 列出）。

### 验证

```
cargo test -p llm_context            # 234 passed
cargo test -p agent_tool --lib       # 247 passed, 5 ignored
```

### 遗留

- 宿主提供的 background 行随 tool result 附加（本文“以后”）。
- remote_ssh runtime 的 task CLI 桥（需要随 SSH 通道转发或文件协议）。

