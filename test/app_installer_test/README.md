# app_installer_test

独立工程示例，直接通过 `package.json` 里的 GitHub 依赖安装 `buckyos`：

```json
{
  "dependencies": {
    "buckyos": "github:buckyos/buckyos-websdk"
  }
}
```

运行：

```bash
pnpm install
pnpm run demo
```

完整生命周期测试：

```bash
pnpm install
pnpm test
```

可选环境变量：

```bash
BUCKYOS_SYSTEM_CONFIG_URL=http://127.0.0.1:3200/kapi/system_config
BUCKYOS_CONTROL_PANEL_URL=http://127.0.0.1:4020/kapi/control-panel
BUCKYOS_VERIFY_HUB_URL=http://127.0.0.1:3300/kapi/verify-hub
BUCKYOS_TASK_MANAGER_URL=http://127.0.0.1:3380/kapi/task-manager
BUCKYOS_TEST_OWNER_DID=did:bns:root
BUCKYOS_TEST_DOCKER_BASE_IMAGE=busybox:1.36.1
```

示例代码从发布包的 Node 入口导入：

```js
import { buckyos, RuntimeType, parseSessionTokenClaims } from 'buckyos/node'
```

注意：

当前只有在 GitHub 上的 `buckyos/buckyos-websdk` 已经包含 `./node` 条件导出和 AppClient 实现时，这个示例才能直接跑通。
如果仓库还没推送到包含这些改动的提交，`pnpm install` 虽然会成功，但 `pnpm run demo` 会因为找不到 `buckyos/node` 而失败。

`pnpm test` 会按以下顺序执行完整流程：

1. 用本地 fixture 目录调用 `app.publish`
2. 再调用 `apps.install`
3. 验证 system_config / task-manager 中的安装结果
4. 调用 `apps.uninstall`
5. 再验证状态已进入 `deleted`

测试目录下已包含三类本地构造 app 所需配置：

- `fixtures/static-web/`: 静态网页内容
- `fixtures/agent/`: agent 行为与 prompts
- `fixtures/docker/`: 本地构建镜像的 Dockerfile 与入口脚本
- `fixtures/templates/*.json`: 三类 app 的 `app_doc` 模板

说明：

- 测试默认使用 `buckyos/node` 的 AppClient 本地自签方式生成初始 JWT，并把 `appid` 固定成 `control-panel`。
- 自签 JWT 之后，测试会显式调用 `verify-hub.login_by_jwt`，换取 `control-panel` 可接受的 verify-hub session token。
- `app_installer_flow.test.mjs` 不再允许通过环境变量覆盖测试 `appid`。
- 当前自签 token 的 `sub` 取决于本机找到的是 `user_private_key.pem` 还是 `node_private_key.pem`。
- `app.publish` 依赖 `repo-service`；测试启动时会检查 `services/repo-service/info`，缺失时直接报错。
- docker case 会先在本地 `docker build`，再 `docker save` 成 `amd64_docker_image.tar` 或 `aarch64_docker_image.tar`，然后再 publish。
- 如果当前机器没有可用的 Docker daemon，docker case 会被跳过；web 和 agent case 仍会执行。
