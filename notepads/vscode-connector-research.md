# VS Code Connector 研究报告

日期：2026-11-01。源码基线：buckyos `b719ec6dc`；VS Code 上游 `microsoft/vscode` main（CLI 源码 `cli/src/commands/serve_web.rs`、`cli/src/util/command.rs`、`cli/src/update_service.rs`、`src/vs/code/node/cli.ts`），本机官方 CLI 1.144.0（commit `2c33b50b`）。

背景：贡献者希望尽快把日常工作迁进 BuckyOS（狗粮需求），写代码是第一位。2026-11-01 用户定方向：BuckyOS 上的 App 不是 VS Code 本身，而是 **VS Code connector**——连接用户机器上已经装好的官方 VS Code，符合"BuckyOS 链接用户既有安装"的逻辑。本文核对这个方向的事实依据和缺口。

本文只做源码与官方 CLI 帮助/源码级分析。**还没有实际运行 serve-web**：运行需要接受 VS Code Server 许可（`--accept-server-license-terms`），要由用户本人同意（§6）。

## 0. 结论

| 问题 | 结论 | 依据 |
| --- | --- | --- |
| 官方能提供什么 | `code serve-web`：官方 Web UI + 官方 Server，能用微软扩展市场。三平台桌面安装都带（`bin/code-tunnel[.exe]`），Windows x64/arm64、macOS x64/arm64、Linux x64/arm64 都有 Server 包 | §1 |
| connector 能否今天就做 | **Lite 上可以，零内核改动**：做成 `product.json` 预装 Deno 脚本 App（已支持 `--allow-run`），node_daemon 的原生进程生命周期现成。Windows/macOS Lite 以登录用户身份运行，正合适；Linux Lite 安装包也是 root（§4.2） | §2 |
| 必须改的一处 | serve-web 回 `X-Frame-Options: SAMEORIGIN` + `frame-ancestors 'self'`，Desktop 跨子域 iframe 打不开 → connector 做反向代理并改写这两个头 | §1.4 |
| 推广成通用能力的方向 | **复用 Node Executor 的 owner 开放策略与身份模式**，不另起一套：放置条件、运行身份（A-desktop 当前用户 / A-system `run_as`）、监听 `127.0.0.1` 都已有先例 | §3 |
| 运行身份 | Windows、macOS：node_daemon 就在登录用户的交互会话里，connector 天然是"用户自己的 VS Code"。**Linux（Full 和 Lite 安装包）都是 systemd root**：直接跑等于 root 终端；按 `run_as` 降权又写不了 nfs-server 以 root 身份建的 home 文件——这是要拍板的核心矛盾 | §4.2 |
| Zone 路径 → 本地路径 | home 是 `$BUCKYOS_ROOT/data/home/<user>` 下的普通文件，现成 helper `dfs_to_data_path`；VS Code 的外部修改会被 nfs-server 当作 bypass 修改识别 | §4.4 |
| 顺带发现的缺陷 | deb 的 systemd unit 没有 `KillMode=process`（rpm 有、代码注释也声称有）→ 重启/升级 buckyos.service 会把原生子进程连同 VS Code 终端里正在跑的构建一起杀掉 | §4.7 |
| 仍需拍板 | 见 §5（6 项） | |

## 1. 官方 `serve-web` 的事实

### 1.1 安装形态

- 独立 CLI 与桌面安装都提供。桌面安装的 `code` 脚本把 `tunnel` / `serve-web` / `agent` 三个子命令（`argv.ts` `NATIVE_CLI_COMMANDS`）转给同目录 `bin/<tunnelApplicationName>`：Windows `<安装目录>\bin\code-tunnel.exe`，macOS `Visual Studio Code.app/Contents/Resources/app/bin/code-tunnel`，Linux `<安装目录>/bin/code-tunnel`（`cli.ts`）。官方 stable 的 `tunnelApplicationName` 是 `code-tunnel`（本机 Server `product.json` 已核对）。connector 应直接调 `code-tunnel`，绕开 Electron-as-Node 与 Windows `cmd` 两层壳。
- Linux 的 `code` 脚本以 root 运行时会拒绝启动，`tunnel` / `serve-web` 例外（`resources/linux/bin/code.sh`）。
- Server 包按平台下载：`server-linux-x64-web`、`server-linux-arm64-web`、`server-darwin[-arm64]-web`、`server-win32-x64-web`、`server-win32-arm64-web`；Linux armhf 没有（`update_service.rs`）。

### 1.2 Server 获取、缓存、离线

- 启动时不下载；第一次有请求进来才按需下载并解压，按 commit 缓存在 CLI 数据目录下（`--cli-data-dir`，环境变量 `VSCODE_CLI_DATA_DIR`）。下载期间页面收到 202 + 自动刷新页。
- 没指定 `--commit-id` 时每 3600 s 查一次最新版；查询失败而本地有缓存时用旧版继续服务；**没缓存又没网时返回 500**。
- `--commit-id <sha>` 固定版本，不再查更新。connector 可固定为本机桌面版的 commit，Web 与桌面同版本。首次仍要下载该 commit 的 Server 包（桌面安装里没有 Server），所以"首次使用必须联网"无法避免。
- 缓存没有淘汰逻辑，版本会累积。

### 1.3 进程模型

- serve-web 本身是 TCP（或 `--socket-path` 指定的 Unix socket / named pipe）上的反向代理；真正的 Server 以普通子进程启动，二者之间走 socket，不占 TCP 端口；WebSocket 由 `copy_bidirectional` 拼接。
- 子进程不建新进程组、不 `setsid`（Windows 只加 `CREATE_NO_WINDOW`）。无客户端 1 小时后 serve-web 用 `kill_tree` 收掉 Server（Unix 逐层 `pgrep -P`，Windows `taskkill /t`），下次请求再拉起。
- 对 node_daemon 的含义：node_daemon 以 `setsid` 启动原生 App、停止时对整个进程组 `kill -TERM/-KILL -<pid>`（[service_pkg.rs:934](../src/kernel/node_daemon/src/service_pkg.rs#L934)），Windows 用 `taskkill /T`。serve-web 与 Server、扩展宿主同组，会被一并收掉。**终端里的 shell 不在此列**：node-pty 为每个终端 `setsid`，它们在 pty 主端关闭时收到 SIGHUP 退出，`nohup` 的残留按 OS 语义保留——与"执行器子进程遵循 OS 父子进程语义、不追杀 nohup 残留"的既定原则一致。（推断，待实测。）

### 1.4 路由、鉴权与 iframe

- URL 布局 `/<quality>-<commit>/...`，`--server-base-path` 可整体加前缀。
- `--without-connection-token` 关闭 token；否则 token 写在 `<cli-data-dir>/serve-web-token`，URL 带 `?tkn=`。另有 `vscode-cli-secret-half`（HttpOnly、SameSite=Strict）与 `vscode-secret-key-path` 两个 cookie，用于浏览器端加密存储密钥。
- **所有代理响应都带 `X-Frame-Options: SAMEORIGIN` 和 `Content-Security-Policy: frame-ancestors 'self'`**。Desktop 在 Zone 根域，App 在子域，属于跨源 iframe，会被浏览器拒绝。connector 反向代理时把它们换成 `frame-ancestors <Desktop 源>`。子域与根域同站，SameSite=Strict 的 cookie 在 iframe 里照常发送。
- 浏览器端的 crypto.subtle、Service Worker、剪贴板都要求安全上下文：Zone 域名 HTTPS 没问题，局域网 IP 的 HTTP 模式会失效。

### 1.5 用户数据与扩展

- `--server-data-dir` 默认就是 Remote-SSH 用的 `~/.vscode-server`。对今天用 Remote-SSH 连 Linux 开发机的贡献者，serve-web 与 Remote-SSH 会共用同一套远端扩展和 Machine 设置。（推断，待实测。）
- 桌面版的本地扩展（`~/.vscode/extensions`）不会出现在 Web 里，和 Remote-SSH 的体验一样；Settings Sync 可同步。

### 1.6 许可边界

VS Code Server 许可：只能与 VS Code 一起用；不得 "host, share, publish, rent or lease"；不得 "combine it with any of your applications for others to use"。所以 connector 不打包、不分发 Server，只调用户自己装的 VS Code，只给 owner 用（不开 `allow_guest`，不分享给 Zone 内其他用户）。另：1.144 新增 `code agent host`（无头 agent 会话服务，可远程访问），以后可以评估接到 BuckyOS 的 Agent/消息体系，本文不展开。

## 2. 最小形态：Lite 预装 Deno 脚本 App

可用的现成能力：

- `product.json` 的 `preinstalled_script_apps` 已支持任意 Deno 权限（systest 用了 `--allow-run`，[lite.json](../src/product/lite.json)），由 node_daemon 用节点自带 Deno 原生运行（[app_loader.rs:1781](../src/kernel/node_daemon/src/app_loader.rs#L1781)）。
- `NativeProcess` 提供 pid 记录、崩溃退避、端口就绪、日志轮转（[native_app.rs](../src/kernel/node_daemon/src/native_app.rs)）。
- 网关对私有 App 校验 `buckyos_session_token`，并要求它绑定该 App 的 appid、实例和 owner（[boot_gateway.yaml:366](../src/rootfs/etc/boot_gateway.yaml#L366)）；同节点转发到 `127.0.0.1:<port>`（[boot_gateway.yaml:441](../src/rootfs/etc/boot_gateway.yaml#L441)）。Desktop 的 `prepare_app_launch` 免密打开现成。

connector 脚本职责：

1. 发现：PATH 与三平台常见安装位置找 `code-tunnel`（含 Insiders 与用户级安装），读出 commit。
2. 没找到：在 `BUCKYOS_SERVICE_PORT` 上回一个"未检测到 VS Code"页面（含安装指引），不崩溃、不偷装。
3. 找到：`code-tunnel serve-web --socket-path <app 私有目录>/vscode.sock --without-connection-token --accept-server-license-terms --commit-id <桌面版 commit> --disable-telemetry`。用 socket 而不用 TCP，Server 只有同用户进程能访问，也不占端口、不会端口冲突。
4. 代理：自身监听 `127.0.0.1:$BUCKYOS_SERVICE_PORT`，HTTP 与 WebSocket 转发到 socket，改写 §1.4 的 iframe 头。
5. 入口：`?folder=<本地路径>` 打开目录；浏览器就在本机时提供 `vscode://file/<路径>` 交给桌面版（完整功能）。

许可同意：首次打开时由 connector 页面展示 VS Code Server 许可链接，用户点同意后才带 `--accept-server-license-terms` 启动，同意记录存在 App 数据目录。

## 3. 推广：复用 Node Executor 的 owner 开放策略

connector 的本质是"owner 的 App 在某个节点的宿主上运行"，和原生 owner Agent 是同一类授权。Node Executor 已经把这件事建模好了（[node_exector.md §4.2](../doc/arch/node_exector.md)）：

| connector 需要的 | Node Executor 已有的 |
| --- | --- |
| 跑在哪个节点 | `nodes/<node>/settings.executor.owner_agents`：节点对本 Zone owner 开放宿主；原生 owner Agent 用它 + `node_affinity` 定位节点（[app_schema.rs:717](../src/kernel/buckyos-api/src/app_schema.rs#L717)、[system_config_agent.rs:96](../src/kernel/scheduler/src/system_config_agent.rs#L96)） |
| 以谁的身份跑 | A-desktop（Windows、macOS Desktop：node_daemon 在用户交互会话里，`CurrentUser`，以登录用户身份执行）；A-system（root 守护进程，`SwitchUser`，降权到 profile 的 `run_as` uid/gid）（[node_executor_host.rs:196](../src/kernel/node_daemon/src/node_executor_host.rs#L196)） |
| 只监听回环 | "Executor 在形态 A/B 监听 `127.0.0.1`，由 gateway 接入"（node_exector.md §8） |
| Desktop 种子 | Desktop 版首次以桌面模式启动时写入 `owner_agents.enabled=true`、roots=[登录用户 home] |

据此，connector 的推广形态：

- **放置**：AppDoc 声明"宿主原生"（新字段，见 §5-1），调度器对它 `need_container=false`、`node_affinity`=owner 已开放宿主的节点、`run_mode=Native`，不论节点支不支持容器。现状下脚本 App 在支持容器的节点上一律进容器（[app.rs:51](../src/kernel/scheduler/src/app.rs#L51)），进了容器就看不到宿主软件——这是必须补的一处。
- **身份**：A-desktop 节点直接继承 node_daemon 的用户身份——正是 connector 要的（真实工具链、git/ssh 凭据）。A-system 节点（Linux 安装包，systemd root）要么以 root 跑，要么按 `run_as` 降权；node_daemon 的 `spawn_detached_logged` 目前不降权，降权需复用 executor 的 `drop_privileges`（[spawn.rs:150](../src/kernel/node_executor/src/spawn.rs#L150)）。两条路各有代价，见 §4.2。

## 4. 缺口逐项

### 4.1 "必须宿主原生"的声明

见 §3。只有 Lite 预装路径不需要这个声明（Lite 节点 `support_container=false`，D1 auto 自然给 Native）。Full 版开发机（贡献者最常见的环境）需要它。

### 4.2 运行身份

原生子进程由 `spawn_detached_logged` 启动，不切换 uid/gid、不清环境，完整继承 node_daemon 的身份与环境（[service_pkg.rs:1017](../src/kernel/node_daemon/src/service_pkg.rs#L1017)；Lite TODO 已把"原生进程以 node_daemon 身份运行"列为已知风险）。各平台 node_daemon 的身份：

| 平台 / 档位 | node_daemon 怎么起 | 身份与会话 | 对 connector |
| --- | --- | --- | --- |
| Linux Full（deb/rpm） | postinstall 写 `/etc/systemd/system/buckyos.service`，`User=root`（[deb buckyos_postinstall:50](../src/publish/deb_pkg/buckyos_postinstall#L50)） | root，无登录会话、无 DISPLAY | A-system |
| Linux Lite（deb） | 同一个 root systemd unit，只是去掉 Docker 依赖；Linux 没有 computer_driver 且 euid=0，executor 仍是 SwitchUser（[node_executor_host.rs:161](../src/kernel/node_daemon/src/node_executor_host.rs#L161)） | root，无 GUI 会话 | A-system |
| Windows（Full/Lite 相同） | 不是 Windows 服务（`--as_win_srv` 无人传）；计划任务 `BuckyOSNodeDaemonKeepAlive`（无 `/RU`）+ `HKCU\...\Run`，经 vbs 隐藏启动（[buckyos_postinstall.ps1:33](../src/publish/win_pkg/scripts/buckyos_postinstall.ps1#L33)） | 安装用户的交互会话（不是 session 0）；Lite DV 已见所有原生进程属控制台用户 | A-desktop。坑：标准用户用另一个管理员账号提权安装时，HKCU 与计划任务落在管理员的配置里（推断） |
| macOS（Full/Lite） | `BuckyOS Runtime.app` 作为绑定用户的 LaunchAgent（`gui/<uid>`、Aqua），`register` 拒绝 root，`desktop_runtime.json` 记录绑定用户（[node_control.rs:1296](../src/kernel/buckyos-api/src/node_control.rs#L1296)） | 绑定用户的 GUI 会话；用户未登录 GUI 时整个 node_daemon 不跑 | A-desktop |

没有找到任何"Zone 用户 → OS 账号"的进程身份映射，也没有 CreateProcessAsUser 一类调用。已有的只有：executor profile 的 `run_as {uid,gid}`（Linux 开发种子用 65534），以及 smb_service 按 Zone 用户名建的 OS 账号（Linux `useradd -s /sbin/nologin`、macOS `dscl` 隐藏用户，只用于 Samba 登录）。

**Linux 上的核心矛盾**：nfs-server 也由 node_daemon 拉起，home 里的文件归 root 所有（postinstall 只把 `data/home` 等顶层目录设成 0777）。于是：

- connector 以 root 跑：能编辑 Zone 文件，但浏览器里拿到的是 root 终端。对贡献者自己的开发机，这和他们今天 SSH 成 root 等价（本机即如此）；对家用 NAS 这类"owner 不该等于 OS root"的设备不合适。
- connector 按 `run_as` 降权：终端安全了，但写不了 root 建的 home 文件，"用 VS Code 改 Zone 里的文件"失效；还要给降权账号准备 home、工具链和 git 凭据。
- 中间路线：用 smb_service 已有的"按 Zone 用户名建 OS 账号"做身份映射，并让 nfs-server 对该用户 home 按这个账号落盘（或加组 ACL）。这是新的跨服务工作，不是 connector 自己能解决的。

### 4.7 deb 的 systemd unit 缺 `KillMode=process`（单列缺陷）

rpm 的 unit 有 `KillMode=process`，`node_control.rs:113`、`:553` 的注释也声称 Linux unit 已显式设置；deb 的 unit 没有（已核对）。systemd 默认按 cgroup 杀，所以 deb 安装上 `systemctl restart/stop buckyos` 会把 node_daemon 拉起的所有原生进程一并杀掉。对狗粮场景影响直接：贡献者升级 BuckyOS 时，VS Code 终端里正在跑的构建会跟着死。与 connector 无关，可单独修。

### 4.3 准入与信任

现状只有 `product.json` 预装的脚本 App 能原生运行，否则 `not_admitted`（[app_loader.rs:1746](../src/kernel/node_daemon/src/app_loader.rs#L1746)）。官方 connector 走预装没问题。第三方 connector 拿到的是用户全部权限，放开前需要信任模型；按"先能力后安全"，建议先只允许官方 connector。

### 4.4 Zone 路径到本地路径

home 是磁盘上的普通文件：nfs-server 以本地文件系统为真源，把 `$BUCKYOS_ROOT/data/` 的一级目录作为导出根（[config.rs:155](../src/frame/nfs_server/src/config.rs#L155)、[namespace.rs:191](../src/frame/nfs_server/src/namespace.rs#L191)），filedb 只存视图、绑定、元数据与 ACL。`/home/<user>/Documents/x`（即 `cyfs:///home/<user>/Documents/x`）对应 `$BUCKYOS_ROOT/data/home/<user>/Documents/x`，现成 helper 是 `buckyos_api::dfs_to_data_path`（[object_url.rs:447](../src/kernel/buckyos-api/src/object_url.rs#L447)，逐段解码并拒绝 `..`；agent-did-object-lib 已这样用）。例外：`home/<user>/Apps/…`、`Collected/…` 是桶存储的只读投影，没有本地路径。

VS Code 直接改盘上文件，在 nfs-server 看来是 bypass 修改：扫描器会识别，并发提交冲突时报 `TARGET_MISMATCH{bypass_modified}`（nfs_server README）。这是可接受的既有语义，但要实测 Desktop 文件管理器与 AIWS 等编辑器同时打开同一文件时的体验。

Desktop 的 content handler 当前只支持 `open` 意图、`web` 入口，按 mime/objType/schema/ext 选择（[content_handler.rs](../src/kernel/buckyos-api/src/content_handler.rs)），传给 App 的是内容引用，不是本地路径；"用 VS Code 打开文件夹"需要目录类选择器和引用到本地路径的换算。

### 4.5 设备绑定

调度器只把实例放到 OOD / Server 节点（[scheduler.rs:1215](../src/kernel/scheduler/src/scheduler.rs#L1215)），而 Desktop 今天只能作为 OOD 激活，没有非 OOD 设备加入流程（G8）。所以现阶段 connector 只能连"OOD 那台机器上的 VS Code"：Lite 是用户自己的电脑，Full 是开发机/服务器。"连我笔记本上的 VS Code"要等设备加入流程，或评估由形态 B（buckyos-agent-daemon）承载 connector 的 Web 流量。

### 4.6 监听地址

10-26 的端口约定是 App 监听 `0.0.0.0:$BUCKYOS_SERVICE_PORT`（[buckyos-desktop-lite-todo.md](buckyos-desktop-lite-todo.md) T2）。connector 必须例外：它背后是不需要登录的终端，监听 `0.0.0.0` 等于把终端开放给局域网。同节点转发本来就走 `127.0.0.1`；跨节点经 ROUTES 的 rtcp 转发应当落在远端回环（推断，待验证）。端口占用检查按 `0.0.0.0` 绑定测试，对回环监听同样有效。

## 5. 待拍板

1. "宿主原生"怎么声明：AppDoc 新字段（如 `service_config_tips.runtime_caps.host_native = "required"`），还是新 `AppType`。
2. connector 用哪个节点开关：直接复用 `owner_agents`（"本机宿主对 owner 开放"一个开关管 Agent 和 App），还是在 `ExecutorSettings` 加平行的 `owner_apps`。
3. Linux 节点上 connector 以什么身份跑（§4.2）：root（贡献者开发机够用，NAS 不合适）、`run_as` 降权（写不了 home），还是先做 Zone 用户 → OS 账号映射。首期可以只做 Windows/macOS（A-desktop，无矛盾）。
4. 端口约定对 connector 的例外：是"宿主原生 App 一律监听回环"，还是 connector 自己决定。
5. 第三方 connector：首期是否只允许官方预装。
6. deb unit 补 `KillMode=process`（§4.7）：建议直接修，与本方向无关。

## 6. 待实测（需要用户同意 VS Code Server 许可）

运行 serve-web 要传 `--accept-server-license-terms`（或交互回答 Y），代表接受微软许可，须由用户本人同意后再做：

1. 本机 Linux：serve-web 首次下载与缓存位置；`unshare -n` 断网后用缓存能否服务；node_daemon 式 `setsid` + 进程组终止后有无残留（含终端里的后台进程）；`--server-data-dir` 默认值是否与 Remote-SSH 共用。
2. Windows VM 113 / macOS VM 114：装官方 VS Code，确认 `bin/code-tunnel` 位置与 `serve-web` 可用；`taskkill /T` 的收尾；A-desktop 会话下 `vscode://` 交接。
3. 端到端：预装脚本 connector 原型 + 网关 + Desktop iframe（改写 frame 头后）+ 扩展市场安装一个微软扩展（如 Pylance）。
