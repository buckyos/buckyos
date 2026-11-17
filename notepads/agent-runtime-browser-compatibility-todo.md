# Agent 浏览器普通浏览与登录兼容性 TODO

状态：Linux 部分已实施并提交（`2c390732f`），部署本机 Zone 验证，owner 手动 Google 登录成功。owner 定（2026-11-16）：本版发布时 Linux 用有窗口 + Xvnc，Windows / macOS 继续无头，发布后收集不同环境用户的反馈再定下一步（§11）。实施记录在 §9，待 owner 定的在 §10。日期：2026-11-16。源码核对基线：`6c0e2eef8`；行号以本次核对为准，实施时重新定位。

前置：[Agent 浏览器实施 TODO](agent-runtime-browser-todo.md) 的主体能力已实现；现有接口见 [Agent Browser Protocol](<../doc/opendan/protocol/Agent Browser Protocol.md>)。本文规划后续改造，尚未改变现有协议和运行方式。

## 1. 目标与范围

Owner 的目标：**用户能像用普通浏览器一样使用 Agent 浏览器。** 当前具体失败：用户手动登录 Google 时收到 “This browser or app may not be secure”。Owner 提出以浏览器指纹与标准浏览器一致为核心，并参考 [agent-browser-runtime](https://github.com/energypantry/agent-browser-runtime)。

交付结果：用户从 BuckyOS 打开浏览器，能正常浏览、登录、处理登录弹窗和验证、上传下载；交给 Agent 后继续使用同一份登录态，接回来仍能正常操作。Google 登录是必验场景。验收同时检查原生浏览器环境和真实操作结果，不能用某个检测页的分数代替登录成功。

本期改造对象是 **Agent 的常驻浏览器**。继续复用每 Agent 的 private runtime、持久 profile、浏览器会话、控制权交接、重排和现有工具。项目测试和 `page-render` 继续按各自用途使用 headless 与独立 profile；优先复用同一份浏览器二进制。

“普通浏览器”在这里指远程运行的浏览器。网站访问从 Personal Server 发出，硬件能力来自运行节点；用户手机是显示和输入终端。手机摄像头、麦克风、系统 passkey 等能否转交远程浏览器，需要独立能力支持，不能靠修改指纹获得。

本期的 Google 登录指网站账号登录与第三方 Google OAuth；Chrome 浏览器自身的账号同步单独列入能力清单，不与网页登录混为一项。

## 2. 浏览器指纹一致性的定义

对照基线是：**同一运行节点、同一操作系统和架构、同一浏览器发行版与版本、相同显示及网络环境下，由用户手动使用、未经 BuckyOS 注入和模拟的 headed 浏览器。** 先比较同一份 Chrome for Testing；Chrome Stable 作为发行版对照另测。

一致性意味着保留浏览器自身给出的身份与能力，消除 BuckyOS 制造的矛盾。不同机器、显卡、字体、版本之间允许有正常差异；不要求复制用户手机或笔记本的指纹，不要求所有机器的 canvas/TLS 结果逐字节相同。

| 表面 | 实施要求 |
| --- | --- |
| UA、UA-CH、平台与版本 | 由实际运行的浏览器产生；HTTP 请求头与 JS 读数相符；保留上游的版本缩减与隐私行为，不硬编码品牌、GREASE 值或高熵字段 |
| 自动化状态 | 人工使用时对照原生基线；通过选择正确运行方式消除额外自动化启动行为。记录 CDP 连接前后变化，不用脚本改写 `navigator.webdriver` |
| 语言、时区与网络出口 | 使用浏览器/系统的真实配置；用户设置可持久化。记录代理和出口差异，不根据网站随机切换身份，不伪造定位 |
| 图形、字体、音频、媒体能力 | 使用实际图形栈、已安装字体和实际编解码器；软件渲染如实呈现。禁止用 canvas/audio 噪声、WebGL 字符串覆盖来填平差异 |
| DOM 与原生 API | 普通使用时保持原生方法和权限语义；去掉为显示层服务的 main-world API 包装及 DOM 事件劫持 |
| 窗口、视口、DPR、焦点 | 符合实际虚拟显示器与窗口；调整视口可以触发响应式布局，不能顺带改变 OS/UA；真实前后台和窗口焦点行为需验证 |
| 网络传输 | 使用浏览器原生 TLS/HTTP 栈；先沿用同一网络出口，不增加指纹代理或改写 TLS 的网关 |
| profile | 账号、Cookie、站点存储与权限选择持续保存；切换操作者、观看设备、画质或重启不会创建新身份 |

Google 官方说明，软件自动化控制、某些扩展等可能导致登录被拒。[登录帮助](https://support.google.com/accounts/answer/7675428?hl=en)。因此指纹一致性是本期可控制的工程目标；实际登录仍必须在真实网络与账号条件下验收，不能预先承诺每个账号、每个网站都接受 Agent 操作。

## 3. 已确认的现状

| 事实 | 代码落点 | 要解决的问题 |
| --- | --- | --- |
| 使用完整 Chrome for Testing，当前清单版本 `159.0.8154.6` | `publish/exttool/ext-core/ext-manifest.json:10` | 不是 CEF/WebView；不能直接归因于嵌入式浏览器限制或缺少完整浏览器 |
| 固定 `--headless=new` 和 `--remote-debugging-port=0` | `src/frame/opendan/src/browser/process.rs:280` | 常驻浏览器需要新的启动基线；只删一项参数不足以判定问题已解决 |
| 启动时手工拼桌面 Chrome UA | `process.rs:266`、`:354` | 取消身份覆盖，并检查页面、请求头与 UA-CH 是否恢复原生行为 |
| 手机模式拼 Android 10/K 及 UA-CH | `src/frame/opendan/src/browser/screen.rs:114`；Desktop `screen/viewport.ts` | 拆开窄屏布局、触摸输入和平台身份模拟 |
| 所有页面注入权限包装与屏幕辅助脚本 | `src/frame/opendan/src/browser/requests.rs:378`、`screen_permissions.js`、`screen_page.js` | 去掉对原生 API、下拉菜单和页面事件的干预，同时补齐原生界面的显示与操作 |
| 每个页面常驻 CDP attach，启用 Runtime、焦点模拟等 | `src/frame/opendan/src/browser/mod.rs:942` | 人工浏览的环境不能只靠没有 Agent 租约来判断 |
| 接手主要是作废租约并设置 `user_hold` | `groups.rs:444`、`mod.rs:1174` | 没有停止全部连接、注入、后台采集与重排；需要核对实际活动 |
| 用户输入和画面依赖 CDP | `input.rs`、`screen.rs` | 要独立关闭调试连接，必须先提供可用的画面与输入路径 |
| 权限统一拒绝 | `requests.rs:420`；协议 §3 的权限说明 | 包括 storage-access 等与站点会话相关的权限；需要恢复可解释的原生权限流程，具体登录影响待测 |
| profile 已持久保存 | `process.rs:153`、`:283` | 复用现有目录；无需另建 Cookie 导入系统 |

2026-11-16 本机诊断：使用已安装的 Chrome for Testing `159.0.8154.6`、两个独立临时 profile、本地 HTTP 页面和 raw CDP；未访问 Google、未使用真实账号。两组同为 headless，第二组增加现有代码生成的桌面 UA：

| 观测 | 原生 headless UA | 加当前桌面 UA 覆盖 |
| --- | --- | --- |
| `navigator.userAgent` 中的产品 | `HeadlessChrome/159.0.0.0` | `Chrome/159.0.0.0` |
| `navigator.webdriver` | `true` | `true` |
| UA-CH `architecture` / `bitness` | `x86` / `64` | 空 / 空 |
| UA-CH `fullVersionList` | 包含浏览器版本 | `[]` |

该实验说明 UA 覆盖改变了元数据而未消除自动化状态；它不是无 CDP 的人工基线，也没有证明 Google 拒绝的唯一原因。

更正（同日 BC1/BC2 复测，§9）：用 `diag.mjs` 在页面自报的方式下复测，`--headless=new` 只带本服务的启动参数时 `navigator.webdriver` 是 `false`（条件 H、E），上表的 `true` 没能复现，可能来自当时的连接方式（`--remote-debugging-pipe` 或 Playwright 的 `--enable-automation` 会打开它）。无头本身留下的痕迹是 `HeadlessChrome` UA、`(hover: hover)` 与 `(pointer: fine)` 为假、默认 800×600 的屏幕。

参考项目已核对的实现：headed Chromium、Xvfb、完整 profile 持久挂载、noVNC 人工入口；默认 `trusted-real-browser` 保留原生身份，关闭 JS/UA/时区/TLS 伪装。默认安装 Debian Chromium，扩展仍使用 CDP；公开验证主要是 Reddit，未找到 Google 登录成功记录。[启动代码](https://github.com/energypantry/agent-browser-runtime/blob/17c0d6335900150fa52170654fe7600465ef1d8c/runtime/chrome/launch-browser.sh)、[扩展代码](https://github.com/energypantry/agent-browser-runtime/blob/17c0d6335900150fa52170654fe7600465ef1d8c/extension/background.js)、[README](https://github.com/energypantry/agent-browser-runtime/blob/17c0d6335900150fa52170654fe7600465ef1d8c/README.md)。这些固定链接用于记录研究证据，不是新增 Git 依赖。

## 4. 方案与实施顺序

推荐方向：常驻浏览器采用 headed 模式；显示设备由运行环境提供；用户看到并操作同一个真实浏览器。Agent 在获得控制权时连接，交回后停止操作。普通浏览默认保持原生身份，不设置面向特定网站的伪装规则。

Linux 无显示器节点先验证虚拟显示方案，候选是 Xvfb；完整桌面的传输候选是 x11vnc/noVNC。组件选择由下面的 PoC 收口，新增依赖按 §8 处理。macOS/Windows 使用各自的显示会话与捕获方式，不套用 Xvfb。

屏幕实现须满足两项能力：原生浏览器弹窗可见、可操作；人工使用可独立于 CDP。如果沿用 CDP 页流做不到，就采用桌面级画面和输入，复用 BuckyOS 已有鉴权与会话入口。不能通过恢复页面注入来掩盖缺失的原生 UI。

### 4.1 先做可归因的对照

固定节点、网络出口、浏览器版本、语言/时区、显示配置和扩展清单。A–D 使用同一二进制。每种条件使用独立测试 profile，记录新建/已使用状态；不同时打开同一 profile，不清理用户正在使用的 profile。对照顺序和登录尝试次数需要记录，避免把账号风控或前一次尝试造成的变化当成代码效果。

| 编号 | 条件 | 用途 |
| --- | --- | --- |
| A | headed、无调试启动参数、无 CDP/扩展控制连接、原生配置、人工操作 | 普通浏览器基线 |
| B | A 加一个明确分配的非零回环调试端口，保持未连接 | 区分调试启动参数与连接本身；端口分配处理冲突与重试 |
| C | B 加服务端 CDP 连接；依次测试仅连接、target attach、启用 Page/Runtime/采集等 | 找出连接或具体域带来的差异 |
| D | C 逐项加入现有 UA 覆盖、手机模拟、页面注入、焦点模拟、屏幕输入、Playwright 连接 | 确认哪些现有行为需要移除；每次仅增加一组变量 |
| E | 当前生产方式 headless + 全部现有行为 | 与用户报障环境对应；不能用 E 对 A 一次性证明某个单项原因 |
| F | Chrome Stable 重复 A，必要时重复 B/C | 检查发行版或版本因素；版本不能对齐时明确记录，不能声称只改变发行版 |

A/B 的页面信息由测试页面自行展示/上传到本地测试服务，画面由桌面通道观测；不能为了截图、读取 navigator 或收日志而连接 CDP。先在本地诊断页走全矩阵，再对少量必要条件做用户手动 Google 登录。网页登录截图只保留脱敏结果，诊断不保存密码、Cookie 值、验证码、token 或登录过程的 HAR。

每项条件从新的浏览器进程开始；连接过再 detach 的进程不能重新标为 A。记录控制库实际发出的初始化命令，自动启用域、修改配置的连接不能写成“仅建立 WebSocket”。分别对比由相同初始化条件建立的未登录 profile，以及同环境、同 OS 身份下已人工登录的测试 profile；后者只验证会话复用，不能替代首次登录。

### 4.2 PoC 的完成条件

- 同节点原生基线能完成 Google 登录，并记录 MFA/挑战情况。基线也失败时，先排查版本、网络、账号与站点状态；不以继续叠加伪装作为结论。
- 推荐运行方式完成首次人工登录、同一浏览器进程内交给 Agent、交回用户，并另外验证重启后继续使用。交接本身不能重启浏览器、刷新页面或丢失未提交表单。
- 如果只有无调试启动参数的 A 能登录，而 B 失败，先验证运行中建立控制连接的可行机制；重启成带调试端口的浏览器不是无损交接，该路线尚不能收口。
- 明确普通使用是否可保留某些 CDP 连接，以及各连接启用的域；没有实测不能写“CDP 不影响登录”。
- 确定显示/输入通道、浏览器发行版、每个平台的运行条件和新增组件，随后更新实现任务的选择结果。

## 5. 实施 TODO

### P0 诊断与方案收口

- [x] **BC1 建立原生环境诊断与差异报告。**（2026-11-16：`test/agent_browser_dv/diag_site.mjs`、`diag.mjs`，§9）
  - 落点：扩展 `test/agent_browser_dv/site.mjs` 与配套 runner，原始报告放测试输出目录。
  - 记录实际 executable/版本、启动参数、OS/架构、显示栈、代理是否启用、页面 UA/UA-CH、请求头、webdriver、语言/时区、屏幕/视口/DPR、WebGL、字体/媒体能力、权限状态和原生 API 包装情况；敏感路径和代理凭据脱敏。
  - 同时覆盖顶层页面、跨源 iframe、新窗口和导航后状态。先用页面自报方式采样，CDP 采样必须标明连接状态。
  - 验证：能重现 §3 的差异；同配置重复采样的正常变动有解释；原生基线不会被采样工具 attach。

- [ ] **BC2 完成 headed 与人工登录 PoC。**（2026-11-16：本地矩阵与资源测量已完成，显示组件 owner 定为 Xvnc；Google 人工登录在用户手里，§9、§10）
  - 落点：独立测试启动脚本、`process.rs` 的启动方案、现有屏幕原型。
  - 执行 §4 对照，先验证同一份 CfT，再决定是否需要 Stable。列出显示组件及桌面传输组件的体积、运行权限、许可证、五平台支持情况，确定依赖落在 ext-core、paios 还是原生平台组件中。
  - 验证：保存对照结果、Google 登录结果、原生弹窗画面以及 CPU/内存/启动时间；结论足以选定一条实现路线。PoC 成功不等于整项完成。

### P1 常驻浏览器与普通使用

- [x] **BC3 实现 headed 常驻进程及显示生命周期。**（Linux；§9）
  - 落点：`src/frame/opendan/src/browser/process.rs`、`mod.rs`、`config.rs`，以及确定后的镜像/工具安装入口。
  - 常驻浏览器移除固定 headless、手工 UA 与仅为 headless 服务的参数；逐项复核 `--no-startup-window`、双虚拟屏 `--screen-info`、CaretBlinking、窗口创建和焦点模拟。显示服务与浏览器同启停、崩溃恢复、空闲回收，多个 Agent 的显示与 profile 隔离。
  - 需要调试端口时显式分配非零回环端口，处理抢占与重试；不再假定总能从 `DevToolsActivePort` 获取端口。保留专用 `user-data-dir`。[Chrome 对远程调试目录的要求](https://developer.chrome.com/blog/remote-debugging-port?hl=en)
  - 验证：无物理显示器的 Linux 节点可启动；多 Agent、进程崩溃、端口冲突、显示服务崩溃和重启不串用 profile，不遗留进程；未满足图形会话条件时给出明确错误，不静默改回 headless。

- [x] **BC4 恢复浏览器原生身份与页面环境。**（`virtual` 下；无头保留原做法，§9）
  - 落点：`process.rs` 的 `user_agent`；`screen.rs` 的 `mobile_agent`/`emulate`；`requests.rs` 的 `instrument`/`refuse_permissions`；两个 `screen_*.js`。
  - 移除桌面/Android UA 与 UA-CH 覆盖；取消为人工屏幕服务的权限 API 包装、原生下拉框劫持和不必要的焦点/设备模拟。屏幕需要的信息优先从浏览器/桌面接口取得。
  - 权限回到真实浏览器的站点选择与能力状态，逐项检查现有全局拒绝清单，尤其 `storage-access`。权限改变必须与 BC6 的可见可操作交互一起交付，不能单独全局放开。
  - 已注入脚本、已设 CDP override 需要明确清理：停止后续注入不等于当前文档恢复；正常升级时重启浏览器并保留 profile。运行中的交接不依赖强制刷新登录表单。
  - 验证：本地诊断页在普通使用、新 tab、OAuth 弹窗、跨站跳转、交接后都保持选定基线；不通过 JS 改写检测值来通过测试。

- [x] **BC5 分离设备适配、浏览器身份与传输质量。**（服务端；真机和弱网未验，§9）
  - 落点：服务端 `screen.rs`；Desktop `screen/viewport.ts`、`screen/quality.ts`、`ScreenView.tsx`、`screen/input.ts` 及 API 类型。
  - 手机端通过真实窗口大小或显示缩放提供窄屏/桌面布局；触摸手势转换为输入操作，不自动声明 Android 平台或修改 UA-CH。
  - 固定会话显示配置，弱网自适应只改变编码质量、帧率和输出图像缩放；不因带宽在两块虚拟屏之间切换而改变网页 DPR。用户主动改显示设置与传输自适应分开处理。
  - 验证：手机/电脑切换、旋转、画质四档与弱网下，身份信息不跳变；坐标、滚动、缩放、中文 IME、选区、组合键和剪贴板输入正确。

- [ ] **BC6 交付原生界面的画面与输入。**（画面、鼠标、键盘、中文、原生下拉框 / 弹框 / 权限提示已做；剪贴板带出、输入法组字预览没做，§9、§10）
  - 落点：`screen.rs`、`input.rs`、`requests.rs`、`transfer.rs`、`http.rs`；Desktop `screen/`、`requests/`、`shell/`；必要时增加选定的桌面通道实现。
  - 显示并操作浏览器窗口、OAuth 弹窗、账号选择、JS 对话框、原生 select/date/color 控件、文件选择及权限弹窗；正确处理焦点、弹窗与当前标签的关联、关闭和返回原页面。
  - 保留本机/Zone 文件上传、下载存到 Zone 的入口；对话框对应实际远程文件系统，不能把用户终端路径误传给服务器。普通浏览时支持查看和修改站点权限；不具备的摄像头/安全密钥等能力如实显示。
  - 独立桌面通道必须沿用 owner 鉴权、控制权互斥与会话映射，只提供当前 Agent 浏览器环境；服务端口不直接暴露到外网。
  - 验证：上述原生界面可实际完成操作，用户接手后无需 Agent 帮忙点击；独立关闭 CDP 时画面与输入仍可用。

- [x] **BC7 将人工接管落实为实际控制生命周期。**（租约代理 + 立即切断；后台重排未纳入，§9、§10）
  - 落点：`groups.rs`、`mod.rs`、`cdp.rs`、`reflow.rs`、`rpc.rs`、`sessions.rs`；`agent-browser.mjs`；Desktop 交接状态。
  - 区分“无人持有租约”“正在交接”“用户独占”“Agent 操作”；复用现有 `controller`/`user_hold` 的产品含义，内部状态与错误按确定后的协议落地。
  - 接手先阻止新 acquire/重排任务，终止或等待在途操作完成，再停止自动重连、采集、截图和选定方案要求解除的连接。排查内部 `_reflow` 标签与后台任务，不能只清外部租约；处理 Agent 直接持有的 CDP 连接，不能只断开服务自身连接。
  - 设置有界的交接超时，数值在 BC2 实测后写进协议；挂起的导航、弹框或 Playwright 等待不能让接手无限等待。超时优先取消/结束自动化执行者，保留浏览器和页面；确认旧执行者不能继续输入后再开放用户控制。释放失败时如实报告、保留画面与恢复入口，不宣称已完成接手。
  - 若实现不能关闭全部连接，状态必须如实报告，并选择能满足人工基线的控制入口；不能仅凭 lease 清空宣称“已脱离自动化”。这属于兼容性目标，不声称隔离同 runtime 中恶意绕过工具的进程。
  - 交还时重新发现当前 tabs/popups、恢复必要连接与租约；继续使用同一浏览器和 profile，不导航回登录页，不刷新用户尚未提交的表单。
  - 验证：用户登录期间后台任务不再操作，旧租约/连接不能继续发输入；在途命令永不返回时能在限定时间内取消并交接；连续接手/交还、连接失败、取消、App 断线重连不出现双控制，释放失败状态与恢复入口可用。

- [ ] **BC8 保留 profile 并处理升级。**（目录不变，无头时代的 profile 在有窗口时照常用；Chrome 版本升级未做演练）
  - 落点：`process.rs`、`mod.rs`、profile 清理入口、ext-core 在线项与版本信息。
  - 沿用现有持久目录，保留 Cookie、localStorage、IndexedDB、站点权限和必要的密钥存储；升级不因启动参数变化而自动清空 profile。
  - 若更换发行版/版本，先用独立测试 profile 验证迁移、加密存储与文件属主。备份必须在浏览器停止后进行；不能把较新版本打开过的 profile 直接交给旧版本作为回滚。日常 headless 测试和 PDF 渲染使用独立目录。
  - 验证：进程/Agent 容器/节点重启、浏览器升级后登录态仍可用；已有用户的 profile 保留；用户主动清除后确实退出登录。

### P2 打包、平台与回归

- [ ] **BC9 完成发行与五平台适配。**（Linux x86_64 镜像已改；aarch64 未构建，macOS / Windows 仍无头，§10）
  - 落点：`publish/exttool/ext-core/{build.sh,ext-manifest.json,official-sources.json,conda-env.yml}`、在线项 overlay、`build_aios`、相关 CI、原生启动环境。
  - 先交付 Linux x86_64 的完整链路；Linux aarch64、macOS 两种架构、Windows x86_64 分别验证显示会话、浏览器、传输、安装与生命周期。记录 macOS LaunchAgent/权限条件、Windows 交互会话/锁屏限制，避免把 service 会话当普通桌面。
  - 继续使用现有在线下载机制；如确需单独的用户浏览器与测试浏览器，说明体积、更新、路径和 profile 分工，再调整清单及版本展示。新组件依赖与许可随包检查，Git 源依赖按仓库规则显式用 `main`。
  - 验证：各平台通过本期矩阵；缺失平台结果标为未验证，不以 Linux 通过代表五平台完成。

- [x] **BC10 同步协议、界面与 Agent 手册。**（协议、容器文档、`browser` 手册、Desktop 类型与文案；§9）
  - 落点：`doc/opendan/protocol/Agent Browser Protocol.md`、`src/frame/opendan/README.md`、Desktop `api/{types,backend,channel}.ts` 与 mock、相关 SDK 消费方、`doc/app_service/paios容器需求.md`、原 TODO 的当前说明。
  - 协议同步启动方式、tab/window 关系、画面输入、权限、DPR、实际交接状态、版本与不可用原因；字段变更同批更新服务端和消费方，不保留两套互相矛盾的约定。
  - 检查 `browser`（含 reflow 引用）、`ui_core`、`buckyos_app_dev` 手册。用户界面使用“接手/交给 Agent”等操作语言，技术诊断留在排查入口。
  - 验证：手册示例连接实际浏览器；mock、类型检查和真实 Zone 的状态含义一致。

- [ ] **BC11 完成真实使用验收和性能回归。**（不调模型的 DV、交接、性能已测；Google 登录、第三方 OAuth、真机手机、带模型的 Agent 步骤未跑）
  - 落点：`test/agent_browser_dv/`、Rust browser tests、ext-core browser tests、Desktop e2e/DV。
  - 本地 fixture 覆盖诊断和交接；第三方账号登录由用户手动完成，不写密码自动化脚本。每个真实站点记录版本、环境、时间、首次/已有 profile、MFA、失败步骤和最终结果。
  - 跑 §6 全部必验项，并记录显示服务新增内存、CPU、首帧时间、静态/动画带宽、弱网输入反馈和空闲回收。以原 TODO §12 的实测作历史参考，性能阈值在 BC2 根据同机对照收口。

依赖顺序：BC1 → BC2 → BC3；BC4/BC5/BC6 协调交付，BC7 依赖可独立工作的人工通道，BC8 贯穿整个改造；BC9/BC10 随实现同步，BC11 完成后才关闭本 TODO。

## 6. 验收清单

| # | 必验结果 |
| --- | --- |
| A1 原生身份 | 默认人工使用通过 §2 的原生对照；新增页面、跨源 iframe、弹窗和跳转无残留 UA/UA-CH 或 API 包装；差异逐项有来源 |
| A2 Google 首次登录 | 在用户此前未登录的独立 profile，从 BuckyOS App 手动登录 Google，能完成账号选择、密码和实际出现的 MFA/验证，进入登录后的 Google 页面；无本次“浏览器不安全”阻断 |
| A3 登录态复用 | 用户交给 Agent 后，Agent 能读取约定的非敏感登录后页面；交还后用户继续使用；浏览器、Agent 容器和节点分别重启后再次检查 |
| A4 第三方 Google 登录 | 在一个已选定的第三方站点完成实际 Google OAuth popup/redirect，返回原站且站点会话生效；访问 accounts.google.com 成功不能替代此项 |
| A5 普通操作 | 导航/前进后退/刷新/新标签、原生控件、弹窗、中文输入、复制粘贴、上传下载正常；弹窗关闭后焦点与当前页正确 |
| A6 权限 | 权限弹窗可处理、选择可保存和撤销；storage-access 等行为与基线一致；无设备时返回真实结果；不靠全局预授权通过 |
| A7 手机与弱网 | 手机窄屏、桌面布局、旋转、多设备观看切换可用；自动降画质不改变浏览器身份/DPR，输入仍准确 |
| A8 交接 | 在加载、弹窗和输入中接手/交还都只有一方实际操作；后台重排和自动重连不干扰登录；切换不丢未提交表单 |
| A9 既有能力 | Agent 工具、重排、项目 Playwright 测试、Workflow PDF/图片渲染继续可用；profile 和系统渲染隔离不退化 |
| A10 故障与平台 | 崩溃恢复、空闲回收、缺少图形会话、多 Agent、升级与清除 profile 各自验证；五个平台结果分别登记 |
| A11 重新登录 | 用户明确退出账号后，能再次完成人工登录；不能只凭已有 Cookie 长期可用判定登录问题解决 |

Google 验收失败要分别记录“原生基线也失败”“BuckyOS 特有失败”“账号/MFA/服务限制导致未完成”。前两项必须有对照证据，未完成不能写通过，也不能直接关闭需求。第三方检测页结果只作为诊断附件。

另建能力列表登记媒体播放/音频输出、DRM、摄像头/麦克风转发、passkey/安全密钥、外部协议等普通浏览器场景：逐项注明已验证、受远程设备限制、或需要后续任务。本期不把这些硬件和媒体通道默认当成已具备，也不借“指纹一致”承诺它们可用。

## 7. 验证入口

以下是现有入口；实现改变所需环境后，同批修正命令与前置条件。真实 Google 登录按 §6 手工验收。

仓库根目录：

```bash
cargo test --manifest-path src/Cargo.toml -p opendan --lib browser:: -- --test-threads=1
OPENDAN_CHROMIUM=/opt/buckyos/tools/bin/chromium cargo test --manifest-path src/Cargo.toml -p opendan --lib browser::tests::real -- --ignored --test-threads=1
OPENDAN_CHROMIUM=/opt/buckyos/tools/bin/chromium cargo test --manifest-path src/Cargo.toml -p opendan --lib browser::screen_tests::real -- --ignored --test-threads=1
node --test publish/exttool/ext-core/browser/test/agent-browser.test.mjs publish/exttool/ext-core/browser/test/page-render.test.mjs
uv run src/check.py
AGENT_BROWSER_DV=1 uv run test/agent_browser_dv/main.py
```

Node 测试需要可解析的 `playwright-core` 与对应浏览器/运行库，使用 ext-core 配套环境或现有开发依赖；测试 helper 当前固定 headless，BC3 同步调整常驻浏览器测试入口，`page-render` 测试保持独立。真实 Zone DV 前检查已激活、Agent 可用及 `test/agent_browser_dv/README.md` 的前置条件。

Desktop 目录 `src/frame/desktop/`：

```bash
pnpm check
pnpm exec playwright test tests/e2e/pages/agent-browser*.spec.ts
pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/agent-browser*.spec.ts
```

DV 按两个 spec 文件头配置 Zone、测试 Agent、第二个用户和测试站点地址。现有 reflow DV 会调用 `browser.clear_profile`，必须使用专门的测试 Agent，不能指向用户日常浏览或准备保留登录态的验收 profile。真实账号登录验收单独执行，并关闭自动截图/trace/HAR 对敏感登录过程的采集。

普通用户界面测试、真实 Zone DV 与第三方账号验收分别记录。只改本文时检查链接、路径、任务依赖和 `git diff --check`，不需要运行浏览器或构建系统。

## 8. 与原方案的关系及待收口选择

本轮已经明确的是用户普通浏览目标与原生指纹方向。下面列出为实现目标建议调整的技术选择，不能把候选组件写成已经安装或已经得到 Google 验证。

| 原约定 | 本期处理 |
| --- | --- |
| D12 五平台均 headless | 常驻浏览器改为 headed 方向，按平台落实显示环境；测试/系统渲染继续使用各自模式 |
| D7 CDP 页流和输入，RFB 留以后 | 以原生 UI 和人工独立操作为标准，在 BC2 选择完整画面路径；需要桌面通道时本期补齐 |
| D19 跟随观看设备、D37 自动调画质 | 保留布局和清晰度目标，取消“终端变化就切换浏览器身份”，将传输质量与网页 DPR 分开 |
| D26 CfT 与 Playwright 对齐 | 先保留同一二进制做对照；如发行版确实影响目标，BC2 给出证据和在线项变更方案 |
| D16 Playwright 经 CDP 直连 | Agent 操作继续优先复用；人工阶段的连接生命周期必须能够满足 BC7。不预设改成扩展就会解决 |
| D5 私有 runtime、D35 每 Agent 一个组与浏览器会话 | 继续沿用；不移植参考项目的共享浏览器、多 Agent tab-group broker |
| 权限全部拒绝、注入脚本补 UI | 改为原生权限交互与真实能力，跟随 BC4/BC6 同批交付 |

BC2 需要收口三件事：显示/传输组件及所属安装层；CfT 是否满足真实登录；人工阶段可保留哪些控制连接。结论填写到 §9，并更新任务中的候选方案。

收口结果（2026-11-16）：

- 显示 / 传输组件：owner 定 **TigerVNC `Xvnc`**，进 paios/aios 工作镜像（不是 ext-core，不是扩展工具）。opendan 自己做 RFB 客户端，把显示器的画面转成原有屏幕协议的 JPEG 帧、把输入转成显示器上的鼠标键盘，Desktop App 的协议基本不变；不引入 noVNC、x11rb。
- 控制连接：本地矩阵里，有窗口的浏览器加调试端口（B）、服务端连接并 attach、打开 Page/Runtime（C）、焦点模拟、截屏流、Playwright 连接，页面都看不出差别（我们采集的全部项目，含 Runtime 的控制台序列化探测）。所以服务端的调试连接在人工使用时保留，用来列标签页、导航、上传、管窗口；不用来出帧和输入。Agent 的连接走租约代理，交接时切断（BC7）。
- CfT 是否满足真实登录：等用户在 Xvnc 上的人工 Google 登录结果（§10）。

[AGENTS.md](../AGENTS.md) 要求“引入新的依赖项或通用组件时，必须和用户确认”。本次只写 TODO；实施时先交出具体组件、安装位置、平台支持、许可和体积，再对尚未获授权的新增依赖完成确认。已有依赖内的诊断和代码调查不因此停下。

## 9. 实施记录

| 日期 | 任务 | 改动与验证 | 未验证/待处理 |
| --- | --- | --- | --- |
| 2026-11-16 | 立项 | 核对当前代码、参考项目及本机 headless UA 对照，结果见 §3；建立后续任务 | 未实施；headed/Google 登录及五平台矩阵均未验证 |
| 2026-11-16 | BC1 | `diag_site.mjs`（挂在 `site.mjs` 的 `/diag`）：页面自报 UA/UA-CH、`navigator`、屏幕窗口、媒体查询、WebGL、字体、编解码、EME、权限、API 是否原生、注入的全局量、控制台序列化探测，覆盖顶层、跨源 iframe、点击打开的弹窗、导航后；请求头带 `Accept-CH` 取高熵提示。`diag.mjs` 跑 §4.1 的 A、A2、B、C（逐步）、D-ua/mobile/inject/focus/screencast/permissions/playwright、H、E，每个条件新进程新 profile，出 `report.md`。A/B 不连 DevTools，弹窗用 X 服务器的点击（按页面自报的窗口位置算坐标） | 只在一次性容器里跑过（Ubuntu 24.04 + Xvfb、Debian 13 + Xvnc，挂只读的节点工具目录） |
| 2026-11-16 | BC2 本地矩阵 | CfT 159 有窗口（Xvfb）：A 与 A2 只差网络估计的 `downlink`；B、C 全部阶段、D-focus、D-screencast、D-playwright 与 A 无差别（`webdriver` 都是 `false`，控制台探测都没触发；只有 D-playwright 期间打开的弹窗少一个 `chrome.app`）。现有做法的差别：UA 覆盖让 UA-CH 的 `fullVersionList`/`architecture`/`bitness`/`formFactors` 变空（JS 与请求头都是）；注入让 7 个 API 不再是原生方法、`window` 上多一个 Symbol；全局拒绝改了 12 个权限状态，`storage-access` 由 `granted` 变 `prompt`；手机模拟改 UA/UA-CH 为 Android；无头（H）是 `HeadlessChrome` UA、无悬停无精细指针、800×600 屏；E 是这些全部加上。同一二进制的有窗口浏览器 WebGL 被拉黑（没有 GPU），与无头一致，如实呈现。资源（同机）：有窗口 PSS 约 470 MiB / 15 进程，Xvfb 约 15 MiB；无头约 340–380 MiB | Chrome Stable（F）没测：本机和镜像里都没有，Linux arm64 也没有 Stable |
| 2026-11-16 | BC2 显示组件 | Ubuntu 24.04 装 `tigervnc-standalone-server` 会经 `libGL` 拉进 Mesa/LLVM、经 `vncserver` 拉进 Perl，共约 +290 MB；`build_aios` 用 `dpkg-deb` 造的占位包 `buckyos-xvnc-shim`（`Provides: libglx-mesa0, libfile-readbackwards-perl (= 1.06), perl`）和它一起装，镜像 231 → 249 MB，`apt-get check` 一致，`Xvnc` 不用 GLX 照常运行。Xvnc 下的基线与 Xvfb 下只差两项：`(hover: hover)`、`(pointer: fine)` 变假（TigerVNC 的指针设备没有 MOUSE 类型，Chromium 不把它算成鼠标）——见 §10 | 许可：TigerVNC GPL-2，独立进程，不链接 |
| 2026-11-16 | BC3 | `browser/display.rs`：`[loader.browser] display = auto|virtual|headless`（Linux 默认 `virtual`），`device_scale_factor`（默认 2）。浏览器启动前起 `Xvnc :<20 起空闲号>`（`-rfbport -1 -nolisten tcp`，只开 `/tmp/opendan-display-<n>.rfb`，0600、无密码，`-AcceptSetDesktopSize`，dpi 按倍率），浏览器停、崩溃时一起停、一起重启；没有 `Xvnc` 时报新的 `unavailable_code: no_display`，不退回无头。`process.rs`：有窗口时不带 `--headless`/`--user-agent`/`--screen-info`，加 `--window-position=0,0 --window-size=<显示器 CSS 尺寸> --force-device-scale-factor=<倍率>`、`DISPLAY`；调试端口由服务端在回环上找空闲端口给出，从标准错误的 `DevTools listening on ws://…` 读地址（有窗口的浏览器给了端口就不写 `DevToolsActivePort`），被抢占时换端口重启最多 4 次。保留 `--no-startup-window`、`CaretBlinking`（页面观察不到，省掉光标闪烁的整帧） | 多 Agent 同节点：每个 Agent 一个容器、一块显示器，未在同一容器里并发测；空闲回收沿用 `idle_stop_secs`（显示器随浏览器停） |
| 2026-11-16 | BC4 | `virtual` 下：不覆盖 UA、不做手机/设备模拟、不开焦点模拟、不注入主世界的权限包装、不拦下拉框和对应按键、不全局拒绝权限、不把 JS 弹框转成屏幕的请求（`attend`、`raise_dialog` 都跳过）。隔离环境里只留被动的焦点监听（给 App 弹软键盘，`rect` 换成显示器坐标），文件选择照旧拦截给 App 上传。无头路径原样保留（macOS/Windows） | 正常升级时浏览器会重启，旧文档里的注入随之消失；没做运行中撤销注入 |
| 2026-11-16 | BC5 | 显示器 CSS 尺寸 = 最近设置视口的连接的 `width`、`height`（宽度至少 500：Chromium 的窗口不比这更窄）；`mobile` 不起作用，`state.viewport.mobile` 恒为 `false`，App 因此把触摸换成鼠标和滚轮；页面 `devicePixelRatio` 固定为 `device_scale_factor`，连接的 `pixel_ratio`、画质、帧率只决定帧怎么缩放编码。服务端也收 `touch`：不动是点击，移动超过 8 px 是反向滚轮 | 真机手机、旋转、弱网未验；IME 组字预览不再画进页面 |
| 2026-11-16 | BC6 | `browser/rfb.rs`：RFB 3.8 客户端（None 安全类型、Raw/CopyRect/DesktopSize/ExtendedDesktopSize/LastRect、没人看时暂停更新请求）。`display.rs` 把帧缓冲按要的像素比做面积平均缩小、用工作区已有的 `image` crate 编 JPEG（新增的是 opendan 对它的引用），出帧上限取各连接帧率的最大值；输入：鼠标按键位、滚轮按 53 px 一格累计、键用 X keysym（命名键表 + 字符的 Latin-1/Unicode keysym，修饰键按消息同步）、`text` 逐字符打（Xvnc 给 Unicode keysym 临时分配键位）。`screen.rs` 的连接在 `virtual` 下取显示器的帧、把输入做在显示器上 | 剪贴板只解析了 Latin-1 的 ServerCutText，没下发给 App；没测“关掉服务端调试连接时画面与输入”（结构上不依赖它） |
| 2026-11-16 | BC7 | `browser/proxy.rs` + `http.rs` 的 `…/browser/cdp/<lease_id>/{json/version,devtools/browser}`：`browser.acquire` 的 `cdp_url` 指向本服务（有 `OPENDAN_BROWSER_RPC` 时），只收回环地址、租约有效的连接，消息原样转发；租约结束（`release`、过期、`take_over`、`request_handover`）立刻关掉它的连接（`cut_agent`），失效租约再连回 410 `lease_lost`。浏览器窗口：标签页窗口铺满显示器（弹出窗口保持大小、挪进显示器），组的当前标签页变化或标签页集合变化时 `Target.activateTarget` 把它放到最前（后台重排的窗口压在下面） | 后台重排（`_reflow` 组，服务自己的直连）在接手时不中止，见 §10 |
| 2026-11-16 | BC10 | 协议 §1、§2.1–2.5、§3（新增 §3.6）、§6；`doc/app_service/paios容器需求.md` §6.3/§6.4；`browser` 手册（有窗口、不改身份、权限提示要交给用户、交接时连接被切断）；Desktop `api/types.ts`（`display`、`device_scale_factor`、`no_display`）、`AgentBrowserShell` 文案（中英）、mock（`headless`）；`test/agent_browser_dv/README.md`；`main.py` 的屏幕检查改为在自己开的标签页里用键盘登录、只关自己的标签页 | `src/frame/opendan/README.md` 没提浏览器显示方式，未改 |
| 2026-11-16 | 无头能调到多像（E-fix） | owner 问“无头只影响 HTTP 头、能否直接调”。实测：无头本身只让 `User-Agent` 头带 `HeadlessChrome`（`sec-ch-ua` 不变）；`--user-agent` 只能改这一个头，并让 UA-CH 高熵头和 JS 值变空。E-fix = 无头 + 经 DevTools 给每个页面（自动附着、停在启动处直到设好）设 UA 和完整 UA-CH 元数据 + 单屏 `--screen-info` + `--blink-settings` 声明有悬停的精细指针：顶层页、弹窗、导航后与 A 只差窗口内外尺寸（无头没有真工具栏）；但跨源 iframe 是独立 target，没被附着到，`navigator.userAgent` 仍是 `HeadlessChrome`。要做全得覆盖 OOPIF、各类 worker，品牌和 GREASE 值随 Chrome 版本硬编码维护（违背 §2）；Google 能否接受没测 | 可作为 macOS / Windows 的过渡方案之一（Q8） |
| 2026-11-16 | Google 登录 | owner 在 Linux 上手动登录 Google 成功（Q1）| macOS / Windows 未做（Q8） |
| 2026-11-16 | 无头能否过 Google | owner 授权用他的账号自动测到“输入邮箱 → 下一步”（之后加一次错误密码）。在一次性容器里（同机、同出口、同一份 CfT 159）：旧的生产无头配置（无头 + `--user-agent` + 主世界权限包装 + 全局拒绝 + 焦点模拟）邮箱后进了密码页，错误密码回 “Wrong password”（浏览器被接受）；加上手机伪装（Android UA/UA-CH）邮箱后也进了密码页。随后 owner 把 `jarvis` 临时切回 `display = "headless"`（完整的旧链路：截屏流、隔离环境脚本、手机伪装都在），先清空 profile，用 App 手动以正确密码登录也成功。和基线 `6c0e2eef8` 比，无头路径的代码只差调试端口（0 → 服务端分配）、Agent 经代理连接、`nav` 等标签页附着，解释不了当初的拒绝 | 结论不干净：今天所有无头成功都发生在同一账号、同一 IP、同一机器与二进制（同一指纹）有窗口登录成功之后；清空 profile 去不掉服务端对账号 + IP + 指纹的记忆。当初的拒绝可能被这份“信任”掩盖，也可能是当时的情形（手机伪装、第三方网站的“用 Google 登录”、风控状态）。要干净的对照需要一个没在这台机器 / 这个 IP 上登录过的账号。`jarvis` 测完已改回 `virtual` |
| 2026-11-16 | 验证 | `cargo test -p opendan`（全部）通过：lib 101、loader 集成 30（loader 的假浏览器是无头的，测试的 `agent.toml` 明确写 `display = "headless"`；`acquire` 的 `cdp_url` 改为断言代理地址、代理的 `json/version`、租约结束后 410）；其中 `browser::` 70 通过；无头真浏览器 13 条全部通过（`OPENDAN_CHROMIUM` 指本机 CfT 159）；新增有窗口真浏览器 3 条（`browser::screen_tests::headed`，在 Debian 13 + Xvnc 的一次性容器里跑本机构建的测试二进制）全部通过：身份（无 Headless/Android、`webdriver` 假、UA-CH 完整、DPR 2、权限 `prompt`、API 原生、无注入 Symbol）、帧尺寸与缩放、视口改窗口、点击聚焦、打中文、退格、Ctrl+A、Shift、原生下拉框用方向键选、原生 `confirm` 回车确认、滚轮、没有 select/dialog/permission 请求、杀掉 Xvnc 后浏览器和显示器连同标签页回来。Desktop `pnpm check` 通过。`build_aios --local-test` 后 `start.py --skip-update` 部署本机 Zone：`AGENT_BROWSER_DV=1 main.py`（Agent `jarvis`）三项通过；租约代理在真实容器里：Playwright 经代理 17 ms 连上，`take_over` 后在途命令立刻失败，旧租约再连被拒，持有期间 `acquire` 回 `user_in_control`。资源（本机 Zone，滚动字幕页，10 秒）：桌面 1280×800 / 倍率 2：opendan 0.20 核、浏览器 0.06 核、Xvnc 0.01 核，PSS 分别 59 / 620 / 76 MiB；手机视口（显示器 500×915）：0.12 / 0.03 / 0.01 核，50 / 565 / 41 MiB | DV 的第一次跑法在 `jarvis` 恢复出来的知乎登录页上按了键（`nav` 撞上标签页还没 attach 的竞态：已修，DV 改为自己开标签页）：没有字段被填写、没有提交，回车点开了“下载 App”链接；那个标签页被 `close-tabs` 关掉后已经重新打开。交接检查往 `jarvis` 的浏览器会话写了 `take_over`/`hand_back` 记录 |

## 10. 待 owner 确认

| # | 问题 | 现状 / 建议 |
| --- | --- | --- |
| Q1 | ~~Google 登录（A2、BC2 第一条）：在 Xvnc 上的原生基线能否登录~~ | 已答（2026-11-16）：owner 在 Linux 上手动登录 Google 成功 |
| Q2 | Xvnc 的指针不被 Chromium 当成鼠标：`(hover: hover)`、`(pointer: fine)` 为假，和无头一样 | 三个办法：A 维持（同一显示环境下原生如此，Q1 的结果能说明 Google 在不在意）；B 给浏览器加 `--blink-settings=primaryPointerType=4,availablePointerTypes=4,primaryHoverType=2,availableHoverTypes=2`，按显示器实际是鼠标操作来声明（不加新依赖，但这是一个显式设定）；C 改用 Xvfb（带 MOUSE 类型的设备）+ x11vnc（`-unixsock`），opendan 的 RFB 客户端不用改，镜像多一个组件。建议先看 Q1 |
| Q3 | 浏览器发行版（F） | CfT 的启动窗口有“只用于自动化测试”的信息条（服务端经 CDP 开的窗口没有），UA-CH 品牌是 `Chromium` 不是 `Google Chrome`。Chrome Stable 没有 Linux arm64、再分发另议。建议 Q1 通过就维持 CfT |
| Q4 | 时区、语言 | 容器里是 UTC、`C.UTF-8`，浏览器照实报告；真人的浏览器一般是本地时区。要和节点一致需要 node_daemon 把宿主时区传进容器，没做 |
| Q5 | App 的地址栏与浏览器自己的工具栏同时出现 | 现在两者都在（App 的地址栏、标签列表经 CDP 操作）。手机上浏览器工具栏约占 90 CSS px。11-16 owner 定：App 的地址栏保留，桌面上并进顶部一行（Agent 名字后面是地址栏，标签页按钮在 ⋮ 左边），手机仍是两行，已实施（Desktop `shell/AgentBrowserShell.tsx`）。屏幕里浏览器自己的标签栏、地址栏、书签栏是否收起，未定 |
| Q6 | 剪贴板、输入法 | 远端复制的文字带回手机（需要 TigerVNC 扩展剪贴板，UTF-8 走 zlib）没做；输入法组字中间态不再画进页面（手机输入法自己显示），上屏后才到页面 |
| Q7 | 接手时的后台重排 | 重排在自己的 `_reflow` 标签页里跑，用服务端自己的连接，不碰用户的标签页和输入；接手不中止它（有超时上限）。要不要接手时一并中止 |
| Q8 | ~~macOS / Windows~~ | 已定（2026-11-16）：本版继续无头（`display = auto` 的取值，身份补偿和注入照旧）。Windows、macOS 上没有容器环境，这是 BuckyOS Desktop Lite 的核心约束，Xvnc 方案只用于 Linux。发布后按 §11 收集反馈再定；可选的方向（无头 + 完整身份覆盖、真窗口 + 系统截屏输入）记在 §9 的 E-fix 一行和对话记录里 |

## 11. 发布与反馈

发布顺序：先发新的 paios/aios 镜像（带 Xvnc），再发新的 opendan。节点上还是旧镜像时，Linux 上的浏览器报 `no_display`（不会退回无头）。

ext-core 升到 1.3.2（owner 同意，2026-11-16）：`agent-browser` 在连接断开或连不上而失败时先 `renew` 一次，租约已结束就按 `EXIT_LEASE_LOST`（5）退出，不再按普通脚本失败（1）；`json/list` 读不出列表时不报错。opendan 的租约代理同时转发 `json/list`，连不上时的“哪些标签页卡在弹框上”提示照旧有效。测试的假服务改为每个租约一个可切断的 TCP 转发（和代理一样在租约结束时切断），新增“接手时在途命令立刻以 5 退出”。发布顺序：aios 镜像、ext-core 1.3.2、opendan 一起发；旧的 ext-core 1.3.1 配新 opendan 也能用，只是接手时 Agent 多重试一次。

反馈要收集的（每个登录被拒或成功的案例）：

| 项 | 为什么 |
| --- | --- |
| 节点平台、`browser.status` 的 `display`、`version.browser` | 有窗口（Linux）和无头（Windows / macOS）分开统计 |
| 用户看的设备（手机 / 电脑），App 的画质档 | 无头时手机会被模拟成 Android |
| 站点、入口（直接登录 / 第三方网站的“用 Google 登录”）、被拒在哪一步（账号之后 / 密码之后 / 二次验证） | 区分站点规则和浏览器问题 |
| 这个账号以前是否在这台节点、这个出口 IP 上登录成功过；profile 是新的还是用过的 | 今天的测试表明，曾经成功过会掩盖问题 |
| 节点的出口网络（家宽、云主机、代理）和时区 | 云主机 IP 和 UTC 时区都可能加分风险 |

