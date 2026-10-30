# key url Zone提供的一些重要的路径

这里列出的是，会被外链的，需要按协议级维护的URL。
本文档有两部分：

1. **URL 规范** —— 系统里所有 URL 必须遵守的协议（scheme）约定、host 段规则、缩写展开和转换边界。这是唯一规范源：Prompt、SDK、网关、Agent 工具和 RBAC 都引用这里，不各自另猜一套。
2. **稳定 URL 清单** —— 会被反向使用（外链）、系统承诺协议级稳定的具体 URL。

设计裁决记录见 [notepads/agent-object-url-todo.md](../notepads/agent-object-url-todo.md) §1.5（D1–D12）；本文是其定稿结果，不再重复讨论过程。

---

# 一、URL 规范

## 1. 总原则

- 系统里所有的 url 都必须是合法的 URL（RFC 3986）。路径段按 URL 规则编码，大小写、空格、`#`、`?` 不能裸写。
- **URI 标识资源，不保证当前工具具备访问能力，也不自带授权。**"URI 可解析""工具实现了该协议""调用者获准访问"分别检查；不支持或无权限时返回明确错误，不换一个同形本地路径碰运气。
- **按资源语义选 scheme，不按落盘方式。** 能在 Files / 传统文件系统里直接修改同一资源的，用 `cyfs://` 路径；只能通过业务 action 修改的（会话、Feed、工作区实体、Task），用 `obj://`。SQLite 文件有 CYFS 路径，不等于其中的业务对象有 CYFS 路径。
- 一个资源可以有多个 URL，但**每个对象必须有一个明确的默认 URL**（见 §6）。授权与去重不随别名拼写改变。

## 2. scheme 一览

| scheme | 说明 |
| --- | --- |
| `https://` | 标准的 http 链接。BuckyOS public 服务提供稳定 HTTPS URL；**跨 Zone 引用服务对象、跨 Zone 投递 NamedObject 一律用 https**（§4）。cyfs 的 R-Link 也可以在这里表达。 |
| `file://` | 标准（RFC 8089）。host 段是 **device id**：`file://<device_id>/abs/path`；空 host 与 `localhost` 按 §3 解释。 |
| `cyfs://` | 我们的扩展。路径型 `cyfs://<zone>/<path>` 表示传统文件系统视角下的 CYFS 逻辑路径；ObjId 型 `cyfs://<objid>[.<zone>]/<innerpath>` 表示内容寻址的 NamedObject。 |
| `obj://` | 我们的扩展，表示 Agent 所见的 DID Object 世界。运行在 BuckyOS 上的信息都可以用 obj:// 表达（最大兼容），但 host 段只允许 §4 规定的两类值。 |
| `buckyos://` | 我们的扩展，用来拉起 current zone / buckyos app 的特定流程。 |

## 3. `file://`：设备文件

```text
file://<device_id>/abs/path     指定设备上的文件
file:///abs/path                空 host
file://localhost/abs/path       等价于空 host
```

- **device id 是唯一的 host 形式。** 由 node_daemon 管理的设备有稳定 device id（如 `ood1`）；通过 buckyos-agent-daemon 临时加入的执行环境也必须构造稳定、用户可读的 device id 并注册，使 `file://<device_id>/...` 可解析到对应 node / executor。不用 Docker 容器名，不用 Agent DID 充当文件 host。
- **空 host 的解释顺序**：
  1. 有来源 runtime 上下文（Agent 工具、SSH / node_executor 工具的返回值）：属于产生它的 runtime，按该 runtime 的 cwd / 挂载解释。
  2. 没有来源上下文（用户输入、持久化引用、跨 Agent 传递）：按 Zone 的 **default device** 解释。
- **default device 是 Zone 级设置**，指向 cluster 中某一设备（Desktop 常见值为 ood1，只是现状不是规则）。它只用于解释已经写成 `file:///path` 的输入；相对路径、`/tmp/x` 这类裸路径由来源 runtime + cwd 推导，不走 default device。
- 跨上下文输出（发消息、写 Hint / Memory / Notebook、交给另一个 Agent）前，把空 host 补成显式 device id；经真实挂载确认位于 DFS 映射目录的文件，改用 `cyfs://` 引用（§5）。容器临时文件或镜像层文件不能自动转 CYFS。

## 4. `obj://`：DID Object 引用

```text
obj://<service>/<inner-path>       current zone 的服务对象
obj://<entity-id>/<inner-path>     实体对象，entity-id 自带 Zone
```

**host 段只允许两类值，其余一律报错：**

1. **系统约定的服务名**（表见 §4.1）。它直接表示 **current zone** 的该服务，**不展开**：`obj://config/users/alice/profile` 就是当前 Zone 的 config 服务，不存在 `obj://config.alice.bns.did/...` 这种展开形式。服务名不带 Zone，所以 `obj://<service>/` 只在 current zone 内有意义。
2. **带 Zone 的实体 id**：有明确 `did:bns:<id>.<zone>` / `did:web:<host>` 身份的实体，host 写其 DID 的 canonical hostname（`DID::to_host_name`），例如 Agent `obj://xiaobai.test.buckyos.io/sessions/<sid>`、群组 `obj://<group-did-host>/...`。id 自带 Zone，可以跨 Zone 解析。

不允许的写法：类型简写（`obj://task_xxxx/`），一律写完整服务路径 `obj://taskmgr/task_xxxx`；NamedObject id 进 host（`obj://cyfile:<hash>/`，冒号与端口语法冲突），内容对象用 `cyfs://<objid>/`；原始 `did:...` 字符串进 host。

**跨 Zone 引用服务对象用 https。** 另一个 Zone 的 config / msg-center / homestation 对象，用该 Zone 发布的 https Object URL（§6），不用 `obj://<service>/` 加 Zone 后缀。

`obj://` 是 resolver 的输入，不是 wire endpoint：真正的 HTTP 地址由服务的 Provider route 或 DID Object Card / Profile 决定，不从 URL 字符串推（见 buckyos-base `doc/did-object-protocol.md` §5.4）。

当前 Provider route（2026-10-29 实现，`buckyos_api::obj_provider_path`）：`obj://<host>/<path>` → `https://<zone>/kapi/<host>/obj/<path>`，路径型 Object URL，GET 直接返回 Card，`?type=profile` 返回 Profile。已接入：`msg-center`、`aiworkspace`、`homestation`、`dfs`（网关把 `/kapi/dfs/*` 转给 nfs-server）、Agent 实体（`/kapi/<agent_id>/obj`，由该 Agent 的 OpenDAN 承接）。Provider 的属性 / 索引查询就是服务已有 kRPC 方法，以调用者自己的会话执行，权限与 kRPC 一致。

语法与规范化的实现：Rust `buckyos_api::object_url`（`parse_object_url`、`expand_input_shorthand`、`rbac_resource_key`、`object_did`），websdk `objectUrl`（同名 camelCase），两端共用 `src/kernel/buckyos-api/testdata/object_url_vectors.json`（websdk `tests/fixtures/` 同一份副本）；CLI `buckyos obj resolve|read`。

### 4.1 系统约定的服务名

| host | 服务 | 路径约定 |
| --- | --- | --- |
| `config` | 系统配置 KV（system-config） | `obj://config/<key_path>`，与 system-config 的 key 一一对应 |
| `dfs` | 面向 Agent 的分布式文件系统服务对象，不是文件路径 | `obj://dfs/<path>` 描述与 `cyfs:///<path>` 同一文件的对象能力 |
| `taskmgr` | 任务管理器（task-manager） | `obj://taskmgr/<task_id>` |
| `kmsg` | 消息队列服务（kmsg） | `obj://kmsg/<queue_id>` |
| `bucket` | 应用对象桶（nfs-server） | `obj://bucket/<scope>/<appid>/<owner>/<name>/<key>`（zone 桶的 owner 写 `_`），指向某个桶里 key 的当前绑定。设计见 [应用对象桶 Bucket 设计](arch/应用对象桶%20Bucket%20设计.md) §4.2；Rust `buckyos_api::bucket_obj_url` / `parse_bucket_obj_url`，websdk `bucket.bucketObjUrl` / `parseBucketObjUrl` |
| `ndm` | NamedData 管理（nfs-server 内） | RBAC 资源 `obj://ndm/{uploads,objects,retain,manage}` |
| `msg-center` | Message Center | `obj://msg-center/owners/<owner_did>/sessions[/<sid>]`、`.../owners/<owner_did>/contacts[/<did>]`、`obj://msg-center/messages/<msg_id>`；`inbox / sent / group_inbox / request_box / objects / notify / contact_tags / contact_membership` 是 RBAC 资源 key，不是对象路径 |
| `aiworkspace` | AI Workspace | `obj://aiworkspace/<ws_id>`、`.../entities/<entity_id>`、`.../path/<...>` |
| `homestation` | HomeStation | `obj://homestation/home/<user>`、`/feed`、`/feed/@/<key>`、`/inbox`、`obj://homestation/home/~zone` |
| `opendan` | OpenDAN Agent Loader | `obj://opendan/` 只做发现（列本 Zone 托管的 Agent）；Agent 对象本身用 `obj://<agent-id>/...`（§4.2） |
| `aicc` | AICC | RBAC 资源 `obj://aicc/on_behalf_of` 等 |

新增服务名在此表登记；未登记的服务名 resolver 报错。

### 4.2 实体对象：Agent

Agent 的对象以 agent-id 为 authority（agent-id 即 Agent DID 的 canonical hostname，形如 `xiaobai.test.buckyos.io`）：

```text
obj://<agent-id>                    Agent 根对象
obj://<agent-id>/sessions/<sid>
obj://<agent-id>/workspaces/<ws-id>
obj://<agent-id>/goals/<gid>
obj://<agent-id>/artifacts/<aid>
```

AgentRoot 目录是 `cyfs://<zone>/home/<owner>/agents/<agent-id>/`，Agent 对象及 Session 对象不是该目录的别名。`agent://self` 已废弃。

## 5. `cyfs://`：文件系统路径与内容寻址

```text
cyfs:///path            current zone 的 CYFS 逻辑路径（等价 cyfs://_/path）
cyfs://<zone>/path      指定 Zone 的 CYFS 逻辑路径
cyfs://<objid>/inner    NamedObject 内容寻址；<objid> 用小写、无 padding 的 base32（hostname 安全）
cyfs://<objid>.<zone>/inner   同上，并给出内容来源 Zone 线索
```

- **路径型的判据**：非 ObjId 路径必须能在 Files / 传统文件系统中定位，并接受传统文件操作直接修改同一资源。只提供只读展示、修改仍要调业务 API 的对象不满足，应使用 `obj://`。"可以修改"描述资源契约，不表示所有调用者都有写权限。
- `cyfs:///path` 与 `cyfs://_/path` 都表示当前 **Zone**（不是当前物理节点）；文档里优先写 `cyfs:///path`，需要跨 Zone 定位时才写出 Zone。
- `cyfs:///xxxx` 的 xxxx 是路径；`cyfs://xxxx` 的 xxxx 是 authority。是否为 ObjId 用严格编解码判定，不靠"看起来像 hash"。
- ObjId 指向确定内容，编辑产生新 ObjId；可变路径和它的某个 ObjId 快照只能声明版本关联，不能声明永久等价。
- `$BUCKYOS_ROOT/data/<path>` 对应 `cyfs:///<path>`，映射按真实挂载与规范路径做（检查路径段边界、`..`、符号链接、容器→宿主映射），不是字符串删前缀。NFS 当前只导出启动时的非点号一级目录，未导出的路径是"访问能力缺失"，不是 URI 非法。
- `cyfs:///srv/publish/<path>` 对应 `https://public.<zone>/<path>`（已有 public 发布映射）。

## 6. 默认 URL、别名与跨 Zone

- **每个 DID Object 有一个默认 URL**：Card 的 `id` 对应的 Object URL。其余引用（本 Zone 的 `obj://` 简写、带 selector 的视图、发布页）在 Card 的 `alsoKnownAs` 中声明，且只列同一对象的引用；附件、导出文件、某个版本快照不是对象本身，不进 `alsoKnownAs`。
- **跨 Zone 的默认 URL 是 https**：`https://<zone-host>/<path>`。服务对象的本 Zone 形式 `obj://<service>/<path>` 与其 https 默认 URL 的对应关系由各服务的 Provider 声明（如 HomeStation `obj://homestation/home/alice/feed` ↔ `https://<zone>/home/alice/feed`）。
- **跨 Zone 投递 NamedObject 的 target 也是 https**：`https://<zone-host>/<path>`，由 cyfs-ndn 的 dispatch helper 统一产出，msg-center 群组、HomeStation、cyfs-gateway 共用同一格式。
- **版本 / selector / 视图用 query**：path 定位对象或子对象；`version`、`selector`、`mode` 等已声明的选择器放在 query。`selector` 等 JSON 值进 query 前做规范化序列化（键排序、无空白），缓存键与签名用规范化后的串。快照引用与 live 引用分开，不混用。

## 7. 对象事件的 kevent 命名

对象事件的 kevent key 统一为对象路径形式，发布者与订阅者使用同一规则，不再各服务自定前缀：

```text
/obj/<host>/<inner-path...>/<event>
示例：
  /obj/msg-center/owners/<owner>/sessions/<sid>/changed
  /obj/aiworkspace/<ws_id>/head
  /obj/<agent-id>/sessions/<sid>/finished
```

- `<host>` 与 `obj://` 的 host 段同值；kevent 段名只允许字母、数字、`_`、`-`，含 `.`、`:`、`@` 等字符的段用小写 hex 编码（`x` 前缀）。以 `x` 开头的段即使字符合法也编码，保证编码段与原样段不会相撞。段按解码后的路径段编码（`did:bns:alice` → `x6469643a626e733a616c696365`）。
- kevent 限制（总长 ≤ 256、深度 ≤ 8）放不下时由发布方落到上一级对象（如 msg-center 过长的 session 落到 `.../sessions/<box>`），payload 带完整对象信息。
- 实现：Rust `buckyos_api::{obj_event_key, obj_event_pattern, ObjectUrl::object_event_key}`，websdk `objectUrl.{objEventKey, objEventPattern, objectEventKey}`。
- Profile 声明的事件名就是最后一段 `<event>`，与 kevent key 一一对应，不再需要 bridge 映射表。
- 首期只做本 Zone 的事件；远端订阅能力按实际实现报告。

## 8. 缩写路径的展开

为了书写方便，声明采用此语法的输入入口（CLI、Prompt 环境）允许用缩写路径表达上面的 url，展开规则如下；不改变 shell / UNC 对 `//` 的原有解释：

| 缩写 | 展开 | 含义 |
| --- | --- | --- |
| `/local/path` | `file:///local/path` | 本地 FS 路径（空 host，按 §3 解释） |
| `//config/nodes/node1/config` | `obj://config/nodes/node1/config` | current zone 的服务对象 |

> sys_config_service 已实现 `obj://config/` 的归一化：`obj://config/`、`/config/`、前导 `/` 会被剥离得到归一化 key，反向也会把裸 key 补全为 `obj://config/...`。参见 [src/kernel/sys_config_service/src/main.rs](../src/kernel/sys_config_service/src/main.rs)（`strip_config_key_prefix` / `get_full_res_path`）。

## 9. 不是无条件等价的转换

- 任意 `https://` 换 scheme 变成 `cyfs://` / `obj://`：只有发布映射或 Card / 注册表确认是同一资源时才能转换；普通网页、业务首页、附件正文与业务对象不能互相冒充。
- 业务 Feed / Session / 联系人转 DFS 路径：业务对象不因"底层落盘"获得 CYFS 路径。
- 可变路径转固定 ObjId：丢失版本语义，只能声明版本关联。
- 原始 `did:...` 或 `cyfile:...` 直接塞进 host。
- 转换时丢弃 selector、version，或改变授权范围。
- 换 scheme / host 简写扩大权限：同一资源在 RBAC 中只有一个资源 key（见 §10）。

## 10. 在 RBAC 中使用 url 定义权限

RBAC（Casbin ABAC）的 policy 用上面的 url 来表达受控资源。`p` 规则形如 `p, <subject>, <resource>, <action>, <effect>`，resource 段就是这些 url：

```text
p, root,  obj://config/*,                              read|write, allow
p, ood,   obj://config/nodes/{device}/*,               read|write, allow
p, app,   obj://config/users/*/apps/{app}/settings,    read|write, allow
p, users, obj://config/users/{users}/*,                read,       allow
p, users, obj://config/users/{users}/profile,          read|write, allow
p, users, obj://dfs/users/{users}/*,                    read|write, allow
```

`{users}` / `{app}` / `{device}` / `{service}` 为 enforce 时匹配的占位通配。policy 存于 system-config 的 `system/rbac/policy`，参见 [src/rootfs/etc/scheduler/boot.template.toml](../src/rootfs/etc/scheduler/boot.template.toml)。

资源 key 的布局规则：对象路径与 RBAC 资源 key 同形（msg-center 用 `owners/<owner_did>/...` 前缀）；别名归一化在 enforce 之前完成，归一化后的资源 key 唯一。各服务的资源 key 表、agent 组的默认权限、Profile 的 `x-buckyos:rbac / risk` 与 RBAC action 的对应表见 [BuckyOS RBAC权限设计](arch/verify-hub/BuckyOS%20RBAC权限设计.md)"基于 URL 的资源 key 布局"一节。

---

# 二、稳定 URL 清单

这里列出的是，会被外链的，需要按协议级维护的 URL。

## 可以匿名访问的

### Zone级别的公开内容


http://public.$zone_hostname/xxxx -> 映射到 data/srv/publish




### Zone内的公开实体的DID Document (json链接)

- 域名是合法did时，获取对应的did

- 查询任意did(原则上只包含zone内实体)



### Desktop中定义的公共URL

- 用户邀请链接
- share_content相关页面 （实体内容是如何引用的？）


### zone内的 ndn 标准路径
http://$zone_hostname/ndn/$chunkid (GET | HEAD | PUT/PATCH)

### 应用对象桶的数据面
https://$zone_hostname/bucket/v1/$bucket_id/$key (GET | HEAD | PUT | DELETE)

`obj://bucket/$bucket_id/$key` 的浏览器形式。同源页面带 SSO cookie 即可放进 `<img src>`；跨主体读取带
`?token=bkt_...`（`bucket.create_access_token` 签发的单 key 只读令牌）；`?version=N` 读历史版本。
Desktop 公共入口是 `components/preview/bucketSource.ts` 的 `bucketObjectUrl()`。

### HomeStation 的跨 Zone 对象 URL

```text
https://$zone_hostname/home/$user            主页（默认 Object URL）
https://$zone_hostname/home/$user/feed       Feed
https://$zone_hostname/home/$user/feed/@/$key   可变条目（entry，进入签名与 Follow）
https://$zone_hostname/home/$user/inbox      投递入口（dispatch target）
https://$zone_hostname/home/~zone            Zone 公共流
```

本 Zone 内对应 `obj://homestation/home/$user/...`。旧的 `cyfs://$zone/home/...` 写法已废弃。

### 用户（实体）profile

https://$zonehost/profile?id=xxxx
https://test.buckyos.io/userprofile?user=devtest （现在情况）

### 给Zone投递NamedObject (sendmsg)

投递 target 统一为 `https://$zone_hostname/<path>`（见第一部分 §6），由 cyfs-ndn 的 dispatch helper 产出。

## 用户首页（和用户的默认app有关）

https://$username.$zonehost/

用户的username是无法改变的，能修改的是nickname/show name/fullname这类

## app url

root用户安装的app

https://$appid.$zonehost/ 


为特定用户安装的app

https://$appid-$userid.$zonehost/


## 钱包相关协议
