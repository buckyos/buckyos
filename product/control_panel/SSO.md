# BuckyOS 中的SSO 

> Migration note:
> - Canonical auth and session docs now live in `doc/control_panel/SPEC.context.md` and `doc/control_panel/CONTEXT.context.md`.
> - This file is retained as historical PRD input during migration.

SSO是 BuckyOS 为 Web 类应用提供的认证安全整体解决方案。

本文件现已降级为 historical stub，保留少量兼容背景和未定稿设想。

## Control Panel 路由约定（2026-03）

> Migrated-to: `doc/control_panel/SPEC.context.md` for current route surface.

- `/login`: control_panel 自身登录页面（登录后进入 desktop）
- `/sso/login`: SSO 授权弹窗页面（供其他应用获取授权）

## Historical Core Model

> Migrated-to: `doc/control_panel/SPEC.context.md` for contract-level auth rules, and `doc/control_panel/ARCHITECTURE.context.md` for session flow.

- Web 页面访问系统能力时，需要携带正确的 `session-token`。
- `session-token` 表示当前登录用户与 `appid` 绑定后的授权上下文。
- POST/kRPC 场景倾向走 client token 传递，GET 场景历史上常走 cookie。
- 当前 control panel 的 canonical auth surface 请以 `doc/control_panel/SPEC.context.md` 为准。

## 对兼容应用的支持

> Historical/planned note: this remains a compatibility model reference. Canonical current control-panel auth behavior is documented in `doc/control_panel/SPEC.context.md`.

- 兼容应用指未接入 `buckyos-web-sdk` 的历史 Web app。
- 历史兼容模型中，node-gateway 可把首个请求导向 `login_index.html` 完成 cookie 写入。
- 这是兼容迁移路径，不代表当前 control panel 主认证体验。

## 不要混用 app_service自己的 seession_token和来自页面的session_token

> Migrated-to: `doc/control_panel/CONTEXT.context.md` as a non-obvious architectural invariant.

- 历史兼容模型里，`app-web-page -> app-service -> kRPC` 链路必须继续使用来自页面的 token。
- 原因是页面 token 代表当前操作者，而 app service 自己的 token 往往只代表 app owner。
- 这个原则已经迁移为 canonical 的架构不变量，不应再只存在于旧 PRD 中。

## 更多实现细节

> Canonical split:
> - token contract -> `doc/control_panel/SPEC.context.md`
> - implementation caveats and security invariants -> `doc/control_panel/CONTEXT.context.md`

### session token

> Migrated-to: `doc/control_panel/SPEC.context.md`.

- 历史模型中，`session-token` 是由 verify-hub 私钥签名的 JWT。
- 一次 login 返回短期 `session token` 和长期 `refresh token`。
- 当 session 即将过期时，client 使用 refresh 流程换取新 token，旧 refresh token 立即失效。


### 超级用户

> Historical/planned note: retain here until sudo/superuser UX is formalized in canonical docs.

- 某些高权限页面曾规划为需要 sudo / 私钥签名授权。
- 因为系统不保存普通用户私钥，这类授权需要由 app 或签名页触发。
- 这部分仍属于未完全 canonicalize 的历史规划。
