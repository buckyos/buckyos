# Agent Daemon：Linux 接入与执行协议

当前交付独立 Linux 宿主及平台接口骨架。macOS/Windows GUI、Android、桌面 helper 和截图不在这次实现范围。Linux 不依赖桌面组件、Docker、BuckyOSApp、目标机网关或 Device DID。

## 使用与构建

控制面 Settings → Cluster Manager → Nodes/Devices 创建轻量设备，设置获准 Agent 的 profile 和到期时间。创建记录不会使执行端上线。拿到 URL 后运行独立包的 `start-buckyos-agent-daemon.sh '<URL>'`；脚本校验产物摘要并 `exec` 前台进程。也可预下载包，通过 `--artifact FILE --sha256 HASH` 启动。

```sh
# 在 buckyos/src 执行；只构建独立宿主，不准备 Web/SDK 分发包
uv run buckyos-build.py --agent-daemon

# 避免在进程参数中传递 URL；读到 EOF 后开始接入
buckyos_agent_daemon run --url-stdin
# 凭据文件必须属于当前用户且仅该用户可读
buckyos_agent_daemon run --url-file /private/access-url
buckyos_agent_daemon doctor --json
buckyos_agent_daemon status --executor <executor_id> --json
buckyos_agent_daemon stop --executor <executor_id>
```

`--data-dir` 可显式指定独立数据根；默认 `$XDG_DATA_HOME/buckyos-agent-daemon` 或 `$HOME/.local/share/buckyos-agent-daemon`。它不使用 `$BUCKYOS_ROOT`，与完整 Node 的身份、端口、ACL 和账本隔离。宿主只支持当前 OS 身份，任何 `run_as` 配置都拒绝；完整 Node 保留默认的系统降权模式。Linux 命令后端需要 Bash，文件后端不需要 GUI 库。`workdir_roots` 约束 cwd 和文件 API，不是 shell 沙箱。

发布打包入口为 `src/publish/agent_daemon/package.py`，产物携带平台/架构/运行依赖清单与 SHA-256 摘要。发布脚本通过 HTTPS 下载，不允许关闭 TLS 验证。开发测试的 `--allow-loopback-ws` 只允许回环 WS，不允许远程明文接入。生产 WSS 由现有 Zone HTTPS 网关终止 TLS，再路由到 control-panel。

部署时需同步更新 control-panel、Desktop 与 SDK/CLI，并确保目标机信任 Zone 的 TLS 证书。当前网关源码支持这些路径的 Upgrade、认证头和 query 转发。本轮 E2E 使用独立 TLS 代理与新 control-panel fixture，未替换现有生产服务，未把生产 443 入口的完整 WSS 会话标为已验证。

本轮本地安装包位于 `/tmp/buckyos-agent-daemon-dist/`，尚未上传公共下载地址；可先使用 `--artifact` 与对应 `.sha256` 文件中的摘要启动。x86_64 musl 包为静态链接；aarch64 GNU 包需要 glibc 2.39 或更高版本，以及 libgcc_s/libm。两个包均已在 Ubuntu 24.04 无桌面虚拟机验收，ARM64 使用 QEMU TCG 模拟。测试入口与报告见 [Linux 验收说明](../../test/agent_daemon/README.md)。

## 控制面与授权

管理 RPC 使用现有 control-panel 用户会话与管理员权限：

- `executor.device.create`：创建稳定 UUID、profile、有效期；仅此响应返回新 URL。
- `executor.device.list`：持久管理记录和当前在线状态，不返回秘密。
- `executor.device.issue`：轮换 URL，旧连接和凭据失效。
- `executor.device.revoke` / `executor.device.delete`：立即摘除并关闭关联连接。
- `executor.online.list`：仅 App session 可用，按真实 Agent/App binding、安装状态、permission_mode 和 profile 过滤在线条目。
- `executor.stream.open`：必须提供 `executor_id/generation/profile_id`，校验实时路由后签发短期单次流票据。

记录存入 `services/control_panel/agent_executors/<executor_id>`，不写入 scheduler 消费的 `nodes/*` 或 `devices/*`。秘密仅保存 SHA-256 摘要；在线连接和流票据只存在内存，服务重启后必须重新接入。轻量执行端不是调度 Node，不需要新增 scheduler ServiceSpec。

profile 和 Agent binding 复用 `ExecutorConfig`，WSS 配置使用 `validate_execution()`，完整 Node 的 `validate()` 仍要求 RTCP Device ACL。RBAC 资源分别为 `obj://executor/<id>/<profile>` 和原有 Node 资源。独立宿主只信任已认证 WSS 下发的授权快照，不初始化全局 BuckyOS runtime。

## WSS 与 Command Stream

共享类型位于 `src/kernel/buckyos-api/src/agent_daemon.rs`，接入协议版本为 1。

URL 形式为 `wss://<zone>/kapi/control-panel/executor/manage?executor_id=<uuid>#<secret>`。daemon 将 fragment 中的秘密移入 `Authorization: Bearer`，不在请求 URL、日志、状态文件中保留。

管理通道按顺序执行 `hello → welcome → ready`。hello 带进程级随机 instance_id；welcome 带服务端时间、绝对到期时间、配置、generation 和仅驻留内存的恢复凭据。核心初始化及配置验证成功后才发送 ready。控制面收到 ready 才公布可用目标。

每 5 秒发送 heartbeat/授权 snapshot；15 秒未收到有效心跳则移除在线路由。正常 goodbye/关闭立即移除。重连换 generation，同进程必须出示恢复凭据才能接管仍保留的旧连接。旧 heartbeat/close 不影响新 generation。独立的第二个进程不能凭同 URL 接管活动实例。

Agent 使用 `executor.stream.open` 取得 Agent 侧票据和 route URL；管理通道同时通知 daemon 打开独立 data WSS。两侧票据互不通用，绑定 executor、generation、profile、请求方、短期有效期，使用一次即消耗。浏览器以首帧发送票据，收到 `{ready:true}` 后才发送 Command Stream；原生客户端可使用认证头。

data/route WSS 只承载现有 Command Stream 二进制字节，不新增 HTTP shell 协议。共享适配器处理消息边界、ping/pong/close、有界缓冲和背压。daemon 在送入核心前检查首个 login 的真实 App token 摘要及 profile，与控制面已验证的 App 会话一致；接入 URL 不能替代 Agent 登录。操作仍使用已有 op_id/store_id/query/cancel、账本去重和限额。

## 生命周期与结果

URL 有效期约束整个在线生命周期。daemon 根据服务端时间建立单调截止时间；Linux 同时使用包含休眠时间的 `CLOCK_BOOTTIME`，重连和本机时钟回拨不能延长第一次确认的截止时间。所有数据流读取还检查管理租约门禁。

管理连接失效时暂停新操作、关闭数据流并重新接入；不重放已接收的命令。到期、撤销、SIGINT、SIGTERM 或显式 stop 停止接单，取消受管进程组并退出。普通数据流断开不自动取消已接受操作，后续通过相同账本查询。无法确定是否完成时保持 Unknown，不把离线当成功或已取消。

Linux capability 把 command/file 的支持与就绪分开；ui/accessibility/capture/input 永远为 `unsupported`。直接 `ui.*` 请求返回 Capability 错误，不提示安装桌面或申请 GUI 权限。macOS/Windows 后续可在同一宿主/能力边界接入原生后端。

## 本地数据与更新

安装包附带 `service.sh`，显式选择 Linux systemd user service。安装会私有保存 URL；默认启动脚本不会调用它。需要当前用户已有可用的 systemd 用户会话；脚本不设置 lingering，也不提升权限。

```sh
./service.sh install --binary ./buckyos_agent_daemon --sha256 <binary_sha256> --url-file /private/access-url
./service.sh stop
./service.sh upgrade --binary ./new/buckyos_agent_daemon --sha256 <new_binary_sha256>
./service.sh uninstall
```

二进制摘要取自包的 manifest；URL 文件需为当前用户所有且权限 0600。服务设置 `Restart=no`，到期或撤销不会被服务管理器反复启动。升级先校验新产物再停止旧进程；启动失败回退旧二进制。卸载删除该服务及保存的接入凭据，保留执行账本与用户业务文件。

每个 executor_id 使用独立 0700 目录。`ledger/` 复用 Node Executor Store 的单写锁、store_id、操作记录与保留策略；`status.json` 为 0600，只含执行端标识、PID/启动时间、uid、状态和能力。stop 校验 PID 对应的进程启动时间，避免误停复用该 PID 的其他进程。URL、恢复秘密和 App token 默认不落盘。

更新前先 stop，确认进程退出后切换到新的校验通过的产物，再以同一 URL/数据根启动。保留 ledger 才能查询旧结果；删除 ledger 是显式清理，不等同于 revoke、退出或删除控制面记录。回退旧产物也不能自动重新执行未知动作。当前默认交付前台可移除包，不修改完整 Node 的 systemd、DEB/RPM 生命周期；独立可选常驻服务须使用其专用安装入口。移除产物不删除用户工作文件。
