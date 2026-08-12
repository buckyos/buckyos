# AI Center — 前端原型实现文档

> 目标读者：Claude Code（前端工程师角色）
> 产出物：可运行的 UI 原型，全部使用 Mock 数据驱动，不依赖任何后端接口。

---

## 0. 核心原则

1. **这是原型，不是最终产品。** 目标是让产品/设计能在浏览器里点通所有流程，验证信息架构和交互主线。
2. **没有后端，没有 API。** 所有数据来自 `mock/` 目录下的静态 JSON + 内存状态。数据层用一个轻量 `MockDataStore` 管理，支持增删改，刷新后重置。
3. **响应式从第一行代码开始。** 每个组件都同时适配桌面（≥768px）和移动端（<768px）。
4. **先跑通再打磨。** Phase 1 聚焦完整流程闭环，Phase 2 再补图表和深钻。

---

## 1. 技术栈

| 项目 | 选型 | 说明 |
|------|------|------|
| 框架 | React 18 + TypeScript | strict 模式 |
| 构建 | Vite | 开发服务器 + HMR |
| 路由 | React Router v6 | |
| 样式 | Tailwind CSS | 8pt 间距体系 |
| 图标 | Lucide React | |
| 图表 | Recharts |  |
| 拖拽 | @dnd-kit | Phase 2 Routing 高级模式才用 |
| 动画 | framer-motion | Wizard 步骤过渡 |
| 数据 | 内存 MockDataStore | 无网络请求 |

---

## 2. 推荐目录结构

AICenter在BuckyOS中只会在Desktop环境下运行：只需要做PanelAdapter

```
src/app/ai-center/
├── AICenterAppPanel.tsx                        
├── mock/
│   ├── store.ts                      # MockDataStore 类（内存状态管理）
│   ├── seed.ts                       # 初始种子数据
│   └── types.ts                      # 全部 TypeScript 类型
├── HomePage.tsx                  # 首页：未启用→引导 / 已启用→Usage
├── ProvidersPage.tsx             # Provider 列表 + 详情
├── AddProviderPage.tsx           # Wizard 全屏页
├── ModelsPage.tsx                # 本地模型
└── RoutingPage.tsx               # Routing 只读概要

### 组件优先复用Desktop已有的

├── components/
│   ├── layout/
│   │   ├── AICenterShell.tsx         # 桌面Sidebar + 移动TopTab 容器
│   │   ├── Sidebar.tsx
│   │   └── MobileTabBar.tsx
│   ├── home/
│   │   ├── EnableAIGuide.tsx         # 首启引导
│   │   ├── StatusCard.tsx            # AI 状态卡
│   │   ├── CreditCard.tsx            # SN Credit 卡
│   │   ├── CostCard.tsx              # 估算花费卡
│   │   ├── BalanceOverviewCard.tsx   # Provider 余额概览卡
│   │   ├── UsageTrendChart.tsx       # 用量趋势折线图（Phase 2）
│   │   └── UsageSummarySection.tsx   # 用量摘要聚合区
│   ├── providers/
│   │   ├── ProviderList.tsx
│   │   ├── ProviderDetailPanel.tsx
│   │   ├── ProviderCard.tsx
│   │   └── wizard/
│   │       ├── WizardShell.tsx       # Stepper + 步骤容器 + 底部按钮
│   │       ├── StepChooseType.tsx
│   │       ├── StepConnection.tsx
│   │       ├── StepValidation.tsx
│   │       └── StepReview.tsx
│   ├── models/
│   │   ├── LocalModelList.tsx
│   │   └── ModelsEmptyState.tsx
│   ├── routing/
│   │   ├── RoutingOverview.tsx       # 只读逻辑模型映射表
│   │   └── LogicalModelEditor.tsx    # Phase 2 高级编辑
│   └── shared/
│       ├── EmptyState.tsx
│       ├── StatusBadge.tsx
│       ├── Stepper.tsx
│       ├── ConfirmDialog.tsx
│       └── SummaryCard.tsx           # 通用摘要卡片（icon + 标题 + 值 + 副文案）
└── hooks/
    └── use-mock-store.ts             # React context 暴露 MockDataStore
```

---

## 3. 类型定义

文件：`mock/types.ts`

```typescript
// ========== 枚举 ==========

export type ProviderType =
  | "sn_router" | "openai" | "anthropic" | "google" | "openrouter" | "custom";

export type AuthMode = "api_key" | "oauth";
export type ProtocolType = "openai_compatible" | "anthropic_compatible" | "google_compatible";
export type AuthStatus = "ok" | "expired" | "invalid" | "unknown";
export type ModelSyncStatus = "ok" | "syncing" | "failed";
export type UsageCategory = "text" | "image" | "audio" | "video";
export type AISystemState = "disabled" | "single_provider" | "multi_provider";

// ========== Provider ==========

export interface ProviderConfig {
  id: string;
  name: string;
  provider_type: ProviderType;
  auth_mode?: AuthMode;
  endpoint?: string;
  protocol_type?: ProtocolType;
  auto_sync_models: boolean;
  created_at: string;
}

export interface ProviderStatus {
  provider_id: string;
  is_connected: boolean;
  auth_status: AuthStatus;
  usage_supported: boolean;
  balance_supported: boolean;
  discovered_models: string[];
  model_sync_status: ModelSyncStatus;
  last_verified_at?: string;
  last_model_sync_at?: string;
}

export interface ProviderAccountStatus {
  provider_id: string;
  usage_supported: boolean;
  cost_supported: boolean;
  balance_supported: boolean;
  usage_value?: number;
  estimated_cost?: number;
  balance_unit?: "usd" | "credit";
  balance_value?: number;
  topup_url?: string;
}

// 前端展示用的聚合视图
export interface ProviderView {
  config: ProviderConfig;
  status: ProviderStatus;
  account: ProviderAccountStatus;
}

// ========== Usage ==========

export interface UsageEvent {
  id: string;
  timestamp: string;
  provider_id: string;
  model_name: string;
  category: UsageCategory;
  app_id?: string;
  agent_id?: string;
  session_id?: string;
  tokens_in: number;
  tokens_out: number;
  estimated_cost?: number;
  status: "success" | "failed";
}

export interface UsageSummary {
  total_tokens: number;
  total_requests: number;
  total_estimated_cost: number;
  today_tokens: number;
  this_month_tokens: number;
  by_category: Record<UsageCategory, number>;
  by_provider: Record<string, number>;
  by_model: Record<string, number>;
}

export interface UsageTrendPoint {
  timestamp: string;
  tokens: number;
  estimated_cost: number;
}

// ========== Routing ==========

export interface LogicalModelCandidate {
  provider_id: string;
  model_name: string;
  priority: number;
  enabled: boolean;
}

export interface LogicalModelConfig {
  name: string;
  candidates: LogicalModelCandidate[];
  // 便于展示：当前实际解析到的模型名
  resolved_model?: string;
}

// ========== Local Model ==========

export interface LocalModel {
  id: string;
  name: string;
  size_bytes?: number;
  status: "ready" | "loading" | "error";
  last_used_at?: string;
}

// ========== System ==========

export interface AIStatus {
  state: AISystemState;
  provider_count: number;
  model_count: number;
  default_routing_ok: boolean;
}

// ========== Wizard ==========

export interface WizardDraft {
  provider_type: ProviderType | null;
  name: string;
  endpoint: string;
  protocol_type: ProtocolType | null;
  api_key: string;
  auto_sync_models: boolean;
}

export interface ValidationResult {
  endpoint_reachable: boolean;
  auth_valid: boolean;
  models_discovered: string[];
  balance_available: boolean;
  errors: string[];
}
```

---

## 4. MockDataStore 规格

文件：`mock/store.ts`

这是原型的数据引擎。纯内存，同步操作，刷新即重置。

```typescript
class MockDataStore {
  // ---- 内部状态 ----
  private providers: Map<string, ProviderView>;
  private usageEvents: UsageEvent[];
  private logicalModels: LogicalModelConfig[];
  private localModels: LocalModel[];
  private listeners: Set<() => void>;   // 通知 React 重渲染

  constructor() {
    // 从 seed.ts 加载初始数据
  }

  // ---- 订阅机制（配合 useSyncExternalStore）----
  subscribe(listener: () => void): () => void;
  getSnapshot(): StoreSnapshot;

  // ---- Provider 操作 ----
  getAIStatus(): AIStatus;
  getProviders(): ProviderView[];
  getProvider(id: string): ProviderView | undefined;
  addProvider(draft: WizardDraft): ProviderView;      // 模拟创建
  deleteProvider(id: string): void;
  refreshProviderModels(id: string): void;             // 模拟刷新（noop）

  // ---- Wizard 模拟 ----
  validateConnection(draft: WizardDraft): ValidationResult;
  // 根据 draft.provider_type 返回预设的验证结果
  // Custom 且 endpoint 为空 → endpoint_reachable = false
  // api_key 为空 → auth_valid = false
  // 否则全部 pass，返回该类型对应的模型列表

  // ---- Usage ----
  getUsageSummary(): UsageSummary;
  getUsageTrend(granularity: string): UsageTrendPoint[];
  getUsageEvents(filters?: { provider_id?: string; model?: string }): UsageEvent[];

  // ---- Models ----
  getLocalModels(): LocalModel[];

  // ---- Routing ----
  getLogicalModels(): LogicalModelConfig[];
  updateLogicalModel(name: string, config: LogicalModelConfig): void;
}
```

### 种子数据场景

文件 `mock/seed.ts` 提供两套种子数据，用一个 URL param `?scenario=empty|populated` 切换：

**Scenario A: `empty`（默认）**
- 0 个 Provider → AI 状态 = disabled
- 用于验证首启引导流程

**Scenario B: `populated`**
- 3 个 Provider：
  1. SN Router（connected, 余额 500 credit）
  2. OpenAI（connected, 余额 $23.50, 12 个模型）
  3. Anthropic（auth_expired, 余额 unknown）
- 150 条 UsageEvent（覆盖过去 30 天，text/image 混合）
- 4 个 LogicalModel：llm.chat, llm.plan, llm.code, txt2img
- 0 个 LocalModel（空状态）
- 用于验证 Usage Dashboard、Provider 管理、Routing 展示

### React Hook

文件：`hooks/use-mock-store.ts`

```typescript
// 用 React Context + useSyncExternalStore 暴露 store
const MockStoreContext = createContext<MockDataStore>(null!);

export function MockStoreProvider({ children }: { children: ReactNode }) {
  const [store] = useState(() => new MockDataStore());
  return <MockStoreContext.Provider value={store}>{children}</MockStoreContext.Provider>;
}

export function useMockStore() {
  return useContext(MockStoreContext);
}

// 便捷 selector hook
export function useAIStatus(): AIStatus { ... }
export function useProviders(): ProviderView[] { ... }
export function useProvider(id: string): ProviderView | undefined { ... }
export function useUsageSummary(): UsageSummary { ... }
// ...
```

---

## 5. 路由定义

```typescript
// index.tsx
<Route path="/ai-center" element={<AICenterShell />}>
  <Route index element={<HomePage />} />
  <Route path="providers" element={<ProvidersPage />} />
  <Route path="providers/add" element={<AddProviderPage />} />
  <Route path="models" element={<ModelsPage />} />
  <Route path="routing" element={<RoutingPage />} />
</Route>
```

---

## 6. 分 Phase 任务清单

### Phase 1：流程闭环原型

> 完成后产出：可在浏览器里完整走通"首启 → 添加 Provider → 看到 Usage 首页 → 浏览 Provider 列表/详情 → Models 空状态 → Routing 只读"全流程。

---

#### T01: 脚手架 + 类型 + MockDataStore

**创建文件：**
- `mock/types.ts` — 粘贴上方第 3 节完整类型定义
- `mock/seed.ts` — 实现两套种子数据（empty / populated）
- `mock/store.ts` — 实现 MockDataStore 类，所有方法
- `hooks/use-mock-store.ts` — Context + hooks

**验收：**
- 在一个临时测试组件里能 `useMockStore()` 拿到数据，控制台打印出 AIStatus
- `?scenario=empty` 时 state=disabled，`?scenario=populated` 时 state=multi_provider

---

#### T02: Layout Shell + 路由

**创建文件：**
- `index.tsx` — 路由定义
- `components/layout/AICenterShell.tsx`
- `components/layout/Sidebar.tsx`
- `components/layout/MobileTabBar.tsx`

**规格：**

```
桌面（≥768px）：
┌─────────────┬───────────────────────────────────┐
│ Sidebar     │  <Outlet />                       │
│ w-60 (240px)│  max-w-5xl mx-auto px-8           │
│             │                                   │
│ • Home      │                                   │
│ • Providers │                                   │
│ • Models    │                                   │
│ • Routing   │                                   │
└─────────────┴───────────────────────────────────┘

移动（<768px）：
┌──────────────────────────────┐
│ App Bar: "AI Center"         │
├──────────────────────────────┤
│ Home │ Providers │ Models │⋯ │  ← 水平滚动 Tab
├──────────────────────────────┤
│ <Outlet />                   │
│ px-4                         │
└──────────────────────────────┘
```

- Sidebar 导航项用 Lucide icon + 文字
- 当前路由项高亮（左侧 2px 蓝色边框 + 背景色）
- 移动端 Tab 超出时可横向滚动，Routing 在 "更多" 里或直接作为第 4 个 Tab

**验收：**
- 桌面/移动端切换时布局正确切换
- 点击导航项路由跳转正常
- 每个路由显示一个占位 `<h1>` 文字

---

#### T03: 共享组件

**创建文件：**
- `components/shared/EmptyState.tsx`
- `components/shared/StatusBadge.tsx`
- `components/shared/Stepper.tsx`
- `components/shared/ConfirmDialog.tsx`
- `components/shared/SummaryCard.tsx`

**SummaryCard 规格：**
```
Props:
  icon: ReactNode          // Lucide icon
  title: string            // "AI 状态"
  value: string | number   // "已启用" / "500 Credit"
  subtitle?: string        // "3 Providers · 24 Models"
  variant?: "default" | "warning" | "error"
  action?: { label: string; onClick: () => void }  // "充值"

渲染：
┌──────────────────────┐
│ 🔵  AI 状态           │
│     已启用            │  ← value 大字
│     3 Providers · 24  │  ← subtitle 灰字
│              [充值]   │  ← action 可选
└──────────────────────┘

卡片：rounded-xl border bg-white p-4，
warning 时左侧 border-l-4 border-yellow-400，
error 时 border-l-4 border-red-400
```

**Stepper 规格：**
```
Props:
  steps: string[]
  current: number        // 0-based

桌面：水平排列
  ① 选择类型 ─── ② 连接配置 ─── ③ 验证 ─── ④ 确认
  完成的步骤绿色实心，当前蓝色，未来灰色

移动：简化
  步骤 2 / 4 · 连接配置    ← 一行文字 + 下方进度条
```

**StatusBadge 规格：**
```
Props:
  status: "ok" | "warning" | "error" | "unknown"
  label?: string

渲染：● 正常（绿） / ● 警告（黄） / ● 异常（红） / ● 未知（灰）
小圆点 + 文字，inline-flex
```

**验收：**
- 每个共享组件有基本 Storybook 级别的可视化（在一个 `/ai-center/dev` 临时路由里展示所有共享组件变体）

---

#### T04: 首启引导页

**创建文件：**
- `components/home/EnableAIGuide.tsx`
- 修改 `pages/HomePage.tsx`

**逻辑：**
```typescript
function HomePage() {
  const status = useAIStatus();
  if (status.state === "disabled") return <EnableAIGuide />;
  return <UsageDashboard />;  // T08 实现
}
```

**EnableAIGuide 规格：**
```
居中布局，垂直排列：

┌────────────────────────────────────┐
│          [AI Icon 插图]            │
│                                    │
│     AI 功能尚未启用                │  ← text-xl font-semibold
│                                    │
│  需要至少添加一个 AI Provider，    │  ← text-gray-500
│  才能开始使用系统中的 AI 能力。    │
│                                    │
│     [ 开始启用 ]   了解 Provider   │  ← 主按钮 + 文字链接
│                                    │
│  ┌──────┐ ┌──────┐ ┌──────┐       │
│  │  SN  │ │OpenAI│ │Claude│ ...   │  ← 可选：Provider Logo 预览
│  └──────┘ └──────┘ └──────┘       │
└────────────────────────────────────┘
```

- "开始启用" → `navigate("/ai-center/providers/add")`
- AI Icon 可以先用 Lucide `<BrainCircuit>` 放大

**验收：**
- `?scenario=empty` → 进入 `/ai-center` 看到引导页
- `?scenario=populated` → 进入 `/ai-center` 看到（暂时的占位）Dashboard
- 点"开始启用"跳转到 Wizard 页

---

#### T05: Add Provider Wizard — 框架 + Step 1

**创建文件：**
- `pages/AddProviderPage.tsx`
- `components/providers/wizard/WizardShell.tsx`
- `components/providers/wizard/StepChooseType.tsx`

**WizardShell 规格：**
```
全屏占据内容区（不是弹窗），顶部 Stepper，底部固定操作栏。

┌────────────────────────────────────────┐
│  ← 返回     添加 Provider             │  ← 顶部 Bar
├────────────────────────────────────────┤
│  ①──②──③──④  Stepper                  │
├────────────────────────────────────────┤
│                                        │
│  { 当前步骤内容 }                      │
│                                        │
├────────────────────────────────────────┤
│              [上一步]  [下一步]         │  ← 固定底部
└────────────────────────────────────────┘

内部状态：
  const [step, setStep] = useState(0);
  const [draft, setDraft] = useState<WizardDraft>(初始空值);
  const [validation, setValidation] = useState<ValidationResult | null>(null);

步骤切换用 framer-motion AnimatePresence，左右滑动。
"返回" 按钮：step > 0 时回退步骤，step === 0 时 navigate(-1)。
```

**StepChooseType 规格：**
```
卡片网格选择 Provider 类型。

桌面：3 列  移动：1 列

每张卡片：
┌─────────────────────┐
│ [Icon]  SN Router   │
│ 系统默认路由，      │
│ 适合大多数用户      │
│           [推荐] ←──│── 仅 SN Router 有此标签
└─────────────────────┘

排序：SN Router → OpenAI → Anthropic → Google → OpenRouter → Custom

选中态：ring-2 ring-blue-500 + 背景淡蓝
```

**Provider 类型元数据（hardcode 在组件里）：**
```typescript
const PROVIDER_TYPES = [
  { type: "sn_router",  name: "SN Router",  desc: "系统默认 AI 路由，适合大多数用户", recommended: true },
  { type: "openai",     name: "OpenAI",      desc: "GPT 系列模型" },
  { type: "anthropic",  name: "Anthropic",   desc: "Claude 系列模型" },
  { type: "google",     name: "Google",      desc: "Gemini 系列模型" },
  { type: "openrouter", name: "OpenRouter",  desc: "多模型聚合路由" },
  { type: "custom",     name: "Custom",      desc: "自定义 API 端点，支持 OpenAI/Anthropic/Google 协议" },
];
```

**验收：**
- Wizard 页面正常渲染，Stepper 显示 4 步
- 选中一个类型后卡片高亮
- 点"下一步"只在已选择时可用
- 移动端正常堆叠

---

#### T06: Wizard Step 2 — 连接配置

**创建文件：**
- `components/providers/wizard/StepConnection.tsx`

**表单按 provider_type 动态渲染：**

| provider_type | 字段 |
|---|---|
| sn_router | Provider Name（默认 "SN Router"）+ 状态展示文案（"账号已激活"）|
| openai | Provider Name（默认 "OpenAI"）+ API Key* + Endpoint（可选，placeholder: `https://api.openai.com`）|
| anthropic | Provider Name（默认 "Anthropic"）+ API Key* + Endpoint（可选）|
| google | Provider Name（默认 "Google AI"）+ API Key* + Endpoint（可选）|
| openrouter | Provider Name（默认 "OpenRouter"）+ API Key* |
| custom | Provider Name* + Endpoint URL* + Protocol Type*（下拉）+ API Key* |

\* 必填字段

**交互：**
- API Key 输入框：`type="password"` + 右侧眼睛图标切换可见性
- Protocol Type 下拉选项：OpenAI Compatible / Anthropic Compatible / Google Compatible
- 必填项为空时 "下一步" 按钮 disabled
- 表单值写入 `draft` 状态

**验收：**
- 选择 OpenAI 后看到 API Key + Endpoint 表单
- 选择 Custom 后看到完整表单（含 Protocol Type）
- 表单校验正常

---

#### T07: Wizard Step 3 + Step 4 — 验证与创建

**创建文件：**
- `components/providers/wizard/StepValidation.tsx`
- `components/providers/wizard/StepReview.tsx`

**StepValidation 规格：**

进入时自动调用 `store.validateConnection(draft)`。用 `setTimeout` 模拟 1.5s 延迟。

```
验证进行中：
  ⏳ 检查 Endpoint 连通性...
  ⏳ 验证认证信息...
  ⏳ 发现可用模型...
  ⏳ 检查余额能力...

验证完成：
  ✅ Endpoint 可达
  ✅ 认证有效
  ✅ 已发现 12 个模型
  ⚠️ 余额查询不可用          ← balance_available=false 时

验证失败：
  ✅ Endpoint 可达
  ❌ 认证无效：API Key 格式错误
       [返回修改]
```

- 逐项依次出现（200ms 间隔，framer-motion fade-in）
- 发现模型时展示可滚动模型列表（最多显示 10 条 + "等 N 个模型"）
- 全部通过或仅 balance 不可用 → 可进入下一步
- 有 ❌ 项 → "下一步" disabled，显示 "返回修改" 按钮

**StepReview 规格：**

```
摘要确认卡片：
┌────────────────────────────────────┐
│ Provider 类型      OpenAI          │
│ 名称              My OpenAI       │
│ Endpoint          (默认)           │
│ 认证方式          API Key          │
│ 连接状态          ✅ 已验证        │
│ 已发现模型        12 个            │
│ 自动同步模型列表  [========] ON    │  ← Toggle
└────────────────────────────────────┘

        [ 上一步 ]    [ 创建 Provider ]
```

- 点击"创建 Provider" → 调用 `store.addProvider(draft)`
- 成功后 → `navigate("/ai-center")` + AI 状态自动刷新为 enabled
- 添加模拟 300ms 延迟 + loading 态

**验收：**
- 完整走通 Step1→2→3→4→创建 → 跳回首页 → 首页变为 Dashboard（而非引导页）
- 故意留空 API Key → Step 3 显示验证失败
- 可以返回上一步修改

---

#### T08: Home / Usage Dashboard

**修改文件：**
- `pages/HomePage.tsx`

**创建文件：**
- `components/home/UsageSummarySection.tsx`
- `components/home/StatusCard.tsx`（基于 SummaryCard）
- `components/home/CreditCard.tsx`
- `components/home/CostCard.tsx`
- `components/home/BalanceOverviewCard.tsx`

**布局：**

```
桌面：
┌────────────────────────────────────────────────────────┐
│ [Status]  [SN Credit]  [Est. Cost]  [Balance]          │  ← 4 卡并排
├────────────────────────────────────────────────────────┤
│ 用量趋势（Phase 2 占位：灰色矩形 + "图表开发中"）      │
├────────────────────────────────────────────────────────┤
│ 用量摘要                                               │
│   今日: 12,340 tokens · 本月: 456,789 tokens           │
│   累计: 1,234,567 tokens · 估算花费: $12.34            │
├────────────────────────────────────────────────────────┤
│ 分类用量（Phase 2 占位）                               │
└────────────────────────────────────────────────────────┘

移动：所有卡片纵向堆叠
```

**四张摘要卡数据来源：**

| 卡片 | 数据 |
|------|------|
| StatusCard | `useAIStatus()` → 状态文案 + Provider/Model 数量 |
| CreditCard | 遍历 providers 找 sn_router 的 account.balance_value |
| CostCard | `useUsageSummary().total_estimated_cost`，标注 "Estimated" |
| BalanceOverviewCard | 遍历所有 provider 的 account，列出可查询的余额 |

**验收：**
- `?scenario=populated` 下首页显示 4 张卡片 + 用量摘要
- 数据正确反映种子数据
- 移动端堆叠正常

---

#### T09: Provider 列表 + 详情

**创建文件：**
- `components/providers/ProviderList.tsx`
- `components/providers/ProviderDetailPanel.tsx`
- `components/providers/ProviderCard.tsx`

**修改文件：**
- `pages/ProvidersPage.tsx`

**布局：**

```
桌面：
┌────────────────────┬──────────────────────────────────┐
│ Provider List      │  Provider Detail                 │
│ (w-80, ~320px)     │  (flex-1)                        │
│                    │                                  │
│ ┌──────────────┐   │  OpenAI                          │
│ │ SN Router  ●↗│   │  ──────────────────────          │
│ ├──────────────┤   │  类型: openai                    │
│ │◉OpenAI    ●↗│   │  Endpoint: (默认)                │
│ ├──────────────┤   │  认证: API Key · ✅ 有效         │
│ │ Anthropic ●↗│   │  模型: 12 个 · 已同步            │
│ └──────────────┘   │  余额: $23.50                    │
│                    │  本月用量: 123,456 tokens         │
│ [+ 添加 Provider]  │                                  │
│                    │  [更新 Key] [刷新模型] [删除]     │
└────────────────────┴──────────────────────────────────┘

移动：
纯列表页，点击某个 Provider → 进入详情全屏页（用路由 state 传数据，或 URL params）
```

**ProviderCard（列表项）：**
```
│ [Icon] OpenAI        ● 正常   12 模型  │
```
- 左侧 icon 按 provider_type 映射
- 右侧 StatusBadge + 模型数

**ProviderDetailPanel：**
- 基础信息区
- 运行状态区（StatusBadge 展示连接/授权/同步状态）
- 模型列表（简单展示 discovered_models，可折叠）
- 余额与用量区
- 操作区：删除需 ConfirmDialog

**验收：**
- populated 场景下列表显示 3 个 Provider
- 点击切换右侧详情
- Anthropic 显示 auth_expired 警告态
- 删除后列表刷新

---

#### T10: Models 空状态 + 列表

**创建文件：**
- `components/models/LocalModelList.tsx`
- `components/models/ModelsEmptyState.tsx`

**修改文件：**
- `pages/ModelsPage.tsx`

**逻辑：**
```typescript
function ModelsPage() {
  const models = useLocalModels();
  if (models.length === 0) return <ModelsEmptyState />;
  return <LocalModelList models={models} />;
}
```

**ModelsEmptyState：**
```
居中：
  [Package Icon]
  尚未安装本地模型
  本地模型需要通过 Store 安装。
  [ 前往 Store 安装模型 ]      ← 按钮，当前 console.log("navigate to store")
```

**验收：**
- populated 场景下（0 个本地模型）→ 显示空状态
- 按钮可点击

---

#### T11: Routing 只读概要

**创建文件：**
- `components/routing/RoutingOverview.tsx`

**修改文件：**
- `pages/RoutingPage.tsx`

**RoutingOverview 规格：**

```
标题：AI 路由配置

┌─────────────────────────────────────────────────┐
│ 逻辑模型名        当前模型              状态    │
│ ─────────────────────────────────────────────── │
│ llm.chat          OpenAI / gpt-4o        ✅    │
│ llm.plan          OpenAI / gpt-4o        ✅    │
│ llm.code          OpenAI / gpt-4o        ✅    │
│ txt2img           (未配置)               ⚠️    │
└─────────────────────────────────────────────────┘

底部：
  想要自定义路由策略？[进入高级模式]    ← Phase 2，当前 disabled
```

- 纯只读表格/列表
- 逻辑模型名用等宽字体
- "当前模型" 从 `resolved_model` 取值
- 移动端改为卡片列表

**验收：**
- populated 场景显示 4 个逻辑模型映射
- 无交互，纯展示

---

### Phase 2：图表 + 深钻 + 高级模式

Phase 2 不在原型初版范围内，但列出任务清单供后续排期：

- T12: UsageTrendChart — Recharts AreaChart，时间粒度切换
- T13: CategoryBreakdown — PieChart
- T14: ProviderBreakdown + ModelBreakdown — BarChart
- T15: AppAgentBreakdown — 层级列表 App→Agent→Session
- T16: UsageDetailTable — 分页 + 筛选 + 桌面表格/移动卡片
- T17: Routing 高级模式 — @dnd-kit 拖拽排序 + 启停 Toggle
- T18: Provider 余额增强 — 迷你趋势图 + 精确/估算/不可用分级展示

---

## 7. 视觉规范速查

来自 PRD Section 12，在原型中尽量遵守：

| 项 | 桌面 | 移动 |
|----|------|------|
| 内容区最大宽度 | 1280px | 100% |
| 左右内边距 | 32px | 16px |
| 模块间距 | 24px | 16px |
| 卡片圆角 | 12px | 12px |
| 卡片内边距 | 16px | 16px |
| 按钮高度 | 36-40px | 44px |
| 输入框高度 | 40px | 44px |
| 图表最小高度 | 220px | 180px |

颜色不做强制要求，用 Tailwind 默认色板即可。系统级面板风格，不要花哨。

---

## 8. 执行规则

1. **严格按 T01 → T11 顺序执行，不跳步。**
2. **每完成一个任务，`npm run dev` 确认页面无白屏无报错。**
3. **所有数据走 MockDataStore，不写任何 `fetch` / `axios` / API 调用。**
4. **每个组件都做响应式：用 Tailwind `md:` 断点。**
5. **commit message**: `feat(ai-center): T03 首启引导页`
6. **不要提前做 Phase 2 的事**，Phase 1 图表区用灰色占位块 + "开发中" 文字代替。
