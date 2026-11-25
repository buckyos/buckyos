# 腾讯云 DNSPod

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。新建子用户的页面没有逐步核对。**未核对**：只授权一个域名的子用户能否列出域名。节点会调用 `DescribeDomainList`、`DescribeRecordList`、`CreateRecord`、`DeleteRecord`；第一个被拒时先改用 QcloudDNSPodFullAccess。

## 适用条件和费用

- 域名的 DNS 在腾讯云 DNSPod（云解析）。用的是腾讯云 API 密钥，不是旧的 DNSPod Token。
- 官方文档没有提到 API 收费。

## 申请步骤

1. 打开[访问管理](https://console.cloud.tencent.com/cam)>“用户列表”，新建一个子用户，访问方式选编程访问，先不给权限。
2. 打开 [DNSPod 控制台](https://console.dnspod.cn/)，点这个域名>“权限管理”>“CAM 权限管理”>“添加子账号”，选刚才的子用户，权限选“完全读写”。
3. 回到“用户列表”，点这个子用户>“API 密钥”>“新建密钥”。
4. 保存 SecretId 和 SecretKey。SecretKey 只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| SecretId | 第 4 步 |
| SecretKey | 第 4 步 |

## 权限建议

- 不要用主账号的密钥。
- 系统策略 QcloudDNSPodFullAccess 覆盖账户里的全部域名，尽量按第 2 步只给一个域名。

## 撤销和更换

每个子用户最多 2 个密钥，先建新的再停用旧的。删除前要先禁用，删除后不能恢复。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [子账号访问密钥管理](https://cloud.tencent.com/document/product/598/37140)
- [给子用户授权一个域名](https://www.tencentcloud.com/document/product/1295/76989)
- [DNSPod 的 CAM 策略](https://www.tencentcloud.com/document/product/1295/76991)
