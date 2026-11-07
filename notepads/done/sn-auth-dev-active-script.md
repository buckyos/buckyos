# SN-Auth 两阶段：dev 脚本 RPC 激活路径（完成记录 2026-07-06）

背景：SN-Auth 两阶段改造后（自签 sn_device_proof 已删），active_server 的
do_active / do_active_by_wallet 在 sn_url 存在时强制要求 `sn_access_token`
（SN 账号 access token，aud="sn"，1h）。node_active 前端已跟进
（active_lib.ts acquire_sn_access_token）。本轮补齐 dev 脚本侧：make_config
系只做离线预生成、不触 RPC，dev 侧此前没有任何走 do_active 的路径。

## 交付

- 新增 `src/active_ood.ts`（deno，与 make_config.ts 并列）：对一台
  `node_daemon --enable_active` 的目标机走真实激活 RPC。
  1. `<schema>://sn.<sn_base_host>/kapi/sn/auth` 调 auth.login
     （`--active-code` 时 auth.register）拿 access_token；
  2. `<target>/kapi/active` 调 do_active，带 `sn_access_token`。
- 种子事实全部复用：group 参数取 devenv_config.ts，owner/device 密钥取
  websdk DEV_TEST_KEYS（与 make_sn_config.ts 写进 SN DB / BNS 链的同源），
  pwd_hash = b64(sha256(password+username+".buckyos"))（websdk hashPassword
  同构），admin_password_hash 与 SN 账号密码同源（前端 SecurityStep 同构）。
- netid 映射：devenv "lan" → RPC "nat"；wan/wan_dyn/portmap 原样。
  zone_name 传 group 的 zone_id（alice.bns.did → did:bns:alice；
  charlie.me → did:web，自有域名档）。
- BNS 发布默认跳过（种子 zone 文档已上链，重放 apply_mutations 会被 EVM
  revert）；`--bns-evm-key` + dv-env.json 的链参数可显式发布。
- kRPC 用 node:http 手写最小实现：fetch 按规范丢 Host 头（实测 Deno 如此），
  `--sn-ip` 需要 IP 直连 + `Host: sn.<base>` 做 vhost 匹配。
  `--sn-auth-url` 覆盖脚本拨号地址（本地 SN/非标准端口）；传给 do_active 的
  sn_url 始终是 OOD 视角可达的规范域名。
- 凭证纪律：token/pwd_hash/私钥不进日志（SENSITIVE_LOG_KEYS 与
  active_lib.ts、服务端 is_sensitive_param_key 同名单）；服务端已在落
  start_config.json 前剥离 sn_access_token 等激活期字段。
- make_config.ts printUsage 增加互链提示。

## 种子凭证（sntest）

- 用户 alice/bob/charlie 已预注册，密码 `devtest-pwd`（真值：cyfs-gateway
  src/make_sn_config.ts DEV_TEST_PASSWORD，同步进 sn_seed.yaml）。
- 激活码 SEED_ACTIVATION_CODES=["dev-code-1","dev-code-2"]；
  **dev-code-1 已被一次验证性注册消耗（用户 clsnauthtest）**，新用户注册
  测试用 dev-code-2 或先在 SN 侧重置。

## 验证

- stub 集成（本机，假 SN auth + 假 active server 对拍）：auth.login 请求
  形状、pwd_hash 双实现对拍（node:crypto vs WebCrypto）、sn_access_token
  透传、alice/bob/charlie 三组 net_id/zone/rtcp_port、拒绝路径退出码与
  提示、stdout 无 token/hash/私钥泄漏。全部通过。
- sntest 端到端（真实 VM）尚未跑：需要一台不做 make_config 预生成身份、
  只装 rootfs 起 `node_daemon --enable_active` 的 OOD VM。alice-ood1 的
  iptables 只 REJECT 80/443/2980（模拟 NAT），3182 默认放行，宿主机可
  直接 `deno run -A src/active_ood.ts alice.ood1 --target http://<vm-ip>:3182`。
  注意：对已由 make_config 激活过的 VM 重复激活会覆盖其身份文件。

## 用法速查

```bash
cd buckyos/src
# 种子用户登录激活（宿主机 DNS/hosts 已指向测试 SN）
deno run --allow-all active_ood.ts alice.ood1 --target http://<ood-ip>:3182
# 宿主机没配 DNS：IP 直连 SN（Host 头自动保持 sn.devtests.org）
deno run --allow-all active_ood.ts alice.ood1 --sn-ip <sn-ip> --target http://<ood-ip>:3182
# 注册流（消耗激活码；种子用户已注册会被拒，适用于 SN DB 重置后的首次注册）
deno run --allow-all active_ood.ts alice.ood1 --active-code dev-code-2 ...
```
