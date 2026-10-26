# BuckyOS 应用对象桶（Bucket）设计

> 版本：v0.6 设计定稿，D1–D20 全部定案（修订记录见 §19；v0.6 按内容网络视角把数据策略重排为"自己创作 / 别人创作但认真收集 / 别人创作最好能有"三档，新增 `collected` 与 `ambient`，并入原 `archive`）  
> 日期：2026-10-26  
> 上游文档：[buckyos 数据落盘架构方向](./buckyos%20数据落盘架构方向.md) §6.3–§6.6、§9.3、§10.4  
> 底层契约：cyfs-ndn `doc/GC/named_store_gc.md`（三档存活、pins、outbox）、`doc/NDM Protocol/*`（tus 上传、store JSON API）、`doc/CYFS Protocol/CYFS 标准对象.md`（FileObject / DirObject / ChunkList）  
> 性质：本文把落盘方向文档里"待设计"的 Bucket 收敛成可实施的中间件设计。标注 **【已定】** 的是已拍板的判断（全部裁决见 §18）；标注 **【现状】** 的是当前代码事实。

## 0. 一句话

**Bucket 是应用对 named store 的"可变命名空间 + GC 根"：`key → ObjId` 的强一致索引，每一条绑定就是一条保留承诺（pin），内容本体永远只在 named store 里存一份。**

它对应用呈现为 S3 风格的对象桶（PUT/GET/LIST/DELETE、前缀列举、条件写、分段上传、生命周期），对系统呈现为可统计、可备份、可回收的应用数据资源，和 RDB Instance 并列。

## 1. 问题与目标

### 1.1 要解决的问题

落盘方向文档已经把方向定了：原生应用的非结构化数据直接进 Named Data，不经文件系统。但只有 ObjId / ChunkId 时缺四样东西：

| 缺口 | 现状 | 后果 |
| --- | --- | --- |
| 归属 | named store 是内容寻址缓存，对象不属于任何人；pin owner 是自由字符串 | 系统答不出"这个 App 保留了多少数据" |
| 名字 | 应用只能把 ObjId 存进 RDB，再无别的入口 | 浏览器 `<img src>`、Agent 工具、备份都需要先查库 |
| 保留 | HomeStation、msg-center 各自写了一套 pending→held→releasing 保留表 | 每个新业务重复实现，且各自的坑（事务自锁、补齐）重复踩 |
| 生命周期 | 对象"只增不减"是现状不是承诺；chunk 从不回收 | 个人设备容量有限，违反"每次写入都要有删除机会" |

### 1.2 目标

1. 给应用一个**自描述的数据资源**：和 `rdb_instances` 一样在安装配置里声明，系统能列出、统计、备份、回收。
2. **一份内容、多处引用**：同一 ObjId 被多个 Bucket 绑定不复制字节；跨应用分享是 O(1) 的元数据操作。
3. **保留语义机械化**：绑定即保留，解绑即释放，应用不再自己写 pin 表；底层 GC 只看 pin。
4. **上传入口直达**：浏览器 / SDK 上传时指明 `bucket + key`，完成即绑定，减少各应用的落盘、转换、归集代码。
5. **S3 兼容面**：让 rclone、restic、Litestream、DuckDB、各类 AI 工具链和依赖 S3 的 Docker 应用无需改造即可把 Zone 当对象存储用，成为 DFS 目录之外第二条兼容路线。
6. **容量可解释**：逻辑保留量、新增量、物理占用三个口径分开，UI 能直接展示。
7. **传统的配额与权限接口齐全**：两级配额（桶 + 资源池）、软硬限与告警、到前缀的动作授权、有期限可撤销的能力令牌，让运维工具和应用开发者按熟悉的方式接入（§4.4、§8）。
8. **存储语义显式**：应用用不同的桶表达“用户文档，永久保留”、“应用数据”、“临时”、“缓存”这些不同承诺，系统据此决定备份、卸载、回收和配额，而不是靠目录名猜（§4.5）。

### 1.3 非目标

- 不做 POSIX 文件系统语义（目录、rename 子树、append、随机写）。这些留给 DFS / NamedFileSystem 路线。
- 不替代 named store 的分布式放置、副本和纠删（NamedStoreMgr 的 Maglev 分片与未来 DCFS 的职责）。
- 不做跨 key 事务（S3 也不提供）；提供有限的原子批量绑定（§15.2）。
- 不做服务端加密、Object Lock、跨 Zone 复制策略。跨 Zone 的分发走 CYFS 协议 pull。
- 第一期不实现 S3 的 Bucket Policy / ACL 子语言，授权沿用 RBAC（§4.4）。

## 2. 业界经验与取舍

对象存储这个领域已有二十年经验，下表是本设计借用与明确不借用的部分。

| 系统 | 借用 | 不借用 / 原因 |
| --- | --- | --- |
| **AWS S3** | 扁平 key + 前缀/分隔符列举；强一致读写（2020 起）；条件写 `If-Match` / `If-None-Match`（2024 起）；分段上传三段式；生命周期规则；默认阻断公开访问（2023 起默认）；版本化默认关闭 | 存储类与跨区复制；Bucket Policy 子语言；ACL（S3 自己也已标记 legacy） |
| **GCS** | generation 作为乐观并发令牌；软删除窗口 | 层级命名空间桶（我们的层级由 DirObject 快照表达） |
| **Azure Blob** | Block Blob 的 PutBlock + PutBlockList 两段式：数据块先入库、再一次性提交块表；未提交块 7 天过期 | Blob Lease 独占写（我们的内容不可变，不需要） |
| **MinIO** | 兼容层 ETag 策略：单段 PUT 返回 MD5，分段返回 `md5(md5s)-N`，最大化客户端兼容 | 自有纠删布局 xl.meta（named store 已有自己的落盘） |
| **Ceph RGW** | 教训：bucket index 是列举与扩展的瓶颈，需能分片；对象尾部走 GC 队列异步回收 | omap 索引实现；多站点同步 |
| **Garage** | 元数据三表模型 `objects → versions → block_refs`，块按哈希引用计数、延迟回收 | CRDT 多主元数据（Zone 内单写者足够） |
| **SeaweedFS** | filer 元数据与 volume 数据分离；小文件走 needle | 自有 volume 格式 |
| **OCI Registry** | tag 是可变引用、manifest/blob 不可变；跨仓库 blob mount 不复制字节；manifest 决定 blob 的可达性 | mark-sweep GC 需只读窗口（我们用 pin 引用计数，无需停机） |
| **Git / OSTree / Nix** | refs / gcroots 是唯一 GC 根，对象图不可变；`prune` 按根可达性；提交即快照 | 完整对象图遍历 GC（named store 已用 outbox 级联代替） |
| **Chrome Storage Buckets API** | 浏览器给每个站点多个命名 bucket，各自声明 `persisted`、`durability`、`expires`、`quota`；best-effort 桶可被浏览器在压力下整体清除 | 单源同配额（我们按档分池） |
| **iOS / Android 应用目录约定** | iOS `Documents`（备份、用户在 Files 可见、卸载询问）/ `Library/Caches`（可清除、不备份）/ `tmp`；Android `filesDir` / `cacheDir` / MediaStore（用户所有、卸载保留）；语义由位置决定而不是由应用自觉 | 用目录名承载语义（我们用桶的声明承载） |
| **Pocket / Instapaper / Wayback Machine** | 保存别人的内容要存全文副本而不是只存链接，因为来源会消失（link rot） | — |
| **IPFS pin 与 cache 的区分** | pin = 我承诺保留；未 pin 的内容是可逐出缓存，需要时按 CID 再取 | — |
| **IPFS Pinning Service API** | pin 有状态（queued / pinning / pinned / failed），可带名字与元数据 | 公网 DHT 发布 |
| **Dropbox（Dropship 事件，2011）** | 教训：跨用户内容去重若允许"报哈希即拥有"，会变成任意内容的读取与分发通道 | — |

归纳成三条本设计的基本判断：

1. **元数据才是难点**，数据面交给内容寻址存储。索引放 RDB，走 `rdb_mgr`，后端可换。
2. **可变引用 + 不可变内容** 是所有成功的内容寻址系统（Git、OCI、OSTree、Nix）共同的形状；Bucket 就是这个形状在 named store 上的应用层表达。
3. **兼容面是外壳不是内核**：S3 API 作为 Bucket 的一张皮，内核语义以原生接口为准，S3 做不到的地方明确列差异而不是迁就。

## 3. 核心模型

### 3.1 三层关系

```text
Bucket（可变，RDB 索引，Zone 级服务管理）
  └── Binding: (bucket_id, key[, version]) → ObjId + 绑定元数据
          │  pin(ObjId, owner="bucket:<bucket_id>", scope=Recursive)
          ▼
named store（不可变，内容寻址）
  └── NamedObject（FileObject / DirObject / 任意 ObjType）
          └── ChunkList → Chunk（非结构化字节）
```

- Bucket 索引回答"谁、以什么名字、保留着哪些对象"。它是**权威**。
- named store 的 `pins` 表是索引的**物化**：每个被至少一条 Binding 引用的 ObjId，恰有一条 `(obj_id, owner="bucket:<id>")` 的 Recursive pin。
- Chunk 的保留完全由 named store 的级联（`incoming_refs` / outbox）推导，Bucket 不直接接触 chunk。

### 3.2 不变量

| # | 不变量 | 维护者 |
| --- | --- | --- |
| B1 | 索引里每个 distinct `(bucket_id, obj_id)` 对应 named store 中恰一条 pin；索引里没有的 `bucket:<id>` pin 是垃圾 | 绑定事务 + 周期 reconcile（§7.3） |
| B2 | Binding 指向的 ObjId 永不改写；"修改"只能是换绑定（新 ObjId、新版本） | 内容寻址天然保证 |
| B3 | 单个 key 的读写线性一致：PUT 成功返回后，任何 GET / LIST 立即可见 | 索引事务（SQLite 单写者） |
| B4 | 解绑后内容是否仍在，由 named store 决定；Bucket 只保证"我不再保留" | GC 三档模型 |
| B5 | 绑定一个 ObjId 需要对该内容的**持有或读取权**，不能凭哈希认领 | §6.4 |

### 3.3 桶有类型：`data` 与 `object`【已定 D1】

落盘方向文档把 Bucket 放在“非结构化数据”一节，但 §5.3 / §9.3 指出结构化对象（消息、动态、Package）同样缺归属和 GC 根。两者是同一个问题，用同一套 Bucket 机制解；但**不在一个桶里混装**，桶在创建时声明类型，之后不可改：

| kind | Binding 的值 | 典型内容 | S3 兼容面 | 读取形态 |
| --- | --- | --- | --- | --- |
| `data` | `cyfile`（及 `cydir`，表示一棵已对象化的文件树） | 上传的文件、图片、Agent 产物、导入的目录 | 暴露 | 字节流，Range，Content-Type |
| `object` | 其他任意 ObjType（`cymsg`、`cyact`、应用自定义对象） | 消息、动态、便签、应用的领域对象 | 不暴露 | `application/cyfs+json` 对象正文 |

分开的理由：两类桶的消费者、列举习惯、备份粒度和兼容面都不同，混装后任何一边的调整都会牵动另一边；分开后 `object` 桶也可以独立演进（例如按 ObjType 过滤、按对象字段建二级索引），不受 S3 语义约束。

【现状】named store 的级联展开（`reconcile_expand_state` → `parse_obj_refs`）只识别 Dir / File / ChunkList 三类子项。`object` 桶绑定一个 `cymsg` 只保护消息对象本身，它引用的附件 `DataObj` 不会被级联保护。cyfs-ndn 需要把 `MsgObject` 等标准对象的子项遍历（标准对象文档 §19）接进 `parse_obj_refs`（§14.4）；在此之前，应用要把附件放进自己的 `data` 桶，消息放进 `object` 桶，两边各自绑定。这也正是分型的好处：附件的归属从一开始就清楚。

### 3.4 三个概念：key、version、ObjId

开发者只需要记住三个名词，它们的责任边界固定：

| 概念 | 含义 | 可变性 | 带来保留责任 | 带来读取权 |
| --- | --- | --- | --- | --- |
| **key** | 桶内的可变名字 | 可变（PUT 覆盖、DELETE） | 是（当前版本） | 按桶 / 前缀授权 |
| **version** | 某一次写入；`(key, version)` 唯一定位一条 Binding | 不可变 | 是（版本化桶中直到被清理） | 同 key |
| **ObjId** | 不可变的内容身份 | 不可变 | **否** | **否** |

规则：把 ObjId 单独存进 RDB 或传给别人，**既不建立保留责任，也不授予读取权**。需要保留就绑定到桶；需要让别人读就授权桶 / 前缀或签令牌。RDB 里需要精确指向某次写入时存 `(bucket_id, key, version)`，需要内容身份时存 ObjId，两者可以都存。

## 4. 资源模型：归属、命名、声明

### 4.1 归属主体

Bucket 的归属沿用 RDB Instance 的三元组：`(appid, owner_user_id | zone, bucket_name)`。

| scope | 归属 | 典型用途 | 对应 RDB 分区 |
| --- | --- | --- | --- |
| `user`（默认） | `users/{user}/apps/{appid}` | 用户在该 App 里的数据：相册、附件、工作区资产 | `user_data` |
| `zone` | `services/{appid}` 或 App 的 Zone 级实例 | **服务级归属**：msg-center 的附件、HomeStation 的已发表内容、Repo 包。默认只有该服务能访问；谁能读由服务自己通过授权或令牌决定 | `user_data` |

这遵守"Zone 服务默认多用户、状态隔离"的原则：默认每个用户在每个 App 下各有自己的桶，App 不得假设只服务 owner。

### 4.2 bucket_id 与 URL

```text
bucket_id  = <scope>/<appid>/<owner>/<name>
             user/photos.app.bns.did/alice/library
             zone/homestation/_/published

obj://bucket/<bucket_id>/<key>                 Agent 与 RBAC 使用的资源 URL
https://$zone/bucket/v1/<bucket_id>/<key>      HTTP 数据面（§12.2）
https://s3.$zone/<s3-bucket-name>/<key>        S3 兼容面（§12.3）
```

`obj://bucket/...` 遵守 `doc/key url.md` 中 `obj://$service/...` 的约定，可直接作为 RBAC policy 的 resource。

### 4.3 在应用包里声明【已定 D7】

与 `rdb_instances` 并列，`install_config.buckets`。每个桶必须声明 `kind` 与 `data_policy`（§4.5）：

```json
"buckets": {
  "library":  { "kind": "data",   "data_policy": "documents", "scope": "user", "versioning": true },
  "index":    { "kind": "data",   "data_policy": "app_data",  "scope": "user", "quota_mib": 2048 },
  "notes":    { "kind": "object", "data_policy": "documents", "scope": "user" },
  "saved":    { "kind": "data",   "data_policy": "collected", "scope": "user", "versioning": true },
  "related":  { "kind": "object", "data_policy": "ambient",   "scope": "user", "quota_mib": 512 },
  "scratch":  { "kind": "data",   "data_policy": "temp",      "scope": "user", "default_ttl_days": 7 },
  "thumbs":   { "kind": "data",   "data_policy": "cache",     "scope": "user", "quota_mib": 1024 }
}
```

- 声明了 `app_data` 或 `temp` 桶的 App 必须同时声明 App 池配额 `install_config.storage_quota_mib`【已定 D17】；安装器据此向用户显示“该应用最多占用 X”。缺失时打包校验（pikg）直接拒绝。
- 安装期由 deployer 合成到 spec 中，运行期 Bucket 服务按 spec 懒创建，与 RDB 实例“安装期决定、运行期只读 spec”的边界一致。
- **声明了才有**。未声明 `buckets` 的 App 没有桶，访问报 `bucket_not_found`；不自动补 `default` 桶，因为每个桶都必须显式选档，系统替它选任何一档都是在替应用做承诺。
- `kind`、`data_policy`、`scope` 创建后不可改。要换档，声明一个新桶，用 `bucket.copy` 按前缀把绑定搬过去（纯元数据，O(1)/key），再删旧桶。
- 升级 App 时 spec 可以新增桶、调整 `quota_mib` / `versioning` / `lifecycle` / `default_ttl_days`；去掉一个已存在桶的声明不会删桶，只让它变成“孤儿桶”出现在存储空间页，由用户处置。

### 4.4 权限模型

对象存储的权限在业界已经收敛成四件事：**一组动作**、**到前缀的资源粒度**、**身份策略 + 显式授权两层**、**有期限的能力令牌**。本节按这四件事给出接口，底层仍是 BuckyOS 的 RBAC 四元组 `(user, app, action, resource)` 交集模式。

#### 4.4.1 动作集

| 动作 | 覆盖的操作 | S3 对应 |
| --- | --- | --- |
| `list` | `list_objects`、`list_versions`、`stat`、`get_quota` | `s3:ListBucket`、`s3:ListBucketVersions` |
| `read` | `get`、`head`、HTTP `GET` / `HEAD`、作为 `copy` 的源 | `s3:GetObject`、`s3:GetObjectVersion` |
| `create` | 只能新建 key：服务端强制 `if_none_match: "*"`，覆盖已有 key 被拒 | Azure SAS 的 `c`/`a` 权限；S3 无对应 |
| `write` | `put`、`copy` 的目标、`set_metadata`、`batch` 中的 put、分段上传、`auto_bind`；蕴含 `create` | `s3:PutObject`、`s3:AbortMultipartUpload` |
| `delete` | `delete`、`batch` 中的 delete | `s3:DeleteObject`、`s3:DeleteObjectVersion` |
| `manage` | `set_quota`、`set_lifecycle`、`grant` / `revoke`、`create_access_token`、`snapshot` / `restore`、`delete_bucket`、`set_versioning` | `s3:PutBucketPolicy`、`s3:PutLifecycleConfiguration` 等桶管理类 |

动作独立授予：能 `write` 不自动能 `delete`；能 `read` 不自动能 `list`（给单个 key 的链接不暴露同桶其他内容）。"只能追加的收集箱"要授 `create` 而不是 `write`：限制 `delete` 挡不住用 `write` 覆盖别人的 key，只有服务端强制 create-only 才挡得住。

#### 4.4.2 资源粒度：桶 + 前缀

RBAC 资源形如：

```text
obj://bucket/<scope>/<appid>/<owner>/<name>              整桶
obj://bucket/<scope>/<appid>/<owner>/<name>/photos/*     前缀
obj://bucket/<scope>/<appid>/<owner>/<name>/avatar.png   单 key
```

前缀是最小授权单位，与 S3 IAM 的 `Resource: arn:aws:s3:::bucket/photos/*`、GCS 的 IAM Conditions、Azure SAS 的目录级 SAS 一致。`list` 带前缀授权时，列举结果自动收窄到被授权前缀（S3 需要 `s3:prefix` 条件才做到，这里内建）。

#### 4.4.3 第一层：身份策略（RBAC 默认）

| 主体 | 默认策略（写进 `rbac_config.rs` 默认 policy） |
| --- | --- |
| App（本桶所属 appid） | 对自己声明的桶 `list` `read` `write` `delete`；`manage` 只对 `app_data` / `temp` / `cache` 档 |
| 用户（本桶 owner） | 对自己的桶全部五个动作；`documents` 档的 `manage`（含 `delete_bucket`）**只有**用户有 |
| admin | 对所有桶 `list` `read` `manage`；不默认有 `write` / `delete`（避免管理员无意改写用户文档）【已定 D16】 |
| Agent（读写模式，随 `owner_agents` 派生） | 对 owner 的桶 `list` `read` `write`；`delete` 需 Agent 设置里显式开启 |
| 服务（`scope=zone` 桶所属 appid） | 对自己的 zone 桶全部动作 |
| zone 用户（`scope=zone` 桶） | **默认无**。zone 只表达服务级归属，不表达"全 Zone 共享"；服务要开放共享，用 grant 显式给 `zone_users` 授权（§4.4.4），要做 `/srv/library/` 那种"人人可放、只动自己的"就授 `list` `read` `create`，配合 §4.4.6 的创建者规则 |
| 其他 App / 其他用户 | 无；靠第二层显式授权 |
| 匿名 / 分享链接 / 外部用户 | 无桶级身份权限；只能持能力令牌（§4.4.5）或经 ShareContentMgr / HomeStation |

Casbin policy 片段示例（占位符语义同 `doc/key url.md`）：

```text
p, users, obj://bucket/user/*/{users}/*,           list|read|write|delete|manage, allow
p, app,   obj://bucket/*/{app}/*,                  list|read|write|delete,        allow
p, admin, obj://bucket/*,                          list|read|manage,              allow
```

`documents` 档对 App 不放 `manage`、`delete` 仅限自己创建的 key 这两条，是桶服务在 RBAC 放行之后再做的档位检查，不写进 Casbin（Casbin 不知道档位）。

#### 4.4.4 第二层：显式授权（grant）

身份策略覆盖"自己的桶"。跨 App、跨用户、按前缀、带期限的共享用 grant：

```text
bucket.grant {
  bucket_id, 
  grantee: { kind: app | user | group | zone_users | agent, id },
  prefix?: "photos/2026/",
  actions: ["list", "read"],
  expires_at?: ts
} → grant_id

bucket.revoke { grant_id }
bucket.list_grants { bucket_id }
```

- 授予者需对该桶有 `manage`，且只能授出自己拥有的动作子集（不可越权转授，S3 bucket policy 同理）。
- `group` 用用户类型定义里的系统用户组（admin / users / limited 等），一次授一组。
- `zone_users` 等价于"本 Zone 所有用户"，是 `scope=user` 桶对外共享的常用形态（相册给家人看）。
- grant 不能授出 `manage`（避免链式转授），也不能给 `documents` 档以外的 App 授 `delete`（用户文档只能被用户或写入者删）。
- 三个常见场景的正确写法：**家庭共享相册** = 用户的 `documents` 桶 + `grant zone_users list,read`；**Zone 公共图库** = 服务的 zone 桶 + `grant zone_users list,read,create`；**会话附件** = msg-center 的 zone 桶，不授任何用户，成员读取由 msg-center 按会话成员关系代为读取或签发单 key 的只读令牌（§4.4.5）。三者在桶层面没有任何默认共享。
- 判定顺序：RBAC 身份策略 **或** 任一匹配的未过期 grant 放行；都不放行即拒绝。没有 deny 规则：保持"默认拒绝、只做加法"，这是 BuckyOS 和 GCS uniform bucket-level access 都选的简化。

#### 4.4.5 能力令牌（access token）

对应 S3 presigned URL、Azure SAS、GCS signed URL：**持有即授权、有期限、可撤销、不需要对方有 Zone 身份**。

```text
bucket.create_access_token {
  bucket_id,
  prefix | key,                     # 二选一
  actions: ["read"] | ["write"] | ["read","write"],   # 不可含 list（除非 prefix 形式显式给）、不可含 manage
  expires_at: ≤ now + 7d,
  max_uses?: n,
  bind_to?: { user_id } | { app_id }                 # 可选：只允许某已登录主体使用
} → { token, url }

bucket.revoke_access_token { token_id }
bucket.list_access_tokens { bucket_id }
```

- 令牌以 `Authorization: Bearer bkt_…` 或 `?token=` 携带，数据面 `/bucket/v1/...` 与 S3 面都接受。S3 面另有标准 SigV4 预签名 URL（§12.3），那是无状态的客户端签名，不登记、不能逐条撤销；`bkt_` 令牌是平台签发的、登记在 `bucket_access_tokens` 的能力，两者不是同一张表的两种编码。
- 允许 `write` 令牌【已定 D15】：限前缀、≤7 天、不含 `list` / `delete`。典型用途：给外部联系人一个 7 天的下载链接；给浏览器一个只能 `write` 到 `uploads/<session>/` 前缀的直传令牌（S3 presigned PUT 的标准用法）；Agent 把中间结果交给外部工具处理。
- 与 D10 不冲突：令牌是**点对点、有期限、可撤销**的能力，不是公开；永久公开仍只经 ShareContentMgr / HomeStation。
- 签发需要 `manage`，或对该前缀同时有被授出的全部动作（用户给自己的照片签一个只读链接不需要 manage）。
- 既有的 `gt_` 访客令牌与自托管群 48 小时链接（EXT-11）是同一类东西，实施时并入同一个签发与校验模块【已定 D18】。

#### 4.4.6 对象所有权

每条 Binding 记录 `created_by`（`user|app` 主体）。它只在 **`scope=zone` 桶且调用方是经 grant 进来的用户**时起作用：覆盖（`write` 已有 key）与 `delete` 只允许创建者。这复刻 `/srv/library/` 的"人人可放、只动自己的"，并且堵住"不能删但能覆盖"的漏洞。桶所属服务与 `user` 桶的 owner 用户对所有 key 都有全部动作，不看 `created_by`。S3 的 Object Ownership 问题（跨账号上传后桶主无权）在这里不存在：桶 owner 永远有全部动作。

#### 4.4.7 权限查询与审计

- `bucket.check_access { bucket_id, key|prefix, action, principal? }` → `{ allowed, via: rbac|grant:<id>|token }`。给 UI 决定按钮灰不灰，给 Agent 在动手前自检；对应 k8s `SelfSubjectAccessReview` 与 IAM Policy Simulator。
- 拒绝的写入与所有 grant / token 变更都打 info 日志（log first 原则），`bucket.list_grants` / `list_access_tokens` 即审计视图。访问统计不在桶服务做，沿用 ShareContentMgr 的访问统计口径。

#### 4.4.8 要点

- **知道 ObjId 不构成读取权**（沿用 NDM 网关现有规则）。通过桶 key 读取时，权限判在桶与前缀上，不再判对象。
- "公开桶"不存在，等价于 S3 Block Public Access 默认开且不可关。
- S3 兼容面的访问密钥（§12.3）映射到 `(user, app)` 主体，权限仍由本节决定；密钥可附带 `scope`（桶列表 + 前缀 + 动作）进一步收窄，即 MinIO service account 的 inline policy 形态。

#### 4.4.9 业界对照

| 系统 | 对应本节 |
| --- | --- |
| S3 IAM / bucket policy / presigned URL / Block Public Access | 动作集、前缀资源、grant、access token、无公开桶 |
| GCS uniform bucket-level access、signed URL | 不做对象级 ACL、只做加法 |
| Azure RBAC 角色 + SAS（`rwdl`、目录级、存储访问策略可撤销） | 动作独立授予、能力令牌可撤销 |
| MinIO service account + inline policy、`mc anonymous` | 受限 S3 密钥；匿名能力不提供 |
| Ceph RGW user / subuser / caps | 用户 + App 两主体交集 |
| POSIX `/srv/library` 风格"可写但只删自己的" | zone 桶 `created_by` 规则 |

### 4.5 数据策略（data_policy）

这是桶最重要的属性。它回答系统最常被问到的四个问题：**这份数据是谁的、丢了算不算事故、什么时候可以回收、算在谁的额度里**。应用通过建几个不同档的桶来表达不同的存储语义，而不是在一个桶里给每个对象打标签。

档位背后有两个坐标：**归谁 / 能不能清**（传统对象存储只有这一个），和**是不是我创作的 / 丢了能不能从内容网络再拿回来**。第二个坐标是 CYFS 内容网络带来的：别人的内容有可验证的 ObjId 和来源 Zone，所以"只留描述、要用再拉"是一种真实可行的保留策略，而不是传统意义上的归档。按第二个坐标，持久数据分三档：

| 档 | 谁创作 | 丢了怎么办 | data_policy |
| --- | --- | --- | --- |
| 第一档 | **我**（用户或替用户工作的 App） | 无处再拿，必须完整保留并备份 | `documents`、`app_data` |
| 第二档 | **别人**，但我认真收集、对我很重要（收件箱里收到的附件、保存下来的文章） | 来源可能消失（link rot），必须本地完整保留并备份，同时记住来源以便校验与更新 | `collected` |
| 第三档 | **别人**，最好能有（一个视频的评论、字幕、相关推荐） | 来源还在就再拉，不在就算了；本地只保证描述，内容按缓存对待 | `ambient` |

`temp` 与 `cache` 不在这三档里，它们是派生物与短期产物，没有"创作"可言。

设计原则：档是**预设档位**，不是可自由组合的属性集。每一档固定一组语义，应用只能在档内调少数参数。原因同“没有潜规则”：备份、卸载、GC 这些系统行为必须能只看一个字段就决定，运维和用户也只需要理解六个词。

#### 4.5.1 档位

| data_policy | 一句话语义 | 归属 | 保留（pin） | App 卸载时 | 备份 / 迁移 | 生命周期规则 | 配额计入 | 用户可见 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **`documents`** 用户文档 | 用户创造、导入或明确要求保存的内容，产品承诺**永久保留** | **用户**（App 只是写入者） | Recursive，无 TTL | **保留**；桶转为用户的孤儿桶，重装同一 App 自动接回 | 必备份；跟随用户迁移 | 禁止过期规则；只允许清理非当前版本 | 用户的 `res_pool` 存储配额 | 是：出现在用户的存储空间页，并只读投影到 Files（§4.5.4） |
| **`app_data`** 应用数据 | App 替用户生成、运行必需、重建昂贵（索引、派生资产、下载的模型） | App | Recursive，无 TTL | 默认删除，用户可选保留 | 备份；跟随用户迁移 | 允许 | App 声明的 `quota_mib` | 否；只在存储空间页按 App 聚合 |
| **`collected`** 收集 | **别人创作**、我认真收集的内容；来源可能消失，所以本地必须有完整副本 | **用户** | Recursive，无 TTL；Binding 必带 `origin` | **保留**（同 documents） | 必备份；跟随用户迁移 | 禁止过期规则；只允许清理非当前版本 | 用户的 `res_pool` 存储配额 | 是：Files 投影在 `Collected/` 下 |
| **`ambient`** 周边 | **别人创作**、最好能有的内容；描述必须在，内容按需从内容网络再拉 | 用户 | **Skeleton**：锁描述对象（FileObject / MsgObject 等），不级联到内容；内容若在本地则是 class 0 缓存 | 删除（描述可随备份恢复） | 只备描述；恢复后内容按需再拉 | 允许；另有软配额 LRU | 描述字节计用户配额；内容字节按 `cache` 软配额 | 否（App 内呈现） |
| **`temp`** 临时 | 处理期间的中间产物，**到期必失效** | App | Recursive + 必填 `expires_at`（缺省取桶的 `default_ttl_days`，上限 30 天） | 删除 | 不备份 | 强制过期 | App 的 `quota_mib` | 否 |
| **`cache`** 缓存 | 能从别处重建的派生物（缩略图、转码、预取） | App | **不 pin**：内容停留在 class 0，设备压力下按 LRU 逐出 | 删除 | 不备份 | 允许 | 软配额：超出只触发本桶淘汰，不拒绝写入 | 否 |

原先预留的 `archive`（内容由别处保证、本地只留描述）并入 `ambient`：它就是"来源是我自己的另一个 Zone 或云备份"的 ambient，不需要单独一档。

#### 4.5.2 各档的关键语义

**`documents`**

- 这是“用户文档，永久保留”。它的归属是用户不是 App，所以：App 卸载不删；用户在 Files 里能看到；用户删除整个桶需要走和删除应用同级的 sudo 确认（既有规则：会导致数据丢失的操作需要 sudo）；App 删除单个 key 是正常业务操作，但 `bucket.delete_bucket` 对 documents 档只允许用户发起。
- 不允许任何会自动丢数据的规则。`expires_at` 被拒绝，lifecycle 只接受 `noncurrent_expire_days`。
- 默认 `versioning: true`【已定 D12】。未改动内容的重复写入不占新空间，改动了的历史版本会占空间；在“永久保留”的承诺下，误覆盖比多占空间更不可接受，空间由 `noncurrent_expire_days` 与用户自己清理。
- 配额算在用户头上。这是 iOS Documents / Android MediaStore 的模型：数据是用户的，额度也是用户的。
- 对 `object` 桶同样成立：用户写的便签、批注、标注是 documents；App 自己的运行对象是 app_data。

**`app_data`**

- 与今天的 `$BUCKYOS_DATA_DIR` / RDB `user_data` 分区同义，只是对象化了。备份与迁移跟随用户，卸载时按既有 App 数据目录的规则询问。
- 这是大多数“应用内部非结构化数据”的默认去处。

**`collected`**

- 与 `documents` 的保留语义完全相同（完整副本、永久、备份、用户配额、用户可见），差别只在**作者不是我**。分开成一档的原因：
  - Binding 必须记录 `origin`：作者 DID、来源 `cyfs://` 地址或 Zone、收集时间，以及拿得到的可验证材料（签名 JWT 对象本身、`PathObject` / `InclusionProof`）。这让收集的内容始终可以回到来源校验、比对更新，也让 UI 能标出"来自谁"。
  - 它不是我发表的：HomeStation / ShareContentMgr 不会把 `collected` 桶的内容当作本人创作列出；再分享时署名仍是原作者（内容网络里 ObjId 与签名随对象走，这一点天然成立）。
  - 备份优先级与 `documents` 同级：来源 Zone 可能下线，这份副本可能是唯一副本。这是 Pocket / Instapaper 式"保存全文而不是只存链接"的理由，也是 Wayback Machine 存在的理由。
- 典型流程：收件箱里的附件先在 msg-center 的 zone 桶里（服务归属，§14.3）；用户点"收藏"，MessageHub 调 `bucket.copy` 把它复制进用户的 `collected` 桶（纯元数据，O(1)），`origin` 由 MessageHub 从消息的发送者与消息 id 填好。之后即使消息被撤回、群被解散，收藏仍在。
- `object` 桶同样适用：收藏别人的动态、便签、评论对象。

**`ambient`**

- 描述对象（FileObject、MsgObject 等）用 `Skeleton` pin 锁住，**不级联到内容**；内容若在本地就是 class 0 缓存，设备压力下可逐出，要用时按 `origin` 从内容网络再拉。这是 named store GC 设计里 Skeleton 的本意（"这一支留个壳，不要内容"）在桶层的直接用法。
- `GET` 命中已逐出的内容时返回 `202 + Retry-After` 并触发拉取（这是 §6.2 里 202 唯一的 v1 来源）；App 对 ambient 内容必须按"可能要等"来设计 UI。拉取失败返回 `404 origin_unavailable`，描述仍在，key 不删。
- Binding 同样必带 `origin`；绑定时只要求描述对象在库，不要求内容在库（§6.1 持久条件的例外）。
- 备份只带描述；恢复后内容按需再拉，所以 ambient 桶在迁移中几乎不占备份体积。
- 典型内容：视频的评论与字幕、订阅的别人 HomeStation 动态、Agent 为一个任务拉来的参考资料、联系人的公开资料。
- 用户想把某条 ambient 内容"认真留下"，就是 `bucket.copy` 到 `collected` 桶：复制时要求内容在库（没有就先拉），之后变成 Recursive 保留。两档之间的升级是一次 copy，不是改桶属性。

**`temp`**

- 与 `cache` 的区别是**承诺方向相反**：temp 承诺“到期前一定在”，cache 承诺“随时可能不在”。Agent 的中间结果、分步处理的半成品、待用户确认的导入要 temp；缩略图要 cache。
- 过期是解绑，不是物理删除；物理回收由 named store 决定（§7.4）。

**`cache`**

- 绑定不 pin，内容是 class 0【已定 D13】。这正是 named store “cache-first” 哲学在桶层的直接表达：`put` 廉价、不守护。刚写入就可能被逐出是接受的代价；需要保底窗口的用 `temp`。
- `GET` 遇到内容已被逐出时返回 `404 content_evicted` 并在索引中懒删除该 key；应用按缓存未命中处理。
- 软配额：超出 `quota_mib` 时，桶服务按 `last_access` 解绑本桶最旧的 key（named store 侧随后自然回收），不拒绝新写入。这与 Chrome best-effort bucket 和 iOS Caches 一致。
- 不备份、不迁移、不出现在用户视图。

#### 4.5.3 档内可调参数

| 参数 | documents | app_data | collected | ambient | temp | cache |
| --- | --- | --- | --- | --- | --- | --- |
| `quota_mib` | 不可设（用用户配额） | 可设，硬 | 不可设（用用户配额） | 可设，软（只约束内容缓存） | 可设，硬 | 可设，软 |
| `versioning` | 可设，默认 true | 可设，默认 false | 可设，默认 true | 不可设（false） | 不可设（false） | 不可设（false） |
| `lifecycle` | 仅 `noncurrent_expire_days` | 全部 | 仅 `noncurrent_expire_days` | 全部 | 不可设（强制过期） | 全部 |
| `default_ttl_days` | 不可设 | 不可设 | 不可设 | 可设 | 必填，≤30 | 可设（作为 LRU 之外的上限） |
| 对象级 `expires_at` | 拒绝 | 允许 | 拒绝 | 允许 | 必填 | 允许 |
| Binding `origin` | 可选 | 可选 | **必填** | **必填** | 可选 | 可选 |

`origin` 的形状：`{ author: DID, source: cyfs:// 或 obj:// 地址, collected_at, proof?: jwt 或 cyinc 或 cypath }`。桶服务只校验形状与 `author` 是合法 DID，不验签；验签是读取方在需要时按 CYFS 协议做的事。

#### 4.5.4 Files 投影【已定 D11】

`documents` 桶由 nfs-server 以只读 namespace binding 投影到用户目录：

```text
cyfs:///home/<user>/Apps/<App 显示名>/<桶名>/<key 按 / 展开>
```

- 这是 iOS “On My iPhone/<App>” 的模型：用户在 Files 里看得到、能复制走、能删除；删除 = 在桶里解绑，App 下次列举时看不到该 key。
- 投影是只读的：用户不能在那里新建或改写文件（那会绕过 App 的业务逻辑与元数据）。要往桶里放东西走 App 或上传直达桶。
- `collected` 桶投影到 `cyfs:///home/<user>/Collected/<App 显示名>/<桶名>/`，与自己创作的 `Apps/` 分开，让用户在 Files 里就能分清"我写的"和"我收的"。
- key `a` 与前缀 `a/` 同时存在时，投影按 §10.1 tree 的同一改名规则显示（目录保持 `a`，文件显示为 `a~file`）；改名只在视图层，桶内 key 不变。
- `object` 桶不投影（内容是 JSON 对象，不是用户意义上的文件）。
- 孤儿桶（App 已卸载）仍投影，目录名后缀 `（已卸载）`。

#### 4.5.5 与系统其他分类的对齐

| 本文 data_policy | RDB 分区（`rdb_mgr`） | `path_usage.md` 目录 | beta2.2 兼容清单分类 |
| --- | --- | --- | --- |
| `documents` | —（RDB 没有“用户所有”这一档，是桶特有的） | `/home/<user>/...` 用户文件 | 必须保留 |
| `app_data` | `user_data` | `/home/<user>/.local/share/<app>` | 必须保留 |
| `temp` | — | `/tmp/buckyos/<app>` | 必须恢复或明确终止（到期即终止） |
| `collected` | — | `/home/<user>/Collected/...`（投影） | 必须保留 |
| `ambient` | — | — | 描述必须保留；内容可重建（从内容网络） |
| `cache` | `cache` | `data/cache/...` | 可丢弃 |

备份工具、卸载器、存储空间页、GC 都只读 `data_policy` 做决定，不读桶名，不猜。

## 5. 对象与键

### 5.1 key

- UTF-8，1 到 1024 字节，不以 `/` 开头，不含 `\0`，不含 `..` 段，不能是空串。
- `/` 没有语义，只在列举时作为 `delimiter` 用于折叠出 `common_prefixes`。
- 建议应用用业务 id 作 key（`messages/<msg_id>/att/0`），这样 RDB 与桶的对账只是一次前缀列举（§15.1）。

### 5.2 绑定元数据与对象内在元数据分开

| 存放位置 | 内容 | 变化是否产生新 ObjId |
| --- | --- | --- |
| NamedObject（FileObject）本体 | 名字、大小、`content`（chunk / clist）、mime、作者等**内容内在**元数据 | 是 |
| Binding（索引行） | key、version、`content_type` 覆盖、用户元数据（≤2 KiB，S3 `x-amz-meta-*`）、tags（≤10 对）、`expires_at`、ETag、创建时间、创建主体 | 否 |

这样改一个对象的用户元数据不产生新对象、不动 pin；同一 ObjId 在不同 key 下可带不同元数据（S3 的元数据本就是版本级的）。

### 5.3 ETag

- 原生接口：`etag = ObjId`（字符串），天然等于内容身份。
- S3 兼容面：单段 PUT 返回 **MD5**（网关在流式接收时顺带计算，与 ObjId 一起存进 Binding）；分段上传返回 `md5(各段 md5)-N`。这是 MinIO 的做法，是对 AWS SDK、rclone、s3fs 等客户端最稳的兼容策略。`If-Match` 同时接受 MD5 形式与 ObjId 形式。

### 5.4 条件写

`put / delete / copy` 支持：

| 条件 | 语义 |
| --- | --- |
| `if_none_match: "*"` | key 不存在才写（create-only，幂等创建） |
| `if_match: <etag>` | 当前版本 etag 相等才写（CAS） |
| `if_version: <n>` | 当前版本号相等才写（GCS generation 风格） |

这是应用做"先写桶再写 RDB"而不丢更新的基础（§15.1）。

### 5.5 版本

- 桶级开关 `versioning`，默认 **关**（S3 同；个人设备容量有限，每次写入都要有删除机会）。
- 开启后：PUT 产生新版本，旧版本仍绑定（仍 pin）；DELETE 写 delete marker；`list_versions` 可查；`noncurrent_expire_days` 生命周期规则回收旧版本。
- 版本号 = 桶内单调递增的 `version_seq`（u64）。版本的成本要说准：**重复引用同一 ObjId 的成本很低**（只占一条 pin、一行索引），但内容真的改了，历史版本就真的占空间；改 FileObject 的名字、MIME 等内在元数据也会产生新 ObjId（内容 chunk 仍去重，FileObject 行多一条）。版本化桶的 `unique_bytes` 含非当前版本，§9 的 `noncurrent_expire_days` 是它的删除机会。

### 5.6 对象级 TTL【Bucky 扩展】

Binding 可带 `expires_at`。到期由生命周期任务解绑。S3 没有对象级 TTL（只有规则级），但个人 OS 里临时产物（截图、Agent 中间结果、预览图）非常多，规则级不够用。

它与 HTTP 的 `Expires` / `Cache-Control` **无关**：那两个是缓存过期，S3 把它们原样存为对象元数据并在 GET 时回传，本设计同样如此（存进 Binding 的 `http_headers`）。删除 TTL 在 S3 面用独立扩展头 `x-bucky-expires-at`（RFC 3339）传递；把 `Expires` 解释成删除会让"图片缓存一天"变成"图片一天后消失"。

## 6. 数据面

### 6.1 写入：两段式

```text
① 内容入库（data plane，已有）
   浏览器 / SDK：切 chunk → tus POST /ndm/v1/uploads（Upload-Metadata 带 bucket, key）
                → put_object(FileObject / ChunkList)
   完成的 chunk 与对象带 24h Lease pin（ndm-upload:<sid> / ndm-hold:<主体>）

② 绑定（control plane，本设计）
   bucket.put(bucket_id, key, obj_id, meta, conditions)
   事务内：写 Binding 行；若该 obj_id 在本桶首次出现 → pin(obj_id, "bucket:<id>", Recursive)
   返回：version, etag
```

这与 Azure Block Blob（PutBlock → PutBlockList）、S3 分段（UploadPart → Complete）同构，也与 named store 的 cache-first 哲学一致：**① 是廉价可淘汰的，② 是昂贵需守护的**。① 与 ② 之间崩溃，Lease 到期后内容自然回收，不留垃圾承诺。

**② 成功的含义是持久保存，不是"保留请求已登记"。** `bucket.put`（以及 `put_file`、S3 `PutObject` / `CompleteMultipartUpload`、`auto_bind`）返回成功当且仅当：

1. 内容已在 Zone 内（`is_object_stored(obj_id)` 为真，含 FileObject 引用的全部 chunk）；
2. 根对象的 Recursive pin 已在 named store 落库（class 2）；
3. Binding 事务已提交。

三者任一不成立就返回错误，不留半成功状态。调用方因此可以在成功后删除源文件。`ambient` 桶的条件 1 只要求**描述对象**在库（内容可以不在），条件 2 是 Skeleton pin；这是它的定义使然，不是弱化。级联把 chunk 从 class 0 提升为 class 1 是毫秒级的异步收敛，这段窗口由上传时打在 chunk 上的 24h Lease 覆盖；Lease 是补偿窗口，不是保证本身。`cache` 桶是唯一例外：它不 pin，成功只表示绑定已记录。

**上传直达桶**（落盘方向文档 §6.6）：`Upload-Metadata` 增加 `bucket` 与 `key`，并增加 `auto_bind: true`。SDK 在 `put_object(FileObject)` 成功后，网关检查该上传会话的所有 chunk 已完成且 FileObject 引用的内容由本会话主体持有，然后在服务端执行 ②。应用只收到一个结果：`{ bucket, key, obj_id, version, etag }`。【现状】FileObject 由 websdk 在客户端组装，上传链路末尾已有一次服务端调用 `POST /ndm/v1/import/finalize`（校验持有、做 24h Recursive 保护）。`auto_bind` 的落点就是这里：finalize 请求带 `bucket`、`key` 与条件头，服务端在同一处把 24h 临时保护换成桶的长期绑定。tus 会话本身不需要知道桶。

### 6.2 读取：按名字取流

```text
GET https://$zone/bucket/v1/<bucket_id>/<key>[?version=n]
  → 200 + 字节流（FileObject 的 content 解析为 chunk / clist，支持 Range，ETag，Last-Modified，Content-Type）
  → 非 FileObject：application/cyfs+json 对象正文
HEAD 同上，只返回头
```

这补上了 NDM 网关"下载路径未实现"的缺口，而且**权限判在桶而不是对象**，浏览器 `<img src>` 带 SSO cookie 即可访问，不需要应用自己签 token。Agent 工具按 `obj://bucket/...` 解析到同一入口，满足落盘方向文档 §8 的 URL 化要求。

v1 的 `documents` / `app_data` / `temp` 桶保证绑定时内容已在 Zone 内（§6.1 的持久条件），所以 `GET` 要么 200 要么 404。只有两种情况例外，都是桶的档位决定的、App 在建桶时就知道的：`cache` 桶内容被逐出返回 `404 content_evicted`；`ambient` 桶内容未在本地时返回 `202 + Retry-After` 并按 `origin` 触发拉取（拉不到返回 `404 origin_unavailable`）。普通 `<img>` 不会按 `Retry-After` 重试，所以 App 不应把 ambient 内容直接放进 `<img src>`，而要经 `bucket.head` 判断 `availability` 后再渲染。

### 6.3 复制与跨桶共享

`bucket.copy(src_bucket, src_key, dst_bucket, dst_key)` 是纯元数据操作：读源 Binding 的 ObjId → 在目标桶绑定。要求调用方对源有 `read`、对目标有 `write`。不传输字节，不产生新对象。这就是 OCI 的 cross-repo blob mount。

跨 App 分享（例如 MessageHub 把聊天附件存到用户相册）用的就是 copy：两个桶各自 pin，任一方解绑都不影响另一方。

### 6.4 持有证明与去重边界【已定 D6】

内容寻址带来一个 S3 没有的安全问题：如果 `bucket.put(key, obj_id)` 接受任何 ObjId，那么知道哈希就等于拥有内容（Dropbox Dropship）。规则：

1. 绑定一个 ObjId，调用主体必须满足其一：（a）本主体上传 / `put_object` 过它（NDM 的 `ndm_holdings` 已记录）；（b）对某个已绑定它的桶有 `read`（即 copy 语义）；（c）有 `obj://ndm/objects` 读权限（admin、frame 服务）。
2. 上传前查重（`/ndm/v1/objects/lookup`）的 **跳过上传** 短路只在本主体已持有的范围内生效；跨主体即使 chunk 已存在也要求上传完整字节（存储层仍自然去重，只是不省流量）。这堵上 NDM 文档里登记的"秒传无持有证明"已知风险。

### 6.5 大小与分段

- Chunk 上限 32 MiB（named store 约束），大文件是 ChunkList。S3 兼容面的 `UploadPart` 允许 5 MiB–5 GiB 的段，网关流式接收、按 32 MiB 切 chunk、每段记 MD5；`CompleteMultipartUpload` 组装 ChunkList + FileObject 并绑定。
- 小对象（≤ 几 KiB）仍是 FileObject + chunk 两个对象行；named store 把小内容放 SQLite 行，不落独立文件。不另设内联格式。

## 7. 保留、GC 与缓存

### 7.1 pin owner 约定

```text
owner = "bucket:" + bucket_id           # 普通绑定
owner = "bucket-snapshot:" + snapshot_id # 快照根（§10）
```

pin scope 由数据策略决定（§4.5.1）：`documents` / `app_data` / `collected` / `temp` 为 `Recursive`（绑定即要求内容本体在 Zone 内可用）；`ambient` 为 `Skeleton`（只锁描述对象）；`cache` 不 pin。§7.2 的状态机只对需要 pin 的档运行，cache 桶的绑定事务不写 `bucket_pins`。

### 7.2 绑定与解绑的事务

```text
put(key, obj_id):
  require is_object_stored(obj_id)                 # 内容必须已在库，否则 content_not_available；ambient 只要求描述对象在库
  if ref_count(bucket_id, obj_id) == 0:
    pin(obj_id, owner, scope_of(data_policy))      # Recursive 或 Skeleton；同进程 named store 本地事务，幂等；失败即整个 put 失败
  txn(bucket index):
    upsert binding
    upsert bucket_pins(bucket_id, obj_id, state='held', ref_count += 1)
  commit                                           # 到这里才返回成功

delete(key):
  txn:
    remove binding (或写 delete marker)
    if ref_count 1→0: bucket_pins.state = 'releasing'
  commit
  unpin(obj_id, owner)
  txn: delete bucket_pins row
```

pin 在前、索引在后：崩溃只会留下"有 pin 无索引"的多余保留（reconcile 回收），不会留下"有索引无 pin"的假承诺。所以状态只有 `held` / `releasing` 两个，没有 `pending`。这比 msg-center / HomeStation 各写一遍的 pending → held → releasing 更简单，前提是 nfs-server 与 named store 同进程、pin 是本地事务。

### 7.3 reconcile

每 10 分钟一轮，加启动时一轮：

1. `releasing` 超过 1 分钟的行补 unpin。
2. 拉取 named store 中 owner 前缀 `bucket:<id>` 的 pins，与索引 distinct obj_id 对比：索引有、store 无 → 补 pin；store 有、索引无 → unpin。【现状】named store 只有 `unpin_owner`，没有"按 owner 列 pin"的查询（`pins_by_owner` 索引已存在），需要在 cyfs-ndn 增加 `list_pins_by_owner(owner_prefix, cursor)`，见 §14.4。
3. 对索引里 `state=held` 但内容读不到的对象记 `bucket_issues`（HomeStation 的 `retention_issues` 同款），供 UI 显示"内容缺失"。

### 7.4 释放的真实语义

解绑只撤销 **本桶** 的承诺：

- 同一 ObjId 被别的桶、HomeStation、fs_anchor 引用 → 它仍是 class 2，不回收。
- 没人引用 → 变 class 0，由 named store 的水位 / LRU 决定何时真正删字节；chunk 的回收依赖级联收敛（`await_cascade_idle`）。
- UI 的"已释放 X MiB"只能是**逻辑保留量**的变化；物理回收另有口径（§8）。

### 7.5 保留 vs 缓存

落盘方向文档 §6.5 要求区分"应用仍需要这份数据"与"这台设备当前缓存了它"。映射：

| 概念 | 在本设计里 |
| --- | --- |
| 保留范围 | Bucket 的 Binding 集合（pin） |
| 缓存状态 | named store 每个 bucket（分片）上的 `state` / `eviction_class` / `last_access_time`，按设备 |
| "关键时刻要准备好" | `bucket.prefetch(prefix)`：对前缀下对象触发拉取到当前设备（P2） |
| "内容在别处" | `ambient` 档：Skeleton pin 锁描述，内容按 `origin` 从内容网络再拉（§4.5.2） |

## 8. 配额与统计

### 8.1 口径

| 口径 | 定义 | 用途 |
| --- | --- | --- |
| `key_bytes` | 所有 Binding 指向对象的 `size` 之和（同一对象多 key 重复计） | S3 兼容的桶大小 |
| `unique_bytes` | 桶内 distinct ObjId 的 `size` 之和 | **配额依据**；UI 上"该应用保留的数据" |
| `key_count` / `version_count` | 当前 key 数 / 含非当前版本与 delete marker 的总条目数 | 对象数限额 |
| `added_bytes / removed_bytes`（按天） | 当天新绑定 / 解绑的 distinct 对象字节 | "最近新增了多少" |
| `snapshot_only_bytes` | 只被快照 pin、已无 Binding 引用的字节 | 快照占用，计入资源池（§10.1） |
| 物理占用 | named store 各分片 `owned_bytes`，跨桶去重后 | 系统级磁盘视图，不按应用拆分 |

用户 UI 展示"应用 X 保留 12.3 GiB，本周 +800 MiB"用的是 `unique_bytes` 与日增量；系统 UI 的"磁盘已用"来自 named store。两者不相等是预期，不要在界面上强行对齐。

【现状】系统今天没有任何按应用或用户的 named data 用量统计（`owned_bytes` / `logical_size` 列存在但无人聚合）；控制面板的存储空间页需求（`doc/control_panel/BuckyOS 的分享_发布_上线 功能的产品设计.md` §16）尚未实现。Bucket 的 `unique_bytes` 是这页的第一手数据源。

### 8.2 两级配额：桶配额 + 资源池配额

Ceph RGW 与 OpenStack Swift 都同时提供**用户级**与**桶级**配额，前者防一个用户吃光集群，后者防一个用途失控。本设计同样两级：

| 级别 | 作用对象 | 由谁设 | 说明 |
| --- | --- | --- | --- |
| **桶配额** | 单个桶 | 安装声明 `quota_mib` / `max_objects`；admin 运行期可覆盖；App 可在声明值以内调低 | `documents` 档不设桶配额（见下） |
| **资源池配额** | 一个用户的全部 `documents` 桶之和（**用户池**）；一个 App 的全部 `app_data` + `temp` 桶之和（**App 池**） | 用户池来自 `res_pool` 的存储项（D14）；App 池来自 `install_config.storage_quota_mib`，admin 可覆盖 | 池是真正的“额度”，桶配额只是池内的再切分。【已定 D14】`res_pool` 尚无存储项，第一期用户池只统计、不拒绝写入，配额执行随 res_pool 落地 |

写入要同时通过桶配额与所属池配额。`cache` 档只有桶级软配额，不计入任何池。

### 8.3 限额项

| 限额 | 含义 | 默认 | 来源 |
| --- | --- | --- | --- |
| `max_bytes` | 硬限，按 `unique_bytes` | 桶：声明值；池：res_pool / 声明值 | Ceph `max_size`、Swift `Quota-Bytes`、MinIO hard quota |
| `soft_bytes` | 软限，超过只告警不拒绝 | `max_bytes` 的 80% | Linux quota soft limit、Windows FSRM 阈值 |
| `max_objects` | key 条目上限（含非当前版本），保护索引 | 1,000,000 / 桶 | Ceph `max_objects`、Swift `Quota-Count` |
| `max_object_bytes` | 单对象大小上限 | `documents` / `app_data` 无；`temp` 2 GiB；`cache` 256 MiB | Swift 单对象上限、各网关 body 限制 |
| `warn_percent` | 触发告警事件的阈值列表 | `[80, 95]` | FSRM 阈值通知 |

不引入 Linux quota 的"软限 + 宽限期后转硬"：个人 OS 里多一个状态机不值得，软限只告警。

### 8.4 超限行为

| 情况 | 行为 |
| --- | --- |
| `put` / `copy` 目标 / `batch` 中的 put / 分段完成 / `auto_bind` 使 `unique_bytes` 超过任一硬限 | 拒绝，`quota_exceeded`，响应体带 `{level: bucket 或 pool, limit, usage}`；S3 面返回 `507 Insufficient Storage` + `QuotaExceeded` |
| 绑定的是桶内已存在的 ObjId（去重命中） | 不增加 `unique_bytes`，**不拒绝**，即使已超限 |
| `delete`、`set_metadata`、`list`、`read`、`revoke` | 永不因配额拒绝（Linux quota、Swift 同；超限时必须能删） |
| 版本化桶 PUT 覆盖 | 旧版本仍计入，直到生命周期清理 |
| `cache` 桶超软限 | 不拒绝；按 `last_access_at` 解绑本桶最旧的 key 直到回到限额以下 |
| 跨越 `warn_percent` 阈值 | kmsg 事件 `bucket.quota_warning {bucket_id 或 pool, percent, usage, limit}`，同时经 System Notify 通知用户（用户池）或写 App 日志（App 池） |
| named store 返回 `ENOSPC` | 发生在上传 ①，与配额无关；用户看到的是"设备空间不足"，绑定 ② 不会遇到它 |

### 8.5 记账与修复

- 用量在绑定事务内增量维护（`buckets.unique_bytes` 等列与 `bucket_usage_daily`），任何时刻读到的都是当前值。这与 Ceph RGW 的缓存式配额统计（有同步间隔、可能短暂超卖）不同，Zone 规模允许我们做精确记账。
- 池用量 = 对该用户 / App 所有桶的 `unique_bytes` 求和，写入时在同一事务内读取；桶数少，不另建池用量表。
- `bucket.recompute_usage` 从 `bindings` 重算 `unique_bytes` / `key_count`，供索引修复后或怀疑漂移时调用；reconcile（§7.3）每日自动跑一次比对，不一致记 `bucket_issues`。

### 8.6 接口

| 方法 | 参数 | 权限 | 说明 |
| --- | --- | --- | --- |
| `bucket.get_quota` | `{bucket_id}` | `list` | 返回 `{bucket: {max_bytes, soft_bytes, max_objects, max_object_bytes, usage}, pool: {kind: user 或 app, id, max_bytes, usage}}` |
| `bucket.set_quota` | `{bucket_id, max_bytes?, soft_bytes?, max_objects?, max_object_bytes?, warn_percent?}` | `manage`；App 只能调低到不超过声明值，admin 可任意 | `documents` 档拒绝 `max_bytes` |
| `bucket.clear_quota` | `{bucket_id}` | admin | 恢复为安装声明值 |
| `bucket.get_pool_quota` | `{kind: user 或 app, id}` | 本人 / 本 App / admin | 池额度与用量，按桶拆分 |
| `bucket.set_pool_quota` | `{kind, id, max_bytes}` | admin | 用户池写回 `res_pool`；App 池覆盖声明值 |
| `bucket.usage_report` | `{kind: user 或 app 或 zone, id?, days?}` | 本人 / admin | 存储空间页的数据源：各桶 `unique_bytes`、按档汇总、日增量序列 |
| `bucket.recompute_usage` | `{bucket_id}` | `manage` | §8.5 |

S3 面没有标准配额 API（AWS 自己也没有），不映射；`HeadBucket` 响应加 `x-bucky-quota-bytes` / `x-bucky-usage-bytes` 两个扩展头，供 rclone 类工具的 `about` 显示。

### 8.7 业界对照

| 系统 | 借用 | 不借用 |
| --- | --- | --- |
| Ceph RGW 用户配额 + 桶配额（`max_size` / `max_objects`） | 两级配额、对象数限额 | 缓存式统计与同步间隔 |
| OpenStack Swift 账户 / 容器配额、删除不受限 | 超限仍可删、`Quota-Count` | 用 header 承载配额 |
| Linux quota 软限 / 硬限 / inode 限额；XFS project quota | 软限告警、`max_objects` 类比 inode、按目录树计量类比按桶计量 | 宽限期状态机 |
| Windows FSRM 阈值通知 | `warn_percent` 事件 | 文件屏蔽 |
| MinIO `mc quota set` | 桶硬限、`QuotaExceeded` | — |
| Chrome Storage Buckets `quota` + best-effort 清除 | cache 档软限淘汰 | — |

## 9. 生命周期规则

规则是桶级声明（安装配置或运行期 `bucket.set_lifecycle`），每小时评估一次：

| 规则 | 作用 |
| --- | --- |
| `{prefix, expire_days}` | 当前版本超龄解绑 |
| `{noncurrent_expire_days}` | 非当前版本超龄解绑（仅 versioning 桶） |
| `{expired_delete_marker: true}` | 清理没有版本的 delete marker |
| 对象级 `expires_at` | 到期解绑（§5.6） |
| 未完成上传 | 由 NDM 会话 TTL 处理（已有），不在此重复 |

生命周期动作只做"解绑"，不碰 named store；后者按 §7.4 自行回收。各档允许的规则见 §4.5.3：`documents` 只接受非当前版本清理，`temp` 强制过期，`cache` 另有软配额淘汰。

## 10. 快照、备份、恢复、迁移

### 10.1 快照 = 一个 DirObject 根

`bucket.snapshot(bucket_id, label)`：

1. 在一个索引事务里读出当前全部 Binding（版本化桶可选 `include_noncurrent`），生成 **manifest**：一份 FileObject，内容是按 key 排序的 `(key, version, obj_id, size, etag, content_type, metadata, tags, created_at, created_by)` 列表。manifest 是快照的**权威内容**，恢复只读它。
2. 另外派生一棵 **tree**：按 key 的 `/` 切成 DirObject 层级，给人浏览与 `export` 用。它是有损视图，冲突按固定规则改名：key `a` 与前缀 `a/` 同时存在时，目录名保持 `a`，文件改为 `a~file`；叶子名与 `~` 后缀再冲突则追加 `~1`、`~2`。改名只影响 tree，manifest 里的 key 原样。
3. 快照根是一个两项的 DirObject：`manifest` → 第 1 步的 FileObject，`tree` → 第 2 步的 DirObject。保留名不会与用户 key 冲突，因为用户 key 只出现在 `tree` 之下。
4. 根 ObjId 以 `owner="bucket-snapshot:<snapshot_id>"` 做 Recursive pin。
5. 返回 `snapshot_id, root_obj_id, key_count, unique_bytes, snapshot_only_bytes`。

因为内容不可变且去重，快照新增的物理内容只有 manifest 与 tree 对象本身。这就是 Git commit / OSTree commit / OCI manifest 的做法。

**快照计费**：快照 pin 的内容若已不再被任何 Binding 引用（快照后 key 被删或覆盖），这部分字节记为 `snapshot_only_bytes`，计入所属资源池用量，在存储空间页单列为"快照占用"；删除快照即释放。配额只按 Binding 算会让"删了 key 额度下降、责任还在"，所以池用量 = Binding 的 `unique_bytes` + 各快照的 `snapshot_only_bytes`。

### 10.2 备份与恢复的组合

备份范围由数据策略决定：`documents`、`app_data`、`collected` 桶进入备份集并跟随用户迁移；`ambient` 只备描述对象与 `origin`，恢复后内容按需再拉；`temp` 与 `cache` 不进入。快照接口对任何档都可用，但备份工具只对前四档自动调用。

落盘方向文档 §9.1 的"应用恢复所需数据"在本设计下变成：

```text
应用备份集 = RDB Dump + 各桶的 snapshot root ObjId 列表
恢复        = RDB Restore + 对每个 snapshot root 做 pull（CYFS 协议或离线包）+ bucket.restore_from_snapshot
```

- `restore_from_snapshot(bucket_id, root_obj_id)`：读 manifest 重建 Binding（含 version、元数据），逐个 pin；不读 tree。
- **时间点契约**：快照是备份集的时间点，RDB dump 必须在快照**之后**紧接着做，并且 RDB 里对桶内容的引用按 §3.4 存 `(bucket_id, key, version)` 或 ObjId，不存裸 key。这样 dump 里引用的任一版本要么在快照 manifest 里，要么是 dump 与快照之间的新写入。恢复后跑 `bucket.verify_references(bucket_id, refs)` 对账，缺失的列出来而不是静默恢复成另一份内容。系统级备份任务按此顺序编排；应用自己做备份也要遵守。
- 跨 Zone 迁移 = 新 Zone 从旧 Zone pull 这些 root（CYFS 协议天然可验证），然后 restore。
- 这解决了 §9.4 "数据库恢复了但引用内容缺失"的一半：桶里的内容一定随快照走。RDB 里直接存的、不经桶的 ObjId 仍需 §9.3 的引用遍历，那是把结构化对象也放进桶（D1）的又一个理由。

### 10.3 导出为普通文件

`bucket.export(snapshot_id, target: cyfs://.../dir)`：把快照树物化成 DFS 目录，供用户用 FileBrowser 看。这是"两条路线并不冲突"的具体接口。

## 11. 事件通知

绑定 / 解绑 / 版本变化发到 kmsg 主题 `bucket.<bucket_id>`，消息体 `{event: object_created|object_removed|object_expired, key, obj_id, version, etag, size, actor}`。Agent 用它做"有新上传就处理"；S3 兼容面的 Bucket Notification 配置映射到同一机制。事件是尽力投递，不是事务一部分。

## 12. 接口

### 12.1 原生控制面：kRPC `bucket.*`

| 方法 | 参数 | 说明 |
| --- | --- | --- |
| `bucket.list` | `{scope?, appid?}` | 列出调用主体可见的桶及统计 |
| `bucket.stat` | `{bucket_id}` | 配额、`unique_bytes`、`key_bytes`、key 数、日增量 |
| `bucket.delete_bucket` | `{bucket_id}` | 全部解绑 + `unpin_owner`；`documents` 档只接受用户主体且需 sudo 会话 |
| `bucket.put` | `{bucket_id, key, obj_id, content_type?, metadata?, tags?, expires_at?, if_match?, if_none_match?, if_version?}` | 绑定；返回 `{version, etag, created}` |
| `bucket.get` | `{bucket_id, key, version?}` | 返回 Binding（含 obj_id、元数据），不返回内容 |
| `bucket.head` | 同上 | 另附 `availability`：`local`；`cache` 桶可能为 `evicted`；`ambient` 桶可能为 `remote`（未拉取）/ `fetching` |
| `bucket.delete` | `{bucket_id, key, version?, if_match?}` | 解绑或写 delete marker |
| `bucket.list_objects` | `{bucket_id, prefix?, delimiter?, start_after?, limit≤1000}` | 返回 `{items, common_prefixes, next_start_after}`；强一致 |
| `bucket.list_versions` | `{bucket_id, key}` | versioning 桶 |
| `bucket.copy` | `{src_bucket_id, src_key, src_version?, dst_bucket_id, dst_key, metadata_directive}` | 元数据复制 |
| `bucket.batch` | `{bucket_id, ops}`（≤100 个 put 或 delete） | 单桶内原子批量（§15.2） |
| `bucket.set_metadata` | `{bucket_id, key, metadata, tags}` | 不产生新版本 |
| `bucket.grant / revoke / list_grants` | §4.4.4 | 显式授权：按主体、前缀、动作、期限 |
| `bucket.create_access_token / revoke_access_token / list_access_tokens` | §4.4.5 | 能力令牌（presigned / SAS 等价物） |
| `bucket.check_access` | `{bucket_id, key 或 prefix, action, principal?}` | 权限查询 |
| `bucket.get_quota / set_quota / clear_quota / get_pool_quota / set_pool_quota / usage_report / recompute_usage` | §8.6 | 配额与用量 |
| `bucket.set_lifecycle / get_lifecycle` | 规则数组 | §9 |
| `bucket.snapshot / list_snapshots / delete_snapshot / restore_from_snapshot / export` | §10 | |
| `bucket.verify_references` | `{bucket_id, refs: [(key, version) 或 obj_id]}` | 恢复后对账，返回缺失列表（§10.2） |
| `bucket.prefetch` | `{bucket_id, prefix}` | P2 |

所有方法走 nfs-server 的 kRPC 入口，鉴权复用 `NfsSessionClaims` + RBAC（§4.4）。

### 12.2 原生数据面：HTTP

| 路由 | 方法 | 说明 |
| --- | --- | --- |
| `/bucket/v1/{bucket_id}/{key}` | `GET` / `HEAD` | 按名字读内容，Range、ETag、`If-None-Match` → 304 |
| `/bucket/v1/{bucket_id}/{key}` | `PUT` | 小对象直传：网关接收字节、切 chunk、建 FileObject、绑定；≤ 32 MiB 单请求；大文件仍走 tus + `auto_bind` |
| `/ndm/v1/uploads` | tus | `Upload-Metadata` 增加 `bucket`、`key`、`auto_bind` |

`PUT` 直传是给 curl、Agent 脚本、小图标这类场景的便利入口，不是主路径。

### 12.3 S3 兼容面

**范围限定**：只有 `kind=data` 的桶出现在 S3 面；`object` 桶对 S3 客户端不可见。任何档都可以暴露，但 `cache` 桶的 `GET` 可能返回 `NoSuchKey`（内容被逐出），S3 客户端应把 cache 桶当缓存用。

**宿主**：`s3.$zone_hostname`（virtual-host 风格）与 `https://$zone/s3/`（path 风格）都接受；网关路由到 nfs-server 的 S3 模块。S3 桶名到 `bucket_id` 的映射：`<appid-slug>-<name>` 对 `(user=当前密钥的用户, appid, name)`；密钥本身绑定了用户，所以 S3 客户端看到的桶列表就是该用户在该 App 下的桶。

**鉴权**：SigV4（header 与 query 预签名两种）。密钥由用户在控制面板为某 App 签发 `(access_key_id, secret)`，可撤销、可设有效期、可附 `scope` 收窄到桶 / 前缀 / 动作（§4.4.8）；`access_key_id` 前缀编码 `(user, app)` 以便路由。预签名 URL 的有效期上限 7 天（S3 同）。标准 SDK 在本地算 SigV4，服务端不知道哪些 URL 已签出，所以**预签名 URL 是无状态的**，只能通过撤销其 access key 整体失效，这与 AWS 一致。需要登记、限次、逐条撤销的用平台签发的 `bkt_` 令牌（§4.4.5），两者是不同的东西，文档里不混称。

**范围**：

| 分类 | 操作 |
| --- | --- |
| 第一期实现 | `ListBuckets`、`HeadBucket`、`ListObjectsV2`、`GetObject`（Range、条件头）、`HeadObject`、`PutObject`（含 `Content-MD5`、`x-amz-meta-*`、`If-None-Match: *`、`If-Match`）、`DeleteObject`、`DeleteObjects`、`CopyObject`、`CreateMultipartUpload`、`UploadPart`、`CompleteMultipartUpload`、`AbortMultipartUpload`、`ListParts`、`ListMultipartUploads`、`GetBucketLocation`、`GetObjectTagging` / `PutObjectTagging` |
| 第二期 | `ListObjectVersions`、`GetBucketVersioning` / `PutBucketVersioning`、`GetBucketLifecycleConfiguration` / `PutBucketLifecycleConfiguration`、`PutBucketNotificationConfiguration`、`UploadPartCopy`、`ListObjects`（v1） |
| 明确不支持 | `CreateBucket` / `DeleteBucket`（桶由安装声明；返回 `MethodNotAllowed` 带说明）、ACL 系列（返回 bucket-owner-enforced）、Bucket Policy、Website、CORS 配置（Zone 网关统一处理）、SSE-KMS、Object Lock、Replication、Inventory、Select、Torrent |

**兼容差异清单**（写进对外文档）：

1. ETag 规则同 MinIO（§5.3）；`x-amz-checksum-*` 第一期仅接受不校验。
2. 桶不能由 API 创建。
3. 单段 PUT 上限 5 GiB 同 S3，但服务端会切成 32 MiB chunk。
4. `x-amz-storage-class` 接受 `STANDARD` 并忽略；数据策略是桶级属性，不按对象选，也刻意不叫 storage class 以免与 S3 的 STANDARD / GLACIER 混淆。
5. 时间戳精度毫秒；`LastModified` 为绑定时间，不是内容时间。
6. `Expires` / `Cache-Control` / `Content-Disposition` 等按 S3 语义存为元数据并回传；删除 TTL 用 `x-bucky-expires-at` 扩展头（§5.6）。
7. 预签名 URL 无状态，撤销只能撤 access key；可撤销链接用 `bkt_` 令牌。

### 12.4 SDK

- **日常操作与高级能力分层**：日常三件事是 `put_file(key, 源)`（上传 + 绑定，返回即持久保存）、`get(key)`、`list(prefix)`；跨 App 引用复用（`copy`）、对象图（`object` 桶）、快照、远端引用与预取（P2）都是显式调用的高级能力，默认路径不出现。
- **Rust（buckyos-api）**：`BucketClient { put, get, head, delete, list, copy, batch, snapshot… }`，`put_file(path|reader, key)` 封装"上传 + 绑定"，成功即满足 §6.1 的持久条件。
- **websdk（TypeScript）**：`BucketClient` 同名方法；`ndm_client.uploadFile()` 增加 `bucket`/`key` 选项即走 `auto_bind`，返回值带 `binding`。MessageHub、HomeStation、AIWorkspace 的上传代码据此收敛。
- **Agent 工具**：`buckyos bucket ls|get|put|rm` CLI，`obj://bucket/...` 可直接作为工具参数。

## 13. 索引存储

Bucket 索引是一个系统管理的 RDB Instance：`bucket-index`，归 nfs-server，分区 `user_data`，在 `rdb_mgr` 登记为 sqlite 实例。这使它满足落盘方向文档 §4.2：是可识别、可备份的实例，不是目录里的一个文件。lib 用 rusqlite 按解析出的路径打开它（§14.1）；Postgres 后端留给 `BucketIndexDb` trait 的第二个实现，v1 不做。

```sql
CREATE TABLE buckets (
  bucket_id      TEXT PRIMARY KEY,         -- scope/appid/owner/name
  kind           TEXT NOT NULL,            -- data | object
  data_policy  TEXT NOT NULL,            -- documents | app_data | collected | ambient | temp | cache
  scope          TEXT NOT NULL,
  appid          TEXT NOT NULL,
  owner          TEXT NOT NULL,            -- user id 或 '_'
  name           TEXT NOT NULL,
  versioning     INTEGER NOT NULL DEFAULT 0,
  quota_bytes    INTEGER,                  -- documents 为 NULL（用用户配额）
  default_ttl_secs INTEGER,                -- temp 必填；cache 可选
  orphaned_at    INTEGER,                  -- App 卸载/撤销声明后非空（documents 保留为孤儿桶）
  lifecycle_json TEXT NOT NULL DEFAULT '[]',
  version_seq    INTEGER NOT NULL DEFAULT 0,
  unique_bytes   INTEGER NOT NULL DEFAULT 0,
  key_bytes      INTEGER NOT NULL DEFAULT 0,
  key_count      INTEGER NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL,
  deleted_at     INTEGER
);

CREATE TABLE bindings (
  bucket_id      TEXT NOT NULL,
  key            TEXT NOT NULL,
  version        INTEGER NOT NULL,         -- 来自 buckets.version_seq
  is_current     INTEGER NOT NULL,         -- 1 = 当前版本
  is_delete_marker INTEGER NOT NULL DEFAULT 0,
  obj_id         TEXT,                     -- delete marker 为 NULL
  obj_type       TEXT,                     -- data 桶：cyfile / cydir；object 桶：其他
  size           INTEGER NOT NULL DEFAULT 0,
  etag_md5       TEXT,                     -- S3 兼容 ETag，可为 NULL
  content_type   TEXT,
  metadata_json  TEXT,
  tags_json      TEXT,
  expires_at     INTEGER,
  origin_json    TEXT,                     -- collected / ambient 必填：{author, source, collected_at, proof?}
  last_access_at INTEGER,                  -- cache / ambient 桶的 LRU 依据，读路径更新
  created_at     INTEGER NOT NULL,
  created_by     TEXT NOT NULL,            -- user|app 主体
  PRIMARY KEY (bucket_id, key, version)
);
CREATE UNIQUE INDEX bindings_current ON bindings(bucket_id, key) WHERE is_current = 1;
CREATE INDEX bindings_prefix ON bindings(bucket_id, is_current, key);   -- 前缀列举
CREATE INDEX bindings_expire ON bindings(expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX bindings_obj   ON bindings(bucket_id, obj_id);

CREATE TABLE bucket_pins (                 -- B1 的本地物化
  bucket_id      TEXT NOT NULL,
  obj_id         TEXT NOT NULL,
  ref_count      INTEGER NOT NULL,
  state          TEXT NOT NULL,            -- held | releasing
  updated_at     INTEGER NOT NULL,
  PRIMARY KEY (bucket_id, obj_id)
);
CREATE INDEX bucket_pins_state ON bucket_pins(state, updated_at);

CREATE TABLE bucket_grants (
  grant_id      TEXT PRIMARY KEY,
  bucket_id     TEXT NOT NULL,
  grantee_kind  TEXT NOT NULL,              -- app | user | group | zone_users | agent
  grantee_id    TEXT NOT NULL,
  prefix        TEXT NOT NULL DEFAULT '',   -- '' = 整桶
  actions       TEXT NOT NULL,              -- 逗号分隔：list,read,write,delete
  expires_at    INTEGER,
  granted_by    TEXT NOT NULL,
  created_at    INTEGER NOT NULL
);
CREATE INDEX bucket_grants_lookup ON bucket_grants(bucket_id, grantee_kind, grantee_id);

CREATE TABLE bucket_access_tokens (        -- §4.4.5 平台签发的能力令牌；S3 预签名 URL 不在这里（无状态）
  token_id      TEXT PRIMARY KEY,
  token_hash    TEXT NOT NULL,
  bucket_id     TEXT NOT NULL,
  prefix        TEXT,                       -- 与 key 二选一
  key           TEXT,
  actions       TEXT NOT NULL,
  expires_at    INTEGER NOT NULL,
  max_uses      INTEGER,
  uses          INTEGER NOT NULL DEFAULT 0,
  bind_user_id  TEXT,
  bind_app_id   TEXT,
  created_by    TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  revoked_at    INTEGER
);
CREATE INDEX bucket_access_tokens_bucket ON bucket_access_tokens(bucket_id, expires_at);

CREATE TABLE bucket_quotas (               -- 运行期覆盖；缺行时用安装声明值
  bucket_id        TEXT PRIMARY KEY,
  max_bytes        INTEGER,
  soft_bytes       INTEGER,
  max_objects      INTEGER,
  max_object_bytes INTEGER,
  warn_percent     TEXT NOT NULL DEFAULT '[80,95]',
  last_warned_pct  INTEGER NOT NULL DEFAULT 0,
  set_by           TEXT NOT NULL,
  updated_at       INTEGER NOT NULL
);

CREATE TABLE pool_quotas (                 -- App 池覆盖值；用户池权威在 res_pool，这里只缓存
  pool_kind TEXT NOT NULL, pool_id TEXT NOT NULL,
  max_bytes INTEGER, set_by TEXT NOT NULL, updated_at INTEGER NOT NULL,
  PRIMARY KEY (pool_kind, pool_id)
);

CREATE TABLE bucket_usage_daily (
  bucket_id TEXT NOT NULL, day INTEGER NOT NULL,
  added_bytes INTEGER NOT NULL DEFAULT 0, removed_bytes INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (bucket_id, day)
);

CREATE TABLE bucket_snapshots (
  snapshot_id TEXT PRIMARY KEY, bucket_id TEXT NOT NULL, label TEXT,
  root_obj_id TEXT NOT NULL, key_count INTEGER NOT NULL, unique_bytes INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE s3_access_keys (
  access_key_id TEXT PRIMARY KEY, secret_hash TEXT NOT NULL,
  user_id TEXT NOT NULL, appid TEXT NOT NULL,
  scope_json TEXT,                          -- 可选收窄：{buckets:[], prefix, actions}（MinIO service account 形态）
  expires_at INTEGER, revoked_at INTEGER, created_at INTEGER NOT NULL
);

CREATE TABLE s3_multipart_uploads (       -- S3 兼容面的分段会话；段数据是已入库的 chunk
  upload_id TEXT PRIMARY KEY, bucket_id TEXT NOT NULL, key TEXT NOT NULL,
  metadata_json TEXT, created_by TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE s3_multipart_parts (
  upload_id TEXT NOT NULL, part_number INTEGER NOT NULL,
  chunk_ids_json TEXT NOT NULL, size INTEGER NOT NULL, md5 TEXT NOT NULL,
  PRIMARY KEY (upload_id, part_number)
);
```

`obj_id` 一律存固有的 `type:hex` 拼写（与 `pins.obj_id`、`/ndn/` URL 一致）；`Display` 出来的 base32 只用于展示，不进库、不进协议。这是现有代码里已经踩过的坑。

关于 Ceph 的教训：前缀列举走 `bindings_prefix` 索引的范围扫描，在 SQLite 上百万 key 量级没有问题；真到需要分片的规模，`bucket_id` 本身就是天然的分片键，换 Postgres 后端或按桶分库即可，不需要改协议。

## 14. 部署落点

### 14.1 代码归属：lib 在 cyfs-ndn，nfs-server 做 loader【已定 D3】

沿用既有惯例：协议与存储逻辑放 cyfs-ndn（`named_store`、`fs_meta`、`NamedDataMgrZoneGateway` 都在那里，且不依赖 buckyos 主仓），BuckyOS 身份、RBAC、system_config、kRPC 这些宿主事务放 nfs-server。Bucket 同样一分为二：

| 放 cyfs-ndn：`src/bucket_store` crate | 放 buckyos nfs-server：`src/bucket/` 模块 |
| --- | --- |
| 索引 schema 与事务（§13），rusqlite，与 `named_store` / `fs_meta` 同一套 DB 层 | 解析 `bucket-index` 实例位置（`rdb_mgr` 登记 sqlite 实例，lib 按路径打开），构造 `BucketService` |
| `BucketStore` API：put / get / delete / list / copy / batch / 条件写 / 版本 | kRPC `bucket.*` 处理器：`NfsSessionClaims` → `Principal`，错误码映射 |
| `bucket_pins` 状态机、reconcile、与 `NamedDataMgr` 的 pin / unpin 对接 | RBAC 第一层（默认 policy 写在 `buckyos-api` 的 `rbac_config.rs`），放行后再调 lib 的第二层 |
| 数据策略语义、配额记账与超限判定、生命周期评估、`last_access` LRU 淘汰 | spec 消费：从 system_config 读 `install_config.buckets` / `storage_quota_mib` 懒建桶；admin 覆盖写回 |
| grant 与能力令牌的**数据模型与判定**（主体是不透明字符串） | 令牌**签发与校验模块**（并入 `gt_` 与群 48h 链接，D18）、S3 密钥管理与控制面板 UI |
| 快照 / 恢复 / 导出（复用 ShareMgr 已有的 DirObject 构造，该代码随之下沉到 cyfs-ndn） | `res_pool` 用户池、kmsg 事件、System Notify 告警 |
| HTTP 数据面处理器（`GET/HEAD/PUT /bucket/v1/...`，`buckyos-http-server` 的 `HttpServer` 实现，无鉴权，同 `NamedDataMgrZoneGateway` 的形态） | 鉴权后把请求转交数据面处理器；`boot_gateway.yaml` 路由；Files 只读投影（filedb namespace binding） |
| S3 协议编解码（SigV4 校验给定 secret、XML 请求响应、分段会话表），M3 时作为同仓第二个 crate `s3_gateway` | S3 密钥查找回调、`s3.$zone` 主机名路由 |
| §14.4 的 GC 宿主能力本身（outbox sender、后台 GC、`list_pins_by_owner`）在 `named_store` 补 | nfs-server 作为 Zone 内唯一 GC 宿主，启动这些后台任务 |

边界规则：

- lib 不知道用户、App、Agent 是什么，只认 `Principal(String)` 和宿主传入的"RBAC 已放行"事实；它自己只做档位规则、grant、令牌三类判定。这样 cyfs-ndn 的单测不需要 BuckyOS 身份体系，与 `ndm_zone_gateway` 的测试方式一致。
- lib 不依赖 buckyos 主仓；依赖 buckyos-base 的 `buckyos-kit` / `name-lib` / `buckyos-http-server` 与 `named_store` 已有的依赖面一致。
- 索引后端 v1 只有 sqlite（rusqlite）。"可换 Postgres"落实为 lib 内部的 `BucketIndexDb` trait 边界，不在 v1 实现第二个后端；`rdb_mgr` 登记实例是为了让系统能列出、备份它（落盘方向文档 §4.2），不是为了用 sqlx 访问它。
- 开发节奏同 buckyos-base：本地用 `[patch]` 指向同级 `cyfs-ndn` 目录联调；发布时先推 cyfs-ndn main，再在 buckyos `cargo update -p bucket_store -p named_store`。websdk 的 `BucketClient` 在 buckyos-websdk 仓。

不新开系统服务，符合"能不在系统服务中实现就不实现"。

### 14.2 网关路由

`boot_gateway.yaml`：`/bucket/*`、`/s3/*` 与 `s3.$zone` 主机名转 nfs-server（注意 process-chain 里 `$` 要写 `\$`）。

### 14.3 与现有保留实现的关系

| 现有 | 处置 |
| --- | --- |
| nfs-server `ndm_holdings` + `ndm-hold:<主体>` 24h Lease、`import/finalize` 的 24h Recursive | 保留，作为"持有证明"与两段式之间的 grace；`finalize` 增加 `auto_bind` 后直接接力成桶绑定 |
| nfs-server `ShareMgr`（owner `share:<share_id>`，Recursive；快照把文件复制成 chunk + ChunkList + FileObject + DirObject） | 保留现状；它的快照构造代码是 §10.1 `bucket.snapshot` 的直接基础 |
| msg-center `attachment_retention.rs`（owner `msg-center:<msg_id>`） | 迁到 zone 桶 `zone/msg-center/_/attachments`，key = `<session>/<msg_id>/<n>`；**不给任何用户授权**，附件读取仍由 msg-center 按会话成员关系代为读取或签单 key 只读令牌；私聊永不释放的规则变成"不写 lifecycle" |
| HomeStation `retention.rs`（owner `homestation:<entry>`） | 迁到 `zone/homestation/_/published`，key = `<user>/<entry>/<version>/<n>`；撤回 = 解绑 |
| AIWorkspace `FsObjectStore`（自己的 `objects/` + `chunks/` 内容寻址目录，导入资产时**复制字节**，不 pin） | 第一个迁移对象：改为 `user/aiworkspace/<user>/assets` 桶绑定，去掉第二份存储 |
| OpenDAN Agent 产物（只 put，不 pin，LocalLink ≥ 12 KiB） | 写入 `user/<agent_id>/<owner>/artifacts`，MessageHub 的 DataObj ref 可同时带 `obj://bucket/...` |
| 短期 Lease 使用者：`nfs-import:<uuid>`（1h）、`control-panel:pikg-import:<uuid>`（24h）、`aicc:input:<ns>:<n>`（24h 续租） | 不变；它们是"处理期间持有"，不是保留 |

迁移不是第一期的前置条件；第一期先让新代码走 Bucket，旧的保留表在各自业务改造时替换。两套机制在 named store 里是不同的 pin owner，互不干扰。

另一个现状值得写明：App 运行时的 `RuntimeNamedStore`（Rust）**没有 pin 方法**，只有 Kernel / KernelService 才拿得到带 pin 的本地 `NamedDataMgr`。也就是说，今天一个普通 App 根本没有合法途径表达"我要保留这个对象"。Bucket 是它唯一的保留入口，而不是又一个可选项。

### 14.4 对 cyfs-ndn 的前置依赖

Bucket 把保留表达成 pin，但 pin 要有效果，依赖 named store 侧几件未完成的事。它们不属于本设计，但要同排期：

| 事项 | 现状 | 需要 |
| --- | --- | --- |
| 生产环境运行 GC 循环与 outbox 投递 | 没有任何进程调用 `OutboxSender::spawn`、`run_background_gc`、`expire_pins`、`flush_lru`；级联只在测试里手工投递 | 指定 **一个** 宿主（建议 nfs-server）负责 Zone 内 named store 的后台任务；其他进程只读写不 GC |
| chunk 字节回收 | `gc_round` 只扫 `objects` 表；chunk 只在 `chunk_items` 里有 `owned_bytes`，永远不是 GC 候选 | 实现 GC 设计 §4.1 对 `chunk_items` 的回收 |
| `last_access_time` | 没有调用方 `touch`，LRU 无效 | 读路径（§6.2）调 `touch` |
| 按 owner 列 pin | 只有 `unpin_owner` | `list_pins_by_owner(prefix, cursor)`，供 reconcile |
| 结构化对象的子项遍历 | `parse_obj_refs` 只认 Dir / File / ChunkList | 接入 `MsgObject` 等标准对象（D1 的前提） |
| Skeleton 硬屏障 | GC 设计 §0.3 已定义，实现状态待核实 | `ambient` 档依赖它；按 `origin` 拉取依赖 CYFS 协议 pull 路径（NDM 网关下载路径目前未实现） |
| 多进程共用 SQLite | nfs-server、msg-center、homestation、control-panel、aicc、aiworkspace、repo-service、node-daemon、cyfs-gateway 各自打开同一组 `named_store.db` | 不是 Bucket 的阻塞项，但 GC 宿主唯一化后应逐步收敛为经 nfs-server 的 proxy 访问 |

在这些落地前，Bucket 仍然**正确地表达**保留与释放（pin 行是对的），只是释放不会变成磁盘空间。这一点要写进 M1 的验收说明，避免把"解绑了但磁盘没变"当成 Bucket 的缺陷。

## 15. 一致性与故障模型

### 15.1 与 RDB 业务写入的衔接（落盘方向文档 §6.6 的"先后与失败"）

推荐模式：**先绑定、后写 RDB、key 用业务 id**。

```text
1. 上传内容（Lease 保护）
2. bucket.put(key = "msgs/<msg_id>/att/0", obj_id, if_none_match="*")   # 幂等
3. RDB 写业务行，引用 key（或同时存 obj_id）
失败在 2 之后 3 之前 → 桶里多一个无主 key；应用按前缀 list 对账，或给这类 key 设 expires_at
失败在 1 之后 2 之前 → Lease 到期自动回收，无痕
```

另一种模式"先写 RDB pending，再绑定，再 confirm"（msg-center 当前做法）也成立，SDK 两种都支持；但第一种不需要应用维护 pending 状态。

### 15.2 原子批量

`bucket.batch` 在单桶内对 ≤100 个 put/delete 做一个索引事务，要么全成要么全败。pin 的发起仍在事务后异步补齐（§7.2），这对可见性没有影响：索引是权威，pin 是物化。

### 15.3 故障

| 故障 | 行为 |
| --- | --- |
| pin 成功、索引事务未提交（崩溃） | 多余的 pin，reconcile 第 2 步按 owner 比对后 unpin |
| pin 失败 | `put` 失败，不写索引；调用方重试 |
| 内容不在库（上传未完成、已被逐出） | `put` 失败 `content_not_available`；ENOSPC 只会发生在上传 ①，不会发生在绑定 ② |
| 索引库损坏 | 从最近 snapshot 的 manifest 重建 bindings；无快照则只能重建 pin（扫描 `bucket:<id>` owner），key 丢失 |
| nfs-server 崩溃于两段式之间 | 同 15.1 |

## 16. 已知风险

1. **单写者索引**：nfs-server 一个进程持有 `bucket-index`，集群多 OOD 时需要把它变成 Postgres 后端或单实例服务。协议不变，但第一期不验证多节点。
2. **S3 客户端的 ETag 假设**：极少数客户端把 ETag 当 MD5 做完整性校验（老版本 s3cmd、某些备份工具的 multipart 校验）。MinIO 策略覆盖了绝大多数，但要在文档里说明。
3. **大目录快照**：百万 key 的桶一次 snapshot 要构造很大的 DirObject；cyfs-ndn 的"大容器"方案（`doc/Reviews/对象大容器需求草案.md`）落地前，快照按前缀分层多个 DirObject 嵌套即可。
4. **跨用户去重不省流量**（§6.4）：这是刻意的安全取舍。
5. **级联收敛延迟**：解绑后 chunk 的真正回收依赖 outbox 收敛与水位，用户看到的"释放"不是即时的物理释放。
6. **named store 当前既不跑 GC 也不回收 chunk 字节**【现状】：详见 §14.4。GC 落地之前，Bucket 能正确表达保留，但释放不产生磁盘空间。这是 cyfs-ndn 侧的待办，不是本设计的缺陷，但要在排期上一起看。
7. **D1 的级联缺口**：结构化对象的子项在 cyfs-ndn 接入遍历之前不受桶保护（§3.3）。
8. **持久条件依赖同进程**：§6.1 的"pin 失败即 put 失败"成立的前提是 nfs-server 与 named store 同进程、pin 是本地事务。将来 named store 分进程时，要么 pin 变成同步 RPC 并保持失败即失败，要么退回 pending 状态机并把 put 改成异步任务，二者都要改 §6.1 的承诺措辞。

## 17. 分期

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| **M0 索引与状态机** | cyfs-ndn `bucket_store` crate：schema、put/get/delete/list/copy/batch、条件写、`bucket_pins` 状态机、reconcile、数据策略语义、两级配额记账与超限行为、grant 判定 | 单测 + 隔离 NamedDataMgr 下验证 pin / unpin 对称、reconcile 修复人为破坏 |
| **M1 接入 nfs-server** | kRPC `bucket.*`（含 quota / grant / check_access）、RBAC 资源 `obj://bucket/*` 默认策略、HTTP `GET/HEAD/PUT /bucket/v1/...`、`install_config.buckets` + `storage_quota_mib` 的 spec 合成与懒建桶、`import/finalize` 的 `auto_bind`、能力令牌签发校验模块（并入 `gt_` 与群链接）、documents 的 Files 只读投影 | DV：浏览器上传直达桶，`<img src>` 按 key 读取；admin 看到每应用保留量 |
| **M2 SDK 与首批调用方** | Rust / websdk `BucketClient`、`uploadFile({bucket,key})`、Agent CLI；AIWorkspace 资产或 Agent 产物作为第一个调用方迁入 | 一个真实应用零自写保留代码 |
| **M3 S3 兼容面** | SigV4、密钥管理 UI、第一期操作集、multipart、兼容差异文档 | rclone sync / restic backup / aws-cli 冒烟；Litestream 把 SQLite 复制进桶 |
| **M4 生命周期、版本、快照、事件** | lifecycle 任务、versioning、对象 TTL、snapshot / restore / export、kmsg 事件 | 备份集 = RDB dump + snapshot roots，在另一台 DV 上恢复出可用业务状态 |
| **cyfs-ndn 并行项** | §14.4：GC 宿主唯一化并真正运行、chunk 回收、`touch`、`list_pins_by_owner`、结构化对象子项遍历 | 隔离 store 下解绑 → 收敛 → `gc_round` 后 `owned_bytes` 下降 |
| **P2** | prefetch、`ambient` 的按 `origin` 自动拉取（v1 的 ambient 只锁描述，内容有则用、无则 404）、evicted 感知、Postgres 后端验证、大容器快照 | — |

## 18. 裁决清单（D1–D18 定于 2026-10-25，D19–D20 定于 2026-10-26）

| # | 事项 | 裁决 | 落点 |
| --- | --- | --- | --- |
| D1 | Binding 的值 | 桶分 `data` / `object` 两种 kind，不混装 | §3.3 |
| D2 | 索引权威 | RDB 权威 + DirObject 快照；不用 fs_meta overlay | §3.1、§13 |
| D3 | 宿主进程 | nfs-server 内模块 + 独立 `bucket_store` crate | §14.1 |
| D4 | S3 兼容面排期 | M3，紧随 M1 / M2 | §17 |
| D5 | 版本与 TTL | versioning 默认关（`documents` 档例外，见 D12）；对象级 `expires_at` 作为扩展 | §5.5、§5.6 |
| D6 | 持有证明 | 绑定需持有或读权；跨主体不秒传 | §6.4 |
| D7 | 桶的来源 | 声明了才有；不补 `default`，每个桶显式选档 | §4.3 |
| D8 | URL 形态 | `obj://bucket/<id>/<key>` 与 `/bucket/v1/...` | §4.2 |
| D9 | 既有保留迁移 | 不作为 M1 前置；各业务改造时替换，AIWorkspace 先行 | §14.3 |
| D10 | 公开访问 | 桶不可公开；永久公开只经 ShareContentMgr / HomeStation | §4.4.8 |
| D11 | documents 投影 | 只读投影到 `cyfs:///home/<user>/Apps/<App>/<桶>/`，用户删除 = 解绑 | §4.5.4 |
| D12 | documents 版本 | 默认 `versioning: true` | §4.5.2 |
| D13 | cache 档保留 | 完全不 pin，纯 class 0 | §4.5.2 |
| D14 | 用户池配额 | `res_pool` 存储项落地前只统计不拒绝 | §8.2 |
| D15 | 写令牌 | 允许，限前缀、≤7 天、不含 `list` / `delete` | §4.4.5 |
| D16 | admin 权限 | 对用户桶默认只有 `list` `read` `manage` | §4.4.3 |
| D17 | App 池配额声明 | 声明 `app_data` / `temp` 桶时 `storage_quota_mib` 必填 | §4.3 |
| D18 | 令牌模块 | `gt_` 访客令牌与群 48h 链接并入 `bucket_access_tokens` 的签发校验模块 | §4.4.5 |
| D19 | 第二、三档的落法 | 新增 `collected`（Recursive、必带 `origin`、与 documents 同级备份）与 `ambient`（Skeleton 锁描述、内容按 origin 再拉），原 `archive` 并入 ambient | §4.5 |
| D20 | ambient 的 v1 范围 | v1 只做 Skeleton 保留与"有则用、无则 404"，按 `origin` 自动拉取放 P2（依赖 NDM 下载路径） | §17 |

## 19. 修订记录

| 版本 | 日期 | 内容 |
| --- | --- | --- |
| v0.1 | 2026-10-25 | 初稿：模型、接口、分期、D1–D10 |
| v0.2 | 2026-10-25 | 定 D1 / D3 / D7；新增 §4.5 数据策略档位 |
| v0.3 | 2026-10-25 | 补 §4.4 权限模型与 §8 两级配额接口 |
| v0.4 | 2026-10-25 | §18 全部裁决定案；§14.1 代码归属 cyfs-ndn lib + nfs-server loader |
| v0.6 | 2026-10-26 | 内容网络视角：数据策略按"我创作 / 别人创作我认真收集 / 别人创作最好能有"三档重排，新增 `collected`、`ambient`，并入 `archive`；Binding 增 `origin`；Files 投影增 `Collected/`；D19–D20 定案 |
| v0.5 | 2026-10-26 | 按 review：zone 只表达服务级归属、默认不共享，新增 `create` 动作与创建者覆盖规则；PUT 成功 = 持久保存三条件，状态机去掉 pending；`Expires` 保持 HTTP 缓存含义，删除 TTL 用 `x-bucky-expires-at`；预签名 URL 无状态，与 `bkt_` 令牌区分；快照 manifest 权威 + tree 派生、冲突改名、快照计费、备份时间点契约；版本成本措辞收窄；`storage_class` 改名 `data_policy`；新增 §3.4 三概念 |

## 附录 A：与落盘方向文档待定清单的对应

| 方向文档 §10.4 议题 | 本文位置 |
| --- | --- |
| Bucket 模型：每应用资源组织、兼容范围、命名与权限 | §4、§12.3 |
| 上传链路：Bucket ID 使用、返回引用、失败与业务写入衔接 | §6.1、§15.1 |
| 应用容量统计口径 | §8 |
| 对象归属：多应用引用时谁有保留/删除权 | §3、§6.3、§7.4（各桶独立承诺，删除只撤自己） |
| 引用完整性：发现 RDB 与 Object Body 中的引用 | §10.2、D1（桶内对象随快照走；桶外仍需遍历） |
| 数据生命周期：RDB 引用与保留范围不一致 | §15.1（key 用业务 id 对账）、§9 |
| GC 与缓存：永久删除、解除保留、缓存淘汰 | §7.4、§7.5 |
| URL 与工具适配 | §4.2、§6.2、§12.4 |
| 备份与恢复 | §10 |
