# OVHcloud

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。三个入口都要登录，只确认了页面存在。激活页只能放一个链接，放的是欧洲的。

## 适用条件和费用

- 域名的 DNS 区（DNS zone）在 OVHcloud。
- 官方文档没有提到 API 收费。
- 分三个入口，账户在哪个区就用哪个，互相不通用。

## 申请步骤

1. 打开你所在区的页面并登录：
   - 欧洲：<https://auth.eu.ovhcloud.com/api/createToken>
   - 加拿大和亚太：<https://auth.ca.ovhcloud.com/api/createToken>
   - 美国：<https://auth.us.ovhcloud.com/api/createToken>
2. 填名称（Script name），“Validity”选“Unlimited”，不要限制 IP。
3. 在“Rights”里添加下面几条，把 `example.com` 换成你的域名（DNS 区的名字，不是子域名）：

   | 方法 | 路径 |
   | --- | --- |
   | GET | `/domain/zone/example.com/*` |
   | POST | `/domain/zone/example.com/*` |
   | PUT | `/domain/zone/example.com/*` |
   | DELETE | `/domain/zone/example.com/*` |

4. 点“Create keys”，复制三个值。

节点只调用这个路径下面的记录接口和 `refresh`，不需要 `/domain/zone/example.com` 本身。规则没有用真实账号试过。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| Region | 第 1 步用的入口 |
| Application key | AK |
| Application secret | AS |
| Consumer key | CK |

## 权限建议

不要用 `/*`，那等于整个账户。

## 撤销和更换

在 OVHcloud 控制台“Identity and Access Management”>“API keys”里打开这个 key，点“Delete API key”。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [First Steps with the OVHcloud APIs](https://docs.ovhcloud.com/en/guides/manage-and-operate/api/first-steps)（2026-11-18 更新）
