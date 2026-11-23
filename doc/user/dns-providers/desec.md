# deSEC

核对日期：2026-11-22，看了官方文档和网页端的公开源码，没有用真实账号操作。

## 适用条件和费用

- 域名已加到 [desec.io](https://desec.io)，NS 是 `ns1.desec.io` 和 `ns2.desec.org`。
- 可以免费注册。
- deSEC 会给域名做 DNSSEC 签名。要启用 DNSSEC，把它给的 DS 记录填到注册商那里。

## 申请步骤

1. 登录 desec.io，打开“Token Management”。
2. 新建 token（Generate New Token），填名称，“Can create domains”和“Can delete domains”保持关闭。
3. 高级设置里的“Can manage tokens”保持关闭，“Maximum age”“Maximum unused period”和客户端子网都留空。
4. 提交后复制 token。它只显示这一次。

## 填到激活页

| 字段 | 填什么 |
| --- | --- |
| Token | 上面复制的 token |

## 权限建议

网页上新建的 token 能改账户里所有域名的记录。要限定一个域名，用 API 给它加策略：先加一条默认拒绝，再加一条允许这个域名。

```bash
# 第一条：默认不许写
curl -X POST https://desec.io/api/v1/auth/tokens/<token id>/policies/rrsets/ \
  -H "Authorization: Token <有管理权限的 token>" -H "Content-Type: application/json" \
  -d '{"domain": null, "subname": null, "type": null}'
# 第二条：允许写 example.com
curl -X POST https://desec.io/api/v1/auth/tokens/<token id>/policies/rrsets/ \
  -H "Authorization: Token <有管理权限的 token>" -H "Content-Type: application/json" \
  -d '{"domain": "example.com", "subname": null, "type": null, "perm_write": true}'
```

加了策略的 token 仍然能读账户里的所有记录。这两条命令按官方文档的字段写成，没有试过。

## 撤销和更换

在“Token Management”里删除。更换的做法见[总说明](README.md#换-token撤销-token)。

## 官方链接

- [Token 与策略](https://desec.readthedocs.io/en/latest/auth/tokens.html)
- [域名与最小 TTL](https://desec.readthedocs.io/en/latest/dns/domains.html)
