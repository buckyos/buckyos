# Device Identity 本机路径调整 TODO

## 背景

上游设计见：

- `/Users/liuzhicong/project/buckyos-base/doc/did-identity-certificate-manager.md`
- `/Users/liuzhicong/project/buckyos-base/src/name-client/src/identity_mgr.rs`

目标是把 device 相关身份材料按上游 identity path protocol 保存和读取：

- public identity root：`$BUCKYOS_ROOT/local/identity`
- security root：`$BUCKYOS_ROOT/security`
- 目录名：`DID::to_raw_host_uri()` 后按上游规则编码
- device DID 一般为 `did:web:ood1.$zonename` 或 `did:bns:ood1.$zonename`

这是 breaking change，不做旧路径兼容。

## 本次范围

本次只约束本机身份材料的保存和读取，不改变 system-config 的分布式数据模型。

明确不改：

- system-config 中 `devices/<short_name>/doc`
- system-config 中 `devices/<short_name>/info`
- verify-hub / scheduler / zone resolver 对 system-config 的分布式读取语义

明确要改：

- activation 生成的本机 identity 文件布局
- node-daemon 本机启动时读取 device DID document / private key 的逻辑
- boot RTCP gateway 使用的 device config / private key 路径
- buckyos-api runtime 对设备身份和设备私钥的加载策略

## 新文件布局

以 device DID `did:web:ood1.example.com` 为例：

```text
$BUCKYOS_ROOT/etc/node_identity.json

$BUCKYOS_ROOT/local/identity/ood1.example.com/
  did.json
  device_doc.jwt
  device_mini_doc.jwt

$BUCKYOS_ROOT/security/ood1.example.com/
  authentication.private.pem
  authentication.keyref.json
```

说明：

- `node_identity.json` 必须保留。它是判断 node 是否已激活的关键 marker。
- `node_identity.json` 不再保存 `device_doc_jwt` / `device_mini_doc_jwt` 正文。
- `device_doc.jwt` 和 `device_mini_doc.jwt` 是公开信息，按 DID 路径放入 public identity root。
- `did.json` 保存 decoded DeviceConfig DID document。里面已包含公钥，不需要额外保存 device public key。
- 设备私钥放在 security root，对应 usage 建议使用 `authentication`。
- `authentication.keyref.json` 不能生成，物理文件和keyref二选1

`node_identity.json` 建议保留字段：

```json
{
  "schema": "buckyos.node_identity.v2",
  "zone_did": "did:web:example.com",
  "owner_did": "did:bns:alice",
  "owner_public_key": {},
  "device_name": "ood1",
  "device_did": "did:web:ood1.example.com",
  "zone_iat": 0
}
```

保留 `owner_public_key`：启动早期仍需要用它验证 zone boot / device doc，除非同时完成从 boot/config 或 owner DID document 取 owner key 的启动前置逻辑。

## TODO

### 1. 引入本机 identity path helper

- 优先复用上游 `name-client::identity_mgr` 的 `IdentityRoots` / `IdentityUsage` / `IdentityMaterial`。
- 如果当前依赖版本还没有这些 API，先升级 `name-client` / `name-lib` 到包含上游实现的版本。
- 不要在 buckyos 内复制一份路径编码算法，避免和上游协议漂移。

验收点：

- 能通过 device DID 算出 public identity dir 和 security dir。
- 能通过 `authentication` usage 算出 private key / keyref 路径。

### 2. 修改 DeviceConfig 生成 DID 的规则

当前上游 `DeviceConfig::new()` / `new_by_jwk()` 默认生成 `did:dev:<pkx>`。

需要新增或使用显式构造方式，使 activation 能生成：

```text
did:web:ood1.<zone-host>
did:bns:ood1.<zone-name>
```

注意：

- `verificationMethod[*].controller` 要跟随新的 device DID。
- 公钥仍保存在 `verificationMethod[*].public_key`。
- 不要再从 device DID 字符串反推公钥。

### 3. 修改 activation 写入逻辑

入口：

- `src/kernel/node_daemon/src/active_server.rs`

需要调整两条路径：

- `handle_active_by_wallet`
- `handle_do_active`

改动：

- 不再写 `$BUCKYOS_ROOT/etc/node_private_key.pem`
- 不再写 `$BUCKYOS_ROOT/etc/node_device_config.json`
- `node_identity.json` 继续写入，但使用 v2 简化结构，不包含 doc JWT 正文
- 写入 public identity root：
  - `did.json`
  - `device_doc.jwt`
  - `device_mini_doc.jwt`
- 写入 security root：
  - `authentication.private.pem`
  - `authentication.keyref.json`

验收点：

- 激活后 `etc/node_identity.json` 存在，`check.py` 仍能判断已激活。
- 激活后能按 device DID 从 identity root 读取 DID document 和 JWT。
- 激活后能按 device DID 从 security root 读取私钥或 keyref。

### 4. 修改 node-daemon 启动读取逻辑

入口：

- `src/kernel/node_daemon/src/node_daemon.rs`

当前逻辑：

- 读 `etc/node_identity.json`
- 从 `device_doc_jwt` decode DeviceConfig
- 读 `etc/node_private_key.pem`

目标逻辑：

- 读 `etc/node_identity.json` 判断激活并获得 `device_did` / `zone_did` / `owner_public_key`
- 按 `device_did` 从 public identity root 读取 `device_doc.jwt`
- decode / verify 得到 DeviceConfig
- 按 `device_did` 从 security root 读取 `authentication.keyref.json`
- 只有 node-daemon 自身启动和必须签名的 boot 流程，才解析 keyref 并加载 `authentication.private.pem`

验收点：

- node-daemon 启动不依赖 `etc/node_device_config.json`。
- node-daemon 启动不依赖 `etc/node_private_key.pem`。
- `BUCKYOS_THIS_DEVICE` 仍可以设置为 decoded DeviceConfig JSON，供现有内存态消费者使用。

### 5. 修改 boot RTCP gateway 路径

入口：

- `src/rootfs/etc/boot_gateway.yaml`
- `src/kernel/scheduler/src/system_config_agent.rs`

当前配置：

```yaml
key_path: ./node_private_key.pem
device_config_path: ./node_device_config.json
```

目标：

- 切换到新路径下的 `authentication.private.pem`
- `device_config_path` 切换到 public identity root 下的 `did.json`，或 gateway 支持 JWT 时使用 `device_doc.jwt`
- 如果 cyfs-gateway 支持 identity manager / keyref，优先改成 identity manager 配置，不直接写死私钥路径

验收点：

- boot 阶段 RTCP 可用。
- 不再需要 rootfs `etc/node_private_key.pem` / `etc/node_device_config.json`。

### 6. 重新设计 buckyos-api runtime 的设备私钥加载

入口：

- `src/kernel/buckyos-api/src/runtime.rs`

当前 `fill_by_load_config()` 会自动从 config root 加载设备私钥。

目标：

- 读设备身份：默认允许，从 `node_identity.json` + identity root 读取。
- 读设备私钥：默认不自动加载。
- 需要设备私钥的调用方必须显式声明，例如新增类似：
  - `runtime.enable_device_signing(...)`
  - `runtime.load_device_private_key(...)`
  - 或初始化参数 `require_device_private_key: true`

需要重点审计真实用途：

- `node_daemon` 启动和 kernel service token 生成是否仍需要直接私钥。
- `KernelServiceRunItem::start()` 是否应该由 node-daemon 统一签发 token，而不是 service runtime 自己自动持有设备私钥。
- `app_loader` / `buckycli loader` / `aicc DeviceJwt` 是否真的应该用设备私钥，还是应改用 verify-hub/session token。
- `refresh_session_token()` 是否仍应自动用设备私钥续签。

验收点：

- 普通 app/frame/kernel service runtime 初始化不会隐式读取设备私钥。
- 真正需要签名的组件在初始化时显式声明，并能从 keyref 读取。
- 缺少私钥时错误信息明确，不静默降级到奇怪状态。

### 7. 保持 system-config 逻辑不动

不要因为本地路径调整改动以下语义：

- scheduler 写 `devices/<short_name>/doc`
- node-daemon 写 `devices/<short_name>/info`
- verify-hub 从 `devices/<issuer>/doc` 取 device public key
- ZoneDidResolver 通过 DID / short name 解析 system-config 中的 device doc / info

如果后续决定 JWT issuer 从 short name 改成 full device DID，再单独开任务处理 system-config lookup normalization。

### 8. 最后再改 make_config.ts 和测试环境生成

入口：

- `src/make_config.ts`
- `src/kernel/buckyos-api/src/test_config.rs`
- `src/tools/buckycli/src/did.rs`
- cyfs-gateway 仓库 `src/make_sn_config.ts`（原本仓库 `src/make_sn_configs.ts`，已迁出）

顺序要求：

1. 先完成 runtime / node-daemon / gateway 的新路径读取。
2. 再改 dev config 生成和复制逻辑。
3. 不要先改 `make_config.ts`，否则生成出来的环境会无法被当前 runtime 启动。

## 需要同步更新的辅助工具

- `src/check.py`：继续用 `etc/node_identity.json` 判断激活即可，必要时识别 v2 schema。
- `src/rootfs/etc/backup.py`：备份列表要加入 `local/identity/<device>` 和 `security/<device>`，不再备份旧 `node_private_key.pem`。
- `src/frame/control_panel/src/zone_mgr.rs`：Zone Overview 不再读 `node_device_config.json`，改从 node_identity + identity root 读取 device doc。
- `src/rootfs/bin/service_debug.tsx`：改为读取新路径，且私钥读取应显式说明。

## 建议验证

最小验证：

```bash
cd src
cargo test
uv run buckyos-build.py --skip-web
```

DV 验证：

```bash
uv run src/check.py
uv run test/run.py --list
uv run test/run.py -p <relevant_test_name>
```

手工检查：

- 激活后 `etc/node_identity.json` 存在。
- 激活后 `local/identity/<device-dir>/did.json` 存在。
- 激活后 `local/identity/<device-dir>/device_doc.jwt` 存在。
- 激活后 `local/identity/<device-dir>/device_mini_doc.jwt` 存在。
- 激活后 `security/<device-dir>/authentication.keyref.json` 存在。
- 激活后 `security/<device-dir>/authentication.private.pem` 存在。
- `etc/node_private_key.pem` 和 `etc/node_device_config.json` 不再被新流程依赖。

## 风险点

- cyfs-gateway boot RTCP 是否支持读取 `did.json` 或 keyref，需要实际确认。
- `owner_public_key` 从 `node_identity.json` 移除会影响启动早期验签；本任务暂不移除。
- runtime 私钥显式加载会暴露以前隐藏的依赖，可能需要逐个调用方改造。
- 如果 device DID method 在 `did:web` 和 `did:bns` 之间切换，identity dir 也会变；测试要覆盖两种 DID。
