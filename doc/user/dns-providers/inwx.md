# INWX

**未核对**：子账户的创建位置和 DNS 权限的名称。官方帮助页只核对了 Mobile-TAN 对 API 的影响。核对日期：2026-11-22，没有用真实账号操作。激活页的链接只到 INWX 首页。

## 适用条件和费用

- 域名使用 INWX 的 NS。
- 费用没有核对。有免费的测试系统 ote.inwx.com。

## 申请步骤

INWX 的接口用账户的用户名和密码，没有单独的 token。

1. 建议建一个只能管理 DNS 的子账户，不要开 Mobile-TAN，专门给 BuckyOS 用。
2. 用主账户时：开了 Mobile-TAN 的话，接口也需要动态验证码，要填 Shared secret。它只在开启 Mobile-TAN 时显示一次，没有记下来的，可以在“Settings”>“Access details”关掉 Mobile-TAN 再开一次（没有核对）。
3. 同一个动态验证码只能用一次，两次登录要间隔 30 秒，所以更推荐不开 Mobile-TAN 的子账户。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| 用户名 | 子账户的用户名 |
| 密码 | 子账户的密码 |
| Shared secret（Mobile-TAN） | 开了 Mobile-TAN 才填 |

## 权限建议

主账户的密码能做所有事情，包括买卖域名，不要填主账户的。

## 撤销和更换

改子账户的密码，或删除子账户。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [API 与测试系统](https://www.inwx.com/en/offer/api)
- [Mobile-TAN](https://kb.inwx.com/en-us/5-customer-details/70-what-is-the-mobile-tan-service-and-how-can-i-activate-it)
- 子账户和动态验证码的限制来自第三方：[DNSControl 的 INWX 说明](https://github.com/StackExchange/dnscontrol/blob/main/documentation/provider/inwx.md)
