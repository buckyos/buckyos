# AWS Route 53

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。策略按官方示例的写法编写，没有在 AWS 上试过。

## 适用条件和费用

- 域名在 Route 53 有公有托管区（Public hosted zone）。
- 托管区和查询按 AWS 的价格收费，见[定价](https://aws.amazon.com/route53/pricing/)。
- 策略里的 ARN 以 `arn:aws:` 开头，只适用于 AWS 全球区，不适用于中国区和 GovCloud。

## 申请步骤

1. 打开 Route 53 的“Hosted zones”，点你的域名，记下 Hosted zone ID（以 `Z` 开头）。
2. 打开 [IAM](https://console.aws.amazon.com/iam/)，点左边“Policies”>“Create policy”，选“JSON”，粘贴下面的策略。把 `Z0123456789EXAMPLE` 和 `home.example.com` 换成你的，点“Next”，起个名字，点“Create policy”。
3. 点“Users”>“Create user”，不要开控制台访问，权限选直接附加策略，勾选刚建的策略。
4. 打开这个用户，点“Security credentials”>“Create access key”，用途选“Other”，点“Next”>“Create access key”。
5. 复制 Access key ID 和 Secret access key。后者只显示这一次。

第 3 步的按钮名称没有逐个核对。

### IAM 策略

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "FindHostedZone",
      "Effect": "Allow",
      "Action": "route53:ListHostedZonesByName",
      "Resource": "*"
    },
    {
      "Sid": "ReadRecords",
      "Effect": "Allow",
      "Action": "route53:ListResourceRecordSets",
      "Resource": "arn:aws:route53:::hostedzone/Z0123456789EXAMPLE"
    },
    {
      "Sid": "WriteBuckyOSRecords",
      "Effect": "Allow",
      "Action": "route53:ChangeResourceRecordSets",
      "Resource": "arn:aws:route53:::hostedzone/Z0123456789EXAMPLE",
      "Condition": {
        "ForAllValues:StringEquals": {
          "route53:ChangeResourceRecordSetsNormalizedRecordNames": [
            "home.example.com",
            "\\052.home.example.com",
            "_pkx.home.example.com",
            "_acme-challenge.home.example.com"
          ],
          "route53:ChangeResourceRecordSetsRecordTypes": ["A", "AAAA", "CNAME", "TXT"]
        }
      }
    }
  ]
}
```

- `\\052` 是通配符 `*` 在这个条件里的写法，不能直接写 `*`。名字全部小写，末尾没有点。
- 节点只调用 `ListHostedZonesByName`、`ListResourceRecordSets`、`ChangeResourceRecordSets`。激活页填了 Hosted zone ID 时，`FindHostedZone` 这一段可以删掉。
- 以后换域名，这条策略要一起改。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| Access key ID | 第 5 步的 Access key ID |
| Secret access key | 第 5 步的 Secret access key |
| Hosted zone ID | 第 1 步记下的值，可以不填 |

## 权限建议

- 不要用 root 用户的 access key。
- 用上面的策略，不要用 AmazonRoute53FullAccess。
- 不要加 `aws:SourceIp` 条件。

## 撤销和更换

每个用户最多两个 access key，可以先建新的再停旧的。在用户的“Security credentials”里对旧的点“Actions”>“Deactivate”，确认没问题后“Delete”。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [按条件限制记录名和类型](https://docs.aws.amazon.com/Route53/latest/DeveloperGuide/specifying-conditions-route53.html)
- [Route 53 的 IAM 策略示例](https://docs.aws.amazon.com/Route53/latest/DeveloperGuide/access-control-managing-permissions.html)
- [用 JSON 创建策略](https://docs.aws.amazon.com/IAM/latest/UserGuide/access_policies_create-console.html)
- [创建和停用 access key](https://docs.aws.amazon.com/IAM/latest/UserGuide/access-key-self-managed.html)
