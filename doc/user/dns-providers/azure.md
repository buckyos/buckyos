# Azure DNS

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。激活页的链接只到 Entra 管理中心首页。

## 适用条件和费用

- 域名在 Azure DNS 有公有区域。
- 只支持 Azure 全球云，不支持中国区和美国政府区。
- 按区域和查询收费，见[定价](https://azure.microsoft.com/pricing/details/dns/)。
- 你要能注册应用、能在 DNS 区域上分配角色。

## 申请步骤

1. 打开 [Entra 管理中心](https://entra.microsoft.com)>“Entra ID”>“应用注册”（App registrations），点“新注册”（New registration），账户类型选仅此组织目录，注册。
2. 在应用的概述页复制“目录(租户) ID”和“应用程序(客户端) ID”。
3. 打开“证书和密码”（Certificates & secrets）>“客户端密码”>“新客户端密码”（New client secret），期限选最长，最多 24 个月。
4. 复制“值”（Value）。离开页面后不能再看。不要复制“机密 ID”。
5. 到 [Azure 门户](https://portal.azure.com) 打开你的 DNS 区域>“访问控制 (IAM)”>“添加”>“添加角色分配”，角色选“DNS Zone Contributor”，成员搜索刚才的应用名。
6. 在 DNS 区域的概述页复制订阅 ID 和资源组。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| 目录(租户) ID | 第 2 步 |
| 应用程序(客户端) ID | 第 2 步 |
| 客户端密码的值 | 第 4 步 |
| 订阅 ID | 第 6 步 |
| 资源组 | 第 6 步 |

## 权限建议

角色分配在单个 DNS 区域上，不要分配在订阅上。

## 撤销和更换

客户端密码最长 24 个月，请在到期前换新的，否则证书续不了。一个应用可以同时有多个密码，先加新的再删旧的。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [保护 DNS 区域和记录（角色分配）](https://learn.microsoft.com/zh-cn/azure/dns/dns-protect-zones-recordsets)
- [注册应用和服务主体](https://learn.microsoft.com/zh-cn/entra/identity-platform/howto-create-service-principal-portal)
- [添加客户端密码](https://learn.microsoft.com/zh-cn/entra/identity-platform/how-to-add-credentials)
