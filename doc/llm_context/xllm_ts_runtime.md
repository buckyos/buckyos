# xllm TypeScript 运行时

- 日期：2026-11-07
- 实现：`buckyos-websdk/src/xllm/`，入口 `buckyos/xllm`（core、`createSiteRuntime`、`siteTools`、`IndexedDbRunStore`、AICC provider）、`buckyos/xllm/dev`（`ScriptedLlmClient`、`OpenAiLlmClient`）与 `buckyos/xllm/deno`（Deno 宿主，§8，不进浏览器包）
- 依据：[xllm TS 运行时 TODO](../../notepads/xllm-ts-runtime-todo.md)；Rust 版的协议决定见 [xllm Rust SDK 参考](xllm_rust_sdk.md)，本文只写 TS 版与它的关系和 TS 独有的部分
- 示范页：`buckyos-websdk/examples/xllm_browser/`（构造 runtime）、`examples/xllm_site/`（现有页面改造，§7）

## 1. 能力边界

1. **上限是页面内嵌 JS。** 浏览器里的 xllm 是页面引入的一个库：没有新的服务端接口、特权 token、浏览器扩展 API，也不支持 `on_behalf_of`。工具是页面写的 JS 函数，模型调用用页面自己的 session。SDK 不提供 DOM、存储、网络这类通用工具。
2. **上限不等于授权。** 页面 JS 持有用户 session，而 loop 由模型驱动，输入里可能有提示词注入。实际能力由场景声明收窄：页面代码写明这个场景有哪些工具；配置、模板、导入的 run 只能进一步收窄。
3. **浏览器是 Deno 的子集。** core 和 `site` runtime 只用 `fetch`、`AbortController`、`TextEncoder`、`crypto`、`setTimeout` 这类两边都有的 Web 标准 API，不读 `window`、`document`、`location`、`navigator`、`indexedDB`。同一组用例在 jest、Deno（构建产物）和真实浏览器上跑（§10）。
4. **模型生成的代码不算页面 JS。** loop 和 runtime 从不执行模型输出：没有 `eval`、`new Function`，也不动态 `import`。

## 2. 分层

| 层 | 文件（`src/xllm/`） | 对应 Rust |
| --- | --- | --- |
| L2 waist | `core/loop.ts`（function_call 循环）、`behavior_loop.ts`、`behavior_result.ts`、`suspension.ts`、`context_window.ts`、`json_check.ts`、`interrupt.ts`、`deps.ts`、`messages.ts`、`rust_text.ts`（Rust 的裁剪与 serde_json 写法） | `llm_context` 的 `context_loop.rs`、`behavior_loop.rs` 等 |
| L4 xllm | `core/config.ts`（JSON 形式的 `.llm_context`）、`site_policy.ts`、`prompt.ts`、`protocol.ts`、`prepare.ts`、`open_runtime.ts`、`run.ts`（`XllmTask` / `XllmRun`）、`action_parser.ts`、`store.ts`、`observer.ts`、`result.ts`、`xml.ts` | `agent_tool/src/xllm.rs` |
| runtime | `runtime/types.ts`（`AgentRuntime`）、`site.ts`、`site_manager.ts`、`tool_result.ts`、`hints.ts`、`task_model.ts`、`step_renderer.ts`、`compress.ts`、`site_tools.ts`、`did_object_resources.ts` | `agent_tool/src/runtime/mod.rs`、`agent_tool_result.rs`、`step_record.rs`、`llm_compress.rs` |
| 宿主 | `hosts/browser/store.ts`（IndexedDB）；`hosts/deno/`：`store.ts`（`FileRunStore`、flock）、`native_runtime.ts`、`native_tools.ts`、`identity.ts`、`provider.ts`、`index.ts` | `RunStore`、`runtime/mod.rs` 的 native、`bash` 组工具 |
| provider | `providers/aicc.ts`、`timeout.ts` | `AiccLlmClient`、`TimeoutLlmClient` |
| dev | `dev/scripted.ts`、`dev/openai.ts` | wishkit `ScriptedModel`、`OpenAiLlmClient` |

命名和 JSON 字段照抄 Rust（snake_case、serde 的 tag、哪些可选字段省略）：同一份 run.json / 快照用同一套眼睛排查，TS 写的记录 Rust 能读，反之亦然。快照 v6、RunRecord v7、协议 `xllm/2`。

## 3. 网站怎么构建自己的 runtime

### 3.1 `createSiteRuntime`

```ts
import { AiccLlmClient, MemoryRunStore, XllmRun, XllmTask, createSiteRuntime } from 'buckyos/xllm'
import { buckyos } from 'buckyos/browser'

const runtime = createSiteRuntime({
  scene: 'demo.notes',              // 场景名：runtime 身份的一部分
  origin: location.origin,          // 页面传入，runtime 自己不读 location
  hostname: location.hostname,
  env: { PAGE_TITLE: document.title },  // {{env.*}} 只能读到这里给的
  tools: {
    append_note: {
      spec: { description: 'Append text to the saved note',
              args_schema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] } },
      idempotent: true,                                  // 按 call_id 去重，恢复时重跑不会写两次
      call: ({ text }, { call_id }) => appendNoteOnce(call_id, text),
    },
    clear_note: {
      spec: { description: 'Delete the saved note', args_schema: { type: 'object', properties: {} } },
      call: () => clearNote(),
      describeInterrupted: () => (getNote() === null ? 'The note is already deleted.' : 'The note was not deleted.'),
    },
  },
})

const deps = { llm: new AiccLlmClient(buckyos.getAiccClient()), runtime, store: new MemoryRunStore(), hostPromptSections: 'NONE' }
const task = await XllmTask.prepareHosted(
  { model: 'llm.chat', max_tool_iterations: 6, tools: { enabled: true, tools: [{ name: 'append_note' }, { name: 'clear_note' }] } },
  'demo.notes', 'You help the user take notes from this page.', deps)
const run = await XllmRun.start(task, deps, { input: 'Append the selected text to my note.' })
const outcome = await run.execute()   // { status, record }
```

工具是 `{ spec, call(args, ctx), idempotent?, describeInterrupted?(args, ctx), cancellable? }`，`ctx = { run_id, call_id, signal }`：

- 返回值是结果：字符串原样，其它值序列化成 JSON，`undefined` 记为 `ok`；抛出的异常是模型看得到的失败（`Observation.Error`）。参数先按 `args_schema` 校验，不合格同样是 `Observation.Error`。
- `idempotent`（布尔值或 `(args) => boolean`）：再跑一次没有额外效果，恢复时重跑。
- `describeInterrupted`：恢复时调用可能已经开始、又不能重跑，就把它的返回值作为 `[unresolved: result unknown] …` 交给模型；没给就用通用说明。
- `signal`：中断（以及 run 的总时长到期）时触发。`cancellable: true` 表示工具收到 `signal` 后会停下并 reject，此时结果是 `[cancelled]`，效果已知；没有它的工具在中断时被放弃，结果是 `[cancelled, result unknown]`。

### 3.2 为什么 runtime 不是工具清单

只有“名字 → 函数”的话，runtime 和 MCP 的工具列表没有区别，xllm 的恢复、副作用和环境语义都用不上。`AgentRuntime` 比工具清单多四件事：

| | Rust | 网站 runtime |
| --- | --- | --- |
| 身份 | `descriptor()`，恢复时逐字段比较 | `{ kind: "site", runtime_id: "site:<scene>", host: null, target: { origin, scene }, workdir: "/", capabilities: { tools: {}, network: true, os: "browser" } }`；不放 user agent 和工具表，浏览器升级或页面加工具不影响旧 run；换场景恢复报 `RuntimeMismatch` |
| 绑定到 run | `open(ctx, tools_cfg)` 按 run 给出工具 | 工具调用拿到 `run_id` / `call_id` / `signal`；配置里的 `tools` 只能从页面给的工具里选，manager 只装选中的工具 |
| 副作用账 | 非幂等调用恢复时 `Unresolved`，文本来自 `describe_interrupted` | `idempotent`、`describeInterrupted` |
| 环境描述 | `info()` → `{{runtime.*}}` | `RuntimeInfo { kind: site, os: browser, platform: browser, shell: "", cwd: "/", hostname, tools: [] }` |

### 3.3 直接实现 `AgentRuntime`

`AgentRuntime` 公开导出：`descriptor() / config() / info() / open(ctx, toolsCfg) / describeInterrupted(run, call) / env?(name)`，`open` 返回 `{ tools: EffectiveTools, manager: RunToolManager }`。有自己派发器的网站可以直接实现它（示范页的 `CounterRuntime`；AiWorkspace 预计如此）。下面的门按 `descriptor.kind` 判，自建的 runtime 走同一道门。

### 3.4 配置只能收窄

`kind` 是 `site` 时，`prepareHosted` 对下面这些报 `XllmError` `capability`，不静默降级（`core/site_policy.ts`）：`group:*`（含 `bash` / `bash_core`）、`mcp:` 来源、`bash_tools`、`tools.shell`；`runtime.kind` 不是 `site`，或 `runtime` 里有 `id` / `workdir` / `env` / 连接块；`provider.type` 不是 `buckyos`；`provider` 里的 `base_url` / `api_key` / `api_key_env` / `session_token` / `headers` / `on_behalf_of`；页面没给的工具名。`{{env.NAME}}` 只读页面传入的 `env`，缺了是 `template` 错误。

`tools: { enabled: true }` 不列工具时，用场景的全部工具（Rust 在这里用 `bash` 组；网站没有 `bash`）。

## 4. API

- `XllmTask.prepareHosted(llmContext, origin, hostSystem, deps): HostedTask`：等价 Rust `prepare_hosted`。`llmContext` 是 `.llm_context` 的 JSON 形式（严格键，未知字段、类型错误、别名冲突报 `config { file: origin, field, reason }`）；system = 宿主文本（可用 `{{runtime.*}}`、`{{env.*}}`，不含 `current_time` / `timezone`）+ `deps.hostPromptSections` 允许的 `capabilities` / `cmd_manual` / `runtime_protocol` 段。`hostPromptSections: 'NONE'` 只允许 function_call 循环。`json: true` / `json_schema` 是回复契约（`OutputSpec::Json { strict: true, max_repairs: 1 }`）。
- `XllmRun.start(task, deps, { input })`：分配 run id（`YYYYMMDD-HHMMSS-<6hex>`）、取 run 锁、首次推理前写 run.json。
- `run.execute(): { status, record }`：驱动到本次停止点。状态映射同 Rust（xllm_rust_sdk.md §6）。`usage.llm_requests` 每个执行段只加本段的推理次数；被杀掉的执行段没写就丢了，与 Rust 相同。
- `run.interrupter().interrupt(reason)` / `.finish(reason)`；同一个 `XllmRun` 被中断后可以再 `execute()` 续跑。
- `run.injectInput(text)`：paused / interrupted 的 run 注入一条新输入再续跑，停在一批工具调用中间时报 `input`。
- `XllmRun.resume(store, runId, limits, deps)`：终态只返回 `{ kind: 'terminal', record }`（带 limits 报 `run_terminal`），否则 `{ kind: 'run', run }`。
- `MemoryRunStore`、`exportRun(store, id)` / `importRun(store, data)`：存哪里由宿主决定。

## 5. 恢复规则

同 Rust `XllmRun::resume`：终态只读；`running` 且锁被持有 → `run_busy`；`version` 不是 7、`prompt.protocol_version` 不是 `xllm/2` → `not_resumable`；`host_commit_pending`、`handover` → `not_resumable`。TS 多一条：`host` 非空（libopendan 装配的 run）→ `not_resumable`，Rust 校验环境后可以接手，TS 不接。

然后：runtime 的 descriptor 必须与 `config.runtime_descriptor` 逐字段相等，否则 `runtime_mismatch`；按 run.json 里的工具名重建工具集，名字必须全部能派发，否则 `not_resumable`；最近快照里第一个没有结果的调用交给 `describeInterrupted`，续跑时幂等工具重跑，其余得到 `Unresolved { effect_unknown: true }`。

浏览器里导入的 record 和快照不可信（可能来自服务端或被改过）：模型用页面注入的 `LlmClient`，不读 record 里的 provider 配置；工具名超出场景的报 `not_resumable`；对模型广告的是页面当前的 spec。最后一点与 Rust 的 F6（广告 run.json 保存的 spec）不同：页面代码是可信的一方，导入的 record 不是。

## 6. 错误映射（AICC）

`providers/aicc.ts`，顺序同 Rust `aicc_provider_failure` / `provider_error_from_rpc`，只读结构化字段（`RPCError.code` / `data`），不读消息文本：

| 来源 | 分类 | run |
| --- | --- | --- |
| `quota_exhausted` / `quota_budget_exceeded` / `resource_not_permitted` / `attribution_unavailable` | `Refused { code }` | failed，`last_error.kind = code` |
| `quota_unavailable` | Transient | paused |
| `context_length_exceeded`、`details.http_status == 413` | ContextLimit | 压缩后续跑（§8）；每个 run 最多 3 次，之后 failed `context_capacity` |
| `timeout`、`retriable: true` | Transient | paused |
| kRPC `http_error` 413 / 408、504 / 401、403 / 其它 | ContextLimit / Transient / `Refused { credentials }` / Unknown | 401、403 为 paused `credentials` |
| kRPC `transport` | Transient | paused |
| 响应 `status: failed` | 按其 `error` 同上，无法判断为 Unknown | |
| 响应 `status: running` | Permanent（不轮询异步任务） | failed `provider_permanent` |
| 其它（含没有 `code` 的 kRPC 失败） | Unknown | failed `provider_error` |

`krpc_client.ts` 为此把非 2xx 写成 `code: 'http_error', data: { status }`（与 Rust 同形），请求没完成写成 `code: 'transport'`；消息文本不变。

已知缺口：kRPC 服务端的 `InvalidToken` / `TokenExpired` / `NoPermission` 在线上只有 `error` 文本，没有 `code`（`buckyos-base` `RPCResponse::to_wire_value` 只给 `Detailed` 加 code），所以页面 session 过期在 TS 里是 Unknown → failed。要让它变成 `credentials` → paused，需要服务端给这几种错误加 code。

## 7. 现有页面怎么改造：基础工具（试验，M5）

`siteTools({ resources, commands })`（`runtime/site_tools.ts`）给出四个工具，规格固定，资源和命令再多也不增加每轮的 token。它是试验：T5-6 用真实模型记录轮数和误用，再由用户决定留不留；不留就删掉 `site_tools.ts`，其它不受影响。

| 工具 | 语义 | 恢复时 |
| --- | --- | --- |
| `read { uri, offset?, limit? }` | GET。`read uri="/"` 列出全部资源（含方法）、action（含 `risk`）和命令；集合的 read 就是列表，过滤写进 query（`notes?tag=x`）；文本按行分页，单页上限 64 KB，超出时给总行数和下一页的 `offset` | 重跑 |
| `write { uri, content, mode? }` | PUT，`mode` 是 `write` / `new` / `append` | `Unresolved` |
| `edit { uri, old_string, new_string }` | PATCH。资源没有自己的 `edit` 时由 read → 检查恰好出现一次 → write 组合，并把 read 返回的 `version` 作为 `ifVersion` 交给 write | `Unresolved` |
| `exec { command }` | 一条命令，不是 shell：分词同 CLI 的 `parseShellLine`，引号外的 `| ; & > < \` $` 和未知命令都报错，并说明 exec 不是 shell、列出可用命令。内置 `xcall <object> <action> [--params <json>] [key=value ...] [--key=value ...]`（解析同 Rust `parse_object_x_call_cli_command`，`--config` / `--session-id` / `--idempotency-key` / `--confirm-token` / `--trace-id` 报不支持）；页面的命令按 CLI 规则解析，`--help` 给用法 | `xcall` 的 action 为 `read_only` / `idempotent_write` 时重跑；命令 `access: read` 时重跑；其余 `Unresolved` |

页面怎么声明（`examples/xllm_site/after/main.ts`，与改造前共用同一个 `notes_api.ts`）：

```ts
const tools = siteTools({
  resources: {
    notes: { description: 'All notes; filter with ?tag=', read: ({ query }) => notesApi.list(query.get('tag')) },
    'notes/{id}': {
      description: 'One note: its body',
      read: ({ params }) => { const n = notesApi.get(params.id); return { content: n.body, version: String(n.version) } },
      write: ({ params }, { content, mode, ifVersion }) => notesApi.update(params.id, content, Number(ifVersion)),
      actions: { delete: { description: 'Delete this note', risk: 'destructive', input: { type: 'object', properties: {} },
                           call: ({ params }) => notesApi.remove(params.id) } },
    },
  },
  commands: {
    'notes-export': { summary: 'Export notes as one markdown document', access: { mode: 'fixed', level: 'read' },
                      options: [{ name: 'tag', type: 'string', description: 'Only notes with this tag' }],
                      call: ({ tag }) => notesApi.exportMarkdown(tag) },
  },
})
const runtime = createSiteRuntime({ scene: 'demo.site.notes', origin: location.origin, hostname: location.hostname, tools })
```

改造只做三件事：用 `siteTools` 把页面已有的数据层声明成资源、action 和命令；加一个 agent 面板；写操作后刷新页面状态（这个页面本来就监听数据层变化）。页面按钮和 agent 调用的是同一个 `notesApi`，人看得到 agent 改了什么，agent 下一次 `read` 读到的是人改过的内容，人随时可以中断。

**什么时候走 `xcall`，什么时候转成命令。** 能直接修改资源本身的用 `write` / `edit`，只能通过业务动作修改的是资源上的 action（`xcall`，删除也是 action，`risk: destructive`）。符合 DID Object 形状的 RPC 声明成 action；不想封装的旧 RPC 声明成命令（`notes-export`）。

**边界。** 资源模式只有字面段和 `{name}`（只匹配一段），命令名按字面匹配，未声明的都报错。URI 先规范化再匹配：只接受相对路径和页面声明过的绝对 URL（如 `obj://…`），解码后的段含 `/`、`\` 或是 `..` 时拒绝。**不要写把 uri 转发给 `fetch` 的资源，也不要写把参数原样转成 kRPC 调用的命令**（`krpc <service> <method> <json>`）：那等于把 session 交给模型，§1 第 2 条就失效了。配置只能在四个工具里选（`tools: [{ name: 'read' }]` 就是只读）；更细的收窄由页面代码决定。非只读操作要不要用户确认，由页面在实现里决定，SDK 不设审批门。

**`didObjectResources({ client, roots, maxRisk? })`**（`runtime/did_object_resources.ts`）把 BuckyOS 服务已经用 `KrpcObjectProvider` 提供的 DID Object 变成资源：读 `roots` 里每个对象的 Profile，`read` 给出与 agent-did-object-lib `render_object_document` 同格式的对象文档（含 summary 属性的值），IndexTrait 集合带 query 时调 `query` / `page`，action 成为 `xcall` 的 action，`risk` 取 `x-buckyos:risk`，高于 `maxRisk`（缺省 `read_only`）的不暴露。能到达什么由 Profile 和 `roots` 决定，不由模型给的 URI 决定。HTTP 客户端是 `DIDObjectClient`（`src/did_object_client.ts`）：`GET <endpoint>` 是 Card，`?type=profile` 是 Profile，`props/<name>`，`POST methods/<name>`（`{ method, params }` → `{ result }`），endpoint 由 `object_url.ts` 的 `objectEndpoint` 算出（`/kapi/<service>/obj/<path>`），同源时带 SSO cookie，给了 token 就用 bearer。

**刷新后续跑。** `IndexedDbRunStore`（`hosts/browser/store.ts`，库名缺省 `xllm-runs`，`records` 按 `run_id`、`snapshots` 按 `[run_id, idx]`）让 run 在页面重新加载后 `resume`；run 锁是 Web Lock `xllm-run:<run_id>`，tab 关掉锁就释放，没有 Web Locks 时假设只有一个 tab。前提是同源下没有模型生成的代码（AiWorkspace 的程序已在 sandbox iframe 里，见 MW），否则程序能读写这个库。读回来的数据照样按 §5 不可信处理。

## 8. 其它 API

- `XllmTask.prepare(llmContext, origin, input, overrides, deps)`：xllm 自己装配提示词的任务（Rust `XllmTask::prepare`），配置同样是一个 JSON 对象。`input = { user?, attachments?, stdin?, structured? }`，附件由宿主给出内容：`{ kind: 'text', label, text }` 成为 `<material name="…">`，`{ kind: 'image', source }`（URL 或 `ResourceRef`）成为 `[image N: label]` + 图片块；配置了 `file_model` 时图片先由文件模型分析，主模型收到 `<image_analysis model="…">`。`overrides` 同 Rust `TaskOverrides` 去掉 runtime / runs_dir。
- behavior 循环：`loop_model: behavior` 时用 `XllmActionParser`（与 Rust 同一套 XML 规则，向量 (b)）和 `XmlStepRenderer`（`runtime/step_renderer.ts`，渲染结果与 Rust 逐字相同，向量 (c)）；原生工具在 step 内调用，`<actions>` 里的动作结果渲染进下一步的 `<<last_step_action_results>>`；没有动作、带 `<report end="true">` 的回复即完成，`result.raw` 是整段 XML 回复（同 Rust，用 `result_format: result.report` 取报告）；只有进度报告的回复是 yield，run 停为 `paused`（`last_error.kind = yield`），`injectInput` 后续跑。
- 上下文压缩：配置了 `context_window` 时，推理前估算超过 `context_window × context_yield_ratio`（缺省 0.8），或 provider 报 ContextLimit，run 用 `LlmSummarizeCompressor`（`runtime/compress.ts`，同 Rust `compress_history`，向量 (i)）把历史压到 `min(30% 窗口, ratio/2 × 窗口, 32768)` token，提交快照后续跑；摘要模型 AICC 是 `llm.summarize`，OpenAI 是 run 的模型；摘要调用计入 `usage.llm_requests`，次数记在 run.json 的 `compactions`。压缩失败（摘要模型出错、返回空摘要）run 为 failed `context_compaction`。
- AgentToolResult 信封：页面工具可以返回 `{ agent_tool_protocol: '1', status, output?, summary, detail, task_id?, check_after?, hints?, dedup? }`。`pending` + `task_id` 时在调用内经 `XllmDeps.tasks`（`RunningTaskResolver`）等待，`min(check_after, runtime.task_wait_ms)`，到期给“仍在运行”；function_call 回复时本 run 仍有运行中的 task，run 停为 `paused`（`tasks_running`）。`hints` 的 `once` 键记入 run.json 的 `hints_once`。
- Deno 宿主（`buckyos/xllm/deno`，`dist/xllm_deno.mjs`）：在 core 上加文件、进程和锁，能接手同一台机器上 Rust `agent_tool xllm` 留下的 run，Rust 也能接手它的。
  - `prepareNativeTask(llmContext, origin, input, overrides, options)` → `{ task, deps }`，`XllmRun.start(task, deps)` 执行；`resumeNativeRun(store, runId, limits, options)` 按 run.json 里的 `config.runtime`、工具和 provider 重建 runtime 与模型客户端后走 `XllmRun.resume` 的同一道门；`nativeDeps` 给自己组装的宿主用。
  - `FileRunStore`：布局同 Rust（`<runs_dir>/<run_id>/run.json`、`snapshots/NNNN.json`、`.lock`），锁是 `Deno.FsFile.lock`，即 flock(2)，与 Rust 的 fs2 双向互斥（实测）；工作目录锁 `~/.xllm/locks/<blake3(workdir) 前 24 位>.lock`（`@noble/hashes`），与 Rust 同名。
  - `native` runtime：descriptor 与 Rust 逐字段相同（`AGENT_TOOL_HOST_ID` / `/etc/hostname` / machine-id 得出的 host、uid、同一段 `LOCAL_PROBE` 探测的 os 与命令，blake3 派生 `runtime_id`，向量 (h)）；`bash` 组的 `read_file` / `write_file` / `edit_file` / `shell`，`shell` 的执行目录 `runs/<run_id>/exec/<call_id>/{command,stdout,stderr,exit}` 同 Rust。没有任务后端，`shell` 只在调用内等，所以要求 `runtime.task_wait_ms ≥ tools.shell.timeout_ms`；缺省配置（25 s < 1800 s）的新任务报 `capability`，带 `wait_task` 等任务工具的 Rust run 接手时报 `not_resumable`。
  - provider：`openai` 直连；`buckyos` 用 `BUCKYOS_APPCLIENT_SESSION_TOKEN`（或 `provider.session_token`）、`BUCKYOS_ZONE_HOST`、`BUCKYOS_APP_ID`，没有 session 时每次推理都是 `credentials`，run 停为 paused。
- dev：`ScriptedLlmClient`（按调用序号回复，支持 `hang` / `delay_ms` / 函数步骤）、`OpenAiLlmClient`（OpenAI 兼容 Chat Completions；错误表同 Rust `openai_http_failure`）。

## 9. 共享向量

一致性不靠共享二进制，靠共享向量：Rust 生成并校验，TS 消费。生成与校验在 `agent_tool` 的 `xllm::vectors` 测试里（`cargo test -p agent_tool --lib xllm::vectors`；`XLLM_VECTORS_REGEN=1` 时重写文件，否则比对），文件只在 `buckyos/src/frame/llm_context/tests/vectors/` 一处，websdk 的测试直接读同级 buckyos 检出里的这个目录（或环境变量 `XLLM_VECTORS_DIR` 指定的目录），找不到就跳过：jest 的向量用例、Deno 的身份向量和接手测试都是这样。websdk 的 CI 不检出 buckyos，所以那里不跑这些比对；改 TS 时在两个仓库并排的环境里跑。两边以文件头的 `vectors_version` 对齐。模型看到的 JSON 由 `core/rust_text.ts` 的 `serdeJson` 按 serde_json 的写法生成（键按码点排序、数字格式同 serde），裁剪用 Rust 的空白集合，长度按 Unicode 标量计，所以逐字比较不需要 canonical 化。

每个文件是 `{ "vectors_version": 1, "cases": [ { "name": …, … } ] }`。消息文本在协议文本（a）、提示（f）、任务文本（g）、behavior 循环的 `last_request_messages`（c）和压缩（i）里逐字比较；错误类（c）（d）只比 kind 与 failure。

| 文件 | 一个 case | TS 怎么用 |
| --- | --- | --- |
| (a) `prompt_assembly.json` | `fn` 区分：`runtime_protocol { flavor, loop_model, tools: EffectiveTools, json, expect: 文本 }`；`rules { loop_model, tools, expect }`；`cmd_manual { tools, expect: 文本 \| null }`；`assemble_prompt { llm_context, origin, runtime_vars, env, loop_model, tools, json, override_system?, override_select?, expect: { mode, system_prompt, runtime_protocol, sections, template_vars, user_request? } \| { error: { kind, field? } } }`；`hosted_system { host_system, origin, runtime_vars, env, sections: HostPromptSections, flavor, loop_model, tools, json, expect: { system_prompt, runtime_protocol, template_vars } \| { error } }`；`parse_config { llm_context, origin, expect: { ok: true } \| { error: { kind, field? } } }` | 调同名纯函数，`system_prompt` 等逐字比较 |
| (b) `xml_behavior.json`（M3） | `{ actions: ResolvedTool[], text, expect: { result: LLMBehaviorResult } \| { error: 文本 } }` | `XllmActionParser` |
| (c) `loop_scripts.json` | `{ source: 来源测试名, request: LLMContextRequest, tools: ToolSpecLite[], tool_results: { <call_id>: { observation } \| { dispatch_error: { message, effect_unknown } } }, script: [ { response: AiResponse } \| { error: LLMComputeError } ], expect: { outcome: { kind, which?, error?: { kind, failure? }, output_kind? }, inferences, dispatched: [call_id], state: { tool_iterations_left, consecutive_errors, output_repairs }, run: { status, kind?, limit_reason? } } }`；behavior case 另有 `behavior: { actions, renderer: timestamps \| no_timestamps, seed?: { steps, next_step_index, inner_transcript } }`，`expect` 另有 `behavior_result`、`state.{steps（条数）, last_step, next_step_index, next_action_id}`、`last_request_messages`、`last_request_tools`（不允许工具调用时为 null）；`seed` 表示从 `from_request(request, 1)` 加这些 step、把 `inner_transcript` 接到 `accumulated` 后以 `ResumeFromMidRun` 续跑 | 用脚本化 LLM 与按 call_id 回放的 ToolManager 跑 waist，behavior case 配上 `XllmActionParser` 与 `XmlStepRenderer`；`run` 用 outcome → RunStatus 的映射 |
| (d) `provider_errors.json` | `{ input, expect: { failure } \| { ok: true }, ts: bool }`；`input` 是 `rpc_detailed { code, message, data? }`、`rpc_reason { message }`、`rpc_http_status { status }`、`rpc_local { variant }`（只有 Rust 有，`ts: false`）、`aicc_response { response }`、`openai_http { status, body }` | `rpc_*` 构造成带 `code` / `data` 的错误交给 `providerErrorFromRpc`；`aicc_response` 交给 `AiccLlmClient`；`openai_http` 交给 OpenAI 客户端 |
| (f) `hints.json` | `{ steps: [ { result: AgentToolResult, expect: { kind, text } } \| { clear_dedup: true } ], expect_once: [key] }` | 同一个 hint 账本依次处理 |
| (g) `task_text.json` | `fn: observation { call_id, task_id, state: TaskState, followup, expect: Observation }`；`fn: general_hint { followup, expect: 文本 }`；`followup` 是 `{ kind: tools }` / `{ kind: command, cli }` / `{ kind: input, cli? }` | 调同名纯函数 |
| (h) `runtime_identity.json`（M4） | `fn` 区分：`native_host_id { agent_tool_host_id, etc_hostname, hostname_env, machine_id（未设或读不到为 null）, expect: host_id 文本 }`；`native_identity { configured_id, host_id, probe: { uid, cwd, os, tools }, expect: { target, target_text（blake3 的输入）, runtime_id, descriptor } }`；`parse_probe { stdout（本地探测脚本的 NUL 分隔输出）, expect: { identity, info, path } \| { error } }`；`workdir_lock_path { lock_dir, workdir, expect: { file_name, path } }`（按给定字符串哈希，调用方先规范化） | Deno 宿主 |
| (e) `runs/`（M4） | 真实 `agent_tool xllm` 对着 `mock_openai.mjs` 跑出的 run 目录，`generate.sh` 重生成（`AGENT_TOOL_HOST_ID=xllm-fixture`）：`paused_transient/`（一次 `write_file` 后 HTTP 503，paused）、`shell_sigkill/`（`shell` 运行中被 SIGKILL，最新快照是该调用的 checkpoint；`expected.json` 是 Rust 续跑时给模型的文本）。每个目录有 `fixture.json`、`runs/<run_id>/`、`work/`；websdk 侧的 `rewrite_paths.ts` 把根目录、`target.uid`、`runtime_id` 和 mock 地址改成本机的 | `tests/deno/xllm_takeover_test.ts` |
| (i) `compress.json`（M3） | `fn` 区分：`compress_history { history, options: { target_token_budget, model_alias, summary_chunk_tokens, note?, extra_focus_prompt? }, replies: [摘要模型第 i 次的回复文本], expect: { messages, head_end, tail_start, summary, memory_candidates, report（无 elapsed_ms）, calls: [ { model_alias, max_completion_tokens, force_json, json_schema, allow_tool_calls, system, user } ] } \| { error: { kind }, calls } }`；`segment { history, start, expect: [ { start, end, kind } ] }`；`summary_message { summary, note, expect: { message, parsed } }`；`parse_summary_message { message, expect }`；`merge_keep_exact { keep, new, expect: { keep, candidates } }`。token 估算同 waist：每个文本块 ceil(字节/4)，每条消息加 4 | `runtime/compress.ts`；`messages` 与摘要提示逐字比较 |

## 10. 验证

```bash
# buckyos-websdk
pnpm test:xllm              # jest，源码：共享用例、behavior、site tools、DID Object、向量
pnpm build && pnpm test:xllm:deno   # Deno：共享用例跑构建产物；FileRunStore、native 工具、身份向量 (h)、接手 fixture (e)，有二进制时含与 Rust 的锁互斥
# pnpm types 会删掉 dist 里构建出的 JS，之后要再 pnpm build
pnpm test:xllm:browser      # 构建示范页，真实浏览器：xllm_browser ?auto=1、xllm_site after/ 验收与刷新续跑
pnpm test:xllm:conformance  # Rust 与 Deno 宿主对同一 mock：结果、身份、提示一致；Rust paused → Deno 续跑完成
                            # 需要 agent_tool 二进制（buckyos/src 里 cargo build -p agent_tool_cli_dev，或 XLLM_RUST_BIN）；没有就 skip
# buckyos/src
cargo test -p agent_tool --lib xllm::vectors          # 向量与 Rust 一致；XLLM_VECTORS_REGEN=1 重生成
```

同一组用例在 `tests/xllm_cases.ts`。向量由 `tests/xllm/vectors.test.ts` 逐条比对（读取位置见 §9）；`provider_errors.json` 里 `ts: false` 的输入只有 Rust 有（本地生成的 kRPC 错误变体），跳过；`xml_behavior.json` 的 `artifacts_must_be_strings`、`result_must_be_json` 的错误文本来自 serde_json，只比对“是错误”。能力边界检查（都应为空）：

```bash
grep -rn "window\.\|document\.\|process\.\|Deno\.\|navigator\.\|location\." src/xllm/core src/xllm/runtime src/xllm/providers
grep -rn "eval(\|new Function\|import(" src/xllm/core src/xllm/runtime src/xllm/providers
grep -l "Deno\.\|child_process" dist/xllm.mjs
```

## 11. 与 Rust SDK 文档的对照

[xllm_rust_sdk.md](xllm_rust_sdk.md) §4 / §6 的每条在 TS 里对应的用例。“用例”是 `tests/xllm_cases.ts`（jest、Deno、真实浏览器都跑），“向量”是 `tests/xllm/vectors.test.ts` 里按文件名和 case 名跑的条目。

| Rust SDK | TS |
| --- | --- |
| §4 section 拼接、系统说明 20 / 30 / 40、custom 模式 | 向量 `prompt_assembly` 的 `assemble_*`、`rules_*`、`cmd_manual_*`、`hosted_*` |
| §4 模板变量、`\{{` 转义、缺失变量报 `Template` | 向量 `assemble_template_escape_and_referenced_contexts`、`assemble_missing_runtime_variable`、`assemble_unset_env_variable` |
| §4 user：任务要求 / 组默认 / 附件 / stdin | 用例 `prepare assembles sections, the group default task, materials and stdin` |
| §4 `file_model` 先分析图片 | 用例 `images go through the file model when one is configured` |
| §4 behavior 协议、正文 / CDATA 参数、`<report end="true">` 完成、yield、`artifacts` | 向量 `xml_behavior` 全部、`loop_scripts` 的 `behavior_*`；用例 `a behavior run parses XML steps; native calls run inside the step` |
| §4 `context_window` / `context_yield_ratio`、压缩、最多 3 次 | 用例 `a context over the window is compacted and the run continues`、`a context that compaction cannot shrink fails with context_capacity`、`a failing summarizer fails the run with context_compaction`、`context_yield_ratio without context_window warns`；向量 `compress` 全部、`loop_scripts` 的 `provider_context_limit`；`compress.test.ts` 的 `compaction_lands_below_the_yield_threshold` |
| §4 `max_tool_iterations`：按批计、额度用完只许最终答案、behavior Step 计一次 | 向量 `batches_count_tool_iterations_not_rounds`、`tool_iteration_budget`、`behavior_step_exhausts_tool_iterations`；用例 `the tool iteration budget ends the run as limit_reached` |
| §4 `max_calls_per_round` | 向量 `max_calls_per_round_rejected` |
| §4 `usage.llm_requests` 含文件模型与压缩 | 用例 `completes a tool loop and records usage`、`images go through the file model…`、`a context over the window is compacted…` |
| §6 interrupted | 用例 `an interrupt during inference stops the run and execute continues it`、`an interrupt during a tool call pairs it as cancelled` |
| §6 paused：超时 / Transient / `credentials` | 向量 `provider_timeout`、`provider_transient`、`refused_credentials`、`provider_errors` 全部；用例 `AICC errors map to run states without reading message text` |
| §6 paused：checkpoint 写不进（`storage`） | 用例 `a checkpoint the store cannot write pauses the run (storage)` |
| §6 paused：工具基础设施失败 | 向量 `dispatch_error_effect_unknown`、`behavior_action_dispatch_error` |
| §6 paused：yield / `tasks_running` | 向量 `behavior_progress_report_yields`；用例 `a task still running at the reply pauses the run (tasks_running)` |
| §6 paused：宿主 Run 交接 | 不适用：TS 不执行宿主装配的 run（§12） |
| §6 failed：Permanent / Unknown / 其它 `Refused` / 解析与工具错误超限 / 压缩 3 次仍超限 | 向量 `provider_permanent`、`provider_unknown`、`refused_quota`、`consecutive_errors_over_the_cap`、`strict_json_errors_exhausted`、`reply_contract_exhausted`；用例 `a context that compaction cannot shrink…` |
| §6 limit_reached：工具迭代 / 总时长 / token | 向量 `tool_iteration_budget`、`token_budget`；用例 `the wallclock timeout cancels a running tool` |
| §6 resume：终态只读，带限制报 `RunTerminal` | 用例 `a terminal run only returns its record` |
| §6 resume：调高 `max_tool_iterations` 只加差额 | 用例 `resume limits: a higher max_tool_iterations adds only the difference` |
| §6 resume：record 不是 v7、快照不是 v6、`handover` → `NotResumable` | 用例 `old formats and host-assembled runs are not resumable` |
| §6 resume：上下文上限挂起的快照先压缩 | 用例 `a run saved at the context limit compacts first when it is resumed` |
| §6 resume：按 call_id 处理可能在执行的调用 | 用例 `export / resume re-runs idempotent calls and gives the rest describeInterrupted` |
| §6 `inject_input`；停在一批工具调用中间时不能注入 | 用例 `a paused run takes a new input and continues`、`a message cannot be injected into an unfinished tool batch` |

## 12. 与 Rust 的差异

有意的：

- **不接手宿主装配的 run。** `host` 非空报 `not_resumable`（§5），所以 Rust `run_step` 里只对宿主 run 生效的 `HostReport` / `Handover` 在 TS 里走不到。页面的 hosted 任务（`prepareHosted`）由 `XllmRun` 自己执行，record 里没有 `host`；step 渲染器按任务是否 hosted 选无时间戳形态（Rust 按 `record.host` 选，原则相同：hosted run 不论谁执行都不带时间戳）。
- **恢复时广告页面当前的 spec**，不是 run.json 保存的（§5，与 F6 相反）。
- **`SiteTool.cancellable`** 是 TS 独有的：Rust 的工具由 runtime 决定能否取消，网站的工具只有页面知道。
- **`XllmTask.prepare(llmContext, origin, input, overrides, deps)`** 接一个 JSON 配置，不读 `.llm_context` 目录、不合并多层；`overrides` 没有 runtime / runs_dir。

跟着 Rust 的（Rust 改了 TS 一起改）：

- 压缩的摘要推理不响应中断，`interrupt` 在压缩结束后才生效。
- kRPC 的 `InvalidToken` 等没有线上 code（§6）。

Deno 宿主（M4）：

- `shell` 超时或中断只能结束直接子进程（包装用的 bash）：`Deno.Command` 没有进程组，命令再起的子进程会留下；Rust 杀整组。
- `shell` 的执行目录已有 `exit` 时：Rust 把它当作已结束的 task `local:shell:<id>` 读回结果；Deno 宿主没有任务解析器，给 `Unresolved`，文本用 `describe_interrupted` 的有退出码分支（`expected.json` 两种都记了）。
- 不支持（新任务 `capability`，接手 `not_resumable`）：`bash_core`、`executor` 组、MCP、具名宿主工具、`remote_ssh` / `remote_node`、Windows（没有 bash）。
- `buckyos` provider 只从环境变量取 session（上面），没有设备密钥或 `~/.buckycli` 登录；没有 session 时 run 停为 paused，Rust 在建客户端时就失败；`on_behalf_of` 报 `capability`；shell 命令前不刷新 token。
- 凭据引用只在 `config_file` 是 JSON 时重读；指向 YAML `.llm_context` 的 Rust 引用报 `capability`。
- 接手时先建模型客户端再过恢复门：缺 API key 报 `capability`，排在 `run_busy` 等之前。
- `workdir_busy` 的 `run_id` 是 `unknown`：Rust 从锁文件读出持有者。
- `{{runtime.tools}}`：`open()` 之后 `info()` 给探测到的命令加上本 run 的工具名，排序，最多 32 个；Rust 从打开后的工具管理器读。
- 参数类型错误的文本近似 serde 的措辞；run.json 的键序不同（内容相同）。

只在边角上不同：

- 摘要提示里工具参数的键序：TS 按插入顺序，Rust 用 `HashMap`，多键参数每次运行顺序不定（向量只用单键参数）。
- 超过 2^53 的整数：JS 的 `number` 本来就存不下，`serdeJson` 只保证 2^53 以内与 serde_json 相同。

