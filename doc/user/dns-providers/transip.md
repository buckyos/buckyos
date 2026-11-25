# TransIP

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。删除 Key Pair 的位置没有核对。

## 适用条件和费用

- 域名使用 TransIP 的 NS。
- 官方文档没有提到 API 收费。

## 申请步骤

1. 登录[控制面板](https://www.transip.eu/cp/account/api/)，点右上角头像>“My account”>“API”，把“Status”切到“On”。
2. 在“Key Pairs”里填标签，去掉“Whitelisted IP”的勾选，添加。
3. 复制显示的 Private Key，从 `-----BEGIN PRIVATE KEY-----` 到 `-----END PRIVATE KEY-----`。它只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| 账户名 | TransIP 的登录账户名 |
| 私钥 | 第 3 步的整段文字 |

## 权限建议

Key Pair 没有看到限定域名或只读的选项，它能操作整个账户，包括下单。请保管好。

## 撤销和更换

在同一页面删除 Key Pair。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [使用 TransIP REST API](https://www.transip.eu/knowledgebase/77-using-the-transip-rest-api)
- [REST API 文档](https://api.transip.nl/rest/docs.html)
