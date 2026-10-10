# OpenDAN Agent Workspace 设计

> 基于 2026 年 10 月 10 日的架构讨论整理。
>
> 本文是新的架构设计目标，并结合当前仓库实现补充落地边界；不表示全部能力已经实现。旧版[《Agent workspace》](<Agent workspace.md>)中仍适用的生命周期、并发、审计和远程交付要求已合入本文，取舍见 §16.3。
>
> 本文统一使用 OpenDAN、Agent Workspace、Work Session、Runtime 等术语。设计约束与当前实现分别描述；元信息文件格式、接口签名及分布式协调协议尚未定稿，文中的逻辑字段与流程不代表最终协议。当前实现依据及差距见 §16。

相关边界与[《OpenDAN Agent Session 架构设计》](<OpenDAN Agent Session架构设计.md>)、[《Agent Memory 认知管理需求》](<Agent Memory 认知管理需求.md>)对齐。本文只定义 Workspace 的职责，不重复定义 Session 协议、TODO 状态机或 Runtime 传输协议，也不等同于 `src/frame/aiworkspace/` 所承载的 AI Workspace 产品与服务。

## 1. 设计背景与目标

### 1.1 旧设计的基本假设

旧版 Workspace 的实现比较直接：在 Agent Root FS 下设置 Workspace 目录，通过 Workspace ID 找到对应子目录。Agent、Session 与 Workspace 的使用方式主要建立在单机环境之上。

旧设计区分内部 Workspace 与外部 Workspace：内部 Workspace 由 Agent 独占读写；外部 Workspace 可能与人共享。当前实现已支持 Session 使用外部绝对路径，也保留了外部目录登记与链接的工具抽象；尚未形成基于稳定 ID、Runtime 与目录元信息的统一管理机制。

同时，每个 Session 自身也有运行目录。创建需要长期工作目录的 Work Session 时，需要正确选择并绑定 Workspace。绑定错误可能导致工作在错误的目录、资料或项目背景下执行，最终结果完全偏离用户意图。为确保正确绑定，系统已经引入了 Workspace 管理工具和辅助逻辑，但不同实现入口尚待统一。

### 1.2 新架构需要解决的问题

新的 Agent Session 架构面向分布式运行。在 Kubernetes 等部署环境中，不同 Session 不再必然共享同一个本地文件系统，不能继续将 Agent Root FS 下的路径视为 Workspace 的通用定位方式。当前代码已有 Session 驱动者、Agent State 访问和 Runtime 绑定基础，但不代表远程 Session 与 Workspace 管理已经全部接通；例如当前 Session 绑定会明确拒绝尚未部署 helper 的 `remote_ssh` Runtime。

另一个需要保留的事实是：**不是所有 Work Session 都会产生需要长期管理的文件产物。** 调整配置、创建一组虚拟机等工作，主要结果可能是外部系统状态的改变；执行过程中下载的日志、临时文件和中间数据，并不因此成为长期项目产物。

因此，本次重构不是将 Work Session 与 Workspace 合并，也不是取消 Workspace 绑定，而是重新定义 Workspace 的身份、位置及管理方式，使其同时适用于分布式运行、人机协作和不同技术背景的用户。

### 1.3 核心设计结论

**Agent Workspace 是 Agent 已知、用于长期开展工作的目录。Workspace List 是 Agent 对这些长期工作目录的登记与管理，而不是本地目录的简单枚举。**

一个可实际使用的 Workspace，由以下三部分共同描述：

```text
Workspace = Runtime + Directory + Metadata
```

其中，稳定的 Workspace ID 表达身份；Runtime 与 Directory 表达当前位置。身份与位置解耦后，Workspace 可以跨 Session 存续，也可以在目录迁移后继续被识别和引用。

默认使用方式从“Agent 私有目录”转向“可以与人共同使用的外部 Workspace”。Agent 私有 Workspace 仍然存在，但不再是不加判断的默认选择。

**Work Session 的 Workspace 绑定一旦确定，在本次运行期间保持不变，包括挂起后的恢复和进程重启后的继续执行。无法访问已绑定 Workspace 时，本次 Work Session 直接失败，不通过重新绑定延续执行。** Workspace Manager 仍可维护长期目录的位置，但更新后的登记供后续新 Work Session 使用。

## 2. 核心概念与边界

### 2.1 Workspace

Workspace 的核心语义是**长期工作目录**，而不只是“某次执行的输出目录”。

用户围绕一个项目持续修改资料、迭代产物、补充说明，或者由多个 Work Session 先后推进同一项工作，都属于 Workspace 需要支持的使用方式。代码项目只是其中一个例子，文档编制、数据分析、课程材料整理等工作同样适用。

Workspace 中可以存在输入资料、工作文件、中间结果、正式产物、README 和 Notes。是否成为 Workspace，取决于它是否承担长期工作的角色，而不是目录中是否已经有最终产物。

### 2.2 Work Session 与 Session 运行目录

Work Session 是完成一项明确工作的执行过程；Workspace 是可能由多个执行过程持续使用的长期对象。两者不应一一对应。

| 对象 | 核心用途 | 与工作结束的关系 |
| --- | --- | --- |
| Work Session | 执行本次有界工作 | 工作结束后结束该执行过程 |
| Session 运行目录 | 保存本次执行需要的日志、下载文件、临时材料等 | 留存与清理策略由 Session 机制另行定义 |
| Workspace | 承载长期工作资料、上下文和产物 | 不随某个 Work Session 结束而消失 |

一个 Work Session 可以使用已有 Workspace；一个 Workspace 可以承接多个 Work Session。无需长期工作目录的任务，也不必为了满足形式而创建一个空 Workspace。

**没有文件产物，不等于没有工作结果；Work Session 是一次性的，也不等于它使用的 Workspace 是一次性的。**

### 2.3 Runtime 与 Directory

Runtime 是 Agent 实际访问文件系统、执行命令的运行环境，也承担相应的 Sandbox 能力。Directory 是该 Runtime 中的具体目录。

Workspace 不是一个脱离 Runtime 的绝对路径。同样的路径字符串出现在不同 Runtime 中，不能视为同一个位置。

```text
Workspace 的位置 = Runtime 引用 + 该 Runtime 内的目录
```

Agent 自己容器中的运行环境也是一个 Runtime。旧版私有 Workspace 并不是“没有 Runtime”，而是默认使用了 Agent 容器的 Runtime。

Session 分布式运行后，应通过 Workspace 所关联的 Runtime 定位和使用目录，不能假定 Session 当前进程看到的本地路径就是 Workspace 路径。具体的远程访问、命令执行和节点安排，由 Runtime 体系负责，本文不另行规定传输或调度协议。

这里的 Runtime 引用应能解析到稳定的访问环境及其文件系统命名空间，不能直接等同于一次 Session 的进程号或 tmux 会话。当前 tmux Runtime 的 ID 与 Session ID 绑定；长期 Workspace 的位置引用与这种执行实例标识需要显式映射，不能让 Workspace 的寿命依赖某次 Session 是否仍然存在。

### 2.4 Agent RootFS、管理状态与远程交付目标

旧文档中的 Workshop 将 Agent 私有文件、Workspace 索引、执行目录与交付管理放在同一概念下。新设计保留其能力，按职责分开：

| 对象 | 职责 |
| --- | --- |
| Agent RootFS / Agent 包 | 承载 Agent 自身配置、tools、skills 等；可以提供私有长期目录，但不决定所有 Workspace 的位置 |
| Agent State / Workspace Manager | 保存 Known Workspaces、来源、位置、状态与操作索引；不复制保存所有项目内容 |
| Session 运行目录 | 保存本次执行状态、日志和临时材料，具体结构由 Session 协议定义 |
| Runtime / 文件访问能力 | 解析目标目录、执行文件操作与命令、落实实际访问限制 |
| 交付目标 / connector | 对接 Git remote、共享服务、对象存储、发布平台等，提供同步或发布能力 |

远程 Runtime 中的项目目录，或 Runtime 可访问的 SMB 挂载目录，都可以是 Workspace。一个仅提供 HTTP API 的发布端点或一个 Git remote URL，本身没有本模型中的工作目录，应作为交付目标关联到 Workspace 或任务，而不勉强赋予 Workspace 身份。任务也可以不经 Workspace 直接改变外部服务状态。

## 3. 从私有优先转向外部协作优先

### 3.1 保留内部与外部的语义区分

内部与外部 Workspace 的区分仍有价值，但它描述的是使用和协作关系，不应简单等同于本机与远程。

| 类型 | 语义 |
| --- | --- |
| 内部／私有 Workspace | 由 Agent 独占读写，主要由 Agent 自行管理 |
| 外部／可协作 Workspace | 可能与人共同使用，用户或其他工具也可能修改其中内容 |

外部 Workspace 可以就在用户的 PC 上；使用远程 Runtime，也不自动意味着目录是共享的。目录放置在哪里，与谁会参与读写，需要分别理解。

### 3.2 外部 Workspace 成为默认考虑

对于用户开展项目、制作长期材料等常见需求，Agent 应优先考虑可供用户共同使用的 Workspace，而不是默认在自己的私有 Root FS 中创建目录。

**创建者与使用者不是同一个维度：由 Agent 创建的 Workspace，也完全可以是用户日常使用和维护的工作目录。**

例如，程序员让 Agent 新建项目，通常仍然希望自己能够打开目录、修改代码、运行工具和移动项目。不能因为目录是 Agent 创建的，就将其视为 Agent 独占空间。

### 3.3 私有 Workspace 的适用情形

私有 Workspace 不被取消。在用户明确希望 Agent 全权处理、不关心底层文件管理，或者部署环境本身适合由 Agent 管理文件时，使用 Agent 自己的 Runtime 和目录仍然合理。

这一选择应基于用户表达、历史设定、任务性质以及可用 Runtime，而不是将“非程序员”固化为某种用户等级，也不是将“创建新 Workspace”自动解释为“创建私有 Workspace”。

## 4. 稳定身份与位置分离

### 4.1 Workspace ID

每个受管理的 Workspace 具有稳定的 Workspace ID。它用于跨 Session 引用、目录发现、位置更新，以及 Agent Memory 中的长期关联。

Workspace ID 不应由当前路径直接决定。对于被确认是同一个 Workspace 的迁移，目录从 D 盘移动到 E 盘，或者其所在 Runtime 发生变化，都不应仅因位置改变而产生新的身份。

```text
迁移前：Workspace W → Runtime R1 / Directory P1
迁移后：Workspace W → Runtime R2 / Directory P2

保持不变：Workspace ID
需要更新：位置相关元信息
```

上述规则针对“同一个 Workspace 的迁移”。复制、Fork 或多个位置同时持有同一 ID 时如何处理，属于需要另行定义的冲突问题，不能直接按普通迁移处理。

### 4.2 Agent State 中的 Known Workspaces

设计上由 Agent State 中的 Workspace Manager 维护 Agent 已知的 Workspace 集合。原讨论也使用了 Root State 的表述；当前 `AgentStateClient` 尚无 Workspace Manager 门面，具体存储结构仍待实现。

这里保存的是长期工作目录的逻辑登记、描述与位置关系，而不是要求所有 Workspace 的内容都存放在 Agent State 或 Agent Root FS 中。

Workspace Manager 应能够回答：Agent 知道哪些长期工作目录、某项工作应在哪个 Workspace 中开展、该 Workspace 当前位于何处，以及现在是否能够使用。

### 4.3 逻辑信息模型

以下是需要表达的信息，不是最终字段名或序列化格式。

| 信息 | 作用 |
| --- | --- |
| Workspace ID | 表达稳定身份，供 Session、Memory 和管理器引用 |
| 名称／描述 | 说明该目录承载的工作，供用户辨认和 Agent 选择 |
| Runtime 引用 | 指向 Agent 使用该目录所需的运行环境 |
| Directory | 表达该 Runtime 中的目录位置 |
| 使用属性 | 表达私有、可协作等使用特征；具体权限模型待定 |
| 可用性信息 | 表达当前位置是否仍可访问，或是否需要重新定位 |
| 创建与登记来源 | 创建者、登记者、来源 Session、创建/导入时间；区分 Agent 创建和用户提供，但不据此推导资产所有权 |
| 策略引用 | 引用访问范围与操作策略；登记、元信息中的声明都不能自行授予权限 |
| 登记修订与位置修订 | 支持并发更新检测，区分描述变更与执行位置变更 |
| 生命周期 | 活跃、归档等管理状态，与访问可用性分开 |
| 交付关联 | 可选 connector 类型、远端标识、凭据引用及最近同步点；与目录位置独立 |
| 诊断与工作关联 | 最近检查/变更时间、错误或冲突摘要、关联 Session/Task/Worklog 引用 |

名称或描述可以由用户编辑，也可以由 Agent 根据理解补充。描述有助于选择 Workspace，但不能替代稳定 ID。

可用性属于当前位置的状态，不决定 Workspace 的逻辑身份是否存在。

### 4.4 信息的权威来源与一致性

目录元信息负责随目录携带身份及可共享描述；Agent State 负责这个 Agent 的登记关系、已确认位置、策略引用、操作历史与可用性观察；Session binding 负责本次执行实际采用的位置快照。Memory 不能充当这三类事实的真相源。

目录元信息不应保存 Agent 私有凭据、Session 锁或只能在某一台机器解释的运行状态。用户在目录中修改描述后，管理器可刷新展示缓存；Agent 私有备注应单独保存。不得把两处内容都作为可任意覆写的主副本，更不能因描述更新隐式改变 ID 或执行位置。重复字段的最终同步协议仍待定。

更新登记应带上期望修订，冲突时重新读取；目录与 Agent State 之间没有天然的跨文件、跨 Runtime 事务。创建或导入过程中出现部分成功时，应能通过操作标识、已有 ID 与结果记录检查并继续，不能重试一次就生成一个新身份，也不能为清理失败记录而删除用户原有目录。

## 5. 目录自描述、发现、导入与迁移

### 5.1 Workspace 自描述元信息

Workspace 目录中需要有系统能够识别的元信息文件，使 Agent 打开目录时可以识别该 Workspace 的身份，而不只是看到一组普通文件。

这份元信息至少需要支持稳定 ID 的识别，并可承载描述等信息。具体文件名称、存放位置、格式，以及哪些字段保存在目录中、哪些只由 Agent State 管理，尚待细化。

格式定稿时需包含版本识别和字段校验。未知版本、损坏内容或同一目录中的身份冲突必须明确报错，不能按“没有元信息”重新初始化并覆盖；元信息中的 Runtime、路径或权限声明只能作为线索，实际位置以本次访问环境和管理器校验为准。

**本轮讨论没有确定元信息文件名，不应将语音中的示例转写当作最终规范。**

### 5.2 打开目录时的识别行为

Agent 接到用户指令、通过某个可用 Runtime 打开目录后，可以读取该目录的 Workspace 元信息，并与自己的 Known Workspaces 对照。

| 发现结果 | 管理行为 |
| --- | --- |
| 找到已知 ID，位置与登记一致 | 识别为已有 Workspace，按既有关系使用 |
| 找到已知 ID，位置与登记不同 | 识别为可能的位置变化；无歧义时更新登记位置 |
| 找到尚未登记的 ID | 具备导入并建立 Known Workspace 记录的条件 |
| 未发现 Workspace 元信息 | 仍按普通目录理解，根据用户意图决定是否创建或纳入管理 |

打开任意目录不等于自动把它登记为长期 Workspace。导入与新建仍然需要符合用户的工作意图。

### 5.3 自动识别迁移

典型场景是用户将一个项目目录从 D 盘移动到 E 盘，之后要求 Agent 打开新位置。Agent 读取元信息，发现其中的 Workspace ID 与已有记录相同，即可知道“原来跟踪的 Workspace 到这里来了”。

在没有位置冲突的情况下，Agent 更新管理器中的 Runtime、Directory 等位置元信息，不要求用户理解并手动修改内部登记。该更新不改写已有 Work Session 的绑定。

这里的“自动迁移”主要指**自动识别已有 Workspace 并更新位置登记**，不意味着系统自动搬运目录、复制文件，或持续扫描所有机器寻找它。发现可以发生在用户指令驱动的正常目录访问过程中。

### 5.4 迁移机制的边界

不能仅凭“新目录里出现了同一个 ID”，就在所有场景下无条件覆盖旧位置。旧目录仍然存在、用户复制了目录，或者旧位置仍有工作在执行时，都可能产生歧义。

本轮明确的是稳定身份与重新定位能力。实现至少应守住以下边界，具体冲突交互和协调协议再细化：

1. 旧位置是否仍存在、是否有执行占用、用户是否明确移动了原目录，都应作为判断依据；旧 Runtime 离线本身不能证明发生迁移。
2. 多个可访问位置出现同一个 ID 时，先记录位置冲突；用户意图是复制或 Fork 时，为独立副本建立新 ID，可记录来源 Workspace，而不抢占原登记。
3. 新目录元信息必须与待迁移 ID 一致，更新位置需要检查登记修订；并发发现不能以最后写入者无条件覆盖。
4. 位置更新供后续新 Work Session 解析使用，不能改变已有 Work Session 的 Workspace、Runtime、Directory 或绑定的 `cwd`，包括两次工具调用之间和挂起/重启恢复时。原绑定无法继续核验或访问时，本次 Work Session 按 §6.5 失败。

只读目录可以作为参考输入；已有有效且可读的身份元信息时，仍可识别并导入其身份。若元信息缺失且无法写入，或没有权限读取身份，只能明确提供受限的目录登记能力，不能声称已经支持随目录迁移的稳定身份识别。

## 6. Work Session 创建与 Workspace 绑定

### 6.1 绑定仍是关键决策

新设计没有弱化绑定的重要性。绑定错误仍然可能使整个工作基于错误的目录、资料或上下文展开。

变化在于：Agent 应当承担更多理解、推荐和解释工作，而不是将 Workspace 选择完全变成用户需要处理的底层参数。

### 6.2 在意图复述中确认 Workspace

在深度意图分析后、决定创建 Work Session 时，Agent 通常会向用户复述拟开展的工作。复述中应包含 Workspace 的选择与确认。

对于已有 Workspace，应让用户知道使用的是哪一项长期工作；对于新 Workspace，应说明准备创建什么工作空间，并在需要时给出建议位置。

例如：

> 我准备在现有的“课程资料整理”Workspace 中更新本学期材料，使用你已经配置的主机 Runtime，目录保持不变。

或者：

> 我准备新建一个项目 Workspace，放在你 PC 的工作目录中，方便你继续打开和修改；本次 Work Session 负责完成第一版。

这些是交互语义示例，不要求界面使用固定措辞。用户已明确指定或已充分授权的情况下，不必重复询问同一个问题；重点是避免在有歧义时静默绑定到错误空间。

### 6.3 创建前需要明确的内容

涉及 Workspace 的 Work Session，在启动工作前应明确：使用已有 Workspace 还是新建 Workspace、具体身份或创建意图、目标 Runtime 和 Directory，以及当前位置是否具备执行本次工作的必要条件。

若创建前发现 Workspace 因 Runtime 缺失而无法使用，应说明障碍，由创建方重新决策，不启动依赖该目录的工作。已经创建并绑定的 Work Session 无法访问该 Workspace 时直接失败，不能在另一个目录创建同名空间并继续。

不需要长期目录的工作，可以使用 Session 自身的运行目录处理执行材料，无需强制绑定一个没有实际意义的 Workspace。

### 6.4 生命周期保持独立

Work Session 完成后，属于长期工作的文件留在 Workspace 中，后续 Work Session 继续引用同一个 Workspace ID。Workspace 的长期性，不应反过来要求 Work Session 一直存活。

本轮不定义一个 Work Session 同时绑定多个 Workspace 的数据模型；跨 Workspace 访问能力也不等于已经确定了多重绑定机制。

### 6.5 绑定持久化与执行恢复

首轮以 **0 或 1 个主 Workspace** 组织本次工作；额外参考目录与交付目标单独表达。无主 Workspace 时使用 Session 运行目录，不能回退到 Agent RootFS 根目录并将其隐式当作项目空间。

涉及 Workspace 的执行，应持久化以下逻辑信息：`workspace_id`、位置修订、解析后的 Runtime/Directory、实际执行 target/`cwd`，以及本次需要的访问模式。具体字段归属需与 Session binding 协议共同定稿，不能只在 prompt 中保存绑定关系。

绑定应在工作开始前确定，一旦确定，本次 Work Session 不再换绑、解绑后重绑，也不追随 Workspace Manager 的最新位置更新。恢复执行只核验和复用原 binding，不重新选择 Workspace 或改写执行位置；用户确认也不使同一个运行中的 Work Session 获得换绑路径，需要改变绑定时应由上层创建新的 Work Session。

启动或恢复工具执行前，应按顺序核验：

1. Workspace 仍已登记，登记的位置修订与原 binding 一致；原绑定目录中的身份与预期一致。
2. Runtime 可达且能力满足要求，实际执行位置与保存的 binding 一致。
3. 当前权限仍允许所需读写与命令执行；写操作具备相应协调条件。
4. 上次未完成的调用和后台任务已核实归属，具备安全恢复条件后再继续。

发现已绑定 Workspace 无法访问，或身份、位置、权限、实际执行环境不再满足原 binding 时，直接判定本次 Work Session 失败并停止启动新的副作用操作。不进入等待重新定位、等待访问恢复或重新绑定的分支，也不能改用本机同名目录、重新创建旧目录或只改登记就继续旧调用。

失败收尾保留原 binding，报告失败原因、已产生的修改以及未结束或结果未知的后台任务；失败本身不代表回滚已发生的副作用。上层 UI/Goal 可以在恢复访问或完成 Workspace 重新定位后重新决策，并创建新的 Work Session。已失败的 Work Session 不因访问恢复而自动恢复执行。

当前 binding 已记录 Runtime target 与 `workdir` 并校验一致性，这是可复用基础；尚缺 Workspace ID/位置修订的联动校验。配置中另行指定的 Runtime `workdir` 也必须与 Workspace 决策一致，不能形成两个相互矛盾的工作目录来源。

## 7. 新 Workspace 的 Runtime 与目录选择

### 7.1 先满足可用条件，再给出建议

Agent 能使用哪些 Runtime，取决于用户为它配置并开放了哪些可用 Runtime。位置选择不能超出这一范围，也不能因为知道某台机器或某个路径存在，就假定自己能够访问。

在可用范围内，用户明确指令优先。没有明确指令时，再参考历史设定、任务的长期性、用户是否需要直接参与文件管理，以及各 Runtime 的能力和用途。

### 7.2 部署场景中的典型倾向

| 场景 | 合理的默认倾向 | 需要保留的前提 |
| --- | --- | --- |
| BuckyOS Desktop，通过 Docker Desktop 在 PC 上运行 | 优先使用 Host Runtime 下方便用户访问的目录 | Host Runtime 已配置且具备相应访问能力 |
| 标准 BuckyOS，运行在类似 NAS 的个人设备上 | 可以使用 Agent 自己 Runtime 中的目录，包括适合长期保存的 Root FS 位置 | 符合用户的文件管理习惯与部署条件 |
| 用户明确表示不熟悉开发，希望 Agent 全权处理 | 可以倾向由 Agent 自己管理的 Runtime 和目录 | 不覆盖用户已有明确设置 |
| 用户指定一台已加入可用列表的机器 | 使用指定 Runtime 及相应目录 | 路径和执行条件满足工作需要 |

这些是推荐依据，不应写成依赖产品版本名称的硬编码规则。实质上，Agent 是根据可用 Runtime、用户偏好和协作需求选择位置。

同样，Docker Desktop 中运行的 Agent 容器与 Host Runtime 应被明确区分；不能将容器内路径直接当作用户主机目录。

### 7.3 面向不同用户的体验

普通用户可以表达“帮我做一个项目，后续也由你维护”，由 Agent 提出适合的 Workspace 与位置建议。高级用户可以明确指定 Runtime、Directory 或已有 Workspace，并参与后续目录管理。

两类用户使用同一个 Workspace 模型，不需要为不同技术背景建立两套身份或存储体系。差别主要在于用户参与位置决策的深度。

## 8. 长期项目、一次性任务与产物归属

### 8.1 项目类工作倾向于使用 Workspace

不能因为第一条指令很短、第一版产物很小，就认定工作只会发生一次。用户开始做一件事情后，往往还会修改、补充和继续使用。

因此，项目型工作应倾向于选择或建立长期 Workspace，而不是默认将所有内容留在某次 Session 的运行目录中。

### 8.2 一次性任务不必新建 Workspace

一次性工作与是否使用 Workspace，是两个不同判断。

| 工作情形 | 处理倾向 |
| --- | --- |
| 开始一个预计持续修改的项目 | 选择已有 Workspace，或创建适合长期工作的 Workspace |
| 对已有项目做一次修改 | 使用该项目现有 Workspace |
| 在已有工作背景下写一个简单 Demo、生成一份一次性材料 | 作为已有 Workspace 中的一个产物，不必单独新建 Workspace |
| 调整系统配置、创建虚拟机等，无长期文件组织需求 | 不强制创建 Workspace；日志等执行材料按 Session 机制管理 |

“一次性”不能自动推导为“不保存结果”，也不能自动推导为“创建一个 Agent 私有 Workspace”。Agent 需要判断的是该工作与长期目录的关系。

### 8.3 产物登记、交付与回滚

长期文件应留在 Workspace；Session 临时材料中需要长期保存的部分，应显式提升为项目文件或可交付对象。Final Report 至少说明产物归属、相对路径或版本/对象引用，以及未完成的发布或同步；使用 Workspace 时同时给出 Workspace ID。跨 Runtime 引用不能只返回一个在接收方不可解释的绝对路径。

Workspace 的文件状态、Session 的执行状态和产物的接受状态是不同事实。当前 Agent State 已有 `ArtifactHead`、`ArtifactVersion` 和 `workspace_ref` 扩展点，但接受/丢弃产物主要更新登记与指针，不代表文件系统已回滚。Git revert、快照恢复或远端撤回，应由具有该能力的工具执行并报告实际结果；不支持的副作用必须在报告中保留。

Workspace Manager 负责关联这些记录，不接管 TODO 的拆分、依赖和完成判定，也不把“登记了一个产物”自动解释为用户已经接受或对外发布。

## 9. Runtime 依赖与 Workspace 可用性

### 9.1 Workspace 依赖可用 Runtime

Workspace Manager 应知道哪些 Workspace 依赖哪些 Runtime。这个关系既用于正常定位，也用于 Runtime 变更时的影响提示。

当用户准备从 Agent 的可用列表中移除某个 Runtime 时，应能够告知：哪些已知 Workspace 依赖它，移除后哪些工作目录可能变得不可访问。

### 9.2 不可访问不等于被删除

移除 Runtime 后，相关 Workspace 的登记应继续保留，并反映无法访问或需要重新定位的状态。不能将“失去访问路径”直接等同于“长期工作不再存在”。

移除可用 Runtime 配置，不应默认联动删除 Workspace 的逻辑记录或目录文件。Runtime 自身资源的销毁语义，也需要与 Workspace 管理分开处理。

后续恢复访问、重新配置 Runtime，或者在新位置发现同一 Workspace 时，可以重新建立可用的位置关系，供新的 Work Session 使用。已经绑定的 Work Session 若因此无法访问 Workspace，应直接失败；恢复登记可用性不恢复该失败 Session。

可用性观察应带检查时间与原因，至少能区分 Runtime 不可达、目录不存在、无权限、身份不匹配和位置冲突。`READY` 只表示最近检查满足条件，不保证下一次访问必然成功；创建 Work 前仍需核验。生命周期、可用性、执行占用和同步状态应分别表达，例如“已归档且 Runtime 离线”不能被压缩成一个含混的 `ERROR`。

```text
Runtime 被移除
    → 相关 Workspace 可能无法访问
    → 依赖它且无法继续访问的 Work Session 失败
    → 保留 Workspace 身份、描述和已知位置关系
    → Workspace 恢复访问或重新定位后，供新的 Work Session 使用

不等于：自动删除 Workspace 或其工作成果
```

## 10. Workspace Notes 与 Agent Memory

### 10.1 优先将适合共享的知识写入 Workspace

Workspace 跨 Session 存续，因此不应依靠某个 Session 的上下文记住全部工作信息。

鼓励 Agent 优先把适合以文件形式保存的说明与 Notes 写入 Workspace，例如目录用途、资料组织方式、操作说明、项目约定、后续工作提示。README 可以承担其中一部分，但不限定为某一种固定文件结构。

这样，用户和后续参与工作的 Session 都能在实际工作目录中获得必要背景。

### 10.2 不适合落入目录的认知放入 Memory

部分信息不适合直接写为 Workspace 文件，可以按 Memory 机制记录为感知，再由整理 Goal 形成认知，并使用 Workspace ID 建立稳定关联。例如，Agent 对该工作空间的处理习惯，有些适合保留在自身认知中，而不作为共同项目资料。普通 Work Session 不直接修改共享认知；必须保证正确执行的路径、权限、任务要求和锁状态应保存在管理状态或任务数据中，不能只靠 Memory 记住。

```text
Agent Memory 中的某条认知
    → 关联 Workspace ID
    → 使用时通过 Workspace Manager 解析当前位置
```

目录迁移后，只需更新 Workspace 的位置登记，不必将 Memory 中的稳定引用全部改写为新路径。

这里的当前位置解析用于理解关联对象、选择新工作的执行位置，不得借 Memory 召回改变已有 Work Session 的主 Workspace 绑定；已有 Work 始终核验并使用原 binding。

### 10.3 两类记录的分工

Workspace 内文件优先承载适合随工作目录共同维护的说明；Agent Memory 承载不适合落入目录的关联认知。两者互补，不要求把所有知识都写入 Memory，也不要求把 Agent 的所有认知都暴露为项目文件。

同一描述或规则被用户与 Agent 同时修改时，如何合并与确定优先级，仍需另行设计。

## 11. 跨 Workspace 访问

Workspace 首先是一种长期工作的语义组织，不天然构成文件系统层面的绝对隔离。

在一个 Workspace 的工作过程中，Agent 可以在条件允许时访问另一个 Workspace 的资料。例如，新项目需要参考旧项目的文件，Agent 可以通过旧 Workspace 的 ID 查找其 Runtime 与 Directory，再按相应 Runtime 的能力访问。

这种访问仍然受 Runtime 可用性、文件权限和执行能力约束。知道 Workspace ID，不等于自动获得访问权限；知道两个目录的位置，也不等于两个 Runtime 的文件系统已经互通。

因此，应区分三个问题：当前工作围绕哪个 Workspace 开展、执行中需要参考哪些其他目录，以及 Agent 是否有权且有能力访问这些目录。跨目录读取不应让当前工作的 Workspace 归属变得含糊。

## 12. Workspace Manager 与 Homepage

### 12.1 管理器的职责

Workspace Manager 的重点从“管理 Agent 私有目录”转向“管理 Agent 已知长期工作目录的身份、描述与位置关系”。

核心能力包括登记与查询、按工作意图辅助选择、配合 Runtime 创建目录、识别和导入已有 Workspace、更新迁移后的位置，以及检查 Runtime 变更对 Workspace 可用性的影响。

导入与迁移不再只是边缘功能，而是外部 Workspace 成为常态后需要经常使用的管理能力。具体工具数量、接口名称及是否合并操作，本轮不作规定。

### 12.2 生命周期与最小管理语义

以下为管理操作的语义集合，不预设 CLI 或 RPC 名称：

| 操作 | 主要效果与约束 |
| --- | --- |
| 初始化 / 加载 | 加载 Agent 的已知集合与状态；不要求扫描或挂载全部目录 |
| 列举 / 查询 / 解析 | 按 ID 查询身份、用途、位置、最近检查结果及工作关联；名称用于检索，歧义时不能代替 ID |
| 创建 | 在选定 Runtime 的获授权目录创建长期空间和元信息，再登记；失败要保留可恢复的阶段结果 |
| 导入 | 读取并核验已有目录身份，建立本 Agent 的登记关系；不覆盖用户文件，不把导入等同于取得所有权 |
| 建立绑定 / 结束关联 | 工作开始前建立绑定；运行期间禁止换绑或解绑，结束后清理活动关联但保留历史 binding；绑定不代表已获得写锁，结束关联也不删除目录 |
| 检查 / 重新定位 | 更新可用性观察，或按身份与修订检查更新位置；不顺带复制数据 |
| 归档 / 恢复 | 调整活跃列表和新工作选择；保留身份、历史与物理数据，归档不等于修改文件系统权限 |
| 取消登记 | 移除本 Agent 的使用关系；先检查执行引用并保留必要历史，不能联动删除用户目录 |
| 清理 / 导出 | 按明确范围处理缓存、临时材料或指定产物；与物理删除、对外发布分别授权和记录 |

归档、取消登记或重新定位前应检查所有未结束的关联 Work Session（含挂起或等待恢复的 Session）、活跃写入与未结束后台任务。这些管理操作不能改写已有 Work Session 的绑定；使原绑定失效时，相关 Work Session 按失败收尾。清理仅针对有明确归属和保留策略的材料，不能因 Runtime 离线、长期未访问或未出现在当前列表中，就删除项目、用户输入、未交付产物或未知挂载目录。若后续提供物理删除功能，应作为独立操作定义范围与授权。

可选的文件数、大小、最近变更统计用于展示与清理判断，应按需计算并标明观察时间，不以每次列举都遍历远程目录为前提。

### 12.3 Homepage 的展示价值

从 Agent Homepage 看到“这个 Agent 知道哪些 Workspace”，本身就是重要能力。它帮助用户理解 Agent 已经参与了哪些长期工作，并找到继续工作的入口。

展示信息应能够让用户辨认 Workspace 的名称与用途，了解它位于哪个 Runtime、哪个目录，以及当前是否可用。私有或可协作属性也可以作为辅助信息。

还应能展示最近关联的 Work/产物、归档状态、执行占用提示及错误/冲突摘要，并给出恢复访问或重新定位的入口。占用提示是观察结果，不能当作写锁；凭据值和 Agent 私有认知不进入共享展示。

这是对已知长期工作范围的展示，不应仅显示当前机器上恰好可见的目录。暂时不可访问的 Workspace 仍有保留和提示的价值。

## 13. 典型流程串联

### 13.1 在 Desktop 上创建可协作项目

用户提出项目需求后，Agent 分析意图，判断需要长期 Workspace。在已配置的 Runtime 中，结合用户设定推荐 Host Runtime 的合适目录，向用户复述项目目标及 Workspace 安排。确认后创建目录和元信息、登记 Known Workspace，并让本次 Work Session 在该工作背景下执行。

后续用户自己打开、修改或移动该目录，不改变它作为可协作 Workspace 的性质。

### 13.2 用户要求 Agent 全权管理

用户明确不关心代码目录，希望 Agent 负责持续维护。Agent 可以根据部署条件，推荐自身 Runtime 中的长期目录。

即使用户不直接打开这个目录，它仍然是一个有稳定身份的长期 Workspace，而不是随本次 Session 结束即可丢弃的临时空间。后续任务继续关联它。

### 13.3 已有目录被移动

用户移动目录后，要求 Agent 使用新路径。Agent 在新位置发现元信息，识别已有 Workspace ID，并在确认没有冲突的情况下更新登记。原有 Memory 引用和跨 Session 的工作身份保持不变。

这一过程完成的是重新定位，不要求 Agent 重新复制一份项目。

### 13.4 删除 Runtime 配置

用户移除某个可用 Runtime 前，系统列出可能受影响的 Workspace。移除后保留相关登记，并提示这些 Workspace 当前不可访问。后续重新找到相同身份的 Workspace 时，再恢复其位置关联。

### 13.5 Work Session 执行期间无法访问 Workspace

Work Session 已绑定 Workspace W 的 Runtime R1、Directory P1。执行或恢复时发现 R1 不可达、P1 被移动，或访问权限已撤销，本次 Work Session 直接失败，输出原绑定、失败原因与已有副作用。即使管理器随后在 R2/P2 找到同一 Workspace W，也只更新长期登记；上层确认任务仍需继续后创建新的 Work Session，在新绑定下执行，不修改旧 Work Session 的绑定或恢复其运行。

## 14. 并发写入、权限与审计

### 14.1 Session 串行不等于 Workspace 串行

同一个 Session 的执行锁在其覆盖的协调域内，保证该 Session 不被两个驱动者同时推进；不同 Session、其他 Agent、用户编辑器和后台进程仍可能写入同一目录。Activity 或 `current_session` 等占用提示用于发现关联工作，不能替代写锁。

保留旧文档“避免同一工作目录被多个执行同时改坏”的要求，但不再以“同一 Workspace 只能有一个 RUNNING Session”概括全部情况：只读参考可以并行，独立 checkout 可以另行组织；共享可写目录则必须有明确的协调范围。

首轮采用保守的单写者策略：同一协调域内，对同一 Workspace 可写目录执行修改的 Session，需要先取得写入权；没有取得时不得启动写操作，可以有界等待或明确返回忙碌。Workspace 仍可被多个 Session 登记引用。文件级并行和多目录锁不作为首轮前提。

这里的有界等待仅指可访问 Workspace 上的写入竞争，不适用于 Workspace 无法访问；后者直接使 Work Session 失败。

### 14.2 写入协调与恢复要求

写入协调至少需要关联 Workspace ID、位置修订、持有者 Session、操作范围，以及可验证的持有代次。绑定成功不等于取得写入权；写入权也不能自动扩展文件权限。

- 协调必须由能覆盖实际写入者的权威服务或共享机制执行。进程内 mutex、某台宿主上的锁文件，不能宣称保护另一 Runtime 或另一 Agent 中的独立写入者。
- 单机可复用可回收的 OS 锁机制；跨节点若采用租约，必须同时定义过期后的旧持有者拒写机制，例如 fencing token。只等待 TTL 不足以防止失联的旧进程继续写。
- 写入权覆盖一次连续修改及仍在写文件的子进程。暂停推理、等待工具或 Session 退出，不代表后台写入已停止，不能直接释放后允许另一 Session 写入。
- 崩溃恢复先核验原 binding 与旧任务状态，再接管写入权；重新取得后应重读原绑定目录的文件和版本，不能继续使用过期的修改前提，也不能改用新的登记位置。原绑定无法访问时，本次 Work Session 失败。
- 父 Session 持有写入权时若创建共享目录的子 Session，需明确交接或按同一操作协调；不能让父持锁等待子、子等待父释放而死锁。

多 Agent 跨管理器共享写入的权威协调协议仍待设计。在未具备共同协调能力时，应明确限制并发写入或采用独立工作副本，不能把 Agent 内部锁描述为全局保护。

### 14.3 与用户共同修改

用户编辑器通常不参与 Agent 的锁。对外部可协作目录，Agent 在修改前后还应检测预期内容、版本或 Git 基线是否变化；发现非预期修改时重新读取、合并或记录冲突，不应覆盖不属于本次工作的改动。

“读取后比较再写入”本身也不是完整事务。需要强一致写入时，由文件服务或工具提供条件写入、原子替换与并发校验；没有这些能力时必须如实标明限制。Git worktree 等独立副本能减少同目录写冲突，但合并时仍需要冲突处理，复制元信息造成的重复 ID 也必须按 §5 处理。

### 14.4 访问权限与能力边界

实际操作取决于用户授权、Runtime 能力、文件系统权限和工具执行策略共同允许的范围。Workspace ID、目录元信息、创建来源和绑定关系都不是授权凭据。

路径校验应在目标 Runtime 的文件系统中完成，包括绝对路径、相对路径和符号链接解析；不能在控制进程上把一个远程路径 `canonicalize` 后当作远端校验结果。跨 Workspace 访问需要独立解析并检查目标权限。子 Agent 或子 Session 继承 Workspace 引用时，权限只能保持或收窄，不能由此扩大作用域。

文件读写、命令执行、网络、版本操作和发布能力分别检查。不能因内建文件工具限制了目录，就推导 shell 中的任意命令也被限制。当前 `xllm` 的内建文件工具和 shell `cwd` 有策略检查，但 shell 命令仍按当前 OS 用户权限执行；生产隔离需要 Runtime/OS 实际提供相应机制。

远程认证仅保存 credential/secret 引用，由实际访问层按授权取得。密钥与 token 不写入共享元信息、Notes、Git remote 示例或审计正文。元信息属于待校验输入，不得因读取它而自动执行指令、连接新端点或授予权限。

### 14.5 写入归因与可观测性

保留旧文档的“可审计、可追溯”目标。Agent 发起的创建、绑定、位置更新、归档、写入、同步、发布和冲突处理，都应产生结构化操作记录，并关联 Session Worklog；长期操作再关联 Task。无需为 Workspace 复制一套 Session 日志库。

建议最小记录包括：

| 信息 | 要求 |
| --- | --- |
| 定位 | Workspace ID、位置修订、实际 Runtime 与相对路径；跨目录操作分别列出 |
| 归因 | Agent、Session、Turn/Run、Step（可用时）、`call_id` 或等价 action ID；另关联 Task/操作 ID |
| 变更 | 操作类型、修改前后版本/摘要、文本 diff 或 patch 引用；创建、删除、重命名也纳入范围 |
| 结果 | 成功、失败、取消、部分完成或未知；错误摘要、冲突及实际产生的产物引用 |

文本保存 diff 或前后版本；大文件和二进制保存大小、摘要与可访问的对象/快照引用，避免把全部内容塞入 prompt 或日志。diff 被截断、快照缺失、审计覆盖不足时必须显式标注，不能把摘要当作完整恢复依据。

内建文件工具可以在写入入口记录精确变更；shell、编译器等批量修改需要 Runtime 或任务前后快照/Git diff 等补充。用户或其他工具在两次观察之间的修改只能先标记为外部或归因未知，不能全部算作本 Session 的动作。无法覆盖的写入应说明覆盖范围，不承诺“任何程序写文件都已完整拦截”。

写入与日志落盘不是天然事务。审计失败时应记录待补偿状态和已经发生的副作用，不能报成“未写入”后盲目重试；高审计要求场景需要先验证审计通道可用，并设计操作意图、结果补录与幂等恢复机制。现有 `FileWriteAuditBackend` 只是接入点，尚不满足这一完整契约。

## 15. 远程交付、Git 与扩展能力

### 15.1 按能力扩展，不以 Git 定义 Workspace

旧稿 Remote Workspace 的价值在于连接外部协作与交付场所。新模型将目录访问交给 Runtime，将同步和发布能力交给 connector/工具；不要求所有 Workspace 都有远端，也不要求所有外部目录先复制进 Agent RootFS。

交付关联可记录 connector 类型、远端标识、凭据引用、可选缓存位置、最近成功同步点、错误/冲突摘要。多个交付目标不等于多个主 Workspace。若有本地缓存，其位置也应带 Runtime 引用；共享可写缓存必须参与写入协调，不能通过“这是缓存”绕过并发控制。

能力声明应独立表达 `read`、`write`、`versioned`、`sync`、`publish` 等。执行命令仍需实际 Runtime 支持，不能从“远端可写”推导“远端可执行”。读写、列举、同步和状态查询可通过共同的语义接口暴露，但不强求只读服务、Git 和 SMB 都实现相同操作，更不把状态为在线等同于允许发布。

### 15.2 Git 桥接

Git 可作为优先验证的交付适配，但不是 Workspace 的必备条件。对已有仓库，应复用工作目录和现有 Git 工作流，按需检测状态、获取变更、生成提交及关联版本；没有必要固定采用“项目目录复制到另一缓存，再 push”的两段式。

需要保留的设计包括：

- 记录工作开始时的基线与已有未提交修改，提交只包含本次明确选择的内容。
- 将 commit、branch、PR/Issue 等引用与 Session/操作记录关联；模板与平台适配作为扩展点。
- fetch、pull、push、merge 或发布发生冲突时，保存冲突摘要及下一步所需条件，不能通过强制覆盖用户改动来维持表面成功。
- 创建 Workspace、生成文件、commit、push 和创建 PR 分别表示不同副作用；创建或归档 Workspace 不自动触发发布。

Git commit 不替代文件级执行归因，也不保证任意外部副作用可以撤销。没有 Git 的文档和数据目录同样应具备基本的工作记录与产物引用能力。

### 15.3 同步、挂载与长期操作

同步/交付操作需表达空闲、运行中、成功、失败、冲突、取消或结果未知，并保留最近成功点。状态只影响对应关联能力，不应抹掉 Workspace 的身份或其他可用交付目标。

clone、批量复制、挂载、同步或发布等长期操作应接入 Task 管理，提供操作 ID、进度、日志、超时和能力允许的取消。取消意味着停止后续动作，不能承诺已提交的远端数据自动撤回；重试前应核实远端结果，避免重复发布。

SMB/共享盘需要由适配层处理挂载归属、断线重连和只读降级。Workspace Manager 只消费可用性与目录访问结果，不接管全部挂载实现；清理失效挂载时也不能删除底层项目数据。跨 Runtime 数据搬运、持续镜像与双向同步均是显式能力，不随位置登记自动发生。

## 16. 当前实现基础与落地路径

### 16.1 当前仓库核对结果

下表描述本次核对的代码，而非旧需求文档中的承诺。尤其需要区分新的 `libopendan` Session 链路、旧通用工具抽象和开发 CLI。

| 入口 | 已有能力 | 与本设计的差距 |
| --- | --- | --- |
| [Session 配置](../../src/frame/lib_opendan/src/protocol/config.rs)：`WorkspaceRef`、`SessionConfig.workspace` | `Agent { id }` / `External { path }` 两种引用，主 Workspace 可选 | External 仍以路径表示身份，没有 Known Workspace 的稳定 ID 和独立位置记录 |
| [执行目录解析](../../src/frame/lib_opendan/src/runtime/mod.rs)：`resolve_workdir`、`settle_for_session` | 未绑定默认 SessionDir；Agent 目录为 `<agent_root>/workspace/<id>`；External 要求绝对路径 | 显式 Runtime `workdir` 可覆盖默认目录，尚未核验与 Workspace 的语义一致性；没有目录自描述识别 |
| [绑定结构](../../src/frame/lib_opendan/src/protocol/misc.rs)：`Binding`；[绑定核验](../../src/frame/lib_opendan/src/runtime/mod.rs)：`bind_or_verify` | 在 Session lease 内保存实际 Runtime ID、kind、target 和 `workdir`，恢复时比较 | 尚未联动 Workspace ID、位置修订、可用 Runtime 登记与迁移；`remote_ssh` Session 因 helper 未部署而被明确拒绝 |
| [Agent State 门面](../../src/frame/lib_opendan/src/state/mod.rs)：`AgentStateClient` | 已有 Session 登记、activity、perception、cognition、artifact、锁与 behavior 门面 | 尚无统一 Workspace Manager；目录存在不能替代已知集合 |
| [通用 Workspace 工具库](../../src/frame/agent_tool/src/workspace.rs)：`ManagedWorkspaceRecord`、`ManagedWorkspaceToolBackend` | 保留来源、策略引用、状态、锁等字段；抽象 create/bind，可写 `SUMMARY.md` | `WorkspaceRuntimeBackend` 的现有实现为测试 Fake；未接入当前 Session 链路，锁字段不代表已有锁逻辑，`SUMMARY.md` 不是身份元信息 |
| [外部目录工具抽象](../../src/frame/agent_tool/src/workspace.rs)：`ManagedExternalWorkspaceBackend` | 通过 backend 取得 AgentRoot，在 `workspaces/<name>` 建本地软链，以 `workspaces/bindings.json` 记录 `{name, source, mount}` | backend 仅见测试接入；不是远程挂载、同步或稳定 ID 导入，没有自动迁移发现 |
| [开发 CLI](../../src/frame/agent_tool_cli_dev/src/lib.rs)：`CliWorkspaceBackend` | 实际注册 create/bind，写 `<state_root>/index.json`、`workspaces/session_workspace_bindings.json` 和旧 Session 状态 | 使用旧 `session.json` 模型，不等于更新新 `SessionConfig.workspace`；直接文件覆写不提供跨记录事务、CAS 或 Workspace 锁 |
| [布局与锁](../../src/frame/lib_opendan/src/state/fs_client.rs)：`AgentLayout::lock_path`；[活动视图](../../src/frame/lib_opendan/src/state/activity.rs) | Agent 级锁处理 self_improve/artifact；另有 Session/run 锁；activity 提示同 Workspace、同产物和路径重叠 | 当前无 Workspace 写锁；本地 flock 和活动提示均不提供跨 Runtime 单写者保证 |
| [Runtime](../../src/frame/agent_tool/src/runtime/mod.rs)、[文件访问](../../src/frame/agent_tool/src/runtime/files.rs)、[xllm](../../src/frame/agent_tool/src/xllm.rs) | native/tmux/remote_ssh 基础及文件路径策略；SSH 可供独立 xllm 使用 | 不等于 Session 已支持所有远程环境；内建文件权限检查不隔离任意 shell 命令；Host/Container 等设计不能因名称出现就视为已实现 |
| [文件工具审计](../../src/frame/agent_tool/src/file_tools.rs)：`FileWriteAuditRecord` / `FileWriteAuditBackend`；[Worklog](../../src/frame/lib_opendan/src/protocol/worklog.rs) | 文件工具可产生 diff，Worklog 有工具调用和结果归因基础 | 当前 xllm 文件工具使用 `NoopFileWriteAudit`；写后审计失败仅告警，尚无所有写入的持久审计与补偿闭环 |
| [产物登记](../../src/frame/lib_opendan/src/state/artifacts.rs)；[丢弃处理](../../src/frame/lib_opendan/src/runner/inputs.rs) | 管理版本状态和 head，保留 Workspace 关联扩展点 | discard 明确返回 Workspace 回滚不支持；不能据产物状态声称目录或远端副作用已撤销 |

当前 OpenDAN 私有目录使用单数 `workspace/`，旧工具和开发 CLI 使用复数 `workspaces/`，两者不能混写为统一存储协议。旧稿中的 `workshop/index.json` 是建议格式，也不是当前实现事实。

已有测试覆盖工具 Fake backend 的创建/重复绑定、本地软链登记、CLI 名称目录与旧 Session 状态，以及 Session 绑定和活动视图；不能用这些测试代替外部 Runtime、身份迁移、共享写锁和完整审计的验收。

### 16.2 建议实施顺序

1. **收敛身份与解析入口。** 确定 ID、自描述格式、登记修订及 Runtime 位置引用，将创建、导入、查询和重新定位接到 Agent State；从已支持的本地 Runtime 验证内部与外部目录。
2. **接通 Session。** 创建时解析 Workspace 并固定绑定，恢复时只核验原 binding 与实际 `cwd`；无法访问或绑定失效直接失败，不提供运行期重新绑定。模板变量、子 Session、产物引用、CLI 和 Homepage 共用绑定结果。无 Workspace 的任务继续使用 SessionDir。
3. **补齐共享写与审计。** 先落实明确作用域内的单写者、崩溃接管、用户改动检测和日志归因，再开放对应范围的并发协作；不能用旧锁字段替代实现。
4. **扩展远程执行。** 补齐 Runtime 注册/授权、远端 helper、目标文件系统校验及持久化目录约定，验证相同路径字符串在不同 Runtime 中不会混淆。
5. **增加交付适配。** 优先验证 Git，再按实际需求增加共享盘和其他服务；长操作接入 Task，同步/发布状态独立展示。

涉及 `WorkspaceRef`、Session binding 或持久格式变更时，需要同步修改协议/schema、fixtures、创建与恢复路径、CLI、提示词及消费方。当前工程允许 breaking change，不应另建长期双写的旧新索引来维持表面兼容；如需保留既有数据，应提供明确的导入/转换步骤并报告不能转换的记录，而不是猜测旧格式。

### 16.3 旧设计的合并取舍

| 旧设计主题 | 新文档中的处理 |
| --- | --- |
| Workshop 管全部私有目录与 Workspace | 拆为 RootFS、Session、Agent State、Runtime 和交付适配职责，见 §2.4 |
| 来源、时间、策略、冲突和状态元信息 | 保留并区分权威来源与状态维度，见 §4、§9 |
| 创建、加载、归档、清理和产物导出 | 保留，明确归档/取消登记/删除的区别及用户数据边界，见 §8.3、§12.2 |
| local/remote 与固定 `workspaces/` 路径 | 改为位置、协作属性、来源、能力分别表达；旧路径只作为现状说明 |
| 可恢复绑定与一个主工作目录 | 保留并加强 Runtime/位置核验；运行期绑定不可变，无法访问直接失败，见 §6.5；旧稿多重绑定不直接移植 |
| 同一 local workspace 不得多 Session RUNNING | 保留防止冲突的目标，改为按写入范围协调并考虑用户和后台进程，见 §14 |
| diff、任务归因、Policy 与长任务管理 | 保留为可验证的设计要求，说明已有工具覆盖及缺口，见 §14—§16.1 |
| Git/SMB/服务型 Remote Workspace | 目录保留为 Workspace，非目录服务改为交付目标；保留能力声明、凭据引用、同步状态与插件扩展，见 §15 |
| TODO 体系 | 继续由独立模块负责，Workspace 只保留关联和交付边界 |

## 17. 待细化的实现问题

以下问题需要后续设计，不作为本轮讨论已经确定的机制。

| 问题 | 已明确的边界与后续需要 |
| --- | --- |
| 元信息文件规范 | 必须支持稳定身份识别；文件名、目录结构、格式、版本字段尚未确定 |
| Agent State 与目录元信息的分工 | 已明确身份、登记、执行快照的职责；最终字段、原子更新、补偿与同步协议待定 |
| 复制、Fork 与重复 ID | 迁移保留 ID，独立副本需要新身份；冲突检测、操作交互及与 Git clone/worktree 的规则待细化 |
| 多 Agent 共用 Workspace | 允许外部协作；共享管理权、协调服务、租约/fencing 与锁粒度需要另行定稿 |
| 执行期间迁移或 Runtime 被移除 | 已确定运行期绑定不可变，原绑定失效或无法访问直接使 Work Session 失败；仅失败错误协议、旧调用核实与收尾细节待定 |
| Runtime 的长期位置引用 | 不与 Session tmux ID 混用；Runtime 注册表、命名空间和实例映射待定 |
| 权限与安全校验 | 已明确登记不授权；目标 Runtime 的隔离、路径校验、凭据获取和子 Session 授权接口待实现 |
| 元信息更新冲突 | 共享描述与 Agent 私有备注分开；条件更新、合并及历史保留规则待定 |
| Session 文件的保留与交付 | 已区分执行材料和长期产物；自动清理、归档和交付机制不在本轮定稿范围内 |
| 同时绑定多个 Workspace | 已明确允许条件具备时跨 Workspace 访问；是否提供多重绑定模型尚未确定 |
| 跨 Runtime 数据搬运 | 识别位置变化不等于实现复制或同步；实际迁移工具与数据一致性另行设计 |
| 容器 Runtime 中的长期保存 | 使用 Agent Root FS 不改变 Workspace 的长期语义；容器重建后的目录持久化与重新定位方式需在部署设计中明确 |
| 审计与写入一致性 | 归因、覆盖声明及未知结果必须可见；shell 变更检测、日志补录和保留策略待定 |

## 18. 实现检查点与验收场景

首轮实现应能够验证以下核心设计，而不必先引入完整的分布式文件同步或复杂协作机制。

1. **逻辑登记与实际存储分离。** Agent 可以登记不位于自身 Root FS 的 Workspace，并通过 Runtime 与 Directory 定位。
2. **绑定可解释、可确认。** 深度意图分析后的工作复述能够明确已有或新建 Workspace，避免有歧义时静默选错目录。
3. **外部协作成为正常路径。** 在 Runtime 条件具备时，可以在用户可直接使用的位置创建项目，而不是固定创建私有目录。
4. **稳定 ID 支持重新定位。** 打开迁移后的目录时能够识别原 Workspace，更新位置后保持 Memory 等稳定引用有效。
5. **Runtime 变更有影响提示。** 移除 Runtime 前能够识别受影响的 Workspace，移除后保留其登记并反映不可用状态。
6. **长期与临时职责不混淆。** 一个长期 Workspace 可以被多个短期 Work Session 使用；无长期目录需求的任务不被强制分配 Workspace。
7. **绑定不可变，无法访问直接失败。** 显式 Runtime `workdir` 与所选 Workspace 不符时拒绝执行；两次工具调用之间、挂起和重启恢复时均不允许换绑。位置、Runtime 或权限变化使原绑定失效或无法访问时，Work Session 进入失败终态；访问恢复或重新定位后，只能由新的 Work Session 使用新的执行安排。
8. **身份冲突不会覆盖用户数据。** 移动、复制、重复 ID、损坏元信息和未知版本分别给出结果；重复创建/导入和部分失败恢复不产生无意义的新身份或删除已有文件。
9. **共享写入边界可验证。** 两个 Session 在声明的协调域内不能同时写同一受控目录；崩溃、后台写任务和父子 Session 交接可恢复；用户外部改动不会被当作本次改动盲目覆盖。
10. **管理动作不删除成果。** 归档、取消登记、移除 Runtime 和清理 Session 缓存不连带删除长期目录或尚未交付的唯一副本。
11. **写入和交付可追溯。** 文本、二进制、失败与部分完成都能关联实际变更、Session/调用和 Task；审计不完整、结果未知和不支持回滚必须显式报告。
12. **权限不会因引用而扩大。** 跨 Workspace、符号链接、子 Session 继承和不可信元信息不能扩大授权；shell 的实际隔离能力与产品声明一致。

远程能力启用时再增加对应验收：不同 Runtime 中相同路径不会混淆；远端离线后恢复不重建同名空目录替代原 Workspace；Git 冲突、同步取消和发布重试保留实际结果；秘密信息不进入共享文件或日志。未完成远程 helper 与隔离接入前，不能以本地测试通过宣称分布式 Workspace 已可用。

---

**最终定位：Agent Workspace 不是 Agent 私有文件系统下的一个特殊子目录，而是 Agent 对长期工作目录的稳定识别与管理。Runtime 负责实际访问和执行，Directory 表达位置，Metadata 维持身份与描述；Work Session 在明确的工作意图下选择并使用它。**
