# Node Active：无 SN / BNS 命令行激活

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
- `$BUCKYOS_ROOT/local/identity/<device>/...`
- `$BUCKYOS_ROOT/security/<device>/...`

Owner 私钥不会写入 `start_config.json`，而是以 `0600` 权限单独保存到交互确认的备份路径。该文件是后续更新 Owner/Zone 文档所需的恢复材料，必须另行安全备份。

脚本拒绝覆盖已经存在 `node_identity.json`、`start_config.json` 或 `zone_document.jwt` 的系统，不提供强制重激活选项。

## DNS 与公网入口

激活完成后，脚本输出并保存以下 DNS 配置到 `$BUCKYOS_ROOT/etc/zone_txt_record.json`：

- 若交互时提供了固定公网 IP，保存域名的 `A` 或 `AAAA` 记录；
- `BOOT=<ZoneBootDocument JWT>;`
- `PKX=<Owner Ed25519 public key x>;`
- `DEV=<DeviceMiniDocument JWT>;`

用户需要自行把 A/AAAA 与三条 TXT 记录配置到域名服务商，并保证公网的 80/443 与 RTCP 端口能到达该 OOD。此模式没有 SN 代办证书，TLS/ACME 也必须由公网 gateway 自行配置。

完成激活后重启 BuckyOS，使 `node_daemon` 退出激活模式并按新的本地身份启动。
