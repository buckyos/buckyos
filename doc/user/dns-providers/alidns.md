# 阿里云云解析 DNS

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。策略按官方示例的写法编写，没有试过。

## 适用条件和费用

- 域名的 DNS 在阿里云云解析。
- 官方文档没有提到 API 收费。

## 申请步骤

1. 打开 [RAM 控制台](https://ram.console.aliyun.com/)>“权限管理”>“权限策略”>“创建权限策略”，用脚本编辑，粘贴下面的策略。把 `<账号ID>` 和 `example.com` 换成你的。
2. 打开“身份管理”>“用户”>“创建用户”，访问方式选“使用永久AccessKey访问”。
3. 保存 AccessKey ID 和 AccessKey Secret。Secret 只显示这一次。不要配置网络访问限制策略。
4. 给这个用户授权第 1 步的策略。

第 1、4 步的按钮名称没有逐个核对。

### 权限策略

```json
{
  "Version": "1",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["alidns:DescribeDomains"],
      "Resource": ["*"]
    },
    {
      "Effect": "Allow",
      "Action": [
        "alidns:DescribeDomainRecords",
        "alidns:AddDomainRecord",
        "alidns:DeleteDomainRecord"
      ],
      "Resource": ["acs:alidns:*:<账号ID>:domain/example.com"]
    }
  ]
}
```

- 这就是节点会调用的四个接口。`DescribeDomains` 不支持限定域名，只能写 `*`，它只能列出域名。
- 授权最细到域名，不能限定到某条记录。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| AccessKey ID | 第 3 步 |
| AccessKey Secret | 第 3 步 |

## 权限建议

- 不要用主账号的 AccessKey。
- 不想写策略时可以用系统策略 AliyunDNSFullAccess，它能改全部域名。
- 如果设置了 AccessKey 的最大闲置时间，不要短于证书的续期间隔。

## 撤销和更换

每个 RAM 用户最多 2 个 AccessKey，先建新的再停用旧的。删除前必须先禁用。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [云解析 DNS 自定义权限策略参考](https://help.aliyun.com/zh/dns/alibaba-cloud-dns-custom-permission-policy-reference-dns)
- [云解析 DNS 授权信息](https://help.aliyun.com/zh/dns/api-alidns-2015-01-09-ram)
- [创建 RAM 用户](https://help.aliyun.com/zh/ram/user-guide/create-a-ram-user)
- [创建 AccessKey](https://help.aliyun.com/zh/ram/user-guide/create-an-accesskey-pair)
