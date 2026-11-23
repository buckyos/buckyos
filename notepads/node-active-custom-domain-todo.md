# Node Active 激活流程 TODO：自定义域名与流程简化

日期：2026-11-22。状态：立项，尚未实施。owner 已 Review 两轮，Q1–Q20 全部已定（§6），没有待裁决项，可以按 §9 开工。本期范围是**不动服务器的客户端流程优化**：SN、BNS Server、sn-business 不改（R9、§3.9）。

目标：激活时用户选“使用自己拥有的域名”后，按域名由谁托管分成三条清楚的流程：SN 托管、手工管理、DNS API 自动管理。SN 托管和 DNS API 的证书由系统自动签发和续期，手工管理由用户上传。每条流程都能走到“浏览器打开 `https://<域名>` 可用”。同时简化激活向导，并用真实可用性代替倒计时。

涉及仓库：buckyos（node_active、node_daemon、scheduler、control_panel、Desktop）、cyfs-gateway（cyfs-acme、cyfs-dns-provider、BNS 文档，都是节点侧）。cyfs-sn、bns-server、sn-business、cyfs-gateway-api、buckyos-websdk 的 SN client 不改（R9）。cyfs-acme 等节点侧代码 SN 也会编译到，改动不能改变 SN 现有的证书行为。

除 §3.5 引用的外部资料外，本文结论来自 2026-11-22 读代码，没有在真实域名上跑过。

## 0. 结论先行

1. **自定义域名有三个独立的选择。** 谁管 DNS（SN 托管 / 手工 / DNS API）、证书从哪来（自动签发 / 用户上传）、流量怎么进来（现有的 `nat` / `portmap` / `wan_dyn` / `wan`）。DNS 决定谁写记录，接入方式决定记录的内容，证书来源决定要不要 ACME。对 SN 的依赖也分三件事：中转、应答地址、应答其它记录，各自独立（Q8，§3.1.1）。
2. **现在的流程没有分叉。** 前半段让用户自己发布 `did.json`（手工方式的要求），后半段只提示委派 NS（SN 托管的做法），DNS API 没有。
3. **`did.json` 从激活流程里拿掉（R2）。** 它是设备的 device document，boot 不读它；boot 之后由 Zone 自己在 `https://ood1.<域名>/.well-known/did.json` 提供，供其它 Zone 使用。
4. **域名全部配置好才能走下一步（Q17）。** 域名步骤持续检测归属、NS、入口记录和证书挑战，全部通过才放行（§3.3.1）。激活之后只等节点启动和证书签发。
5. **手工和 DNS API 用同一份“记录计划”。** 区别是由用户写还是由节点写，以及手工模式的证书由用户上传，不设 `_acme-challenge`（Q6）。动态 IP 和 NAT 用 CNAME 指到 `<用户名>.web3.<SN>`，不需要节点自己做 DDNS。
6. **ACME 客户端用现有的 cyfs-acme，不引入第二个。** 每家 DNS 服务商只实现一次“写记录”，同时供 ACME DNS-01 和记录计划使用。
7. **引入 Rust 库 `dns-update`**（Q2），它已支持 70 多家。名单 20 家，含日本 1 家（Q4），先把代码写完，实测靠官方测试环境和贡献者（Q3），见 §3.5。
8. **上传证书不需要新的加载机制。** 网关的 TLS 栈本来就按身份路径读证书，每 60 秒重新加载。要做的是检查、写入，以及不让 ACME 覆盖它。
9. **激活结果以 `https://public.<域名>/readme` 真的能打开为准。** 这条路径会走完 DNS、转发、节点证书和网关，比打开主域名更严。DNS 在激活前已经配好（Q17、Q18），所以满 5 分钟打不开就是启动失败，自定义域名不再单列状态。
10. **最大的风险是服务商实现没有实测**（Q3）。写错会改坏用户的 DNS，所以写之前列出改动、只碰计划里的名字、没实测的标“实验”。
11. **上传的证书系统不管理**（Q13）：不续期、不提醒、不替换，也接受自签和私有 CA（Q14）。后者会让浏览器检测不到启动成功，要单独处理（§3.8.3）。
12. **不改 SN 的 API，也不改 SN 服务端代码**（R9）。需要 SN 配合的都用现有接口和现有规则完成，唯一缩水的是分区：只有新注册的账号能改（§3.9）。

## 1. 已有裁决

| 编号 | 裁决（owner，2026-11-22） | owner 批注 | 本文的落实 |
| --- | --- | --- | --- |
| R1 | 自定义域名分 SN 托管和非 SN 托管；非 SN 托管再分手工管理和自动管理（主流 DNS API + 用户的 token） | 确认，自有域名配置成SN托管要去改NS记录，并且有一个SN的挑战流程，这个要在激活流程里确认这个SN挑战能成功。 | §3.1、§3.3.1。“SN 挑战”按两件事都确认：归属挑战（`_pkx`）和经 SN 的证书挑战 |
| R2 | `did.json` 属于 owner，解析是 BNS Server 的责任，本 Zone 不做处理 | did.json如果是设备的device-document，boot流程并不依赖，boot后其它zone链接本zone才需要这个文件可以通过https获得。 | §3.2。已核对代码，boot 确实不依赖。按批注，BNS Server 不需要改 |
| R3 | 自动管理要列出主流的几家，引导用户申请 token，并支持对应的 ACME client | 主流的10-20家我想差不多够了 | §3.4、§3.5，名单 20 家 |
| R4 | 选择自有域名时，允许用户手工管理证书，当场上传（有的用户会买商用证书）；上传后检查是不是 Zone 需要的通配证书 | | §3.7。对三种 DNS 托管都开放 |
| R5 | SN 中转时，可以看到自动选择的分区，并允许自己调整 | | §3.8.1 |
| R6 | 删除填写 AI Token 和 TG 的步骤，简化流程 | | §3.8.2 |
| R7 | 去掉倒计时，显示“启动中，首次启动大概需要 2 分钟”；2 分钟后开始检测域名真实可用（`https://public.<域名>/readme` 就绪），成功后改成成功状态；超过 5 分钟没检测成功报启动失败 | Q16：从激活成功算起 | §3.8.3 |
| R8 | 成功时显示域名二维码，方便手机扫码打开验证 | | §3.8.4 |
| R9 | 本次修改不调整与 SN 之间的 API；这一期的修改范围明确是不动服务器的客户端流程优化 | 已有账号不能改分区可以接受，留 TODO 给 SN | §3.9。之前写的 `domain.prepare`、`domain.bind` 新语义、账号 token 写挑战、`zone.set_region` 和探测配置的显示名全部取消，需要 SN 后续做的写在 `cyfs-gateway/doc/SN/SN-自定义域名激活跟进-TODO.md` |

## 2. 现状

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 域名步骤 | 填域名 → SN `domain.bind` → 通过或显示 TXT 挑战 | `src/kernel/node_active/src/components/steps/DomainStep.tsx` |
| 设备文档 | 用户把签名 JWT 发布到 `https://ood1.<域名>/.well-known/did.json`，`commit_active` 在写任何数据前先校验 | `ReviewStep.tsx:187`、`active_server.rs:470` |
| 再次绑定 | `commit_active` 里再调一次 `bind_domain`，TXT 必须还在 | `active_server.rs:494` |
| 归属证明 | SN 经 DoH（默认 dns.google）查 `_pkx.<域名>`。每次都查，已有的有效绑定也不例外 | `cyfs-sn/src/api/domain.rs:95` |
| NS 委派后的 `_pkx` | SN 只应答显式写入的 `_pkx` 记录，绑定时不写，所以委派后查不到 | `cyfs-sn/src/sn_resolver.rs`（`is_explicit_only_dns_name`） |
| 证书 | 调度器只会生成 `sn-dns` 的 DNS-01；`wan` 没有 SN 地址，不生成 ACME 和 TLS 配置 | `src/kernel/scheduler/src/system_config_agent.rs:1406` |
| ACME 客户端 | 自研 cyfs-acme，支持 dns-01、http-01、tls-alpn-01；DNS provider 可以是注册的 Rust factory，也可以是 JS 包 | `cyfs-acme/src/cert_mgr.rs:994` |
| JS DNS 插件 | 只有 aliyun 一个，只会加删 ACME 的 TXT；BuckyOS 没有打包。它会覆盖同名 TXT、按最后两段截主域名（`co.uk` 会错）、请求走明文 HTTP | `cyfs-gateway/src/rootfs/etc/cyfs_gateway/acme_dns_provider/aliyun` |
| DNS 生效等待 | `sn-dns` 固定等 65 秒；JS 插件等待时间为 0 | `cyfs-acme/src/cert_mgr.rs:1267` |
| `self_cert` | relay 只在 `self_cert=true` 时转发自有域名的 HTTPS；只有 `sn-dns` provider 会上报 | `acme_sn_provider.rs:296`、`sn_relay.yaml` 的 `tls_raw_forward` |
| TLS 加载证书 | TLS 栈按身份路径读 `server.fullchain.pem` 和 `server.private.pem`，先精确目录（`<域名>`）再通配目录（`_.<域名>`），每 60 秒刷新，握手找不到时也会立即重读 | `cyfs-gateway-lib/src/stack/tls_cert_resolver.rs`、`tls_stack.rs:46` |
| ACME 与外来证书 | cyfs-acme 读同一组文件，别人装好的有效证书它会直接用，但到期前 7 天会自己下单并覆盖，不看 `server.meta.json` 的 `renewal.manager` | `cyfs-acme/src/cert_mgr.rs:222` |
| 证书安装函数 | cyfs-acme 已有校验（密钥匹配、SAN、过期）和写入身份路径的代码，只供自己用；不支持商用 CA 常用的 EAB | `cyfs-acme/src/cert_mgr.rs:498` |
| Zone 用到的主机名 | `<域名>` 和它的一级子域（`sys`、`ood1`、应用、`aiws-<id>` 等）。网关只接受单级标签 | `src/rootfs/etc/boot_gateway.yaml` 的 `get_app_info_from_req` |
| 备份 | `backup.py` 只备份设备身份目录，不含 Zone 域名的证书 | `src/rootfs/etc/backup.py` |
| relay 认设备 | 2026-10-30 起，已绑定域名的设备由 SN 用 AuthDB / BNS 数据作答，不访问该域名的公网 HTTPS。设备连入 relay 时也不联网；后台确认取不到 `did.json` 不影响隧道，取到的与当前设备文档不同才会断开 | `cyfs-gateway/doc/SN/SN-DID-Resolver.md`“SN 中继出站”一节（buckyos#710）、`doc/rtcp/rtcp.md` §3.4 |
| boot 与设备文档 | node_daemon 从本机身份目录读自己的 device document，启动时只解析 Zone Document，然后把设备文档写入 system_config。单 OOD 不解析任何设备文档；多 OOD 靠局域网发现交换 | `node_daemon.rs:1890`、`:2098`，`zone_boot_resolve.rs`，`finder.rs:780` |
| boot 之后 | 节点自己提供 `/.well-known/*`：`https://ood1.<域名>/.well-known/did.json` 就是激活时签好的设备文档 JWT。其它 Zone 校验跨 Zone 请求时要解析它 | `zone_did_resolver.rs:1587`、`:395`，`cross_zone_auth.rs:325` |
| 首次启动取 Zone Document | 先请求 `https://<域名>/.well-known/zone.json` 和 `did.json`（每次连接超时 5 秒），整步只有 10 秒；超时就不再用激活时存到本机的那份，改查 BOOT TXT，也没有就 Boot Failed | `zone_boot_resolve.rs:396`、`web_provider.rs` |
| 激活前 SN 的应答 | `dns.<SN>` 对已绑定域名应答 NS 和 SOA；A 在没有设备时回落到分配的 relay；否定应答只缓存 60 秒 | `sn_resolver.rs:1982`、`cyfs-dns/src/dns_server.rs:599` |
| 中转、地址、其它记录混在一起 | 激活时只有一个 `uses_sn_relay`（等于“不是 `wan`”），它同时决定 `ZoneDocument.sn` 和 `ddns_sn_url`；后面的隧道、地址上报、ACME、SN 自带 AI 都看这两个字段 | `active_lib.ts:320`、`active_server.rs:337`、`:362` |
| `wan` 的后果 | 两个字段都是空：不上报地址、没有 ACME 和 443 的 TLS 配置；SN 自带 AI 默认开启时，首次调度会因缺少 `ZoneDocument.sn` 报错（读代码，未实测） | `system_config_agent.rs:1426`、`system_config_builder.rs:49`、`:443` |
| SN 应答的地址 | 代码里只对 `wan_dyn` 和 `portmap` 应答设备地址，`wan` 应答 relay。但 SN 是从 mini 设备文档读 `net_id` 的，页面激活发布的 mini 文档没有这一项，按代码四种接入方式都会应答 relay，`wan_dyn` 也不例外（读代码，未实测） | `sn_resolver.rs:1982`、`:2130`、`:3302`，`device.rs:37`，`active_server.rs:1254` |
| 隧道的判断 | `net_id` 以 `wan` 开头且签名文档里有 IP 才不连 relay；向导生成的设备文档没有写 IP | `sn_zone_info.rs:38` |
| SN 写挑战记录 | 账号 token 也要带已登记的 `device_did`，首次激活前设备还没登记，写不了；所以证书试挑战放在提交时、设备登记之后（§3.3.1） | `cyfs-sn/src/api/dns.rs:262` |
| BNS Server | 对 `did:web` 返回 404 NotApplicable | `bns-server/src/lib.rs:3207` |
| SN 能存的记录 | 只有 A、AAAA、TXT | `cyfs-sn/src/sn_auth.rs:173` |
| 分区 | 只有网页路径有，在注册账号那一步选，注册后锁定；钱包路径没有。SN 只在 `auth.register` 接受 `region`，没有改分区的公开接口；relay 管理器内部支持改派和迁移窗口，节点会定期刷新 relay | `SecurityStep.tsx`、`cyfs-sn/src/api/auth.rs:252`、`relay_mgr.rs:2553`、`node_daemon.rs:1178` |
| 分区数据 | 节点已能测出每个分区的延迟（`score_ms`）；SN 下发的只有 `region_id`，没有显示名 | `src/kernel/node_daemon/src/region_probe.rs` |
| AI 步骤 | 填 5 家厂商的 token，写进 `start_config.ai_provider_config`，scheduler 转成 AICC provider。SN 自带的 AI（`llm_router`）与它无关：网页有邀请码、钱包路径都默认开启 | `system_config_builder.rs:793`、`:846` |
| TG 步骤 | 填的是 owner 的 Telegram 账号 ID，不是 Bot Token。激活后可在“用户与 Agent”里填，Agent 创建向导缺身份时已有“去填写”的按钮 | `OwnerTelegramStep.tsx`、`agent-setup/ChannelPage.tsx` |
| 成功页 | 固定 120 秒倒计时，到点就当成功，不检测 | `SuccessStep.tsx:15` |
| `public.<域名>/readme` | 文件随安装包带到 `data/srv/publish/readme`，网关匿名提供。本机实测返回 200，**没有跨域响应头** | `src/rootfs/data/srv/publish/readme`、`boot_gateway.yaml:236` |
| 激活页的生命 | `commit_active` 成功 2 秒后 node_daemon 退出，3182 关闭；之后页面只剩浏览器里已加载的代码 | `active_server.rs:652` |
| 二维码 | 全仓库没有二维码组件 | — |
| 完全不用 SN | `src/active.ts` 命令行，只支持 `wan` | `product/node_active/Node_Active_无SN命令行激活.md` |

## 3. 方案

### 3.1 三条流程

```text
填域名
  │  节点查：当前 NS、是否根域名、CAA、SN 上的绑定状态
  ▼
NS 已指向本 SN ───────────────> SN 托管（预选）
NS 属于支持的服务商 ──────────> DNS API（预选）
其它，子域名 ────────────────> SN 托管（预选）
其它，根域名 ────────────────> 手工（预选），同时提示可改用子域名走 SN 托管

SN 托管：加 1 条 _pkx TXT ─> 归属通过 ─> 加 1 条 NS ─> NS 生效、证书挑战可用 ─┐
DNS API：填 token ─> 节点写记录 ─> 权威 NS 上全部可见、证书挑战可用 ─────────┼─> 下一步 ─> 激活
手工：  加 3 条记录 + 上传证书 ─> 记录逐条通过、证书检查通过 ───────────────┘

证书：SN 托管、DNS API 默认自动签发和续期，也可以改成上传（R4）；手工只能上传（Q6）
```

三个选项始终都能改选，预选只是默认值。三种托管与四种接入方式可以任意组合（Q8），只有 SN 托管不支持根域名（Q7）。

**SN 托管**：用户在自己的 DNS 后台一共写两次，`_pkx` TXT 和 NS。

- 先 TXT 后 NS 的顺序不能变：NS 委派不带用户身份，只看 NS 会让别人抢先绑定一个已经委派过来的域名。
- NS 在归属通过之后、激活之前加（Q18），没有“稍后再加”，检测到生效才能走下一步（Q17）。旧流程放在激活之后，是因为 `commit_active` 还要再调一次 `domain.bind`；改成先查 `user.get_profile`，已经绑定就不再调（T2-2）。
- 证书挑战分两步确认（R1 批注）：域名步骤确认委派从公网看已到 SN，提交时再真写一次试挑战，见 §3.3.1。
- 根域名（域名就是 SOA 所在名）不提供，由向导拦住：SN 只支持 A、AAAA、TXT，委派根域名会丢掉 MX 等记录，用户的邮件会中断。提示改用子域名，例如 `home.example.com`。

**手工**：用户自己管 DNS，也自己管证书（Q6）。一屏列出 3 条记录，每条有“复制名称”“复制值”和实时状态；证书按 §3.7 上传。没有证书的用户引导到另外两种。用户要离开页面去 DNS 后台写记录，回来后进度不能丢（Q10）。

**DNS API**：选服务商 → 按教程申请 token → 粘贴 → 节点试写 `_pkx`。试写成功同时证明 token 可用和域名归属，不另做 token 校验。token 由节点长期保存，用来定期续证和修复记录，没有“只用一次”（Q11）。

#### 3.1.1 对 SN 的依赖分三件事（Q8）

| 事情 | 由什么决定 | 规则 |
| --- | --- | --- |
| 流量经 SN 中转 | 接入方式 | `nat` 全部；`portmap` 只有 HTTP(S)，RTCP 直连（Q12）；`wan_dyn` 平时直连，保留现有的后备隧道；`wan` 不用 |
| SN 应答地址（A / AAAA） | 名字是否由 SN 解析：SN 托管，或记录 CNAME 到 `<用户名>.web3.<SN>` | 内容看接入方式：`nat`、`portmap` 是 relay；`wan_dyn` 是上报的公网 IP；`wan` 是签名的固定 IP |
| SN 应答其它记录（证书挑战等 TXT） | 只有 NS 委派给 SN，即 SN 托管 | 手工和 DNS API 不用 SN 的这一项 |

所以 `wan` 也可以 SN 托管：SN 应答固定 IP 和证书挑战，不中转。要把代码里的一个布尔值拆开：

- 走页面激活的都有 SN 账号（Q9），`ZoneDocument.sn` 和 `ddns_sn_url` 一律写入，`uses_sn_relay` 删除。没有 SN 的企业版走激活脚本，不受影响。
- `wan` 把用户确认的公网 IP 写进签名的 `DeviceDocument.ips`，现有判断就不会再连 relay。IP 会变的用户应选 `wan_dyn`。
- 隧道只看接入方式；ACME 只看 `domain_hosting`（§3.4）；地址和 `self_cert` 上报对所有接入方式都做。
- 地址应答用 SN 现有的规则，改的是激活发布的文档（T5）：`wan` 在 Zone 文档写 `gateway_ips`，SN 优先应答它；`wan_dyn` 在 mini 设备文档写 `net_id`，SN 就会应答上报的 IP；`nat`、`portmap` 不写，SN 应答 relay，Q12 由此解决。
- 顺带修好 `wan` 加 SN 自带 AI 的首次调度报错。

### 3.2 拿掉 `did.json`（R2、Q1）

`https://ood1.<域名>/.well-known/did.json` 就是 OOD 的 device document。逐个环节核对，boot 都不需要它：

| 环节 | 是否需要公网上的 `did.json` |
| --- | --- |
| 节点启动 | 不需要。自己的设备文档从本机读，启动只解析 Zone Document |
| 节点连 relay | 不需要。relay 用 SN 自己的 AuthDB / BNS 数据认设备（buckyos#710） |
| relay 转发到节点 | 不需要，同上 |
| 其它 Zone 访问本 Zone | 需要，boot 之后由 Zone 自己经 HTTPS 提供，不用用户发布 |

唯一依赖它的是激活流程自己：`commit_active` 在写任何数据前和 BNS 发布后各检查一次。这两次检查是 #710 之前的要求，现在可以去掉。

激活流程：

- 向导删除“生成签名文档”“下载 did.json”和“验证发布并激活”。自定义域名的激活与默认域名一样，在点“激活”时签名。
- `commit_active` 对 `did:web` 设备删除两次 `wait_for_device_authority`。BNS 里的设备文档就是 relay 要用的，`wait_for_bytes_projection` 已经确认它发布成功，不用再向 SN 核对。`did:bns` 设备保留第二次检查。
- BNS Server 不改，继续不回答 `did:web`。
- `BNS-API.md` 第 259–276 行要求用户在独立静态站点发布，改为“由 Zone 在 boot 之后提供”。

代价：节点离线、或证书还没签下来时，其它 Zone 解析不了 `did:web:ood1.<域名>`。按批注这可以接受。

### 3.3 记录计划

手工和 DNS API 共用一个纯函数：输入域名、接入方式、SN 域名、用户名、公网 IP、owner 公钥，输出应有的记录。

| 记录 | `nat` / `wan_dyn` / `portmap` | `wan` | 写入时机 |
| --- | --- | --- | --- |
| `_pkx.<域名>` TXT | `PKX(<owner 公钥 x>)` | 同左 | 激活前 |
| `<域名>` | CNAME `<用户名>.web3.<SN>` | A / AAAA 公网 IP | 激活前 |
| `*.<域名>` | CNAME `<用户名>.web3.<SN>` | A / AAAA 公网 IP | 同上 |

- **为什么用 CNAME：** relay 按绑定关系路由，不要求 NS 委派；SN 已经为 `<用户名>.web3` 做动态 IP 解析，relay 换 IP 时也不用改。这样 DNS API 模式不需要节点自己探测公网 IP 和做 DDNS。
- **根域名：** 根域名一般不能写 CNAME。`wan` 用 A / AAAA 没问题；其它接入方式要求用子域名，DNS 在 Cloudflare 的除外（Q7）。
- **冲突处理：** 计划中的名字已有不同类型或不同值的记录时，手工模式标红说明，DNS API 模式先列出将要改动的记录，用户确认后再写。不静默覆盖。
- **公网 IP（`wan`）：** 默认取网卡上的公网地址，用户可以改；同一个值写进设备文档（§3.1.1）。
- **CAA：** 自动签发时，域名或上级有 CAA 且不允许 Let's Encrypt，证书会一直签不下来，所以不放行，提示改 CAA 或改用上传证书。上传证书不检查 CAA。
- **手工模式没有 `_acme-challenge`（Q6）。**
- **不再需要的记录：** BOOT / DEV TXT 不写。首次启动用激活时留在本机的 Zone 文档（Q20）。旧文档里手工写这些 TXT 的描述要改（T7-1）。

#### 3.3.1 检测与放行（Q17、Q18）

域名步骤每 10 秒自动检测一次，不用用户点“再次验证”。全部通过前“下一步”不可用。

| 检测项 | SN 托管 | 手工 | DNS API | 怎么查 |
| --- | --- | --- | --- | --- |
| 归属 | ✓ | ✓ | ✓ | 权威 NS 上看到 `_pkx` TXT，再由 SN `domain.bind` 确认；`user.get_profile` 的 `user_domain` 已是这个域名就算通过（NS 委派后 `_pkx` 查不到，T2-2） |
| NS | ✓ | — | — | 父域权威 NS 对 `<域名>` 的委派只有 `dns.<SN>`；再经公共递归解析器（校验 DNSSEC）查 `<域名>` 的 SOA，应答来自 SN 且认这个域名 |
| 入口 | ✓ | ✓ | ✓ | `<域名>`、一个随机一级子域，以及 `ood1`、`sys`、`public`，都与计划一致 |
| 证书挑战 | 自动签发时，提交时做 | — | 自动签发时，域名步骤做 | 试挑战，见下 |
| CAA | 自动签发时 | — | 自动签发时 | 允许 Let's Encrypt |
| 上传的证书 | 选上传时 | ✓ | 选上传时 | §3.7 没有“拒绝”项 |

- **试挑战：** 往挑战名写一个随机 TXT，经公共递归解析器查 `_acme-challenge.<域名>`，看到同一个值就通过，然后删除。它按 CA 的路径走一遍，NS、DNSSEC 和写权限有一处不对都会暴露。
- **试挑战在哪一步：** DNS API 在域名步骤由服务商写。SN 托管要用现有的 `user.add_dns_record`，它要求设备已登记，所以放在 `commit_active` 里 `device.register` 之后、写本机身份之前；失败就不完成激活，停在确认页显示原因，可以重试。登记不提前到域名步骤做：它会替换 `ood1` 的绑定，用户重新激活又中途放弃时会影响正在运行的 Zone。
- **为什么单独查 `ood1`、`sys`、`public`：** 更具体的旧记录会盖住通配记录。旧流程正好要求用户给 `ood1` 单独建记录去放 `did.json`，留着它会让其它 Zone 取到旧设备文档，relay 也会因此断开隧道。
- **先权威、后公共：** 权威 NS 上看到之后才问公共解析器，避免“没有这条记录”被缓存。
- **Cloudflare：** 记录开了代理（橙色云）时不放行，提示改成“仅 DNS”。根域名 CNAME 会被展开成 A，改为与 `<用户名>.web3.<SN>` 的 A 比较；激活前 SN 已应答 relay 地址，T0-3 确认。
- **失败要说清楚：** 每一项显示期望值、实际值和下一步该做什么。一直等待时说明服务商可能要几分钟到几小时才生效。
- **后端也要把关：** `commit_active` 提交时由节点重新检查一遍，不信任页面状态。
- 改域名、托管方式、证书来源或接入方式后，相关检测项重新开始。

### 3.4 DNS provider 与 ACME

新建 `cyfs-gateway/src/components/cyfs-dns-provider`，一个 trait，供三处使用：

```rust
trait DnsRecordProvider {
    /// 覆盖整个 RRset；空值表示删除
    async fn set_rrset(&self, name: &str, ty: RecordType, ttl: u32, values: Vec<String>) -> Result<()>;
    /// 只加、只删指定的值，不动同名的其它值
    async fn add_values(&self, name: &str, ty: RecordType, ttl: u32, values: Vec<String>) -> Result<()>;
    async fn remove_values(&self, name: &str, ty: RecordType, values: Vec<String>) -> Result<()>;
}
```

- **使用者：** node_daemon 的激活服务（写记录计划）、cyfs-acme（DNS-01）、激活后的检测（修复缺失记录）。
- **实现：** 默认包装 `dns-update`（Q2）。`0.6.2`（2026-11-13），Apache-2.0 或 MIT，上面三个操作与它的 `set_rrset` / `add_to_rrset` / `remove_from_rrset` 一一对应，支持 A、AAAA、CNAME、NS、TXT。它不带的服务商自己写一个实现。
- **主域名识别：** `dns-update` 要调用方给出 origin。我们沿域名向上查 SOA 得到，不按最后两段截取。
- **续期：** Let's Encrypt 的证书 2027-02-10 起 64 天、2028-02-16 起 45 天（[来源](https://letsencrypt.org/2025/12/02/from-90-to-45)），由节点用 token 定期续（Q11）。
- **ACME 接入：** 在 cyfs-acme 注册一个通用 factory，把 `add_challenge` / `del_challenge` 映射到 `add_values` / `remove_values`。`<域名>` 和 `*.<域名>` 共用一个挑战名，必须按值增删。
- **生效等待：** 改为轮询权威 NS，看到期望的 TXT 再通知 CA，上限默认 5 分钟。固定等待只作为 `sn-dns` 的现状保留。
- **`self_cert` 上报：** 从 `sn-dns` 移到通用位置。以 TLS 栈实际加载到 `<域名>` 和 `*.<域名>` 两张证书为准，且设备登记在 SN 时上报，与证书来源无关。否则 DNS API 和上传证书时，`nat` 的 HTTPS 会被 relay 拒绝。
- **JS 插件：** 保留机制给长尾用户，aliyun 插件由 Rust 实现替代后删除。
- **现成的 ACME 客户端（lego、acme.sh）不用：** 它们自带订单和证书管理，与 cyfs-acme 重复，也不管 A / CNAME 记录。

**凭据存放：** token 写入节点本地 `$BUCKYOS_ROOT/security/dns_credentials/<id>.json`，root 属主、0600。网关配置只引用 `credential_path`，与 `sn-dns` 的 `key_path` 做法一致。token 不进 `start_config.json`、不进 system_config、不进日志。续期和修复记录要长期使用，所以不能只放在浏览器里（Q5、Q11）。

**托管信息：** `start_config.json` 增加 `domain_hosting`，由 scheduler 写入 system_config，供生成 ACME 配置和设置页使用：

```json
{ "mode": "sn | manual | dns_api", "certificate": "acme | upload",
  "domain": "home.example.com", "provider": "cloudflare", "credential_id": "..." }
```

`update_node_gateway_config` 改为按 `domain_hosting.mode` 选 ACME provider，不再由“有没有 SN 地址”推断。这样 `wan` 在 SN 托管和 DNS API 下也会有 ACME。`certificate = upload` 时只生成 443 的 TLS 配置，不生成 ACME；`manual` 只能是 `upload`。

### 3.5 服务商名单

“核对”列：✅ 已看官方页面；◐ 只看过 lego 文档、第三方或源码；空白未核对，实施时补。

名单共 20 家（R3）。20 家都写代码、进向导、有申请教程。DNS token 敏感，不等账号（Q3）：先用 mock 和官方测试环境验证，真实环境由贡献者用自己的账号测；测过之前标“实验”。

**官方测试环境**（2026-11-22 看过官方页面）：

| 服务商 | 测试环境 | 注意 |
| --- | --- | --- |
| Porkbun | 免费沙箱，key 以 `pk1_sb_` 开头，地址与正式环境相同（[文档](https://porkbun.com/api/json/v3/documentation)） | |
| Name.com | `https://api.dev.name.com`，用户名加 `-test`，单独的测试 token（[文档](https://docs.name.com/guides/testing-environment)） | 记录接口可用，但不会出现在公网 DNS；域名要先在沙箱注册 |
| Gandi | `https://api.sandbox.gandi.net/v5/`，含 LiveDNS（[文档](https://api.sandbox.gandi.net/docs/sandbox/)） | PAT 要在沙箱后台创建；只有 .com / .net / .org / .fr |
| INWX | 免费的 OT&E 测试系统 `ote.inwx.com`（[说明](https://www.inwx.com/en/offer/api)） | API 地址要在实施时确认 |
| TransIP | 官方公开的 demo token，测试模式只校验不生效（[文档](https://api.transip.nl/rest/docs.html)） | 能验证请求格式，验证不了写入结果 |

Cloudflare 没找到沙箱。GoDaddy 的[快速入门](https://developer.godaddy.com/getstarted)写明所有请求都进生产环境。其余没查。这些环境都不解析到公网，只能验证接口，验证不了签发证书。

**第一组（8 家）**

| 服务商 | 用户要填 | 权限能收到多窄 | 门槛与注意 | dns-update | 核对 |
| --- | --- | --- | --- | --- | --- |
| Cloudflare | API Token | 可以只选一个域名的 DNS 编辑 | 有[预填链接](https://developers.cloudflare.com/fundamentals/api/how-to/account-owned-token-template/)，权限 key 为 `dns`；域名要用户在页面上自己选。动态 IP 用户不要设 IP 过滤 | 有 | ✅ |
| GoDaddy | Personal Access Token | token 有 scope；能否限到单个域名没看到 | 2026-04 起账户有 1 个域名即可用（[公告](https://www.godaddy.com/resources/news/godaddy-dns-api-now-works-with-a-single-domain)）。旧的 sso-key 在[认证文档](https://developer.godaddy.com/docs/api-users/auth)里标为“Deprecated for Domains (through 2026)” | 有，但用的是 sso-key，**要自己补 PAT** | ✅ |
| AWS Route 53 | Access Key ID、Secret | IAM 策略可限到单个 hosted zone 和记录名 | 步骤多，提供可复制的 IAM 策略 | 有 | ◐ |
| Porkbun | API Key、Secret Key | 可限域名、权限和来源 IP（[文档](https://porkbun.com/api/json/v3/documentation)） | 可能要对域名单独开启 API 访问 | 有 | ✅ |
| IONOS | API Key | | | 有 | |
| OVHcloud | Application Key、Secret、Consumer Key | 可用访问规则按路径限制；OAuth2 方式按域名授权 | 分 EU / US / CA 入口，要让用户选 | 有 | ◐ |
| Hetzner | Cloud API Token | 项目级，只分读和读写 | 旧 DNS API 已于 2026-05 关闭，旧 token 不能用；dns-update 用的是新地址 | 有 | ✅ |
| Gandi | Personal Access Token | 创建时选定资源和权限 | token 会过期，要提醒轮换 | 有 | ✅ |

**第二组（11 家，`dns-update` 有现成实现）：** DigitalOcean（scope 只到类型，限不到单个域名）、Google Cloud DNS、Azure DNS、Name.com、Hostinger、deSEC、INWX、netcup、TransIP、阿里云、腾讯云 DNSPod。

**日本（1 家，Q4）：さくらのクラウド DNS。** `dns-update` 没有，要自己写。选它是因为官方文档公开，没有账号也能把代码写出来：

- 凭据是 access token 加 secret（[lego](https://go-acme.github.io/lego/dns/sakuracloud/)）。
- 支持 A、AAAA、NS、CNAME、TXT、CAA、ALIAS 等，主机名可以写 `*`（[手册](https://manual.sakura.ad.jp/cloud/appliance/dns/index.html)）。
- 写入是对整个 DNS 项目的 `PUT /commonserviceitem/<id>`，记录在 `ResourceRecordSets` 数组里。文档没说是替换还是合并，按“先读全部、改完整体写回”实现，同一个域名串行。
- 费用没有查。

备选是 VALUE DOMAIN（官方 API 页要登录，没能核对）和 ConoHa v3。お名前.com 只对合作方提供 API，ムームードメイン没找到公开 API，Xserver ドメイン单独契约没有公开 DNS API（[第三方比较](https://vpshikaku.com/domain/dns-api-comparison.html)）。

**不做 DNS API，引导到 SN 托管（有自己证书的可以用手工）：**

| 服务商 | 原因 |
| --- | --- |
| Namecheap | 要满足“20 个域名”“余额 50 美元”“两年内消费 50 美元”之一，且必须登记 IPv4 白名单（[官方 FAQ](https://www.namecheap.com/support/knowledgebase/article.aspx/9739/63/api-faq/)）；`setHosts` 会删掉没有一起提交的记录 |
| Squarespace（原 Google Domains） | [官方 FAQ](https://domains.squarespace.com/faq) 写明接手 Google Domains 的过渡期不支持动态 DNS 和 ACME DNS API，也没找到公开的 DNS API |
| お名前.com、ムームードメイン、Xserver ドメイン | 普通用户没有可用的 DNS API |

对这些用户也可以提示“把 DNS 换到 Cloudflare”，但只作为文字建议，不做自动迁移。

**服务商数据是一份表**，放在 `cyfs-dns-provider`，经 RPC 给向导和设置页：`id`、显示名、要填的字段、申请页链接、NS 后缀（用于自动识别）、是否实测过、测试环境地址。教程文字走 i18n，9 种语言。新增一家不改界面代码。

### 3.6 向导与激活后

激活服务新增 RPC（全部严格解析，未知字段拒绝）：

| 方法 | 作用 |
| --- | --- |
| `inspect_domain` | 查 NS、SOA、CAA、已有记录、SN 绑定状态，给出推荐模式 |
| `list_dns_providers` | 返回服务商表 |
| `plan_domain_records` | 返回记录计划和冲突 |
| `apply_domain_records` | 用 token 写记录；token 只在节点内存里，激活成功才落盘 |
| `check_domain_records` | 按 §3.3.1 逐项返回状态、期望值和实际值 |
| `inspect_certificate` | 检查上传的证书（§3.7），返回逐项结果；通过的证书只放在节点内存里，激活成功才写入 |

- 调用服务商 API 的一律是节点。浏览器直接调会被跨域限制拦住，也会把 token 暴露给网页。
- 检查记录时直接问权威 NS，避免公共解析器的负缓存。现在的流程先让 SN 查一次拿挑战值，`dns.google` 会把“没有这条记录”缓存下来，用户写好后还要等。改为节点自己算：记录名是 `_pkx.<域名>`，值是 `PKX(<owner 公钥 x>)`，公钥取自 BNS 上的 OwnerDocument，与 SN 的算法相同；在权威 NS 上看到 TXT 之后才调现有的 `domain.bind`。
- 域名步骤的放行条件见 §3.3.1。
- 网页路径主要在 `127.0.0.1:3182` 访问（本机，或云主机经 SSH 隧道，Q15），按桌面浏览器设计，窄屏也能用；钱包路径在 BuckyOS App 里，按手机设计。记录卡片的名称和值各有复制按钮；申请 token 的链接用 `openExternalUrl` 在外部浏览器打开，回来后输入内容不丢。
- 成功页按 §3.8.3，不再有 NS 提示和就绪清单。
- 激活后在 Desktop 设置的 Zone 页：查看各检测项的状态、重新检查、换 token、替换证书、在三种模式和两种证书来源之间切换。更换域名不在本期，它会改变 Zone DID。
- token 失效或自动签发的证书续期失败时给 owner 发通知，证书到期前 14 天仍失败再发一次。上传的证书不发通知（Q13）。

### 3.7 自备证书（上传）

**入口：** 手工模式必须上传（Q6）；SN 托管和 DNS API 默认“自动签发”，可改成“上传自有证书”（R4）。激活后在 Desktop 设置的 Zone 页替换。

**接受的内容：**

- PEM：证书、中间证书、私钥，可以是一个文件、多个文件或粘贴文本。
- PFX / P12 加密码（Windows 用户和不少商用 CA 给的是这种）。
- 可以传一张同时覆盖两个名字的证书，也可以为 `<域名>` 和 `*.<域名>` 各传一张。
- 中间证书的顺序由节点整理，多余的根证书丢弃。

**Zone 需要的名字只有两个：`<域名>` 和 `*.<域名>`。** 应用、`sys`、`ood1`、`aiws-<id>` 都是一级子域，而且随时会新增，所以第二个名字必须是字面的 `*.<域名>`，逐个列出子域名的证书不算通过。

**检查在节点上做**（浏览器只显示结果），逐项列出。有“拒绝”就不能走下一步，“提示”要用户确认：

| 检查 | 不通过时 |
| --- | --- |
| 文件能解析；私钥没加密，或 PFX 密码正确 | 拒绝 |
| 私钥与证书匹配 | 拒绝 |
| SAN 含 `<域名>` | 拒绝 |
| SAN 含字面的 `*.<域名>` | 拒绝 |
| 现在处于有效期内 | 过期或未生效：拒绝，并显示节点时间；剩余不足 30 天：提示 |
| 声明了用途时，含服务器认证（没有声明视为允许，`openssl` 生成的自签证书通常没有） | 拒绝 |
| 不是 CA 证书 | 提示。自签证书常带 CA 标记，部分客户端会拒绝 |
| 证书链能连到公共根 | 提示（Q14 接受自签和私有 CA）：可能缺中间证书，也可能是自签或私有 CA；只有信任它的客户端能连 |
| 密钥算法网关能用（RSA 2048 以上、ECDSA P-256 / P-384） | 拒绝 |

只认 SAN，不认 CN。不查 OCSP / CRL 吊销状态。结果按“需要的名字 → 是否覆盖 → 由哪条 SAN 覆盖”显示，常见错误给出专门的说明：

| 用户传的 | 说明 |
| --- | --- |
| Zone 域名是 `home.example.com`，证书是 `*.example.com` | 最常见的误解。它覆盖 `home.example.com`，但不覆盖 `notes.home.example.com`。需要 `*.home.example.com`，或改用 `example.com` 作 Zone 域名（受 Q7 限制） |
| 单域名证书（`example.com` 加 `www.example.com`） | 不含通配名，应用打不开 |
| 只有 `*.<域名>`，没有 `<域名>` | 通配名不包含域名本身。再传一张 `<域名>` 的，或让 CA 把它加进 SAN |
| EV 证书 | 行业规则不允许 EV 用通配名（[CA/B Forum EV 指南](https://cabforum.org/working-groups/server/extended-validation/documents/CA-Browser-Forum-EV-Guidelines-2.0.4.pdf)），请改用 OV 或 DV 通配证书 |

**自签和私有 CA（Q14）：** 用于测试，只有信任它的客户端能连。确认页写明这些会受影响：没有安装该 CA 的浏览器和手机会报不安全；其它 Zone 解析不了本 Zone 的 `did:web`，跨 Zone 功能不可用；BuckyOS App 连不上；激活后的自动检测可能看不到成功（§3.8.3）。

**写入：** 随激活写到身份路径，`<域名>` 和 `_.<域名>` 两个目录各一份，公开部分在 `$BUCKYOS_ROOT/local/identity/`，私钥在 `$BUCKYOS_ROOT/security/`（0600）。`server.meta.json` 的 `renewal.manager` 写 `manual`。文件逐个原子替换，整体不是原子的，中间可能短暂读到不匹配的一对；网关下次握手会重读，可以接受。

**与 ACME 的关系：** 上传模式不生成 ACME 主机；另外 cyfs-acme 见到 `renewal.manager = manual` 的证书一律不下单、不覆盖，防止配置切换的间隙把用户买的证书换掉。用户自己在设置页切回自动签发时，由 node_daemon 去掉这个标记。

**系统不管理上传的证书（Q13）：** 不续期、不提醒、到期不替换。设置页只显示到期日和“替换证书”。商用证书的有效期在缩短（2026-03-15 起最长 200 天，2027-03-15 起 100 天，2029-03-15 起 47 天，[SSL.com](https://www.ssl.com/article/ssl-certificate-validity-changes-what-you-need-to-know/)），所以上传页写明到期日，并说明到期前要自己来换，否则浏览器打不开。

### 3.8 向导简化、分区与启动结果

步骤变为：

- 网页：接入方式 → 身份 → 域名 → 确认 → 启动结果
- 钱包：接入方式 → 域名 → 确认 → 启动结果

网页路径把接入方式提到身份之前，这样注册时就知道用户选的分区（R9：分区只能在注册时告诉 SN）。

#### 3.8.1 SN 分区（R5、R9）

- 放在接入方式步骤。选“SN 中转”，或“直连，只映射 RTCP 端口”（浏览器流量仍走 relay）时显示；其它选项不显示，注册时仍静默带上自动检测的结果。
- **新注册的账号可以改：** 显示自动选中的分区和延迟，下拉列出全部分区及各自延迟，可改选，可重新检测。选定值作为现有 `auth.register` 的 `region` 传给 SN。检测失败时显示“由服务器自动分配”，不阻断。
- **已有账号只显示不能改：** 钱包路径、“我已经注册过”（Q10）和重新激活，用现有的 `zone.get_info` 显示当前 relay。SN 没有改分区的公开接口。
- 身份步骤里的分区面板删除。
- 分区显示名不由 SN 下发，前端按 `region_id` 做 i18n，没有对应翻译就显示 `region_id`。

#### 3.8.2 删除 AI 和 Telegram 步骤（R6）

- 整条链路一起删，不留空字段：两个步骤组件、`types.ts`、`active_lib.ts`、两个教程链接配置、9 种语言的键、`active_server.rs` 的 `ai_provider_config` 和 `owner_telegram_account_id`、`start_config.json` 的同名字段、scheduler 里对应的生成代码、`make_config.ts`、`active.ts` 和测试。
- `llm_router` 保留：网页有邀请码、钱包路径默认开启，激活后 AI 由 SN 提供，直接可用。自己的厂商 token 到 AI 中心添加。
- owner 的 Telegram 账号在“用户与 Agent”里填，Agent 创建向导已有入口，不用补。

#### 3.8.3 启动与检测（R7）

| 时间（从激活成功算起） | 页面 |
| --- | --- |
| 0–2 分钟 | “启动中，首次启动大概需要 2 分钟”。没有倒计时和进度条，不给“打开”按钮，避免过早打开让浏览器缓存“域名不存在” |
| 2–5 分钟 | 文字改为“正在检查域名”，每 5 秒请求一次 `https://public.<域名>/readme`，单次超时 8 秒，不走缓存 |
| 返回 200 | 成功：域名、二维码、“打开”“复制链接” |
| 满 5 分钟仍不成功 | “启动失败”和原因；“重新检测”“仍然打开” |

- **由浏览器检测**，节点上的激活服务这时已经退出。这也正是用户真实的网络视角。
- **跨域：** `public.<域名>` 是匿名内容，网关用 `post_hook_point` 给这个主机名加 `Access-Control-Allow-Origin: *`，页面才能读到状态码。不用 `no-cors`，那样 404、503 也会被当成成功。
- **readme 要保证存在：** `data/srv/publish/` 是保留数据，重装后用户可能已删掉它。节点启动时缺了就补写。
- **页面代码：** 激活成功后 3182 已关闭，启动结果页用到的代码、文案、图标都必须在激活前加载好，不能按需加载。
- **失败原因分三层，** 按顺序判断：
  1. 节点没起来：请求 `http://<当前页面的主机>/1.0/identifiers/<Zone DID>`（80 端口，网关转 system_config，已有跨域头）。连不上或 503 是“没连到节点，可能没有启动完成”。
  2. DNS 没生效：经公共 DoH 查 `public.<域名>`，没有结果是“域名还没有解析”。
  3. 都正常但打不开：“证书或转发还没就绪”。
- **自定义域名：** 与默认域名走同一套状态。DNS 在激活前已检测通过，这里的“域名还没有解析”通常是用户这边的 DNS 缓存，提示换个网络或稍后再试。
- **上传的证书不是公共信任（Q14）：** 用户的浏览器信任它时照常检测成功；否则请求一定失败，页面分不出原因。这时满 5 分钟不报失败，显示“无法自动检测，请手工打开确认”，给“打开”和二维码；第 1 层能连上时改为“节点已启动”。
- **SSH 隧道（Q15）：** 隧道只转发 3182，第 1 层请求到的 `127.0.0.1:80` 不是节点。所以应答里的 Zone DID 对得上才算连上；连不上时文案写“没连到节点；通过 SSH 隧道激活的话这是正常的，请在服务器上查看”，不直接断定节点没起来。
- **同一局域网：** `wan_dyn` 的用户在家里访问自己的公网 IP，路由器不支持回环时会失败，外网却是通的。原因说明里要写这一条。

#### 3.8.4 二维码（R8）

- 检测成功后显示，内容是 `https://<域名>`，扫码直接打开登录页。
- 窄屏（手机上激活）不显示二维码，自己扫不了自己的屏幕，主按钮是“打开”。
- 深色模式下二维码保持白底和留白。
- 新依赖 `qrcode.react`（Q19）。

### 3.9 与 SN 的接口（R9）

不新增方法，不改参数、权限、语义和错误码；SN 服务端代码也不用改，新节点可以直接对接线上的 SN。

| 需要 SN 做的事 | 用现有的什么 |
| --- | --- |
| 归属证明 | `domain.bind`；记录名和值由节点自己算 |
| 查是否已绑定 | `user.get_profile` 的 `user_domain` |
| 证书挑战、试挑战 | `user.add_dns_record` / `user.remove_dns_record`（设备登记之后） |
| `self_cert` 上报 | `user.set_self_cert` |
| 地址上报 | `device.register`、`device.update` |
| SN 应答地址 | 现有规则：Zone 文档的 `gateway_ips`、mini 设备文档的 `net_id`，由激活写入 |
| SN 认域名 | 现有的 NS、SOA 应答 |
| 选分区 | `auth.register` 的 `region`；显示用 `zone.get_info` |
| 账号恢复（Q10） | `auth.login` |

因此有三处不如原方案：

1. 已有账号不能在激活页改分区。
2. SN 托管的试挑战在提交时才做，不在域名步骤；域名步骤只能确认委派已到 SN。
3. 同一账号先后绑过两个都已 NS 委派的域名、又要回到前一个时，`user_domain` 只记最新的，`_pkx` 又查不到，绑定会失败；向导提示先临时撤掉 NS。

域名被另一用户接管时的通知、更细的错误码也不做，向导只按现有错误码给说明。

以上都接受（owner），需要 SN 后续做的写在 `cyfs-gateway/doc/SN/SN-自定义域名激活跟进-TODO.md`。SN 那边改地址应答时有一个先后约束：读完整设备文档和 `portmap` 改应答 relay 必须同一次改。

## 4. 范围外

- 改 SN 的 API 和服务端代码（R9），见 `cyfs-gateway/doc/SN/SN-自定义域名激活跟进-TODO.md`。激活页和设置页给已有账号改分区依赖它。
- 更换已激活 Zone 的域名、一个 Zone 多个域名。
- 完全不用 SN / BNS 的激活，继续走 `src/active.ts`。
- 代用户购买域名、把域名迁到别家。
- Domain Connect（[IONOS、Cloudflare、GoDaddy 等](https://www.domainconnect.org/dns-providers/)支持的授权式写记录，不用 token）。它对手机用户更友好，但要向每家提交模板，二期再评估。
- 商用 CA 的 ACME（DigiCert、Sectigo 等要用 EAB 账号）。cyfs-acme 现在不支持 EAB。

## 5. 已知风险

1. 激活页是 HTTP。基本在 `127.0.0.1` 访问，云主机用 SSH 隧道（Q15），不另加处理。
2. 有些服务商的 token 管到整个账户或项目（Hetzner、DigitalOcean 等），页面要写明。
3. `dns-update` 依赖 reqwest 0.13、hickory 0.26，我们是 0.12 / 0.25，会多编译一份；它的默认 feature 用 aws-lc-rs，我们的 rustls 用 ring，两个 provider 同时启用时没有显式安装默认 provider 会 panic；它的 `ring` feature 又会带上 native-tls。T0-1 先验证。
4. 网页用户中途刷新会丢助记词和设备密钥，用户名却已注册。Q17 把等 DNS 的时间放到激活前，会更常见；本期用“我已经注册过”解决（Q10，T6-10）。
5. 上传的证书不能重新签发回来，丢了要向 CA 重新下载或重签。备份要包含它（T4-8）。
6. `self_cert` 是用户级标记，不区分域名。证书过期或被删时 relay 仍会转发，浏览器看到的是证书错误，不会回退到 SN 的证书。
7. SN 只有一个 NS 主机名 `dns.<SN>`。子域名委派可用，冗余依赖 SN 自己的部署。
8. `public.<域名>` 要等节点自己的通配证书签下来才能打开。`sn-dns` 每次下单固定等 65 秒，失败 30 秒后重试，首次启动可能超过 5 分钟。T0-6 先实测，超了就调快签发或放宽时间。
9. 20 家服务商的实现多数没有真实账号测过（Q3）。整份读写的接口（さくらのクラウド）写错会影响用户的其它记录，靠写前列出改动、写后读回核对、“实验”标记来降低。
10. 自签证书激活后，自动检测、其它 Zone 和 BuckyOS App 都不能用（Q14）。
11. readme 是网关直接提供的静态文件，它能打开不代表登录页已就绪。T0-6 同时记录两者的先后；登录页明显更晚时，加测 `https://<域名>/`。

## 6. 裁决

全部已定（owner，2026-11-22），没有待裁决项。“owner 答复”是原文。

| 编号 | 问题 | owner 答复 | 落实 |
| --- | --- | --- | --- |
| Q1 | `did.json` 由谁处理 | 设备的 device document 不被 boot 依赖，boot 后其它 Zone 才需要经 HTTPS 取它 | §3.2。Zone DID 仍是 `did:web:<域名>`；BNS Server 不改 |
| Q2 | 引入 `dns-update` | 好 | §3.4。以 T0-1 通过为前提；不通过就先自己写第一组 8 家 |
| Q3 | DV 的账号和测试域名由谁提供 | DNS的token都比较敏感，先写代码吧。（有官方测试Token的可以用），后续主要靠各个贡献者实测了 | §3.5。mock + 官方测试环境 + 贡献者自测工具（T3-10）；没测过的标“实验” |
| Q4 | 日本是否首批就要有一家 | 应该有一家。 | §3.5。选さくらのクラウド（我替 owner 选的），替换名单里的 Vercel |
| Q5 | token 长期存在节点本地 | 这个肯定要存buckyos/security/ | §3.4。`$BUCKYOS_ROOT/security/dns_credentials/` |
| Q6 | 手工模式的证书 | 手工模式是指用户自己上传证书，DNS不设置_acme-challenge | §3.1、§3.3。记录减为 3 条；`_acme-challenge` CNAME 方案作废 |
| Q7 | 根域名 | 按你的建议 | `wan` 可用；其它接入方式要求子域名，DNS 在 Cloudflare 的放开；SN 托管不支持根域名 |
| Q8 | `wan` 是否提供 SN 托管 | 需要SN中转，SN解析地址，和SN解析DNS其它记录是三件不同的事情，不要搞混了 | §3.1.1。三件事拆开，`wan` 也提供 SN 托管；原建议作废 |
| Q9 | 非 SN 托管仍要 SN 账号 | 同意，buckyos企业版使用激活脚本来实现完全无SN，走页面的都是有账号的。 | §3.1.1 |
| Q10 | 刷新后丢身份本期是否解决 | 同意 | T6-10 |
| Q11 | “token 只用一次” | 不提供"只用一次"，因为这类证书的时间都比较短，要帮用户定期刷新 | §3.4。token 长期保存 |
| Q12 | `portmap` 的 A 记录 | 修吧，portmap主要是让RTCP能直连 | T5-2 |
| Q13 | 上传的证书到期没换 | 不换，用户上传了后系统就不管理证书了 | §3.7。到期提醒也一并取消 |
| Q14 | 自签或私有 CA | 接受，用户也许是测试用途，只需要几个特定的客户端能连就好 | §3.7、§3.8.3 |
| Q15 | 节点生成 CSR | 不用，激活页基本是127.0.0.1访问的，云端机器用ssh隧道访问127.0.0.1:3182也很容易，我们不用增加麻烦 | 不做。§3.8.3 处理 SSH 隧道下的检测 |
| Q16 | “超过 5 分钟”从哪里算起 | OK（从激活成功算起） | §3.8.3 |
| Q17 | DNS 没配完怎么办 | 要监测是否完成配置了才走下一步 | §3.3.1 |
| Q18 | NS 委派提前到激活前 | 提前 | §3.1。旧 TODO §1.4 冻结的“NS 在激活后提示”作废（T7-6） |
| Q19 | 引入 `qrcode.react` | 引入 | §3.8.4 |
| Q20 | 首次启动会先请求自己的 `https://<域名>`，连接被丢包时可能 Boot Failed | 好 | T0-4 先实测；T1-6 由 `commit_active` 写入 Last Known Good State，首次启动就按 Warm Restart 走 |

## 7. 任务

### T0 先验证（决定方案是否成立）

- [ ] T0-1 `dns-update` 放进 cyfs-gateway 编译：TLS provider 冲突、包体积、三平台构建。Cloudflare 实测三个操作。
- [ ] T0-2 用一个真实子域名走 SN 托管，Let's Encrypt staging 能签 `<域名>` 和 `*.<域名>`。
- [ ] T0-3 用一个真实域名验证 CNAME 到 `<用户名>.web3.<SN>`：不委派 NS，经 relay 打开 HTTPS（先手动设 `self_cert`）。同时验证 Cloudflare 根域名 CNAME 展开，且记录没有开代理（Q7）；以及注册后、激活前 `<用户名>.web3.<SN>` 已应答 relay 的 A（§3.3.1）。
- [ ] T0-4 首次启动时让 `https://<域名>` 分别丢包、拒绝连接、返回 404，看 node_daemon 能否用激活时的本机 Zone 文档起来；重新激活同一域名再测一次（Q20）。
- [ ] T0-5 用页面激活一个 `wan_dyn`，查 `dns.<SN>` 应答的 A 是设备地址还是 relay（§2“SN 应答的地址”）。`wan` 现状复核：SN 自带 AI 开启时首次调度是否报错；`ZoneDocument.sn`、`gateway_ips` 写入、设备文档带公网 IP 之后，SN 应答固定 IP，节点不连 relay、能上报、能写 `_acme-challenge`（§3.1.1）。
- [ ] T0-6 实测激活成功到 `public.<域名>/readme` 返回 200、到登录页可用的时间：默认域名和自定义域名、`nat` 和 `wan_dyn`，每种至少 5 次。据此确认 2 分钟 / 5 分钟是否合适。

### T1 拿掉 `did.json`（buckyos + cyfs-gateway）

- [ ] T1-1 `active_server.rs`：`did:web` 设备的两次检查都删除；`did:bns` 保留发布后的检查。
- [ ] T1-2 `active_lib.ts`、`ReviewStep.tsx`：删除 `customDomainPublication`、`prepareSignedActivation` 的单独入口和三个按钮，两种域名走同一条激活路径。
- [ ] T1-3 删除 9 种语言里 `device_publication_*`、`prepare_device_publication`、`download_device_publication`、`verify_publication_and_activate`；`test/active-publication.test.cjs` 改写为新流程。
- [ ] T1-4 改 `cyfs-gateway/doc/BNS/BNS-API.md` 中自有 `did:web` 的段落（§3.2）；`node_active_authority.py` 回归只保留 `did:bns`。只改文档和测试，不改 BNS Server。
- [ ] T1-5 boot 之后 `https://ood1.<域名>/.well-known/did.json` 返回设备文档 JWT 原文，另一个 Zone 能解析 `did:web:ood1.<域名>` 并通过 owner 验签。现有代码应已满足，补回归。
- [ ] T1-6（按 Q20）`commit_active` 写入 Last Known Good State，覆盖同一 Zone 的旧状态；Booting 文档同步。

### T2 域名绑定（buckyos，SN 不改）

- [ ] T2-1 节点自己算 `_pkx` 的记录名和值，与 SN 现有算法做对照测试；权威 NS 上看到之后才调 `domain.bind`。
- [ ] T2-2 `commit_active` 和域名步骤先查 `user.get_profile`，`user_domain` 已是目标域名就不调 `domain.bind`。解决 NS 委派后重新激活卡死和第二次绑定。
- [ ] T2-3 向导按现有错误码显示说明，不显示原始报文。
- [ ] T2-4 SN 托管的试挑战：`commit_active` 在 `device.register` 之后写、查、删，失败不写本机身份，可重试。
- [ ] T2-5 回归：新节点对接未改动的 SN（cyfs-sn 的 SQLite 和 sn-business 的 Postgres）走完三种托管。

### T3 DNS provider 与 ACME（cyfs-gateway）

- [ ] T3-1 `cyfs-dns-provider`：trait、`dns-update` 包装、SOA 主域名识别、20 家的服务商表、凭据文件读取。
- [ ] T3-2 GoDaddy 的 PAT 实现。
- [ ] T3-3 cyfs-acme 通用 factory；按值增删；轮询权威 NS 等待生效。
- [ ] T3-4 さくらのクラウド的实现：先读后整体写回、同一域名串行、写后读回核对。
- [ ] T3-5 `self_cert` 上报移出 `sn-dns`，以 TLS 栈实际加载的证书为准，仍用 `user.set_self_cert`。
- [ ] T3-6 证书状态接口：每个主机名的签发状态、到期时间、最近一次错误。
- [ ] T3-7 删除 aliyun JS 插件；`cyfs_gateway.yaml` 示例更新。
- [ ] T3-8 把 cyfs-acme 里校验证书、写入身份路径的代码抽成共用 crate，补齐 §3.7 的检查（PFX、链整理与公共根校验、字面通配名、用途、算法、自签与私有 CA），供 cyfs-acme 和 node_daemon 使用。
- [ ] T3-9 cyfs-acme 不处理 `renewal.manager = manual` 的证书：不下单、不覆盖、不因它续期失败而重试。
- [ ] T3-10 贡献者自测工具：指定服务商、凭据文件和域名，写入、读回、删除临时的 TXT 和 CNAME，输出不含 token 的报告，支持官方测试环境地址；服务商表据此改成“已实测”。

### T4 记录计划与激活服务（buckyos）

- [ ] T4-1 记录计划纯函数和冲突检测，覆盖三种模式 × 四种接入方式 × 根域名 / 子域名。
- [ ] T4-2 §3.6 中域名相关的五个 RPC；`apply_domain_records` 可重复执行，写一半失败后重试不产生重复记录。
- [ ] T4-3 `commit_active` 写 `domain_hosting` 和凭据文件；凭据不出现在日志和 `start_config.json`。
- [ ] T4-4 scheduler 按 `domain_hosting` 生成 ACME 配置，`wan` 也生成；支持覆盖 ACME issuer，供 DV 用 staging。
- [ ] T4-5 `active.ts` 和 `make_config.ts` 不受影响的回归。
- [ ] T4-6 `inspect_certificate` RPC；`commit_active` 写入上传的证书，失败时不留下半套文件；私钥不进日志和 `start_config.json`；请求体大小上限。
- [ ] T4-7 激活后替换证书、切换证书来源由 node_daemon 执行；control_panel 只转发，仅限 owner。
- [ ] T4-8 `backup.py` 备份和恢复 `<域名>`、`_.<域名>` 的证书和私钥。
- [ ] T4-9 §3.3.1 的检测：权威 NS 查询、NS 与 SOA、通配与 `ood1` / `sys` / `public`、DNS API 的试挑战、CAA；`commit_active` 提交前复查。
- [ ] T4-10 删除 `uses_sn_relay`，按 §3.1.1 拆开：`ZoneDocument.sn`、`ddns_sn_url` 一律写入，`wan` 写 `DeviceDocument.ips`，隧道、上报、ACME、路由的判断同步；`active.ts` / `make_config.ts` 的无 SN 激活不变。

### T5 地址发布（buckyos，SN 不改）

- [ ] T5-1 SN 托管时确认 `ood1.<域名>`、`*.<域名>` 的应答与 `<域名>` 一致。
- [ ] T5-2 `portmap` 的 mini 设备文档不写 `net_id`，SN 应答 relay（Q12）；RTCP 直连地址仍由完整设备文档提供。
- [ ] T5-3 `wan` 在签名的 Zone 文档写 `gateway_ips`；`wan_dyn` 在 mini 设备文档写 `net_id`，修好动态解析。对照 SN 现有解析代码写测试，确认四种接入方式的应答。

### T6 界面（buckyos）

- [ ] T6-1 域名步骤：输入 → 检测 → 三选一 → 对应面板 → 全部通过后放行。
- [ ] T6-2 手工面板：记录卡片、复制、逐条状态、冲突说明。
- [ ] T6-3 DNS API 面板：服务商列表、教程、token 表单、将要改动的记录、写入结果。
- [ ] T6-4 检测面板：自动轮询、逐项状态与期望值 / 实际值、未通过不能下一步；删除成功页的 NS 提示。
- [ ] T6-5 Desktop 设置 Zone 页：状态、重新检查、换 token、切换模式；control_panel 接口和权限（只限 owner）。
- [ ] T6-6 token 失效、自动续期失败的通知。
- [ ] T6-7 9 种语言。现有 TXT 挑战的 7 个键（`domain_txt_challenge`、`dns_record_name`、`dns_record_value`、`dns_propagation_hint`、`domain_verified`、`verify_again`、`verify_domain`）在所有语言里都没有，仍用到的一并补；`ns_after_activation_hint` 删除。
- [ ] T6-8 在接入步骤点“下一步”但没有改动时，不清空已完成的域名状态。
- [ ] T6-9 证书上传面板（激活页和设置页共用）：选文件、粘贴、PFX 密码、逐项检查结果、常见错误说明、到期日、自签与私有 CA 的确认。手机上能从系统文件选择器选文件，也能粘贴。
- [ ] T6-10（按 Q10）身份步骤“我已经注册过”：用户名、密码、助记词恢复后回到域名步骤，归属和 NS 的进度由检测重新得到。

### T7 文档

- [ ] T7-1 `doc/arch/gateway/zone-boot-config与zone-gateway.md` 的 B1 和 C 节改为三种托管，写清中转、应答地址、应答其它记录是三件事；`doc/arch/gateway/SN.md` 同步。
- [ ] T7-2 `doc/key data.md`、`doc/path_usage.md` 登记 `domain_hosting` 和凭据文件。
- [ ] T7-4 用户文档：20 家服务商各一页申请教程，可在手机上照做。
- [ ] T7-5 用户文档：向商用 CA 购买时该选哪种证书（OV / DV 通配，SAN 同时含 `<域名>` 和 `*.<域名>`），拿到后怎么上传。
- [ ] T7-6 `product/node_active/Node_Active_新流程实现TODO.md` §1.4 等处的“NS 在激活后提示”改为激活前。
- [ ] T7-7 贡献者说明：怎么用 T3-10 的工具测一家服务商、怎么提交结果。

### T8 向导简化、分区与启动结果

- [ ] T8-1 网页路径步骤改为接入方式在前、身份在后；所选分区作为 `auth.register` 的 `region`。
- [ ] T8-2 分区面板移到接入方式步骤：新注册可选，显示每个分区的延迟；已有账号用 `zone.get_info` 只读显示。
- [ ] T8-3 分区名前端 i18n。
- [ ] T8-4 删除 AI 和 Telegram 步骤（§3.8.2 的整条链路）；`stepOrder` 和步骤标题重排；`title_step_*` 去掉数字前缀。
- [ ] T8-5 `boot_gateway.yaml` 给 `public.<域名>` 加跨域头，网关 process chain 的调试用例同步；节点启动时补写缺失的 readme。
- [ ] T8-6 启动结果页状态机：启动中 / 检查中 / 成功 / 失败 / 无法自动检测（证书不是公共信任）；用假时钟做单元测试。
- [ ] T8-7 失败三层判断与文案，含局域网回环和 SSH 隧道的说明。
- [ ] T8-8 二维码（Q19）：宽屏显示、窄屏隐藏、深色模式。
- [ ] T8-9 确认启动结果页没有按需加载的代码和文案；9 种语言，删除 `refresh_note` 等旧键。
- [ ] T8-10 文档：`product/users_and_agents/BuckyOS Agent的初始化流程PRD.md` 里激活收集 Telegram 账号的段落；检查 `buckyos_admin` 等手册是否提到被删的激活步骤。

## 8. 验收

单元与集成：

- 记录计划的组合测试；冲突不静默覆盖。
- DNS provider 20 家都用 mock 服务器：按值增删、重复执行、限流重试、token 失效、不动计划外的记录。有官方测试环境的再跑一遍（§3.5）。
- 接入方式拆分：三种托管 × 四种接入方式的 `sn`、`ddns_sn_url`、`ips`、隧道、ACME 配置逐格断言。
- ACME 用 pebble：两个主机名并发签发、挑战值互不覆盖、生效等待超时。
- 激活：三种模式提交、提交失败重试、重新激活已绑定域名。
- cyfs-gateway 的共享改动：SN 的配置和现有测试不变，SN 自己的证书签发与加载行为不变。
- 证书检查：§3.7 两张表的每一行都有用例（测试 CA 现场签发）；PEM 与 PFX；乱序链、缺中间证书、密钥不匹配、P-521、过期、未生效；自签（带和不带 CA 标记）、私有 CA 确认后可激活。
- ACME 与手工证书：到期前 7 天内不覆盖 `manual` 证书；切回自动签发后能签发。
- 启动结果页：2 分钟前不请求；404、503、跨域失败都不算成功；满 5 分钟失败；失败后重新检测能转成功。
- 放行：§3.3.1 每一项不通过时都不能进入下一步，绕过页面直接调 `commit_active` 也被拒绝；Cloudflare 代理、旧的 `ood1` 记录、CAA 不允许各有用例；试挑战的 TXT 用完即删。
- 没有 AI 和 Telegram 字段的激活请求与 `start_config.json` 能正常生成系统配置，`llm_router` 仍按邀请码开启。

真实环境 DV（需要真实域名）：

| 用例 | 通过标准 |
| --- | --- |
| SN 托管 + `nat` | 两次 DNS 操作后 `https://<子域名>` 和应用子域名可打开 |
| SN 托管，重新激活 | NS 已委派时不卡在归属验证 |
| SN 托管 + `wan` | SN 应答固定 IP，节点不连 relay，证书签发成功 |
| 默认域名 + `wan_dyn` | SN 应答设备公网 IP，IP 变化后跟着变 |
| 手工 + `nat` | 写三条记录、上传证书后可打开，没有 `_acme-challenge` |
| 手工 + `wan` 根域名 | A 记录 + 上传证书，443 直接可用 |
| DNS API（贡献者账号，先 Cloudflare）+ `nat` | 粘贴 token 后无需再去 DNS 后台，强制续期一次成功 |
| DNS API（同上）+ `wan` | 节点不连 relay 也能签发和续期 |
| DNS API（同上）+ `wan_dyn` | IP 变化后仍可打开 |
| `portmap` | 浏览器经 relay 打开，RTCP 直连 |
| 上传证书 + `nat`（手工 DNS，三条记录） | 真实的通配证书上传后，`https://<域名>` 和应用子域名经 relay 可打开，`self_cert` 已上报 |
| 上传自签证书 | 提示并确认后能激活；装了该 CA 的客户端可打开，没装的显示“无法自动检测” |
| 上传不合格的证书 | `*.example.com` 用于 `home.example.com` 时被拒绝，说明正确 |
| SSH 隧道激活云主机 | 走完全流程，启动结果不误报“节点没起来” |
| 替换证书 | 设置页换新证书，1 分钟内新连接用上新证书，网关不重启 |
| token 被吊销 | 设置页显示原因，owner 收到通知，换 token 后恢复 |
| 分区 | 新注册时改选非自动的分区，SN 分配的 relay 属于所选分区，节点隧道连到该 relay；钱包路径显示当前 relay，不能改 |
| 启动成功 | 默认域名激活后页面自动变为成功，手机扫码能打开登录页 |
| 启动失败 | 激活后拔掉节点网络，5 分钟时显示启动失败和原因；恢复后重新检测变为成功 |
| 没加 NS | 停在域名步骤，不能下一步；加上 NS 后页面自动变为通过，激活后直接成功 |
| 等 DNS 时离开 | 刷新、关掉再打开激活页，能用用户名、密码和助记词恢复身份并继续（Q10） |
| `wan_dyn` 首次启动 | 路由器不支持回环时也能启动（Q20） |
| 跨 Zone | 激活后另一个 Zone 能解析 `did:web:ood1.<域名>` 并访问本 Zone |
| 钱包路径 | 以上至少一条在 BuckyOS App 里用手机走完 |

## 9. 建议顺序

1. T0。T8-4 到 T8-9（删步骤、启动结果、二维码）对默认域名也有用，不依赖自定义域名，可以最先做；T8-1 到 T8-3 随后。
2. T1 + T2 + T6-10：先让归属部分走通，不再需要 `did.json`，刷新后能恢复身份。
3. T4-10 + T5 + T3-5 + T4-4 + T4-9，以及 T3-8 + T3-9 + T4-6 上传证书：SN 托管和手工完整可用，四种接入方式都拆开。
4. T3 其余 + T4：DNS API，20 家的代码、mock 和官方测试环境。
5. T6 + T7，贡献者逐家实测。

没有部署先后要求：SN 不改，新节点直接对接线上的 SN。
