# Agent 浏览器内存占用优化 TODO

日期：2026-11-23 · 源码基线 `6c80c119c`

状态：待实施。owner 已同意按四个方向立项：空闲回收、标签页休眠、屏幕倍率、内存压力处理。§8 的 4 个问题有默认做法，M1 不依赖它们，可以先做。

前置：[Agent 浏览器实施 TODO](agent-runtime-browser-todo.md)、[兼容性 TODO](agent-runtime-browser-compatibility-todo.md)。接口以 [Agent Browser Protocol](<../doc/opendan/protocol/Agent Browser Protocol.md>) 为准，改接口先改它。

## 1. 目标与范围

BuckyOS 的服务整体不吃内存，15 个服务进程的 RSS 合计约 470 MB。Agent 浏览器打开几个普通网站就会超过这个数。

目标：

1. 没人用浏览器时，它按时停掉，不留内存。
2. 用着的时候，内存取决于正在用的标签页，不取决于一共开过多少标签页。
3. 节点内存紧张时，先收掉没人用的浏览器和标签页。整机不变卡，系统服务不受影响。

不做：

- 不改 Agent 容器外的服务，不改 `page-render`。它在 Workflow 的 Browser 通道里，同时只跑一个（[native.rs:50](../src/kernel/workflow/src/native.rs#L50)），用完就退。
- 不限制渲染进程数，不关站点隔离，不把标签页合并进一个窗口。原因见 §2.4。

低配参照：`demo.buckyos.io` 是 2 vCPU / 3.8 GiB，跑着 10 个 opendan，空闲时可用约 2.9 GiB（[2026-11-13 采样](runtime-probe-container-polling-cpu-todo.md)）。一个 Agent 开 12 个真实网站就能把这 2.9 GiB 用完。

## 2. 实测

### 2.1 方法

2026-11-23 在本机一次性容器里测，没有动正在运行的 Zone。镜像是 `local/aios-test:0.7.0-amd64`，只读挂载 `/opt/buckyos/tools`，Chromium 159.0.8154.6。启动参数照抄 [process.rs](../src/frame/opendan/src/browser/process.rs) 的 `arguments()`，默认屏幕 1280×800。每个标签页用 `Target.createTarget {newWindow: true}` 打开，和 `open_in` 一致。

数值是容器 cgroup `memory.stat` 的 `anon + shmem + kernel`，也就是不可回收的部分，单位 MB。没算进去的：

- Chromium 程序文件的页缓存约 150 MB，可回收，同节点的多个 Agent 共享。
- opendan 自身的内存，以及它的 RFB 帧缓冲和编码。

### 2.2 不同配置

装载完成约 25 秒后的读数。简单页是几行文字；真实网站依次是 Wikipedia、GitHub、NYTimes、Amazon、YouTube、BBC、Reddit、CNN、Hacker News、Apple、Booking、Stack Overflow。

| 标签页数 | 简单页，无头 1 倍 | 简单页，有窗口 2 倍（现状） | 真实网站，有窗口 1 倍 | 真实网站，有窗口 2 倍（现状） |
| --- | --- | --- | --- | --- |
| 1 | 262 | 357 | 381 | 468 |
| 3 | 336 | 518 | 1184 | 1393 |
| 6 | 375 | 679 | 1779 | 2124 |
| 12（`max_tabs`） | 582 | 1118 | 2871 | 3458 |

- 再放 45 秒左右会自己降一些：简单页 12 个降到约 930，真实网站 6 个降到约 1780–1870。比较方案时要在同一时刻读数。
- 真实网站的无头数据没有列：部分网站的标题只剩域名，疑似给无头返回了拦截页，不能直接比。
- 空白时，Xvnc 加有窗口模式比无头多约 45 MB。VNC 本身不是大头。
- 每个标签页的窗口约多占 40 MB（2 倍），分在 Xvnc 和 GPU 进程的窗口缓冲里。12 个真实网站时，2 倍比 1 倍多约 590 MB。

### 2.3 对后台标签页做处理的效果

最后一个标签页留在前面，其余的处理掉，45 秒后读数。

| 处理 | 简单页 12 个 | 真实网站 6 个 | 页面状态 |
| --- | --- | --- | --- |
| 处理前 | 约 1150 | 约 1900–2010 | |
| 最小化窗口 | 928 | 1865 | 保留。Xvnc 上没有窗口管理器，最小化没有实际效果，这组就是不处理的对照 |
| `Page.setWebLifecycleState: frozen` | 929 | 1777 | 保留 |
| `Memory.simulatePressureNotification: critical` | 905 | 1775 | 保留 |
| `HeapProfiler.collectGarbage` | 未测 | 1783 | 保留 |
| 窗口缩到 500×300 | 556 | 1672 | 保留，但页面会收到 resize |
| 关闭标签页 | 330 | 567 | 丢失，再开时重新装载 |

结论：关闭（休眠）是唯一能把真实网站降下来的做法。缩小窗口对简单页有用，对真实网站只省约 190 MB。其余三种各约 80–90 MB，不做。

### 2.4 试过、不采用的做法

| 做法 | 结果 |
| --- | --- |
| 标签页合并进一个窗口 | 12 个真实网站降到 2674。但后台标签页的 `Page.captureScreenshot` 会超时，没开焦点模拟时 3 个里有 2 个，开了之后仍有 1 个。Agent 截图会坏 |
| `--renderer-process-limit=4` | 12 个真实网站 3348，约省 3% |
| 关站点隔离 | 3195，约省 8%，失去跨站隔离 |
| Chromium 自带的标签页丢弃（`chrome://discards`） | 它能保留表单、滚动位置和 sessionStorage，所以值得一试。这个页面默认停用；在 `Local State` 里打开之后再访问，整个浏览器不再应答调试连接（两种打开方式都复现）。没有查原因，不依赖它 |

### 2.5 已确认的行为

- 容器限到 1536 MB、开 6 个真实网站时，渲染进程从 26 个降到 11 个，应是被内核杀掉；Chromium 主进程和 Xvnc 活着，继续应答。
- `oom_score_adj`：渲染进程 300，GPU 和 utility 进程 200；Chromium 主进程、Xvnc 和 BuckyOS 服务都是 0。整机 OOM 时先死渲染进程。
- `Target.closeTarget` 不触发 `beforeunload` 弹框，有未保存内容的页面会被直接关掉。休眠前必须自己检查。
- 在隔离环境里执行脚本（`Page.createIsolatedWorld`）能看出表单改过没有，页面看不到这段脚本。
- 标签页崩溃时，浏览器级连接收到 `Target.targetCrashed`，目标仍在列表里，`Page.reload` 能恢复。
- Desktop 里把 App 窗口最小化会断开屏幕（mock 上实测，`data-status` 变成 `idle`，之后只有每几秒一次的 `browser.status`，它不算使用）。浏览器页签隐藏时仍连着（模拟 `visibilityState: hidden` 25 秒，状态还是 `open`）。

## 3. 现状与缺口

| # | 现状 | 落点 |
| --- | --- | --- |
| G1 | App 页面隐藏后屏幕不断开。隐藏页面的计时器会被浏览器限到每分钟一次，服务端 30 秒没有上行就断，客户端随后又自动重连；每次重连先调 `browser.ensure`，停了的浏览器也会被拉起来。这期间空闲计时一直被刷新，`idle_stop_secs` 不会到 | [channel.ts](../src/frame/desktop/src/app/agent-browser/api/channel.ts) `wake`、`lost`、`startPing`；[store.ts:298](../src/frame/desktop/src/app/agent-browser/store/store.ts#L298) `syncChannel`；[screen.rs:771](../src/frame/opendan/src/browser/screen.rs#L771)、`:857` |
| G2 | 空闲判断只看观看者、租约和重排，不看下载。补上 G1 之后，用户离开页面等大文件下载时，浏览器会在中途停掉 | [mod.rs:1199](../src/frame/opendan/src/browser/mod.rs#L1199) `idle` |
| G3 | 浏览器重新启动时，把存下来的标签页全部重开 | `mod.rs` `start()` 里 `take_saved` 之后的循环；[groups.rs](../src/frame/opendan/src/browser/groups.rs) `take_saved` |
| G4 | 没有标签页休眠，后台标签页一直占着渲染进程和窗口缓冲。每个标签页都是自己窗口的当前页，Chromium 的省内存模式也不会丢弃它们 | `mod.rs:496` `open_in` |
| G5 | 屏幕固定 2 倍，和节点内存无关 | [config.rs](../src/frame/opendan/src/config.rs) `default_device_scale_factor`；`mod.rs` `scale()` |
| G6 | 启动浏览器、开标签页之前不看节点还剩多少内存；运行中内存紧张也没有动作。每个 Agent 各管各的，节点上没有总量概念 | `mod.rs` `ensure`、`open_tab`、`housekeep` |
| G7 | 标签页崩溃没有处理。`on_event` 不认 `Target.targetCrashed`，App 和 Agent 都看不到原因 | `mod.rs` `on_event` |
| G8 | Agent 容器不设内存上限，也不看浏览器用了多少。应用安装配置的 `container_param` 可以手工加 `--memory` | [app_loader.rs:2005](../src/kernel/node_daemon/src/app_loader.rs#L2005) |

## 4. 设计

### 4.1 空闲回收

- App：页面隐藏满 5 分钟就断开屏幕，变回可见时立即重连。复用最小化时已有的断开路径，把“页面隐藏”并进 `syncChannel` 的 `wanted` 条件，`ScreenChannel` 不加第二套状态。
- 5 分钟是为手机用户留的：切到短信、验证器或文件 App 取个东西再回来，不应该断。
- 断开后没有观看者，没人处理的 JS 弹框按现有规则关掉（`close_unattended_dialogs`）。这和最小化窗口时一样，不另做。
- 服务端：有下载在进行时算在用（G2）。

### 4.2 标签页休眠

休眠就是关掉标签页的目标，在组里留下它的位置、网址和标题。用户点它，或 Agent 唤醒它，才按网址重新打开。

**何时休眠。** 后台标签页最后一次使用超过 `tab_sleep_secs`（默认 300 秒），并且下面的条件都不成立。内存紧张时不等这个时间（§4.4）。

“使用”包括：成为当前页、新打开、网址变化、所在组的租约结束。Agent 通过 CDP 直接操作，服务端分不清它用了哪个标签页，所以租约结束时组内所有标签页一起记一次。

**不休眠：**

- 组的当前标签页；组上有租约；有没处理的接手请求（`handover`）；在等用户回答（`waiting_answer`）。
- 正在装载、有 JS 弹框、有等用户回应的请求（选文件、登录框、权限等）、有进行中的下载。
- 弹窗，以及它的打开者。
- 网址不是 `http` / `https`（`about:`、`blob:`、`data:`、`file:`、`chrome:`）。
- 表单改过没提交，或有媒体正在播放。休眠前在隔离环境里查一次，1 秒没答按“不休眠”。
- 内部组（`_` 开头）。

**会丢什么。** 返回历史、sessionStorage、没被上面识别出来的页面内状态，以及 POST 得到的结果页（重开是 GET）。Cookie、localStorage 和登录态保留。滚动位置休眠时记下，唤醒装载完成后恢复。

**重新启动时**（G3）：只打开当前标签页，其余直接进入休眠。

**上限。** 休眠的标签页仍计入 `max_tabs`，用户看到的标签页数量不变。

**标签页编号。** 休眠的标签页沿用休眠前的 `tab_id`；启动时直接休眠的由服务端生成一个。唤醒后是新的目标，`tab_id` 会变，位置不变。协议本来就说重启后 `tab_id` 会变，App 按 `state` 更新。

**并发要点：**

- 先在锁内标记休眠（复查租约和当前页），再 `closeTarget`。`targetDestroyed` 到达时，已标记休眠的标签页不移除。
- `resync`、`refresh_tabs`、`keep_windows`、`tab_ids()` 只看醒着的；`suspend()` 存盘时两种都存，顺序不变。
- 唤醒时同一个标签页只允许一个在途操作，并发的请求等同一个结果。
- `activate_tab` 现在是同步函数，要改成异步。

### 4.3 屏幕倍率

`device_scale_factor` 增加取值 `auto`，作为默认。节点物理内存不足 6 GiB 时取 1.5，否则取 2。写了具体数字的配置照用。倍率在浏览器启动时定，运行中不变，这是现有行为。

1.5 倍的内存和手机清晰度都没测过，先做 C1 再定档（Q3）。

### 4.4 内存压力

各 Agent 各自看节点的可用内存，不设中心协调。每个 Agent 只收自己没人用的部分，合起来就是节点的总量控制。

- 读 `/proc/meminfo` 的 `MemTotal`、`MemAvailable`。Docker 容器里看到的是宿主或虚拟机的值，正好是要的。如果容器自己有 cgroup 上限，再看 `memory.max - memory.current`，取小。macOS、Windows 原生运行用 `sysinfo`（工作区已有这个依赖，opendan 还没引用）。
- 读不到就当作不紧张，不影响使用。

| 状态 | 条件（初值） | 动作 |
| --- | --- | --- |
| 正常 | 其余情况 | 只按时间休眠 |
| 紧张 | 可用低于 `max(512 MiB, 总量的 10%)`，连续 3 次检查 | 立刻休眠所有可休眠的后台标签页；没人看且没租约的浏览器立刻停掉，不等 `idle_stop_secs` |
| 不足 | 可用低于 700 MiB，在启动浏览器前检查 | 不启动，回 `browser_unavailable`，`unavailable_code: low_memory` |

- 检查放进 `housekeep`，每 5 秒读一次，动作之间至少隔 30 秒，留时间让内存降下来（关标签页后约 45 秒才释放完）。
- 正在看的、Agent 正在用的浏览器，不因为紧张被停。开新标签页也不拒绝，避免打断正在做的事。真的用完时靠 OOM 优先级兜底：先死渲染进程（§2.5）。
- 数值是初值，D1 按 §6 的机器规格测完再定。

### 4.5 标签页崩溃

- `Target.targetCrashed` 把标签页标成 `crashed`。装载新文档后清掉。
- App 的标签页列表显示“已崩溃”，提供重新装载。原始页面视图里，Chromium 自己的崩溃页会出现在画面上，这一点要实际确认。
- `agent-browser status` 和 `tabs` 显示 `crashed`。

## 5. 协议改动

先改 [Agent Browser Protocol](<../doc/opendan/protocol/Agent Browser Protocol.md>)，再改代码。

| 位置 | 改动 |
| --- | --- |
| §2.1 `Tab` | 加 `asleep: bool`、`crashed: bool` |
| §2.1 `Status` | 加 `memory`：`{ available_mib, total_mib, pressure: "normal" \| "tight" }`，读不到时为 `null`。`unavailable_code` 加 `low_memory`，判定放在现有各项之后；它会自己恢复，App 给出“节点内存不足，稍后再试”和重试 |
| §2.2 | 加 `browser.wake { lease_id, tab_id }`：唤醒并设为当前标签页，返回新的 `Tab` |
| §3.1 | 说明 App 隐藏后会断开；浏览器因下载而不被回收 |
| §3.2 `tab activate` | 目标休眠时先唤醒再切换。唤醒失败回 `browser_unavailable` 或 `invalid_argument`，标签页仍是休眠 |
| §3.3 `state` | `tabs` 含休眠和崩溃标记 |
| §6 配置 | 加 `tab_sleep_secs`（默认 300，0 表示不休眠）；`device_scale_factor` 加 `auto` |

兼容：旧版 `agent-browser` 在 CDP 里找不到休眠的标签页，会跳过它们（`joinTabs` 已有 `filter(Boolean)`），照常工作。

## 6. 实施 TODO

每条做完在 §9 记落点、验证和与本文的差别。

### M1 空闲回收（不依赖 §8）

- [ ] **A1 App 隐藏断开。** `store.ts` 监听 `visibilitychange`，隐藏满 5 分钟断开，变可见立即重连。`prepare` 里的 `browser.ensure` 只在页面可见时调用。
  - 验证：e2e 用可控时钟模拟隐藏 4 分钟仍连着、6 分钟已断、恢复可见后重连且收到新画面；隐藏期间没有新的 `browser.ensure`。现有 `agent-browser*.spec.ts` 全过。
- [ ] **A2 下载算在用。** `idle()` 计入进行中的下载。
  - 验证：Rust 用例；假浏览器里下载未完成时不停，完成后按时停。
- [ ] **A3 真实 Zone 验证。** App 开着切到别的页签，确认浏览器在约 5 + 10 分钟后停掉，容器内没有残留进程。手机锁屏同样测一次。

### M2 标签页休眠

- [ ] **B1 协议和数据模型。** 先改协议；`groups.rs` 的 `TabState` 加 `asleep`、`crashed`、`last_used`、`scroll`；`Tab` 对外字段；`suspend` / `take_saved` / `tab_removed` / `tab_ids` 按 §4.2 处理。
  - 验证：`groups.rs` 单元测试覆盖排序、计数、存盘往返、休眠期间的 `targetDestroyed`。
- [ ] **B2 休眠与唤醒。** `housekeep` 里挑候选、隔离环境检查、标记、关闭；`wake`；`activate_tab` 改异步；`browser.wake`。
  - 验证：§4.2 “不休眠”的每一条各有用例；租约与休眠竞争、两个请求同时唤醒、唤醒时浏览器刚好停掉。
- [ ] **B3 启动只开当前页。**
  - 验证：存 12 个标签页，启动后只有 1 个目标；列表和顺序与停之前一致。
- [ ] **B4 App。** 标签页列表显示休眠状态，点击即唤醒；装载期间显示进度；唤醒失败保留休眠并提示。mock 支持休眠。新文案按现有流程补齐各语言。
  - 验证：桌面和手机视口的 e2e。
- [ ] **B5 Agent 侧。** `agent-browser tabs` 显示 `(asleep)`；新命令 `agent-browser wake <n>`，唤醒后成为当前标签页；`run` 的 `pages` 只含醒着的，帮助文字写明。ext-core 升版本；手册 [cmd_skills/browser](../src/apps/jarvis_agent/agent/cmd_skills/browser) 补一句。
  - 验证：`agent-browser.test.mjs`；旧版命令对新服务、新命令对旧服务各跑一次，失败要有明确说明。
- [ ] **B6 真实网站验证。** 按 §2.2 的 12 个网站，休眠后读数；抽查登录态、滚动位置、表单改过的页面没有被休眠。
  - 目标：只剩当前页醒着时，不可回收部分不超过 1 GB。参照：6 个真实网站关到剩 1 个是 567 MB。

### M3 倍率与内存压力

- [ ] **C1 测 1.5 倍。** 内存按 §2 的方法，加手机实际观看的清晰度对比（1 / 1.5 / 2）。结果给 owner 定档（Q3）。
- [ ] **C2 `auto` 取值。** 配置解析、`Status.device_scale_factor` 报实际值。
  - 验证：单元测试覆盖边界；`auto` 与具体数字两种配置。
- [ ] **D1 内存读取。** 一个小模块，Linux 容器 / cgroup / 原生三种来源，读不到返回 `None`。
  - 验证：用固定文件内容做单元测试；本机容器和一台有内存上限的容器各验一次。
- [ ] **D2 紧张时的动作。** 按 §4.4 的表。
  - 验证：注入读数的用例覆盖三档、连续 3 次、动作间隔；两个 Agent 同时紧张时，正在用的那个不受影响。
- [ ] **D3 启动前检查。** `low_memory` 从 opendan 到 App、`agent-browser` 的提示一致。
- [ ] **D4 标签页崩溃。** §4.5。用 `Page.crash` 做用例。
  - 验证：App 与 `agent-browser` 都能看出崩溃并能恢复；确认 VNC 画面上是否出现 Chromium 的崩溃页。
- [ ] **D5 容器内存上限。** 按 Q4 的答复，默认不做。

### M4 测量与收口

- [ ] **E1 测量脚本进仓库。** 放 `test/agent_browser_dv/`，按 §2.1 的方法：起一次性容器、按配置开标签页、读 `memory.stat`、输出一行 JSON。参数为窗口模式、倍率、标签页数、简单页或真实网站。真实网站的数据随网站变化，只用于人工对比，不进 CI。
- [ ] **E2 低配整机验证。** 4 GiB 的 VM，部署 3 个 Agent，同时用浏览器。看内存、换页、系统服务响应，以及 §4.4 的阈值是否合适。`demo.buckyos.io` 是真实用户环境，部署前另外征得 owner 同意。
- [ ] **E3 多 Agent 累计。** 同一节点 1 / 2 / 3 个浏览器的总占用，确认 Chromium 程序文件页确实共享。
- [ ] **E4 文档。** 协议 §6、兼容性 TODO 的相关说明、[agent-runtime-browser-todo.md](agent-runtime-browser-todo.md) 的“内存”风险条目更新为本文的结论。

## 7. 验收

| # | 场景 | 通过条件 |
| --- | --- | --- |
| V1 | 页面隐藏 | 约 5 分钟后断开，再过 `idle_stop_secs` 浏览器停掉，无残留进程 |
| V2 | 重新进入 | 恢复可见后 5 秒内重连；浏览器停过时只装载当前页 |
| V3 | 多标签页 | 12 个真实网站，后台休眠后不可回收部分 ≤ 1 GB（§6 B6） |
| V4 | 休眠的正确性 | 表单改过、正在播放、有下载、有弹框、被租约占用的标签页都没有休眠 |
| V5 | 唤醒 | 用户点击、Agent `wake` 都能回到原网址和滚动位置，登录态在 |
| V6 | 紧张 | 空闲浏览器和后台标签页被收掉；用户正在看的、Agent 正在用的不受影响 |
| V7 | 内存不足 | 启动被拒绝并给出 `low_memory`，App 与 `agent-browser` 提示一致；恢复后重试能用 |
| V8 | 崩溃 | 一个标签页崩溃，其余正常；两侧可见，可重新装载 |
| V9 | 回归 | 现有 `browser::` 用例、`agent-browser*.spec.ts`、`test/agent_browser_dv` 通过；登录、上传下载、交接、重排不受影响 |

无头模式（macOS / Windows）也有休眠、压力处理和崩溃处理，要跑同样的 V4–V8。这两个平台本机没有条件，要在测试 VM 上验。

## 8. 待 owner 确认

| # | 问题 | 默认做法 |
| --- | --- | --- |
| Q1 | 休眠会丢返回历史、sessionStorage 和没识别出来的页面内状态（§4.2），能否接受 | 接受。理由：手机浏览器本来就会回收后台标签页 |
| Q2 | 时间取多少 | 隐藏 5 分钟断开；后台 5 分钟休眠；`idle_stop_secs` 维持 10 分钟 |
| Q3 | 低配节点是否降低屏幕倍率 | `auto`：不足 6 GiB 用 1.5，否则 2。先做 C1，看过手机效果再定 |
| Q4 | Agent 容器要不要设内存上限 | 先不设。同一个容器里还跑 Agent 的构建和命令，上限会把编译一起限住；压力处理加 OOM 优先级已能先收浏览器。要限的节点可以用 `container_param` 手工加 |

Q4 与 2026-11-23 口头建议的“容器内存上限必须有”不同：核对后发现上限管的是整个容器，分不开浏览器和别的命令，所以改成默认不设。

## 9. 实施记录

| 日期 | 条目 | 记录 |
| --- | --- | --- |
| 2026-11-23 | 立项 | §2 的测量完成，只写文档，没有改代码。测量脚本没有进仓库（E1） |
