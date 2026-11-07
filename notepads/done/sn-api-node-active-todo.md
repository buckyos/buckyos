# SN API / node_active TODO

背景：SN API 已切到新版路径职责，`/kapi/sn` 下只保留 `auth` 和 `deviceinfo`，BNS 文档写入迁到 `/kapi/bns`。当前 BuckyOS 侧已先做最小适配，目标是让编译和基础构建通过。

## 已确认的 Node Active 密钥约束

- 上游 NameLib 已支持通过同一个助记词构造两套密钥：
  - EVM 密钥；
  - 过去 BuckyOS 体系使用的 Owner 密钥。
- Node Active 需要支持两条激活路线：
  - 有钱包路线是主路径：钱包内已经有 EVM 密钥和 Owner 密钥，`node_active` 不生成、不保存这两套密钥，只在需要时要求钱包用对应密钥签名。
  - 网页激活路线是兼容性路径：用于用户暂时无法安装或使用钱包的特殊场景，例如当前只有纯 Web 环境，没有安卓手机或桌面钱包。
- Owner 密钥原则上必须保存在钱包里；BuckyOS 体系内默认不应有长期需要 Owner 密钥的地方。
- 按设计，除了激活瞬间需要签名 DID Document，后续正常运行不应再出现需要 BuckyOS 本地签名 DID Document 的场景。
- 网页激活路线的临时密钥流程：
  - 先生成助记词；
  - 通过 NameLib 从同一个助记词派生 EVM 密钥和 Owner 密钥；
  - 在激活阶段使用这些密钥完成必要签名和 BNS 写入；
  - 不把这些密钥设计成 BuckyOS 的长期托管密钥。
- 网页激活路线的保存策略：
  - 助记词是恢复两套密钥的根材料，但只能作为网页激活兼容路径中的临时秘密，不应成为 BuckyOS 的长期托管数据。
  - EVM 密钥和 Owner 密钥可以在激活期写入临时配置或激活上下文，作为由助记词派生出的激活期材料。
  - 不能再独立生成两套互不相关的密钥。
  - 后续如果确实需要 Owner Key 签名，应通过钱包发起，而不是依赖 BuckyOS 本地保存 Owner 私钥。
- 将网页激活产生的密钥导回钱包是独立产品流程，不在本文档定义。

## 已确认的 BNS 访问约束

- 上游登录接口返回的 Token / Access Token 是给网页使用的，不是给 BuckyOS 系统服务使用的。
- 原则上，`node_active`、`node_daemon` 或其它 BuckyOS 组件如果依赖上游登录 Access Token 访问上游接口，都应视为错误设计，需要清理。
- BNS 读操作是公共读操作，应支持匿名访问，不应要求 Access Token。
- BNS 写操作不通过 Access Token 授权，语义上需要使用 EVM 密钥构造合法的上链 TX。
- 具体实现不一定手工构造 TX：如果底层 NameLib / BNS 库已经封装了 TX 构造、签名、提交流程，应优先使用库提供的写入接口。
- 因此不需要设计 `auth.register` access token 到 BNS scoped token 的转换流程。

## 已确认的文档发布与权限约束

- 原理上，现在所有文档发布都走 BNS 写路径。
- zone-config 当前集成度已经较高，可以整合 boot 信息和 device mini doc；Node Active 应优先发布 zone-config，而不是继续拆成过去多个独立文档发布步骤。
- DID document 等相关文档写入也应纳入 BNS 写路径；具体写入接口和 payload 以当前 NameLib / BNS / DID 库为准，编码时通过阅读相关库确认。
- 域名管理需要分流：只有 BNS 域名可以使用 BNS 合约管理；`user_domain` 仍然走传统路径，由 SN 协助添加或更新 DNS 记录。
- BNS 写权限不来自网页登录 Token，而来自写操作对应的链上权限或已注册身份。
- SN Device Info 上报原则上只需要使用当前设备的私钥签名或证明。
- 注册阶段如果已经正确地把设备注册到 SN，注册信息会携带设备公钥；后续使用该设备私钥即可证明设备身份，并具备对应写权限。

## 已完成的临时适配

- Rust 侧不再调用旧 `sn_bind_zone_config` / `sn_register_device` / `sn_update_device_info`。
- `node_daemon` 激活和运行期上报只向 `/kapi/sn/deviceinfo` 上报设备在线态。
- `node_active` 里旧 `/kapi/sn/bns` 路径已改成 `/kapi/bns`，并跳过已移除的 `user.bind_owner_key`。

## 2026-06-28 本次落地

- `node_active` 不再保存 `auth.register` / `auth.login` 返回的 access token，也不再把它传给 active RPC。
- 删除 `node_active` 中临时 no-op 的 `bind_owner_key()` 和旧 `bind_sn_zone_config()` 调用链。
- `node_active` 网页激活不再用 access token 调 `/kapi/bns zone.bind_config`；`node_daemon` 已接入 `bns-client` / `bns-indexer`，在 `bns_url`、`bns_evm` 和 `bns_evm_private_key` 齐备时通过 EVM TX 发布 BNS 文档。
- `active_server` 的激活期 SN deviceinfo 上报改为 `sn_device_proof` 字段，由当前设备私钥签出短期 JWT；钱包激活只要求钱包签 Owner DID/boot/device doc，不再要求钱包签 SN RPC token。
- `generate_key_pair` 只返回 key pair，不再顺手生成本地 access token。
- `WAN + 自有域名` 无 SN 路线修正为空 SN URL 时不触发 SN 上报。
- 文档同步更新 `doc/arch/gateway/SN.md`，明确 `/kapi/sn/auth`、`/kapi/sn/deviceinfo`、`/kapi/bns` 的职责边界。

## 2026-06-28 继续落地：BNS EVM 依赖接入

- `src/Cargo.toml` 新增 `bns-client`、`bns-indexer` workspace 依赖，`node_daemon` 使用它们构造 BNS EVM 写入请求。
- `active_server` 新增 BNS 发布链路：BNS name 不存在时尝试 `name.register` 并带初始 `zone` / `boot` / `device_mini_doc` 文档；已存在时用 `mutation.apply` 更新这些文档。
- BNS 写入使用 `/kapi/bns` 匿名读查询当前 name/document 状态，使用 EVM 私钥签名并提交 raw TX，不依赖网页登录 access token。
- `node_active` 支持从 `active_config.json` 透传 `bns_evm`，并支持激活期传入 `bns_evm_private_key`；这些私钥不会写入 `start_config.json`。
- 当前仓库内钱包 bridge 只有 `walletSignWithActiveDid`，还没有 EVM raw TX 签名接口；钱包模式的 EVM 签名仍需钱包运行时补充 bridge 后接入。

## 实现意图与后续处理

- 排查并移除 BuckyOS 侧对上游登录 Access Token 的依赖，特别是 `node_active` 中访问 `/kapi/bns` 或 SN API 的路径。
- 落地 BNS 匿名读流程，读 BNS 文档时不携带 Access Token。
- 落地 BNS 写入的 EVM TX 流程：
  - 网页激活路线使用 NameLib 从助记词派生出的 EVM 密钥完成 BNS 写入；
  - 钱包激活路线要求钱包用 EVM 密钥完成对应签名或 TX 构造；
  - 具体是暴露原始 TX 还是封装写接口，以 NameLib / BNS 库为准，编码时优先复用底层库封装；
  - `node_active` 不把 Access Token 当成写权限来源。
- 落地网页激活的 NameLib 助记词派生流程：
  - 先生成助记词；
  - 用同一个助记词派生 EVM 密钥和 Owner 密钥；
  - 激活阶段使用派生出的 EVM 密钥完成 BNS 写入；
  - 激活阶段使用派生出的 Owner 密钥完成 DID Document 签名；
  - 不把助记词、EVM 私钥、Owner 私钥作为 BuckyOS 的长期运行依赖。
- 落地钱包激活的签名流程：
  - BNS 写入需要 EVM 密钥完成对应签名或 TX 构造；
  - 激活阶段的 DID Document 签名需要 Owner 密钥，具体签名内容、签名时机和验证点以现有 DID Document 库为准；
  - 前端/钱包侧需要暴露对应签名能力，`node_active` 不保存钱包内的 EVM / Owner 密钥。
- 落地网页激活和钱包激活下统一的 BNS 文档发布链路，优先发布集成后的 zone-config。
- 落地域名分流：
  - BNS domain 走 BNS 合约管理和 BNS 写路径；
  - `user_domain` 走传统 SN 协助添加 DNS 记录路径，不走 BNS 合约管理。
- 去掉 `node_active` 中临时 no-op 的 `bind_owner_key()`，替换为 NameLib 派生密钥或钱包签名驱动的新步骤。
- 落地 SN `deviceinfo` 在线态上报的设备私钥签名/证明流程，不依赖上游登录 Access Token。
- 补充前后端联动文档，避免再次出现 `/kapi/sn/bns` 这类旧路径。

## 验证项

- `pnpm --dir src/kernel/node_active build`
- `cargo check`
- `uv run buckyos-build.py --skip-web`
- 方案落地后补 DV Test：网页激活、钱包激活、SN deviceinfo 上报、BNS 文档可解析。
- 补充网页激活密钥验证：同一助记词派生出的 EVM / Owner 密钥在激活期保持一致，且不成为 BuckyOS 长期运行依赖。
- 补充 DID Document 签名验证：激活阶段 Owner 密钥可用于 BuckyOS DID Document 签名和验证；正常运行阶段不依赖本地 Owner 私钥。
- 补充 BNS 访问验证：匿名读可用；写操作使用 EVM 密钥或底层库封装；不依赖上游登录 Access Token。
- 补充 zone-config 发布验证：zone-config 可通过 BNS 发布，并覆盖 boot 信息和 device mini doc 所需内容。
- 补充域名分流验证：BNS domain 走 BNS 合约；`user_domain` 走 SN 协助添加 DNS 记录。
- 补充 SN deviceinfo 验证：已注册设备使用当前设备私钥即可完成上报。
