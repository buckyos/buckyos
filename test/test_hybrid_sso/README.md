# Hybrid SSO DV

## 当前原生 DID 回归

资源邀请用 Playwright 从真实分享 URL 进入，覆盖归属区确认、资源区的邀请用途页、原始 App URL 和实际资源读写。在 `src/frame/desktop` 运行，映射中的地址使用测试区的 OOD 或 SN。运行跨区账号资格用例时，提供第三个 Zone 的映射。

```bash
BUCKYOS_TEST_HOST_RULES='MAP bob.web3.devtests.org <bob_ip>, MAP charlie.me <charlie_ip>, MAP alice.web3.devtests.org <alice_ip>' \
pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/hybrid-sso-federation.spec.ts
```

管理员邀请入驻使用独立协议脚本。从仓库根目录运行，需要 Node.js 24、已激活且开启 DID 登录的 Alice、Bob、Charlie 三个测试 Zone，以及本机的测试 CA。DNS 由指定 SN 解析，HTTPS 校验测试 CA。

```bash
BUCKYOS_TEST_DNS=192.168.1.253 node test/test_hybrid_sso/test_user_invite_dv.ts
```

原生入驻前，Charlie 的受信 OwnerDocument 应已通过归属端发布对 Alice Zone 的绑定。协议 DV 从 Alice 发出的指定邀请启动，使用短时、仅限邀请的 HTTP Cookie 证明持有人身份，验证浏览器未收到根域会话。管理员邀请建立的本地账号 DID 为 `did:bns:multi-zone-charlie.alice`，同时保留 Charlie 的原生 DID 作为入站凭据。直接在请求 JSON 中填写 OwnerDocument 无法建立归属绑定。

`test_user_invite_dv.ts` 覆盖 multi-zone F-6：Alice 创建指向 Charlie DID 的邀请；匿名、Bob DID 会话、Alice 密码会话和同 DID 的密码会话均不能接受，拒绝后邀请仍为 pending；Charlie 经自己的 canonical Zone 完成联合登录后接受成功，重新登录映射为本地账户。Bob 给该账户的本区 DID 发出独立邀请后，账户以 Alice 为归属完成联合并在 Bob 取得邀请身份证明。原生 BNS 子账号仅在配置的 BNS 注册表确认该子名尚未独立注册时，才从直接父 Zone 的公开 HTTPS 解析端点读取有效的 OwnerDocument。另验证目标 DID 不符和邀请过期，HTTP/网络错误不能被当作权限拒绝通过。只检查三 Zone 密码登录、不修改邀请时可追加 `--preflight`。

测试保留 Alice 上的 `multi-zone-charlie` 普通账户和 `multi-zone-charlie-f6-v2` 已接受邀请，以便重复运行；账户密码使用测试密码变量，已有 `bobfed` 不受影响。首次运行在邀请接受时设置密码以验证同 DID 密码会话仍被拒绝；重复运行沿用该账户。额外的拒绝探针不预留用户名，正常探针一小时过期，过期用例创建时已过期；脚本输出其邀请 ID 供检查。请仅在开发测试 Zone 运行。

可覆盖的环境变量：

| 变量 | 默认值 / 用途 |
| --- | --- |
| `BUCKYOS_TEST_DNS` | 必填，测试 SN DNS IP |
| `BUCKYOS_TEST_CA` | `~/buckycli/ca/buckyos_test_ca_ca_cert.pem` |
| `BUCKYOS_TEST_PASSWORD` | 三 Zone owner 及保留测试账户的开发密码，默认 `bucky2025` |
| `BUCKYOS_TEST_ZONE_A` | `alice.web3.devtests.org` |
| `BUCKYOS_TEST_ZONE_B` | `bob.web3.devtests.org` |
| `BUCKYOS_TEST_ZONE_C` | `charlie.me` |
| `BUCKYOS_TEST_HOST_MAP` | 可选 `host=ip,host=ip`，固定直连/中转路径 |

### 历史基线

`test_hybrid_sso_federation_dv.ts` 记录旧实现的协议行为，包括未入驻的原生 DID 在资源区根域取得可刷新的 External 会话。新实现给访客签发目标 App 或 service 的受限会话，资源区系统登录另需管理员邀请或其他既有系统资格。复跑当前验收时使用上方 Playwright 与独立入驻脚本；复查旧版本时可按历史脚本开头的说明选择步骤或 canonical Zone 停启命令。

## OAuth 浏览器 DV

`src/frame/desktop/tests/dv/oauth-share.spec.ts` 使用两个开发 Zone，默认 `charlie.me` 接入来源，`bob.web3.devtests.org` 保持未接入 Google。每次运行创建独立的工作区、编辑组和原生测试账号。OIDC 和纯 OAuth 各自验证取消同意、未绑定账号系统登录被拒且无系统 cookie、未验证邮箱、定向与开放邀请、原 URL 返回、工作区写入和 owner 读回。同主体改邮箱不会取得新邮箱邀请，同邮箱换主体也不会接管旧授权。另一用例在未接入 Google 的资源区验证邮箱与 Google DID 的拒绝提示，然后用 `user.create` 建立的普通账号在归属区输入本地密码，使用该账号 DID 完成原生联合并编辑。

在测试网络中准备 `oidc.sso-dv.test` 与 `oauth2.sso-dv.test` 两个 HTTPS 名称，指向同一夹具，浏览器和接入来源的 Zone 都要能访问。给这两个测试域名签发短期服务器证书并让 CP 信任签发 CA。以下变量分别指定 PEM 文件、绑定地址、本区主机及共享的专用测试凭据文件。把同一凭据文件部署到该开发 Zone 的 `$BUCKYOS_ROOT/security/oauth/buckyos-dv`，限制文件权限。

```bash
BUCKYOS_DV_OAUTH_CERT=/path/test-cert.pem \
BUCKYOS_DV_OAUTH_KEY=/path/test-key.pem \
BUCKYOS_DV_OAUTH_SECRET_FILE=/path/dv-secret \
BUCKYOS_DV_OAUTH_BIND=<fixture_ip> BUCKYOS_DV_OAUTH_ZONE=charlie.me \
node test/test_hybrid_sso/oauth_fixture.mjs
```

在 `src/frame/desktop` 运行下列命令，`BUCKYOS_TEST_CA` 默认用 `~/buckycli/ca/buckyos_test_ca_ca_cert.pem`。端口默认 8443，其他端口或测试域名通过 `BUCKYOS_DV_OIDC_ORIGIN`、`BUCKYOS_DV_OAUTH2_ORIGIN` 同时传给夹具与 Playwright。测试网关的 HTTP/2 静态资源请求返回 403 时，可设置 `BUCKYOS_TEST_HTTP1=1` 让 Chromium 使用 HTTP/1.1。

```bash
BUCKYOS_DV_OAUTH=1 BUCKYOS_DV_OAUTH_CONFIGURE=1 \
BUCKYOS_TEST_HOST_RULES='MAP charlie.me <charlie_ip>, MAP bob.web3.devtests.org <bob_ip>, MAP oidc.sso-dv.test <fixture_ip>, MAP oauth2.sso-dv.test <fixture_ip>' \
pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/oauth-share.spec.ts
```

`BUCKYOS_DV_OAUTH_CONFIGURE=1` 允许测试使用管理员的公开 RPC 新增或启用专用的 `dv-oidc`、`dv-oauth2` 来源，结束时停用本次启用的来源并保留已登记的命名空间。已预配置时可省略此变量。夹具只向显式配置的开发 Zone 回调，其四个测试身份分别表示首次接受、同主体改邮箱、同邮箱换主体和邮箱未验证。

绑定 Google 的浏览器场景另需已配置 Google 客户端的开发 Zone、尚未绑定的专用测试邮箱 `BUCKYOS_DV_GOOGLE_TEST_EMAIL`，以及本机保存且能直接完成该客户端同意的测试浏览器状态文件 `BUCKYOS_DV_GOOGLE_STORAGE_STATE`。脚本用管理员邀请绑定 Google 主体并开通归属区普通账号，检查 Google 登录签发的会话为本地 Account，再以该账号的 DID 联合登录资源区。未提供这两项时仅该例跳过。Playwright 禁用该套件的 trace、截图与录像，以免保存凭据。

后端的可控 Google 回归通过本机 token/JWKS 交换和临时 RS256 密钥运行，涵盖既有外部授权、待接受邀请、账号绑定后的密码与 Google 登录，以及停用来源或暂停账号。

```bash
cargo test --offline --manifest-path src/Cargo.toml -p control_panel -p verify_hub -- oauth
cargo test --offline --manifest-path src/Cargo.toml -p control_panel google_enrollment_binds
```

2026-11-21 UTC 在三 Zone 候选版本以默认 Chromium 配置运行，6 项中 5 项通过、1 项跳过，用时 32.9 秒。浏览器在 Charlie 与 Bob 的登录入口均协商 HTTP/2。OIDC 与纯 OAuth 各通过两项：一项验证取消和未绑定账号的系统登录，另一项验证定向及开放文档分享，也覆盖同一邀请的取消、兑换前停用来源、邮箱未验证、恢复后原页重试、实际编辑和 owner 读回。重试保留邀请的查询参数和片段。收件人不匹配时页面隐藏接受按钮，带原证明直接调用接受接口也被拒绝。

第 5 项验证 Bob 在未接入 Google 时拒绝邮箱与合成 DID 邀请，然后按 Charlie 本地账号 DID 邀请，借助 Charlie 密码完成原生联合并编辑。第 6 项需要已同意授权的专用 Google 账号，本次跳过。受控 Google 签名验证、账号绑定及会话签发另有后端测试。测试结束后，专用来源已停用，临时 DNS、TLS 私钥与客户端凭据已删除。

同日更新 Bob 的桌面与 AI Workspace 后，单独运行 `--grep 'an open Workspace'` 通过了第六项（7.9 秒）。用例从系统登记的开放 URL 进入临时工作区，以 Guest 读取本次写入的表字段，并检查浏览器没有系统会话或续期 Cookie。另一位访客从同一邀请完成自助开户，返回原用途页，以本地账号 DID 接受并读回同一字段；管理员的接受记录使用该 DID，工作区仅授予读权限。工作区所有者关闭入口后，全新浏览器看见不可用提示，Guest 与自助注册按钮消失，直接加入和读取也遭拒绝。用例结束时移除自己创建的临时工作区，并将本次注册的测试账号标记为已删除。
