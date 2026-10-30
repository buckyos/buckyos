# jump

`https://jump.buckyos.org`（可由 Zone 设置 `system/sso/settings.jump_base_url` 覆盖）的静态页面。第三方网页不知道用户的 Zone 时，把用户带到 jump，由用户选择或沿用自己的 Zone，再跳到该 Zone 上同一路径的 action 页面。契约见 [混合 SSO 技术契约](<../../../doc/arch/verify-hub/混合SSO技术契约.md>) §11 与 TODO SSO-11.1。

- 登记的 action 与路径：`app_installer → /sysdlg/app_installer`（`jump-core.js` 的 `ACTION_PATHS`，与 WebSDK `JUMP_ACTION_PATHS` 保持一致）。未登记路径显示“不支持的操作”。
- 参数只放 fragment（不进入 jump 的 HTTP 请求），jump 去掉自己的候选参数 `zone` 后作为 query 带到 `https://<zone><action path>?<params>`，由目标 Zone 页面按登记的参数表校验（`app_installer` 接受 `identifier`、`ref`、`options` 或 `task_id`）。fragment 对页面脚本可见，不能放凭据；jump 不接收、不保存任何 token、secret 或私钥。
- 浏览器偏好 `localStorage["buckyos.jump"] = {current_zone_hostname, auto_jump}`：只有用户勾选才写入，可清除；存储不可用时回到手动选择。第三方在 fragment 里给的 `zone=` 只作为候选显示，不覆盖偏好，也不触发自动跳转。`auto_jump` 只省去选择 Zone，到达后仍要在该 Zone 登录并确认操作。
- 主机名只接受裸 hostname（拒绝协议、userinfo、端口、路径、query、fragment 与编码），允许任意自有域名。

## 部署

纯静态文件，无构建步骤、无第三方脚本：`index.html`、`jump-page.js`、`jump-core.js`、`jump.css`、`favicon.svg`、`favicon.ico`、`apple-touch-icon.png`，每个登记路径放一份 `index.html`（当前 `sysdlg/app_installer/index.html`，内容与根 `index.html` 相同）。服务端需返回以下响应头（`<meta>` 无法设置 `frame-ancestors`）：

```
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
Referrer-Policy: no-referrer
Cache-Control: no-store
X-Content-Type-Options: nosniff
```

测试：`deno test src/frame/jump/jump-core.test.js`。

未做：App / 扩展的 Active DID 提示与 SN 浏览器会话作为 Locator 来源（可选来源，缺失时用户手填即可）；正式域名的发布部署。
