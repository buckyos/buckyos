# BuckyOS APP 直连 Zone 实施 TODO：断网时也能用

日期：2026-11-24。状态：**已实施、未提交**（当天；记录、与正文不同的地方和等 owner 的事在 §11）。没有真机，iOS 没有编译过。owner 已答复 Q3–Q6（§6.1，本文已按答复改写），Q9 已由 iOS 内容视图的合入落实（§11.5），§6.2 是替 owner 选的做法。强制 http 和过期证书的处理按 owner 要求另起 [强制 http TODO](buckyos-app-force-http-todo.md)，本文全程 https；Q1、Q2、S5、T0-6、H 系列任务随之移过去，编号沿用，本文不再用这些编号。

涉及仓库：

- BuckyOSApp：客户端（Tauri 2，Rust + React + Android Kotlin + iOS 原生）。大部分工作在这里。
- buckyos：node_daemon、control_panel、`boot_gateway.yaml`、Desktop Web UI、buckyos-api。
- cyfs-gateway：证书续期时间。

源码核对基线：BuckyOSApp `3a691a7`（main）、buckyos `a552208d7`、cyfs-gateway `e80b6e28`。

标了【实测】的是 2026-11-24 跑过的：Linux 上的 Chromium 连真实 Zone；macOS 26 上的 WKWebView 连一个最小代理（§2.2）。其余是读代码的结论。标了“待验证”的由 T0 确认，结果写到 §11；与正文不一致时以 §11 为准。

用词：

- **客户端**：BuckyOS APP。**内容视图**：客户端里显示 Zone 页面的 WebView。
- **Zone 主机**：Zone 的主机名和它的所有子域（`<zone>`、`*.<zone>`）。
- **直连**：客户端不经过 SN relay，直接连到 Zone 里的某台设备。分“本机”（127.0.0.1）和“局域网”两种。
- **系统线路**：照系统的域名解析去连，也就是现在的走法，多数情况下经过 SN relay。

## 0. 结论先行

1. **目标是断网时还能用。** 桌面端总是直连本机，飞机上能用本 Zone 的应用；手机优先走局域网地址，家里断了外网，手机还能用家里的 OOD（§1）。少绕一圈 relay 是顺带的好处。
2. **做法是客户端自带一个本地代理（§3）。** 它只接 Zone 主机的连接，按“本机 → 局域网 → 系统线路”选一条，不解开 TLS。页面的地址、证书、Cookie、源都不变，应用不用改。
3. **断网时这条路走得通【实测】。** 把 Chromium 放进只有回环网卡的环境：不走代理时直接报 `ERR_INTERNET_DISCONNECTED`；走本地代理时 https 页面和登录页都正常打开，是安全上下文。macOS 26 的 WKWebView 关掉网卡后，用 `proxyConfigurations` 也正常打开。
4. **节点这一侧离线基本能用。** 单 OOD 断网冷启动、登录、令牌校验、服务之间的调用都不依赖外网（§2.3）。但有一处会让电脑整个不能用：**休眠超过 7 天再唤醒，node_daemon 会停在鉴权失败，直到重启**（N1）。笔记本很容易碰到，联网时也一样，建议先修。
5. **页面这一侧问题很少。** Desktop 没有从外网加载的资源。要改的是几处用 `navigator.onLine` 判断能不能操作的地方：在飞机上它是 `false`【实测】，但本机 Zone 是通的（F1）。
6. **手机这一侧最难的是 Android 的网络选择。** Wi-Fi 没有外网时，系统会把默认网络换成蜂窝，发往 192.168.x.x 的连接就出不去。代理要把自己的连接绑到 Wi-Fi 上（M3）。这也是必须由客户端自己做代理的原因之一。
7. **局域网发现现在用不上 finder。** 它只在多 OOD 的 Zone 启动时开，启动完就关；而且要求对方有本 Zone 的设备身份，手机没有。第一版用“记住的地址 + 同网段扫描”（§3.4）。
8. **证书能撑多久。** 现在到期前 7 天才续期，所以断网时最少还能撑 7 天。[Let's Encrypt](https://letsencrypt.org/2026/10/07/64-day-certs) 2027-02-10 起默认发 64 天的证书，[2028-02-16 起 45 天](https://isrg.org/post/from-90-to-45/)。改成剩三分之一时续期，最少能撑 30 / 21 / 15 天（N2）。过期之后怎么办见 [强制 http TODO](buckyos-app-force-http-todo.md)。
9. **iOS 和 Android 同时做（owner，Q4）。** iOS 的内容视图另有人在推进，还没合入 main（[ios.rs](../../BuckyOSApp/src-tauri/src/content/mobile/ios.rs) 仍是占位）。iOS 的任务排在它后面，但有几条要求要现在就告诉做内容视图的人（§3.6）。
10. **系统浏览器靠用户自己设代理（owner，Q5），在 macOS 上有一个硬限制【实测】：** 电脑一个网络都没连的时候，系统代理设置整个失效。连着随便哪个网络时才有用。客户端自己的窗口不受影响（§3.7）。

## 1. 目标（owner，2026-11-24）

| 编号 | 目标 | 含义 |
| --- | --- | --- |
| G1 | 当前主推的 desktop 版本，可以在飞机上使用本 Zone 的 app | 客户端和 OOD 在同一台电脑上，客户端**总是**直连本机，联网时也是 |
| G2 | 家庭局域网失去互联网接入时，手机 app 依旧可以访问局域网里的 OOD，并使用上面的功能 | 手机 app **优先**使用局域网 IP，不通才走系统线路 |
| G3 | 先用 https。http 在特殊情况下可以强制开启，比如证书到期了 | 本文全程 https；http 另见 [强制 http TODO](buckyos-app-force-http-todo.md) |

由此定的边界：

- 主要管客户端里的访问。系统浏览器只提供手工设置的说明（§3.7），不自动改系统设置。
- 断网时不要求可用的：云端模型、经 SN 的跨 Zone 访问、激活、安装应用、证书续期。
- 桌面客户端的 macOS 最低版本是 14（owner，Q3）。
- G1 按单 OOD 的 Zone 验收。多 OOD 的 Zone 断网时凑不够多数，不在本文范围。

## 2. 现状

### 2.1 为什么会绕 relay

Zone 在 NAT 后面时，SN 对 Zone 主机名返回的 A 记录只有 relay 的地址，只有 AAAA 才带设备自己的公网 IPv6（`cyfs-gateway` 的 `cyfs-sn/src/sn_resolver.rs`）。没有 IPv6 就只能走 relay。

relay 进来的 443 在节点上的最后一跳是 `forward tcp:///127.0.0.1:443`（[boot_gateway.yaml](../src/rootfs/etc/boot_gateway.yaml) 的 `node_rtcp` / `external`）。所以直连 127.0.0.1:443 到的是同一个 TLS 入口，没有功能差别。

耗时【实测】：本机 TLS 握手 5 到 8 ms，取 322 KB 共 7 到 10 ms；经 relay 分别是 92 到 255 ms 和 215 到 262 ms。

### 2.2 断网时 WebView 的行为【实测】

**Linux Chromium。** 环境：`unshare -rn` 建一个只有 `lo` 的网络命名空间，里面跑 Chromium 和一个最小代理；代理用 unix socket 连到命名空间外的真实 Zone。

| 做法 | 结果 |
| --- | --- |
| 不走代理 | `ERR_INTERNET_DISCONNECTED` |
| PAC 指到 HTTP CONNECT 代理，https | 200，安全上下文 |
| PAC 指到 SOCKS5 代理，https | 200，安全上下文；登录页完整显示，没有失败的请求 |
| 以上各项的 `navigator.onLine` | 都是 `false` |

**macOS 26（测试 VM）。** 环境：一个最小的 WKWebView 程序，`proxyConfigurations` 指到 127.0.0.1 上的代理，match domain 是 `zone.invalid`，打开 `http://app.zone.invalid/`；断网是 `ifconfig en0 down`。

| 做法 | 联网 | 没有任何网络 |
| --- | --- | --- |
| `proxyConfigurations`，HTTP CONNECT | 正常，子域也匹配，域名原样传给代理 | 正常，`navigator.onLine` 为 `false` |
| `proxyConfigurations`，SOCKS5 | 正常 | 正常 |
| 系统设置里的自动代理（PAC） | 正常（WKWebView 和 URLSession 都走） | 不走，报 -1009“没有连接互联网”；`scutil --proxy` 里 PAC 消失 |
| 系统设置里的手工 HTTP 代理 | 正常（只测了 URLSession） | 同上 |
| 不设代理 | -1003 | -1009 |

只测了 http 页面（代理收到的是 `CONNECT …:80`）。没有测 https、Service Worker、真实 Zone 页面，也没有图形登录，所以看不到有没有弹本地网络权限。

联网时另外测过的（Linux Chromium）：

- 只把一部分 Zone 主机改指到 127.0.0.1（比如根域走公网、子域走回环），子域请求会被 Chromium 的本地网络访问检查（LNA）拦掉。把全部 Zone 主机都指过去时，Zone 外的页面访问 Zone 又会被拦。**代理没有这个问题**，所以不用改解析的办法（hosts、`--host-resolver-rules`）。
- Chromium 不支持带用户名密码的 SOCKS5（`ERR_NO_SUPPORTED_PROXIES`）。
- Playwright 的 headless shell 不认 PAC，测试要用 `channel: 'chromium'`。

### 2.3 节点侧离线情况

按“单 OOD、已激活、正常启动过至少一次”核对：

| 环节 | 断网时 | 依据 |
| --- | --- | --- |
| 冷启动取 Zone 文档 | 能启动，最多慢 20 秒 | 先 `resolve_did` 再查 DNS，各有 10 秒上限；都失败就用 `etc/<zone>.zone_lkgs.json`，激活时就会写（`node_daemon/src/zone_boot_resolve.rs`） |
| 没有任何网卡 | 不影响主循环 | `DeviceInfo` 的系统信息填充总是成功（`name-lib/src/device.rs`） |
| 运行中断网 | 服务照跑，主循环变慢 | 向 SN 取 Zone 信息和上报都在主循环里等（`node_daemon.rs:1536-1595`），每次最多 15 秒。飞机上那种“连着 Wi-Fi 但不通”的网络最明显 |
| IP 变化 | 网关不重启 | keep_tunnel 列表不变就不重启 |
| 网关启动、443 | 正常 | 证书从磁盘读，ACME 在后台跑 |
| 证书过期 | 照常握手，发的是过期证书 | 由客户端拒绝；恢复联网后约 10 分钟内会续上 |
| 登录、刷新、令牌校验 | 正常 | 密钥都在本地 system-config |
| 服务之间的调用 | 正常 | 走回环或 `BUCKYOS_HOST_GATEWAY` |
| AppClient 类的运行时 | **要解析 Zone 主机名**，除非设了 `BUCKYOS_HOST_GATEWAY` | `buckyos-api/src/runtime.rs:2438-2450`，例如 `agent_tool` 的 xllm |
| 应用启动 | 镜像和包在本地就能起；缺了会去公网拉 | `node_daemon/src/app_loader.rs:1254-1330` |
| 原生 Deno 应用（Lite） | 依赖缓存过才行 | 用 `deno run`，没有加 `--cached-only`；sys_test 引了 `jsr:` |
| node_daemon 重启 | 443 中断约 35 秒 | 既有问题：每次启动都重启网关，旧连接的 TIME_WAIT 让 443 绑定失败约 30 秒（[macOS TODO](macos-desktop-install-runtime-todo.md) 第 239 行记录） |
| 文档有效期 | 不是问题 | 设备文档 5 年，Zone 和 owner 文档 10 年 |

**休眠超过 7 天的问题（读代码确认，没有复现）：** verify-hub 的 refresh token 有效期 7 天。`renew_token_from_verify_hub_once`（`buckyos-api/src/runtime.rs:1185` 起）只有在当前令牌**不是** verify-hub 签的时候才会用设备私钥重新登录。休眠 7 天以上醒来，refresh token 已过期，被拒后进入终止状态，`get_system_config_client` 之后一直报错，node_daemon 主循环每轮都 `continue`（`node_daemon.rs:1519-1526`），不再管服务、不再上报、不再更新网关。别的服务进程也是同一套代码。

### 2.4 局域网访问的条件

- **监听。** 443、80、3180 和 RTCP 2980 都绑 `[::]`。443 只有在 Zone 有 SN 或自定义域名时才会生成（`scheduler/src/system_config_agent.rs` 的 `build_domain_gateway_config`），不区分 OOD 和普通节点。
- **防火墙。** Windows 安装器按程序放行入站（Profile Any），443、80、3180、2980 都在内。macOS、deb、rpm 的安装脚本不动防火墙。
- **按 Host 路由。** 网关要求 Host 精确等于 Zone 主机名。`https://192.168.x.x/` 打不开应用，也没有证书。所以局域网访问也必须保留 Zone 主机名，只换连接的 IP。代理正好是这样。
- **地址从哪来。** control-panel 的 `zone.devices.list` 返回每台设备的 `ips`、`endpoints`、`is_ood`、`online`，登录用户就能调（受限会话和外部身份不行）。设备每 30 秒左右更新一次自己的地址，断网时也更新。列表里会有 docker、tailscale 之类的地址，要靠探测筛掉。
- **finder 不能用。** `node_daemon/src/finder.rs`：只有多 OOD 的 OOD 在启动期间开 UDP 服务（`node_daemon.rs:2020`），启动完就停；请求要带 owner 签过的设备文档和设备私钥签名。
- 仓库里没有 mDNS、Bonjour、SSDP。

### 2.5 客户端里要改走代理的地方

| 位置 | 现在 |
| --- | --- |
| 桌面内容窗口、主窗口、托盘弹窗、签名窗口 | `WebviewWindowBuilder`（`content/windows.rs:503` 等），主窗口由 `tauri.conf.json` 创建 |
| Android 内容视图 | `ContentView.kt`，每个 Zone 一个 WebView profile（`Profiles.kt`） |
| iOS 内容视图 | 占位，另有人在做；工程最低版本 iOS 13 |
| Rust 的 Zone 请求 | `zone/api.rs` 的 `ReqwestTransport` |
| 通知长连接 | `notify/core.rs` 的 `stream_client`；Android 在单独的 `:notify` 进程里 |
| Android 下载 | `Downloads.kt`，用 `HttpURLConnection` |
| 媒体代理 | `zone/media.rs`，走 `ReqwestTransport` |
| DID 解析、SN、BNS | 不是 Zone 主机，不改。断网时失败只记日志，Zone 列表保留 |

## 3. 方案

### 3.1 总体

```
内容视图 ─┐
Rust 请求 ─┼→ 客户端本地代理 ─┬→ 127.0.0.1:443        （桌面）
下载、通知 ─┘   只接 Zone 主机   ├→ 局域网设备 IP:443
               不解 TLS        └→ 系统线路（现在的走法）
```

代理在客户端进程里，监听 `127.0.0.1` 的随机端口，同时认 HTTP CONNECT 和 SOCKS5（按首字节区分）。收到连接后查这台主机属于哪个 Zone、这个 Zone 现在选的是哪条线路，连上去，之后只搬字节。只接已登记 Zone 的主机、端口 443，其它一律拒绝。系统浏览器用的是另一个固定端口的入口，默认不开（§3.7）。

### 3.2 选路

**候选顺序：** 本机（仅桌面）→ 上次成功的局域网地址 → 其它局域网地址（OOD 在前）→ 系统线路。按 Zone 分别选，多个 Zone 同时在用时互不影响。

**探测：** 对候选地址做一次真的 TLS 握手，SNI 用 Zone 主机名，按正常规则验证证书链和主机名。通过才算可用，所以别的网络里碰巧同 IP 的设备、占了 443 的其它程序都不会被选中。结果分三种：可用、证书过期（其它都对）、不可用。本机超时 300 ms，局域网 800 ms。

**什么时候探测：** 客户端启动、网络变化、从休眠恢复、连接失败、Zone 登录成功，以及停在系统线路上时每 60 秒一次。探测在后台做，不让每次打开页面都等。

**切换：**

- 本机探测通过过的 Zone，桌面端就固定用本机。连接被拒时先重试，连续 60 秒不通才退到系统线路：节点重启会断约 35 秒（§2.3），而且单 OOD 时 relay 最后到的也是本机。
- 局域网线路失效（离开家、OOD 换了地址）时，主动断开这条线路上的隧道，让 WebView 马上重连。否则浏览器要等很久才发现连接已死。
- 从系统线路变成直连时，新连接走新线路，旧隧道空闲 30 秒后关掉。HTTP/2 会一直复用旧连接，不关就一直留在 relay 上。

### 3.3 各平台怎么接

| 平台 | 做法 | 待验证 |
| --- | --- | --- |
| Windows | PAC，只把 Zone 主机指到代理。PAC 由代理自己在 http 上提供，Zone 列表变化时内容跟着变 | 参数怎么覆盖到所有窗口（含 `tauri.conf.json` 建的主窗口）；PAC 变化多久生效；与系统代理的关系（R2） |
| macOS 14+ | `WKWebsiteDataStore.proxyConfigurations` 加 match domain，只影响 Zone 主机。macOS 26 上已通（§2.2） | wry 的 `mac-proxy` 没有设 match domain，要自己调；macOS 14 上是否一样 |
| iOS 17+ | 同 macOS，每个 Zone 的数据仓库各设一次 | §3.6 |
| Android | `ProxyController` 加 reverse bypass，只有 Zone 主机走代理。要 `PROXY_OVERRIDE_REVERSE_BYPASS` | 对多个 WebView profile 是否都生效；Service Worker 是否也走 |
| Linux | WebKitGTK 的 `NetworkProxySettings`。不能按主机筛选，所以其它主机由代理原样转出去 | 优先级低 |

系统不支持时，内容视图保持现状（系统线路），不报错；界面上说明这台设备不支持直连。

### 3.4 手机找到局域网里的 OOD

1. **记住。** 每次连上 Zone（不管走哪条线路）调 `zone.devices.list`，把设备和地址存进 Zone 登记表，之后每小时更新一次。最近成功过的地址单独记。
2. **先试记住的。** 断网当天 OOD 的地址一般没变，这一步就够。
3. **地址变了就扫。** 记住的地址都不通，而且手机现在的网段和某个记住的地址相同，才自动扫这个 /24 的 443（64 并发，约 6 秒）。激活流程已有同样的扫描（`network/network.rs` 的 `scan_device_batch`）。网段不同时不自动扫，设置里给一个“在当前网络查找”按钮。
4. 扫到的地址也要过 §3.2 的 TLS 验证。

Android 和 iOS 同一套，桌面端不是 OOD 的电脑也用。不改 OOD，所以对旧版本的 Zone 也有效。finder 支持手机留作以后（§4）。

**Android 绑定 Wi-Fi：** 用 `registerNetworkCallback`（`TRANSPORT_WIFI`，不要求 INTERNET）拿到 Wi-Fi 网络，把 network handle 交给 Rust，代理连局域网地址之前用 `android_setsocknetwork` 绑定。系统线路不绑。

### 3.5 界面

- Home 的 Zone 状态显示当前线路：本机、局域网、经互联网、连不上。
- 打不开页面时，本地的失败页写清每条线路为什么不通，尤其是“证书已过期”和“这台电脑的 BuckyOS 没有运行”。
- 证书剩余不到 7 天时提示（探测时能读到有效期）：需要联网续期。
- 设置里每个 Zone 一项“连接方式”：自动（默认）、只用互联网。后一项用于排查问题。

### 3.6 iOS

选路、代理、探测、地址记录、扫描都是同一份 Rust 代码。iOS 特有的：

- **最低版本 iOS 17。** `proxyConfigurations` 和按标识的数据仓库（`WKWebsiteDataStore(forIdentifier:)`）都从 17 开始。工程现在是 13（Q9）。
- **监听会被收走。** 应用挂起后系统会回收监听套接字。回到前台先检查监听，不在了就重新监听，端口变了要重设每个数据仓库的 `proxyConfigurations`，再让页面重连。
- **本地网络权限。** 第一次连局域网地址时系统会问，127.0.0.1 不会。说明文字已有（`src-tauri/Info.plist` 的 `NSLocalNetworkUsageDescription`），要确认它进了 iOS 包。用户拒绝时走系统线路，失败页里说明怎么打开。第一次触发选在用户能理解的时候（登录成功后取到局域网地址时），不要在启动时。
- **不用广播和组播**，所以不需要另外申请组播权限。
- **后台没有长连接**，断网时的通知 iOS 做不到，验收不含这一项。
- **蜂窝和无外网的 Wi-Fi 同时在。** 同网段地址按路由应走 Wi-Fi，不需要像 Android 那样绑定，待验证（T0-11）。

**给做 iOS 内容视图的人：**

1. 最低版本按 iOS 17 定。
2. 每个 Zone 一个 `WKWebsiteDataStore(forIdentifier:)`；建 WKWebView 之前由统一的函数设好 `proxyConfigurations`，别在别处直接用 `.default()`。
3. Zone 的请求（下载、图片、媒体）不要直接用 `URLSession`；要用就套同一份 `proxyConfigurations`。
4. 导航失败的错误要带上原始错误码交给失败页。

### 3.7 系统浏览器（手工设置）

owner 的意思是只要设系统代理就行，就给用户一份说明，让用户自己设。

- **入口：** 客户端设置里一项“让系统浏览器也直连”，默认关。打开后客户端在固定端口 `127.0.0.1:3187` 上提供 PAC 和代理，页面给出当前系统的设置步骤和地址 `http://127.0.0.1:3187/proxy.pac`。固定端口是因为地址要写进系统设置。端口被占用时说明原因，不换端口。
- **步骤：** Windows 是“设置 → 网络和 Internet → 代理 → 使用设置脚本”；macOS 是“系统设置 → 网络 → 当前网络 → 详细信息 → 代理 → 自动代理配置”；Linux 按桌面环境。Firefox 默认跟随系统，也可以在自己的设置里填。
- **要写清的限制：**
  - macOS 上一个网络都没连（关了 Wi-Fi、没插网线）时，系统会忽略代理设置，浏览器打不开；连着任意一个网络就行，不需要能上外网（后半句没有实测）。完全断网时用客户端。
  - 客户端要在运行。没运行时浏览器回到原来的走法。
  - 系统里已经设了别的代理的用户不要用，会被替换。
- **安全：** PAC 里有 Zone 主机名，不能让网页读到。响应带 `X-Content-Type-Options: nosniff` 和非脚本的类型，拒绝带 `Sec-Fetch-*`、`Origin`、`Referer` 的请求。代理部分的规则同 §3.1。

## 4. 范围外

- 强制 http、过期证书放行：见 [强制 http TODO](buckyos-app-force-http-todo.md)。本文里证书过期就是连不上，失败页说明原因。
- 自动修改系统代理、hosts。系统浏览器只做 §3.7 的说明。
- 打洞、经 RTCP 直连 OOD。以后要做时，作为代理的又一个候选接进来。
- 复用 cyfs-gateway 现有的 SOCKS。3185 只放行 3184 端口，3186 要密码、只放 80、拒绝本 Zone；cyfs-socks 也不能改写目的地址。手机和没装 BuckyOS 的电脑上更没有网关。
- finder 应答手机、mDNS。
- 多 OOD Zone 的离线可用。
- 离线时的云端模型、跨 Zone 访问、应用安装。
- 没有 SN 也没有自定义域名的 Zone（没有 443 入口）。
- 局域网共享的本地 CA 或自签证书。
- `GET /1.0/identifiers/<device>?type=info` 不登录就能读到设备的局域网地址【实测】。本文不依赖它，owner 定暂时不收紧（Q6）。

## 5. 已知风险

| 编号 | 风险 | 处理 |
| --- | --- | --- |
| R1 | 各平台 WebView 的代理行为只实测了 Linux Chromium 和 macOS 26 的 http | T0 排在最前；某个平台做不到时保持现状，不阻塞别的平台 |
| R2 | Windows 上设了 PAC 就不再用系统代理。靠系统代理上网的用户，页面里的外部资源和 Zone 的系统线路会受影响 | PAC 里把系统设置的代理也写进去；系统用的是自动配置脚本时，T0-1 定做法 |
| R3 | 本机其它程序也能连这个代理 | 只通到已登记 Zone 的 443，本机程序本来就能连，不增加能力；不接受 127.0.0.1 以外的来源 |
| R4 | 在陌生网络里试记住的内网地址，会把 Zone 主机名（SNI）告诉那台设备 | TLS 验证挡住令牌泄露；主机名暴露记为已知 |
| R5 | Android 上 VPN 开着时，绑定 Wi-Fi 的连接不走 VPN；锁定模式的 VPN 会拦掉 | 失败就走系统线路，失败页说明 |
| R6 | Android 17（API 37）起，[局域网访问要运行时权限](https://developer.android.com/privacy-and-security/local-network-permission)。现在 targetSdk 是 36，暂时自动给 | M7；升 targetSdk 之前必须做完 |
| R7 | 飞机、酒店的 Wi-Fi 会劫持解析、卡住连接 | 桌面走本机不受影响；节点侧由 N3 处理 |
| R8 | 电脑时钟错得多时，令牌和证书都会失败 | 会话已有 `clock` 状态，失败页沿用 |
| R9 | 固定端口 3187 可能被占用，也让本机网页能探到客户端在运行 | 默认关；§3.7 的响应规则；被占用时只影响系统浏览器 |
| R10 | iOS 的任务依赖别人正在做的内容视图 | §3.6 的要求先发过去；在它合入前只做 Rust 侧和 T0 |

## 6. 待定与已选

### 6.1 owner 的答复

2026-11-24：

| 编号 | 问题 | owner 的答复 | 落实 |
| --- | --- | --- | --- |
| Q3 | macOS 13 及以下不支持直连，可以吗？ | “我们的最低版本设置在macOS 14是可以的” | §1、§3.3、C8 |
| Q4 | G2 这一版只做 Android，iOS 以后再接，可以吗？ | “同时做，iOS那边的工作已经在推进了” | §3.6、I 系列任务、§8 |
| Q5 | 同一台电脑上的系统浏览器要不要也能离线用？ | “如果只是设置系统的代理就能成功，可以给用户一个说明让用户手工设置。” | §3.7、B 系列任务。实测结果见 §2.2：连着网络时成功；macOS 完全断网时不成功，写进说明 |
| Q6 | 设备的局域网地址现在不登录就能读到，要不要收紧？ | “暂时不用” | 不做 |

待答：

| 编号 | 问题 | 建议 |
| --- | --- | --- |
| Q9 | iOS 最低版本提到 17（现在工程是 13）？iOS 16 及以下没有设代理的接口 | 是，和 macOS 14 是同一代系统 |

### 6.2 替 owner 选的

| 编号 | 选择 | 理由 |
| --- | --- | --- |
| S1 | 客户端自带代理，不用网关的 SOCKS | 手机上没有网关；旧 Zone 不用升级；探测、切换、绑定 Wi-Fi 都要自己控制 |
| S2 | 用代理，不改解析 | LNA 会拦；通配子域没法写 hosts；WebView 没有统一的改解析接口 |
| S3 | 同 Zone 的其它桌面设备（不是 OOD 的电脑）也用局域网线路 | 同一套代码，顺带得到 |
| S4 | 自动扫描只限“和记住的地址同网段” | 不在别人的网络里扫 |
| S12 | 系统浏览器入口默认关，固定端口 3187 | 不用的人不多开一个端口；3180–3186 已被占用 |
| S13 | macOS 完全断网时系统浏览器不可用，仍然提供说明 | 连着网络时有用；完全断网有客户端 |
| S14 | BuckyOS 的 macOS 安装包也把最低版本写成 14 | Q3 说的是“我们的最低版本”，我理解含桌面版安装包；现在两处都没有声明 |
| S6 | 桌面本机不通时等 60 秒才退到系统线路 | 节点重启约 35 秒；单 OOD 时 relay 最后到的也是本机 |

## 7. 任务

优先级：P0 是 G1 必需，P1 是 G2 必需，P2 是补强。

### T0 验证（先做，结论写进 §11）

- [x] T0-1 Windows WebView2：PAC 的传参方式（环境变量或 `additional_browser_args`，同一数据目录的所有窗口要一致）、新增 Zone 后 PAC 多久生效、系统代理、禁用全部网卡时能否打开、Service Worker / WebSocket / 下载是否都走代理。
- [x] T0-2 macOS：macOS 26 上 `proxyConfigurations` + match domain 联网和断网都通（§2.2，只测了 http）。还要测：https 和真实 Zone 页面、Service Worker / WebSocket / 下载、窗口建好以后再改是否生效、macOS 14、连 127.0.0.1 会不会弹本地网络权限。
- [x] T0-3 Android：`ProxyController` 对各 profile 和 Service Worker 是否生效；WebView 版本不够时的表现。
- [ ] T0-4 Android **真机**：Wi-Fi 无外网、蜂窝开着，绑定 Wi-Fi 后能否连到局域网地址；VPN 开着时的表现。模拟器做不了这一项。
- [ ] T0-5 断网冷启动：Windows、macOS、Linux 各一次，断网重启系统，记录节点各服务多久可用、哪些应用起不来。
- [x] T0-10 系统浏览器：Windows 上 Edge / Chrome / Firefox 用系统 PAC，联网、连着不通的 Wi-Fi、没有任何网络三种情况；macOS 上 Safari / Chrome / Firefox 连着不通的 Wi-Fi。（2026-11-24 在 Windows 测试 VM 上没测成：没有用户登录时 Edge 起不来。）
- [ ] T0-11 iOS 17 和最新版**真机**：`proxyConfigurations` 的同 T0-2 各项；挂起再回来；Wi-Fi 无外网、蜂窝开着时连局域网地址；本地网络权限弹出的时机和拒绝后的表现。
- [x] T0-7 复现休眠超过 7 天（调系统时间或缩短 refresh token 有效期），确认 §2.3 的结论和受影响的进程。

### C 客户端：选路与代理（P0）

- [x] C1 选路模块：候选、探测、选择、线路状态事件；逐 Zone；单元测试覆盖顺序、过期证书、错证书、超时、切换。
- [x] C2 本地代理：CONNECT 和 SOCKS5，主机和端口白名单，线路失效时断开隧道，PAC 输出。
- [x] C3 桌面接线：Windows、macOS、Linux（§3.3）。代理在任何 WebView 创建之前就绪。
- [x] C4 Rust 请求接线：`ReqwestTransport`、`stream_client`、`media.rs` 用同一个代理（或同一个选路模块）。
- [x] C5 网络变化与休眠恢复：触发探测，重建连接；保留 Android 现有的 `reset()`。
- [x] C6 界面（§3.5），9 种语言。
- [x] C7 断网启动检查：`syncOwner`、DID 解析、头像等失败不挡界面；靠 `online` 事件重试的地方（`ZoneContext.tsx:93`、`RuntimeHost.tsx:293`）改为也听线路状态，断网时 `online` 不会来。
- [x] C8 macOS 最低版本 14：客户端 `tauri.conf.json` 的 `bundle.macOS.minimumSystemVersion`，桌面版安装包（`src/publish/macos_pkg`），下载页和文档。

### N 节点离线（N1 到 N3 为 P0，其余 P2）

- [x] N1 refresh token 被拒后用设备私钥重新登录（buckyos-api `renew_token_from_verify_hub_once`），覆盖 node_daemon、内核服务和框架服务；应用和 Agent 容器没有设备私钥，确认它们由谁重新发令牌。
- [x] N2 证书在剩余三分之一有效期时续期（cyfs-gateway `cert_mgr.rs` 的 `renew_before_expiry`，现在是 7 天）。顺带看 ARI 要不要接。
- [x] N3 node_daemon 主循环里的 SN 请求移到后台任务，失败退避。
- [x] N4 有 LKGS 时先用它启动，后台再刷新 Zone 文档。
- [x] N5 OOD 上的 AppClient 默认走 `127.0.0.1:3180`。
- [x] N6 已安装应用的镜像和依赖不被清理；原生 Deno 应用的依赖随包或在安装时缓存好。
- [x] N7 node_daemon 重启不再让 443 中断（不重启网关，或 443 栈开 `reuse_address`）。
- [x] N8 断网时域名检查不报“域名有问题”。

### F 前端（F1、F2 为 P0）

- [x] F1 Message Hub 的“全部已读”不再因 `navigator.onLine` 禁用（`MessageHubView.tsx:157`）；Desktop 的离线标记（`DesktopRoute.tsx:215`）改成看 Zone 是否连得上（kevent 连接状态），不看系统网络。
- [x] F2 SWR 的 `isOnline` 改成同一个来源，断网时轮询和聚焦刷新照常（Desktop、OpenDAN WebUI）。
- [x] F3 检查其余用到 `navigator.onLine` 的地方（AIWS `api/session.ts:311`、`WorkspaceList.tsx:259`），文案改成“Zone 连不上”。

### M 手机（P1，除 M7 外）

M2、M4 是两端共用的 Rust 代码，其余是 Android。iOS 见 I 系列。

- [x] M1 `ProxyController` 接线，随 Zone 列表更新。
- [x] M2 地址记录（§3.4 第 1 步），存 Zone 登记表。
- [x] M3 绑定 Wi-Fi 网络。（代码已写；模拟器没有 Wi-Fi，绑定没有真正执行过）
- [x] M4 同网段扫描和“在当前网络查找”。
- [x] M5 `:notify` 进程用同一个选路模块，断网时通知也走局域网。
- [x] M6 `Downloads.kt` 走代理；逐一确认播放、上传、录音上传没有绕开。
- [x] M7 局域网权限：为 targetSdk 37 准备 `ACCESS_LOCAL_NETWORK` 的声明、申请时机和拒绝后的表现（P2）。

### I iOS（P1，排在 iOS 内容视图之后）

- [x] I1 把 §3.6 的要求交给做内容视图的人，工程最低版本改到 17（Q9 定了才改）。
- [ ] I2 内容视图的数据仓库接 `proxyConfigurations`，随 Zone 列表更新。（代码已写，没有编译过，见 §11.7）
- [ ] I3 前后台切换：监听检查、重新监听、重设代理、页面重连。（代码已写，没有编译过，见 §11.7）
- [ ] I4 本地网络权限的时机、说明文字和拒绝后的表现。（代码已写，没有编译过，见 §11.7）
- [ ] I5 Rust 请求、下载、媒体走同一选路。（代码已写，没有编译过，见 §11.7）
- [ ] I6 iOS 上 `online` 事件和 `navigator.onLine` 的处理同 C7、F1。（代码已写，没有编译过，见 §11.7）

### B 系统浏览器（P2）

- [x] B1 固定端口的 PAC 和代理入口，默认关，§3.7 的安全规则，单元测试覆盖网页来源被拒。
- [x] B2 设置页的说明，按当前系统给步骤，写清三条限制，9 种语言。
- [x] B3 `doc/port_usage.md` 登记 3187。

## 8. 验收

G1（Windows 和 macOS 各一遍，用正式安装的 desktop 版本，单 OOD）：

| 场景 | 通过标准 |
| --- | --- |
| 联网，正常使用 | 抓包确认客户端到 Zone 的流量全在回环上，没有到 SN relay 的 443 |
| 使用中关掉 Wi-Fi、拔网线 | 已打开的页面继续可用；新开 Files、Message Hub、AI 工作区（不跑模型）、设置都能打开并保存 |
| 断网重启电脑 | 节点起来后客户端自动恢复登录，不要求输密码；5 分钟内上面的应用都能用 |
| 连着不通的 Wi-Fi（飞机、酒店的登录页） | 同上，没有额外卡顿 |
| 客户端断网期间新开、退出再开 | 正常 |
| 节点重启 | 客户端显示“本机没有运行”，起来后自动恢复 |
| 断网后恢复 | 不用重启客户端；通知不丢不重 |
| 休眠 8 天后唤醒（改时钟模拟） | 节点恢复正常，不用重启 |
| 两个 Zone，一个在本机、一个在远端 | 本机的走回环，远端的走系统线路，互不影响 |
| 公网页面访问 Zone、Zone 页面访问子域 | 与改之前一致，没有被 LNA 拦 |

G2（Android 和 iOS 真机各一遍，普通家用路由器，拔掉 WAN）：

| 场景 | 通过标准 |
| --- | --- |
| 家里联网 | 走局域网，抓包确认 |
| 断外网，蜂窝关 | 打开应用、收发消息、看图和视频、传照片都正常 |
| 断外网，蜂窝开 | 同上（Android 见 M3） |
| 断外网，通知（仅 Android） | 客户端在后台时仍能收到 Zone 里的通知 |
| 断外网，切到后台 10 分钟再回来 | 页面能继续用，不用重开（iOS 重点看监听被收走的情况） |
| 断外网期间冷启动客户端 | 自动恢复登录 |
| 断外网期间 OOD 换了地址 | 自动扫描后恢复，或用按钮恢复 |
| 离开家 | 10 秒内切到系统线路，页面能继续用 |
| 回到家 | 60 秒内切回局域网 |
| 别的网络里有同 IP 的设备 | 不会连上去，没有证书警告 |

系统浏览器（B，Windows 和 macOS）：

| 场景 | 通过标准 |
| --- | --- |
| 照说明设好，联网 | Zone 页面走本机，其它网站不受影响 |
| 连着不通的 Wi-Fi | 能打开本机 Zone |
| 客户端退出 | 浏览器照原来的走法打开 Zone |
| 任意网页尝试读 `http://127.0.0.1:3187/proxy.pac` | 读不到内容 |

没有跑过的平台和场景，在实施记录里写明。

## 9. 建议顺序

1. I1 现在就做，另外 T0-1、T0-2、T0-5、T0-7，与 N1、N3、F1、F2 并行。N1 不依赖其它任务，可以先合入。
2. C1 到 C8，做完跑 G1 验收。
3. T0-3、T0-4、T0-11，M 和 I，跑 G2 验收。iOS 内容视图还没合入时，Android 先验收，iOS 合入后补跑。
4. B，N2、N4 到 N8。N2 要赶在 2027-02-10 之前发布。

## 10. 接手时必须遵守的约束

- **不解 TLS。** 代理只搬字节，不在客户端终结 TLS，不装根证书。
- **验证通过才直连。** 任何候选都要过完整的证书验证。放宽只在 [强制 http TODO](buckyos-app-force-http-todo.md) 里做。
- **页面看到的地址不变。** 不改主机名、端口、协议；应用不需要知道走的是哪条线路。
- **直连失败不能比现在差。** 平台不支持、探测失败时，行为与现在一致。
- **不看 `navigator.onLine`。** 判断 Zone 能不能用，看 Zone 本身通不通。
- **Git 依赖** 按各仓库 `AGENTS.md`，写分支名 `main`。
- **提交顺序：** C、M 不依赖新版 Zone，可以单独发布；N、F 随 buckyos 和 cyfs-gateway 发布。
- **工作树：** 同一台机器上可能有别的会话在改同一棵树，动手前看 `git status`，不用 `git stash` 做基线。
- **文档同步：** 改了行为就更新 BuckyOSApp `doc/当前需求文档.md`。
- **公开仓库：** 不写具体用户的主机名、IP、账号。

## 11. 实施记录

实施时在这里追加：T0 的结论、每个任务的落点和验证结果、与正文不同的地方、新出现的待定问题。

- 2026-11-24：立项。§2.2 的断网实测和 §2.1 的耗时在一个真实 Zone 上做过（Linux Chromium），产品代码没有改动。
- 2026-11-24：owner 要求强制 http 另起 TODO，相关内容移到 [强制 http TODO](buckyos-app-force-http-todo.md)。
- 2026-11-24：owner 答复 Q3–Q6。据此：macOS 最低 14；iOS 与 Android 同时做，加 §3.6 和 I 系列；系统浏览器改为手工设置说明，加 §3.7 和 B 系列；Q6 不做。新增 Q9、S12–S14、T0-10、T0-11。为核对 Q5 在 macOS 测试 VM 上实测了系统代理和 `proxyConfigurations` 断网的表现（§2.2）；Windows 没测成。
- 2026-11-24：任务都已写完，未提交；I2–I6 没有编译过，T0-4、T0-11 没做（BuckyOSApp、buckyos、cyfs-gateway、`src/publish` 子模块）。下面按“落点、T0 结论、验证、与正文不同、替 owner 选的、等 owner 的、没验证的、提交顺序”记录。

实施基线：BuckyOSApp `7153768`、buckyos `1704812f8`、cyfs-gateway `e80b6e28`。与立项时的差别：iOS 内容视图已合入 main（PR #54，iOS 17、仅 iPhone），所以 I 系列直接做了。

### 11.1 落点

| 范围 | 文件 |
| --- | --- |
| 选路、探测、代理、PAC（不依赖 Tauri，主进程和 `:notify` 共用） | BuckyOSApp `src-tauri/src/route/`：`mod.rs`（选路）、`tls.rs`（探测）、`net.rs`（出站、扫描、绑 Wi-Fi、系统代理）、`proxy.rs`、`pac.rs`，测试 `tests.rs` 和 `testdata/`（自签的测试证书） |
| 接到客户端 | `route/client.rs`（事件、地址记录、网络变化、命令 `zone_lines` / `zone_find` / `route_status` / `route_set_browser`）、`route/webview.rs`（三个桌面系统）、`http_client.rs` 的 `zone_builder`、`zone/registry.rs` 的 `route` 字段、`content/windows.rs` 的 `content_lines` |
| Android | `Wifi.kt`、`ClientPlugin.kt`（`setProxy`、`localNetworkPermission`）、`Downloads.kt`、`NotifyService.kt`、`notify/service.rs`、`AndroidManifest.xml` |
| iOS | `ios/ClientPlugin/` 的 `Profiles.swift`（`proxyConfigurations`）、`Client.swift`（回前台前重新监听）、`ContentView.swift`（失败带错误码） |
| 客户端界面 | `src/features/zone/{lineView,lineEvents}.ts`、`LineNotice.tsx`、`LineSection.tsx`、`src/features/route/`、`src/pages/main/BrowserSection.tsx`、`src/windows/ContentStatus.tsx`，9 种语言 |
| 节点 | buckyos-api `runtime.rs`；node_daemon 新增 `app_sessions.rs`、`background_call.rs`，改 `node_daemon.rs`、`zone_boot_resolve.rs`、`domain/`、`run_plist.rs`；verify_hub；control_panel `zone_mgr.rs`；`src/apps/sys_test/prepare.mjs` |
| 网关 | `cyfs-acme/src/cert_mgr.rs`（N2）；`cyfs-gateway-lib` 的 tcp / tls / rtcp 栈（N7） |
| 前端 | Desktop `src/api/{zone_link,useZoneReachable}.ts` 及各处改用它；OpenDAN `lib.ts`、`main.tsx`；AIWS `api/`、`offline/` |
| 其它 | 安装包 `src/publish/make_local_osx_pkg.py`（C8）、`doc/port_usage.md`、BuckyOSApp `doc/当前需求文档.md` §9.13 |

### 11.2 T0 的结论

| 项 | 结论 |
| --- | --- |
| T0-1（Windows 10，WebView2 158） | 用环境变量 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 加 `--proxy-pac-url`，所有窗口一致，也与调试参数共存。禁用网卡后页面、跨子域 fetch、Service Worker、WebSocket、Worker、下载、断网冷启动都正常。**PAC 只在浏览器进程启动时读一次**：客户端运行中新加的 Zone 要等重启才走代理，在那之前照系统线路（见 §11.4 第 1 条）。系统静态代理：Zone 仍走本机，其它网站和 Zone 的系统线路经系统代理。系统用自动代理脚本时页面不装 PAC，客户端自己的请求仍直连 |
| T0-2（macOS 26） | 真实 Zone、https：`en0 down` 后已打开的页面、新窗口、POST、断网冷启动都正常，`navigator.onLine` 为 `false`。窗口建好以后再设 `proxyConfigurations` 生效。只连回环时没有弹本地网络权限；连局域网地址会弹，授权前报 `No route to host`。没有测：macOS 14、Service Worker |
| T0-3（模拟器 API 35，WebView 124） | `ProxyController` 对各 profile 都生效，跨子域 fetch、Service Worker、WebSocket、`Downloads.kt` 的下载都走代理。WebView 没有反向绕过能力时不装，界面写明不支持 |
| T0-5 | 只做了 Linux（测试 VM，只放行测试机）：断网冷启动 22 秒后可用，容器应用都起来，后台刷新 Zone 文档失败只记日志。Windows、macOS 没有做整机断网重启 |
| T0-7 | 真实 Zone 上复现：旧版时钟拨快 8 天，全部服务停在续期失败，`zone.presentation.get` 报 `JWT decode header error`；新版 5 秒内自动恢复，443 没有断，应用容器自动重建。**原因比 §2.3 写的更宽**：会话过期（约 15 分钟）就会卡死，不必等 7 天（见 §11.4 第 8 条） |
| T0-10 | Windows Edge：联网、去掉默认路由、禁用网卡三种都能打开本机 Zone；客户端退出后照原来的走法；网页读不到 PAC。macOS（URLSession，系统 PAC）：连着网但去掉默认路由时走代理，正常；没有任何网络时系统忽略 PAC，与 §2.2 一致；客户端退出后直连。没有测 Chrome、Firefox、Safari 界面 |
| T0-4、T0-11 | 没有真机，没做。这台模拟器没有 Wi-Fi，绑定 Wi-Fi 的代码没有执行到真正的绑定 |

### 11.3 验证

| 内容 | 结果 |
| --- | --- |
| 单测 | BuckyOSApp Rust 132、前端 183；buckyos-api、verify_hub 49、node_daemon 255、control_panel zone 17；cyfs-acme 24、cyfs-identity-cert 38、cyfs-gateway-lib 498；scheduler 92 + 21 |
| 桌面回归（真实三 Zone） | 核心 29、规则 9、攻击 5：macOS、Windows 全过（Windows 另有音乐 5）；Linux 核心 29 |
| 直连，Linux | 本机线路、局域网线路、断网继续、断网新开、断网冷启动、节点重启等待、60 秒后换线路、只用互联网，共 23 项；第一次 22 过，“本机恢复后 1.5 分钟内切回”不过，已改（桌面走局域网时也每 60 秒看本机），改后由单测覆盖，没有重跑这一条 |
| 直连，macOS 真实本机 Zone | 断网 17 项全过 |
| 直连，Windows 模拟 Zone（最小的 TLS 服务，不是真实节点） | 26 项过 25；不过的是 §11.4 第 1 条已知的限制 |
| 直连，Android 模拟器 | 断外网继续、新开、冷启动登录、`:notify` 连着、离开 5.9 秒切到互联网、回来 59 秒切回局域网，18 项全过一次；最后一版重跑 17 项，不过的一条是 `:notify` 进程还占着一条经 SN 的连接，它按设计不主动找（§11.4 第 6 条） |
| Android 回归 | 核心 29、其他 16、安全 20、通知 21 |
| N7 | 测试 VM 上重启 node_daemon：443 断 1 秒，接口 8 秒内恢复。从旧版升级的那一次在 Linux 上仍要等约 1 分钟 |
| N3 | 出站全丢时主循环：旧版每 30 秒卡 6.6 秒，新版不超过 1.6 秒 |
| Desktop | e2e 785 过 / 40 失败，逐条对照基线：35 条基线也失败，4 条是改源码触发热更新、重跑过，`my-network.spec.ts:350` 两边都不稳（基线 1/12，改后 4/12，没有完全排除） |

断网时 MessageHub 在 macOS 的旧版 Zone 上显示“Waiting for network…”，这就是 F1 要改的；F1 之后的版本没有在真实 Zone 上看过界面。

### 11.4 与正文不同

1. **§3.3 Windows：** PAC 不会随 Zone 列表更新（Chromium 读成功后 12 小时才再读）。改成预先包含 `*.web3.<SN 域名>`：激活出来的 Zone 立即直连；自有域名的新 Zone 要重启客户端。因此代理多放行一类主机：SN 托管域名下还没登记的主机，只走系统线路。
2. **§3.1 端口：** Zone 主机的其它端口不拒绝，走系统线路。macOS、iOS 只能按域名匹配，拒绝会比现在差。
3. **§3.2 切换：** 不是“空闲 30 秒后关”。一直在轮询的页面永远不空闲，会一直留在 relay 上（实测）。改成换线路 30 秒之后，在第一个 3 秒没有数据、也没有未答复请求的间隙关闭。
4. **§3.2 失效判断：** 另加一条：局域网连接上发出请求 5 秒没有任何回应就重新探测。没有网络变化事件（例如 OOD 关机）时靠它。
5. **§3.2 网络变化：** 3 秒后再探测一次，刚连上的网络当时还不通。
6. **§3.4 `:notify`：** 不每 60 秒探测，只在网络变化、连接失败和 15 分钟心跳时重新选，省电。
7. **§3.4 局域网地址：** 包括 100.64/10（Tailscale 一类）和 IPv6 ULA，不包括公网地址。
8. **N1：** 内核服务会话一过期就读不到 system-config，refresh 请求根本发不出去，所以要改的不只是“refresh 被拒”。内核服务多数不加载设备私钥，改为重新登录那一刻临时读取。
9. **N1，应用和 Agent：** LoginAssertion 只在 node_daemon 启动实例时签，运行中的实例断开 7 天后无法恢复。node_daemon 与 Zone 失联满 7 天（减 1 小时）后，把目标为 Started 的 App 各重启一次。
10. **N1，verify_hub：** 设备公钥缓存在进程里，移除的设备仍能登录；有了自动重登，移除会失效，所以登录和续期都先查设备还在不在。
11. **N2：** 失败重试在证书还剩 1 天以上时最慢 1 小时一次（原来 10 分钟），否则窗口变长后会碰到 Let's Encrypt 连续 1152 次失败的暂停。ARI 评估后没接。
12. **N4：** 只对“本机是唯一 OOD”的节点先用 LKGS 启动；多 OOD 和非 OOD 仍先解析。刷新到的变化只写 LKGS 供下次启动用，不自动重启。
13. **N6：** 没有会清掉在用镜像或包的逻辑，不用改；只把 sys_test 的依赖随包（`vendor`）。
14. **N7：** 没有在配置里写 `reuse_address`（旧网关收到会整个 reload 失败），改成网关在 Unix 上默认开；没有做“不重启网关”。
15. **F2：** SWR 的 `isOnline` 恒为 true，恢复触发用 Zone 连通性，不把轮询停掉。
16. **顺带：** 客户端主进程和 `:notify` 进程的登录 nonce 用奇偶错开。两边同一毫秒登录同一账号会被当成重放而吊销会话，走局域网后更容易撞上（测试中出现过一次）。

### 11.5 实施中替 owner 选的

- Q9 视为已定：main 上 iOS 已是 17。
- 没有新引入第三方代码：`rustls`、`tokio-rustls`、`rustls-native-certs`、`webpki-roots`、`hyper-util`、`http`、`webkit2gtk` 原来就在依赖树里，现在直接声明。
- 已连接时 Home 写“已连接 · 本机”；本机没有运行或连不上时只写这一句。
- 找到同网段设备的按钮只在线路是互联网时出现。
- 系统代理的用户名密码只在代理自己出站时带上。
- `pages` 不支持（旧 WebView、Windows 上有自动代理脚本）时，客户端自己的请求仍直连。

### 11.6 owner 的答复与等 owner 的

2026-11-24 owner 已答：

| 问题 | 答复 | 落实 |
| --- | --- | --- |
| N1：失联约一周后自动重启该节点全部 Started 的 App | 好 | 维持 §11.4 第 9 条 |
| N2：续期失败通知放宽到失败满 24 小时 | 好 | node_daemon `domain/runtime.rs` 的 `RENEWAL_GRACE_SECS`。只放宽现有证书还能用 24 小时以上的续期；从没签发成功，或一天内到期的，仍按 30 分钟（`ISSUANCE_GRACE_SECS`），否则通知到的时候已经打不开 |
| N5：非 OOD 节点上的 AppClient 也默认走本机网关 | 可以 | 维持现状 |

同日第二批答复（原来的 7 条）：

| 问题 | owner 的答复 | 落实 |
| --- | --- | --- |
| 1. N1：AppService“刷新结果未知 8 秒后终止” | 可以放宽到 30 秒 | buckyos-api `verify_hub_client.rs` 的 `REFRESH_RECOVERY_SECONDS = 30`。verify-hub 的旧令牌宽限要跟着放宽（10 → 35 秒），否则 8 秒之后重试会被当成旧令牌重用而吊销会话；两边用同一个常量，文档三处同步 |
| 2. N2：从没签发成功的证书 10 分钟重试、约 10 天会被 Let's Encrypt 暂停 | Settings 的集群管理要能看到证书状态，自动停止后可以手工再启动 ACME | 见下面“证书停止与重新申请” |
| 3. N4：只对“本机是唯一 OOD”先用 LKGS 启动，变化不自动重启 | 好 | 维持 |
| 4. sys_test 要不要升版本号 | 升级 | 0.5.2：`dapp_meta`、`bucky_project.yaml`、`boot.template.toml`、`product/lite.json`（Lite 只放行清单里的版本）。`rootfs/local/did_docs` 里那份原来就与实际包不一致（缺 presentation、buckets，pkg_objid 不同），没有动 |
| 5. websdk 静默断流重连 | 好 | `kevent_client.ts` 的 `BrowserKEventReader`：连续 3 个 keepalive 周期没收到数据就关，订阅者重连。按周期计数，页面冻结后恢复不误关 |
| 6. MessageHub `checkIdentity` 误判 | 修复 | `api/store.ts`：读不到会话、而 SDK 存的身份仍是本账号时，不当成换账号；退出、401、换账号照旧 reload |
| 7. Desktop 恒显示“中继 / Relay” | 先删除 | 连得上只写“浏览器”等容器名，连不上才加“Zone 连不上”；状态点连得上是绿色（浏览器里原来是黄色） |

证书停止与重新申请（第 2 条）：

- 网关：一直没有可用证书的名字，签发累计被拒 144 次（按 10 分钟一次约一天）后停止，`acme.state` 为 `stopped`。离线、网络错误、5xx、限额（429）不计，休眠的笔记本不会误停。次数落盘，重启网关仍停止。停止期间仍每 60 秒看有没有上传的证书。已有证书的续期不停止。控制命令 `restart_cert_order {host}`。
- node_daemon：`domain.restart_certificate {host}`（sudo）；停止时通知 owner；`domain.update` 成功后自动解除停止。control_panel 转发 `zone.domain.restart_certificate`。
- Desktop：Cluster Manager 的证书卡片显示签发状态、上次错误、下次重试和“重新申请”，没有证书的名字也列出来；Zone Config 同一个按钮。只有 Zone owner 看得到签发状态。9 种语言加英文。

验证：cyfs-acme 28、cyfs-gateway-app-lib 80、`test_cert_status` 通过；buckyos-api 428、verify_hub 49、node_daemon 256、control_panel 236、scheduler 92 + 21；websdk kevent 5（全量 1404 过、2 不过，见下）；Desktop `tsc`、eslint、settings 两个 e2e 文件 30 项、zone-link 等共 61 过 3 不过；MessageHub 身份用例新增一条。sys_test 0.5.2 打包通过。

不过的：websdk `app-client/runtime.unit` 的 ownerUserId（HEAD 上同样不过）、`file_meta` 的 dk10000.pdf 样本（与这次改的文件无关，没能对照基线）；Desktop e2e `desktop.spec.ts` 的 SN Router（已知基线失败）、systest 窗口地址多了 `?locale=` 和 `#bfp=`、“设置已保存”的提示带来第二个“关闭”按钮，都不涉及这次改的代码，没有对照基线；datamodel 的 `desktop-folder`、`messagehub-guest` 在 HEAD 上同样不过，`messagehub-search` 引用的文件已不存在。

这一批没有部署，没有在真实 Zone 上验证；真实 Let's Encrypt 的停止和重新申请也没有验证。

### 11.7 没验证的

iOS 全部（没有 Xcode，Swift 只有 `Profiles.swift` 用 macOS SDK 做过类型检查，Rust 的 iOS 分支没有编译）；Android 真机、Wi-Fi 无外网加蜂窝、VPN、API 37 的局域网权限；macOS 14、Windows 11、正式安装包；抓包；Windows 上的真实节点；休眠唤醒（只拨过时钟）；真正的飞机或酒店 Wi-Fi；N8 的真实断网表现；F 系列在真实 Zone 上的界面；真实 Let's Encrypt 续期。

### 11.8 提交和发布顺序

客户端、节点、网关互不依赖，可以分别发布（例外：证书的“重新申请”要新网关，旧网关会报 `gateway_unavailable`；websdk 的断流重连要先发 npm，`dist` 随源码提交）。verify-hub 和 buckyos-api 的刷新窗口要同一版发布，不要只升一边。网关不需要配套改配置；N2 赶在 2027-02-10 之前发布。C8 的安装包改动在 `src/publish` 子模块（buckyos-pack-installer）里，要单独提交。发布说明写明：Linux 上升级到这一版网关的那次重启仍可能等约 1 分钟。

重跑方法：开发机 `/home/lzc/buckyosapp-dev/` 的 `direct/`（`run.sh`、`mockzone.mjs`、`watch.ps1`）、`e2e/direct*.mjs`、`e2e/edge-win.mjs`、`macdrv/direct_mac.py`、`macdrv/pac_mac.py`、`android/e2e/{direct,mock}.mjs`。
