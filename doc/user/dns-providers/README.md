# DNS API：各家服务商的 token 怎么申请

激活时使用自有域名，DNS 选“DNS API”，需要你在 DNS 服务商那里申请一个 token。节点用它写 DNS 记录、签发和续期证书。

这里每家一页。填到激活页的字段名与各家后台的叫法一致。

| 服务商 | 页面 | token 能否只限一个域名 | 会不会到期 |
| --- | --- | --- | --- |
| Cloudflare | [cloudflare.md](cloudflare.md) | 能 | 可以不设 |
| GoDaddy | [godaddy.md](godaddy.md) | 没看到 | 最长 1 年 |
| AWS Route 53 | [route53.md](route53.md) | 能，可限到记录名 | 不会 |
| Porkbun | [porkbun.md](porkbun.md) | 能 | 官方文档没写有效期 |
| IONOS | [ionos.md](ionos.md) | 没看到 | 官方文档没写有效期 |
| OVHcloud | [ovh.md](ovh.md) | 能 | 可以选不限 |
| Hetzner | [hetzner.md](hetzner.md) | 不能，整个项目 | 官方文档没写有效期 |
| Gandi | [gandi.md](gandi.md) | 能 | 最长 1 年 |
| DigitalOcean | [digitalocean.md](digitalocean.md) | 不能 | 创建时选 |
| Google Cloud DNS | [google_cloud.md](google_cloud.md) | 部分可以 | 默认不会 |
| Azure DNS | [azure.md](azure.md) | 能 | 最长 24 个月 |
| Name.com | [namecom.md](namecom.md) | 没看到 | 官方文档没写有效期 |
| Hostinger | [hostinger.md](hostinger.md) | 不能，等于你的用户 | 创建时选 |
| deSEC | [desec.md](desec.md) | 能，要用 API 设置 | 可以不设 |
| INWX | [inwx.md](inwx.md) | 没核对 | 用的是账户密码 |
| netcup | [netcup.md](netcup.md) | 没看到 | 官方文档没写有效期 |
| TransIP | [transip.md](transip.md) | 没看到 | 官方文档没写有效期 |
| 阿里云 | [alidns.md](alidns.md) | 能 | 不会 |
| 腾讯云 DNSPod | [dnspod.md](dnspod.md) | 能 | 不会 |
| さくらのクラウド | [sakura.md](sakura.md) | 没核对 | 官方文档没写有效期 |

## 共同的规则

- **节点会写哪些记录。** 以域名 `home.example.com` 为例，只有这四个名字：`home.example.com`、`*.home.example.com`、`_pkx.home.example.com`、`_acme-challenge.home.example.com`。类型是 A、AAAA、CNAME、TXT。别的记录不碰。已有的记录要改时，先列出来等你确认。
- **不要限制来源 IP。** 家用宽带的公网地址会变，限制之后 token 会突然失效。
- **不要设短有效期。** 节点要长期用这个 token 续证书。有期限的，选最长的，并在到期前换掉。
- **不要用账户的总密钥。** 能限定域名和权限的，按页面上的做法限定。标了“整个账户”或“整个项目”的，拿到 token 的人能改那里面的其它东西。
- **token 存在哪里。** 只存在节点本机 `$BUCKYOS_ROOT/security/dns_credentials/`，权限 0600，不写进系统配置、日志和激活文件。备份工具会备份它。
- **节点要能访问外网的 53 端口。** 检测时直接问你的域名的权威 NS，UDP 和 TCP 53 都要放行。
- **“实验”标记。** 激活页标了“实验”的服务商，代码写好了，还没有人用真实账号测过。

## 根域名

域名就是你在服务商那里管理的整个域名（例如 `example.com`）时：

- 接入方式是固定公网 IP（`wan`）：写 A / AAAA，可以用。
- 其它接入方式：根域名一般不能写 CNAME，请改用子域名，例如 `home.example.com`。DNS 在 Cloudflare 的例外。

## 换 token、撤销 token

1. 在服务商后台新建一个 token。
2. 打开 Desktop 的“设置”>“Zone 配置”，在“域名与证书”里点“更换 token”，填新的。需要 Zone 所有者登录并输入密码确认。
3. 确认检测通过后，再到服务商后台删除旧的。

token 失效或续期失败时，系统会通知所有者。

## 没有做的服务商

| 服务商 | 原因 | 怎么办 |
| --- | --- | --- |
| Namecheap | 要先满足域名数或消费额，还要登记 IPv4 白名单；它的接口会删掉没有一起提交的记录 | 用“SN 托管” |
| Squarespace（原 Google Domains） | 没有公开的 DNS API | 用“SN 托管” |
| お名前.com、ムームードメイン、Xserver ドメイン | 普通用户没有可用的 DNS API | 用“SN 托管” |

有自己证书的也可以用“手工管理”，见[商用证书怎么买、怎么上传](../自备证书.md)。
