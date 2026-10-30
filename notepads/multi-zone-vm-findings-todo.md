# 三 Zone VM 环境暴露的问题 TODO

日期：2026-10-22。状态：F-1～F-9 修复完成，针对性测试与 VM 验收通过；F-3 按用户确认仅验收监听端口，保留 Zone 隔离。

来源：在 PVE 多 VM 环境 `src/dev_configs/ssotest.json` 上做混合 SSO 三 Zone 验收（[混合 SSO 实施 TODO](hybrid-sso-implementation-todo.md) §11“三 Zone 验收”）时发现的问题。下文 F-1～F-9 保留发现时的现象与证据；当前实现和验收结果以本节及末尾“修复验收记录”为准。

用户已确认：F-5 按 README 恢复 `wan_dyn` / `portmap` 直连；F-6 本期修复，要求目标 DID 联合登录会话；F-3 保持 `same_zone` 隔离，本次只修监听端口，不以跨 Zone RTCP 握手成功为验收条件。

| 项目 | 修复与验收状态 |
| --- | --- |
| F-1 | 已修复。RTCP 限流/配额拒绝在原超时内退避重试，建流完成即释放 pending 配额。Alice 经 SN 的 `/login` 与 `/` 冷加载失败请求均为 0。 |
| F-2 | 已修复。中转设备 DID 从 Zone/boot 的权威 DID 推导，严格拒绝无效身份；强制 Charlie 经 SN 返回 HTTP 200，已移除 Alice/Bob 的 hosts 绕行。 |
| F-3 | 已修复。Rust、SDK、开发种子和网关模板统一传递设备 `rtcp_port`，Charlie 实测仅监听 2981。跨 Zone 连接仍受用户要求保留的 `same_zone` 策略限制。 |
| F-4 | 已修复。msg-center、HomeStation、AppClient runtime 统一自托管 owner 回退规则；实测 Bob 查 Alice 可路由且原生消息进入 Alice INBOX。 |
| F-5 | 已修复。DDNS 与中转配置分开，`wan_dyn` 不写 Zone SN；DNS 对 Bob/Charlie 返回上报地址，Alice 仍返回 SN。2026-10-29 补：scheduler 原先只在 Zone SN 存在时生成 `zone_tls`（443）与 ACME，按新规则重建的 bob 种子因此没有 443；现在回退到设备的 `ddns_sn_url`（`node_sn_api_url`），重建后 bob 直连 HTTPS 正常。 |
| F-6 | 已修复。后端要求匹配 DID 的 Control Panel 联合登录会话，接受邀请使用原子事务与版本检查；新增前端邀请入口。三 Zone 实测正确 DID 接受成功，匿名、错误 DID、密码会话、过期邀请均被拒绝。 |
| F-7 | 已修复。物化前校验按实际用户名派生的开发口令哈希，旧种子明确报错且不写目标目录。 |
| F-8 | 已修复。PVE 默认等待 guest agent 900 秒，创建失败汇总并非零退出，跳过 instance_commands，输出按行刷新。与 Cargo 构建并行的新建 VM 验证通过。 |
| F-9 | 已修复。未登记 `/kapi/<service>` 返回 503；SN cloud-init 维护本机 hosts，当前 SN 已实测不再产生 sudo 主机名告警。 |

## 0. 环境与复现

**拓扑**（种子见 `src/devenv_config.ts`）：

| 节点 | Zone 主机 | Zone DID / owner | 网络形态 |
| --- | --- | --- | --- |
| `sn` | `sn.devtests.org`、`web3.devtests.org` | — | SN（web3-gateway） |
| `alice-ood1` | `alice.web3.devtests.org` | did:bns:alice（自托管 Zone，owner DID = Zone DID） | NAT（入站 80/443/2980 被拒），只能经 SN 中转 |
| `bob-ood1` | `bob.web3.devtests.org` | did:bns:bob（自托管） | `wan_dyn`，无防火墙，可直连 |
| `charlie-ood1` | `charlie.me` | did:web:charlie.me，owner did:bns:charlie | `portmap`，种子 `rtcp_port: 2981`，入站 2980 被拒 |

三个 owner 的口令都是 `bucky2025`。

**建环境**（在 `buckyos/src` 下；本机是 PVE，非 PVE 主机去掉两个 export 即为 multipass）：

```bash
export BUCKYOS_VM_BACKEND=pve
export BUCKYOS_PVE_VM_PARAMS='{"image": "/var/lib/vz/template/iso/noble-server-cloudimg-amd64.img", "storage": "local-lvm"}'
uv run buckyos-devtest ssotest create_vms && uv run buckyos-devtest ssotest snapshot init
uv run ./build_for_vm_test.py
uv run buckyos-devtest ssotest install && uv run buckyos-devtest ssotest snapshot installed
uv run buckyos-devtest ssotest start
uv run buckyos-devtest ssotest info_vms        # 各 VM 的 IP
uv run buckyos-devtest ssotest run <node> "<cmd>"
```

- 2026-10-22 建好的四台 VM 是 PVE 107（sn）、108（charlie）、109（alice）、110（bob），IP 依次为 192.168.1.253 / .113 / .90 / .221（DHCP 分配，重建后会变）。用户可能已经清理，先用 `qm list` 查。
- 改代码后的更新：先 `uv run ./build_for_vm_test.py`，再 `stop` → `update` → `start`。
- 如果本机 `~/buckycli/<zone>/ood1/start_config.json` 是旧文件，owner 口令登录不了，见 F-7。

**主机侧访问**：宿主机不用 SN 做 DNS。

- 命令行：`dig @<sn ip> <host>`；`curl --resolve <host>:443:<ip>`。
- Node：用 `dns.Resolver` 并 `setServers([sn ip])`，写法见 `test/test_hybrid_sso/test_hybrid_sso_federation_dv.ts`。
- Chromium：`--host-resolver-rules="MAP <host> <ip>, ..."`，写法见 `src/frame/desktop/tests/dv/hybrid-sso-federation.spec.ts`。
- VM 已信任 dev CA；宿主机侧的 CA 是 `~/buckycli/ca/buckyos_test_ca_ca_cert.pem`。

## F-1 经 SN 中转时浏览器冷加载被 RTCP 入站流限速打断（P1，cyfs-gateway）

**现象**：经 SN 中转访问的 Zone（alice，也包括走中转的 bob）在 Chromium 里打开 `/login` 是白屏。几十个静态资源报 `net::ERR_CONNECTION_CLOSED`。直连同一 OOD（bob 用 192.168.1.221、charlie）失败数为 0。实际影响：NAT 后的 Zone 在浏览器里不可用。

**证据**（2026-10-22）：

- 冷加载 `https://alice.web3.devtests.org/login`：35 个请求失败；bob 经 SN 时 38 个。
- 同一时间窗内 SN 日志 `/opt/buckyos/logs/web3_gateway/*.log` 中：
  - `SNI found: alice` 共 164 次，即一次页面加载在 SN 上建了 164 个 TLS 连接；
  - 其中 76 次出现 `RTcp ropen stream ... rejected by peer, result=8` 与 `Open stream by url rtcp://ood1.alice.web3.devtests.org/:443 failed`。
- `result=8` 是 `OPEN_RESULT_RATE_LIMITED`（`cyfs-gateway/src/components/cyfs-gateway-lib/src/rtcp/rtcp.rs:4386`）。拒绝发生在 OOD 侧的 `admit_inbound_stream_build`（约 `rtcp.rs:4777`）：令牌桶 `consume_stream_request_token`，默认每隧道 `stream_requests_per_second: 32`、`stream_request_burst: 64`（`rtcp.rs:184-185`）。
- `curl -v` 经 SN 连续请求两次可以复用连接（HTTP/1.1，未协商 h2）。所以单个客户端本身能做 keep-alive，但 Chromium 加载时连接数远超每主机 6 个的常规上限。

**待查**：

1. 为什么一次加载要建 164 个连接：中转链路上的连接是否被提前关闭（例如 SN 或 OOD 在响应后关流、空闲超时过短），导致浏览器不断重连；被拒的流又触发重试，进一步耗尽令牌。
2. 限速的对象是“每条 SN 隧道”。一个 Zone 的所有外部访客共用这条隧道，同一个桶对正常的多用户访问也太小。

**建议修法**：先修连接复用（1），再按真实负载调整限速或改为按来源计数。被限速的 ropen 应排队或退避，不要直接让客户端看到连接被关。具体改法由 cyfs-gateway 侧决定。

**验收**：用 Chromium 经 SN 冷加载 `https://alice.web3.devtests.org/` 与 `/login`，失败请求为 0；把 `tests/dv/hybrid-sso-federation.spec.ts` 的 canonical 设为 alice（`BUCKYOS_TEST_CANONICAL=alice.web3.devtests.org BUCKYOS_TEST_CANONICAL_USER=alice`，host rules 指向 SN IP）能通过。

探针（在 `src/frame/desktop` 下运行，统计失败请求）：

```js
// probe.mjs <url>; env RULES="MAP alice.web3.devtests.org <sn ip>"
import { createRequire } from 'node:module'
const { chromium } = createRequire(process.cwd() + '/package.json')('@playwright/test')
const browser = await chromium.launch({ args: [`--host-resolver-rules=${process.env.RULES}`] })
const page = await browser.newPage({ ignoreHTTPSErrors: true })
page.on('requestfailed', (r) => console.log('failed', r.url(), r.failure()?.errorText))
await page.goto(process.argv[2], { waitUntil: 'networkidle' }).catch((e) => console.log(String(e)))
await browser.close()
```

## F-2 SN 不能中转自定义域名 Zone（did:web）（P1，cyfs-gateway SN）

**现象**：`https://charlie.me/` 经 SN（DNS 答 SN IP）时 TLS 握手即被断开；直连 192.168.1.113 返回 200。

**证据**：SN 日志依次出现：

- `SNI found: charlie.me`
- `sn_resolver gateway address result: zone=charlie device=ood1 reason=explicit_non_wan_requires_relay network_type=portmap ips=[…, 192.168.1.113, 192.168.1.253]`
- `resolve RTCP peer did:bns:ood1.charlie with AuthorityCurrent policy failed: did:bns:ood1.charlie#zone has no authority-current document: bns-provider: HTTP 404`
- `Create tunnel for rtcp://ood1.charlie.web3.devtests.org/:443 failed`

charlie 的实际身份见 `/opt/buckyos/etc/node_identity.json`：`zone_did = did:web:charlie.me`、`device_did = did:web:ood1.charlie.me`。

**定位**：SN 按 SN 用户名 `charlie` 拼出 `did:bns:ood1.charlie` / `ood1.charlie.web3.devtests.org` 作为中转目标，没有用该 Zone 自己的 DID（did:web）。先看 `sn_resolver.rs` 附近的“zone → device → RTCP 目标”推导。

**验收**：去掉 alice、bob 上 `/etc/hosts` 的 `charlie.me` 映射（`ssotest.json` 的 instance_commands 为绕开本问题而加，修好后删掉这条命令）后，`curl --resolve charlie.me:443:<sn ip> https://charlie.me/` 返回 200；联合 SSO 脚本的 `bound-canonical` 步骤仍通过（B 的 verify-hub 经 SN 访问 charlie.me）。

## F-3 OOD 的 RTCP 监听端口不跟随 `rtcp_port`（P1，buckyos）

**现象**：charlie 的种子 `rtcp_port: 2981`（模拟路由器只映射 2981），但 `cyfs_gateway` 只在 2980 监听（`ss -lntp`）。

**定位**：

- `src/rootfs/etc/boot_gateway.yaml` 的 `node_rtcp` 写死了 `bind: "[::]:2980"`。
- 对端连接时用的是设备文档里的端口：`node_daemon` 的 `finder.rs` / `gateway_tunnel_probe.rs` 取 `device_doc.rtcp_port`。

结果是对端按 2981 直连，而 2981 上没有监听。当前是 SN 中转在兜底；F-2 修好之前，charlie 的 RTCP 直连和中转都不可用。

**建议修法**：网关配置从本机设备文档（或 node_identity）取 `rtcp_port` 渲染 bind，与 `device_doc.rtcp_port` 保持一致。改模板时检查 `src/test/test_boot_gatweay` 的用例。

**验收**：charlie 上 `ss -lntp` 显示 2981；从 bob 用 RTCP 直连 charlie 成功（经 `gateway_tunnel_probe` 或跨 Zone kRPC），不依赖 SN。

## F-4 msg-center 与 HomeStation 找不到自托管 Zone 用户的 Zone（P1，buckyos）

**现象**：B 上调用 msg-center `contact.lookup {input:"did:bns:alice"}`，返回 `reachability: {state:"unreachable", reason:"no-zone"}`；查 did:bns:charlie 则返回 `routable, zone:"charlie.me"`。影响：给 alice 这类用户的跨 Zone 原生投递得到 `native-route-unresolved`，HomeStation 定位不到其 home。

**原因**：自托管 Zone 的 owner DID 就是 Zone DID（`alice.bns.did` ↔ did:bns:alice）。`make_config.ts` 的 `normalizeOwnerDocument` 只在 owner ≠ zone 时写 `binded_zone_list`，所以 alice 的 OwnerDocument 没有绑定列表。下面两处只认 `binded_zone_list` 首项：

- `src/frame/msg_center/src/network_directory.rs` 约 99–108 行：`LookupKind::User` 只读 `binded_zone_list`；
- `src/frame/homestation/src/directory.rs` 的 `zone_of`（约 220–235 行）：`doc.get_default_zone_did()`。

另有 `src/kernel/buckyos-api/src/runtime.rs` 约 595 行，开发模式加载 owner 配置时也只用 `get_default_zone_did()`，失败报 `default_zone_did is not set`；需判断该路径是否会遇到自托管 owner。

**建议修法**：复用混合 SSO 中新增的 `buckyos_api::owner_canonical_zone`（`src/kernel/buckyos-api/src/hybrid_sso.rs`：取绑定列表首项，空则取 DID 自身）。DID 若不是 Zone，后续解析 ZoneDocument 时自然失败。

- msg-center 解析的是 JSON `Value`：要么先解析成 OwnerDocument 再调用，要么按同一规则补“列表为空 → DID 自身”。
- 同步 `doc/message_hub/Contact Mgr.md` 与 `doc/message_hub/Message Center.md` 中“用户取 `binded_zone_list` 第一项”的描述。

**依赖**：`owner_canonical_zone` 属于混合 SSO 的改动，截至 2026-10-22 尚未提交。本项须在其提交后实施，或与之一同提交。

**验收**：

- 单测覆盖“自托管 owner → 自身 Zone”和“绑定他处 Zone”两种情形。
- ssotest 上 B 查 did:bns:alice 得到 `routable, zone:"alice.web3.devtests.org"`。
- bob 给 did:bns:alice 发一条原生消息，在 A 侧可收到（发消息路径参考 `test/test_msg_center/test_messagehub_groups.ts`）。

## F-5 `wan_dyn` / `portmap` 的 DNS 应答与设计说明不一致（P2，待确认）

`src/dev_configs/readme.md` 对两种形态的设计说明：

- bob（`wan_dyn`）：端口映射 443/80/2980，`zone_boot` 不配 SN，只用 SN 做 DDNS；
- charlie（`portmap`）：除 RTCP 外的流量都已映射。

实际情况：

- `make_config.ts` 对所有 `netid != "wan"` 的 Zone 都写 `bootDocument.sn` 和 `ddns_sn_url`（`usesSnRelay`）；
- SN 的 DNS 对 bob、charlie 都答 SN IP；SN 日志对 portmap 给出 `reason=explicit_non_wan_requires_relay`。

因此这两种形态实际上都走中转，readme 描述的直连从未生效；它们又会撞上 F-1、F-2。

需要先确认哪一边是目标设计：

- 若 readme 是目标：SN DNS 应对 `wan_dyn` / `portmap` 返回设备上报的地址，`make_config.ts` 对 `wan_dyn` 不写 `zone_boot.sn`；
- 若当前行为是目标：改 readme。

两仓 `devenv_config.ts` 有同步约定，改种子时一并处理。

## F-6 `user.invite.accept` 不验证接受者拥有目标 DID（P2，buckyos，安全）

**现象**：

- `user.invite.accept` 是公开方法（`src/frame/control_panel/src/sys_auth_backend.rs` 中无需会话）。
- 处理函数在 `src/frame/control_panel/src/user_mgr.rs`，约 1330 行起：
  - 只检查 `owner_config.id == target_did`、`owner_config` 声明绑定本 Zone；
  - 不校验 `owner_config` 的签名，也不要求证明持有该 DID 的密钥；
  - 可以顺带设置 `password_hash`；
  - 写入 `users/<id>/doc` 后，该 DID 的 DID 联合登录会映射到这个账号（verify-hub `account_for_did`）。

**风险**：拿到 invite_id 的任何人都能替目标 DID 接受邀请、设置密码，从而以本地账号身份使用；之后真正的 DID 持有者经联合登录也会落到这个已被他人设密码的账号上。

**处理**：按“先能力后安全”的原则，先由用户决定是否本期修。可选修法有两种，二选一：

- 接受时要求 DID 密钥对 invite_id 与 nonce 的签名；
- 改为“先以该 DID 联合登录到本 Zone（External），再在会话内接受”。

不修的话，至少写进 `doc/arch/10_user_lifecycle_and_permissions.md` 的已知风险。

## F-7 本地种子目录里的 owner 口令哈希过期（P2，buckyos 开发工具）

**现象**：本机 `~/buckycli/{alice.bns.did,bob.bns.did,charlie.me}/ood1/start_config.json` 的 `admin_password_hash` 是旧工具写的共用值，由 devtest 派生（`o8XyToejrbCYou84h/...`）。`make_config.ts` 原样复制 `start_config.json`，而 `seedIdentitySourceHash` 不覆盖这个文件，所以不会被判为过期。结果是 VM Zone 的 owner 用 `bucky2025` 登录不上。

新生成的种子目录不受影响：websdk `provision.ts` 写的是 `hashAdminPassword(username, 'bucky2025')`。

2026-10-22 已手工把本机三个文件改成按账号名派生，原文件备份为 `.bak-20261022`。

**建议修法**：`make_config.ts` 在物化 `start_config.json` 时，按种子事实（用户名 + 开发口令）写入或校验 `admin_password_hash`，发现不一致就报错并提示重建种子目录。不要静默兼容。

**验收**：用旧种子目录跑 `build_for_vm_test.py --node-group alice.ood1`，要么写出正确的哈希，要么明确报错。

## F-8 devkit PVE 建机：guest agent 超时与失败处理（P2，buckyos-devkit）

**现象**：

- `ubuntu_basic` 的 cloud-init 先做 `package_upgrade` 再装 `qemu-guest-agent`。宿主机忙时，`agent_timeout` 默认的 180 秒不够；alice、bob 因此在创建中途被销毁。目前 `ssotest.json` 用 `vm_params.agent_timeout: 900` 绕过。
- `Workspace.create_vms` 忽略 `create_vm` 返回的 False，继续创建后续节点，最后在 instance_commands 阶段报 `PVE VM not found: alice-ood1` 退出。哪些节点失败没有汇总输出。
- devtest 的输出被缓冲，要设 `PYTHONUNBUFFERED=1` 才能实时看到进度。

**建议修法**（`buckyos-devkit/src/remote/vm_mgr.py`、`worksapce.py`）：

- 让 agent 先于全量升级可用：例如在 `bootcmd` 或 `packages` 阶段先装并启动 agent；或者把 PVE 的默认超时调大。
- `create_vms` 汇总失败的节点并以非零退出，跳过这些节点的 instance_commands。
- 入口处设置行缓冲。

**验收**：在 `ssotest.json` 不配 `agent_timeout` 的情况下，与一次 `cargo build` 并行建机能成功；故意让一台失败时，输出列出失败节点并非零退出。

## F-9 小问题（P3）

- 服务重启后的一两分钟内，`POST /kapi/aiworkspace` 返回 `405 Method not allowed`，随后自行恢复（路由在服务实例上报后才登记）。未登记的服务最好返回 503 或明确的“服务未就绪”，405 会误导排查。来源未定位：多个内核服务的 HTTP server 都会返回这段文本，先查网关对未登记的 `/kapi/<service>` 落到了哪个上游。
- `ubuntu_sn` 模板设置了 `manage_etc_hosts: false`，`hostnamectl set-hostname sn` 之后每条 `sudo` 都打印 `unable to resolve host sn`。可以在 init_commands 中把主机名写进 `/etc/hosts`。

## 修复验收记录（2026-10-22）

### 实现补充

- **F-1 根因修正**：原探针的 context 级 `ignoreHTTPSErrors` 在该 Chromium 环境触发证书重试，放大 TLS 连接数；信任开发 CA 或使用 browser 级证书忽略时仅 7 条 TLS 连接，未发现 HTTP keep-alive 被中转主动关闭。仍修复了真实的建流配额持有过久问题，以及限流拒绝直接透传连接关闭的问题；保留原限额，只重试明确的限流/配额拒绝，不重试鉴权失败，不重放已接受的流。
- **F-2/F-5**：自定义域名中转使用 `did:web:ood1.charlie.me`，不再由 SN 用户名拼 BNS 身份。SN 自身也需能解析自定义域名，因此 `ssotest.json` 配置 systemd-resolved 使用本机 SN DNS，并保留公共 DNS；已在 VM 重启 resolved 后验证。`wan_dyn` 保留 DDNS、移除 Zone/boot SN；`portmap` 保留 SN 候选，但有上报地址时直接解析到设备。
- **F-3**：`node_gateway_params.json` 中 `rtcp_port` 是字符串，供网关参数展开；缺省为 `2980`，拒绝 0 或超过 65535。联动 `buckyos-websdk` 的普通身份保存、离线激活产物和已生成 dist，防止后续激活重新丢失端口。
- **F-4**：实投还发现 scoped Device 文档被泛型解析成 Zone，以及 Alice 的本地 bobfed 映射遮蔽发送者 canonical Bob Zone。已分别改为显式验证 Device 签名，以及允许消息来自本地托管 Zone 或 owner canonical Zone，并补回归测试。
- **F-6**：`user.invite.get` 保持公开；`accept` 允许 External 联合登录会话，但拒绝匿名、其他 DID、密码、专属免密邀请、App/设备及非 Control Panel 会话。仅可更新本邀请明确预留的 Pending 账号，账号/文档/资料/邀请状态在同一事务中提交，邀请版本 CAS 防止重复消费；读取异常不当作记录不存在。前端入口 `/users/invite/:inviteId` 保留完整邀请返回地址，先验证 DID，再提交 OwnerDocument。原 `/sso/invite` 专属免密登录流程保留。
- **Control Panel 实测补充**：debug 构建的大型 async 路由分派器在登录时栈溢出。改为同步选择 boxed handler 后再 await，保持全部路由映射；新增 1 MiB 工作线程的真实 RPC 调用回归，未通过增大生产线程栈掩盖问题。

### 自动化验证

以下为本次实际执行的针对性检查，Cargo 测试均使用 `--test-threads=1`：

| 范围 | 结果 |
| --- | --- |
| cyfs-gateway RTCP 单测 | 79 通过，1 个既有用例忽略 |
| cyfs-sn resolver 单测 | 48 通过，包括无效/缺失权威 DID 拒绝 |
| SN 种子生成 Deno 测试 | 14 通过 |
| BuckyOS 配置生成 Deno 测试 | 4 通过，包括三种 owner 旧口令哈希拒绝、直连配置和 RTCP 参数 |
| msg-center | 178 通过，1 个需 Telegram 凭据的既有用例忽略 |
| HomeStation | 24 通过 |
| buckyos-api owner canonical / gateway params | 2 + 1 通过 |
| Control Panel | 最终全套 187 通过，含邀请事务、鉴权和小栈路由回归 |
| Desktop | 新邀请入口 E2E 6 通过；既有 SSO 回归 17 通过；类型检查、改动文件 ESLint 和生产构建通过 |
| boot gateway debug fixtures | 32 通过，含根域与 App 域未登记服务返回 503 |
| buckyos-websdk provision | Jest 9 通过，SDK 构建通过，dist 同步生成 |
| buckyos-devkit | remote 52、console 3、CLI 13 通过，包括创建失败汇总与退出码 |

已构建 `web3_gateway`、`cyfs_gateway`、`node_daemon`、`msg_center`、`homestation`、`control_panel` 和 Desktop Web。未执行完整 Rust workspace 测试或全量 `build_for_vm_test.py`。本机忽略跟踪的旧 Deno lock 与可变 `main` 远程 bundle 校验和不符，BuckyOS 配置测试通过本地最新 SDK import-map 或 `--no-lock --reload` 执行；SN 测试临时移开旧 lock 后执行并恢复，未手改锁文件或新增依赖。

### VM 验收

- Alice 经 SN：原冷加载探针 `/login` 134 请求、`/` 252 请求，失败均为 0；canonical Alice 与 target Charlie 都映射到 SN 的 Playwright 联合 SSO DV 2/2 通过。
- Charlie 强制中转：`curl --resolve charlie.me:443:192.168.1.253 --cacert ~/buckycli/ca/buckyos_test_ca_ca_cert.pem https://charlie.me/` 返回 200；移除 Alice/Bob 的 `charlie.me` hosts 映射后，Node 联合 SSO 的 `bound-canonical` 验收通过。
- DNS：SN 对 Alice 回答 `192.168.1.253`，Bob 回答 `192.168.1.221`，Charlie 回答 `192.168.1.113`。SN 本机重启 systemd-resolved 后仍能解析 Charlie。
- Charlie 端口：更新 node-daemon 与 boot gateway 后，`ss -lntp` 仅见 RTCP 2981；Bob 的 tunnel probe 已到达正确的 `.113:2981`，握手因 `same_zone` 拒绝。按用户确认保留隔离，未放宽 RTCP 入站策略。
- 原生消息：新增 [native route DV](../test/test_msg_center/README.md)，Bob 查询 Alice/Charlie 均 routable；Bob→Alice 消息在 Alice INBOX 收到（direction=in），原 bobfed 映射保持。
- 邀请接受：新增 [user invite DV](../test/test_hybrid_sso/README.md)，最终 Control Panel 与 Desktop Web 已部署 Alice/Bob/Charlie，三 Zone owner 登录正常。Charlie 经 `charlie.me` 联合登录后接受 Alice 邀请成功，重登映射为 `multi_zone_charlie`；匿名、Bob DID、Alice 密码、同 DID 密码会话、target mismatch、过期邀请均被拒绝，失败后邀请保持 pending。首次创建与再次运行已有账号分支均通过。保留该普通测试账号、`multi-zone-charlie-f6` 邀请及拒绝探针记录供复验。
- PVE：在 Cargo 构建期间，以无 `agent_timeout` 覆盖的 ubuntu_basic 创建临时 VM，guest agent 在 106.87 秒可用，总耗时 139.99 秒，cloud-init 与 uv/deno 安装完成；验收后仅删除该临时 VM 和片段，原 VM 100～110 保留。失败场景由 devkit 自动化测试验证，未破坏现有节点。
- SN 主机名：模板改为 `hostname: sn` / `manage_etc_hosts: localhost`；当前 VM 的 hosts 已补 `127.0.1.1 sn`，`getent hosts sn` 与 sudo 无主机名告警。
- 未登记服务：Charlie 实测 `POST /kapi/multi_zone_missing` 返回 HTTP 503，响应体为 `service_not_ready:multi_zone_missing`。

改动分布在 `buckyos`、`cyfs-gateway`、`buckyos-devkit`、`buckyos-websdk` 四个仓库，需一起交付。相关架构/API/开发环境文档与 `buckyos_admin` 命令手册已同步；本次未改变发布命令或 Agent 工具协议。四仓 `git diff --check` 通过，未提交 Git commit。

当前 VM 保留部署前备份和带验收标识的消息：SN 的 `/opt/web3-gateway/web3_gateway.before-multi-zone`、Alice 的 `/opt/buckyos/bin/cyfs-gateway/cyfs_gateway.before-multi-zone`、Charlie 的 `/opt/buckyos/local/multi-zone-backup`；三 Zone 的 `/opt/buckyos/bin/control-panel/control_panel.pre-multizone-f6-1792689576` 和同目录的 `web.pre-multizone-f6-1792689576`。最终 Control Panel SHA256 为 `9c8662e531453eecae8420d5256f33775e6f8c42b07eb463e6e53ebd9a8d48de`；SN web3_gateway 为 `c0426d43553262c3f70ccdaccd4756ebe18820bdcc9de7a4fd6e59e26dd5df0b`。临时部署 HTTP 服务已停止。HomeStation 通过构建和单测，本次未另做其 UI VM 验收；IPv6-only 拓扑未实测。
