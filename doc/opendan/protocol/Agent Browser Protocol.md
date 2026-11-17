# Agent Browser Protocol

Agent Service（opendan）的浏览器模块对外的接口：kRPC `browser.*`、屏幕 WebSocket、本机文件上传、`agent-browser` 的入口、重排库的规则。设计依据见 [Agent 浏览器实施 TODO](<../../../notepads/agent-runtime-browser-todo.md>) §5。

消费方：Desktop 的「Agent 浏览器」App（`src/frame/desktop/src/app/agent-browser/`）、`agent-browser` 命令（`publish/exttool/ext-core/browser/`）、`reflow` 命令（`src/tools/buckyos-agent/commands/reflow.ts`）、nfs-server 的 `reflow.<zone>` 主机。

## 1. 名词

| 名词 | 含义 |
| --- | --- |
| 组（group） | 浏览器的标签页。每个 Agent 只有一个组，`group_id` 固定为 `default`：Agent 的每个会话操作的都是它，用户在「Agent 浏览器」里看的、操作的也是它。字段和参数里的 `group_id` 保留，取值只会是 `default` |
| 浏览器会话 | owner 和这个 Agent 的一个 MessageHub 会话：用户在「Agent 浏览器」里和 Agent 说话的地方，和浏览器有关的话都在这里说（交接、重排的请求）。用 `browser.bind_default` 绑定，它的 id 是 `default_session_id`。写给会话的记录（§2.4）投给它，它的 UI Session 带 `browser` 手册 |
| 标签页（tab） | `tab_id` 是 CDP 的 target id |
| 操作方（controller） | `user` 或 `agent`。Agent 持有有效租约、用户又没有接手时是 `agent`，其他时候是 `user` |
| 租约（lease） | Agent 操作浏览器的凭据，默认 60 秒，靠 `browser.renew` 续。可以同时有几个租约（一个 UI Session 和它的 Work 会话、或几个会话各跑一条命令），只要有一个有效，操作方就是 `agent` |
| 接手（user_hold） | 用户明确接手，或 Agent 请用户接手之后，直到用户交还。期间 `browser.acquire` 失败 |

服务端把会话换成组：哪个 `session_id` 都是 `default`（登记表里没有的也是）。

浏览器会话怎么来：MessageHub 里和 Agent 的会话都是具名会话（id 是 msg-center 生成的），没有稳定的“默认会话”，所以由 App 建一个、绑定上。App 第一次要用到它时（打开对话面板、交给 Agent、交还后发消息、请 Agent 重排）在这个 Agent 名下带来源标记（owner 自己的会话 UI 状态里键 `ui.source`，值 `agent-browser`）的会话里找：已绑定的那个还在、也带标记就用它，否则取带标记的里最近活跃的，都没有就新建一个（标题“浏览器”，带标记；有会话的标记读不出来时不新建，报失败让用户重试）；`default_session_id` 不是它时调 `browser.bind_default`。没绑定之前浏览照常，只是没有会话收记录。

浏览器会话的 UI Session：服务端建 UI Session 时，它对应的 MessageHub 会话（`channels.outbound.to_session`）就是 `default_session_id` 的话，手册在模板的默认集合之外加上 `browser`（记在会话配置的 `extensions.opendan.cmd_skills`，提示词重新冻结时也带上）；Agent 没有这本手册、手册关着、或运行时达不到它的基线时，照常在冻结时去掉。别的会话不加。绑定改到一个已经有 UI Session、而它的手册里没有 `browser` 的会话时，服务端把那个 UI Session 归档（同 `/new`），这个会话的下一条消息开出的 UI Session 就有了；它正在跑一个 Turn 或有没结束的 Work 时不归档，等它以后开新的 UI Session。

反过来，浏览器会话的 UI Session 是：回复去向是 owner、`to_session` 等于 `default_session_id`、还没结束的 UI Session 里最新的一个。UI Session 在第一条消息到达时才创建，所以绑定了也可以还没有。

`group_id` 以 `_` 开头的组留给服务端自己用（重排刷新时开的后台标签页这一类）：不出现在 `browser.status` 里，不落盘，不计入 `max_tabs`，`take_over`、`hand_back`、`hand_to_agent` 引用它回 `no_such_group`。

新出现的标签页归哪个组，按顺序：有 opener 的归 opener 的组；服务端正在为某个组开标签页时归那个组；其他都归 `default`。

每个标签页单独一个窗口（`Target.createTarget {newWindow: true}`）：同一个窗口里排在后面的标签页不渲染，截屏流和 Playwright 的点击、截图都会停住。`agent-browser` 自己开标签页也要用 `newWindow: true`。无头浏览器（§6 的 `display`）的标签页服务端另开着焦点模拟（`Emulation.setFocusEmulationEnabled`）；有显示器的浏览器（`virtual`）不开，焦点就是显示器上的焦点。

浏览器有两种跑法（§6 `[loader.browser] display`）：

- `virtual`（Linux 的默认）：有窗口地跑在这个 Agent 自己的虚拟显示器上（TigerVNC 的 `Xvnc`），和桌面上的浏览器一样：身份（UA、UA-CH）、权限提示、下拉框、JS 弹框、账号选择这些都是浏览器自己的，画在显示器上。屏幕看到的是整块显示器，用户的操作是显示器上的鼠标和键盘（§3.6）。服务端不改浏览器的身份，不往页面里注入改变页面行为的脚本，不替用户拒绝权限。
- `headless`（其他平台的默认，或明确配置）：无头，屏幕是当前标签页的截屏流，输入走 DevTools；服务端把标签页模拟成看它的设备（手机的 UA 和触屏）、给它换掉 `HeadlessChrome` 的 UA、拦下拉框、拒绝全部权限（§3）。

## 2. kRPC

挂在 `/kapi/<agent_id>`，和 Agent Service 的其他方法一样。

### 2.1 共用的结构

```jsonc
// Tab
{ "tab_id": "E3F1…", "url": "https://example.com/", "title": "Example", "loading": false, "active": true }

// Group
{
  "group_id": "default",
  "controller": "user",            // user | agent
  "user_hold": false,
  "agent_state": "idle",           // idle | working | waiting_answer | waiting_takeover | done | stuck
  "note": null,                    // request_handover 带的说明；没有接手请求时是最近一次 release 带的
  "handover": null,                // { "reason": "password|captcha|other", "note": "...", "at_ms": 0 }
  "lease": null,                   // { "session_id": "...", "expires_at_ms": 0 }；有几个租约时是最晚到期的那个
  "tabs": [ /* Tab */ ]            // 浏览器没在运行时为空：存下来的标签页在下次启动时重开，那时才有 tab_id
}

// Status
{
  "enabled": true,                 // [loader.browser].enabled
  "available": true,               // 这个节点上有浏览器可以启动：ext-core 装了，浏览器也下载好了。不看 enabled
  "running": false,
  "unavailable": null,             // 不可用时的一句说明（英文，给人和 Agent 看）；可用但最近一次启动失败时是失败的原因
  "unavailable_code": null,        // 不可用的原因：disabled | no_package | not_downloaded | no_display；其他时候为 null
  "display": "virtual",            // 浏览器怎么跑：virtual（有窗口，在虚拟显示器上）| headless（§1、§6）
  "device_scale_factor": 2,        // virtual 时显示器的设备像素比（§6）；headless 时为 null
  "version": { "browser": "159.0.8154.6", "playwright": "1.65.1" },   // disabled、no_package 时为 null
  "default_session_id": null,      // 浏览器会话（browser.bind_default 绑定的 MessageHub 会话），没绑定时为 null
  "groups": [ /* Group */ ]        // 总是只有一个：default
}
```

`unavailable_code` 是稳定的取值，调用方按它判断；`unavailable` 的措辞可以变。按这个顺序判定，取第一个成立的：

| `unavailable_code` | 条件 |
| --- | --- |
| `disabled` | `[loader.browser] enabled = false` |
| `no_package` | 工具目录里没有 `chromium` 启动器：这个节点没装 ext-core |
| `not_downloaded` | 启动器在，浏览器本体没到 `ready`。浏览器是 ext-core 的在线项 `chromium`：不随包分发，包装好以后在节点上下载（§6）。还没下载、正在下载、下载失败都是这个值，`unavailable` 按记录文件区分 |
| `no_display` | `display` 是 `virtual`，运行环境里找不到 `Xvnc`（`PATH` 上，或 `OPENDAN_XVNC` 指的文件）：Agent 的容器镜像比浏览器要求的旧 |
| `null` | 上面都不成立。这时 `unavailable` 也是 `null`，除非浏览器在但最近一次启动失败、现在没在运行：那是失败的原因 |

现在的 `unavailable`：

```text
disabled        the browser is switched off for this agent ([loader.browser] enabled = false)
no_package      no `chromium` on this node (it comes with the ext-core extension tools)
no_display      this runtime has no virtual display for the browser (`Xvnc` of TigerVNC is not on PATH; it comes with the Agent's container image)
not_downloaded  the browser is not downloaded on this node yet (the online item `chromium` of ext-core): download it in Settings › Extension tools, or with `buckyos ext-tools download ext-core chromium`
                the browser is being downloaded to this node (42% of 171 MiB)
                the browser's download to this node failed: <记录里的原因>. Start it again in Settings › Extension tools, or with `buckyos ext-tools download ext-core chromium`
```

正在下载的百分比是记录里的 `downloaded_bytes / size_bytes`，记录没有大小时不带括号里那一段。`not_downloaded` 还有两种少见的说明：记录是 `ready` 而目录不在（`the browser's download is recorded as ready, but <目录> is not there: download it again in …`），记录读不出来或 `state` 不认识（`the record of the browser's download (<文件>) …`）。

- 每次调用都现查磁盘，不记“不可用”的结论：浏览器下载好（或 ext-core 装好）之后，下一次 `status`、`ensure`、`acquire` 就可用，不用重启 Agent Service。反过来，记录离开 `ready`（管理员重新下载）期间这些方法回 `browser_unavailable`，已经在运行的浏览器不会因此被停掉。
- `available` 不看 `enabled`：模块关着而浏览器在时是 `available: true`、`unavailable_code: "disabled"`。
- 用 `OPENDAN_CHROMIUM` 指定了可执行文件时（调试和测试）不看在线项，指的文件在就是可用；不在按 `no_package` 报，`unavailable` 里写的是这个路径。
- `version.browser`：浏览器在运行时取它自己报的版本，没在运行时取 ext-core 的 `browser/versions.json`；`version.playwright` 只来自 `versions.json`。取不到的那一项是 `null`（用 `OPENDAN_CHROMIUM` 指到别处的浏览器时没有 `versions.json`）。`versions.json` 随包，所以 `not_downloaded` 时 `version` 照旧有值：`version.browser` 是包声明的、将要下载的那个版本。
- `Tab.title` 是读出来的，不是浏览器推过来的（无头模式下标题变了没有事件）：页面加载完、有人在看（每秒一次）、以及 `status`、`acquire`、交接之前各读一次。

`agent_state` 的来源：

| 值 | 条件 |
| --- | --- |
| `waiting_takeover` | Agent 调了 `request_handover`，用户还没交还 |
| `working` | Agent 持有有效租约 |
| `waiting_answer` | 没有租约，浏览器会话里有 Agent 提出、用户还没回答的问题：这个 Agent 发出的、还开着的 `input` 通知（`ask_user`），来源是浏览器会话（它的 UI Session，或从它开出的 Work、Goal）。别的会话里的问题不算：App 让用户到浏览器会话里回答。没绑定浏览器会话时不会是这个值 |
| `done`、`stuck` | Agent 最近一次 `release` 带的 `state`。出现新的租约、接手、交还时清掉 |
| `idle` | 其他 |

`waiting_answer` 不看会话登记表的 `needs_input`：那一项在会话等下一条消息时就是真（每个空闲的对话都是），而在会话等自己提的问题时反而是假。服务端在 `browser.status` 里现查通知，有人在看时每 3 秒查一次。

### 2.2 方法

| 方法 | 参数 | 返回 | 谁能调 |
| --- | --- | --- | --- |
| `browser.status` | `{group_id?, session_id?}` | Status。`group_id` 只能是 `default`，别的回 `no_such_group`；`session_id` 照收，结果一样（每个会话都在 `default`） | owner、Agent |
| `browser.ensure` | `{}` | Status | owner、Agent |
| `browser.acquire` | `{session_id}` | `{lease_id, group_id, ttl_ms, expires_at_ms, cdp_url, tabs, active_tab_id}` | Agent |
| `browser.renew` | `{lease_id}` | `{expires_at_ms}` | Agent |
| `browser.release` | `{lease_id, state?, note?}` | `{}`。`state`：`done`、`stuck` | Agent |
| `browser.request_handover` | `{session_id, reason, note?}` | `{group_id}`（`default`）。`reason`：`password`、`captcha`、`other` | Agent |
| `browser.take_over` | `{group_id}` | Group。`group_id` 只能是 `default`，下两个同 | owner |
| `browser.hand_back` | `{group_id}` | Group | owner |
| `browser.hand_to_agent` | `{group_id}` | Group | owner |
| `browser.bind_default` | `{session_id}` | `{default_session_id}` | owner |
| `browser.upload` | `{request_id, zone_path, done?}` | `{}` | owner |
| `browser.download_export` | `{download_id}` | `{obj_id, name, size}` | owner |
| `browser.clear_profile` | `{}` | `{}` | owner |
| `browser.reflow_state` | `{url}` | 见 §5.3 | owner、Agent |
| `browser.reflow_refresh` | `{url}` | 见 §5.3 | owner、Agent |
| `browser.reflow_action` | `{url, selector}` | 见 §5.3 | owner |

- `status` 不启动浏览器。`ensure` 没在运行就启动。`acquire` 也会启动（等于先 `ensure`）；用户接手着的时候先回 `user_in_control`，不为此启动浏览器。
- `cdp_url` 给 `chromium.connectOverCDP` 用，是这个服务上这个租约的入口：`<OPENDAN_BROWSER_RPC>/browser/cdp/<lease_id>`（§2.5）。Playwright 先读它的 `json/version`，再连上它给的 WebSocket（`…/browser/cdp/<lease_id>/devtools/browser`），服务端把消息原样转给浏览器。`json/list` 原样转发浏览器自己的标签页列表（`agent-browser` 连不上时用它找出因为弹框没人回应而卡住的标签页；列表里的地址是浏览器自己的端口，只用于这种诊断）。只收本机回环地址上的连接；租约无效时回 `lease_lost`。租约一结束（`release`、过期、`take_over`、`request_handover`、浏览器停掉），这个租约的连接立刻被服务端关掉：正在执行的命令马上失败，之后的命令到不了浏览器，用户接手时不用等 Agent 停手。服务端没有 `OPENDAN_BROWSER_RPC` 时（测试）`cdp_url` 是浏览器自己的 `http://127.0.0.1:<端口>`。这只是让交接干净，不是安全边界：同一个运行环境里自己去找浏览器端口的进程拦不住。
- `active_tab_id` 是当前的标签页，没有标签页时为 `null`。`acquire` 不替 Agent 开标签页。
- 再次 `acquire`（同一个会话或别的会话）得到另一个租约，原来的照旧有效：几个会话同时操作的是同一组标签页。`renew`、`release` 用到已经过期或被作废的租约回 `lease_lost`（`release` 带的 `state` 这时不生效）。浏览器停掉或崩溃时，所有租约作废。
- `request_handover` 作废全部租约，并置 `user_hold`。
- `take_over` 作废租约，置 `user_hold`。`hand_back`、`hand_to_agent` 清 `user_hold` 和 `handover`。两者只差写给会话的记录（§2.4）。
- `bind_default` 把浏览器会话绑定到一个 MessageHub 会话，落盘在 `groups.json`。只改 `default_session_id`：标签页、租约、接手都不变。重复绑定同一个会话没有效果。改了绑定时，新绑定的会话已有的、手册里没有 `browser` 的 UI Session 按 §1 归档。`session_id` 是 `default` 或以 `_` 开头时回 `invalid_argument`。
- `groups.json` 只存 `default` 一组。加载时别的组（每个会话一组时的旧文件）的标签页和接手状态都不恢复。
- `clear_profile` 停掉浏览器，删除用户数据目录和缓存目录（下载目录不动）。标签页还在，下次启动时重开，登录状态没有了。它不重新启动浏览器：App 清除之后接着调 `browser.ensure`。
- 没人处理的 JS 弹框由服务端立刻关掉：弹框所在的组没有人在看、操作方也不是 Agent 时，`beforeunload` 按“离开”处理，其他按取消处理。有人在看的留给屏幕（§3.4）；操作方是 Agent 的留给 Agent 的 Playwright，租约结束时还开着的照上面关掉。开着的弹框会卡住它的页面，也会卡住任何一个 `connectOverCDP`，不分组。
- “owner”一列的方法在 `is_owner_only` 里（屏幕的握手和上传按同样的规则校验，方法名是 `browser.screen`、`browser.upload`）。“Agent”一列的方法 owner 也能调；Agent 自己的 App 身份不论对 owner 目录是只读还是读写都调得通。

### 2.3 错误

错误走 Agent Service 现有的线格式：kRPC 的 `error` 里是 JSON 文本 `{"kind", "code", "message"}`。浏览器模块的错误都带一个稳定的 `code` 字段；`kind` 是 Agent Service 通用的分类（`no_such_*` 是 `not_found`，`lease_lost`、`invalid_argument` 同名，其他是 `other`），调用方按 `code` 判断。

| `code` | 含义 |
| --- | --- |
| `browser_unavailable` | 这个 Agent 现在没有浏览器可用（`message` 里有原因）。模块关着、节点没装 ext-core、浏览器还没下载好、运行环境没有虚拟显示器这四种，错误的 JSON 另带 `unavailable_code`（`disabled`、`no_package`、`not_downloaded`、`no_display`，含义和 `message` 同 §2.1 的 Status）：`{"kind": "other", "code": "browser_unavailable", "unavailable_code": "not_downloaded", "message": "…"}`。浏览器起不来、中途断了、Agent Service 正在退出这些没有这个字段 |
| `invalid_argument` | 参数缺失或取值不对，或浏览器拒绝了这条命令（比如打不开的网址） |
| `user_in_control` | 用户接手着，`acquire` 失败 |
| `lease_lost` | 租约过期或被作废 |
| `no_such_group`、`no_such_tab`、`no_such_request`、`no_such_download` | 引用的对象不存在 |
| `too_many_tabs` | 超过 `max_tabs` |
| `upload_too_large` | 上传超过上限 |
| `reflow_no_package` | 这个网址没有匹配的重排包 |
| `other` | 上面都不是的失败。重排库（桶）的读写被拒或失败时是它：`message` 是 `reflow library: <bucket_id>/<key>: <桶的错误码>: <说明>`，桶的错误码照原样（`permission_denied`、`bucket_not_found`、`token_invalid` 等），连不上 nfs-server 时是连接的错误（§5.3） |

### 2.4 写给会话的记录

隐式半订阅 `_browser`（每个会话都有，不用声明，已有的会话也有），来源 `system:browser:default`，只保留最新一条，在下一次输入前呈现，不单独触发推理。记录投给浏览器会话的 UI Session（§1）；没绑定浏览器会话、或它还没有 UI Session 时不写。别的会话不收：在别的会话里操作浏览器的 Agent 从 `agent-browser` 的退出码知道用户接手了。`event` 是 `take_over`、`hand_back`、`hand_to_agent`，`summary` 以 `[browser]` 开头。

| 时机 | 内容 |
| --- | --- |
| `take_over` | 用户已接手浏览器，在交还之前不要操作 |
| `hand_back` | 用户已交还，带当前网址和标题；提示先重新读页面 |
| `hand_to_agent` | 用户请 Agent 从当前页面接着做，带当前网址和标题 |

要叫醒 Agent 时，由 App 往浏览器会话里发一条普通消息。

### 2.5 Agent 侧入口

Agent Service 在自己的进程环境里设 `OPENDAN_BROWSER_RPC`，值是本机回环上的 kRPC 地址（`http://127.0.0.1:<端口>/kapi/<agent_id>`）。会话里的命令继承它，用会话的 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 调用，`session_id` 取 `OPENDAN_SESSION_ID`。没有这个变量的地方（其他节点上的会话）用不了常驻浏览器。

`agent-browser`（`publish/exttool/ext-core/browser/agent-browser.mjs`）这样用这些方法：

- 要操作页面的子命令（`run`、`open`、`snapshot`、`screenshot`）依次调 `browser.ensure`、`browser.acquire`，用 Playwright 的 `connectOverCDP(cdp_url)` 连上（经过服务端，§2.2），做完断开，再调 `browser.release`。用户接手时连接被服务端关掉，命令按“被切断”退出：连接断开（或连不上）而失败时，`agent-browser` 先 `renew` 一次，租约已结束就按 `lease_lost`（退出码 5）退出，不当成脚本出错（ext-core 1.3.2 起）。`tabs` 只取租约、不连接。`done`、`stuck` 取到租约后立刻带 `state` 释放。
- 持有租约期间每隔 `ttl_ms / 3` 调一次 `browser.renew`。返回 `lease_lost` 时断开连接、中止脚本，不再 `release`。
- 错误的 `code` 从 kRPC 的 `error` 文本里的 JSON 取：先取 `code`，没有时取 `kind`。
- 连接带 `noDefaults`：Playwright 不改默认上下文的下载行为、媒体模拟和焦点模拟。
- 连接看得见浏览器里所有的标签页。本组的标签页是 `acquire` 返回的 `tabs`、这次连接里新开的、以及它们弹出的窗口。对别组的标签页只做一件事：在上下文上挂一个弹框监听，让 Playwright 不去自动关闭它们的弹框。
- 本组的标签页在连接期间开着焦点模拟（`Emulation.setFocusEmulationEnabled`，随 CDP 会话结束而撤销），所以排在别的标签页后面时也能操作。`screenshot` 会先把标签页切到它所在窗口的最前面。
- 新标签页用 `Target.createTarget {url: "about:blank", newWindow: true}` 开在单独的窗口里，不挤占别的窗口的前台标签页。
- 连接时任何一个标签页上有没人处理的 JS 弹框，新的 CDP 会话都附着不上去，`connectOverCDP` 会等到超时（30 秒）。超时后命令报错，并列出没有回应的标签页。

## 3. 屏幕

本节按无头浏览器写；有显示器的浏览器（`display: virtual`）哪里不同见 §3.6。

### 3.1 连接

`GET /kapi/<agent_id>/browser/screen` 升级成 WebSocket（`/kapi/opendan`、`/api/opendan` 两个挂载点下也有）。连上后 5 秒内客户端发 `hello`，服务端按 owner 校验令牌（和 kRPC 同一套判定，方法名按 `browser.screen` 算），通过后回 `ready`，否则回 `error` 并关闭。连接看的是这个 Agent 的浏览器，也就是它唯一的组 `default`（§1）。不带 WebSocket 升级头的请求回 400。

- 浏览器没在运行时，`hello` 通过后服务端先把它启动起来（等于 `browser.ensure`），起不来回 `browser_unavailable`（原因是 §2.1 那三种之一时带 `unavailable_code`，§3.3）。
- 连接存在期间算“有人在看浏览器”：浏览器不会因为空闲被停掉，JS 弹框留给屏幕（§3.4）。
- 浏览器崩溃后重启、或者被停掉又启动，标签页的 `tab_id` 全都变了。连接不用重连：服务端先发一条 `running: false` 的 `state`，浏览器回来后发新的 `state`，画面换成新的当前标签页。

文本帧是 JSON，都有 `type`。二进制帧是一张 JPEG，前面总有一条 `frame_meta`。客户端发来的单条消息不超过 1 MiB。

### 3.2 上行

```jsonc
{ "type": "hello", "token": "<会话令牌>", "group_id": "default",
  "viewport": { "width": 412, "height": 915, "mobile": true, "pixel_ratio": 2 } }

{ "type": "ack", "seq": 17 }

{ "type": "input", "kind": "touch", "phase": "start|move|end|cancel",
  "points": [ { "id": 0, "x": 120, "y": 300 } ] }
{ "type": "input", "kind": "mouse", "action": "down|up|move", "x": 10, "y": 20,
  "button": "left|middle|right|none", "click_count": 1, "modifiers": 0 }
{ "type": "input", "kind": "wheel", "x": 10, "y": 20, "delta_x": 0, "delta_y": 120, "modifiers": 0 }
{ "type": "input", "kind": "key", "action": "down|up", "key": "Enter", "code": "Enter",
  "modifiers": 0, "text": "" }
{ "type": "input", "kind": "text", "text": "整段文字" }
{ "type": "input", "kind": "composition", "text": "zhong", "selection_start": 5, "selection_end": 5 }

{ "type": "nav", "action": "url", "url": "https://example.com" }
{ "type": "nav", "action": "back|forward|reload|stop" }

{ "type": "tab", "action": "open", "url": "https://example.com" }   // url 可省，省略时开空白页
{ "type": "tab", "action": "close|activate", "tab_id": "E3F1…" }

{ "type": "viewport", "width": 1280, "height": 800, "mobile": false, "pixel_ratio": 1 }

{ "type": "respond", "request_id": "r12", "result": { /* 见 §3.4 */ } }

{ "type": "quality", "max_fps": 15, "jpeg_quality": 60, "pixel_ratio": 1 }

{ "type": "ping" }
```

- `hello` 的 `token` 在 Zone 之外（`--dev`）可以不带；`group_id` 可以省略，带的话只能是 `default`，别的值回 `bad_hello`；`viewport` 必填。
- 坐标是 CSS 像素，相对当前视口的左上角（页面被缩放时也是屏幕上的位置，和 `frame_meta` 的宽高同一个坐标系）。
- `touch` 的 `points` 是这个事件之后仍然按着的全部触点：`start`、`move` 带全部触点；`end` 带剩下的，最后一指抬起时为空；`cancel` 为空。
- 桌面版视口（`mobile: false`）下 App 把触摸换成鼠标：点一下是 `mouse` 的 move、down、up，拖动是 `wheel`。`wheel` 的 `delta_x`、`delta_y` 和 DOM 的 `WheelEvent` 同号（正数向下、向右滚）。`mouse` 的 `move` 带着按住的键（没有按键时 `button: "none"`）。
- `nav url` 的网址不带 scheme 时服务端补 `https://`。组里没有标签页时，`nav url` 等于 `tab open` 带这个网址。
- `tab open` 新开的标签页先是空白页，套上这条连接的视口之后才去网址，所以第一个请求就带着手机的 UA。它成为这个组的当前标签页。`close`、`activate` 只能指本组的标签页。
- `modifiers` 是位掩码：Alt 1、Ctrl 2、Meta 4、Shift 8。
- `key` 的 `key`、`code` 用 DOM 的 `KeyboardEvent` 的值（软键盘没有 `code` 时留空）。可打印字符在 `down` 的 `text` 里带上；带 `text` 的按键会打出这个字，不带的只是按键（快捷键、方向键、Backspace）。`Enter` 不带 `text` 时服务端补上回车。
- `text` 是整段上屏的文字（输入法提交的、粘贴的）。`composition` 是输入法组字的中间状态；`text` 为空表示取消组字。组字结束时上屏的文字发 `kind: "text"`，它替换掉正在组的字。
- `viewport.mobile` 为真时服务端按手机模拟：触屏（5 个触点）、设备像素比 2、UA 是 `Mozilla/5.0 (Linux; Android 10; K) … Chrome/<主版本>.0.0.0 Mobile Safari/537.36`；为假是这个尺寸的桌面窗口，UA 是浏览器自己的。尺寸取值会被夹到 320–2560 × 320–2560。
- 一个标签页同时只有一个生效的视口：正在看它的连接里，最近设置视口的那一条的。`hello` 和每一条 `viewport` 消息都算设置一次，内容和上次一样也算。别的连接看到的是这个视口下的画面，各条连接的 `state.viewport` 报的都是它；和自己发的不一样，说明画面不是按自己的尺寸排的，想换回来就再发一次 `viewport`。那条连接断开、或者不再看这个标签页之后，标签页换成剩下的连接里最近设置的那个视口，页面按它重新排版；没有连接在看了，标签页保持最后的设置。因为别的连接来了或走了而换视口时，页面不重新加载。帧的 `pixel_ratio` 和画质（`jpeg_quality`）算视口的一部分：标签页按生效视口那条连接的值出帧，`state.viewport` 带着生效的 `pixel_ratio`。`quality` 消息改的是这条连接自己的值，不算设置视口：这条连接的视口正在生效时马上用上，否则等它的视口生效时（别的连接走了，或者它再发 `viewport`）才用上。
- `pixel_ratio`（`hello.viewport`、`viewport`、`quality` 里都可以带）是客户端要的帧的像素密度：一个 CSS 像素对应帧里几个像素，取 1–2（超出的夹进来，保留两位小数）。`hello` 不带是 1；`viewport`、`quality` 不带时沿用这条连接上一次的值。它只改帧，不改页面：布局、`innerWidth`、`devicePixelRatio`（手机版仍是模拟的 2，桌面版仍是 1）、输入坐标都不变。做法：浏览器有两块屏（§6 的 `--screen-info`），第二块的像素密度是 2。生效值大于 1 时服务端把标签页的窗口挪到第二块屏，再重套一次视口（挪窗口会冲掉设备模拟），截屏流就按 2 倍出帧；在 1 和 2 之间时截屏流再用 `maxWidth`、`maxHeight` 缩到这个倍数；回到 1 时挪回第一块屏。密度变了，截屏流重开一次（会出一帧）。代价见 §3.5。副作用：密度大于 1 的期间（没有连接在看之后也保持），Agent 的 Playwright 截图也是这个密度的像素（`page.screenshot()` 的宽高翻倍，手机版和桌面版都是）；Agent 操作时设备模拟会失效（见下面 Playwright 那条），这期间页面的 `devicePixelRatio` 是这块屏的 2；页面在挪窗口时收到一次尺寸不变的 `resize`；`screen.isExtended` 恒为真。
- `viewport` 消息里的 `mobile` 和这条连接上一次的不一样（切到桌面版或切回来），当前标签页会重新加载一次，因为网站按 UA 决定下发什么。只改尺寸不重新加载。
- Agent 的 Playwright 截一张图，页面上的设备模拟就失效了（Chromium 按截图那条调试会话的状态恢复）：设备像素比、`screen` 的尺寸回到没模拟的样子，没有 viewport meta 的页面连布局宽度和帧的尺寸都变。所以这个组的操作方从 `agent` 回到 `user` 时（租约被释放、过期、被接手作废），服务端给正在看的标签页把生效的视口重套一次（连同窗口所在的屏），不重新加载页面；连接开始显示一个标签页时也重套。Agent 操作期间不重套（会和它正在截的图打架），这段时间画面可能不是手机的样子。
- `quality`：`max_fps` 取 1–30，默认 15；`jpeg_quality` 取 10–95，默认 60；`pixel_ratio` 见上。三个字段都可以省略，和这条连接上一次的值一样时什么都不做。`max_fps` 只管发给这条连接的帧；`jpeg_quality`、`pixel_ratio` 是标签页的，按上面的规则取生效视口那条连接的。客户端怎么按网速调它们见 §3.5。
- 操作方是 `agent` 时，`input`、`nav`、`tab` 被丢弃，服务端回一条 `error`（`code: "agent_in_control"`），每秒最多一条。`ack`、`viewport`、`quality`、`respond`、`ping` 照常处理。

### 3.3 下行

```jsonc
{ "type": "ready", "state": { /* 同 state 消息去掉 type */ } }

{ "type": "frame_meta", "seq": 17, "tab_id": "E3F1…", "width": 412, "height": 915 }
// 紧跟一个二进制帧

{ "type": "state",
  "group": { /* Group，§2.1 */ },
  "active_tab_id": "E3F1…",
  "url": "https://example.com/", "title": "Example", "loading": false,
  "can_go_back": true, "can_go_forward": false,
  "viewport": { "width": 412, "height": 915, "mobile": true, "pixel_ratio": 2 },   // 当前标签页生效的视口，不一定是这条连接发的
  "running": true }

{ "type": "request", "request_id": "r12", "kind": "select|dialog|file_chooser|download|permission",
  "tab_id": "E3F1…", /* 各 kind 的字段见 §3.4 */ }
{ "type": "request_closed", "request_id": "r12" }    // 请求已经失效（页面走了、别处处理了）

{ "type": "focus", "tab_id": "E3F1…", "editable": true, "input_type": "text",
  "rect": { "x": 20, "y": 300, "width": 300, "height": 40 } }

{ "type": "error", "code": "agent_in_control", "message": "…" }

{ "type": "pong" }
```

- 状态一变就发 `state`，内容是全量；没变不发。会让它变的有：组的状态（操作方、接手、Agent 状态、租约、标签页列表）、当前标签页、它的网址、标题、是否在加载、能不能前进后退、当前标签页生效的视口（§3.2；组里没有标签页时是这条连接自己的）、浏览器是否在运行。组里没有标签页（或浏览器没在运行）时 `active_tab_id` 是 `null`，`url`、`title` 是空串。
- `frame_meta` 的 `width`、`height` 是 CSS 像素，和输入坐标同一个坐标系；JPEG 自己的像素尺寸是它们乘以生效的 `pixel_ratio`（§3.2；1 倍时两者相等，1 和 2 之间时可能差 1 个像素）。客户端按 `width`、`height` 显示，坐标也按它们换算，不看 JPEG 的像素。`seq` 每条连接从 1 递增。
- `focus`：当前标签页里获得焦点的元素变了。`editable` 为真表示它能输入文字（`input`、`textarea`、`contenteditable`），这时带 `input_type`（`input` 的 `type`，另有 `textarea`、`contenteditable`）和它在视口里的位置 `rect`（整数）；焦点离开可编辑元素时发 `editable: false`。App 据此弹出和收起软键盘。切换标签页、页面跳转后服务端重发一次。算作能输入文字的 `input` 类型：`text`、`search`、`url`、`tel`、`email`、`password`、`number`、`date`、`time`、`datetime-local`、`month`、`week`（日期时间这几种第一期当文本框用）；只读、禁用的不算。
- 重连之后，还没处理完的 `select`、`dialog`、`file_chooser` 请求由服务端在 `ready` 之后重发；`download`、`permission` 不重发。
- 握手阶段的 `error.code`：`hello_timeout`、`bad_hello`、`unauthorized`、`browser_unavailable`，发完就关闭连接。
- `code` 是 `browser_unavailable` 的 `error`，握手阶段的和握手之后的一样，原因是模块关着、节点没装 ext-core、浏览器还没下载好时另带 `unavailable_code`（§2.1、§2.3）：`{ "type": "error", "code": "browser_unavailable", "unavailable_code": "not_downloaded", "message": "…" }`。别的原因没有这个字段。
- 握手之后的 `error.code`，连接不关闭：`agent_in_control`；`bad_message`（不是 JSON、不认识的 `type`、字段不对）；以及做不成的 `nav`、`tab`、`respond` 带的 §2.3 的错误码（`no_such_tab`、`too_many_tabs`、`invalid_argument`、`browser_unavailable`、`no_such_request`）。
- 客户端 30 秒没有任何上行帧，服务端关闭连接。客户端空闲时每 10 秒发 `ping`，服务端回 `pong`。服务端的一条消息 10 秒发不出去（客户端不收）也关闭连接。

### 3.4 页面之外的请求

| `kind` | `request` 里的字段 | `respond.result` |
| --- | --- | --- |
| `select` | `multiple`、`options: [{index, label, selected, disabled, group?}]`、`rect: {x, y, width, height}` | `{selected: [index…]}`，取消时 `{cancel: true}` |
| `dialog` | `dialog_type: alert\|confirm\|prompt\|beforeunload`、`message`、`default_prompt?` | `{accept: bool, prompt_text?}`。60 秒没有回应时服务端按 §2.2 的规则关掉它并发 `request_closed` |
| `file_chooser` | `multiple`、`accept: [".png", "image/*"…]` | 选好后调 `browser.upload` 或 PUT 上传（§4），不用 `respond`；取消时 `respond {cancel: true}` |
| `download` | `download_id`、`name`、`size`、`state: completed\|failed` | 不用回应。要存到 Zone 时调 `browser.download_export` |
| `permission` | `permission`（如 `geolocation`）、`origin` | 不用回应：服务端已经拒绝，这条只是告知 |

请求发给正在看这个标签页所在组的每一条连接。`select`、`dialog`、`file_chooser` 是等回应的：有一条连接回应了（或请求失效了），所有收到过它的连接都会收到 `request_closed`，回应的那一条也收到。对已经结束的请求再 `respond` 得到 `error`（`no_such_request`）。一个标签页同一种请求同时只有一个，新的来了旧的就结束。标签页关了、浏览器停了，它的请求全部结束；页面跳到别的文档，它的 `select` 和 `file_chooser` 结束。`request_id` 在 Agent Service 重启之后不延续。

`select`：

- 服务端给每个文档注入一段脚本（在单独的隔离环境里，页面自己的脚本看不到它），拦下对下拉框的点按和打开它的按键（空格、回车、F4、Alt+上下）。只拦下拉框（不是 `multiple`、`size` 不大于 1）：列表框本来就画在页面里。所以现在 `multiple` 总是 `false`。
- 只有组里有人在看、操作方是用户时才发；Agent 操作时页面里的下拉框点了没有反应（Agent 用 Playwright 的 `selectOption`）。
- `options` 按文档顺序，最多 2000 项；`group` 是所在 `<optgroup>` 的标签，`disabled` 把 `<optgroup>` 的禁用算进去。`rect` 是下拉框在视口里的位置（整数）。
- 回应后服务端在页面里设值；值变了才派发 `input` 和 `change`（和浏览器自己的行为一致）。取消时什么都不做。
- 跨站的 iframe 里的下拉框拦不到（它在别的渲染进程里），第一期不处理。

`dialog`：

- 弹框所在的组有人在看时才发（没人看的按 §2.2 立刻关掉）。连接连上时已经开着的弹框（Agent 操作时页面弹出来的）在 `ready` 之后发。
- `default_prompt` 只有 `prompt` 带。`respond` 的 `prompt_text` 只在 `accept: true` 时有用。
- 触发弹框的那次输入不会卡住连接：弹框开着的时候 `ping`、`respond` 照常。
- Agent 操作时弹框也会发给屏幕，用户可以回应；Agent 的 Playwright 先处理掉的，屏幕收到 `request_closed`。

`file_chooser`：

- `accept` 取自 `<input type="file">` 的 `accept` 属性，按逗号分开；没有时是空数组。不是由 `<input type="file">` 打开的文件选择（`showOpenFilePicker`）不发请求。
- `browser.upload` 的 `zone_path` 是 Zone 文件系统里的路径（`cyfs://` 引用去掉前缀，如 `/home/Documents/a.pdf`），服务端用调用者的令牌读它：和 Desktop 读 Zone 文件走同一个接口（nfs-server 的 `stat` 加 `/nfs/v1/read/<node_id>`）。读不到（不存在、没有权限、是文件夹）回 `invalid_argument`。
- 一个请求可以收多个文件：依次上传，最后一次带 `done`（kRPC 里 `browser.upload {request_id, zone_path, done: true}`；PUT 里 `?done=1`）。只传一个文件时直接带 `done`。带 `done` 的那次返回时，文件已经一起交给页面的输入框，请求结束。
- `multiple: false` 的请求只收一个文件，`multiple: true` 的最多 32 个，多了回 `invalid_argument`。每个文件不超过 100 MiB（`upload_too_large`）。没传成的文件不占名额，可以重传。
- 页面看到的文件名是上传时给的名字（`zone_path` 的最后一段、PUT 的 `name`），去掉路径部分。
- 文件先存在浏览器目录的 `uploads/<request_id>/` 下。取消或失败的请求立刻删；已经交给页面的要留到页面真正读它的时候（提交表单时才读），所以在标签页关闭、浏览器停止时删，否则至少留 1 小时。

`download`：

- 下载的文件在浏览器的下载目录里，按 `download_id` 命名。完成时发 `state: "completed"`，`size` 是文件的字节数；被取消或失败时发 `state: "failed"`。
- 找得到是哪个标签页触发的，就发给它所在的组，`tab_id` 是它；找不到时 `tab_id` 是空串，发给每一条连接。
- 服务端记着最近 64 个下载，每个留 24 小时，过了就连文件一起删。记录在内存里，Agent Service 重启后以前的 `download_id` 都不认（`no_such_download`）。
- `browser.download_export` 把文件登记成命名对象，绑进这个 Agent 的 `artifacts` 桶，再把它交给调用者（nfs-server 的 `grant_holdings`），返回 `{obj_id, name, size}`。App 拿 `obj_id` 走 `useSaveToZone`，由用户自己的会话 `commit_object` 存进 Zone。交给调用者这一步要 nfs-server 的 `obj://ndm/grant`，按现在的 RBAC 它不总是成立（Agent 是只读权限、或 owner 不是管理员时没有这一项）：被拒绝时只记日志，`obj_id` 照常返回。owner 是管理员时不受影响（管理员的会话读得到任何对象）；owner 是普通用户时 `commit_object` 会被拒绝，要等 RBAC 补上。

`permission`：

- 页面会弹权限提示的那些权限，浏览器启动后一律设成拒绝：`geolocation`、`notifications`、`push`、`camera`、`microphone`、`midi`、`clipboard-read`、`idle-detection`、`window-management`、`local-fonts`、`nfc`、`storage-access`、`top-level-storage-access`、`display-capture`、`payment-handler`、`captured-surface-control`、`speaker-selection`。
- 告知靠注入的脚本，所以只覆盖脚本看得到的这几种调用：`geolocation`（`getCurrentPosition`、`watchPosition`）、`camera` 和 `microphone`（`getUserMedia`）、`notifications`（`Notification.requestPermission`）、`clipboard-read`（`clipboard.read`、`readText`）、`midi`（`requestMIDIAccess`），并且只在顶层文档里。其他权限只是被拒绝，不告知。
- 同一个标签页、同一个权限、同一个来源一分钟内只告知一次。`origin` 是发起请求的页面的来源。

### 3.5 帧的节奏

服务端手里最多有一帧没被 `ack`。新帧到了而上一帧还没确认，只留最新的一帧，旧的丢掉。另有帧率上限（默认每秒 15 帧，`quality` 可以调，最高 30）。客户端画完一帧再发 `ack`。

- `ack` 的 `seq` 大于等于最近发出的那一帧才算确认；确认更早的帧不放行下一帧。
- 客户端一直不 `ack`，服务端就不再发帧（`state`、`request` 这些照发）。
- 页面不变就没有新帧。客户端确认之后，服务端手里有比它新的画面才发。输入框有焦点时也一样：浏览器启动时关掉了光标闪烁（`--disable-blink-features=CaretBlinking`），光标常亮，不会每半秒出一帧。
- 切换标签页后的第一帧是新标签页当时的画面，`frame_meta.tab_id` 说明是哪个标签页的。
- 页面重新加载、换了渲染进程之后浏览器有时不出新帧。服务端在页面加载完 0.4 秒内没等到帧时把截屏流重开一次（重开一定会出一帧），所以这种情况下画面会晚不到半秒。标签页换了视口之后也一样：套完 0.4 秒内没等到新尺寸的帧就重开一次。
- 几条连接看同一个标签页时共用一路截屏流，各自按自己的 `ack` 和帧率取帧；画面是同一个视口下的（§3.2），`frame_meta` 的宽高是那个视口的。
- 帧的代价（手机 412×780、画质 60、每秒 15 帧，Chromium 159；测试页是小字和输入框，动画是一个方块来回滑，每帧都是整屏）：

  | `pixel_ratio` | 帧的像素 | 静止的一帧 | 动画时 | 动画时浏览器的 CPU |
  | --- | --- | --- | --- | --- |
  | 1 | 412×780 | 20.9 KB | 310 KiB/s | 0.29 核 |
  | 1.5 | 618×1170 | 36.9 KB | 540 KiB/s | 0.73 核 |
  | 2 | 824×1560 | 56.3 KB | 840 KiB/s | 1.07 核 |

  同样的画面，画质 30、40、70 的字节约是画质 60 的 0.71、0.82、1.12 倍。没有连接在看（没有截屏流）时，标签页在哪块屏上 CPU 都一样。

客户端建议（服务端不依赖这些，Desktop App 现在这样做，见 `src/frame/desktop/src/app/agent-browser/screen/quality.ts`）：

- 档位从清楚到省：清晰（`pixel_ratio` 取设备的像素比，最多 2，像素比不到 1.5 时取 1：1.25 这样的屏换来的清晰有限，服务端却要在 2 倍屏上画再缩小，CPU 约 2.5 倍；画质 70；每秒 15 帧）、标准（1、60、15）、省流（1、40、8）、极省（1、30、3）。用户可以固定选“清晰”或“省流”，默认“自动”。换档用一条 `quality` 同时发这三个值；`hello.viewport` 带当前档的 `pixel_ratio`，`ready` 之后马上发一次 `quality`。
- “自动”按帧本身量网速：一帧从 `frame_meta` 到它的 JPEG 收齐用的时间（传输）、从收齐到画完发 `ack` 的时间（绘制）、字节数。换档时已经在路上的那一帧还是旧档的，不算。
- 降档要快：最近 3 秒的帧平均传输超过 250 毫秒，或有一帧超过 700 毫秒，或传输加绘制占了这 3 秒的 70% 以上，或平均绘制超过 120 毫秒，马上降一档。
- 升档要稳：最近 10 秒至少 2 帧、合计至少 16 KB，按上面的字节倍数推算上一档的平均传输不到 100 毫秒、绘制（按像素数推算）不到 50 毫秒、占用不到 35%，连续 5 秒都这样才升一档。降档后 10 秒内不升；升档后 60 秒内又降了，这个等待加倍，最多 2 分钟，2 分钟没降过就回到 10 秒。没有帧时不升也不降。
- 这些数值只在 mock 的限速上验证过，真实的弱网还要再调。

### 3.6 有显示器的浏览器

`display` 是 `virtual` 时（§1、§6），屏幕看的是浏览器所在的整块虚拟显示器，用户在显示器上操作。连接、握手、`state`、`ack`、帧的节奏、`nav`、`tab`、`respond`、接手的规则不变，下面这些不同：

- 帧：显示器的画面（浏览器的窗口，带它自己的标签栏、地址栏、弹出的窗口、对话框、下拉列表、权限提示）。`frame_meta` 的 `width`、`height` 是显示器的 CSS 尺寸，坐标都相对显示器左上角；`tab_id` 是组的当前标签页。几条连接看到的是同一块显示器。
- 视口：最近设置视口的那条连接的 `width`、`height` 就是显示器的 CSS 尺寸，浏览器的窗口铺满它（页面打开的弹出窗口保持自己的大小，挪进显示器里）。宽度至少 500（浏览器的窗口不比这更窄），更窄的视口得到 500 宽的显示器，由 App 缩小显示。`mobile` 不起作用：不模拟手机，不改 UA，不开触屏，不重新加载；`state.viewport.mobile` 总是 `false`，App 因而把触摸换成鼠标和滚轮（§3.2）。改视口时浏览器的窗口跟着变，页面像在桌面上拖窗口一样重排。
- 像素：浏览器按固定的设备像素比画（`Status.device_scale_factor`，§6），页面的 `devicePixelRatio` 就是它，不随连接和网速变。连接要的 `pixel_ratio`（取 1 到设备像素比）只决定帧缩到多大：显示器本来的像素按比例缩小后编码。
- 输入：`mouse` 是显示器上的指针（按下的键随消息）；`wheel` 换成滚轮的格数（每格 53 个 CSS 像素，不足一格的留到下一次）；`key` 换成 X 的 keysym 按下、抬起，消息的 `modifiers` 先同步成按住的修饰键；`text` 一个字符一个字符地打（非拉丁字符由 Xvnc 临时映射到空闲的键上）；`composition` 不处理：输入法自己显示组字，上屏的文字作为 `text` 到页面。`touch` 也收：没移动就是在按下处点一下，移动超过 8 个 CSS 像素就是反方向的滚轮。
- 页面之外的请求（§3.4）：没有 `select`、`dialog`、`permission`。下拉列表、JS 弹框、权限提示都是浏览器自己的，画在显示器上，用户直接点；权限的选择由浏览器记在用户数据里，不再一律拒绝。`file_chooser` 照旧由服务端拦下、交给 App 上传本机或 Zone 的文件（页面拿到的仍是那几个文件）；`download` 照旧。没人在看、也不是 Agent 在操作时开着的 JS 弹框照 §2.2 关掉，只因为它会卡住 Agent 的 `connectOverCDP`。
- `focus`：仍由服务端在单独隔离环境里的脚本报告（只监听焦点，不拦任何事件、不改页面的 API），`rect` 是显示器上的坐标。
- 浏览器之外的显示器：服务端用它自己到显示器的 VNC 连接（只有服务端能打开的本地套接字），浏览器的调试连接只用来列标签页、读网址标题、导航、上传文件、管窗口的大小和前后（组的当前标签页的窗口在最前），不用来出帧和输入。

## 4. 本机文件上传

`PUT /kapi/<agent_id>/browser/upload/<request_id>?name=<文件名>[&done=1]`，请求头 `Authorization: Bearer <会话令牌>`，请求体是文件的字节。按 owner 校验（和 kRPC 同一套判定，方法名是 `browser.upload`）。上限默认 100 MiB，超了回 413 和 `upload_too_large`。成功回 200 和 `{}`。

- `request_id` 是 `file_chooser` 请求的 id。`name` 是页面看到的文件名，必填，按 URL 编码；`done=1`（或 `done=true`）表示这是这个请求的最后一个文件（§3.4）。
- 请求体边收边写盘。`Content-Length` 超过上限时不读请求体直接回 413；没有 `Content-Length` 的，收到超过上限时中止。
- 失败时正文是 `{"code", "message"}`：

| 状态 | `code` | 情况 |
| --- | --- | --- |
| 400 | `invalid_argument` | 没有 `name`；这个请求不再收文件（单选的已经有一个，或超过 32 个） |
| 401 | `unauthorized` | 没带令牌 |
| 403 | `unauthorized` | 令牌不是 owner 的（包括 Agent 自己的 App 身份） |
| 404 | `no_such_request` | 没有这个 `file_chooser` 请求，或它已经结束 |
| 413 | `upload_too_large` | 超过上限 |
| 503 | `browser_unavailable` | 这个 Agent 没有浏览器；正文另带 `unavailable_code`（§2.3） |
| 500 | `other` | 写盘失败 |

## 5. 重排

### 5.1 桶和 key

桶 `user/control-panel/<用户>/reflow`，由 control-panel 声明（`buckyos_api::CONTROL_PANEL_REFLOW_BUCKET`：用户桶、`app_data`、配额 1024 MiB、不开版本）。key：

| key | 内容 |
| --- | --- |
| `<site>/manifest.json` | 包的声明 |
| `<site>/refresh.mjs` | 刷新脚本 |
| `<site>/index.html` 和它引用的文件 | 页面代码 |
| `<site>/results/<page>.json` | 一个页面最近一次制作的结果 |
| `<site>/fallback/<page>.html`、`<site>/fallback/<page>.json` | 兜底重排的页面和它的元信息 |
| `<site>/preference.json` | 站点还没有包时用户对它的偏好（有包时偏好在 `manifest.json` 里，见 §5.2） |

- 只有 `http`、`https` 的网址有重排。别的 scheme、解析不了的网址、主机名里有 `a-z`、`0-9`、`.`、`_`、`-` 以外字符的网址（IPv6 字面量在内）都没有。
- 规范化网址：按 WHATWG URL 解析后再序列化，去掉片段、用户名和密码。序列化的结果是：scheme 和主机名小写，默认端口去掉，空路径是 `/`，`.` 和 `..` 段折叠，非 ASCII 字符写成百分号编码。查询串原样保留，参数不排序。JS 的 `URL` 和 Rust 的 `url` crate 都这样实现，下面的向量在 Deno 和 `url` 2.5.8 上核对过。
- `<site>`：规范化网址的主机名（不带端口，去掉末尾的点），点换成短横线；换完不含短横线的加后缀 `-host`。`www.facebook.com` → `www-facebook-com`，`localhost` → `localhost-host`。`<site>` 总带短横线，所以不会和网关按路径分流的前缀（`kapi`、`bucket`、`home` 等）相同。
- 不同的主机名可能得到同一个 `<site>`（`a-b.com` 和 `a.b.com`）。一个 `<site>` 下只有一个包，它是哪个主机的由 `match[].host` 决定。结果和兜底页按 `<page>` 存，不会混。
- `<page>`：规范化网址（UTF-8）的 SHA-256 的前 16 个十六进制字符，小写。
- 这些算法和 §5.2 的匹配规则有一份共用的测试向量 [`fixtures/reflow_vectors.json`](fixtures/reflow_vectors.json)：`normalize` 是一组 `{url, normalized, site, page, view}`，`invalid` 是没有重排的网址，`match` 是一组 `{manifest_match, url, matches}`。`reflow` 命令（`src/tools/buckyos-agent/lib/reflow.ts`）的测试用它，Agent Service 的实现也用它核对。

### 5.2 包的格式

`manifest.json`：

```json
{
  "schema": "buckyos.reflow/1",
  "site": "www-example-com",
  "match": [ { "host": "www.example.com", "path": "/news/*" } ],
  "preference": "只看标题、日期和正文，手机单栏",
  "author": { "kind": "agent", "id": "did:bns:jarvis.alice" },
  "updated_at": 1794600000000,
  "refresh": { "script": "refresh.mjs", "timeout_s": 60 }
}
```

- `site` 等于 key 里的 `<site>`。
- `match[]` 至少一条。`host` 是小写的主机名，不带端口，它的 `<site>` 必须就是这个包的 `<site>`。`path` 以 `/` 开头，是通配：`*` 匹配任意一段字符（含 `/`，也可以是空的），其余字符按原样比较，区分大小写，要从头到尾对上；省略等于 `/*`。
- 包适用于一个网址的条件：`match[]` 里有一条，它的 `host` 等于规范化网址的主机名（去掉末尾的点），`path` 匹配规范化网址的路径。比较的是序列化之后的路径（百分号编码的形式），不含查询串。
- `preference` 可以省略。`author` 是 `{kind: "agent" | "user", id}`。`updated_at` 是放进库的时间（毫秒），由放入的一方写：`reflow put` 会写它，manifest 没有 `author` 时用 Agent 的 DID 补上。
- `refresh` 可以省略，省略时超时按 60 秒。`script` 第一版固定是 `refresh.mjs`；`timeout_s` 是 1 到 600 的整数。
- 一个包至少有 `refresh.mjs` 和 `index.html` 之一。只有 `index.html` 的包不能刷新，结果由 Agent 用 `reflow result put` 放进去。写了 `refresh` 就必须有 `refresh.mjs`。`results/`、`fallback/` 是库的数据，不属于包。
- manifest 不满足上面这些规则时，当作这个站点没有包。
- 放入一个包是整包替换：先写文件，`manifest.json` 最后写，再删掉新包里没有的旧文件。`results/` 和 `fallback/` 保留。
- 偏好是用户对一个站点的一句话，由 App 的重排视图读写。站点有合法的包时它是 manifest 的 `preference`：App 改写 `manifest.json` 的这一个字段（清空就去掉这个字段），别的字段不动。没有合法的包时它在 `<site>/preference.json` 里：`{ "schema": "buckyos.reflow.preference/1", "preference", "updated_at" }`，清空就删掉这个文件。
- `preference.json` 是库的数据，不属于包：包的根下不能有这个名字的文件（校验不过），`reflow get` 不取它，删除站点时连它一起删。
- 放入包时偏好这样定，`reflow put` 和 App 的导入相同：新包的 manifest 写了 `preference`（空串也算写了）就用它；没写时把站点现有的偏好写进去，站点原来有合法的包时取旧 manifest 的 `preference`，否则取 `preference.json` 里的，都没有就不带这个字段。`manifest.json` 写完之后删掉 `preference.json`，包的 `preference` 接替它。Agent 写包时仍应把用户的偏好写进 manifest：偏好在请求里，`reflow match <网址>` 也会给出站点现有的偏好。
- App 的管理页也能放入包（导入一个 zip，或 Zone 里的一个包目录），规则同 `reflow put`：同样的校验和文件大小上限，同样的写入顺序，按扩展名带 `Content-Type`，写 `updated_at`，manifest 没有 `author` 时补 `{ "kind": "user", "id": "<用户 id>" }`。`<site>` 取 manifest 的 `site`。任何一段以 `.` 开头的文件不算包的文件；zip 里只有一个顶层目录、`manifest.json` 在它里面时，这个目录就是包。
- 包里的文件在 `reflow.<zone>` 上按存进桶时声明的类型下发，没有声明或声明成 `application/octet-stream` 时按扩展名。`reflow put` 按扩展名带上 `Content-Type`。
- `refresh.mjs` 的默认导出：`async ({ page, url, save }) => data`。`page` 是已经打开 `url` 的 Playwright 页面。`save.image(网址)` 把图片取回、压缩，返回 `data:` 引用。返回值必须能序列化成 JSON。
- 结果 `results/<page>.json`：`{ "schema": "buckyos.reflow.result/1", "source_url", "made_at", "made_by", "data" }`。`made_at` 是毫秒时间戳，`made_by` 是 Agent 的 DID。
- 结果里依赖原网页的动作这样记：`{ "action": { "selector": "<原网页上的 CSS 选择器>" } }`，放在 `data` 里页面代码认得的位置。
- 兜底的元信息 `fallback/<page>.json`：`{ "schema": "buckyos.reflow.fallback/1", "source_url", "made_at", "made_by" }`。
- 页面代码把取回的数据按文本渲染，不当 HTML 插入。

### 5.3 kRPC

`browser.reflow_state {url}` 返回：

```jsonc
{
  "url": "<规范化网址>", "site": "www-example-com", "page": "0123456789abcdef",
  "package": null,     // { "updated_at", "author", "preference" }：有匹配的包
  "result": null,      // { "made_at", "made_by", "source_url" }
  "fallback": null,    // { "made_at", "made_by", "source_url" }
  "refresh": { "status": "idle", "error": null, "at_ms": null },   // idle | running | failed
  "view": null         // 能显示时：重排页相对 reflow.<zone> 的路径
}
```

- `view`：有包也有结果时是 `<site>/?url=<编码后的规范化网址>&page=<page>`；否则有兜底结果时是 `<site>/fallback/<page>.html`；都没有是 `null`。`url` 的值按 `encodeURIComponent` 的规则编码（字母、数字和 `-_.!~*'()` 以外的字节都写成 `%XX`），向量里的 `view` 就是有包有结果时的值。
- `package`：这个站点的包合规（§5.2）并且适用于这个网址时有值，否则是 `null`。`result`、`fallback` 取自 `results/<page>.json`、`fallback/<page>.json` 里的这三个字段，文件不在或不是 JSON 时是 `null`；有没有 `result` 不看有没有包。
- `refresh`：这个网址的刷新。`running` 时 `at_ms` 是开始的时间；`failed` 表示最近一次刷新失败了，`error` 是原因，`at_ms` 是失败的时间；`idle` 时两项都是 `null`（没刷新过，或最近一次成功了，制作时间看 `result.made_at`）。这个状态只在 Agent Service 的内存里，服务重启后回到 `idle`。
- 不需要浏览器：不启动它，模块关着（`enabled = false`）、这个节点没装 ext-core、浏览器还没下载好时也照常返回，已经做好的页面照样能显示。`reflow_refresh`、`reflow_action` 这时回 `browser_unavailable`（带 `unavailable_code`，§2.3）。
- `refresh` 是读桶之前取的：刷新在读桶期间结束时，这一次返回的仍是 `running`（旁边是结束之前的 `result`），下一次才是 `idle` 和新的 `result`。不会出现 `idle` 而看不到这次结果的情况。
- 只读 `manifest.json`、`results/<page>.json`、`fallback/<page>.json` 三个 key；`<site>/preference.json`（§5.2）不读，返回里也没有它。
- 桶的读写用调用者的令牌，走 nfs-server 的 `/bucket/v1`：调用者在重排库里能做什么由 Zone 的权限决定。调用者没带令牌时（本机调试，回环地址上的调用）用 Agent Service 自己的身份。读写被拒或失败时回 `code: other`，`message` 是 `reflow library: <bucket_id>/<key>: <桶的错误码>: <说明>`。
- `url` 没有重排（§5.1）时回 `invalid_argument`。

`browser.reflow_refresh {url}`：没有匹配的包回 `reflow_no_package`。否则在后台标签页里开始刷新，立刻返回和 `reflow_state` 一样的结构（`refresh.status` 是 `running`）。结束后 `reflow_state` 里能看到新的 `result`，失败时 `refresh.status` 是 `failed`、`error` 有说明。同一个网址同时只跑一个刷新：还在跑的时候再调，返回的是同一次刷新的状态。

- 这个节点上找不到 `agent-browser` 时回 `browser_unavailable`。环境变量 `OPENDAN_AGENT_BROWSER` 可以指定它（调试和测试用），不设时在工具目录里找，规则同 `chromium`（§6）。
- 一个 Agent 的浏览器里同一时刻只有一个刷新在跑，别的网址的刷新排队，排队期间状态也是 `running`。刷新进行中算浏览器在用，不会被空闲回收。
- 结果用发起这次刷新的调用者的令牌写进桶。只读权限的 Agent 读得了库、发得起刷新，但结果写不进去：状态是 `failed`，`error` 是桶的原话（`reflow library: …/results/<page>.json: permission_denied: …`）。
- 包里没有 `refresh.mjs`（只有页面代码的包）时，状态是 `failed`，`error` 说明这个包不能刷新。

`browser.reflow_action {url, selector}`：在后台标签页里打开原网址，点一下 `selector` 指的元素，等页面稳定后按包重新刷新这一页。返回同 `reflow_refresh`。这个网址已经有刷新在跑时，动作不会丢：这一次结束后紧接着再刷新一次，带上这个点击，期间状态一直是 `running`；等待期间又来的动作取代前一个。

刷新由 Agent Service 启动 `agent-browser` 的内部子命令执行：

```
agent-browser reflow-run --cdp <cdp_url> --url <网址> --package <本地目录> --out <结果文件> [--click <selector>] [--timeout <秒>]
```

它连上常驻浏览器，在一个新标签页里打开网址，跑 `<本地目录>/refresh.mjs`，把 `data` 写进结果文件（JSON），关掉标签页。成功退出码 0；失败非 0，标准错误的最后一行是原因。Agent Service 负责把包从桶里取到本地目录、把结果写回桶。

- 结果文件的内容就是 `refresh.mjs` 返回值的 JSON，不带 `schema`、`source_url` 这一层。
- `--timeout` 默认 60，管从连接到写完结果的全过程。
- 带 `--click` 时：打开网址，点一下选择器匹配到的第一个元素，等页面装载完、网络空闲（最多再等 10 秒），再跑 `refresh.mjs`。传给它的 `url` 仍是 `--url` 的值。
- 不调 kRPC，不取租约。新标签页开在单独的窗口里（§2.5）。
- Agent Service 这一侧：本地目录在浏览器目录（§6）下的 `reflow/<page>/`，刷新结束就删掉。取到里面的只有 `manifest.json` 和 `refresh.mjs`，所以 `refresh.mjs` 不能引用包里别的文件。`--url` 是规范化网址，结果的 `source_url` 也是它；`--timeout` 取 manifest 的 `refresh.timeout_s`；`--click` 写成 `--click=<selector>`。`made_at` 是把结果写进桶的时间。
- Agent Service 等它 `timeout_s` 加 10 秒，到时还没结束就连同它启动的进程一起结束，状态是 `failed`。它失败时 `error` 是它标准错误的最后一行。
- `refresh.mjs` 里页面动作（定位、等待）的默认超时是 `timeout_s` 的三分之一，最长 15 秒，脚本可以逐次另给；页面跳转的超时是 `timeout_s`。这样页面改版、选择器落空时，`error` 说的是哪一个选择器没等到，而且不用等满整次刷新的时间。
- 它不继承 Agent Service 的环境。留下的只有 `PATH`、`LANG`、`LC_ALL`、`LC_CTYPE`、`TZ`，外加指到本地目录里的 `HOME`（`home/`）和 `TMPDIR`（`tmp/`）；Windows 上是 `PATH`、`PATHEXT`、`ComSpec`、`SystemRoot`、`SystemDrive`、`windir`、`ProgramData`、`ProgramFiles`、`ProgramFiles(x86)`，外加 `USERPROFILE`、`APPDATA`、`LOCALAPPDATA`、`TEMP`、`TMP`。所以服务的会话令牌（`BUCKYOS_APPCLIENT_SESSION_TOKEN` 等）和 `OPENDAN_BROWSER_RPC` 到不了 `refresh.mjs` 手里：包可能是别的 Agent 或用户写的。它仍以 Agent Service 的系统身份运行。
- 刷新只读包的 `manifest.json` 和 `refresh.mjs`，只写 `results/<page>.json`，不列目录、不删任何 key：`<site>/preference.json` 和包里别的文件它都不碰。
- 它开的标签页归 `_reflow` 组（§1）：从它启动到这个标签页出现，新出现的没有 opener 的标签页都归这个组。它结束后 `_reflow` 里还剩下的标签页（它被强行结束时留下的）由 Agent Service 关掉。
- `save.image(网址)`：相对网址按页面地址解析；带着浏览器上下文的 cookie 和 `Referer` 取回；长边缩到 1280 以内，转成 JPEG（质量 0.8，透明处填白），返回 `data:image/jpeg;base64,…`。SVG 原样转成 `data:`，`data:` 网址原样返回。

### 5.4 重排页

- 地址 `https://reflow.<zone>/<key>`，由 nfs-server 提供（`reflow-<zone>` 这种写法网关也送到同一处）。路径按段做百分号解码后就是登录用户的重排桶里的 key。以 `/` 结尾的路径补 `index.html`；`/<site>` 这样不带斜杠的路径，在 `<site>/index.html` 存在时回 307 到 `/<site>/`。路径里有 `.`、`..`、空段或 `\` 的回 404。
- 只有 `GET` 和 `HEAD`，其他方法回 405。nfs-server 的其他接口在这个主机上都不应答。网关按路径分流的前缀（`/kapi/*`、`/bucket/*`、`/ndn/*`、`/ndm/*`、`/nfs/v1/*`、`/s3/*`、`/home/*`、`/sso_callback`、`/sso_refresh`、`/sso_logout` 等）在这个主机上照旧先被分走。
- 响应：`Content-Type` 是文件存进桶时声明的类型（§5.2）；没有 `Content-Disposition`，没有 CSP；有 `X-Content-Type-Options: nosniff`、`Referrer-Policy: no-referrer` 和 `ETag`；`Cache-Control` 默认是 `private, no-cache`。支持 `If-None-Match` 和 `Range`。
- 登录：用户是谁由这个主机上的会话决定。App 调 control-panel 的 `auth.prepare_app_launch { service_id: "nfs-server", redirect_url: "https://reflow.<zone>/<view>" }`，得到 `{ status: "ready", callback_url }`，把 iframe 指到 `callback_url`（`https://reflow.<zone>/sso_callback?nonce=…&redirect_url=…`，一次性，60 秒内有效）。control-panel 在这个主机上种下会话 cookie，再跳到 `redirect_url`。
- nfs-server 在这个主机上只认这种会话：用户会话，目标是 `System(nfs-server)`，放在 cookie 或 `Authorization: Bearer` 里都行。没有、过期，或者是别的会话（Desktop 自己的、App 的、Agent 的），都回 401，正文是纯文本，不跳登录页。读到的总是会话所属用户自己的桶。
- 会话 cookie 15 分钟过期，刷新令牌 7 天。App 每次给 iframe 换地址都重新走一遍 `prepare_app_launch`。页面自己取文件（如 `results/<page>.json`）遇到 401 时，先 `POST /sso_refresh`（同源，control-panel 会重新种下 cookie），再重试一次。
- 这种会话只用来打开重排页：在 nfs-server 的文件、桶和上传接口上，带着它的 Bearer 被拒，cookie 按匿名处理。
- 有包的页面：`https://reflow.<zone>/<site>/?url=<原网址>&page=<page>`。页面代码读同目录下的 `results/<page>.json`。
- 兜底页面：`https://reflow.<zone>/<site>/fallback/<page>.html`。

### 5.5 页面和 App 的消息

重排页在 App 的 iframe 里。页面 `window.parent.postMessage(消息, '*')`；App 只收来源是 `https://reflow.<zone>` 的消息。

| 方向 | 消息 | 含义 |
| --- | --- | --- |
| 页面 → App | `{ "type": "reflow.ready" }` | 页面画完了 |
| 页面 → App | `{ "type": "reflow.open", "url": "<网址>" }` | 打开一个链接：App 按重排的触发规则处理 |
| 页面 → App | `{ "type": "reflow.action", "selector": "<选择器>" }` | 在原网页上执行动作：App 调 `browser.reflow_action` |
| 页面 → App | `{ "type": "reflow.refresh" }` | 请求刷新 |
| 页面 → App | `{ "type": "reflow.source" }` | 切回原始页面 |
| App → 页面 | `{ "type": "reflow.updated" }` | 结果更新了，页面重新读 `results/<page>.json` |

- 有包的页面和兜底页用同一套消息、同一个来源校验。兜底页也可以发 `reflow.ready`、`reflow.open`（以及 `reflow.refresh`、`reflow.source`）；它没有包，不发 `reflow.action`。
- App 只收自己那个 iframe 的窗口发来、并且来源是 `https://reflow.<zone>` 的消息，两样都校验。App 发给页面的消息以这个来源作 `targetOrigin`。
- `reflow.ready` 可以不发：iframe 的 `load` 事件也算画完。
- `reflow.open` 的 `url` 是 `http`、`https` 的网址；相对网址按这一页的原网址解析，别的 scheme 忽略。
- `reflow.action`：同一时间只执行一个，执行期间进行中的提示由 App 显示，页面不用等回应。成功后 App 发 `reflow.updated`。失败（包括 `reflow_no_package`）时页面收不到消息：App 告诉用户，并把这件事交给 Agent（§5.6）。
- `reflow.refresh`、`reflow.source` 和 App 自己的“刷新”“原始页面”按钮是同一件事。刷新完成后，有包的页面收到 `reflow.updated`；兜底页不会收到它，内容变了由 App 重新加载 iframe。

### 5.6 App 的重排视图

App 的重排模式不连屏幕，地址栏里是原网址。对当前网址：

1. 调 `browser.reflow_state {url}`。有 `view` 就在 iframe 里显示（§5.4），旁边显示这一页的制作时间和制作者：有包有结果时取 `result`，否则取 `fallback`。
2. 没有 `view`、有包：调 `browser.reflow_refresh`，显示“正在制作”，每 2 秒查一次 `reflow_state`。`refresh.status` 已经是 `running` 时不再调，直接等。
3. 没有 `view`、没有包，或者刷新失败（`refresh.status` 变成 `failed`，或刷新结束了仍然没有 `view`）：往浏览器会话发一条普通消息请 Agent 处理，显示“正在制作”，每 4 秒查一次 `reflow_state`，直到出现 `view`。`browser.reflow_refresh` 这次调用本身出错（刷新没有开始）不算刷新失败：App 只提示，不请 Agent。

- App 发给 Agent 的消息有五种，都带规范化网址：请它做兜底页（带用户对这个站点的偏好）；刷新失败（带 `refresh.error`、`<site>` 和偏好，请它先做兜底页再修包）；用户要求重做一张兜底页；`reflow.action` 失败（带选择器和原因）；用户改了偏好（带新的偏好，请它改包并重做这一页）。
- 同一个 Agent、同一个网址只请一次（`reflow.action` 的失败按选择器另算），直到这一页显示的内容变了（`view` 变了，或它的 `made_at` 变了）；用户可以明确要求再请一次，改偏好每次都发。
- `browser.reflow_state` 在浏览器不可用时照常返回（§5.3），已经做好的页面照样显示。`browser.reflow_refresh`、`browser.reflow_action` 回 `browser_unavailable` 时，App 不刷新、不执行动作、不请 Agent，只说明原因。
- 请 Agent 的消息要等对话一侧就绪才发得出去。等的这段时间里用户已经换了网址的话，请它做兜底页、请它重做这两种消息不再发出。
- “原始页面”：切到原始页面模式，在浏览器里打开这一页的 `source_url`（还没有结果时是规范化网址）。
- 进入重排模式时，当前网址取浏览器当前标签页的网址；之后它跟着重排页里的链接和地址栏走，不跟着浏览器走，直到下次进入重排模式时浏览器已经到了别的网址。

## 6. 配置和目录

`agent.toml` 的 `[loader.browser]`：

| 键 | 默认 | 含义 |
| --- | --- | --- |
| `enabled` | `true` | 关掉后所有 `browser.*`（`status`、`reflow_state` 除外）回 `browser_unavailable`（`unavailable_code` 是 `disabled`），进程环境里也不设 `OPENDAN_BROWSER_RPC` |
| `idle_stop_secs` | `600` | 没有人看、没有租约，超过这个时间停掉浏览器。停之前把各组标签页的网址存下来，下次启动时重开 |
| `max_tabs` | `12` | 所有组的标签页总数上限。服务端到了上限不再开（`too_many_tabs`）；别处开出来的超出上限的标签页会被立刻关掉 |
| `display` | `auto` | `virtual`：有窗口地跑在虚拟显示器上；`headless`：无头；`auto`：Linux 上是 `virtual`，其他平台是 `headless`（macOS、Windows 的显示器还没做）。`virtual` 而没有 `Xvnc` 时报 `no_display`，不改成无头 |
| `device_scale_factor` | `2` | 虚拟显示器的设备像素比（1–3）：浏览器按它画，页面看到的 `devicePixelRatio` 就是它。手机上要清楚就要 2；节点 CPU 紧时可以设 1 |

这个节点上没有浏览器可以启动时，模块报“不可用”（`browser.status` 的 `available: false`，`browser.reflow_state` 照常，其他方法回 `browser_unavailable`），Agent Service 的其他功能照常。分两种，`unavailable_code` 不同（§2.1）：

- 工具目录里没有 `chromium` 启动器（`no_package`）：这个节点没装 ext-core。
- 启动器在，浏览器本体不在（`not_downloaded`）。浏览器（Chrome for Testing）是 ext-core 的在线项 `chromium`：不随包分发，包装好以后由节点从官方地址下载到工具目录的 `online/chromium/`，状态记在旁边的 `online/chromium.json`（`state`：`downloading`、`ready`、`failed`；没有这个文件就是还没下载）。记录是 `ready` 并且目录在才算下载好。下载失败后不会自动重试，管理员在“设置 > 扩展工具”里再次触发。

两种都是每次调用现查，装好、下载好之后不用重启 Agent Service。

目录：

| 用途 | 容器里 | 原生运行、宿主调试 |
| --- | --- | --- |
| 用户数据、缓存、下载、等着交给页面的上传文件 | `${BUCKYOS_INSTANCE_VOLUME}/browser/{profile,cache,downloads,uploads}` | `<AgentRoot>/.cache/browser/{profile,cache,downloads,uploads}` |
| 各组的标签页 | `<AgentRoot>/state/browser/groups.json` | 同左 |
| 浏览器本体（ext-core 的在线项，节点上只有一份，各 Agent 共用，只读）和它的下载记录 | `/opt/buckyos/tools/online/chromium/`、`/opt/buckyos/tools/online/chromium.json` | `$BUCKYOS_ROOT/tools/online/chromium/`、`$BUCKYOS_ROOT/tools/online/chromium.json` |

环境变量 `OPENDAN_BROWSER_DIR` 可以指定第一行的位置（入口脚本在容器里设它）。`OPENDAN_CHROMIUM` 可以指定浏览器的可执行文件（调试和测试用）：设了就直接启动它，不看在线项；指的文件不存在，按 `no_package` 报。不设时在工具目录（`$BUCKYOS_ROOT/tools/bin`、`$BUCKYOS_ROOT/bin/agent-tools`）里找 `chromium` 启动器，启动的也是它（启动器再去执行 `online/chromium/` 里的浏览器）；在线项按启动器的位置找：`<启动器所在目录>/../online/chromium{,.json}`。`OPENDAN_AGENT_BROWSER` 同样指定 `agent-browser`（重排刷新由它执行，§5.3），不设时在同一组工具目录里找 `agent-browser`；找不到时只有 `reflow_refresh`、`reflow_action` 回 `browser_unavailable`。

第一行的位置下还有 `chromium.log`：最近一次启动的标准错误，启动失败时它的末尾会进 `unavailable`；以及 `reflow/<page>/`：一次重排刷新的临时目录（取出来的包、结果文件、子进程的 `HOME` 和 `TMPDIR`），刷新结束就删掉。`groups.json` 的格式见 [Agent State Protocol](Agent%20State%20Protocol.md) §1。

进程：

- 虚拟显示器（`display: virtual`）：浏览器启动前服务端起一个 `Xvnc :<n>`（`n` 从 20 起找空闲的，`-geometry` 是默认 1280×800 乘设备像素比，`-dpi` 96 乘设备像素比，`-depth 24`），不开 TCP（`-rfbport -1 -nolisten tcp`），只开一个本地套接字 `/tmp/opendan-display-<n>.rfb`（`-rfbunixmode 0600`，不要密码，`-AcceptSetDesktopSize`）；日志在 `display.log`。浏览器停掉后它也停掉，浏览器崩溃重启时一起重来。`Xvnc` 来自 Agent 的容器镜像（`build_aios`，一个占位包替掉它用不到的 Mesa 和 Perl，见 `doc/app_service/paios容器需求.md`）；`OPENDAN_XVNC` 可以指定别的。有显示器时浏览器的参数：`--user-data-dir=<profile> --disk-cache-dir=<cache> --remote-debugging-port=<端口> --no-first-run --no-default-browser-check --disable-dev-shm-usage --no-startup-window --disable-blink-features=CaretBlinking --window-position=0,0 --window-size=<显示器的 CSS 尺寸> --force-device-scale-factor=<设备像素比>`，环境变量 `DISPLAY=:<n>`；不带 `--headless`、`--user-agent`、`--screen-info`。
- 调试端口：服务端先在回环地址上找一个空闲端口给浏览器（`--remote-debugging-port=<端口>`），从浏览器标准错误里的 `DevTools listening on ws://127.0.0.1:<端口>/devtools/browser/<id>` 读到地址（有窗口的浏览器给了端口就不写 `DevToolsActivePort`）；端口被别的进程抢了（`Cannot start http server for devtools`）就换一个再起，最多 4 次。
- 无头时的启动参数：`--headless=new --user-data-dir=<profile> --disk-cache-dir=<cache> --remote-debugging-port=<端口> --no-first-run --no-default-browser-check --disable-dev-shm-usage --no-startup-window --window-size=1280,800 --screen-info={0,0 800x600}{1600,0 1600x1200 devicePixelRatio=2} --disable-blink-features=CaretBlinking`。`--screen-info` 给无头浏览器两块屏：第一块和不带这个参数时一样（800×600，像素密度 1），标签页的窗口都开在这里；第二块 CSS 尺寸相同、像素密度 2，有连接要更密的帧时把标签页的窗口挪过去（`Browser.setWindowBounds` 的 `left` 设 1600，回来设 0；只设位置，不设宽高），帧就按 2 倍出（§3.2 `pixel_ratio`）。这个参数本身不改变什么：没有窗口在第二块屏时 CPU 和不带它一样，只有 `screen.isExtended` 变成真。最后一个让输入框的光标常亮不闪（闪烁的光标每秒让截屏流出两帧整帧）；它是 Blink 的特性开关，Chromium 159 认、156 不认，换 Chromium 版本时要重跑光标那条真浏览器用例（`browser::screen_tests::real::the_caret_of_a_focused_field_paints_no_frames`）。浏览器来自 ext-core（读得到 `browser/versions.json`）时再加 `--user-agent=<同版本有窗口的 Chrome 的 UA>`：无头模式默认的 UA 带 `HeadlessChrome`，很多网站会把它当机器人。`versions.json` 随包，按启动器的位置找（`<启动器所在目录>/../store/ext-core/browser/versions.json`），和浏览器本体下载到哪里无关。Unix 上有效用户是 root 时加 `--no-sandbox`（两种跑法都是）；不是 root 时先不加，因为 `No usable sandbox` 起不来时加上再试一次。调试端口只监听回环地址。下载用 `Browser.setDownloadBehavior {behavior: "allowAndName", downloadPath: <downloads>, eventsEnabled: true}` 指到下载目录。
- 浏览器没有自己的标签页（`--no-startup-window`）：每个标签页都属于某个组。
- 用户数据目录上一次留下的锁（`SingletonLock` 等，记着主机名和进程号，容器每次重建主机名都会变）在启动前删掉；锁指着的进程还在、并且确实是用这个目录起的浏览器时，先把它结束。
- Chromium 在 `TMPDIR` 下建一个套接字，路径太长（`TMPDIR` 超过约 60 字节）就起不来。Agent Service 自己的 `TMPDIR` 超过 50 字节时，浏览器进程改用 `/tmp`。
- 停止：先发 `Browser.close`，等 5 秒；没退出就给进程组发终止信号，再等 3 秒；最后强杀进程组里剩下的，并回收其中成了 Agent Service 子进程的那些（容器里 Agent Service 是 1 号进程）。Windows 上用 Job Object 结束整棵进程树。
- Chromium 的崩溃处理进程（`chrome_crashpad_handler`，两个）会脱离浏览器的进程组，父进程变成 1 号进程。浏览器启动时带一个环境变量 `OPENDAN_BROWSER_MARK=<每次启动不同的值>`，它启动的进程都继承；服务端在启动后和停止前按这个变量找出不在进程组里的进程，记下进程号和启动时间，停止或崩溃时一并结束、回收。只回收这两类记下来的进程（进程组里的、带标记的），回收前核对启动时间，不会回收别的子进程。
- 崩溃或被杀：把当时的标签页存下来，1 秒后重启并重开，连续崩溃时间隔加倍；连续 5 次起不来就不再自动重启，等下一次 `ensure` / `acquire`。
