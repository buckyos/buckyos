# Users & Agents 删除与恢复原账号（#685）

日期：2026-10-26。Issue：[buckyos/buckyos#685](https://github.com/buckyos/buckyos/issues/685)。

## 根因

`LocalUserDetailPage` 确认删除后同步调用 `removeLocalUser`；后者只过滤内存列表，没有调用已有的 `user.delete`。后端账号仍为 active，刷新会重新出现，同名创建被名称索引拒绝。

真正的 `user.delete` 采用软删除，保留 settings、DID 文档、profile、密钥及数据。名称仍被占用，新建事务也使用 `KVAction::Create`，不能仅放开名称检查来创建独立的新身份。删除原先在保存 deleted 后刷新 RBAC，刷新失败会返回 RPC error，无法区分状态写入与权限刷新。前端列表加载还会将 `user.list` 失败转成空列表。

## 修复

- 删除经管理员 sudo 提交 `user.delete`，等待响应及状态读回；请求中禁止重复提交，列表读取失败保留已有快照并支持只读刷新。
- 删除或恢复响应丢失时通过包含 deleted 的名单核实；读回也失败时保持“无法确认”，禁止重复提交，仅提供只读刷新；确认已提交时提示权限刷新结果未知。
- 管理员可以在桌面和手机端“已删除用户”页面明确恢复原账号。恢复保留原 DID、密钥、密码、角色、组和数据；同名创建返回 `user_deleted`，创建向导提供恢复入口。
- 删除与状态切换共用 CAS 写入，避免覆盖并发修改；禁止通过 `user.change_state` 绕过 root/本人保护。
- 返回 `state`、`rbac_refreshed` 与可选 `warning`，状态已写入而权限刷新失败时明确报告部分成功；前端保留警告。权限刷新结果不确定时不声称访问权限已全部撤销。
- Mock 持久保存账号状态，默认名单排除 deleted，显式读取可包含 deleted；支持创建、恢复和目标用户读回，避免刷新后从固定 seed 复活用户。

## 联动与验证

协议变化只涉及 Control Panel 的用户状态响应及创建错误码，Desktop 的 RPC 包装类型已同步；共享 `UserSettings`、存储结构和第三方依赖未变。核对 `buckyos-api` 和 `buckyos-websdk`：没有这些管理 RPC 的独立类型化调用需要更新。

受影响的手册为 `src/apps/jarvis_agent/agent/cmd_skills/buckyos_admin.md`，已同步软删除、恢复、部分成功与读回要求；协议说明更新于 `doc/control_panel/Control_Panel_Service.md`。修改 Agent 包内的手册源文件，没有修改安装目录或生成副本。

已通过：

- `cargo test -p control_panel user_mgr::tests -- --test-threads=1`：21 项，含 5 项新增回归。
- `cargo test -p verify_hub refresh_and_grace_recheck_the_account -- --test-threads=1`：1 项。
- `cargo test -p verify_hub only_active_users_can_receive_or_refresh_tokens -- --test-threads=1`：1 项。
- `pnpm exec playwright test tests/e2e/pages/users-agents-lifecycle.spec.ts`：新增 9 项全部通过，浏览器 console/pageerror 断言为空；涵盖提交等待、失败、恢复、同名冲突引导、持久化、读回失败、响应丢失、权限刷新警告和保护边界。
- 已有 `tests/e2e/pages/users-agents.spec.ts` 与 `user-groups.spec.ts`：10 项通过。
- `pnpm check`、`pnpm build`、改动前端文件的定向 ESLint、任务范围的 `git diff --check` 通过。ESLint 仅报告已有 `form.watch()` React Compiler 警告；构建有 chunk 大小提示，无构建错误。

真实 Zone DV 已扩展为创建 → UI 删除 → 登录拒绝 → 同名创建指向恢复 → UI 恢复原身份，`playwright --config=playwright.dv.config.ts --list` 选例解析通过；本轮未部署修复版本，未执行此在线流程。`uv run src/check.py` 只确认现有本机 Zone 核心服务和端口可用，不作为新版本验收证据。Rust 认证回归使用内存 Zone，不替代在线验收。未运行完整 workspace、完整发行包构建或其他平台部署验证。
