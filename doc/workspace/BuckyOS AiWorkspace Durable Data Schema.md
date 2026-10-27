# BuckyOS AiWorkspace Durable Data Schema

日期：2026-10-26（Named Bucket 迁移 P1-1，`notepads/bucket-migration-todo.md` §3.3）

## 1. Overview

`aiworkspace`（`src/frame/aiworkspace`）保存用户的 AI Workspace：每个 Workspace 一个文件夹（`doc.sqlite` 可移植文档、`local.sqlite` 本部署状态），对象本体（资产字节、物化快照、导出包）在 named store，保留关系由应用对象桶的绑定表达。协议见 `src/frame/aiworkspace/README.md`，对象格式见 `BuckyOS AiWorkspace 内核架构改进.md` §13（FORMAT 1.0）。

服务不再有自己的内容寻址目录（`objects/`、`chunks/`）、`retentions` 表、mark-sweep 回收器、上传 staging（`staging/<up_id>.part`、`upload/<id>` 路由、`import-receipts/`）和本机路径下载（`export/`、`replica/` 路由）。

## 2. Data Classification

| 数据 | 类别 | 去处 |
| --- | --- | --- |
| Workspace 文档（`workspaces/<ws>/doc.sqlite`）、本部署状态（`local.sqlite`） | 持久 | 文件夹 + rusqlite（不在本次迁移内，另起 TODO P3-6：Workspace 按 App 拿自己的 RDB Instance 与桶） |
| 资产字节（上传、导入、Source 抓取、许愿格结果文件、程序源码） | 持久 | named store + `assets` 桶 |
| 物化快照与生成版本（`jobj` / `cyfile` / `cydir` 闭包） | 持久 | named store + `objects` 桶 |
| 导出包 `.bcanvas`、离线副本 sqlite、固定版本发布、待导入的包 | 可丢弃（7 天） | named store + `exports` 桶（`temp`） |
| `trash/<ws>-<ms>`、`trash/restore_<id>`、`.tmp-*` | 持久 30 天 / 处理中 | 文件夹（同 Workspace 文件夹，P3-6） |
| 许愿格 `runs/<run_id>/{work,xllm}`、`cache/llm_map/` | 可丢弃 | 文件系统：执行器工作目录（Deno 按路径读写）与可重建缓存 |
| 导出 / 副本 / 导入时的临时文件 | 可丢弃 | `$TMPDIR/buckyos/aiworkspace/`，函数返回前删除 |

## 3. Storage Strategy

### 3.1 桶

三个桶都在 `scheduler/src/system_config_builder.rs::add_aiworkspace` 的 `spec_config.buckets` 声明（定义在 `buckyos_api::aiworkspace_bucket_decls()`），`scope: user`，属主是 **Workspace 的 owner**（`local.sqlite.local_meta.owner`）；`storage_quota_mib = 16384`（`AIWORKSPACE_STORAGE_QUOTA_MIB`，约束 `app_data` + `temp`）。服务是 frame，对 `user/aiworkspace/*` 全权，绑定的持有证明由它对 `obj://ndm/objects` 的读权满足。

| 桶 | kind / data_policy | key | 绑定的对象 | 谁解绑 |
| --- | --- | --- | --- | --- |
| `user/aiworkspace/<owner>/assets` | `data` / `app_data` | `<ws>/<object_id>` | 资产的 FileObject（`cyfile`） | 暂存过期未引用（24 h）；历史资产过保留期（`history_retention_days`，默认 30）；`ws.purge_deleted` 立即；`ws.clear_staged`；Workspace 删除 30 天后 |
| `user/aiworkspace/<owner>/objects` | `data` / `app_data` | `<ws>/snapshot/<snapshot_id>`、`<ws>/version/<epoch>/<seq>` | **保留集 DirObject**（见 3.2） | `ws.delete_checkpoint` / `ws.release_retention{snapshot}`；Workspace 删除 30 天后 |
| `user/aiworkspace/<owner>/exports` | `data` / `temp`（7 天） | `<ws>/export/<export_id>.bcanvas`、`<ws>/replica/<replica_id>.sqlite`、`<ws>/fixed/<content_root>` | 包、副本库（FileObject，可多 chunk）、固定版本的保留集 | TTL；`ws.release_retention{export}` |
| 导入者自己的 `user/aiworkspace/<user>/exports` | 同上 | 浏览器任选（Desktop 用 `import/<random>.bcanvas`） | 浏览器上传的包（`ndm` 上传 `bind`） | `ws.import` 成功后解绑（消费）；Agent 等没有自己 aiworkspace 桶的主体传 `obj_id`（NDM 网关以其 token 判可读），不经此桶 |

`objects` 声明为 `data` 而不是 `object`：快照、内容根、实体内容对象是 `jobj`，named store 的级联只展开 Dir / File / ChunkList（桶设计 §3.3 现状、§16.7），绑定一个 `jobj` 只保住它自己。所以每个保留根绑定的是一个 **保留集 DirObject**：`cydir`，成员是该根闭包里的每个对象（`<hex>.<type>` → ObjId，时间字段为 0，`canonical::dir_object`），级联由此覆盖每个 `jobj`、`cyfile`（及其 chunk）、`cydir` 与它引用的资产。一个根一个绑定，释放就是删一个 key，不需要服务自己做引用计数或标记清除。

### 3.2 保留集

| 根 | 闭包 |
| --- | --- |
| 快照 `<ws>/snapshot/<snapshot_id>` | 快照根 `jobj`、内容根、`entities` 文件、每个实体内容对象及其 `records` / `doc` 文件、`cydir`（目录实体）、实体引用且在库的资产、谱系里的来源快照（`lineage.forked_from.snapshot`，在库时） |
| 生成版本 `<ws>/version/<epoch>/<seq>` | 该 Commit 中每个 `entity.set_derived` 实体的内容对象及其文件、`cydir`、资产 |
| 固定版本 `<ws>/fixed/<content_root>` | 该实体内容对象及其文件（发布方被读固定版本时写入，7 天） |

每次物化（checkpoint、export、fork 源、导入校验）都写一个快照行并绑定其保留集，与原先“每个快照都是保留根”一致。

### 3.3 写入路径（谁 pin）

| 写入 | 位置 | 绑定 |
| --- | --- | --- |
| 服务端字节暂存（Source 抓取、许愿格结果与程序源码、包内资产、测试 fixture） | `Workspace::stage_asset` | 同函数 `assets` `<ws>/<object_id>` |
| 浏览器单文件上传 | Desktop `ndm.ensureUploaded({ bind })` → `asset.import_object { bucket_id, key }` | finalize 时 `auto_bind` 进上传者自己的 `assets`；服务 `bucket.get` 校验后绑到 owner 的 `<ws>/<object_id>`，不同则解绑上传者的 key |
| 浏览器文件夹 / 多文件导入 | `ndm.ensureUploaded`（NDM 24 h Lease）→ `asset.import_object { obj_id }` | 服务先经 NDM 网关以调用者 token 校验可读，再绑定 |
| 物化（`StoreSink`） | `Workspace::materialize_notes` | 同函数绑定快照保留集 |
| 生成版本（Commit 内物化） | `Workspace::write_commit` | SQLite 事务提交前绑定 `<ws>/version/<epoch>/<seq>`，事务失败解绑 |
| 固定版本读取 | `Workspace::public_content` | `exports` `<ws>/fixed/<root>` |
| 导出包、副本库 | `Workspace::export` / `replica_bootstrap` | 临时文件 → `store_file` + 绑定 `exports`，随即删除临时文件 |
| 包导入解出的对象 | `export::open_package` | 导入校验快照的保留集 + 资产暂存（同一 `Service::import` 调用）；导入失败时解出的对象无主，由 named store GC 回收（桶设计 §6.1 ① 的语义），包本身仍在导入者的 `exports` 里直到 TTL |

### 3.4 读取

- 服务内：一律 ObjId → named store（`runtime.get_local_named_store()`；独立模式 `--named-store-config` 指向与 nfs-server 相同的 store）。
- 浏览器资产：保留代读 `GET /kapi/aiworkspace/asset/<ws>/<object_id>`。权限在 Workspace 授权（协作者、`public` 访客、External、run token），这些主体对 owner 的桶没有权限；与 msg-center 的 Q5 同理。
- 浏览器下载（导出、副本）：RPC 返回 `{ bucket_id, key, token }`，`token` 是服务签的单 key 只读令牌（1 h），Desktop 用 `/bucket/v1/<bucket_id>/<key>?token=` 读。

## 4. Schema Definitions（`storage_schema_version` 5）

只列与对象和保留有关的表；其余表见 `store/src/schema.rs`。

```
Table: assets（doc.sqlite，进入离线副本）
| Column         | Type    | Nullable | Description |
| object_id      | TEXT PK | NO  | 资产 FileObject 的 ObjId（`cyfile:<hex>`） |
| media_type     | TEXT    | NO  | 由内容嗅探 |
| size           | INTEGER | NO  | 字节数 |
| first_seq      | INTEGER | NO  | 首次被引用的 Commit |
| ended_seq      | INTEGER | YES | 最后一个活引用结束的 Commit；0 = 已清除 |
| bucket_key     | TEXT    | YES | `assets` 桶里的 key（`<ws>/<object_id>`） |
| bucket_version | INTEGER | YES | 绑定版本；NULL = 已解绑（undo / 版本恢复答 RESOURCE_RECLAIMED） |

Table: staged_assets（local.sqlite）：object_id、media_type、size、principal、expires_at、bucket_key、bucket_version（NOT NULL）。
Table: snapshots（doc.sqlite）：snapshot_id、content_root、seq、kind、created_at、created_by、bucket_key、bucket_version（NULL = 已释放，`{epoch, seq}` 引用答 VERSION_NOT_RETAINED）。
Table: entity_versions（doc.sqlite）：… kind、created_at、bucket_key、bucket_version（`generated` 行指向其 Commit 的版本保留集；`checkpoint` 行经快照保留，为 NULL）。
local_meta.show_clones：正在运行的演示克隆 id（JSON 数组，按文件夹是否存在计数，`MAX_CLONES`）。
```

删除：`retentions` 表、`upload_sessions` 表、`usage.exports` 类别（导出受 `temp` 桶 TTL 与 App 池配额约束）。

## 5. Lifecycle

- 每小时的 `Service::sweep`（替代原 `Service::gc`）：每个非克隆 Workspace 解绑过期未引用的暂存与过保留期的历史资产；`trash/` 中满 30 天的文件夹先解绑它持有、且同 id 的活 Workspace 不再持有的 key，再删除文件夹。
- 演示克隆复制 `doc.sqlite` / `local.sqlite`，引用源的绑定；删除克隆时只解绑克隆自己建立、源不持有的 key。
- 卸载 / 删用户：`app_data` 与 `temp` 桶按 data_policy 删除（P0-5）。

## 6. Compatibility

开发阶段不兼容旧数据：`storage_schema_version` 4 的文件夹被拒（`UNSUPPORTED_VERSION`），旧 `objects/`、`staging/` 目录不再读也不再创建；DV 用 `uv run src/start.py --all` 全新安装。
