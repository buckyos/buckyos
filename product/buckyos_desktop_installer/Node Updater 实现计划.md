# BuckyOS Desktop · Node Updater 实现计划

本期交付一个随目标版本发布的 Rust 程序 `node_updater`。在线和离线安装都执行同一顺序：准备目标包 → 检查应用兼容性 → 确认停服 → 迁移已有数据 → 覆盖程序文件 → 启动新版。建议用 **System Config、Task Manager 和 Message Center** 先跑通。三者分别覆盖共享配置事务、有版本标记的分区库，以及只有目标版本声明的存量库。[已确定的产品设计](./Node%20Updater%20设计.md)

**核对范围。** 以 BuckyOS `88554310f`、其打包子模块 `src/publish` 的 `5e07858`、在线安装与发布脚本 `acfd993e` 为基线。从 Desktop 的打包清单追到服务的实际读写入口，并核对相邻 `cyfs-ndn` 的底层存储。下列版本均指这个源码快照。表中“未见库级标记”指检查过该库的创建、打开或验证逻辑。用户机器上的实际数据、可选组件和第三方应用要由安装时的只读探测确定。[Desktop 模块清单](../../src/bucky_project.yaml#L102) · [安装包子模块](../../.gitmodules)

## 1. 应注册哪些 data reader

以**拥有持久化格式的模块**为注册单位，以实际数据区为探测和提交单位。同一个进程可以负责多个库，同一个服务也可能给多个应用托管数据。下表的“基线”是指模块负责人提供现行结构的只读识别，以及把识别结果和 schema 标记原子写回的逻辑。已存在的数据库不能仅凭安装版本补一个“当前版本”。

### 已能读到持久化版本的模块

| 注册模块与数据区 | 版本实际存放处 | 实现动作与源码 |
| --- | --- | --- |
| Task Manager，`task-mgr-main` 的 UserData 和 Local 分区 | 两个库各自的 `task_store_meta.meta_value`，键为 `schema_version`，当前 **8** | 两个分区都探测。抽取现有读版本逻辑，给探测连接使用只读模式。逐分区验证后再报告整个模块完成。[读取及校验](../../src/kernel/task_manager/src/task_store.rs#L193) · [目标版本](../../src/kernel/buckyos-api/src/task_mgr.rs#L33) |
| AICC，`aicc-usage-log`（含 quota） | `aicc_schema_meta.schema_version`，`schema_key='aicc'`，当前 **3** | 复用已有表检测和拒绝未知版本的分支。计费记录 revision、provider inventory 的记录版本各有独立含义。[建库、读取与拒绝条件](../../src/frame/aicc/src/storage/mod.rs#L364) |
| HomeStation，`homestation-main` | `meta.value`，`key='schema_version'`，当前 **3** | 写独立只读 probe。现有 `Db::open` 会先执行 DDL，缺标记时还会插入当前版本。[开库顺序](../../src/frame/homestation/src/db.rs#L68) · [版本常量](../../src/kernel/buckyos-api/src/homestation_client.rs#L20) |
| AIWorkspace，每个 workspace 的文档库 | `doc.sqlite` 的 `workspace_meta['storage_schema_version']`，当前 **7**。现有 open 含 5 → 6 → 7。另有 `format_version` 和实体级版本 | 从 workspace 列表逐实例处理，提取现有转换。普通 open 会先改 `local.sqlite`。配对的 local 库以及服务级 `sessions.sqlite`、`users.sqlite` 分别核对结构和提交边界。[创建及升级](../../src/frame/aiworkspace/store/src/workspace.rs#L537) · [服务级数据库](../../src/frame/aiworkspace/store/src/service.rs#L108) |
| Node Executor，节点执行记录 | `local/node_daemon/executor/meta.json.schema_version`，当前 **1**；请求等文件还有各自的记录标记 | Node Daemon 负责定位，Node Executor 的 reader 验证目录级与记录级标记。只读探测绕开正常打开时的初始化和恢复动作。[目录来源](../../src/kernel/node_daemon/src/node_executor_host.rs#L193) · [数据格式](../../src/kernel/node_executor/src/store.rs#L33) |
| OpenDAN / libopendan，各 Agent 的会话 | 每个会话的 `session_config.json`、`state.json`、`summary.json` 内有 `schema`，当前分别为 `opendan.session_config/7`、`opendan.session_state/6`、`opendan.session_summary/3` | 按已注册 Agent 和会话定位。复用协议类型，盘点同实例的 goal、skills 和 worklog 依赖。不能凭一个会话文件断言整个 Agent 已升级。[会话文件名](../../src/frame/lib_opendan/src/protocol/mod.rs#L36) · [运行时校验](../../src/frame/lib_opendan/src/session/live.rs#L30) |
| Agent Memory，每个 Agent 的记忆库 | `memory/.meta/meta.json.schema_version`，当前 **4.0**，已有主次版本兼容规则 | 复用 `open_read` 和原有兼容判定，保护源日志 `.meta/occasions.jsonl`，派生索引由模块按需重建。[版本与路径](../../src/frame/agent_tool/src/agent_memory/mod.rs#L47) · [只读入口](../../src/frame/agent_tool/src/agent_memory/mod.rs#L490) |

### 需要补充版本标记或拆分存储范围的模块

| 注册模块与数据区 | 现在能从代码确认什么 | 首次接入动作与源码 |
| --- | --- | --- |
| System Config，`local/system_config` | 实际后端是 sled。`__meta/revision/<key>` 记录并发写 revision。配置值可自带 schema，未见覆盖所有业务域的迁移版本 | 登记一个物理存储 reader，再为下节列出的键空间登记所有者。共享库只提交一次。[路径与内部键](../../src/kernel/sys_config_service/src/sled_provider.rs#L20) · [事务与 flush](../../src/kernel/sys_config_service/src/sled_provider.rs#L303) |
| Message Center，`msg-center-main` | **13** 来自 RDB 实例声明；打开时执行 DDL。群组锁行也有 `schema_version`，只描述该行。未见全库已应用版本 | 对消息、群组、通知等同库表做一次完整基线识别并落标。保留模块已兼容的历史消息对象。[声明](../../src/kernel/buckyos-api/src/msg_center_client.rs#L464) · [主库](../../src/frame/msg_center/src/msg_box_db.rs#L168) · [群组表](../../src/frame/msg_center/src/group_store.rs#L31) |
| Task Dispatcher，`task-dispatcher-main` | 目标 **4**。打开逻辑明确说明不存库版本，通过已有表和列验证 | 直接复用结构验证，改为只读后再决定是否写基线；与 Task Manager 两个已标记的分区单独登记。[结构验证](../../src/kernel/task_manager/src/dispatcher/dispatch_db.rs#L129) · [目标声明](../../src/kernel/buckyos-api/src/task_dispatcher.rs#L30) |
| Repo Service，`repo-service-main` | 目标 **2**。打开 RDB 后直接应用 DDL，未见库内版本读取 | 只读识别索引、证明和公告等表，给物理库补标记。包内容和对象保持原样。[RDB 声明](../../src/kernel/buckyos-api/src/repo_client.rs#L21) · [开库与 DDL](../../src/frame/repo_service/src/repo_db.rs#L87) |
| Verify Hub；Workflow | 分别为 `data/var/verify-hub/sessions.db`、`data/srv/workflow/workflow.db`。均有开库 DDL，未见全库 schema。Workflow 的执行 epoch 描述运行状态 | 各登记一个 SQLite reader，保留登录会话族、撤销状态和工作流执行记录。探测时避免创建空库。[Verify Hub 路径](../../src/kernel/verify_hub/src/main.rs#L2579) · [会话库](../../src/kernel/verify_hub/src/family_store.rs#L395) · [Workflow 路径](../../src/kernel/workflow/src/main.rs#L175) · [DDL](../../src/kernel/workflow/src/persistence.rs#L89) |
| KMsg，`local/kmsg` | sled 的多个 tree 共同承载队列，未见库级 schema；正常打开还会清理、重建索引 | 离线识别 tree 和记录格式，区分队列本体与派生索引，转换和标记同批生效。[sled 打开路径](../../src/kernel/kmsg/src/sled_msg_queue.rs#L135) |
| NFS Server、Bucket Store、Named Store | NFS 服务模式下 `data/.fsdb/filedb.sqlite` 存 ACL、集合等元数据；复制任务另有 `copy.sqlite`，桶索引用 RDB 实例 `bucket-index`。共享 Named Store 由 `storage/named_store.json` 定位。上述建库路径未见共同库版本 | 元数据至少按 filedb、copy、bucket index、Named Store 分开登记。底层格式由库所有者提供 reader，多个消费者共用。只处理所需元数据，保留用户文件、对象和 chunk；按实际存储配置纳入 fs_meta。[NFS 布局](../../src/frame/nfs_server/src/config.rs#L193) · [filedb](../../src/frame/nfs_server/src/filedb.rs#L273) · [copy](../../src/frame/nfs_server/src/copy.rs#L134) · [bucket](../../src/frame/nfs_server/src/bucket/mod.rs#L46) · [Named Store 入口](../../src/kernel/buckyos-api/src/runtime.rs#L379) |
| Node Daemon，节点本地配置、工具收据和包索引 | `etc/<zone>.zone_lkgs.json` 有 `schema=buckyos.zone_lkgs.v1`；`etc/host_account.json` 使用严格 JSON 类型且没有版本字段；ext tools 安装收据有独立格式；PIKG 环境有 `pkgs/meta_index.db` | 按格式分项注册，包括旧 `start_config.json` 的位置迁移。保留签名、身份和密钥，验证派生索引可重建后再决定如何升级。严格 JSON 类型需用兼容方式引入标记。[节点状态](../../src/kernel/node_daemon/src/zone_boot_resolve.rs#L41) · [host account](../../src/kernel/node_daemon/src/host_account.rs#L8) · [工具收据](../../src/kernel/buckyos-api/src/ext_tools_install.rs#L37) · [PIKG 索引](../../src/kernel/node_daemon/src/node_daemon.rs#L252) |

**包内应用按数据所有权登记。** Text Editor 的设置由 System Config 提交；业务桶中的 buffer 和 recovery 记录自带 `format`（均为第 1 版），可以由应用 reader 识别。浏览器 IndexedDB 属于客户端，用户打开的正文文件原样保留。Desktop UI、Node Active、CLI 和 Agent Loader 应登记实际拥有的持久化格式；共享服务已认领的数据无需给每个可执行文件再建一份 reader。宿主 BuckyOSApp 的 `app_config_dir()/config.json` 属于桌面客户端自身的升级范围。日志、缓存、可选的 klog/slog、第三方应用要依据实际启用项和所有者登记，避免通过全盘扫描猜测归属。[Text Editor 设置](../../src/apps/text_editor/src/platform/settings.ts#L12) · [buffer 格式](../../src/apps/text_editor/src/fs/bufferStore.ts#L7) · [recovery 格式](../../src/apps/text_editor/src/fs/recoveryStore.ts#L6) · [组件安装映射](../../src/bucky_project.yaml#L102)

**跨仓库格式由底层库认领。** 此快照使用的 `cyfs-ndn` 版本是 `e36948c`。命名存储的表结构见该仓 `src/named_store/src/store_db.rs:392`，桶索引见 `src/bucket_store/src/db.rs:211`，文件系统元数据的列检查与升级见 `src/fs_meta/src/fs_meta_service.rs:564`。PIKG 的 `meta_index.db` 建库入口是 `src/package-lib/src/meta_index_db.rs:73`。这些源文件也应列入对应 reader 的审计路径。随 Desktop 交付的 cyfs-gateway 程序及托管模板按模块清单覆盖，用户的 gateway 配置按保留路径处理；独立 SN、BNS 服务的数据不随这次 Desktop 安装迁移。[打包依赖与保留路径](../../src/bucky_project.yaml#L145)

## 2. System Config 和 RDB 的离线入口

**给 System Config 增加可链接的存储库。** 把 `SledStore` 的已有库打开、遍历和事务能力从服务入口中抽出，显式接受目标 root，区分“路径不存在”与“已有库读失败”。导出时保留完整的键和值，只将已登记的 JSON 键交给相应模块解码。诸如 `boot/config` 的签名内容和 RBAC 策略文本要保真。现有调度器 dump 仅列若干前缀，而 `list_data` 会跳过迭代错误；升级需要严格遍历，避免把读取不完整当成配置缺失。[服务 dump](../../src/kernel/sys_config_service/src/main.rs#L1235) · [遍历实现](../../src/kernel/sys_config_service/src/sled_provider.rs#L254)

| System Config 中的注册所有者 | 应从哪里读版本／如何处理 |
| --- | --- |
| 系统内核 | `system/buckyos_info`、`system/buckyos_dev_config`、`system/zone_presentation` 的 JSON 各自有 `schema_version=1`。`buckyos_info.version/build_version` 单独表示系统发布版本。RBAC、用户和设备记录按实际格式认领。[内核配置类型](../../src/kernel/buckyos-api/src/system_config.rs#L22) |
| Scheduler / Control Panel / Agent 管理 | `system/app_registry` 和 Agent spec 有自己的 v1 标记；用户应用的 install record 写入 v4，Agent 记录按其自身类型解析。按 `owner/appid` 或 `owner/agent_id` 枚举，每个键只交给一个业务所有者。[注册表与 Agent spec](../../src/kernel/buckyos-api/src/app_schema.rs#L12) · [应用安装记录写入](../../src/kernel/scheduler/src/install_plan_executor.rs#L589) · [Agent 写入](../../src/frame/control_panel/src/agent_mgr.rs#L881) |
| AICC、SMB、Text Editor 等配置写入方 | AICC 的 `services/aicc/driver_metadata` 有文档版本，`services/aicc/settings` 单独解析。SMB 的 `users/<uid>/samba/settings` 是用户配置，`latest_*` 是同步状态。Text Editor 的 `users/<uid>/apps/<appid>/settings` 内有 `schema_version=1`。无自带版本的配置由所有者定义基线和标记。[AICC](../../src/frame/aicc/src/settings/mod.rs#L20) · [SMB](../../src/frame/smb_service/src/main.rs#L247) · [Text Editor](../../src/apps/text_editor/src/platform/settings.ts#L19) |

各 reader 对同一旧快照生成 OP list。汇总时检查重复键及不兼容的修改，再把业务 OP、内核 OP 和相应域的 schema 标记一次交给现有 `exec_tx`，沿用事务提交和 `flush`。任一 reader 失败，整批取消。事务已提交但返回中断时，重跑读取实际标记。[KV 接口](../../src/kernel/sys_config_service/src/kv_provider.rs#L25) · [原子写入](../../src/kernel/sys_config_service/src/sled_provider.rs#L419)

**让 RDB reader 能在停服时找到实际的库。** 现有 resolver 在线读取服务或用户应用 spec 中的 `spec_config.rdb_instances`，解析 backend、connection、instance 和 partition。提取可接受旧快照及目标 root 的路径解析，关闭建目录、建库和 `mode=rwc` 等副作用。读取服务级 `data/<appid>/`、用户级 `data/<uid>/<appid>/`、`local/`、`storage/` 时沿用真实分区映射和路径校验。若该实例是 PostgreSQL，就走该 reader 已验证的后端事务；遇到未支持或不可访问的后端，在覆盖前返回具体实例和原因。RDB 配置里的 `version` 只能说明声明的格式，是否完成迁移要读物理库。[RDB 实例声明](../../src/kernel/buckyos-api/src/rdb_mgr.rs#L72) · [在线读 spec](../../src/kernel/buckyos-api/src/rdb_mgr.rs#L287) · [分区映射](../../src/kernel/buckyos-api/src/rdb_mgr.rs#L469)

## 3. Rust 程序、reader 注册及首次发版

1. **建立编译入口和唯一注册文件。** 建议新增 `src/kernel/node_updater/`，加入 Cargo workspace 和 `bucky_project.yaml` 的 rust module、app 安装映射，产物为 `bin/node-updater/node_updater[.exe]`。在该 crate 的 `readers.toml` 登记 reader ID、所有者、Rust lib、拥有的数据区或键前缀、依赖和需审计的源文件。构建时核对注册项都映射到直接链接的 Rust 实现，发现重复所有权即报错。reader 由模块所有者维护；仅有服务 binary 的模块先抽取存储代码为 lib。后续 Git hook 使用相同文件触发 schema 审计。[Rust workspace](../../src/Cargo.toml#L4) · [发布映射](../../src/bucky_project.yaml#L102)
2. **先做无写入探测，再做迁移。** 对同一实例区分“尚未创建”“已到目标或仍兼容”“可识别的旧格式”“未知／高版本”“读取失败”。缺失的可选应用可跳过；已有且无标记的库只有在模块能确认完整布局后，才在本次 apply 中登记基线。无法识别就中止并指出模块和数据区。当前没有历史发布链，无需编造旧版本映射。源码中已有的格式演进（如 AIWorkspace 5、6）可用于真实测试夹具。
3. **落实一次原子 apply。** SQLite 用同一事务写数据与标记，备份需包含已提交的 WAL 内容；sled 复用事务。多文件目录先在独立位置准备完整新数据，以一个可恢复的原子切换发布数据和版本。Task 两个分区、AIWorkspace 的文档和本地库均需明确原子边界。涉及多个独立库的变更，先准备全部结果，再通过模块级代际切换或等效恢复协议保证对外一致；分别给两个库提交事务不能保证跨库原子性。中断重试、普通迁移失败并回滚、迁移成功但覆盖失败三种路径分别验收。
4. **定义安装程序能可靠使用的结果。** 提议 CLI 接收 `--root`、目标构建标识和结构化结果文件路径；显式传入 profile 及适用的外置安全目录。二进制自报目标版本，安装程序核对与包一致。退出成功且结果为 `success` 才能覆盖；全部 reader 跳过也是成功。失败返回 `unsupported_schema` 或 `migration_failed`、模块、数据区、原因及恢复状态。以后需要过渡安装包时再填其发布版本。进程崩溃、输出缺失或无法解析，均保持停机。
5. **SDK 检查与停机迁移分开。** 目标版本确实声明 SDK 下限时，在停服前从已安装 AppDoc 列出受影响应用，让用户确认。平台加载应用时共用同一判定。已有 `AppDoc.sdk_version` 是可选字符串；明确缺字段兼容应用的行为，按实际版本语义比较。现有 App 安装规划器检查的是 runtime 能否满足 App 的要求，需要另加发行方对旧 SDK 的下限判断。[AppDoc](../../src/kernel/buckyos-api/src/app_doc.rs#L653) · [现有检查方向](../../src/frame/control_panel/src/app_install_planner.rs#L303)

## 4. 接进在线和离线安装包

**从同一份目标 payload 取 updater。** 每个平台的 CI 将 `buckyos` staging 归档，并生成包含 `modules/data_paths/clean_paths` 的 `.install-buckyos.json`。在线发布选择同一次构建的 payload，正式离线打包也可使用这个版本的 staging；本地构包走 `make_local_pkg.py`。先把 updater 放进服务 payload，再让两种安装方式在动现有安装目录前都能独立执行它。发布校验需要检查平台、架构、构建版本和包的完整性。只选 BuckyOSApp 客户端或仅生成 staging 时，跳过服务停机和迁移。[安装描述生成](../../.github/scripts/write-install-descriptor.py#L22) · [Linux 归档](../../.github/workflows/build-linux.yml#L175) · [发布 payload](../../.github/workflows/build-all.yml#L278) · [本地构包入口](../../make_local_pkg.py)

### 在线安装的三个入口

下表路径及行号对应在线安装器源码快照 `acfd993e`。修改共享 hook 时要同步会嵌入 hook 的在线脚本，再运行同步校验，避免在线和离线各执行一次迁移。

| 平台与现有入口 | 接入位置 |
| --- | --- |
| Linux · `installer/linux.sh:535, 573–585` | 下载校验并在工作目录解包后，即可运行目标 updater。把停服检查和迁移放在删除旧 `bin`、复制 payload 之前。现有部分 stop 错误被忽略；区分“已停止”与“仍有写入进程”。 |
| macOS · `installer/macos.sh:776–798` | 现有序列是 preinstall、清理、复制、postinstall。向服务 preinstall 传 staging 中 updater 的路径，拆开其中的停服与清理，确保只调用一次。GUI-only 分支跳过。 |
| Windows · `installer/windows.ps1:1323–1383` | 在服务组件的 `Install-InstallerPackages` 中，停止并确认后、首次删改之前运行 staging 内的目标 exe。等待退出并校验结果；交互、静默模式都把失败返回上层。 |

同时调整 `bin/onlinepack.py:413` 的最低 installer 版本：上线顺序先发布能识别新约定的脚本，再允许生产依赖迁移的新 payload。验证旧脚本会拒绝新版 manifest。安装器探测覆盖安装时要检查已有数据区，即使上次安装失败导致 `bin` 或版本文件缺失，仍须走重试路径。

### 离线原生包的四种交付

| 格式 | 目标 updater 如何在覆盖前执行 | 需要改动的源码 |
| --- | --- | --- |
| macOS PKG | 在构包时将同架构 updater 一份放到服务组件的 Scripts 中并纳入包签名。preinstall 从脚本目录执行，位置在确认旧服务和 LaunchAgent 已停之后、第一条模块清理命令之前。postinstall 负责完成部署和启动。 | [Scripts 打包](../../src/publish/make_local_osx_pkg.py#L577) · [preinstall](../../src/publish/macos_pkg/scripts/buckyos_preinstall#L66) |
| Windows NSIS | 复用独立释放 stop 脚本的做法，把 updater 释放到 `$PLUGINSDIR`。在服务组件内等待它完成，成功后才进入 `File /r`。失败走已有日志、退出码和 Abort 路径。 | [停服](../../src/publish/win_pkg/installer.nsi.in#L374) · [覆盖入口](../../src/publish/win_pkg/installer.nsi.in#L481) · [生成器](../../src/publish/make_local_win_installer.py#L665) |
| Linux DEB | `preinst` 发生在新 payload 解包前。首选验证方案是在构包时把压缩的同架构 updater 随 preinst 携带，安装时释放到独立、可执行的临时目录，校验后执行。先在真实 dpkg 上验证脚本大小、失败状态与同版本重装，再确定封装。 | [preinstall](../../src/publish/deb_pkg/buckyos_preinstall#L3) · [控制脚本生成](../../src/publish/make_local_deb.py#L415) |
| Linux RPM | `%pre` 同样需要在解包前拿到目标 updater。沿用 DEB 的封装验证，测试 rpm 头部/scriptlet 的体积限制与同版本 reinstall。若单包无法可靠携带，改为随离线介质交付、强制同版本的前置 helper 包，并先验证完整离线安装和重试。 | [preinstall](../../src/publish/rpm_pkg/buckyos_preinstall#L3) · [scriptlet 生成](../../src/publish/make_local_rpm.py#L569) |

**同步修正覆盖范围。** 当前 Linux 的部分入口会忽略停服错误并删除整个 `bin/`，而 `bin/applist.json` 在清单中属于保留数据。在线脚本还有直接写入 cluster 配置的路径，平台 postinstall 还会处理 `etc/start_config.json` → 安全目录。逐项按“程序文件、缺失才 seed、已有数据由 updater 转换”核对，移除整目录误删，将旧配置的位置迁移交给其 reader，并传入真实的 `BUCKYOS_SECURITY_ROOT`。覆盖成功才启动。迁移成功后部署报错，保留新数据并提示重跑同版本安装包。[保护路径](../../src/bucky_project.yaml#L145) · [Linux 清理](../../src/publish/deb_pkg/buckyos_preinstall#L3) · [启动配置现有搬移](../../src/publish/deb_pkg/buckyos_postinstall#L27)

**验证停机窗口覆盖整个写入过程。** Linux 的 systemd unit、macOS 的 LaunchAgent 和 Windows 的 `BuckyOSNodeDaemonKeepAlive` 都可能再次拉起服务。停服步骤要处理相应的守护机制，并确认写库进程已退出。只有所有文件就位才恢复守护并启动。用同一目标 root 做安装互斥，重跑从新包解压 updater 和停服工具，避免依赖上次覆盖到一半的程序。[Linux unit](../../src/publish/deb_pkg/buckyos_postinstall#L51) · [macOS 停服](../../src/publish/macos_pkg/scripts/buckyos_preinstall#L66) · [Windows keepalive](../../src/publish/win_pkg/scripts/buckyos_postinstall.ps1)

## 5. 实施顺序与验收

| 顺序 | 交付 | 可以进入下一步的条件 |
| --- | --- | --- |
| 1 · 存储基线 | reader 注册文件、只读 RDB 定位、System Config 严格快照，以及 Task Manager、Message Center 两类物理库的夹具 | 现行、有标记旧版、未建库、已知无标记、未知布局分别返回正确结果。探测前后数据一致，缺失路径仍然缺失。 |
| 2 · 迁移程序 | Rust CLI、JSON 结果、System Config OP 合并、模块 apply/备份/恢复；按第 1 节扩展其他 reader | 注入 apply 前、apply 后返回前、两个数据区之间的中断。重跑只处理未完成项；正常失败恢复本次已改数据。SDK 预检覆盖用户确认和取消。 |
| 3 · 共用发布产物 | CI 和构包验证目标 updater 可在旧服务停止、旧目录不完整的机器上独立运行 | 所支持 OS/架构载荷均有正确二进制；无网络的离线预安装能运行它。构包 staging、仅装客户端、首次全新安装分别走正确分支。 |
| 4 · 平台安装与故障演练 | 在线 Linux/macOS/Windows，离线 DEB/RPM/PKG/NSIS，更新相应 `verify-pkg`、平台模板测试和在线安装器测试 | 用临时安装根或平台 VM 验证：停服失败或 updater 失败时，旧程序文件完整；成功后故意中断覆盖，重跑同包，所有已迁模块跳过并启动目标版；既有 System Config、AppData、`bin/applist.json` 和个人文件保留。 |
| 5 · 发布审计 | 每个数据所有者确认支持范围，生成“BuckyOS 发布版本 → 模块 schema”记录；同步在线脚本与离线打包子模块版本 | 正式发布用的脚本和 staging 均通过上述测试。以后 reader 或被登记的格式文件变更时，复用注册表提示 schema 审计。 |

本计划基于源码核对。库内样本、Linux 预安装脚本能否容纳实际编译体积、各平台包管理器在故障重装时的行为，均列为实施验收项；本次未执行真实数据迁移或安装。
