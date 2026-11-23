# Google Cloud DNS

核对日期：2026-11-22，看了下面的官方文档，没有用真实账号操作。缩小权限的做法没有试过。

激活页按 NS 识别服务商，`ns-cloud-*.googledomains.com` 会识别成 Google Cloud DNS。原 Google Domains（现 Squarespace）的域名可能也用这样的 NS，识别错了请自己改选。

## 适用条件和费用

- 域名在 Cloud DNS 有公开的代管区域（managed zone）。
- Cloud DNS 按区域和查询收费，见[定价](https://cloud.google.com/dns/pricing)。
- 2024-05-03 及以后建立的组织默认不允许创建服务账号密钥（组织政策 `iam.disableServiceAccountKeyCreation`），要由组织管理员给这个项目放行。

## 申请步骤

1. 在 DNS 区域所在的项目里打开“IAM 和管理”>“服务账号”，创建一个服务账号。
2. 给它角色：
   - 简单的做法：在项目上给“DNS Administrator”（`roles/dns.admin`）。
   - 缩小范围：在项目上给“DNS Reader”（`roles/dns.reader`），再打开 Cloud DNS 的这个区域，在“资源权限”里添加这个服务账号为主账号，角色选 DNS Administrator。节点要在项目上列出区域（`dns.managedZones.list`），在区域上读记录（`dns.resourceRecordSets.list`）和提交更改（`dns.changes.create` 与记录的增改删），项目上的只读角色是给前一项用的。
3. 打开这个服务账号，点“密钥”>“添加密钥”>“创建新密钥”，选 JSON，点“创建”。文件会自动下载，之后不能再下载。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| 服务账号密钥（JSON） | 下载的文件的全部内容 |
| 项目 ID | 可以不填，用文件里的 `project_id` |

## 权限建议

给这件事单独建一个服务账号，不要用默认的服务账号。

## 撤销和更换

在服务账号的“密钥”页删除。每个服务账号最多 10 个密钥，可以先加新的再删旧的。密钥默认不过期，组织政策可能会限制。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [创建和删除服务账号密钥](https://docs.cloud.google.com/iam/docs/keys-create-delete)
- [创建服务账号](https://docs.cloud.google.com/iam/docs/service-accounts-create)
- [单个区域的 IAM](https://docs.cloud.google.com/dns/docs/zones/iam-per-resource-zones)
- [Cloud DNS 的权限](https://docs.cloud.google.com/dns/docs/access-control)
