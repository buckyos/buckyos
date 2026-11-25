# GoDaddy

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。

## 适用条件和费用

- 域名的 DNS 在 GoDaddy。
- 账户里至少有 1 个域名（2026-04 起，以前要 10 个）。公告没有提到收费，每月 20,000 次请求、每秒 60 次。
- 旧的 API Key + Secret（sso-key）已标为对域名 API 弃用，这里不用它。

## 申请步骤

1. 登录 [developer.godaddy.com/personal-access-token](https://developer.godaddy.com/personal-access-token)，点“Generate Token”。
2. “Token Name”随便填，例如 BuckyOS。
3. “Permissions”里展开“Domains & DNS”，只选 `domains.domain:read` 和 `domains.dns:update`。
4. “Expires After”选 1 year（可选 7、30、60、90 天、1 年或不超过 1 年的日期）。
5. 点“Create Token”，在弹出的窗口里点“Copy Token”。它只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| Personal Access Token | 上面复制的 token |

## 权限建议

- 只选上面两个权限，不要选整组“Domains & DNS”，那会包含注册和转移域名。
- 没有看到限定单个域名的选项，token 对账户里的所有域名有效。

## 撤销和更换

token 最长 1 年，请在到期前换新的，否则证书续不了。

在 [Personal Access Token](https://developer.godaddy.com/personal-access-token) 页面点 token 右边的“⋯”>“Revoke”>“Revoke Token”。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [申请和撤销](https://developer.godaddy.com/docs/api-users/auth/how-to)
- [认证方式和权限列表](https://developer.godaddy.com/docs/api-users/auth)
- [一个域名即可使用 DNS API 的公告](https://www.godaddy.com/resources/news/godaddy-dns-api-now-works-with-a-single-domain)
