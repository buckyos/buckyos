# sntest keep-tunnel 修复 TODO

## 背景

当前环境是刚通过 BuckyOS 仓库的 `buckyos-devtest sntest` 从零构建出的两 VM 测试环境：

- BuckyOS 仓库：`/Users/liuzhicong/project/buckyos`
- cyfs-gateway 仓库：`/Users/liuzhicong/project/cyfs-gateway/src`
- devtest group：`src/dev_configs/sntest.json`
- VM：`sn` + `alice-ood1`
- SN app：`web3-gateway`，由 BuckyOS 的 `src/dev_configs/apps/web3-gateway.json` wrapper 调用同级 cyfs-gateway 仓库中的 `build.py`、`make_sn_config.ts` 与 app 资源
- SN 部署目录：VM 内 `/opt/web3-gateway`
- Alice BuckyOS 部署目录：VM 内 `/opt/buckyos`

本轮构建时的 VM IP 是：

- `sn`: `192.168.252.18`
- `alice-ood1`: `192.168.252.19`

Multipass IP 不稳定，后续复现时必须通过 `buckyos-devtest sntest info_vms` 或 devtest 变量重新获取，不要写死上面的地址。

已通过的 smoke：

```bash
cd /Users/liuzhicong/project/cyfs-gateway/src
./web3-gateway/scripts/sn-dev-smoke.sh --vm \
  --expected-a 192.168.252.18 \
  --dns-server 192.168.252.18 \
  --http-origin http://192.168.252.18
```

结果是 `7 passed, 0 failed`，但这个 smoke 只覆盖了 DNS A/TXT 与 identifier resolver，不覆盖最基础的 OOD -> SN keep-tunnel。

用户已登录 VM 观察到：`alice-ood1` 到 `sn` 的 keep-tunnel 不成功。

## 问题

`sntest` 的核心目标之一是验证 NAT/LAN OOD 能通过 SN 建立 keep-tunnel。按设计，配置了 SN 且不是 `wan` 的 OOD 应该对 SN 建立 keep-tunnel；相关实现入口包括：

- `src/kernel/node_daemon/src/node_daemon.rs`
- `src/kernel/node_daemon/src/boot.rs`
- `src/rootfs/etc/boot_gateway.yaml`
- `src/dev_configs/sntest.json`
- `/Users/liuzhicong/project/cyfs-gateway/src/web3-gateway/scripts/sn-dev-smoke.sh`

当前 `sn-dev-smoke.sh --vm` 没有检测这个条件，导致 DNS 和 seed resolver 都通过时，keep-tunnel 仍可能是坏的。

## TODO

要求后续 CodeAgent 在现有 `sntest` 环境上下文中修复 keep-tunnel，并把 keep-tunnel 纳入 smoke 验收。

1. 先修复或绕过当前已记录的 `buckyos-devtest run` 缺陷，但正式复现与验证命令必须落回 `buckyos-devtest <group> run ...`，不能要求开发者直接调用 `multipass exec`。BuckyOS devtest 要保持 VM 后端透明。
2. 用 `buckyos-devtest sntest run alice-ood1 ...` 和 `buckyos-devtest sntest run sn ...` 复现 keep-tunnel 失败，记录稳定的检查方法。检查应覆盖：
   - Alice 侧生成的 `node_rtcp.keep_tunnel` 目标是否正确写入 gateway config。
   - Alice 侧 `cyfs_gateway` 是否实际加载 keep-tunnel 配置。
   - SN 侧是否能观察到来自 Alice OOD DID 的 RTCP tunnel。
3. 定位并修复根因。优先检查：
   - `make_config.ts` / `devenv_config.ts` 生成的 Alice `net_id`、SN host、zone doc 是否符合 keep-tunnel 判定。
   - `node_daemon` boot 阶段与运行期合并 `keep_tunnel` 的逻辑是否把 SN target 写进最终 `cyfs_gateway.yaml`。
   - SN `web3-gateway` 的 RTCP / relay / keep-tunnel target 配置是否监听并允许 Alice 建连。
   - linked `web3-gateway` app wrapper 是否遗漏了 cyfs-gateway 仓库新配置所需的 install/start/init 步骤。
4. 扩展 `/Users/liuzhicong/project/cyfs-gateway/src/web3-gateway/scripts/sn-dev-smoke.sh --vm`，新增 keep-tunnel 检查。该检查必须在 Alice 没有成功连上 SN keep-tunnel 时失败，不能只检查 DNS、HTTP 或 resolver。
5. 重新从当前环境验证，不要求重建 VM，除非修复涉及 VM cloud-init/template。最小验证应包括：
   - `uv run buckyos-devtest sntest info_vms`
   - `uv run buckyos-devtest sntest run alice-ood1 "<keep-tunnel alice side check>"`
   - `uv run buckyos-devtest sntest run sn "<keep-tunnel sn side check>"`
   - `./web3-gateway/scripts/sn-dev-smoke.sh --vm ...`，并确认新增 keep-tunnel case 通过

## 验收标准

- `alice-ood1` 能稳定与 `sn` 建立 keep-tunnel。
- `sn-dev-smoke.sh --vm` 包含 keep-tunnel case，且 keep-tunnel 断开时会失败。
- 全部修复与验证命令通过 `buckyos-devtest` 抽象执行，不把 `multipass` 作为正式开发流程入口。
- 若改动协议、配置字段或共享数据结构，必须同步检查 BuckyOS 与 cyfs-gateway 两个仓库的前后端、脚本和文档联动。

---

## 完成记录（2026-07-06，CodeAgent）

### 根因（SN 侧三层叠加，Alice 侧配置生成本来就是对的）

1. **SN 的 bns 权威指向生产环境**：SN VM 上没有 machine.json，web3_gateway
   进程内 name-client 的 `web3_bridge.bns` 落到内置默认 `web3.buckyos.ai`，
   RTCP 验证 Alice 的 device_doc_jwt 时去生产环境解析 `did:bns:ood1.alice` /
   `did:bns:alice`，两级验证全挂 → reject。make_sn_config.ts 里"SN 不消费
   machine.json"的旧断言是错的。
2. **链上种子 zone 文档不满足 ZoneDocument schema**：`bns_seed_docs/<u>/zone.json`
   只有 oods/sn/exp，name-lib 侧 `resolve_auth_key(owner)` 解析 zone 文档
   缺 id/verificationMethod/authentication/iat/hostname/owner/boot_jwt，
   parse 失败 → 自声明回落验证也挂。
3. **RTCP 授权 hook 要求 state=active（online）**：online 由 device.update
   上报驱动，而 keep-tunnel 正是设备变得可达的手段，鸡生蛋；且 node_daemon
   的 device.update 用 zone verify-hub token，SN 新 auth 只认自己签发的
   账号 token（协议断裂，已另立任务），设备永远 suspended → 永拒。

### 修复（全部落在 cyfs-gateway 仓库 + buckyos-devkit）

- cyfs-gateway `make_sn_config.ts`：新增 `writeMachineConfig`（bns bridge →
  `web3.<sn_base_host>`，纯 host，不能带 scheme——bridge 根域同时承担
  did:bns↔hostname 映射）；`toZoneDocumentJson` 把种子 zone 文档补全成
  合法 ZoneDocument（verificationMethod[0]=owner 公钥）。
- cyfs-gateway `web3-gateway/start.py`：启动前把 machine.json 装到
  `{BUCKYOS_ROOT:-/opt/buckyos}/etc/`，把部署目录 ca/ 下 dev CA 装入系统
  信任（reqwest 是 rustls-native-roots）。
- cyfs-gateway `web3-gateway/web3_gateway.yaml`：main_rtcp 两处 hook 的
  授权判定从 `eq state active` 改为 `!eq state banned`（注册且未 ban 即可）。
- buckyos-devkit：`devtest run` 修复（env_params TypeError、回显 stdout/stderr、
  退出码传播 exec_command 返回 (stdout,stderr,returncode)）；外部 app config
  新增 `{{app.dir}}` 变量（cyfs-gateway 的 web3-gateway.json wrapper 改用它，
  修复 build 命令在错误 cwd 执行）。buckyos 仓库 pyproject.toml 临时指向
  本地 devkit checkout（devkit fix 推上游后应改回 git source）。

### 稳定检查方法（全部走 devtest 抽象）

```bash
cd buckyos/src
# Alice 侧：keep_tunnel 目标已生成 + gateway 已加载 + TCP 长连接在
uv run buckyos-devtest sntest run alice-ood1 \
  "grep -o 'keep_tunnel[^]]*]' /opt/buckyos/etc/node_gateway.json; \
   ss -tnH state established '( dport = :2980 )'"
# SN 侧：来自 Alice 的 rtcp 长连接在（keep-tunnel 断开则此输出为空）
uv run buckyos-devtest sntest run sn \
  "ss -tnH state established '( sport = :2980 )'"
# SN 侧：拒绝原因排查（验证层 rtcp.rs reject / 授权层 process_chain reject）
uv run buckyos-devtest sntest run sn \
  "PID=\$(pgrep -f 'web3_gateway --config' | head -1); \
   sudo grep -E 'reject rtcp' /opt/buckyos/logs/web3_gateway/web3_gateway.\$PID.log | tail -5"
# smoke（S6 = keep-tunnel case，断开时 FAIL 且退出码非零）
cd cyfs-gateway/src && ./web3-gateway/scripts/sn-dev-smoke.sh --vm \
  --expected-a <sn-ip> --dns-server <sn-ip> --http-origin http://<sn-ip>
```

### 已知遗留（已开后续任务）

- devtest install 会把 sn.sqlite3 / sn_token_key 一并 push 覆盖（SN 会
  重新生成 auth key，已发 token 全失效）。
- bns_dv 种子 apply_mutations 对内容变化的重放会被 EVM revert，目前变更
  种子后需 `web3-gateway.init_anvil_fresh`。
- SN DNS 缺 `web3.<base>` A 记录（SN 自己靠 /etc/hosts，Alice 侧 BnsProvider
  解析不到，keep-tunnel 不依赖它所以不阻塞）。
- ~~node_daemon↔SN 的 device.update 认证协议断裂（见上），设备在线态暂时
  无法由上报驱动。~~ 已修复（2026-07-06）：SN 侧新增设备级凭证
  （`AuthContext::Device`，协议见 cyfs-gateway/doc/SN/SN-Auth.md「设备级凭证」
  小节，cyfs-gateway 19133dfb），node_daemon 改用
  `cyfs_gateway_api::generate_sn_device_token`（设备私钥签名，
  sub=did:dev:<x>、iss=did:bns:ood1.alice、aud=sn-device）上报。sntest 验证：
  `resolve_ood_by_did(did:bns:ood1.alice)` 在 OOD 上报后 state=active，
  alice 侧 node_daemon 每 30s `update ood1's info to sn ... success!`。
  查询命令：
  `curl -s -H 'Host: sn.devtests.org' http://<sn-ip>/kapi/sn/deviceinfo -d '{"method":"deviceinfo.resolve_ood_by_did","params":{"source_device_id":"did:bns:ood1.alice"},"sys":[1]}'`。
  遗留：激活流程 active_server.rs 的 generate_sn_device_proof 仍是旧式
  自签 proof（aud="sn"），对新 SN 无效，已另立任务。
