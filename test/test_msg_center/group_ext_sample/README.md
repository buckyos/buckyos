# Self-host Group 应用扩展最小样例（客服群）

对应 [应用扩展实施 TODO](../../../notepads/self-host-group-extension-todo.md) EXT-08：一个“客服群”应用，把 L1 配置、L2 面板、L3 Hook 和 L4A 代登录四件事各写成一个最小文件，供开发者照抄，也供 DV 驱动。全部 Deno，依赖只有 `../deno.json` 里的 `buckyos`（websdk node 构建）。

**分工（2026-10-24）**：真正装在 Zone 里、点按钮就能跑的官方例子是内置 App `buckyos-systest` 的客服群用例（[src/apps/sys_test](../../../src/apps/sys_test/README.md)，TODO §11）：建群 + 面板 + 48 小时客户链接（`user.app_login.issue {kind:"link"}`）。本目录保留它不覆盖的两件事：L3 Hook 验签服务与 L4A 一次性 code 代登录。

| 文件 | 层 | 作用 |
| --- | --- | --- |
| `setup_group.ts` | L1 | 以管理员身份原子建“客服群”：`support` 模板 + `tickets` Session、`display`、两个面板声明、`join` / `post` Hook；把群 DID 与 Hook secret 写进 `state.json`（secret 只在这一步出现一次） |
| `hook_server.ts` | L3 | Hook 服务：用 `state.json` 的 secret 验 `X-BuckyOS-Hook-Signature`（5 分钟时钟偏差），`join` 按白名单放行，`post` 对含 `#deny` 的消息拒绝 |
| `panel/index.html` | L2 | 面板页：读 `group_did` / `session_id` / `session` / `locale` 四个 query，用所在 App 子域的会话调 `/kapi/msg-center` `group.list_messages` 显示工单摘要；未登录时提示“在新窗口打开” |
| `app_backend.ts` | L4A | 模拟 CRM：`/login/<external_id>` 以 App 服务身份调 CP `user.app_login.issue` 领 code 并渲染兑换页；`/tickets/<sid>` 是客户看到的自定义工单页；同时静态托管 `panel/` |
| `dv.ts` | — | 真实 Zone 验收脚本（见下） |

## 运行

```bash
cd buckyos/test/test_msg_center/group_ext_sample
# 1. 建群（默认 test.buckyos.io / devtest / bucky2025，可用 BUCKYOS_TEST_ZONE_HOST 等覆盖）
deno run --config ../../deno.json --allow-net --allow-env --allow-read --allow-write --unsafely-ignore-certificate-errors setup_group.ts
# 2. Hook 服务（msg-center 容器要能访问这个地址；默认 0.0.0.0:8787）
deno run --allow-net --allow-env --allow-read hook_server.ts
# 3. App 后端 + 面板静态页（默认 0.0.0.0:8788；真实 Zone 需要把它作为 App 装到 <app>.<zone> 子域）
BUCKYOS_APP_TOKEN=<App 服务 token> BUCKYOS_APP_INSTANCE=<app_id@owner> deno run --config ../../deno.json --allow-net --allow-env --allow-read app_backend.ts
```

`setup_group.ts` 里的面板 `entry_url` 默认是 `https://${BUCKYOS_TEST_APP_ID:-crm}.${zone}/panel/`。D-2 要求面板 host 是群主 Zone 的 App 子域，所以要在真实 Zone 看到面板，App 后端必须以 AppId `crm`（或你改的名字）安装并由网关转发到 `app_backend.ts`；本地只跑脚本时面板声明照常通过校验，只是 Message Hub 加载不到页面，会显示“面板不可用”（这正是故障隔离要验的状态）。

Hook secret、App 服务 token、code 都不要进日志；`state.json` 只在开发机上用，不要提交。

## DV（`dv.ts`）

在仓库根目录对一个已激活的单 Zone 运行：

```bash
deno run --config test/deno.json --allow-net --allow-env --allow-read --unsafely-ignore-certificate-errors \
  test/test_msg_center/group_ext_sample/dv.ts
```

环境：`BUCKYOS_TEST_ZONE_HOST`（test.buckyos.io）、`BUCKYOS_TEST_ADMIN_USER` / `_PASSWORD`（devtest / bucky2025）、`BUCKYOS_TEST_MEMBER_USER` / `_PASSWORD`（bob / bucky2025）、`BUCKYOS_TEST_HOOK_URL`（msg-center 能访问到本脚本内置 Hook 服务的 URL，默认 `http://127.0.0.1:8787/hook`，容器内 msg-center 通常要写宿主机地址）、`BUCKYOS_TEST_APP_ID`（crm）。可选 `BUCKYOS_TEST_APP_INSTANCE` 加 `BUCKYOS_TEST_APP_TOKEN`（App 的 verify-hub 服务会话）或 `BUCKYOS_TEST_APP_LOGIN_JWT`（容器 env `BUCKYOS_APP_TOKEN` 那枚登录断言，脚本经 verify-hub `login_by_jwt` 换成会话）：给了才跑代登录一节。本机 DV 可直接 `docker inspect buckyos-app-<app> ` 取容器的 `BUCKYOS_APP_TOKEN`。

脚本按 TODO §6 EXT-08 的顺序覆盖，并在内部起一个 Hook 服务：

1. 建群 → `list_sessions` 带 `panels` / `display`，`get_config` / `list_sessions` 不含 Hook secret，`check_access(session.post)` 返回 `hook_dependent`；外部来源面板 `apply_config` 被拒。
2. Hook：签名正确放行；`decision: deny` 得 `hook-denied`；`rotate_hook_secret` 后服务仍用旧 secret → 验签失败按不可用（`deny` 策略）拒绝；换新 secret 后恢复。
3. 外部客户经专属邀请进访客路径：`auth.invite.login` 得到 callback，账号以 Limited 调 `group.list_sessions` 看到同一面板与 `display`，拿不到 `group_doc`。
4. （可选）App 代登录：owner 开 `apps.app_login.set`，App 领 code，`auth.app_login.exchange` 返回 App 子域的 `callback_url`，同 nonce 重试同结果、换 nonce 被拒；关闭开关后再领 code 被拒。
5. 删群收尾。

浏览器侧的真实 Zone 验收用 `src/frame/desktop/tests/dv/group-extensions.spec.ts`（以已安装的 App 子域充当面板来源）：

```bash
cd src/frame/desktop && BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_APP_HOST=buckyos-systest \
  pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/group-extensions.spec.ts
```

2026-10-24 两个脚本都在本机单 Zone 通过（记录见 TODO §10）。**仍未在真实 Zone 验证的项**：以 `app_backend.ts` 作为独立 App 装到子域后面板内容本身、App 子域 iframe 内的 `check_oauth` 302 与 host-only cookie 送达、停掉容器（网关回 5xx 页，iframe 内原样显示，宿主判不出）后的表现、代登录会话在关闭开关后浏览器侧的 refresh 拒绝。
