# Gandi

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。权限名称“Manage domain name technical configurations”来自 DNSControl 的代码说明，官方文档没有列出，**未核对**。

## 适用条件和费用

- 域名使用 Gandi 的 LiveDNS。
- 官方文档没有提到 API 收费。
- 旧的 API Key 已弃用，用 Personal Access Token（PAT）。

## 申请步骤

1. 登录后点右上角用户名>“Settings”，找到“Personal Access tokens (PAT)”，点“View my personal access tokens”，再点“Create a token”。也可以直接打开 [admin.gandi.net/organizations/account/pat](https://admin.gandi.net/organizations/account/pat)。
2. 选组织，填名称（不要用重音和特殊字符）。
3. “Expires in”选 1 year（可选 7、30、60、90 天或 1 年）。
4. 资源选“Restrict to selected products”，选中你的域名。
5. 权限勾选“Manage domain name technical configurations”。
6. 点“Create”，点“Copy”。它只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| Personal Access Token | 上面复制的 token |

## 权限建议

只选你的域名，只给上面的权限。

## 撤销和更换

token 最长 1 年，请在到期前换新的，否则证书续不了。

在“View my personal access tokens”列表里点垃圾桶图标，立即失效。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [Personal Access Token](https://docs.gandi.net/en/managing_an_organization/organizations/personal_access_token.html)
- [API 认证](https://api.gandi.net/docs/authentication)
