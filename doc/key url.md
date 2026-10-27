# key url Zone提供的一些重要的路径

这里列出的是，会被外链的，需要按协议级维护的URL。
本文档有两部分：

1. **URL 规范** —— 系统里所有 URL 必须遵守的协议（scheme）约定与缩写展开规则。
2. **稳定 URL 清单** —— 会被反向使用（外链）、系统承诺协议级稳定的具体 URL。

---

# 一、URL 规范

系统里所有的 url 都必须是合法的 url，主要是下面几种 scheme：

| scheme | 说明 |
| --- | --- |
| `https://` | 标准的 http 链接。注意 cyfs 的 R-Link 也可以在这里表达。 |
| `file://`  | 标准。支持 `file:///local_path`、`file://localhost/local_path`、`file://$device_id/path`。 |
| `cyfs://`  | 我们的扩展，表示传统文件系统视角下的 CYFS 逻辑路径。 |
| `obj://`   | 我们的扩展，表示 Agent 所见的 DID Object 世界。 |
| `buckyos://` | 我们的扩展，用来拉起 current zone / buckyos app 的特定流程。 |

## 缩写路径的展开

为了书写方便，系统里允许用缩写路径表达上面的 url，展开规则如下：

| 缩写 | 展开 | 含义 |
| --- | --- | --- |
| `/local/path` | `file:///local/path` | 本地 FS 路径 |
| `//config/nodes/node1/config` | `obj://config/nodes/node1/config` | NamedObject 路径 |

即：
- 以单个 `/` 开头 → `file://` 本地路径。
- 以 `//` 开头 → `obj://` 命名对象路径。

## Current Zone 的 CYFS 路径

`cyfs:///path` 与 `cyfs://_/path` 都表示当前 Zone 下的 CYFS 逻辑路径，解析后等价于
`cyfs://$current_zone/path`。文档中的当前 Zone 路径优先使用 `cyfs:///path`；只有在需要明确
跨 Zone 定位时，才在 authority 中写出具体的 Zone。

> sys_config_service 已实现这一展开/归一化：`obj://config/`、`/config/`、前导 `/` 会被剥离得到归一化 key，反向也会把裸 key 补全为 `obj://config/...`。参见 [src/kernel/sys_config_service/src/main.rs](src/kernel/sys_config_service/src/main.rs)（`strip_config_key_prefix` / `get_full_res_path`）。

## 我们定义的服务名

`obj://$service` 形式，host 段是系统约定的服务名：

```
obj://config      # 系统配置 KV (system-config)
obj://dfs         # 面向 Agent 的分布式文件系统服务对象，不是文件路径
obj://taskmgr     # 任务管理器 (task-manager)
obj://kmsg        # 消息队列服务 (kmsg)
obj://bucket      # 应用对象桶 (nfs-server)：obj://bucket/<bucket_id>/<key>
```

`obj://bucket/<scope>/<appid>/<owner>/<name>/<key>` 指向某个桶里 key 的当前绑定（zone 桶的 owner 写 `_`），
是 Agent、CLI（`buckyos bucket get`）和服务持久引用一个对象时的标准写法；它本身不带读权，读取按桶的授权或
`bkt_` 令牌判定。设计见 [应用对象桶 Bucket 设计](arch/应用对象桶%20Bucket%20设计.md) §4.2；
Rust `buckyos_api::bucket_obj_url` / `parse_bucket_obj_url`，websdk `bucket.bucketObjUrl` / `parseBucketObjUrl`。

## 带类型的实体 id

`obj://$entity_id` 鼓励各种 id 带类型前缀，方便路由层识别并找到归属服务，例如：

```
obj://task_xxxx/   # 系统可识别这是一个 taskid，等价于 obj://taskmgr/$taskid
```

## 在 RBAC 中使用 url 定义权限

RBAC（Casbin ABAC）的 policy 用上面的 url/缩写路径来表达受控资源。`p` 规则形如 `p, <subject>, <resource>, <action>, <effect>`，resource 段就是这些 url：

```
p, root,  obj://config/*,                              read|write, allow
p, ood,   obj://config/nodes/{device}/*,               read|write, allow
p, app,   obj://config/users/*/apps/{app}/settings,    read|write, allow
p, users, obj://config/users/{users}/*,                read,       allow
p, users, obj://config/users/{users}/profile,          read|write, allow
p, users, cyfs:///users/{users}/*,                       read|write, allow
```

`{users}` / `{app}` / `{device}` / `{service}` 为 enforce 时匹配的占位通配。policy 存于 system-config 的 `system/rbac/policy`，参见 [src/rootfs/etc/scheduler/boot.template.toml](src/rootfs/etc/scheduler/boot.template.toml)。

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


### 用户（实体）profile

https://$zonehost/profile?id=xxxx
https://test.buckyos.io/userprofile?user=devtest （现在情况）

### 给Zone投递NamedObject (sendmsg)

## 用户首页（和用户的默认app有关）

https://$username.$zonehost/

用户的username是无法改变的，能修改的是nickname/show name/fullname这类

## app url

root用户安装的app

https://$appid.$zonehost/ 


为特定用户安装的app

https://$appid-$userid.$zonehost/


## 钱包相关协议


