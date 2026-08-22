> **本机 DV 运行结论（2026-07-16, macOS ood1）**：
> - static web 用例全链路通过（pikg build/pack→stage→种证据→install_package→confirm→
>   Acquire/Verify/Prepare/Deploy/Activate→install_record(state=installed)）。
> - agent 用例在 Linux DV 才能过：macOS 上 Docker Desktop 默认文件共享不含
>   `/opt/buckyos`，容器 bind mount 报
>   `error while creating mount source path '/host_mnt/opt/buckyos/...'`。
>   需在 Docker Desktop Settings → Resources → File sharing 手工加入
>   `/opt/buckyos` 后重跑（属本机 Docker 配置，测试不代改）。
> - `BUCKYOS_TEST_SKIP_DOCKER=1` 可跳过 docker 用例（本机镜像拉取被网络阻断时用）。
> - 登录凭证：优先 zone owner key；否则从
>   `/opt/buckyos/etc/node_identity.json` 解析当前设备，读取
>   `/opt/buckyos/security/<device-host>/authentication.private.pem`。
> - fixture 的 pkg 名由 `buckyos-tool pikg` 按 App DID 派生的
>   `$owner_$app-$subpackage` namespace 生成。未绑定 App DID 的名字会在 Inspect
>   阶段以 `APP_PACKAGE_NAMESPACE_MISMATCH` 拒绝。

> **PIKG 样例更新（2026-08-21）**：测试现场复制 `pikg_samples/` 中的标准工程，
> 使用 `buckyos-tool pikg build/pack/info` 构造并校验 `.pikg`，再放入本机受控
> staging 目录。测试
> 先向 zone resolver 数据面（`resolver/cache/{did}/app/{state|doc}`，root 权限）
> 种入解析证据，再走 `apps.install_package(staging_handle)` →
> 等待 `WaitingForApproval` → 读取 Task.data 中的持久 plan（断言
> `OFFLINE_READY`）→ `apps.install.confirm` → 严格等待 `Completed`（不再接受
> "等待 ready 超时也算通过"）。完成后断言 `users/{uid}/apps/{app}/install_record`
> （state=installed、task_id、proof 回填）与运行证据。Agent 用例已恢复启用；
> Docker 用例仍在无 docker 环境下跳过。
>
> 已知缺口：Control Panel 中途重启恢复用例需要能重启服务进程的 DV 编排，
> 暂未在本 node 测试内实现（恢复语义已由 control_panel 引擎单测覆盖）。

# app_installer_test

独立工程示例，直接通过 `package.json` 里的 GitHub 依赖安装 `buckyos`：

```json
{
  "dependencies": {
    "buckyos": "git+https://github.com/buckyos/buckyos-websdk"
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

现场构造并离线校验四类 `.pikg` 样例：

```bash
pnpm run generate:pikg-samples
```

`pikg_samples/` 只保存 static web、script host、agent 和 Docker 的标准构建工程，
不保存 `dapp_dist/` 或 `.pikg`。生成器调用仓库中的 `buckyos-tool pikg`
完成 build、pack、info，产物默认写入系统临时目录
`buckyos-pikg-samples/`；可通过 `BUCKYOS_PIKG_OUTPUT_DIR` 改到其它目录。

可选环境变量：

```bash
BUCKYOS_SYSTEM_CONFIG_URL=http://127.0.0.1:3200/kapi/system_config
BUCKYOS_CONTROL_PANEL_URL=http://127.0.0.1:4020/kapi/control-panel
BUCKYOS_VERIFY_HUB_URL=http://127.0.0.1:3300/kapi/verify-hub
BUCKYOS_TASK_MANAGER_URL=http://127.0.0.1:3380/kapi/task-manager
BUCKYOS_TEST_OWNER_DID=did:bns:root
BUCKYOS_TEST_DOCKER_BASE_IMAGE=busybox:1.36.1
BUCKYOS_TEST_TOOL_PATH=/path/to/buckyos-tool/buckyos
BUCKYOS_PIKG_OUTPUT_DIR=/tmp/buckyos-pikg-samples
BUCKYOS_ROOT=/opt/buckyos
BUCKYOS_TEST_INSTALL_EVIDENCE_TIMEOUT_MS=120000
BUCKYOS_TEST_UNINSTALL_AFTER_INSTALL=0
```

示例代码从发布包的 Node 入口导入：

```js
import { buckyos, RuntimeType, parseSessionTokenClaims } from 'buckyos/node'
```

注意：

当前只有在 GitHub 上的 `buckyos/buckyos-websdk` 已经包含 `./node` 条件导出和 AppClient 实现时，这个示例才能直接跑通。
如果仓库还没推送到包含这些改动的提交，`pnpm install` 虽然会成功，但 `pnpm run demo` 会因为找不到 `buckyos/node` 而失败。

`pnpm test` 默认会按以下顺序执行：

1. 把 `pikg_samples/` 中的构建工程复制到临时目录，并写入本次测试的 App ID/version
2. 调用 `buckyos-tool pikg build/pack/info` 现场构造并校验 `.pikg`
3. 把 `.pikg` 放入本机 Control Panel staging 目录，再调用 `apps.install_package`
4. 等待安装完成并验证 system_config / task-manager / runtime 中的结果

如果你要恢复“安装后自动卸载”的完整生命周期测试，显式加上：

```bash
BUCKYOS_TEST_UNINSTALL_AFTER_INSTALL=1 pnpm test
```

测试目录下已包含四类本地构造 App 的完整 PIKG 工程：

- `pikg_samples/static-web/`：静态网页
- `pikg_samples/script-host/`：Host Script Python 服务
- `pikg_samples/agent/`：Agent 行为与 prompts
- `pikg_samples/docker/`：Dockerfile、入口脚本及 PIKG 元数据

说明：

- 测试默认使用本机 zone owner 或当前 device 信任凭证生成初始 JWT，并把 `appid` 固定成 `control-panel`。
- 自签 JWT 之后，测试会显式调用 `verify-hub.login_by_jwt`，换取 `control-panel` 可接受的 verify-hub session token。
- `app_installer_flow.test.mjs` 不再允许通过环境变量覆盖测试 `appid`。
- 当前自签 token 的 `sub` 固定为测试用户，签名 key 来自 zone owner 或当前 device 的 `authentication.private.pem`。
- PIKG 构造是完全离线的，不再依赖 `app.publish` 或 `repo-service`；安装测试仍需运行中的 BuckyOS DV 环境。
- 测试里生成的 App/subpackage version 保持在 `0.1.x` 且 `x <= 65535`。
- static web case 按 `/opt/buckyos/bin/<owner>_<app>-web` 是否落地来判断安装成功，不依赖 ready 状态。
- agent case 会严格等待安装完成并验证 OpenDAN pid 文件。
- docker case 按容器是否已运行来判断安装成功。
- docker case 会先在本地 `docker build`，再由 `buckyos-tool pikg build` 固定镜像 ID 并导出 payload。
- 如果当前机器没有可用的 Docker daemon，docker case 会被跳过；web 和 agent case 仍会执行。
- 当前默认不会自动卸载已安装 app，也不会清理对应 docker image，方便安装后观察实际落地状态。
