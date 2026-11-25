# netcup

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。激活页的链接只到 Customer Control Panel 首页。

## 适用条件和费用

- 域名使用 netcup 的 NS。
- netcup 有两套 DNS，要填的字段不同。在 [Customer Control Panel（CCP）](https://www.customercontrolpanel.de/) 的“Domains”里点域名旁的放大镜，看标签页：

  | 标签页 | 用哪套接口 | 填什么 |
  | --- | --- | --- |
  | CloudDNS | netcup API | 只填 API Key |
  | DNS | Legacy API | 客户编号、Legacy API Key、Legacy API Password |

- 激活页按 NS 能自动识别旧 DNS，CloudDNS 识别不到，请自己选 netcup。
- 费用没有核对。

## 申请步骤

### CloudDNS

1. 登录 CCP，打开“Master Data”>“API”。
2. 在“API Keys”里点“Creating an API Key”，填说明（最多 50 个字符），勾选条款，点“Confirm”。
3. 复制“API Keys”里的 key。

### DNS（Legacy API）

1. 登录 CCP，打开“Master Data”>“API”。
2. 在“Legacy API Keys”里点“Creating an API Key”，勾选条款，点“Confirm”。
3. 点“Regenerating API Password”，勾选条款，点“Confirm”，复制密码。它只显示这一次，不能自己设。
4. 记下客户编号（Customer number）。

## 填到激活页

| 字段 | CloudDNS | DNS（Legacy API） |
| --- | --- | --- |
| 客户编号 | 不填 | 你的 netcup 客户编号 |
| API Key | 第 3 步的 key | 第 2 步的 Legacy API Key |
| Legacy API Password | 不填 | 第 3 步的密码 |

域名以后迁到 CloudDNS 的，要换成 CloudDNS 的 API Key。

## 权限建议

- 没有看到限定域名的选项，按能操作账户里的全部域名对待。
- 每个账户只有一个 Legacy API Password。重新生成会让其它程序的密码失效。

## 撤销和更换

两种 key 都可以逐个删除。Legacy API Password 只能重新生成。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [netcup API（CloudDNS）](https://www.netcup.com/en/helpcenter/documentation/domain/netcup-api)（2026-11-05 更新）
- [Legacy API](https://www.netcup.com/en/helpcenter/documentation/domain/our-api)
