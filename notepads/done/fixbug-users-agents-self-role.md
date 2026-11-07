# Users & Agents 将普通用户误标为 Owner

2026-10-27，dvtest（`test.buckyos.io`）。真实 bob 登录后，`user.get` 返回 `user_type: user`，旧版侧栏却显示 Owner，已通过浏览器复现。

根因是 `SelfDetailPage`、`EntityCard` 和 `MobileHomeScreen` 把当前登录用户 `kind: self` 固定显示为 Owner。后端和 SDK 返回的类型正确；`admin` 是管理账号，只有不可登录的 `root` 记录代表 Zone owner。

修复在 `SelfEntity` 中保留经过 `parseUserType` 解析的 `role`，三处展示都使用该角色；初始化或缺少类型时显示 Unknown type。Mock 同步角色，保留中英文标签。未修改后端授权、SDK、协议、用户数据或依赖。

验证（命令在 `src/frame/desktop` 下执行）：

- `pnpm run check`：通过。
- `pnpm exec eslint` 检查全部修改的 TypeScript 文件及新增 DV 脚本：通过。
- `deno test --config tests/datamodel/messagehub.deno.json --no-check --allow-read --allow-env tests/datamodel/users-agents-mapping.test.ts`：5 通过，覆盖 self 的 user/admin/limited/root、未知类型及未加载状态。
- `VITE_CP_USE_MOCK=false pnpm run build`：通过；保留已有大 chunk 警告。
- 设置 `BUCKYOS_TEST_MEMBER_PASSWORD`、`BUCKYOS_TEST_ADMIN_PASSWORD` 后，`BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/users-agents-role.spec.ts --output=/tmp/users-agents-role-after`：4 通过。bob 显示 User，devtest 显示 Admin；覆盖桌面侧栏/详情及 375px 手机卡片/详情，8 张截图均已审查。

DV 中 Chromium 因测试 CA 信任问题报告 Service Worker SSL 错误，沿用现有测试的 `known-environment-error` 注解记录；没有其他 console/pageerror。本次没有验证离线启动，也没有运行无关 Rust 测试。

构建产物已更新至本机 `/opt/buckyos/bin/control-panel/web`，355 个文件逐一核对哈希；旧静态资源保留，未重启服务。旧包备份为 `/home/lzc/project/deployment-backups/20261027-202520-users-agents-role/web`。修复前证据位于 `/tmp/users-agents-role-before`，修复后位于 `/tmp/users-agents-role-after`。

已检查 `src/apps/jarvis_agent/agent/cmd_skills/buckyos_admin.md`：其账号类型定义与当前后端一致。本次仅修正前端展示，不改变命令、API 或操作流程，无需更新手册。
