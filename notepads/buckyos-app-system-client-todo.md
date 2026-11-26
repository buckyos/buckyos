# BuckyOS APP 新版本实施 TODO：常驻系统客户端

日期：2026-11-23。状态：2026-11-24 全部任务已实施，**未提交**（L1 是 BuckyOSApp 分支 `feature/system-client` 上单独的一个提交，未推送），三 Zone 环境实测通过的范围和没验证的部分见 §11。owner 在 11-23 分两轮答复了 Q1–Q28，并定了 BuckyOSApp 改用 AGPL-3.0，本文已按答复改写。答复原话和落实位置在 §6.1。§6.2 列的是立项时我替 owner 选的做法，实施中新选的和等 owner 定的在 §11.5、§11.6。

依据：[BuckyOS APP 新版本系统设计](<../../BuckyOSApp/doc/buckyos app 新版本系统设计.md>)（下称设计稿）。设计稿第 3 章要求的“BuckyOS API 与现有代码 Review”就是本文 §2，后面的方案和任务都以这次 Review 为准。

涉及仓库：

- BuckyOSApp：Tauri 2 客户端（Rust + React）。现在是 Apache-2.0，owner 已定改成 AGPL-3.0（任务 L1）。
- buckyos：Desktop Web UI（`src/frame/desktop`）、control_panel、msg_center、verify_hub、kevent、buckyos-api，AGPL-3.0。
- buckyos-websdk：npm 包 `buckyos`，Apache-2.0，不变。

源码核对基线：BuckyOSApp `8399a73`（main）、buckyos `849bec4ee`、buckyos-websdk `16683a6`（0.7.202）。

§0 到 §10 的结论来自 2026-11-23 读代码和文档，写的时候没有运行客户端，也没有在真实 Zone 上验证。标了“待验证”的结论 T0 都已实测，结果在 §11.1，与正文不一致时以 §11 为准。

用词：

- **客户端**：BuckyOS APP 原生客户端。
- **应用**：装在 Zone 上的业务 APP，也包括 Message Hub、Files 这类系统应用。
- **本地界面**：客户端自带的界面，是 BuckyOSApp 里的 React 页面。
- **内容视图**：客户端里承载 Zone 页面的 WebView。桌面端是一个原生窗口，移动端是一个原生 WebView。
- **Shell**：Zone 提供的 Web UI，也就是 buckyos 的 Desktop Web UI，在浏览器里使用。
- **四种环境**：设计稿 §2.2 的桌面 Web UI、桌面 APP、移动 Web UI、移动 APP。

阅读顺序：§0 是结论，§1 是设计稿已经定下的方向，§2 是代码现状，§3 是方案，§6 是 owner 的答复和我替 owner 选的几处做法，§7 是任务，§10 是接手时必须遵守的约束。

## 0. 结论先行

1. **客户端现在比设计稿描述的旧版还要少。** APPS 页读的是本机的 `applist.json`，卡片不能点；Home 的“打开主页”是调系统浏览器；客户端不持有任何 Zone 的登录会话，也没有通知、深链接、开机启动。新版本的大部分能力要从头建。
2. **现有的 WebView 容器承载不了登录后的 Zone 页面。** 容器是“本地页面里套 iframe”。Zone 的会话 Cookie 是 `SameSite=Lax`，嵌在本地页面里的 Zone 页面属于跨站 iframe，请求不带 Cookie。Zone 页面必须是某个 WebView 的顶层文档（按 Cookie 规则推断，待 T0-1 实测）。
3. **总体结构（owner 已定，Q1）：应用窗口以外的界面由客户端自带。** Home、APPS、Activity、Settings、移动端的底部导航、“＋”和应用容器的系统 Header 都是本地界面。Zone 提供的只有应用页面；移动端的 Message Hub Tab 和 Home Station Tab 是例外，显示 Zone 的页面。客户端在没有绑定 Zone 时也能运行。
4. **界面有两套，代码可以共用。** 浏览器里仍然用 Shell，客户端里用本地界面。owner 定了 BuckyOSApp 改成 AGPL-3.0（和 buckyos 一致），改完以后客户端可以直接用 buckyos 仓库里的源码和 crate。两边都要用的东西只留一份（§3.1 的“共用代码放哪”）。websdk 仍是 Apache-2.0，AGPL 的代码不能搬进去。
5. **登录（owner 已定，Q2）：客户端自己登录，保存密码摘要自动重登，界面上不出现 Zone 的登录页。** verify-hub 存的就是这个摘要，保存它足够重登。通过客户端激活的 Zone，管理员密码摘要本来就是钱包密码算出来的（§2.4）。
6. **多个 Zone 的应用可以同时运行，所以客户端取任何资源都要先确定 Zone（Q4）。** “当前 Zone”只决定本地界面显示谁的内容，不能当作取资源时的隐含参数。
7. **应用发通知的接口已经有了。** msg-center 的 `notify.*`（System Notify）已经在用。缺的是客户端接收通道、系统提醒、普通应用的通知权限、Activity、汇总未读数。
8. **Android 的通知靠客户端自己保持长连接，不走 SN（Q9）。** 没有推送兜底：进程被系统停掉以后收不到通知，前台服务要常驻一条通知（R12）。
9. **现成的事件流没有鉴权。** `POST /kapi/kevent/stream` 不验 token。owner 定为另行安排（Q8），本文先用它，记为已知风险。
10. **系统应用大多没有独立的页面地址。** Shell 里只有 Message Hub、Home Station、AI Workspace、Task Center 有独立路由，Files、设置等是 Shell 文档里的面板，目录也只写在前端代码里。客户端要用原生窗口打开它们，Shell 得先补独立页面和一份清单（W3）。
11. **窗口规则在 Web UI 里已有大半，客户端要有同一套。** 缺四样：第三方应用不能声明窗口属性（AppDoc 没有字段）；PRD 定的“窗口 ID = AppID + 路径”没有实现；没有 `buckyos://` 处理；规则只写在 Shell 里，客户端用不上。
12. **移动端要自己管多个 WebView。** 本地界面一个，每个打开的 Zone 页面一个。按我对 Tauri 2 的了解它在移动端只管一个 WebView，其余要写原生插件（待 T0-7 确认）。这是移动端最大的一块新工作。
13. **`buckyos://` 的写法已定（Q11）。** 应用的 https 地址是规范形式，打开应用写成 `buckyos://app.open?…`。三份旧文档各说各的，owner 定以 §3.5 为准，由 W7 把它们改过来。代码里还没有任何实现。
14. **最不确定的是 WebView 和系统的实际行为。** Cookie、隐藏窗口里的长连接和播放、系统通知的点击回调、托盘悬停事件、Android 后台长连接能活多久，都要在 Windows、macOS、Android 上实测。所以 T0 排在所有实现之前。

## 1. 设计稿已定的方向

状态一栏照设计稿的说法：已明确、当前倾向、候选、未定。

| 编号 | 方向 | 状态 | 本文落实 |
| --- | --- | --- | --- |
| D1 | 桌面端先行，每项能力同时考虑移动端和 Web UI | 已明确 | 各阶段先做桌面，接口按四种环境设计 |
| D2 | 能力接口统一，界面各端自己适配；所有应用界面都要能在浏览器里运行 | 已明确 | §3.2 |
| D3 | 基础建设顺序：API 与代码 Review → 多 Zone → Notify 与 Activity → Window System | 已明确 | §2、§9 |
| D4 | 当前 Zone 是整个客户端的全局上下文；同一个 DID 登录多个 Zone 并切换；入口在 Home 的账号区 | 已明确 | §3.3 |
| D5 | 用明确切换，不做跨 Zone 内容聚合 | 当前倾向 | §3.3、§4 |
| D6 | 每个 Zone 一个主题色，只做小范围强调，明暗两套基础色不变 | 已明确，细节未定 | §3.3、Z9 |
| D7 | 应用发通知，系统负责各端触达；系统应用自带权限，普通应用要申请 | 已明确 | §3.4、N5 |
| D8 | 独立的 Activity，不并入 Message Hub | 当前倾向 | §3.4、N6 |
| D9 | 桌面客户端与 Zone 保持长连接收通知；移动端由客户端做平台适配 | 方向已明确 | N1、N7 |
| D10 | 桌面客户端是常驻的竖长小窗，配合托盘；托盘标记或闪动 | 交互方向 | N3 |
| D11 | Message Hub 保持完整的单窗口应用 | 已明确 | W2 的声明里是 `single` |
| D12 | 统一的打开规则：新建、定位已有、进入指定页面；支持单窗口和多窗口、Window ID、标准 URL | 方向已明确，格式未定 | §3.5 |
| D13 | 桌面 APP 用原生窗口承载网页；应用可声明固定大小、可调整、最大化、最小化、单窗口或多窗口 | 已明确 | W2、W4 |
| D14 | 应用窗口最小化后继续存在，点 × 就结束 | 当前倾向 | W4、W8 |
| D15 | 移动 APP 用全屏容器；系统 Header 有应用切换、返回、标题、搜索、更多、最小化；两种 Header 风格 | 已明确，细节未定 | §3.6 |
| D16 | 后台播放是通用能力；录音“统一采集再分发”是候选方案，验证后再定 | 候选 | §3.7、A4 |
| D17 | 桌面面板：Home、APPS（暂留）、Activity（新增）、Settings（暂留） | 已明确 | U1–U4 |
| D18 | 移动主导航：Home、Message Hub、＋、Home Station / Activity、Settings；Library 放下一版本 | 已明确 | C2、U5–U7 |
| D19 | 两条首次使用路径都保留：桌面安装后激活本机服务；手机配合无头盒子 | 已明确 | Z10 |

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
| 原生桥 `window.BuckyApi` | 插件把 `public/bucky-api.js` 注入所有 frame；脚本用 `window.parent.postMessage` 把请求交给外层容器页。五个方法：`getPublicKey`、`getCurrentUser`、`signJsonWithActiveDid`、`resolve_did`、`openExternalUrl`。签名时的密码框画在外层容器页里。websdk 把这几个方法包了一层（`walletSignWithActiveDid` 等），激活页经 websdk 调用 | `src-tauri/src/lib.rs:11`、`public/bucky-api.js`、`src/bridges/iframeBridge.ts`、`doc/BuckyApi.md`、`buckyos-websdk/src/sdk_core.ts:64-640` |
| 权限配置 | 只有一份 capability，对窗口 `main` 和 `webview_*` 生效，并且把所有 `http://*`、`https://*` 远程地址列了进去 | `src-tauri/capabilities/default.json` |
| 托盘 | 只有桌面。菜单是本机服务的状态和启停、打开 App、退出。左键右键都弹菜单。托盘启用时主窗口点 × 是隐藏，退出只能走菜单。桌面进程单实例 | `src-tauri/src/tray.rs`、`src-tauri/src/lib.rs:70-92`、`doc/托盘图标需求.md` |
| 身份与签名 | 助记词用密码加密后存在本机，每次签名都要输密码。签名返回里带 `pwd_hash`：用本地保存的 SN 用户名和这次输入的密码算出 | `src-tauri/src/did/`、`src/bridges/iframeBridge.ts:228` |
| 主题 | 明暗两套，存 localStorage `buckyos.theme`；Android 通过 `BuckySystemBars` 同步系统栏 | `src/theme.ts` |
| 移动端 | Android 工程有安全区和键盘处理，锁竖屏，权限只有 INTERNET。`MainActivity` 继承 `TauriActivity`，在 `onWebViewCreate` 里给 WebView 加了一个 JS 接口，说明工程里已经有原生扩展的做法。iOS 工程存在，没有发布 CI | `src-tauri/gen/android/` |
| 外部调用协议 | `buckyos://<action>?v=1&request_url=…&request_hash=…` 只有设计文档，代码里没有注册 scheme | `doc/BuckyOS外部调用协议设计.md` |
| 音频 | main 上没有。远程分支 `feature/audio_record`（2026-03，约 9000 行）做了录音到本地文件、Android 前台服务（`AudioRecordingForegroundService.kt`）、iOS 插件，并给 `BuckyApi` 加了录音方法。没有合入，main 在分叉点之后又走了一百多个提交 | `origin/feature/audio_record` |
| 许可证 | Apache-2.0，写在九处：两份 `LICENSE`、两份 `NOTICE`、README 的 License 一节、`package.json`、`Cargo.toml`、`tauri.conf.json` 的 `bundle.license`、`node_control.rs` 的 SPDX 头。README 写明 `node_control.rs` 是从 buckyos 拷来的历史副本，之后不能再同步 AGPL 的更新 | `LICENSE`、`NOTICE`、`README.md:34-38`、`package.json`、`src-tauri/LICENSE`、`src-tauri/NOTICE`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`src-tauri/src/node_control.rs:1` |

### 2.2 Shell（buckyos 的 Desktop Web UI）

路径都相对 `buckyos/src/frame/desktop/src`，另有说明的除外。

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 窗口管理 | `DesktopUIStore`。图标启动走 `openApp`：同一个 appId 只有一个窗口，已有就置前。`openAppWindow(appId, {windowId, newInstance, instanceKey, launch})` 用来开指定窗口或多开。有关闭前询问、最小化、最大化。打开的窗口不持久化，位置大小按 appId 存 localStorage | `models/DesktopUIDataModel.ts:1059-1418`、`models/ui.ts:253` |
| 窗口 ID | PRD §16.4 定的是“AppID + 路径”，没有实现。现在是 `${appId}-${时间戳}`，多窗口靠 `instanceKey` 区分 | `buckyos/product/desktop/BuckyOS_Web_Desktop_需求文档.md` |
| 窗口声明 | 有 `WindowManifest` 类型：`defaultMode`、`allowMinimize`、`allowMaximize`、`desktopWindow{width,height,minWidth,minHeight}`、`mobileStatusBarMode: compact\|standard` 等。内置应用的声明写死在目录里；第三方应用用一份固定默认值。AppDoc 没有窗口字段，第三方应用无法声明。没有“固定大小 / 可调整”开关，也没有单窗口 / 多窗口声明 | `models/ui.ts:132`、`mock/data.ts`（内置目录，真实模式也用它）、`app/backend-apps.ts:76-125`、`buckyos/src/kernel/buckyos-api/src/app_doc.rs` |
| 应用承载 | 内置应用是同一文档里的 React 面板。第三方应用每个窗口一个 iframe，地址是 `https://<web_hosts[0]>.<zone host>/`；加载前调 control-panel `auth.prepare_app_launch` 换一次性登录回调地址。最小化的窗口保持挂载 | `app/web-frame/WebAppFramePanel.tsx`、`app/web-frame/launch.ts` |
| 内置应用的独立页面 | 路由表里只有 `/messagehub`、`/homestation`、`/workspace`、`/taskcenter`、`/userprofile` 和几个登录、邀请页面。内置目录有二十多个应用（设置、Files、AI Center、市场、用户与 Agent 等），其余都只能在 Shell 的桌面里打开。目录本身只在前端代码里，后端没有接口给出 | `App.tsx:94-212`、`mock/data.ts` |
| 应用列表 | control-panel `apps.list`，返回 `AppSummary{app_id, app_instance_id, show_name, app_icon_url, web_hosts, runtime_type, …}`，只有安装的应用；Shell 在前端把它和内置目录合并 | `api/app_mgr.ts:36-63` |
| Shell 与应用的协议 | `buckyos.app-frame` v1，走 iframe postMessage。Shell 发给应用：`frame.init{windowId, formFactor, theme{mode, accent?}, locale, launch, shellCapabilities}`、`frame.open`、`frame.beforeClose`、`frame.back`，以及主题、语言、焦点变化。应用发给 Shell：`window.setTitle`、`window.requestClose`、`content.open`（按内容打开）、`sysdlg.open`。没有“按应用 ID 打开另一个应用”的消息。应用不在 Shell 里时（独立标签页）SDK 有回退：`content.open` 用 `window.open` 打开 Zone 根域的系统入口，文件选择器把 `/embed/sysdlg/select` 嵌在自己页面上 | `buckyos-websdk/src/app-frame.ts:229-252、401-412` |
| 打开入口 | Shell 根路径支持 `?open=`、`?settings=` 等查询参数。没有通用的“打开应用 X 的页面 Y”地址，也没有 `buckyos://` 处理 | `App.tsx`、`buckyos/doc/sdk/context ext.md:358` |
| 移动布局 | 宽度不超过 768px 时进入。顶部系统栏有返回、标题、“…”菜单、最小化；`standard` 是固定的一行，`compact` 是悬浮在应用上面。左侧抽屉里有全部应用和运行中的应用。全屏容器只挂载最上面的窗口，切到别的应用时原来的 iframe 被销毁。没有底部 Tab，没有应用切换视图，没有接浏览器历史 | `desktop/StatusBar.tsx`、`desktop/MobileTaskbar.tsx`、`desktop/windows/MobileWindowSheet.tsx` |
| 通知界面 | 消息、通知、任务三个托盘只在移动布局启用，宽屏桌面没有通知中心。没有用 Web Notification、Web Push、角标接口 | `desktop/DesktopRoute.tsx:852`、`desktop/tray/` |
| 小部件 | 时钟、便签、系统信息、音乐播放器。CPU、内存、磁盘、网络显示在任务栏里，来自 control-panel `system.metrics {lite:true}`。音乐播放器是一个 `<audio>` 加 Media Session，卸载时暂停 | `desktop/widgets/`、`desktop/widgets/MusicPlayerWidget.tsx:161` |
| 主题与外观 | 明暗两套 token，强调色 `--cp-accent` 写死。“设置 → 外观”里的主题、语言、图标大小、窗口、壁纸跟着账号走：存在账号的桌面状态 `appearance` 里（control-panel `/api/desktop`，system-config 的 `users/{user}/desktop/default/appearance`），每项一个字段，可以用 `X-Auth` 头带 token 读写。Zone 展示信息 `system/zone_presentation` 只有名称、描述、图标，`zone.presentation.get` 公开可读。`frame.init` 里已经有 `theme.accent` 的位置，Shell 没有填 | `index.css`、`desktop/appearance/model.ts`、`api/desktop_state.ts`、`buckyos/src/kernel/buckyos-api/src/system_config.rs:87` |
| 运行容器 | `runtimeContainer`（browser、desktop-app、mobile-app）是设置里的手动选项，不是检测出来的 | `models/DesktopUIDataModel.ts:417` |

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
| Activity | 没有。最接近的是移动托盘的归并逻辑，和 Message Hub 里“系统通知”的三个会话 | `src/frame/desktop/src/desktop/tray/trayModel.ts` |

### 2.4 登录、会话与多 Zone

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 运行环境判定 | websdk 看到 `window.BuckyApi` 就返回 `RuntimeType.AppRuntime`，否则是 `Browser`。`AppRuntime` 的实现直接继承 `Browser`，行为相同 | `buckyos-websdk/src/sdk_core.ts:886`、`src/runtime.ts:344` |
| 密码登录 | 密码摘要是 `Base64(SHA256(password + username + ".buckyos"))`（websdk `hashPassword(username, password)`）。verify-hub 存的就是这个值；登录时客户端发 `SHA256(摘要 + login_nonce)`，服务端用同样的算法比对，nonce 要接近当前时间且只能用一次。有两个入口：verify-hub 的 `login_by_password`，和 control-panel 的 `auth.login`。`auth.login` 不带 `redirect_url` 时要传结构化的 `target`，直接返回 token 对；带 `redirect_url` 时把 token 对存成一条待取的登录，返回 `sso_nonce`，页面随后访问目标源的 `/sso_callback?nonce=…` 写 Cookie | `buckyos-websdk/src/account.ts:64`、`buckyos/src/kernel/verify_hub/src/main.rs:1096-1160`、`buckyos/src/frame/control_panel/src/sys_auth_backend.rs:165-247` |
| 激活时的管理员密码 | 通过客户端激活时，激活页用钱包签名返回的 `pwd_hash` 作为 Zone 的 `admin_password_hash`。也就是说，这类 Zone 的管理员登录摘要等于“钱包密码 + SN 用户名”算出的摘要。在网页上激活的 Zone 用的是激活页里输入的密码 | `buckyos/src/kernel/node_active/active_lib.ts:455-658` |
| 浏览器登录 | 跳到 `https://sys.<zone>/login`，成功后在目标源的 `/sso_callback` 写 Cookie：`buckyos_session_token`（15 分钟）、`buckyos_refresh_token`（HttpOnly，7 天）。两个都是 `SameSite=Lax`，不带 Domain，只对当前主机有效。SDK 通过 `/sso_refresh` 续期。会话族（一次登录签出的一串 refresh token，代码里叫 family）已经持久化 | `sys_auth_backend.rs`、`buckyos/src/kernel/verify_hub/src/family_store.rs` |
| 把会话带给页面 | `auth.prepare_app_launch{app_instance_id \| service_id, redirect_url}`。调用方必须是 control-panel 的用户会话，返回目标主机上的一次性登录回调地址。目标只能是应用实例，或者 `aiworkspace`、`nfs-server` 这两个系统服务；Zone 根域自己的页面（Message Hub、Home Station、设置）不在范围里 | `sys_auth_backend.rs:251-378`、`buckyos/src/kernel/buckyos-api/src/app_availability.rs:86` |
| 多个 Zone 并存 | Cookie 和本地存储都按主机隔离，同一个 WebView 里可以同时登录多个 Zone。SDK 的全局运行时在一个页面里只有一个当前 Zone，从 `location.host` 得出 | `buckyos-websdk/src/sdk_core.ts:800` |
| 用钥匙登录 | verify-hub `login_by_jwt` 只接受本 Zone 认识的签发者：`root`（Zone owner 的钥匙）、本地用户、设备、Agent。本版本不用（Q2） | `buckyos/src/kernel/verify_hub/src/main.rs:660-731` |
| 跨 Zone 登录 | 浏览器联合登录：目标 Zone 调 `auth.did.begin`，跳到 home Zone 的 `/sso/federate` 确认。没有本地账号的 DID 得到的是 External 会话，没有密码。`user.invite.create / accept` 可以把一个 DID 变成目标 Zone 的本地账号 | `buckyos/src/frame/control_panel/src/federation.rs`、`notepads/done/hybrid-sso-implementation-todo.md` |
| 可登录的 Zone 名单 | 只有 owner 文档的 `binded_zone_list`，是自己绑定的 Zone | `buckyos-base/src/name-lib/src/user.rs:86` |
| 应用地址 | Zone owner 装的应用是 `https://<appid>.<zone host>/`，其他用户装的是 `https://<appid>-<userid>.<zone host>/` | `buckyos/src/kernel/buckyos-api/src/app_schema.rs:166`、`buckyos/doc/key url.md` |

### 2.5 能直接复用的部分（Q19）

owner 问 websdk 里是否有可复用的组件。核对结果：协议和数据层有，界面组件没有。BuckyOSApp 改成 AGPL-3.0 以后，buckyos 仓库里的代码也可以用，所以分两张表。

**websdk（Apache-2.0）。** 位置相对 `buckyos-websdk/src`。

| 部分 | 位置 | 客户端怎么用 |
| --- | --- | --- |
| kRPC 客户端，可以注入 `fetcher` 和 token 来源 | `krpc_client.ts:77` | 本地界面对任意 Zone 发请求。`fetcher` 接到 Rust 侧（Z2），token 不进页面 |
| 带类型的服务客户端：`MsgCenterClient`（含 `notify.*` 和会话）、`TaskManagerClient`、`VerifyHubClient`、`SystemConfigClient`，构造时传 kRPC 客户端 | `msg_center_client.ts`、`task_mgr_client.ts`、`verify-hub-client.ts`、`system_config_client.ts` | 本地界面直接用，不手写类型 |
| `hashPassword` | `account.ts:64` | 算登录摘要 |
| app-frame 协议：`AppFrameHost`、`AppFrameClient`、消息和 `OpenRequest` 类型 | `app-frame.ts` | 客户端当宿主时照这份定义收发；应用侧不用改 |
| 内容登记和解析：`ContentRegistryClient`、`resolveContentHandlers` | `content.ts` | `content.open` 的解析直接用，客户端只写把结果交给打开规则的那一层 |
| 对象地址解析 `objectUrl` | `object_url.ts` | 通知链接、内容引用 |

**buckyos 仓库（AGPL-3.0，L1 之后可以用）。** Desktop 的位置相对 `buckyos/src/frame/desktop/src`。

| 部分 | 位置 | 用之前要处理的 |
| --- | --- | --- |
| control-panel 接口封装：应用列表、账号桌面状态、系统状态、应用启动、Zone 展示信息 | `api/app_mgr.ts`、`api/desktop_state.ts`、`api/system_metrics.ts`、`api/app_launch.ts`、`api/settings.ts` | 都经 `api/rpc.ts` 调 websdk 的全局运行时（一个页面一个 Zone），并带 mock 分支。要先改成请求通道由调用方传入 |
| 托盘的归并和未读口径：消息、通知、任务合成一份快照 | `desktop/tray/trayModel.ts`、`desktop/tray/trayRuntime.ts` | 类型、路径函数和 React hook 写在同一个文件里，取数走全局运行时。Activity 要的是不含 React 的那部分 |
| 外观模型、内置应用目录、窗口声明的类型 | `desktop/appearance/model.ts`、`mock/data.ts`、`models/ui.ts` | 目录和图标、各语言名称绑在一起，W3 导出清单时一起整理 |
| 把内容解析结果接到窗口的那一层 | `app/content/open.ts` | 绑着 Shell 的窗口管理和预览，客户端照着写自己的 |
| 小部件：时钟、便签、系统信息、音乐播放器 | `desktop/widgets/` | 依赖 Shell 的桌面网格和同源 Cookie，要移植 |
| 设计 token 和明暗主题 | `index.css`、`theme/provider.tsx` | 本地界面用同一套，观感和 Shell 一致 |
| Rust：verify-hub 客户端（`login_by_password`、`refresh_token`）、通知的类型、msg-center 客户端、AppDoc 类型 | `buckyos/src/kernel/buckyos-api/src/verify_hub_client.rs`、`msg_notify.rs`、`msg_center_client.rs`、`app_doc.rs` | `buckyos-api` 是一个大 crate，带 vendored openssl、bundled rusqlite、sysinfo、named_store、ndn-lib 等依赖，客户端现在的依赖树里没有 openssl。在 Windows、macOS、Android 上能不能编译、安装包大多少，T0-10 先试 |

两边都没有的：

- 按 Zone 建一组客户端的入口。websdk 的全局运行时 `initBuckyOS` 假定一个页面一个 Zone，本地界面不能用它（W1 补）。
- 从 Zone 外面连 kevent 流的 Rust 客户端。`buckyos-api` 的 `kevent_client.rs` 只有共享内存和连本机 node-daemon 两种通道，N1 要自己写读 HTTP 流的部分。
- control-panel 服务 `auth.login`、`auth.prepare_app_launch`、`apps.list` 的 Rust 客户端。`buckyos-api` 的 `ControlPanelClient` 读的是 system-config，不是这个服务，这几个调用在 Rust 里要自己拼 kRPC 请求。
- websdk 里的界面组件。websdk 没有 React 组件；会话界面在 Desktop 里，对第三方是 `/embed/conversation` 页面。

### 2.6 能力清单与四种环境

| 能力 | 桌面 Web UI | 桌面 APP | 移动 Web UI | 移动 APP | 结论 |
| --- | --- | --- | --- | --- | --- |
| 运行环境识别 | 有（Browser） | 只知道在客户端里 | 有（Browser） | 只知道在客户端里 | 补容器类型和能力清单 |
| 当前 Zone 上下文 | 由页面地址决定 | 没有 | 由页面地址决定 | 没有 | 客户端新增 |
| Zone 切换 | 没有 | 没有 | 没有 | 没有 | 客户端新增 |
| 应用发通知 | 有 | 有，同一接口 | 有 | 有，同一接口 | 保留，补权限 |
| 系统提醒 | 没有 | 没有 | 没有 | 没有 | 客户端新增 |
| 通知中心、Activity | 没有 | 没有 | 只有铃铛托盘 | 没有 | 客户端自带；Web UI 本版本不做（Q27） |
| 打开应用、定位已有窗口 | 有，只支持单窗口规则 | 没有 | 有 | 没有 | 规则写进 websdk，两边共用 |
| 窗口声明 | 只有内置应用 | 没有 | 只有内置应用 | 没有 | 放进 AppDoc |
| 多窗口与窗口 ID | 部分（`instanceKey`） | 没有 | 不适用 | 不适用 | 补全 |
| 通过地址进入页面 | 部分（查询参数） | 没有 | 部分 | 没有 | 标准地址已定（§3.5） |
| 移动容器 Header | 不适用 | 不适用 | 有，缺应用切换、搜索、第二行 | 没有 | 客户端自带；Web UI 本版本不做（Q27） |
| 移动端应用保活 | 不适用 | 不适用 | 没有 | 没有 | 客户端用多个内容视图 |
| 后台播放 | 页面开着就能播 | 待验证 | 待验证 | 没有 | 新增能力 |
| 录音 | 应用内 `getUserMedia` | 分支原型 | 应用内 `getUserMedia` | 分支原型 | 待验证 |
| Zone 主题色 | 没有 | 没有 | 没有 | 没有 | 新增 |

### 2.7 与新要求冲突的现有行为

1. **iframe 容器和登录 Cookie。** 见 §0 第 2 条。影响桌面和移动两端的容器做法。
2. **原生桥只能过一层 `window.parent`。** 新结构里应用是内容视图的顶层文档，没有 parent，现在的 `bucky-api.js` 失效。
3. **签名密码框画在外层容器页里。** Zone 页面变成顶层文档后没有外层页。密码不能画进远程页面，否则页面脚本能读到。
4. **capability 对所有远程地址开放。** 应用页面成为顶层文档后，远程页面可能可以直接调用全部 Tauri 命令，包括切换身份、停止本机服务（按 Tauri 2 的权限模型推断，待 T0-2 验证）。
5. **设计稿对旧版的描述与代码不符。** 设计稿 §1.1 说旧版会读取所绑定 Zone 的应用列表并启动应用，代码里 APPS 页读的是本机文件，不能启动。
6. **主窗口固定 400×800。** 不能调高度，也不记位置。
7. **托盘左键就弹菜单。** 新行为见 Q14。
8. **`buckyos://` 有三种写法。** owner 已定以 §3.5 为准。
9. **系统应用没有独立页面地址，目录只在前端。** 见 §0 第 10 条。
10. **`auth.prepare_app_launch` 不支持 Zone 根域的页面。** Message Hub、Home Station 都在根域。
11. **Message Hub 的新消息不产生通知记录。** Activity 如果只读 `notify.list`，就看不到消息。
12. **Home 的小部件写在 Shell 里。** 它们依赖 Shell 的桌面网格和同源 Cookie，客户端的 Home 要移植才能用。

## 3. 方案

本章的做法 owner 都已确认（§6.1）。少数几处是我在这些决定之下自己选的，列在 §6.2。

### 3.1 总体结构：本地界面加内容视图（Q1）

| 归属 | 内容 |
| --- | --- |
| 客户端的本地界面 | 桌面主面板的 Home、APPS、Activity、Settings；移动端的底部导航、Home、“＋”、全部动态、Settings、应用容器的系统 Header 和应用切换；账号、设备发现与激活、Zone 名单与切换、登录表单；托盘、系统通知、托盘悬停小窗 |
| Zone 的页面，显示在内容视图里 | 所有应用，包括 Message Hub、Home Station、Files、Zone 设置等系统应用；移动端 Message Hub Tab 和 Home Station Tab 里的内容 |
| Shell | 继续服务浏览器里的桌面 Web UI 和移动 Web UI，规则与客户端一致 |

本地界面要的 Zone 数据（应用列表、系统状态、通知、会话、任务）由客户端用自己的会话直接调 Zone 的接口取得，不经过任何 Zone 页面。

每个内容视图顶层加载一个 Zone 页面：

| 环境 | 内容视图 |
| --- | --- |
| 桌面 APP | 每个应用窗口是一个原生窗口。主面板窗口只显示本地界面 |
| 移动 APP | 主 WebView 显示本地界面。每个打开的 Zone 页面是另一个原生 WebView，摆在系统 Header 和底部导航之间。这些 WebView 由一个原生插件管理（C1） |

这个结构的几个后果：

1. **界面有两套，共用的代码只留一份。** 本地界面和 Shell 的布局不同：一个是竖长的面板和手机界面，一个是网页桌面。页面各写各的；两边都要用的规则、数据处理和组件不重复实现，放在哪见下面的“共用代码放哪”。
2. **没有 Zone 也能运行。** 没有绑定或登录任何 Zone 时，Home 显示账号、设备发现和“添加 Zone”，其余 Tab 显示空状态。
3. **Zone 版本不同时好处理。** 本地界面只依赖 Zone 的接口。某个接口在老 Zone 上没有时，对应的那一块显示“这个 Zone 需要升级”，其它照常。
4. **系统应用要有独立页面。** 见 W3。
5. **移动端浮层要处理层叠。** 本地界面的菜单、应用切换、“＋”面板要盖在内容视图上面，而内容视图是另一个原生 WebView（T0-7 定做法）。

**共用代码放哪。** BuckyOSApp 改成 AGPL-3.0（L1）之后，许可证不再限制客户端使用 buckyos 的代码。规则是每样东西只留一份，放在它现在所在的地方：

| 内容 | 放在 | 客户端怎么拿到 |
| --- | --- | --- |
| 应用也要用的：app-frame 协议、`OpenAppRequest`、能力清单、音频接口 | websdk | npm 包 `buckyos`，和现在一样 |
| 本文新写、不含 Desktop 现有代码的宿主逻辑：打开规则和窗口 ID、按 Zone 建一组客户端的入口 | websdk | 同上 |
| Desktop 里已有、两边都要用的：control-panel 接口封装、托盘和 Activity 的归并、外观模型、系统应用清单、设计 token、能直接用的组件和小部件 | 留在 buckyos 的 Desktop 里，把客户端要用的部分整理成不依赖 Shell 全局状态的模块 | 客户端引用这份源码，不拷贝。引用方式由 T0-10 定 |
| Rust 的类型和服务客户端 | `buckyos-api` | Cargo 的 Git 依赖，分支 `main`。前提是 T0-10 确认它在各目标平台上能编译 |

websdk 仍是 Apache-2.0，第三方应用依赖它。buckyos 和改完许可证的 BuckyOSApp 里的代码不能搬进 websdk，websdk 也不能依赖 AGPL 的包。

### 3.2 应用接口分层与运行环境识别

应用只直接使用 websdk。websdk 下面有两条通道，由 SDK 自己选：

| 层 | 现状 | 新版本 |
| --- | --- | --- |
| websdk | 运行环境只分浏览器和客户端 | 增加运行环境描述：容器类型（`browser`、`desktop-app`、`mobile-app`）、外形（`desktop`、`mobile`）、能力清单。应用按能力清单决定用什么功能 |
| 宿主协议 `buckyos.app-frame` | 只有 iframe postMessage 一种传输，宿主是 Shell | 增加原生传输：应用是内容视图的顶层文档时，同样的消息经 `BuckyApi` 收发，宿主是客户端。新增 `app.open`（打开另一个应用或页面） |
| 原生桥 `window.BuckyApi` | 五个方法，靠 `window.parent` | 改成直接调原生。增加 `BuckyApi.env`（同步可读：容器类型、平台、客户端版本、能力清单）、通用入口 `BuckyApi.call(name, payload)` 和事件订阅 `BuckyApi.on(event, handler)`。`invoke` 这个名字留给《外部调用协议》 |

能力清单的名字在实施时定，至少包括：`window.native`（原生窗口）、`notify.system`（系统提醒）、`app.open`、`audio.background`、`audio.record`。同一个能力在不同环境里可用就列出，不可用就不列；应用不靠猜平台来判断。

客户端当宿主时要处理 Shell 现在处理的那些消息：

| 消息 | 客户端的做法 |
| --- | --- |
| `frame.init`、主题、语言、焦点变化 | 客户端提供。`theme.accent` 填这个 Zone 的主题色 |
| `window.setTitle`、`window.requestClose`、`frame.beforeClose`、`frame.back` | 对应原生窗口或移动容器的标题、关闭、返回 |
| `app.open` | 交给打开规则（§3.5） |
| `content.open` | 用内容登记解析出处理它的应用，再走打开规则。解析用 websdk 已有的 `resolveContentHandlers`；Shell 里把结果接到窗口的是 `app/content/open.ts`，客户端写自己的这一层（W5） |
| `sysdlg.open`（文件选择器） | 第一版沿用 SDK 已有的独立页面回退：应用把 Zone 根域的 `/embed/sysdlg/select` 嵌在自己页面上。它和应用页面同站，Cookie 可用 |

内容视图发来的 `BuckyApi` 调用，属于哪个 Zone、哪个应用由客户端按视图认定，不采信页面传的参数。

### 3.3 多 Zone

**取资源先定 Zone（Q4）。** 多个 Zone 的应用可以同时开着，所以客户端里每一条取资源的路径都带 Zone DID：Rust 命令、会话、缓存、通知记录、窗口 ID、内容视图。任何函数都不从“当前 Zone”取隐含参数。“当前 Zone”只有两个用途：决定本地界面显示哪个 Zone 的内容；作为新建操作（“＋”、从本地界面打开应用）的默认 Zone。内容视图在创建时绑定一个 Zone，之后不变。

**数据分三层。** 账号级：DID 钱包、每个 DID 的 Zone 名单和当前 Zone。客户端级：语言、明暗、托盘、开机启动、通知总开关。Zone 级：登录凭据和会话、主题色缓存、通知开关。用户在某个 Zone 里的界面偏好存在该 Zone 的账号外观配置里（Q20），跟着账号走。

**Zone 名单的来源（Q3）。** 两个：当前 DID 的 owner 文档里的 Zone 列表；用户在设置里手动添加。手动添加时先登录一次，成功才写进名单。每个 Zone 记录：DID、主机名、名称、图标、主题色、登录用户名、会话状态。

**登录（Q2）。** 客户端自己登录，界面是本地的，不加载 Zone 的登录页。

1. 凭据是用户名加密码摘要（§2.4）。verify-hub 存的也是这个摘要，所以保存它就能自动重登，不用保存密码本身。摘要存系统钥匙串，按“DID + Zone”分开。
2. 自己的 Zone（来自 owner 文档）：通过客户端激活的 Zone，管理员密码摘要等于钱包密码和 SN 用户名算出的摘要。客户端在用户输入钱包密码的时候（创建或导入身份、签名）顺带算出并保存，第一次登录直接用它。登录失败（Zone 是在网页上激活的，或者之后改过密码）就在本地界面问一次这个 Zone 的密码。
3. 手动添加的 Zone：本地表单填 Zone 地址、用户名、密码。
4. 登录得到 control-panel 的用户会话，由 Rust 侧持有并续期。续期失败就用保存的摘要重新登录；摘要也不对了才打扰用户。走 verify-hub 的 `login_by_password` 还是 control-panel 的 `auth.login`，由 T0-3 定。
5. 不用钥匙登录。在目标 Zone 没有密码的账号（联合登录得到的 External 身份、Google 登录）第一版进不了客户端，owner 已确认（Q22）。添加 Zone 的表单上写明要用这个 Zone 的用户名和密码。

**让内容视图登录。** 内容视图里的 Zone 页面靠 Cookie 认会话。加载页面之前，客户端用自己的会话换一个一次性的登录回调地址（`/sso_callback?nonce=…`），让内容视图先访问它，Cookie 写好后自动跳到目标页面。

- 应用页面用现成的 `auth.prepare_app_launch`。
- Zone 根域的页面不在它的范围里。先用 `auth.login` 带 `redirect_url` 的做法：它返回 `sso_nonce`，和登录页走的是同一条路，客户端有摘要就能调（T0-3 验证）。不合适再做 Z5。
- 内容视图的 Cookie 失效后，页面会跳去 `sys.<zone>/login`（根域的页面跳 `<zone>/login`）。客户端拦下这个跳转，重新换回调地址再加载，用户看不到登录页。
- 这样每个内容视图拿到的是独立的会话族，和客户端自己的会话互不影响。refresh token 每次续期都会轮换，共用一组会互相踩。

**切换（Q4、Q5）。** 切换当前 Zone 后，本地界面改显示目标 Zone 的内容。

| 对象 | 桌面 APP | 移动 APP |
| --- | --- | --- |
| 原 Zone 已打开的应用 | 窗口不关，仍属于原 Zone，窗口上显示所属 Zone 的名称和主题色 | 全部结束。切换前对有未保存内容的应用走 `frame.beforeClose` |
| 原 Zone 的播放、录音 | 窗口还在就继续 | 随应用结束而停止，切换前提示 |
| 非当前 Zone 的通知 | 照常接收和弹出，标明来自哪个 Zone | 同左 |
| 点击其他 Zone 的通知 | 直接打开那个 Zone 的应用窗口，不改当前 Zone | 先切换到那个 Zone，再打开 |

**主题色（Q20、Q23）。** owner 的答复是“系统外观配置”，并确认了下面的落实：入口在 Zone 的“设置 → 外观”和客户端的 Zone 管理页；值存在账号的外观状态里，加一个 `accent` 字段，和主题、语言一样跟着账号跨设备。也就是说主题色是每个用户给自己的每个 Zone 各设一个，不是管理员给全 Zone 定一个。客户端登录后读它，缓存在 Zone 记录里，用在 Zone 列表、应用窗口的 Zone 标识、托盘菜单，并通过 `frame.init.theme.accent` 传给应用。只做小范围强调，明暗两套基础色不动。

### 3.4 Notify 与 Activity

**应用侧不变。** 应用继续调 `notify.send`。要补的是下面几件。

**客户端通道。** 连接放在客户端的 Rust 侧，每个开了通知的已登录 Zone 一条。用现有设施：订阅 kevent 流里该用户的 msg-center 会话事件和任务事件，收到事件就拉 `notify.list` 和 `msg.list_sessions`，另加低频轮询兜底。kevent 流没有鉴权，owner 定为另行安排（Q8），本文不做带鉴权的新通道。

**什么事件弹系统提醒（Q6）。**

| 来源 | Activity | 系统提醒 |
| --- | --- | --- |
| 通知，`quiet` | 进 | 不弹 |
| 通知，`normal` | 进 | 弹，无声 |
| 通知，`timely` | 进 | 弹，有声 |
| 通知，`urgent` | 进 | 弹，有声，托盘闪动，直到处理 |
| Message Hub 新消息（会话没有免打扰） | 每个会话一行 | 弹，同一会话合并；该会话正显示在聚焦的窗口里时不弹 |
| 带 `system_notify` 的任务 | 进 | 只在完成或失败时弹 |

**点击去哪。** 通知优先用 `source.link`（应用自己的页面地址），没有就用通知的统一链接 `/messagehub?notify=<id>`；消息去 `/messagehub?entityId=…&sessionId=…`；任务去 `/taskcenter?taskid=…`。这些地址都交给 §3.5 的统一打开规则。

**权限（Q7）。** 应用在 AppDoc 的 `permissions` 里声明要发通知，安装时展示，用户可以在设置里按应用关闭。msg-center 在 `notify.send` 时对普通应用检查声明和用户开关，系统服务和内置应用不受限。这是 breaking change：没有声明的应用发通知会被拒绝，不照顾旧应用。操作系统的通知许可由客户端在第一次开启通知时申请，客户端设置里有总开关和每个 Zone 的开关。

**Activity。** 是本地界面的一个页面，按时间倒序合并三类内容：通知（`notify.list`，全部状态，游标翻页）、有未读的会话（每个会话一行）、带 `system_notify` 的任务。归并逻辑从 Shell 托盘现有的归并（`trayModel.ts`、`trayRuntime.ts`）整理出来，Shell 和客户端用同一份，不建新存储。它和 Message Hub 里的“系统通知”读的是同一批记录，两处都保留（Q10）。

**汇总数。** msg-center 加一个汇总接口，返回未处理的通知数和消息未读总数。托盘角标、Tab 角标都用它。

**Android（Q9）。** 不走 SN，也不用 FCM。客户端起一个前台服务，在后台保持和桌面端同一种长连接，收到事件后发系统通知。连接放在 Rust 里还是 Kotlin 服务里，由 T0-9 定。owner 的原话是“WebSocket 长链”；现有的事件流是 HTTP 流，不是 WebSocket。owner 已确认先用现有的流（Q24）；要不要另做 WebSocket 端点，随 Q8 另行安排。

**托盘（Q14）。** 现有托盘继续管本机服务。新增未读角标、紧急通知时闪动、菜单里列最近几条通知。有未读时左键打开主窗口的 Activity，右键弹菜单；没有未读时保持现状。有未读时鼠标悬停在托盘图标上，弹出一个小窗显示最近几条通知的内容，点其中一条就打开它，鼠标离开后收起。拿不到悬停事件的平台退回成 tooltip 文字（T0-4）。

### 3.5 Window System 与标准地址

**一个打开请求。** 不论来自图标、Activity、系统通知、别的应用还是外部链接，都先变成同一个请求：

```ts
interface OpenAppRequest {
  zone: string            // Zone DID；必填，没有“当前 Zone”这种缺省
  app: string             // app_id；系统应用用它在清单里的 id，例如 messagehub
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

规则在 websdk 里只写一份 TypeScript 实现，带一份测试向量（JSON）。Shell 和客户端的本地界面都调它；客户端的 Rust 侧只提供建窗口、置前、导航这些操作。如果 T0-5 发现主窗口隐藏时本地页面不可靠，再在 Rust 里按同一份测试向量实现一遍。

**标准地址（Q11，owner 已定）。** owner 的决定：按下面三条做，并且以这里的设计为准。旧文档里和这三条不一致的写法都作废，由 W7 改过来；文档改完之前，实现照这里做。

1. 应用页面的规范地址是它的 https 地址，`key url.md` 已经把它列为稳定 URL。它在浏览器里就是普通链接；在客户端里，凡是指向名单内 Zone 的应用地址都被截下来，反查出应用，变成打开请求。
2. `buckyos://` 的语法只认《外部调用协议》一份。打开应用写成 `buckyos://app.open?v=1&zone=<zone 主机>&app=<app_id>&path=…&window=…`。`context ext.md` 里的 `buckyos://app/<app_id>` 改成这个写法，`key url.md` §2 加一句指向《外部调用协议》。
3. `app.open` 只做导航，不带 `request_url`。在《外部调用协议》里加一条：只做导航、不签名也不写入的 action 可以不带 request object。目标 Zone 不在名单里或应用不存在时拒绝。

定案之前各文档的写法如下，留给 W7 改文档时对照：

| 文档 | 写法 |
| --- | --- |
| `buckyos/doc/key url.md` §2（自称 URL 规范的唯一规范源，11-20） | 只有一句：`buckyos://` 是“我们的扩展，用来拉起 current zone / buckyos app 的特定流程”，没有语法。同一文档把应用地址 `https://$appid.$zonehost/` 和 `https://$appid-$userid.$zonehost/` 列为稳定 URL，并规定跨 Zone 引用一律用 https |
| `BuckyOSApp/doc/BuckyOS外部调用协议设计.md`（06-26） | 有完整语法：`buckyos://<action>?v=1&request_url=…&request_hash=…`。host 就是 action，至少两段（`<namespace>.<verb>`）；`app.*` 是保留给“打开或路由到 BuckyOS app”的命名空间；第一版必须带 `request_url` 和 `request_hash`；计划的第一个 action 是 `sign.payload` |
| `buckyos/doc/sdk/context ext.md` §8（11-10） | `buckyos://app/<app_id>?...`，表示拉起应用页面。host 是单段的 `app`，不符合上一份文档的 action 命名规则 |
| `BuckyOSApp/doc/第三方签名接入需求.md`、`第三方签名端到端设计.md`（06-18） | 更早的 `buckyos://sign?request_id=…`，已被外部调用协议的 `sign.payload` 取代 |
| 代码 | 没有任何地方注册或解析这个 scheme；`buckyos-api` 的 `object_url.rs` 只是原样保留 |

**窗口声明（Q12）。** 放进 AppDoc 的 `window` 段，经 `apps.list` 带给 Shell 和客户端。字段沿用 Shell 现有的 `WindowManifest`，补上缺的两项：

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

**系统应用。** Shell 给每个内置应用一个独立页面地址，并在构建时导出一份清单（id、各语言的名称、图标图片地址、页面地址、窗口声明）。客户端读这份清单，和 `apps.list` 合起来就是这个 Zone 的全部应用。

**四种环境各自怎么落实。**

| 环境 | 窗口或容器 | 关闭与最小化 |
| --- | --- | --- |
| 桌面 Web UI | Shell 现有的网页内窗口，按新规则和新声明调整 | 现有行为 |
| 桌面 APP | 客户端开原生窗口，顶层加载应用地址，按声明设置大小和按钮 | 最小化保留；点 × 先问 `frame.beforeClose`，然后销毁 |
| 移动 Web UI | Shell 现有的全屏容器 | 最小化回 Home；关闭销毁 |
| 移动 APP | 客户端的全屏容器：本地的系统 Header 加一个内容视图 | 同上 |

内容视图里的页面要跳到名单内 Zone 以外的地址时，交给系统浏览器打开，视图本身不跳转。这样原生桥不会暴露给任意网站。

### 3.6 移动容器

移动 APP 的容器是本地界面加内容视图，由客户端实现。

- **内容视图和保活。** 每个打开的应用一个内容视图。切到别的应用时原来的视图只是隐藏，不销毁；设数量上限，超过后淘汰最久没用的，被淘汰的应用回来时重新加载。
- **Header。** 本地界面画。两种风格对应窗口声明里的 `standard`（固定底色）和 `compact`（透明或半透明）。元素有应用切换、返回、标题、搜索、更多、最小化。固定底色风格支持第二行和搜索入口，由应用通过 app-frame 声明和更新。透明风格不追加第二行。
- **返回。** Android 的系统返回键先到客户端的返回栈，再到当前应用的 `frame.back`。
- **底部导航。** Home、Message Hub、＋、Home Station / Activity、Settings。Message Hub Tab 和 Home Station Tab 的内容各是一个内容视图，加载 Zone 的 `/messagehub` 和 `/homestation`。Tab、Home 里的图标、通知唤起用的是同一个视图（Q15）。这两个页面放在 Tab 里时要去掉自己的外层导航，由 `frame.init` 告诉页面当前的呈现方式。
- **签名。** 密码输入用原生对话框。

移动 Web UI 仍然用 Shell 的移动布局，本版本不给它补保活和 Header（Q27）。

### 3.7 后台音频

- **播放。** 声音仍由应用页面里的 `<audio>` 发出，客户端只负责让它不被系统停掉。桌面端的预期是应用窗口最小化后播放不受影响（待 T0-5 验证）。移动端需要原生配合：Android 起媒体播放类型的前台服务并显示媒体通知。对应用是一对接口：申请保活、释放保活。如果 T0-8 发现 WebView 里的音频在后台撑不住，就改成原生播放器，接口不变。
- **多个应用同时播放（Q17）。** 第一版不做协调，各播各的，由系统混音。
- **录音。** 先把 `feature/audio_record` 的原生录音移植到当前 main，整理成 `audio.record.*` 接口。它是“一个应用录到一个本地文件”的模型。
- **统一采集再分发（Q16）。** 设计稿的候选方案。先做可行性验证（A4），看各平台的后台限制、延迟、权限提示归谁，再决定做不做。
- **Home 音乐小部件。** 从 Shell 的 `MusicPlayerWidget.tsx` 移植。它从本地页面播放 Zone 里的文件，而本地页面带不上 Zone 的 Cookie，媒体地址要另想办法（带令牌的地址，或者经 Rust 转发）。放到音频阶段的最后，第一版 Home 不带它（Q26）。

### 3.8 两端主界面

主界面都是本地界面。

- **桌面主面板。** 四个 Tab：Home、APPS、Activity、Settings。主窗口保持竖长，可以调高度并记住位置。
  - Home 从上到下：账号区（含 Zone 切换）、系统状态卡片（`system.metrics`）、常用应用快捷方式（可添加和移除）。第一版不带别的小部件（Q26）。
  - APPS：当前 Zone 里当前用户可用的全部应用，来自 `apps.list` 加系统应用清单。
  - Settings：客户端自己的设置（语言、明暗、通知、开机启动、Zone 管理、身份与备份），另有一项“当前 Zone 的设置”，用应用窗口打开 Zone 的设置页面（Q25）。
- **移动端。** Home 以应用启动为主。“＋”弹出快速创建菜单，预置发消息、发动态、加笔记、传照片四项，创建发生在当前 Zone；每一项是一个指向处理应用的打开请求。目标应用（Q28）：发消息给 Message Hub，发动态给 Home Station，传照片给 Files。内置目录里没有笔记应用，“加笔记”先交给 AI Workspace 新建文档（我选的，见 §6.2）。四项的目标写成一处配置，换应用只改这一处。`doc/sdk/context ext.md` 定义了 `create` 这个意图，但排在它的 P2，实施时先确认登记表是否已经支持，不支持就先把四项写死。
- **动态入口。** 默认 Home Station（内容视图），可以切到全部动态（本地的 Activity）。选择存在账号外观配置里（Q20）。
- **去掉的。** 原 Apps 页（本机组件清单）和它的读取命令删除（Q21）。“打开主页”调系统浏览器的做法去掉。

## 4. 范围外

- Library，以及 Settings 向 Library 的迁移（设计稿定为下一版本）。
- 跨 Zone 的内容聚合（D5）。
- 是否删除桌面 APPS（设计稿定为后续评估）。
- 经 SN 的推送、FCM、APNs（Q9）。
- kevent 流的鉴权，以及带鉴权的用户事件通道（Q8，owner 另行安排）。
- iOS 的发布和系统通知（Q18：Android 跑顺并上架后再安排）。iOS 的内容容器已提前开发，见 [iOS 内容容器 TODO](buckyos-app-ios-container-todo.md)，说法不一致时听那一份的。
- 用钥匙登录 Zone（Q2）。
- 在目标 Zone 没有密码的账号进入客户端：联合登录得到的 External 身份、Google 登录的账号（Q22）。
- 管理员给全 Zone 统一设主题色（Q23：主题色是每个用户自己设的）。
- 另做 WebSocket 的通知端点（Q24，随 Q8 另行安排）。
- 浏览器里的 Activity 页面和 Web Notification、移动 Web UI 的保活和 Header 升级（Q27；任务 N9、C6 保留编号，本版本不做）。
- 从已加入的群推荐组织 Zone（Q3 只定了两个来源）。
- Web Push（浏览器关闭后的提醒）。
- [Flutter 重实现方案](<../../BuckyOSApp/doc/Flutter重实现方案.md>)。本文按现有的 Tauri 代码规划。

## 5. 已知风险

| 编号 | 风险 | 处理 |
| --- | --- | --- |
| R1 | 各 Zone 版本不同，老 Zone 缺新接口 | 本地界面按接口是否存在逐块降级，提示这个 Zone 需要升级 |
| R2 | 三种 WebView 内核（WebView2、WKWebView、Android WebView）对 Cookie、隐藏窗口节流、媒体播放的处理不同 | T0 逐项实测，结论写回本文 |
| R3 | 在目标 Zone 没有密码的账号进不了客户端 | 第一版不支持，owner 已确认（Q22）；添加 Zone 的表单和登录失败的提示里说明 |
| R4 | kevent 流没有鉴权，常驻客户端让更多连接长期挂在这个入口上 | owner 另行安排（Q8）；本文不处理 |
| R5 | 桌面系统通知的点击回调、托盘悬停事件在各平台上不一定拿得到 | T0-4；备选是客户端自己画的提示小窗和 tooltip |
| R6 | 移动端多个 WebView 要写原生插件；本地界面的浮层盖不到另一个原生 WebView 上面 | T0-7 定做法；做不到时的退路是原生画 Header 和导航 |
| R7 | 改许可证的过程：L1 合入之前就引入 buckyos 的代码，仓库的许可证状态会说不清；AGPL 的代码流进 websdk（Apache-2.0）会连带影响第三方应用。另外，按我的了解，Apple App Store 的分发条款和 GPL 系许可证有过冲突的先例（2011 年 VLC 因此下架）：版权方上架自己的代码不受影响，收了外部贡献或带了第三方 GPL 代码时才有问题 | L1 单独提交并先做；§10 的约束。iOS 不在本版本（Q18），做 iOS 之前请 owner 确认应用商店这一点；这不是法律意见 |
| R8 | 签名密码需要原生输入界面，移动端要写原生对话框 | Z6；在此之前移动端不开放内容视图里的签名 |
| R9 | 保存的密码摘要等同登录凭据。通过客户端激活的 Zone，这个摘要还是由钱包密码算出来的 | 存系统钥匙串，不写明文文件、不进页面；按“先能力再安全”记为已知风险 |
| R10 | Zone 用自签或私有 CA 的证书时，WebView 不信任，无法加载 | 本地“连接失败”页说明原因；不做证书例外 |
| R11 | 本地界面和 Shell 是两套界面，行为会走样 | 规则、归并、接口封装只留一份，两边引用（§3.1）；§8 的验收对比两边 |
| R12 | Android 没有推送兜底：进程被系统停掉或用户强行停止后收不到通知；前台服务要常驻一条通知；Google Play 对前台服务类型有审核。iOS 不允许后台长连接，做 iOS 时这个问题会回来 | T0-9 实测并选前台服务类型；客户端设置里说明并引导关闭电池优化 |
| R13 | W3 完成之前，桌面客户端只能打开有独立路由的四个系统页面和第三方应用 | W3 和阶段 1 并行，尽早做 |
| R14 | 客户端的构建开始依赖 buckyos 仓库。`buckyos-api` 依赖面大，移动端可能编不过；buckyos 的 main 有破坏性改动时，客户端的构建跟着断 | T0-10 先试，编不过就只手写用到的类型。Rust 的 Git 依赖由 Cargo.lock 记着提交，升级是显式的一步；前端按同级检出引用时没有锁，CI 里记下用的是 buckyos 的哪个提交 |

## 6. owner 的答复

### 6.1 已答复（2026-11-23，两轮）

“答复”一栏是 owner 的原话。第一轮（Q1–Q21）是 owner 写在本文里的。第二轮是对话里的一段话，全文如下，表里按问题拆开引用：

> BuckyOS 可以改成AGPL没关系的（本来就是和buckyos一体的），Q11按你的建议，并说明以这个问的设计为准。，Q22-Q28的问题按你的建议做就好。这么大规模的重构做完后肯定也是要再做细节调整的。

| 编号 | 问题 | 答复 | 落实 |
| --- | --- | --- | --- |
| Q1 | 总体结构：界面由 Zone 的 Shell 提供，还是客户端自带 | 客户端可以在未bind zone的情况下运行，对非app窗口内的内容，我们都倾向于客户端自带。(messagehub tab, homestation tab 除外)。 | §3.1；阶段 4、6 改为在 BuckyOSApp 里实现 |
| Q2 | 客户端怎么登录 Zone | 客户端可以有独立的自动登录（可以保存密码hash）,尽量不要走密钥。原则上不会在客户端里看到zone的登录页 | §3.3；Z2–Z4 |
| Q3 | 用户能登录的 Zone 从哪里知道 | 1）来自owner doucment里的zone list 2) 用户可以手工在设置里添加，添加的时候会触发一次登录，如果登录成功就可以切换 | §3.3；Z1、Z3 |
| Q4 | 切换 Zone 时，原 Zone 已打开的窗口、播放和录音怎么处理 | 按你的建议。允许多个zone的app同时运行实际上要求buckyos app在获得资源的时候，都需要先定Zone did | §3.3 的表和“取资源先定 Zone”；§10；Z11 |
| Q5 | 非当前 Zone 的通知是否接收；点击后是否切换当前 Zone | 按你的建议 | §3.3 的表 |
| Q6 | 通知等级、消息、任务到系统提醒的映射 | 按你的建议 | §3.4 的表 |
| Q7 | 普通应用的通知权限怎么申请；已经装好的应用怎么办 | 标准的AppDoc 声明，这个版本braking change,不用管旧应用 | §3.4；N5 |
| Q8 | 通知通道：先用没有鉴权的 kevent 流是否可以接受；鉴权由谁改 | 另行安排，先不管 | §3.4、§4；原来的“带鉴权的用户事件通道”任务取消 |
| Q9 | 移动推送走什么 | android上先支持 基于 websocks长链的通知吧。走SN是个大活，我们希望用户尽量少依赖SN | §3.4；T0-9、N7 |
| Q10 | Activity 收哪些内容；和 Message Hub 的“系统通知”是什么关系 | 按你的建议 | §3.4；N6 |
| Q11 | 标准地址用什么；`buckyos://` 的几种写法怎么统一 | 第一轮：这个需要看一下buckyos://的定义文档。第二轮：Q11按你的建议，并说明以这个问的设计为准。 | §3.5：三条建议成为定案，并写明以 §3.5 为准、旧文档里不一致的写法作废；W7 不再等待 |
| Q12 | 窗口声明放在哪，字段叫什么 | 按建议 | §3.5；W2 |
| Q13 | 主客户端点 × 和退出客户端时的行为 | 按建议 | W8：托盘启用时点 × 隐藏；退出时逐个询问应用窗口后全部关闭 |
| Q14 | 托盘的点击行为 | 按建议，有通知时，悬浮还需要弹出通知内容 | §3.4；N3 |
| Q15 | 移动端 Message Hub 的 Tab、Home 里的图标、通知唤起是不是同一个实例 | 是同一个 | §3.6；C2 |
| Q16 | 录音“统一采集再分发”做不做 | 按你的建议 | 先做 A4 的验证，再定 |
| Q17 | 多个应用同时播放时要不要协调 | 按你的建议 | 第一版不协调 |
| Q18 | iOS 是否进入本版本 | 按你的建议，android跑顺并上架后，我们才会安排ios版本 | §4。2026-11-24 的 IQ1 把开发提前（[iOS 内容容器 TODO](buckyos-app-ios-container-todo.md)），上架顺序不变 |
| Q19 | 客户端要用的协议类型，是复用还是手写 | websdk里应该有可复用组件? | 核对结果在 §2.5：协议和数据层直接用 websdk；websdk 里没有界面组件。许可证放开以后，Desktop 的接口封装和 `buckyos-api` 也可以用 |
| Q20 | Zone 主题色谁能设；Home Station / Activity 的偏好存在哪 | 系统外观配置 | §3.3、§3.8；Z9、U7。我对这句话的理解见 Q23 |
| Q21 | 原 Apps 页（本机组件清单）的去留 | 先去掉吧。 | U8 |
| Q22 | 在目标 Zone 没有密码的账号（联合登录得到的 External 身份、Google 登录的账号）怎么进客户端 | Q22-Q28的问题按你的建议做就好。 | 第一版不支持，添加 Zone 时要求用户名和密码（§3.3、Z3、§4） |
| Q23 | “系统外观配置”是不是账号的外观状态，主题色是不是每个用户各设一个 | 同上 | 是。值存在账号外观状态的 `accent` 字段，每个用户给自己的每个 Zone 各设一个（§3.3、Z9） |
| Q24 | Q9 说的“WebSocket 长链”是否必须是 WebSocket 协议 | 同上 | 先用现有的 HTTP 流；另做 WebSocket 端点随 Q8 另行安排（§3.4、N1、N7） |
| Q25 | 桌面 Settings Tab 放什么 | 同上 | 客户端自己的设置，加一项“当前 Zone 的设置”，用应用窗口打开 Zone 的设置页面（§3.8、U4） |
| Q26 | Home 的小部件第一版做哪些 | 同上 | 只有系统状态卡片和应用快捷方式；音乐小部件放到音频阶段最后（§3.8、U2、A6） |
| Q27 | 浏览器里的 Web UI 本版本要不要同步补 Activity 页面、移动布局的保活和 Header | 同上 | 只同步规则和窗口声明（W6）；界面类的不进本版本，N9、C6 不做（§4） |
| Q28 | “＋”里的四项交给哪个应用 | 同上 | 发消息给 Message Hub，发动态给 Home Station，传照片给 Files。建议里“加笔记请 owner 指定”这一项没有另外指定，先按 §6.2 做（§3.8、U6） |
| 许可证 | 客户端能不能用 buckyos 仓库（AGPL-3.0）的代码。这一条不是提问，是 owner 看到“两份界面实现”的代价后定的 | BuckyOS 可以改成AGPL没关系的（本来就是和buckyos一体的） | BuckyOSApp 改成 AGPL-3.0（L1）；§3.1 的“共用代码放哪”；T0-10；§10 |

### 6.2 我替 owner 选的做法

现在没有等 owner 答复的问题。下面几处是按第二轮答复改写时我自己选的。owner 说过这种规模的重构做完以后还要调细节，所以先照这些做，并在实施记录里写明；owner 调整时改这张表和对应的任务。

| 事项 | 我选的 | 要改的话动哪里 |
| --- | --- | --- |
| 第二轮答复里“BuckyOS 可以改成AGPL”指什么 | 读作 BuckyOSApp 这个仓库改成 AGPL-3.0-only，写法和 buckyos 的 Desktop 一致。buckyos 本来就是 AGPL | L1 |
| “以这个问的设计为准”指什么 | 读作以 Q11 这一问定下的设计（§3.5）为准，`key url.md`、《外部调用协议》、`context ext.md` 里不一致的写法作废 | §3.5、W7 的文档部分 |
| websdk 的许可证 | 不动，仍是 Apache-2.0。第三方应用依赖它 | §10 |
| 共用代码放哪 | §3.1 的表：应用也要用的和新写的宿主逻辑在 websdk；Desktop 里已有的留在 buckyos，由客户端引用 | W1、N6、Z9 |
| 客户端怎么引用 Desktop 的源码 | 先试同级目录检出 buckyos 加路径别名，T0-10 验证后定 | T0-10 |
| “＋”里的“加笔记”交给谁 | AI Workspace，新建一篇文档 | U6 里的一处配置 |

## 7. 任务

任务编号的前缀：L 是许可证；T0 是先做的验证；Z 是多 Zone 与会话；N 是 Notify 与 Activity；W 是 Window System；C 是移动容器；A 是音频；U 是主界面。方括号里是改动所在的仓库：App 指 BuckyOSApp，Shell 指 `buckyos/src/frame/desktop`。

每个任务交付时记录：改了哪些入口、验证命令和结果、没有验证的平台。

### 许可证

- [x] **L1 [App] 改许可证。** BuckyOSApp 改成 AGPL-3.0-only，写法和 `buckyos/src/frame/desktop/package.json` 一致。要改的九处见 §2.1 的“许可证”一行：两份 `LICENSE` 换成 AGPL-3.0 全文（取自 `buckyos/LICENSE`），两份 `NOTICE`、README 的 License 一节、`package.json`、`Cargo.toml`、`tauri.conf.json` 的 `bundle.license`、`node_control.rs` 的 SPDX 头。README 里“`node_control.rs` 之后不能再从 buckyos 同步”的那一段删掉。把前端和 Rust 依赖的许可证列一遍，确认没有和 AGPL-3.0 不兼容的。这一项单独一个提交，交 owner 过目后合入。

完成条件：仓库里不再有 Apache-2.0 的声明（第三方依赖自带的除外）；L1 合入之前，没有任何 buckyos 的代码被拷进或引用进 BuckyOSApp。

### T0 先验证

在 BuckyOSApp 里开验证分支，做最小的试验页面和命令。结论写到本文 §11，验证代码不合入。Windows、macOS、Android 三个平台都要有结论。

- [x] **T0-1 Cookie 与顶层加载。** 把一个真实 Zone 的页面分别放在本地页面的 iframe 里和作为窗口的顶层文档加载，确认 Cookie 的推断。让窗口先访问一次性的 `/sso_callback` 地址，确认 Cookie 写入后能跳到目标页面。确认 Cookie 在客户端的多个窗口之间共享，重启客户端后还在。
- [x] **T0-2 远程顶层页面里的原生桥。** 注入脚本在页面脚本之前执行；远程页面调用原生命令的路径可用；确认现有 capability 下远程页面到底能调哪些命令。
- [x] **T0-3 客户端登录。** 在 Zone 外面用 Rust 拿摘要登录：分别试 verify-hub 的 `login_by_password` 和 control-panel 的 `auth.login`（结构化 `target`），看哪一个能拿到 control-panel 的用户会话，并用 verify-hub `refresh_token` 续期。用这个会话调 `auth.prepare_app_launch`，把回调地址交给窗口，确认应用页面登录成功。用 `auth.login` 带 `redirect_url` 给 Zone 根域的页面登录，确认可行。确认钱包返回的 `pwd_hash` 能登录一个通过客户端激活的 Zone。
- [x] **T0-4 系统通知与托盘事件。** 在 Windows 和 macOS 上发一条系统通知，确认能否拿到点击事件、没有前台窗口时点击能否唤起。确认托盘图标的鼠标进入、离开事件在三个桌面平台上是否可用。拿不到就评估平台原生接口和自绘小窗。
- [x] **T0-5 隐藏和最小化时的存活。** 主窗口隐藏到托盘后，本地页面的定时器和事件处理是否继续；应用窗口最小化后 `<audio>` 是否继续播放；Media Session 是否接上系统的媒体键和媒体面板。
- [x] **T0-6 长连接。** 用 Rust 对 `/kapi/kevent/stream` 保持连接，分别经直连和 SN 中转访问，记录多久被断、断开后的表现、网关 HTTP/2 下是否正常。
- [x] **T0-7 移动端的内容视图。** 先确认 Tauri 在 Android 上能不能再建一个 WebView；不能就用原生插件在 Activity 里加 `WebView`。要有结论的几点：顶层加载 Zone 页面并登录；注入桥脚本并把调用送到 Rust；本地界面的浮层怎么盖在内容视图上面；安全区和键盘；同时保活 3 到 5 个视图的内存。
- [x] **T0-8 音频。** Android 上起前台服务后，内容视图里的 `<audio>` 在切到后台、锁屏时能否持续。评估 `feature/audio_record` 移植到当前 main 的工作量，列出冲突的文件。
- [x] **T0-9 Android 后台长连接。** 前台服务下连接在息屏、Doze、切到后台、从最近任务里划掉之后各能活多久。Activity 进后台或被销毁后 Tauri 的 Rust 运行时是否还在跑；不在就把连接放进 Kotlin 服务。选定前台服务类型，查 Google Play 对它的审核要求。记录耗电。
- [x] **T0-10 共用代码的引用方式。** 分两部分，结论决定 §3.1 那张表后两行怎么落地。
  - Rust：在验证分支里把 `buckyos-api` 加成 Git 依赖（分支 `main`），在 Windows、macOS、Android 三个目标上编译。记录是否通过、编译时间和安装包大小的变化。编不过或代价太大，就只手写用到的类型（Z2、N1 两种做法都写了）。
  - 前端：让 BuckyOSApp 引用 Desktop 的一个模块，例如 `api/app_mgr.ts`。先试“同级目录检出 buckyos，Vite 和 tsconfig 配路径别名”：Desktop 引用 `../aiworkspace/` 下的源码、buckyos 的构建使用同级的 websdk，用的都是这个做法。确认类型检查和打包能过，四个 CI workflow 多检出一个仓库（分支 `main`）后也能过。走不通再试把共用部分做成 buckyos 里的一个包，BuckyOSApp 用 Git 依赖安装。两种都不行就退回拷贝，并在两边放同一份测试。

完成条件：§0 第 2、12 条和 §2.7 第 4 条里标“待验证”的结论都有实测结果；T0-3、T0-4、T0-7、T0-9、T0-10 各自给出选定的做法。

### 阶段 1：多 Zone 与会话

- [x] **Z1 [App] Zone 名单与当前 Zone。** 新模块 `src-tauri/src/zone/`，用独立的 store 文件，不和钱包的 vault 放在一起。命令：列出、添加、移除、设置当前、读取当前；当前 Zone 变化时发事件。名单按 DID 分开，切换 DID 时当前 Zone 跟着换。来源一：owner 文档的 Zone 列表，复用 `src/features/did/zoneBindingStatus.ts` 的 `resolveZoneBindingSnapshot`。来源二：手动添加，用 `resolve_did(…, "zone")` 取主机名，用 `zone.presentation.get` 取名称和图标；登录成功后才写入。所有命令的参数都带 Zone DID。
- [x] **Z2 [App] Zone 会话管理（Rust）。** 按 T0-3 选定的接口登录。凭据（用户名、摘要）和 token 对放系统钥匙串，按“DID + Zone”存。到期前续期；续期失败用摘要重新登录；摘要失效时把这个 Zone 标成“需要输入密码”并通知界面。提供一个带 token 的调用命令（Zone DID、服务、方法、参数），用 buckyos-base 的 kRPC crate。T0-10 通过的话，登录和续期直接用 `buckyos-api` 的 `VerifyHubClient`；control-panel 的 `auth.*` 没有现成的 Rust 客户端，自己拼请求。本地界面里 websdk 的 kRPC 客户端把 `fetcher` 接到这个命令，token 不进页面。
- [x] **Z3 [App] 登录的本地界面。** 添加 Zone 的表单：地址或 DID、用户名、密码。自己的 Zone：在用户输入钱包密码的地方（创建或导入身份、签名）顺带算出摘要并保存，第一次登录直接用；失败时问这个 Zone 的密码。任何情况下都不加载 Zone 的登录页。没有密码的账号第一版不支持（Q22）：表单上写明要用这个 Zone 的用户名和密码，登录失败的提示里说明联合登录、Google 登录的账号暂时不能用客户端。
- [x] **Z4 [App] 内容视图的登录。** 加载 Zone 页面前换一次性回调地址：应用页面走 `auth.prepare_app_launch`，Zone 根域的页面走 T0-3 验证过的做法。拦截内容视图里跳往 `sys.<zone>/login` 的导航，重新换回调地址再加载。客户端退出登录某个 Zone 时清掉这个 Zone 的 Cookie。
- [ ] **Z5（不需要，T0-3 的做法可行）[buckyos] 让 `auth.prepare_app_launch` 支持 Zone 根域的页面。** 只在 T0-3 发现 `auth.login` 带 `redirect_url` 的做法不合适时才做。补允许和拒绝两类用例。
- [x] **Z6 [App] 原生桥改传输。** `public/bucky-api.js` 改成直接调原生，不再依赖 `window.parent`。新增 `BuckyApi.env`、`BuckyApi.call`、`BuckyApi.on`。签名的密码输入改到原生界面：桌面用一个本地页面的小窗口，移动端用原生对话框。旧的五个方法 websdk 和激活页在用，要改签名就三处一起改。同步 `doc/BuckyApi.md`。
- [x] **Z7 [App] 收紧权限。** capability 拆开：本地页面可以用全部命令；内容视图（名单内 Zone 的源）只能用 `BuckyApi` 暴露的那一组；其它远程源什么都不能用。窗口 label 约定：`main`、应用窗口 `app_*`、辅助窗口 `aux_*`。
- [x] **Z8 [App] 账号区与 Zone 管理。** Home 的账号区列出 Zone（名称、图标、主题色、会话状态），点选切换。设置里的 Zone 管理页：添加、移除、重新登录、设主题色。
- [x] **Z9 [Shell、App] Zone 主题色与偏好。** 账号外观状态加 `accent`（`#RRGGBB`）和移动端动态入口的默认项两个字段（`desktop/appearance/model.ts`），Shell 的“设置 → 外观”加对应的设置项。客户端读写账号桌面状态用 Desktop 的 `api/desktop_state.ts`，先按 W1 把它改成请求通道由调用方传入。客户端登录后读取并缓存。`frame.init.theme.accent` 填上（Shell 和客户端两边）。
- [x] **Z10 [App] 首次使用路径与没有 Zone 时的运行。** 没有 Zone 时各 Tab 有可用的空状态。桌面端先探测 `http://127.0.0.1:3182/device`；现在的扫描把 `127.*` 过滤掉了（`src/pages/main/ScanDevice.tsx`）。激活完成后自动把新 Zone 加进名单、保存摘要、设为当前并登录。手机配合盒子的路径做回归。
- [x] **Z11 [App] 切换语义。** 按 §3.3 的表实现：切换前询问未保存内容；桌面的应用窗口标出所属 Zone；移动端切换前提示会结束的播放和录音。

完成条件：同一个 DID 在客户端里登录两个 Zone，切换后 Home、APPS 都是目标 Zone 的内容；重启客户端后仍是上次的 Zone 且不用输密码；全程没有出现 Zone 的登录页；一个 Zone 连不上不影响另一个；没有任何 Zone 时客户端能正常打开。

### 阶段 2：Notify 与 Activity

- [x] **N1 [App] 通知连接。** Rust 侧，每个开了通知的已登录 Zone 一条。订阅 kevent 流，事件到了就拉 `notify.list{status:[open], since_ms}` 和 `msg.list_sessions`；另有低频轮询兜底。断线后退避重连，重连后先补拉。把连接状态（已连接、重连中、需要登录）报给界面和托盘。通知的类型在 T0-10 通过时用 `buckyos-api` 的 `msg_notify`，否则只手写用到的几个字段，解析时忽略不认识的字段。读 kevent HTTP 流的部分没有现成的 Rust 客户端，要自己写。
- [x] **N2 [App] 系统提醒。** 按 §3.4 的映射生成系统通知，标明来自哪个 Zone；同一会话的新消息合并；同一条通知不重复弹（按 `notify_id` 和消息记录去重，重连补拉时尤其要注意）。点击后交给打开规则。实现方式按 T0-4 的结论。客户端设置里加通知总开关和每个 Zone 的开关，第一次开启时申请系统许可。
- [x] **N3 [App] 托盘。** 未读角标、紧急通知时闪动、菜单里最近几条通知。有未读时左键打开主窗口的 Activity，右键弹菜单。有未读时悬停弹出通知内容小窗，点一条就打开，离开后收起；没有悬停事件的平台用 tooltip。同步 `doc/托盘图标需求.md`。
- [x] **N4 [buckyos、websdk] 汇总接口。** msg-center 加 `notify.summary`：未处理的通知数（按分类）、消息未读总数（排除免打扰和系统通知会话，口径与 `trayRuntime.ts:74-104` 一致）。websdk 加对应方法，Shell 的托盘改用它。
- [x] **N5 [buckyos] 应用的通知权限。** 在 AppDoc 的 `permissions` 里定义通知项；安装流程展示；设置里按应用开关；`notify.rs` 的 `notify_caller` 对普通应用检查，没有声明的拒绝。系统服务和内置应用不受限。仓库里会发通知的应用补上声明。同步 `doc/message_hub/System Notify.md` §4 和 `doc/App 安装协议.md`。
- [x] **N6 [Shell、App] Activity。** 把 Shell 托盘的归并（`trayModel.ts`、`trayRuntime.ts`）里不含 React 的部分整理成独立模块：合并通知、有未读的会话、带标记的任务，按时间排序。补上游标翻页和全部状态的通知，请求通道由调用方传入。Shell 的托盘继续用它，客户端的 Activity 页面引用同一份。每行点击走打开规则；支持单条已读或忽略、全部已读。
- [x] **N7 [App] Android 后台通知。** 按 T0-9 的结论实现：前台服务、常驻通知、长连接、网络切换后重连、系统通知权限。设置里可以关闭，并说明关闭电池优化的方法。前台服务的做法参考 `feature/audio_record` 分支里的 `AudioRecordingForegroundService.kt`，不直接合并分支。
- [x] **N8 [buckyos] 应用接入。** AI Workspace 的评论和提及、日程到点、待办提醒改成调 `notify.send`，`source.link` 填应用内的页面地址。`dcrontab remind` 是否改走 System Notify，和 `opendan-d4-timer-reminder-todo.md` 一起定。Message Hub 的新消息不新增通知记录，由通知连接直接产生提醒。
- [ ] **N9 [Shell] 浏览器里的 Activity 与提醒。** 本版本不做（Q27）。以后要做时：`/activity` 页面用 N6 的归并；页面开着并且用户授权后用 Web Notification。

完成条件：桌面客户端隐藏在托盘时，另一个账号发来消息、一个应用调 `notify.send`，都能在几秒内弹出系统提醒；点击后打开对应的应用页面；Activity 里能看到并标记已读；断网再恢复后不丢、不重复。Android 客户端切到后台十分钟后仍能收到提醒。

### 阶段 3：Window System

- [x] **W1 [websdk、Shell、文档] 打开请求和规则。** 在 websdk 里定义 `OpenAppRequest`、窗口 ID 的拼法、§3.5 的规则函数和一份 JSON 测试向量，并补按 Zone 建一组客户端的入口。control-panel 的接口不在 websdk 里重写：把 Desktop `api/` 里客户端要用的几个模块（§2.5）改成请求通道由调用方传入，Shell 自己传全局运行时的那一个，客户端引用这些模块时传自己的。更新 `product/desktop/BuckyOS_Web_Desktop_需求文档.md` 的 §16.4 和 §21。
- [x] **W2 [buckyos] 窗口声明进 AppDoc。** `src/kernel/buckyos-api/src/app_doc.rs` 加 `window` 段；`apps.list` 的 `AppSummary` 带出；`app/backend-apps.ts` 用声明值替换固定默认值；同步 `doc/App 安装协议.md`。
- [x] **W3 [Shell] 系统应用的独立页面和清单。** 每个内置应用有一个独立页面地址，只渲染这一个应用，没有桌面外壳，支持按路径进入应用内的页面。构建时导出系统应用清单（id、各语言名称、图标图片地址、页面地址、窗口声明，Message Hub 是 `single`），放在 Zone 根域的固定地址。先做 Files、设置、Task Center、AI Center，其余按目录逐个补。
- [x] **W4 [App] 原生窗口管理（桌面）。** Rust 侧维护“窗口 ID → 窗口”，提供新建、置前、窗口内导航、关闭。新窗口顶层加载应用地址（先经 Z4 登录），按声明设置大小、最小尺寸、能否调整、最大化和最小化按钮。标题跟随页面。窗口上显示所属 Zone 的名称和主题色。位置大小按“Zone + 应用”记忆。点 × 先走 `frame.beforeClose`。指向名单外的链接交给系统浏览器。打开规则由本地页面调 W1 的函数算出，再调这些操作。
- [x] **W5 [websdk、App] app-frame 的原生传输和客户端宿主。** `AppFrameClient` 在没有父窗口而有 `BuckyApi` 时走原生通道。新增 `app.open` 消息，`AppFrameHost` 同步支持。客户端按 §3.2 的表处理各条消息；`content.open` 的解析用 websdk 已有的 `resolveContentHandlers`，客户端写把结果交给打开规则的那一层（Shell 的对应代码是 `app/content/open.ts`）。应用不改代码就能在两种传输下工作。
- [x] **W6 [Shell] 对齐。** 窗口 ID 改成“AppID + 路径”；`openApp` 和 `openAppWindow` 改调 W1 的规则；遵守新的声明字段（固定大小时不显示调整手柄）。`runtimeContainer` 的手动选项只留给调试。
- [x] **W7 [App、文档] 标准地址。** 按 §3.5 做，§3.5 是 owner 定的准绳。
  - 文档部分不依赖客户端代码，可以马上做：`BuckyOSApp/doc/BuckyOS外部调用协议设计.md` 加 `app.open` 这个 action 和“只做导航的 action 可以不带 request object”；`buckyos/doc/sdk/context ext.md` §8 的 `buckyos://app/<app_id>` 改成新写法；`buckyos/doc/key url.md` §2 加一句指向《外部调用协议》。
  - 实现部分：https 应用地址与 `OpenAppRequest` 的互转。注册 `buckyos://`：桌面端由单实例插件把第二次启动的参数转给已运行的进程，Android 加 intent filter。第一版只接打开应用这一个 action，其它直接拒绝。
- [x] **W8 [App] 主客户端生命周期。** 托盘启用时点 × 隐藏；退出前依次询问各应用窗口后全部关闭。

完成条件：从图标、Activity、系统通知、另一个应用四个入口打开同一个应用，单窗口应用始终只有一个窗口并能进入指定页面；多窗口应用按窗口路径复用；同一组操作在桌面 Web UI 和桌面 APP 里结果一致；Files 和 Zone 设置能在原生窗口里打开。

### 阶段 4：移动容器

- [x] **C1 [App] 内容视图（Android）。** 按 T0-7 的结论实现：新建、显示、隐藏、销毁、设置位置和大小；顶层加载并登录；注入 `BuckyApi` 并把调用送到 Rust；保活上限和按最近使用淘汰。
- [x] **C2 [App] 底部导航。** 五项。Message Hub Tab 和 Home Station Tab 各用一个内容视图，和图标、通知唤起是同一个视图。Tab 上显示 N4 的未读数。
- [x] **C3 [App] 系统 Header 与应用切换。** 两种风格；应用切换、返回、标题、搜索、更多、最小化；固定底色风格的第二行和搜索入口，应用通过 app-frame 声明和更新。应用切换视图列出已打开的应用。
- [x] **C4 [App] 返回与最小化。** Android 系统返回键先到客户端的返回栈，再到应用的 `frame.back`。最小化回到 Home，应用留在已打开列表里。
- [x] **C5 [Shell、websdk] Tab 里的页面形态。** `frame.init` 增加呈现方式；`/messagehub`、`/homestation` 在 Tab 里时去掉自己的外层导航。
- [ ] **C6 [Shell] 移动 Web UI 的保活和 Header。** 本版本不做（Q27）。

完成条件：Android 客户端里打开三个应用，来回切换不重新加载；最小化回到 Home；系统返回键的行为和 Header 上的返回一致；Message Hub Tab 和从 Home 图标进入的是同一个页面状态。

### 阶段 5：后台音频

- [x] **A1 [App] 桌面播放。** 按 T0-5 的结论处理最小化时的播放；把 Media Session 的可用情况写进能力清单。
- [x] **A2 [App] 移动播放保活。** 能力 `audio.background`：申请保活时 Android 起前台服务和媒体通知；释放时撤掉。需要的系统权限加到 Android 清单里。
- [x] **A3 [App] 录音。** 把 `feature/audio_record` 的原生部分移植到当前 main，不直接合并分支。接口整理成 `audio.record.*`，走 Z6 的新桥。桌面端要么实现，要么在能力清单里不列。
- [x] **A4 [App] 统一采集分发的验证。** 一个原生采集源，把音频块分发给多个订阅的应用。记录各平台的后台限制、延迟、权限提示归属，给出做或不做的建议。
- [x] **A5 [websdk] 音频接口。** 封装保活和录音接口；不在客户端里时退回浏览器自身的能力，并通过能力清单告诉应用。
- [x] **A6 [App] Home 音乐小部件。** 放在音频阶段最后（Q26）。从 Shell 的 `desktop/widgets/MusicPlayerWidget.tsx` 移植。要先解决本地页面播放 Zone 文件的地址和鉴权。

完成条件：桌面的音乐应用窗口最小化后继续播放，点 × 后停止；Android 客户端切到后台和锁屏后继续播放，媒体通知可以暂停和继续。

### 阶段 6：两端主界面

- [x] **U1 [App] 桌面主面板。** 四个 Tab。主窗口可以调高度并记住位置。用鼠标操作，不依赖长按之类的触摸手势。样式用 Desktop 的设计 token 和明暗主题（§2.5），观感和 Shell 一致。
- [x] **U2 [App] Home。** 账号区和 Zone 切换（Z8）、系统状态卡片、常用应用快捷方式（可添加和移除，存在该 Zone 的账号桌面状态里，跨设备一致）。纵向滚动。第一版不带别的小部件（Q26）；以后要加，从 Shell 的 `desktop/widgets/` 移植。
- [x] **U3 [App] APPS。** `apps.list` 加系统应用清单，点击走打开规则。
- [x] **U4 [App] Settings。** 按 Q25：客户端设置（语言、明暗、通知、开机启动、Zone 管理、身份与备份），加“当前 Zone 的设置”入口。
- [x] **U5 [App] 移动 Home。** 应用启动器，数据同 U3。
- [x] **U6 [App] “＋”快速创建。** 预置四项：发消息给 Message Hub，发动态给 Home Station，传照片给 Files（Q28），加笔记给 AI Workspace 新建文档（§6.2）。四项的目标应用写成一处配置。创建发生在当前 Zone。内容处理器登记已支持 `create` 意图时菜单项从登记里取，否则先写死。
- [x] **U7 [App] 动态入口。** 默认 Home Station，可切到全部动态；偏好存在账号外观配置里（Z9）。
- [x] **U8 [App] 清理。** 删除原 Apps 页和读本机 `applist.json` 的命令；去掉“打开主页”调系统浏览器的做法；旧的三个 Tab 换成新结构；旧的 iframe 容器只留给激活页，其余改用内容视图。
- [x] **U9 [文档]** 更新 `BuckyOSApp/doc/当前需求文档.md`、`doc/BuckyApi.md`、`doc/目录结构.md`，`buckyos/product/desktop/` 下的 PRD，以及 websdk 的应用开发文档。

完成条件：设计稿 §8、§9 描述的入口在两端都能到达，并且 §8 的验收全部通过。

## 8. 验收

对应设计稿 §12 的检查场景。多 Zone 的场景用三 Zone 测试环境（`src/dev_configs/ssotest.json`）。桌面场景在 Windows 和 macOS 上各跑一遍。

| 场景 | 通过标准 |
| --- | --- |
| 没有 Zone | 新装的客户端不绑定任何 Zone 也能打开，各 Tab 有空状态，能进入设备发现和添加 Zone |
| PC 首次安装 | 安装后从桌面图标进入，注册账号，发现并激活本机服务，自动进入该 Zone 的 Home，中间不需要手动找地址，也不需要再输一次密码 |
| 手机配合盒子 | 只用手机完成注册、局域网发现、激活绑定，随后进入该 Zone 的 Home |
| 同一 DID 访问个人和组织 Zone | 手动添加组织 Zone 时登录一次；之后切换不用输密码，全程不出现 Zone 的登录页；切换后各 Tab 的内容属于目标 Zone；主题色跟着变；重启后保持 |
| 两个 Zone 的应用同时开着 | 桌面端两个窗口各自属于自己的 Zone，互不串数据；窗口上标出所属 Zone |
| 收到消息、评论、日程提醒 | 客户端在托盘里时弹出系统提醒；悬停托盘能看到内容；点击后进入对应应用的对应页面；Activity 里有记录 |
| Android 在后台收通知 | 客户端切到后台、息屏十分钟后，新消息仍能弹出系统通知 |
| 应用已经打开时再次进入 | 单窗口应用置前并进入指定页面；多窗口应用按窗口路径复用或新建；四个入口结果一致；同一组操作在桌面 Web UI 里结果相同 |
| 桌面音乐播放器最小化 | 继续播放；点 × 后停止 |
| 移动端打开和切换应用 | 全屏容器；应用切换入口能回到已打开的应用且不重新加载；最小化回到 Home |
| 从“＋”快速写入 | 预置动作都能在当前 Zone 完成；切换 Zone 后写入的是新的 Zone |
| 移动动态视图切换 | 默认 Home Station；切到全部动态后重进仍是全部动态 |
| 四种环境运行同一个应用 | 同一个应用不改代码，在四种环境里都能打开；能力清单如实反映各环境可用的能力；不可用的能力有明确的返回 |
| 权限 | 第三方网站作为内容视图的顶层页面时，不能调用身份切换、钱包删除、本机服务启停等命令；没有声明通知权限的应用调 `notify.send` 被拒绝 |
| 降级 | 连接一个没有升级的老 Zone，本地界面缺数据的块提示需要升级，其余可用，不崩溃 |

## 9. 建议顺序

1. L1 和 T0。T0 三个平台并行；L1 要先于任何引用 buckyos 代码的任务。
2. 阶段 1。Z1 到 Z4 是客户端的地基。同时在 buckyos 里开始 W2、W3，以及 W1 里“把 Desktop 的接口模块改成请求通道由调用方传入”这一步。它们不依赖客户端，后一步是 U2、U3、Z9、N6 的前提。
3. Z2 完成后，阶段 6 的桌面部分（U1 到 U4）就可以开始，和后面的阶段并行。
4. 阶段 2 的 N1 到 N4、N6 和阶段 3 的 W1、W4、W5 并行：通知的“点击后打开”先用最简单的“打开地址”实现，W4 完成后换成统一规则。
5. 阶段 4，然后 N7、U5 到 U7。移动端的工作量集中在 C1。
6. 阶段 5 的 A1 可以提前，A2 之后的部分依赖 T0-8。
7. N5、N8 是服务端和各应用的工作，和客户端并行。W7 的文档部分可以马上做，实现部分跟在 W4 后面。

设计稿定的顺序是 Review、多 Zone、Notify、Window System，上面的安排保持了这个主线。

## 10. 接手时必须遵守的约束

- **许可证。** owner 定 BuckyOSApp 改成 AGPL-3.0（L1）。L1 合入之前，不要把 buckyos 仓库的代码拷进或引用进 BuckyOSApp。L1 之后可以引用 Desktop 的源码和 `buckyos-api` 等 crate，做法按 §3.1 的“共用代码放哪”：只留一份，引用，不拷贝。websdk 仍是 Apache-2.0，第三方应用依赖它：不要把 buckyos 或 BuckyOSApp 里的代码搬进 websdk，也不要让 websdk 依赖 AGPL 的包。确实要搬时先问 owner，因为这等于把那段代码改按 Apache-2.0 发布。
- **取资源先定 Zone。** 客户端里任何取资源的函数、命令、缓存都带 Zone DID，不从“当前 Zone”取隐含参数（Q4）。
- **不出现 Zone 的登录页，不用钥匙登录。** 登录界面是本地的；内容视图跳向登录页时由客户端接管（Q2）。
- **通知不依赖 SN。** 不接 FCM、APNs，不加 SN 中转（Q9）。
- **Git 依赖。** BuckyOS 家族仓库里，所有从 GitHub 引入的依赖都写分支名 `main`，不用 `rev`、提交 SHA 或 tag（见各仓库的 `AGENTS.md`）。
- **提交顺序。** websdk 先发布，然后是 buckyos，最后是 BuckyOSApp。BuckyOSApp 用的是 npm 上的 `buckyos` 版本。
- **兼容。** 当前版本允许 breaking change，不做旧版本兼容。例外是客户端对 Zone 版本的容忍：用户的各个 Zone 版本不会一样，按 R1 降级。
- **工作树。** 同一台机器上可能有别的会话在改同一棵树。动手前看 `git status`，不要用 `git stash` 做基线对比，不要回退不是自己改的文件。
- **文档同步。** 改了行为就同步对应的文档：BuckyOSApp 的 `doc/当前需求文档.md`（它的 §17 有维护规则）、`doc/BuckyApi.md`、`doc/托盘图标需求.md`；buckyos 的 `doc/message_hub/System Notify.md`、`product/desktop/` 下的 PRD、`doc/App 安装协议.md`。
- **验证。** 客户端相关的结论不能只凭单元测试。涉及 WebView、窗口、通知、音频的任务，要在对应平台的真机或虚拟机上跑过，并写明没有验证的平台。
- **已定的和我选的。** §6.1 的问题 owner 都已定，照做。§6.2 是我替 owner 选的做法，也照做，并在实施记录里写明；owner 调整后改 §6.2 和对应的任务。`buckyos://` 和应用标准地址以 §3.5 为准，别的文档与它不一致时听 §3.5 的。

## 11. 实施记录

实施时在这里追加：T0 的实测结论、每个任务的落点和验证结果、与正文不同的地方、新出现的待定问题。

- 2026-11-23：owner 答复 Q1–Q21。Q1、Q2、Q9 改了总体结构，据此改写了 §0、§3、§5 到 §10，任务重新编号。改写时补查并写进 §2 的事实：密码摘要的算法和激活时管理员密码的来源、`auth.login` 与 `auth.prepare_app_launch` 的范围、Shell 的路由表、账号外观状态的存法、websdk 可复用的部分、`buckyos://` 的几份定义。还没有任何实现。
- 2026-11-23（第二轮）：owner 定 BuckyOSApp 改成 AGPL-3.0，Q11 按建议并以 §3.5 为准，Q22–Q28 按建议。据此撤掉了“客户端不能用 buckyos 代码”的约束，加了 L1、T0-10 和 §3.1 的“共用代码放哪”；W7 不再等待；N9、C6 移出本版本；§6.2 改成我替 owner 选的做法。补查并写进 §2 的事实：BuckyOSApp 声明许可证的九处位置；Desktop `api/` 的封装都走 websdk 的全局运行时；`buckyos-api` 没有 Zone 外可用的 kevent 客户端，也没有 control-panel `auth.*` 的客户端，并且带 vendored openssl 等依赖。更正一处：上一版说内容解析的逻辑在 Desktop 里，实际 `resolveContentHandlers` 已经在 websdk 的 `content.ts`，Desktop 的 `app/content/open.ts` 只是接到窗口的那一层。`key url.md`、《外部调用协议》、`context ext.md` 三份文档本身还没有改，留给 W7。还没有任何实现。
- 2026-11-24：全部任务实施完（T0 到 U9），未提交。下面按“T0 结论、落点、验证、与正文不同、替 owner 选的、等 owner 的、没验证的、提交顺序”记录。

### 11.1 T0 的实测结论

环境：三 Zone 环境（alice 经 SN、bob 直连、charlie 自定义域名）、Windows 10（WebView2）、macOS 26、Linux 容器（WebKitGTK）、Android 模拟器 API 35 / 36。没有真机。

| 项 | 结论 |
| --- | --- |
| T0-1 | 本地页面里的 iframe 带不上 Zone 的 Cookie（跳登录页）；作为窗口顶层文档、先走一次性 `/sso_callback` 可行；Cookie 在客户端的多个窗口间共享，重启后还在。Android 上 Cookie 约 30 秒才落盘，要主动 flush |
| T0-2 | 证实：不声明应用自己的 ACL 时，远程顶层页面能调全部应用命令。现在 `build.rs` 用 `AppManifest::commands` 声明，capability 按窗口拆开，远程页面只剩 `bucky_api_call` |
| T0-3 | 选 verify-hub `login_by_password`（摘要 = Base64(SHA256(密码 + 用户名 + ".buckyos"))）拿会话、`refresh_token` 续期；应用页用 `auth.prepare_app_launch`，根域页用 `auth.login` 带 `redirect_url`，不需要 Z5。nonce 必须严格递增。钱包密码的摘要能登录通过客户端激活的 Zone（真实激活验过，§11.3） |
| T0-4 | Windows 用 toast、macOS 用 `UNUserNotificationCenter`（要 .app），都能拿到点击并在没有窗口时唤起；托盘的进入 / 移动 / 离开在 Windows、macOS 可用，Linux 没有，退回 tooltip。Windows 上“进入”只报一次，之后靠“移动”，所以两个都认 |
| T0-5 | Windows 不节流；macOS 默认节流并停掉音频，窗口都设 `backgroundThrottling: disabled` 后最小化继续播。Rust 发给隐藏主窗口的事件两端都到 |
| T0-6 | kevent HTTP 流直连和经 SN 各保持 20 分钟，15 秒一个 keepalive，没有断；HTTP/2 正常 |
| T0-7 | Tauri / wry 在 Android 不能建第二个 WebView（会顶替主 WebView）。用 Kotlin 加 `WebView`：内容视图在下，透明的主 WebView 在上，按区域穿透触摸。**每个账号 + Zone 一个 WebView Profile**，否则任何一个应用页面卡住都会卡住本地界面（忙等 5 秒：4802 ms → 3 ms） |
| T0-8 | `mediaPlayback` 前台服务加平台 `MediaSession` 后，切后台、息屏继续播；录音原型按 `audio.record.*` 重写，没有合并分支 |
| T0-9 | 选独立进程 `:notify` 的 `remoteMessaging` 前台服务。退后台、划掉任务、息屏、强制 Doze 610 秒后仍在 1 秒内弹出。Google Play 的审核和真机耗电没有数据 |
| T0-10 | Rust：不引 `buckyos-api`（vendored openssl 等依赖、Android 编译代价），手写用到的字段。前端：同级检出 buckyos 加路径别名可行，CI 多检出一份到 `.buckyos/` |
| 另外 | 未登录的根域页面跳的是 `<zone>/login`，不是 `sys.<zone>/login`，两个都拦。macOS / Linux 的导航回调不分主框架和子框架：macOS 换了导航代理的方法来区分，Linux 不区分 |

### 11.2 落点

| 范围 | 文件 |
| --- | --- |
| 客户端 Rust | `src-tauri/src/zone/`（名单、会话、钥匙串、`zonemedia` 协议）、`content/`（桌面窗口 `windows.rs`、移动视图 `mobile/`、桥、导航策略、签名小窗）、`notify/`（连接、系统通知、`:notify` 服务）、`audio/`、`tray.rs`、`lifecycle.rs`、`deeplink.rs`、`android.rs`、`did/bridge.rs`；`build.rs`、`capabilities/`、`tauri.conf.json`、`Info.plist` |
| Android | `gen/android/app/src/main/java/com/buckyos/buckyosapp/` 的 `content/`、`notify/`、`audio/`、`NativeBridge.kt`、`Secrets.kt`、`Labels.kt` |
| 客户端前端 | `src/features/{zone,apps,content,notify,activity,home,devices,container,create,feed,music,tray}`、`src/app/`（含 `phone/`）、`src/windows/`、`src/pages/main/`；`public/bucky-api.js` 重写 |
| websdk | `src/{open_app,zone_clients,runtime_env,audio}.ts`、`app-frame.ts`、`msg_center_client.ts`、`cli/modules/pikg*.ts`，测试向量 `tests/open_app_vectors.json` |
| buckyos 服务端 | msg-center（`notify.summary / list_apps / set_app`、应用的通知权限）、AppDoc `window`、AI Workspace 的评论 / 提及 / 日程 / 待办通知 |
| Desktop | `src/shared/`（客户端引用的唯一一份）、`src/app/standalone/`（系统应用独立页面、`/system-apps.json`）、窗口规则改用 websdk、通知设置页、Zone 主题色 |

各包的详细报告（落点、接口、逐条偏差）在实施机的 `/home/lzc/buckyosapp-dev/reports/`，不在仓库里。

### 11.3 验证

| 内容 | 结果 |
| --- | --- |
| 单测 | BuckyOSApp Rust 96、前端 174；websdk `pnpm test` 1401 过、2 条本机环境失败（改动前就失败）；msg_center、control_panel、aiworkspace 的相关测试；Desktop 的类型检查、单测、e2e 与基线对照没有新增失败（细节见各包报告） |
| 桌面核心（登录、切换、内容视图、权限、通知连接、重启保持） | 29 项，Linux / Windows / macOS 全过，真实三 Zone |
| 打开规则（图标、通知、Activity、另一个应用、内容处理器、`target=_blank`、两个 Zone、登出） | 9 项，三个桌面系统全过 |
| 通知 | 事件到系统提醒约 0.5 秒，跨 Zone 消息约 1 秒；断开重连补报、不重复；Windows、macOS 实际点过通知、悬停过托盘小窗、点过小窗里的一行 |
| 首次使用（PC） | Linux 上真实走完：在测试 SN 注册（含 BNS）→ 自动发现本机未激活的服务 → 激活向导 → 约 1 分钟后主窗口自动切到新 Zone 并登录，全程只在签名时输过钱包密码；解绑也验过 |
| 生命周期 | 点 × 前问应用、退出逐个问（macOS ⌘Q 和 Dock 退出也问）、Dock 唤回、`buckyos://app.open`（Windows、macOS；名单外的 Zone 被拒并提示） |
| 播放 | Windows、macOS 应用窗口最小化继续播、点 × 停止；Home 音乐经 `zonemedia` 播放、拖动、下一首、关到托盘继续（Windows；macOS 用 WAV 验，测试机没有声卡，MP3 放不了） |
| Android 模拟器 | 最终版 API 35 debug：核心 29、安全 20、激活 4、身份 6、其他 16、音频 22、Home 音乐 9 + 7（长曲目）、权限 3、通知 21、退出 5、逐页走查 26，全过；API 36 debug 跑核心、安全、激活、身份、其他、走查，全过。深色走查 26 在断网恢复那处修改之前。release（R8）包只在审查修复之前跑过（核心 29、音频 22、通知 19/21 与其他 15/16，差的是测试脚本点通知的问题和下载重名，后者已改），修复之后只编译和验 16 KB 对齐 |
| 权限 | 远程顶层页面调不到任何客户端命令；没有声明 `msg-center/notify` 的应用（text-editor）在真实 Zone 上被拒（`NOTIFY_NOT_DECLARED`） |
| AI Workspace 通知 | 真实 Zone：日程到点后 37 秒收到 System Notify，链接指向工作区 |
| 降级 | alice 去掉 `/system-apps.json` 后，APPS 提示需要升级，第三方应用和四个固定的系统应用仍能打开 |

实测中修掉的问题，后来人要知道：

1. **钱包密码摘要的泄露。** 旧桥 `signJsonWithActiveDid` 的返回里有 `pwd_hash`，它就是可以登录账号自己 Zone 的摘要。内容视图里现在恒为 `null`，只有激活页拿得到。钱包凭据也只发给 owner 来源的 Zone，不发给手动添加的。
2. macOS 上 `cookies_for_url` 会锁着主 WebView 等主线程，和窗口事件互相等，整个客户端卡死；改用 `cookies()`。
3. 激活页连续请求两次签名时，第二次被判成 busy（React 状态晚一拍），激活失败；改用 ref。这是旧代码就有的。
4. 刚激活的 Zone 还没起来时被当成“要密码”；现在算连不上，30 秒一次自动再试。
5. 会话状态以前只看 token 不看网络，Zone 连不上也显示已连接；现在跟着请求结果走。
6. 窗口还在登录途中又被通知唤起，目标页面会丢；现在改走新的登录。
7. macOS 的 ⌘Q 不触发 Tauri 的退出事件；给 app delegate 补了 `applicationShouldTerminate:`（注销、关机不拦）。
8. Message Hub 页面不可见时也标已读，通知会被藏着的窗口读掉；改成可见时才标，Windows 最小化时同步 WebView 的可见性。

实现完之后另做了一次独立代码审查（只读，32 条，报告在实施机 `/home/lzc/buckyosapp-dev/reports/REVIEW.md`），已全部修掉，其中影响结构的：

- **一次性登录地址不经过页面脚本。** 原来用 `location.replace` 把回调地址交给当前页面，应用页面可以用 Navigation API 截到 nonce，换走根域会话。现在远程页面一律由客户端原生导航；页面自己的 `redirect_url` 只认自己的主机；`/sso_callback` 只有客户端发起的那一次放行。
- **激活页不再是本地页面里的 iframe**（桌面、Android）。Tauri 按主机名第一段判断“本地页面”，Windows / Android 又会把脚本注入每个 frame，激活页的 frame 等于拿到主窗口的命令。现在是 `act_*` 独立窗口（Android 是独立 Profile 的原生视图），只有五个钱包方法；`aux_*` 的权限按窗口拆开；桌面的命令入口另查请求的 Origin。
- **通知先显示再记“已见”**，中途失败会重读；先开流再补拉，流打开前的事件不丢；消息不排在任务事件后面；任务列表按新的在前取。
- **身份切换**：同步 Zone 名单带身份参数；切换和删除身份前询问并关闭全部窗口；签名绑定请求时的身份，签名框始终显示主机地址和身份；托盘里别的身份的提醒清掉。
- **会话**：自己退出的 Zone 不再被自动登录；被拒绝的摘要不反复试；输错密码不影响还有效的会话；换用户登录清 Cookie；后台重试不刷状态事件。
- 应用不能替用户打开另一个 Zone 的东西（要问）；不在前台的视图不能抢屏幕。
- **Android**：登录回调一律由 `WebView.loadUrl` 加载；桥要求文档主机等于视图的主机；表单 POST 在 `onPageStarted` 补判；系统通知的 intent 带随机令牌，伪造的忽略；签名框随视图关；最后一个 Zone 退出时 `:notify` 先注销会话再停；下载在进程内带会话进行；`clear_cookies` 按 `ZoneKey` 找 Profile（原来删身份、移除 Zone 之后找不到，网页数据没清）。

Android 收尾时另外发现并修的：返回键到不了本地界面；标签页与从别的 Zone 链接打开的应用互相抢前台；Home 音乐跳转后卡住（WebView 会对拦截来的流再按 `Range` 跳一次字节）；断网恢复后第一个请求挂 20 多秒（连接池里是死连接，网络恢复时换 client）；UI 线程上同步取当前身份会死锁（`on_navigate` / `on_event` 里不要调 `active_account` / `handles`）。

最后一轮（含以上全部修改）：Linux 核心 29、规则 9、通知 11、攻击 5；Windows 核心 29、规则 9、攻击 5、音乐 5；macOS 核心 29、规则 9、攻击 5，全过。Windows toast、macOS 通知、托盘、签名框的手动点击，以及 Linux 上的真实注册激活，是在这一轮之前做的，没有重做。

### 11.4 与正文不同

- Z2：没有用 kRPC crate，直接发 HTTP 并原样转发回答，错误码和数据不丢；`buckyos-api` 没引（T0-10）。
- Z6：桥有三种传输：桌面走 Tauri IPC、移动走 `__buckyPort`、激活页仍经外层本地页面。
- Z4：客户端另外预写根域的会话 Cookie，应用里嵌的文件选择器、Preview 才是登录状态。
- W3：诊断页不在清单里（离开桌面没有内容）；老 Zone 对不存在的路径回 `200 text/html`，按“不是 JSON”判断。
- N5：检查放在 `notify.send` 里而不是 `notify_caller`，否则关掉通知的应用连撤销都做不到。
- N8：AI Workspace 自己展开标准日历的重复规则，和 D4“Agent 滚动安排”的方向不同；Agent 维护的记录留给 Agent。`dcrontab remind` 没改，建议随 D4 定。
- N6：Activity 的任务看 30 天。
- A6：Linux 上放不了（WebKitGTK 的媒体管线不认自定义协议），Linux 不在交付平台里。
- U5：手机 Home 只有启动器，没有系统状态卡片。
- `frame.beforeClose` 桌面等应用答复，超时当作同意；主页面 5 秒不接手就直接关。
- U8：激活页不用 iframe 了（iOS 随内容容器一起改掉），iframe 容器只剩开发环境的测试页。
- W5：`app.open` 带别的 Zone 时先问用户，视图不在前台直接拒绝。

### 11.5 实施中替 owner 选的（改起来都不大）

- 手机动态 Tab 顶部常驻“Home Station / 全部动态”的分段控件；从图标打开 Home Station 不改已选的模式。
- Activity 里打开一条未处理的 notice 就算已读；全部已读不动等待回答的。
- 通知默认开，第一次有 Zone 登录成功时申请系统许可；还没接受的陌生人消息也提醒。
- 切到需要密码的 Zone 立刻弹登录框，启动时只给提示条。
- 移除自己的 Zone 只是从列表隐藏，与解绑分开。
- Home 音乐：桌面切 Zone 时正在播会先问；请求偶尔连不上的 20 秒内不停；选文件夹从 `/home/<用户>` 起可以往上走。
- 内容打开：应用没声明 `instance` 时仍按内容处理器的 `window`；窗口位置大小按“Zone + 应用”记。
- 独立页面在浏览器里跟着账号外观。
- AI Workspace：评论通知谁、`@用户名` 按文本解析、5 分钟合并、提醒一条记录只给一个人、日期类从 09:00 算。
- Android：麦克风每个视图第一次由客户端先问再由系统授权，摄像头和定位一律拒绝；系统通知按 Zone 分组、已读即撤。
- Android：客户端发起的登录到达后清空该视图的后退历史，会话过期重登后退不回之前的页面；激活页不能下载、不能用麦克风；Home 音乐暂停 10 分钟后放掉后台持有。
- 从链接或通知进另一个 Zone：当前 Zone 有打开的应用才问，只有标签页时直接切过去；用户自己点的通知不问。
- 麦克风统一采集分发（A4）：技术上可行，第一版不做。桌面实测 Rust → 3 个 WebView（含隐藏窗口）、20 ms 一块：不丢，延迟 p95 1 ms（macOS 偶发 640 ms 停顿）；Android 5 个视图 p95 1 ms。理由是现在没有两个应用要同时听，Android 同一客户端里多个 WebView 本来就能同时录。

### 11.6 等 owner 的

1. L1 的提交 `63cfdcf` 过目后合入。`buckyos-pack-installer` 的 `license.md`、`license-cn.md` 还写着 BuckyOSApp 是 Apache-2.0，没有改（不在本 TODO 的三个仓库里）。依赖树里有一个 GPL-3.0 的 crate（`ed25519_to_curve25519`，来自 name-lib），AGPL 下兼容。
2. §11.4 的 N8 与 D4 的边界。
3. 桌面客户端和本机 BuckyOS 共用 `/opt/buckyos/local/did_docs` 这个 DID 文档缓存目录（name-client 的缺省），没有权限时只是写不进缓存、有报错日志。要不要给客户端单独的目录。
4. Google Play 对 `remoteMessaging` 前台服务的审核材料，上架时准备。

### 11.7 没验证的

真机（Android 厂商的后台限制、真机耗电、arm64）、审查修复后的 Android release 包在设备上运行、Android 上对真实 OOD 的完整激活（用的是本机假设备页）、下载重定向到 Zone 之外、Android 10 以下的下载位置、iOS（当时只有 `Unsupported` 的占位；后来的实现和验证见 [iOS 内容容器 TODO](buckyos-app-ios-container-todo.md) §11）、macOS 的正式签名与公证、Windows 安装包、Windows 11、手机配合盒子的完整路径（模拟器上只验了扫描和激活容器的代码路径）、AppDoc 声明了 `window` 的真实应用、`notify.list_apps` 之外的安装界面整条链、Safari / Firefox 上的新页面、大量工作区时 AI Workspace 每分钟扫描的开销。

### 11.8 提交和发布顺序

websdk 先发布（比 0.7.202 新的版本）→ buckyos 合入 main（含 `src/frame/desktop/src/shared`）→ BuckyOSApp 把 `package.json` / 锁文件里的 `buckyos` 升到那个版本再合入。开发时 `BuckyOSApp/node_modules/buckyos` 是指向同级 websdk 的符号链接，`pnpm install` 会把它换成 npm 上的旧版，编译会缺接口。

### 11.9 之后的调整：应用窗口用应用的图标

owner：桌面版弹出的应用窗口，图标应该是应用的，不是 BuckyOS 的。

- 本地页面按应用磁贴的样子画一张 128 px 的 PNG（有图片就裁成圆角方块，没有就是首字母加 `toneOf` 的颜色），新窗口建好后经 `content_set_icon` 交给 Rust（`BuckyOSApp/src/features/content/windowIcon.ts`、`src-tauri/src/content/window_icon.rs`）。Linux 设窗口图标；macOS 的窗口没有图标，不做。
- Windows：Tauri 的 `set_icon` 只设标题栏的小图标，任务栏和 Alt+Tab 用的大图标直接发 `WM_SETICON`。所有窗口原本都归在客户端一个任务栏按钮下（按钮显示客户端的图标），所以**每个应用的窗口有自己的 AppUserModelID**（“Zone + 应用”一个，同一应用的多个窗口归一组），像 Chrome 安装的网页应用：任务栏上是应用自己的按钮，可以固定，固定的按钮用 `buckyos://app.open` 重新打开应用，图标存在 `%APPDATA%\com.buckyos.buckyosapp\app-icons\`。这一条是我替 owner 选的；不要的话去掉 `claim` 即可，只剩窗口图标。
- 从应用的固定按钮冷启动时，进程会继承那个按钮的身份，主窗口也归到应用下；启动时识别这种情况（快捷方式启动且参数里有 `app.open` 链接），改回客户端自己的身份 `com.buckyos.buckyosapp`。没有一律设成客户端身份，因为 `src/publish/win_pkg/installer.nsi.in` 建的快捷方式没有 AppUserModelID（Tauri 自己的 NSIS 安装包有），一律设会让用户已固定的客户端和运行中的分成两个按钮。要统一的话，先给那份安装脚本的快捷方式加上 `com.buckyos.buckyosapp`。
- 验证：113（Windows 10）上 Message Hub（系统应用，SVG 图标）和 Text Editor（没有图片，首字母）的标题栏、任务栏、跳转列表、固定、客户端运行中和未运行时从固定按钮打开；Linux 容器里每个应用窗口有自己的 `_NET_WM_ICON`。没验：macOS（没有可做的）、Windows 11、高 DPI、安装包。
