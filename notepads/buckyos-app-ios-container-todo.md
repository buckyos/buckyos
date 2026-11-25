# BuckyOS APP iOS 内容容器 TODO

日期：2026-11-24。状态：**阶段 0 到 3 已实现，在 iOS 26.0 和 Android 16 模拟器上连 DV 验过；阶段 4（真机）没有做**。代码在 BuckyOSApp 的 `feature/ios-container`（2026-11-24 已推送到 origin，未合入）。实测结论、落点、没验证的在 §11；§1 到 §5 是动手之前的情况和方案，与实现不同的地方列在 §11.1.4。答复原话在 §6.1，我替 owner 选的做法在 §6.2。

依据：[BuckyOS APP 新版本实施 TODO](buckyos-app-system-client-todo.md)（下称主 TODO）。那里的 Q18 定了“Android 跑顺并上架后，才会安排 iOS 版本”，本版本 iOS 只保证能编译。IQ1 把**开发**提前了：现在就可以用模拟器做，目的之一是把跨平台功能的设计缺陷逼出来；**上架顺序不变，仍是 Android 优先**。主 TODO 里关于 iOS 排期的说法和本文不一致时，听本文的。

涉及仓库：BuckyOSApp（绝大部分）、buckyos-websdk（只有能力清单，可能不用改）。buckyos 服务端和 Desktop 不用改。

源码核对基线：BuckyOSApp `3a691a7`（main）、buckyos `a552208d7`、buckyos-websdk `ae5e885`（0.7.203）、Tauri 2.10.3、wry 0.54.4、Xcode 26.2（iOS SDK 26.2）。

用词沿用主 TODO：本地界面、内容视图、激活页、Zone 根域。另外：

- **主 WebView**：Tauri 建的那个 WKWebView，显示本地界面。
- **舞台**：本地界面里留给内容视图的那块区域，前端用 `useStage` 报给原生侧。

可信度：标“已实测”的是 2026-11-24 在 iOS 模拟器（26.0、26.3）上连本地 DV Zone 看到的；标“SDK”的是从 iOS SDK 头文件确认的最低版本；标“待验证”的是推断，由 IT0 回答。其余来自读代码。

阅读顺序：§0 结论，§2 现状，§3 方案，§6 owner 的答复，§7 任务，§10 接手约束，§11.2 跨平台缺陷。

## 0. 结论先行

1. **现在 iOS 上一个 Zone 页面都打不开（已实测）。** 图标、消息 Tab、HomeStation Tab、“＋”、深链都提示“没有打开成功”。`content/mobile/ios.rs` 的 `content_open`、`content_focus`、`content_layout` 等直接返回 `Unsupported`。
2. **缺的主要是原生侧，本地界面和规则基本都在。** 容器 Header、应用切换、Tab、触摸归属、返回栈、打开规则、导航策略、桥的协议都已经有，而且不分平台。登录、应用列表、Activity、文件浏览、主题色在 iOS 上已经能用（已实测）。
3. **Rust 侧先抽公共层，不要照着 Android 再写一份。** `android.rs` 有 1412 行，其中的视图状态机、登录、淘汰、重试、签名和平台无关；真正只属于 Android 的是插件注册、前台服务、`read_media`、`nudge` 这几块。抽出来以后 iOS 只要实现同一组插件方法和事件（§3.2）。
4. **原生侧用 Swift 的 Tauri 插件。** `register_ios_plugin` 和 Android 一样能用 `run_mobile_plugin_async` 调用（Tauri 2.10.3 源码）。`origin/feature/audio_record` 里有一个 Swift 插件工程和它在 `build.rs` 里的接法可参考，没有重新编译验证过。
5. **导航判断是同步的，要走 C FFI。** “这个视图能不能去这个地址”要当场回答，不经过 Tauri 的插件通道，做法对应 Android 的 `NativeBridge`。
6. **最低系统版本是 iOS 17（owner 已定，IQ2）。** 每个账号加 Zone 一个独立数据区（`WKWebsiteDataStore(forIdentifier:)`）要 iOS 17（SDK）。工程现在还是 13.0，由 IC0 改。下面提到的接口都在 17 以内，不写低版本分支。
7. **容器是多 WebView 叠放，最不确定的是触摸穿透、键盘和安全区。** 其中键盘已经有一个现存问题：键盘弹出时主 WebView 会整体上推，标题压到状态栏下面（已实测）。内容视图是按坐标摆的，主 WebView 一动就会错位，必须先解决。
8. **iOS 没有系统返回键，也不能退到后台。** 返回要靠左边缘手势接进现有的返回栈，根部不做事。
9. **通知只在前台收，后台通知另写一份 TODO（owner 已定，IQ5）。** iOS 不允许后台长连接（主 TODO R12）。现在前台能收到数据（Activity 有内容，已实测），系统提醒是空实现，也留给那份 TODO。
10. **激活页在 iOS 上还是本地页面里的 iframe，要一起改掉。** 桌面和 Android 已经因为安全原因换成独立视图（主 TODO §11.3、U8），iOS 是最后一个。
11. **第一版只支持 iPhone，音频和“拍照选图”包含在内（owner 已定，IQ3、IQ6、IQ7）。** 网页直接调摄像头仍然拒绝，和 Android 一致。
12. **开发分两段：先模拟器，后真机。** 模拟器能回答大部分 IT0；内存、后台、锁屏播放、本地网络权限、摄像头、录音只有真机算数，列在阶段 4，上架之前补完。

## 1. 目标与完成的样子

iOS 客户端能和 Android 一样打开、切换、关闭 Zone 应用：

- 图标、Tab、Activity、深链、另一个应用，五个入口按同一套打开规则打开页面，页面自动登录，用户看不到 Zone 的登录页。
- 同时开几个应用来回切换不重新加载；超过上限按最近最少使用淘汰。
- 应用通过 `BuckyApi` 使用客户端能力，能力清单如实反映 iOS 有什么。
- 一个应用页面卡死或崩溃不影响本地界面。
- 激活一台设备的页面走独立视图，不再用 iframe。
- 应用能后台播放、录音；网页选文件时能现拍照片。
- 过程中发现的跨平台设计缺陷记在 §11.2，归到公共代码里修，不在 iOS 上绕过。

## 2. 现状

### 2.1 已有、可以直接用的

| 部分 | 位置 | 说明 |
| --- | --- | --- |
| 容器界面 | `src/features/container/AppContainer.tsx`、`AppSwitcher.tsx`、`header.ts` | 两种 Header、应用切换、更多、最小化、搜索，全部是本地界面 |
| 舞台和触摸归属 | `src/features/container/stage.ts`、`src/features/content/mobileViews.ts` 的 `ViewStage`、`Overlays` | 用 `content_layout`、`content_overlay` 把位置和挡住的区域告诉原生侧，单位是 CSS px，正好等于 iOS 的 point |
| 返回栈 | `mobileViews.ts` 的 `BackStack`、`src/app/RuntimeHost.tsx` | 只差 iOS 的触发来源（§3.6） |
| Tab 页 | `src/app/phone/PhoneTabPage.tsx`、`FeedPage.tsx` | 与平台无关 |
| 打开规则、应用桥的宿主 | `src/features/content/clientRuntime.ts`、`openService.ts`、`frameHosts.ts` | 与平台无关 |
| 导航策略 | `src-tauri/src/content/policy.rs` | `decide`、`decide_new_window`，有单测 |
| 桥的协议 | `src-tauri/src/content/bridge.rs`、`public/bucky-api.js` | 移动端走 `__buckyPort`，iOS 只要提供一个同样形状的对象 |
| 淘汰与命名 | `src-tauri/src/content/shelf.rs` | `to_discard`、`profile_name`（16 字节哈希，可以直接当 UUID 用） |
| 会话与回调地址 | `src-tauri/src/zone/session.rs` | `launch_url`、`separate_session` |
| 保存登录、系统证书 | `zone/secret.rs`、`http_client.rs`（`3a691a7`） | iOS 用 Keychain 和系统证书校验；模拟器已实测，真机没测 |
| Home 音乐的取流 | `src-tauri/src/zone/media.rs` 的 `respond` | 非 Android 走自定义协议；iOS 上没验证播放 |

### 2.2 iOS 现在的占位

| 位置 | 现状 |
| --- | --- |
| `src-tauri/src/content/mobile/ios.rs` | 命令都在，`content_open`、`content_layout`、`activation_open`、`native_status` 等返回 `Unsupported`，`content_list` 是空的；`sign_json` 返回 `unsupported` |
| `src-tauri/src/audio/mobile.rs` | iOS 的 `dispatch`、`audio_hold` 返回 `Unsupported` |
| `src-tauri/src/notify/alerts/ios.rs` | 不弹通知，`permission` 返回 `None` |
| `src-tauri/src/content/bridge.rs` 的 `capabilities()` | iOS 列了 `notify.system`，实际弹不出来；没有 `header.native` |
| `src/features/devices/ActivationContext.tsx`、`src/utils/webview.ts` 的 `mayEmbed`、`src/pages/WebContainer.tsx` | iOS 的激活页走 iframe |

### 2.3 工程现状

- 只有 `gen/apple/Sources/buckyosapp/main.mm` 一个原生源文件，没有 Swift 代码，没有插件。
- 最低系统 13.0（要改成 17.0，IC0），`Podfile` 没有实际依赖，`project.yml` 还带着 `tauri-test` 的旧名字，和 `project.pbxproj` 不一致（本机没有 xcodegen，pbxproj 是 Tauri 改的）。
- `TARGETED_DEVICE_FAMILY` 是 `1,2`（第一版只做 iPhone，要改成 `1`，IC0），iPhone 和 iPad 都只声明了竖屏。
- 没有 `NSAppTransportSecurity`、`UIBackgroundModes`；entitlements 为空；没有签名团队；`ExportOptions.plist` 是 debugging。
- CI 没有 iOS（`.github/workflows` 只有 android、macos、windows）。
- 每次 `tauri ios build` 会改写 `Info.plist`、`project.pbxproj` 和 entitlements 的换行，仓库里提交的版本和构建后的不一致。

### 2.4 2026-11-24 模拟器实测到、和容器有关的现象

| 现象 | 影响 |
| --- | --- |
| 键盘弹出时主 WebView 整体上移，“登录 Zone”弹窗打开时 Home 的标题压到状态栏下面 | 内容视图的坐标会和本地界面错开，IC3 必须解决 |
| 系统选择框（`<select>`）和输入附件条正常；“添加 Zone”页面的提交按钮会被键盘附件条挡一部分 | 本地界面的小问题，和 IC3 一起处理 |
| 触摸后应用图标、设置行一直保持高亮（`:hover` 粘住） | 样式问题，`IU2` |
| 从 Home 的账号菜单进 Zone 列表，返回去的是设置页 | 现有规则（`back.ts`），边缘返回手势会更常用到，要一起看 |
| iOS 26.3 模拟器的 WebKit（包括 Safari）把中文画成方块，26.0 正常 | 测试环境问题，用 26.0 |
| WebView 不信任 DV 的私有 CA 时图片不显示；装上 CA 就正常 | 内容视图同理，测试前先让模拟器信任 CA（§10） |

## 3. 方案

### 3.1 视图结构

```
UIWindow
└─ 根视图（tao）
   └─ ContainerView（新）
      ├─ contentHost      各内容视图（WKWebView），一次只显示一个
      ├─ mainHost         透明，里面是 Tauri 的主 WebView；按区域放行触摸
      └─ 全屏视频用的宿主   由系统全屏时用不到
```

和 Android 的 `ContainerLayout` 一样：内容视图在下，透明的主 WebView 在上。插件 `load(webview:)` 时把主 WebView 移进 `mainHost`（Tauri 在 WebView 建好时调用它；那一刻是否已经进了视图树待验证，没有就推迟到下一轮主循环）。

- **主 WebView 要透明。** 用公开接口 `isOpaque = false`、`backgroundColor = .clear`、`scrollView.backgroundColor = .clear`，不用 wry 的 `transparent`（它在 iOS 上写私有键 `drawsBackground`，审核有风险）。本地界面已经有 `html[data-content-view]` 的透明样式。待验证：切换主题和冷启动时不闪白。
- **触摸穿透。** `mainHost` 重写 `hitTest(_:with:)`：落点在放行区内、不在挡住的区块里、也没有模态层时返回 `nil`，交给下面的内容视图。规则和 `ContainerLayout.kt` 的 `MainHost` 相同。读屏（VoiceOver）走的是另一条路，要另外处理 `accessibilityElementsHidden`（待验证）。
- **位置。** `content_layout` 给的矩形直接当 point 用，不用乘屏幕倍率。
- **隐藏。** 隐藏的内容视图用 `isHidden`，保留在视图树里；用 `inactiveSchedulingPolicy`（iOS 17，SDK）。后台播放的视图要 `none`，其它用 `throttle`，由 IT0-4 定。
- **淘汰。** 沿用 `MAX_KEPT = 5`，按 iOS 真机内存再定。

### 3.2 Rust 公共层

把 `android.rs` 里和平台无关的部分移到 `content/mobile/views.rs`：

| 留在公共层 | 只属于平台 |
| --- | --- |
| `View`、`ContentViews`、`set_stage`、`set_front`、`shelve`、`start`、`retry_later`、`prime_root`、`sign_in_again`、`at_page`、`follow`、`on_failed`、`on_gone`、`dispatch`、全部 `#[tauri::command]`、激活页、签名的流程、Zone 变化的处理 | 插件怎么注册、`secrets()`、`audio` 的原生部分、Android 的前台通知服务（`service`、`notifications_changed`、`alert`）、`read_media`、`nudge`、`status` 的内容 |

平台层只剩一组插件方法。iOS 用同样的名字和 JSON 参数，Rust 这边不用分叉：

| 方法 | iOS 做法 |
| --- | --- |
| `create` | 按 `profile` 取数据区，建 `WKWebView`，加脚本和消息处理器，加进 `contentHost`，先隐藏 |
| `load` | `load(URLRequest)`，并记下这次到达后的历史下限（§3.4） |
| `goTo` | `load(URLRequest)`，不动历史下限 |
| `show` | 切换显示、摆位置、设背景、设放行区 |
| `overlay` | 更新模态标记和挡住的区块 |
| `close` | 停止加载，移除视图，释放 |
| `eval` | `evaluateJavaScript`，页面世界 |
| `back`、`stop` | 见 §3.4、§3.6 |
| `reply` | 用消息处理器的 `replyHandler` 返回 |
| `getCookies`、`setCookies`、`clearProfile` | 见 §3.3 |
| `signPrompt`、`signDone` | 原生弹窗，见 §3.7 |
| `askMicrophone` | 先客户端问一次，再交给系统，见 §3.8 |
| `openSettings` | `UIApplication.openSettingsURLString` 一种 |
| `status` | 系统版本、是否有独立数据区 |
| `toBackground` | iOS 没有，直接返回 |
| `playback`、`record` | 阶段 3 |
| `notificationPermission`、`service`、`alert` | 不在本文（§4） |

原生侧发给 Rust 的事件，名字和 Android 一样：`started`、`visited`、`finished`、`failed`、`title`、`gone`、`download`、`resumed`、`paused`、`online`，加 `media`、`record`（阶段 3）。

**拆的时候保持 Android 行为不变。** 这一步单独一个提交，先过 Android 的现有验证（主 TODO §11.3 的模拟器用例），再写 iOS。

### 3.3 数据区和登录

- **每个账号加 Zone 一个数据区。** `WKWebsiteDataStore(forIdentifier:)`，UUID 取 `profile_name` 那个哈希的 16 字节（iOS 17，SDK）。wry 也是这样建的，但内容视图不经过 wry。
- **激活页**用 `WKWebsiteDataStore.nonPersistent()`，打开一次就是新的，不用像 Android 那样先清。
- **清除**（退出登录、移除 Zone）：先关掉用着它的视图，再 `removeData(ofTypes: 全部)`；确认没有视图在用之后再 `remove(forIdentifier:)`，删不掉的记下来，下次启动时用 `fetchAllDataStoreIdentifiers` 对照清理（SDK，iOS 17）。
- **写根域的 Cookie**（`prime_root`）：用 `HTTPCookie.cookies(withResponseHeaderFields:for:)` 从 `Set-Cookie` 文本生成，再写进 `httpCookieStore`。待验证：`HttpOnly`、`SameSite=Lax`、`Max-Age` 都能保住。不行就改成加载一个由服务端下发 Cookie 的地址。
- **登录过程不变**：原生侧直接加载 `launch_url` 给的一次性回调地址，不经过页面脚本；`/sso_callback` 只有客户端发起的那一次放行。
- **落盘。** Android 要主动 flush；WKWebView 由系统管，待验证：刚轮换的会话 Cookie 在杀进程之后还在。

### 3.4 导航

- `WKNavigationDelegate.decidePolicyFor`（navigationAction）里区分主框架和子框架：`targetFrame.isMainFrame`，`targetFrame == nil` 就是新窗口。macOS 上为了拿到这一点换了 wry 的代理方法（`content/windows.rs` 的 `mac_frames`），iOS 的视图是自己建的，不需要。
- 判断要当场得到。Swift 调 Rust 的 `extern "C"` 函数 `bucky_on_navigate(view, url, main_frame, new_window) -> bool`，对应 Android 的 `on_navigate`，在主线程上调，**里面不能等**，也不能问当前身份（主 TODO 记过 Android 的死锁）。
- 表单 POST 也会到 `decidePolicyFor`，不用像 Android 那样在页面开始时补判（待验证）。
- 新窗口：`createWebViewWith` 一律返回 `nil`，把地址交给 `decide_new_window`。`target=_blank` 和 `window.open` 都走这里。
- 地址和标题用 KVO 观察 `url`、`title`，这样单页应用的 `pushState` 也能记到 `info.path`。
- **历史下限。** Android 登录到达后清空历史，WKWebView 没有公开的清空接口。做法：记下到达后的 `backForwardList.currentItem`，`content_back` 不退到它之前；手势返回也用同一个判断。禁用 `allowsBackForwardNavigationGestures`（§3.6）。
- 出错：`didFailProvisionalNavigation` 和 `didFail` 区分“主动取消”（`NSURLErrorCancelled`、`WebKitErrorFrameLoadInterruptedByPolicyChange`）和真失败，只有后者报 `failed`。
- 证书：不覆盖默认的证书校验，错误就是错误，不做例外（主 TODO R10）。
- 进程退出：`webViewWebContentProcessDidTerminate` 报 `gone`，后面的处理和 Android 一样（前台自动重来一次，一分钟内第二次就显示错误）。iOS 在内存紧张和后台时会更频繁地这样做（待验证），不能当成异常提示用户。

### 3.5 桥

- 用 `WKScriptMessageHandlerWithReply`（iOS 14，SDK），名字固定，只注册在内容视图的 `userContentController` 上。
- 在 `API_SCRIPT` 之前注入一小段，把它包装成 `__buckyPort`（`postMessage(text)`、`onmessage`），`bucky-api.js` 不用改：移动端本来就是检查 `window.__buckyPort`。
- **来源由原生判断。** Android 用 `addWebMessageListener` 的来源规则限定页面；iOS 的消息处理器对视图里的每个页面和每个框架都可见，所以：
  1. 脚本用 `WKUserScript(injectionTime: .atDocumentStart, forMainFrameOnly: true)` 注入；
  2. 注入的脚本先看 `location.origin` 是不是这个 Zone 的 https 地址，不是就不定义 `__buckyPort` 和 `__BUCKY_ENV__`。`decide` 允许 `about:`、`data:`、`blob:` 在顶层打开，没有这一步它们会拿到环境信息；
  3. 处理器把 `frameInfo.isMainFrame` 和 `frameInfo.securityOrigin` 交给 Rust，Rust 的 `dispatch` 照旧要求“主框架、https、主机等于视图的 `page_host`、不是 sys 主机”。页面绕过包装直接调消息处理器也一样被这里拦住。
- 用 `WKContentWorld.page`，因为应用页面要直接用 `window.BuckyApi`。
- **能力清单**按 iOS 真实情况列：`app.open`、`header.native`；阶段 3 再加 `audio.background`、`audio.record`（做完哪个列哪个）。**去掉 iOS 的 `notify.system`**，直到通知真的能弹。
- 把 Rust 的事件推给页面：`bridge::emit_script` + `eval`，和 Android 相同。

### 3.6 返回、生命周期、屏幕

- **返回。** 在 `ContainerView` 加 `UIScreenEdgePanGestureRecognizer`（左边缘），手势完成时触发现有的 `BackStack`。Tauri 的 `onBackButtonPress` 是给 Android 的，iOS 上现在只会打一行日志。新增事件名不要和 Android 的混在一起，前端在 `BackStack.start` 里按平台选来源。
- 内容视图关掉 WebKit 自带的左右滑动返回，否则会绕过应用的 `frame.back` 和历史下限。
- 根部（Home）左滑不做事。
- **进出后台。** `UIApplication` 的 `willResignActive`、`didBecomeActive` 报 `paused`、`resumed`。恢复时重试连不上的视图、唤醒通知连接，逻辑已在公共层。
- **网络恢复。** `NWPathMonitor` 报 `online`，Rust 换掉连接池（`transport().reset()`，现在只有 Android 调用）。
- **安全区。** 本地界面已经自己留了安全区（`viewport-fit=cover`、`ios_webview.rs` 关掉了自动内边距）。内容视图摆在舞台里，同样设 `contentInsetAdjustmentBehavior = .never`，避免叠加。透明 Header（compact）时页面会延伸到刘海下方，页面的 `env(safe-area-inset-*)` 取值待验证。
- **键盘。** 两件事分开处理：
  - 在本地界面里打字：主 WebView 的滚动要锁住，不让它整体上移；`--keyboard-inset-bottom` 现在只有 Android 写，iOS 要补（`UIResponder.keyboardWillChangeFrameNotification`）。
  - 在内容视图里打字：舞台底边按键盘收起，和 Android 的 `place()` 相同；只在焦点在内容视图里时收。
- **主题。** 用户在客户端里选了和系统不同的明暗时，状态栏文字、键盘、系统弹窗要跟着变（`overrideUserInterfaceStyle`），对应 Android 的 `BuckySystemBars.setTheme`。
- **竖屏。** 全屏视频时放开横屏，退出恢复。

### 3.7 签名

页面请求签名时，用原生弹窗显示 Zone、应用、身份和要签的内容，并输入钱包密码。页面不能画、不能读、不能自己答。要求和 Android 的 `Dialogs.SignPrompt` 一致：内容可滚动可选中；密码框不进系统建议；错了留在弹窗里重输；视图关掉时按拒绝处理；同一时刻只有一个。文案来自 `native_set_labels` 写的文件，iOS 也要读它。

### 3.8 网页能力

| 能力 | iOS 做法 |
| --- | --- |
| 麦克风 | `requestMediaCapturePermissionFor`（iOS 15，SDK）。只允许本 Zone 的 https 页面、只给音频，每个视图先由客户端问一次。摄像头一律拒绝（IQ6，和 Android 相同） |
| 定位 | 不加用途说明，系统自然不给 |
| 选文件 | `<input type=file>` 由 WebKit 自带，不用实现；“拍照”一项要支持（IQ6），依赖 `NSCameraUsageDescription`（已在 `src-tauri/Info.plist`）。现在的用途说明写着“拍照或通话”，通话用不到摄像头，改成只说拍照。模拟器没有摄像头，只能在真机上验 |
| 下载 | `WKDownload`（iOS 14.5，SDK）。存到应用的文稿目录并在“文件”App 里可见，完成后弹提示可分享。`download(_:willPerformHTTPRedirection:...)` 里跳出 Zone 就取消。`blob:`、`data:` 下载待验证，不行就移植 Android 的页面取字节做法 |
| 视频全屏 | 打开 `isElementFullscreenEnabled`（iOS 15.4，SDK） |
| 内联播放、自动播放 | `allowsInlineMediaPlayback = true`，`mediaTypesRequiringUserActionForPlayback = []`，和 Android 一致 |
| `alert`、`confirm` | 要实现 `WKUIDelegate` 对应方法，否则静默失败；标题写上应用和主机 |
| 调试 | 只在 debug 构建里打开 `isInspectable`（iOS 16.4，SDK） |

### 3.9 激活页

`activation_open` 按 Android 的做法建独立的、不持久的视图，只给五个钱包方法（`bridge::WALLET_METHODS` 已经限定）。前端删掉 `isIosShell` 走 iframe 的分支、`mayEmbed` 的 iOS 特例，`WebContainer` 只留开发用的测试页。

设备的激活地址是 `http://<局域网 IP>:端口`，受 ATS 限制：IT0-6 要试出最小的放行配置（优先 `NSAllowsLocalNetworking`），不要用 `NSAllowsArbitraryLoads`。真机上还有本地网络权限弹窗（用途说明已有）。

## 4. 范围外

- 系统通知的弹出、APNs、后台收通知（主 TODO Q9、N7、R12）：owner 定要单独写一份 TODO（IQ5）。容器只要保证“从通知打开”的入口规则能用。
- 统一采集再分发录音（主 TODO A4）。
- iPad（IQ3，第一版不支持）、多窗口、外接屏。
- App Store 上架本身（签名团队、审核材料）。AGPL 与商店条款是法务问题，owner 会单独讨论，不影响开发（IQ4）。
- 网页直接调摄像头（IQ6）。
- 移动 Web UI 的容器（主 TODO Q27）。

## 5. 风险

| 编号 | 风险 | 处理 |
| --- | --- | --- |
| IR1 | 把 Tauri 的主 WebView 移进新的父视图，可能影响 wry 的尺寸、安全区或输入 | IT0-1 先做；不行就不移动它，改在它下面插内容宿主，用代理的 `hitTest` 放行 |
| IR2 | 抽公共层时改坏 Android | IC1 单独提交，先跑 Android 现有用例。这台 Mac 没有装 Android 的 Rust 目标（2026-11-24），回归要先在这里补环境，或到有 Android 环境的机器上跑 |
| IR3 | iOS 17 以下的设备不能装 | owner 已接受（IQ2）。不为低版本共用默认数据区 |
| IR4 | 同时几个 WKWebView 的内存在真机上被系统杀 | 上限先 5，`gone` 当成常态；真机测完再定 |
| IR5 | 后台播放：WebView 里的声音在锁屏后被停 | IT0-5；不行就用原生播放器，接口不变（主 TODO §3.7 同样的退路） |
| IR6 | 商店审核：壳应用（4.2）、小程序类（4.7）、私有接口 | 不用 wry 的私有透明键；上架前单独评估 |
| IR7 | 在 iPad 上以 iPhone 兼容模式运行时 UA 可能不同，`isMobileShell()` 判错 | 不支持 iPad（IQ3），不处理；IC0 把设备族改成只有 iPhone |
| IR8 | 现在的 iOS 工程文件和 Tauri 生成的不一致，构建后工作区总是脏的 | IC0 先定一份干净的 |
| IR9 | 签名弹窗、桥、激活页是安全边界，iOS 没有 Android 那样的来源规则 | §3.5 的三层检查；IC9 的攻击用例 |
| IR10 | 只用模拟器开发，真机问题集中在最后暴露 | 阶段 4 列清楚；有真机时尽早先跑 IT0 里标了“真机补”的几项 |

## 6. owner 的答复

### 6.1 已答复（2026-11-24）

“答复”一栏是 owner 的原话。IQ2 是对话里答的，其余是 owner 写在本文里的，IQ6 又在对话里确认过一次。

| 编号 | 问题 | 答复 | 落实 |
| --- | --- | --- | --- |
| IQ1 | 什么时候开工 | 想了下，可以先开工用模拟器测试。至少可以把一些跨平台功能的设计缺陷逼出来。产品上的上架顺序依旧是android优先 | 开头、§7、§9、§11.2 |
| IQ2 | 最低系统版本 | 定 iOS 17 | §0 第 6 条；IC0 |
| IQ3 | 第一版支持 iPad 吗 | 不支持 | §4；IC0 |
| IQ4 | 怎么发布、AGPL 怎么处理 | 不影响开发，这个是个法务问题会单独讨论 | §4 |
| IQ5 | 后台通知 | iOS现在前台收。这个肯定是需要做个专门的TODO的。 | §0 第 9 条、§4；专门的 TODO 还没写 |
| IQ6 | 摄像头 | 应该支持。（确认：只支持拍照选图） | §3.8；IC12、IV5 |
| IQ7 | 音频放不放进第一版 | 可以放进来 | 阶段 3 属于第一版 |

### 6.2 我替 owner 选的做法

| 事项 | 我选的 | 要改的话动哪里 |
| --- | --- | --- |
| “前台收”包不包括系统横幅 | 不包括。前台只保证 Activity 和 Tab 上的未读数；系统横幅和点击唤起随专门的通知 TODO 一起做 | §0 第 9 条、IC6、IU3 |
| “先开工用模拟器测试”对 IT0 的要求 | IT0 以模拟器结论为准就可以往下做；只有真机能回答的列到阶段 4，在上架前补 | IT0、阶段 4 |
| 发现跨平台缺陷怎么处理 | 记到 §11.2，在公共代码里修，修完 Android 也要回归；改动大的先报告再动 | §10、§11.2 |
| 音频进第一版后的顺序 | 仍排在容器之后，容器先能用；第一版要等阶段 3 完成才算做完 | §9 |
| 代码合入 main | 可以合入，合入前 Android 现有用例要过；不做 iOS 的发布包 | §10 |

## 7. 任务

### 阶段 0：先验证（IT0）

**勾选的意思是模拟器部分有结论**（§11.1.1），“真机补”仍在阶段 4。

在单独的分支里做最小试验，不合入。每项给结论和选定的做法，写进 §11。先出模拟器的结论就可以往下做（IQ1）；标了“真机补”的在阶段 4 用真机复核。

- [x] **IT0-1 视图叠放。** 主 WebView 放进 `mainHost` 并设透明，下面放一个 WKWebView；触摸按区域穿透、弹层不穿透、VoiceOver 能分辨。冷启动、切主题、旋转、进出后台都试。
- [x] **IT0-2 数据区与 Cookie。** `forIdentifier` 建两个 Zone 的数据区，互不可见；写入的 Cookie 带齐 `HttpOnly`、`SameSite`；一次性回调登录成功；杀掉 app 重开还在；清除和删除数据区。
- [x] **IT0-3 桥。** `WithReply` 的往返；子框架、`data:`、`about:`、别的 Zone 的页面能不能拿到桥；确认 Rust 侧拦住。顺带确认现在激活页 iframe 能不能碰到 Tauri 的 IPC。
- [x] **IT0-4 存活。** 3 到 5 个内容视图同时开着的内存；隐藏后定时器、长连接、播放是否继续；退后台 1、5、15 分钟回来；进程被杀后的 `gone`。（真机补：内存上限、后台挂起）
- [x] **IT0-5 音频。** `UIBackgroundModes: audio` 加 `AVAudioSession`，WebView 里的 `<audio>` 切后台、锁屏是否继续；`zonemedia` 自定义协议在主 WebView 里放 MP3、拖动、断续。（真机补：锁屏、来电打断、蓝牙）
- [x] **IT0-6 局域网。** 设备扫描和激活页能打开；最小的 ATS 配置（模拟器上先试）。（真机补：本地网络权限弹窗）
- [x] **IT0-7 键盘和安全区。** 在本地界面和内容视图里分别打字，舞台不错位；透明 Header 下页面的安全区取值；左边缘返回手势不和页面的横滑冲突。
- [x] **IT0-8 插件接入。** 用 Swift Package 加 `register_ios_plugin`，参考 `origin/feature/audio_record` 的 `src-tauri/ios/`、`build.rs`；确认模拟器的 debug、release 都能编译；（真机补：签名后能装能跑）Swift 调 Rust 的 C FFI 能用。

完成条件：§3 里每个“待验证”都有实测结论；定下 Swift 还是 Rust（objc2）写原生侧。

### 阶段 1：能打开和切换应用

- [x] **IC0 工程清理。** `project.yml` 去掉 `tauri-test`，和 `pbxproj` 一致；提交一份构建后不再变的 `Info.plist`、entitlements；最低版本改成 17.0，几处一起改：`project.yml` 的 `deploymentTarget`、`project.pbxproj` 的 `IPHONEOS_DEPLOYMENT_TARGET`（两处）、`Podfile`、`tauri.conf.json` 的 `bundle.iOS.minimumSystemVersion`、Swift 插件的 `Package.swift`；设备族改成只有 iPhone：`project.yml`、`project.pbxproj` 的 `TARGETED_DEVICE_FAMILY`（两处）改成 `1`，去掉 `UISupportedInterfaceOrientations~ipad`；摄像头的用途说明改成只说拍照；加 ATS、横屏等设置；构建脚本说明写进 README。旧产物会让 `tauri ios build` 最后一步失败，脚本里先清 `gen/apple/build/arm64-sim`。
- [x] **IC1 Rust 公共层。** 按 §3.2 拆出 `views.rs`。Android 行为不变。（单独提交 `b9d7618`；Android 回归见 §11.1.7）
- [x] **IC2 插件骨架和内容视图。** `create`、`load`、`goTo`、`close`、`show`、`overlay`、`eval`、`back`、`stop`；事件回传；能加载一个 https 页面。
- [x] **IC3 布局。** 触摸穿透、键盘、安全区、主题、旋转；锁住主 WebView 的滚动。
- [x] **IC4 登录和数据区。** §3.3。图标、Tab、深链打开应用，全程不出现 Zone 登录页；Cookie 失效后自动重登。
- [x] **IC5 导航。** §3.4，接 `policy.rs`；新窗口；历史下限；错误页和自动重试；`alert`、`confirm`。
- [x] **IC6 桥与能力。** §3.5；能力清单改成真实的；`frame.*`、`app.open`、Header 声明都能用。
- [x] **IC7 返回和生命周期。** §3.6。
- [x] **IC8 打开失败的提示。** 把现在笼统的“没有打开成功”改成有原因的：iOS 还没做的功能明确写“iOS 暂不支持”，其它照 `ViewStatus` 显示。

完成条件：Message Hub、HomeStation、AI 工作区、文件、文本编辑器五个应用都能打开、登录、来回切换；Tab 与图标是同一个视图；切换 Zone 后各归各的 Zone；断网恢复后自动继续。（两个 Zone、真实断网没有验，§11.1.5）

### 阶段 2：安全、激活、网页能力

- [x] **IC9 安全检查。** 子框架、`data:`、`about:`、别的 Zone 页面、外站调桥被拒；`/sso_callback` 伪造被拦；一次性登录地址不进页面脚本；Zone 登录页不露出；激活页只有钱包方法。把主 TODO §11.3 里那套攻击用例在 iOS 上跑一遍，脚本不在仓库里，要重写。
- [x] **IC10 签名。** §3.7。
- [x] **IC11 激活页。** §3.9；删掉 iframe 分支和 `mayEmbed` 的 iOS 特例。
- [x] **IC12 下载、选文件（含拍照选图）、麦克风、全屏视频。** §3.8。网页直接调摄像头要被拒绝。

完成条件：IC9 的用例全过；一台未激活的设备能在 iOS 上走完激活并自动进入新 Zone。（前半句过了；后半句只验到假设备页的扫描、打开、签名，没有真的未激活设备和 SN，§11.1.5）

### 阶段 3：音频（属于第一版）

- [x] **IA1 播放保活。** `audio.background`：后台模式、`AVAudioSession`、`MPNowPlayingInfoCenter`、`MPRemoteCommandCenter`（锁屏控制）。
- [x] **IA2 录音。** `audio.record.*`，参考 `origin/feature/audio_record`，走新桥，不合并那个分支。
- [x] **IA3 Home 音乐。** 在 iOS 上放 DV 的真实音乐。

完成条件：模拟器上应用切后台继续播放，锁屏控制能暂停和继续；能录音、列出、取回、删除；能力清单列出 `audio.background`、`audio.record`。真机项在阶段 4。

### 阶段 4：真机补验

不挡开发，挡发布。需要一台 iOS 17 以上的真机和一个签名团队。

- [ ] **IV1 装得上、跑得起。** 签名、debug 和 release；Keychain 保存登录，卸载重装后的表现。
- [ ] **IV2 内存和后台。** 3 到 5 个视图的内存上限，定 `MAX_KEPT`；退后台 1、5、15 分钟回来；被系统结束后的恢复。
- [ ] **IV3 音频。** 锁屏和后台连续播放 30 分钟；来电和其它应用打断；蓝牙和耳机控制；录音质量与权限。
- [ ] **IV4 局域网。** 本地网络权限被允许和被拒绝两种情况下的扫描与激活。
- [ ] **IV5 拍照选图。** 现拍一张传到 Zone；拒绝权限时的提示。
- [ ] **IV6 弱网和切网。** Wi-Fi 与蜂窝切换、飞行模式，前台视图自动恢复。
- [ ] **IV7 最低版本。** iOS 17 上过一遍打开、切换、登录、退出。

### 界面

- [x] **IU1 iOS 的返回手势动画和边缘冲突。**
- [x] **IU2 触摸后 `:hover` 粘住。** 用 `@media (hover: hover)` 包住样式。
- [ ] **IU3 通知文案。** 系统通知随专门的 TODO 做了以后把“iOS 版暂时不显示系统通知”去掉（`notify_ios_note`）。

## 8. 验收

对应主 TODO §8 的移动端场景，开发完成以 iOS 26.0 模拟器为准，第一版发布前再过阶段 4 的真机项。iOS 17（最低版本）至少过一遍打开、切换、登录、退出：2026-11-24 这台开发机上只有 18.3、18.4、26.0、26.3 的模拟器，要另装 17 的运行时或用真机（IV7）。Zone 用本地 DV 或三 Zone 测试环境。

| 场景 | 通过标准 |
| --- | --- |
| 打开应用 | 从图标、Tab、Activity、深链、另一个应用五个入口打开，规则和 Android 一致；不出现 Zone 登录页 |
| 切换 | 打开三个应用来回切换不重新加载；最小化回 Home；超过上限的被淘汰后再进自动重载 |
| 触摸 | 内容视图能滑动、点击；Header、弹窗、菜单、应用切换盖在上面时点不到下面 |
| 键盘 | 在本地界面和应用里打字，舞台不错位、不遮住输入框 |
| 返回 | 左边缘手势先走应用的 `frame.back`，再退应用历史，再最小化；不会退回登录往返 |
| 两个 Zone | 各自的应用互不串 Cookie 和存储；切 Zone 后原 Zone 的视图关掉 |
| 会话 | 应用的 Cookie 失效后自动重登；退出登录后页面数据清空 |
| 崩溃 | 让一个应用页面卡死或崩溃，本地界面仍能操作，前台的应用自动恢复一次 |
| 断网 | 断网后显示原因，恢复后自动继续 |
| 桥 | 应用能读环境、用 `app.open` 和 Header；能力清单没有做的不列 |
| 权限 | IC9 的攻击用例全部被拒 |
| 签名、激活 | 原生弹窗签名；激活设备走完并进入新 Zone |
| 私有证书 | 设备信任了 Zone 的 CA 就能打开；没信任时报错，不提供忽略 |
| 重启 | 登录保留；视图不保留（和 Android 相同） |
| 音频 | 应用切后台继续播放，锁屏控制可用；录音能存能取（锁屏时长、打断在 IV3） |
| 拍照选图 | 网页选文件时能现拍（IV5）；网页直接调摄像头被拒绝 |
| 前台通知 | 客户端在前台时，新消息进 Activity、未读数更新；不弹系统横幅 |

## 9. 建议顺序

1. 七个问题都已答复，从 IT0 和 IC0 开始，全部先在模拟器上做。
2. IT0-1、IT0-2、IT0-3、IT0-8 并行，它们决定能不能做、用什么写；其余 IT0 跟上。
3. IC0、IC1 可以在 IT0 期间做，对 Android 也有用，单独提交。
4. IC2 → IC3 → IC4 → IC5 → IC6 → IC7；IC2 完成后 IC3、IC4 可以并行。
5. 阶段 2 在 IC6 之后；IC9 的用例边做边加，不要留到最后。
6. 阶段 3 依赖 IT0-5 的结论，排在容器之后；第一版包含它。
7. 阶段 4 等有真机和签名团队时做，不挡前面的开发。
8. 后台通知的 TODO 单独写，可以和阶段 1 并行。

## 10. 接手时必须遵守的约束

主 TODO §10 的约束全部有效，另外：

- **上架顺序。** Android 优先；本文只把 iOS 的开发提前，不做 iOS 的发布。
- **跨平台缺陷先记后修。** 做 iOS 时发现公共代码或 Android 也有的问题，写进 §11.2，在公共层修并回归 Android，不在 iOS 上绕过。
- **Rust 不为 iOS 另写一份。** 状态、登录、导航判断只有公共层一份；发现要在 iOS 分叉，先说明原因。
- **主线程。** `bucky_on_navigate` 和事件回调都在主线程上，里面不能等、不能问当前身份、不能发同步的插件调用。
- **不用私有接口。** 包括 wry 的 `transparent`、`_removeAllItems` 一类。
- **凭据不进文件。** 摘要和令牌走 Keychain（`3a691a7`）。Keychain 的内容一般在卸载后仍在，重装导入同一身份可能自动登录，要在设计里知道这一点。
- **模拟器测试**（2026-11-24 踩过的）：
  - 用 iOS 26.0 的设备；26.3 的 WebKit 没有中文字形。
  - DV 用私有 CA，WKWebView 和客户端都要模拟器信任它：`xcrun simctl keychain <udid> add-root-cert /opt/buckyos/etc/ca.cert`。装之前问 owner。`simctl keychain reset` 清不掉根证书；撤的时候关机后删 `data/private/var/protected/trustd/private/TrustStore.sqlite3` 里那一条，或者换一台模拟器。
  - 登录 Zone 之前客户端要有本地身份，注册和导入都要 SN（`sn_host` 读客户端配置目录的 `config.json`）。测试可以用一个从单测助记词生成的临时钱包，不要用真实身份。
  - 每次构建后核对二进制时间再装：旧产物会让 `tauri ios build` 报“Directory not empty”，`build/arm64-sim` 里留下的是旧包，新包在 `buckyosapp_iOS.xcarchive` 里。
  - 不要手工还原 entitlements，Xcode 会报“modified during the build”；真出现了对工程做一次 `xcodebuild clean`。
- **不要碰别的会话用着的模拟器**，先看 `simctl list` 和哪个在用。
- **文档同步。** 做完一项改 BuckyOSApp 的 `doc/当前需求文档.md`、`doc/BuckyApi.md`，以及主 TODO 的 Q18、§4、§11.7 里关于 iOS 的说法。

## 11. 实施记录

### 11.1 记录

实施时在这里追加：IT0 的结论、每个任务的落点和验证、与正文不同的地方、新的待定问题。

- 2026-11-24：owner 定在这台 Mac 上装 Android 环境（接受 `android-sdk-license`、`android-sdk-arm-dbt-license`），Android 回归做完，见 §11.1.7；`test/ios/` 改成两端共用的 `test/mobile/`。回归中发现并修了 X16–X18。
- 2026-11-24：阶段 0 到 3 和界面项做完，见下面 §11.1.1 到 §11.1.6。没有另开分支做 IT0 的最小试验：插件一接通就直接在正式代码上验，结论回填到这里。
- 2026-11-24：owner 答复 IQ1、IQ3–IQ7（IQ6 在对话里确认为“只支持拍照选图”）。据此改写：开发提前到现在、先用模拟器；只支持 iPhone；音频进第一版；加阶段 4（真机补验）、§6.2、§11.2；通知拆出去另写 TODO。`3a691a7` 已进 main。还没有任何实现。
- 2026-11-24：owner 定最低版本 iOS 17（IQ2），正文已按此改写，工程设置还没动（IC0）。
- 2026-11-24：写成本文，没有实现。同一天先做的相关修复在 `3a691a7`（Keychain、系统证书、通知文案），见上面的基线。

#### 11.1.1 IT0 的结论（模拟器，iOS 26.0）

| 项 | 结论 |
| --- | --- |
| IT0-1 | `load(webview:)` 时主 WebView 已在视图树里，移进 `mainHost` 没有问题；`hitTest` 按区域放行、弹层不放行，都对。公开接口设透明够用。切主题、冷启动不闪白（只有系统启动屏跟着系统明暗）；进出后台正常；手持横放时主界面仍是竖屏。VoiceOver 模拟器上验不了，只做了 `accessibilityElementsHidden` 跟着弹层、双指 Z 手势等于返回 |
| IT0-2 | `forIdentifier` 可用。`HTTPCookie.cookies(withResponseHeaderFields:)` 写入的 `HttpOnly`、`Secure`、`SameSite=Lax`、到期时间读回来都在。一次性回调登录成功；杀进程重开 Cookie 还在。登出清空（Cookie、localStorage、IndexedDB）；数据区目录下次启动时删，期间又登录就不删 |
| IT0-3 | `WithReply` 可用。处理器对每个框架都可见，`about:blank`、`srcdoc`、同源、`data:`、沙箱框架直接调它都被 Rust 以 `not_allowed` 拒绝；脚本没有注入到它们里面。顶层 `about:blank` 也没有桥；顶层 `data:` WebKit 自己就不让去；顶层 `blob:` 与页面同源，等同于页面。激活页的 iframe 已删，它能不能碰到 IPC 不再有意义 |
| IT0-4 | `isHidden` **不够**：WebKit 仍告诉页面 `visible`。改成隐藏时移出窗口，页面收到 `hidden`，`inactiveSchedulingPolicy = .throttle` 下 100 ms 的定时器约 1.4 次/秒，播放继续。退后台 75 秒、5 分 26 秒回来页面都在（15 分钟没测），约 30 秒后进程被挂起。六个页面时每个网页进程约 200 MB（debug、模拟器，只能作参考）。杀网页进程：前台的自动重来，一分钟内第二次显示错误 |
| IT0-5 | `UIBackgroundModes: audio` 加 `AVAudioSession(.playback)`：应用的 `<audio>` 切后台 45 秒按 1:1 走；Home 音乐锁屏 95 秒不断并自动换曲。锁屏控件显示 `MPNowPlayingInfoCenter` 的标题，暂停、继续、下一首都到页面（模拟器不画按钮图标，按位置点有效）。`zonemedia` 放 MP3、拖动、下一首正常 |
| IT0-6 | 扫描能找到设备，`http://<局域网 IP>:端口` 的激活页能打开。配置只有 `NSAllowsLocalNetworking`；模拟器上去掉它也能打开，是否必要留给 IV4 |
| IT0-7 | 把 WKWebView 自己对键盘的监听移除（`NotificationCenter.removeObserver`，公开接口），主 WebView 不再上移也不缩放，内容视图的底边收到键盘上沿。两边打字都不错位。每种 Header 下内容视图都不进安全区，页面的 `env(safe-area-inset-*)` 是 0。左边缘手势的快慢判定与系统的返回手势一致 |
| IT0-8 | Swift Package 加 `register_ios_plugin` 可用，`build.rs` 用 `tauri_utils::build::link_apple_library`（要把它写成 macOS 上的 build-dependency，版本就是锁文件里已有的那个）。C FFI 可用。模拟器 debug、release 都编过并跑过；`aarch64-apple-ios` 的 `cargo check` 和 Swift release 编译通过，没有签名安装 |

原生侧用 Swift。

#### 11.1.2 落点

| 范围 | 文件（BuckyOSApp） |
| --- | --- |
| Rust 公共层 | `src-tauri/src/content/mobile/views.rs`；`audio/mobile.rs` 两端共用；`json_file.rs` |
| iOS 的 Rust | `content/mobile/ios.rs`（插件注册、三个 C 函数） |
| iOS 的 Swift | `src-tauri/ios/ClientPlugin/`：`ClientPlugin`（方法表）、`Client`（布局、键盘、生命周期）、`Container`、`ContentView`、`Profiles`、`BackGesture`、`Dialogs`（签名、麦克风、页面对话框）、`Downloads`、`Playback`、`Recorder` |
| 工程 | `build.rs`、`Cargo.toml`、`Info.ios.plist`、`gen/apple/`、`package.json` 的 `build:ios-sim`、README 的 iOS 一节 |
| 前端 | `mobileViews.ts`（返回）、`platform.ts` 与 `theme.ts`（改名、iOS 的来源）、`openFailure.ts`（失败原因）、`ActivationContext.tsx`、`PhoneActivation.tsx`、各 CSS 的 `:hover` |
| Android 的改动 | `MainActivity.kt` 的事件名和主题（X12）、`Dialogs.kt` / `ContentView.kt` / `Labels.kt` 的麦克风文案；`ContentView.kt` 的页面对话框（X16）和新窗口探针（X17） |
| 用例 | `test/mobile/`（iOS 经 Web Inspector 协议，Android 经 Chrome DevTools 协议，只用 Python 标准库） |

#### 11.1.3 验证

环境：从 iPhone 17（26.0）克隆的模拟器（带着已信任的 DV CA、临时钱包、已登录 `test.buckyos.io`），原来那台没动。

| 内容 | 结果 |
| --- | --- |
| 打开 | 图标、Tab、Activity、深链（含冷启动）、另一个应用（文件 → 文本编辑器的 `content.open`、`target=_blank`、`window.open`）五个入口；Message Hub、HomeStation、AI 工作区、文件、文本编辑器，约 1 秒，不出现登录页 |
| 切换、淘汰 | 来回切不重新加载；第 6 个起淘汰最久没用的，回来重新加载；Tab 与图标是同一个视图 |
| 会话 | 删掉 Cookie 后自动重登；登出清空、换新会话；重启登录保留、视图不保留 |
| 返回 | `frame.back` → 应用历史 → 最小化 → 根部不动；拖回去取消；退不回登录往返 |
| 崩溃 | 一个应用忙等时本地界面 0.0 秒响应；杀进程自动恢复一次，第二次显示错误，重试可用 |
| 安全（IC9） | `test/mobile/container.py` 九组 53 条全过（切换、历史、会话、崩溃、新窗口、下载、安全、泄漏、激活），安全一组就是重写的攻击用例 |
| 签名 | 内容视图和激活页的原生对话框；错密码留在框里；第二个请求 `busy`；视图关闭按拒绝；成功时 `pwd_hash` 为 null |
| 网页能力 | 下载 `blob:`、`data:`、Zone 的 https，外站不下载，重名加编号；选相册图片；麦克风先客户端后系统；摄像头拒绝；全屏视频横屏，退出恢复竖屏 |
| 音频 | 播放见 IT0-5；录音开始、暂停、继续、停止、列出、取回（ADTS AAC）、删除、随视图结束并保留 |
| 键盘、主题 | 见 IT0-7；浅色、深色、跟随系统，状态栏和键盘跟着变 |
| 证书 | 没信任 CA 的模拟器（iOS 18.4）：登录被拒并写明证书不受信任，页面打不开，没有放行入口 |
| iOS 18.4 | 能启动、显示本地界面 |
| 单测 | Rust 98、前端 179；Android 目标 `cargo check` 无警告 |

#### 11.1.4 与正文不同

- §3.1：隐藏的视图移出窗口，不用 `isHidden`（IT0-4）。
- §3.2：插件方法多一个 `share`（下载完成后的“分享”，命令 `native_share`）。
- §3.4：`target=_blank` 的链接在 `decidePolicyFor` 就判断并取消，`window.open` 才到 `createWebViewWith`。后退用 `go(to: backItem)`：`goBack` 会跳过没有用户手势的条目，可能越过历史下限。
- §3.5、§3.8：定位不能“不加说明就算了”：系统不答复的请求会一直拽住 WKWebView，视图关了页面还活着、也没有导航判断。现在每个框架里直接拒绝定位；视图关闭时另外让页面立刻结束。
- §3.6：返回不是事件，是 `window.__buckyBack` 的 `available()` 和 `go()`，原生在手势开始时问有没有可返回的，没有就不显示提示。
- §3.6：主题传用户的选择（含“跟随系统”）而不是算出的明暗，否则系统切换时不再通知；存一份，冷启动就用对的明暗。
- §3.8：摄像头的用途说明、ATS、后台音频、文件共享、URL scheme 放在 `Info.ios.plist`（`Info.plist` 也是 macOS 的）。URL scheme 原来靠 deep-link 插件编译时写，走缓存的构建里没有，深链是坏的。
- §3.8：录音器在自己的串行队列上：系统启动录音可能要几秒（蓝牙输入），在主线程上会卡住整个客户端。
- 不在前台的视图弹的 `alert`、`confirm`、`prompt`、麦克风询问一律按拒绝。
- CSP 的 `frame-src` 收紧成 `'self'`：本地页面里已经没有外部的 iframe。
- IC0 的构建：clean 构建时 deep-link 插件会改写 entitlements，Xcode 偶尔报“modified during the build”，做一次 `xcodebuild clean`。

#### 11.1.5 没验证的

- Android：只有 debug 包、arm64、Android 16 的模拟器；release 包（R8）没有编，真机没有跑。主 TODO §11.3 的原脚本在另一台机器上，没有跑，范围由 `test/mobile/` 和 §11.1.7 的手工项覆盖。
- 阶段 4 的全部真机项；VoiceOver；iOS 17 运行时。
- 两个 Zone 互不串数据（DV 只有一个 Zone）；切 Zone。
- 真实的断网和恢复（`NWPathMonitor` 的 `online`）：只验了连不上时写明原因并按 3 秒重试。
- 真的未激活设备走完激活并进入新 Zone（SN 的虚拟机没开）；激活页签名成功时的 `pwd_hash`。
- 下载重定向到 Zone 之外；录音被来电打断；锁屏时的“上一首”。
- 桌面端只跑了单测和前端构建，没有重新打包运行（本机正开着上午那个包）。

#### 11.1.6 等 owner 的

1. X3 的返回规则是我定的：设置子页回到来的地方，没有来处回上一级。
2. `feature/ios-container` 的合入。

#### 11.1.7 Android 回归

环境：这台 Mac，JDK 17、SDK 36、NDK 27.0.12077973、Gradle 8.14.3；新建的 Android 16（API 36，google_apis，arm64）模拟器，WebView 133，DV CA 只装进这台模拟器，域名解析到 127.0.0.1 后用 `adb reverse tcp:443 tcp:443`。同一台模拟器上先后装 main（`3a691a7`）和分支的 debug 包对照。

| 内容 | 结果 |
| --- | --- |
| 编译 | `pnpm tauri android build --debug --apk --target aarch64` 通过，Kotlin 无错误 |
| `test/mobile/container.py` | 分支 51 条全过，连跑两次，X14 后加到 53 条，两端各跑一次全过；main 48 条，不过的 3 条是 X7、X16、X17。X18 有时序，main 上两次里失败一次 |
| iOS 复跑 | 公共层改过之后重编，51 条全过 |
| 麦克风 | 询问写明应用和 Zone（X9）；允许后系统再问；录音开始、暂停、继续、停止、列出、取回、删除 |
| 播放 | 前台服务和媒体控件出现，暂停命令到页面，切后台 1:1 继续，释放后服务结束 |
| 签名 | 对话框写明应用、Zone、身份，看不见的字符写成 `\u202e`（X10）；取消返回 `cancelled`；内容视图、Tab、激活页、切后台回来都能弹，隐藏和最小化的按取消返回（X14）。没有在 Android 上输密码签成功 |
| 激活页 | 设备不响应时显示原因和“重试”，重试打开，返回回到扫描页且视图关闭（X11） |
| 返回 | 应用历史 → 最小化 → 退到后台；Zone 列表回到来的地方（X3） |
| 键盘 | 内容视图和本地页都不被挡；“添加 Zone”末尾的按钮滚得出来（X13） |
| 触摸 | `(hover: hover)` 为假，`:hover` 不生效（X4） |
| 下载 | 存到系统的下载目录，提示里没有“分享”（只有 iOS 有） |
| 主题 | X12 修后 15 项全过，跑两次（跟随系统、选浅色、选深色，运行中、后台、冷启动；本地页、已打开的应用、状态栏）；main 上 9 项不过。全屏旋转后系统栏仍隐藏、返回能退出。iOS 跟随系统和选深色复验正常 |

模拟器上看到、没有定位的：有应用在后台时点 Home 的搜索框，状态栏和账号一行背后留着上一帧的图标。main 上一样。

### 11.2 跨平台缺陷

IQ1 的目的之一。发现一条记一条：在哪、影响哪些平台、怎么修、修了没有。X1–X6 是动手之前记的，X7 起是实施中发现的；来自 iOS 模拟器测试和读代码；X16 起是 Android 回归中发现的。Android 上核对过的见 §11.1.7。

| 编号 | 现象 | 可能影响 | 位置 | 状态 |
| --- | --- | --- | --- | --- |
| X1 | 能力清单按平台写死，没有跟着实现走：iOS 没有系统通知，却列了 `notify.system` | 应用按清单用功能，会用到不存在的能力 | `content/bridge.rs` 的 `capabilities()` | 已修：三个平台各列各的，iOS 去掉 `notify.system` |
| X2 | 打开失败只有一句“没有打开成功”，`Unsupported` 和真失败分不开 | 手机端都一样 | `open.failure.failed`、`PhoneTabPage.tsx` | 已修：`openFailure.ts` 按错误码给原因，其它带错误文字 |
| X3 | 从 Home 的账号菜单进 Zone 列表，返回去了设置页 | 手机端都一样，系统返回键也走这里 | `container/back.ts` 的 `backTarget` | 已修：设置子页先回来的地方（规则是我定的） |
| X4 | 触摸之后 `:hover` 样式粘住，只有 Activity 用了 `@media (hover: hover)` | 有触摸的平台 | 各 CSS | 已修：全部放进 `@media (hover: hover)`，按下的反馈用 `:active` |
| X5 | 键盘和安全区的公共机制以 Android 命名（`--android-*`、`android-window-insets-change`），iOS 没有对应来源 | iOS 接入时要么改名，要么另写一套 | `App.css`、`platform.ts`、`mobileViews.ts` | 已修：事件改成 `window-insets-change`（Kotlin 同步改）；`--android-*` 只有 Android 写，名字不动 |
| X6 | 登录凭据丢失时显示“你已退出”，和用户主动退出分不开 | 凭据读不到的平台 | `zone/session.rs`、`SessionNotice` | 已修：不是用户自己退出的，显示“需要密码” |
| X7 | 手机的内容视图放行 `tauri:` 和 `*.localhost` 的顶层导航，页面能把自己弄成“连不上” | Android、iOS | `content/policy.rs` | 已修：只有桌面放行，手机上拦下（Android 回归时改：原来会去开浏览器） |
| X8 | 视图连不上时切走，再切回来不会自己重试 | Android、iOS | `views.rs` 的 `set_front` | 已修 |
| X9 | 麦克风询问里的 `{zone}` 没有填，原样显示 | Android、iOS | `Dialogs.kt`、`Dialogs.swift`、文案 | 已修，两端核对过 |
| X10 | 手机的签名对话框不转义看不见和改变顺序的字符，桌面有 | Android、iOS | `views.rs`、`bridge.rs` 的 `visible` | 已修（应用名、Zone 名、身份也转义） |
| X11 | 激活页打不开时没有提示：iOS 空白，Android 是 WebView 自带的错误页 | Android、iOS | `views.rs`、`PhoneActivation.tsx` | 已修：显示原因和重试 |
| X12 | “跟随系统”时 `BuckySystemBars.setTheme` 传的是算出的明暗，Android 会把它定死，之后系统切换明暗可能不再跟 | Android（实测成立） | `theme.ts`、`MainActivity.kt` | 已修（owner 定）：传用户的选择，“跟随系统”用 `MODE_NIGHT_FOLLOW_SYSTEM`；另外在客户端里选主题时，已打开的页面原来不会跟着变，现在把变化告诉各 WebView |
| X13 | 页面末尾的输入框和按钮被键盘挡住、滚不出来（“添加 Zone”） | Android、iOS | `shell.css` | 已修：`.shell-page-body` 按键盘留白 |
| X14 | 不在前台的视图也能弹签名对话框 | Android、iOS | `views.rs` 的 `sign_json` | 已修（owner 定）：请求时不在前台，直接按取消返回；框已经弹出之后再切走的不收回 |
| X15 | 录音正在启动时进程被杀，留下 0 字节的文件和描述，不列出也不清理 | Android、iOS | `audio/recordings.rs` | 未改，影响只是磁盘上两个小文件 |
| X16 | 不在前台的页面能弹 `alert`、`confirm`、`prompt`，盖在别的应用上 | Android（iOS 一开始就拒绝） | `ContentView.kt` | 已修：隐藏的视图按拒绝 |
| X17 | 接新窗口地址用的 WebView 不销毁：它不在窗口里，`View.post` 不会执行，要等垃圾回收 | Android | `ContentView.kt` 的 `onCreateWindow` | 已修：用主线程的 `Handler` |
| X18 | 链接指向一个页面已被回收的应用，有时打开的是它原来的页面：置前和打开各自重建了一次视图，后完成的用旧地址 | Android、iOS（只在 Android 上复现） | `views.rs` 的 `revive` | 已修：只建一次，用链接的地址 |
| X19 | 不在前台的页面能弹客户端的麦克风询问 | Android（iOS 一开始就拒绝；实测成立） | `ContentView.kt` 的 `askMicrophone` | 不改（owner 定，2026-11-24：先不改）；要改时和 X16 一样按隐藏拒绝 |
