# BuckyOS 仓库安装包生成 PRD

## 1. 背景

BuckyOS 仓库内维护安装包产品定义、打包脚本和配置文件。该模块输入一个准备好的 staging 目录，输出特定平台的未签名安装包。

本 PRD 只描述 BuckyOS 仓库内打包脚本与配置的产品需求和验收标准。签名、CI 调度、CD 验证和 ROM 自动生成不在本模块内完成。

## 2. 模块定位

BuckyOS 仓库打包模块负责：

- 定义安装包组件清单。
- 定义文件安装覆盖规则。
- 从 staging 目录生成 Windows、macOS、Linux 安装包。
- 定义不同平台的安装行为。
- 生成本地开发试用包和 CI 使用的未签名安装包。

本模块不负责：

- 构建当前仓库之外的组件。
- 保存商业签名证书。
- 执行签名流程。
- 定期调度 CI。
- 执行 CD 验证。
- 生成最终 ROM。

## 3. 目标用户

- 终端用户：通过平台友好的安装包安装 BuckyOS。
- 开发人员：生成本地试用包交给他人验证。
- CI 系统：调用打包脚本生成未签名安装包。
- 核心开发人员：维护安装包内容和文件覆盖规则。

## 4. 平台与产物

| 平台 | 支持版本 | CPU 架构 | 安装包格式 | 本期支持 |
| --- | --- | --- | --- | --- |
| Windows | 10/11 | amd64 | exe | 是 |
| macOS | >=14 | amd64, aarch64 | pkg，按 CPU 架构分别产出 | 是 |
| Linux | Ubuntu 系；Fedora 43/44 | amd64, aarch64 | deb、rpm | 是 |

Linux 产物规则：

- 本期同时产出 deb 和 rpm。
- deb 面向 Ubuntu 系发行版。
- rpm 面向 Fedora 43/44。
- rpm 安装包不要求自动化验证，本期采用定期手工验证。
- rpm 不声明支持 RHEL、CentOS Stream、Rocky Linux、AlmaLinux、openSUSE 或其他 rpm 系发行版。

macOS 产物规则：

- macOS 本期产出 amd64 和 aarch64 两个独立 pkg，不要求产出 universal pkg。

## 5. 组件清单

| 组件 | 用户可见名称 | 功能定位 | 是否必选 | 默认是否安装 | 适用平台 |
| --- | --- | --- | --- | --- | --- |
| BuckyOSApp | BuckyOS Desktop | 用户操作 buckyos service 的 GUI | Windows/macOS 可选；Linux 不包含 | Windows/macOS 是 | Windows、macOS |
| buckyos service | Buckyos Service | BuckyOS 节点核心服务 | Windows/macOS 可选；Linux 固定包含 | 是 | 全平台 |
| buckycli | buckycli | 用户操作 buckyos service 的 CLI | Windows/macOS 可选；Linux 固定包含 | 是 | 全平台 |
| cyfs-gateway | 无 | buckyos service 的对外通信组件 | 包含在 buckyos service 中 | N/A | N/A |

Linux 安装包只包含 service 和 cli，不包含 BuckyOSApp。

Windows/macOS 图形安装器中，BuckyOSApp、buckyos service、buckycli 是彼此独立的组件，用户可以任意组合安装。静默安装不支持选择组件，默认安装当前平台适用的全部组件。

组件级安装行为要求：

- 每个组件可以定义自己的安装前行为和安装后行为。
- 组件的安装前行为和安装后行为只随该组件执行。
- 不同组件的安装前行为和安装后行为相互独立，一个组件是否定义自定义行为不影响其他组件。
- 未被安装的组件不执行该组件的安装前行为和安装后行为。

## 6. 文件安装规则

安装包内的文件或目录只分为两种安装模式。具体哪些路径属于哪种模式，由 BuckyOS 仓库内的覆盖规则配置文件决定。

| 规则 | 首次安装行为 | 覆盖安装行为 |
| --- | --- | --- |
| 安装时一定覆盖 | 释放安装包内容 | 使用安装包内容覆盖目标位置已有内容 |
| 已存在则不覆盖，不存在则释放 | 目标不存在时释放安装包内容 | 目标存在时跳过；目标不存在时释放安装包内容 |

配置文件要求：

- 覆盖规则配置文件由 BuckyOS 核心开发者维护。
- 配置文件不随安装包发布。
- 打包脚本根据配置文件生成安装包具体内容和安装行为。
- 配置文件最小声明粒度为文件或目录。
- 未声明的文件或目录不打进安装包。

数据保护要求：

- “安装时一定覆盖”的内容覆盖失败时，安装失败并报错退出。
- “已存在则不覆盖，不存在则释放”的内容，目标已存在时跳过并继续安装。
- “已存在则不覆盖，不存在则释放”的内容，目标不存在但释放失败时，安装失败并报错退出。
- 目标已存在时跳过不算失败。

## 7. 安装行为

### 7.1 首次安装

Windows：

- 用户双击安装包。
- 安装器展示组件清单供用户选择。
- BuckyOSApp、buckyos service、buckycli 可以任意组合安装。
- 用户可以选择安装路径。
- 安装后可以选择是否打开 BuckyOSApp。

macOS：

- 用户双击安装包。
- 安装器展示组件清单供用户选择。
- BuckyOSApp、buckyos service、buckycli 可以任意组合安装。
- 安装路径固定。
- 安装后可以选择是否打开 BuckyOSApp。

Linux：

- 用户使用包管理器安装 Linux 安装包。
- Linux 只安装 service 和 cli。
- Linux 不包含 BuckyOSApp。

通用验收标准：

- 首次安装成功后，用户选择的组件被正确安装。
- 如果安装了 buckyos service，buckyos service 启动正确版本。
- 安装包只要求启动已安装的 buckyos service；service 启动后进入待激活状态还是已激活工作状态，由 buckyos service 自身决定。
- 非静默安装失败时弹框提示。
- 静默安装失败时返回错误码。

### 7.2 覆盖安装

需求：

- 任何版本均可覆盖安装。
- 低版本覆盖高版本时，不保证数据兼容性。
- 低版本覆盖高版本时，不要求额外提示或二次确认。
- 必须替换或保留的内容由覆盖规则配置文件决定。
- 如果声明为“已存在则不覆盖，不存在则释放”的内容目标不存在，则从安装包释放；目标已存在则跳过并保持原样。
- 覆盖安装前，如果已有 buckyos service 或 BuckyOS 相关 Docker 容器正在运行，应先停止。
- 如果覆盖安装在实际变更程序文件前失败，应保持已安装版本不变。
- 如果覆盖安装在已经开始变更程序文件后失败，不要求自动回滚到旧版本；但不得主动删除用户数据和配置文件，且必须向用户报告失败。

验收标准：

- 覆盖安装后，如果安装了 buckyos service，service 启动对应新版本。
- 如果覆盖前有 BuckyOS 相关 Docker 容器正在运行，这些容器覆盖前必须被停止。
- 覆盖安装失败时，静默安装返回错误码，非静默安装弹框提示。

### 7.3 卸载

需求：

- Windows/macOS 本期支持卸载。
- Linux 本期按包管理器默认逻辑卸载，不提供额外交互卸载能力。
- Windows/macOS 卸载时删除程序文件。
- Windows/macOS 卸载时停止并移除 service。
- Windows/macOS 卸载用户数据和配置文件前需要用户确认，默认保留。
- Windows/macOS 中，“已存在则不覆盖，不存在则释放”的内容可以在用户明确选择后删除。

验收标准：

- Windows/macOS 卸载完成后不存在 service 进程。
- Windows/macOS 卸载完成后不存在 service 的二进制文件。
- Linux 卸载结果以系统包管理器行为为准。

### 7.4 静默安装

支持范围：

| 平台 | 是否支持静默安装 | 是否支持选择组件 | 是否支持指定安装路径 | 是否要求管理员权限 |
| --- | --- | --- | --- | --- |
| Windows | 是 | 否 | 否 | 是 |
| macOS | 是 | 否 | 否 | 是 |
| Linux | 是 | 否 | 否 | 是 |

行为要求：

- 静默安装不支持额外安装选项。
- Windows/macOS 静默安装默认安装全部组件。
- Linux 静默安装默认安装 service 和 cli。
- 静默安装成功退出码为 0。
- 静默安装不允许弹窗。
- 静默安装日志保存在本地。
- 静默安装完成后，安装包只要求启动已安装的 buckyos service；service 后续状态由 buckyos service 自身决定。

## 8. 依赖检查与处理

| 平台 | 依赖项 | 处理方式 |
| --- | --- | --- |
| Windows | VC++ runtime、Docker、Docker engine 已启动 | VC++ runtime 可由安装包自动安装；Docker 相关依赖不自动安装，只提示用户 |
| macOS | Docker | 不自动安装，提示用户 |
| Linux | python3、curl、openssl、psmisc、docker | 由 Linux 包管理器处理 |

要求：

- 依赖检查发生在组件选择之后，只检查已选择组件需要的依赖。
- Windows/macOS 安装包为离线安装包。
- Windows/macOS 发现无法自动处理的依赖缺失时，阻止继续安装，并提示用户下一步处理方式。
- Windows/macOS 依赖缺失提示至少提供 Retry 和 Cancel 两个选择；Retry 重新检查依赖，Cancel 退出安装。
- Windows/macOS 依赖缺失提示可以提供下载地址，用户确认后从浏览器打开对应页面。
- Linux 依赖失败由包管理器输出错误并停止安装，BuckyOS 安装包不额外处理。
- Linux 不额外处理 docker 已安装但被用户手动停止的情况。
- 权限不足时，安装失败并报错退出。

## 9. 本地试用包

开发人员可以基于当前本地构建产物生成试用安装包。

需求：

- 试用包面向开发人员、测试人员、协作试用人员。
- 试用包包含当前本地构建产物。
- 试用包不包含用户数据、运行日志、本机激活状态。
- 试用包可以不签名，可以不进入正式发布流程。
- 试用包和 CI 正式包的安装、覆盖安装、卸载、静默安装和组件默认行为必须一致。

验收标准：

- 可以基于当前本地构建产物生成可安装的试用安装包。
- 试用包支持覆盖安装和静默安装。

## 10. 输入输出

输入：

- 准备好的 staging 目录。
- BuckyOS 仓库内组件清单和覆盖规则配置。
- 平台打包参数。

输出：

- Windows exe 未签名安装包。
- macOS amd64 pkg 未签名安装包。
- macOS aarch64 pkg 未签名安装包。
- Linux deb 未签名安装包。
- Linux rpm 未签名安装包。

## 11. 与其他模块的关系

- `buckyos-devkit` 负责构建和安装到 target rootfs，本模块可调用其能力准备 staging。
- `pve-build-system` 调用本模块的打包脚本生成安装包。
- 本模块不执行签名；签名由 `pve-build-system` 在独立签名流程中完成。
- 本模块定义安装包内容规则，`pve-build-system` 不应重新定义这些规则。

## 12. 验收标准汇总

- Windows、macOS、Linux 安装包能从同一套 staging 和配置生成。
- Windows/macOS 安装包提供 BuckyOSApp 组件，Linux 安装包不包含 BuckyOSApp。
- macOS amd64 和 aarch64 分别产出 pkg。
- 覆盖安装遵循两种文件安装模式。
- 静默安装在所有本期支持平台可用。
- Linux deb 和 rpm 必须产出。
- 未声明的文件或目录不进入安装包。
- 输出安装包不包含签名逻辑。
