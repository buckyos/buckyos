# webui-prototype skill

# Role

You are an expert Frontend Engineer & AI Harness Agent specializing in BuckyOS WebUI. Your task is to从 PRD 出发，完成一个 Mock-first 的 UI 原型，通过 Playwright 自动化循环收敛质量，最终沉淀出稳定的 UI DataModel 文档。

# Context

本 Skill 对应 WebUI Dev Loop 的三个连续阶段：

```
阶段一：Mock-first Prototype 开发 — 基于 PRD 快速出原型
阶段二：UI Developer Loop (Playwright) — 自动化测试、截图、修复循环
阶段三：UI DataModel 定义 — 从收敛后的原型中提炼稳定数据模型文档
```

核心思路：**先做出来，跑起来，看到了再定义数据模型**。不要试图在没有原型的情况下凭空设计 DataModel。

# Applicable Scenarios

Use this skill when:

- PRD 和 UI Design Prompt 已就绪，需要实现 UI 原型。
- 已有原型需要通过 Playwright 循环收敛质量。
- 原型已收敛，需要提炼 UI DataModel 文档。

Do NOT use this skill when:

- KRPC 接口仍在频繁变化（等后端稳定）。
- 任务是 DataModel × Backend 集成（用 `integrate-ui-datamodel-with-backend`）。
- 任务是真实系统链路 DV 测试（用 `ui-dv-test`）。

# Input

1. **PRD** — 用户任务、页面、交互流程、关键状态。
2. **UI Design Prompt (Optional)** — 布局、组件、视觉方向。
3. **KRPC Protocol Document (Optional)** — 后端接口作为参考，但不驱动 UI 设计。
4. **Target Location** — UI 模块在项目中的位置。

# Output

1. 可独立运行的 UI 原型（`pnpm run dev`，零后端依赖）。
2. Playwright 测试脚本覆盖主要用户流程。
3. UI DataModel 文档（TypeScript interface + 状态定义 + Mock 数据契约）。

---

# Tech Stack (BuckyOS Default)

未特别指定时，使用系统默认技术栈：

| Concern | Choice |
|---------|--------|
| Framework | **React 19** with TypeScript |
| Build Tool | **Vite** (rolldown-vite) |
| Styling | **Tailwind CSS 3** + BuckyOS CSS Variables |
| Icons | **Lucide React** |
| Charts | **@nivo** (bar, line, pie) — 按需使用 |
| Data Fetching | **SWR** |
| Routing | **React Router DOM v7** |
| State Management | React hooks + Context API |
| Forms & Input Validation | **react-hook-form** + **Zod** |
| Fonts | **Space Grotesk** (headings), **Work Sans** (body) |
| i18n | BuckyOS custom i18n (`useI18n()` hook, `I18nProvider`) |

# BuckyOS Visual Style

### Color Palette (CSS Variables)

```css
--cp-primary: #0f766e;        /* Teal — 主操作、活跃导航 */
--cp-primary-strong: #0b5f59;  /* 深 Teal — hover */
--cp-primary-soft: #d1f2ef;    /* 浅 Teal — 背景高亮 */
--cp-accent: #f59e0b;          /* Amber — 次要强调 */
--cp-success: #16a34a;         /* Green — 成功 */
--cp-warning: #d97706;         /* Orange — 警告 */
--cp-danger: #dc2626;          /* Red — 错误、危险操作 */
--cp-bg: #eef4f3;              /* 页面背景 */
--cp-surface: #ffffff;         /* 卡片/面板表面 */
--cp-surface-muted: #f4f8f7;   /* 弱化表面 */
--cp-border: #d7e1df;          /* 边框 */
--cp-ink: #0f172a;             /* 主文本 */
--cp-muted: #52606d;           /* 次要文本 */
```

### Component Classes

- `.cp-shell` — 主容器（max-width: 1280px，居中，桌面 32px 水平内边距）
- `.cp-panel` — 大卡片/面板（白色，bordered，rounded-24px，soft shadow）
- `.cp-panel-muted` — 弱化面板
- `.cp-card` — 小卡片（rounded-18px）
- `.cp-pill` — 状态徽章（rounded-full，小字号）
- `.cp-nav-link` — 导航链接
- `.cp-divider` — 水平分隔线

### Typography

- 标题：`var(--cp-font-heading)` — Space Grotesk，letter-spacing: -0.01em
- 正文：`var(--cp-font-body)` — Work Sans

### Layout

- 最大内容宽度 1280px，居中
- 圆角：24px（面板）、18px（卡片）、16px（导航）、999px（pill）
- 阴影：`var(--cp-shadow)` 或 `var(--cp-shadow-soft)`

### Responsive

- Desktop-first，移动端断点 768px
- 桌面与移动浏览器均 MUST 可用

### Accessibility

- 尊重 `prefers-reduced-motion` — 减弱动画
- 使用语义化 HTML 元素

---

# 阶段一：Mock-first Prototype 开发

## 目标

基于 PRD 快速产出可独立运行的 UI 原型。**不需要先定义 DataModel 文档**——直接在代码中用 TypeScript interface 和 mock 数据边做边探索。

## 实现步骤

TODO:这里要区分几种 UI形态，来确定在哪个项目pnpm run dev

### Step 1: 项目搭建

在现有 Control Panel 中新增模块：

- 添加路由到路由配置。
- 在 `src/` 下创建页面组件目录。

独立模块：

- Vite + React + TypeScript 初始化。
- 导入 BuckyOS CSS Variables 和 Tailwind 配置。
- 设置 `I18nProvider`。

### Step 2: Mock 数据层

在组件目录内创建 mock 数据：

```
mock/
  data.ts        — Mock 数据对象
  provider.ts    — 模拟异步加载（延迟 300-800ms）
```

Mock 数据 MUST：

- 覆盖主要用户路径（happy path + 边界情况）。
- 支撑所有关键状态：正常态、空态、错误态、加载态、进度态（如适用）。
- 使用合理的系统数据（非 "test123" 或 "Lorem ipsum"）。
- 包含足够条目以演示分页行为。
- 让 Playwright 能不卡住地跑完整流程。

### Step 3: 页面实现

对 PRD 中的每个页面：

1. 创建页面组件。
2. 接入 mock 数据 provider。
3. 实现五种状态：正常、空、加载、错误、进度。
4. 应用 BuckyOS 视觉风格（CSS Variables + 组件类）。
5. 所有面向用户的文本通过 i18n：`const { t } = useI18n();`

如果页面包含表单、筛选器、向导、配置面板或任何用户输入区域，MUST：

1. 使用 `react-hook-form` 管理表单状态。
2. 使用 `zod` 定义输入 schema，并通过 resolver 接入表单校验。
3. 让输入 schema 成为 UI DataModel 中“输入模型”的单一事实来源，避免在组件内散落手写校验逻辑。
4. 对默认值、必填项、枚举值、字符串长度、数值范围、跨字段约束给出明确限制。
5. 将校验错误映射为可见、可本地化的 UI 文案，而不是仅依赖浏览器原生校验。

推荐模式：

```typescript
const formSchema = z.object({
  name: z.string().trim().min(1).max(64),
  mode: z.enum(['auto', 'manual']),
  retries: z.number().int().min(0).max(10),
});

type FormValues = z.infer<typeof formSchema>;

const form = useForm<FormValues>({
  resolver: zodResolver(formSchema),
  defaultValues,
});
```

### Step 4: 双视图模式

UI SHOULD 支持两种视图：

1. **独立页面模式** — 全页，开发用。
2. **桌面窗口模式** — 嵌入 BuckyOS 桌面 shell，集成预览。

### Step 5: 验证可独立运行

```bash
pnpm install && pnpm run dev
# → UI 启动，所有页面可访问，控制台无后端错误
```

## i18n 要求

- 默认语言：`en`、`zh-CN`。
- 所有面向用户的字符串 MUST 走 i18n 系统。
- 翻译格式：`t('key.path', 'Fallback text', { interpolation: 'values' })`

## 阶段一 Done

- [ ] `pnpm run dev` 无错启动。
- [ ] PRD 中所有页面已实现并可导航。
- [ ] 正常态、空态、加载态、错误态均正确呈现。
- [ ] 移动端视口（≤768px）可用。
- [ ] i18n：所有字符串走 `t()`，`en` 和 `zh-CN` 均有翻译。
- [ ] 使用 BuckyOS 视觉风格（CSS Variables + 组件类）。
- [ ] 无真实后端依赖——纯 mock 数据。
- [ ] 控制台无错误。

---

# 阶段二：UI Developer Loop (Playwright)

## 目标

通过 Playwright 自动化循环收敛原型质量，使其满足 PRD 要求。**AI 不应依赖人工盯图来解决基础布局和状态问题。**

## Loop 协议

```
pnpm run dev（后台运行）
→ Playwright 自动操作 UI
→ 截图
→ 对照 PRD / UI Prompt 评估
→ 发现问题
→ 修改代码
→ 热重载后重新截图验证
→ 循环直至收敛
```

## Playwright 配置

```bash
pnpm add -D @playwright/test
npx playwright install chromium
```

```typescript
// playwright.config.ts
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  use: {
    baseURL: 'http://localhost:5173',
    screenshot: 'only-on-failure',
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'pnpm run dev',
    port: 5173,
    reuseExistingServer: true,
  },
});
```

## 测试组织

```
tests/e2e/
  pages/
    page-name.spec.ts       — 按页面组织的测试
  flows/
    user-flow-name.spec.ts  — 跨页面用户流程测试
```

## 评估维度

### 布局
- 内容在 `cp-shell` 容器内（max-width 1280px，居中）。
- 面板使用 `cp-panel` / `cp-card`。
- 间距一致（4px 倍数）。
- 无内容溢出或裁切。

### 状态
- 正常：数据正确渲染，列表填充，操作可见。
- 空态：清晰提示，非空白/破损页面。
- 加载：skeleton 或 spinner 可见。
- 错误：错误信息 + 重试按钮。

### 视觉
- 颜色使用 `--cp-*` 变量。
- 字体正确（Space Grotesk / Work Sans）。
- 图标来自 Lucide React。
- 状态指示器使用语义色（success/warning/danger）。

### 响应式
- 桌面（≥769px）：完整布局。
- 移动（≤768px）：可用，无水平滚动，触摸目标 ≥44px。

### i18n
- 无硬编码的面向用户字符串。
- 切换 locale 后所有文本变化。

## 问题分级

- **P0**：布局崩溃、缺失状态、崩溃 → 立即修复。
- **P1**：视觉不一致、间距错误、颜色错误 → 本轮修复。
- **P2**：微调、微交互 → 记录，时间允许再修。

## MUST 测试项

- 所有页面无 console error 渲染。
- 每个数据视图的五种状态。
- PRD 主用户流程（happy path）。
- 页面间导航。
- 移动端视口（375px）。

## AI 行为规则

1. **基础问题自主解决。** 布局崩溃、缺失状态、颜色错误——不要问人。
2. **产品判断交给人。** "这个功能用卡片还是表格？"——问人。
3. **逐个修复。** 不要一次改 10 个文件，改一个、验一个。
4. **追踪修复记录。** 避免回退。
5. **知道何时停止。** P0/P1 清零 + Playwright 通过 = 收敛。同一问题 3 次尝试未解决 → 上报用户。

## 阶段二 Done

- [ ] Dev server 无错启动并服务所有页面。
- [ ] Playwright 测试套件通过。
- [ ] 所有页面正常态截图验证通过。
- [ ] 空态、加载态、错误态验证通过。
- [ ] 移动端视口（375px）验证通过——无水平滚动，内容可用。
- [ ] Playwright 运行期间无 console error。
- [ ] 所有 P0、P1 问题已解决。
- [ ] i18n：locale 切换生效，无硬编码字符串。

---

# 阶段三：UI DataModel 定义

## 目标

从已收敛的原型中**提炼**出 UI DataModel 文档。此时 UI 中实际使用的数据结构已经过原型验证，DataModel 是对现实的文档化，而非凭空设计。

## 提炼方法

1. 审查原型代码中所有 TypeScript interface 和 mock 数据。
2. 识别哪些是稳定的（多个组件依赖）、哪些是临时的。
3. 整理为正式 DataModel 文档。

## DataModel 文档结构

### 1. Overview

- 服务名称。
- 支持的页面/视图。
- 对应 PRD 和 KRPC 协议文档引用。

### 2. DataModel Definitions

用 TypeScript interface 定义每个数据实体：

```typescript
/**
 * 实体的简要描述。
 */
export interface EntityName {
  /** 字段说明 */
  fieldName: FieldType;
}
```

对“用户可编辑输入”类数据，除了展示/读取用的 TypeScript interface，还 MUST 提供对应的 Zod schema：

```typescript
export const entityInputSchema = z.object({
  name: z.string().trim().min(1).max(64),
  description: z.string().trim().max(280).optional(),
  mode: z.enum(['auto', 'manual']),
});

export type EntityInput = z.infer<typeof entityInputSchema>;
```

要求：

- 输入 schema 表达 UI 层真实约束，而不是后端 DTO 的机械镜像。
- `react-hook-form` 表单字段类型从 Zod schema 推导，避免重复定义。
- 在文档中区分“展示模型 / 查询结果模型”与“输入模型 / 编辑模型”。
- 对可选字段、默认值、枚举、格式、范围、跨字段约束写清楚。

### 3. State Definitions

每个数据获取点 MUST 定义：

```typescript
export type LoadingState = 'idle' | 'loading' | 'success' | 'error';

export interface DataState<T> {
  status: LoadingState;
  data: T | null;
  error: string | null;
}
```

列明每个页面的：正常态、空态、加载态、错误态、进度态。

### 4. Pagination & Aggregation

对列表型数据：

- 分页策略（offset / cursor / 无限滚动）。
- 默认及可配置的每页条数。
- 排序字段和默认排序。
- 筛选字段。
- 聚合/派生字段。

### 5. Field Stability Classification

| Field | Stability | Notes |
|-------|-----------|-------|
| id | Frozen | 主标识，不可变 |
| name | Frozen | 核心展示字段 |
| extra | Extensible | 可能新增子字段 |

- **Frozen**：前后端共同依赖，变更为高影响事件。
- **Extensible**：可演进，当前消费者不受新值影响。
- **Volatile**：实现细节，集成阶段可能变。

### 6. Mock Data Contract

为每个 interface 提供样本 mock 对象，覆盖正常、空、错误、边界情况。

对输入模型还应补充：

- 合法输入样本。
- 非法输入样本及预期校验错误。
- 默认值样本。
- 编辑态回填样本。

### 7. KRPC Mapping Notes（Optional）

如有 KRPC 协议文档，记录预期映射：

| UI DataModel Field | KRPC Source | Transform |
|-------------------|-------------|-----------|
| displayName | user.name | Direct |
| statusLabel | task.state | Enum → i18n key |

此节为集成阶段参考，不需要在此阶段完全确定。

## 阶段三 Done

- [ ] 原型中所有实际使用的 interface 已文档化。
- [ ] 五种状态已为每个数据视图定义。
- [ ] 分页策略已明确。
- [ ] 字段稳定性已分类。
- [ ] Mock 数据契约已提供。
- [ ] DataModel 文档可直接交给 `integrate-ui-datamodel-with-backend` 使用。

---

# Common Failure Modes（全阶段通用）

1. **先设计 DataModel 再做原型** — 凭空设计的模型会在原型阶段被推翻。先做原型，后提炼模型。
2. **硬编码字符串** — 破坏 i18n。始终用 `t()`。
3. **缺失状态** — 空页或白屏。每个数据视图 MUST 有五种状态。
4. **后端依赖泄漏** — 原型必须独立运行；任何真实 API 调用都是 bug。
5. **忽略移动端** — 768px 断点是 MUST，不是可选。
6. **自定义颜色替代系统变量** — 使用 `--cp-*` 变量，不要自造颜色。
7. **Playwright 中使用固定 timeout** — 用 `waitForSelector`，不要 `waitForTimeout`。
8. **DataModel 1:1 照搬 KRPC** — DataModel 应由 UI 需求驱动，不是后端结构的镜像。
9. **过度抽象** — 这是原型阶段，聚焦用户流程，不要追求架构完美。

# Overall Pass Criteria

本 Skill 全部完成的标志：

- `pnpm run dev` 独立运行，零后端依赖。
- PRD 中所有主要用户流程可走通。
- 所有关键 UI 状态已正确渲染。
- Playwright 测试套件通过，P0/P1 问题清零。
- UI DataModel 文档已从原型中提炼产出。
- 原型可交付产品负责人进行体验审查（进入 UI PR 阶段）。
