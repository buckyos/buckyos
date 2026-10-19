# BuckyOS Desktop

Desktop 与控制面板 WebUI 的 React / TypeScript / Vite 工程。应用入口在 `src/app/registry.tsx`，数据模型、服务适配器和 UI 在各 app 目录；构建产物安装到 BuckyOS 的控制面板 Web 目录。项目命令以 [package.json](package.json) 为准。

## 开发与检查

在本目录执行：

```bash
pnpm run dev
pnpm run check
pnpm run lint
pnpm run build
```

以上命令要求已准备依赖。Vite 开发端口为 5174；`.env.development` 默认启用 `VITE_CP_USE_MOCK=true`。可用 `/?scenario=normal` 观察 Mock 桌面，但这不证明真实系统身份或服务链路。

真实 Zone 开发代理由 [vite.config.ts](vite.config.ts) 的 `VITE_ZONE_PROXY`、`VITE_ZONE_HOST` 和 `VITE_ZONE_PROXY_IP` 控制，需关闭 Mock runtime。AI Workspace 的独立后台使用自己的 `AIWS_BACKEND` 与显式 dev override，详见 [AI Workspace README](src/app/aiworkspace/README.md)。HomeStation 的独立后台使用 `HS_BACKEND`。

## E2E 入口

`pnpm run test:e2e` 仅使用默认配置、运行 `tests/e2e`。不同后台与环境需要明确选择配置：

| 配置 | 实际测试范围 |
| --- | --- |
| [playwright.config.ts](playwright.config.ts) | 默认 Desktop 页面；部分 spec 有独立夹具或真实环境跳过条件 |
| [playwright.aicc.config.ts](playwright.aicc.config.ts) | AI Center 模型与路由的 Mock UI |
| [playwright.aiworkspace.config.ts](playwright.aiworkspace.config.ts) | `tests/aiworkspace`：真实独立后台、静态身份、受控模型、离线生产构建 |
| [playwright.aiworkspace.zone.config.ts](playwright.aiworkspace.zone.config.ts) | [AI Workspace Zone DV](../../../doc/testing/aiworkspace-zone.md)：真实登录、同 Zone 双用户、权限、持久化、幂等及手机视口 |
| [playwright.homestation.config.ts](playwright.homestation.config.ts) | HomeStation devnet 与 RSS 夹具 |
| [playwright.real.config.ts](playwright.real.config.ts) | `tests/e2e/real`：已运行 Zone 或代理服务，需各 spec 的显式开关与数据前置 |
| [playwright.dv.config.ts](playwright.dv.config.ts) | `tests/dv`：真实 Zone UI |

例如，在依赖和浏览器已准备好的环境中核对 AI Workspace 选例：

```bash
pnpm exec playwright test --config=playwright.aiworkspace.config.ts --list
```

去掉 `--list` 会构建或启动独立后台与 Web 服务，并执行浏览器测试；不是单纯的检查命令。真实模型质量、Zone 登录与 Gateway 权限需要另行验证。套件环境、CI 范围和证据要求见 [BuckyOS E2E 入口](../../../doc/testing/e2e.md)。

Canvas 探索性原型已删除；保留的 [Canvas E2E](../../../doc/ai-canvas/e2e/README.md) 是历史规范及用户目标材料，当前工作区 app 为 AI Workspace。
