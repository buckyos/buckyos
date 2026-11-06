# Workflow Service DV Testcases

本文档为 [src/kernel/workflow](../../src/kernel/workflow) 的 workflow service 规划 DV
环境冒烟测试。目标不是函数级单测，而是验证真实 BuckyOS 链路：

```text
DV TS Script -> AppClient login -> Gateway /kapi/workflow
-> WorkflowRpcHandler (workflow service)
-> WorkflowOrchestrator (compile -> tick -> emit events)
-> ExecutorAdapter (service::aicc.* / msg_center)
-> task_manager 镜像 / get_history 事件序列
```

服务代码已经落地（[server.rs](../../src/kernel/workflow/src/server.rs)、
[orchestrator.rs](../../src/kernel/workflow/src/orchestrator.rs)），所有 §3 RPC 都
返回真实结果，而不是 `not_implemented`。**第一阶段直接跑断言即可，不需要 pending
gate。** 当前 executor adapter 注册情况（见
[main.rs:165-175](../../src/kernel/workflow/src/main.rs)）：

| executor scheme | 状态 | 说明 |
| --- | --- | --- |
| `service::aicc.*` | ✅ 已注册 | aicc 客户端可用即接入；不可用时该步落到错误路径 |
| `service::msg_center.*` | ⚠️ 未注册 adapter | 用例里如需通知，先跳过或换成 aicc 步骤 |
| `http::*` | ❌ 未注册 | 仅有占位 trait，没有具名 HTTP adapter |
| `appservice::*` | ❌ 未注册 | 同上 |
| `/skill/*` `/agent/*` | ⚠️ 编译保留 | `ExecutorRef::SemanticPath`，`fun_id` 为 null，运行时报 unresolved |
| `/tool/<name>@<version>` | ✅ FunctionRegistry 解析 | 编译期绑定到真实 FunctionObject（`func:<hex>`）；未注册报 `unresolved_function` |
| `func::<obj_id>` | ✅ LocalThunkRunner | 进程内 Native runner 执行，回执按 `execution_id` 关联；内置 `test.echo/test.sleep/test.fail@1` 与 9 个 `media.*@1` |

凡是依赖 ❌ / ⚠️ 行的用例都明确标 `Blocked: <原因>`，不要按 ✅ 标准跑断言。

AICC 步骤的输入、输出是 [buckyos-api](../../src/kernel/buckyos-api/src/aicc_client.rs) 的 typed request / response，定义以 `fixtures/` 为准：

- `vision.*`、`image.*` 要 `exact_model`（`模型@provider`），它因 Zone 而异，fixture 写 `route:<api_type>`，DV 脚本加载时经 `route.resolve` 替换。只有 `helper.*` 接受 `logical_model`。
- provider 异步的方法（`image.upscale`、`image.bg_remove` 等）调用时返回 `running`，adapter 等 AICC 任务结束，步骤输出是任务的最终结果（`artifacts`、`value`），等待期间占着该 Run 的锁，最长 30 分钟。
- 引用路径只能走对象字段，不能下标进数组：`${a.output.captions}` 可以，`captions[0].text` 不行。

## 运行前提

- DV 环境已启动并激活：`uv run src/check.py`
- workflow service 已由 scheduler 拉起，Gateway 可访问
  `https://test.buckyos.io/kapi/workflow`（路径常量见
  [workflow_service.rs](../../src/kernel/buckyos-api/src/workflow_service.rs)）。
- AppClient 登录可用（[test/test_helpers/buckyos_client.ts](../test_helpers/buckyos_client.ts)）。
- AICC 至少有以下能力中的若干（一期主路径用到）：
  `vision.caption`、`vision.ocr`、`vision.detect`、`llm.chat`、`image.upscale`、
  `image.bg_remove`。
- task_manager 在线时 ServiceTracker 会镜像 run / step / thunk（不在线则 noop，不致命）。
- WF-DV-009 / 011 / 012 / 013 通过 NFSP（`/nfs/v1`，同文件浏览器路径）上传 `fixtures/dv-image.png`
  与 `fixtures/dv-doc.pdf`，取 `ident.obj_id` 作为系统管线的 `obj_id` 输入；需要调用者在 NFSP
  上有可写导出根（默认 `home`）。
- WF-DV-011 按宿主是否安装 poppler（`get_capabilities.environment.missing`）走不同分支，报告目录
  `branch.json` 记录实际分支；两条分支都算通过。

## API 速查

所有 method 都接受 `service.<name>` 与裸 `<name>` 两种形式。响应统一是
`{ "ok": true|false, ... }`，**不是抛异常**——脚本必须先断言 `ok == true` 再读字段。
例外：身份校验失败（无 token、`owner` 与已认证 principal 不符、越权读取他人 Run、客户端自报
`source: system`）由 kRPC 抛 `NoPermission` 错误，`rpc.call` 会 reject。
`ok:false` 的结构化错误为 `{error:<code>, message, detail:{details, retryable}}`，执行错误码见
`buckyos-api::error_codes`（tools_missing / timeout / cancelled / interrupted / permission_denied /
resource_limit / environment_changed / invalid_params / invalid_result / input_unavailable /
tool_failed / function_not_found / queue_full / internal）。

| RPC | 必填参数 | 关键返回字段 |
| --- | --- | --- |
| `submit_definition` | `definition`，可选 `owner`（须等于已认证 principal）、`tags`、`source`（`user_imported` / `app_registered` / `agent_generated`；`system` 拒绝；id 以 `media.` / `sys.` 开头拒绝） | `workflow_id`、`version`、`revision`、`source`、`analysis`、`definition` |
| `dry_run` | `definition` | `analysis`、`graph`、`revision`、`resolved_functions` |
| `get_definition` | `workflow_id`（记录 id 或系统定义逻辑 id 如 `media.pdf`） | `definition`（含 `source`、`read_only`、`revision`、`resolved_functions`、`compiled`） |
| `list_definitions` | （可选 `owner`、`status`、`tag`、`source`） | `definitions[]`（含 `source`、`read_only`、`revision`） |
| `archive_definition` | `workflow_id` | `status`；系统定义返回 `error: system_definition_read_only` |
| `list_functions` | （可选 `prefix`） | `functions[]`：`{fun_id, registration, object}` |
| `get_function` | `fun_id` 或 `name` + `version` | `function`（含 `published`） |
| `get_capabilities` | — | `execution_epoch`、`environment: {revision, tools, missing, searched}`、`functions` |
| `create_run` | `workflow_id`，可选 `owner` / `input` / `auto_start` / `ensure` / `wait_ms`(≤5000) / `callback_url` | `run_id`、`workflow_id`、`status`、`revision`、`reused`、`output`、`events`、`seq` |
| `start_run` / `tick_run` | `run_id`，可选 `wait_ms` | `accepted`、`status`、`events`、`from_seq`、`to_seq`（后台驱动，立即返回） |
| `cancel_run` | `run_id`，可选 `reason` | `status`（`aborted`）、`events` |
| `get_run_graph` | `run_id` | 旧字段 + `workflow_name`、`revision`、`requested_by`、`executor`、`owner`、`node_errors`、`node_sources`、`node_attempts`、`output`、`deadline_at`、`execution_epoch`、`created_at`、`updated_at` |
| `get_run_output` | `run_id` | `status`、`terminal`、`revision`、`output`、`node_errors`、`run_error`、`delivery: {artifacts, held, hold_error}` |
| `list_runs` | （可选 `owner`、`workflow_id`、`status`） | `runs[]`（含 `revision`、`requested_by`） |
| `submit_step_output` | `run_id`、`node_id`、`output`，可选 `actor` | `events`、`status`、`from_seq`、`to_seq` |
| `submit_human_action` | `run_id`、`node_id`、`kind`，可选 `payload` | `events`、`status`、`from_seq`、`to_seq` |
| `report_step_progress` | `run_id`、`node_id`、`progress` | `events` |
| `request_human` | `run_id`、`node_id`，可选 `prompt`、`subject` | `events` |
| `submit_amendment` | `run_id`、`patch` | `amendment` |
| `approve_amendment` / `reject_amendment` | `run_id`、`amendment_id`，可选 `reason` | `amendment`、`plan_version` |
| `get_history` | `run_id`，可选 `since_seq`、`limit` | `status`、`events[]`、`next_seq`、`current_seq` |
| `subscribe_events` | `run_id` | `channel`、`transport`、`history` |

`Owner` 序列化形态 = `{ "user_id": "...", "app_id": "..." }`（见
[state.rs](../../src/kernel/workflow/src/state.rs) 的 `Owner::from_value`）。

## 公共验证点

每个 DV 用例至少验证：

- `dry_run` 返回 `ok == true` 且 `analysis.errors` 为空（warnings 可有）。
- `submit_definition` 返回 `ok == true` 与 `workflow_id`；同 owner + 同 definition 二
  次提交，应返回**同一** `workflow_id`，`version` 推进（DefinitionStore upsert 语义）。
- `create_run` 返回 `ok == true` 与 `run_id`，`status == "created"`。
- `start_run` 后 `status` 推进到 `running` / `waiting_human` / `completed` 之一。
- `get_run_graph` 的 `graph` 与提交的 DSL 拓扑一致，`node_states` 覆盖所有 step + 控制
  节点；`human_waiting_nodes` 在等待人工时非空。
- `get_history` 的 `events[].seq` 严格单调递增；以 `since_seq=N` 续拉时不丢事件。允许出现的事件
  名（来自 [orchestrator.rs](../../src/kernel/workflow/src/orchestrator.rs)）：
  `run.created`、`run.started`、`run.completed`、`run.failed`、`run.aborted`、
  `step.waiting_dispatch`、`step.dispatched`、`step.started`、`step.completed`、
  `step.failed`、`step.retrying`、`step.skipped`、`step.waiting_human`、`step.progress`。
- `RunStatus` 枚举值只能是：`created` / `running` / `waiting_human` / `completed` /
  `failed` / `paused` / `aborted` / `budget_exhausted`（snake_case 序列化）。
- `NodeRunState` 枚举值只能是：`pending` / `ready` / `running` / `completed` /
  `failed` / `retrying` / `waiting_human` / `skipped` / `aborted` / `cancelled`。
- 所有 `service::*` 步必须落到 ExecutorRegistry 中的真实 adapter，不能在 DV 脚本里
  绕过 workflow 直接调 AICC。
- 结束后 `list_runs` 可按 `workflow_id` / `status` 查到本次 run。

## 公共输入

默认图片使用稳定公网小图，脚本可通过 `WORKFLOW_TEST_IMAGE_URL` 覆盖；如果 DV 环境不
允许出网，请改成 zone 内 Named Object Store 的 URL：

```json
{
  "image_url": "https://www.gstatic.com/webp/gallery/1.jpg",
  "mime_hint": "image/jpeg",
  "owner": { "user_id": "devtest", "app_id": "workflow-dv" }
}
```

---

## WF-DV-001: 图片理解、人工确认与增强（主路径）

### 目标

一期主路径用例。从一个 URL 指向的图片开始，先并行做内容提取，再生成可读摘要，
等待用户确认；确认通过后做图片增强和背景移除，最后写入一条 metrics。

覆盖的一期能力：

- `service::aicc.vision.caption` / `vision.ocr` / `vision.detect`
- `service::aicc.helper.llm_chat`
- `service::aicc.image.upscale` / `image.bg_remove`
- `human_confirm` 步骤
- `branch` 与 `parallel` 控制节点
- step 输出引用 `${node.output.field}`
- 通过 RPC `submit_step_output` 提交人工动作

### Workflow Definition

见 [fixtures/wf-dv-image-review-enhance.json](fixtures/wf-dv-image-review-enhance.json)。

> 注：`msg_center` adapter 当前未注册，主路径里通知步骤暂时用 `aicc.llm.chat` 充当
> "终结节点"。`msg_center::*` adapter 接入后，可把 `completed_marker` /
> `rejected_marker` 替换为 `service::msg_center.notify_user`。

### 执行步骤

1. `dry_run`，断言 `ok == true && analysis.errors.length == 0`；记下 `graph` 的
   `start_nodes` 应包含三个并行起点。
2. `submit_definition` （带 `owner`），记录 `workflow_id`。
3. `create_run`（带 `owner` 与 `workflow_id`），记录 `run_id`，`status == "created"`。
4. `start_run`，断言 `status` 进入 `running`。
5. 轮询 `get_run_graph` + `get_history`：
   - `caption_image`、`ocr_image`、`detect_objects` 是 direct adapter 步骤，在同一个
     tick 里依次执行。
   - 三步都 `completed` 后 `inspect_fork` 才完成，`summarize_review` 才进入 `running`。
6. 等到 `human_review` 出现 `step.waiting_human` 事件，`status == "waiting_human"`，
   `human_waiting_nodes` 包含 `human_review`。
7. 调 `submit_step_output`：

   ```json
   {
     "run_id": "<run_id>",
     "node_id": "human_review",
     "actor": "devtest",
     "output": {
       "decision": "approved",
       "comment": "质检摘要可接受，继续增强。",
       "final_subject": { "approved": true }
     }
   }
   ```

8. 验证 `upscale_image` 与 `remove_background` 完成（真实 provider 调用，有费用）。
9. 验证 `completed_marker` 完成，`status == "completed"`。

### 关键断言

- `inspect_fork` 在三个视觉 step 全部 `completed` 后才 `completed`。
- `human_review` 等待期间 `status == "waiting_human"`。
- approve 后只走 `approved -> enhance_fork` 分支；`rejected_marker` 的
  `node_states` 应停留在 `pending`，不出现 `step.started` 事件。
- `upscale_image.output.artifacts` 与 `remove_background.output.artifacts` 非空。
- `get_history` 的 `events[].seq` 严格单调递增；用 `since_seq = current_seq - 5`
  续拉，得到的 `events` 与全量末尾完全对齐。
- `list_runs({ workflow_id, status: "completed" })` 能查到本次 `run_id`。

## WF-DV-002: 人工拒绝分支不执行增强

### 目标

复用 WF-DV-001 的定义，验证 `human_confirm` 的 reject 路径能真正阻止有成本的图片
增强 step。

### 执行差异

在 `human_review` 等待时调用：

```json
{
  "run_id": "<run_id>",
  "node_id": "human_review",
  "actor": "devtest",
  "output": {
    "decision": "rejected",
    "comment": "图片主体不适合当前商品，不要继续消耗增强额度。",
    "final_subject": { "approved": false, "reason": "wrong_product" }
  }
}
```

### 关键断言

- run 从 `waiting_human` 恢复后进入 `rejected_marker`，最终 `status == "completed"`
  （拒绝是合法终止，不是 `failed`）。
- `node_states.upscale_image == "pending"`、`node_states.remove_background == "pending"`，
  且历史中**不出现**这两个节点的 `step.started`。
- `rejected_marker` 的 input 中能解析到 `${human_review.output.comment}`。

## WF-DV-003: 人工动作走 `submit_human_action`

### 目标

验证用户动作的唯一入口（workflow service §3.3）。不依赖 AICC。定义见 [fixtures/wf-dv-human-actions.json](fixtures/wf-dv-human-actions.json)：`draft` 产出草稿，`review` 等人，通过走 `publish`，拒绝走 `discard`。

### 关键断言

- `approve`：Run 完成，`publish` completed，`discard` pending；`human.action` 事件的 `actor` 以已认证用户开头，调用方不能指定。
- `modify {modified_subject}`：`publish` 拿到修改后的内容。
- `reject {feedback}`：走 `discard`，`publish` 保持 pending。
- 未知 `kind` → `invalid_human_action`；不可跳过的节点 `skip` → `node_not_skippable`；第二主体调用被拒。这些之后 Run 仍是 `waiting_human`。
- `abort`：Run `aborted`；之后再下动作 → `run_terminal`。
- `retry`、`skip` 成功路径和系统定义被拒由 `server/tests.rs` 单测覆盖。

## WF-DV-004: AppService Executor 资产归档（Blocked）

### 状态：Blocked: appservice adapter 未注册

[main.rs:165](../../src/kernel/workflow/src/main.rs#L165) 仅注册了
`service::aicc.*` adapter，没有为 `appservice::*` 注册任何具名 adapter。
ExecutorRegistry 找不到 adapter，对应 step 会落到通用错误路径。

### 接入条件

为 `appservice::asset_library.archive` 实现 `ExecutorAdapter` 并在 `start_workflow_service`
中 `registry.register(...)`，再启用本用例。

### 接入后的 Definition 摘要

```json
{
  "schema_version": "0.2.0",
  "id": "wf-dv-appservice-asset-archive",
  "name": "DV AppService Asset Archive",
  "trigger": { "type": "manual" },
  "steps": [
    {
      "id": "upscale",
      "name": "Upscale",
      "type": "autonomous",
      "executor": "service::aicc.image.upscale",
      "idempotent": true,
      "input": {
        "resources": [
          {
            "kind": "url",
            "url": "https://www.gstatic.com/webp/gallery/1.jpg",
            "mime_hint": "image/jpeg"
          }
        ],
        "input_json": {
          "scale": 2,
          "response_format": "object_id",
          "output": { "resource_format": "named_object" }
        },
        "model": "image.upscale.default"
      },
      "output_schema": {
        "type": "object",
        "properties": { "artifacts": { "type": "array" } },
        "required": ["artifacts"]
      }
    },
    {
      "id": "archive",
      "name": "Archive Asset",
      "type": "autonomous",
      "executor": "appservice::asset_library.archive",
      "idempotent": false,
      "input": {
        "source_artifacts": "${upscale.output.artifacts}",
        "collection": "dv-workflow",
        "labels": ["image", "upscaled", "dv"]
      },
      "output_schema": {
        "type": "object",
        "properties": {
          "asset_id": { "type": "string" },
          "stored": { "type": "boolean" }
        },
        "required": ["asset_id", "stored"]
      }
    }
  ],
  "edges": [
    { "from": "upscale", "to": "archive" },
    { "from": "archive" }
  ]
}
```

### 接入后的关键断言

- `archive` 通过 AppService registry 解析到 Zone 内 endpoint。
- `archive.output.asset_id` 非空。
- 因 `archive.idempotent == false`，重复 run 不命中 workflow cache（参见 WF-DV-007）。

## WF-DV-005: 语义链接编译保留与执行降级

### 目标

[compiler 测试](../../src/kernel/workflow/src/dsl.rs#L210-L221) 已确认 `/skill/...`
和 `/agent/...` 编译后落到 `ExecutorRef::SemanticPath`，且 `fun_id` 为 null（registry
解析未实现）。本用例验证：

1. DSL 作者可以引用 semantic path，dry_run / submit_definition 不报错。
2. compiled graph 中 semantic path 字面保留。
3. 真正执行时给出**明确**的失败/不可调度信号，而不是把 SemanticPath 当 namespace
   adapter 处理。

### Workflow Definition

见 [fixtures/wf-dv-semantic-skill-review.json](fixtures/wf-dv-semantic-skill-review.json)。

### 关键断言

- `dry_run.ok == true`、`dry_run.analysis.errors == []`。
- `get_run_graph.nodes` 里 `semantic_review` 的 `executor` 保留 `/skill/image-quality-review`。
- `caption` 正常完成；`semantic_review` 既没有 adapter 也没有 function object，tick 报
  `unresolved_semantic_executor`，Run 结束为 `failed`，错误写入 `node_errors` 与 `run_error`，不会停在 `running`。
- registry 实现完成后，本用例应升级为：换一个 resolved executor 后启新 run，断言走
  到新解析结果，旧 run 不受影响。

## WF-DV-006: 外部 Agent 回填 step output

### 目标

验证 `submit_step_output` 的 `actor` 字段透传，覆盖外部 Agent / 外部系统通过 RPC 回
填人工任务的真实场景。

### Workflow Definition

见 [fixtures/wf-dv-agent-callback.json](fixtures/wf-dv-agent-callback.json)。

### 执行步骤

1. `start_run` 后等待 `request_manual_tag` 进入 `waiting_human`。
2. 调 `submit_step_output`：

   ```json
   {
     "run_id": "<run_id>",
     "node_id": "request_manual_tag",
     "actor": "agent/dv-callback",
     "output": {
       "tags": ["sample", "product", "needs-review"],
       "source": "agent_callback"
     }
   }
   ```

### 关键断言

- `request_manual_tag` 从 `waiting_human` 变为 `completed`。
- `echo_tags` 被激活并完成。
- `get_history` 中至少有一条事件的 `actor == "agent/dv-callback"`（来自 RPC 的 actor
  透传，对照 [server.rs:700-706](../../src/kernel/workflow/src/server.rs#L700)）。
- 不带 `actor` 字段调用时，事件 actor 默认为 `agent`（不是 `human`）。
- 未实现：`submit_step_output` 目前不按 `output_schema` 校验，缺必填字段也会接受。

## WF-DV-007: 幂等缓存与重复启动

### 目标

验证 [orchestrator 缓存路径](../../src/kernel/workflow/src/orchestrator.rs#L548)：
`idempotent: true` 的 step 在第二次同输入运行时直接命中 cache，发出
`step.completed` 事件且 payload 中带 `{"source": "cache"}`，不重复调用 adapter。

### 用例设计

第一阶段使用一个最小定义：

见 [fixtures/wf-dv-idempotent-cache.json](fixtures/wf-dv-idempotent-cache.json)。Step cache 跨服务重启持久，脚本给输入加每轮不同的 `idempotency_key`，否则第一个 Run 也会命中；换图时 key 也要换，AICC 拒绝同一 key 配不同请求体（`idempotency_conflict`）。

执行：

1. 同 owner 提交两次 `submit_definition`，断言 `workflow_id` 相同（DefinitionStore
   upsert 幂等）。
2. 启 `run_a`：跑完，记录 `caption` 节点的 `step.completed` 事件 payload。
3. 启 `run_b`：使用相同 `workflow_id`、相同 owner、相同输入。
4. 把图片 URL 改成 `https://www.gstatic.com/webp/gallery/2.jpg`，提交新的 definition
   并启 `run_c`。

### 关键断言

- `run_a` 的 `caption.step.completed` 事件 payload 中 `source` 字段为 `null` 或缺失。
- `run_b` 的 `caption.step.completed` 事件 payload 中包含 `{"source": "cache"}`，
  且整次 run 的 history 不出现 `step.started`。
- `run_b` 的 `node_outputs.caption` 与 `run_a` 一致。
- `run_c` 因为输入 hash 不同，重新执行；`source` 字段同 `run_a`。
- Cache key 形态遵循 [orchestrator.rs:2328](../../src/kernel/workflow/src/orchestrator.rs#L2328)：
  `executor + normalized input` 的稳定 hash；脚本可以选择性断言两个 cache 命中行为
  之间 InMemoryObjectStore 中存在 `workflow_cache:*` 键（DV 环境若暴露 store 调试入口）。

## WF-DV-008: executor 失败、人工接管与 retry

### 目标

验证 `step.failed` 事件、`guards.retry.fallback = "human"` 的自动降级，以及 human
接管后通过 `submit_step_output` 注入"修复后的输出"让 run 恢复——这是文档中"失败后
不让整条 run 直接不可恢复地失败"的核心保证。

### Workflow Definition

见 [fixtures/wf-dv-failure-human-retry.json](fixtures/wf-dv-failure-human-retry.json)。

### 执行步骤

1. `start_run`，等待 `get_history` 中 `caption_bad` 出现 `step.failed` 与至少一次
   `step.retrying`。
2. 第二次失败后 `caption_bad` 转为 `waiting_human`（因为 `retry.fallback = "human"`），
   `status == "waiting_human"`。
3. 调 `submit_step_output`，由人工/agent 直接注入合规输出：

   ```json
   {
     "run_id": "<run_id>",
     "node_id": "caption_bad",
     "actor": "devtest",
     "output": { "text": "manually-supplied caption text" }
   }
   ```

4. `caption_bad` 转为 `completed`，`echo_caption` 被激活并完成，`status == "completed"`。

### 关键断言

- 首次失败后历史包含 `step.failed`、`step.retrying` 至少各一条；事件 payload 中能
  看到 attempt 计数。
- 重试耗尽后转为 `step.waiting_human`，并写入 `human_waiting_nodes`。
- 人工提交后历史出现 `step.completed`，且 `node_outputs.caption_bad.text == "manually-supplied caption text"`。
- run 最终 `status == "completed"`，**不是** `failed`。

---

## WF-DV-009: 系统管线与 test.echo 的 Step cache 复用

### 目标

验收 TODO §5 条目 1 / 22：同一输入、同一执行规格的第二个 Run，具备缓存资格的纯转换步骤
`node_sources == "cache"`，`capabilities`（`is_pure:false`）每次重新执行；改变有效参数后不命中。

### 用例设计

1. 提交 `fixtures/wf-dv-echo-cache.json`（两步 `/tool/test.echo@1`），连续 `create_run`
   两次（`auto_start`），第二次 `node_sources.a == node_sources.b == "cache"`，`output` 相同，
   history 中没有 `step.started`。
2. 上传 `dv-image.png`，对 `media.image` 以 `{obj_id, max_long_edge: 512}` 连续创建两个 Run：
   第二个 Run `probe` / `fit_image` 为 `cache`，`capabilities` 为 `executed`，
   `output.fit_image.model_media_obj_id` 与第一个 Run 相同。
3. `max_long_edge: 256` 的第三个 Run `fit_image` 重新执行。

### 关键断言

- 已完成的 Run 不被复用（run_id 不同），复用的是 Step cache。
- 相同产物 obj_id 说明 FileObject 身份确定（§2.3 产物身份）。

## WF-DV-010: 同范围并发 ensure

### 目标

验收 TODO §5 条目 2：相同 principal、definition_revision、规范化输入与环境 revision 的并发
`create_run(ensure=true)` 只建一个活动 Run；终态后不再占用索引。

### 用例设计

- 提交 `fixtures/wf-dv-ensure-sleep.json`（`/tool/test.sleep@1`，默认 4000ms，`idempotent:false`）。
- 并发 4 次 `create_run {ensure:true, wait_ms:0, input:{ms:4000}}`：run_id 唯一，`reused` 为
  true 的恰好 3 个。
- `input: {}`（由 `trigger.input_schema` 默认值补齐）仍命中同一 Run；`{ms:4500}` 得到另一个 Run。
- 等待完成后再次 `ensure` 得到新 run_id、`reused:false`。

## WF-DV-011: 缺工具诊断 / media.pdf 垂直

### 目标

验收 TODO §5 条目 3 / 11 / 19：宿主缺 poppler 时 `media.pdf` 以 `tools_missing` 失败且不进入
WaitingHuman；工具齐全时产出 manifest。

### 用例设计

- `get_capabilities` 读取 `environment.missing`，决定分支并写入 `branch.json`。
- 上传 `dv-doc.pdf`（2 页，答案在尾页），`create_run media.pdf {obj_id, dpi:72, limits:{max_pages:2}}`。
- `tools_missing` 分支：`status == failed`，某个 step 的 `node_errors.<step>.code == tools_missing`，
  `details.tools_missing` 含 poppler 命令，`step.failed` 事件携带结构化错误，`human_waiting_nodes` 为空。
- `tools_present` 分支：`status == completed`，`output.build_catalog.manifest_obj_id` 为字符串，
  `summary.family == pdf`、`summary.total == 2`、`summary.complete == true`、
  `text_entries >= 1`、`image_entries >= 1`；`get_run_output.delivery.artifacts` 含 manifest id。

## WF-DV-012: RPC 与 TaskMgr 可观察性

### 目标

验收 TODO §5 条目 4（Phase A 的 RPC / TaskMgr 部分）、13、14。

### 关键断言

- `list_definitions {source:"system"}` 恰为 `media.image/pdf/video/document`，均 `read_only:true`、
  带 `revision`。
- `get_definition media.pdf` 的 `resolved_functions` 每项都是 `func:<hex>`。
- `list_functions {prefix:"media."}` 返回 9 个；`get_function media.probe@1` 的 `published:true`。
- 一个 `media.image` Run 的 `get_run_graph` 含 `revision`（等于 create_run 返回值）、
  `requested_by == "<user_id>/<app_id>"`、`executor == "workflow"`、`node_sources`。
- `run_id` 就是 Run 的根任务 id（`t-…`）：task_manager `get_task {task_id: run_id}` 得到
  `workflow.run_tree/v1` 根任务；`get_task_tree` 中存在 `workflow.step/v1` 与 `workflow.thunk/v1`，thunk 任务全部
  `phase == terminal`。

## WF-DV-014: media.video 经 `/ndn/` 读取输入

### 目标

验收 Bucket 设计 §21 N17（T2d）：ffprobe / ffmpeg 不落临时文件，改读 `/ndn/<file_id>?hint={"token":...}`。

### 关键断言

- `media.video` 完成，帧和音轨都有结果；`extract_frames` 确实执行（`frame_max_edge` 每轮不同，不走缓存）。
- `zone/workflow/_/scratch` 有 `inputs/<run_id>/...` 的令牌记录，证明走的是令牌路径；全部已撤销，key 已解绑。
- run graph 和 history 里没有 `bkt_`、`hint=`。
- 相同输入再跑一次，`extract_frames` 来自 Step cache，结果相同。

## WF-DV-015: media.document 的跨 Run 缓存

### 目标

LibreOffice 每次转出的 PDF 字节不同（`CreationDate`、`/ID`、`DocChecksum`），文档只有复用第一次的
转换结果才可能命中后续步骤。

### 关键断言

- 上传 `dv-note.txt`，`create_run media.document {obj_id, format:"txt", dpi:72, limits:{max_pages:2}}` 跑两次，都完成。
- 第二次 `capabilities` 为 `executed`；`probe`、`to_pdf`、`extract_pdf_text`、`render_pdf_pages`、`build_catalog` 都是 `cache`。
- 两次 `build_catalog.manifest_obj_id` 相同。

## WF-DV-013: 系统定义只读与身份校验

### 目标

验收 TODO §5 条目 13：保留 id、`source:system`、归档系统定义、伪造 owner 均被拒；调用者
无法指定 `content_id`。

### 关键断言

- `submit_definition` id `media.x` → `error: system_definition_read_only`。
- `submit_definition {source:"system"}` → kRPC `NoPermission`（reject）。
- `archive_definition media.pdf` → `system_definition_read_only`。
- `create_run` / `list_runs` 带与已认证 principal 不同的 `owner` → `NoPermission`。
- `create_run media.image` 输入里的 `content_id` 被服务端覆盖（`node_outputs.trigger.content_id`
  不等于伪造值）。
- **Blocked 部分**：读取他人 Run 被拒需要第二个登录身份；本 harness 只有一个 principal，用例
  在验证完上述断言后标记 Blocked 并写入 `results.json`。

## WF-DV-016: 产物交付给已授权的请求者

### 目标

验收设计文档 §10.2 G13：普通用户（非管理员）的请求能读到转换结果，走 Agent 与 AICC 实际使用的读取路径
（`/ndm/v1/store/get_object`、`/ndm/proxy/v1/read/object/open`）；只知道 obj_id 仍然读不到。

### 关键断言

第二主体（默认种子用户 `bob`）：

- 第一主体的 `media.pdf` 完成后，只知道 manifest id 时 `get_object` → 403；对他人 Run 调 `get_run_output` 被拒，之后仍 403。
- 自己提交的用户定义用 `test.echo` 输出该 manifest id：`delivery.held == 0`，仍 403。
- 向自己的 `media.pdf` Run 调 `submit_step_output` → `system_definition_read_only`。
- `create_scheduled_task` 以 `media.pdf` 为目标（输入里写他人的 `content_id`）被拒：定时触发没有调用者可以校验输入授权。
- 上传相同字节并跑自己的 `media.pdf`：`build_catalog` 来自 `cache`，manifest id 相同；调 `get_run_output` 之前仍 403，之后
  `delivery.held > 1`、无 `hold_error`，manifest 与其引用的每个产物 `get_object` 和 `object/open` 都是 200。

## 建议执行顺序

1. **WF-DV-001**：主路径，先打通 submit→start→人工 RPC→并行→完成的整条流。
2. **WF-DV-002**：拒绝分支（复用 001 定义）。
3. **WF-DV-003**：人工动作 RPC。
4. **WF-DV-006**：Agent callback 的 actor 透传。
5. **WF-DV-007**：幂等缓存。
6. **WF-DV-008**：失败 / 重试 / 人工接管。
7. **WF-DV-005**：语义链接编译保留 + 执行降级。
8. **WF-DV-004**：标 Blocked，等 appservice adapter 注册后启用。
9. **WF-DV-012**：系统定义 / 函数 / TaskMgr 树可观察性（先于媒体用例，确认 4 条系统定义已播种）。
10. **WF-DV-009**：Step cache 复用（test.echo + media.image）。
11. **WF-DV-010**：并发 ensure。
12. **WF-DV-011**：缺工具诊断 / media.pdf。
13. **WF-DV-013**：只读与身份校验（含一个 Blocked 子项）。
14. **WF-DV-016**：产物交付给已授权的请求者（需要第二主体）。

## DV 脚本落地建议

后续在 `test/workflow_test/` 增加：

```text
test/workflow_test/
├── deno.json              # 复用根 test/deno.json
├── workflow_dv.ts
├── fixtures/
│   ├── wf-dv-image-review-enhance.json
│   ├── wf-dv-agent-callback.json
│   ├── wf-dv-idempotent-cache.json
│   ├── wf-dv-failure-human-retry.json
│   ├── wf-dv-semantic-skill-review.json
│   ├── wf-dv-echo-cache.json
│   ├── wf-dv-ensure-sleep.json
│   ├── dv-image.png
│   ├── dv-note.txt
│   └── dv-doc.pdf
└── testcases.md
```

接入 `test/run.py`：

```bash
uv run src/check.py
uv run test/run.py -p workflow_test
WORKFLOW_DV_CASES=WF-DV-001,WF-DV-002,WF-DV-006 \
  uv run test/run.py -p workflow_test
```

脚本约定：

- 标 `Blocked` 的用例（WF-DV-004 / WF-DV-005 的部分断言）走单独退出码（建议 2），
  避免 CI 把"暂未实现"误报为失败或通过。
- 所有 RPC 响应**先**断言 `body.ok === true` 再读字段；遇到 `ok: false` 直接打印
  `error` / `message` / `detail` 字段并退出。
- `get_history` 轮询用 `since_seq` 断点续拉，避免重复消费历史事件造成日志放大。
- `aicc` 不可用时（`main.rs` 会打 warn），把所有 `service::aicc.*` 用例标 Blocked
  而不是 fail，并提示用户先启动 aicc。

## 2026-11-05 运行记录（宿主已装 poppler / ffmpeg / LibreOffice，`start.py --all` 重建的 dev Zone）

| 用例 | 结果 | 说明 |
| --- | --- | --- |
| WF-DV-009 | PASS | test.echo 两步第二次 Run 全部 `cache`；media.image 第二次 `probe` / `fit_image` 命中，`capabilities` 重跑；改 `max_long_edge` 后重算。输入按 run tag 唯一化：缓存索引跨服务重启持久，旧值会直接命中 |
| WF-DV-010 | PASS | 4 个并发 `ensure` 只建一个 Run，其余 `reused:true`；终态后新建 |
| WF-DV-011 | PASS（tools_present 分支） | 真实 pdfinfo / pdftotext / pdftoppm 跑通 media.pdf，manifest complete=true，2 页文本 + 2 页图 |
| WF-DV-012 | PASS | 4 条系统定义只读、9 个 media 函数已发布可读回；Run root task 通过 `list_tasks{idempotency_key:"wf-run-<run_id>"}` 可见（请求者在 run 树上有 ReadMeta/ReadInput/ReadResult 的 WholeTree grant），`get_task_tree` 含 step / thunk 任务且 thunk 终态 |
| WF-DV-013 | PASS | 保留 id / `source:system` / 归档系统定义 / 伪造 owner 均被拒；`content_id` 被服务端覆盖；第二主体（make_config 种子用户 `bob`，`WORKFLOW_DV_SECOND_USER=user:password` 可覆盖，经 control-panel `auth.login`）对 devtest 的 Run 调 get_run_graph / get_run_output / get_history / cancel_run 均 NoPermission，list_runs 为空，按他人 owner 过滤被拒，拿 devtest 的 obj_id 调 create_run 被输入授权拒绝（NDM `query_object_by_id` 读类检查） |
| WF-DV-015 | PASS | `media.document` 第二次 Run 的 `probe`、`to_pdf`、`extract_pdf_text`、`render_pdf_pages`、`build_catalog` 全部 `cache`，manifest 相同 |
| WF-DV-001 / 002 / 007 | FAIL（第十一轮已修，当时的原因记录有误：007 报的是 `model` 字段） | fixture 的 `service::aicc.vision.*` 步骤用 `input_json` 字段，当前 AICC 协议拒收（`Failed to parse VisionCaptionRequest: unknown field input_json`），步骤落到 waiting_human；与 workflow 改动无关，需随 AICC 协议更新 fixture |
| WF-DV-006 | FAIL（第十一轮已修） | Run 正确进入 waiting_human，但脚本用 TaskMgr 1.x 的 `list_tasks{root_id: run_id}` / `update_task{id,data}` 写 TaskData，与 2.0 任务树（root_id 为 task id，`report_progress` 写入）不符；与 WF-DV-003 同类阻塞 |

## 2026-11-05 第十轮运行记录（`run_id` 合并、G13）

| 用例 | 结果 | 说明 |
| --- | --- | --- |
| WF-DV-009 ~ 015 | PASS | WF-DV-012 改为 `get_task {task_id: run_id}` 得到根任务 |
| WF-DV-016 | PASS | bob 的 `media.pdf` 命中缓存后登记 5 个对象（manifest + 4 个产物），`get_object` 与 `object/open` 均 200；之前的各种路径都是 403 或被拒 |

## 2026-11-05 第十一轮运行记录（AICC fixture、异步任务）

默认 14 个用例全部 PASS，真实调用 AICC（vision → Gemini，LLM → codex-proxy，upscale / bg_remove → fal）。

| 用例 | 这轮的修改 |
| --- | --- |
| WF-DV-001 / 002 | fixture 改用 typed request；人工动作走 `submit_step_output`；upscale / bg_remove 由 adapter 等到结果，`artifacts` 非空 |
| WF-DV-005 | 加入默认集。`start_run` 已是后台执行，改为等 Run 结束再断言；调度不了的步骤让 Run `failed`，之前会永远停在 `running` |
| WF-DV-006 / 008 | 走 `submit_step_output`；008 加入默认集 |
| WF-DV-007 | 输入加每轮不同的 `idempotency_key` |
| WF-DV-004 | 仍 Blocked：appservice adapter 未注册 |

## 2026-11-05 第十二轮运行记录（人工动作 RPC、删除 grants）

默认 15 个用例全部 PASS（WF-DV-001~003、005~016），真实调用 AICC。WF-DV-003 改测 `submit_human_action`，加入默认集。WF-DV-004 仍 Blocked：appservice adapter 未注册。

运行方式：`deno run --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors workflow_dv.ts`（`deno.json` 把 `buckyos` 映射到本地 websdk 构建；`uv run test/run.py -p workflow_test` 需要 `deno` 在 PATH 且会用 npm 发布版 SDK 覆盖 node_modules）。

实测得到的服务端修正：输入授权必须用 NDM 的读类调用（`query_object_by_id`），`is_object_stored` 是任何上传者都允许的存在性探测，不能当作读权限；workflow 写入 Named Store 的对象必须 pin（nfs-server 维护周期会回收未 pin 对象），产物 FileObject 递归 pin 30 天；dev Zone 升级后 `services/workflow/spec` 没有 buckets 声明时 `bucket.put` 报 `bucket_not_found`，需重新生成系统配置（`start.py --all`）。
