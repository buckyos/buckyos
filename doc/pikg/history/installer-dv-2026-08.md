# Installer DV：2026-08 历史记录

> 历史验证参考。以下保留原 README 的带日期结果，不代表当前源码或环境已验收，尤其旧 Agent runtime 用例不等于当前模板注册流程。当前入口见 [Installer 测试](../../../test/app_installer_test/README.md)。

> **本机 DV 运行结论（2026-08-24, macOS ood1）**：
> - static web 用例通过真实 NodeGateway 链路（PIKG build/pack/info → NDM put/finalize →
>   `apps.inspect` → fingerprint-bound `apps.submit` → TaskManager → scheduler →
>   node-daemon 落盘 → `install_record(state=installed)`）；同一用例还验证同 AppDID
>   的两个 owner 得到不同 AppInstanceId/hostname/AppIndex，以及 v1→v2 原位升级后
>   AppInstanceId、AppName、hostname、AppIndex 和 registry allocation 保持不变。
> - Agent runtime PIKG 以 `auto_start=false` 安装，验证它不会在缺少独立 AgentSpec binding 时
>   把 AppDID 伪装成 AgentDID；Docker app case 依赖可用的 Docker daemon。
> - LoginAssertion 凭证：优先 zone owner key；否则从
>   `/opt/buckyos/etc/node_identity.json` 解析当前设备，读取
>   `/opt/buckyos/security/<device-host>/authentication.private.pem`。
> - `resolver/cache/*` 夹具写入使用 Verify Hub 的 System(control-panel) sudo session；
>   boot LoginAssertion 不再作为运行期 system-config 通用写凭证。设置 Zone URL 与管理员
>   密码后，static-web 还验证真实 App SSO、精确 AppInstance/owner claims 和跨 owner refresh 拒绝。
> - fixture 的 pkg 名由 `npx buckyos pikg` 按 App DID 派生的 raw-hostname
>   AppId namespace 生成（如 `all.web.<app_id>`）。不属于该 AppId namespace 的名字会在 Inspect
>   阶段以 `APP_PACKAGE_NAMESPACE_MISMATCH` 拒绝。

> **PIKG 样例更新（2026-08-22）**：测试现场复制 `pikg_samples/` 中的标准工程，
> 使用 `npx buckyos pikg build/pack/info` 构造并校验 `.pikg`，再放入本机受控
> NDM staging。测试
> 先向 zone resolver 数据面（`resolver/cache/{did}/app/{state|doc}`，root 权限）
> 种入解析证据，再走 `apps.inspect(staging_handle)` → `apps.submit` →
> 严格等待 `Completed`（不再接受
> "等待 ready 超时也算通过"）。完成后断言 `users/{uid}/apps/{app}/install_record`
> （state=installed、task_id、proof 回填）与运行证据。
>
> 已知缺口：Control Panel 中途重启恢复用例需要能重启服务进程的 DV 编排，
> 暂未在本 node 测试内实现（恢复语义已由 control_panel 引擎单测覆盖）。
