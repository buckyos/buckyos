# IONOS

核对日期：2026-11-22，看了开发者门户页面里的英文和各语言文字，没有用真实账号操作。key 能否只限一个域名没有核对到，激活页按账户级提示。

## 适用条件和费用

- 域名在 IONOS 的普通主机 / 域名产品里，DNS 由 IONOS 管理。IONOS Cloud（DCD）的 Cloud DNS 是另一套，不适用。
- 第一次要先开通 API 功能，有单独的条款。费用没有核对。
- 门户按国家分：`.com`、`.ca`、`.co.uk`、`.de`、`.es`、`.fr`、`.it`、`.mx`。用你账户所在国家的站点。

## 申请步骤

1. 在 [my.ionos.com/shop/product/ionos-api](https://my.ionos.com/shop/product/ionos-api) 开通 API（换成你所在国家的域名）。
2. 打开 [developer.hosting.ionos.com/keys](https://developer.hosting.ionos.com/keys)，点“Create New Key”，填一个名称，点“Continue”。
3. 复制 Public Prefix 和 Secret。Secret 只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| API Key | `Public Prefix.Secret`，两段用英文句点连起来 |

## 权限建议

创建时没有看到限定权限或域名的选项，按能操作账户里的全部域名对待，请保管好。

## 撤销和更换

在 [API Keys](https://developer.hosting.ionos.com/keys) 页面点“Revoke”。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [Getting Started](https://developer.hosting.ionos.com/docs/getstarted)
- [DNS API](https://developer.hosting.ionos.com/docs/dns)
- [IONOS Developer APIs](https://www.ionos.com/help/hosting/ionos-apis/ionos-developer-apis/)
