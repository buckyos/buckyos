# Rust App Loader 需求文档

## 1. 背景

当前 `app-loader` 以 Python 脚本形式存在，负责应用的 `deploy/start/stop/status` 生命周期控制，并由 `node_daemon` 通过外部进程调用。

这套架构已经暴露出以下问题：

1. `app_doc` 语义在 Rust、Python、TypeScript 之间重复实现，升级时容易产生语义漂移。
2. `app-loader` 与 `node_daemon` 之间通过脚本入口、环境变量和退出码通信，调试和演进成本高。
3. 原本为“单机特化 loader”保留的扩展边界，在实际使用中几乎没有被使用。
4. 当前 Python 版本已经形成“没人敢改”的局面，成为新需求推进的阻力。
5. 后续 Agent Loader 将面向开发者开放，旧架构不适合作为平台级长期能力继续扩展。

因此，本次升级将以不兼容方式重构 `app-loader`：取消外部脚本实现，将应用生命周期管理统一收回 Rust，并作为 `node_daemon` 内部模块实现。

## 2. 产品目标

### 2.1 目标

1. 以 Rust 实现统一的应用生命周期管理能力。
2. 消除 `app_doc` 在多语言之间的重复运行时解释。
3. 将 `app-loader` 从“外部脚本组件”收敛为 `node_daemon` 的内部能力。
4. 为后续面向开发者的 Agent Loader / 标准 Runtime Image 奠定统一运行时基础。
5. 提高跨平台一致性，降低 shell 脚本、解释器、环境变量协议带来的不确定性。
6. 建立可回归测试、可持续演进的实现边界。

### 2.2 非目标

1. 不考虑对旧版 `app-loader` 的兼容。
2. 不要求继续支持旧的 Python `app-loader` 脚本协议。
3. 不要求保留旧的目录结构和脚本入口。
4. 本文不定义完整的开发者 SDK 设计，只定义平台侧生命周期能力。

## 3. 核心决策

### 3.1 架构决策

新的 Rust App Loader 不再作为独立的外部脚本服务存在，而是作为 `node_daemon` 的内部模块实现。

推荐形态：

1. `node_daemon` 内部新增 `app_loader` 模块。
2. `AppRunItem` / `LocalAppRunItem` 不再调用 `bin/app-loader/{deploy,start,stop,status}`。
3. 生命周期操作改为 Rust 内部函数调用。
4. `app_doc` 的运行时解释统一复用 Rust 类型和逻辑，不再维护 Python 版本解释器。

### 3.2 运行时策略决策

新的 Rust App Loader 需要统一管理以下运行策略：

1. Docker App：以容器方式运行。
2. Host Script App：在宿主机直接运行应用包中的入口脚本或可执行文件。
3. Agent Loader App：使用平台维护的标准 Runtime Image，将开发者提供的 `app_pkg` 挂载到容器中运行。
4. VM Runtime App：以虚拟机镜像方式运行，应用由一个 VM 实例承载。

其中，Agent Loader 是未来面向开发者开放的新范式，必须建立在本次 Rust 重构后的统一架构上。
VM Runtime App 是未来规划能力，本次文档需要预留其在模型和状态机中的位置。

## 4. 术语定义

### 4.1 App Loader

平台侧应用生命周期控制器，负责解析应用运行时配置，准备运行环境，并执行 `deploy/start/stop/status`。

### 4.2 Runtime Strategy

应用实际运行方式。一个应用在某个节点上的运行方式必须在运行前被明确决策。

### 4.3 Standard Runtime Image

由平台维护的标准容器镜像，内置 Node、Python 和基础运行工具，用于承载 Agent Loader 模式下的开发者应用。

### 4.4 App Package (`app_pkg`)

开发者交付给平台的应用运行包。平台负责将其安装到节点，并在需要时挂载到标准 Runtime Image 或宿主机执行环境中。

### 4.5 App Type

`app_doc` 中声明的应用逻辑类型，例如：

1. Service
2. Dapp
3. Web
4. Agent

App Type 用于描述应用的业务语义，不直接等同于其运行方式。

### 4.6 Runtime Type

应用在节点上的实际承载方式。当前及规划中的 Runtime Type 包括：

1. Host Script Runtime
2. Docker Runtime
3. Agent Loader Runtime
4. VM Runtime

Runtime Type 由 `app_doc`、节点能力、平台策略共同决定。

### 4.7 VM Runtime

平台以虚拟机镜像运行应用的能力。应用本身以 VM 镜像或 VM 运行包形式交付，由平台分配到某个 VM 实例中运行。

### 4.8 VM Instance

承载 VM Runtime App 的虚拟机实例。一个 VM Instance 可以独占一个应用，也可以由多个应用共享。

## 5. 用户与场景

### 5.1 平台开发者

平台开发者需要在不引入多语言漂移的前提下，安全修改应用生命周期逻辑，并能快速验证影响范围。

### 5.2 应用开发者

应用开发者需要以更低门槛交付应用，不希望每个应用都自行构建、维护完整容器镜像。

### 5.3 节点运维者

节点运维者需要稳定的应用部署、可理解的状态机、清晰的失败日志和一致的跨平台行为。

## 6. 需求范围

### 6.1 本期范围

1. Rust 化 `deploy/start/stop/status` 核心生命周期。
2. Rust 内统一解析 `app_doc` 与安装配置。
3. Rust 内统一实现 Docker、Host Script、Agent Loader 三类运行策略。
4. 替换现有 `node_daemon -> 外部 app-loader 脚本` 的调用路径。
5. 建立新的集成测试矩阵。

### 6.2 后续范围

1. 面向开发者公开 Agent Loader 规范。
2. 提供 Node/Python 模板工程与标准 runtime helper。
3. 在 control panel / CLI 中暴露更强的运维和诊断能力。
4. 支持 VM Runtime App。
5. 支持 VM Instance 共享与调度。

## 7. 功能需求

## 7.1 生命周期接口

Rust App Loader 必须至少提供以下操作：

1. `deploy`
2. `start`
3. `stop`
4. `status`

可选扩展：

1. `remove`
2. `restart`
3. `prepare`
4. `validate`

本期要求先保证 `deploy/start/stop/status` 在行为上完整覆盖当前平台需求。

## 7.1.1 App Type 与 Runtime Type 的关系

Rust App Loader 必须显式区分 App Type 和 Runtime Type。

要求：

1. App Type 描述业务形态，不直接决定运行时。
2. Runtime Type 描述平台承载方式，由平台统一决策。
3. 同一个 App Type 可以映射到不同 Runtime Type。
4. Runtime Type 的决策结果必须可观测、可记录、可测试。

示例：

1. Agent 类型应用可以运行在 Agent Loader Runtime。
2. Service 类型应用可以运行在 Docker Runtime 或 VM Runtime。
3. 某些宿主机脚本应用可以运行在 Host Script Runtime。

## 7.2 deploy

`deploy` 阶段需要完成以下能力：

1. 基于 `app_doc` 和节点能力选择运行策略。
2. 安装应用包及其依赖包。
3. 为 Docker App 准备镜像来源：
   - 本地 tar 导入
   - 远端仓库拉取
4. 为 VM Runtime App 准备镜像来源：
   - VM 镜像包导入
   - 远端 VM 镜像拉取
   - VM 模板或基础镜像选择
5. 为 Agent Loader App 准备：
   - 标准 Runtime Image
   - 挂载目录
   - 必要环境变量
6. 为 Host Script App 准备：
   - 应用工作目录
   - 应用数据目录
   - 启动入口元数据

`deploy` 不要求一定拉起进程，但必须让 `start` 可以在依赖满足时直接执行。

## 7.3 start

`start` 阶段需要完成以下能力：

1. 校验运行所需配置和 token 是否完整。
2. 对不同运行策略执行对应启动流程：
   - Docker App：创建并启动业务容器
   - Host Script App：在宿主机直接执行入口
   - Agent Loader App：启动标准 Runtime Image，并在容器内执行开发者应用
   - VM Runtime App：选择或创建 VM Instance，并在其中启动目标应用
3. 注入运行时环境变量：
   - `app_instance_config`
   - session token
   - zone config
   - device info
   - media info
4. 为后台运行进程建立 pid / instance state 管理。
5. 保证重复 `start` 的幂等性：
   - 已运行时不应创建重复实例
   - 需要时先停后起

## 7.4 stop

`stop` 阶段需要完成以下能力：

1. 正确停止 Docker 容器。
2. 正确停止宿主机上的子进程树。
3. 正确停止 Agent Loader 容器内的业务进程。
4. 正确停止或解绑 VM Runtime App 对应的运行实例。
5. 在 VM Instance 共享模式下，不能错误销毁仍被其他应用使用的实例。
6. 清理 pid 文件、临时目录或中间状态。
7. 对已停止实例返回可预测结果，不应因资源不存在而报错失败。

## 7.5 status

`status` 阶段需要完成以下能力：

1. 统一返回标准状态：
   - `Started`
   - `Stopped`
   - `NotExist`
   - `Deploying`
   - `Exited`
2. 能区分以下情况：
   - 应用包未安装
   - 容器镜像不存在
   - VM 镜像不存在
   - 容器存在但未运行
   - VM Instance 不存在
   - VM Instance 存在但应用未运行
   - 进程已退出
   - 正在部署中
3. 状态判断逻辑必须基于 Rust 内统一语义，不再依赖外部脚本退出码协议。
4. `status` 路径必须足够轻量，适合被 `node_daemon` 周期性调用。

## 8. app_doc 语义要求

Rust App Loader 必须成为 `app_doc` 生命周期语义的唯一运行时解释者。

要求：

1. 直接复用 Rust 侧权威类型定义。
2. 不再维护 Python 版宽松解释器作为正式运行路径。
3. 针对运行时需要的字段，建立明确的校验与错误信息。
4. 对缺失字段、非法字段、运行时不支持字段，返回结构化错误。
5. 支持后续 Agent Loader 需要的新字段扩展。

本次升级后，`app_doc` 新版本可以直接按 Rust 权威语义推进，不再为旧脚本兼容保留额外逻辑。

## 9. 运行策略需求

## 9.0 运行策略总则

Rust App Loader 必须支持统一的 Runtime Strategy 抽象层。

要求：

1. 运行策略是平台内部接口，不暴露为多语言脚本协议。
2. 每种 Runtime Strategy 都必须实现 `deploy/start/stop/status`。
3. 不同 Runtime Strategy 共享统一的实例身份、日志和状态模型。
4. 新增 Runtime Strategy 时，不允许复制一份新的 `app_doc` 解释器。

## 9.1 Docker App

Rust App Loader 需要支持：

1. 按当前主机架构选择镜像。
2. 优先从本地包导入镜像 tar。
3. 必要时从远端仓库拉取镜像。
4. 对镜像 digest 进行校验。
5. 基于 `install_config` 构造：
   - 挂载目录
   - 端口映射
   - 容器参数
6. 容器命名必须可稳定映射到实例。
7. 需要显式支持 Windows/macOS/Linux 下 Docker 的差异化路径处理。

## 9.2 Host Script App

Rust App Loader 需要支持：

1. 宿主机直接运行应用入口。
2. 为应用提供统一数据目录和缓存目录。
3. 支持 pid 管理与状态检查。
4. 支持平台级 token / config 注入。
5. 支持后续逐步减少“应用自带跨平台脚本”复杂度。

## 9.3 Agent Loader App

Rust App Loader 需要支持：

1. 使用平台维护的标准 Runtime Image 运行应用。
2. 将开发者的 `app_pkg` 挂载到容器中。
3. 在容器中以约定入口启动业务逻辑。
4. 支持 Node 与 Python 运行时共存。
5. 将平台通用能力标准化注入到容器：
   - session token
   - zone config
   - device info
   - service ports
   - data/cache/local 路径
6. 将“开发者必须自行构建完整镜像”的要求，降级为可选高级用法。

## 9.4 VM Runtime App

Rust App Loader 需要为规划中的 VM Runtime 预留统一接口。

能力要求：

1. 支持应用以 VM 镜像或 VM 运行包形式交付。
2. 支持平台选择或创建 VM Instance 来承载应用。
3. 支持一个 VM Instance 只承载一个应用。
4. 支持多个应用共享同一个 VM Instance。
5. 支持在 VM 内建立应用级启动、停止、状态检查能力。
6. 支持 VM Runtime 的镜像来源管理：
   - 本地导入
   - 远端拉取
   - 平台标准模板
7. 支持后续对 VM Instance 资源配额、网络、存储和共享策略进行扩展。

本期不要求完整交付 VM Runtime 实现，但模型、接口和状态语义必须为其预留位置。

## 10. 跨平台需求

Rust App Loader 必须支持以下平台：

1. Linux
2. macOS
3. Windows

平台要求：

1. 不通过 shell 字符串拼接作为主路径，统一使用参数化进程调用。
2. 正确处理 Windows 与 POSIX 的进程组、后台启动、超时和 kill tree 语义。
3. 正确处理 macOS Docker Desktop 下的宿主路径映射问题。
4. 正确处理 Windows 路径分隔符、命令行 quoting、可执行文件后缀。
5. 避免要求平台必须预装 Python 解释器来完成平台内部生命周期逻辑。

## 11. 安全需求

1. 不允许将 token 通过不受控的 shell 拼接暴露。
2. 所有外部命令调用必须使用参数化方式执行。
3. 数据目录挂载权限必须明确。
4. App 只能访问声明范围内的数据目录和缓存目录。
5. Agent Loader 的标准 Runtime Image 必须由平台维护和签名。
6. 标准 Runtime Image 的版本必须可追踪、可升级、可审计。

## 12. 可观测性需求

Rust App Loader 必须提供足够的运维可见性：

1. 生命周期关键日志：
   - 选择了哪种运行策略
   - 选择了哪个镜像或 runtime image
   - 挂载了哪些目录
   - 绑定了哪些端口
   - 启动失败的关键原因
2. 结构化错误：
   - 配置错误
   - 环境错误
   - 镜像错误
   - 进程错误
   - 权限错误
3. 状态诊断输出：
   - 当前实例状态
   - 最近一次启动失败原因
   - 当前 pid / container id / image id

## 13. 测试需求

本次升级必须建立新的 Rust 集成测试矩阵。

### 13.1 必测场景

1. Docker App：
   - deploy from local tar
   - deploy by pull
   - start
   - stop
   - status
2. Host Script App：
   - start
   - stop
   - status
3. Agent Loader App：
   - 使用标准 Runtime Image 启动
   - 挂载 `app_pkg`
   - 注入 env
   - 停止与状态检查
4. VM Runtime App：
   - 至少保留接口级测试与状态模型测试
   - 后续补充真实 VM 集成测试

### 13.2 平台场景

1. Linux 基本路径
2. macOS Docker 路径修正
3. Windows 后台进程与结束流程

### 13.3 回归要求

1. 现有 `app_installer` 测试需要能够在新架构下继续验证完整安装路径。
2. 新实现必须覆盖当前 Python `app-loader` 中已经承载的核心能力。

## 14. 交付物

本次项目交付物至少包括：

1. `node_daemon` 内部 Rust App Loader 模块
2. 新版生命周期测试
3. Agent Loader 的标准 Runtime Image 规范文档
4. 升级说明文档
5. 旧 Python `app-loader` 的退场计划
6. VM Runtime 预留接口与模型说明

## 15. 不兼容变更说明

本次升级明确允许以下不兼容变更：

1. 删除旧的外部 `app-loader` 脚本接口。
2. 删除基于退出码的外部协议依赖。
3. 删除 Python 版 `app_doc` 运行时解释路径。
4. 删除为旧版脚本兼容保留的行为分支。
5. 允许对 `app_doc` 生命周期相关字段重新定义权威语义。

## 16. 里程碑建议

### M1：架构收口

1. 明确 Rust 模块边界
2. 明确 `app_doc` 权威类型与运行时接口
3. 明确 Docker / Host Script / Agent Loader 三个策略接口

### M2：替换旧路径

1. 在 `node_daemon` 内完成 `deploy/start/stop/status`
2. 跑通当前系统应用和本地测试应用
3. 去掉外部脚本依赖

### M3：Agent Loader

1. 定义标准 Runtime Image contract
2. 跑通开发者 `app_pkg` 挂载执行
3. 输出开发者文档与模板

## 17. 成功标准

本项目完成后，应满足以下标准：

1. 平台内部不再依赖 Python `app-loader`。
2. `app_doc` 生命周期语义只有一套 Rust 权威实现。
3. `node_daemon` 可以直接管理 Docker App、Host Script App、Agent Loader App。
4. 关键生命周期路径具备集成测试覆盖。
5. 平台团队可以在不担心多语言漂移的前提下继续演进应用运行时能力。

## 18. 参考现有 Python Loader 的核心流程

本节用于指导 Rust 实现时对齐现有系统行为，减少反复比对 Python 脚本的成本。

原则：

1. 参考现有行为，不照抄现有脚本结构。
2. 保留业务语义，移除脚本协议、shell 拼接和历史兼容包袱。
3. 把现有实现中的坑显式记录下来，避免 Rust 版本重复引入。

## 18.1 现有 Python Loader 的总体动机

现有 Python `app-loader` 实际承担了三类职责：

1. 解释 `app_instance_config` 和 `app_doc`
2. 根据节点能力决定“走 Docker / 走宿主机 / 走 Agent”
3. 调用外部命令完成部署和生命周期控制

Rust 版本仍应保留这三类职责，但要以统一类型和内部接口实现，而不是拆散在多个脚本中。

## 18.2 现有 deploy 的主流程

当前 Python `deploy` 的核心意图很简单：

1. 读取 `app_instance_config`
2. 尝试从 `app_media_info.full_path` 下找到 `{appid}.tar`
3. 如果 tar 存在，则执行 `docker load -i`
4. 如果 tar 不存在，则从 zone docker repo 或默认仓库执行 `docker pull`
5. 为后续 `start` 提前准备镜像

Rust 版本应保留的业务语义：

1. `deploy` 的目标是“准备运行环境”，不要求一定启动实例。
2. Docker 路径下优先使用本地随包镜像，再回退到远端拉取。
3. zone 级镜像仓库配置需要继续支持。

Rust 版本不应继承的实现方式：

1. 不再使用 shell 字符串拼接来执行 `docker load` / `docker pull`
2. 不再同时接受多个不一致的镜像字段来源
3. 不再依赖脚本进程退出码表达内部状态

### 18.2.1 deploy 的已知坑

现有 Python `deploy` 中，镜像名读取路径与 `start` 不一致：

1. `deploy` 直接读取 `app_instance_config["docker_image_name"]`
2. `start` 则从 `app_doc.pkg_list` 中按架构解析镜像

这会导致行为不一致。Rust 版本必须统一为单一权威来源：

1. Docker 镜像一律从 Rust 权威 `app_doc` 语义中解析
2. `deploy` 和 `start` 必须共享同一解析路径

## 18.3 现有 start 的主流程

当前 Python `start` 的核心流程可以概括为：

1. 读取：
   - `app_instance_config`
   - `app_media_info`
   - session token
   - zone config
   - device info
2. 解析 `app_doc`
3. 判定 app type 是否为 Agent
4. 如果是 Agent：
   - 找到 `opendan` 可执行文件
   - 找到 agent 包路径
   - 计算服务端口
   - 清理旧 pid
   - 后台拉起 `opendan`
5. 如果不是 Agent：
   - 根据 `device_info.support_container` 决定是否走 Docker
   - 如果走 Docker：
     - 确认镜像存在，不存在则尝试导入或拉取
     - 计算挂载目录
     - 计算端口映射
     - 注入 token 和 zone config
     - `docker run`
   - 如果不走 Docker：
     - 直接执行应用包内的 `start` 脚本

Rust 版本应保留的业务语义：

1. 启动前必须统一准备运行上下文。
2. Agent、Docker、Host Script 三条主路径必须清晰分流。
3. `start` 需要具备自修复能力：
   - 镜像缺失时尝试补齐
   - 已运行实例可先停后起
4. token、zone config、device info 仍然是平台级注入能力。

Rust 版本建议新增的改进：

1. 运行上下文构造抽成独立步骤，不要散落在分支里。
2. 运行策略决策抽成统一接口。
3. `start` 中需要区分：
   - 运行环境准备失败
   - 镜像准备失败
   - 实例创建失败
   - 业务入口启动失败

### 18.3.1 start 的已知坑

#### 坑 1：`app_doc` 类型推断过于宽松

当前 Python `app_doc` 在缺少 `app_type` 和 `categories` 时，会根据 `pkg_list` 猜测类型。

这个逻辑的动机是兼容旧数据，但它也带来问题：

1. 类型判断可能依赖包布局副作用
2. 不同语言实现容易出现推断分歧

Rust 版本建议：

1. 保留必要的推断能力，但把规则写成显式的权威逻辑
2. 对关键缺失字段给出结构化 warning / error
3. 不再把“宽松兼容”当成默认原则

#### 坑 2：Docker 镜像检查和 digest 校验逻辑分散

当前 Python `start` 中镜像检查逻辑包括：

1. `docker images -q`
2. `RepoDigests` 检查
3. `Id` 检查
4. 拉取后重新 tag

这些逻辑是必要的，但不应散落在业务流程中。Rust 版本需要将其抽成独立的 Docker 镜像能力模块。

#### 坑 3：挂载目录权限和路径修正依赖脚本细节

当前 Python 实现中：

1. Docker 挂载目录会先尝试 `mkdir -p`
2. 可写目录会做 `chmod 777`
3. 宿主路径会做 `realpath`

这些行为背后的真实动机是：

1. 兼容 Docker Desktop
2. 避免 macOS `/opt` 等路径映射问题
3. 避免挂载目标不存在导致容器启动失败

Rust 版本应保留这些动机，但要把它们收敛为平台路径准备模块，而不是散落的 shell 命令。

#### 坑 4：Host Script 路径强绑定 `python3`

当前非 Docker 模式下，`start/status/stop` 会继续调用应用包内的 Python 脚本。

这说明旧实现实际上把“宿主运行时选择”硬编码成了 Python。Rust 版本不应继承这个假设，而应改为：

1. Host Script Runtime 明确声明入口类型
2. Agent Loader Runtime 通过标准 Runtime Image 提供 Node / Python
3. 平台本身不再依赖 Python 来实现平台内部生命周期

#### 坑 5：Windows agent 启动路径已存在缺陷

当前 Python 版本在 Windows 下的后台启动和 pid 返回逻辑并不完整，说明“现有脚本天然更稳”并不成立。

Rust 版本需要把：

1. detached start
2. pid 管理
3. kill tree
4. timeout

统一抽成跨平台进程管理模块。

## 18.4 现有 stop 的主流程

当前 Python `stop` 的核心流程可以概括为：

1. 读取 `app_instance_config`
2. 解析 `app_doc`
3. 如果是 Agent：
   - 读 pid 文件
   - 判断进程是否仍在运行
   - 杀进程组或 taskkill
   - 清理 pid 文件
4. 如果是 Docker：
   - `docker stop {container_id}`
5. 如果是 Host Script：
   - 调应用包中的 `stop` 脚本

Rust 版本应保留的业务语义：

1. stop 必须是幂等的。
2. stop 必须能清理平台侧中间状态。
3. 不存在的实例不能被视为致命错误。

### 18.4.1 stop 的已知坑

1. 当前 stop 逻辑对 Docker、Agent、Host Script 的清理深度不一致。
2. Host Script 继续依赖应用自己实现 stop，平台缺乏统一约束。
3. 共享运行时场景下，stop 不能简单等同于“销毁整个承载环境”。

这对后续 VM Runtime 和共享 Agent Loader Runtime 很重要。Rust 版本必须在接口上显式区分：

1. 停止应用实例
2. 回收承载环境
3. 仅解绑当前应用与承载环境的关系

## 18.5 现有 status 的主流程

当前 Python `status` 的核心流程可以概括为：

1. 读取 `app_instance_config`
2. 解析 `app_doc`
3. 如果是 Agent：
   - 读 pid 文件
   - 判断 pid 是否存活
4. 如果节点支持容器：
   - 判断容器是否运行
   - 判断镜像是否存在
   - 镜像不存在时返回“NotExist”
   - 镜像存在但容器未运行时返回“Stopped”
5. 如果节点不支持容器：
   - 调应用包内 `status` 脚本

Rust 版本应保留的业务语义：

1. `status` 是状态机接口，不是仅供调试使用。
2. 必须区分“镜像不存在”和“实例未运行”。
3. Agent、Docker、Host Script 必须能映射到统一状态模型。

### 18.5.1 status 的已知坑

#### 坑 1：状态语义绑在外部退出码上

当前 `node_daemon` 依赖外部脚本退出码来推导：

1. `0` -> Started
2. `255` -> NotExist
3. `254` -> Deploying
4. 其他 -> Stopped

Rust 版本必须把状态语义移回内部枚举，不能继续用外部脚本协议承载平台状态机。

#### 坑 2：Host Script 状态完全外包给应用脚本

当前非 Docker 模式下，平台对运行状态没有统一判断标准，而是继续调用应用包内脚本。

Rust 版本应尽量把平台可判断的部分前移：

1. pid
2. 启动记录
3. 平台级健康状态

应用自定义状态检查可以保留，但不应成为唯一来源。

## 18.6 Rust 实现时建议保留的模块边界

为了避免再次把流程打散，Rust 版本建议至少拆出以下模块：

1. `runtime_selector`
   - 根据 `app_doc`、节点能力、平台策略选择 Runtime Type
2. `runtime_context`
   - 统一构造 token、zone config、device info、media info、挂载目录等上下文
3. `docker_runtime`
   - 镜像准备、容器创建、容器状态、端口和挂载处理
4. `agent_loader_runtime`
   - 标准 Runtime Image、`app_pkg` 挂载、容器内入口执行
5. `host_script_runtime`
   - 宿主入口执行、pid 管理、状态检查
6. `vm_runtime`
   - 为后续 VM Image / VM Instance 能力预留
7. `process`
   - 跨平台进程启动、超时、kill tree、后台运行

## 18.7 这一节的使用方式

本节的用途不是要求实现者复刻 Python 细节，而是要求：

1. 在设计 Rust 模块时对齐现有业务流程
2. 在迁移测试时覆盖现有关键语义
3. 在代码 review 时显式检查是否重复引入旧坑

如果 Rust 设计与现有 Python 行为不一致，必须在评审中明确说明：

1. 是修复旧行为
2. 还是有意的产品变更
