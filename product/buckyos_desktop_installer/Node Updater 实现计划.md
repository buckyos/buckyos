# BuckyOS Desktop · Node Updater 实现计划

**Node Updater 只适用于 BuckyOS Desktop（单机版）。** 该版本没有 etcd，System Config 使用本机 sled，所有 RDB 实例均为本机 SQLite。下文的停服、迁移、覆盖和恢复均在同一台主机上完成。[已确定的产品设计](./Node%20Updater%20设计.md)

本期交付一个随 Desktop 目标版本发布的 Rust 程序 `node_updater`。在线安装脚本默认选择本机平台最新发布的版本。在线和离线覆盖安装都先准备目标包、检查应用兼容性，再停服并验证写入进程已退出。Node Updater 在临时目录准备全部迁移结果，全部校验通过后才执行最终提交（apply）。安装程序收到成功结果后覆盖程序文件并启动新版。建议用 **System Config、Task Manager 和 Message Center** 先跑通。三者分别覆盖共享配置事务、有版本标记的 SQLite 分区库，以及只有目标版本声明的 SQLite 存量库。

**核对范围。** 以 BuckyOS `88554310f`、其打包子模块 `src/publish` 的 `5e07858`、在线安装与发布脚本 `acfd993e` 为基线。从 Desktop 的打包清单追到服务的实际读写入口，并核对相邻 `cyfs-ndn` 的底层存储。下列版本均指这个源码快照。表中“未见库级标记”指检查过该库的创建、打开或验证逻辑。用户机器上的实际数据、可选组件和第三方应用要由安装时的只读探测确定。[Desktop 模块清单](../../src/bucky_project.yaml#L102) · [安装包子模块](../../.gitmodules)

## 1. 应注册哪些 data reader

以**拥有持久化格式的模块**为注册单位，以实际数据区为探测和暂存单位。同一个进程可以负责多个库，同一个服务也可能给多个应用托管数据。本版是首个有效的 Desktop 持久化格式。已登记的本仓数据库在建库时记录当前 schema，独立版本化的 JSON 由写入方在文件或记录中写明 schema。覆盖安装只接受当前格式；已存在的数据缺少必需标记、标记不同或结构不符，均返回 `unsupported_schema` 并保留原数据。尚未创建的可选数据区可以跳过。第三方数据采用底层库实际定义的原生格式。完整的 reader、数据路径和目标版本见[注册清单](../../src/kernel/node_updater/readers.toml)。

### 已能读到持久化版本的模块

| 注册模块与数据区 | 版本实际存放处 | 实现动作与源码 |
| --- | --- | --- |
| Task Manager，`task-mgr-main` 的 UserData 和 Local 分区 | 两个库各自的 `task_store_meta.meta_value`，键为 `schema_version`，当前 **8** | 两个分区都探测。抽取现有读版本逻辑，给探测连接使用只读模式。逐分区验证后再报告整个模块完成。[读取及校验](../../src/kernel/task_manager/src/task_store.rs#L193) · [目标版本](../../src/kernel/buckyos-api/src/task_mgr.rs#L33) |
| AICC，`aicc-usage-log`（含 quota） | `aicc_schema_meta.schema_version`，`schema_key='aicc'`，当前 **3** | 复用已有表检测和拒绝未知版本的分支。计费记录 revision、provider inventory 的记录版本各有独立含义。[建库、读取与拒绝条件](../../src/frame/aicc/src/storage/mod.rs#L364) |
| HomeStation，`homestation-main` | `meta.value`，`key='schema_version'`，当前 **3** | 新建空库时写入标记。服务打开已有库和 updater 只读探测时，均先校验原生标记。[开库顺序](../../src/frame/homestation/src/db.rs#L68) · [版本常量](../../src/kernel/buckyos-api/src/homestation_client.rs#L20) |
| AIWorkspace，每个 workspace 的文档库 | `doc.sqlite` 的 `workspace_meta['storage_schema_version']`，当前 **7**。基线快照曾支持 5 → 6 → 7。另有 `format_version` 和实体级版本 | 从 workspace 列表逐实例处理，校验首版文档 schema 7。以开发期 5、6 为拒绝样本，打开和覆盖安装均保留原数据。配对的 local 库以及服务级 `sessions.sqlite`、`users.sqlite` 分别核对结构和提交边界。[创建及升级](../../src/frame/aiworkspace/store/src/workspace.rs#L537) · [服务级数据库](../../src/frame/aiworkspace/store/src/service.rs#L108) |
| Node Executor，节点执行记录 | `local/node_daemon/executor/meta.json.schema_version`，当前 **1**；请求等文件还有各自的记录标记 | Node Daemon 负责定位，Node Executor 的 reader 验证目录级与记录级标记。只读探测绕开正常打开时的初始化和恢复动作。[目录来源](../../src/kernel/node_daemon/src/node_executor_host.rs#L193) · [数据格式](../../src/kernel/node_executor/src/store.rs#L33) |
| OpenDAN / libopendan，各 Agent 的会话 | 每个会话的 `session_config.json`、`state.json`、`summary.json` 内有 `schema`，当前分别为 `opendan.session_config/7`、`opendan.session_state/6`、`opendan.session_summary/3` | 按已注册 Agent 和会话定位。复用协议类型，盘点同实例的 goal、skills 和 worklog 依赖。不能凭一个会话文件断言整个 Agent 已升级。[会话文件名](../../src/frame/lib_opendan/src/protocol/mod.rs#L36) · [运行时校验](../../src/frame/lib_opendan/src/session/live.rs#L30) |
| Agent Memory，每个 Agent 的记忆库 | `memory/.meta/meta.json.schema_version`，当前 **4.0**，已有主次版本兼容规则 | 校验 4.0 后调用 `open_read`，保护源日志 `.meta/occasions.jsonl`，派生索引由模块按需重建。[版本与路径](../../src/frame/agent_tool/src/agent_memory/mod.rs#L47) · [只读入口](../../src/frame/agent_tool/src/agent_memory/mod.rs#L490) |

### 需要补充版本标记或拆分存储范围的模块

| 注册模块与数据区 | 现在能从代码确认什么 | 首次接入动作与源码 |
| --- | --- | --- |
| System Config，`local/system_config` | 实际后端是 sled。`__meta/revision/<key>` 记录并发写 revision。配置值可自带 schema，未见覆盖所有业务域的迁移版本 | SledStore 新建时写 `__meta/schema_version`，各键空间由下节列出的所有者验证。[路径与内部键](../../src/kernel/sys_config_service/src/sled_provider.rs#L20) · [事务与 flush](../../src/kernel/sys_config_service/src/sled_provider.rs#L303) |
| Message Center，`msg-center-main` | **13** 来自 RDB 实例声明；打开时执行 DDL。群组锁行也有 `schema_version`，只描述该行。未见全库已应用版本 | 建库时写 `user_version=13`。打开时验证标记和同库表。保留模块已兼容的历史消息对象。[声明](../../src/kernel/buckyos-api/src/msg_center_client.rs#L464) · [主库](../../src/frame/msg_center/src/msg_box_db.rs#L168) · [群组表](../../src/frame/msg_center/src/group_store.rs#L31) |
| Task Dispatcher，`task-dispatcher-main` | 目标 **4**。打开逻辑明确说明不存库版本，通过已有表和列验证 | 新建库写 `user_version=4`，复用结构验证并检查标记。与 Task Manager 两个已标记的分区单独登记。[结构验证](../../src/kernel/task_manager/src/dispatcher/dispatch_db.rs#L129) · [目标声明](../../src/kernel/buckyos-api/src/task_dispatcher.rs#L30) |
| Repo Service，`repo-service-main` | 目标 **2**。打开 RDB 后直接应用 DDL，未见库内版本读取 | 新建库写 `user_version=2`，只读验证索引、证明和公告等表。包内容和对象保持原样。[RDB 声明](../../src/kernel/buckyos-api/src/repo_client.rs#L21) · [开库与 DDL](../../src/frame/repo_service/src/repo_db.rs#L87) |
| Verify Hub；Workflow | 分别为 `data/var/verify-hub/sessions.db`、`data/srv/workflow/workflow.db`。均有开库 DDL，未见全库 schema。Workflow 的执行 epoch 描述运行状态 | 分别在建库时写 `user_version=1`。各自的 reader 验证登录会话族、撤销状态和工作流执行记录。探测时避免创建空库。[Verify Hub 路径](../../src/kernel/verify_hub/src/main.rs#L2579) · [会话库](../../src/kernel/verify_hub/src/family_store.rs#L395) · [Workflow 路径](../../src/kernel/workflow/src/main.rs#L175) · [DDL](../../src/kernel/workflow/src/persistence.rs#L89) |
| KMsg，`local/kmsg` | sled 的多个 tree 共同承载队列，未见库级 schema；正常打开还会清理、重建索引 | 新建时在 `meta` tree 写 `schema_version`。离线校验标记和各 tree 记录。[sled 打开路径](../../src/kernel/kmsg/src/sled_msg_queue.rs#L135) |
| NFS Server、Bucket Store、Named Store | NFS 服务模式下 `data/.fsdb/filedb.sqlite` 存 ACL、集合等元数据；复制任务另有 `copy.sqlite`，桶索引用 RDB 实例 `bucket-index`。共享 Named Store 由 `storage/named_store.json` 定位。上述建库路径未见共同库版本 | 元数据至少按 filedb、copy、bucket index、Named Store 分开登记。底层格式由库所有者提供 reader，多个消费者共用。只处理所需元数据，保留用户文件、对象和 chunk；按实际存储配置纳入 fs_meta。[NFS 布局](../../src/frame/nfs_server/src/config.rs#L193) · [filedb](../../src/frame/nfs_server/src/filedb.rs#L273) · [copy](../../src/frame/nfs_server/src/copy.rs#L134) · [bucket](../../src/frame/nfs_server/src/bucket/mod.rs#L46) · [Named Store 入口](../../src/kernel/buckyos-api/src/runtime.rs#L379) |
| Node Daemon，节点本地配置、工具收据和包索引 | `etc/<zone>.zone_lkgs.json` 有 `schema=buckyos.zone_lkgs.v1`；`etc/host_account.json` 内置必填 `schema=buckyos.host_account.v1`；ext tools 安装收据有独立格式；PIKG 环境有 `pkgs/meta_index.db` | 按格式分项注册，包括旧 `start_config.json` 的位置迁移。保留签名、身份和密钥，验证派生索引可重建后再决定如何升级。`start_config.json` 同样内置必填 `schema=buckyos.start_config.v1`。`security/dns_credentials/<id>.json` 内置必填 `schema=buckyos.dns_credential.v1`，只检查 `system/domain_hosting` 引用的那一个：文件缺失时放行（token 可以重新申请），存在但读不了时中止，报错不含 token。仅通过格式校验的文件可迁往安全目录。[节点状态](../../src/kernel/node_daemon/src/zone_boot_resolve.rs#L41) · [host account](../../src/kernel/node_daemon/src/host_account.rs#L8) · [工具收据](../../src/kernel/buckyos-api/src/ext_tools_install.rs#L37) · [PIKG 索引](../../src/kernel/node_daemon/src/node_daemon.rs#L252) |

**包内应用按数据所有权登记。** Text Editor 的设置由 System Config 提交；业务桶中的 buffer 和 recovery 记录自带 `format`（均为第 1 版），可以由应用 reader 识别。浏览器 IndexedDB 属于客户端，用户打开的正文文件原样保留。Desktop UI、Node Active、CLI 和 Agent Loader 应登记实际拥有的持久化格式；共享服务已认领的数据无需给每个可执行文件再建一份 reader。宿主 BuckyOSApp 的 `app_config_dir()/config.json` 属于桌面客户端自身的升级范围。日志、缓存、可选的 klog/slog、第三方应用要依据实际启用项和所有者登记，避免通过全盘扫描猜测归属。[Text Editor 设置](../../src/apps/text_editor/src/platform/settings.ts#L12) · [buffer 格式](../../src/apps/text_editor/src/fs/bufferStore.ts#L7) · [recovery 格式](../../src/apps/text_editor/src/fs/recoveryStore.ts#L6) · [组件安装映射](../../src/bucky_project.yaml#L102)

**跨仓库格式由底层库认领。** 此快照使用的 `cyfs-ndn` 版本是 `e36948c`。命名存储的表结构见该仓 `src/named_store/src/store_db.rs:392`，桶索引见 `src/bucket_store/src/db.rs:211`，文件系统元数据的列检查与升级见 `src/fs_meta/src/fs_meta_service.rs:564`。PIKG 的 `meta_index.db` 建库入口是 `src/package-lib/src/meta_index_db.rs:73`。这四类库均使用底层库的完整表结构和原生 SQLite `user_version=0` 校验。BuckyOS 不往这些库写私有版本号。底层源文件已列入对应 reader 的审计路径。随 Desktop 交付的 cyfs-gateway 程序及托管模板按模块清单覆盖，用户的 gateway 配置按保留路径处理；独立 SN、BNS 服务的数据不随这次 Desktop 安装迁移。[打包依赖与保留路径](../../src/bucky_project.yaml#L145)

## 2. 本机 System Config（sled）和 RDB（SQLite）的离线入口

**给 System Config 增加可链接的存储库。** 把 `SledStore` 的已有库打开、遍历和事务能力从服务入口中抽出，显式接受目标 root，区分“路径不存在”与“已有库读失败”。导出时保留完整的键和值，只将已登记的 JSON 键交给相应模块解码。诸如 `boot/config` 的签名内容和 RBAC 策略文本要保真。现有调度器 dump 仅列若干前缀，而 `list_data` 会跳过迭代错误；升级需要严格遍历，避免把读取不完整当成配置缺失。[服务 dump](../../src/kernel/sys_config_service/src/main.rs#L1235) · [遍历实现](../../src/kernel/sys_config_service/src/sled_provider.rs#L254)

| System Config 中的注册所有者 | 应从哪里读版本／如何处理 |
| --- | --- |
| 系统内核 | `system/buckyos_info`、`system/buckyos_dev_config`、`system/zone_presentation`、`system/domain_hosting` 的 JSON 各自有 `schema_version=1`。`buckyos_info.version/build_version` 单独表示系统发布版本。RBAC、用户和设备记录按实际格式认领。[内核配置类型](../../src/kernel/buckyos-api/src/system_config.rs#L22) |
| Scheduler / Control Panel / Agent 管理 | `system/app_registry` 和 Agent spec 有自己的 v1 标记；用户应用的 install record 写入 v4，Agent 记录按其自身类型解析。按 `owner/appid` 或 `owner/agent_id` 枚举，每个键只交给一个业务所有者。[注册表与 Agent spec](../../src/kernel/buckyos-api/src/app_schema.rs#L12) · [应用安装记录写入](../../src/kernel/scheduler/src/install_plan_executor.rs#L589) · [Agent 写入](../../src/frame/control_panel/src/agent_mgr.rs#L881) |
| Control Panel 的登录来源与账号绑定 | `system/sso/settings`，`services/control_panel/` 下的 `oauth/`、`native_login_bindings/`、`user_login_sources/`。记录没有自带版本，使用物理库版本，按所有者类型解析；`oauth/subjects` 还核对路径与内容一致。报错不带记录内容。[OAuth 类型](../../src/kernel/buckyos-api/src/oauth.rs) |
| AICC、SMB、Text Editor 等配置写入方 | AICC 的 `services/aicc/driver_metadata` 有文档版本，`services/aicc/settings` 的 `schema` 为 `buckyos.aicc.settings.v1`。SMB 用户配置 `users/<uid>/samba/settings` 的 `schema` 为 `buckyos.samba.settings.v1`，`latest_*` 仍使用服务定义的同步数组。Text Editor 的 `users/<uid>/apps/<appid>/settings` 内有 `schema_version=1`。各配置使用物理库版本和所有者定义的记录格式。[AICC](../../src/frame/aicc/src/settings/mod.rs#L20) · [SMB](../../src/frame/smb_service/src/main.rs#L247) · [Text Editor](../../src/apps/text_editor/src/platform/settings.ts#L19) |

各 reader 只读验证同一份 System Config 快照。首版的物理 schema 在新建库时设置，业务域的 JSON 保持原有字节。首次出现正式发布过的旧 schema 时，再为实际变更生成迁移事务。[KV 接口](../../src/kernel/sys_config_service/src/kv_provider.rs#L25) · [原子写入](../../src/kernel/sys_config_service/src/sled_provider.rs#L419)

**从旧配置快照定位本机 SQLite 文件。** 现有 resolver 在线读取服务或用户应用 spec 中的 `spec_config.rdb_instances`。离线 reader 从 System Config 快照读取同一份声明，结合目标 root、实例和分区解析文件路径。服务级 `data/<appid>/`、用户级 `data/<uid>/<appid>/` 以及 `local/`、`data/cache/`、`storage/` 均沿用现有分区映射和路径校验。只读探测须关闭建目录、建库和 `mode=rwc` 等副作用。若声明的后端并非 SQLite，在准备阶段报告实例名和后端并中止，避免误迁移范围外的数据。RDB 配置里的 `version` 只说明声明的格式，迁移是否完成要读本机库内的 schema 标记或验证其实际结构。[RDB 实例声明](../../src/kernel/buckyos-api/src/rdb_mgr.rs#L72) · [在线读 spec](../../src/kernel/buckyos-api/src/rdb_mgr.rs#L287) · [分区映射](../../src/kernel/buckyos-api/src/rdb_mgr.rs#L469)

## 3. Rust 程序、reader 注册及首次发版

1. **建立编译入口和唯一注册文件。** 建议新增 `src/kernel/node_updater/`，加入 Cargo workspace 和 `bucky_project.yaml` 的 rust module、app 安装映射，产物为 `bin/node-updater/node_updater[.exe]`。在该 crate 的 `readers.toml` 登记 reader ID、所有者、Rust lib、拥有的数据区或键前缀、依赖和需审计的源文件。构建时核对注册项都映射到直接链接的 Rust 实现，发现重复所有权即报错。reader 由模块所有者维护；仅有服务 binary 的模块先抽取存储代码为 lib。定期 review 使用相同文件做 schema 审计，步骤见[产品设计第 5 节](./Node%20Updater%20设计.md)。[Rust workspace](../../src/Cargo.toml#L4) · [发布映射](../../src/bucky_project.yaml#L102)
2. **先做无写入探测，再做迁移。** 区分尚未创建、格式有效、格式非法和读取失败。缺失的可选应用可跳过。对已有数据同时验证 owner 定义的标记和布局；缺少必需标记或遇到其他版本，中止并指出模块和数据区。可用 AIWorkspace 开发期的 5、6 验证拒绝路径。
3. **在临时目录准备全部结果。** `node_updater` 为本次运行建立私有工作目录。各 reader 只读正式数据，按依赖顺序将转换后的副本或待执行的变更集连同目标 schema 写入该目录，逐项校验。SQLite 副本要包含已提交的 WAL 内容，sled 在副本上执行事务。任一转换或校验失败，或静默模式下任一步需要用户确认，即停止本次安装、丢弃暂存结果。此时所有正式数据和 schema 均为原状态。
4. **最后统一 apply，再向安装程序报告成功。** 所有 reader 均已校验或可跳过，且所有需要用户决定的事项均已解决，才可开始更新正式数据。对 Task 两个分区、System Config 和 AppData 等跨库变更，在首次正式写入前持久化提交清单及恢复所需的旧数据。各数据区的数据与 schema 一起生效。部分数据区写入后出错，按清单恢复全部已变更的数据区；断电重跑先核对持久进度和实际状态。恢复所需材料保留至能确认整批已提交或已回滚，恢复未完成期间保持停服。
5. **定义安装程序能可靠使用的结果。** 提议 CLI 接收 `--root`、目标构建标识和结构化结果文件路径，安装程序显式传入运行模式、profile 及适用的外置安全目录。二进制自报目标版本，安装程序核对与包一致。整批 apply 持久完成或所有 reader 均可跳过，才返回 `success` 和零退出码，允许覆盖程序。失败返回 `unsupported_schema` 或 `migration_failed`、失败阶段、模块、数据区、原因及恢复状态。准备失败报告“未改动”。以后需要过渡安装包时再填其发布版本。进程崩溃、输出缺失或无法解析时保持停机，重跑同一目标先核对提交状态。
6. **SDK 检查与停机迁移分开。** 目标版本声明 SDK 下限时，在停服前检查已安装 AppDoc。若有应用升级后将无法加载，交互安装列出应用和原因，由用户确认或取消；无人值守安装将同一清单写入日志并以非零状态退出，旧服务继续运行。必要的预检无法完成时也在停服前报错。通过预检或目标版本未设下限时自动更新可进入迁移准备阶段。平台加载应用时共用同一判定。已有 `AppDoc.sdk_version` 是可选字符串；明确缺字段兼容应用的行为，按实际版本语义比较。现有 App 安装规划器检查的是 runtime 能否满足 App 的要求，需要另加发行方对旧 SDK 的下限判断。[AppDoc](../../src/kernel/buckyos-api/src/app_doc.rs#L653) · [现有检查方向](../../src/frame/control_panel/src/app_install_planner.rs#L303)

## 4. 接进在线和离线安装包

**更新设置页中的 BuckyOS 版本。** 在线和离线服务安装在部署目标程序并通过启动检查后，调用已安装的 `node_updater record-install --root <root> --target-build <组件构建号>`。它原子写入 `etc/.last-buckyos-install.json`，其中包含 `schema=buckyos.completed_install.v1`、组件构建号 `target_build`、带分支和提交后缀的完整构建号 `runtime_build`（如有）以及完成时间 `completed_at`。System Config 用与运行构建匹配的完成记录刷新 `system/buckyos_info`，保留首次安装时间。仅更新程序、相同构建重装和有数据迁移的更新使用同一收尾步骤。仅暂存、只安装桌面客户端和迁移准备失败都不会生成服务安装完成记录。

**从同一份目标 payload 取 updater。** 每个平台的 CI 将 `buckyos` staging 归档，并生成包含 `modules/data_paths/clean_paths` 的 `.install-buckyos.json`。在线发布选择同一次构建的 payload，正式离线打包也可使用这个版本的 staging；本地构包走 `make_local_pkg.py`。先把 updater 放进服务 payload，再让两种安装方式在动现有安装目录前都能独立执行它。发布校验需要检查平台、架构、构建版本和包的完整性。只选 BuckyOSApp 客户端或仅生成 staging 时，跳过服务停机和迁移。[安装描述生成](../../.github/scripts/write-install-descriptor.py#L22) · [Linux 归档](../../.github/workflows/build-linux.yml#L175) · [发布 payload](../../.github/workflows/build-all.yml#L278) · [本地构包入口](../../make_local_pkg.py)

### 在线安装的三个入口

下表路径及行号对应在线安装器源码快照 `acfd993e`。修改共享 hook 时要同步会嵌入 hook 的在线脚本，再运行同步校验，避免在线和离线各执行一次迁移。

**在线脚本统一使用 `--silent` 选择静默安装。** 可交互安装是默认模式。遇到需确认的事项时，安装程序向用户展示原因并等待选择；若无法取得交互输入，报错退出。显式传入 `--silent` 后，即使有终端也不读取输入或弹窗，需要确认就记录原因并以非零状态退出。由脚本把选定模式传给预检、安装 hook 和 `node_updater`。

| 平台与现有入口 | 接入位置 |
| --- | --- |
| Linux · `installer/linux.sh:14–34, 535, 573–585` | 为现有参数解析新增 `--silent`，引导脚本已透传参数。下载校验并在工作目录解包后，即可运行目标 updater。把停服检查和迁移放在删除旧 `bin`、复制 payload 之前。现有部分 stop 错误被忽略；区分“已停止”与“仍有写入进程”。 |
| macOS · `installer/macos.sh:22, 776–798` | 沿用已有 `--silent`。现有序列是 preinstall、清理、复制、postinstall。向服务 preinstall 传 staging 中 updater 的路径，拆开其中的停服与清理，确保只调用一次。GUI-only 分支跳过。 |
| Windows · `installer/windows.ps1:12, 1323–1383, 1474–1479` | 沿用已有 `--silent`。静默启动时若缺管理员权限，应直接报错退出，避免现有提权分支弹出 UAC。在服务组件的 `Install-InstallerPackages` 中，停止并确认后、首次删改之前运行 staging 内的目标 exe。等待退出并校验结果；交互、静默模式都把失败返回上层。 |

同时调整 `bin/onlinepack.py:413` 的最低 installer 版本：上线顺序先发布能识别新约定的脚本，再允许生产依赖迁移的新 payload。验证旧脚本会拒绝新版 manifest。安装器探测覆盖安装时要检查已有数据区，即使上次安装失败导致 `bin` 或版本文件缺失，仍须走重试路径。

### 在无图形界面的 Linux 主机上每天检查 Desktop 单机版更新

管理员可用 root 的 crontab 定时调用同一在线安装脚本。`--silent` 单独使用即可静默执行普通安装。拟新增的 `--auto-update` 只控制版本策略，即检查已安装节点并在有新版时更新；单独使用时仍可交互。下例假定两个参数均已发布，管理员已将发布的引导脚本 `installer/install.sh` 保存到示例路径。cron 按节点本地时间每天 03:00 执行，引导脚本下载当次的 Linux 安装脚本并透传参数。[引导脚本](https://github.com/buckyos/pve-pack-system/blob/acfd993ec9351182444cfbe252873827b43d2312/installer/install.sh#L14-L17)

```cron
PATH=/usr/sbin:/usr/bin:/sbin:/bin
0 3 * * * /bin/sh /usr/local/sbin/buckyos-install.sh --silent --auto-update --tag latest </dev/null >>/var/log/buckyos-auto-update.log 2>&1
```

- **先判断是否有更新。** 现有脚本在取锁前运行 apt/yum，读到本机版本后只打印，即使版本相同仍会下载并停服重装。定时入口应先取得同一安装根的非阻塞安装锁，优先检查未完成的安装。正常情况下按 `version.tsv` 解析本机平台的 `latest`，与已完整安装的发布版本（含 build）比较。目标相同或更旧时记录结果并返回 0，跳过依赖安装、payload 下载和停服。只有目标更新才进入覆盖安装。已有节点的版本无法判定时报错，显式卸载后的残留数据也不触发重装。手动入口仍可重装同一版本。[依赖和锁](https://github.com/buckyos/pve-pack-system/blob/acfd993ec9351182444cfbe252873827b43d2312/installer/linux.sh#L53-L79) · [版本解析与下载](https://github.com/buckyos/pve-pack-system/blob/acfd993ec9351182444cfbe252873827b43d2312/installer/linux.sh#L493-L535)
- **任何需要用户确认的事项都使本次静默安装失败。** 安装脚本、依赖安装、停服工具和 updater 全程在无桌面、无 TTY、标准输入接 `/dev/null` 的环境下执行。发现需确认的事项即记录原因并以非零状态退出。停服前发现 SDK 冲突，旧服务继续运行。停服后在准备数据时发现歧义，丢弃本次临时目录，确认正式数据仍可由旧版读取后恢复原先运行的服务。两种情况都结束本轮安装，下次检查重新预检和读取数据。权限不足、锁被占用或下载校验失败也须报错退出。沿用节点原有 cluster、profile 和安全目录，需要特别设置的环境变量在任务中显式提供。
- **覆盖中断后续装原构建。** 目标包校验和预检通过后，在停服前保存目标构建、预检结果及原包的下载位置和校验信息，并在各阶段原子更新进度。准备失败且已安全结束时清除待恢复记录。最终 apply 或程序覆盖期间中断，下次运行先按记录恢复原构建，即使 `latest` 已前移，或旧服务无法提供版本和应用信息。apply 已成功而覆盖中断时保持停服，从原包重新执行 updater，确认整批已提交后完成覆盖。新版启动成功后标记安装完成、清除待恢复记录。恢复未完成或状态不明时持续停服并记录原因。

### 离线原生包的四种交付

| 格式 | 目标 updater 如何在覆盖前执行 | 需要改动的源码 |
| --- | --- | --- |
| macOS PKG | 在构包时将同架构 updater 一份放到服务组件的 Scripts 中并纳入包签名。preinstall 从脚本目录执行，位置在确认旧服务和 LaunchAgent 已停之后、第一条模块清理命令之前。postinstall 负责完成部署和启动。 | [Scripts 打包](../../src/publish/make_local_osx_pkg.py#L577) · [preinstall](../../src/publish/macos_pkg/scripts/buckyos_preinstall#L66) |
| Windows NSIS | 复用独立释放 stop 脚本的做法，把 updater 释放到 `$PLUGINSDIR`。在服务组件内等待它完成，成功后才进入 `File /r`。失败走已有日志、退出码和 Abort 路径。 | [停服](../../src/publish/win_pkg/installer.nsi.in#L374) · [覆盖入口](../../src/publish/win_pkg/installer.nsi.in#L481) · [生成器](../../src/publish/make_local_win_installer.py#L665) |
| Linux DEB | `preinst` 发生在新 payload 解包前。首选验证方案是在构包时把压缩的同架构 updater 随 preinst 携带，安装时释放到独立、可执行的临时目录，校验后执行。先在真实 dpkg 上验证脚本大小、失败状态与同版本重装，再确定封装。 | [preinstall](../../src/publish/deb_pkg/buckyos_preinstall#L3) · [控制脚本生成](../../src/publish/make_local_deb.py#L415) |
| Linux RPM | `%pre` 同样需要在解包前拿到目标 updater。沿用 DEB 的封装验证，测试 rpm 头部/scriptlet 的体积限制与同版本 reinstall。若单包无法可靠携带，改为随离线介质交付、强制同版本的前置 helper 包，并先验证完整离线安装和重试。 | [preinstall](../../src/publish/rpm_pkg/buckyos_preinstall#L3) · [scriptlet 生成](../../src/publish/make_local_rpm.py#L569) |

**同步修正覆盖范围。** 当前 Linux 的部分入口会忽略停服错误并删除整个 `bin/`，而 `bin/applist.json` 在清单中属于保留数据。在线脚本还有直接写入 cluster 配置的路径，平台 postinstall 还会处理 `etc/start_config.json` → 安全目录。逐项按“程序文件、缺失才 seed、已有数据由 updater 转换”核对，移除整目录误删，将旧配置的位置迁移交给其 reader，并传入真实的 `BUCKYOS_SECURITY_ROOT`。覆盖成功才启动。迁移成功后部署报错，保留新数据。交互安装提示重跑同版本安装包，无人值守安装按上节记录续装。[保护路径](../../src/bucky_project.yaml#L145) · [Linux 清理](../../src/publish/deb_pkg/buckyos_preinstall#L3) · [启动配置现有搬移](../../src/publish/deb_pkg/buckyos_postinstall#L27)

**验证停机窗口覆盖整个写入过程。** Linux 的 systemd unit、macOS 的 LaunchAgent 和 Windows 的 `BuckyOSNodeDaemonKeepAlive` 都可能再次拉起服务。停服步骤要处理相应的守护机制，并确认写库进程已退出。只有所有文件就位才恢复守护并启动。用同一目标 root 做安装互斥，重跑从新包解压 updater 和停服工具，避免依赖上次覆盖到一半的程序。[Linux unit](../../src/publish/deb_pkg/buckyos_postinstall#L51) · [macOS 停服](../../src/publish/macos_pkg/scripts/buckyos_preinstall#L66) · [Windows keepalive](../../src/publish/win_pkg/scripts/buckyos_postinstall.ps1)

## 5. 实施顺序与验收

| 顺序 | 交付 | 可以进入下一步的条件 |
| --- | --- | --- |
| 1 · 存储基线 | reader 注册文件、本机 SQLite 实例定位、System Config 的 sled 快照，以及 Task Manager、Message Center 的 SQLite 测试夹具 | 现行格式通过，未建库可跳过，开发期旧版、缺少必需标记及未知布局均被拒绝。非 SQLite 的实例声明在准备阶段报错。探测前后数据一致，缺失路径仍然缺失。 |
| 2 · 迁移程序 | Rust CLI、JSON 结果、System Config 快照校验、各模块暂存与校验、最终 apply 及恢复；按第 1 节扩展其他 reader | System Config 已校验后让最后一个 AppData reader 失败，验证 apply 调用为零，全部正式数据和 schema 与升级前一致。有变更的成功路径只进入一次最终 apply。在跨数据区提交中途及整批提交后、返回前分别中断，验证恢复一致后方可启动，已提交的转换只生效一次。SDK 预检另测交互确认和取消。 |
| 3 · 共用发布产物 | CI 和构包验证目标 updater 可在旧服务停止、旧目录不完整的机器上独立运行 | 所支持 OS/架构载荷均有正确二进制；无网络的离线预安装能运行它。构包 staging、仅装客户端、首次全新安装分别走正确分支。 |
| 4 · 平台安装与故障演练 | 在线 Linux/macOS/Windows，离线 DEB/RPM/PKG/NSIS，更新相应 `verify-pkg`、平台模板测试和在线安装器测试 | 用临时安装根或平台 VM 验证：停服失败或 updater 失败时，旧程序文件完整；成功后故意中断覆盖，重跑同包，所有已迁模块跳过并启动目标版；既有 System Config、AppData、`bin/applist.json` 和个人文件保留。 |
| 5 · 发布审计 | 每个数据所有者确认支持范围，生成“BuckyOS 发布版本 → 模块 schema”记录；同步在线脚本与离线打包子模块版本 | 正式发布用的脚本和 staging 均通过上述测试。以后 reader 或被登记的格式文件变更时，复用注册表提示 schema 审计。 |

第 4 项先用同一处 SDK 冲突检验参数。普通安装和单传 `--auto-update` 时均让用户选择继续或取消；单传 `--silent` 时，即使在终端运行也应在停服前自行非零退出。

Linux 定时验收在无显示会话、无 TTY、stdin 接 `/dev/null` 的环境运行上述 cron 命令，以日志、退出码和服务启停次数核对结果。先发布一个兼容的新构建，验证首次检查成功升级，第二次检查返回 0 且没有下载 payload 或重启服务。模拟 SDK 冲突、锁被占用和下载失败，验证任务在停服前非零退出。再让最后一个 reader 在停服后提出需确认的事项，验证脚本自行非零退出、临时结果已清理、正式数据和旧程序完整、原服务恢复。最后在覆盖中途终止安装并推进 `latest`，验证次日先完成原构建，再次检查才升级到新的 `latest`。

本计划基于源码核对。库内样本、Linux 预安装脚本能否容纳实际编译体积、各平台包管理器在故障重装时的行为，均列为实施验收项；本次未执行真实数据迁移或安装。
