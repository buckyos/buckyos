# DigitalOcean

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。“Expiration”里有哪些选项，官方文档没有列出。

## 适用条件和费用

- 域名的 DNS 在 DigitalOcean（Networking > Domains）。
- 费用没有核对。

## 申请步骤

1. 打开控制面板的“Account”>“API”，在“Tokens”页点“Generate New Token”。
2. 填名称，“Expiration”选最长的。
3. “Scopes”选“Custom Scopes”，找到 domain，勾选 `domain:create`、`domain:read`、`domain:update`、`domain:delete`。
4. 点“Generate Token”，复制 token。它只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| Personal access token | 上面复制的 token |

## 权限建议

- 只给 domain 的四个权限，不要选“Full Access”。
- 权限只能到“域名”这一类，账户里的全部域名都能改。

## 撤销和更换

选了期限的，到期前换新的。到期后 token 会从账户里消失。

在“Tokens”页点“…”>“Delete”。权限建好后不能改，要改就重新建。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [Create a Personal Access Token](https://docs.digitalocean.com/reference/api/create-personal-access-token/)
- [Scopes](https://docs.digitalocean.com/reference/api/scopes/)
