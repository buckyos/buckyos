# 11. 环境变量协议

环境变量是 BuckyOS 进程启动边界的一部分。它们不是持久化配置源，也不是 system-config 的替代品；它们只负责把“这个进程启动时必须知道的身份、路径、端口、调试覆盖项”从启动者传给被启动进程。

本文只按当前仓库实现整理。发生歧义时，以代码为准：

- `src/kernel/node_daemon/src/node_daemon.rs`
- `src/kernel/node_daemon/src/app_loader.rs`
- `src/kernel/node_daemon/src/kernel_mgr.rs`
- `src/kernel/buckyos-api/src/lib.rs`
- `src/kernel/buckyos-api/src/runtime.rs`
- `publish/aios/entrypoint.sh`

## 命名和边界

- `BUCKYOS_*` 是 BuckyOS 自有命名空间。新增对外契约优先放在这个命名空间。
- 动态 token 变量由 `get_session_token_env_key()` 生成：先把 app/service id 转成大写，再把 `-` 替换成 `_`。AppService 用 `*_TOKEN`，KernelService/FrameService 用 `*_SESSION_TOKEN`。
- `app_instance_config`、`app_media_info`、`local_app_instance_config` 是历史形成的小写内部变量，只在 node-daemon 拉起 app/local app 时使用；不要继续扩展新的小写变量。
- `OPENDAN_*`、`SCRIPT_*` 当前仍有兼容读写点，但新协议应优先用 `BUCKYOS_*` 和文件/RootFS 元数据表达。
- 含 token、私钥、API key 的变量不应写入普通日志。诊断脚本需要 redact。
- JSON 类变量必须传 JSON 字符串，不传文件路径或 JWT 字符串。

## 系统根和安装变量

| 变量 | 来源 | 消费方 | 语义 |
| --- | --- | --- | --- |
| `BUCKYOS_ROOT` | 用户、安装脚本、`start.py`、`node_control` | 路径工具、node-daemon、OpenDAN、安装/卸载脚本 | BuckyOS host 运行时根目录。类 Unix 默认 `/opt/buckyos`；Windows 默认 `%APPDATA%/buckyos`，部分工具再回退到 `%USERPROFILE%/buckyos`。 |
| `BUCKYOS_BUILD_ROOT` | 用户或打包命令 | `make_local_pkg.py`、`src/publish/make_local_*` | 本地打包/发布 staging root，默认类 Unix `/opt/buckyosci`，Windows `C:\opt\buckyosci`。 |
| `BUCKYOS_DEV_HOME` | 开发者 | `buckyos-api` AppClient runtime | AppClient 开发态配置目录覆盖；存在时优先于 `$BUCKYOS_ROOT/etc` 加载 runtime 配置。 |
| `APPDATA` / `USERPROFILE` / `LOCALAPPDATA` | OS | Windows 路径解析与打包变量展开 | Windows 平台默认目录和 installer 模板展开使用。 |

`BUCKYOS_ROOT` 是最基础的路径协议。它可以被开发脚本覆盖，但生产安装后应保持稳定；很多派生路径如 `$BUCKYOS_ROOT/etc`、`$BUCKYOS_ROOT/data`、`$BUCKYOS_ROOT/storage` 都依赖它。

## Boot 和系统服务变量

| 变量 | 来源 | 消费方 | 格式/语义 |
| --- | --- | --- | --- |
| `BUCKYOS_ZONE_DOC` | node-daemon boot 流程 | scheduler `--boot` | `ZoneDocument` JSON 字符串。只用于首次 boot schedule，不是 ZoneBootConfig JWT。缺失会导致 boot scheduler 失败。 |
| `BUCKYOS_THIS_DEVICE` | node-daemon boot 流程 | `buckyos-api` runtime、system-config、app/service runtime | `DeviceDocument` JSON 字符串。用于建立设备信任 key、服务 runtime 初始化，并会被 node-daemon 转发给 app worker。 |
| `BUCKYOS_ZONE_CONFIG` | node-daemon 从 `boot/config` 得到后设置 | `buckyos-api` runtime、app loader、app/service runtime | `ZoneConfig` JSON 字符串。runtime 用它确定 zone id、trust/config 入口；app loader 还会读取其中的 docker repo 配置。 |
| `SCHEDULER_SESSION_TOKEN` | node-daemon boot 流程 | scheduler `--boot`、scheduler thunk runner fallback | 设备签名 session token。boot scheduler 用它写 system-config。 |
| `<SERVICE>_SESSION_TOKEN` | node-daemon `kernel_mgr` | KernelService/FrameService runtime | 服务启动 token，动态变量名。例如 `verify-hub` 对应 `VERIFY_HUB_SESSION_TOKEN`。 |
| `SYSTEM_CONFIG_URL` | 开发/测试覆盖 | scheduler thunk runner | 覆盖 thunk runner 访问 system-config 的 URL；默认走本地 system-config URL。 |

Boot 阶段有两层 zone 变量：`BUCKYOS_ZONE_DOC` 是 scheduler 创建 `boot/config` 前的输入；`BUCKYOS_ZONE_CONFIG` 是 `boot/config` 已经存在后的运行时输入。

## App/Agent Worker 变量

node-daemon 的 `AppLoader` 是 app/agent worker 环境变量的权威注入者。`publish/aios/entrypoint.sh` 也会为裸镜像调试补默认值，但生产语义应以 node-daemon 注入为准。

| 变量 | 来源 | 消费方 | 语义 |
| --- | --- | --- | --- |
| `BUCKYOS_APP_ID` | node-daemon | app/agent/脚本、OpenDAN | app id，不包含 owner 前缀。 |
| `BUCKYOS_APP_TYPE` | node-daemon | `publish/aios/entrypoint.sh` | worker 分发类型：当前入口支持 `agent`、`script`、`custom`/空命令。 |
| `BUCKYOS_OWNER_USER_ID` | node-daemon | app/agent、路径与权限逻辑 | app 实例所属 owner user id。 |
| `BUCKYOS_DATA_DIR` | node-daemon | app/agent/脚本 | worker 内 app 持久 user/app data 目录，当前形态为 `/opt/buckyos/data/home/<owner>/.local/share/<app_id>`。 |
| `BUCKYOS_LOG_DIR` | node-daemon | app/agent | worker 内日志目录。 |
| `BUCKYOS_STORAGE_DIR` | node-daemon | app/agent | worker 内 app storage 目录。 |
| `BUCKYOS_PKG_SOURCE_DIR` | node-daemon/aios 默认值 | aios entrypoint | 上游只读 package 挂载，aios 默认 `/mnt/buckyos/pkg`。 |
| `BUCKYOS_PKG_DIR` | node-daemon/aios 默认值 | app/agent/aios entrypoint | worker 内可写 package 工作副本路径；aios 会把它指向 instance volume 的 `pkg`。 |
| `BUCKYOS_INSTANCE_VOLUME` | node-daemon/aios 默认值 | aios entrypoint、OpenDAN | 每个 app instance 私有可写执行卷，aios 默认 `/opt/buckyos/instance`。 |
| `BUCKYOS_EXTTOOL_DIR` | node-daemon/aios 默认值 | aios entrypoint | 节点共享 ExtTool 只读卷，aios 默认 `/opt/buckyos/tools`。entrypoint 会把其 `bin` 放到 `PATH` 前面。 |
| `BUCKYOS_SAFE_MODE` | node-daemon/aios 默认值 | aios entrypoint | `1` 时重置 package 工作副本和 sync metadata；默认 `0`。 |
| `BUCKYOS_SERVICE_PORT` | node-daemon | agent/service | app/agent 暴露服务端口。agent 未注入时 aios 入口默认 `4060`。 |
| `BUCKYOS_HOST_GATEWAY` | node-daemon | `buckyos-api` runtime | 容器访问 host 本地服务的 host 名。runtime 会把 app/frame service 的本地 system service URL 解析到该 host。 |

同时注入的内部变量：

| 变量 | 来源 | 语义 |
| --- | --- | --- |
| `app_instance_config` | node-daemon | `AppServiceInstanceConfig` JSON。`buckyos-api` 会从中解析 app id 和 owner。 |
| `<FULL_APPID>_TOKEN` | node-daemon | AppService 启动 token。`FULL_APPID` 当前由 `<owner>-<app_id>` 组成，转大写并把 `-` 替换为 `_` 后加 `_TOKEN`。 |
| `app_media_info` | node-daemon | package media 信息 JSON；worker 内会把 `full_path` 改写为 `BUCKYOS_PKG_DIR`。 |
| `local_app_instance_config` | node-daemon | local app config JSON。 |
| `loca_app_instance_config` | node-daemon | `local_app_instance_config` 的拼写兼容变量。不要新增依赖。 |

aios entrypoint 派生或兼容变量：

| 变量 | 来源 | 语义 |
| --- | --- | --- |
| `DENO_DIR` | aios entrypoint | Deno cache，默认 `$BUCKYOS_INSTANCE_VOLUME/deno-cache`。 |
| `UV_CACHE_DIR` | aios entrypoint | uv cache，默认 `$BUCKYOS_INSTANCE_VOLUME/uv-cache`。 |
| `NPM_CONFIG_CACHE` | aios entrypoint | npm cache，默认 `$BUCKYOS_INSTANCE_VOLUME/npm-cache`。 |
| `PIP_CACHE_DIR` | aios entrypoint | pip cache，默认 `$BUCKYOS_INSTANCE_VOLUME/pip-cache`。 |
| `SCRIPT_APP_ID` | aios entrypoint | 旧 script-service 兼容变量，值来自 `BUCKYOS_APP_ID`。 |
| `SCRIPT_PACKAGE_ROOT` | aios entrypoint | 旧 script-service 兼容变量，值来自 `BUCKYOS_PKG_DIR`。 |
| `SCRIPT_DATA_ROOT` | aios entrypoint | 旧 script-service 兼容变量，值来自 `BUCKYOS_DATA_DIR`。 |
| `OPENDAN_SERVICE_PORT` | node-daemon | OpenDAN 兼容变量，值同 `BUCKYOS_SERVICE_PORT`。 |
| `OPENDAN_AGENT_ID` | node-daemon | OpenDAN 兼容变量，值同 `BUCKYOS_APP_ID`。 |

## AppClient 和 AgentTool 变量

| 变量 | 来源 | 消费方 | 语义 |
| --- | --- | --- | --- |
| `BUCKYOS_APPCLIENT_SESSION_TOKEN` | 外部客户端、AgentTool runner、调试脚本 | `buckyos-api` AppClient runtime、AgentTool | AppClient 调用 kRPC 的 session token。缺失时 AppClient runtime 会尝试从本地 user/device config 和私钥生成 token；AgentTool 需要访问 runtime 时要求该变量存在。 |
| `BUCKYOS_NODE_GATEWAY_PORT` | 调用方或调试环境可选覆盖；node-daemon/aios 当前不显式注入 | `src/tools/buckyos-agent` | 使用注入的 AppClient session token 时，指定容器访问宿主机 NodeGateway 的端口，默认 `3180`；不适用于外部 Zone 登录模式。若 `BUCKYOS_HOST_GATEWAY` 已包含端口，则以其中的端口为准。 |
| `BUCKYOS_AICC_TOOL_ROOT` | AIOS launcher 调试环境可选覆盖；node-daemon/aios 不注入 | `/usr/local/bin/aicc-tool` launcher | 覆盖只读 AICC TypeScript 工具安装目录，默认 `/opt/buckyos/bin/opendan/buckyos-agent`。仅用于镜像测试或非标准安装布局，不是 Agent 业务配置。 |
| `OPENDAN_AGENT_ROOT` | AgentTool runner | AgentTool runtime context | Agent RootFS 根目录。新的 AgentTool 最小契约之一。 |
| `OPENDAN_SESSION_ID` | AgentTool runner | AgentTool runtime context | 当前 agent session id。 |
| `OPENDAN_TRACE_ID` | AgentTool runner | AgentTool runtime context | 可选 trace id。 |

AgentTool 新实现目标是只依赖 `OPENDAN_AGENT_ROOT`、`OPENDAN_SESSION_ID`、`BUCKYOS_APPCLIENT_SESSION_TOKEN` 和可选 `OPENDAN_TRACE_ID`；其它 `OPENDAN_*` 应从 Agent RootFS、session state 或 BuckyOS runtime 推导。

## 服务私有和功能开关变量

这些变量属于具体模块，不应提升为全系统通用协议。

| 变量 | 模块 | 语义 |
| --- | --- | --- |
| `BUCKYOS_CONTENT_MGR_DB_PATH` | control-panel share content manager | 覆盖 share content manager SQLite/DB 路径。 |
| `BUCKYOS_TG_API_ID` | msg-center Telegram tunnel | Telegram API id，启用 grammers gateway 时必填。 |
| `BUCKYOS_TG_API_HASH` | msg-center Telegram tunnel | Telegram API hash，启用 grammers gateway 时必填。 |
| `BUCKYOS_TG_SESSION_DIR` | msg-center Telegram tunnel | Telegram session 目录覆盖。 |
| `BUCKYOS_KEVENT_RINGBUFFER_PATH` | kevent shared ringbuffer | 覆盖共享 ringbuffer 文件路径，默认 `/tmp/buckyos_kevent_ringbuffer_v2.shm`；测试中需要串行管理。 |
| `BUCKYOS_KEVENT_KMSG_CASES` | `test/kevent_kmsg` | 选择 kevent/kmsg 测试 case。 |
| `BUCKYOS_WEBSDK_ROOT` | sys_test | 覆盖 web sdk 搜索目录。 |

## 开发、构建和 DV Test 变量

这些变量主要由脚本和测试读取，可以作为开发工具契约，但不是 app/service runtime 契约。

| 变量 | 消费方 | 语义 |
| --- | --- | --- |
| `BUCKYOS_CYFS_GATEWAY_REPO` | `build.py` | 覆盖 cyfs-gateway git repo URL。 |
| `BUCKYOS_APP_REPO` | `build.py` | 覆盖 BuckyOSApp git repo URL。 |
| `BUCKYOS_VMTEST_ROOT` | `src/build_for_vm_test.py` | VM test rootfs staging directory。 |
| `BUCKYOS_WEB3_GATEWAY_ROOT` | `src/build_for_vm_test.py` | VM test web3-gateway staging directory。 |
| `BUCKYOS_SN_IP` | cyfs-gateway 仓库 `src/make_sn_config.ts` | 生成 SN 配置时覆盖 SN IP。 |
| `BUCKYOS_IDENTITY_ROOT` | `src/rootfs/etc/backup.py` | 覆盖 identity backup/restore 搜索根，默认 `$BUCKYOS_ROOT/local/identity`。 |
| `BUCKYOS_SECURITY_ROOT` | `src/rootfs/etc/backup.py` | 覆盖 security backup/restore 搜索根，默认 `$BUCKYOS_ROOT/security`。 |
| `BUCKYOS_SYSTEM_CONFIG_URL` | app installer/DV tests | 覆盖测试访问 system-config URL。 |
| `BUCKYOS_VERIFY_HUB_URL` | app installer/DV tests | 覆盖测试访问 verify-hub URL。 |
| `BUCKYOS_CONTROL_PANEL_URL` | app installer tests | 覆盖测试访问 control-panel URL。 |
| `BUCKYOS_TASK_MANAGER_URL` / `BUCKYOS_TASK_MANAGER_GATEWAY_URL` | app installer/kevent task manager DV tests | 覆盖 task-manager URL。 |
| `BUCKYOS_GATEWAY_BASE_URL` | kevent DV tests | 覆盖 gateway base URL。 |
| `BUCKYOS_TEST_APP_ID` | DV tests / tools | 测试 app id，常见默认 `buckycli`。 |
| `BUCKYOS_TEST_USER_ID` | DV tests | 测试用户 id，常见默认 `devtest`。 |
| `BUCKYOS_TEST_ZONE_HOST` | DV tests / tools | 测试 zone host，常见默认 `test.buckyos.io`。 |
| `BUCKYOS_TEST_APP_CLIENT_DIR` | DV tests / tools | 测试 AppClient 私钥搜索目录。 |
| `BUCKYOS_TEST_OWNER_DID` | app installer tests | 测试 owner DID。 |
| `BUCKYOS_TEST_DOCKER_BASE_IMAGE` | app installer tests | 测试安装包使用的 base image。 |
| `BUCKYOS_TEST_POST_INSTALL_SETTLE_MS` | app installer tests | post-install settle timeout。 |
| `BUCKYOS_TEST_INSTALL_EVIDENCE_TIMEOUT_MS` | app installer tests | 安装证据等待 timeout。 |
| `BUCKYOS_TEST_UNINSTALL_AFTER_INSTALL` | app installer tests | `1` 表示测试结束后卸载。 |
| `BUCKYOS_TEST_ADMIN_USER` / `BUCKYOS_TEST_ADMIN_PASSWORD` | control-panel tests | 管理员测试账号。 |
| `BUCKYOS_TEST_UV` | kevent restart DV tests | 覆盖 uv 可执行文件名/路径。 |
| `BUCKYOS_APP_CLIENT_DIR` | `src/tools/buckyos-agent` | 工具私钥搜索目录覆盖；兼容 `BUCKYOS_TEST_APP_CLIENT_DIR`。 |
| `BUCKYOS_ZONE_HOST` | `src/tools/buckyos-agent` / sys_test | 工具或测试访问的 zone host；兼容 `BUCKYOS_TEST_ZONE_HOST`。 |

## 标准外部变量

| 变量 | 使用点 | 语义 |
| --- | --- | --- |
| `PATH` | aios entrypoint、AgentTool shell | 命令搜索路径；aios 会把 `$BUCKYOS_EXTTOOL_DIR/bin` 放到前面。 |
| `HOME` | AppClient 私钥搜索、AgentTool、若干工具 | 用户 home 路径。 |
| `RUST_LOG` | slog/klog | Rust 日志过滤。 |
| `RUST_BACKTRACE` | 启动/调试脚本 | Rust backtrace。 |
| `RUST_BUILD` | `make_local_pkg.py` | 覆盖 Rust build output root。 |
| `VIRTUAL_ENV` | `make_local_pkg.py` | Python virtualenv 探测。 |

## 已出现但不应作为新依赖

| 变量 | 当前状态 |
| --- | --- |
| `BUCKYOS_KRPC_TIMEOUT_SECS` / `BUCKYOS_KRPC_TIMEOUT_SECS_<SERVICE>` | `buckyos-api` 中有常量声明，但当前 `get_zone_service_krpc_client_with_default_timeout()` 没有读取环境变量；不要依赖它们。 |
| `BUCKYOS_TEST_SYSTEM_CONFIG_URL` / `BUCKYOS_TEST_VERIFY_HUB_URL` | 只在 `test/test_helpers/buckyos_client.ts` 注释中出现，当前 helper 实现没有读取；测试覆盖应优先使用实际读取的 `BUCKYOS_SYSTEM_CONFIG_URL` / `BUCKYOS_VERIFY_HUB_URL`。 |
| `BUCKYOS_RUNTIME`、`BUCKYOS_FILE`、`BUCKYOS_SESSION_TOKEN`、`BUCKYOS_THIS_DEVICE_INFO` | 当前搜索只看到零散历史/文档/构建产物痕迹，未形成明确 runtime 契约。新增代码不要使用。 |
| `loca_app_instance_config` | 拼写错误兼容变量，只为旧实现保留。 |

## 新增或修改变量的规则

1. 先判断是否真的是启动边界。如果是持久状态、可调 settings、调度结果或权限策略，应进入 system-config 或文件协议，不应新增环境变量。
2. 新增正式变量使用 `BUCKYOS_*`；模块私有变量要写清楚模块所有者和默认值。
3. 涉及 app/service/Agent 可见变量时，同步检查 `node_daemon/app_loader`、`publish/aios/entrypoint.sh`、`buckyos-api runtime`、相关文档和测试。
4. 涉及 token 命名时复用 `get_session_token_env_key()`，不要手写另一个命名规则。
5. 改 JSON 变量的 schema 时按协议变更处理，同时检查前后端、文档和测试。
6. 环境变量是进程级全局状态，Rust/TS/Python 并行测试里修改变量必须串行化或用独立进程隔离。
