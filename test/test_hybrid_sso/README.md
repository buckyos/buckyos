# Hybrid SSO DV

这些脚本使用 Node.js 24 的 TypeScript 类型擦除，无需安装依赖。运行前需要已激活且开启 DID 登录的 Alice、Bob、Charlie 三个测试 Zone，以及本机的测试 CA 和 Charlie OwnerDocument。DNS 默认由指定 SN 解析；HTTPS 会校验测试 CA。

```bash
BUCKYOS_TEST_DNS=192.168.1.253 node test/test_hybrid_sso/test_hybrid_sso_federation_dv.ts
BUCKYOS_TEST_DNS=192.168.1.253 node test/test_hybrid_sso/test_user_invite_dv.ts
```

`test_user_invite_dv.ts` 覆盖 multi-zone F-6：Alice 创建指向 Charlie DID 的邀请；匿名、Bob DID 会话、Alice 密码会话和同 DID 的密码会话均不能接受，拒绝后邀请仍为 pending；Charlie 经自己的 canonical Zone 完成联合登录后接受成功，重新登录映射为本地账户。另验证目标 DID 不符和邀请过期，HTTP/网络错误不能被当作权限拒绝通过。只检查三 Zone 密码登录、不修改邀请时可追加 `--preflight`。

测试保留 Alice 上的 `multi_zone_charlie` 普通账户和 `multi-zone-charlie-f6` 已接受邀请，以便重复运行；账户密码使用测试密码变量，已有 `bobfed` 不受影响。首次运行在邀请接受时设置密码以验证同 DID 密码会话仍被拒绝；重复运行沿用该账户。额外的拒绝探针不预留用户名，正常探针一小时过期，过期用例创建时已过期；脚本输出其邀请 ID 供检查。请仅在开发测试 Zone 运行。

可覆盖的环境变量：

| 变量 | 默认值 / 用途 |
| --- | --- |
| `BUCKYOS_TEST_DNS` | 必填，测试 SN DNS IP |
| `BUCKYOS_TEST_CA` | `~/buckycli/ca/buckyos_test_ca_ca_cert.pem` |
| `BUCKYOS_TEST_PASSWORD` | 三 Zone owner 及保留测试账户的开发密码，默认 `bucky2025` |
| `BUCKYOS_TEST_ZONE_A` | `alice.web3.devtests.org` |
| `BUCKYOS_TEST_ZONE_B` | `bob.web3.devtests.org` |
| `BUCKYOS_TEST_ZONE_C` | `charlie.me` |
| `BUCKYOS_TEST_CHARLIE_CONFIG` | 邀请测试使用的 `~/buckycli/charlie.me/user_config.json` |
| `BUCKYOS_TEST_HOST_MAP` | 可选 `host=ip,host=ip`，固定直连/中转路径 |

原 federation DV 的步骤筛选和 canonical Zone 停启参数见该脚本开头；停启命令未提供时会跳过 canonical-down 步骤。
