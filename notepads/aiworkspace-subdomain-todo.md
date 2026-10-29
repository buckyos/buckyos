# AI Workspace 独立域名（T4-8 剩余）TODO

日期：2026-10-28

状态：2026-10-28 Q1–Q5 已裁决（见 §2），§3.1–3.3 已实施未提交（勾选项），§4 真实 Zone DV 待做。来源是《BuckyOS AiWorkspace 内核架构改进》T4-8「独立域名」（D21、D10）。

## 1. 已完成（不要重做）

- 网关：`rootfs/etc/boot_gateway.yaml` 把 `aiws-[a-z0-9]+.<zone>` 折到 aiworkspace 服务；scheduler 为服务登记 `aiws` 主机项。demo.buckyos.io 已部署生效。
- 服务：`frame/aiworkspace/server/src/source.rs` 在子域上处理 `/`、`/<workspace_name>`（都 307 到 `<zonehost>/workspace/<id>`）与 `/<workspace_name>/<name>`（发布名 → JSON/HTML 发布页；否则 `surface_by_name`：Surface 的 name、entity_id、标题 slug → 307 到 `?surface=<id>`）。`Limits.public_base = https://aiws-{workspace_id}.<zonehost>`，经 `ws.get_info.public_base` 给前端；子域里的 ID 去掉 `ws_` 前缀（DNS 标签不含 `_`）。
- Desktop（2026-10-28）：分享链接 `links.ts publicWorkspaceUrl` 生成 `https://aiws-<id>.<zone>/<pub_name(标题)||workspace>/<surface id>`；无 `public_base` 时回退 Zone 主机页。e2e UI-P10/P16 覆盖回退路径。

现状本质：子域只是重定向器，用户最终仍落在 `<zonehost>/workspace/<id>`，没有源隔离。

## 2. 裁决（2026-10-28）

- Q1 登录态：**(b) 子域自持**。`aiws-<id>.<zone>` 上的页面靠本主机的 host-only cookie（`/sso_callback` 落、`/sso_refresh` 续），凭据由 Zone 根经 `auth.prepare_app_launch` 委派（已登录者无感），或经 `<zone>/login?redirect_url=<aiws url>` 登录后回落。目标为 `AuthTarget::System(aiworkspace)`；所有 `aiws-*` 主机共用该目标（cookie 按主机隔离，token 不绑定工作区——服务按 grant 判权）。
- Q2 子域页面：**(b) 独立页**。Desktop 构建增加第二个入口 `aiws.html`（与桌面共享 `assets/`），不含桌面壳；由 aiworkspace 服务从 Desktop 构建目录直接出页与静态文件（网关已把子域全部非 `/kapi`、非 `/sso_*` 路径交给服务；发布链接与页面同形，只有服务能分辨）。
- Q3 路径：`/<工作区 slug>/<surface slug>?id=<surface id>[&block=<block id>]`。slug 只为可读（SEO），`id` 决定导向；无 `?id=` 时服务按名解析后 302 到带 `?id=` 的规范地址；`?id=` 存在时即使同名发布也出页面。工作区 slug 为空或撞网关保留前缀（`home`、`kapi`、`assets`、`aiworkspace`、`s3`、`bucket`、`ndm`、`nfs`、`sso_*`、`1.0`、`.well-known`）时用 `workspace`。
- Q4 `block`：`?block=` 透传（服务 302 时保留 query）。
- Q5 旧链接：**breaking change**。Zone 根的 `/workspace/<id>` 页面路由删除；`/workspace`（列表、手机入口）保留，从列表打开即跳子域。放映提示器链接 `/workspace/<id>/show/<show>` 不属分享链接，不动。

## 3. 实施项

### 3.1 Rust

- [x] buckyos-api `app_availability.rs`：`SYSTEM_LOGIN_TARGETS` 加 `aiworkspace`；新增 `is_system_delegation_target()`（只 `aiworkspace`）。
- [x] verify-hub `handle_delegate_user_session`：目标允许 `App` 或 `System(可委派服务)`；External 仍拒。补测试。
- [x] control-panel `parse_sso_redirect_url`：`aiws-[a-z0-9]+` 折到 `aiws`（与网关一致）；`prepare_app_launch` 参数 `app_instance_id` 可省、新增 `service_id`，解析目标须等于请求的 App 或 System(service)；补测试。scheduler `validate_beta22_app_state` 保留名加 `aiws`。
- [x] aiworkspace 服务：`--web <dir>`（standalone）/ Zone 模式 `exe 旁 web/` 否则 `../control-panel/web/`；子域路由 `/`、`/<ws>`、`/<ws>/<name>` 按 Q3/Q4 出页或 302；静态 `/assets/*`、`/aiworkspace/*`、`/favicon.svg`、`/icons.svg`、`/manifest.json`、`/sw.js`；删 `desktop_url`；`entry.describe`/`ws.join_entry` 的 `path` 改为子域绝对 URL（`EntryResource.path` 契约注释改“资源页 URL，可为绝对”）；`server/tests` 补主机路由测试、改 `sso.rs` 断言。

### 3.2 Desktop

- [x] `aiws.html` + `src/aiws.tsx`：initBuckyOS、身份可选、Theme+I18n、注册 SW；`app/aiworkspace/standalone/StandalonePage.tsx` 由 host 解出工作区、`?id/&block` 为目标，TabHost：close→Zone 根、signOut→本主机 `/sso_logout` 后到 Zone 根登录页。
- [x] `links.ts`：`zoneOrigin()`、`workspacePageUrl(publicBase, ws{id,title}, surface{id,title}|null, block?)`、`loginUrl/entryUrl` 指向 Zone 根、`targetOf` 读 `id/block`、保留 `WORKSPACE_ROUTE='/workspace'`（列表）。`launch.ts`：`openWorkspacePage(url, newTab?)`——Zone 根已登录走 `prepare_app_launch{service_id:'aiworkspace'}` 拿 callback 再导航，mock/dev-override/aiws 主机上直接导航。
- [x] `AIWorkspaceRoute` 只保留列表（`/workspace`），选中/新建 → `ws.get_info.public_base` → `openWorkspacePage`；App.tsx 删 `/workspace/:id`；`publicRoutes` 删对应模式；SW 模板按主机选入口（`aiws-*` 任何导航 → `aiws.html`）。
- [x] `MainMenu` 新标签、`ShareDialog` 链接、`ShareEntries` entry 链接、`AIWorkspaceAppPanel.loginUrl`、`privacy.ts` 公开列表链接、`sourceLinks.parseLink`（带 `id` 的不是发布）、mock entry path、`EntryPage.enter`（绝对 path 且已登录 → 委派后导航）。
- [x] `LoginPage`：`redirect_url` 指向 `aiws-*` 且已登录 → 自动委派并跳 callback。
- [x] vite：`build.rollupOptions.input` 双入口；dev/preview 中间件：`aiws-*` 主机的 HTML 导航 → `/aiws.html`。
- [x] e2e：后端加 `--public-base http://aiws-{workspace_id}.localhost:<web port>`；ui/mobile/entries/offline spec 与 DV fixtures 改走子域；sso-entry mock 测试改。

### 3.3 文档

- [x] 《内核架构改进》T4-8、D21 行；`frame/desktop/src/app/aiworkspace/README.md`、`frame/aiworkspace/README.md`；`doc/arch/verify-hub/混合SSO技术契约.md` §8.3 `path`；`doc/sdk/SSO.md`；`cmd_skills/aiworkspace_template_dev.md:51`；websdk `hybrid_sso.ts` 注释与 fixture。

## 4. 验收

- [x] control-panel / verify-hub / buckyos-api 测试过（control-panel 另有既有失败 `sso_identity::account_identity_carries_ceiling_method_and_password_state`：期望 scopes 少了 `Aicc`，与本次无关）；`server/tests/pages.rs` 新增并通过；Desktop `pnpm check`/eslint/双入口构建过。
- [ ] aiworkspace e2e 相关 spec（ui UI-P01/P10/P15/P16、entries、mobile、offline 子域冷启动、presentation P3-15、sources）与全套。
- [ ] 真实 Zone DV：子域打开（已登录成员无感、访客公开链接免登录、私有链接→登录→回落）、`?id=` 稳定、改名后旧链接仍开、SW 离线冷启动；`/sso_refresh` 在子域续期。

## 5. 实施备注（2026-10-28）

- 文件落点：服务 `frame/aiworkspace/server/src/pages.rs`（页/静态/规范 302，`Limits.web_dir`，`pages::locate_web_dir`，`pages::page_url`）；Desktop `src/aiws.tsx`、`aiws.html`、`app/aiworkspace/standalone/StandalonePage.tsx`、`app/aiworkspace/links.ts`、`app/aiworkspace/launch.ts`；SSO 侧 `kernel/buckyos-api/src/app_availability.rs`（`is_system_delegation_target`）、`aiworkspace_client.rs`（`AIWORKSPACE_GATEWAY_APP_KEY`、`is_aiworkspace_host_label`）、`kernel/verify_hub/src/main.rs`、`frame/control_panel/src/sys_auth_backend.rs`。
- 跨工作区导航（fork / New 对话框选择另一个工作区）从子域页出发时无法直接委派（委派源必须是 Control Panel 会话）：`openWorkspacePage` 在子域上直接导航，目标源无 cookie 时页面以访客打开或转 Zone 根 `/login?redirect_url=`，已在 Zone 根登录者由 LoginPage 自动接力。
- 地址栏不随切换画布更新（与旧 `/workspace/<id>?surface=` 行为一致：目标打开后 `id/block` 从地址去掉）；要“地址即当前画布”需 shell 把活动 Surface 上报给宿主，另立项。
- 子域页面上“打开其他工作区”（`shell.close`）回 Zone 根 `/workspace` 列表；“主页”回 Zone 根 `/`。
- 部署：Desktop 构建仍只装到 `bin/control-panel/web/`，aiworkspace 服务按 `exe 旁 web/` → `../control-panel/web/` 找；多节点 Zone 若 aiworkspace 与 control-panel 不同节点需另给 `web/`（devkit 模块多路径安装未验证）。
- 旧版 `/workspace/<id>` 链接（HomeStation 自动分享、聊天里的历史链接）现在落到 Zone 根路由 → 列表页；按 Q5 不迁移。
