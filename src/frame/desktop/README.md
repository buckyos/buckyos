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

## 桌面系统信息卡片

系统信息卡片默认展示。新用户的桌面种子直接包含它；已有用户首次打开新版 Desktop 时，若尚无卡片则自动补齐一次，保留便笺、快捷方式和保存的位置。已经手动添加或重命名的卡片按内容识别，不重复创建。

拖动标题可调整位置，右键菜单可移除。主动移除后刷新或换设备不会重新添加；可在 **Settings → Appearance → Desktop → Add system info to desktop**（中文：添加系统信息到桌面）恢复显示，“恢复默认”也会恢复卡片。

电脑端首次添加时，系统信息接在左侧已有小组件下方；默认排列为时钟 → Getting started → 系统信息，保留原便笺的内容、尺寸和位置。若下方空间已被占用或屏幕高度不足，则在后续页面的左侧空位放置；已有卡片继续使用保存的位置。手机端沿用原有网格分页。

卡片显示系统版本、构建号、当前登录用户 ID、账号类型、Zone 名称和域名；详情包含 Zone DID、发布渠道和平台。身份来自当前 SDK 登录会话，版本来自 `system.buckyos_info.get`，Zone 来自 `zone.presentation.get`，不把 Zone 所有者当作当前用户，也不使用前端编译版本冒充运行系统版本。每分钟、页面重新可见及身份资料变更时刷新，也可手动刷新。读取失败的部分显示暂不可用，复制结果中对应部分为 `null`；复制只包含选定的诊断字段和采集时间，不包含会话令牌。

添加操作通过现有 Desktop 文件接口保存 `{ "widget": "system-info" }` 描述文件到当前用户的 `/home/<user_id>/Desktop/`，位置沿用本设备的桌面布局保存机制。描述文件不保存用户或 Zone 身份。实现入口为 [SystemInfoWidget.tsx](src/desktop/widgets/SystemInfoWidget.tsx)。这是 Desktop 内置小组件，需要更新 Desktop Web 产物，不是独立 App 安装包。

自动初始化通过现有 Desktop 状态接口在 `users/<user_id>/desktop/default/system-info-widget` 保存 `{ "initialized": true }`。确认描述文件存在后才写入该标记；读取或创建失败时保留已有桌面，下次加载重试。自动补齐使用固定文件名 `System Info.widget`（同名其它文件存在则编号），以避免不同语言的浏览器同时升级产生不同文件名；卡片标题仍按界面语言显示。移除卡片前也确认初始化标记已保存，防止之后再次补齐。该标记跟随账号，不以浏览器本地缓存作为依据。

浏览器回归用例位于 [system-info-widget.spec.ts](tests/e2e/pages/system-info-widget.spec.ts)，覆盖添加、重载、移除、同名文件、写入失败、多用户、部分服务失败、复制字段、中英文及窄屏/深色布局。它使用 Mock 数据；真实 Zone 试用时仍需核对管理员和普通用户的读取权限、版本与 Settings → General 一致、刷新后描述文件保留及另一台 Zone 显示自身信息。

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

Files 的对象上传与 NFSP 实体提取可以用独立真实 NFS 服务验证。在 `buckyos/src` 构建包含 NamedStore 的 `cargo build -p nfs_server` 后，从本目录运行：

```bash
FB_NFSP_AUTO=1 pnpm exec playwright test tests/e2e/pages/filebrowser.nfsp.spec.ts
```

夹具自动启动端口 `3262` 的 NFS 和端口 `4178` 的 Vite，创建临时 export/NamedStore，退出时清理；构建目录不在默认 `buckyos/src/target` 时设置 `CARGO_TARGET_DIR`。测试通过真实 HTTP 验证单文件、重复内容不再上传数据、嵌套目录、同名叶子、空文件及落盘字节回读，并确保浏览器未调用旧 NFSP 上传接口。该模式仅模拟 Desktop 外壳，不依赖已安装 Zone；SSO、Gateway 和容器授权需另做 Zone DV。

Canvas 探索性原型已删除；保留的 [Canvas E2E](../../../doc/ai-canvas/e2e/README.md) 是历史规范及用户目标材料，当前工作区 app 为 AI Workspace。
