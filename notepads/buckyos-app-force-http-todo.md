# BuckyOS APP 强制 http 访问 Zone 实施 TODO：https 用不了时的应急

日期：2026-11-24。状态：立项，**未实施**。按 owner 要求从 [直连 Zone TODO](buckyos-app-zone-direct-connect-todo.md)（下称直连 TODO）拆出来。§6.1 的问题等 owner 答复，§6.2 是立项时替 owner 选的做法。Q1、Q2、S5、T0-6、H0 到 H5 沿用拆分前的编号。

**依赖：** 直连 TODO 的 C1（选路）、C2（本地代理）、C3（WebView 接线），Android 另要 M1、M3，iOS 另要 I2。那些没做完之前，本文除 H1、H2、H4 外都做不了。

涉及仓库：buckyos（`boot_gateway.yaml`、control_panel、Desktop Web UI）、buckyos-websdk、BuckyOSApp。

源码核对基线：BuckyOSApp `3a691a7`（main）、buckyos `a552208d7`、buckyos-websdk `ae5e885`、cyfs-gateway `e80b6e28`。标了【实测】的在真实 Zone 上跑过（Linux Chromium），其余是读代码的结论。

用词沿用直连 TODO：**客户端**、**内容视图**、**Zone 主机**、**直连**、**系统线路**。

## 0. 结论先行

1. **owner 的要求：** 先用 https；http 在特殊情况下可以强制开启，比如证书到期了（2026-11-24）。
2. **只走直连，不上公网。** http 只用在本机（127.0.0.1）和局域网，绝不走系统线路和 SN relay。
3. **现在打开 http 也用不了。** 页面能显示【实测】，但正式安装的 Zone 不允许 http 登录，登录跳转写死 https，前端四十多个文件会在 http 下抛错（§2）。服务端、前端、客户端都要改。
4. **目标设备的 3180 端口正好做入口，但光看端口不够。** SN relay 和其它 Zone 到不了 3180；可是有公网 IP 的机器上，3180 会直接暴露在公网。所以服务端要同时看端口和来源地址（§3.2）。
5. **打开 http 会丢东西。** http 和 https 是两个源：要重新登录，草稿、离线内容、界面设置都看不到；麦克风、Service Worker、OPFS 不可用。
6. **如果原因只是证书到期，有便宜得多的办法：** 仍用 https，只放过“已过期”这一种错误（Q1）。只改客户端，旧 Zone 也有效，功能不打折。做了它，http 只剩“根本没有证书”这一种用途。**建议先定 Q1，再决定 http 做多少。**

## 1. 什么时候 https 用不了

| 情况 | 会不会发生 | 放过“已过期” | 强制 http |
| --- | --- | --- | --- |
| 证书过期：断网太久，或续期一直失败 | 会。到期前 7 天才续期，断网最少撑 7 天（直连 TODO N2 改善） | 能用 | 能用 |
| 一直没有拿到证书：激活后还没签发、签发额度用完、域名配置有问题 | 会，多在新装的 Zone 上 | 不能 | 能用 |
| 证书文件损坏，或主机名对不上（换了域名） | 少见 | 不能 | 能用 |
| 没有 443 入口：Zone 没有 SN，也没有自定义域名 | 企业版可能 | 不能 | 能用 |
| 电脑时钟错得多 | 少见 | 看偏差方向 | 登录令牌也会失败，http 解决不了 |

## 2. 现状

### 2.1 网关

- 80 和 3180 都是同一个 `server node_gateway`，绑 `[::]`，没有跳转到 https，没有 HSTS（`src/rootfs/etc/boot_gateway.yaml:79-104`）。
- 按 Host 精确匹配 Zone 主机名。`http://<ip>:3180/`、`http://<zone>:3180/` 都匹配不到应用（403【实测】）。所以只有代理（换 IP 和端口、不换 Host）能这样用，别的页面没法把浏览器引过来。
- `X-Forwarded-Proto` 由网关覆盖写：目标端口是 443 写 `https`，其余写 `http`（`:248-251`）。
- 未登录的应用访问跳到写死的 `https://sys.<zone>/login?redirect_url=https://…`（`:237`、`:272`）。
- 从 RTCP 进来的 Zone 外流量（SN relay、其它 Zone）只能到 80 和 443，80 在这里直接进 `node_gateway`。**所以在 80 上分不出直连和 relay，3180 上没有 relay。**
- 3180 的来源有：本机、容器、局域网、同 Zone 的其它设备（经 RTCP）；**机器有公网 IP 又没有防火墙时，还有公网**。`doc/port_usage.md` 写的是“建议只允许本机访问”，代码没有限制。
- 请求链里能拿到对端地址：`REQ_conn_remote_ip`（连接的真实对端）、`REQ_target_port`。

### 2.2 登录

- `force_https` 默认 `true`（`buckyos-kit/src/machine_config.rs:30`）。仓库里的 `rootfs/etc/machine.json` 写的是 `false`，但安装包不带这个文件，正式安装是 `true`。
- `force_https` 为 `true` 时，control-panel 拒绝：http 的 `redirect_url`（`sys_auth_backend.rs:1259` `parse_sso_redirect_url`）、http 的请求来源（`:930` `request_origin`，`/sso_callback` 和 `/sso_refresh` 都经过它）。
- 判断用的是全局的 `!runtime.force_https`，在 `sys_auth_backend.rs`、`app_login.rs`、`exclusive_invite.rs`、`federation.rs`、`share_session.rs`、`resource_share.rs`、`main.rs` 里各自传，没有按请求区分。
- 会话 Cookie 只在请求是 https 时带 `Secure`，没有 `Domain`（`:979` `build_host_cookie_header`）。http 下能写。
- **https 时留下的 Secure Cookie 会挡住 http 写同名 Cookie【实测】。** 同一个 WebView 里从 https 切到 http，会话写不进去。
- Google 登录（`oauth.rs`）固定用 `__Host-` 加 `Secure`，http 下不可用，断网时本来也不可用。

### 2.3 前端

http 页面不是安全上下文，`crypto.randomUUID` 等不存在【实测】。按文件数（不含测试和 sys_test）：

| 接口 | 用到的地方 | http 下 |
| --- | --- | --- |
| `crypto.randomUUID` | 约 46 个文件，Desktop 各应用 31 个 | 只有 3 处有替代，其余抛错。文件浏览器标签页、Message Hub 发消息、发起打开、任务中心、下载、文本编辑器都在内 |
| `crypto.subtle` | 约 12 个文件 | Message Hub 有替代；壁纸、许愿格、PDF 导入、文本编辑器缓存等抛错 |
| `navigator.clipboard` | 约 45 个文件 | 4 处有 `execCommand` 替代，其余不生效，少数抛错 |
| Service Worker、locks、OPFS | 约 10 个文件 | 多数有判断，不可用；文本编辑器的本地存储没有核对 |
| `getUserMedia` | 4 个文件 | 语音输入有判断；AIWS 公开课没有核对 |

websdk：登录和令牌不依赖这些接口。会抛错的有 `app-frame.ts`（预览和会话组件）、`audio.ts`、`executor_client`、`owner_schedule`、`aiws_ext`、`xllm`。写死 https 的有 `zone_clients.ts:39`、`hybrid_sso.ts:265`、`msg_center_client.ts:1453`、`object_url.ts:177`。`account.ts:86,104` 写带 Secure 的 Cookie，http 下被丢掉，网关不读它，不影响登录。

第三方应用的前端不受我们控制，http 下能不能用看它自己。

### 2.4 客户端

- `content/policy.rs:68` 拦掉 Zone 主机的 http 导航；`zone/host.rs` 的 `web_url` 只认 https。
- 写死 `https://` 的地方：`zone/api.rs:205`、`zone/mod.rs:664`、`zone/media.rs:85,134`、`notify/core.rs:493`、`content/windows.rs:447,919,1055-1056`（后两处是页面能用原生桥的来源）、`content/mobile/android.rs:100,383,480,1244`、`Downloads.kt:86`、前端 `openService.ts:179,297`、`catalog.ts:81,104,186`、`images.ts:14`。
- 麦克风权限要求来源是 https（`ContentView.kt` 的 `onPermissionRequest`）。
- Android 的 debug 和 release 包都允许明文（`build.gradle.kts:44`、`:58`）。iOS 没有放开 ATS。
- 已有按 Zone 清 Cookie 的功能（`content::clear_cookies`、`Profiles.clear`）。

## 3. 方案

### 3.1 总体

用户在客户端里逐个 Zone 打开“强制 http”。打开后，这个 Zone 的页面地址变成 `http://<zone 主机>/`，本地代理把它（端口 80）接到目标设备的 **3180**：

```
内容视图 http://<zone>/  →  客户端本地代理  →  127.0.0.1:3180  或  局域网 IP:3180
```

没有可用的直连线路时，直接报连不上，不退到系统线路。

### 3.2 服务端只对直连放开

网关判断一个请求是不是“直连来的 http”，两个条件都要满足：

1. 目标端口是 3180。
2. 连接对端（`REQ_conn_remote_ip`）是回环、私有网段或链路本地地址：`127/8`、`10/8`、`172.16/12`、`192.168/16`、`169.254/16`、`::1`、`fc00::/7`、`fe80::/10`。公网地址一律不算，包括公网 IPv6。

满足就加一个网关自己控制的标记（先删掉请求里带来的同名头，和 `X-Cyfs-Rtcp-*` 一样处理）。之后：

- 登录跳转按来路写 http 或 https。
- control-panel 看到标记，这一次请求按“允许 http”处理；没有标记时照旧看 `force_https`。80 和 443 上的行为不变。

OOD 上不另设开关。证书坏掉的时候，用户往往没有办法先进设置页去打开它；而且只有代理能造出这样的请求（§2.1）。

### 3.3 客户端

- 每个 Zone 一个连接协议，默认 https。§2.4 的各处改成从这里取。
- 打开或关闭时清掉这个 Zone 的 Cookie，用保存的密码摘要自动重新登录。不清的话 http 写不了会话（§2.2）。
- 探测时没有证书可验，改为取一次 `/1.0/identifiers/self`，核对是不是这个 Zone 的 DID。这只能防连错设备，防不了有人冒充（R2）。
- Rust 的请求、通知、下载一起切换。
- 原生桥、`buckyos://` 等凡是按来源放行的，跟着切换。签名类的桥接口在 http 页面上一律不给。

### 3.4 入口和提示

- 入口在两处：设置里这个 Zone 的“连接方式”；打不开页面时的失败页，且只在原因是证书、直连又可用的时候出现。
- 打开前说明：要重新登录；看不到原来的草稿和离线内容；部分功能不可用；在局域网上内容是明文。
- 打开期间，Home 的 Zone 状态和窗口标题上一直有“http”标记。
- 后台每小时探测一次 https，恢复正常就提示关掉。

### 3.5 前端

- websdk 入口补一个 `crypto.randomUUID`（用 `getRandomValues`，它在 http 下可用）。Desktop、OpenDAN WebUI、文本编辑器的入口最先加载它，各处代码不用改。
- `crypto.subtle` 的 SHA-256 换成纯 JS 实现，只在 `subtle` 不存在时用。签名和密钥类的用途不做。
- 剪贴板写入统一走一个带 `execCommand` 回退的函数。
- websdk 写死 https 的四处改成按页面协议取。
- 其余不可用的功能，界面上给出“需要 https”的说明，不抛错。

## 4. 范围外

- 系统浏览器里的 http 访问。
- 经 SN relay、公网、跨 Zone 的 http。
- 局域网自签证书、本地 CA。
- 第三方应用在 http 下的适配。
- iOS 上的强制 http：要放开 ATS，上架审核要说明理由。直连 TODO 已定 iOS 与 Android 同时做，所以 H0（过期证书放行）含 iOS，http 第一版不含（S15）。
- 收紧 3180 的监听范围（Q7）。

## 5. 已知风险

| 编号 | 风险 | 处理 |
| --- | --- | --- |
| R1 | 局域网上会话令牌是明文，同网的人能截到 | 用户主动打开并看到说明；按“先能力再安全”记为已知（Q2） |
| R2 | http 下不能确认对面是自己的 OOD，有人冒用它的地址时会把令牌交出去 | 本机不受影响；局域网记为已知。要补的话加一个由设备私钥签名的应答（H6） |
| R3 | 容器的来源也是私有网段，会被当成直连 | 容器本来就能经 3180 调登录接口，不增加能力 |
| R4 | 云主机的内网里，别的租户的机器可能落在私有网段 | 记为已知；这类 Zone 一般没有局域网使用场景，Q7 若收紧则一并解决 |
| R5 | 用户忘了关 | 一直有标记；https 恢复后提示 |
| R6 | 多节点时，网关之间转发会再经过一次网关，标记可能丢失或被错加 | T0-8 确认；标记只由第一跳加，后面的跳要能认出来自己人 |

## 6. 待定与已选

### 6.1 等 owner 答复

| 编号 | 问题 | 建议 |
| --- | --- | --- |
| Q1 | 证书只是到期时，先不用 http，改为仍用 https、只放过“已过期”这一种错误（主机名和证书链都对，只在直连时）？ | **是，并且先做它。** 只改客户端，旧 Zone 也有效，登录状态和本地数据都在。这会放宽 [客户端 TODO](buckyos-app-system-client-todo.md) R10 的“不做证书例外”。各平台能不能做到由 T0-6 确认 |
| Q2 | 强制 http 允许用在局域网（手机）上吗？还是只用于本机？ | 允许，打开时说明是明文 |
| Q7 | 3180 现在绑 `[::]`，有公网 IP 的机器会暴露到公网，与 `port_usage.md` 的“建议只允许本机访问”不一致。要不要收紧？ | 另开 issue：拒绝公网来源，保留本机、容器、局域网。本文不依赖它，因为 §3.2 已经看来源地址 |
| Q8 | Q1 做了以后，http 还做到什么程度？ | 做到登录、Files、Message Hub、AI 工作区能用；其余说明不可用，不逐个适配 |

### 6.2 替 owner 选的

| 编号 | 选择 | 理由 |
| --- | --- | --- |
| S5 | http 用 3180，不用 80 | 80 上分不出直连和 relay |
| S7 | 端口之外再看来源地址 | 公网 IP 的机器上 3180 是公开的 |
| S8 | OOD 上不设开关，按请求放开 | 证书坏了以后很难再去开 |
| S9 | 切换时清 Cookie，不另建一份 WebView 数据 | 现成的功能；localStorage 本来就按源隔离 |
| S10 | 补 `randomUUID` 而不是逐处改 | 一处生效，改动小 |
| S15 | iOS 第一版只做 H0，不做 http | 要放开 ATS；过期是主要情况 |
| S11 | 没有直连线路时不退到系统线路 | http 不上公网 |

## 7. 任务

### T0 验证

- [ ] T0-6 过期证书（Q1）：WebView2 的 `ServerCertificateErrorDetected`、WKWebView 的 `didReceiveAuthenticationChallenge`（`SecTrustSetVerifyDate`）、Android 的 `onReceivedSslError`、WebKitGTK 的 TLS 例外，各自能否只放过“已过期”，子资源和 Service Worker 是否正常。
- [ ] T0-8 网关：请求链能不能按网段判断 `REQ_conn_remote_ip`；多节点转发时端口和来源地址的值（R6）；Windows 和 macOS 上 Docker 的来源地址。
- [ ] T0-9 各平台 WebView 打开 `http://<zone>/`（经代理）：有没有自动升级到 https、Cookie、WebSocket、混合内容、子域 iframe。

### H 实施

先后：H0 → H1、H2、H4（可并行）→ H3 → H5。

- [ ] H0 过期证书放行（Q1 定了才做）：客户端选路把“只过期”的候选当可用，各平台 WebView 和 Rust 请求一起放过；只限直连；界面有标记。只依赖直连 TODO 的 C1 到 C3（手机另加 M1 / I2），不依赖下面任何一项。
- [ ] H1 网关：直连标记（§3.2），登录跳转按来路用 http 或 https。加载到新旧节点上都不影响 80 和 443。
- [ ] H2 control-panel：`allow_http` 从全局改成按请求（标记或 `force_https = false`），七个文件的调用处一起改；单元测试覆盖：有标记放行、没有标记拒绝、客户端伪造标记无效、公网来源无效。
- [ ] H3 客户端（§3.3）：协议逐 Zone 取、代理接 80 到 3180、清 Cookie 与自动重登、探测、来源放行。
- [ ] H4 websdk 和前端（§3.5）：补 `randomUUID`、SHA-256 回退、剪贴板、四处写死的 https；逐个列出 http 下不可用的功能。
- [ ] H5 入口、说明和标记（§3.4），9 种语言。
- [ ] H6（可选）设备签名的应答：客户端给随机数，设备用私钥签，客户端按已存的设备文档验证后才发令牌。

## 8. 验收

| 场景 | 通过标准 |
| --- | --- |
| 桌面，本机 Zone，把证书换成过期的（H0） | 提示后可继续用 https，登录状态和草稿都在 |
| 桌面，本机 Zone，删掉证书 | 失败页给出强制 http；打开后自动登录，Files、Message Hub、AI 工作区能用 |
| Android 真机，局域网，同样两种证书状态 | 同上（Q2 允许时） |
| 打开期间抓包 | 没有任何到公网的 http；没有直连线路时报连不上 |
| 经 SN relay 用 http 登录 | 与现在一样被拒 |
| 从公网直接连 3180 用 http 登录（有公网 IP 的机器） | 被拒 |
| 自己造请求带上直连标记 | 无效 |
| 关掉开关 | 回到 https，自动登录，原来的草稿都在 |
| 证书恢复 | 提示关掉 |
| 没打开开关的 Zone 和系统浏览器 | 行为与改之前一致 |

## 9. 建议顺序

1. owner 定 Q1。
2. 直连 TODO 的 C1 到 C3 做完后，先做 T0-6 和 H0，随直连的第一个版本发布。
3. T0-8、T0-9，H1、H2、H4。
4. H3、H5。

## 10. 接手时必须遵守的约束

- **http 绝不走系统线路。** 包括回落、预取、图标、通知。
- **没有标记时服务端行为不变。** 不改 `force_https` 的默认值，不放开 80。
- **标记只能由网关加。** 先删后加，和现有的 `X-Cyfs-Rtcp-*` 一样。
- **默认关。** 不因为探测失败自动打开，只提示。
- **提交顺序：** websdk → buckyos → BuckyOSApp。
- **兼容：** 旧 Zone 没有 H1、H2，客户端要能看出来，并提示这个 Zone 需要升级，不要反复重试登录。
- **文档同步：** `doc/port_usage.md`（3180 的用途）、BuckyOSApp `doc/当前需求文档.md`。
- **工作树、Git 依赖、公开仓库：** 同直连 TODO §10。

## 11. 实施记录

- 2026-11-24：从直连 TODO 拆出。拆分时更正一处：原文写“3180 上的请求只可能来自本机、容器或局域网”，有公网 IP 的机器不成立，据此加了来源地址判断（§3.2、S7）和 Q7。产品代码没有改动。
