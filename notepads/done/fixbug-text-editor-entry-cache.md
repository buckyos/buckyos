# Text Editor 直接打开加载旧 JS

## 根因

2026-10-23，DV 的 Text Editor 从桌面图标直接打开时请求
`/assets/index-Cm1dKCh0.js` 并得到 404，而 File Browser 双击文件正常。
当时网关实际返回的 `index.html` 已引用 `/assets/index-Dc5CbPc2.js`，该资源返回 200；
`Cm1dKCh0` 可在本机旧 Text Editor 包中找到。

`WebAppFramePanel.tsx` 的普通启动访问 `/`，文件启动通过 `expandOpenPath` 访问
`/?src=...`。两者是不同的 HTTP 缓存键；`#bfp=...` 不发送给服务器，也不能刷新缓存。
因此浏览器沿用根 URL 的旧 HTML 时，会请求已经随包升级移除的资源，而文件打开仍可正常工作。

此前确实有相关修复：`buckyos-base` 的 `97ea629`（2026-10-22）禁止使用归档的
epoch mtime 生成 ETag / Last-Modified，避免等长新版 HTML 错误命中 304。
本次用旧 `If-None-Match: "18e-0-0"` 和 epoch `If-Modified-Since` 请求 DV 均返回新版
200，说明那项修复已经生效。但 HTML 响应没有 Cache-Control，未要求浏览器每次复用前验证。

## 修复

在同级仓库 `buckyos-base/src/buckyos-http-server/src/dir_server.rs`，按最终被服务的
文件 MIME，为 HTML 的 200、206、304 响应设置 `Cache-Control: no-cache`。
覆盖目录首页、显式 HTML 和 SPA fallback；保留有效的条件请求，JS/CSS 缓存行为不变。
无需 Text Editor 专用路由或随机查询参数。

浏览器已经保存并认为仍新鲜的旧 HTML 不会收到新响应头，需要一次强制刷新。
此修复防止拿到新响应后继续无验证地复用入口；它不会保留旧构建的 JS 供升级前已打开的页面使用。

## 验证

- `buckyos-base/src`：`cargo test -p buckyos-http-server dir_server -- --test-threads=1`，12 项通过。
- 测试覆盖 HTML 首页、显式路径、SPA fallback、200/206/304、旧 epoch 验证器，以及普通 JS 不受影响。
- Desktop 目录执行 `pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/text-editor.spec.ts --grep 'HTML entry points|the Text Editor icon'`，2 项通过（6.8 秒）。
  部署前缓存头断言失败；部署后 12 次普通/条件请求均得到当前 HTML 与 `no-cache`，引用的 JS 返回 200。
  同一 Chromium 上下文保留真实缓存，连续两次点 Text Editor 图标、直接访问 `/`、File Browser 双击临时 `notes.md` 均成功，无失败资源请求。
- 新增测试的 ESLint、独立严格 TypeScript 检查、`git diff --check` 均通过。
- 使用临时 Cargo 本地源码覆盖编译网关，未改动仓库依赖声明或锁文件；Linux musl release 构建通过。
  已替换 DV 与网关 rootfs 二进制，运行进程 SHA-256 与新产物一致。
  `uv run src/check.py` 显示 Running，核心服务与端口全部正常。

部署备份与运行哈希在工作区 `deployment-backups/20261023-text-editor-entry-cache/`。
重启时发现既有 TCP 栈未启用地址复用，旧连接 TIME_WAIT 导致绑定端口暂时失败；旧版也出现相同行为。
等待连接过期后新网关成功启动，未为此次缓存修复更改端口配置。

## 手册影响

已检查 `buckyos_admin`、`buckyos_app_dev`、`buckyos_dev_release`。
此次仅修正 HTTP 响应缓存策略，不更改 CLI、AppDoc、安装步骤或配置字段，手册命令和示例无需变更。
