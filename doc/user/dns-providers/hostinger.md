# Hostinger

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。有效期有哪些选项，官方文档没有列出。

## 适用条件和费用

- 域名的 DNS 在 Hostinger。
- 费用没有核对。

## 申请步骤

1. 打开 [hpanel.hostinger.com/profile/api](https://hpanel.hostinger.com/profile/api)，点“New”。
2. 填名称，有效期选最长的，点“Generate”。
3. 复制 token。刷新页面后不再显示。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| API token | 上面复制的 token |

## 权限建议

token 的权限与创建它的用户相同，不能只限定 DNS 或一个域名。

## 撤销和更换

有期限的请在到期前换新的。在同一页面删除 token。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [API 概述](https://docs.hostinger.com/api-reference/overview)
- [What is Hostinger API](https://www.hostinger.com/support/10840865-what-is-hostinger-api/)
