# rdb_mgr 数据分区改进需求

- 状态：已实施（R1-R7、R9-R10；R8 作为 P2 单独实施）
- 版本：implementation 1
- 目标读者：buckyos-api（rdb_mgr）、Scheduler（system_config_builder）、Control Panel 安装器、
  所有持有 RDB instance 的内核/Frame 服务
- 相关文档：
  - `doc/path_usage.md`：RootFS 各目录的语义、备份和卸载规则（分区划分的唯一依据）
  - `doc/task_mgr/task-mgr 2.0.md` §15.2：第一个明确需要多分区的调用方（Storage Domain）
  - `doc/arch/internet级别的OLTP.md`：为什么服务状态优先回归 RDB

## 1. 背景

`buckyos-api/src/rdb_mgr.rs` 现在只做一件事：把 `(appid, owner_user_id, instance_id)` 解析成一个
sqlx 连接串。配置来自安装期写入 system_config 的 spec：

```text
services/{appid}/spec            .spec_config.rdb_instances[instance_id]   # Kernel/Frame service
users/{user}/apps/{appid}/spec   .spec_config.rdb_instances[instance_id]   # AppService
```

```rust
pub struct RdbInstanceConfig {
    pub backend: RdbBackend,                 // sqlite | postgres
    pub version: u64,
    pub schema: HashMap<RdbBackend, String>, // 每 backend 一份 DDL
    pub connection: String,                  // 模板，支持 $appdata / $instance
}
```

`connection` 为空时按 `sqlite://$appdata/{instance}.db?mode=rwc` 生成，`$appdata` 解析为
`$buckyos_root/data/{user}/{appid}` 或 `$buckyos_root/data/{appid}`。当前在册的 instance：
`task-mgr-main`、`task-dispatcher-main`、`repo-service-main`、`msg-center-main`、
`aicc-usage-log`，全部落在 `data/` 下。

**问题：`data/` 不是唯一正确的位置。** RootFS 已经把持久化目录按“这份数据要不要跟着用户走”
分开（`doc/path_usage.md`）：

- `$buckyos_root/data/**`：用户/Zone 数据，跟随用户数据备份/恢复迁移。
- `$buckyos_root/local/**`：本机系统数据，卸载即删除，备份恢复/换机/软重置后通常整体丢失
  （`system_config` 的 sled store、kmsg 队列、node_daemon finder cache 都在这里）。
- `$buckyos_root/data/cache/**`：可随时重建的派生数据。
- `$buckyos_root/storage/**`：与物理磁盘绑定的内核基础设施存储，卸载不删除。

但凡通过 rdb_mgr 拿连接串的服务，今天只能把所有状态一律写进用户数据区。第一个被卡住的是
TaskMgr 2.0（`doc/task_mgr/task-mgr 2.0.md` §15.2/§15.4）：它需要 `task-mgr-main` 落在 `data/`、
`task-mgr-main` 的另一份库落在 `local/`，两边同一份 schema；`task-dispatcher-main` 这种纯运行期调度状态
整体应该在 `local/`。这不是 TaskMgr 特有的诉求——消息、日志、索引、投递队列都有同样的问题。

## 2. 目标

1. RDB instance 能显式声明自己属于哪个**数据分区（Data Partition）**，rdb_mgr 按分区解析出
   不同的物理位置。
2. 同一个 instance 声明（同 backend / 同 version / 同 DDL）可以在多个分区各有一份物理库，
   调用方用 `(instance_id, partition)` 取到对应连接串。
3. 分区语义是自描述的：备份、卸载、软重置和诊断工具只读 spec 就能判断某个物理库该不该被
   带走、该不该被删除。
4. 完全向后兼容：现有 5 个 instance 不改 spec、不改代码行为，DB 文件不搬家。

## 3. 非目标

1. 不实现跨分区事务、跨分区 join 或跨分区复制。
2. 不实现分布式 RDB（dRDB）；`storage` 分区只是给未来留出位置。
3. 不改变 `$appdata` 现有的路径含义（见 §9 开放问题里的已知不一致，本次不顺手修）。
4. 不实现备份/恢复工具本身，只提供它需要的分区元信息。
5. 不改变“安装期决定 backend 和连接串、运行期只读 spec”这条现有边界，rdb_mgr 依然不写
   system_config，也不从本地 manifest 里种子化配置。

## 4. 数据分区定义

| Partition | 逻辑含义 | 内核/Frame 服务 base dir | 备份恢复 | 卸载/软重置 |
| --- | --- | --- | --- | --- |
| `user_data`（默认） | 用户/Zone 数据，服务替用户保存的长期事实 | `$buckyos_root/data/{appid}`（App：`data/{user}/{appid}`） | 跟随用户数据走 | 按 `doc/path_usage.md` 的规则保留或删除 |
| `local` | 本机系统数据，只对当前 host-node 有意义 | `$buckyos_root/local/{service}`（`$buckyos_service_local_data`） | 不带走，恢复后可能整体缺失 | 随 `local/` 一起删除 |
| `cache` | 可随时重建的派生数据（索引、物化视图、统计） | `$buckyos_root/data/cache/{service}`（App：`data/cache/{user}/{appid}`） | 不带走 | 删除 |
| `storage` | 内核基础设施持久存储，为 dRDB 预留 | `$buckyos_root/storage/{appid}` | 不带走 | 不随普通卸载删除 |

约束：

- 分区只回答“数据保存在哪里、会不会跟着用户走”，不改变 backend、schema 和 SQL 语义。
- `local`、`cache`、`storage` 的数据允许整体丢失；调用方必须能在空库上自举。
- App（AppService）只允许 `user_data` 和 `cache`；`local` 和 `storage` 仅限 Kernel/Frame 服务，
  rdb_mgr 必须拒绝 App spec 里出现这两个分区，避免 App 直接往 host 的本机目录写库。

## 5. 功能需求

### R1 分区枚举

```rust
#[serde(rename_all = "snake_case")]
pub enum RdbPartition { UserData, Local, Cache, Storage }
```

`Default` 为 `UserData`。取值集合是稳定契约，新增取值等价于新增存储位置，属于独立设计变更。

### R2 instance 声明分区

`RdbInstanceConfig` 增加：

```rust
#[serde(default = "default_partitions")]
pub partitions: Vec<RdbPartition>,   // 默认 vec![UserData]
```

- 老 spec 不含该字段时反序列化为 `[user_data]`，行为与今天完全一致（R10）。
- 列表里的每个分区各对应一个独立的物理数据库，共用同一个 `backend`、`version` 和 `schema`。
- 列表不得为空、不得重复；出现重复或空列表时 `get_rdb_instance*` 直接报错，不做静默纠正。

### R3 连接串模板与占位符

现有占位符 `$appdata`、`$instance` 保留，新增：

| 占位符 | 展开为 |
| --- | --- |
| `$partdata` | 当前解析分区的 base dir（§4 表格） |
| `$partition` | 分区名（`user_data`/`local`/`cache`/`storage`），用于 postgres 的 database/schema 命名 |

规则：

- `$appdata` 等价于 `partition = user_data` 时的 `$partdata`。当一个 instance 声明了非
  `user_data` 分区却在 `connection` 里写 `$appdata` 时必须报错，禁止“配置自相矛盾”地把系统
  数据写进用户数据区。
- 多分区 instance 的 `connection` 必须使用 `$partdata`/`$partition`（或留空走默认生成），否则
  两个分区会解析到同一个物理库——这种情况必须报错而不是静默共用。

### R4 默认连接串按分区生成

`connection` 为空时：

- sqlite：`sqlite://$partdata/{instance}.db?mode=rwc`（保持现有 `mode=rwc` 与路径归一化行为）。
- postgres：仍然报错，要求安装期显式给出连接串；多分区 postgres instance 必须在连接串里用
  `$partition` 区分 database 或 schema。

### R5 API

```rust
// 兼容入口：instance 只声明一个分区时可用；声明多个分区时返回 partition_ambiguous，
// 提示调用方改用带分区的接口。
pub async fn get_rdb_instance(appid, owner_user_id, instance_id) -> Result<RdbInstance>;

// 新入口：显式指定分区；分区未在 spec 的 partitions 里声明时报错。
pub async fn get_rdb_instance_in(appid, owner_user_id, instance_id, partition) -> Result<RdbInstance>;

// 元信息：给备份、卸载、诊断和 Control Panel 用，不需要打开数据库。
pub async fn list_rdb_instances(appid, owner_user_id)
    -> Result<Vec<(String /*instance_id*/, RdbPartition, String /*connection*/)>>;
```

`RdbInstance` 增加 `partition: RdbPartition` 字段，让调用方拿到的结果自描述（日志、错误信息和
版本校验都需要它）。

### R6 base dir 解析集中且不可逃逸

- 分区 base dir 的解析必须只有一个实现，同时覆盖“解析自己的 instance”和“跨 app/service 解析”
  两条路径。现在 `resolve_appdata_dir` 里 self 分支走 `runtime.get_data_folder()`、跨 app 分支
  手工拼路径，两者容易漂移，改进后必须由同一个函数产出。
- 解析完成后必须校验：sqlite 文件的绝对路径必须落在该分区 base dir 之内。带 `../` 逃逸、绝对
  路径覆盖或解析结果落到别的分区时一律报错。这是防止“系统数据被写进用户备份”“用户数据被
  写进 local 然后在恢复时静默丢失”的最后一道闸。

### R7 缺失目录自举

`local`/`cache`/`storage` 分区在恢复、换机或首次运行后可能整个目录都不存在。rdb_mgr 必须像现在
`ensure_sqlite_dir` 一样创建父目录，让调用方拿到的连接串可以直接打开一个空库。调用方不需要
区分“首次安装”和“恢复后丢失”。

### R8 version 校验（P2）

`RdbInstanceConfig.version` 目前返回给调用方但没人使用，每个服务各自实现 no-compat 重建。建议
（不阻塞本需求）：由 rdb_mgr 提供统一的 `open_rdb_pool(...)` helper，负责 apply schema、在库内
写入 `__rdb_meta(instance_id, partition, version)` 并在版本不一致时明确失败。同一 instance 的多个
分区必须版本一致，只升级其中一个必须启动失败。

### R9 错误语义

新增稳定错误：`partition_not_declared`、`partition_ambiguous`、`partition_not_allowed_for_app`、
`partition_path_escape`、`partition_placeholder_conflict`。错误信息必须带上 appid、instance_id、
分区名和解析后的路径，方便现场排障。

### R10 兼容性

- 不改 spec 的情况下，`task-mgr-main`、`task-dispatcher-main`、`repo-service-main`、
  `msg-center-main`、`aicc-usage-log` 的连接串逐字节不变。
- 已部署 Zone 升级后不得出现数据库文件搬家；任何需要搬家的变化必须是一次显式的、带迁移步骤的
  独立改动。
- Scheduler `system_config_builder` 和 Control Panel 安装器写 spec 时可以逐步补 `partitions`
  字段；没补的按默认值工作。

## 6. 配置示例

TaskMgr 2.0：Task Core 一份声明覆盖两个分区，Dispatcher 只在 `local`。

```json
{
  "rdb_instances": {
    "task-mgr-main": {
      "backend": "sqlite",
      "version": 8,
      "partitions": ["user_data", "local"],
      "connection": "",
      "schema": { "sqlite": "CREATE TABLE task(...)" }
    },
    "task-dispatcher-main": {
      "backend": "sqlite",
      "version": 4,
      "partitions": ["local"],
      "connection": "",
      "schema": { "sqlite": "CREATE TABLE dispatch_record(...)" }
    }
  }
}
```

解析结果：

```text
get_rdb_instance_in("task-manager", None, "task-mgr-main", UserData)
  -> sqlite:/opt/buckyos/data/task-manager/task-mgr-main.db?mode=rwc
get_rdb_instance_in("task-manager", None, "task-mgr-main", Local)
  -> sqlite:/opt/buckyos/local/task-manager/task-mgr-main.db?mode=rwc
get_rdb_instance("task-manager", None, "task-dispatcher-main")
  -> sqlite:/opt/buckyos/local/task-manager/task-dispatcher-main.db?mode=rwc
```

postgres 多分区示例：

```json
"connection": "postgres://svc:pw@pg:5432/taskmgr_$partition"
```

## 7. 验收与测试

1. 老 spec（无 `partitions`）解析出的连接串与改进前逐字节一致；覆盖 self 和 cross-app 两条路径。
2. 每个分区的 base dir 与 §4 表格一致，`BUCKYOS_ROOT` 改变时跟着变，不出现写死的绝对路径。
3. 多分区 instance：同一 instance_id 在两个分区解析出不同路径；`get_rdb_instance` 不带分区调用
   返回 `partition_ambiguous`；请求未声明的分区返回 `partition_not_declared`。
4. 占位符冲突：非 `user_data` 分区的 `connection` 里出现 `$appdata`、或多分区 instance 的
   `connection` 里没有 `$partdata`/`$partition` 时报错。
5. 路径逃逸：`sqlite://$partdata/../../data/evil.db` 被拒绝。
6. App spec 声明 `local`/`storage` 被拒绝。
7. 目录不存在时自动创建并能打开空库；删除整个 `local/` 后，`user_data` 分区的库仍可正常打开。
8. Windows 路径归一化（现有 `normalize_sqlite_url` 用例）在每个分区上都成立。

## 8. 实施影响

- `buckyos-api/src/rdb_mgr.rs`：`RdbPartition`、`RdbInstanceConfig.partitions`、
  `RdbInstance.partition`、`$partdata`/`$partition`、集中的 base dir 解析与路径校验、新 API。
- `buckyos-api/src/app_mgr.rs` / `app_doc.rs`：`rdb_instances` 序列化随之扩展（`#[serde(default)]`
  保证老数据可读）。
- `kernel/scheduler/src/system_config_builder.rs`：沿用各服务的 default RDB config；这些 config
  已显式声明 `partitions`，生成的 service spec 会携带分区。
- `frame/control_panel/src/app_install_deployer.rs`：App 安装时校验 App 只声明允许的分区。
- 调用方：`task_manager/src/task_store.rs` 当前显式打开 `user_data`（TaskMgr 按 StorageDomain
  路由两份 store 时复用同一 API）、`task_manager/src/dispatcher/dispatch_db.rs` 改到 `local`；
  `msg_box_db.rs`、`repo_db.rs`、`aicc_usage_log_db.rs` 保持 `user_data` 并显式化。
- `doc/path_usage.md`：把“RDB instance 按分区落盘”写进目录职责表，卸载/软重置章节同步说明
  各分区的库怎么处理。

## 9. 实施决策与后续项

1. `$appdata` 为保证已部署数据库不搬家，继续使用 `data/{appid}`（App 为
   `data/{user}/{appid}`），不改成 `$buckyos_service_data` 的 `data/var/{service}`。
2. `cache` 统一使用 `data/cache/{service}`；App 按 owner 隔离为
   `data/cache/{user}/{appid}`。不沿用 `runtime.get_cache_folder()` 的旧 `cache/**` 路径。
3. 当前 spec 没有 `disk_id`，且 `doc/path_usage.md` 把卸载保留的内核基础设施定义在
   `$buckyos_root/storage`，因此 `storage` 先解析为 `storage/{appid}`。未来 dRDB 引入按物理盘
   放置时，需要独立增加 disk/res_pool 元信息和迁移设计，不能复用旧的
   `get_lcoal_storage_folder()` 把 storage 数据写入 `local/`。
4. 暂不增加 `zone` 分区；postgres 连接串已经能表达远端集群，等 dRDB 设计明确后再评估。
5. R8 的 `open_rdb_pool`、`__rdb_meta` 与统一 version 校验仍是 P2，单独实施；本次不改变五个服务
   现有的 schema apply/no-compat 重建逻辑。
