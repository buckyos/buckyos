# 基于 Node Executor 实现 buckyos_agent_daemon：跨平台实施 TODO

日期：2026-10-22

状态：待实现。本文是实施计划，不表示下列接口、安装包或桌面能力已经存在。初稿代码基线为 `401d7b071`，本次在 `5ee602826` 核对接入相关源码，并按已确认的“URL 启动、进程在线、到期下线”流程更新；本次只修改本文档。

相关设计：[Node Executor 设计](../doc/arch/node_exector.md)。其中形态 A 为 node_daemon 内嵌，形态 B 为独立 Agent Daemon，形态 C 为移动端等宿主中的嵌入式运行。本文落实形态 B、桌面操作后端及 Android 受限形态 C；旧文档的规划不能作为已实现依据。

## 1. 目标与交付边界

交付 `buckyos_agent_daemon`：让已有 BuckyOS Agent 在获准设备上执行命令、访问文件、观察和操作应用界面。目标机可以不安装完整 BuckyOS，也不安装 BuckyOSApp；独立 daemon 不承载模型推理、OpenDAN Session、容器调度或 Zone 内核服务。

已确定的要求：

1. **复用 Node Executor。** 复用 Command Stream、登录鉴权、profile、操作账本、查询、取消与文件传输，不建立第二套远程执行协议。
2. **不依赖 BuckyOSApp 或目标机网关。** WSS 客户端、平台后端、用户会话 helper 与权限引导随 daemon 交付；目标机不要求 Device DID、DeviceDocument、cyfs-gateway 或完整节点身份。已有完整节点也能获得同一套桌面后端。
3. **辅助功能优先。** macOS AX、Windows UI Automation、Linux AT-SPI、Android Accessibility 提供结构化观察和语义操作；截图、坐标输入按需补充。普通表单、菜单、文本操作不要求每一步截图。
4. **URL 启动即接入。** 用户在 BuckyOS 添加设备并生成可设有效期的接入 URL，用启动脚本加 URL 参数运行 daemon；目标机不再做一次账号登录或设备配对。脚本负责取得对应平台产物并以前台进程运行，不要求 Python、Homebrew、xdotool 或开发工具。平台必需的首次授权仍需用户完成。
5. **能力按实际环境报告。** 没有桌面会话、控件不支持辅助功能、授权缺失、锁屏、后台受限等情况要明确返回；不能把命令执行可用等同于桌面可用。
6. **保持 Agent 当前运行位置。** Agent 通过设备工具访问目标机；完整 OpenDAN Session 迁移到 remote_node 不作为交付前置条件。
7. **可见性跟随进程与授权连接。** daemon 就绪并保持有效连接时，获准 Agent 能发现该 node_executor；退出后从可用执行端列表消失。管理页保留设备配置和离线状态。默认不安装自启动项，不在用户退出后自动拉起进程。
8. **URL 到期使在线实例下线。** 有效期约束整个接入生命周期；到期停止接收新命令并关闭接入，心跳、重连或会话续租不能延长它。用户已明确选择此语义。

本次不实现 Thunk 调度、Owner 恢复脚本、跨 Zone 控制、通用多租户沙箱、自动解锁或权限绕过。与这些方向的复用保持在既有类型边界内。

### 1.1 用户操作流程

1. 在 BuckyOS 的“添加设备”中设置名称、获准 Agent/profile 与可选到期时间，生成接入 URL 和当前平台的启动命令；此时设备记录存在，执行端尚不可用。
2. 在目标机运行启动脚本，将 URL 作为参数传入。daemon 以当前 OS 用户运行，主动连接 Zone 的 WSS 入口，取得配置并完成核心初始化后上线；无需开放目标机入站端口。
3. Agent 发现并使用该 node_executor。用户退出 daemon、URL 到期或管理员撤销 URL 后，执行端下线；异常退出或失联按有界心跳检测窗口移除。

拟议命令仅示意参数形态，脚本名称和发布路径实施时确定：

```bash
./start-buckyos-agent-daemon.sh '<BuckyOS生成的接入URL>'
```

Windows 提供等价 PowerShell 脚本。Android 用独立轻量 APK 接收同一接入 URL 并显式启停，不要求通过 ADB 执行桌面脚本。

## 2. 平台范围：以实际构建入口为准

“全部支持平台”分为节点平台和移动客户端，分别验收，不能用相同能力承诺覆盖所有系统。

| 平台 | 当前仓库依据 | 本 TODO 的交付目标 |
| --- | --- | --- |
| Linux x86_64 | [主 CI](../.github/workflows/build-all.yml)，构建目标为 musl | 独立 daemon、命令/文件、X11 与 Wayland 桌面适配、无桌面运行 |
| Linux aarch64 | 同上，构建目标为 GNU | 与 x86_64 相同的协议和功能验收 |
| macOS x86_64 / aarch64 | 主 CI 两种架构 | 独立 daemon、AX、按需截图/输入、随包安装的用户会话 helper |
| Windows x86_64 | 主 CI 当前启用的 Windows 架构 | 独立 daemon、PowerShell/文件、UIA、按需截图/输入 |
| Android arm64 | [BuckyOSApp Android CI](../../BuckyOSApp/.github/workflows/build-android.yml) 已有 APK 构建、签名和上传入口；不是 Node Executor 的现有支持 | 单独的轻量 Agent Daemon APK；复用核心协议，开放 Android 实际允许的应用/文件能力 |
| Windows ARM64 | [Windows 子 workflow](../.github/workflows/build-windows.yml) 接受该 target，主 CI 未启用 | 扩展目标：补 CI 和原生机器验收后才能标为支持 |
| iOS | [现有 Apple 工程](../../BuckyOSApp/src-tauri/gen/apple/project.yml) 有 iOS 配置，未见对应发布 CI | 单列调研，不计入当前支持平台完成条件；不能承诺普通 iOS App 获得 macOS 式跨应用桌面控制 |

说明：以上是构建配置事实，不代表本次验证了各平台远程构建结果。Linux X11/Wayland 是运行环境维度，不是增加 CPU target。Android 的独立 APK 不依赖 BuckyOSApp，但移动系统仍要求安装应用包，不能按桌面后台服务的方式直接部署裸可执行文件。

- [ ] 冻结各交付包最低 OS/API 版本与桌面环境版本，并写入能力说明；以实际 SDK/API 与原生验收结果决定，不凭 CI runner 名称推断最低版本。
- [ ] 将上述平台矩阵落实到专用构建任务与 DV 清单；桌面版本先交付，Android 单独里程碑验收，不用桌面通过代替移动端完成。

## 3. 当前已有基础与真实缺口

| 源码入口 | 当前事实 | 本次缺口 |
| --- | --- | --- |
| [node_executor/lib.rs](../src/kernel/node_executor/src/lib.rs) | `serve<S>` 接受 `AsyncRead + AsyncWrite`，不要求 TCP listener；已有配置应用与能力查询 | 主动 WSS 接入宿主、字节流适配、可选执行后端与桌面能力 |
| [node_executor.rs](../src/kernel/buckyos-api/src/node_executor.rs)、[协议](../src/kernel/buckyos-api/src/node_executor_proto.rs) | 已有 exec/read/write/edit/query/cancel；启用的配置强制要求非空 Device DID 准入列表 | 将 RTCP 准入与执行授权拆开；增加通用执行端目标、UI 操作和动态状态 |
| [spawn.rs](../src/kernel/node_executor/src/spawn.rs)、[worker.rs](../src/kernel/node_executor/src/worker.rs) | Unix 当前要求 root 切换到 profile uid/gid；Windows 以宿主身份执行 | 普通用户 Unix daemon 不能直接复用现有降权分支；Android 也不能套用通用 Unix worker |
| [node_executor_host.rs](../src/kernel/node_daemon/src/node_executor_host.rs) | node_daemon 装配 verifier/RBAC、读取 Node 配置、维护网关 ACL；本地监听 3184 | 独立宿主从 URL 接入服务取得可信配置，不依赖全局 node_daemon runtime |
| [scheduler/executor_auth.rs](../src/kernel/scheduler/src/executor_auth.rs) | 派生 `nodes/<node>/config.executor` | 复用执行授权推导，为轻量 executor 产生配置；不把它登记为调度 Node |
| [device_identity.rs](../src/kernel/buckyos-api/src/device_identity.rs) | 已有完整 Device 身份与密钥落盘能力 | WSS 轻量接入不使用此注册流程；以服务端分配的稳定 executor_id 定位目标 |
| [cyfs-gateway-lib](../../cyfs-gateway/src/components/cyfs-gateway-lib/src/lib.rs)、[网关装配入口](../../cyfs-gateway/src/components/cyfs-gateway-app-lib/src/composition.rs) | 现有 Node 使用设备信道与 RTCP 转发 | 完整 Node 保留该路径；独立 daemon 不内嵌网关，Zone 侧新增 WSS 接入与路由 |
| [agent_tool/runtime/node.rs](../src/frame/agent_tool/src/runtime/node.rs) | 已有远程命令、文件读取、DID/SOCKS connector 和恢复 locator | 增加在线 executor 发现与 WSS 路由 connector；目标不再只能是 Node DID |
| [lib_opendan/runtime/mod.rs](../src/frame/lib_opendan/src/runtime/mod.rs) | 完整 Agent Session 当前拒绝 remote_node | 本任务不以迁移 Session 解决桌面调用 |
| [agent_tool/xllm.rs](../src/frame/agent_tool/src/xllm.rs)、[context_loop.rs](../src/frame/llm_context/src/context_loop.rs) | 图片输入已有；普通工具结果到模型仍以文本为主 | AX/UIA/AT-SPI JSON 可直接复用；截图工具要另补多模态结果传递 |

现有默认 `max_file_bytes` 为 1 MiB、`max_concurrency` 为 2。高分辨率截图要缩放、压缩或配置额度；桌面输入需要额外的会话级串行队列。`workdir_roots` 约束文件 API 和 cwd，不是通用 shell 沙箱。

## 4. 组件形态与执行身份

建议新增薄宿主 crate `src/kernel/buckyos_agent_daemon/`，复用 `node_executor`，增加 WSS 客户端与接入生命周期；平台 UI 后端放在 `node_executor` 的按平台编译模块中。Zone 侧提供 URL 签发、连接管理、配置下发与 Agent 路由。Android 通过轻量原生壳接入协议/授权/账本核心，平台执行能力由宿主注入。目录和接入服务是实施建议，尚未创建。

```mermaid
flowchart LR
    A[既有 Agent 与设备工具] <--> Z[Zone WSS 接入服务与在线目录]
    D[目标机 daemon] <-->|daemon 主动建连| Z
    D <--> E[Node Executor 核心]
    E --> W[命令与文件 worker]
    E --> U[用户会话 UI 后端]
    U --> S[AX / UIA / AT-SPI / Android Accessibility]
    U --> V[按需截图与输入]
```

### 4.1 两种宿主，共用一套核心

| 部署 | 默认身份与生命周期 | 桌面后端 |
| --- | --- | --- |
| 已有完整 BuckyOS Node | 保持当前 node_daemon 的系统服务/用户进程模型 | 节点安装器自带 helper，在目标 OS 用户的桌面会话中启动 |
| 独立桌面 daemon | 脚本以前台当前用户进程启动；退出即结束接入，登录自启动是后续显式可选项 | 同用户上下文，helper 随主进程启动和退出 |
| Linux 无桌面设备 | 同样支持前台启动；常驻 user/system service 是显式可选模式，仍受 URL 有效期约束 | 报告无桌面，只提供获准的命令与文件能力 |
| Android | 应用沙箱身份，用户在 APK 中以 URL 启动/停止；系统终止服务则下线 | AccessibilityService / 截图会话，经 JNI 或窄接口接核心 |

可复用同一二进制的不同启动模式，但不能把后台系统服务与交互桌面的上下文混为一谈。helper 是随包内部组件，不要求 BuckyOSApp、托盘或常驻可见窗口。权限引导可以由 daemon 自带的最小界面或系统设置入口完成。

- [ ] 明确执行身份模型：`current_user` 与 `switch_user`，由实际宿主能力决定，不能由远程请求任意切换。
- [ ] Unix 当前用户模式校验真实 uid/gid/组身份并以原身份执行，不调用仅 root 可用的 setgroups；系统模式保留现有显式降权与核验。
- [ ] Windows 当前用户模式绑定 SID 与 session；不能因为进程位于 Windows 就无条件公布全部 worker/UI 能力。
- [ ] 将平台能力从硬编码 euid/cfg 判断改为实际可执行后端报告；Android 未实现的 exec 和跨应用文件能力必须拒绝。
- [ ] UI 后端建立专用事件循环/线程或进程，不让 AX、COM、D-Bus、JNI 的阻塞调用卡住 Tokio 网络与撤权处理。
- [ ] Unix IPC 验证 peer credentials，Windows named pipe 验证 SID/session 并设置 ACL；helper 只接受本机受信宿主的受约束请求。
- [ ] helper 仅接收所需的用户/session/操作授权上下文，不获得接入 URL 凭据、Owner 私钥或任意远程 shell 凭据。

### 4.2 与现有 Node 共存

- [ ] 完整 Node 继续由 node_daemon 托管原有执行端；独立 WSS executor 通过独立记录、配置根和账本定位，不冒用 Node DID 或自动继承 Node 权限。
- [ ] 独立 daemon 默认不监听 3184、不覆盖 `etc/node_executor_acl.json`，不打开完整节点账本；已有 `Store` 单写锁继续生效。
- [ ] 完整 Node 的 RTCP 接入与轻量 WSS 接入共用执行核心，网络准入由各自宿主负责；不要求轻量目标机安装或发现网关。
- [ ] 同一 executor_id 默认只允许一个活动实例；同机不同执行端分别绑定当前 OS 用户和数据根，桌面输入仍受目标 session 的串行约束。
- [ ] 显式迁移到完整 Node 时另行定义目标标识、RBAC 和账本交接；不把两种入口自动合并或隐式清空 `store_id`。

## 5. URL 接入、在线目录与授权

以下字段、路径与接口均为拟议设计，尚未实现；实施时同步共享类型、control-panel、配置生成、SDK 和文档。管理页称为“设备”，底层是轻量 executor 记录，不要求它成为 BuckyOS Device 或 Node。

### 5.1 在 BuckyOS 添加设备并签发 URL

- [ ] 管理端创建稳定的 `executor_id`，保存名称、owner、获准 Agent/profile、启用状态与可选 `expires_at`；生成包含接入地址与随机凭据的 URL，并提供各平台启动命令。
- [ ] URL 凭据只授权该 executor 的接入、配置读取和状态上报，不能用于调用其他设备或替代 Agent 的执行授权；服务端保存可校验摘要与撤销状态，不依赖难猜的 executor_id 作为凭据。
- [ ] 不生成目标机 Device DID、DeviceDocument 或设备私钥，不增加目标机浏览器登录、pair/unpair 等前置步骤；BuckyOS 管理端创建 URL 已完成所需授权。
- [ ] URL 在有效期内可用于重启/断线重连，默认同一执行端只允许一个活动实例，第二个独立进程接入返回占用错误；重置 URL 使旧凭据及其活动连接失效。没有设置到期时间的 URL 仍可主动撤销。
- [ ] daemon 解析 URL 后，以认证头等专用字段发送凭据，日志、诊断、错误和遥测只输出脱敏地址；启动脚本不打印含凭据的完整命令，不把 URL 写入全局启动项或默认明文配置，提供可选的受限文件/stdin 输入方式。
- [ ] URL 删除/撤销使执行端下线；删除在线记录、删除管理记录、清理本地账本分别定义，不把“不可见”实现成每次退出都删除设备配置。

### 5.2 主动 WSS 与 Command Stream

- [ ] Zone 提供可达的 HTTPS/WSS 接入入口；目标机只发起出站连接。接入服务验证 URL 的归属、状态与绝对到期时间，daemon 验证 TLS 服务端，不需要目标机证书或入站端口。
- [ ] 脚本下载/校验匹配平台的产物后以前台方式运行 daemon，正确转发 Ctrl-C/终止信号并等待退出；不默认后台脱离、不自动注册登录启动或退出重启服务。
- [ ] 建立 WSS 管理连接，完成接入鉴权、配置下发、`apply_config`、执行核心初始化和 readiness 上报后才发布在线执行端；初始化失败只报告诊断，不公布可用路由。
- [ ] 首版每条 Command Stream 对应一条独立数据 WSS，按需由 daemon 主动建立并使用短期流票据关联到当前活动实例；管理连接负责配置、心跳和开流通知，避免先实现复杂多路复用。票据绑定 executor、连接 generation、请求方与有效期，数据连接不能独立维持在线状态。
- [ ] 数据 WSS 经 binary-message/字节流适配后交给 `NodeExecutor::serve`；daemon 是 WebSocket 客户端，同时是执行协议服务端。Agent 仍先发 `login`，不同 Agent/runtime 使用独立 Command Stream，不能共用一次 App 登录冒充其他调用方。
- [ ] 复用现有 JSON 控制帧、二进制数据、op_id、账本与 query/cancel；实现 WSS ping/close、消息边界适配、大小限制、背压和有界重连，接入服务不另造 HTTP shell API。

### 5.3 在线可见性与进程生命周期

| 情况 | Agent 可用执行端列表 | 管理记录与执行行为 |
| --- | --- | --- |
| 已创建 URL，尚未启动或初始化未完成 | 不可见 | 保留配置，显示未连接/初始化状态 |
| daemon 就绪、授权有效且连接租约有效 | 对获准 Agent 可见 | 可新建 Command Stream 并提交获准操作 |
| 用户正常退出 daemon | 收到关闭后立即移除 | 保留离线记录，停止接单并清理受管工作/helper |
| 崩溃、断网、休眠或半开连接 | 检测断开或心跳租约到期后移除 | 请求失败为不可用/结果待确认，不宣称任务已取消 |
| URL 到期或被撤销 | 移除，拒绝新操作和重连 | 关闭关联连接，触发本地停止接单与取消流程 |

- [ ] 持久保存管理记录，单独维护易失在线目录；在线目录只暴露授权给调用 Agent 的执行端。列举、路由和动作派发都检查有效期，不能仅靠 UI 隐藏。
- [ ] 定义心跳间隔与在线租约上限并提供可测量指标；首版建议 5 秒心跳、15 秒超时。正常关闭立即处理，非正常退出从最后一次有效心跳起至多经过该超时移除，不承诺网络故障下零延迟感知。
- [ ] 每次进程启动使用新的 `instance_id`，每次重连签发新的连接 generation；同进程凭服务端签发且仅存进程内的恢复凭据接管旧连接，不以自报 instance_id 作为接管依据，凭据不越过 URL 到期时间。旧连接的 heartbeat/close 不得恢复或删除新实例，旧请求与 UI 句柄不得漂移到新实例。
- [ ] 管理连接失效立即停止发布可用数据通道并暂停新动作派发；同一进程仅在 URL 仍有效时重连。重连后重新同步配置/readiness，查询原操作状态，不自动重放已接受的命令或 UI 动作。
- [ ] 正常退出先撤下在线路由、停止接单，尽力取消受管命令、释放输入状态并停止 helper，最后关闭连接和退出；脚本不能遗留仍可执行远程请求的子进程。
- [ ] 保留当前执行语义的限制：单条数据 Command Stream 断流不自动取消已接受的任务，执行端崩溃也不代表子进程和副作用消失；管理连接失效暂停新派发，到期/撤销另行触发取消。失联操作保持待查询，重启后仅将没有持久化最终结果的操作标为 `Unknown`，已完成操作保留原结果，不得凭离线状态伪造成功或取消。

### 5.4 URL 到期、授权配置与身份解耦

- [ ] 到期时间是接入授权的绝对截止时间；控制面不签发越过它的会话/流票据或授权快照。接入服务和 daemon 都在截止时停止新派发，移除在线状态并关闭连接；daemon 结束本轮接入，不无限重试已过期 URL。
- [ ] daemon 按服务端确认的期限保守建立本地单调时钟截止时间，重连校验原绝对到期时间；控制面不可达或本机时钟回拨均不能延长授权。休眠恢复后先重新验证授权再派发动作，不能依赖不计休眠的计时器续命。授权快照还受现有短租约上限限制，取两者更早的截止时间。
- [ ] 到期/撤销对已接受工作请求取消，停止后续 UI 步骤并尽力结束受管进程；已发生的副作用不能回滚，未确认结果保持未知。操作自己的 execution timeout 不能替代 URL 截止或延长接入。
- [ ] 定义通用 executor target，用稳定 `executor_id` 替代独立路径对 Node DID 的依赖；同步 `ExecutorHost`、ledger 归属、describe、客户端 connector/locator 和 RBAC resource。保留 `store_id`，不同机器/账本使用同一 URL 时不能混认旧结果。
- [ ] 拆开 `ExecutorConfig.allowed_devices` 与执行授权：完整 Node 保留 RTCP 设备 ACL；WSS 使用 URL/活动连接准入，不能为通过校验填造假 DID。共用 profile、Agent/App binding、permission_mode、ops、limits、revision/generation 与租约逻辑。
- [ ] 为轻量 executor 单独定义管理配置与派生快照的存储和 RBAC，复用现有授权推导。不要直接写入被 scheduler 当作 Node 消费的 `devices/*/info`，也不沿用此前拟议的 Device enrollment 前置条件。
- [ ] 独立宿主通过接入服务下发的可信 issuer/校验材料、RBAC 和配置实现 `TokenVerifier`/`RbacChecker`；保留 App session → Agent binding 校验，不用接入 URL 代替 Agent 登录，不依赖全局 BuckyOS runtime。
- [ ] capability 是能力声明，不是授权。最终允许范围取配置、Agent 权限、宿主身份、平台授权和实际后端支持的交集；缺 GUI 授权不应隐藏仍可用的命令/文件能力。

WSS 负责加密双向传输，客户端认证由应用定义；设计依据 [RFC 6455 §10.5–10.6](https://www.rfc-editor.org/rfc/rfc6455.html#section-10.5)。接入 URL、在线目录和上述生命周期是本项目拟议接口，不是 WebSocket 自带功能。

## 6. UI 协议与操作语义

### 6.1 最小工具面

UI 请求复用 Command Stream 的操作生命周期。下面是逻辑工具名，wire 字段、`ExecutorOp` 及 RBAC action 在协议实现阶段一起定义。不要长期使用 `exec "helper ..."` 作为唯一 UI 接口，否则无法独立授权只观察和可操作。

| 拟议工具 | 行为 | 授权分类 |
| --- | --- | --- |
| `ui.status` | 返回会话、后端、授权与不可用原因 | 查询 |
| `ui.inspect` | 枚举应用/窗口，读取有界控件树 | 观察 |
| `ui.find` | 按应用、role、名称、identifier 等查找 | 观察 |
| `ui.perform` | 执行控件支持的 press/invoke/select/expand/scroll 等动作 | 修改 |
| `ui.set` | 检查属性可写性后设置值或焦点 | 修改 |
| `ui.wait` | 有界等待元素出现或状态满足，通知与重读结合 | 观察 |
| `ui.capture` | 按需获取授权窗口/屏幕图像及坐标元数据 | 截图观察，单独授权 |
| `ui.input` | 控件语义操作不足时执行坐标/键盘/手势输入 | 修改，单独报告支持范围 |

- [ ] 扩展共享协议、操作枚举、`is_mutating`/只读判定、profile ops 与 RBAC；只读 Agent 不能通过 UI set/perform/input 绕过限制。
- [ ] 同步协议版本/握手、服务端、Rust/TypeScript 客户端、配置生成和测试；不增加旧版本兼容分支，版本不匹配在执行前明确拒绝。
- [ ] 为每个后端映射公共语义动作，只公布控件实际支持的动作/可写属性；必要的原生动作名称放显式扩展字段，不假设所有平台都有 AXPress。
- [ ] 观察结果至少包含应用标识、窗口、role、label/value、enabled、bounds、actions、settable 属性与截断标记；提供 depth/max_nodes/max_text_bytes、分页和超时。
- [ ] 返回由 helper 管理的 opaque `element_id`，绑定 executor、活动实例/连接 generation、OS 用户、session、应用进程及后端 generation；不能把数组下标、PID 或指针本身当稳定远程句柄。
- [ ] UI 变化、进程退出、helper 重启或用户切换后，失效引用返回明确错误；执行前核对目标仍匹配，并读取实际结果。
- [ ] `ui.wait` 使用平台事件通知加有限轮询，所有等待都有截止时间，可取消；不以无限轮询替代生命周期管理。

### 6.2 动态状态与执行恢复

- [ ] capability 分开表达“实现支持”和“当前就绪”；按 session 报告 accessibility、capture、input、后台操作、命令和文件能力。网络在线与 GUI 就绪分别表达，连接状态遵守 §5.3。
- [ ] 至少区分 `permission_required`、`no_gui_session`、`session_locked`、`session_changed`、`backend_unavailable`、`unsupported`、`stale_element`、`target_not_found`、`outcome_unknown`；名称为拟议值，实施时统一类型。
- [ ] 缺授权时只返回状态与本地启用步骤；由用户主动的本地设置流程请求权限，避免远端轮询不断触发系统弹窗。
- [ ] 输入按目标桌面/session 串行化，并在每次派发前重查身份、授权与 session；默认不在多用户桌面之间自动选择或漂移。
- [ ] 对注入的按键/鼠标按下状态建立本地跟踪；异常、取消或连接终止时尽力释放本次操作持有的状态，避免留下卡键，不干扰用户自行按下的输入。
- [ ] 复用 op_id/call_id 与账本去重，将 helper 派发与结果关联。发生断线/崩溃/超时后先 query 或重读状态，不能自动重复点击、提交或输入。
- [ ] 不能承诺 exactly-once。已派发操作可能已经生效；cancel 只停止后续可停止步骤，不声称能撤销已发生的 UI 副作用。
- [ ] 长任务中的锁屏、切换用户、权限撤销、接入 URL 到期/撤销和失租要打断后续操作；解锁/重新授权后重新观察，不重放旧坐标或旧句柄。

### 6.3 截图作为可选观察

- [ ] 定义 `capture_id`、目标 session/窗口/显示器、捕获时间、图像尺寸、逻辑坐标范围、缩放/旋转转换；坐标输入绑定相应观察，避免多屏/DPI变化后误点。
- [ ] 复用二进制结果/文件通道；图片不塞入 stdout 的大段 base64 文本。设置图片大小、保留期与清理规则。
- [ ] 处理黑屏、受保护内容、失效窗口、权限拒绝，不能把空白图像当成功观察。
- [ ] 语义观察足够时不截图；语义后端不足或任务要求视觉确认时才请求图像。UI 截图本身不等于已经完成输入权限授权。

## 7. 各平台实施 TODO

### 7.1 macOS：AX 为主，无 BuckyOSApp

- [ ] 实现 AX 应用/窗口发现、属性读取、动作枚举、`AXUIElementPerformAction`、可写属性设置和按应用注册的 AXObserver。
- [ ] 支持异步权限检查与最小授权引导；区分 Accessibility 与 Screen Recording。只使用 AX 的主路径不要求先完成截图权限。
- [ ] helper 使用稳定安装位置与签名身份；需要 bundle 元数据时随 daemon 打包最小后台 bundle，不引入完整 BuckyOSApp。
- [ ] 独立模式由当前用户脚本启动，helper 跟随 daemon 生命周期，不默认注册 LaunchAgent。完整节点将 helper 安装到 `/Library/LaunchAgents`，按目标已登录用户加载 `gui/<uid>`；独立版的登录自启动仅作为显式可选模式。
- [ ] 使用独立 launchd label，如 `com.buckyos.agent-daemon.ui`；现有安装脚本会删除旧 `buckyos.service` LaunchAgent，不能复用该 label。
- [ ] 系统进程到用户 helper 通过经过身份检查的本地 IPC；不把 setuid 或设置 HOME 视为进入 Aqua 会话。
- [ ] 截图使用目标 OS 支持的 ScreenCaptureKit 接口；输入补充使用 CGEvent。实现 Retina/多屏映射、中文/组合键与动作后状态核验。
- [ ] 为不支持 AX、AX 消息超时、元素消失提供明确结果；超时不意味着动作一定没发生。
- [ ] 真机验证首次授权、拒绝后恢复、撤权、升级后授权状态、注销/登录、用户切换与锁屏；不承诺普通安装可静默批准 TCC 或操作登录/解锁界面。

AX 支持读取控件属性、执行动作与状态通知；信任检查针对调用进程，提示授权是异步过程。上述实现依照 [Apple AX 接口](https://developer.apple.com/documentation/applicationservices/axuielement_h)、[授权检查](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions) 与 [Daemon/Agent 上下文说明](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/DesigningDaemons.html)。

### 7.2 Windows：UI Automation 为主

- [ ] 默认由 PowerShell 脚本作为交互用户进程启动，处理终止信号、等待子进程退出并按 executor 保证单实例；登录启动任务是显式可选模式。不要求 BuckyOSApp，也不默认提升为管理员。
- [ ] 保持现有 PowerShell/Job Object/文件路径校验能力，明确宿主 SID、完整性级别和 session。
- [ ] 实现 UIA 元素查询、属性读取及 Invoke/Value/Text/Selection/ExpandCollapse/Scroll 等实际支持的 patterns；COM 初始化与事件订阅在专用线程处理。
- [ ] 为 HWND/进程退出、控件 provider 不响应及 UIA 元素失效设置超时和重新发现机制。
- [ ] 输入补充使用 SendInput，截图首选 DXGI Desktop Duplication；处理显示模式变化、失效重建、旋转、DPI awareness、多屏负坐标与键盘布局。
- [ ] 对 UAC secure desktop、较高完整性应用、锁屏与会话断开明确报告限制；不默认使用 UIAccess 或提升权限绕过边界。
- [ ] 若交付机器级 Windows Service，必须增加用户会话 broker/helper 生命周期；Service 位于 Session 0，不能直接作为桌面操作后端。该模式单独验收，不能由普通用户版测试代替。
- [ ] 检测 console/RDP/多用户 session，绑定明确目标；RDP 断开或 session 变化后重新检查可用能力，不沿用旧窗口/输入句柄。
- [ ] 在 Windows x86_64 原生验收普通应用、浏览器、自绘区域、不同 DPI、权限限制、重启与升级；ARM64 在纳入矩阵后另验。

Windows 的服务/session 边界、控件操作及输入限制分别依据 [Interactive Services](https://learn.microsoft.com/en-us/windows/win32/services/interactive-services)、[交互用户登录任务](https://learn.microsoft.com/en-us/windows/win32/api/taskschd/ne-taskschd-task_logon_type)、[UIA Client](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-clientsoverview) 与 [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)。截图与桌面变化处理参照 [Desktop Duplication](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api)。

### 7.3 Linux：AT-SPI + X11 / Wayland 分支

- [ ] 实现 AT-SPI 控件树、Action、Text/EditableText、Selection、Value 等能力与事件监听；限制递归、文本量和远端应用响应等待。
- [ ] 在目标用户会话中发现 session bus，并通过 `org.a11y.Bus` 取得 accessibility bus；不能把 root 的 D-Bus 环境或 SSH 的 DISPLAY 当成目标桌面会话。
- [ ] 独立版默认脚本前台运行，GUI helper 跟随 daemon 启停；user service/XDG autostart 是显式可选模式。完整节点的 system service 通过本地 IPC 接用户 helper，明确启动项去重和退出行为。
- [ ] X11：经本机合法 display 授权使用系统截图与 XTest 等输入接口；不依赖 shell 调用 xdotool，不设置 `xhost +`。
- [ ] Wayland：使用 RemoteDesktop/ScreenCast portal，按提供者支持情况使用 PipeWire 捕获与 EIS/libei 输入；AT-SPI 语义操作与这些通道分别报告能力。
- [ ] 探测 portal 版本、可用 backend 与获准设备/屏幕；按 portal 规则处理用户选择、恢复 token、会话关闭和撤权，不承诺首次或每次重启都能无提示恢复。
- [ ] 不以 root、XWayland 或读取 `/dev/uinput` 作为 Wayland 缺权限时的自动回退；缺 backend 返回原因，不冒充可控。
- [ ] 无 GUI / 缺 AT-SPI / 缺 portal 时仍提供已授权的命令和文件能力；脚本产物交付核心必需依赖，可选 GUI 库由独立 helper 或动态加载隔离，不能在进程加载时阻止核心启动。明确系统必需库的最低版本；DEB/RPM 安装模式用包依赖补齐，脚本模式探测缺项并提供可选安装指引，doctor 如实报告降级。
- [ ] 明确 GNOME/KDE 的测试版本，分别验证 X11、Wayland；不能仅在 Xvfb 通过就标记 Linux 桌面完成。
- [ ] 同时验证 x86_64 musl 与 aarch64 GNU 的链接/运行：引入 AT-SPI、D-Bus、PipeWire、libei 后检查动态依赖和包依赖，避免破坏现有静态产物构建。

Linux 会话接入参照 [XDG Autostart](https://specifications.freedesktop.org/autostart/latest/) 与 [AT-SPI accessibility bus](https://github.com/GNOME/at-spi2-core/blob/main/bus/README.md)，语义接口参照 [AT-SPI API](https://gnome.pages.gitlab.gnome.org/at-spi2-core/libatspi/)。Wayland 授权、输入与图像通道依据 [RemoteDesktop portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)、[ScreenCast portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html) 与 [libei](https://libinput.pages.freedesktop.org/libei/api/index.html)；不能将 InputCapture 接口误用为远端输入注入。

### 7.4 Android：独立轻量 APK，受限执行后端

- [ ] 单独交付 `buckyos_agent_daemon` APK，支持输入/打开接入 URL、状态、启停和系统权限入口；不要求 BuckyOSApp、Device 身份注册、ADB、root 或开发者模式。
- [ ] 先把共享协议、授权、账本与桌面进程 worker 解耦；Android 注入宿主支持的操作，不通过添加 `cfg(unix)` 来假装已有完整 Unix 执行后端。
- [ ] 实现 AccessibilityService 节点读取、支持的 `performAction`、输入值/滚动与事件监听；需要手势时按系统能力使用 `dispatchGesture`。
- [ ] 明确用户启用/停用 AccessibilityService 的生命周期；服务重连、应用被杀、重启及系统限制均更新 readiness。用户停止接入后不因服务回调再次上线，系统终止进程则按心跳规则摘除执行端。
- [ ] 文件操作限定应用沙箱及用户通过系统授予的文件访问；URI 授权不得伪装为无限制 POSIX 路径。任意系统 shell 和访问其他应用私有文件不作为普通 APK 能力。
- [ ] 截图优先评估当前 API 版本允许的辅助功能截图；需 MediaProjection 时单独实现用户授权与相应前台服务。Android 14+ 的 MediaProjection 授权按 capture session 管理，不能缓存旧 token 当永久授权。
- [ ] 选择符合用途的服务类型，处理系统后台启动限制和可见状态；不能把 foreground service 等同于永不被停止。
- [ ] 验证安全窗口、密码字段、锁屏及跨应用差异；未公开或不可访问的内容返回限制，不通过隐藏接口补齐。
- [ ] arm64 真机完成文本控件操作与可选截图闭环；记录系统/API/OEM差异、重连与耗电表现，发布能力矩阵。

Android 适配依据 [AccessibilityService](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService)、[MediaProjection](https://developer.android.com/media/grow/media-projection) 与 [前台服务类型](https://developer.android.com/about/versions/14/changes/fgs-types-required)。它与桌面后台服务是不同交付形态，不能以相同名称掩盖能力差异。

## 8. Agent、SDK 与工具手册接入

- [ ] 在已有 Agent runtime 内增加在线 executor 的 list/watch 与显式目标工具；参数包含 executor_id、活动连接 generation、profile/OS session。完整 Node 的 DID 目标另行表达，不用把整个 Session 改为 remote_node。
- [ ] Agent 发现只返回获准且在线的执行端；下线事件更新工具目标缓存，调用时重新验证路由。失效目标返回不可用，不自动转发到其他设备或新实例。
- [ ] 封装 UI JSON 观察为普通工具结果，模型可通过 inspect/find → perform/set → wait/inspect 完成任务；不强制使用截图型 `agent.computer_use` API。
- [ ] 平台工具统一报告已支持/已授权/当前就绪及降级原因，Agent 根据结果选择语义操作或截图，不由模型猜测权限。
- [ ] 实现截图结果到 `AiToolResultContent::Image` 或等价结构化图片输入的完整链路；只返回远端路径或文本 base64 不算完成。
- [ ] 复用已有远端 operation locator、query 与恢复机制，扩展 executor 目标但保留 store_id/op_id 校验。重连到同一账本可查询旧结果，旧连接请求不能自动作为新动作派发；新增 UI 操作遵守未知结果不盲重放的规则。
- [ ] 补齐 buckyos-websdk 的轻量设备记录、URL 签发/撤销、在线目录与 Executor 客户端及人可用 CLI；当前不能假定 TypeScript SDK 已有 Node Executor 客户端。复用共享协议 schema，避免再造 HTTP 执行入口。
- [ ] 系统管理命令按仓库约定放 websdk CLI；本地接入 URL 启动、status/doctor/stop 保留在自包含 daemon 中，不要求目标机登录账号或运行 Node.js。
- [ ] 同步 [buckyos_admin](../src/apps/jarvis_agent/agent/cmd_skills/buckyos_admin.md)、[opendan_agent_dev](../src/apps/jarvis_agent/agent/cmd_skills/opendan_agent_dev.md)；按最终工具面增加 computer-use 手册，覆盖发现、授权失败、控件引用失效、截图回退与停止。

## 9. 安装、数据与发布

### 9.1 自包含启动与可选安装

- [ ] 增加独立产品/module 构建与安装映射，不直接复用完整 BuckyOS 安装入口的 Docker/容器检查；独立 daemon 不要求 Docker、WSL 或 OOD 服务。
- [ ] 交付 macOS/Linux 启动脚本和 Windows PowerShell 脚本，自动选择 OS/架构、验证下载产物并缓存到稳定用户目录，传入 URL 后以前台方式运行；支持预下载产物，不因脚本下载模式改变签名或 OS 授权主体。
- [ ] 同时支持完整节点分发同一 helper；两种分发的 core/协议/后端来自同一源码。
- [ ] 提供 `doctor`：脱敏 executor 标识、URL 有效期、WSS/在线登记状态、配置是否有效、执行身份、helper/session、平台权限与缺失后端；输出人可读及 JSON 格式。
- [ ] 启动不带 URL 时给出使用说明，不启动匿名执行入口；带 URL 时自动完成鉴权、配置与在线登记。平台权限不足按能力报告，不额外增加配对流程。
- [ ] 默认退出即下线，不安装启动项、不自动重启。后续显式可选常驻模式要说明凭据保存、stop 和 URL 到期行为，不能让过期凭据被服务管理器无限重试。
- [ ] 缺少 OS 授权时仍能启动和诊断，提供本地启用路径；普通用户不需要手写 plist、systemd unit 或计划任务。

### 9.2 数据与更新

- [ ] 独立用户模式使用平台标准应用数据/配置/日志目录；完整节点沿用 `$BUCKYOS_ROOT`。通过显式 roots 注入，不能让独立实例误读 `/opt/buckyos` 身份。
- [ ] 区分接入凭据、授权快照、操作账本、临时截图与用户文件；设置目录权限和保留期。独立版不生成 Device 私钥，默认不持久保存 URL 秘密；按稳定 executor_id 隔离账本，升级与同目标重启保持 `store_id`。
- [ ] 更新顺序为停止接单、处理/标记在途操作、停止 helper、替换签名产物、重启、重新探测能力；失败可回滚，不能将未知动作当作未执行重发。
- [ ] 卸载删除本产品已显式安装的启动项和 helper，停止相应会话进程；保留用户业务文件。退出、撤销 URL、删除设备记录、卸载和清除本地账本分别定义，不要求退出时执行 unpair。
- [ ] 不手改系统权限数据库；卸载/重装后的 OS 授权状态由系统决定，并由 doctor 如实反映。

### 9.3 仓库联动落点

| 范围 | 需要检查/修改的现有入口 |
| --- | --- |
| Rust/构建 | [src/Cargo.toml](../src/Cargo.toml)、[bucky_project.yaml](../src/bucky_project.yaml)、[buckyos-build.py](../src/buckyos-build.py) 与各平台 workflow |
| 控制面与 Agent | control-panel 的轻量设备/URL 管理、Zone WSS 接入/在线目录、共享授权类型、agent_tool connector/locator、websdk 与 CLI；新增接口不能只改目标机 |
| 轻量启动 | 新增三大 OS 脚本与产物分发入口，核验下载、URL 参数、退出信号、凭据脱敏和无启动项残留 |
| macOS | [postinstall](../src/publish/macos_pkg/scripts/buckyos_postinstall)、[preinstall](../src/publish/macos_pkg/scripts/buckyos_preinstall)、[打包器](../src/publish/make_local_osx_pkg.py)；新增独立产品 hooks，补 LaunchAgent 全生命周期 |
| Windows | [postinstall](../src/publish/win_pkg/scripts/buckyos_postinstall.ps1)、[preuninstall](../src/publish/win_pkg/scripts/buckyos_preuninstall.ps1)、[NSIS 模板](../src/publish/win_pkg/installer.nsi.in)；复用模式而非要求安装完整节点 |
| Linux DEB | [postinstall](../src/publish/deb_pkg/buckyos_postinstall)、[make_local_deb.py](../src/publish/make_local_deb.py)；当前生成入口缺少完整 prerm/postrm，要补 remove/purge 与 user helper 清理 |
| Linux RPM | [postinstall](../src/publish/rpm_pkg/buckyos_postinstall)、[make_local_rpm.py](../src/publish/make_local_rpm.py) 的升级/卸载 scriptlets |
| 停止/卸载 | [stop.py](../src/stop.py)、[uninstall.py](../src/uninstall.py)，保证开发安装与正式安装不会遗留 helper |
| Android | 参考既有 APK CI 组织独立包、签名与 Rust native library 交付；不把 BuckyOSApp 二进制作为运行依赖 |

- [ ] 新增依赖先确认现有 workspace 能否复用；需要新 crate/系统组件时按根目录 AGENTS.md 确认选型。Git 源依赖显式使用 `main`，不自行锁定 rev/tag。
- [ ] 为发布包记录 OS/架构、最低版本、支持后端、签名、安装文件清单与运行时依赖。每个声明支持的平台必须拿真实安装包验收。

## 10. 实施顺序与完成门槛

| 阶段 | 任务包 | 完成门槛 |
| --- | --- | --- |
| P0 | URL/到期契约、WSS/在线目录、executor 标识、授权解耦、普通用户模式、共存策略与 UI 协议 | 固定三步体验及状态机；Node 原有命令/文件测试保持通过；无需 Device DID 的配置与路由契约可实施 |
| P1 | 控制面签发 URL、WSS 接入服务、薄宿主、三大 OS 脚本、Agent 最小在线发现与 connector/locator、status/doctor 和命令/文件 | 干净机器脚本+URL上线，Agent 发现并 exec/read/edit，退出不可见；在线到期下线、撤销、失联与重连均可验证，无目标机 CYFS/BuckyOSApp |
| P2 | 公共 UI 生命周期与 macOS AX、Windows UIA、Linux AT-SPI | 各平台在不截图的条件下完成控件发现、填值、触发动作、回读验证；支持取消、失效引用与会话变化 |
| P3 | 截图、多模态工具结果、输入回退；Linux X11/Wayland 分支 | 自绘区域或视觉验证场景闭环；多屏/DPI、portal/TCC/UIPI 限制与拒绝路径通过 |
| P4 | 三大 OS 正式打包、完整节点内置 helper、可选持久安装、升级/卸载、完整 SDK/CLI 分发与手册 | 五个当前节点构建目标通过脚本和安装包验收；默认退出不重启，可选启动项与完整 Node 共存可验证 |
| P5 | Android 受限后端与独立 APK | arm64 真机 URL 接入、在线/退出/到期、语义操作、按需截图和权限边界通过；不能宣布与桌面任意 shell 等价 |
| 扩展 | Windows ARM64、iOS 能力调研 | ARM64 补 CI/实机后纳入支持；iOS 先明确公开 API 能力与发布形态，再单独决策 |

P2 的平台适配可在 P0 共享接口稳定后并行；P3 的平台权限验证应尽早做最小实验，避免最后才发现打包身份或 compositor 限制。P4 不能仅补脚本而跳过实际升级/卸载。

### 10.1 公共回归与故障场景

- [ ] 原有 Node Executor 的命令、文件、取消、限额、结果查询、断流恢复、租期过期与权限撤销测试保持通过。
- [ ] 没有 Device 身份文件、cyfs-gateway、BuckyOSApp 和完整 Node 的干净目标机，脚本加 URL 可完成连接、配置、Agent 发现和真实命令/文件操作，无额外登录/配对。
- [ ] 创建 URL 尚未启动时不可见；正常启动就绪后可见，Ctrl-C/正常停止后立即摘除。强杀、断网、休眠和半开连接在约定心跳窗口内摘除，默认无自启动或重启进程残留。
- [ ] 过期/撤销/错误 URL 不能登记；已在线实例到期停止接单并下线，数据连接不能延续可用性。测试长命令、UI 操作中途到期、本机时钟回拨、断网跨过截止时间及休眠跨截止后恢复，重新校验前不能派发排队动作，过期重连不得刷新截止时间。
- [ ] 同 URL 第二个独立实例被拒绝；同进程重连的新 generation 不被旧 heartbeat/close 覆盖。旧路由、流票据和 UI 句柄不能操作新连接，重装到另一账本后旧 store_id 查询明确拒绝。
- [ ] 在线列表按 Agent 授权过滤；离线/已过期 executor 即使被调用方缓存也拒绝派发；GUI 未授权时仍可发现并使用已就绪的命令/文件能力。
- [ ] 普通用户模式不能选择其他 OS 用户；系统模式依旧按 profile 降权；Android 不支持的操作在执行前拒绝。
- [ ] 未授权 Agent、只读模式、错 profile、错 OS session、过期授权均不能派发修改动作；完整 Node 验证设备 ACL 撤销，轻量模式验证 URL 撤销和活动连接失效。
- [ ] 验证动作派发前、派发后未回执、结果落盘前、helper 重启等故障点；未知结果可查询且不自动重复执行。
- [ ] 对 UI 树失控增长、恶意/异常控件 provider、平台调用超时验证限额、可取消性和服务可恢复性。
- [ ] 会话锁定、注销、快速用户切换、RDP 断开、桌面重启、权限撤销之后重新发现目标，不复用旧句柄。
- [ ] 重装、升级回滚、URL 重置、卸载及与完整节点共存，不残留重复启动项、端口竞争或目标/账本串用；日志与脚本输出不泄露接入凭据。

### 10.2 真机验收表

每项记录 OS/版本/架构、桌面环境、产物版本、授权状态、命令、观察结果与失败原因。所有桌面目标先验收“脚本+URL → Agent 发现 → 操作 → 退出不可见”，再测到期/异常失联；Android 用 APK 输入 URL 完成等价流程。编译通过和 mock 通过不能替代此表。

| 环境 | 最小场景 |
| --- | --- |
| macOS Intel / Apple Silicon | 没有 BuckyOSApp 的首次安装；AX 文本编辑/按钮操作；拒绝与撤回授权；Retina 截图；升级后恢复 |
| Windows x86_64 | PowerShell 脚本前台启动/退出；普通权限文本编辑器/浏览器；UIA 与自绘回退；高权限窗口受限；DPI；可选计划任务与注销重登 |
| Linux x86_64 / aarch64 无桌面 | 命令/文件完整可用，UI 返回无会话；重启、断网失租与取消 |
| Linux GNOME/KDE X11 | AT-SPI 文本控件；X11 截图/输入；会话退出与 display 授权失败 |
| Linux GNOME/KDE Wayland | AT-SPI；portal 用户选择；捕获与输入；拒绝/撤权/会话恢复；后端缺失 |
| Android arm64 | 独立 APK 接收 URL 并显式启停；用户启用服务；跨应用语义操作；截图授权；旋转、锁屏、被杀下线与主动恢复 |

Linux 桌面测试要覆盖两种 CPU 架构；具体 GNOME/KDE 版本与组合在 P0 定稿，未测试组合标注未验证。

建议的既有代码回归入口（后续实现时执行，不代表本次已运行）：

```bash
# 在 buckyos/src 下执行
cargo test -p node_executor -- --test-threads=1
cargo test -p buckyos-api node_executor -- --test-threads=1
cargo test -p scheduler executor_auth -- --test-threads=1
cargo check -p node_daemon
```

新增 daemon/package、UI 协议与后端后增加对应原生测试和安装包 DV。修改 Agent 工具链时运行 agent_tool 相关测试；修改 cmd_skills 装配/分发时按根 AGENTS.md 执行对应 libopendan/opendan 回归。非 root 下跳过旧 Unix 测试不能视为普通用户执行模式已经验证。

## 11. 本文交付记录

本次只更新 TODO，将独立模式确定为“BuckyOS 添加设备并签发 URL → 脚本+URL 前台运行 → 在线可见、退出不可见”，并按用户确认采用 URL 到期使在线实例下线。同步调整了身份、网络、平台启动、SDK、发布与验收要求；未修改运行时代码、协议、依赖、安装器或已有命令手册，因此不需要在本次同步修改 cmd_skills，后续联动项已列在 §8。本文核对相对链接、接入相关源码与 diff 格式；未进行跨平台编译、安装、GUI 真机或模型/provider 验证。
