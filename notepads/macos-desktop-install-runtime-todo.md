# macOS BuckyOS Desktop 安装与运行逻辑调整 TODO

日期：2026-10-25。

状态：**代码已实施（2026-10-25，见 §7），平台验收未完成。** LaunchAgent 真实注册/登录、正式签名与 TCC、pkg 安装和 GUI DV 尚未执行，下列未勾选项即剩余工作。

源码基线：`buckyos` `244b6d7d7`（包含 `0a335026b` 的 host 工具布局/路径注入），`BuckyOSApp` `449ed80`。后续实施以当时源码为准复查。

前置材料：[macOS 用户会话启动 GAP 预研](macos-desktop-user-session-research.md)、[Node Executor 设计](../doc/arch/node_exector.md)、[Runtime / Session 集成裁决 R1/R2](agent-runtime-session-workspace-integration-todo.md)、[session-runtime 与标准工具基线](opendan-primary-runtime-tools-todo.md)。预研中有关“工具布局仍未提交”的描述是当时快照，当前进度以本文代码基线为准。

**1. 已确定的方案与目标**

- macOS Desktop 的 `node_daemon` 随指定 OS 用户登录 GUI 会话启动，以该用户身份运行；使用用户 LaunchAgent，不安装常驻 root LaunchDaemon。
- 将现有 `node_daemon` 作为 **BuckyOS Runtime.app 的主可执行文件**。App bundle 保留在 `BUCKYOS_ROOT/bin/node-daemon/` 内；不增加一个仅负责 spawn daemon 的 launcher 进程。
- 沿用 node-daemon 内嵌 Node Executor、CurrentUser 和共享 Computer Driver 的 A-desktop 路线；本任务不改用独立 `buckyos_agent_daemon`，不新增第二套 executor 身份、配对与默认 host 绑定。
- BuckyOS Runtime.app 承担节点运行与宿主执行；现有 BuckyOS.app 承担用户界面、托盘和权限引导。关闭或退出主 UI 不隐式停止节点。
- 用户在 macOS 隐私设置中识别并授权实际执行截图/点击的 Runtime App。正式安装链必须验证授权归属、生效、撤销和升级连续性。
- 首版按一个安装实例绑定一个 OS 用户设计，不支持多个 OS 用户共享同一 Node 身份/数据并同时运行。OS 用户与 Zone owner 是不同概念，不能仅凭名称相同建立归属。
- 保留现有 `BUCKYOS_ROOT` 配置/数据布局及 Node target 的 host 语义。完整 Desktop 去 Docker、OpenDAN 主进程原生化、Linux/Windows 启动策略调整不属于本任务。
- 按仓库开发阶段约定，全新安装验收，不增加旧 Session、账本或目录迁移。旧系统服务残留仅作为冲突检测/明确清理对象，不作为兼容运行入口。

**2. 目标布局与运行契约**

```text
$BUCKYOS_ROOT/
├── bin/
│   ├── applist.json
│   ├── node-daemon/
│   │   └── BuckyOS Runtime.app/
│   │       └── Contents/
│   │           ├── Info.plist
│   │           ├── MacOS/node_daemon
│   │           └── Resources/...
│   ├── opendan/...
│   └── agent-tools/...           # 复用已有宿主工具布局
├── etc/...
├── security/...
├── data/...
├── storage/...
├── local/...
└── logs/...

<运行用户 HOME>/Library/LaunchAgents/<runtime-label>.plist
/Applications/BuckyOS.app         # 现有 UI 默认安装位置
```

macOS 已安装产物只有 bundle 内的一份 `node_daemon`，不保留旧裸文件、第二份副本或兼容软链。Cargo 自身的编译输出不受此约束。正常用户会话启动等价于：

```text
可执行文件：$BUCKYOS_ROOT/bin/node-daemon/BuckyOS Runtime.app/Contents/MacOS/node_daemon
参数：--enable_active --desktop_daemon
launchd domain：gui/<绑定用户 UID>
会话类型：Aqua
环境：显式 BUCKYOS_ROOT、绑定用户真实 HOME、约定的服务 PATH
WorkingDirectory：$BUCKYOS_ROOT
日志：绑定用户可写的 $BUCKYOS_ROOT/logs/...，不写 /var/log
```

实现约定：Runtime 显示名采用 `BuckyOS Runtime`；Bundle ID 与 launchd label 建议固定为 `com.buckyos.runtime`，在 T0 与现有签名/发布配置核对后统一定义。身份不能包含版本号或用户 UID。用户级 job 的隔离由 domain 与安装归属完成。`Info.plist` 使用原生 `CFBundleExecutable=node_daemon`、稳定 Bundle ID、版本、图标和合适的后台应用属性；无需普通应用窗口。

App bundle 内只保存交付代码和资源。运行状态、密钥、日志、工具生成物继续写根目录下的指定位置，不写入签名 bundle。`BUCKYOS_ROOT` 显式传递；现有向上寻找 `bin/applist.json` 的根发现逻辑保留并回归。[根路径实现](../../buckyos-base/src/buckyos-kit/src/path.rs#L42)

首版采用用户 LaunchAgent 的既有安装方式，由 launchd 直接运行 bundle 内主程序；注册、控制和状态查询使用同一 job。SMAppService 不是本轮额外平台迁移的前置条件。需要的 LaunchServices 注册、应用关联和实际 TCC 归属在 T0/T5 验证，不能仅凭目录名推断成功。

**3. 当前已有基础：实施时复用**

| 基础 | 当前源码事实 | 本任务的工作 |
| --- | --- | --- |
| A-desktop | [node_executor_host.rs:145](../src/kernel/node_daemon/src/node_executor_host.rs#L145) 已按 desktop 参数、平台和非 root 选择 CurrentUser；184 行注入 Driver | 修正安装启动上下文，验证 GUI 会话；不重写 Executor |
| owner_agents | [node_executor_host.rs:241](../src/kernel/node_daemon/src/node_executor_host.rs#L241) 已以 HOME 初始化 roots，run_as=None，已有策略不覆盖 | 保证 HOME 正确、默认 host 和授权生效，管理员禁用不被重开 |
| 标准工具 | [node_executor_host.rs:194](../src/kernel/node_daemon/src/node_executor_host.rs#L194) 已调用 `opendan install-agent-tools`，179 行注入 path_prefix | 保证普通用户有必要写权限，执行真实 baseline 与上下文回归 |
| GUI 探测 | [native.m:47](../src/kernel/computer_driver/src/native.m#L47) 已检测 audit/console、锁屏、截图/输入权限 | 增加实际授权引导与触发接线，复用 readiness 状态 |
| worker 定位 | [node_executor_host.rs:160](../src/kernel/node_daemon/src/node_executor_host.rs#L160) 使用 current_exe 启动 worker 子命令 | 新布局下自然指向同一 bundle 二进制；验证 worker 入口不触发正常节点启动 |
| Executor 退出 | [lib.rs:330](../src/kernel/node_executor/src/lib.rs#L330) 已有 shutdown、停准入、取消和有界等待 | 接入 node-daemon 正常停止/注销/更新退出路径 |

当前 macOS 正式安装仍在 [postinstall](../src/publish/macos_pkg/scripts/buckyos_postinstall) 中创建系统 LaunchDaemon、写 root HOME，且不传 desktop 参数。[preinstall](../src/publish/macos_pkg/scripts/buckyos_preinstall) 仍以 root Docker 可用为门槛。两仓 node_control 的定位和控制、`start.py`、包校验也仍采用旧路径。这些是待修改现状。

**4. 实施任务**

**T0 / P0：验证正式 App 身份与用户会话的最小链路**

- [x] 在目标 macOS GUI 环境制作最小 Runtime bundle，直接使用现有 `node_daemon`，放在最终 `BUCKYOS_ROOT/bin/node-daemon/` 布局；固定应用标识、版本、图标、后台形态。
- [ ] 使用正式发布所需的稳定签名身份测试；开发签名产物单独标记，不用 ad-hoc 构建的授权结果证明发布版升级连续性。签名凭据缺失时明确记录未验证项。
- [ ] 注册 Runtime bundle 的 LaunchServices 信息，并验证用户 LaunchAgent 直接启动后 macOS 识别到的程序/责任应用是预期 Runtime；应用关联设置不能视为自动继承所有 TCC 权限的保证。
- [ ] 验证真实 UID/HOME、GUI audit session、`--desktop_daemon`、CurrentUser 和 Driver describe；从非 Terminal 启动路径实际截取屏幕并点击测试窗口。
- [ ] 验证安装位置中的空格、自定义 BUCKYOS_ROOT，以及 bundle 内主程序/worker 能否正确运行；录下系统设置中的授权名称和实际签名信息。
- [x] 在支持平台上探测基础服务非 root 启动，特别是 gateway 的 80/443 监听需求。若失败，列出具体服务/端口及最小修正；不能直接恢复 root node-daemon 或将整个节点改回系统服务。

完成条件：确定最终启动链能够进入用户 GUI 会话，且实际 Runtime App 可被授权并完成截图/点击。平台限制先形成证据，再调整同一路线内的实现；不静默改成外置第二份二进制或独立 agent-daemon。

**T1 / P0：统一构建、bundle 装配与发布布局**

- [x] 在现有构建流程中加入 macOS Runtime bundle 装配，覆盖完整构建、`-s node_daemon`、rootfs 汇总、开发安装和正式 pkg prepare。复用既有模块/构建扩展点；不只在最终 pkg 脚本临时套壳。
- [x] [bucky_project.yaml](../src/bucky_project.yaml) 保留 `node_daemon → bin/node-daemon/` 的模块边界，但 macOS 模块交付完整 bundle；Linux/Windows 的产物布局保持各自契约。
- [x] Info.plist、资源和 bundle 定义有明确源码模板。固定版本来源，与 `src/VERSION`、包版本保持约定；生成文件不作为唯一修改入口。
- [ ] 打包顺序固定为编译 → 完整装配 → 签名 → 签名校验 → 制作发行 pkg → 按发布要求签署/公证 → 安装后再次验证。签名后不再改 Info.plist、资源或二进制。[Apple 代码签名说明](https://developer.apple.com/library/archive/technotes/tn2206/)
- [ ] ARM64、Intel 产物和当前 Driver 的 macOS 14+ 最低要求在 bundle/包元数据中一致；不把某一架构构建成功当作另一个已验证。
- [x] 检查 devkit 的产物/安装复制流程：当前 [Rust 构建结果](../../buckyos-devkit/src/build_rust.py#L447) 是普通 executable，[目录安装](../../buckyos-devkit/src/install.py#L292) 会删除目标再复制。优先以现有扩展点接入；确需改 devkit 时保持通用、局部，并检查调用方。

完成条件：开发安装和正式 pkg 都得到同一已安装布局，只有一份 Runtime 主程序；bundle 复制前后签名有效，清理后重新构建不依赖旧裸文件残留。

**T2 / P0：安装归属、运行目录与首次配置**

- [x] 将节点安装实例绑定到明确 OS 用户，保存可供 installer、App、node_control 一致读取的 UID、HOME 和实际安装根；优先复用现有本机配置/安装元数据，避免各自猜测当前用户。
- [x] 交互安装可识别并绑定当前 GUI 用户；安装授权用的管理员身份不自动等于运行用户。root 的 HOME、SUDO_USER 等不能单独作为最终归属依据。
- [x] 无有效 GUI 用户时完成文件安装并记录“待用户绑定/待登录配置”，不执行 root fallback、不因缺 PID 返回安装失败。若没有显式目标用户，首次打开 BuckyOS.app 完成归属；所需 root 目录准备由一次性安装/配置步骤完成，不能假定普通用户能自行 chown `/opt`。
- [x] 为目录形成并落实 ownership/mode 清单：`etc/security/data/storage/local/logs` 的运行读写、私钥私有访问、Executor store 当前 UID 所有，以及更新所需路径。Runtime bundle 按程序发布管理，禁止写入运行状态或整树 777。
- [x] 特别准备 `$BUCKYOS_ROOT/bin/agent-tools` 的创建/更新权限；不能只开放 data/logs，随后让已有工具初始化因只读 bin 失败。服务包/cache 的可写需求也按实际消费入口核对。
- [x] 删除 HOME=/var/root、logs chown root 和 /var/log 输出假设；诊断输出使用当前实例日志路径。
- [x] 检测残留 `system/buckyos.service`、旧 plist/旧裸程序和已占用的同节点实例。覆盖安装先明确停掉竞争入口，数据保留遵循现有 data_paths；不迁移旧 root Executor 账本/Session，不能盲目递归 chown 旧安装充当迁移。

完成条件：安装、首次登录及激活以同一已绑定用户运行；普通用户可读写所需状态和生成工具入口，另一 OS 用户不会接管同一节点。

**T3 / P0：登录启动、路径发现与两仓控制入口**

- [x] 生成绑定用户的 LaunchAgent，Aqua 会话、完整可执行文件路径与参数、显式 BUCKYOS_ROOT/HOME/WorkingDirectory、用户日志路径和合理保活/重试策略；生成 plist 时正确转义带空格和特殊字符的路径。
- [x] 启动控制统一使用用户 GUI domain，并按 launchctl 语法区分目标：`bootstrap gui/<uid> <plist>`，`print/kickstart gui/<uid>/<label>`；`bootout` 使用 service target 或 domain + plist。区分目标用户存在、GUI domain 存在与 job 已加载。安装上下文切换同时保证会话和进程身份，不能把仅设置 UserName 或仅执行 asuser 当作完整证明。
- [x] 在主库 [node_control.rs](../src/kernel/buckyos-api/src/node_control.rs) 和 [BuckyOSApp 副本](../../BuckyOSApp/src-tauri/src/node_control.rs) 同步新 binary locator、控制模型、注册/运行状态、start/stop/restart。不得因旧系统 plist 缺失退成 DirectProcess 并额外 spawn。
- [x] 同步 [start.py](../src/start.py)、[check.py](../src/check.py)、[stop.py](../src/stop.py)、[stop_osx.sh](../src/rootfs/bin/stop_osx.sh)、installer 的 executable 检查/ProgramArguments/进程检查及 verify-pkg；不增加旧裸路径 fallback。
- [x] macOS 开发启动使用相同 bundle 路径和 desktop 参数。正式用户会话验收通过 LaunchAgent；显式前台调试可以保留，但必须与受管实例互斥，sudo/SSH 运行不能被报告为 GUI 就绪。
- [x] 处理用户从 Finder 直接打开 Runtime App 的情况：经统一控制逻辑或明确参数默认进入正确桌面模式，并防双启动；不能无参数误起 A-system。worker 子命令仍最先分流，不进入 UI/权限申请或普通启动初始化。
- [x] 固定自启动偏好、当前加载状态和实际运行状态的关系，避免“停止运行”和“禁用登录自启”混成一个不可恢复动作；按下表回归。

| 用户/系统动作 | 目标语义 |
| --- | --- |
| 正常登录，自启已开启 | 加载一个属于绑定用户的实例；依赖未就绪时可重试 |
| 手动启动 | 启动当前会话实例；保留用户原有的登录自启偏好 |
| 停止运行 | 当前会话内停止且不被 KeepAlive 立刻拉起；自启偏好保持，重新登录按该偏好处理 |
| 重启运行 | 在同一 UID/root 下完成受控停机后再启动，不产生第二实例、不改变自启偏好 |
| 关闭登录自启 | 持久关闭后续登录启动，UI 明确当前进程是否仍运行；与“停止运行”分别操作 |
| 退出/关闭 BuckyOS 主 UI | 不隐式停止节点 |
| 注销 | 结束该用户节点和其受管执行；下次登录按自启偏好恢复 |
| daemon 崩溃/仅 controller 被 Host OS 重启 | 保留既有 run item，controller 恢复后继续 reconcile；账本未知操作按现有语义处理，不自动重放副作用 |

实现可复用用户 LaunchAgent 和本机配置表达上述状态，不预设新状态数据库。用户在系统设置关闭后台运行时也须如实呈现，不通过后台反复 enable 覆盖其选择。

**T4 / P0：生命周期收敛与实例边界**

- [x] 为 node-daemon manager 保留可用于有界退出的 handle/停止信号，接上现有 `NodeExecutor::shutdown()`；可处理的退出信号关闭 Executor 准入并有界收敛其活跃命令。进程崩溃/强杀按既有账本恢复，不能声称执行过 graceful shutdown。
- [x] 区分 Executor 活跃命令与已启动的服务/App run item。保留现有“controller 崩溃或被 Host OS 重启不杀 run item、恢复后继续 reconcile”的契约；`AbandonProcessGroup=true` 与服务 detached 行为有这一用途，不能统一改为父进程退出就杀全部子树。[现有控制契约](../src/kernel/buckyos-api/src/node_control.rs#L25)
- [ ] 显式停止整个 Runtime、注销、更新/卸载按各自范围停止 run item：先阻止 job 保活拉起，再执行有界停止和按实例定位的黑盒兜底。bootout 只代表 controller 已退出，不等于服务/容器清理完成；不以父进程消失替代验收。
- [x] 单实例检查绑定 UID 与规范化 BUCKYOS_ROOT；macOS 运行清单由全局 `/tmp/buckyos/run.plist` 改为按安装实例隔离的位置。同步 [run_plist.rs](../src/kernel/node_daemon/src/run_plist.rs)、两仓 reader、停止/诊断脚本；具体路径在同一变更中统一，既有 Linux/Windows 路径不随意改动。
- [x] 删除 macOS 正常管理路径中的全机按名称 kill/查重；核对 PID、进程身份/启动时间和实例归属后操作。清理受管命令、服务和容器，不泛杀用户自行打开的软件。
- [ ] 区分锁屏、快速用户切换和注销：锁屏/失去 console 时 GUI not ready，节点其他能力按既有授权继续；恢复后重新观察桌面再输入。不得控制另一用户桌面。

完成条件：显式停止 Runtime、注销和更新清理目标范围内的执行，不产生拉起竞争；controller 崩溃恢复保留既有 run item 并重新收敛，不重复执行未知命令。

**T5 / P0：Runtime App 的隐私授权与用户引导**

- [x] 由真实 LaunchAgent 中的 Runtime 主进程检测并按用户操作请求屏幕录制、辅助功能/输入权限。复用现有 native bridge 与能力状态；主 UI 负责说明和触发，不在 Tauri 进程请求后假定 daemon 已授权。
- [x] 补齐受控的“申请权限/打开设置/重新检查”链路，优先复用已有本地控制通路；若需扩展控制接口，在实施中明确调用者校验和目标实例，不开放无鉴权的远程权限申请入口。不在周期 probe、后台任务或 worker 中反复弹窗。
- [ ] 根据所用 API 和签名模式准备 Info.plist usage description/entitlement。截图和点击分别验收；Apple Events Automation、文件访问、完全磁盘访问仅按实际功能需要申请，不能把全盘权限当作截图点击的默认前置。
- [ ] 验证 macOS 本地网络权限：LaunchAgent 不享有 root/系统 daemon 的自动许可。检查应用归属、必要 usage description、首次拒绝、重新授权和 gateway/局域网访问；legacy LaunchAgent 的 `AssociatedBundleIdentifiers` 按实际 Runtime Bundle ID 配置并验证。[Apple TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)
- [ ] 稳定签名身份、Bundle ID、可执行文件归属；验证 worker 以及确有需要的 shell 子命令的权限归属，不用给 Terminal 授权的调试结果代替正式链路。[Apple 关于责任应用的说明](https://developer.apple.com/forums/thread/678819)
- [ ] 授权后按系统要求重启 Runtime 或复查；拒绝/撤销后报告具体权限缺项，能从 UI 恢复。只探测 permission=true 不足以验收，必须实际截图、输入并观察结果。

完成条件：干净用户看到名称明确的 Runtime 授权对象，能够自主授权、撤销、恢复；主 UI 在线与否不改变已运行 Runtime 的授权身份。

**T6 / P0：基础服务与 host 标准环境回归**

- [x] 将 Docker 检查从 installer/root 上下文调整到绑定用户的 PATH、context/socket；本任务保留容器功能时允许 Docker/OrbStack 登录后晚于节点就绪，明确报告等待依赖并能恢复。不要额外启动用户未选择的容器产品。
- [ ] 验证用户身份下的激活、设备密钥、gateway、system-config、control-panel、服务部署与所启用的文件服务；处理实际权限/低端口失败，不能只以 node-daemon PID 存在算启动成功。
- [ ] 回归已有 owner_agents 种子：首次以正确 HOME 初始化，新建 owner Agent 的默认 host 可用，profile 无 run_as，管理员关闭后不会重开。不新建第二套 policy 或 executor 注册流程。
- [ ] 回归已有 `opendan install-agent-tools` 与 Node Executor path_prefix；工具入口更新失败必须可诊断，旧目录存在不能替代基线检查成功。
- [ ] 在真实 host session-runtime 执行既有 baseline 检查和 `agent-session`/`xagent`/`buckyos` 最小业务操作，确认 Agent/Session 身份、服务访问和产物位置正确。服务/provider 故障与缺工具分别报告。
- [x] 明确 daemon 的服务 PATH 和 Executor 的工具 PATH 分别配置；Unix shell 仍 env_clear、--noprofile/--norc，不能依赖用户打开 Terminal、加载 zshrc/nvm 后才工作。覆盖需要的 Homebrew 两种架构路径和用户可配置环境，不复制全部登录环境或覆盖系统软件。

完成条件：冷登录、未打开终端时，节点基础服务和 host 标准环境可用；缺 GUI 权限只影响对应能力，不误报整个节点安装失败。

**T7 / P1：完整 bundle 更新、卸载与诊断**

- [ ] 覆盖安装/开发更新前停止目标用户 job 和受管进程，避免 devkit 删除模块目录过程中被 KeepAlive 拉起；以完整 bundle 替换，失败保留明确停止状态，成功验证后按更新前运行状态和自启偏好恢复。
- [x] 同步 [uninstall.py](../src/uninstall.py)、平台包脚本和本地卸载入口：先注销/卸载该用户启动项，再删除程序；配置/用户数据保留或删除沿用明确选项，不误删其他实例。
- [x] 更新现有 [verify-pkg](../src/publish/make_local_osx_pkg.py) 中锁定 system job、旧路径和 root Docker 的规则；检查 bundle 完整性、唯一二进制、签名、启动参数、用户环境和卸载闭环。
- [ ] 主 UI/托盘/CLI 诊断分别呈现安装及用户绑定、登录项启用、当前会话、节点健康、host 工具基线、GUI readiness/权限。必要证据包括 UID、运行根、binary 路径、domain/label、identity_mode、日志路径；不打印私钥/token/完整环境。
- [ ] 验证升级前后 TCC 权限连续性；如果平台要求重新授权，提供准确提示和恢复入口，不能沿用缓存的 ready 状态。

完成条件：可重复覆盖安装、重启和卸载，无双入口、签名破坏或后台复活；诊断能解释待登录、被禁用、依赖未就绪、缺工具和缺权限。

**T8 / P1：自动检查、macOS GUI DV 与文档同步**

- [x] 扩展 [macOS 包测试](../src/publish/tests/test_make_local_osx_pkg.py)、构建/启动脚本测试、两仓 node_control 测试，覆盖最终布局、参数、状态转换及错误分支。测试行为与契约，避免只断言模板字符串存在。
- [ ] 为新安装链增加真正的 GUI 会话 DV；现有 [dev-tests.yml](../.github/workflows/dev-tests.yml) 的 sudo 启动不能证明本任务完成。CI 无 GUI/签名凭据时明确标为未验证，并保留发布前人工/VM 验收入口。
- [x] 同步 [发布说明](../src/publish/readme.md)、[Node Executor 设计](../doc/arch/node_exector.md)、开发运行说明、卸载说明和 BuckyOSApp 使用说明。
- [x] 同变更核对内置 `buckyos_admin`、`buckyos_dev_release`、`opendan_agent_dev`、`ui_core` 手册的路径、启停、发布、权限和诊断。仅修改受影响内容，其他技能无需改动时记录依据。

**5. 实施顺序与验证矩阵**

T0 先验证正式身份/会话可行性；T1 与 T2 并行完成产物和安装归属；T3 接上控制与登录启动；T4/T5/T6 分别收敛退出、权限和真实业务；T7 完成更新卸载；T8 汇总自动检查与平台 DV。T4/T5/T6 可以并行实现，但最终必须用同一个正式安装包联合验收。

| 编号 | 场景 | 通过标准 |
| --- | --- | --- |
| M01 | 全量/单模块构建、开发安装、pkg 安装 | 同一 bundle 布局，唯一 installed binary，rootfs/manifest/payload 一致 |
| M02 | 默认根、自定义根、含空格路径 | root 解析、日志、worker、工具和两仓控制定位正确 |
| M03 | 已登录安装；无 GUI 用户安装后首次配置 | 目标用户正确；无 root fallback；待绑定/待登录状态准确 |
| M04 | 重启到登录页，再登录；注销再登录 | 未登录不运行该 Desktop 节点；登录按偏好启动一个实例；注销收敛 |
| M05 | 托盘启停/重启、退出 UI、关闭自启、系统禁用后台项 | 符合 T3 语义，无重复进程或 KeepAlive 竞争 |
| M06 | 锁屏、快速切换用户、SSH/非 GUI 调试 | GUI readiness 准确，不向其他用户/锁屏桌面发送输入 |
| M07 | 干净用户首次截图/点击授权 | 正确 Runtime 身份，真实截图和测试窗口输入成功；无 Terminal 授权依赖 |
| M08 | 截图/输入拒绝、撤销、重新授权 | 能力分别降级与恢复；节点普通能力不误判失败 |
| M09 | 局域网拒绝/恢复；按需要测试文件/Automation | 对应权限单独说明，实际业务恢复，不靠 root 绕过 |
| M10 | 冷登录 host 标准环境 | 工具布局可写、baseline 通过、Agent/Session 上下文与服务访问正确 |
| M11 | Docker 延迟就绪、gateway/激活/其他服务 | 等待原因准确，依赖恢复后自动收敛，实际服务可用 |
| M12a | 长命令执行中显式停止 Runtime/注销 | 有界退出/可解释终态，按目标范围清理受管执行与 run item |
| M12b | daemon 崩溃或仅 controller 被 Host OS 重启 | 既有 run item 保留，恢复后继续 reconcile；未知操作不自动重放，不将保留工作负载误判为残留 |
| M13 | 覆盖安装/失败、卸载、旧 system job 残留 | 更新期间不拉起半安装产物，签名有效，原状态恢复或明确失败，无后台复活 |
| M14 | 正式签名版本 N → N+1 | Bundle ID/签名连续，权限保留或准确引导重授权，实际截图/点击复验 |

平台范围：macOS 14+，ARM64 与 Intel；记录测试的具体 OS 版本、架构、签名类型、初始隐私状态和登录方式。至少覆盖最低支持版本与当前发布目标版本；缺少平台时列为未验证，不由 Linux 单测代替。

实现后的自动检查按改动范围选择：`cargo test --manifest-path src/Cargo.toml -p node_daemon -p node_executor -p computer_driver -p buckyos-api -- --test-threads=1`；在 `src/` 下运行相关 `tools/tests` 与 `publish/tests` 的 Python 测试；有 App 变更时执行其 Rust 检查与实际托盘操作。构建/打包走仓库的 `buckyos-build.py`、`make_local_pkg.py` prepare/build/verify 入口。host 基线和权限验收必须通过真实服务/GUI 流程，不能以静态包检查替代。

**6. 完成定义与本次记录**

交付完成意味着：一个绑定用户、一个 root 内 Runtime bundle、一个用户会话启动入口；新安装后基础节点可用，默认 host 标准环境可用，用户通过清晰授权即可截图点击；启停、注销、恢复、更新、卸载均有验证证据。发布时列出具体平台通过/未验证项及受影响手册。

本次只新增 TODO 文档，未修改运行代码、安装器或平台配置；未执行构建、安装、签名、GUI/TCC 测试。编写时已核对相关源码与路径，并区分现有能力和待实施项。

**7. 实施记录（2026-10-25）**

本节记录代码落点与验证证据；勾选项表示代码与脚本已改，不代表对应平台验收已通过。

| 任务 | 落点 | 说明 |
| --- | --- | --- |
| T1 产物 | `src/kernel/node_daemon/macos/`（Info.plist、图标）、`src/tools/macos_runtime_bundle.py`、`buckyos-build.py`、`make_local_pkg.py` prepare、`build-macos.yml`/`dev-tests.yml` | `com.buckyos.runtime`，`LSUIElement`，最低 14.0，版本取 `src/VERSION`；组装后裸 `node_daemon` 删除；签名身份 `BUCKYOS_MACOS_CODESIGN_IDENTITY`，缺省 ad-hoc。devkit 未改：bundle 作为 `bin/node-daemon/` 模块目录内容被原样复制，复制后签名仍有效。pkg 组件 plist 改为 `BundleIsRelocatable=false` |
| T2 归属 | `node_control::macos_register_runtime`、`node_daemon service register`、pkg postinstall | 绑定记录 `etc/desktop_runtime.json {user, uid, home}`；root 运行时整棵 root 交给该用户、`bin/node-daemon` 保持 root 所有、`security/` 0700、补齐 `bin/agent-tools`/`local/node_daemon`/`logs/node_daemon`；root 所有的旧 Executor 账本改名 `executor.legacy-root-<ts>` 搁置。已有其他用户绑定或 `node_identity.json` 属于其他非 root uid 时拒绝。无 GUI 用户时 postinstall 成功退出，首次打开 Runtime 经管理员授权绑定 |
| T3 启动与控制 | `node_control.rs`（两仓同步，App 为 vendored）、`start.py`/`stop.py`/`uninstall.py`/`rootfs/bin/stop_osx.sh`、pre/postinstall | LaunchAgent 见 node_control 头部契约；自启偏好 = LaunchAgents 目录中有无 plist，关闭后手动启动从 `local/node_daemon/` 的同一份 plist bootstrap，从不 `launchctl disable`；Login Items 禁用如实上报。未绑定、绑定其他用户、旧 LaunchDaemon 残留时拒绝启动，不退回 DirectProcess。Finder 打开（无参数）走 app 入口：完成绑定 → 请求权限 → 确保受管实例运行；worker 子命令仍最先分流 |
| T4 生命周期 | `node_daemon::run`、`node_executor_host::shutdown_node_executor`、`desktop_runtime::acquire_instance_guard`、`run_plist.rs` | SIGTERM/SIGINT 先有界关闭 Executor（20 s，plist `ExitTimeOut=30`）再退出，run item 不动；flock 单实例 + 绑定 uid 校验；macOS `run.plist` 改到 `local/node_daemon/run.plist`；macOS 黑盒停止只处理绑定用户且可执行文件位于该 root 的进程 |
| T5 权限 | `computer_driver::request_access`、app 入口、`desktop_status.json`、BuckyOSApp 托盘 | Runtime 进程自己请求屏幕录制/辅助功能；常驻进程每 30 s 写 Driver readiness，`node_control` check 呈现；托盘新增“完成设置”“屏幕与输入权限”“登录时启动”。Info.plist 带 `NSLocalNetworkUsageDescription`，LaunchAgent 带 `AssociatedBundleIdentifiers` |
| T6 环境 | LaunchAgent env、`node_executor` 默认工具 PATH、preinstall | daemon 服务 PATH 与 Executor 工具 PATH 分开；Executor 在 macOS 默认含 `/opt/homebrew/bin` 与 sbin；Docker 改为在 Runtime 用户上下文检查且只提示 |
| T8 文档与手册 | `doc/arch/node_exector.md` §4.2/§17.10、`src/publish/readme.md`、`buckyos_admin`、`buckyos_dev_release` | `opendan_agent_dev`、`ui_core` 对宿主环境的描述与本次实现一致，未改 |

已执行的检查：

- `cargo test -p buckyos-api --lib node_control`、`cargo test -p node_daemon` 中 desktop_runtime/run_plist 用例，BuckyOSApp `cargo test --lib node_control` 与 `cargo check`；`tools/tests`、`publish/tests` Python 用例（`test_verify_pkg_infers_rpm_format_from_extension` 在 macOS 宿主上原本就失败，与本任务无关）；BuckyOSApp `tsc` 除 4 个既有的 websdk 类型错误外通过。
- macOS 15.8.1 arm64、ad-hoc、debug 构建、含空格的自定义 root：bundle 组装/签名/复制后校验；未绑定时 `service start` 拒绝；直接运行 bundle 内主程序时根路径发现、单实例锁、绑定其他 uid 拒绝、SIGTERM 有界退出成立；Runtime 自报 uid 501、GUI audit session、两项权限缺失（非 Runtime 身份启动，预期）；`stop_osx.sh` 只停该 root 的进程。gateway 所需端口：普通用户可绑定 `0.0.0.0:80`/`[::]:80`，不能绑定 `127.0.0.1:80`；`boot_gateway.yaml` 用 `[::]:80`，`make_config.ts` 的 443 用 `0.0.0.0`。
- 同机真实 LaunchAgent（2026-10-25，用户在 GUI 会话中执行 `start.py`，release 构建、ad-hoc）：`gui/501`、asid 100002（GUI 会话）、plist 来自 `~/Library/LaunchAgents`、主程序与参数、env（`HOME`/`BUCKYOS_ROOT`/PATH）、`exit timeout = 30` 均符合；进程 uid 501；`service status` 报告绑定、自启、加载与 pid 正确。system-config/scheduler/verify-hub/control-panel 以用户身份运行并监听。cyfs_gateway 起初反复退出，原因是 VS Code 扩展进程占用 `*:443`（与本任务无关）；释放端口后 gateway 以用户身份监听 `*:443`、`*:80`、`*:3180`，`check.py` 为 Running，即基础服务在普通用户 LaunchAgent 下可用（T0 非 root 端口项）。`service stop` 后 job 卸载、本实例进程全部退出且未被 KeepAlive 拉起，LaunchAgents 中的 plist 保留；`service start` 重新 bootstrap，服务以新 pid 恢复，`check.py` 为 Running。
- TCC（T0/T5，ad-hoc 开发签名）：在系统设置中给 “BuckyOS Runtime” 授予屏幕录制与辅助功能后，经 LaunchServices 启动的 app 入口自报两项均已授权；常驻的 LaunchAgent 进程在重启前两项仍报缺失（辅助功能/输入在该进程内也不是实时生效），app 入口 `kickstart -k` 重启后新进程报告 `ui ready`、capture/input ready，即授权对 launchd 启动的同一 bundle 进程生效。重启期间 system-config/verify-hub/scheduler pid 不变，日志有 SIGTERM 有界退出两行。用户第一次重新 `open` 未触发重启，原因未查明；随后同样的入口（`open -n`）触发了重启。
- 控制器崩溃（M12b 部分）：`kill -9` 后 launchd 按 KeepAlive 拉起新实例（`last terminating signal = Killed: 9`，本次连杀 3 次，`runs = 4`），system-config/verify-hub/scheduler pid 不变。cyfs_gateway 例外：既有启动流程在每次 node_daemon 启动时以 `is_restart=true` 调用 `keep_cyfs_gateway_service`（`node_daemon.rs:1966`），会重启正在运行的 gateway；新 gateway 的 443 TLS 栈默认不设 `reuse_address`（cyfs-gateway `tls_stack.rs:702`），旧连接的 TIME_WAIT 使 `bind 0.0.0.0:443` 失败约 30 s，每次控制器重启后 gateway 中断约 35 s。两者均为既有行为，不是本任务引入，但与“controller 崩溃不影响 run item”的契约冲突，需单独修复。
- LaunchServices 同时登记了两份 `com.buckyos.runtime`：安装的 `/opt/buckyos/...` 与构建产物 `src/rootfs/bin/node-daemon/...`（构建后被系统自动登记，标记 launch-disabled）。开发机上每次构建都会出现第二份；pkg 组件 `BundleIsRelocatable=false` 正是为避免安装被重定位到这类副本。

未执行 / 未验证（剩余验收，M01–M14 均未正式跑过）：

- 真实 LaunchAgent：`service autostart`、登录启动、注销、Login Items 禁用、崩溃后 KeepAlive 拉起（注册、启动、停止/再启动与 `kickstart -k` 有界退出已在本机验证，见上）。
- 正式签名身份下的 TCC 归属与升级连续性、worker/shell 子进程归属、授权撤销后的降级与恢复、本地网络权限提示（ad-hoc 下授权对 LaunchAgent 进程生效已验证，见上）。
- pkg 构建、安装、覆盖升级、卸载；旧 root 安装残留的处理；无 GUI 用户安装后首次打开 Runtime 的管理员授权流程。
- Intel 与 macOS 14；GUI DV（`dev-tests.yml` 的 macOS 任务仍以 sudo 直接启动，不能证明本任务）。

已知风险：

- 容器清理仍按 `buckyos.app_instance_id` 标签全机匹配，同机存在两个 root 时会互相影响（首版一个运行实例/机器）。
- 旧 root 安装已写入 system-config 的 owner_agents（roots 为 `/var/root`）不会被改写，需全新安装。
- 屏幕录制授权后需重启 Runtime 才生效；app 入口在本进程已获授权而常驻进程仍报缺失时会 `kickstart -k`，其余场景靠托盘“重启”。
- 自定义 root 且关闭登录自启时，没有 `BUCKYOS_ROOT` 环境变量的 App 回落到 `/opt/buckyos`。

