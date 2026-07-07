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
