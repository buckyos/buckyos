# Agent Daemon：Linux/macOS 接入与执行协议

当前支持独立 Linux/macOS 当前用户宿主。macOS 14+ 提供原生桌面驱动，使用同一进程内的 ScreenCaptureKit、Accessibility 和 Quartz；Linux 的桌面能力仍为 unsupported。两者均不依赖 Docker、BuckyOSApp、目标机网关或 Device DID。Windows、语义树和模型图片结果桥接不属于本次 daemon 实现。

## 使用与构建

控制面 Settings → Cluster Manager → Nodes/Devices 创建轻量设备，设置获准 Agent 的 profile 和到期时间。桌面访问单独选择“截图”或“截图和输入”，默认禁用。创建记录不会使执行端上线。解压独立包，核对 `.sha256` 和 `manifest.json` 中的摘要后，运行其中的 `buckyos_agent_daemon`。

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
# macOS：人工接管/恢复桌面输入；恢复后必须重新观察
buckyos_agent_daemon pause --executor <executor_id>
buckyos_agent_daemon resume --executor <executor_id>
```

`--data-dir` 可显式指定独立数据根；macOS 默认 `$HOME/Library/Application Support/buckyos-agent-daemon`，Linux 默认 `$XDG_DATA_HOME/buckyos-agent-daemon` 或 `$HOME/.local/share/buckyos-agent-daemon`。它不使用 `$BUCKYOS_ROOT`，与完整 Node 的身份、端口、ACL 和账本隔离。宿主只支持当前 OS 身份，任何 `run_as` 配置都拒绝；完整 Node 保留默认的系统降权模式。命令后端需要 Bash，文件后端不需要 GUI 权限。`workdir_roots` 约束 cwd 和文件 API，不是 shell 沙箱。

独立打包入口为 `src/tools/package_agent_daemon.py`，产物携带二进制、使用说明、平台/架构/运行要求清单与 SHA-256 摘要。在仓库根执行 `uv run src/tools/package_agent_daemon.py --target aarch64-apple-darwin --output /tmp/agent-daemon-dist`，Intel Mac 使用 `x86_64-apple-darwin`。macOS 编译需要 Xcode Command Line Tools 和 macOS 14+ SDK，不增加 Cargo 依赖，不安装 LaunchDaemon/LaunchAgent。开发测试的 `--allow-loopback-ws` 只允许回环 WS，不允许远程明文接入。生产 WSS 由现有 Zone HTTPS 网关终止 TLS，再路由到 control-panel。

部署时需同步更新 control-panel、Desktop 与 SDK/CLI，并确保目标机信任 Zone 的 TLS 证书。当前网关源码支持这些路径的 Upgrade、认证头和 query 转发。本轮 E2E 使用独立 TLS 代理与新 control-panel fixture，未替换现有生产服务，未把生产 443 入口的完整 WSS 会话标为已验证。

历史 Linux 验收报告见 [Linux 验收说明](../../test/agent_daemon/README.md)，不作为当前 macOS 桌面输入或已发布二进制的验收证据。当前 checkout 的 `src/publish` 子模块没有旧文档提到的 `agent_daemon/package.py`、下载 launcher 或 `service.sh`，独立归档使用上述仓库内入口。

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

### macOS ComputerUse

桌面协议的 Rust 共享定义是 `src/kernel/buckyos-api/src/computer.rs`，SDK 类型和 `ExecutorCommandStream.computer(locator, request)` 位于同级 `buckyos-websdk/src/{computer,executor_client}.ts`。现有 Command Stream 增加 `type: "computer"`，请求包含 `request_id/store_id/op_id/run_id/call_id/request`。`request.command` 支持 `inspect/observe/click/type/keypress/scroll/wait_for`；不接受未知字段、无效目标联合或越界值。inspect/observe 也作为只读账本操作分配 ID，重传取得原结果，重新观察须使用新 ID。

```json
{
  "type": "computer",
  "request_id": "request-1",
  "store_id": "原登录返回的store_id",
  "op_id": "独立operation-id",
  "run_id": "当前run-id",
  "call_id": "独立子调用-id",
  "request": {
    "command": "observe",
    "binding_id": "default:display:1",
    "mode": "visual"
  }
}
```

binding 必须显式为 `<当前已授权profile_id>:display:<display_id>`，显示器 ID 从目标 daemon 的 doctor/在线能力获取，示例中的 `1` 不能直接照抄。本期只允许当次明确绑定的主显示器；主屏身份、布局或用户会话变化后拒绝旧目标，不暗中切换显示器。绑定从已认证 App session、profile、executor、store 和当前用户桌面装配；action JSON 不能自报授权主体或批准。尚未增加独立的控制面 binding CRUD 或异步批准队列。

macOS 的 `interactive_session_id` 使用 `getaudit_addr` 返回的 audit session ID（`ai_asid`），不使用标识硬件集合的 console-set ID。Driver 同时要求 audit user 与当前用户一致，且会话具有图形、console 和已认证标志，再核对 CoreGraphics 的当前用户、登录与前台 console 状态；注销后重新登录的会话不能沿用旧观察。

权限分别为 `computer_inspect`、`computer_observe`、`computer_act`。动作要求后置截图时还需要 observe 权限；只读 Agent 的 permission_mode 不允许 act。shell 仍具有当前 OS 用户权限，桌面允许列表不构成任意 shell 的强隔离边界。截图通过原 operation 的 Content channel 分块查询，沿用 actor/App/profile/runtime/store 的访问校验；artifact_ref 不是任意文件路径或公共下载 URL。

结果在 `op.result.computer`，外层 `op.state` 保持 Executor 合同；内层 `operation_state`、`execution.state` 和 `feedback.state` 分别说明操作、输入与截图事实。比如输入成功但截图失败：`execution.state=completed`、`feedback.state=failed`，不能重发输入。查询、等待和取消复用 `query(wait_ms)` / `cancel`；取消请求已接受不能等同输入已停止。新 generation 可在核验同 executor/profile/store 后查询旧操作，未知结果不自动重放。

visual/auto 返回 PNG，semantic/hybrid、element target、move/drag、text/element_state 等待和 `delivery=keys` 明确不支持。type 在现有光标/选区输入 Unicode 文本，不清空、不追加 Enter、不使用剪贴板；keypress 区分 Control 与 Meta。截图尺寸和逻辑显示坐标分别保存，point 必须是截图内整数坐标。focus 输入要求能重新核验焦点元素及其所属显示器，无法核验即拒绝。

每个桌面使用跨 daemon 进程锁和共享交互 revision；观察与输入串行。每个输入步骤校验取消、执行截止、桌面代次、锁屏、焦点和系统权限，按键/鼠标 down/up 在同一原生调用中配对。管理重连、授权变化、暂停/恢复使旧观察失效。已知外部输入和显示布局变化也会拒绝旧观察；这些检查不能消除 GUI 自身的检查—执行竞争，应用业务成功仍需新观察验证。

本地 `pause/resume` 使用经过 PID/启动时间校验的控制信号。返回成功仅说明信号已送达；`status --json` 的 `instances[].capabilities.ui.state` 在线时通常会在下一次心跳（约 5 秒）反映暂停。Computer inspect 返回实际 Driver readiness。暂停不会因重连自动恢复，resume 不重放原动作。

### Computer 数据与预算

继续使用 Node Executor Store schema_version=1，不新增数据库或第二份去重日志。`request.json` 的 spec 新增 Computer 联合，`result.json` 新增 ComputerResult，`content` 保存不可变 PNG；操作摘要覆盖规范化的完整请求。未知执行重启后保留 unknown，回收结果保留 tombstone，不能让原 ID 重新执行。当前 0.7 开发阶段不做旧二进制兼容；升级必须同时更新协议消费方，旧版本不能读取新的 Computer 记录。

账本使用本机私有目录是独立离线执行端恢复事实的要求，不依赖 Zone 在线数据库。输入文本随规范化请求持久保存于 0700 目录，不能把长期密码输入当作安全凭据通道。保留和清理由现有 ExecutorLimits 控制；图像最大 16 MiB，并受 ExecutorConfig.limits 的 max_file_bytes/max_store_bytes 进一步限制；截屏最多 4000 万像素，输入文本最多 16 KiB，执行预算最多 30 秒。观察引用仅驻留内存，最多 64 条、TTL 30 秒，进程重启后不能再执行旧观察。PNG 和执行事实可在账本保留期间查询。

`/tmp/buckyos-computer-<uid>.lock` 是所有 daemon 共用的输入资源锁，不是第二份操作账本；权限为 0600，拒绝软链接/异主文件。固定 schema_version=1 的 12 字节内容保存版本、64 位 revision、dirty 标志和待释放的单个键/鼠标按钮。输入前持久标记 dirty，确认释放后清理；崩溃遗留 dirty 时禁止下一写者输入。本地 resume 只有在取得独占锁、确认桌面/权限和没有用户正在按住键鼠后才尝试补释放；仍忙或清理失败则保持暂停，不能直接删除锁文件来绕过。此开发期协调格式不提供旧格式兼容。

### 本机验证入口

在 `src/` 执行 `cargo test -p buckyos_agent_daemon -p node_executor -- --test-threads=1`，覆盖生命周期、Fake Driver 账本故障与纯逻辑桌面测试；部分既有 Node 用例按 root/平台条件跳过，应核对输出。在仓库根执行 `uv run python -m unittest discover -s src/tools/tests -p 'test_package_agent_daemon.py'` 验证归档。

已登录且授予屏幕录制权限的 Mac 可执行只读原生截图 smoke；它验证 PNG 尺寸后丢弃像素，不发送点击/按键。缺权限或会话返回 77，不自动弹出申请：

```sh
xcrun --sdk macosx clang -fobjc-arc -fblocks -O2 -Wall -Werror -mmacosx-version-min=14.0 \
  test/agent_daemon/capture_macos.m -o /tmp/buckyos-capture-smoke \
  -framework Foundation -framework AppKit -framework ApplicationServices \
  -framework CoreGraphics -framework ImageIO -framework ScreenCaptureKit -framework Carbon
/tmp/buckyos-capture-smoke
```

### 未完成的上层链路

本次不添加 LLM tool，不修改 AgentRuntime trait 或 AgentSession binding。`buckyos computer` CLI、派发前 locator/宿主 Run manifest 桥接、`--present` 通用多模态链路和 `computer_use` cmd_skill 尚未交付；SDK 调用成功或取得 PNG 字节不代表模型已看到图像。真实桌面搜索任务、授权撤销/锁屏实机场景、Intel Mac、Developer ID 签名/公证和 Windows 都需要各自验收，不能用 Fake Driver 测试代替。

URL 有效期约束整个在线生命周期。daemon 根据服务端时间建立单调截止时间；Linux 使用包含休眠时间的 `CLOCK_BOOTTIME`，macOS 使用包含休眠时间的 `CLOCK_MONOTONIC_RAW`，重连和本机时钟回拨不能延长第一次确认的截止时间。所有数据流读取还检查管理租约门禁。

管理连接失效时暂停新操作、关闭数据流并重新接入；不重放已接收的命令。到期、撤销、SIGINT、SIGTERM 或显式 stop 停止接单，取消受管进程组并退出。普通数据流断开不自动取消已接受操作，后续通过相同账本查询。无法确定是否完成时保持 Unknown，不把离线当成功或已取消。

Linux capability 把 command/file 的支持与就绪分开；ui/accessibility/capture/input 永远为 `unsupported`。macOS 的 doctor 和 Computer inspect 动态探测用户会话、锁屏、屏幕录制和输入权限；不自动申请权限，不触发授权弹窗。在线能力变化在心跳中更新。`ui.*` 不是 Computer 协议入口。

## 本地数据与更新

独立包默认前台运行。URL 文件需为当前用户所有且权限 0600；凭据文件由用户显式提供，不自动保存 URL。升级先校验新产物，再停止旧进程并更换二进制。macOS 必须从已登录用户会话启动，不能用 sudo 或系统 LaunchDaemon 替代交互用户。

每个 executor_id 使用独立 0700 目录。`ledger/` 复用 Node Executor Store 的单写锁、store_id、操作记录与保留策略；`status.json` 为 0600，只含执行端标识、PID/启动时间、uid、状态和能力。stop 校验 PID 对应的进程启动时间，避免误停复用该 PID 的其他进程。URL、恢复秘密和 App token 默认不落盘。

更新前先 stop，确认进程退出后切换到新的校验通过的产物，再以同一 URL/数据根启动。保留 ledger 才能查询旧结果；删除 ledger 是显式清理，不等同于 revoke、退出或删除控制面记录。回退旧产物也不能自动重新执行未知动作。当前默认交付前台可移除包，不修改完整 Node 的 systemd、DEB/RPM 生命周期；独立可选常驻服务须使用其专用安装入口。移除产物不删除用户工作文件。
