# app_installer_test

打包输入及各样例边界见 [PIKG samples](pikg_samples/README.md)，协议、命令和其他资料见 [PIKG 文档总览](../../doc/pikg/README.md)。带日期的旧 DV 结论已移至 [历史记录](../../doc/pikg/history/installer-dv-2026-08.md)，当前运行需重新验证。

独立工程示例，直接通过 `package.json` 从 npm 安装最新的 `buckyos`：

```json
{
  "dependencies": {
    "buckyos": "latest"
  }
}
```

运行：

```bash
pnpm install
pnpm run demo
```

完整安装测试：

```bash
pnpm install
pnpm test
```

样例类型、生成命令、输出和 Docker 前提统一维护在 [PIKG samples](pikg_samples/README.md)，本页只维护安装测试。

可选环境变量：

```bash
BUCKYOS_SYSTEM_CONFIG_URL=http://127.0.0.1:3200/kapi/system_config
BUCKYOS_CONTROL_PANEL_URL=http://127.0.0.1:4020/kapi/control-panel
BUCKYOS_VERIFY_HUB_URL=http://127.0.0.1:3300/kapi/verify-hub
BUCKYOS_TASK_MANAGER_URL=http://127.0.0.1:3380/kapi/task-manager
BUCKYOS_NODE_GATEWAY_URL=http://127.0.0.1:3180
BUCKYOS_TEST_OWNER_DID=did:bns:root
BUCKYOS_TEST_DOCKER_BASE_IMAGE=busybox:1.36.1
BUCKYOS_TEST_SKIP_STATIC=0
BUCKYOS_TEST_SKIP_AGENT=0
BUCKYOS_TEST_SKIP_DOCKER=0
BUCKYOS_PIKG_OUTPUT_DIR=/tmp/buckyos-pikg-samples
BUCKYOS_ROOT=/opt/buckyos
BUCKYOS_TEST_INSTALL_EVIDENCE_TIMEOUT_MS=120000
BUCKYOS_TEST_UNINSTALL_AFTER_INSTALL=0
BUCKYOS_TEST_ADMIN_PASSWORD=<zone-admin-password>
BUCKYOS_TEST_ZONE_BASE_URL=https://<zone-host>
```

示例代码从发布包的 Node 入口导入：

```js
import { buckyos, RuntimeType, parseSessionTokenClaims } from 'buckyos/node'
```

注意：

npm 上发布的 `buckyos` 必须包含 `./node` 条件导出和 AppClient 实现，否则
`pnpm run demo` 会因为找不到 `buckyos/node` 而失败。

`pnpm test` 默认会按以下顺序执行：

1. 把 `pikg_samples/` 中的构建工程复制到临时目录，并写入本次测试的 App ID/version
2. 调用 `npx buckyos pikg build/pack/info` 现场构造并校验 `.pikg`
3. 通过 NodeGateway 的 `/ndm/proxy/v1` 上传并 finalize，再调用
   `apps.inspect` 和 `apps.submit`
4. 等待安装完成并验证 system_config / task-manager / runtime 中的结果；static web
   还会继续执行第二 owner 安装和同 owner 升级

如果你要恢复“安装后自动卸载”的完整生命周期测试，显式加上：

```bash
BUCKYOS_TEST_UNINSTALL_AFTER_INSTALL=1 pnpm test
```

说明：

- 测试默认使用本机 zone owner 或当前 device 信任凭证生成初始 JWT，并把 `appid` 固定成 `control-panel`。
- 自签 JWT 之后，测试会显式调用 `verify-hub.login_by_jwt`，换取 `control-panel` 可接受的 verify-hub session token。
- `app_installer_flow.test.mjs` 不再允许通过环境变量覆盖测试 `appid`。
- 当前自签 token 的 `sub` 固定为测试用户，签名 key 来自 zone owner 或当前 device 的 `authentication.private.pem`。
- PIKG CLI 构造不依赖 `app.publish` 或 `repo-service`；Docker fixture 的前置 build 可能拉基础镜像。安装测试仍需运行中的 BuckyOS DV 环境。
- 测试里生成的 App/subpackage version 保持在 `0.1.x` 且 `x <= 65535`。
- static web case 按 `/opt/buckyos/bin/all.web.<app_id>` friendly alias 是否落地来判断安装成功；实际运行内容仍由已 pin 的 Package Meta ObjectId 严格目录决定，不依赖 ready 状态。
- 卸载断言以 `Spec.state=deleted`、运行证据消失和 registry allocation 保留为准；PackageEnv 的严格目录/friendly alias 可作为缓存继续存在。
- 现有 agent runtime case 仍断言 inactive AppSpec/安装记录，属于旧 runtime 安装假设；当前公共 submit 已转为模板注册，该 case 不能作为现有模板流程的验收。需要排除该旧场景时设置 `BUCKYOS_TEST_SKIP_AGENT=1`；模板行为应另看 `agent_mgr.rs` 中注册/更新测试。本次只整理文档，未修测试或运行 DV。
- docker case 按容器是否已运行来判断安装成功。
- docker case 会先在本地 `docker build`，再由 `npx buckyos pikg build` 固定镜像 ID 并导出 payload。
- 如果当前机器没有可用的 Docker daemon，只有 docker case 会被跳过；三个 case 也可分别用独立的 skip 开关跳过。
- 当前默认不会自动卸载已安装 app，也不会清理对应 docker image，方便安装后观察实际落地状态。
