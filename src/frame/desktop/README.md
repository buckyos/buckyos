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

## 开源软件清单

设置 → 通用 → 开源软件展示实际解析的依赖版本、许可证标识、项目来源及随包提供的许可原文。清单位于 `public/open-source/`，由 [`src/tools/generate_open_source.py`](../../tools/generate_open_source.py) 生成；不要手改生成文件。相同软件的不同版本分别列出。

`pnpm run build` 在更新 SDK 依赖后运行 `pnpm run licenses:generate`，刷新 Desktop 生产依赖（含传递依赖）与内置素材，其他模块保留仓库内快照并在“清单范围”中标注。完整系统构建在 Desktop 被本轮重建后再次汇总全部模块，并同步到 `dist/` 和 `rootfs/bin/control-panel/web/`。`--skip-web` 或只选择 Rust 模块不会刷新清单；这些开发构建保留上一次 Desktop 构建的清单。开源清单不进入 Desktop service worker 的预缓存，以便构建结束后的汇总可即时生效。

在本目录手动刷新完整清单：

```bash
uv run python ../../tools/generate_open_source.py
```

完整刷新需要 Rust/Cargo 及已准备的各 Web 模块、SDK 分发目录和扩展工具 payload。可加 `--offline` 使用 Cargo 本地缓存；`--fetch-metadata` 允许从 npm 官方 registry 或 Deno 上游补齐对应确切版本的许可证资料。未准备的模块会标为暂无资料，未声明许可证的条目不会猜测许可证。范围包括 Rust 服务依赖、多套 Web 界面和内置应用、SDK/CLI SBOM、可用扩展工具 payload 与素材，以及同级 `cyfs-gateway` 仓库的锁定依赖快照。网关独立构建，其条目始终标为快照，不代表已安装网关的版本核验；宿主操作系统、容器基础镜像及另行安装或运行时下载的组件未完整纳入。“清单范围”会显示收录状态，完整生成失败会使系统构建失败。

## E2E 入口

`pnpm run test:e2e` 仅使用默认配置、运行 `tests/e2e`。不同后台与环境需要明确选择配置：

| 配置 | 实际测试范围 |
| --- | --- |
| [playwright.config.ts](playwright.config.ts) | 默认 Desktop 页面；部分 spec 有独立夹具或真实环境跳过条件 |
| [playwright.aicc.config.ts](playwright.aicc.config.ts) | AI Center 模型与路由的 Mock UI |
| [playwright.aiworkspace.config.ts](playwright.aiworkspace.config.ts) | `tests/aiworkspace`：真实独立后台、静态身份、受控模型、离线生产构建 |
| [playwright.pdf.config.ts](playwright.pdf.config.ts) | [PDF 保真回归](../../../doc/testing/pdf-import-fidelity.md)：独立参考图、真实导入/后台、底图/图片/字符检查 |
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
