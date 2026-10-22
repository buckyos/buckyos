# 三 Zone VM 环境暴露的问题 TODO

日期：2026-10-22。状态：未开始。

来源：在 PVE 多 VM 环境 `src/dev_configs/ssotest.json` 上做混合 SSO 三 Zone 验收（[混合 SSO 实施 TODO](hybrid-sso-implementation-todo.md) §11“三 Zone 验收”）时发现的问题。SSO 本身的问题已在该 TODO 内修复；本文只收与 SSO 无关、当时未修的部分，交给后续接手。

每项给出现象、证据、定位、建议修法和验收。标“待确认”的项先确认设计意图，不要直接改。

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
