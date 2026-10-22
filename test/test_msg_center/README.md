# Message Center DV

`test_native_routes_dv.mjs` 验证 `src/dev_configs/ssotest.json` 的三 Zone 环境：Bob 查询自托管 Alice 与绑定 Charlie 的 Zone，再发送一条原生消息到 Alice，等待其进入 Alice 的会话。会写入一条带 `multi-zone F-4 native-route acceptance` 标识的验收消息，保留在两端历史中；不修改联系人和用户设置。

在仓库根目录运行，要求三个 Zone 已激活、msg-center 可用，宿主机安装 Node.js：

```bash
BUCKYOS_TEST_DNS=<sn ip> node test/test_msg_center/test_native_routes_dv.mjs
```

环境参数与混合 SSO DV 一致：`BUCKYOS_TEST_DNS` 为必填 SN DNS 地址；`BUCKYOS_TEST_CA` 默认为 `~/buckycli/ca/buckyos_test_ca_ca_cert.pem`；`BUCKYOS_TEST_PASSWORD` 默认为开发口令 `bucky2025`。`BUCKYOS_TEST_ZONE_A/B/C` 可覆盖 Alice、Bob、Charlie 的主机名，默认分别为 `alice.web3.devtests.org`、`bob.web3.devtests.org`、`charlie.me`。`BUCKYOS_TEST_HOST_MAP="host=ip,..."` 可覆盖个别主机解析。账号及 DID 使用种子中的 alice、bob、charlie。

现有 `test_messagehub_groups.ts`、`test_messagehub_sessions.ts`、`test_my_network.ts` 的单 Zone 用例及运行参数见各文件开头。
