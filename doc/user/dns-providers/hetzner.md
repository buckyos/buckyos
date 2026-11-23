# Hetzner

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。激活页的链接只到 Hetzner Console 首页。

## 适用条件和费用

- 域名的 DNS 区在 Hetzner Console 的某个项目里。
- 旧的 DNS Console（dns.hetzner.com）和它的 API 在 2026-05 关闭，旧 token 不能用。
- 费用没有核对。

## 申请步骤

1. 打开 [console.hetzner.com](https://console.hetzner.com/)，进入放这个 DNS 区的项目。
2. 点左边“Security”，再点上面“API tokens”，点“Generate API token”。
3. 填说明，权限选“Read & Write”，生成后复制 token。关掉窗口后不能再看。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| API token | 上面复制的 token |

## 权限建议

token 能操作整个项目，包括服务器。把 DNS 区放到单独的项目里可以缩小范围。

## 撤销和更换

在同一个“API tokens”页面删除。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [Generating an API token](https://docs.hetzner.com/cloud/api/getting-started/generating-api-token/)
- [迁移到 Hetzner Console 的变化](https://docs.hetzner.com/networking/dns/migration-to-hetzner-console/features-and-differences/)
