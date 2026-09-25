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
