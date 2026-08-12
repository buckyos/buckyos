# Node Active：无 SN / BNS 命令行激活

## 动机

BuckyOS 除了面向需要低门槛安装和自动网络配置的普通用户，也需要支持另一类主流用户：拥有小型 VPS 的个人高级用户。这类用户通常具有以下特点：

- 拥有自己的域名和固定公网入口；
- 能够从源码编译、安装并自行维护 BuckyOS；
- 希望把 BuckyOS 当作个人网站或个人网络服务的运行环境；
- 不希望激活和运行依赖 BuckyOS 官方提供的 SN、BNS 或其它基础设施。

对这类用户而言，经公网 Node Active 服务完成激活既没有必要，也会引入额外的网络暴露。很多小型 VPS 的防火墙或云安全组只允许对外开放 80/443，不允许为了单次激活额外开放 3182。当前 3182 激活接口使用明文 HTTP，在公网传输激活请求也不具备足够的安全性。

因此提供 `src/active.ts`：直接在 VPS 的本地终端收集激活信息、生成身份和配置，并写入本地 Zone Boot 旁路文件。整个激活过程不监听或访问 3182，管理员密码、Owner 私钥等敏感信息也不会经过公网激活接口。用户只需自行管理域名解析、HTTPS 证书和服务器安全策略。

## 适用范围

`src/active.ts` 用于只有一台 OOD、具有固定公网入口和自有域名的 VPS 场景。它直接生成并保存本地身份，不启动或访问 3182，也不调用 SN、BNS 或其它激活控制面。

该模式的身份关系固定为：

```text
Owner DID = Zone DID = did:web:<domain>
Device DID = did:web:ood1.<domain>
net_id = wan
```

本流程不支持 `nat`、`portmap` 或 `wan_dyn`。这些拓扑缺少用户自建的等价中转/DDNS 能力时仍然需要 SN。

## 使用方法

从仓库根目录运行：

```bash
./src/active.ts
```

脚本会交互收集：

- `did:web` 域名；
- 本地管理员名称和密码；
- 固定公网 IP（可留空，仅用于生成 A/AAAA 配置提示）；
- RTCP 端口，默认 `2980`；
- Owner 私钥备份路径；
- 是否允许 guest access。

也可以预填非敏感参数：

```bash
./src/active.ts \
  --domain home.example.com \
  --owner-name alice \
  --public-ip 203.0.113.10 \
  --rtcp-port 2980
```

自动化调用应通过标准输入传密码，避免密码出现在命令行参数和进程列表：

```bash
printf '%s\n' "$ADMIN_PASSWORD" | ./src/active.ts \
  --domain home.example.com \
  --owner-name alice \
  --password-stdin \
  --yes
```

默认写入环境变量 `BUCKYOS_ROOT` 指向的目录；未设置时使用平台默认 BuckyOS 根目录。可用 `--root <dir>` 显式覆盖。

## 激活结果

脚本生成与 Node Active 同构的 Owner、ZoneBoot、Device、DeviceMini 和 Zone 文档，并写入运行时使用的相同位置：

- `$BUCKYOS_ROOT/etc/node_identity.json`
- `$BUCKYOS_ROOT/etc/start_config.json`
- `$BUCKYOS_ROOT/etc/zone_document.jwt`
- `$BUCKYOS_ROOT/etc/<domain>.zone.json`
- `$BUCKYOS_ROOT/local/identity/<device>/...`
- `$BUCKYOS_ROOT/security/<device>/...`

Owner 私钥不会写入 `start_config.json`，而是以 `0600` 权限单独保存到交互确认的备份路径。该文件是后续更新 Owner/Zone 文档所需的恢复材料，必须另行安全备份。

`<domain>.zone.json` 是本地旁路使用的最小 ZoneBootDocument JSON，只包含 `id`、`oods` 和 `exp`，不保存 Owner、Owner Key、`iat` 或 JWT。该文件与 `node_identity.json` 位于同一个本地信任边界，`node_daemon` 会从本地身份补齐运行所需的 Owner 信息。启动时它优先读取这个 debug override，命中后直接跳过 DID 网络发现和 DNS BOOT 查询。

脚本拒绝覆盖已经存在 `node_identity.json`、`start_config.json`、`zone_document.jwt` 或当前域名对应的 `<domain>.zone.json` 的系统，不提供强制重激活选项。

## DNS 与公网入口

激活完成后，脚本只输出并保存公网地址记录到 `$BUCKYOS_ROOT/etc/zone_dns_records.json`：

- 若交互时提供了固定公网 IP，保存域名的 `A` 或 `AAAA` 记录。

本机首次启动依赖本地 `<domain>.zone.json`，不要求配置 `BOOT`、`PKX`、`DEV` TXT 记录。用于个人网站时，只需为公网访问配置 A/AAAA，并让公网 80/443 到达该 OOD。RTCP 端口不是激活所需端口；只有需要对应的设备直连能力时才需要按部署策略放行。此模式没有 SN 代办证书，TLS/ACME 也必须由公网 gateway 自行配置。

完成激活后重启 BuckyOS，使 `node_daemon` 退出激活模式并按新的本地身份启动。
