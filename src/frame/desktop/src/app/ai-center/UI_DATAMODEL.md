# AI Center UI DataModel

## Overview

WP-17A uses `/kapi/aicc` as its only backend. Provider setup is driven by `provider.catalog` and `protocol_adapter.list`; Provider instances, models, routing, usage, and traces use their corresponding AICC management methods. AI Center does not read or write system-config or control-panel AICC helpers.

## Provider setup

```ts
interface ProviderSetupCatalog {
  catalog_revision: number
  providers: KnownProviderProfile[]
  protocol_families: ProtocolFamilyOption[]
}

interface WizardDraft {
  provider_instance_name?: string
  provider_profile_id: string | null
  display_name: string
  base_url: string
  protocol_family_id: string | null
  protocol_adapter_id?: string
  region?: string
  workspace?: string
  account?: string
  auth_mode: 'api_key' | 'dynamic_login'
  api_key: string
  auto_sync_models: boolean
}
```

`provider_profile_id`, `protocol_adapter_id`, and `base_url` are frozen contract fields. `ui_hints` is extensible. `display_name` is UI-only. For built-in profiles the adapter and optional/required region, workspace, and account fields are selected by the catalog; custom providers expose only `protocol_family_id`, and the resolved adapter is read back from validation.

`provider_profile_id` is an open catalog identity. The UI preserves IDs introduced after the desktop build and does not maintain a Provider allowlist. Only Protocol Adapter implementations remain client-version capabilities.

Catalog loading has distinct loading, error/retry, empty, and success states. One Wizard open issues one catalog request and one adapter-registry request; it does not perform per-profile reads.

## Provider instances and conflicts

`provider.list` supplies every enabled and disabled instance plus `settings_revision`. Provider updates and routing updates use this revision for CAS. `settings_revision_conflict` causes the store to reload the latest snapshot before the UI asks the user to retry. Credentials are write-only and represented in the UI only by `credential_configured` and `auth_mode`.

## Usage finance

```ts
interface Money {
  amount: number
  currency: string
}

interface UsageSummary {
  total_tokens: number
  total_requests: number
  finance_totals: Money[]
  finance_complete: boolean
}
```

Currencies are never converted or summed together. Codes are normalized to uppercase, duplicate currency rows are merged, and totals are ordered by descending numeric amount. A single currency is shown directly. Multiple currencies initially show the largest amount and expose a click target that expands or collapses the full list. Incomplete finance aggregation is preserved and labeled as partial.

The transform is O(n + c log c), where `n` is the returned row count and `c` is the number of currencies; memory is O(c). It has no additional RPC reads. Raw usage events retain their own finance snapshot currency.

## KRPC mapping

| UI field | Method | Backend field |
| --- | --- | --- |
| Known provider profiles | `provider.catalog` | `providers[]` |
| Custom protocol families | `protocol_adapter.list` | unique `adapters[].protocol_family_id` |
| Provider instances | `provider.list` | `providers[]` |
| Provider revision | `provider.list` | `settings_revision` |
| Provider credential/status update | `provider.update` | typed update request |
| Routing weights | `routing.get` / `routing.update` | `provider_weights` |
| Inventory | `models.list` | `models[]` |
| Model catalog | `models.list` | `catalog.vendors[]` |
| Usage totals | `usage.query` | `total.finance_totals[]` |
| Route traces | `trace.query` | `traces[]` |

## Model catalog

模型页以 `datamodel/model-catalog.ts` 的 `ModelCatalog` 为读取契约，`ModelCardView` 为派生视图，替代旧 LocalModel 占位页。厂商身份为 Model Driver ID，模型身份为 `(vendorId, id)`，不使用渠道模型名或 exact model 字符串解析身份。已知厂商展示友好名称，新增厂商回退展示 ID。

- `CatalogModel.metadata`：defaults 合并后的全部 Model Driver 语义，详情完整保留；`local_deployable === true` 才进入可本地部署筛选。
- `CatalogModel.providers`：当前启用 Provider 的库存关联，按 instance 去重，保留本地标记及所有 exact model/variant。`available` 为非空，`local` 为至少一项 `local: true`，不等同于健康或额度判断。
- `CatalogSpec.members`：已知模型名、规格 effort selector、家族成员权重和 `active`。未物化成员展示默认权重，空规格默认保留。规格以厂商 header 中的紧凑卡片展示，尽量同行排列，窄屏自动换行；点击后在下方展开成员，同一厂商一次展开一个规格，再次点击收起。成员名称可打开同一详情。
- `ModelFilters`：由 `modelFiltersSchema`（Zod）定义；`query` 默认空字符串，无长度限制，仅做本地匹配；`available/deployable/local` 默认 false，通过 react-hook-form 管理。关键词忽略大小写和首尾空格，匹配厂商、模型 ID、规格 ID、API 类型；三个状态条件取交集。

目录为有界元数据集合，一次 `models.list` 获取，详情/筛选/规格展开无额外 RPC，无分页。模型 ID 以 numeric locale 排序，统计数字基于完整目录，结果数字基于筛选。SWR 缓存查询结果，30 秒轮询、窗口聚焦、Provider 更新后的 snapshot version 变化和手动刷新均可更新状态；单次目录读取不依赖首页的 usage 或 trace 成功。

状态：首次读取显示 loading；首次失败显示 error/retry；后续刷新失败保留旧目录并显示错误；成功但零元数据显示空目录；筛选无结果有独立提示和清除入口。详情用 MUI Dialog 处理焦点、Esc、遮罩关闭和窄屏布局。本地部署入口当前禁用并解释后续开放，已存在本地 Provider 时展示已部署，不模拟安装进度。

字段稳定性：身份、providers 关联和规格成员结构是前后端共同契约；metadata 字段可扩展；搜索状态和派生计数属于 UI 实现。新增 `local_deployable` 与 `models.list.catalog` 是本次用户授权的 breaking-change 数据定义更新，无旧结构回退，无新增依赖。

Mock 契约：`mock/model-catalog.ts` 提供三个厂商、七个模型、一个空规格；populated 场景含云端、本地、未接入与可部署但未安装模型，empty 场景仍提供全部目录但所有模型无 Provider。Playwright 额外注入 loading、error/retry、空目录响应。

验证命令：

```sh
pnpm run build
pnpm exec playwright test --config playwright.aicc.config.ts
deno test --node-modules-dir=manual --unstable-sloppy-imports tests/datamodel/model-catalog.test.ts
```

厂商名旁的按钮控制整个厂商内容的展开/收起，默认展开，收起时仅保留厂商名、模型数量与按钮。模型卡片仅在 `deployable` 为 true 时显示一个本地部署图标：未部署为下载图标，`local` 为 true 时为点亮的本地图标，不再重复显示底部部署标记。

## Playground

入口为 AI Center 的桌面侧栏和移动端导航。`PlaygroundPage.tsx` 管理表单与单次调用状态，`datamodel/playground.ts` 定义全部 24 种 `ApiType` 的字段、默认值、校验、请求导入和任务结果转换；`api/aicc_playground.ts` 复用 Web SDK 的认证与 RPC 客户端，无新增依赖或后端协议变更。

### 请求与模型

- `PlaygroundModel` 包含 `exact_model`、`provider`、`api_types`、`health`。真实模式独立调用一次 `models.list`，按 API Type 和 Provider 在本地筛选，无逐模型查询或分页。手动刷新、Provider snapshot 更新后可重新读取；加载、空列表和错误/重试分别展示，加载失败时禁止使用旧模型发起调用。
- 请求直接保存为 canonical JSON 对象，字段编辑只替换对应字段，保留已导入的附加字段。`PlaygroundField` 定义文本、数值、下拉框、复选框、嵌套对象、数组、联合类型、JSON 和资源字段。可选字段默认省略，默认 `execution_mode=immediate`，数值范围、必填、资源格式与决策题约束由 `validateRequest` 校验，错误携带字段路径。
- `llm` → `chat.completions.create`，`image.txt2img` → `images.generate`，`decision` → `decision.evaluate`；其他 API Type 与调用方法同名。只允许映射中的推理方法，不能通过导入 JSON 调用管理接口。
- ResourceRef 支持 HTTP(S) URL、Base64（含粘贴 data URL 的解析）、文件和 `named_object`。文件在浏览器读取为 Base64，与请求一并发送，单文件限制 20 MiB；较大资源用 URL。MIME 由文件类型提供，也可下拉选择。文件读取失败不会复用旧文件内容。
- 高级模式接受 `{method, params}`、完整 TaskMgr task、`input.request.request`、request 包装和裸 canonical 参数。包装内的方法决定 API Type；裸参数使用当前选择的 API Type。导入不自动执行，找不到模型时需重新选择。移除旧 `idempotency_key`、`trace_id`、`task_options`，保留 `session_id` 和业务参数，防止重用旧调用或旧父任务。
- Task Center 的 `Task.aiccRequest` 从 AICC task 的不可变 `input.request.request` 提取，方法来自 `AICC <method>` 任务名；即使任务已经有 result，原始请求仍独立展示并可复制。

### 调用与结果

`PlaygroundRun` 保存提交参数快照、开始时间、耗时、原始响应、完整 task、提交/轮询状态及错误。提交期间和任务运行期间禁止重复提交。切换 AI Center 内部页面保留表单和当前调用；关闭应用后清理轮询，后端任务仍由 TaskMgr 管理。

| 操作 | 接口 / 字段 | UI 行为 |
| --- | --- | --- |
| 发起调用 | 当前 API 的 canonical method | 展示 succeeded / failed；running 时记录 task_id |
| 查询任务 | TaskMgr `get_task(task_id)` | 前一次返回后间隔 1.5 秒继续查询；Terminal 后停止 |
| 查询中断 | 查询错误或持续 10 分钟 | 暂停自动轮询，提供继续查询与 Task Center 链接；不判定 Provider 调用失败 |
| 完成结果 | `task.result.result.output.value` | 合并 output 的 usage、cost、artifacts；保留原始 task |
| 失败 / 取消 | `task.outcome`、`task.error` | 展示失败/取消状态及结构化错误 |
| 取消 | AICC `cancel({task_id})` | accepted 只表示已申请取消，继续查询实际终态 |

结果包含文本/思考内容、可预览的图片/音频/视频、资源链接或对象 ID、向量维数与前六项、决策/重排/检测/分割/语音分段/工具调用等结构化结果，以及结束原因、task ID、Provider task reference、用量、币种费用、路由轨迹与进度事件。原始请求、响应和任务 JSON 可复制/下载；超过 64,000 字符仅截断屏幕预览，完整数据仍可导出。浏览器预览失败与 Provider 失败分别处理，不使用未上报的费用或用量推断为零。

Mock 模式使用当前模拟 Provider 库存，响应由 `mock/playground.ts` 生成，页面明确标示模拟结果。浏览器测试使用可控服务夹具覆盖全部 API、TaskMgr 请求保留、资源输入、异步终态、取消、查询中断、异常响应、空模型/读取重试、中英文和 375px 窄屏。此验证不代替真实 Provider 联网验收。

```sh
pnpm run build
pnpm exec playwright test tests/e2e/aicc-playground.spec.ts
```
