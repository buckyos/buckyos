# ui-dv-test skill

# Role

You are an expert QA Engineer & AI Harness Agent specializing in BuckyOS WebUI end-to-end verification. Your task is to用 Playwright 模拟真人操作，在真实系统链路（真实身份、Gateway、SDK、后端服务）中验证 UI 可正确运行，完成从浏览器到后端的完整链路验证。

# Context

本 Skill 对应 WebUI Dev Loop 的 **阶段六：UI DV Test**——整个 WebUI 开发流程的最后一个环节。

```
前置阶段（已完成）：
  webui-prototype → UI 原型已收敛
  integrate-ui-datamodel-with-backend → DataModel 已集成
  bechmark-ui-datamodel → 性能基准已通过

本阶段：
  ui-dv-test → 真实系统链路端到端验证（Playwright 模拟真人操作）
```

**与其他测试的核心区别：**

| 测试类型 | 工具 | 环境 | 验证目标 |
|---------|------|------|----------|
| `webui-prototype` 阶段 Playwright | Playwright | Mock 数据 | UI 布局、状态、交互 |
| `service-dv-test` | TS 脚本直调 KRPC | 真实后端 | 服务接口链路 |
| **`ui-dv-test`（本 Skill）** | **Playwright 模拟真人** | **真实后端** | **浏览器 → Gateway → 服务 → UI 渲染** |

本阶段的核心价值是：**用 Playwright 模拟真人在浏览器中的操作，验证完整链路在真实系统上端到端可用。**

## 真实链路

```
Playwright 控制浏览器
→ 导航到真实 URL
→ 登录（输入用户名密码 / 使用 session token）
→ UI 通过 Web SDK 发起请求
→ 请求进入 Gateway
→ Gateway 权限检测
→ Gateway 路由到后端 Service
→ Service 处理并返回
→ UI 渲染结果
→ Playwright 截图 + 断言
```

**MUST NOT**: 使用 Mock 数据、拦截网络请求、绕过 Gateway 直打服务端口。

# Applicable Scenarios

Use this skill when:

- DataModel × Backend 集成已完成，需要在真实系统上做端到端验证。
- 版本发布前需要验证 UI 在真实环境中可用。
- 系统升级后需要回归验证 UI 功能。
- 后端接口变更后需要验证 UI 兼容性。

Do NOT use this skill when:

- UI 原型尚未完成（use `webui-prototype`）。
- DataModel 集成尚未完成（use `integrate-ui-datamodel-with-backend`）。
- 只需验证后端接口（use `service-dv-test`）。
- 只需验证 UI 布局和交互（use `webui-prototype` 中的 Playwright Loop）。

# Input

1. **UI 项目位置** — UI 代码的路径。
2. **真实环境信息** — Gateway 地址、系统 URL。
3. **登录凭证** — 用户名/密码，或 owner private key 路径（用于获取 session token）。
4. **PRD** — 用户任务与流程定义（用于确定测试覆盖范围）。
5. **Prototype 阶段 Playwright 测试（Optional）** — 已有的测试脚本，可作为 DV Test 的基础改造。

# Output

1. **Playwright DV Test 脚本** — 在真实系统上模拟真人操作的端到端测试。
2. **测试执行报告** — 包含截图证据、通过/失败结果。
3. **问题清单** — 测试中发现的问题及严重程度。

---

# 操作步骤

## Step 1: 测试环境确认

### 1.1 确认系统可用

在编写测试前，**MUST** 先确认真实系统链路可用：

```bash
# 确认 Gateway 可达
curl -s http://<gateway-url>/health || echo "Gateway not reachable"

# 确认 system-config 可达
curl -s http://127.0.0.1:3200/kapi/system_config -X POST \
  -H "Content-Type: application/json" \
  -d '{"method":"sys_config_get","params":{"key":"boot/config"},"sys":[1]}'

# 确认目标服务可达
curl -s http://127.0.0.1:<service-port>/kapi/<service-name> -X POST \
  -H "Content-Type: application/json" \
  -d '{"method":"ping","params":{},"sys":[1]}'
```

### 1.2 确认登录链路

确认能通过浏览器正常登录系统：

- verify-hub 可用。
- 用户凭证有效。
- session token 可正常获取。

### 1.3 确认 UI 已部署

- UI 已构建并部署到真实环境（非 `pnpm run dev` 开发模式）。
- 或 UI 在开发模式下连接真实后端（通过代理配置）。

## Step 2: 测试项目搭建

### 2.1 目录结构

```
tests/
  dv/
    playwright.config.ts       ← DV 专用配置（指向真实环境）
    auth/
      login.setup.ts           ← 登录前置步骤（全局共享）
      storage-state.json       ← 登录状态存储（gitignore）
    flows/
      main-user-flow.spec.ts   ← 主用户流程（happy path）
      crud-operations.spec.ts  ← CRUD 操作验证
      error-handling.spec.ts   ← 错误处理验证
      edge-cases.spec.ts       ← 边界场景验证
    helpers/
      selectors.ts             ← 页面元素选择器集中管理
      actions.ts               ← 可复用的操作序列
      assertions.ts            ← 自定义断言
      wait-strategies.ts       ← 等待策略
    screenshots/               ← 测试截图输出目录（gitignore）
```

### 2.2 Playwright 配置（DV 专用）

```typescript
// tests/dv/playwright.config.ts
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './flows',
  fullyParallel: false,       // DV Test 顺序执行，避免并发干扰
  retries: 1,                 // 允许一次重试，排除偶发网络问题
  timeout: 60_000,            // 单个测试 60 秒超时（真实后端比 mock 慢）
  expect: {
    timeout: 10_000,          // 断言等待 10 秒（真实数据加载需要时间）
  },

  use: {
    // 指向真实环境
    baseURL: process.env.DV_BASE_URL || 'http://localhost:4020',
    // 保留登录状态
    storageState: './auth/storage-state.json',
    // 所有测试截图
    screenshot: 'on',
    // 失败时录制 trace
    trace: 'on-first-retry',
    // 模拟真实用户行为
    actionTimeout: 15_000,
    navigationTimeout: 30_000,
  },

  projects: [
    // 登录前置步骤（只执行一次）
    {
      name: 'auth-setup',
      testMatch: /login\.setup\.ts/,
      use: { storageState: undefined },
    },
    // 桌面浏览器测试
    {
      name: 'desktop-chrome',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1280, height: 800 },
      },
      dependencies: ['auth-setup'],
    },
    // 移动浏览器测试
    {
      name: 'mobile-chrome',
      use: {
        ...devices['Pixel 7'],
      },
      dependencies: ['auth-setup'],
    },
  ],
});
```

## Step 3: 实现登录前置步骤

### 3.1 登录 Setup

登录是 DV Test 的第一道关卡。**MUST** 模拟真人登录流程。

```typescript
// tests/dv/auth/login.setup.ts
import { test as setup, expect } from '@playwright/test';

const DV_USERNAME = process.env.DV_USERNAME || 'devtest';
const DV_PASSWORD = process.env.DV_PASSWORD || '';

setup('authenticate', async ({ page }) => {
  // 1. 导航到登录页
  await page.goto('/login');

  // 2. 等待登录表单可见
  await expect(page.locator('form')).toBeVisible();

  // 3. 模拟真人输入——逐字符输入，带随机延迟
  await page.getByLabel(/username|用户名/i).fill(DV_USERNAME);
  await page.getByLabel(/password|密码/i).fill(DV_PASSWORD);

  // 4. 点击登录按钮
  await page.getByRole('button', { name: /login|登录|sign in/i }).click();

  // 5. 等待登录成功——验证跳转到主页或出现用户标识
  await expect(page).not.toHaveURL(/login/);
  // 或者等待特定元素出现
  // await expect(page.getByTestId('user-avatar')).toBeVisible();

  // 6. 截图记录登录后状态
  await page.screenshot({ path: './screenshots/after-login.png' });

  // 7. 保存登录状态，后续测试复用
  await page.context().storageState({ path: './auth/storage-state.json' });
});
```

### 3.2 Token 方式登录（备选）

如果系统支持通过 owner private key 获取 token，可以编程方式登录后注入：

```typescript
// tests/dv/auth/login.setup.ts (token 方式)
import { test as setup } from '@playwright/test';
import { getSessionToken } from './token-helper';

setup('authenticate via token', async ({ page }) => {
  // 1. 通过 KRPC 获取 session token
  const token = await getSessionToken();

  // 2. 导航到应用并注入 token
  await page.goto('/');
  await page.evaluate((t) => {
    localStorage.setItem('session_token', t);
    // 或根据应用的认证机制设置 cookie
  }, token);

  // 3. 刷新页面，验证登录状态生效
  await page.reload();
  await page.waitForLoadState('networkidle');

  // 4. 保存状态
  await page.context().storageState({ path: './auth/storage-state.json' });
});
```

## Step 4: 编写核心测试——模拟真人操作

### 4.1 设计原则：像真人一样操作

DV Test 的核心是 **模拟真人用户的操作行为**，而非程序化调用接口。

**MUST 遵守的真人模拟原则：**

1. **通过 UI 元素交互，不直接调用 API。**
   - ✅ `page.getByRole('button', { name: 'Create' }).click()`
   - ❌ `fetch('/api/create', ...)`

2. **用可见文本和语义角色定位元素，不用 CSS 选择器。**
   - ✅ `page.getByRole('link', { name: 'Settings' })`
   - ✅ `page.getByText('No data available')`
   - ⚠️ `page.getByTestId('settings-link')` （可接受但非首选）
   - ❌ `page.locator('.nav > li:nth-child(3) > a')`

3. **等待可见信号，不用固定延时。**
   - ✅ `await expect(page.getByText('Created successfully')).toBeVisible()`
   - ❌ `await page.waitForTimeout(3000)`

4. **按用户视角验证结果，不检查内部状态。**
   - ✅ 验证页面上显示了新创建的项目名称
   - ❌ 直接查数据库确认记录存在

5. **操作之间留有自然间隔。**
   - 真人不会在 0ms 内完成表单填写和提交
   - 使用 `page.fill()` 已有合理速度，无需额外 sleep

### 4.2 元素选择器管理

集中管理选择器，便于维护：

```typescript
// tests/dv/helpers/selectors.ts

/**
 * 推荐优先级：
 * 1. getByRole — 语义角色（最佳，最接近用户感知）
 * 2. getByText — 可见文本
 * 3. getByLabel — 表单标签
 * 4. getByPlaceholder — 占位符文本
 * 5. getByTestId — data-testid（最后手段）
 */
export const selectors = {
  // 导航
  nav: {
    sidebar: (page) => page.getByRole('navigation'),
    menuItem: (page, name: string) => page.getByRole('link', { name }),
  },

  // 列表页
  list: {
    table: (page) => page.getByRole('table'),
    row: (page, name: string) => page.getByRole('row', { name }),
    createButton: (page) => page.getByRole('button', { name: /create|新建|添加/i }),
    searchInput: (page) => page.getByPlaceholder(/search|搜索/i),
    emptyState: (page) => page.getByText(/no data|暂无数据|empty/i),
    pagination: {
      next: (page) => page.getByRole('button', { name: /next|下一页/i }),
      prev: (page) => page.getByRole('button', { name: /prev|上一页/i }),
      pageInfo: (page) => page.getByText(/page|页/i),
    },
  },

  // 表单
  form: {
    field: (page, label: string) => page.getByLabel(label),
    submit: (page) => page.getByRole('button', { name: /submit|save|确定|保存/i }),
    cancel: (page) => page.getByRole('button', { name: /cancel|取消/i }),
  },

  // 反馈
  feedback: {
    success: (page) => page.getByText(/success|成功/i),
    error: (page) => page.getByText(/error|failed|失败|错误/i),
    loading: (page) => page.getByText(/loading|加载中/i),
    confirm: (page) => page.getByRole('dialog'),
  },
};
```

### 4.3 可复用操作序列

```typescript
// tests/dv/helpers/actions.ts
import { Page, expect } from '@playwright/test';

/**
 * 导航到指定页面并等待加载完成。
 * 模拟真人：点击侧边栏链接，而非直接 goto URL。
 */
export async function navigateViaMenu(page: Page, menuName: string) {
  await page.getByRole('link', { name: menuName }).click();
  // 等待页面内容加载（loading 消失或内容出现）
  await page.waitForLoadState('networkidle');
}

/**
 * 在列表页搜索。
 * 模拟真人：点击搜索框 → 输入关键词 → 等待结果刷新。
 */
export async function searchInList(page: Page, keyword: string) {
  const searchInput = page.getByPlaceholder(/search|搜索/i);
  await searchInput.click();
  await searchInput.fill(keyword);
  // 等待搜索结果更新（debounce 后）
  await page.waitForResponse(resp =>
    resp.url().includes('list') || resp.url().includes('search')
  ).catch(() => {
    // 如果没有网络请求（客户端过滤），等待 UI 更新
  });
  await page.waitForTimeout(500); // 等待 debounce
}

/**
 * 填写表单并提交。
 * 模拟真人：逐个填写字段 → 点击提交 → 等待反馈。
 */
export async function fillAndSubmitForm(
  page: Page,
  fields: Record<string, string>,
) {
  for (const [label, value] of Object.entries(fields)) {
    const field = page.getByLabel(new RegExp(label, 'i'));
    await field.click();
    await field.fill(value);
  }
  await page.getByRole('button', { name: /submit|save|确定|保存/i }).click();
}

/**
 * 确认危险操作对话框。
 * 模拟真人：看到确认弹窗 → 阅读内容 → 点击确认。
 */
export async function confirmDialog(page: Page) {
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: /confirm|ok|确定|确认/i }).click();
  await expect(dialog).not.toBeVisible();
}

/**
 * 等待操作反馈（成功/失败提示）。
 */
export async function waitForFeedback(
  page: Page,
  type: 'success' | 'error',
): Promise<string> {
  const pattern = type === 'success'
    ? /success|successfully|成功/i
    : /error|failed|失败|错误/i;
  const toast = page.getByText(pattern).first();
  await expect(toast).toBeVisible({ timeout: 15_000 });
  const text = await toast.textContent();
  return text || '';
}
```

### 4.4 等待策略

```typescript
// tests/dv/helpers/wait-strategies.ts
import { Page, expect } from '@playwright/test';

/**
 * 等待真实数据加载完成。
 * 真实后端比 mock 慢，需要更宽松的等待策略。
 */
export async function waitForDataLoad(page: Page) {
  // 策略 1：等待 loading 状态消失
  const loadingIndicator = page.getByText(/loading|加载中/i);
  if (await loadingIndicator.isVisible().catch(() => false)) {
    await expect(loadingIndicator).not.toBeVisible({ timeout: 30_000 });
  }

  // 策略 2：等待网络空闲
  await page.waitForLoadState('networkidle');
}

/**
 * 等待列表渲染完成（至少出现一行数据，或显示空态）。
 */
export async function waitForListReady(page: Page) {
  await expect(
    page.getByRole('row').or(page.getByText(/no data|暂无/i))
  ).toBeVisible({ timeout: 15_000 });
}

/**
 * 等待异步操作完成（如任务执行、文件处理）。
 * 模拟真人：定期刷新页面查看进度。
 */
export async function waitForAsyncOperation(
  page: Page,
  completionIndicator: string | RegExp,
  options: { timeoutMs?: number; pollIntervalMs?: number } = {},
) {
  const { timeoutMs = 120_000, pollIntervalMs = 3_000 } = options;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const indicator = page.getByText(completionIndicator);
    if (await indicator.isVisible().catch(() => false)) {
      return;
    }
    // 模拟真人刷新
    await page.reload();
    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(pollIntervalMs);
  }

  throw new Error(
    `Async operation did not complete within ${timeoutMs}ms. ` +
    `Expected to see: ${completionIndicator}`
  );
}
```

## Step 5: 编写测试用例

### 5.1 主用户流程（MUST）

这是最关键的测试——模拟真人完整走一遍 PRD 中的主流程。

```typescript
// tests/dv/flows/main-user-flow.spec.ts
import { test, expect } from '@playwright/test';
import { navigateViaMenu, fillAndSubmitForm, waitForFeedback } from '../helpers/actions';
import { waitForDataLoad, waitForListReady } from '../helpers/wait-strategies';

test.describe('Main User Flow (Happy Path)', () => {

  test('complete primary user journey', async ({ page }) => {
    // ── Step 1: 进入功能页面 ─────────────────────
    // 模拟真人：从首页通过导航进入目标功能
    await page.goto('/');
    await waitForDataLoad(page);
    await page.screenshot({ path: './screenshots/01-home.png' });

    await navigateViaMenu(page, 'Target Feature');  // 替换为实际菜单名
    await waitForDataLoad(page);
    await page.screenshot({ path: './screenshots/02-feature-page.png' });

    // 验证：页面正确加载，标题可见
    await expect(page.getByRole('heading')).toBeVisible();

    // ── Step 2: 查看列表 ──────────────────────────
    // 模拟真人：到达列表页，浏览现有数据
    await waitForListReady(page);
    await page.screenshot({ path: './screenshots/03-list-view.png' });

    // ── Step 3: 创建新项目 ────────────────────────
    // 模拟真人：点击"新建"按钮 → 填写表单 → 提交
    await page.getByRole('button', { name: /create|新建/i }).click();
    await expect(page.getByRole('dialog').or(page.getByRole('form'))).toBeVisible();
    await page.screenshot({ path: './screenshots/04-create-form.png' });

    const testItemName = `DV-Test-${Date.now()}`;
    await fillAndSubmitForm(page, {
      'Name': testItemName,
      // 根据实际表单字段添加更多
    });

    // 验证：看到成功提示
    await waitForFeedback(page, 'success');
    await page.screenshot({ path: './screenshots/05-create-success.png' });

    // ── Step 4: 验证新项目出现在列表中 ───────────
    // 模拟真人：回到列表，查找刚创建的项目
    await waitForListReady(page);
    await expect(page.getByText(testItemName)).toBeVisible();
    await page.screenshot({ path: './screenshots/06-list-with-new-item.png' });

    // ── Step 5: 查看详情 ──────────────────────────
    // 模拟真人：点击项目名进入详情
    await page.getByText(testItemName).click();
    await waitForDataLoad(page);
    await page.screenshot({ path: './screenshots/07-detail-view.png' });

    // 验证：详情页显示正确数据
    await expect(page.getByText(testItemName)).toBeVisible();

    // ── Step 6: 编辑 ─────────────────────────────
    // 模拟真人：点击编辑 → 修改字段 → 保存
    await page.getByRole('button', { name: /edit|编辑/i }).click();
    const updatedName = `${testItemName}-Updated`;
    const nameField = page.getByLabel(/name|名称/i);
    await nameField.clear();
    await nameField.fill(updatedName);
    await page.getByRole('button', { name: /save|保存/i }).click();
    await waitForFeedback(page, 'success');
    await page.screenshot({ path: './screenshots/08-edit-success.png' });

    // 验证：页面显示更新后的名称
    await expect(page.getByText(updatedName)).toBeVisible();

    // ── Step 7: 删除 ─────────────────────────────
    // 模拟真人：点击删除 → 确认对话框 → 确认
    await page.getByRole('button', { name: /delete|删除/i }).click();

    // 确认对话框
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await page.screenshot({ path: './screenshots/09-delete-confirm.png' });
    await dialog.getByRole('button', { name: /confirm|ok|确定/i }).click();

    // 验证：成功提示 + 项目从列表消失
    await waitForFeedback(page, 'success');
    await waitForListReady(page);
    await expect(page.getByText(updatedName)).not.toBeVisible();
    await page.screenshot({ path: './screenshots/10-after-delete.png' });
  });
});
```

### 5.2 CRUD 操作验证（MUST）

```typescript
// tests/dv/flows/crud-operations.spec.ts
import { test, expect } from '@playwright/test';

test.describe('CRUD Operations with Real Backend', () => {
  // 每个测试用唯一标识，避免数据冲突
  const uniqueId = () => `dv-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;

  test('Create: form validation prevents invalid submission', async ({ page }) => {
    await page.goto('/feature');  // 替换为实际路径

    // 点击新建
    await page.getByRole('button', { name: /create|新建/i }).click();

    // 不填任何字段直接提交
    await page.getByRole('button', { name: /submit|save|确定/i }).click();

    // 验证：表单验证提示出现，不会提交到后端
    await expect(
      page.getByText(/required|必填|不能为空/i)
    ).toBeVisible();

    await page.screenshot({ path: './screenshots/crud-validation.png' });
  });

  test('Read: list displays real data from backend', async ({ page }) => {
    await page.goto('/feature');

    // 等待真实数据加载（非 mock）
    await page.waitForLoadState('networkidle');

    // 验证：页面无 console error
    const consoleErrors: string[] = [];
    page.on('console', msg => {
      if (msg.type() === 'error') consoleErrors.push(msg.text());
    });

    await page.waitForTimeout(2000); // 等待可能的延迟错误

    // 验证数据区域可见（表格/列表/卡片）
    const hasData = await page.getByRole('table')
      .or(page.getByRole('list'))
      .or(page.getByText(/no data|暂无/i))
      .isVisible();
    expect(hasData).toBeTruthy();

    // 检查 console error
    expect(consoleErrors, `Console errors found: ${consoleErrors.join('; ')}`).toHaveLength(0);

    await page.screenshot({ path: './screenshots/crud-read.png' });
  });

  test('Read: detail page renders complete information', async ({ page }) => {
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 点击第一条数据进入详情（如果有数据）
    const firstRow = page.getByRole('row').nth(1); // 跳过表头
    if (await firstRow.isVisible().catch(() => false)) {
      await firstRow.click();
      await page.waitForLoadState('networkidle');

      // 验证详情页有内容，不是空白
      const bodyText = await page.locator('main').textContent();
      expect(bodyText?.trim().length).toBeGreaterThan(0);

      await page.screenshot({ path: './screenshots/crud-detail.png' });
    }
  });

  test('Update: changes persist after page refresh', async ({ page }) => {
    // 前提：已有可编辑的数据
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 进入第一条数据的编辑
    const firstItem = page.getByRole('row').nth(1);
    if (await firstItem.isVisible().catch(() => false)) {
      await firstItem.click();
      await page.waitForLoadState('networkidle');

      await page.getByRole('button', { name: /edit|编辑/i }).click();

      // 修改一个字段
      const editableField = page.getByLabel(/name|名称|description|描述/i).first();
      const originalValue = await editableField.inputValue();
      const newValue = `${originalValue}-edited-${uniqueId()}`;
      await editableField.clear();
      await editableField.fill(newValue);
      await page.getByRole('button', { name: /save|保存/i }).click();

      await expect(page.getByText(/success|成功/i)).toBeVisible({ timeout: 10_000 });

      // 刷新页面验证持久化
      await page.reload();
      await page.waitForLoadState('networkidle');
      await expect(page.getByText(newValue)).toBeVisible();

      await page.screenshot({ path: './screenshots/crud-update-persist.png' });

      // 恢复原值（清理）
      await page.getByRole('button', { name: /edit|编辑/i }).click();
      const field = page.getByLabel(/name|名称|description|描述/i).first();
      await field.clear();
      await field.fill(originalValue);
      await page.getByRole('button', { name: /save|保存/i }).click();
    }
  });
});
```

### 5.3 错误处理验证（MUST）

```typescript
// tests/dv/flows/error-handling.spec.ts
import { test, expect } from '@playwright/test';

test.describe('Error Handling with Real Backend', () => {

  test('network error: UI shows error state, not blank page', async ({ page }) => {
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 模拟网络中断：拦截后续请求
    await page.route('**/kapi/**', route => route.abort('connectionrefused'));

    // 触发一个需要网络的操作（如翻页、搜索）
    const nextButton = page.getByRole('button', { name: /next|下一页|refresh|刷新/i });
    if (await nextButton.isVisible().catch(() => false)) {
      await nextButton.click();
    } else {
      await page.reload();
    }

    // 验证：显示错误提示，不是空白页或崩溃
    await expect(
      page.getByText(/error|failed|网络|失败|retry|重试/i)
    ).toBeVisible({ timeout: 15_000 });

    await page.screenshot({ path: './screenshots/error-network.png' });

    // 恢复网络
    await page.unroute('**/kapi/**');
  });

  test('session expired: UI redirects to login or shows re-auth prompt', async ({ page }) => {
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 清除 session 模拟过期
    await page.evaluate(() => {
      localStorage.removeItem('session_token');
      document.cookie.split(';').forEach(c => {
        document.cookie = c.trim().split('=')[0] + '=;expires=Thu, 01 Jan 1970 00:00:00 GMT;path=/';
      });
    });

    // 触发需要认证的操作
    await page.reload();

    // 验证：跳转到登录页或显示重新认证提示
    await expect(
      page.getByText(/login|sign in|登录|session.*expired|会话.*过期/i)
        .or(page.locator('input[type="password"]'))
    ).toBeVisible({ timeout: 15_000 });

    await page.screenshot({ path: './screenshots/error-session-expired.png' });
  });

  test('no console errors during normal operation', async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on('console', msg => {
      if (msg.type() === 'error') {
        // 过滤已知的无害错误（如 favicon 404）
        const text = msg.text();
        if (!text.includes('favicon')) {
          consoleErrors.push(text);
        }
      }
    });

    // 浏览主要页面
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    // 导航到目标功能页
    await page.getByRole('link', { name: /target feature/i }).click();
    await page.waitForLoadState('networkidle');

    // 等待充分的时间让异步错误出现
    await page.waitForTimeout(3000);

    // 验证无 console error
    expect(
      consoleErrors,
      `Console errors:\n${consoleErrors.join('\n')}`
    ).toHaveLength(0);
  });

  test('failed API requests show user-friendly error, not raw JSON', async ({ page }) => {
    const networkErrors: string[] = [];
    page.on('response', resp => {
      if (resp.status() >= 400) {
        networkErrors.push(`${resp.status()} ${resp.url()}`);
      }
    });

    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 如果有失败的请求，验证 UI 展示了友好错误信息
    if (networkErrors.length > 0) {
      // 页面不应显示原始 JSON 或堆栈信息
      const bodyText = await page.locator('body').textContent() || '';
      expect(bodyText).not.toMatch(/"error":\s*\{/);      // 不应有原始 JSON
      expect(bodyText).not.toMatch(/at\s+\w+\s+\(/);       // 不应有堆栈 trace
      expect(bodyText).not.toMatch(/undefined|null|NaN/);   // 不应有原始 JS 值
    }
  });
});
```

### 5.4 边界场景验证（SHOULD）

```typescript
// tests/dv/flows/edge-cases.spec.ts
import { test, expect } from '@playwright/test';

test.describe('Edge Cases with Real Backend', () => {

  test('empty state: new system with no data', async ({ page }) => {
    // 此测试在全新系统上运行时验证空态
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 如果无数据，应该显示空态提示而非空白
    const hasData = await page.getByRole('row').nth(1).isVisible().catch(() => false);
    if (!hasData) {
      await expect(
        page.getByText(/no data|empty|暂无|没有数据/i)
      ).toBeVisible();
      await page.screenshot({ path: './screenshots/edge-empty-state.png' });
    }
  });

  test('pagination: navigate through multiple pages', async ({ page }) => {
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 检查是否有分页控件
    const nextButton = page.getByRole('button', { name: /next|下一页/i });
    if (await nextButton.isEnabled().catch(() => false)) {
      // 记录第一页内容
      const firstPageText = await page.locator('main').textContent();

      // 翻到下一页
      await nextButton.click();
      await page.waitForLoadState('networkidle');

      // 验证内容发生了变化
      const secondPageText = await page.locator('main').textContent();
      expect(secondPageText).not.toEqual(firstPageText);

      await page.screenshot({ path: './screenshots/edge-pagination.png' });

      // 翻回上一页
      await page.getByRole('button', { name: /prev|上一页/i }).click();
      await page.waitForLoadState('networkidle');
    }
  });

  test('responsive: mobile viewport renders correctly', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 }); // iPhone 尺寸
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 验证无水平滚动
    const scrollWidth = await page.evaluate(() => document.documentElement.scrollWidth);
    const clientWidth = await page.evaluate(() => document.documentElement.clientWidth);
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 5); // 5px 容差

    // 验证内容可见
    await expect(page.locator('main')).toBeVisible();

    await page.screenshot({ path: './screenshots/edge-mobile.png', fullPage: true });
  });

  test('i18n: language switch reflects in all UI text', async ({ page }) => {
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    // 记录当前语言的页面文本
    const originalText = await page.locator('main').textContent();

    // 查找语言切换器
    const langSwitcher = page.getByRole('button', { name: /language|lang|语言/i })
      .or(page.getByRole('combobox', { name: /language|lang|语言/i }));

    if (await langSwitcher.isVisible().catch(() => false)) {
      await langSwitcher.click();
      // 切换到另一种语言
      await page.getByText(/中文|English/i).click();
      await page.waitForLoadState('networkidle');

      const switchedText = await page.locator('main').textContent();
      // 验证文本发生了变化（说明 i18n 生效）
      expect(switchedText).not.toEqual(originalText);

      await page.screenshot({ path: './screenshots/edge-i18n-switch.png' });
    }
  });

  test('browser back/forward navigation works correctly', async ({ page }) => {
    // 模拟真人：进入列表 → 点击详情 → 按浏览器返回
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    const firstItem = page.getByRole('row').nth(1);
    if (await firstItem.isVisible().catch(() => false)) {
      await firstItem.click();
      await page.waitForLoadState('networkidle');
      const detailUrl = page.url();

      // 按浏览器返回
      await page.goBack();
      await page.waitForLoadState('networkidle');

      // 验证回到列表页
      await expect(page).not.toHaveURL(detailUrl);

      // 按浏览器前进
      await page.goForward();
      await page.waitForLoadState('networkidle');

      // 验证回到详情页
      await expect(page).toHaveURL(detailUrl);

      await page.screenshot({ path: './screenshots/edge-back-forward.png' });
    }
  });

  test('rapid repeated clicks do not cause duplicate submissions', async ({ page }) => {
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');

    await page.getByRole('button', { name: /create|新建/i }).click();

    // 填写表单
    const uniqueName = `rapid-click-${Date.now()}`;
    await page.getByLabel(/name|名称/i).fill(uniqueName);

    // 快速多次点击提交
    const submitBtn = page.getByRole('button', { name: /submit|save|确定/i });
    await submitBtn.click();
    await submitBtn.click();
    await submitBtn.click();

    await page.waitForLoadState('networkidle');
    await page.waitForTimeout(2000);

    // 验证只创建了一条记录
    await page.goto('/feature');
    await page.waitForLoadState('networkidle');
    const matchingItems = page.getByText(uniqueName);
    const count = await matchingItems.count();
    expect(count).toBeLessThanOrEqual(1);

    await page.screenshot({ path: './screenshots/edge-rapid-clicks.png' });

    // 清理
    if (count === 1) {
      await matchingItems.first().click();
      await page.waitForLoadState('networkidle');
      const delBtn = page.getByRole('button', { name: /delete|删除/i });
      if (await delBtn.isVisible().catch(() => false)) {
        await delBtn.click();
        const dialog = page.getByRole('dialog');
        if (await dialog.isVisible().catch(() => false)) {
          await dialog.getByRole('button', { name: /confirm|ok|确定/i }).click();
        }
      }
    }
  });
});
```

## Step 6: 执行与报告

### 6.1 执行测试

```bash
# 设置环境变量
export DV_BASE_URL="http://your-real-system-url"
export DV_USERNAME="devtest"
export DV_PASSWORD="your-password"

# 安装浏览器
npx playwright install chromium

# 执行 DV Test
npx playwright test --config=tests/dv/playwright.config.ts

# 查看报告
npx playwright show-report
```

### 6.2 截图审查

每个测试步骤都产出截图到 `screenshots/` 目录。测试完成后 **MUST** 审查所有截图：

- 页面布局是否正常（无错位、无溢出）。
- 数据是否真实显示（非 mock 数据残留）。
- 状态反馈是否正确（成功/错误提示）。
- 移动端是否可用。

### 6.3 Console Error 审查

测试期间捕获的所有 console error **MUST** 逐一检查：

- **关键错误**（接口失败、渲染异常）→ MUST 修复。
- **警告**（废弃 API、性能提示）→ 记录，后续处理。
- **无害错误**（favicon 404）→ 标记为已知。

---

# Common Failure Modes

## 1. 登录失败导致全部测试跳过

**症状**: auth-setup 失败，所有依赖它的测试被跳过。
**原因**: 用户凭证错误、verify-hub 未启动、登录页 UI 变更导致选择器失效。
**修复**: 先手动在浏览器中确认登录流程正常；更新选择器；检查环境变量。

## 2. 选择器脆弱——CSS 选择器绑定 DOM 结构

**症状**: UI 微调后大量测试失败。
**原因**: 使用了 `.nav > li:nth-child(3)` 之类的结构化选择器。
**修复**: 优先使用 `getByRole`、`getByText`、`getByLabel`，最后手段用 `getByTestId`。

## 3. 测试间数据依赖

**症状**: 单独运行测试通过，全部运行时部分失败。
**原因**: 测试 A 创建的数据被测试 B 依赖，但执行顺序不确定。
**修复**: 每个测试自己创建和清理数据；使用唯一标识避免冲突。

## 4. 固定延时导致测试慢且不稳定

**症状**: 测试用 `waitForTimeout(5000)` 等待，有时超时有时过早。
**原因**: 真实后端响应时间不确定，固定延时无法适应。
**修复**: 使用 `waitForSelector`、`waitForResponse`、`expect().toBeVisible()` 等条件等待。

## 5. Mock 数据残留

**症状**: 页面显示的是 mock 数据而非真实后端数据。
**原因**: 环境变量 `VITE_USE_MOCK=true` 未关闭，或代码中硬编码了 mock。
**修复**: 确认构建时 mock 开关已关闭；检查网络面板确认请求走了真实后端。

## 6. 测试污染生产数据

**症状**: 测试创建的 "DV-Test-xxx" 数据残留在系统中。
**原因**: 测试失败中途退出，清理逻辑未执行。
**修复**: 使用 `test.afterEach` 或 `test.afterAll` 确保清理；测试数据使用可识别的前缀（如 `dv-`）。

## 7. 真实后端慢导致大量超时

**症状**: 大部分测试因 timeout 失败。
**原因**: 默认超时对真实后端太短（Prototype 阶段用 mock 很快，DV 阶段后端慢）。
**修复**: DV 配置中设置更宽松的超时（测试 60s、断言 10s、导航 30s）。

## 8. 截图未审查

**症状**: 测试全"通过"但 UI 实际有明显视觉问题。
**原因**: 只检查了断言结果，未审查截图。Playwright 断言无法捕获所有视觉问题。
**修复**: 测试完成后 **MUST** 逐一审查截图，将截图审查纳入 Pass Criteria。

---

# AI 行为规则

1. **测试脚本编写自主完成。** 根据 PRD 和 UI 代码编写测试——不需要问人。
2. **选择器适配自主完成。** 根据实际 UI 调整选择器——不需要问人。
3. **截图审查自主完成。** 对照 PRD 判断截图中的 UI 是否正确——不需要问人。
4. **发现后端 bug 必须报告人。** 如果 UI 正确但后端返回错误数据，报告但不修后端。
5. **发现 UI bug 可自主修复。** 如果是 UI 层问题（布局、文案、状态显示），可以直接修复并重新测试。
6. **测试数据清理。** 每次测试后清理创建的数据，不要污染系统。
7. **不修改后端代码。** 只在 UI 和测试脚本范围内修改。

---

# Pass Criteria

本 Skill 全部完成的标志：

- [ ] 登录流程在真实系统上成功（Playwright 自动登录通过）。
- [ ] 主用户流程（Happy Path）在真实后端下完整走通。
- [ ] CRUD 操作验证通过（创建、读取、更新、删除）。
- [ ] 数据展示与预期一致（真实数据正确渲染，非 mock 残留）。
- [ ] 错误处理验证通过（网络错误、会话过期展示友好提示）。
- [ ] 无关键 console error（接口失败、渲染异常等）。
- [ ] 移动端视口（375px）可用——无水平滚动，内容可见。
- [ ] 所有测试步骤截图已审查，无明显视觉问题。
- [ ] 测试可重复执行（不因残留数据失败）。
- [ ] 测试数据已清理（无 `dv-*` 前缀的残留记录）。
- [ ] Playwright 测试套件通过（桌面 + 移动两个 project）。
