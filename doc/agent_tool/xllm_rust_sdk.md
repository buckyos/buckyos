# xllm Rust SDK 参考

- 日期：2026-09-18
- 实现：`src/frame/agent_tool/src/local_llm_context.rs`（SDK）、`src/frame/agent_tool/src/run_local_llm.rs`（CLI，`agent_tool xllm ...`）
- 依据：[xllm PRD](../../product/xllm/PRD.md)。本文只记录 Rust 实现落实 PRD 时固定下来的协议决定，供 websdk 的 TS 版本对照；产品行为以 PRD 为准。

## 1. 分层与入口

| 层 | 类型 / 函数 | 职责 |
| --- | --- | --- |
| 配置 | `discover_config_files` / `load_config_layers` / `merge_config_layers` → `MergedConfig` | 从工作目录向上查找 `.llm_context`，按“祖先 → 子目录”合并；每个字段记录来源文件 |
| 准备 | `XllmTask::prepare(workdir, TaskInput, TaskOverrides, &XllmDeps) → PreparedTask` | 选组、展开工具（含 MCP 发现）、读取附件、渲染模板、组装提示词、能力校验；**不建立 Run** |
| 执行 | `XllmRun::start(prepared, deps)` → `execute()` → `RunOutcome` | 分配 runid、加锁、首次模型请求前写记录、驱动 waist `LLMContext` 到本次停止点 |
| 恢复 | `XllmRun::resume(store, run_id?, workdir?, ResumeLimits, deps) → ResumeStart::{Terminal, Run}` | 终态只返回记录；非终态重建工具与 Provider，装回最新快照 |
| 查询 | `list_runs` / `latest_run` / `load_run` / `export_result` / `build_result_view` | 只读记录，不触发执行 |

`XllmDeps` 注入：`LlmClientFactory`（默认 buckyos→AICC、openai→Chat Completions）、具名工具（`tools: - name: x`）、`RunObserver`（进度事件）、锁目录。

## 2. `.llm_context` 解析要点

- YAML 手工转换，未知字段、类型错误、别名/行号冲突都报 `XllmError::Config { file, field, reason }`。
- `runs_dir: none`（或 null）表示不落盘（内存 `RunStore`）。相对路径相对声明它的文件；`~` 展开为主目录。
- `prompt.groups.<name>` 为字符串时是外部组文件（根节点即组对象），加载时展开并记录来源。
- section 键：数字行号或固定别名（role=10、contexts/env=20、rules=30、cmd_manual=40、output_format=100）；`text: ""` 显式清空；`name` 与固定别名冲突报错。
- 合并规则：标量覆盖；`tools` 对象按字段合并，`tools`/`actions`/`bash_tools` 列表整体替换；组按名合并、section 按行号合并；`mode: custom` 与同层 `select`/`sections` 冲突报错。
- 凭据只保存引用（`SecretRef::ConfigFile{path, field}` / `Env{name}`），resume 时重新解析。

## 3. 有效配置优先级

`默认值 → 合并后的文件配置 → 选中组 → prompt.tools → CLI（TaskOverrides）`。工具开关：`tools.enabled` 默认 false；`--tools/--no-tools` 最高。仅启用未配置列表时使用 `bash` 组（`read_file`、`write_file`、`edit_file`、`exec`）。function_call 下配置 `actions` 或 `tools2actions` 报能力错误；behavior + `tools2actions` 把 tools 转为 actions，原生列表置空。

## 4. 提示词组装

system = 按行号升序的非空 section（`## <name>` 标题 + 用户文本 + 系统说明）+ `## runtime_protocol`。系统说明：20 补齐未被模板引用的时间/时区/OS/工作目录；30 列出实际可用 tools/actions（或声明没有工具）；40 只在 exec 启用时输出命令手册（含 `bash_tools`），exec 未启用时整段省略。custom 模式 = 用户整段 + `## capabilities` + `## runtime_protocol`。

模板：`{{runtime.current_time|timezone|os|cwd}}`、`{{env.NAME}}`；`\{{` 转义；缺失变量在模型请求前报 `XllmError::Template`。渲染结果与变量随 Run 保存。

user = 任务要求（显式 > 组默认 > stdin）→ 附件（命令顺序；文本用 `<material name="...">`，图片用 `[image N: label]` + 图片块）→ `<stdin>` 材料。配置 `file_model` 且有图片时先由文件模型分析，主模型收到 `<image_analysis model="...">` 而不再收到图片。

behavior 协议（`XllmActionParser`）：`<response><thinking/><actions>…</actions><report/></response>`；动作标签集由本次生效 actions 决定，属性→参数，正文→`command`/`content` 等正文参数或 JSON；**没有动作且带 `<report>` 的回复即为最终答案**。

正文/CDATA 直接承载参数值，不加字段名或 `字段名:` 前缀。例如读取文件使用 `<read_file><![CDATA[fixture.txt]]></read_file>`，也可使用 `<read_file path="fixture.txt"/>`。同一个正文参数已通过属性给出时保留属性值，空 CDATA 不会覆盖它；未通过属性给出的正文参数仍支持空字符串（例如写入空文件）和原始空白。

`max_rounds` 是原生 tools 与 behavior actions 共用的工具轮数预算：每个实际派发的 action 批次消耗一轮，同一步多个 action 只计一轮，工具业务失败也计入；behavior 各步内的原生工具循环沿用剩余额度。额度耗尽后仍允许模型返回无工具的最终答案，再请求工具或 action 则进入 `limit_reached`，不会执行超额调用。

## 5. Run 记录

```
<runs_dir>/<run_id>/run.json        RunRecord
<runs_dir>/<run_id>/snapshots/NNNN.json   waist LLMContextSnapshot（轮前 + outcome 边界）
<runs_dir>/<run_id>/.lock           该 Run 的执行互斥
<lock_dir>/<hash(workdir)>.lock     启用工具的任务按工作目录互斥（默认 ~/.xllm/locks）
```

`run_id` = `YYYYMMDD-HHMMSS-<6hex>`。`RunRecord` 关键字段：`status`、`workdir`、`input`（请求、来源、附件摘要与 sha256、stdin 角色）、`config`（Provider/模型/loop/限制/result_format/工具展开结果/配置文件与字段来源）、`prompt`（section 渲染结果、runtime_protocol 与版本、最终 system、模板变量）、`file_model_stage`、`pending_input`（首次快照前保留的输入，之后清空）、`latest_snapshot_idx`、`last_error`（phase/kind/message/recoverable/condition）、`result`（raw 原文、按 result_format 的提取值、`--json` 校验结果）、`artifacts`、`usage`（主模型/文件模型分别记录）、`limit_reason`、`interrupt_reason`。

## 6. 状态机与错误分类

| 状态 | 终态 | 触发 |
| --- | --- | --- |
| running | 否 | 执行中；记录为 running 但无进程持锁时，查询显示为“已中断（进程退出）” |
| interrupted | 否 | `XllmInterrupter::interrupt`（Ctrl-C）或进程退出 |
| paused | 否 | Provider 超时 / Transient / 疑似凭据问题（Permanent 且信息含 token、401、expired 等）/ 存储 checkpoint 失败 / 工具基础设施失败 |
| completed | 是 | waist `Done` |
| failed | 是 | Provider Permanent（非凭据）/ Unknown、输出解析或工具错误连续超限、上下文压缩 3 次仍超限、deferred tool |
| limit_reached | 是 | 工具轮数 / 总时长（`timeout`，映射为 waist wallclock 预算）/ token 预算 |

resume：终态只返回记录（附带限制参数则报 `RunTerminal`）；非终态重置 wallclock 起点，轮数额度沿用已消耗值（显式调高只增加差额）。

## 7. CLI（`agent_tool xllm`）

参数面与 PRD §6 一致。stdin 只在是管道（FIFO）或文件重定向时自动读到 EOF；终端、socket、`/dev/null` 视为无管道输入；管道为空报错且不建立 Run。

退出码：0 完成/查询成功；1 终态失败或查询目标不存在；2 参数/配置/输入预检错误（无 Run）；3 可恢复暂停；4 用户中断；5 `--output` 写入失败；6 结果提取或 `--json` 校验失败（原文已保存）。

`--format json` 输出 `XllmResult`（runid、状态、是否终态/可恢复、answer、artifacts、usage、error、Provider/模型/实际返回模型、resume 命令）。

## 8. buckyos Provider 的登录方式

`ensure_buckyos_runtime` 按顺序选择身份（`BUCKYOS_APP_ID` 可覆盖默认的 `buckycli`）：

1. 设置了 `BUCKYOS_APPCLIENT_SESSION_TOKEN`：AppClient，直接使用该会话（OpenDAN 给工具注入的方式）。
2. 在 OOD 本机且能读到设备密钥（`/opt/buckyos/security/<device>/authentication.private.pem`）：以 KernelService 语义初始化（服务地址走 127.0.0.1），用设备密钥签 `sub = iss = 设备名` 的登录断言，经 node gateway（默认 3180 端口）上的 verify-hub 换取正式会话后登录。`buckycli` 在 RBAC 中属于 kernel 角色，system-config 与 AICC 均接受。DV Test 环境下 root 直接运行即可，不需要 dev 目录或额外环境变量。
3. 否则：AppClient，用 `$BUCKYOS_DEV_HOME` / `~/.buckycli` 下的用户私钥签断言，经 verify-hub 换会话；该路径要求所选 app 已安装在 zone 中（否则 verify-hub 返回 `AppAccessDenied`）。

## 9. 验证

- `cargo test -p agent_tool --lib local_llm_context`（SDK，ScriptedLlm 驱动，覆盖配置合并、section 组装、工具优先级、behavior/function_call 循环、暂停与恢复、锁、文件模型阶段等）。
- `cargo test -p agent_tool --lib run_local_llm`（CLI 参数规则）。
- 真实环境：在 DV Test 的 OOD 上以 root 运行 `agent_tool xllm "问题"`，即走上面第 2 种登录方式。
- 端到端（无 BuckyOS）：用一个 OpenAI 兼容的 mock HTTP 服务（返回 `choices[0].message` 与可选 `tool_calls`），在 `.llm_context` 里配置 `provider.type: openai`、`base_url`、`api_key_env`，即可跑通新任务、管道串联、工具循环、`--json`、暂停/resume、`--output`、list/status/result。
