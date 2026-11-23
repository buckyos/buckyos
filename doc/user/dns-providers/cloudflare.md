# Cloudflare

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。没有官方测试环境。“资源”一栏在页面上的确切名称没有核对。

## 适用条件和费用

- 域名的 DNS 在 Cloudflare（NS 是 `*.ns.cloudflare.com`）。
- 官方文档没有提到 API token 另收费。
- 根域名也能用在需要 CNAME 的接入方式上，因为 Cloudflare 会把根域名的 CNAME 展开。

## 申请步骤

1. 用[预填链接](https://dash.cloudflare.com/profile/api-tokens?permissionGroupKeys=%5B%7B%22key%22%3A%22dns%22%2C%22type%22%3A%22edit%22%7D%2C%7B%22key%22%3A%22zone%22%2C%22type%22%3A%22read%22%7D%5D&accountId=%2A&zoneId=all&name=BuckyOS%20DNS)打开，两个权限已经填好：Zone > DNS > Edit 和 Zone > Zone > Read。激活页上的链接就是它。
2. 资源从“全部”改成你要用的那一个域名。
3. “Token expiration”和“Client IP Address Filtering”都不填。
4. 点“Continue to summary”，再点“Create Token”。
5. 复制 token。它只显示这一次。

不用预填链接时：[My Profile > API Tokens](https://dash.cloudflare.com/profile/api-tokens)>“Create Token”，选“Edit zone DNS”模板，再自己加一条 Zone > Zone > Read。只有模板里的 DNS 权限不够：节点要按名字找到这个域名，官方接口 `GET /zones` 只接受 Zone Read。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| API Token | 上面复制的 token。不是 Global API Key |

## 权限建议

- 只给一个域名，只给 DNS Edit 和 Zone Read 两个权限。
- 在 DNS 页面把 BuckyOS 用的记录保持“仅 DNS”（灰色云）。开了代理（橙色云）时检测不会通过。

## 撤销和更换

在 [API Tokens](https://dash.cloudflare.com/profile/api-tokens) 列表里删除。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [创建 API token](https://developers.cloudflare.com/fundamentals/api/get-started/create-token/)
- [预填链接的格式](https://developers.cloudflare.com/fundamentals/api/how-to/account-owned-token-template/)
- [`GET /zones` 需要的权限](https://developers.cloudflare.com/api/resources/zones/methods/list/)
- [限制 token 的使用](https://developers.cloudflare.com/fundamentals/api/how-to/restrict-tokens/)
