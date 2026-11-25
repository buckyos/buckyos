# Porkbun

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。

## 适用条件和费用

- 域名在 Porkbun，并使用 Porkbun 的 NS。
- 官方文档没有提到 API 收费。
- 要对每个域名单独打开 API 访问，只有 key 不够。

## 申请步骤

1. 登录后点右上角“ACCOUNT”>“API Access”。
2. 填标题，“Permissions”选“Manage, but no purchases”，域名限定为你的域名，IP 地址留空。
3. 点“Create API Key”，复制 API Key 和 Secret Key。Secret Key 只显示这一次。
4. 点“ACCOUNT”>“Domain Management”，在域名右边点“Details”，打开“API Access”。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| API Key | 第 3 步的 API Key。以 `pk1_sb_` 开头的是测试用的，不要用 |
| Secret Key | 第 3 步的 Secret Key |

## 权限建议

- 权限选“Manage, but no purchases”，并限定域名。
- 权限以后可以在 key 旁边的齿轮里改。

## 撤销和更换

在“API Access”页面删除 key。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [Getting started with the Porkbun API](https://kb.porkbun.com/article/190-getting-started-with-the-porkbun-api)（2026-10-08 更新）
- [API 文档](https://porkbun.com/api/json/v3/documentation)
