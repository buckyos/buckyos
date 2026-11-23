# さくらのクラウド DNS

核对日期：2026-11-22，看了下面的官方手册，没有用真实账号操作。

## 适用条件和费用

- 域名已加到さくらのクラウド的“DNS”，并按它给的 NS（`ns1.gslbN.sakura.ne.jp`、`ns2.gslbN.sakura.ne.jp`，N 是数字）委派。一个月内没有委派的，登记可能被删除。
- 每个 DNS 区每月 44 日元（含税），查询不收费。
- 这是“さくらのクラウド”的 DNS，不是さくらのドメイン附带的 DNS。

## 申请步骤

1. 登录控制面板，点左边的“APIキー”>“APIキーの作成”。
2. 种类选“リソース操作”，填名称，“アクセスレベル”选“設定編集”，“サービスへのアクセス権”不用勾。
3. 点“作成”，记下アクセストークン和アクセストークンシークレット。后者只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| アクセストークン | 第 3 步 |
| アクセストークンシークレット | 第 3 步 |

## 权限建议

- “設定編集”是改 DNS 记录需要的最低级别，不能做会改变费用的操作。
- key 对整个项目有效，没有看到限定某个 DNS 区的选项。
- 手册说控制面板的权限已统一到 IAM 策略，APIキー 仍按アクセスレベル设置。

## 撤销和更换

在“APIキー”列表里打开它，点“APIキーの削除”。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [APIキー](https://manual.sakura.ad.jp/cloud/api/apikey.html)（2025-08-26 更新）
- [アクセスレベル](https://manual.sakura.ad.jp/cloud/controlpanel/access-level.html)
- [DNS](https://manual.sakura.ad.jp/cloud/appliance/dns/index.html)
- [料金](https://cloud.sakura.ad.jp/products/dns/)
