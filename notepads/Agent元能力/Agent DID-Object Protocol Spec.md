# Agent DID-Object Protocol Spec

Status: Draft v0.3
Source protocol: `/Users/liuzhicong/project/buckyos-base/doc/did-object-protocol.md`  
Protocol version: `did-object/1`
Implementation reference: `/Users/liuzhicong/project/buckyos-base/src/buckyos-http-server/src/test_did_obj_server.rs`

本文从 `src/frame/agent-did-object-lib` 视角整理 DID Object Protocol 的落地方式，并按 `buckyos-base` 当前实现状态校正文档表述。基础协议已经收敛为一套**封闭世界、caller-neutral、可验证、可实现**的对象能力协议；本文不再把 `read()`、`ReadResult`、`obj://` 或 Agent Tool Result 当作核心协议，而是说明 `src/frame/agent-did-object-lib` 如何消费 DID Object Card、Profile、Trait、Property、Action 和 Event。

注意：`buckyos-base` 已完成 DID Object Card、Profile、HTTP server adapter、HTTP client、property/action endpoint resolution 和 ObjURL pattern routing 的框架实现，但 DID Document 强验证、完整 JSON Schema 校验、授权/RBAC、ETag/conditional request、WebSocket event lifecycle 等细节尚未全部完成。本文中的 MUST / SHOULD 表示协议目标；“当前实现”小节描述已经落地的事实。

历史名词 `Global Object` 停止作为正式协议名使用。正式协议名是 **DID Object Protocol**。

---

## 1. 核心判断

DID Object Protocol 的核心链路是：

```text
Object URL
  -> DID Object Card (DID Document)
  -> DID Object Profile (constrained WoT Thing Description)
  -> Trait contracts
  -> declared property/action/event endpoint
```

`src/frame/agent-did-object-lib` 可以在上层继续提供：

```text
read(input, options?) -> Agent-facing view
xcall(object, action, params) -> Agent-facing action result
subscribe_event(object, event, options?) -> Agent-facing subscription
```

但协议边界必须清楚：

- `read()` 是开放世界、session-aware、LLM-facing 的解释和渲染能力，不属于 DID Object Protocol 核心。
- `xcall()` 是 `src/frame/agent-did-object-lib` 对 DID Object Action Invocation 的包装，不是协议名。
- `subscribe_event()` 是 `src/frame/agent-did-object-lib` 对 DID Object Event lifecycle 的包装。
- `obj://` 如保留，只能作为 Gateway 层 semantic URI 或 resolver input；进入 `src/frame/agent-did-object-lib` 前应解析为 canonical Object URL。它不是 v1 wire endpoint。
- `read_after`、`known_in_session` 等词属于 Agent-facing rendering；核心协议使用 `refresh_hints`、`affected_objects`、`invalidated_objects`、ETag、schema hash 和 version metadata。

协议层原则：

```text
DID Object Protocol:
  closed-world, caller-neutral, declared capabilities only.

src/frame/agent-did-object-lib:
  open-world, route-dependent, session-aware, agent-facing.
```

### 1.1 当前实现快照

上游 `buckyos-base` 当前已经落地的核心模块：

| 模块 | 已实现能力 |
|---|---|
| `name-lib::DIDObjectCard` | DID Object Card 结构、`DIDObjectService` 解析、基础校验、JWT encode/decode 入口。 |
| `name-lib::ObjectProfile` | TD-style Profile 结构、trait/property/action/event 声明解析、默认 endpoint resolution、IndexTrait 基础成员校验。 |
| `name-client::DIDObjectClient` | 通过 Object URL 获取 Card/Profile，解析 property/action/event endpoint，读取 property，调用 action。 |
| `buckyos-http-server::DIDObjectHttpServer` | 暴露 `did.json`、`profile.json`、`props/{name}`、`methods/{name}`、`events[/name]` 的 HTTP adapter。 |
| `DIDObjectServer` trait | 对象服务实现入口：`object_card_for`、`object_profile_for`、`read_property`、`invoke_action`、`handle_event_request`。 |
| ObjURL pattern | 支持固定 Object URL、`/devices/{camera_id}`、`/devices/:camera_id` 和尾部 `*` pattern，把 path capture 放入 `DIDObjectRequestContext.object_params`。 |

当前实现仍是框架层，不应把以下能力写成已完成：

- DID Document / controller / signature 的完整信任链验证。
- action `input` / property schema 的完整 JSON Schema validation。
- auth、RBAC、quota、freshness、idempotency、confirm token 和审计策略。
- Card/Profile/property 的 ETag、Last-Modified、conditional request。
- Event WebSocket binding 的 subscribe / renew / unsubscribe / status / stream 完整实现。

---

## 2. 与 MCP 的关系

MCP 解决工具连接问题：

```text
model -> tool(name, params)
```

DID Object Protocol 解决对象能力问题：

```text
caller -> resolve(Object URL | DID)
caller -> GET declared property
caller -> POST declared action
caller -> WS subscribe declared event
```

二者可以互通：

- MCP server 可以暴露 DID Object resolver、action invocation 或 event subscription 工具。
- DID Object Host 可以把 MCP resource 或 tool 适配成 DID Object Profile 中声明的 property/action/event。
- `src/frame/agent-did-object-lib` 可以把 DID Object Action 映射成 LLM 可见的 `xcall`。

但 DID Object 的核心抽象是 object-centric，不应退化成平面 tool list。LLM 工具面可以很小，Runtime 内部必须保留 Object URL、DID Object Card、Profile、Trait 和 endpoint 的完整结构。

---

## 3. 术语映射

| Term | Agent 侧含义 | 协议层事实 |
|---|---|---|
| Object URL | Agent 可引用对象的稳定 handle | DID Object 的一等对象引用，通常是 HTTPS URL。 |
| Object DID | 可验证对象身份 | DID Object Card 的 `id`。 |
| DID Object Card | Agent 可用的 control-plane object view | v1 固定为 DID Document。 |
| DID Object Profile | 对象类型能力声明 | constrained WoT TD，声明 properties/actions/events/forms/schema/traits。 |
| Trait | 通用能力契约 | 具名、版本化的 properties/actions/events/schema 约束。 |
| Property | 可读取状态 | Profile `properties`，v1 使用 HTTP GET。 |
| Action | 可调用能力 | Profile `actions`，v1 使用 kRPC-style HTTP POST。 |
| Event | 可订阅事件 | Profile `events`，v1 目标是 WebSocket lifecycle；当前实现已支持 event 声明和 endpoint 解析，HTTP adapter 仍返回 unsupported。 |
| ObjectRef | Agent-facing 轻量引用可选名 | 协议层等价于 Object URL。 |
| ObjectView | Agent-facing 语义视图可选名 | 协议层 DID Object Card 只是 control-plane view。 |
| ObjURL pattern | Object URL path 匹配规则 | 当前 `DIDObjectHttpServer` 用它把一组对象路由给同一个 server。 |

为避免混淆，本文在协议层优先使用 **Object URL** 和 **DID Object Card**。

---

## 4. Object URL、DID 与 Resolver

Object URL 是协议层主 handle，SHOULD 稳定、可解析到 DID Object Card，并出现在 DID Document 的 `alsoKnownAs` 中。

示例：

```text
https://myhome.com/devices/cam01
https://agent.booking.com/objects/stay-offer/abc123
https://repo.example.com/repos/buckyos/did-object-protocol
```

Object DID 是对象身份：

```text
did:web:myhome.com:devices:cam01
did:web:agent.booking.com:objects:stay-offer:abc123
did:bns:myhome:devices:cam01
did:bns:$named_objid.myhome
```

Gateway / Resolver 层可以依赖 DID Object Resolver。协议目标允许输入 Object URL 或 DID：

```ts
resolve(input: ObjectURL | DID): Promise<ResolvedObjectCard>
```

典型返回：

```json
{
  "object_url": "https://myhome.com/devices/cam01",
  "object_did": "did:web:myhome.com:devices:cam01",
  "object_card": {},
  "verified": true,
  "resolution": {
    "route": "did-web-default",
    "fetched_at": "2026-06-07T12:00:00Z",
    "valid_until": "2026-06-07T13:00:00Z",
    "card_etag": "\"card-v12\"",
    "trust": "verified"
  }
}
```

默认 Web-compatible route 已在 `DIDObjectCard::default_card_url` 和 `DIDObjectClient::resolve_card` 中实现：

```text
GET {object_url}/did.json
```

`DIDObjectClient::resolve(object_url)` 当前返回：

```rust
ResolvedDIDObject {
    object_url,
    object_card,
    object_profile,
}
```

其中：

- `object_url` 会做 trim 和去尾部 `/` 的规范化。
- `object_card` 来自 `GET {object_url}/did.json`，并通过 `DIDObjectCard::validate()` 做基础结构校验。
- `object_profile` 根据 `DIDObjectService.serviceEndpoint` 和 `profile` 解析 URL 后 GET，并通过 `ObjectProfile::validate()` 做基础结构校验。

当前 `DIDObjectClient` 的主入口是 Object URL；DID 解析路径仍主要由 `NameClient` / provider 层处理，例如测试中通过 `NameClient::resolve_did()` 解析 `did:web:127.0.0.1%3A...`。

BuckyOS Runtime MAY 支持 local registry、Zone registry、BNS、DID resolver cache、host manifest 或预配置 DID Object Host。无论 route 如何，最终都必须得到合法且可验证的 DID Object Card。

### 4.1 obj:// 兼容

v1 核心协议不要求 `obj://`。如果 Gateway 层继续使用：

```text
obj://booking.com/stay.offer/abc123
```

它必须被视为上层 semantic URI 或 resolver input。进入 `agent-did-object-lib`、DID Object data/control plane 前，Gateway SHOULD 把它解析为 Object URL。

`obj://` MUST NOT 直接作为 property/action/event 的 wire endpoint。

当前 `buckyos-base` 框架没有实现 `obj://` wire binding；`DIDObjectHttpServer` 的 ObjURL pattern 是 Object URL path pattern，不是 `obj://` scheme。

---

## 5. DID Object Card

DID Object Card v1 固定为 DID Document。最小示例：

```json
{
  "@context": [
    "https://www.w3.org/ns/did/v1",
    "https://buckyos.org/ns/did-object/v1"
  ],
  "id": "did:web:myhome.com:devices:cam01",
  "alsoKnownAs": ["https://myhome.com/devices/cam01"],
  "controller": "did:web:myhome.com",
  "service": [
    {
      "id": "#did-object",
      "type": "DIDObjectService",
      "serviceEndpoint": "https://myhome.com/devices/cam01",
      "profile": "https://buckyos.org/profiles/web-camera@1",
      "kind": "web.camera"
    }
  ]
}
```

DID Object Card MUST 是合法 DID Document，并至少包含：

| 字段 | 要求 | 说明 |
|---|---|---|
| `@context` | MUST | 当前 `DIDObjectCard::validate()` 要求包含 `https://www.w3.org/ns/did/v1`；默认构造同时包含 `https://buckyos.org/ns/did-object/v1`。 |
| `id` | MUST | Object DID。 |
| `alsoKnownAs` | SHOULD | Object URL，用于 DID 与 URL 互相确认。 |
| `controller` | SHOULD | 对象 controller DID。 |
| `verificationMethod` | SHOULD | 对象或 controller 的验证方法。 |
| `service` | MUST | 至少包含一个 `DIDObjectService`。 |
| `exp` / `iat` | MAY | 当前结构已保留时效字段；文档 revision 使用 `iat`，缺失时可由 `exp - DEFAULT_EXPIRE_TIME` 推导。 |
| `version_seq` | MAY | 仅作为用户自定义扩展保留，不参与 revision 或 anti-rollback。 |
| `keyScope` | MAY | 当前结构已保留 key scope 映射，并兼容别名 `buckyos:scopes`。 |

`DIDObjectService` 字段：

| 字段 | 要求 | 说明 |
|---|---|---|
| `id` | MUST | DID service id。 |
| `type` | MUST | 固定为 `DIDObjectService`。 |
| `serviceEndpoint` | MUST | 对象交互根 URL。 |
| `profile` | MUST | DID Object Profile URL。 |
| `kind` | SHOULD | 快速识别对象类型，例如 `web.camera`、`object.index`。 |

`profile` 和 `kind` 位于 service 项中，不放在 DID Document 顶层。`src/frame/agent-did-object-lib` 查找 `type = DIDObjectService` 的 service。

当前 `DIDObjectCard::new(id, object_url, controller, profile, kind)` 会：

- 去掉 `object_url` 尾部 `/` 后写入 `alsoKnownAs[0]`。
- 使用同一个 URL 作为 `DIDObjectService.serviceEndpoint`。
- 创建默认 service `id = "#did-object"`、`type = "DIDObjectService"`。

`DIDObjectCard::object_url()` 当前优先返回 `alsoKnownAs[0]`，如果不存在则 fallback 到 primary `DIDObjectService.serviceEndpoint`。`DIDObjectClient::card_declares_object_url()` 会用规范化后的 Object URL 检查 `alsoKnownAs` 或 `serviceEndpoint`。

Card SHOULD 支持 `ETag`、`Last-Modified`、`If-None-Match` 和 `If-Modified-Since`。非 HTTP resolver SHOULD 提供等价的 `card_version`、`fetched_at`、`valid_until`。当前 HTTP adapter 固定返回 JSON，还没有实现 conditional request。

---

## 6. DID Object Profile

DID Object Profile 是 constrained WoT Thing Description。它描述对象类型和能力，不保存对象实例的全部动态状态。

Profile MUST 使用 WoT 交互模型：

| DID Object 语义 | WoT TD 字段 |
|---|---|
| 属性 | `properties` |
| 动作 | `actions` |
| 事件 | `events` |
| endpoint | `forms[].href` |
| 操作类型 | `forms[].op` |
| 输入输出 schema | WoT Data Schema / JSON Schema-compatible subset |
| traits | `x-buckyos:traits` |

Runtime v1 只要求支持 TD-style JSON document，不要求完整 JSON-LD / RDF 推理。

当前 `ObjectProfile::validate()` 已实现的基础校验：

- `id` 不能为空。
- `@context` 必须包含 `https://www.w3.org/2022/wot/td/v1.1`；默认构造同时包含 DID Object context。
- property/action/event 名称不能为空。
- 如果某个 affordance 声明了 `forms`，其中至少一个 form 必须包含对应 op：`readproperty`、`invokeaction` 或 `subscribeevent`。
- 如果 profile 声明 `https://buckyos.org/traits/index@1`，必须包含 `index_schema` property、`query` action 和 `page` action。

当前实现还没有做完整 JSON Schema validation，也没有做完整 trait registry 校验。

相对 endpoint 基于 DID Object Card 中的 `DIDObjectService.serviceEndpoint` 解析：

```text
serviceEndpoint = https://myhome.com/devices/cam01
href            = methods/query_clip
resolved        = https://myhome.com/devices/cam01/methods/query_clip
```

如果 Profile 没有声明 form，BuckyOS DID Object Runtime 使用默认 endpoint 规则。当前 `ObjectProfile::{property_endpoint,action_endpoint,event_endpoint}` 已实现：

```text
property: GET  {serviceEndpoint}/props/{property_name}
action:   POST {serviceEndpoint}/methods/{action_name}
event:    WS   {serviceEndpoint}/events
```

endpoint resolution 规则：

- `forms[].href` 是绝对 URL 时直接使用。
- `forms[].href` 以 `/` 开头时基于 `serviceEndpoint` 的 origin 解析。
- `forms[].href` 以 `?` 或 `#` 开头时拼接到 `serviceEndpoint` 后。
- 其他相对路径拼接到 `serviceEndpoint` 后。
- event endpoint 会把 `http://` 转为 `ws://`，`https://` 转为 `wss://`。

当前 `DIDObjectHttpServer` 对传入 HTTP request 识别的固定 endpoint suffix 是：

```text
{object_path}/did.json
{object_path}/profile.json
{object_path}/props/{property_name}
{object_path}/methods/{action_name}
{object_path}/events
{object_path}/events/{event_name}
```

BuckyOS 扩展字段统一使用 `x-buckyos:*`：

| 字段 | 位置 | 说明 |
|---|---|---|
| `x-buckyos:traits` | profile top-level | 对象实现的 trait URI 列表。 |
| `x-buckyos:action` | action | 当前实现为 action policy object，包含 `effect`、`confirm`、`idempotency`，均为 policy hint。 |
| `x-buckyos:agentResult` | action | Agent Tool Result 兼容建议，非核心强约束。 |
| `x-buckyos:event` | event | 当前实现为 event policy object，包含 `delivery`。 |

---

## 7. Trait 与 IndexTrait

Trait 是具名、版本化的能力契约。实现某个 trait 意味着：

- Profile MUST 提供该 trait 要求的 property/action/event 名称。
- 输入输出 schema MUST 与 trait contract 兼容。
- endpoint 可以不同，但语义必须一致。
- trait URI 的主版本号表示兼容边界，例如 `@1`。

示例：

```json
{
  "x-buckyos:traits": [
    "https://buckyos.org/traits/index@1",
    "https://buckyos.org/traits/task@1"
  ]
}
```

Indexer 是实现 `IndexTrait` 的普通 DID Object，不是独立 tool。

Trait URI：

```text
https://buckyos.org/traits/index@1
```

实现 IndexTrait 的 Profile MUST 提供：

| 名称 | 类型 | 说明 |
|---|---|---|
| `index_schema` | property | 返回 query schema、columns、result profile 和分页约束。 |
| `query` | action | 使用 query 条件创建或读取一个索引页。 |
| `page` | action | 使用 cursor / snapshot 继续翻页。 |

MAY 提供 `count` action、`changed` event 和 `expired` event。

当前 `ObjectProfile::validate_index_trait()` 只校验上述三个必需成员是否存在，不校验 `index_schema`、`query`、`page` 的 schema 细节。

`IndexPage` SHOULD 包含：

- `index`: index object URL。
- `schema_id`: 结果表结构 ID。
- `schema_hash`: 表结构 hash，用于缓存和 session compression。
- `snapshot_id`: 稳定分页快照 ID。
- `rows[].object`: row 指向的 Object URL。
- `rows[].object_did`: row 指向对象的 DID。
- `rows[].version`: row object 或 row snapshot 版本。
- `next_cursor`: 下一页 cursor。

`src/frame/agent-did-object-lib` 可以把 `IndexPage` 渲染成表格、ref list 或 compressed collection，但这不属于核心协议。

---

## 8. Property Protocol

Property 对应 WoT `properties`。v1 property access：

```text
GET {property_endpoint}
```

返回值 MUST 与 Profile 中 property schema 兼容。

简单属性可以直接返回 JSON value：

```json
"AcmeCam"
```

结构化属性可以返回 object：

```json
{
  "value": 87,
  "unit": "percent",
  "updated_at": "2026-06-07T12:00:00Z",
  "version": "battery-v42"
}
```

Property endpoint SHOULD 支持：

- `ETag`
- `Last-Modified`
- `Cache-Control`
- `If-None-Match`
- `If-Modified-Since`
- `304 Not Modified`

当前 `DIDObjectHttpServer` 已实现：

- `GET {object_path}/props/{property_name}`。
- endpoint 中的 property 名称只允许一个 path segment，并支持 percent-decode，例如 `display%20name`。
- request 会构造成 `DIDObjectRequestContext`，包含 `object_url`、`object_path`、`object_params` 和 `endpoint_path`。
- 由具体 `DIDObjectServer::read_property(name, ctx)` 返回 JSON value。

当前 HTTP adapter 不会自动对照 Profile 检查 property 是否声明，也不会自动做 property schema 校验或 ETag/conditional response。`DIDObjectClient::read_property_from_resolved()` 会先从已解析 Profile 中查找 property 并解析 endpoint；服务端仍必须自己处理未知 property、授权和实际值校验。

适合放在 property：

- 不太变化的结构化状态。
- 对象摘要。
- 不需要参数的当前状态。
- 小型 JSON metadata。

不适合放在 property：

- 经常变化的大型数据流。
- 需要分页、过滤或排序的数据。
- 会产生副作用的操作。
- 需要确认、授权升级或支付的操作。

经验规则：

```text
small stable state       -> property
parameterized data       -> action
large media/resource     -> action returns resource descriptor
long-lived notification  -> event
search/pageable dataset  -> IndexTrait
```

---

## 9. Action Invocation 与 xcall

DID Object 的动作对应 WoT `actions`。核心协议名是 **Action Invocation**。`src/frame/agent-did-object-lib` MAY 暴露为：

```text
xcall(object, action, params)
```

Runtime 执行流程：

1. 用 canonical Object URL 得到 verified DID Object Card；DID / `obj://` resolver input 应在 Gateway 层提前转换。
2. 从 `DIDObjectService.profile` 获取 Profile。
3. 确认 action 存在于 Profile `actions`。
4. 根据 action `input` schema 校验参数。
5. 根据 `forms` 解析 `op = invokeaction` 的 endpoint。
6. 按 kRPC-style envelope POST。
7. 将 caller-neutral response 映射成 Agent-facing result。

当前 `DIDObjectClient::invoke_action_from_resolved()` 已实现 1、2、3、5、6、7 的框架：

- 从 resolved card/profile 中按 action 名称解析 endpoint。
- 构造 `ActionInvocation`，包含 `method`、`params`、`obj`、`obj_did` 和 `observed.profile`。
- POST JSON 到 action endpoint。
- 将 response 解码为 `ActionResponse`，并要求 response 恰好包含 `result` 或 `error` 之一。

当前 client/server 还没有实现完整 action input schema validation、auth/RBAC、freshness、idempotency、confirm token 或审计。

v1 default action protocol：

```text
POST {action_endpoint}
Content-Type: application/json
```

请求 envelope：

```json
{
  "method": "query_clip",
  "params": {
    "mode": "clip",
    "start_time": "2026-06-07T10:00:00Z",
    "end_time": "2026-06-07T10:10:00Z"
  },
  "obj": "https://myhome.com/devices/cam01",
  "obj_did": "did:web:myhome.com:devices:cam01",
  "observed": {
    "card_etag": "\"card-v12\"",
    "profile": "https://buckyos.org/profiles/web-camera@1",
    "profile_hash": "sha256:...",
    "object_version": "camera-state-v8",
    "observed_at": "2026-06-07T10:11:00Z"
  },
  "idempotency_key": "idem_01H...",
  "confirm_token": "confirm_01H...",
  "trace_id": "trace_01H..."
}
```

字段要求：

| 字段 | 要求 | 说明 |
|---|---|---|
| `method` | MUST | WoT action name，必须等于 Profile action 名。 |
| `params` | MUST | action 参数，必须匹配 action `input` schema。 |
| `obj` | SHOULD | Object URL。endpoint 服务多个对象时服务端必须用它定位对象。 |
| `obj_did` | MAY | Object DID，用于审计和双向确认。 |
| `observed` | MAY | 调用方执行前观察到的对象/card/profile/version，用于 freshness check。 |
| `idempotency_key` | SHOULD | 可重试或有副作用动作建议提供；Profile 可要求。 |
| `confirm_token` | MAY | 高风险动作确认 token。 |
| `trace_id` | SHOULD | 审计和跨服务 trace。 |

当前 `DIDObjectHttpServer` 在 `POST {object_path}/methods/{action_name}` 上会解析 `DIDObjectActionRequest`，并强制 `request.method == action_name`；不一致时返回 `bad_request` action error envelope。

成功返回 MUST 包含 `result`。`meta` 是 caller-neutral metadata：

```json
{
  "result": {
    "media_type": "video",
    "transport": "http-media",
    "href": "https://myhome.com/devices/cam01/clips/clip123.mp4",
    "content_type": "video/mp4",
    "realtime": false,
    "seekable": true
  },
  "meta": {
    "status": "ok",
    "summary": "Clip is ready.",
    "created_objects": [
      "https://myhome.com/devices/cam01/clips/clip123"
    ],
    "affected_objects": [
      "https://myhome.com/devices/cam01"
    ],
    "invalidated_objects": [],
    "refresh_hints": [
      "https://myhome.com/devices/cam01"
    ]
  }
}
```

错误返回 MUST 包含 `error`：

```json
{
  "error": {
    "code": "stale_object",
    "message": "Object changed since caller observed it.",
    "current_version": "offer-v4",
    "refresh_hints": [
      "https://booking.example.com/objects/stay-offer/abc123"
    ]
  }
}
```

核心协议不使用 `read_after`。`src/frame/agent-did-object-lib` MAY 把 `refresh_hints` 映射为 agent-facing `read_after`。

Runtime MUST 校验 declared capability、schema、授权和确认策略，并记录审计。Provider MUST 重新校验 auth、RBAC、object state、quota、params schema、freshness 和 idempotency。

当前 HTTP adapter 只负责 request/response envelope、endpoint method matching 和错误状态码映射。`DIDObjectServer::invoke_action(request, ctx)` 是 provider 侧实现入口，provider 必须在这里补齐参数、授权、状态和副作用检查。

高风险动作包括支付、预订、取消、删除、公开发布、授权变更和不可逆外部操作。Runtime policy 可以比 Profile 更严格；Provider 不能只依赖 caller 声称已经确认。

长期动作 SHOULD 返回 Task Object 或 Resource Descriptor，而不是阻塞到完成。

---

## 10. Event Protocol

Event 对应 WoT `events`。v1 定义订阅生命周期，不只是数据帧。

当前 `buckyos-base` 已实现 event 的 Profile affordance、包含 `delivery` 的 `x-buckyos:event` policy object、event endpoint resolution、`EventSubscribeRequest` / `EventSubscription` / `EventStatus` / `EventFrame` 数据结构，以及 `DIDObjectHttpServer` 对 `{object_path}/events` 和 `{object_path}/events/{event_name}` 的 endpoint 识别。

但当前 HTTP adapter 的默认 `DIDObjectServer::handle_event_request()` 返回 `unsupported`，并提示 “DID Object event WebSocket binding is not implemented by this HTTP adapter”。因此下述 lifecycle 是协议目标，不是当前 HTTP adapter 已完成能力。

Event endpoint MUST 支持：

```text
event.subscribe
event.renew
event.unsubscribe
event.status
event.stream
```

v1 WebSocket binding 用消息里的 `op` 表达操作。当前数据结构中 `EventSubscribeRequest.op` 保存操作名；完整 op dispatch 尚未落地。

订阅请求：

```json
{
  "op": "subscribe",
  "object": "https://myhome.com/devices/cam01",
  "object_did": "did:web:myhome.com:devices:cam01",
  "event": "low_battery",
  "filter": {},
  "ttl_ms": 300000,
  "cursor": null,
  "trace_id": "trace_01H..."
}
```

订阅响应：

```json
{
  "type": "subscription",
  "subscription_id": "sub_01H...",
  "object": "https://myhome.com/devices/cam01",
  "object_did": "did:web:myhome.com:devices:cam01",
  "event": "low_battery",
  "expires_at": "2026-06-07T13:00:00Z",
  "cursor": "42",
  "delivery": "best_effort",
  "refresh_hints": [
    "https://myhome.com/devices/cam01"
  ]
}
```

EventFrame：

```json
{
  "type": "event",
  "event_id": "evt_01H...",
  "subscription_id": "sub_01H...",
  "object": "https://myhome.com/devices/cam01",
  "object_did": "did:web:myhome.com:devices:cam01",
  "event": "low_battery",
  "seq": 42,
  "cursor": "42",
  "timestamp": "2026-06-07T12:00:00Z",
  "summary": "Camera battery is low.",
  "data": {
    "battery": 12
  },
  "affected_objects": [
    "https://myhome.com/devices/cam01"
  ],
  "invalidated_objects": [],
  "refresh_hints": [
    "https://myhome.com/devices/cam01"
  ]
}
```

Subscription MUST 有 `expires_at`。Runtime SHOULD 在过期前 renew。Provider MAY 拒绝过长 TTL。订阅过期后 Provider SHOULD 停止发送事件并释放资源。

v1 Event delivery 只要求 `best_effort`。Event 是对象状态刷新和 session wakeup 的 accelerator，不是 MQ；协议不要求 Provider 或 Adapter 维护 backlog、ack 状态、重投队列、严格顺序或跨重启补齐。支持 cursor resume 时，Provider MAY 允许 subscribe 携带 `cursor`，但 cursor 只表示尽力恢复和丢失检测 hint，不能被上层当作 MQ offset。cursor 失效或检测到可能丢失事件时，Provider SHOULD 返回 `cursor_expired` error 和 `refresh_hints`，提示 Runtime 重新读取对象状态。

`at_least_once`、`durable` 或 MQ-style replay 不属于 v1 Event Bridge 的默认语义。如果后续需要可靠事件，应作为独立 durable event / MQ 能力设计，而不是要求普通 DID Object Adapter 默认实现。

当前 `ObjectProfile::event_endpoint()` 已能把 event form 的 `href` 或默认 `events` endpoint 解析为 `ws://` / `wss://` URL，但还没有 event subscribe client。

### 10.1 KEvent bridge

远端 Provider 不直接控制本地 KEvent。本地 Object Event Runtime 接收远端 EventFrame 后，MAY 发布到本地 KEvent 用于 fanout / wakeup。KEvent bridge 不提供 durable delivery；EventFrame 丢失后的恢复路径是根据 `refresh_hints` / `invalidated_objects` 重新读取对象状态，而不是 replay 事件流。

不要把 Object URL 原样塞入 KEvent event id。Runtime 应编码成合法 path：

```text
/obj/<host>/<kind>/<id>/<event>
```

示例：

```text
/obj/booking.example.com/stay_offer/abc123/changed
/obj/booking.example.com/stay_offer/by_hash/sha256_xxx/changed
```

payload 保留原始 Object URL 和 DID。

---

## 11. Resource Descriptors

DID Object Protocol 不重新定义媒体、文件、任务或 stream 的传输协议。Action 可以返回薄 Resource Descriptor 指向外部资源。

当前测试 server 的 `query_clip` action 已返回 media resource descriptor 风格的 JSON：`media_type`、`transport`、`href`、`content_type`、`realtime`、`seekable`。这只是 action `result` 的约定形状；当前实现没有单独的 Resource Descriptor 类型校验器。

Media Resource Descriptor：

```json
{
  "media_type": "video|audio|image",
  "transport": "http-media|hls|dash|webrtc-whep",
  "href": "string",
  "content_type": "string",
  "realtime": "boolean",
  "seekable": "boolean",
  "expires_at": "string?"
}
```

Object Resource Descriptor：

```json
{
  "object": "https://booking.example.com/objects/reservation/r789",
  "object_did": "did:web:booking.example.com:objects:reservation:r789",
  "profile": "https://booking.example.com/profiles/stay-reservation@1",
  "kind": "stay.reservation"
}
```

`src/frame/agent-did-object-lib` 可以把 Resource Descriptor 渲染为可读摘要、后续 `read()` 入口或 LLM 可引用对象，但不改变底层协议。

---

## 12. src/frame/agent-did-object-lib 的位置

DID Object Protocol 不定义 `read()`。`src/frame/agent-did-object-lib` 可以使用协议材料构造 LLM-facing ReadResult，例如：

- Gateway canonicalization -> Object URL。
- DID Object Card -> identity / controller / source trust。
- Profile -> properties/actions/events/traits。
- Property values -> object state。
- IndexPage -> table / collection rendering。
- Action meta -> affected objects / refresh hints。
- EventFrame -> invalidation / wakeup / subscription summary。
- ETag / schema_hash / row version -> session-aware compression。

但 `read()` 输出还受以下因素影响：

- input route。
- adapter chain。
- user session。
- cache state。
- authorization policy。
- token budget。
- task purpose。
- LLM-facing rendering strategy。

因此本文不再规定统一 `ReadResult` 结构。可以在 `src/frame/agent-did-object-lib` 内部保留某种 ReadResult，但它是 Agent-facing rendering contract，不是 DID Object Protocol contract。

`src/frame/agent-did-object-lib` 的 session compression 规则应建立在协议层 metadata 上：

```text
same object + same version => likely no need to re-fetch/render
same schema_hash => collection shape unchanged
same snapshot_id + cursor => stable pagination
```

这些规则可以产生 `known_in_session`、`not_changed`、`read_after` 等 Agent-facing 表达，但不应反向写入核心协议。

---

## 13. Security / Policy / Trust

Runtime MUST：

- 验证 DID Object Card 来源。
- 检查 DID Document 签名、controller 或 Zone 信任链。
- 确认 Object URL 与 DID Object Card `alsoKnownAs` / DID resolution 一致。
- 只允许访问 Profile 中声明的 property/action/event。
- 对 action input 做 schema validation。
- 对 event subscription 做权限检查。
- 对高风险 action 做确认。
- 对 action、property、event subscription 和 event delivery 记录审计。

Provider MUST：

- 不信任 caller 的客户端校验结果。
- 重新校验 auth、RBAC、object state 和 quota。
- 对长生命周期 subscription 设置 TTL。
- 对跨对象或外部副作用操作进行额外 policy 检查。

审计字段 SHOULD 至少包含：

```text
who
when
object_url
object_did
property/action/event
endpoint
trace_id
source
confirmation
result
```

HTTP host 只能作为发现入口。强验证场景下，必须把信任回溯到 DID Document 的签名者、controller、Owner 或 Zone。

Resolver 或 Runtime MAY 为 DID Object Card / endpoint response 附加 trust metadata：

```json
{
  "source": {
    "type": "object_host|zone_registry|local_registry|did_resolver|cache|adapter",
    "uri": "https://myhome.com/devices/cam01/did.json",
    "trust": "official|verified|zone|local|unverified|cached|stale",
    "fetched_at": "2026-06-07T12:00:00Z",
    "valid_until": "2026-06-07T13:00:00Z"
  }
}
```

---

## 14. DID Object Host and Resolver Routes

核心协议只要求 Runtime 能从 Object URL / DID 得到 verified DID Object Card。resolver 算法是实现细节。

默认 route：

```text
GET {object_url}/did.json
```

原生 DID Object Host 可以暴露 host-level manifest：

```text
GET https://booking.example.com/.well-known/did-object.json
```

示例：

```json
{
  "protocol": "did-object/1",
  "object_hosts": ["booking.example.com"],
  "profiles": [
    "https://booking.example.com/profiles/stay-search@1",
    "https://booking.example.com/profiles/stay-offer@1",
    "https://booking.example.com/profiles/stay-reservation@1"
  ],
  "auth": ["oauth2", "session_delegation"],
  "url_claims": ["https://www.booking.example.com/*"]
}
```

该 manifest 是 resolver route 的输入，不替代 DID Object Card。每个 DID Object 仍应能解析到自己的 DID Object Card。

---

## 15. Agent Result Compatibility Profile

本节非核心强约束。Provider SHOULD 在 action response `meta` 中使用 caller-neutral 字段：

```json
{
  "meta": {
    "status": "ok|accepted|no_change",
    "summary": "Human-readable short summary.",
    "created_objects": [],
    "affected_objects": [],
    "invalidated_objects": [],
    "refresh_hints": []
  }
}
```

`src/frame/agent-did-object-lib` MAY 映射为：

| Protocol field | Agent-facing meaning |
|---|---|
| `meta.summary` | tool result summary |
| `created_objects` | returned object refs |
| `affected_objects` | objects changed by action/event |
| `invalidated_objects` | cached/read views to invalidate |
| `refresh_hints` | possible `read_after` targets |
| error `code = stale_object` | `status = stale` |
| confirmation error / policy | `status = needs_confirm` |

Profile MAY 声明 `x-buckyos:agentResult` hints，但这些 hints 不影响普通 caller 的协议兼容性。

---

## 16. Implementation Phases

### Phase 1: DID Object Card and Profile

- Object URL -> DID Object Card resolver。
- DID Document verification。
- `DIDObjectService` parsing。
- Profile fetch and cache。
- endpoint resolution rules。

### Phase 2: Property and Action

- property GET。
- action POST kRPC-style envelope。
- schema validation。
- trace_id / audit。
- ETag / conditional access。
- error envelope。

### Phase 3: Event Lifecycle

- event declaration parsing。
- WebSocket v1 binding。
- subscribe / renew / unsubscribe / status。
- lease / expires_at。
- EventFrame with event_id / seq / cursor。
- KEvent bridge。

### Phase 4: Traits and IndexTrait

- `x-buckyos:traits` parsing。
- IndexTrait contract。
- `index_schema` property。
- `query` and `page` actions。
- IndexPage with schema_hash / snapshot_id / row version。

### Phase 5: Agent Compatibility

- action result `meta` compatibility fields。
- event `refresh_hints`。
- stale / confirmation mapping。
- optional `x-buckyos:agentResult` hints。

### Phase 6: Additional resolver routes

- local registry。
- Zone registry。
- BNS。
- host manifest。
- DID Object Host policy and auth integration。

---

## 17. Open Questions

- `xcall` 的 confirm token 由 Runtime 统一签发、Provider 签发，还是两者都支持。
- Agent-facing ReadResult 是否需要单独文档，而不是混在 DID Object Protocol 中。
- Event durable delivery 是否进入 v1.1，还是 v2。
- IndexTrait 是否需要标准 `sort` / `filter` expression language，还是只规定 schema shape。
- TaskTrait 是否需要与 long-running action semantics 一起标准化。
- MCP adapter 暴露 DID Object 能力时，是否需要标准工具命名约定。

---

## 18. 当前结论摘要

- DID Object Protocol 是封闭世界协议，不定义开放世界 `read()`。
- Object URL 是协议层对象引用；DID Object Card 是协议层对象视图。
- DID Object Card 使用 DID Document；DID Object Profile 使用 constrained WoT TD。
- Action Invocation 是 caller-neutral 协议；Agent `xcall` 是上层包装。
- Event lifecycle 是协议语义，必须支持 lease / renew / unsubscribe / status / stream。
- Indexer 是实现 IndexTrait 的普通 DID Object，而不是单独 tool。
- Session-aware context management 不进入核心协议，但协议必须提供稳定 object/version/schema/event metadata。
- `read_after` 不进入核心协议；使用 `refresh_hints` / `affected_objects` / `invalidated_objects`，由 `src/frame/agent-did-object-lib` 自行映射。
