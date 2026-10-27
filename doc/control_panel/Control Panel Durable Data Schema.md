# Control Panel — Durable Data Schema

## 1. Overview

Service: `control-panel`（`src/frame/control_panel`，Desktop WebUI 在 `src/frame/desktop`）。

Control Panel 是 Zone 写操作的收口服务（[Control_Panel_Service.md](Control_Panel_Service.md)）。它的结构化记录全部在 system-config KV；对象本体（已安装 App 的包、Agent 模板 PIKG、安装中的 PIKG、账号壁纸）在 named store，由本服务声明的应用对象桶表达保留（[应用对象桶 Bucket 设计](../arch/应用对象桶%20Bucket%20设计.md)）。本服务不在 `data/`、`srv/` 下自建业务目录。

App 安装协议见 [doc/App 安装协议.md](../App%20安装协议.md)；桶的 kRPC / 数据面见 nfs-server `bucket.*`。

## 2. Data Classification

| 数据 | 分类 | 说明 |
| --- | --- | --- |
| 用户 / Agent / 设备 / App spec / RBAC / Desktop UI state 等 system-config 键 | Durable | 结构化记录，system-config 是真相源（Control_Panel_Service.md §7） |
| Agent 模板记录 `users/{owner}/agent_templates/{app_id}` | Durable | 指向 `agent_templates` 桶的 `(key, version)` |
| 已安装 App 的包（AppDoc、Package Meta、payload） | Durable | `package_objects` / `packages` 桶，卸载即解绑 |
| CLI 安装的 Agent 模板 PIKG | Durable | `agent_templates` 桶 |
| 账号壁纸图片 | Durable | `user/control-panel/{user}/wallpapers` 桶 |
| PIKG staging（finalize 之后、安装之前） | Disposable（≤ 7 天） | `pikg_staging` temp 桶，到期失效 |
| PIKG 导入期间的保留 | Disposable（24h） | named store Lease `control-panel:pikg-import:<uuid>` |
| `cache/control_panel/pikg_staging/incoming/{id}.pikg` | Disposable（24h） | 外部工具在 OOD 本机投放的待导入文件；finalize 成功即删除 |
| `$BUCKYOS_ROOT/tmp/control-panel/` | Disposable | 处理期间文件：解出的 payload、构造中的 Agent runtime PIKG，用完即删 |
| App 发布临时 tar、诊断 zip、日志 zip | Disposable | OS temp / cache，处理期间 |
| `data/home/{user}/Desktop/*.lnk`、`*.widget`，`Downloads/` | 用户文件夹 | 保留文件系统语义（用户以文件夹组织的数据） |
| 用户 / Agent / Zone 头像 | Durable | system-config data URL（≤128 KiB），不进桶 |

## 3. Storage Strategy

### 3.1 Structured Data

system-config KV（不另开 RDB Instance）：记录里只存 ObjId 或 `(bucket_id, key, version)`，不存本机路径。

### 3.2 Unstructured Data → 桶

声明在 `src/kernel/scheduler/src/system_config_builder.rs::add_control_panel`（取自 `buckyos_api::control_panel_bucket_decls()`），`storage_quota_mib = CONTROL_PANEL_STORAGE_QUOTA_MIB`（256 GiB，App 池 `control-panel@_` 与 `control-panel@{user}` 各自计算）。

| bucket_id | kind / data_policy | key | 值 | 谁 pin / 何时解绑 |
| --- | --- | --- | --- | --- |
| `user/control-panel/{user}/wallpapers` | `data` / `app_data`，`quota_mib` 128 | `{sha256[..32]}.{jpg\|png\|gif\|webp\|avif}` | 图片 FileObject（浏览器 `PUT /bucket/v1/...` 直传） | 桶绑定；Desktop 上传后只留最新 3 张（含当前），其余由前端 `bucket.delete` |
| `zone/control-panel/_/agent_templates` | `data` / `app_data` | `{owner}/{app_id}/{pikg_digest}.pikg` | 模板 PIKG FileObject（从 staging 绑定，不复制字节） | 桶绑定；新版本登记成功后解绑旧 key |
| `zone/control-panel/_/packages` | `data` / `app_data` | `{owner}/{app_id}/{version}/{content_id}` | 包 payload：包裹 content chunk / ChunkList 的 FileObject（名为 `{sub_pkg}.tar.gz`），或 payload 本身就是 FileObject | 桶绑定；Acquire 结束绑定已下载内容，Prepare 物化 PIKG 后绑定全部；升级 Activate 成功后解绑其它版本；卸载解绑全部 |
| `zone/control-panel/_/package_objects` | `object` / `app_data` | `{owner}/{app_id}/{version}/{obj_id}` | AppDoc（在库时）与 Package Meta 对象 | 同上 |
| `zone/control-panel/_/pikg_staging` | `data` / `temp`，`default_ttl_days` 1，`quota_mib` 2048 | staging handle | 上传 / 投放 / 生成的 PIKG FileObject；绑定 metadata `staging` = `PikgStagingMetadata` JSON；`expires_at` = 1h（inspect）/ 24h（install）；被安装任务租用时延长到租用起 7 天 | 桶绑定；释放 / 任务结束 / 过期后解绑（`gc` 与桶生命周期双重回收） |

- 包桶按 **App 实例**（owner + app_id）分 key：两个用户安装同一版本各自绑定，任一方卸载不影响另一方（同一 ObjId 去重只占一份内容）。
- 读取：安装驱动、Agent 模板、预装都用 `PikgReader::open_named` 从 named store 读 PIKG（`Read + Seek` 视图，不落本地文件）；Desktop 用 `bucketObjectUrl(ref)` 直接作为 CSS / `<img>` 来源。
- 持有证明：control-panel 是 frame 服务，对 `obj://ndm/objects` 有读权（§6.4 第三条），可绑定它写入或用户上传到 named store 的对象。

### 3.3 App 卸载与孤儿桶

卸载任务（`app.uninstall/v1`）在实例停止后：
1. 写回 `Deleted` spec 时清空 `spec_config.buckets`（已卸载的 App 不再声明桶，nfs-server 的 spec 同步不会再创建或接回它们）；
2. 解绑 `packages` / `package_objects` 中该实例的全部 key；
3. 调 nfs-server `bucket.release_app {appid, owner, keep_app_data}`：`documents` / `collected` 转孤儿桶（`orphaned_at`），`app_data` 在 `data_disposition = retain` 时转孤儿、否则删除，`temp` / `cache` / `ambient` 删除。重装同一 App 时声明恢复，孤儿桶自动接回。

Agent 删除走同一卸载任务（`retain`）：Agent App 的 `artifacts`（documents）留作孤儿，`scratch`（temp）删除。孤儿桶在 Desktop「设置 → 存储空间」列出，用户可删除（documents / collected 需 sudo 会话）。

### 3.4 Exception（文件系统）

- `cache/control_panel/pikg_staging/incoming/`：外部工具按路径投放，Control Panel 只读入 named store 后删除。
- `$BUCKYOS_ROOT/tmp/control-panel/`：`cacl_file_object` 需要本地文件路径的处理期间文件。
- 用户 home 的 `Desktop/`、`Downloads/`：用户文件夹语义。

## 4. Schema Definitions

### System-config: `users/{owner}/agent_templates/{app_id}`

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| app_did | DID | 模板 AppDID |
| app_doc_object_id | ObjId | 模板 AppDoc |
| version | string | AppDoc.version |
| digest | string | 整个 PIKG 的 sha256 hex |
| bucket_key | string | `agent_templates` 桶内 key |
| bucket_version | u64 | 绑定版本；读取按 `(key, version)` 精确定位 |
| installed_at | u64 | 秒 |

### Bucket binding: `pikg_staging/{handle}`

Binding metadata `staging`：`PikgStagingMetadata`（`buckyos_api::app_install`）JSON —
`schema_version, handle, owner_user_id, owner_app_id, zone_did, pikg_digest, size, purpose (inspect|install), created_at, expires_at, leases[]`。Binding `expires_at` = `metadata.expires_at * 1000`。

### Install task data

`CandidateHandle` 只记 `staging_handle` 与 `pikg_digest`（不记本机路径）；后续 Stage 经 handle 重新解析出 FileObject。

## 5. Schema Version

- system-config 记录沿用各自 `schema_version`（`APP_INSTALL_SCHEMA_VERSION` 等）。
- staging metadata 带 `schema_version = APP_INSTALL_SCHEMA_VERSION`；不符即视为无效 handle。
- 桶的 key 方案由本文档定义；改变 key 方案需要声明新桶。

## 6. Upgrade Compatibility Strategy

No-compat（pre-release）：旧版本的 `data/home/{user}/.local/share/control-panel/wallpapers/`、`data/srv/control-panel/agent_templates/`、`cache/control_panel/pikg_staging/{metadata,*.pikg}` 不再读写，DV 全新安装（`start.py --all`）。

## 7. Extensibility Rules

- 桶的 `kind` / `data_policy` / `scope` 创建后不可改；新增用途声明新桶。
- `agent_templates` 记录与 staging metadata 可加字段（serde default）。
- 包桶 key 的四段结构冻结：卸载 / 升级按前缀 `{owner}/{app_id}/` 与版本段工作。

## 8. Query Patterns

| 查询 | 方式 |
| --- | --- |
| 打开已安装模板 | KV 记录 → `bucket.get(key, version)` → ObjId → named store |
| 卸载 / 升级清理包 | `bucket.list_objects(prefix = "{owner}/{app_id}/")` 分页 |
| staging 配额、去重、租约回收 | `bucket.list_objects` 全量（桶上限 2 GiB，条目少） |
| 壁纸保留最新 3 张 | 前端 `bucket.list_objects` 按 `created_at` 排序 |
| 存储空间页 | `bucket.list {scope: user}` 过滤本人，按 appid 汇总 `unique_bytes` |
