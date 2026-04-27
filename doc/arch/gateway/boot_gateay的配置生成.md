# Boot Gateway 配置生成逻辑

本文档定义 BuckyOS 下一阶段要实现的 boot 阶段 gateway 配置与 RTCP route 生成逻辑。

## 当前实现基线

当前 BuckyOS 的 gateway 配置分为三层：

1. `src/rootfs/etc/boot_gateway.yaml`
   - 静态 boot 配置。
   - 定义 `node_rtcp`、`zone_gateway_http`、`node_gateway_http` 和 `node_gateway` HTTP server。
   - 从 `node_gateway_info.json` 读取 `APP_INFO`、`SERVICE_INFO`、`NODE_ROUTE_MAP`、`TRUST_KEY`、`NODE_INFO`。
2. `nodes/<node>/gateway_info`
   - scheduler 生成。
   - node-daemon 拉取后落地为 `$BUCKYOS_ROOT/etc/node_gateway_info.json`。
   - 当前包含 `node_info`、`app_info`、`service_info`、`node_route_map`、`trust_key`。
3. `nodes/<node>/gateway_config`
   - scheduler 生成。
   - node-daemon 拉取后落地为 `$BUCKYOS_ROOT/etc/node_gateway.json` 并触发 cyfs-gateway reload。
   - 当前主要承载 zone TLS、ACME、静态 web dir server 等后置配置。

当前 `node_route_map` 由 scheduler 根据 `devices/*/info` 生成，格式是：

```json
{
  "ood2": "rtcp://ood2.example.zone/",
  "node1": "rtcp://node1.example.zone:2981/"
}
```

这只表达了单一路径，无法表达直连、SN relay、ZoneGateway relay、多端口、多优先级和失败降级。

当前 cyfs-gateway 已支持：

- RTCP `keep_tunnel` 配置和 `--keep_tunnel` 启动参数。
- RTCP `on_new_tunnel_hook_point` 准入控制。
- RTCP nested remote/bootstrap URL：

```text
rtcp://<percent-encoded bootstrap URL>@<target-did>[:port]/<target-stream>
```

这使得 `Node -> SN/ZoneGateway -> Target` 这类 relay route 可以被正式表达。

## 设计目标

BootGateway 的目标是：在 scheduler 产物不可用或不完整时，让 cyfs-gateway 提供最小可用网络能力，并在 scheduler 产物出现后平滑切换到正式配置。

必须满足：

1. OOD 能在 boot 阶段尽量与其它 OOD、SN、ZoneGateway 建立 RTCP tunnel。
2. 非 OOD ZoneGateway 能作为 OOD keep-tunnel 的目标，并在 OOD 连上后访问 system-config。
3. 普通 Node 能在没有完整 system-config 的情况下，尽量通过 ZoneGateway、LAN OOD、SN relay 连接到 system-config。
4. 访问 Zone 内服务时，优先使用短路径：
   - 本机服务：`127.0.0.1:<port>`
   - 远端服务直连：`rtcp://<target-device>/...`
   - 远端服务 relay：`rtcp://<encoded-bootstrap>@<target-device>/...`
5. scheduler 后续生成的正式 `gateway_info/gateway_config` 可以接管 boot 期产物。

不在本阶段解决：

- RTCP 协议本身的加密、握手、anti-replay 逻辑。该部分以 cyfs-gateway `doc/rtcp.md` 为准。
- 大规模 OOD quorum 的一致性协议。本文只定义 gateway 层需要的连接与 route 产物。
- 完整 LAN discovery 协议。本文只定义它产生的 route 信息如何被消费。

## 关键约束

### 3180 绑定

`node_gateway_http` 当前绑定 `0.0.0.0:3180`，这是 Docker/容器访问约束导致的现实要求，不能简单改成 `127.0.0.1`。

因此安全边界不能依赖 bind address，必须由以下机制承担：

- 主机防火墙或部署环境限制外部访问 3180。
- `node_gateway` HTTP server 对 service/app 做鉴权与 RBAC。
- RTCP tunnel 建立前使用 `on_new_tunnel_hook_point` 做来源准入。
- 对敏感 service 保持服务自身鉴权，不把 3180 视为可信入口。

### route 选择职责

BuckyOS 不应在配置层手写 IP 级别的 `ipv4 > ipv6` 竞速逻辑。当前 cyfs-gateway/name-client 已负责 DID 地址解析、候选 IP 排序和 Happy Eyeballs 风格连接竞速。

BuckyOS 负责生成“路径候选”：

- direct RTCP candidate
- via SN candidate
- via ZoneGateway candidate
- local LAN discovery candidate

具体某条 RTCP direct candidate 内部使用哪个 IP，由 RTCP/name-client 决定。

## Boot Route 数据模型

下一阶段应把 `node_route_map` 从 `node_id -> url` 升级为 `node_id -> route candidates`。

建议新结构：

```json
{
  "routes": {
    "ood2": [
      {
        "id": "direct",
        "kind": "rtcp_direct",
        "priority": 10,
        "url": "rtcp://ood2.example.zone/",
        "source": "zone_config"
      },
      {
        "id": "via-sn",
        "kind": "rtcp_relay",
        "priority": 30,
        "url": "rtcp://rtcp%3A%2F%2Fsn.example.org%2F@ood2.example.zone/",
        "relay_node": "sn.example.org",
        "source": "zone_config"
      }
    ]
  },
  "node_route_map": {
    "ood2": "rtcp://ood2.example.zone/"
  }
}
```

字段说明：

- `routes`：正式 route candidate 列表。
- `node_route_map`：兼容当前 `boot_gateway.yaml` 的单 URL 映射，取最高优先级 candidate。
- `kind`
  - `rtcp_direct`：直接连接目标 device RTCP stack。
  - `rtcp_relay`：通过 SN 或 ZoneGateway 建立 bootstrap-backed RTCP tunnel。
  - `local`：本机服务，不进入 `routes`，由 selector 命中本机时直接走 `127.0.0.1:<port>`。
- `priority`：数值越小优先级越高。
- `source`
  - `zone_boot_config`
  - `zone_config`
  - `system_config`
  - `lan_discovery`
  - `manual`

本阶段为了减少改动，可以先保留当前 `NODE_ROUTE_MAP` 单字符串读取方式，同时生成 `routes` 作为新字段。后续再修改 `boot_gateway.yaml` 的 process chain，使它能按 candidate 列表 forward。

## RTCP URL 生成规则

### Direct route

默认 RTCP 端口：

```text
rtcp://<device-did-host>/
```

非默认 RTCP 端口：

```text
rtcp://<device-did-host>:<rtcp-port>/
```

其中 `<device-did-host>` 优先使用可被 name-client 解析的 device DID hostname，例如：

```text
ood2.test.buckyos.io
```

### Relay route

通过 relay 节点建立到 target 的 RTCP tunnel 时，使用 cyfs-gateway 当前 nested remote/bootstrap URL：

```text
rtcp://<percent-encoded bootstrap URL>@<target-device-did-host>[:target-rtcp-port]/
```

示例：

```text
rtcp://rtcp%3A%2F%2Fsn.devtests.org%2F@ood2.test.buckyos.io/
```

语义：

1. 先通过 `rtcp://sn.devtests.org/` 建立 bootstrap stream。
2. 再在该 stream 上与 `ood2.test.buckyos.io` 建立外层 RTCP tunnel。
3. 外层 RTCP 身份认证仍以 target device DID 为准，relay 不参与 target 身份认证。

不再使用旧式模糊表达：

```text
rtcp://relay/rtcp://target/
```

## 角色启动流程

### OOD

输入：

- 本机 device doc/private key。
- `ZoneBootConfig`。
- 本地缓存的 LAN discovery 结果。
- 可选 SN 信息。

Boot 阶段行为：

1. 启动 `node_rtcp`。
2. 根据 `ZoneBootConfig.oods` 为其它 OOD 生成 direct route candidates。
3. 如果存在 SN，生成 via SN relay candidates。
4. 如果 `ZoneBootConfig` 标记了 ZoneGateway 节点，生成 via ZoneGateway relay candidates。
5. 把需要长期保持的目标写入 RTCP `keep_tunnel`：
   - 其它 OOD direct candidate。
   - SN candidate，前提是本机不是稳定 WAN 可达。
   - ZoneGateway candidate，前提是该节点承担 relay/公网入口职责。
6. 通过本机 `system_config` 或 `127.0.0.1:3180/kapi/system_config` 进入后续启动。

多 OOD quorum 的启动门槛需要另行与 system-config/klog 设计对齐。gateway 层只负责提供 route 和 keep tunnel 能力，不在本阶段定义“必须连上 n 个 OOD 才启动 system-config”的一致性规则。

### 非 OOD ZoneGateway

输入：

- 本机 device doc/private key。
- `ZoneBootConfig` 或 ZoneGateway 注册信息。
- 可选 SN 信息。

Boot 阶段行为：

1. 启动 `node_rtcp`，允许 OOD 建立 tunnel。
2. 使用 `on_new_tunnel_hook_point` 限制只有同 Zone OOD 或受信设备可建立 tunnel。
3. 一旦有 OOD tunnel 建立成功，优先通过该 tunnel 访问 system-config。
4. 如果长时间没有 OOD 连入，可主动尝试：
   - direct OOD route
   - via SN route
   - via other ZoneGateway route

非 OOD ZoneGateway 和普通 Node 的区别是：它必须先作为 OOD 的 relay/target 存在，不能只作为 system-config client。

### 普通 Node

输入：

- 本机 device doc/private key。
- zone hostname 或 ZoneBootConfig/ZoneConfig 缓存。
- LAN discovery 缓存。

Boot 阶段目标只有一个：连接上 system-config。

候选路径：

1. ZoneGateway：

```text
https://<zone-host>/kapi/system_config
```

2. 本机 gateway 到 OOD direct：

```text
http://127.0.0.1:3180/kapi/system_config
```

底层 route candidate：

```text
rtcp://<ood-device>/:3200
```

3. 本机 gateway 经 SN 到 OOD：

```text
rtcp://<encoded-sn-bootstrap>@<ood-device>/:3200
```

4. LAN discovery 得到的 OOD direct candidate。

普通 Node 当前实现缺口较大：node-daemon 的非 OOD `get_system_config_client()` 仍需要实现，且需要能使用 boot route candidates。

## scheduler 接管流程

scheduler 启动后，根据 system-config 生成正式产物：

- `nodes/<node>/gateway_info`
  - `node_info`
  - `app_info`
  - `service_info`
  - `routes`
  - `node_route_map`
  - `trust_key`
- `nodes/<node>/gateway_config`
  - zone TLS stack
  - ACME 配置
  - 静态 web dir server
  - 后续需要时可包含 RTCP on_new_tunnel 策略和 keep_tunnel 配置

node-daemon 负责：

1. 拉取 `gateway_info`，落地为 `node_gateway_info.json`。
2. 拉取 `gateway_config`，落地为 `node_gateway.json`。
3. 检测内容变化后 reload cyfs-gateway。
4. 保留现有 tunnel，由 cyfs-gateway 自行复用或重建；除非配置显式删除某类准入或 route。

如果 scheduler 产物缺失，cyfs-gateway 应继续使用 boot 阶段 route，不应导致系统网络能力完全消失。

## 安全边界

### RTCP tunnel 准入

Boot 阶段必须补充 `node_rtcp.on_new_tunnel_hook_point`。

准入策略：

1. 同 Zone device 默认允许。
2. OOD、ZoneGateway、SN 可按角色加入 allow list。
3. 跨 Zone device 只有在明确存在 trust relationship 时允许。
4. 未携带可验证 `device_doc_jwt` 的来源只能使用较弱字段，如 `source_device_id`，默认不应允许敏感 relay 能力。

可用字段以 cyfs-gateway 当前实现为准：

- `REQ.source_device_id`
- `REQ.source_device_name`
- `REQ.source_device_owner`
- `REQ.source_zone_did`
- `REQ.source_device_doc_jwt`
- `REQ.source_addr`

### relay 权限

RTCP relay 不应默认开放给任意来源。

最低要求：

- SN relay：只允许已认证 OOD/ZoneGateway/同 Zone device 使用。
- ZoneGateway relay：只允许同 Zone device 使用，或显式受信 device 使用。
- 普通 Node 默认不作为 relay，除非配置明确标记。

### 3180 HTTP 入口

因为 Docker 约束，3180 绑定 `0.0.0.0`。实现时必须假定 3180 可能被非本机访问，因此：

- `/kapi/*` 仍必须依赖 service 自身认证和 RBAC。
- gateway process chain 不能因为请求来自 3180 就跳过鉴权。
- 部署脚本或文档应建议通过防火墙限制 3180 外部访问。

## 下一阶段实现任务

### 阶段 1：最小 route candidate 产物

1. 在 scheduler 的 `gateway_info` 中新增 `routes`，保留 `node_route_map` 兼容字段。
2. route candidate 先支持：
   - direct RTCP
   - via SN RTCP relay
3. 用当前 `devices/*/info` 和 `boot/config` 生成 route candidates。
4. 单元测试覆盖：
   - 默认 2980 端口。
   - 非默认 RTCP 端口。
   - 有 SN 时生成 relay candidate。
   - `node_route_map` 兼容字段仍保持旧格式。

### 阶段 2：boot route builder

1. 增加 boot route builder，输入 `ZoneBootConfig`、本机角色、本机 device doc、缓存 discovery 结果。
2. 生成 boot 期 `node_gateway_info.json` 或等价临时配置。
3. 生成 RTCP keep_tunnel 列表。
4. node-daemon 启动 cyfs-gateway 时不再只临时传 SN，而是传入或落地完整 keep_tunnel。

### 阶段 3：process chain 使用 candidate 列表

1. 修改 `boot_gateway.yaml` 的 `forward_to_service` 和 `forward_to_app`。
2. 当目标不在本机时，从 `routes[target_node]` 生成 weighted/priority forward map。
3. 保留旧 `NODE_ROUTE_MAP` fallback。
4. debug tests 覆盖 direct 和 relay URL。

### 阶段 4：非 OOD Node 启动

1. 实现 node-daemon 非 OOD `get_system_config_client()`。
2. 支持通过本机 `3180` 和 boot route candidate 访问 OOD system-config。
3. 支持 ZoneGateway 失败后降级到 LAN discovery 或 SN relay。

### 阶段 5：RTCP 准入策略

1. 在 boot gateway 中加入 `on_new_tunnel_hook_point`。
2. scheduler 根据 zone/device/trust 信息生成正式准入配置。
3. 增加同 Zone allow、跨 Zone deny、relay deny/allow 的测试。

## 验证要求

每个阶段完成后至少验证：

```bash
cargo test
uv run buckyos-build.py --skip-web
uv run src/test/test_boot_gatweay/run_debug_tests.py
```

涉及 cyfs-gateway RTCP nested URL 或 keep_tunnel 行为时，还需要在 cyfs-gateway 仓库运行对应 RTCP 测试。

## 需要同步更新的文档

修改实现时必须同步检查：

- `doc/arch/gateway/zone-boot-config与zone-gateway.md`
- `doc/arch/02_boot_and_activation.md`
- `doc/arch/06_network_and_gateways.md`
- `doc/arch/09_pitfalls.md`
- cyfs-gateway `doc/rtcp.md`

如果 route 数据模型从 `node_route_map` 单字符串升级，所有引用 `NODE_ROUTE_MAP` 的文档和测试都必须同步更新。
