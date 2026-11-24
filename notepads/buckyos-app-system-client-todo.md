# BuckyOS APP 新版本实施 TODO：常驻系统客户端

日期：2026-11-23。状态：立项，未实施。§6 的问题等 owner 确认；T0 的验证不依赖这些答案，可以先做。

依据：[BuckyOS APP 新版本系统设计](<../../BuckyOSApp/doc/buckyos app 新版本系统设计.md>)（下称设计稿，写本文时它还没有提交进 BuckyOSApp 仓库）。设计稿第 3 章要求的“BuckyOS API 与现有代码 Review”就是本文 §2，后面的方案和任务都以这次 Review 为准。

涉及仓库：

- BuckyOSApp：Tauri 2 客户端（Rust + React），Apache-2.0。
- buckyos：Desktop Web UI（`src/frame/desktop`）、control_panel、msg_center、verify_hub、kevent、buckyos-api，AGPL-3.0。
- buckyos-websdk：npm 包 `buckyos`，Apache-2.0。
- SN：只有移动推送中转会涉及，本文不展开（Q9）。

源码核对基线：BuckyOSApp `e5cdb92`（main）、buckyos `6c80c119c` 加当前工作树、buckyos-websdk `bb96f3d`（0.7.201）。buckyos 工作树里有 143 个未提交的改动，接手时先看 `git status`，保留在途工作。

本文的结论全部来自 2026-11-23 读代码和文档。没有运行客户端，也没有在真实 Zone 上验证。标了“待验证”的结论，在 T0 里先验证再动手。

用词：**客户端**指 BuckyOS APP 原生客户端；**应用**指装在 Zone 上的业务 APP；**Shell** 指 Zone 提供的系统界面，也就是 buckyos 的 Desktop Web UI；**四种环境**指设计稿 §2.2 的桌面 Web UI、桌面 APP、移动 Web UI、移动 APP。

阅读顺序：§0 是结论，§1 是设计稿已经定下的方向，§2 是代码现状，§3 是方案，§6 是要 owner 定的事，§7 是任务，§10 是接手时必须遵守的约束。

## 0. 结论先行

1. **客户端现在比设计稿描述的旧版还要少。** APPS 页读的是本机的 `applist.json`，卡片不能点；Home 的“打开主页”是调系统浏览器；客户端不持有任何 Zone 的登录会话，也没有通知、深链接、开机启动。新版本的大部分能力要从头建。
2. **现有的 WebView 容器承载不了登录后的 Zone 应用。** 容器是“本地页面里套 iframe”。Zone 的会话 Cookie 是 `SameSite=Lax`，嵌在 `tauri://localhost` 页面里的 Zone 页面属于跨站 iframe，请求不带 Cookie。Zone 页面必须作为 WebView 的顶层文档加载（按 Cookie 规则推断，待 T0-1 实测）。
3. **许可证决定了代码放在哪。** BuckyOSApp 是 Apache-2.0，buckyos 是 AGPL-3.0。客户端不能拷贝 Desktop 的源码，也不能依赖 `buckyos-api` crate；能依赖的是 buckyos-base 和 websdk。
4. **建议的总体结构是：界面由 Zone 提供，客户端提供容器和本机能力**（Q1）。Home、APPS、Activity、Message Hub、移动主导航都是 Shell 的页面，四种环境用同一份；客户端负责账号、激活、Zone 名单与切换、原生窗口、系统通知、托盘、音频。前三条结论都指向这个结构。
5. **应用发通知的接口已经有了。** msg-center 的 `notify.*`（System Notify）已经在用。缺的是客户端接收通道、系统提醒、普通应用的通知权限、Activity 时间线、汇总未读数和移动推送。
6. **现成的事件流没有鉴权。** 浏览器用的 `POST /kapi/kevent/stream` 不验 token，任何能访问网关的人都能订阅任意事件。客户端可以先用它跑通，但常驻客户端会放大这个问题（Q8）。
7. **窗口系统在 Web UI 里已有大半。** 有窗口管理、窗口声明类型、Shell 与应用之间的 `buckyos.app-frame` 协议、移动端系统栏。缺四样：第三方应用不能声明窗口属性（AppDoc 没有字段）；PRD 定的“窗口 ID = AppID + 路径”没有实现；没有 `buckyos://` 处理；移动布局只挂载最上面的窗口，切换应用会销毁原来的页面。
8. **多 Zone 的登录本身不冲突，缺的是名单和上下文。** Cookie 按主机隔离，一个 WebView 可以同时登录多个 Zone。但没有“这个 DID 能登录哪些 Zone”的名单，只有 owner 文档里自己绑定的 `binded_zone_list`；Zone 主题色没有字段。
9. **运行环境识别只做了一半。** websdk 看到 `window.BuckyApi` 就判为 `AppRuntime`，但它的行为和浏览器完全一样；分不出桌面 APP 和移动 APP，也没有能力清单。
10. **后台音频有零散基础。** 桌面音乐小部件用 `<audio>` 加 Media Session；BuckyOSApp 有一个没合入的录音分支 `feature/audio_record`；没有面向应用的音频接口。
11. **有两处现有文档互相冲突。** `buckyos://` 在客户端的《外部调用协议》里是 `buckyos://<action>` 且必须带 `request_url`，在 buckyos 的文档里是 `buckyos://app/<app_id>`（Q11）。设计稿说“一个 Self-hosted Group 背后通常对应一个 Zone”，代码里一个 Zone 可以承载很多群（Q3）。
12. **最不确定的是 WebView 的实际行为。** Cookie、隐藏窗口里的长连接和播放、系统通知的点击回调、内嵌 WebView 里的第三方登录，都要在 Windows、macOS、Android 上实测。所以 T0 排在所有实现之前。

## 1. 设计稿已定的方向

状态一栏照设计稿的说法：已明确、当前倾向、候选、未定。

| 编号 | 方向 | 状态 | 本文落实 |
| --- | --- | --- | --- |
| D1 | 桌面端先行，每项能力同时考虑移动端和 Web UI | 已明确 | 各阶段先做桌面，接口按四种环境设计 |
| D2 | 能力接口统一，界面各端自己适配；所有应用界面都要能在浏览器里运行 | 已明确 | §3.2 |
| D3 | 基础建设顺序：API 与代码 Review → 多 Zone → Notify 与 Activity → Window System | 已明确 | §2、§9 |
| D4 | 当前 Zone 是整个客户端的全局上下文；同一个 DID 登录多个 Zone 并切换；入口在 Home 的账号区 | 已明确 | §3.3 |
| D5 | 用明确切换，不做跨 Zone 内容聚合 | 当前倾向 | §3.3、§4 |
| D6 | 每个 Zone 一个主题色，只做小范围强调，明暗两套基础色不变 | 已明确，细节未定 | §3.3、Z8 |
| D7 | 应用发通知，系统负责各端触达；系统应用自带权限，普通应用要申请 | 已明确 | §3.4、N5 |
| D8 | 独立的 Activity，不并入 Message Hub | 当前倾向 | §3.4、N6 |
| D9 | 桌面客户端与 Zone 保持 WebSocket 长连接收通知；移动端由客户端做平台适配 | 方向已明确，移动机制未定 | N1、N7、N9 |
| D10 | 桌面客户端是常驻的竖长小窗，配合托盘；托盘标记或闪动；双击托盘打开最新提示对应的应用 | 交互方向 | N3、Q14 |
| D11 | Message Hub 保持完整的单窗口应用 | 已明确 | W2 的声明里是 `single` |
| D12 | 统一的打开规则：新建、定位已有、进入指定页面；支持单窗口和多窗口、Window ID、标准 URL | 方向已明确，格式未定 | §3.5 |
| D13 | 桌面 APP 用原生窗口承载网页；应用可声明固定大小、可调整、最大化、最小化、单窗口或多窗口 | 已明确 | W2、W3 |
| D14 | 应用窗口最小化后继续存在，点 × 就结束；主客户端点 × 的行为未定 | 当前倾向 | W3、Q13 |
| D15 | 移动 APP 用全屏容器；系统 Header 有应用切换、返回、标题、搜索、更多、最小化；两种 Header 风格 | 已明确，细节未定 | §3.6 |
| D16 | 后台播放是通用能力；录音“统一采集再分发”是候选方案，验证后再定 | 候选 | §3.7、A5 |
| D17 | 桌面面板：Home、APPS（暂留）、Activity（新增）、Settings（暂留） | 已明确 | U1–U4 |
| D18 | 移动主导航：Home、Message Hub、＋、Home Station / Activity、Settings；Library 放下一版本 | 已明确 | U5–U7 |
| D19 | 两条首次使用路径都保留：桌面安装后激活本机服务；手机配合无头盒子 | 已明确 | Z9 |

## 2. 现状：API 与代码 Review

### 2.1 客户端（BuckyOSApp）

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 技术栈 | Tauri 2.10.3、React 19、HashRouter。已用的插件：http、log、opener、store、single-instance。没有 notification、deep-link、autostart、window-state 这些插件 | `src-tauri/Cargo.toml` |
| 主窗口 | 一个，label 是 `main`，400×800，不能调整大小，不能最大化 | `src-tauri/tauri.conf.json` |
| 主导航 | Home、Apps、Setting 三个 Tab，桌面和移动相同 | `src/app/MainRoutes.tsx` |
| Home | 只有绑定组件。没绑定时扫描局域网或手输地址，打开激活页。已绑定时显示 Zone 选择（owner 文档的 `binded_zone_list` 有多个时）、“打开主页”、APPS、解绑。“打开主页”用系统浏览器打开 `https://<zone hostname>/` | `src/pages/main/components/BindOod.tsx`、`src/features/did/oodHome.ts` |
| Apps | 读本机 `$BUCKYOS_ROOT/bin/applist.json`，显示卡片，卡片没有点击行为。不读 Zone 的应用列表 | `src-tauri/src/applist/applist.rs`、`src/pages/main/Apps.tsx` |
| WebView 容器 | 桌面：新建窗口，窗口加载本地 `/web-container` 页面，再用 iframe 套目标地址；同 label 的窗口已存在就置前。移动：在主 WebView 里跳到 `#/web-container?embedded=1`，同样是 iframe。现在只用来打开激活页、SN 说明页和调试页 | `src/utils/webview.ts`、`src/pages/WebContainer.tsx` |
| 原生桥 `window.BuckyApi` | 插件把 `public/bucky-api.js` 注入所有 frame；脚本用 `window.parent.postMessage` 把请求交给外层容器页。五个方法：`getPublicKey`、`getCurrentUser`、`signJsonWithActiveDid`、`resolve_did`、`openExternalUrl`。签名时的密码框画在外层容器页里 | `src-tauri/src/lib.rs:11`、`public/bucky-api.js`、`src/bridges/iframeBridge.ts`、`doc/BuckyApi.md` |
| 权限配置 | 只有一份 capability，对窗口 `main` 和 `webview_*` 生效，并且把所有 `http://*`、`https://*` 远程地址列了进去 | `src-tauri/capabilities/default.json` |
| 托盘 | 只有桌面。菜单是本机服务的状态和启停、打开 App、退出。左键右键都弹菜单。托盘启用时主窗口点 × 是隐藏，退出只能走菜单。桌面进程单实例 | `src-tauri/src/tray.rs`、`src-tauri/src/lib.rs:70-92`、`doc/托盘图标需求.md` |
| 身份与签名 | 助记词用密码加密后存在本机，每次签名都要输密码 | `src-tauri/src/did/` |
| 主题 | 明暗两套，存 localStorage `buckyos.theme`；Android 通过 `BuckySystemBars` 同步系统栏 | `src/theme.ts` |
| 移动端 | Android 工程有安全区和键盘处理，锁竖屏，权限只有 INTERNET。iOS 工程存在，没有发布 CI | `src-tauri/gen/android/` |
| 外部调用协议 | `buckyos://<action>?v=1&request_url=…&request_hash=…` 只有设计文档，代码里没有注册 scheme | `doc/BuckyOS外部调用协议设计.md` |
| 音频 | main 上没有。远程分支 `feature/audio_record`（2026-03，约 9000 行）做了录音到本地文件、Android 前台服务、iOS 插件，并给 `BuckyApi` 加了录音方法。没有合入，main 在分叉点之后又走了 123 个提交 | `origin/feature/audio_record` |

### 2.2 Shell（buckyos 的 Desktop Web UI）

路径都相对 `buckyos/src/frame/desktop/src`，另有说明的除外。

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 窗口管理 | `DesktopUIStore`。图标启动走 `openApp`：同一个 appId 只有一个窗口，已有就置前。`openAppWindow(appId, {windowId, newInstance, instanceKey, launch})` 用来开指定窗口或多开。有关闭前询问、最小化、最大化。打开的窗口不持久化，位置大小按 appId 存 localStorage | `models/DesktopUIDataModel.ts:1059-1418`、`models/ui.ts:253` |
| 窗口 ID | PRD §16.4 定的是“AppID + 路径”，没有实现。现在是 `${appId}-${时间戳}`，多窗口靠 `instanceKey` 区分 | `buckyos/product/desktop/BuckyOS_Web_Desktop_需求文档.md` |
| 窗口声明 | 有 `WindowManifest` 类型：`defaultMode`、`allowMinimize`、`allowMaximize`、`desktopWindow{width,height,minWidth,minHeight}`、`mobileStatusBarMode: compact\|standard` 等。内置应用的声明写死在目录里；第三方应用用一份固定默认值。AppDoc 没有窗口字段，第三方应用无法声明。没有“固定大小 / 可调整”开关，也没有单窗口 / 多窗口声明 | `models/ui.ts:132`、`mock/data.ts`（内置目录，真实模式也用它）、`app/backend-apps.ts:76-125`、`buckyos/src/kernel/buckyos-api/src/app_doc.rs` |
| 应用承载 | 内置应用是同一文档里的 React 面板。第三方应用每个窗口一个 iframe，地址是 `https://<web_hosts[0]>.<zone host>/`；加载前调 control-panel `auth.prepare_app_launch` 换一次性登录回调地址。最小化的窗口保持挂载 | `app/web-frame/WebAppFramePanel.tsx`、`app/web-frame/launch.ts` |
| 应用列表 | control-panel `apps.list`，返回 `AppSummary{app_id, app_instance_id, show_name, app_icon_url, web_hosts, runtime_type, …}`，与内置目录合并 | `api/app_mgr.ts:36-63` |
| Shell 与应用的协议 | `buckyos.app-frame` v1，走 iframe postMessage。Shell 发给应用：`frame.init{windowId, formFactor, theme{mode, accent?}, locale, launch, shellCapabilities}`、`frame.open`、`frame.beforeClose`、`frame.back`，以及主题、语言、焦点变化。应用发给 Shell：`window.setTitle`、`window.requestClose`、`content.open`（按内容打开）、`sysdlg.open`。没有“按应用 ID 打开另一个应用”的消息 | `buckyos-websdk/src/app-frame.ts` |
| 打开入口 | Shell 根路径支持 `?open=`、`?settings=` 等查询参数。独立路由有 `/messagehub`、`/homestation`、`/workspace`、`/taskcenter`。没有通用的“打开应用 X 的页面 Y”地址，也没有 `buckyos://` 处理 | `App.tsx`、`buckyos/doc/sdk/context ext.md:358` |
| 移动布局 | 宽度不超过 768px 时进入。顶部系统栏有返回、标题、“…”菜单、最小化；`standard` 是固定的一行，`compact` 是悬浮在应用上面。左侧抽屉里有全部应用和运行中的应用。全屏容器只挂载最上面的窗口，切到别的应用时原来的 iframe 被销毁。没有底部 Tab，没有应用切换视图，没有接浏览器历史 | `desktop/StatusBar.tsx`、`desktop/MobileTaskbar.tsx`、`desktop/windows/MobileWindowSheet.tsx` |
| 通知界面 | 消息、通知、任务三个托盘只在移动布局启用，宽屏桌面没有通知中心。没有用 Web Notification、Web Push、角标接口 | `desktop/DesktopRoute.tsx:852`、`desktop/tray/` |
| 小部件 | 时钟、便签、系统信息、音乐播放器。CPU、内存、磁盘、网络显示在任务栏里，来自 control-panel `system.metrics {lite:true}`。音乐播放器是一个 `<audio>` 加 Media Session，卸载时暂停 | `desktop/widgets/`、`desktop/widgets/MusicPlayerWidget.tsx:161` |
| 主题 | 明暗两套 token，强调色 `--cp-accent` 写死。Zone 展示信息 `system/zone_presentation` 只有名称、描述、图标；`zone.presentation.get` 公开可读，`set` 要管理员。`frame.init` 里已经有 `theme.accent` 的位置，Shell 没有填 | `index.css`、`buckyos/src/kernel/buckyos-api/src/system_config.rs:87` |
| 运行容器 | `runtimeContainer`（browser、desktop-app、mobile-app）是设置里的手动选项，不是检测出来的。PRD §6.2 写的是桌面 App 容器里每个应用用一个系统窗口；代码里对应的 `placement: 'new-container'` 只弹一条提示 | `models/DesktopUIDataModel.ts:417` |

### 2.3 通知与事件

路径相对 `buckyos`。

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 应用通知接口 | 已有。msg-center 的 `notify.send / get / list / ack / dismiss / cancel`。两类：`notice` 告知、`input` 提问。四个等级：`quiet / normal / timely / urgent`。三个分类：`todo / agent / app`。每条通知在用户的 `sysnotify:*` 会话里有一条主记录，统一链接是 `/messagehub?notify=<id>`；`source.link` 可以带应用自己的页面地址 | `doc/message_hub/System Notify.md`、`src/frame/msg_center/src/notify.rs`、`buckyos-websdk/src/msg_center_client.ts` |
| 通知权限 | 非系统调用方只能通知自己所属的用户。RBAC 里 `users`、`admin`、`app`、`frame`、`agent` 组都有 `obj://msg-center/notify` 的读写权，所以任何应用都能发。没有“某个应用能不能发通知”这一项 | `notify.rs:606`、`src/kernel/buckyos-api/src/rbac_config.rs:231-254` |
| 实时通道 | kevent。浏览器走 `POST /kapi/kevent/stream`，返回 NDJSON 流，不是 WebSocket。订阅的模式形如 `/obj/msg-center/owners/<owner>/sessions/**`。**没有鉴权**：网关在权限逻辑之前就把 `/kapi/kevent/*` 转给本机 3181 端口，kevent 自己不验 token。事件尽力投递，没有读者就丢，设计上要配合轮询 | `src/kernel/kevent/src/http.rs`、`src/rootfs/etc/boot_gateway.yaml:240`、`doc/arch/kevent/kevent.md` |
| 各界面怎么拿更新 | Message Hub 用 kevent 加轮询。状态栏铃铛每 30 秒调一次 `notify.list{status:[open]}`。任务用 TaskMgr 事件加轮询。AI Workspace 有自己的 WebSocket（`/kapi/aiworkspace/session/ws`，首条消息带 token） | `src/frame/desktop/src/desktop/tray/trayRuntime.ts` |
| 未读 | 每个会话有 `unread_count`。没有总未读数和通知计数的接口，Shell 在前端自己累加 | `src/frame/msg_center/src/owner_session_db.rs:312` |
| 已有的通知来源 | Agent 和 Jarvis、Goal 检查脚本、专属邀请、运行时异常，都调 `notify.send`。Message Hub 的新消息不走 `notify.*`，是普通的收件箱记录。`dcrontab remind` 发的是普通聊天消息。AI Workspace 的评论、日程、待办没有接通知 | `src/frame/lib_opendan/src/notify.rs`、`notepads/opendan-d4-timer-reminder-todo.md` |
| 推送 | 没有。Shell 的 service worker 只做离线缓存。没有 Web Push、FCM、APNs 的任何代码 | `src/frame/desktop/src/serviceWorker.ts` |
| Activity | 没有。最接近的是移动托盘的归并逻辑，和 Message Hub 里“系统通知”的三个会话。`product/system UI.md` 提过“Activity Center = 任务管理 + 收件箱”，没有实现 | `src/frame/desktop/src/desktop/tray/trayModel.ts` |

### 2.4 登录、会话与多 Zone

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 运行环境判定 | websdk 看到 `window.BuckyApi` 就返回 `RuntimeType.AppRuntime`，否则是 `Browser`。`AppRuntime` 的实现直接继承 `Browser`，行为相同。`AppClient` 是 Node 专用的模式，不能在 WebView 里用 | `buckyos-websdk/src/sdk_core.ts:886`、`src/runtime.ts:344` |
| 浏览器登录 | 跳到 `https://sys.<zone>/login`，成功后在目标源的 `/sso_callback` 写 Cookie：`buckyos_session_token`（15 分钟）、`buckyos_refresh_token`（HttpOnly，7 天）。两个都是 `SameSite=Lax`，不带 Domain，只对当前主机有效。SDK 通过 `/sso_refresh` 续期。会话族（一次登录签出的一串 refresh token，代码里叫 family）已经持久化，verify-hub 重启后还能续期 | `buckyos/src/frame/control_panel/src/sys_auth_backend.rs`、`buckyos/src/kernel/verify_hub/src/family_store.rs` |
| 多个 Zone 并存 | Cookie 和本地存储都按主机隔离，同一个 WebView 里可以同时登录多个 Zone。SDK 在一个页面里只有一个当前 Zone，从 `location.host` 得出 | `buckyos-websdk/src/sdk_core.ts:800` |
| 不经浏览器的登录 | verify-hub `login_by_jwt` 只接受本 Zone 认识的签发者：`root`（Zone owner 的钥匙）、本地用户（`users/<id>/doc` 的默认钥匙）、设备、Agent。外 Zone 的 DID 不能直接用。control-panel 的 `auth.login` 只收密码。客户端 `signJsonWithActiveDid` 返回的 `pwd_hash` 与 websdk 的 `hashPassword` 是同一个算法 | `buckyos/src/kernel/verify_hub/src/main.rs:660-731` |
| 跨 Zone 登录 | 已实现两条。一条是浏览器联合登录：目标 Zone 调 `auth.did.begin`，跳到 home Zone 的 `/sso/federate` 确认，用一次性授权码换会话。没有本地账号的 DID 得到的是 External 会话，只能访问 msg-center 的群路径、AI Workspace、HomeStation。另一条是服务之间的断言登录，只允许 msg-center 和 homestation。`user.invite.create / accept` 可以把一个 DID 变成目标 Zone 的本地账号。“用 BuckyOS App 登录”在 PRD 里是后置项，没有实现 | `buckyos/src/frame/control_panel/src/federation.rs`、`notepads/done/hybrid-sso-implementation-todo.md`、`notepads/cross-zone-resource-share-sso-todo.md` |
| 把会话带给应用 | `auth.prepare_app_launch{app_instance_id \| service_id, redirect_url}`。调用方必须是 control-panel 的用户会话，返回目标主机上的一次性登录回调地址 | `sys_auth_backend.rs:251` |
| 可登录的 Zone 名单 | 只有 owner 文档的 `binded_zone_list`，是自己绑定的 Zone。没有“这个 DID 在哪些 Zone 有账号或成员身份”的名单。msg-center 的 `joined_groups` 表记了已加入的群和它的 host Zone | `buckyos-base/src/name-lib/src/user.rs:86` |
| 群与 Zone | 群是 `<group_id>.<zone_id>` 形式的 Group DID，由 host Zone 的 msg-center 承载。一个 Zone 可以承载很多群，代码里没有“一个群对应一个 Zone”的约束 | `buckyos/doc/message_hub/Self-Host-Groupv2.md` |
| 应用地址 | Zone owner 装的应用是 `https://<appid>.<zone host>/`，其他用户装的是 `https://<appid>-<userid>.<zone host>/` | `buckyos/src/kernel/buckyos-api/src/app_schema.rs:166` |

### 2.5 能力清单与四种环境

| 能力 | 桌面 Web UI | 桌面 APP | 移动 Web UI | 移动 APP | 结论 |
| --- | --- | --- | --- | --- | --- |
| 运行环境识别 | 有（Browser） | 只知道在客户端里 | 有（Browser） | 只知道在客户端里 | 补容器类型和能力清单 |
| 当前 Zone 上下文 | 由页面地址决定 | 没有 | 由页面地址决定 | 没有 | 客户端新增 |
| Zone 切换 | 没有 | 没有 | 没有 | 没有 | 新增 |
| 应用发通知 | 有 | 有，同一接口 | 有 | 有，同一接口 | 保留，补权限 |
| 系统提醒 | 没有 | 没有 | 没有 | 没有 | 新增 |
| 通知中心、Activity | 没有 | 没有 | 只有铃铛托盘 | 没有 | 新增 |
| 打开应用、定位已有窗口 | 有，只支持单窗口规则 | 没有 | 有 | 没有 | 统一规则 |
| 窗口声明 | 只有内置应用 | 没有 | 只有内置应用 | 没有 | 放进 AppDoc |
| 多窗口与窗口 ID | 部分（`instanceKey`） | 没有 | 不适用 | 不适用 | 补全 |
| 通过地址进入页面 | 部分（查询参数） | 没有 | 部分 | 没有 | 定标准地址 |
| 移动容器 Header | 不适用 | 不适用 | 有，缺应用切换、搜索、第二行 | 没有 | 补全 |
| 移动端应用保活 | 不适用 | 不适用 | 没有 | 没有 | 新增 |
| 后台播放 | 页面开着就能播 | 待验证 | 待验证 | 没有 | 新增能力 |
| 录音 | 应用内 `getUserMedia` | 分支原型 | 应用内 `getUserMedia` | 分支原型 | 待验证 |
| Zone 主题色 | 没有 | 没有 | 没有 | 没有 | 新增 |

### 2.6 与新要求冲突的现有行为

1. **iframe 容器和登录 Cookie。** 见 §0 第 2 条。影响桌面和移动两端的容器做法。
2. **原生桥只能过一层 `window.parent`。** 新结构里，应用要么是顶层文档（桌面 APP 的应用窗口，没有 parent），要么是 Shell 里的 iframe（parent 是 Shell，不是客户端）。两种情况下现在的 `bucky-api.js` 都失效。
3. **签名密码框画在外层容器页里。** Zone 页面变成顶层文档后没有外层页。密码不能画进远程页面，否则页面脚本能读到。
4. **capability 对所有远程地址开放。** 应用页面成为顶层文档后，远程页面可能可以直接调用全部 Tauri 命令，包括切换身份、停止本机服务（按 Tauri 2 的权限模型推断，待 T0-2 验证）。
5. **设计稿对旧版的描述与代码不符。** 设计稿 §1.1 说旧版会读取所绑定 Zone 的应用列表并启动应用，代码里 APPS 页读的是本机文件，不能启动。
6. **主窗口固定 400×800。** 常驻竖长面板可以沿用这个形态，但 Shell 把不超过 768px 的宽度当成手机布局。
7. **托盘左键就弹菜单。** 与设计稿“双击托盘打开最新提示”冲突（Q14）。
8. **`buckyos://` 有两种定义。** 见 §0 第 11 条。
9. **音乐小部件卸载就暂停。** Home 不在前台时播放会停。
10. **Message Hub 的新消息不产生通知记录。** Activity 如果只读 `notify.list`，就看不到消息。

## 3. 方案

本章是建议。每一节里凡是由我替 owner 选的，都在 §6 列了对应的问题。

### 3.1 总体结构：界面由 Zone 提供，客户端提供容器和本机能力

| 归属 | 内容 |
| --- | --- |
| Shell（Zone 提供，四种环境同一份代码） | Home（系统面板、应用入口、小部件）、APPS、Activity、Message Hub、Home Station、Zone 设置、移动主导航和“＋”、移动应用容器的系统 Header |
| 客户端（本机） | 账号（DID 钱包）、设备发现与激活、Zone 名单与登录、当前 Zone 切换、桌面原生应用窗口、系统通知与托盘、后台音频、客户端自己的设置、Zone 连不上时的本地页面 |

客户端的主窗口顶层加载当前 Zone 的 Shell（`https://<zone host>/`）。桌面端主窗口保持竖长，Shell 按窄面板布局显示。移动端的主 WebView 同样顶层加载 Shell，应用作为 Shell 里的同站 iframe 运行，和移动 Web UI 是同一套容器。桌面端的应用不在 Shell 里开网页窗口，而是由客户端开原生窗口，窗口顶层直接加载应用地址。

选这个结构有四个理由：

1. Cookie 规则要求 Zone 页面是顶层文档（§0 第 2 条）。
2. 许可证不允许把 Desktop 的代码搬进客户端（§0 第 3 条）。
3. 设计稿要求四种环境的打开、定位、容器规则连续。同一份 Shell 代码最容易做到。
4. 各个 Zone 的版本可能不同，界面跟着 Zone 走，不会出现客户端界面和 Zone 后端对不上的情况。

代价有三个：Zone 连不上时没有 Home，只能显示本地页面；老版本的 Zone 没有新界面，客户端要按能力降级，最低限度是把 Zone 首页当普通网页显示；每次进入都要联网加载。

如果 owner 要客户端自带 Home（不采用本节结构），阶段 6 的界面要在 BuckyOSApp 里重写一份，阶段 1 到 5 的会话、通知、窗口、音频任务基本不变。

### 3.2 应用接口分层与运行环境识别

应用只直接使用 websdk。websdk 下面有两条通道，由 SDK 自己选：

| 层 | 现状 | 新版本 |
| --- | --- | --- |
| websdk | 运行环境只分浏览器和客户端 | 增加运行环境描述：容器类型（`browser`、`desktop-app`、`mobile-app`）、外形（`desktop`、`mobile`）、能力清单。应用按能力清单决定用什么功能 |
| Shell 协议 `buckyos.app-frame` | 只有 iframe postMessage 一种传输 | 增加原生传输：应用是顶层文档时，同样的消息经 `BuckyApi` 收发。新增 `app.open`（打开另一个应用或页面） |
| 原生桥 `window.BuckyApi` | 五个方法，靠 `window.parent` | 改成直接调原生。增加 `BuckyApi.env`（同步可读：容器类型、平台、客户端版本、能力清单）、通用入口 `BuckyApi.call(name, payload)` 和事件订阅 `BuckyApi.on(event, handler)`。旧的五个方法保持不变。`invoke` 这个名字留给《外部调用协议》 |

能力清单的名字在实施时定，至少包括：`window.native`（原生窗口）、`notify.system`（系统提醒）、`zone.switch`、`app.open`、`audio.background`、`audio.record`。同一个能力在不同环境里可用就列出，不可用就不列；应用不靠猜平台来判断。

应用是 Shell 里的 iframe 时（桌面 Web UI、移动 Web UI、移动 APP），它只和 Shell 说话，需要本机能力的请求由 Shell 转给客户端。这样应用只面对一套协议。

### 3.3 多 Zone

**数据分三层。** 账号级：DID 钱包、每个 DID 的 Zone 名单和当前 Zone。客户端级：语言、明暗、托盘、开机启动、通知总开关。Zone 级：登录会话、主题色缓存、通知开关。用户在某个 Zone 里的界面偏好（例如移动端动态入口默认看 Home Station 还是全部动态）存在该 Zone 的 system_config 里，跟着用户走。

**Zone 名单的来源。** 第一版两个来源：当前 DID 的 `binded_zone_list`，以及用户手动添加（输入 Zone 的主机名或 DID，或者打开一个邀请链接）。已加入的群的 host Zone 可以作为推荐，放到后面做。每个 Zone 记录：DID、主机名、名称、图标、主题色、当前 DID 在其中的身份（owner、本地账号、External）、会话状态。

**登录。** 在客户端的 WebView 里走 Zone 现有的登录页，密码登录、DID 联合登录都沿用，不新写登录流程。登录成功后 WebView 有了 Cookie 会话，Shell 页面再调 control-panel 的一个新接口，为客户端签发一组独立的会话（Z5），交给客户端的 Rust 侧保存。客户端用这组会话收通知、调 `apps.list`、`auth.prepare_app_launch`。两组会话分开，是因为 refresh token 每次续期都会轮换，verify-hub 发现旧 token 被再次使用时会撤销整个会话族；WebView 和 Rust 侧共用一组就会互相踩。owner 用自己的钥匙免密码登录自己的 Zone（`login_by_jwt`）是体验优化，放到后面（Q2）。

**切换。** 切换当前 Zone 就是把主窗口导航到目标 Zone 的 Shell。我建议的默认行为（Q4、Q5）：

| 对象 | 桌面 APP | 移动 APP |
| --- | --- | --- |
| 原 Zone 已打开的应用 | 窗口不关，仍属于原 Zone，窗口上显示所属 Zone 的名称和主题色 | 全部结束。切换前对有未保存内容的应用走 `frame.beforeClose` |
| 原 Zone 的播放、录音 | 窗口还在就继续 | 随应用结束而停止，切换前提示 |
| 非当前 Zone 的通知 | 照常接收和弹出，标明来自哪个 Zone | 同左（依赖移动推送） |
| 点击其他 Zone 的通知 | 直接打开那个 Zone 的应用窗口，不改当前 Zone | 先切换到那个 Zone，再打开 |

**主题色。** 在 `system/zone_presentation` 里加一个 `accent` 字段，管理员设置，公开可读。Shell 增加一个 Zone 强调色 token，用在 Tab 选中、指示线、账号区，按明暗两套各算一档并保证对比度；品牌色和其它 token 不动。`frame.init.theme.accent` 把它传给应用。客户端把它缓存在 Zone 记录里，用在 Zone 列表、应用窗口的 Zone 标识和托盘菜单。

### 3.4 Notify 与 Activity

**应用侧不变。** 应用继续调 `notify.send`。要补的是下面几件。

**客户端通道。** 连接放在客户端的 Rust 侧，每个开了通知的已登录 Zone 一条。第一步直接用现有设施：订阅 kevent 流里该用户的 msg-center 会话事件和任务事件，收到事件就拉 `notify.list` 和 `msg.list_sessions`，另加低频轮询兜底。这样不改服务端就能跑通。第二步在 msg-center 上做一条带鉴权的用户事件通道（WebSocket，首条消息带 token，只推这个用户自己的通知、消息摘要、汇总数），客户端切过去。设计稿写的“WebSocket 长连接”指第二步的形态（Q8）。

**什么事件弹系统提醒。** 我建议的默认映射（Q6）：

| 来源 | Activity | 系统提醒 |
| --- | --- | --- |
| 通知，`quiet` | 进 | 不弹 |
| 通知，`normal` | 进 | 弹，无声 |
| 通知，`timely` | 进 | 弹，有声 |
| 通知，`urgent` | 进 | 弹，有声，托盘闪动，直到处理 |
| Message Hub 新消息（会话没有免打扰） | 每个会话一行 | 弹，同一会话合并；该会话正显示在聚焦的窗口里时不弹 |
| 带 `system_notify` 的任务 | 进 | 只在完成或失败时弹 |

**点击去哪。** 通知优先用 `source.link`（应用自己的页面地址），没有就用通知的统一链接 `/messagehub?notify=<id>`；消息去 `/messagehub?entityId=…&sessionId=…`；任务去 `/taskcenter?taskid=…`。这些地址都交给 §3.5 的统一打开规则。

**权限。** 应用在 AppDoc 的 `permissions` 里声明要发通知，安装时展示；用户可以在设置里按应用关闭。msg-center 在 `notify.send` 时对普通应用检查声明和用户开关，系统服务和内置应用不受限（Q7）。这是 BuckyOS 自己的通知权限。操作系统的通知许可由客户端在第一次开启通知时申请，客户端设置里有总开关和每个 Zone 的开关。

**Activity。** 是 Shell 的一个页面，按时间倒序合并三类内容：通知（`notify.list`，全部状态，游标翻页）、有未读的会话（每个会话一行）、带 `system_notify` 的任务。第一版在前端合并，复用移动托盘现有的归并逻辑，不建新存储。它和 Message Hub 里的“系统通知”读的是同一批记录（Q10）。

**汇总数。** msg-center 加一个汇总接口，返回未处理的通知数和消息未读总数。托盘角标、Tab 角标都用它。

**移动推送。** Zone 是自托管的，不可能各自持有 FCM 和 APNs 的凭据。我建议由 SN 提供推送中转：Zone 把“某用户有新通知”发给 SN，SN 转给 FCM 或 APNs，客户端被唤醒后再向 Zone 拉内容，推送里不带通知正文（Q9）。这需要 SN 侧立项，本文只在 Zone 记录里留登记状态的位置。

**托盘。** 现有托盘继续管本机服务。新增未读角标、紧急通知时闪动、菜单里列最近几条通知。点击行为见 Q14。

### 3.5 Window System 与标准地址

**一个打开请求。** 不论来自图标、Activity、系统通知、别的应用还是外部链接，都先变成同一个请求：

```ts
interface OpenAppRequest {
  zone: string            // Zone 的主机名或 DID
  app: string             // app_id；系统页面用保留名，例如 messagehub
  path?: string           // 应用内的页面地址，含查询参数和片段
  window?: string         // 窗口路径：应用自己定义，同一路径指同一个窗口
  mode?: 'reuse' | 'new'  // 缺省 reuse
}
```

**窗口 ID** 是 `Zone + 应用实例 + 窗口路径`，沿用 PRD §16.4 的“AppID + 路径”，前面加上 Zone。

**规则。** 应用声明自己是单窗口（缺省）还是多窗口。

| 应用声明 | 请求 | 结果 |
| --- | --- | --- |
| 单窗口 | 任意 | 已有窗口就置前；请求带 `path` 且和当前页面不同，就在这个窗口里进入该页面；没有窗口就新建 |
| 多窗口 | 不带 `window`，`reuse` | 已有窗口就把最近用过的那个置前，带 `path` 时在其中进入该页面；没有就新建 |
| 多窗口 | 带 `window` | 同一路径的窗口已存在就置前，否则新建并记下路径 |
| 多窗口 | `new` | 新建 |

规则写成一份测试向量（JSON），Shell 的 TypeScript 实现和客户端的 Rust 实现都跑同一份，保证四种环境结果一致。

**标准地址。** 我建议把应用页面自己的 https 地址作为规范形式：`https://<应用主机>.<zone 主机>/<path>`。它在浏览器里就是普通链接；在客户端里，凡是指向已登录 Zone 的地址都被截下来，用 `apps.list` 的 `web_hosts` 反查出应用，变成打开请求。`buckyos://app/<app_id>/<path>?zone=…` 只用来从操作系统层面拉起客户端（Q11）。

**窗口声明。** 放进 AppDoc，经 `apps.list` 带给 Shell 和客户端（Q12）。字段尽量沿用 Shell 现有的 `WindowManifest`，补上缺的两项：

```jsonc
"window": {
  "instance": "single",          // single | multi，新增
  "default_mode": "windowed",    // windowed | maximized
  "width": 960, "height": 640, "min_width": 420, "min_height": 280,
  "resizable": true,              // 新增；false 即固定大小
  "allow_minimize": true, "allow_maximize": true,
  "mobile_header": "standard"    // 对应现有 mobileStatusBarMode：standard 固定底色，compact 透明
}
```

**四种环境各自怎么落实。**

| 环境 | 窗口或容器 | 关闭与最小化 |
| --- | --- | --- |
| 桌面 Web UI | 现有网页内窗口，按新规则和新声明调整 | 现有行为 |
| 桌面 APP | 客户端开原生窗口，顶层加载应用地址，按声明设置大小和按钮 | 最小化保留；点 × 先问 `frame.beforeClose`，然后销毁 |
| 移动 Web UI | 现有全屏容器 | 最小化回 Home；关闭销毁 |
| 移动 APP | 与移动 Web UI 同一套（Shell 里的 iframe），加保活和应用切换 | 同上 |

桌面 APP 的应用窗口里，页面要跳到 Zone 以外的地址时，交给系统浏览器打开，窗口本身不跳转。这样原生桥不会暴露给任意网站。

### 3.6 移动容器

移动 APP 的容器就是 Shell 的移动布局，所以这部分工作主要在 Shell 里，移动 Web UI 同时受益。

- **保活。** 现在只挂载最上面的窗口。改成其余窗口隐藏但保持挂载，设数量上限，超过后淘汰最久没用的；被淘汰的应用回来时重新加载。
- **Header。** 两种风格直接对应现有的 `compact`（透明或半透明）和 `standard`（固定底色）。新增应用切换入口，列出已打开的应用。固定底色风格支持第二行和搜索入口，由应用通过 app-frame 声明和更新。透明风格不追加第二行。
- **返回。** Android 的系统返回键由客户端转给 Shell 的返回栈，再到应用的 `frame.back`。Shell 在浏览器里也接上历史记录。
- **Message Hub。** 一级 Tab 和 Home 里的图标指向 Shell 的同一个 `/messagehub`，是同一个实例（Q15）。
- **客户端外壳。** 主 WebView 在本地页面和 Shell 之间做顶层导航。签名时的密码输入用原生对话框。

### 3.7 后台音频

- **播放。** 声音仍由网页里的 `<audio>` 发出，客户端只负责让它不被系统停掉。桌面端的预期是窗口最小化、主窗口隐藏到托盘时播放不受影响（待 T0-5 验证）。移动端需要原生配合：Android 起媒体播放类型的前台服务并显示媒体通知，iOS 配置后台音频会话。对应用是一对接口：申请保活、释放保活。如果 T0-8 发现 WebView 里的音频在后台撑不住，就改成原生播放器，接口不变。
- **Home 音乐小部件。** 把播放状态和 `<audio>` 提到 Shell 级，小部件只是控制界面。切 Tab、切应用都不打断。
- **多个应用同时播放。** 第一版不做协调，各播各的，由系统混音（Q17）。
- **录音。** 先把 `feature/audio_record` 的原生录音移植到当前 main，整理成 `audio.record.*` 接口。它是“一个应用录到一个本地文件”的模型。
- **统一采集再分发。** 设计稿的候选方案。先做可行性验证（A5），看各平台的后台限制、延迟、权限提示归谁，再决定做不做（Q16）。

### 3.8 两端主界面

主界面是 Shell 的页面，客户端只负责承载。

- **桌面面板。** Shell 新增一种窄面板布局，用在客户端的竖长主窗口里。四个 Tab：Home、APPS、Activity、Settings。Home 从上到下是账号区（含 Zone 切换）、系统状态卡片（复用 `system.metrics`）、常用应用快捷方式、小部件。APPS 是当前 Zone 里当前用户可用的全部应用。Settings 是 Zone 的设置，里面有一项进入客户端自己的设置。
- **移动主导航。** 底部五项：Home、Message Hub、＋、Home Station / Activity、Settings。“＋”是动作入口，弹出快速创建菜单。菜单项来自内容处理器登记里声明了“创建”的入口，系统预置发消息、发动态、加笔记、传照片四项。`doc/sdk/context ext.md` 定义了 `create` 这个意图，但排在它的 P2，实施时先确认登记表是否已经支持；不支持就先把四项写死。创建操作都发生在当前 Zone。
- **动态入口。** 默认 Home Station，可以切到全部动态（Activity）。选择记在该 Zone 的用户设置里（Q20）。
- **客户端本地页面。** 保留欢迎与账号流程、身份列表与备份、设备扫描与激活、Zone 管理、客户端设置、关于。原来的 Apps 页显示的是本机组件清单，不是 Zone 应用，我建议改名后移到客户端设置里。

## 4. 范围外

- Library，以及 Settings 向 Library 的迁移（设计稿定为下一版本）。
- 跨 Zone 的内容聚合（D5）。
- 是否删除桌面 APPS（设计稿定为后续评估）。
- Web Push（浏览器关闭后的提醒）。浏览器里只做页面开着时的 Web Notification。
- iOS 的发布和推送（Q18 定之前，只保证工程能编译）。
- [Flutter 重实现方案](<../../BuckyOSApp/doc/Flutter重实现方案.md>)。本文按现有的 Tauri 代码规划。如果改用 Flutter，§3 的结构和协议不变，客户端侧的任务要重排。
- kevent 流自身的鉴权改造（Q8 决定归属）。

## 5. 已知风险

| 编号 | 风险 | 处理 |
| --- | --- | --- |
| R1 | 各 Zone 版本不同，老 Zone 的 Shell 没有新界面和新协议 | 客户端和 Shell 用能力清单互相探测；最低限度是把 Zone 首页当普通网页显示，并提示升级 |
| R2 | 三种 WebView 内核（WebView2、WKWebView、Android WebView）对 Cookie、隐藏窗口节流、媒体播放的处理不同 | T0 逐项实测，结论写回本文 |
| R3 | Google 等第三方登录可能拒绝在内嵌 WebView 里进行 | T0-3 验证；不行就改成系统浏览器登录后用 `buckyos://` 回到客户端 |
| R4 | kevent 流没有鉴权，常驻客户端让更多连接长期挂在这个入口上 | Q8；N7 提供带鉴权的通道 |
| R5 | 桌面系统通知的点击回调在 Windows 和 macOS 上不一定拿得到 | T0-4；备选是客户端自己画的提示小窗 |
| R6 | 移动端只有一个 WebView：切 Zone 或回到本地页面，Shell 和所有应用实例都会丢 | 切换前走 `frame.beforeClose`；本地页面尽量少进出；原生多 WebView 作为后续方案 |
| R7 | 许可证：客户端里出现从 buckyos 拷来的代码或对 AGPL crate 的依赖 | §10 的约束；Q19 |
| R8 | 签名密码需要原生输入界面，移动端要写原生对话框 | Z3；在此之前移动端不开放远程顶层页面的签名 |
| R9 | 客户端的会话 7 天不用就失效，要重新登录；客户端的会话和 WebView 的 Cookie 会话可能一个有效一个失效 | 客户端运行时定期续期；任何一个失效都引导回登录页，登录后重新签发 |
| R10 | Zone 用自签或私有 CA 的证书时，WebView 不信任，无法加载 | 本地“连接失败”页说明原因；不做证书例外 |
| R11 | Shell 的窄面板布局、移动主导航是新的界面工作量，集中在 buckyos 仓库 | 阶段 6 可以和阶段 2 到 5 并行 |

## 6. 待 owner 确认

每一条给出我的建议。“先答”一栏标“是”的，没有答案之前不要动“影响的任务”里列的任务；其余的按建议先做，owner 否决再改。

| 编号 | 问题 | 我的建议 | 先答 | 影响的任务 |
| --- | --- | --- | --- | --- |
| Q1 | 总体结构：界面由 Zone 的 Shell 提供，还是客户端自带 | 由 Shell 提供（§3.1） | 是 | Z2、Z7、阶段 6 |
| Q2 | 客户端怎么登录 Zone | WebView 里走现有登录页，再由 control-panel 的新接口给客户端签发独立会话；owner 钥匙免密码登录后置 | 是 | Z5、Z6 |
| Q3 | 用户能登录的 Zone 从哪里知道。设计稿说组织（群）通常对应一个 Zone，代码里群由 host Zone 承载、一个 Zone 可以有很多群 | 第一版只用 `binded_zone_list` 加手动添加；组织 Zone 怎么发现请 owner 定 | 否 | Z1 的推荐来源 |
| Q4 | 切换 Zone 时，原 Zone 已打开的窗口、正在播放和录音的任务怎么处理 | §3.3 的表：桌面保留，移动结束 | 否 | Z10 |
| Q5 | 非当前 Zone 的通知是否接收；点击后是否切换当前 Zone | 接收并标明来源；桌面不切换，移动切换 | 否 | Z10、N2 |
| Q6 | 通知等级、消息、任务到系统提醒的映射 | §3.4 的表 | 否 | N2、N10 |
| Q7 | 普通应用的通知权限怎么申请；已经装好的应用怎么办 | AppDoc 声明加用户开关；已装应用默认视为已授权，用户可关 | 否 | N5 |
| Q8 | 通知通道：先用没有鉴权的 kevent 流跑通是否可以接受；kevent 流的鉴权由谁改 | 可以先跑通，记为已知风险；N7 做带鉴权的通道；kevent 的鉴权单独立项 | 否 | N1、N7 |
| Q9 | 移动推送走什么 | SN 中转 FCM 和 APNs，推送不带正文；需要 SN 侧立项 | 是 | N9 |
| Q10 | Activity 收哪些内容；和 Message Hub 的“系统通知”是什么关系；历史保留多久 | 通知、有未读的会话、带标记的任务；两处读同一批记录、都保留；保留期跟 msg-center 的记录 | 否 | N6 |
| Q11 | 标准地址用什么；`buckyos://` 的两种定义怎么统一 | 应用页面的 https 地址是规范形式；`buckyos://app/…` 作为《外部调用协议》里一个不需要 `request_url` 的只读动作 | 是 | W6 |
| Q12 | 窗口声明放在哪，字段叫什么 | 放 AppDoc 的 `window` 段，字段见 §3.5 | 是 | W2 |
| Q13 | 主客户端点 × 和退出客户端时的行为 | 沿用现状：托盘启用时点 × 隐藏；退出时逐个询问应用窗口后全部关闭 | 否 | W7 |
| Q14 | 托盘的点击行为。现状是左右键都弹菜单 | 有未读时左键打开主窗口的 Activity，右键弹菜单；没有未读时保持现状 | 否 | N3 |
| Q15 | 移动端 Message Hub 的 Tab、Home 里的图标、通知唤起是不是同一个实例 | 是同一个 | 否 | U5 |
| Q16 | 录音“统一采集再分发”做不做 | 先做 A5 的验证，再定 | 否 | A5 之后的工作 |
| Q17 | 多个应用同时播放时要不要协调 | 第一版不协调 | 否 | 无 |
| Q18 | iOS 是否进入本版本 | 不进入，只保证能编译；Android 先行 | 否 | N9、A3 的 iOS 部分 |
| Q19 | 客户端要用到 buckyos 里的协议类型定义（通知、应用列表等）。是授权复用，还是按文档手写 | 按文档手写，不拷代码 | 否 | Z6、N1 |
| Q20 | Zone 主题色谁能设；Home Station / Activity 的偏好存在哪 | 主题色由 Zone 管理员设；偏好存在该 Zone 的用户设置里，跨设备一致 | 否 | Z8、U7 |
| Q21 | 原 Apps 页（本机组件清单）的去留 | 改名后移到客户端设置里 | 否 | U8 |

## 7. 任务

任务编号的前缀：T0 是先做的验证；Z 是多 Zone 与会话；N 是 Notify 与 Activity；W 是 Window System；C 是移动容器；A 是音频；U 是主界面。方括号里是改动所在的仓库：App 指 BuckyOSApp，Shell 指 `buckyos/src/frame/desktop`。

每个任务交付时记录：改了哪些入口、验证命令和结果、没有验证的平台。

### T0 先验证

在 BuckyOSApp 里开验证分支，做最小的试验页面和命令。结论写到本文 §11，验证代码不合入。Windows、macOS、Android 三个平台都要有结论。

- [ ] **T0-1 Cookie 与顶层加载。** 把一个真实 Zone 的 Shell 分别放在本地页面的 iframe 里和作为顶层文档加载，确认登录是否生效。确认 Cookie 在客户端的多个窗口之间共享，重启客户端后还在。确认 Shell 里的同站 iframe 应用（移动端的做法）能带上 Cookie。
- [ ] **T0-2 远程顶层页面里的原生桥。** 注入脚本在页面脚本之前执行；远程页面调用原生命令的路径可用；确认现有 capability 下远程页面到底能调哪些命令。确认 Shell 里的 iframe 能不能直接调原生，不能的话走 Shell 转发。
- [ ] **T0-3 在客户端里登录 Zone。** 顶层走完 `sys.<zone>/login` 的密码登录和 DID 联合登录。试 Google 登录是否被拒。核对 owner 钥匙走 `login_by_jwt`（`iss=root`）能否拿到 control-panel 的用户会话，以及普通本地账号的 `users/<id>/doc` 钥匙是不是钱包里的那把。
- [ ] **T0-4 系统通知的点击。** 在 Windows 和 macOS 上发一条系统通知，确认能否拿到点击事件、应用没有前台窗口时点击能否把它唤起。拿不到就评估平台原生接口和自绘提示小窗两条路。
- [ ] **T0-5 隐藏和最小化时的存活。** 主窗口隐藏到托盘后，WebView 里的定时器和 fetch 流是否继续；应用窗口最小化后 `<audio>` 是否继续播放；Media Session 是否接上系统的媒体键和媒体面板。
- [ ] **T0-6 长连接。** 用 Rust 对 `/kapi/kevent/stream` 保持连接，分别经直连和 SN 中转访问，记录多久被断、断开后的表现、网关 HTTP/2 下是否正常。
- [ ] **T0-7 移动端顶层加载 Shell。** Android 上主 WebView 顶层加载 Shell 后，安全区和键盘的 CSS 变量是否仍然注入；从 Shell 回到本地页面再回来的耗时；同时保活 3 到 5 个 iframe 应用的内存。
- [ ] **T0-8 音频。** Android 上起前台服务后，WebView 里的 `<audio>` 在切到后台、锁屏时能否持续；iOS 同样验证。评估 `feature/audio_record` 移植到当前 main 的工作量，列出冲突的文件。

完成条件：§0 第 2、12 条和 §2.6 第 4 条里标“待验证”的结论都有实测结果；T0-3、T0-4、T0-8 各自给出选定的做法。

### 阶段 1：多 Zone 与会话

- [ ] **Z1 [App] Zone 名单与当前 Zone。** 新模块 `src-tauri/src/zone/`，用独立的 store 文件，不和钱包的 vault 放在一起。命令：列出、添加、移除、设置当前、读取当前；当前 Zone 变化时发事件。名单按 DID 分开，切换 DID 时当前 Zone 跟着换。来源一：`binded_zone_list`，复用 `src/features/did/zoneBindingStatus.ts` 的 `resolveZoneBindingSnapshot`。来源二：手动添加，用 `resolve_did(…, "zone")` 取主机名，用 `zone.presentation.get` 取名称、图标、主题色。
- [ ] **Z2 [App] 主窗口承载 Shell。** 主 WebView 顶层导航到当前 Zone 的 `https://<zone host>/`。本地页面留在 `tauri://localhost`。加载失败或超时时回到本地的“连接失败”页，可以重试、切换 Zone、进入客户端设置。去掉“打开主页”调系统浏览器的做法。
- [ ] **Z3 [App] 原生桥改传输。** `public/bucky-api.js` 改成直接调原生，不再依赖 `window.parent`。旧的五个方法的参数、返回结构、错误码不变（node_active 在用 `getCurrentUser` 和 `signJsonWithActiveDid`）。新增 `BuckyApi.env`、`BuckyApi.call`、`BuckyApi.on`。签名的密码输入改到原生界面：桌面用一个本地页面的小窗口，移动端用原生对话框。同步 `doc/BuckyApi.md`。
- [ ] **Z4 [App] 收紧权限。** capability 拆成三份：本地页面可以用全部命令；已登录 Zone 的 Shell 源（Zone 根域和 `sys.`）可以用 Zone、窗口、通知相关的命令；其它远程源只能用身份读取、交互式签名和 app-frame 通道。窗口 label 约定：`main`、应用窗口 `app_*`、辅助窗口 `aux_*`。
- [ ] **Z5 [buckyos] 给客户端签发会话。** control-panel 新增一个接口，由已登录的 Shell 页面调用，经 verify-hub 的 `delegate_user_session` 签一组独立的会话族（目标是 control-panel），返回 token 对。会话族带上客户端的名称；退出登录时一起撤销。External 会话也可以签，范围不变。补允许和拒绝两类用例。
- [ ] **Z6 [App] Zone 会话管理（Rust）。** 保存每个 Zone 的 token 对，放系统钥匙串，不写进明文 store。到期前调 verify-hub `refresh_token` 续期；续期失败就把这个 Zone 标成“需要重新登录”。提供带 token 的 kRPC 调用，用 buckyos-base 的 kRPC crate；请求和响应的类型按接口文档手写。
- [ ] **Z7 [Shell] 账号区的 Zone 切换。** Shell 发现自己在客户端里（`BuckyApi.env`）时，在账号区显示 Zone 列表和“添加、管理”入口，数据来自 `BuckyApi.call('zone.list')`；选中后调 `zone.switch`。登录成功后调 Z5 的接口，把会话交给客户端（`zone.registerSession`）。在浏览器里不显示这个入口。
- [ ] **Z8 [buckyos、Shell、App] Zone 主题色。** `ZonePresentationFields` 加 `accent`（`#RRGGBB`），`zone.presentation.set` 校验格式。设置里的 Zone 页面加颜色选择。Shell 加 Zone 强调色 token，明暗各算一档。`frame.init.theme.accent` 填上。客户端缓存到 Zone 记录。注意这个结构体是 `deny_unknown_fields`，新客户端向老服务端写入带新字段的数据会被拒绝，写入方要先看服务端版本。
- [ ] **Z9 [App] 首次使用路径。** 桌面端先探测 `http://127.0.0.1:3182/device`。现在的扫描把 `127.*` 过滤掉了，只能靠本机的局域网地址发现自己（`src/pages/main/ScanDevice.tsx`）。激活完成后自动把新 Zone 加进名单、设为当前并进入登录。手机配合盒子的路径做回归。
- [ ] **Z10 [App、Shell] 切换语义。** 按 Q4、Q5 的结论实现：切换前询问未保存内容；桌面的应用窗口标出所属 Zone；移动端切换前提示会结束的播放和录音。

完成条件：同一个 DID 在客户端里登录两个 Zone，切换后 Home、APPS、Message Hub 都是目标 Zone 的内容；重启客户端后仍是上次的 Zone 且不用重新登录；一个 Zone 连不上不影响另一个。

### 阶段 2：Notify 与 Activity

- [ ] **N1 [App] 通知连接。** Rust 侧，每个开了通知的已登录 Zone 一条。第一步订阅 kevent 流，事件到了就拉 `notify.list{status:[open], since_ms}` 和 `msg.list_sessions`；另有低频轮询兜底。断线后退避重连，重连后先补拉。把连接状态（已连接、重连中、需要登录）报给界面和托盘。
- [ ] **N2 [App] 系统提醒。** 按 Q6 的映射生成系统通知；同一会话的新消息合并；同一条通知不重复弹（按 `notify_id` 和消息记录去重，重连补拉时尤其要注意）。点击后交给 W3 的打开规则。实现方式按 T0-4 的结论。客户端设置里加通知总开关和每个 Zone 的开关，第一次开启时申请系统许可。
- [ ] **N3 [App] 托盘。** 未读角标、紧急通知时闪动、tooltip 里的未读数、菜单里最近几条通知。点击行为按 Q14。同步 `doc/托盘图标需求.md`。
- [ ] **N4 [buckyos、websdk] 汇总接口。** msg-center 加 `notify.summary`：未处理的通知数（按分类）、消息未读总数（排除免打扰和系统通知会话，口径与 `trayRuntime.ts:74-104` 一致）。websdk 加对应方法，Shell 的托盘改用它。
- [ ] **N5 [buckyos] 应用的通知权限。** 在 AppDoc 的 `permissions` 里定义通知项；安装流程展示；设置里按应用开关；`notify.rs` 的 `notify_caller` 对普通应用检查。系统服务和内置应用不受限。已装应用按 Q7 处理。同步 `doc/message_hub/System Notify.md` §4。
- [ ] **N6 [Shell] Activity 页面。** 新路由 `/activity`。合并通知、有未读的会话、带标记的任务；每行点击走打开规则；支持单条已读或忽略、全部已读。宽屏桌面布局也加入口。归并逻辑从 `desktop/tray/trayModel.ts` 抽出来共用。
- [ ] **N7 [buckyos、App] 带鉴权的用户事件通道。** msg-center 提供 WebSocket，首条消息带 token（做法参照 AI Workspace 的 `session/ws`），只推该用户自己的通知、消息摘要和汇总数。N1 切到这条通道，kevent 流留作对老 Zone 的回退。
- [ ] **N8 [buckyos] 应用接入。** AI Workspace 的评论和提及、日程到点、待办提醒改成调 `notify.send`，`source.link` 填应用内的页面地址。`dcrontab remind` 是否改走 System Notify，和 `opendan-d4-timer-reminder-todo.md` 一起定。Message Hub 的新消息不新增通知记录，由通道直接产生提醒。
- [ ] **N9 [App、buckyos、SN] 移动推送。** Q9 定案后另立 TODO。
- [ ] **N10 [Shell] 浏览器里的提醒。** 页面开着并且用户授权后，用 Web Notification 显示，映射与 N2 相同。

完成条件：桌面客户端隐藏在托盘时，另一个账号发来消息、一个应用调 `notify.send`，都能在几秒内弹出系统提醒；点击后打开对应的应用页面；Activity 里能看到并标记已读；断网再恢复后不丢、不重复。

### 阶段 3：Window System

- [ ] **W1 [websdk、文档] 打开请求和规则。** 在 websdk 里定义 `OpenAppRequest`、窗口 ID 的拼法、§3.5 的规则函数和一份 JSON 测试向量。更新 `product/desktop/BuckyOS_Web_Desktop_需求文档.md` 的 §16.4 和 §21。
- [ ] **W2 [buckyos] 窗口声明进 AppDoc。** `src/kernel/buckyos-api/src/app_doc.rs` 加 `window` 段；`apps.list` 的 `AppSummary` 带出；内置应用的目录补上 `instance` 和 `resizable`，Message Hub 是 `single`；`app/backend-apps.ts` 用声明值替换固定默认值；同步 `doc/App 安装协议.md`。
- [ ] **W3 [App] 原生窗口管理（桌面）。** Rust 侧维护“窗口 ID → 窗口”。处理 `app.open`：按 W1 的规则置前、在窗口内导航或新建。新窗口顶层加载应用地址，按声明设置大小、最小尺寸、能否调整、最大化和最小化按钮。标题跟随页面。窗口上显示所属 Zone 的名称和主题色。位置大小按“Zone + 应用”记忆。点 × 先走 `frame.beforeClose`。指向 Zone 以外的链接交给系统浏览器。打开前可以用 Z6 的会话调 `auth.prepare_app_launch` 拿回调地址，省掉一次登录跳转。Rust 的规则实现要通过 W1 的测试向量。
- [ ] **W4 [websdk] app-frame 的原生传输。** `AppFrameClient` 在没有父窗口而有 `BuckyApi` 时走原生通道，`frame.init` 由客户端提供。新增 `app.open` 消息，`AppFrameHost` 同步支持。应用不改代码就能在两种传输下工作。
- [ ] **W5 [Shell] 对齐。** 窗口 ID 改成“AppID + 路径”；`openApp` 和 `openAppWindow` 按 W1 的规则；遵守新的声明字段（固定大小时不显示调整手柄）。`runtimeContainer` 改为从 `BuckyApi.env` 得出，设置里的手动选项只留给调试。在桌面 APP 容器里，图标启动和 `app.open` 交给客户端，不再开网页内窗口。
- [ ] **W6 [App、文档] 标准地址。** 实现 https 应用地址与 `OpenAppRequest` 的互转。注册 `buckyos://`：桌面端由单实例插件把第二次启动的参数转给已运行的进程，Android 加 intent filter。第一版只接 `buckyos://app/…`，其它 action 直接拒绝。按 Q11 的结论更新 `BuckyOSApp/doc/BuckyOS外部调用协议设计.md` 和 `buckyos/doc/sdk/context ext.md`。
- [ ] **W7 [App] 主客户端生命周期。** 按 Q13 实现；退出前依次询问各应用窗口。

完成条件：从图标、Activity、系统通知、另一个应用四个入口打开同一个应用，单窗口应用始终只有一个窗口并能进入指定页面；多窗口应用按窗口路径复用；同一组操作在桌面 Web UI 和桌面 APP 里结果一致。

### 阶段 4：移动容器

- [ ] **C1 [Shell] 应用保活。** 移动布局下非最上层的应用窗口保持挂载；设上限并按最近使用淘汰；被淘汰的应用回来时重新加载。
- [ ] **C2 [Shell] Header。** 应用切换入口；固定底色风格的第二行和搜索入口，应用通过 app-frame 声明和更新；两种风格沿用 `mobileStatusBarMode`。
- [ ] **C3 [App、Shell] 返回。** Android 系统返回键经客户端到 Shell 的返回栈（`desktop/windows/MobileNavContext.tsx`），再到 `frame.back`；Shell 在浏览器里接上历史记录。
- [ ] **C4 [App] 移动端外壳。** 主 WebView 在本地页面和 Shell 之间做顶层导航；安全区和键盘变量对远程顶层页面照常注入；签名用原生对话框（Z3 的移动部分）。
- [ ] **C5 [Shell] 最小化。** 最小化回到 Home，应用留在已打开列表里（现有行为，确认在新的保活逻辑下不变）。

完成条件：Android 客户端里打开三个应用，来回切换不重新加载；最小化回到 Home；系统返回键的行为和 Header 上的返回一致。

### 阶段 5：后台音频

- [ ] **A1 [Shell] 播放器上移。** 音乐播放状态和 `<audio>` 放到 Shell 级，Home 小部件只做控制。
- [ ] **A2 [App] 桌面播放。** 按 T0-5 的结论处理最小化和隐藏时的播放；把 Media Session 的可用情况写进能力清单。
- [ ] **A3 [App] 移动播放保活。** 能力 `audio.background`：申请保活时 Android 起前台服务和媒体通知，iOS 配置后台音频会话；释放时撤掉。需要的系统权限加到 Android 清单里。
- [ ] **A4 [App] 录音。** 把 `feature/audio_record` 的原生部分移植到当前 main，不直接合并分支。接口整理成 `audio.record.*`，走 Z3 的新桥。桌面端要么实现，要么在能力清单里不列。
- [ ] **A5 [App] 统一采集分发的验证。** 一个原生采集源，把音频块分发给多个订阅的应用。记录各平台的后台限制、延迟、权限提示归属，给出做或不做的建议。
- [ ] **A6 [websdk] 音频接口。** 封装保活和录音接口；不在客户端里时退回浏览器自身的能力，并通过能力清单告诉应用。

完成条件：桌面的音乐应用窗口最小化后继续播放；Android 客户端切到后台和锁屏后继续播放，媒体通知可以暂停和继续；Home 小部件在切 Tab 时不中断。

### 阶段 6：两端主界面

- [ ] **U1 [Shell] 桌面窄面板布局。** 在客户端的竖长主窗口里使用。四个 Tab。用鼠标操作，不依赖长按之类的触摸手势。
- [ ] **U2 [Shell] Home。** 账号区和 Zone 切换（Z7）、系统状态卡片、常用应用快捷方式（可添加和移除）、小部件。纵向滚动。
- [ ] **U3 [Shell] APPS。** 当前 Zone 里当前用户可用的全部应用，点击走打开规则。
- [ ] **U4 [Shell] Settings。** Zone 的设置；其中一项进入客户端自己的设置（`BuckyApi.call` 让主窗口回到本地页面）。
- [ ] **U5 [Shell] 移动主导航。** 底部五项；Message Hub 的 Tab 和图标是同一个实例（Q15）；Tab 上显示 N4 的未读数。
- [ ] **U6 [Shell] “＋”快速创建。** 预置四项；创建发生在当前 Zone。内容处理器登记已支持 `create` 意图时菜单项从登记里取，否则先写死（§3.8）。
- [ ] **U7 [Shell] 动态入口。** 默认 Home Station，可切到全部动态；偏好存在 Zone 的用户设置里。
- [ ] **U8 [App] 本地页面整理。** 客户端设置里加 Zone 管理、通知、开机启动；原 Apps 页按 Q21 处理；三个旧 Tab 的导航改成新结构下的本地页面入口。主窗口现在固定 400×800，改成可以调整高度并记住位置，方便常驻在屏幕一侧。
- [ ] **U9 [文档]** 更新 `BuckyOSApp/doc/当前需求文档.md`、`doc/BuckyApi.md`、`buckyos/product/desktop/` 下的 PRD，以及 websdk 的应用开发文档。

完成条件：设计稿 §8、§9 描述的入口在两端都能到达，并且 §8 的验收全部通过。

## 8. 验收

对应设计稿 §12 的检查场景。多 Zone 的场景用三 Zone 测试环境（`src/dev_configs/ssotest.json`）。桌面场景在 Windows 和 macOS 上各跑一遍。

| 场景 | 通过标准 |
| --- | --- |
| PC 首次安装 | 安装后从桌面图标进入，注册账号，发现并激活本机服务，自动进入该 Zone 的 Home，中间不需要手动找地址 |
| 手机配合盒子 | 只用手机完成注册、局域网发现、激活绑定，随后进入该 Zone 的 Home |
| 同一 DID 访问个人和组织 Zone | 两个 Zone 都能登录；切换后各 Tab 的内容属于目标 Zone；主题色跟着变；重启后保持 |
| 收到消息、评论、日程提醒 | 客户端在托盘里时弹出系统提醒；点击后进入对应应用的对应页面；Activity 里有记录 |
| 应用已经打开时再次进入 | 单窗口应用置前并进入指定页面；多窗口应用按窗口路径复用或新建；四个入口结果一致 |
| 桌面音乐播放器最小化 | 继续播放；点 × 后停止 |
| 移动端打开和切换应用 | 全屏容器；应用切换入口能回到已打开的应用且不重新加载；最小化回到 Home |
| 从“＋”快速写入 | 四个预置动作都能在当前 Zone 完成；切换 Zone 后写入的是新的 Zone |
| 移动动态视图切换 | 默认 Home Station；切到全部动态后重进仍是全部动态 |
| 四种环境运行同一个应用 | 同一个应用不改代码，在四种环境里都能打开；能力清单如实反映各环境可用的能力；不可用的能力有明确的返回 |
| 权限 | 第三方网站作为应用窗口的顶层页面时，不能调用身份切换、钱包删除、本机服务启停等命令 |
| 降级 | 连接一个没有升级的老 Zone，客户端能显示它的首页并提示升级，不崩溃 |

## 9. 建议顺序

1. T0 全部，三个平台并行。Q1、Q2 在 T0 期间请 owner 定。
2. 阶段 1。Z1 到 Z4 是客户端的地基，Z5 是唯一要先落地的服务端改动。
3. 阶段 2 的 N1 到 N4、N6 和阶段 3 的 W1 到 W3 可以并行：通知的“点击后打开”先用最简单的“打开地址”实现，W3 完成后换成统一规则。
4. 阶段 3 余下部分，然后阶段 4。
5. 阶段 5 的 A1、A2 可以提前，A3 之后的部分依赖 T0-8。
6. 阶段 6 的 Shell 界面工作不依赖客户端，可以从阶段 1 开始就并行；U8、U9 最后做。
7. N5、N7、N8 是服务端和各应用的工作，和客户端并行；N9 等 Q9。

设计稿定的顺序是 Review、多 Zone、Notify、Window System，上面的安排保持了这个主线。阶段 1 里提前做了最小的容器（主窗口顶层加载 Shell），因为没有它就看不到任何 Zone 的界面。

## 10. 接手时必须遵守的约束

- **许可证。** BuckyOSApp 是 Apache-2.0。不要把 buckyos 仓库（AGPL-3.0）的源码拷进去，不要让它依赖 `buckyos-api`、`kevent` 等 buckyos 仓库里的 crate。可以依赖 buckyos-base 的 crate 和 npm 包 `buckyos`。`src-tauri/src/node_control.rs` 是有记录的历史拷贝，不要再从 buckyos 同步更新（见 BuckyOSApp 的 README）。需要的协议类型按接口文档手写。
- **Git 依赖。** BuckyOS 家族仓库里，所有从 GitHub 引入的依赖都写分支名 `main`，不用 `rev`、提交 SHA 或 tag（见各仓库的 `AGENTS.md`）。
- **提交顺序。** websdk 先发布，然后是 buckyos，最后是 BuckyOSApp。BuckyOSApp 用的是 npm 上的 `buckyos` 版本。
- **兼容。** `window.BuckyApi` 旧的五个方法是对外接口，参数、返回结构、错误码不变。`buckyos.app-frame` 只做增量，版本号不变；新消息要让老 Shell 和老应用忽略后仍能工作。
- **工作树。** buckyos 的工作树里有别的在途改动，同一台机器上可能有别的会话在改同一棵树。动手前看 `git status`，不要用 `git stash` 做基线对比，不要回退不是自己改的文件。
- **文档同步。** 改了行为就同步对应的文档：BuckyOSApp 的 `doc/当前需求文档.md`（它的 §17 有维护规则）、`doc/BuckyApi.md`、`doc/托盘图标需求.md`；buckyos 的 `doc/message_hub/System Notify.md`、`product/desktop/` 下的 PRD、`doc/App 安装协议.md`。
- **验证。** 客户端相关的结论不能只凭单元测试。涉及 WebView、窗口、通知、音频的任务，要在对应平台的真机或虚拟机上跑过，并写明没有验证的平台。
- **待定问题。** §6 里“先答”标了“是”的问题没有答案时，不要自己选一个实现。其它问题按建议做，并在实施记录里写明用的是建议值。

## 11. 实施记录

实施时在这里追加：T0 的实测结论、每个任务的落点和验证结果、与正文不同的地方、新出现的待定问题。
