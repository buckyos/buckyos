# Name.com

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。IP 白名单页面的位置没有核对。

## 适用条件和费用

- 域名在 Name.com，并使用它的 NS。
- 官方文档没有提到 API 收费。

## 申请步骤

1. 打开“Account Settings”>“Security”>“API Tokens”，创建 token。测试和正式环境的 token 是分开的，要正式的（Production）。
2. 不要添加 IP 白名单。
3. 开了两步验证的，在“Security”页底部打开“name.com API Access”，否则接口返回 Permission Denied。
4. 复制 token。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| 用户名 | Name.com 的用户名，不一定是邮箱。不要加 `-test`，那是测试环境用的 |
| API Token | 上面复制的 token |

## 权限建议

没有看到限定域名或权限的选项，token 能操作整个账户。

## 撤销和更换

在“API Tokens”页面删除。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [Getting Started](https://docs.name.com/guides/getting-started)
- [测试环境](https://docs.name.com/guides/testing-environment)
- [开了两步验证时使用 API](https://www.name.com/support/articles/360007989433-using-api-with-two-step-authentication)
