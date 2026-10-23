# Beta2.2 删除旧版本迁移与兼容分支 TODO

> 状态：已实施（2026-10-23，未提交）。实施记录、保留依据、验证结果与未验证项见 §7。
>
> 日期：2026-10-23。审查基线：`buckyos/main@71cd54d9b`，发布子模块 `src/publish@64b75e5`，工程版本 `0.7.0 / Beta2.2`。
>
> 要求：Beta2.2 是 breaking change 版本，删除为旧版本保留的无意义迁移代码，包括早期 Beta 数据和 Beta2.2 开发期历史 schema 的转换、旧格式读取回退、旧布局自动清库重建。不要将这些逻辑改名、搬到另一个模块或保留为备用迁移工具。

## 1. 完成后的统一行为

- 全新安装、空存储：直接生成当前完整格式，不经过历史版本逐级升级。
- 当前格式：正常打开、重启、恢复业务，已有有效数据保持不变。
- 不支持的旧格式、未知格式或未来版本：明确报错并保留现场，不自动转换、补写版本号、搬目录或删除业务数据。缺少版本元数据的非空旧库不能被当成全新库；纯缓存按现有缓存失效策略处理，不迁移。
- 需要重置开发环境时使用已有明确的重装/重置操作；服务启动、数据库打开、普通读取命令不得隐式执行清库。
- 复用现有版本和结构校验，不为本次删除工作新增通用迁移框架。不降低现行 schema 版本，也不复用旧版本号掩盖结构变化。
- 当前版本的初始化、合法默认值、崩溃恢复和业务操作继续保留。是否删除取决于实际用途，不能只按 `migrate`、`legacy`、`upgrade` 名称批量删代码。

## 2. P0：删除有数据破坏风险的旧版本处理

### 2.1 macOS 安装器旧 home 目录迁移

入口：[buckyos_postinstall](../src/publish/macos_pkg/scripts/buckyos_postinstall)，`setup_user_home_symlink()`；生成入口：[make_local_osx_pkg.py](../src/publish/make_local_osx_pkg.py)。

现状：把 `$BUCKYOS_ROOT/home` 复制到交互用户的 `~/my_pi`，再删除旧目录并建立软链。`ditto` 失败被忽略，后续仍执行 `rm -rf`，存在直接丢数据风险。

- [x] 删除整个旧 home 搬迁与兼容软链流程，以及安装时的调用；不要只修复复制错误处理后继续保留迁移。
- [x] 安装路径按当前 [bucky_project.yaml](../src/bucky_project.yaml) 的 `data/home/`、`data/srv/`、`storage/` 规则处理，清理模板和生成脚本中的旧路径假设。
- [x] 保留当前安装器的数据保留规则；发现旧 `$BUCKYOS_ROOT/home` 时不得复制、删除或改为软链。
- [x] 验证脚本重新生成后旧迁移不会再次出现；验证全新安装及当前版本覆盖安装。

### 2.2 Agent Notebook 打开旧库自动删表

入口：[agent_notebook.rs](../src/frame/agent_tool/src/agent_notebook.rs)，`AgentNotebook::open()`、`ensure_schema()`；CLI：[agent_tool_cli_dev/src/lib.rs](../src/frame/agent_tool_cli_dev/src/lib.rs)，`run_agent_notebook_blocking()`。

现状：发现 `notebooks.owner_user_id`、`notebook_items.actor_kind` 或 `metadata` 等旧列，就删除 8 张 Notebook 表。CLI 在分派命令前打开数据库，因此 `agent-notebook list/read` 也可能触发。

- [x] 删除旧列识别后 `DROP TABLE` 的重建分支，仅保留当前完整建库定义和必要的格式校验。
- [x] 不支持的已有库打开失败并保留数据；不能把删表改成清空目录、换名后自动创建空库等变体。
- [x] 回归 `list/read/append`：新库和当前库正常，旧库拒绝，旧笔记及关联表不变。

### 2.3 TaskMgr / Dispatcher 启动清库

入口：[task_store.rs](../src/kernel/task_manager/src/task_store.rs)，`rebuild_if_legacy_layout()`；[dispatch_db.rs](../src/kernel/task_manager/src/dispatcher/dispatch_db.rs)，`apply_schema()`。

现状：TaskMgr 检测不到 `task.storage_domain` 时删除 7 张表；Dispatcher 检测到旧 `dispatch_record.operation` 时删除 9 张表。两者都在正常启动路径，注释中的 dev/DV 并非执行限制。

- [x] 删除两处旧布局自动删除和重建分支，以及仅为它们保留的旧布局常量、日志和辅助代码。
- [x] 在对已有库执行写入型 DDL 前完成当前格式校验；失败时不修改任务、事件、备注、派发、投递尝试、注册和游标。
- [x] 保留当前 TaskMgr User/System 分区及版本一致性检查；同步核对 Dispatcher 当前版本常量，清除过期 v2→v3 描述。
- [x] 回归新库初始化、当前库重开、旧库拒绝及分区版本不一致；不要继续用“旧记录被清空且启动成功”作为正确预期。

## 3. P1：删除主仓库中的数据迁移实现

### 3.1 AICC 历史 schema 升级链

入口：[storage/mod.rs](../src/frame/aicc/src/storage/mod.rs)，`MIGRATIONS`、`ROUTE_TRACE_FILTER_COLUMNS`、`USAGE_ATTRIBUTION_COLUMNS`、`AiccStorage::migrate()`。

现状：打开数据库时执行 v2/v3 的 `ALTER TABLE`，从旧 route trace JSON 回填过滤字段，并更新版本号。全新数据库也依赖这些步骤补齐字段。

- [x] 先把当前完整列和索引合并进初始 DDL：包括 `fallback_applied`、`warning_count`、`operator_id`、`attribution_json` 及对应索引。
- [x] 删除历史迁移列表、旧 JSON 回填、逐版本执行和版本推进逻辑；打开流程改为当前格式初始化/校验。
- [x] 区分真正的空库与缺失版本记录的非空库，删除把任意无版本库当作 v0 继续升级的行为。
- [x] 保留当前表的必要初始化、quota 表、默认 visibility 和正常 retention；仅因旧表改名而保留的兼容说明及专用测试删除，不增加旧表回收或数据搬迁。
- [x] 同步 SQLite/Postgres 路径及共享 RDB 声明；验证新库直接可用、当前库可重开、旧库拒绝且数据不变。
- [x] 更新 [AICC Runtime Durable Data Schema](../doc/aicc/aicc_runtime_durable_data_schema.md)：取消开发期 schema 1→2/3 迁移例外，修正文档 schema 仍为 2、源码已为 3 的差异。最终版本以实施时源码为准。

### 3.2 Msg Center 联系人加载迁移

入口：[contact_mgr.rs](../src/frame/msg_center/src/contact_mgr.rs)，`load_owner_store()`、`migrate_contact_payload()`、`save_owner_store()`。

现状：读取时将 `groups` 合并为 `tags`，将 `access_level: friend` 改为 `allow`，并为符合条件的联系人补 `friend_state`。后续普通消息权限检查也可能将整个 owner 的转换结果落盘。

- [x] 删除 `migrate_contact_payload()` 及调用，只读取当前 Contact 格式。
- [x] 删除专为旧 `groups`、旧 access level 和旧 friendship 语义保留的解码、默认推断及转换测试；可识别的旧格式应明确拒绝，不能被 serde 忽略字段后静默接收并覆盖。
- [x] 保留当前协议允许的可选字段和默认值，避免将合法当前数据误判成旧数据。
- [x] 回归联系人加载、修改、消息权限检查和保存；失败时不得删除或重写该 owner 的联系人。
- [x] 更新 [Message Center Durable Data Schema](<../doc/message_hub/Message Center Durable Data Schema.md>) 中“no-compat 但加载时迁移联系人”的例外；MsgBoxDb 的当前建库和必需表初始化继续保留。

### 3.3 Agent Memory 显式迁移工具

入口：[agent_memory/mod.rs](../src/frame/agent_tool/src/agent_memory/mod.rs)、[agent_memory/migrate.rs](../src/frame/agent_tool/src/agent_memory/migrate.rs)、[agent_tool_cli_dev/src/lib.rs](../src/frame/agent_tool_cli_dev/src/lib.rs)。

- [x] 删除 `AgentMemory::migrate()`、`migrate.rs`、`MigrationReport`、旧 envelope/operation 类型、2.10 日志转换及 legacy 备份目录专用逻辑。
- [x] 删除 `agent-memory migrate` 的命令解析、分派、帮助、输出、示例和相关测试，不保留隐藏命令或离线转换脚本。
- [x] 清理运行时及文档中“运行 migrate 后重试”的失效提示；旧格式明确报告不受支持。
- [x] 保留当前格式的初始化、`open/verify/compact/repair_derived`、日志重放、锁和物化索引恢复；独立检查 minor-version 只读兼容，移除仅服务于历史格式的分支。
- [x] 核对 Agent 工具手册、内置 `cmd_skills`、CLI 分发内容及 libopendan 消费方，保证没有入口继续宣传迁移能力。

### 3.4 Desktop 旧布局转换

入口：[layout.ts](../src/frame/desktop/src/models/layout.ts)，`migrateToSlotModel()`；[DesktopUIDataModel.ts](../src/frame/desktop/src/models/DesktopUIDataModel.ts)；[desktopFolder.ts](../src/frame/desktop/src/models/desktopFolder.ts)。

现状：旧 `x/y` 布局被补成 slot 模型；但当前默认布局也只生成 `x/y`，不能直接删函数及调用。

- [x] 先让当前布局创建入口直接产生必要的 slot 模型字段，保持网格变化时坐标和 slot 的正常计算。
- [x] 删除读取旧布局后补字段、再保存为新格式的兼容流程，以及相应旧格式 fixture/测试。
- [x] 对不支持的旧浏览器布局采用当前缓存版本/默认布局策略，不读取并转换历史字段；保留当前用户布局正常持久化。
- [x] 回归全新用户、当前布局重载、桌面/移动端切换、resize、拖拽及分页，确保清理没有破坏当前布局规范化。

## 4. P2：关联仓库与旧协议兼容一并清理

这些任务在对应源码仓库实施；不能只修改 BuckyOS 的 rootfs、bundle 或安装产物。依赖声明继续遵循 `main` 分支约定。

- [x] **buckyos-websdk**：删除 [runtime.ts](../../buckyos-websdk/src/runtime.ts) 中旧 node identity 内嵌 JWT 和旧 `node_private_key.pem` 读取回退，统一当前身份格式；同步调用方与测试。
- [x] **buckyos-websdk**：删除 [account.ts](../../buckyos-websdk/src/account.ts) 中旧 `buckyos.account_info` 复制到应用隔离 key 的代码，核对无调用的旧 API 并清理；删除 [sdk_core.ts](../../buckyos-websdk/src/sdk_core.ts) 中仅为旧 `zone_host_name` 留下的处理，保留当前缓存初始化和重新探测。
- [x] **buckyos-websdk**：核对并删除 [verify-hub-client.ts](../../buckyos-websdk/src/verify-hub-client.ts) 中仅支持旧扁平登录响应的回退，统一当前 `user_info` 响应与生产端。
- [x] **cyfs-ndn/package-lib**：删除 [env.rs](../../cyfs-ndn/src/package-lib/src/env.rs) 中 `enable_strict_mode → enable_meta_db` 的旧配置映射，模板、生成器、node-daemon 和测试统一使用当前字段。
- [x] **buckyos-base/name-client**：删除 [addr_rtt_db.rs](../../buckyos-base/src/name-client/src/addr_rtt_db.rs) 的 `LegacyPersistedEntry` 回退及旧记录重写。该路径属于可选持久化 RTT 缓存，默认 BuckyOS 未发现启用；不应为清理引入新的持久化功能。
- [x] **cyfs-gateway/SN/BNS**：删除 [sn_bns_store.rs](../../cyfs-gateway/src/components/bns-client/src/sn_bns_store.rs) 的旧交易状态改写及历史补列、[bns-indexer/sqlite.rs](../../cyfs-gateway/src/components/bns-indexer/src/sqlite.rs) 的旧 `asset_owner` 补列/回填、[relay_mgr.rs](../../cyfs-gateway/src/components/cyfs-sn/src/relay_mgr.rs) 的旧 relay 表搬迁；当前初始 DDL 一次建齐，旧库拒绝。保留正常交易恢复和 relay 业务迁移。
- [x] **安装器**：核对 [Windows installer](../src/publish/win_pkg/installer.nsi.in) 中旧 HKLM 路径和旧 Windows service 安装形态的兼容清理，删除仅针对废弃版本的分支，保留当前安装的正常停止、替换和启动步骤。
- [x] **kernel 旧协议读取**：核对 `system_config.rs` 的旧 ZoneDocument hostname 回退、`msg_queue.rs` 的旧 URN 兼容、`zone_did_resolver.rs` 的旧 websdk 返回形态，联动消费者后删除纯历史分支；不改变当前有效协议本身。

## 5. 明确保留的功能与范围边界

- 当前 schema 的 `CREATE TABLE IF NOT EXISTS`、合法默认字段、当前种子和配置生成，不因名称或注释里出现 migration 就删除。
- 当前版本的任务/事务/消息重试、幂等、崩溃恢复、日志重放和派生索引重建。
- AIWorkspace 的 `table.migrate_field` 等用户业务操作；DID 的 `Migrated/migration_target` 身份语义；relay 切换和 NamedStore 存储拓扑 epoch 处理。
- HomeStation retention 的正常收敛和补 pin。若有历史专用分支，仅删除该分支，不能删除承担当前故障恢复的主流程。
- 软件包替换、当前版本覆盖安装、运行中配置更新及用户明确发起的重装，不等同于旧数据格式迁移。
- `cyfs-ndn/fs_meta` 虽有迁移实现，但当前 BuckyOS 依赖链未接入；本任务不扩大成该独立组件的重构。
- 本轮约束本次 breaking change 的历史兼容清理，不取消 [Beta2.2 数据兼容清单](../doc/arch/beta2.2-data-compatibility.md) 对正式基线发布后数据契约的要求。

## 6. 联动、验证与完成条件

- [x] 各模块当前格式定义、源码、schema 常量、共享类型、SDK、CLI、模板、模块文档和内置手册保持一致；清理已失效的迁移说明，不用文档继续承诺已删除的行为。
- [x] 删除以“旧数据升级成功”或“旧库自动清空成功”为目标的历史测试；对涉及持久数据的组件保留或补齐必要的拒绝旧格式及不破坏数据验证。
- [x] 验证矩阵覆盖：全新空存储、当前格式已有数据、历史格式已有数据、缺少版本的非空存储，以及有版本机制组件的未来版本。新/当前格式正常，其余明确失败或按纯缓存策略重建，不改写业务旧数据。
- [x] 迁移专用模块、CLI 命令、死代码和旧 fixture 实际删除；最终搜索 `migrate/legacy/ALTER TABLE/DROP TABLE` 的剩余命中逐项说明当前用途，不要求机械清零。
- [x] 分别运行受影响 Rust 包的相关测试：`aicc`、`msg_center`、`agent_tool`、`agent_tool_cli_dev`、`task_manager`；联动协议或 runtime 时扩展至对应消费者。
- [x] 运行 Desktop 布局相关测试和构建；涉及 SDK/分发包时验证源码构建与打包；涉及安装器时检查生成产物并在对应平台验证。无法运行的平台明确记录未验证。
- [x] 按改动影响完成仓库要求的 Cargo/构建检查；验收环境使用隔离目录和临时数据库，不在真实用户数据上试跑旧格式处理。
- [x] 提交说明列出删除项、仍保留逻辑的依据、受影响仓库/手册、验证命令与结果。全部条目完成后再将本文状态改为已实施。

## 7. 实施记录（2026-10-23）

改动均在各仓库 `main` 工作区，未提交、未推送。涉及仓库：buckyos（含 `src/publish` 子模块）、buckyos-websdk、cyfs-ndn、buckyos-base、cyfs-gateway。

### 7.1 删除项与当前行为

| 条目 | 删除 | 现在的行为 |
| --- | --- | --- |
| 2.1 macOS 安装器 | `buckyos_postinstall` 中的 `setup_user_home_symlink()` 及调用（复制到 `~/my_pi`、删除 `$BUCKYOS_ROOT/home`、建软链） | 按 `bucky_project.yaml` 重新生成 `data_paths` 块（补上 `etc/node_executor_acl.json`、`data/home/`、`data/srv/library/`、`data/srv/publish/`、`security/`），只在目标缺失或为空目录时复制默认值；旧 `$BUCKYOS_ROOT/home` 不再被复制、删除或改成软链。重跑 `sync-macos-scripts` 不会再生成迁移代码。`preinstall` 的 modules 块没有入库重生成，构建时 `sync_macos_scripts` 会按 manifest 自动更新 |
| 4 Windows 安装器 | `installer.nsi.in` 读取 HKLM `BUCKYOS_ROOT` / `BuckyOSServiceDir` 的回退，安装、卸载、组件卸载三处的 `sc stop/delete buckyos`；`src/uninstall.py` 的 HKLM 候选与清理及随之失效的 `only_if_matches` | 只识别当前 HKCU 安装形态；正常停止（计划任务、Run 项、`stop.ps1`）、替换和启动步骤不变 |
| 2.2 Agent Notebook | 发现旧列就删 8 张表的分支、`table_has_column` | `ensure_schema` 在 IMMEDIATE 事务内执行：没有任何表时一次建齐；已有库的表、列、索引签名必须与当前 DDL（内存库建出的签名）一致，否则返回 `NotebookError::UnsupportedSchema`（错误码仍为 `storage_error`）。WAL 切换移到校验之后，被拒的库文件逐字节不变。CLI 各动词都先经过 `AgentNotebook::open`，同样拒绝 |
| 2.3 TaskMgr | `rebuild_if_legacy_layout()`、按 `storage_domain` 列推导布局版本 | 没有任何 TaskMgr 表才算空库，空库在一个事务内建表并写 `task_store_meta.schema_version = 8`；已有库必须带版本 8，缺版本、旧版本、未来版本都在任何 DDL 之前失败；User/System 分区版本一致性检查保留 |
| 2.3 Dispatcher | v2（`dispatch_record.operation`）删 9 张表重建、过期 v2→v3 注释 | 库里没有版本行：没有表时在事务内建 schema v4；已有库 6 张表必须齐全且列名与当前 SQLite DDL 一致，否则失败 |
| 3.1 AICC | `MIGRATIONS`、`ROUTE_TRACE_FILTER_COLUMNS`、`USAGE_ATTRIBUTION_COLUMNS`、旧 route trace JSON 回填、`migrate()`/`apply_script()`、表改名兼容说明与专用测试 | 4 个列和 2 个索引并入初始 DDL；没有任何 `aicc_*` 表时在一个事务内建 `SCHEMA` + `QUOTA_SCHEMA` 并写版本 3；已有库必须记录版本 3，旧版本、未来版本、有表无版本都返回 `StorageError::UnsupportedSchema`，不补列、不删表。默认 visibility 种子和 retention 保留。SQLite/Postgres 走同一流程 |
| 3.2 Msg Center | `migrate_contact_payload()`、`groups`→`tags` 合并、`friend`→`allow`、`friend_state` 推断及转换测试 | 只读当前 Contact 格式；payload 带 `groups` 或旧 access level（`friend`）时该 owner 的 store 加载失败，所有操作返回错误，不重写、不删除行。MsgBoxDb 启动时应用必需 DDL 的逻辑保留 |
| 3.3 Agent Memory | `AgentMemory::migrate()`、`migrate.rs`（`MigrationReport`、旧 envelope/operation 类型、2.10 转换）、`LEGACY_DIR`、`agent-memory migrate` 解析/分派/帮助/测试、“run migrate”提示 | `NeedsMigration` 改为 `Unsupported`；主版本不一致拒绝读写；`occasions.jsonl` 有内容却缺 `meta.json` 的根目录也拒绝，不当作空根目录初始化；拒绝时不改任何文件。libopendan 映射为 `MemoryError::Unavailable("unsupported format: …")` |
| 3.4 Desktop | `migrateToSlotModel()` 及 4 处调用 | 默认排布在 `buildDesktopLayout` 创建时直接写 `slotIndex`（输入新增当前网格 `cols`/`rows`），另有 `preferredPage`/`placementType`/`seq`；保存的位置照原样带回；resize 时仍由 `refreshSlotIndexes` 按新网格重算。旧布局走现有缓存键版本（`buckyos.layout.<ff>.v3`），不读取、不转换历史字段 |
| 4 websdk | `runtime.ts` 旧 node identity 内嵌 JWT 与 `node_private_key.pem` 回退、v1 身份类型与 `newNodeIdentityConfig`；`account.ts` 旧 `buckyos.account_info` 复制及无调用的 `getLocalAccountInfo`/`LOGIN_EVENT`；`sdk_core.ts` 清理旧 `zone_host_name` 和 `did:web` id 推导 hostname；`verify-hub-client.ts` 旧扁平登录响应与 `normalizeLoginResponse` | 只认 `buckyos.node_identity.v2` 与 `security/<device host>/authentication.private.pem`，非 v2 身份文件明确报错；登录只读当前 `user_info` 响应（与 verify-hub 生产端一致）。`dist/`、`cli/dist/` 已重新构建。Desktop `signOut.ts` 去掉清理旧 key |
| 4 kernel 旧协议 | `system_config.rs` 用 DID 推导空 hostname 的回退；`msg_queue.rs` 含 `::` 名字原样透传 | `zone_document_hostname` 只规范化文档发布的 hostname；`calc_queue_urn` 只对绝对路径透传。调用方核对过，不会传完整 URN |
| 4 package-lib | `enable_strict_mode → enable_meta_db` 映射、冲突检查、deprecated 警告及迁移测试 | 只认 `enable_meta_db`；buckyos 源码、模板、生成器、node-daemon 均不使用该字段，仅改了一处安装流程文档 |
| 4 name-client | `LegacyPersistedEntry` 回退与 `from_legacy` | 只按当前 `AddressStats` 解码；持久化缓存的版本缺失或不等于当前值时清空缓存表再写版本（纯缓存失效） |
| 4 cyfs-gateway | `sn_bns_store` 补列与三条旧交易状态改写；bns-indexer `asset_owner` 补列/回填；relay `migrate_relay_assignments_schema` | 初始 DDL 一次建齐并写版本标记；已有库在写入型 DDL 前校验（列集合 / `tx_state_model` / `user_version`），不符就报错且不改数据；正常交易恢复与 relay 业务迁移保留。bns-client 新增 `BnsRegistryError::UnsupportedSchemaVersion` |

文档同步：`doc/task_mgr/task-mgr 2.0.md` §15.7、`doc/aicc/aicc_runtime_durable_data_schema.md` §4.6/§5/§6、`doc/aicc/aicc_token_quota.md`、`doc/message_hub/Message Center Durable Data Schema.md`、`doc/opendan/Agent Memory 认知管理需求.md`（A.6/A.8/附录 B）、`doc/arch/beta2.2-data-compatibility.md`（AICC 现状、Memory 版本 3.0）、`doc/app_service/BuckyOS App安装流程.md`、四处仍写 `node_private_key.pem` 的 SDK/架构文档；关联仓库 `cyfs-ndn/doc/TODO-PackageEnv-MetaDb-Mode.md`、`buckyos-base/doc/addr-rtt-db.md`。

cmd_skills：内置手册和 `src/tools/buckyos-agent` 不引用被删的 `agent-memory migrate` 或 websdk API；Jarvis 行为模板里的 `agent-notebook` 命令仍是当前用法，无需修改。

### 7.2 保留的逻辑及依据

- MsgBoxDb 每次启动应用必需 DDL：§3.2 明确保留的当前建库逻辑。
- AICC `quota.rs` 的 `legacy_sql`：统计没有 quota attempt 的 usage 事件，是当前账本口径，不是格式兼容。
- Agent Memory 次版本、语言或分词器不同时只读挂载（TD-24）：3.0 是首个 3.x，这条只对未来次版本生效。`Legacy CLI load` 是 CLI `load` 动词的当前实现。
- `zone_did_resolver.rs` 的 `self` 别名返回裸 ZoneDocument：当前 websdk 浏览器端探测 zone host 仍在使用。
- relay 的 `Migrating` 状态、`start_relay_migration` 等是 relay 切换业务。
- AICC 共享 RDB 声明的 `version: 1` 未改：AICC 不读取 `instance.version`，只更新了声明注释。

### 7.3 剩余搜索命中

在受影响模块中搜索 `migrat|legacy|ALTER TABLE|DROP TABLE`：`msg_box_db.rs` 文件头对 beta2.2 拆表的说明，以及“过期服务 DDL 仍补齐必需表”测试里的 `DROP TABLE`；`quota.rs` 的 `legacy_sql`；`agent_memory` 的 `Legacy CLI load` 注释与对应测试名；本次新增的拒绝测试里用于构造坏库的 `DROP TABLE`；`layout.ts` 文件头的 “dead-zone migration” 措辞（无对应代码）。关联仓库剩余命中均为 relay 业务迁移，以及 redb 缓存整表重写/失效。

### 7.4 验证

- `cargo test -p task_manager`：45 通过（新增未来版本、无版本旧库、Dispatcher 旧布局、列变化、当前库重开；分区版本不一致改用显式版本 7）。
- `cargo test -p agent_tool`：274 通过（Notebook 当前库重开、旧布局拒绝、缺表拒绝且文件不变；Memory 2.10 与无 meta 的日志拒绝且文件不变、次版本只读）。
- `cargo test -p agent_tool_cli_dev`：36 通过。
- `cargo test -p aicc`：731 通过（旧/未来版本拒绝、有表无版本拒绝、文件库重开）。
- `cargo test -p msg_center`：180 通过（`groups` 与 `friend` 旧 payload 被拒且行不变、缺省字段的当前 payload 正常）。
- `cargo test -p libopendan --test memory_component --test memory_sessions --test l1 --test queue_lifecycle`：全部通过。
- `cargo check --workspace --all-targets`：通过，只有与本次无关的既有警告。
- 子代理执行：`cargo test -p buckyos-api`、`-p system_config`、`-p kmsg` 通过；cyfs-ndn `cargo test -p package-lib` 34 通过；buckyos-base `cargo test -p name-client --lib` 91 通过；cyfs-gateway `cargo test -p bns-client -p bns-indexer -p cyfs-sn -p bns-server` 通过；websdk `pnpm test`（536 通过）、`pnpm build`、`pnpm check:cli`、`pnpm test:cli`（113 通过）。
- Desktop：`deno test --config tests/datamodel/messagehub.deno.json --no-check -A tests/datamodel/desktop-folder.test.ts` 8 通过；`pnpm check`、`pnpm build`、eslint（改动文件）通过；`playwright test tests/e2e/pages/desktop.spec.ts tests/e2e/pages/desktop-folder.spec.ts` 27/28 通过，失败的 `SN Router is shown as a system-managed provider` 在改动前同样失败，与本任务无关。
- 安装器与脚本：`sync-macos-scripts` 重新生成后无迁移代码，`bash -n` 通过；`src/publish` 下 `python3 -m unittest discover -s tests` 56 通过；`uv run python -m unittest discover -s tools/tests` 46 通过。

### 7.5 未验证项与风险

- 未在 macOS、Windows 真机上做全新安装和覆盖安装；未部署 DV。
- DV 上残留的开发期数据（AICC v1/v2 库、无 `task_store_meta` 的 TaskMgr 库、旧 Dispatcher 库、含 `groups` 的联系人、2.10 Memory 根目录、旧 Notebook 库）在新代码下会打开失败，需要 `start.py --all`。
- 已部署 SN 节点上的旧格式 SQLite（旧 relay 表、v1 或无版本的 BNS registry、缺 `tx_state_model` 的写请求库）升级后启动失败，上线前需确认或重置。
- 待决定：package-lib 的旧字段 `enable_strict_mode` 现在作为未知字段被忽略（`enable_meta_db` 取 false），没有显式报错；要显式拒绝需要连带修改 `PackageEnv::new()` 解析失败时回退默认配置的处理。
- `zone_document_hostname` 遇到没有 hostname 的旧文档返回空串而不是报错；改成 `Result` 会牵动约 12 个调用方，本次未做。
- name-client：版本号已是 2、但仍留有旧格式条目的缓存库会在加载时报解码错误（默认 BuckyOS 未启用该持久化）。
- TaskMgr/Dispatcher 的打开错误仍是 `String`，相关测试只能按文本核对关键参数，没有枚举可断言。
- Dispatcher 没有版本行，列结构相同的 v3 库与 v4 无法区分。
- Notebook 旧库拒绝只有库层用例：CLI crate 没有 SQLite 依赖，没有为此新增 dev-dependency。
- websdk 真实浏览器测试与 AppClient 在 DV 上的行为未验证；`buckyos.account_info.<appId>` 现在只写不读，可后续清理。

### 7.6 合入顺序

五个仓库编译层面互不依赖，可以各自提交：buckyos 未使用关联仓库中被删或新增的公开 API，buckyos 侧不需要 `enable_meta_db` 字段。websdk 推到 `main` 后，buckyos 构建才会打包新的 SDK/CLI。运维层面，新的 SN/BNS 二进制上线前先处理 SN 节点上的旧库。
