# Monitor Validation Plan

## Purpose

- 为 `control_panel` 第一阶段验证层提供 `monitor` 范围的执行清单。
- 对齐 Harness 的 Mock-first 要求：`pnpm run dev` 必须可独立运行，并能验证主流程。
- 当前范围只覆盖 desktop 内的 `monitor` 能力，不扩展到 `network`、`containers`、`storage` 等窗口。

## Quick Start

```bash
cd src/frame/control_panel/web
pnpm install
pnpm dev:mock
```

- 访问 `http://127.0.0.1:4020/`
- mock 模式下不依赖 `127.0.0.1:3180`
- 认证会自动进入 mock 会话

## Current Scope

### In Scope

- desktop 首页启动
- desktop 中 monitor 窗口的打开与渲染
- layout / overview / system metrics / system status / network overview / log peek 的 mock 数据驱动
- mock fallback 文案与错误容忍逻辑

### Out Of Scope

- 真实后端联调
- container / zone / gateway / AI Models 的深度验证
- benchmark 与 DV Test

## UI DataModel

### DesktopShellState

- `layout: RootLayoutData`
- `profile: UserProfile`
- `systemStatus: SystemStatus`
- `windows: DesktopWindow[]`

### MonitorState

- `overview: SystemOverview | null`
- `metrics: SystemMetrics`
- `status: SystemStatusResponse`
- `networkOverview: NetworkOverview | null`
- `logPeek: SystemLogEntry[] | null`
- `layoutError: string | null`
- `overviewError: string | null`
- `networkError: string | null`
- `logPeekError: string | null`

### Required UI States

- `loading`
- `ready`
- `fallback-ready`
- `empty-log`
- `degraded-warning`

## Mock Fixtures

当前 monitor mock fixture 由 `src/frame/control_panel/web/src/api/index.ts` 中的 mock payload 提供，并在 `VITE_CP_USE_MOCK=1` 时直接返回。

建议后续拆分为：

- `happy`
- `warning`
- `empty-log`
- `high-load`

## Main Flows

1. 打开 `/`
2. 自动进入 mock 会话
3. desktop 渲染成功
4. 打开 `System Monitor` 窗口
5. 看见 CPU / memory / disk / network 卡片
6. 看见 trend 图表
7. 看见 log preview
8. 刷新页面后仍可低成本重进

## Done For This Stage

- `pnpm dev:mock` 可直接启动
- monitor 数据不依赖真实后端
- desktop + monitor 主流程能人工稳定复现
- 文档说明足够简洁，团队成员无需翻代码猜启动方式

## Follow-up

- 增加 monitor mock fixture 切换入口
- 为 monitor 增加 Playwright smoke
- 为 metrics / logs 增加 benchmark 输入数据梯度
