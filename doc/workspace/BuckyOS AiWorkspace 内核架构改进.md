# BuckyOS AiWorkspace 内核架构改进

> 文档性质：架构改进建议与冻结契约草案。
>
> 整理日期：2026 年 10 月 22 日。
>
> 依据：[《BuckyOS AiWorkspace 核心架构设计冻结思路》](<BuckyOS AiWorkspace 核心架构设计冻结思路.md>)、围绕该文档的架构评审，以及当前仓库实现。
>
> 本文描述建议采用的目标契约。文中的“必须”“不得”约束目标设计，不表示相关能力已经全部实现；现有机制在 §12.1、§12.2 说明。冻结前必须确定的设计决策集中在第十三节（已全部确认），由其推导的实施 TODO 在 §12.3。
>
> 2026 年 10 月 22 日对 D01–D19 做了一轮深度 review，并按源码重新核对了各项“当前实现”。修正直接改在原处；对已确认决策的二阶后果、相互制约和待补裁决，以“**Review 补充**”段落写在对应决策之下；由 review 提出的新决策为 D20–D26（§13.6），已于 10 月 23 日确认。本文所称“托管执行模式”指经 AppLoader 启动一个 Agent Work Session 来执行任务（D23），不再使用“Agent 模式”“用户后台模式”两个名称。

## 一、改进目标与冻结范围

原冻结思路提出的主线继续保留：Workspace 建立在有限、可服务化、可版本化的数据内核上；Block 负责界面和交互；计算具有明确的执行位置、授权身份和资源归属；共享内容、用户私有状态和演示状态分别管理。

本轮改进重点是补齐贯穿这些能力的契约。对象怎样被稳定引用，修改怎样被接受和重放，历史保留到什么时候，计算如何产生副作用，都需要在模块之间保持一致。Source、公式、演示、撤销和配额不能各自定义一套互不衔接的规则。

| 原冻结方向 | 本文建议补充或调整的内容 |
| --- | --- |
| F02、F03：有限类型、长期稳定 | 冻结存储与协作原语；区分业务 Schema；同一协议版本的语义不得静默改变 |
| F04、F05：事务 Apply、确定性合并 | 区分新提交准入与已接受日志重放；补充稳定身份、幂等、历史边界和结果未知处理 |
| F06、F07：URL 资源、本地可用版本 | 区分访问位置、内容身份和保留承诺；固定版本覆盖完整依赖；明确同步身份与副本授权 |
| F08、F09：独立计量、触限只读 | 定义计量口径和准入预留；限制业务写入，同时保留受控维护通道 |
| F10、F11：客户端计算、公式预留 | 统一输入版本和条件写回；Get 不隐式推进权威内容版本；普通公式先禁止循环 |
| F12—F14：用户资源执行、身份分离、Owner 上限 | Workspace 作为任务发起端；执行上下文显式绑定身份与资源；执行意图可靠派发；委托绑定任务版本与能力范围 |
| F15、F16：状态分域、固定版本演示 | 明确共享事务边界、运行状态归属、引用生命周期，以及演示依赖与保留规则 |

许愿格需要补回本地执行模式，但当前首先需要修正的是执行归属：Workspace 服务不能因接收了用户任务，就默认成为该任务的计算宿主和权限来源。本轮应先建立各执行器共同遵守的执行上下文、数据读取、授权、运行和结果提交契约，再分别实现本地与托管执行两种模式（D23）。当前后端运行路径及已补充的 AICC 资源归属见 §12.2。

## 二、逻辑分层与状态归属

### 2.1 内核、服务与执行环境的职责

这里的分层是职责边界，不要求立即对应独立进程或新的代码仓库。

| 层次 | 负责什么 | 边界 |
| --- | --- | --- |
| 数据与协作内核 | 有限值域、协作原语、候选状态、约束检查、操作规范化、确定性重放 | 不执行任意业务脚本，不直接访问外部网络或启动任务 |
| Workspace 服务 | 身份与授权、提交排序、持久化、锁与配额准入、资源保留、同步和任务入口 | 为内核提供明确的校验上下文，不把环境变化隐含进重放过程 |
| 计算与执行环境 | 客户端计算、受控公式求值、后台任务、Agent | 按授权读取输入，通过正常提交协议写回结果 |
| Block 与客户端 | 展示、编辑、组织交互、管理用户工作状态 | 不以客户端校验替代服务端必须保证的约束 |

复杂计算外置后，数据内核仍需有操作成本边界。有限类型并不能保证一次扫描、一次导入或一组修改的工作量天然有限。

### 2.2 状态分域不等于事务分裂

| 状态域 | 主要内容 | 版本与生命周期 |
| --- | --- | --- |
| 共享文档状态 | Data Source、Block Tree、共享的公式和任务定义、显式保存的结果 | 属于同一 Workspace 逻辑事务和一致快照 |
| 用户私有状态 | 表单草稿、视图偏好、未提交交互状态 | 按 Workspace、User、Block 或其他明确命名空间保存，不进入共享内容版本 |
| 运行状态 | Run、执行尝试、同步进度、候选结果、派发与恢复信息 | 独立管理；必要的执行意图须与其生效条件可靠关联 |
| 服务控制状态 | ACL、委托、锁租约、配额预留、凭据绑定 | 按当前授权和服务规则管理，不随普通内容导出传播 |
| Presentation State | 演示基线、房间状态、事件和回放信息 | 引用固定共享版本，具有自己的授权、保留和结束规则 |

Data Source 与 Block Tree 可以分开组织，但必须允许一次逻辑提交原子完成“创建数据源、创建 Block、建立绑定”。修改 Schema 与调整依赖该 Schema 的共享配置，也必须能在同一提交中完成。

共享快照应覆盖一致的 Data Source 与 Block Tree，不能任意拼接两个时间点的状态。私有 UI State 不参与这个共享版本。

业务数据提交与私有草稿清理可以使用不同存储，但需要恢复协议。例如，新增记录已成功而响应丢失时，应先恢复该次提交结果，再清理草稿；不能重新生成一次插入。跨设备草稿的覆盖策略、Block 删除后的清理策略和身份切换规则也需要明确。

## 三、稳定身份、基础原语与约束

### 3.1 身份与版本分别表达

| 概念 | 语义 |
| --- | --- |
| 名称、路径 | 可变的发现与展示入口 |
| Workspace ID、Entity ID | 持续编辑和引用的逻辑对象身份 |
| 行、列、元素、富文本块 ID | 协作操作需要的稳定子项身份 |
| Content ID | 某个不可变内容版本的身份 |
| URL | 访问位置或解析入口；是否绑定固定内容须由引用语义明确 |
| 协议版本 | 命令、规范化、冲突和重放规则的版本 |
| Schema 版本 | 数据结构及其校验契约的版本 |
| Workspace epoch | 隔离不能直接续接的提交历史 |
| CRDT lineage | 某个协作对象的因果历史身份 |

持久引用和修改操作应使用稳定 ID 与明确的 selector。名称解析得到目标后，后续重试不得重新按当前同名对象选择目标。删除后的 ID 不分配给另一个逻辑对象；恢复原对象与新建同名对象是不同操作。

epoch 与 lineage 不能合并成一个版本概念。更换提交历史起点不必销毁每个对象的协作历史；保留 lineage 也不意味着旧提交信封可以绕过新 epoch 和当前权限校验。

### 3.2 冻结可组合的协作原语

类型定义至少要说明以下内容：

| 项目 | 必须回答的问题 |
| --- | --- |
| 值域 | 可表示什么，大小、深度、精度和空值规则是什么？ |
| 寻址 | 如何稳定定位目标，数组插入是否改变后续修改的目标？ |
| 原子修改单位 | 一次操作替换整体值，还是修改独立子项？ |
| 冲突单位 | 哪些并发修改可合并，哪些必须拒绝或要求锁？ |
| 操作集合 | 允许哪些修改，失败如何表达？ |
| 约束单位 | 校验单字段、对象、集合，还是事务最终状态？ |

建议将原子值整体替换、按稳定 key 或元素 ID 修改的容器、具有专用操作的 Rich Text 与 Table 分别定义。JSON 是编码形式，不能仅凭其外观推导修改和合并语义。

业务 Schema 可以组合原语，但不得新增隐含的提交、合并或权限语义。内核提供的声明式约束也必须是有限、可计量的集合；超出该集合的任意业务代码属于执行层。

### 3.3 自动合并必须与不变量保护配套

例如初始状态为 min = 0、max = 10。A 将 min 改成 8，B 将 max 改成 5；两人分别检查旧状态都合法，按字段直接合并却得到 min > max。

因此需要区分三种保证：

| 约束来源 | 保证方式 |
| --- | --- |
| 内核支持的声明式约束 | 由内核强制检查，提交后状态必须满足约束 |
| 普通客户端的业务检查 | 属于应用约定，不能宣称其他写入者也会遵守 |
| 要求系统级保证的外部业务校验 | 限制写入路径至受信执行者，并携带覆盖实际依赖的版本条件 |

读取集合的版本前置条件用于证明决策依据未改变，不能证明调用者确实做了正确校验，也不能自动补全遗漏的依赖。依赖“当前所有记录”的判断，还必须覆盖新增、删除等集合变化，而不只是已经读到的行。

约束是否成立应以事务最终候选状态为准。具体操作仍可有自己的前置条件，但不得因为合法的多步修改需要经过中间状态，就使其无法表达为一个原子事务。

## 四、提交准入、持久化与确定性重放

### 4.1 新提交的准入流程

一次新提交需要明确携带或绑定：提交身份、协议与历史上下文、目标稳定 ID、操作、必要的读取前置条件，以及适用的锁或执行授权。

服务端完成如下逻辑步骤，具体接口字段和实现顺序可继续细化：

1. 验证调用身份，识别该提交是否已有接受记录。
2. 对尚未接受的新提交检查协议、epoch、当前权限、锁和执行资格。
3. 在候选状态上应用操作，检查读取条件、类型、引用和受支持的约束。
4. 完成资源可用性验证和配额准入；必要的容量预留应参与并发控制。
5. 形成规范化操作及足以重放的上下文。
6. 将数据、引用、历史、接受记录和版本推进作为一个原子提交持久化。
7. 在达到约定持久化边界后确认成功，随后通知订阅者或派发已保存的工作。

上面是逻辑步骤。当前实现的实际顺序是：协议与 epoch → 请求大小与摘要 → 幂等查找 → 是否持有任意授权 → 放映锁 → run 状态 → 规划（逐操作能力位、`expect` 与前置条件都在规划内检查）→ **写锁**（锁目标由规划产出，因此只能在规划之后）→ 单事务写入（[workspace.rs](../../src/frame/aiworkspace/store/src/workspace.rs) `commit`）。写锁后置是合理的，13.2 按此表述确认。

二进制资源可先进入暂存区，但业务提交成功时，其要求的内容必须已经可靠可用。暂存成功而业务提交失败，可以留下待回收对象，不能留下已经发布却缺失必要内容的引用。

### 4.2 幂等与结果未知

服务端业务结果区分 accepted、conflict、rejected。客户端超时或断线产生的是对结果的 unknown 状态，不是第四种服务端事务结果。

相同提交身份与相同请求的重试，应恢复原接受结果，不再次产生修改；相同身份对应不同请求必须拒绝。已经接受的提交，不应因为重试时权限撤销、锁过期或配额变化，被重新解释为此前没有执行。

查询接受事实仍需遵守身份和结果可见性规则。恢复已提交事实不等于重新授予写权限，也不意味着可以返回调用者当前无权读取的完整业务内容。

幂等保证需要有明确的保留边界。若接受记录已回收，必须通过接受窗口或历史边界拒绝无法安全判断的旧请求，不能把它当作从未出现的新请求执行。

### 4.3 重放复现已发生的修改

重放依据的是已接受的规范化日志，以及与之匹配的协议、版本格和协作状态。重放不得使用当前时间、当前 ACL、最新 Source 或新的随机值重新判断过去应发生什么。

权限、锁和资源变化属于提交准入环境；影响内容结果的必要输入则必须进入可复现的上下文。执行重放的入口本身仍需授权，“重放不重新准入”不代表任何人都能读取历史。

重放只恢复数据，不调用模型、不启动容器、不重新同步外部 Source，也不重新触发历史中的任务。恢复时可以取得已经固定身份并校验内容的对象，但不能把源站当前返回的新内容替代历史输入。

### 4.4 内容快照、协作快照与升级

内容快照在声明的保留范围内保证某个内容状态可被读取；完整协作快照还保存续接协作历史所需的信息。相同富文本内容不保证具有相同 lineage，不能仅靠 Content ID 接收任意旧增量。

同一协议版本的语义不得静默改变。升级时应明确采用继续支持、显式转换或拒绝并重新同步中的哪种策略；当前开发阶段可以采用明确的 breaking change，不要求增加旧版本兼容层。

需要新 epoch 时，必须说明旧提交如何失效、尚未提交的本地修改如何恢复。普通 Schema 迁移不必一律更换 epoch，epoch 变化也不必一律重建对象 lineage。

## 五、引用、所有权、撤销与历史保留

### 5.1 绑定不表示拥有数据

Data Source 具有独立身份和生命周期。Block 绑定某个数据源，不自动取得该数据的所有权；数据被第二个 Block 引用，也不应改变第一个 Block 的身份或复制规则。

删除 Block 是否删除数据，按数据归属决定（D05）：在数据模式中插入的数据不随任何 Block 删除；随 Block 在画布上创建的数据跟随 Block。删除共享数据应是显式操作，并按照引用种类判断拒绝、解除依赖或保留失效关联。复制应明确区分“复制投影”和“复制内容”，默认规则见 D05；整组复制需要统一的 ID 映射，使原本共享一份数据的 Block 按同一策略处理。

### 5.2 引用语义至少包含两个维度

| 维度 | 选择 | 含义 |
| --- | --- | --- |
| 版本选择 | live / fixed | 跟随逻辑对象更新，或绑定具体内容版本 |
| 完整性要求 | 强依赖 / 可失效关联 | 操作必须维护目标有效性，或允许目标消失并显式报告失效 |

具体类型可以定义更细规则，但不能仅从“有一个 URL”推导这些语义。固定内容的引用还需要独立的保留承诺；固定身份不等于永久可用。

引用完整性约束不赋予额外访问权，也不阻止授权者撤权。目标是否存在、内容是否被保留和当前调用者是否可以读取，应分别判断。

同一 Workspace 内的强引用完整性由事务维护，解除引用和删除目标应能原子完成。跨 Workspace 不隐含分布式事务，可通过固定版本、本地持有和明确的 unavailable 状态建立边界。

### 5.3 撤销是新的补偿提交

Undo 在当前状态上尝试补偿之前的修改，继续检查权限、版本和约束。它不能直接恢复整个旧快照并覆盖其他人的后续编辑。

依赖已改变、权限已撤销或所需历史已回收时，撤销允许明确失败。若提供部分撤销，应先给出可执行范围并由调用者确认，不能默默改变原来的撤销目标。

### 5.4 保留与回收必须覆盖同一套依赖

需要明确哪些对象构成保留根，例如当前内容、保留的历史、备份、演示基线、运行中任务输入和尚未到期的暂存资源。保留一份快照时，应覆盖其承诺可用的必要内容及资源。

保留策略必须说明保留者、资源承担者、有效期及解除方式。保留结束后，历史版本或撤销能力可以过期，但应返回明确状态；不能继续宣称版本完整可用。

删除当前对象未必释放空间：旧内容可能仍被历史、演示或其他 Workspace 保留。垃圾回收应按保留依赖判断，不能只检查当前对象树中是否还存在该对象。

## 六、Source、本地版本与依赖闭合

### 6.1 区分本地物化数据与远端查询入口

作为核心业务依赖并承诺当前版本可用的 Source，首次创建成功前应取得完整的约定结果，并可靠保存到本地。同步过程先准备候选版本，成功后原子切换；失败保留最近一次有效版本并报告刷新失败。

如果产品继续提供远端按需查询入口，应显式声明其能力：是否支持固定源版本、一致分页、稳定记录身份和离线读取。未物化的查询结果不自动获得本地数据源的可用性保证，也不能被计入完整的离线或演示快照。

对于大数据集，完整性针对明确的查询结果或数据切片，不要求复制整个源数据库。查询范围、分页完整性和源版本共同决定“这一份结果”是什么。

### 6.2 同步身份与副本授权

Source 定义应说明由谁提供访问授权、谁可以刷新，以及本地副本如何共享。使用 Owner 身份取得的数据，不能未经明确授权就被解释为所有 Workspace 读者均可访问的数据。

至少区分按读取者权限访问远端数据，与由已授权身份导入后作为独立产物共享两种模式。源端撤权后是否仍允许访问已导入副本，由该模式和发布授权明确约定。

地址、查询和公开同步策略可以属于共享文档；凭据及其运行时绑定属于服务控制状态，不随普通文档或模板导出。

### 6.3 固定版本必须覆盖必要依赖

对外承诺固定内容的快照，应覆盖 Block 配置、数据、文件资源，以及影响结果的公式定义和输入。未闭合的动态依赖必须显式列出，不能因为根对象已经有 Content ID 就宣称全部内容固定。

Folder 原子切换也遵守同一规则：新版本准备完整后切换入口，读取者解析出确定的版本根，再读取其成员。连续访问一个始终指向最新状态的目录入口，不能替代多文件一致读取。

公式可通过冻结结果闭合依赖，也可在保留确定输入和求值语义的前提下承诺可复现。无法履行承诺的动态组件，应在创建固定演示或导出包时被拒绝、物化或明确降级。

## 七、资源准入、限额与维护通道

### 7.1 先明确计量口径

具体限额值可以延后，但以下资源的计量和承担者应先明确：

| 资源 | 需要确定的口径 |
| --- | --- |
| 当前结构化数据与资源 | Workspace 的逻辑占用，与底层物理去重分开核算 |
| 历史、备份、演示保留 | 是否计入 Workspace 配额，保留由谁申请和承担 |
| 暂存上传与派生产物 | 从哪个阶段预留容量，失败及过期后如何释放 |
| 计算缓存与运行文件 | 归属 Workspace、用户执行环境或服务缓存的哪一类资源 |
| 网络流量 | Source 拉取、上传、下载分别由谁承担 |
| 内核工作量 | 命令数、数据规模、扫描范围、引用数量等准入预算 |

建议按 Workspace 自身持有和保留的数据定义逻辑额度，底层内容去重作为部署层优化；确切计量细则仍需单独定义。不能因其他 Workspace 增删了同一物理对象，就无说明地改变当前 Workspace 的额度归属。

### 7.2 容量检查发生在接受之前

上传、同步、任务输出和业务提交都需要资源准入。容量预留必须防止并发操作各自检查通过、合并后超限，并在失败、取消或过期时可恢复地释放。

配额和物理磁盘故障是不同问题。逻辑额度尚有余量，不代表底层写入一定成功；提交仍需遵守原子性和持久化确认规则。

### 7.3 将“全局只读”改为受控写入限制

建议冻结以下表述：**触限后拒绝不符合配额准入规则的业务写入，同时保留受控的维护与恢复通道。**

维护通道用于解除保留、回收、取消任务、修复和必要的状态记录，并具有明确权限和有限资源预算。删除可能新增 tombstone 和历史，不能简单认定“删除不占空间”；恢复能力需要预留空间或其他明确机制。

只读体验可以作为前端呈现，但不能把所有服务控制操作都禁止。流量和计算触限也应分别给出可重试、暂缓或拒绝语义，避免用一个只读标志表达所有资源状态。

## 八、公式与受控求值

### 8.1 定义、输入与结果分别管理

公式需要绑定返回类型、定义版本、求值语义版本和明确的依赖。前后端可以采用不同语言或执行组件，但同一公式定义必须说明适用的求值器，不能由客户端自行选择不等价的解释方式。

固定版本读取应使用固定依赖集合。普通读取可以产生受控缓存、用量记录或运行记录，但不得隐式修改权威业务数据或推进其版本。显式刷新输入或物化结果，通过 Command 形成新的共享版本。

### 8.2 确定性、缓存与权限

可复用的确定性计算，应由公式定义、求值语义及完整输入决定结果。时间、随机数、网络内容等必须成为显式输入，或作为任务能力处理。

缓存身份与访问授权分别判断。只有计算结果确实依赖调用者身份或授权视图时，相关上下文才进入缓存键；命中缓存后仍需执行当前访问检查。

如果结果被独立发布，应按明确的发布授权和结果 ACL 访问。不能以“已经算过”作为绕过依赖权限的理由。固定内容也不意味着访问权限永久冻结。

后端求值必须有成本、超时和失败边界。普通 Get 不能成为无上限的脚本入口；超出同步读取预算的工作，应进入有显式生命周期的执行机制。

### 8.3 普通公式先禁止循环

建议首版普通公式拒绝循环依赖。读取上一版本属于显式的延迟依赖，需要额外定义初始值、推进条件和计算轮次。

例如“本轮 A 等于上一轮 A 加一”，必须说明什么事件推进一轮。读取次数、无关字段编辑或缓存失效，都不能未经定义地改变该值。是否支持这类时序计算可以后续决定，不作为通用消环方法预先引入。

## 九、统一任务入口与执行授权

### 9.1 Workspace 是任务发起端

Workspace 保存可见的任务定义，接收执行请求，管理数据访问和结果提交。容器创建、调度、进程托管与回收由执行平台负责。

统一流程为：

```text
提交执行意图与授权依据
          ↓
可靠保存逻辑运行身份及待派发状态
          ↓
执行系统派发与运行
          ↓
基于固定输入形成候选结果
          ↓
确认计划并带条件提交
```

执行意图及其生效条件必须可靠关联。若触发同时修改共享数据，应有原子记录或可恢复的衔接机制，避免数据已提交但任务永久丢失。队列、outbox 或其他实现方式可以后定。

历史重放、内容导入和 Fork 不重新派发历史任务。服务重启后的恢复，仅处理仍有效、具有当前执行资格的运行记录；恢复旧备份不能自动恢复旧授权并重做外部操作。

### 9.2 逻辑运行、执行尝试与提交分开标识

| 身份 | 作用 |
| --- | --- |
| 逻辑运行 ID | 同一次用户执行意图的稳定身份；重复投递沿用，显式重跑创建新的身份 |
| 执行尝试 ID | 区分同一逻辑运行的实际尝试，并识别失效执行者 |
| 结果提交身份与计划摘要 | 固定候选操作及前置条件，恢复提交结果并去重 |

Workspace 内的结果写回使用幂等协议，但不能据此承诺第三方调用也恰好发生一次。外部副作用的安全重试依赖下游幂等或结果核实能力；结果不明时应记录为待核实，不能盲目当作未发生再执行。

### 9.3 结果协议独立于执行位置

各执行器共同遵守“输入快照与配置版本 → 候选结果 → 确认的计划 → 条件 Apply”。输入或执行定义变化后，默认不得静默覆盖当前数据；可以明确选择冲突、保留旧版本结果或经重新确认后执行新的计划。

用户或策略确认的是确定计划。改变操作、输出目标或前置条件后，应形成新的计划身份。计算缓存和中间文件不自动变成共享业务结果。

### 9.4 委托绑定任务版本与能力范围

授权需要分别识别触发者、任务编辑者、执行身份、委托者和资源承担者。拥有执行权限不自动包含修改任务程序、调整授权范围或任意编辑数据的权限。

使用 Owner 提供的委托环境时，Workspace 操作必须受 Owner 当前权限、明确的委托范围和执行身份适用限制共同约束。专用 App ID 可以承载配置和审计，但身份名称本身不构成权限隔离。

委托应绑定确定的任务配置或程序版本，以及允许的输入范围、输出目标、操作和外部能力。修改程序或扩大能力边界后，旧授权不得自动覆盖新任务。允许哪些动态输入，应作为任务契约的一部分。

AICC、网络访问和计算资源分别授权与计量；Workspace 数据权限不替代这些能力。审计同时保留触发者、执行身份和授权依据。

### 9.5 取消和撤权的有效点

取消请求、执行器停止、结果提交完成是不同事实。失效的执行尝试不得提交新结果；提交端必须检查当前授权和运行有效性，不能只依赖杀死进程。

取消与提交按照服务端确定的顺序裁决。已经接受的结果不会因后续取消自动消失，应通过新的显式修改处理；已发生的外部副作用和已读取的数据，也不能由取消保证撤销。

### 9.6 用户任务必须显式绑定执行上下文

xllm 是可复用的执行组件。用户任务由哪个进程托管、使用哪个身份调用服务、消耗谁的资源，必须由任务入口和执行平台确定，不能从宿主进程已有的全局 runtime 隐式推导。

每次运行至少应明确以下内容，持久记录可以保存引用及版本，不应保存或导出明文凭据：

| 上下文 | 必须明确的内容 |
| --- | --- |
| 发起与执行身份 | 触发者、执行主体、所属用户，以及适用的 App、AppInstance 或 Agent 身份；Workspace ID 仅标识数据空间 |
| 执行位置与隔离 | 本地、用户后台或 Agent；工作目录、文件访问、网络能力、进程与子进程限制 |
| 资源归属 | AI 用量承担者、计算资源承担者、各自预算和拒绝规则；两者不必是同一主体 |
| 委托与结果权限 | 绑定的任务版本、输入与输出范围、凭据或能力来源、有效期，以及取消和撤权规则 |

本地模式使用当前用户明确授权的执行环境；托管执行模式由 AppLoader 启动的 Agent Work Session 承载，绑定具体 Agent 及其授权，使用 Owner 的 Agent 与资源时还需满足 §9.4 的委托要求（D22）。不能只给任务填写一个 userid 或 appid 就视为已经建立隔离，也不能把所有任务默认归到 Workspace 服务名下。

Workspace 服务可以作为经过授权的服务调用中介。例如，以服务 session 向 AICC 提交经过校验的 `on_behalf_of`，可以明确 AI 资源归属；这不意味着任务程序已在对应用户的执行环境中运行，也不自动授予文件、网络或结果写回权限。服务认证身份、有效资源归属和实际执行环境必须分别审计。

用户任务缺少所需执行身份、资源归属或有效授权时应明确失败，不得自动退回系统服务身份、设备身份或其他用户的资源。这个要求应落实到任务入口与执行器接入契约；xllm 对合法系统任务的支持可以保留，但不能成为用户任务的隐式兜底。

迁移应保留现有 xllm 循环、输入快照、候选计划与条件 Apply；将任务执行接入用户执行环境，并显式注入获准的模型和工具能力。仅将 xllm 改成子进程，或仅增加 AICC 计量字段，都不足以完成运行模型调整。

## 十、固定版本演示

创建固定演示时，应取得共享文档的一致版本，并完成该演示承诺范围内的依赖闭合和资源保留。原 Workspace 随后继续编辑，不改变该基线。

演示期间允许交互的 Block，应把修改写入演示自己的分支或状态，不能隐式改动原 Workspace。若支持将演示结果带回原文档，应作为新的、可审查的条件提交处理。

演示事件记录可以引用固定基线以支持回放。回放所需内容的保留期与事件保留期需要协调；只保存事件而回收必要资源，不构成完整回放能力。

演示快照的访问授权独立定义。创建者能读到的内容不自动授予所有观众，普通读取权限也不必等价于导出或 Fork 权限。快照不得包含服务凭据或无关用户私有状态。

固定内容不冻结 ACL。原 Workspace 撤权、删除或停止共享后，已有演示是否继续可访问，必须有明确的授权政策；在该政策确定前，不能将固定版本解释成永久访问许可。

## 十一、冻结条款与验收场景

### 11.1 建议纳入冻结清单的条款

| 编号 | 冻结契约 | 可以延后的机制 |
| --- | --- | --- |
| K01 | 稳定对象身份与名称、内容身份分离；子项寻址不因排序或改名漂移 | 路径语法、展示名称和 URL 格式 |
| K02 | 原语的修改、冲突、约束与成本边界明确；业务扩展不隐含新内核语义 | 业务 Schema 目录和编辑器 |
| K03 | Data Source 与 Block Tree 共用逻辑事务和一致快照 | 物理存储拆分方式 |
| K04 | 新提交按当前权威状态准入；接受事实可幂等恢复 | 请求字段、队列和锁实现 |
| K05 | 已接受日志可确定性重放；内容快照与协作历史分别承诺 | 日志编码和压缩算法 |
| K06 | 引用、删除、恢复、补偿撤销与保留规则相互一致 | GC 算法及具体保留时长 |
| K07 | 固定版本覆盖其承诺范围的完整依赖；动态依赖显式报告 | 复制、pin 或 Fork 的实现选择 |
| K08 | Source 同步身份、副本授权和首次可用条件明确 | 同步频率及连接器实现 |
| K09 | 配额在准入时受控；触限保留有限维护能力 | 配额数值、计费和调度策略 |
| K10 | Get 不隐式推进权威内容版本；求值输入、权限和预算明确 | 公式语言、缓存部署、增量求值算法 |
| K11 | 任务意图可靠派发，重放不触发任务；运行投递与结果提交分别去重，外部副作用单独核实 | 消息投递和执行器组织方式 |
| K12 | 委托绑定任务版本与能力范围；提交端执行取消和撤权规则 | 票据格式、配置 UI、跨 Zone 协议 |
| K13 | 演示基线固定、交互状态独立、资源保留与访问授权明确 | 演示事件编码及回放界面 |
| K14 | 同一协议版本语义不静默改变；历史截断和升级显式处理旧请求 | 迁移工具和支持版本范围 |
| K15 | 用户任务显式绑定执行身份、环境、资源和能力；缺失时拒绝，不隐式借用系统权限 | 执行上下文编码、执行器部署及具体隔离技术 |

这些条款是待完成的目标，不因写入清单而视为已经冻结。每条应有对应协议定义和可验证的失败行为。

### 11.2 验证组合行为

3D Scene 仍是有价值的类型组合案例，但冻结验证还应覆盖以下场景。此表列出目标验收，不表示现有测试全部覆盖或已经通过。

| 场景 | 必须观察到的结果 |
| --- | --- |
| 对象改名后旧名称被复用，离线命令迟到 | 修改仍指向原逻辑对象，或明确失败，不写入同名新对象 |
| 同时创建数据源、Block 和绑定，中途失败 | 全部生效或全部不生效，不留下半成品引用 |
| 两人分别修改 min 和 max；查询依赖期间新增记录 | 受保证的不变量不被合并破坏；所需依赖变化可被识别 |
| 提交成功但响应丢失，随后锁过期或权限变化 | 可按结果可见性规则恢复原接受事实，不重复执行 |
| 去重历史已过期后收到很旧的提交 | 明确拒绝不安全重试，不作为新请求执行 |
| 相同内容由完整备份或内容分享分别恢复 | 协作历史接续能力按协议区分，旧请求和旧增量不被混同 |
| Source 初次下载失败、刷新失败、刷新时达到配额 | 不发布无效初始版本；旧有效版本保留；预留可释放 |
| 演示期间原数据更新、资源替换或删除 | 访问授权仍有效时继续使用固定基线和被保留资源；授权失效时明确拒绝访问，不切换为其他内容 |
| 解除最后一份历史保留后回收资源 | 回收符合保留规则，过期版本与撤销能力给出明确状态 |
| Workspace 满额后执行回收、取消和恢复操作 | 维护路径仍受控可用，不依赖继续增长的普通业务写入 |
| 多人多次读取同一公式，或缓存失效 | 不因读取次数改变权威内容版本；缓存不绕过权限 |
| 任务意图保存后服务崩溃，或执行结果提交时断线 | 意图不永久丢失；结果不重复写入；外部未知副作用可辨识 |
| 已授权任务被修改、运行中撤权、旧尝试迟到 | 旧授权不扩大；失效执行者不能提交新的结果 |
| 用户任务缺失执行上下文或 AI 归属，宿主已有系统 runtime | 明确拒绝，不改用系统服务或其他用户的资源继续运行 |
| 服务代理调用 AICC，AI 费用由 Owner 承担 | 触发者、服务调用者、费用承担者和执行环境可分别追溯；工具能力和结果写回仍按任务授权检查 |
| 表单提交成功后清理草稿失败，再从另一设备打开 | 恢复原提交事实，不重复新增记录 |

## 十二、现有基础与实施顺序

### 12.1 可复用的实现入口

以下是当前源码中已经存在的机制及其用途，不代表本文提出的目标契约已全部实现。链接指向源码，具体行为仍应结合实现和测试核对。

| 能力 | 现有入口 | 对本轮改进的价值 |
| --- | --- | --- |
| 实体身份、字段类型、内置实体校验 | [model.rs](../../src/frame/aiworkspace/core/src/model.rs)、[value.rs](../../src/frame/aiworkspace/core/src/value.rs)、[types.rs](../../src/frame/aiworkspace/core/src/types.rs) | 明确基础值域、稳定身份与业务 Schema 的层次 |
| 候选规划与提交 | [plan.rs](../../src/frame/aiworkspace/core/src/plan.rs)、[workspace.rs](../../src/frame/aiworkspace/store/src/workspace.rs) | 复用版本条件、原子写入、接受记录与补偿提交 |
| 本地副本和富文本历史 | [replica.rs](../../src/frame/aiworkspace/core/src/replica.rs)、[collab.rs 测试](../../src/frame/aiworkspace/store/tests/collab.rs) | 区分内容重现、协作接续和旧请求处理 |
| 物化、导出与保留诊断 | [materialize.rs](../../src/frame/aiworkspace/core/src/materialize.rs)、[export.rs](../../src/frame/aiworkspace/store/src/export.rs) | 建立依赖闭合、快照与资源保留契约 |
| Source 能力和查询 | [urlsource.rs](../../src/frame/aiworkspace/store/src/urlsource.rs)、[reads.rs](../../src/frame/aiworkspace/store/src/reads.rs) | 区分远端查询能力、调用者上下文和本地物化保证 |
| 用户私有状态 | [workspace.rs](../../src/frame/aiworkspace/store/src/workspace.rs)、[userState.ts](../../src/frame/desktop/src/app/aiworkspace/state/userState.ts) | 复用私有 KV 与客户端恢复机制，补齐 Block 草稿生命周期 |
| Wish 输入与结果协议 | [snapshot.rs](../../src/frame/aiworkspace/store/src/wish/snapshot.rs)、[readset.rs](../../src/frame/aiworkspace/store/src/wish/readset.rs)、[plan.rs](../../src/frame/aiworkspace/store/src/wish/plan.rs)、[runs.rs](../../src/frame/aiworkspace/store/src/wish/runs.rs) | 将固定输入、候选计划、条件 Apply 与恢复能力提取为跨执行器契约 |
| 任务与执行环境 | [OpenDAN tasks.rs](../../src/frame/opendan/src/tasks.rs)、[AppLoader](../../src/kernel/node_daemon/src/app_loader.rs) | 复用运行平台，不在 Workspace 中重新建立容器调度体系 |
| 演示克隆与事件转发 | [store/show.rs](../../src/frame/aiworkspace/store/src/show.rs)、[server/show.rs](../../src/frame/aiworkspace/server/src/show.rs) | 为固定演示基线、独立运行状态和资源共享提供基础 |

现有运行记录不等于已经实现可靠派发协议，现有引用索引也不等于已经完成配额和 GC。后续实现需要逐条验证目标保证，不能仅凭存在相关模块就判定完成。

### 12.2 许愿格后端运行模型的复核与修正范围

以下依据 2026 年 10 月 22 日更新本文时的源码复核，描述正常 BuckyOS 服务启动路径，不代表已对部署环境完成运行验证。

**当前执行链。** [main.rs](../../src/frame/aiworkspace/server/src/main.rs) 以 `aiworkspace` 的 `KernelService` 身份初始化全局 runtime；[wish/mod.rs](../../src/frame/aiworkspace/server/src/wish/mod.rs) 的 `proc.start` 保存运行记录后，在同一服务中派生 Tokio 任务；[stages.rs](../../src/frame/aiworkspace/server/src/wish/stages.rs) 直接调用 `XllmTask::prepare`、`XllmRun::start` 和 `run.execute`。这里的 xllm 是进程内 Rust 组件，并未为每次用户任务创建独立的用户执行实例。

生成程序由 [runner.rs](../../src/frame/aiworkspace/server/src/wish/runner.rs) 启动 Deno 子进程执行。该路径设置了工作目录读权限、output 写权限和不限主机的 `--allow-net`，并清理环境变量，但未切换 OS 用户或建立独立用户容器。Deno 只约束生成程序：execute 阶段的 `.llm_context` 以 `shell=true` 给模型 `bash` 工具组（[wish/llm.rs](../../src/frame/aiworkspace/server/src/wish/llm.rs) `llm_context`、[stages.rs](../../src/frame/aiworkspace/server/src/wish/stages.rs) `execute`），该 shell 以服务进程的 OS 权限在 run 工作目录运行，不经 Deno。因此不能把 Deno 权限限制当作整个 xllm 运行的防护边界。

**AI 资源归属部分修正。** 当前 `run_attribution` 按 Workspace 配置选择 `workspace_owner`（默认）或 `current_user`，后者要求触发者是本 Zone 的 Account；归属随新 run 持久化，另存 `operator_id`。`llm.map` 经 [wish/llm.rs](../../src/frame/aiworkspace/server/src/wish/llm.rs) 正确传递 `on_behalf_of`。但主 xllm 循环存在缺陷：`.llm_context` 中的 `provider.on_behalf_of` 能被 [xllm.rs](../../src/frame/agent_tool/src/xllm.rs) 解析，而 `merge_config_layers` 只合并 `kind`、`session_token`、`base_url`、`api_key`、`headers`，`on_behalf_of` 被丢弃；`prepare` 从合并后的配置取 provider，所以 analyze/execute 的模型调用以 `system:aiworkspace` 自身计费。该结论来自源码阅读，未实跑验证；现有测试只检查 `.llm_context` 文件内容，不会发现此问题。合并缺陷是局部修复，列为 §12.3 的 T0-6 先行处理；宿主 bash 工具的问题随托管执行模式（M3）一起解决，不在服务内执行路径上打补丁。

> 2026-10-23 T0-6 已实施并在本机 Zone 验证：`merge_config_layers` 合并 `provider.on_behalf_of`（来源记为 `provider.on_behalf_of`，provider 类型切换时随其他专属字段一起丢弃）；服务测试的模型注入改为 `LlmClientFactory`，断言阶段循环与 `llm.map` 实际请求的 client 都带归属。DV 基线（旧二进制）同一 analyze 阶段 4 次 `llm.plan` 用量记录为 `user_id=ood1, caller=system:aiworkspace, mode=direct`；修复后为 `user_id=devtest, app_id=aiworkspace, operator_id=devtest, mode=workspace_owner, run_ref{workspace_id, entity_id, run_id}`。另外服务侧 `wish::llm::attribution` 对 BuckyOS provider 且无归属的 run 直接返回 `AI_ATTRIBUTION_UNAVAILABLE`，不再以服务自身资源调用 AICC；其他 provider 不带 `on_behalf_of`。

[AICC identity.rs](../../src/frame/aicc/src/service/identity.rs) 限定可代理的调用主体，检查 `obj://aicc/on_behalf_of` 权限和归属账户有效性，再使用该用户的资源归属。因此当前正常新建 Wish run 的身份应分层理解：

| 层次 | 当前实现 |
| --- | --- |
| AICC 服务调用身份 | 使用 aiworkspace 服务 session；记录的服务调用者为 `system:aiworkspace`，没有透传触发者 session |
| AICC 有效资源归属 | `llm.map`：`user_id` / `tenant_id` 为选定的 Owner 或当前用户，业务 `app_id` 为 `aiworkspace`，`operator_id` 记录触发者。主 xllm 循环：因上述合并缺陷，实际归属为服务自身 |
| Workspace 数据权限 | 输入读取和结果 Apply 保留调用者上下文；run 访问检查其所属 principal |
| xllm 与生成程序的执行宿主 | xllm 在 Workspace 服务进程内运行，Deno 由该服务启动；尚未在这条路径中接入用户执行环境 |

因此，不能把“服务使用自己的 session”本身判定为设计错误。已加入的受托 AICC 调用机制值得保留；它在 `llm.map` 上修正了模型资源归属，主循环的归属要随缺陷修复一起生效，而任务运行模型的调整尚未开始。

**仍需落实的边界。** Workspace 当前同时承担数据服务和用户计算宿主，执行环境归属、工具能力与任务版本绑定尚需按 §9.4、§9.6 明确并接入执行平台。AICC 的服务代理权限与账户有效性校验，不能替代逐任务委托检查。现有 `on_behalf_of` 在执行器接口中仍是可选值；当使用系统或设备身份代理用户调用 AICC 时，缺少有效归属应拒绝，不能依赖正常新 run 总会填写。直接使用用户、App 或 Agent 身份调用 AICC，以及使用其他 provider 的路径，应按各自明确的授权与资源归属规则校验，不强制携带 `on_behalf_of`；合法系统任务也继续采用自己的授权规则。

本次发现应作为运行模型的架构修正项处理。保留输入快照、readset、候选计划、条件 Apply 和已有 AICC 归属机制，将用户任务的 xllm 与生成程序执行接到有明确身份、资源和隔离边界的执行环境，而不是只在现有后台实现旁增加一个本地开关。

### 12.3 实施 TODO

> 2026-10-23 由第十三节 D01–D26 与 13.2 推导，替换原“建议实施顺序”。编号 `T<里程碑>-<序号>`；每项写明依据的决策、主要落点和验收方式。标注“再核对”的项对应决策中“由决定推导”的细则，实施前先核对再动手。

**总体约束**

- 开发阶段允许 breaking change：FORMAT、PROTOCOL、`schema.rs` 的 DDL 版本一律直接升级，旧库与旧包拒收，DV 用 `start.py --all` 全新安装；不写迁移或兼容层（D07 冻结点之前）。
- M1 结束时声明 FORMAT 1.0 与 PROTOCOL 1.0；此后按 D07 的冻结点规则演进。§13.6 末尾“必须一次进入格式”的 11 项全部落在 M1。
- 每项改动按 AGENTS.md 的联动要求同步检查：`buckyos-api` 的 `aiworkspace_client.rs`、`schemas/*.json` 与 `tests/schemas.rs`、WASM 产物（`wasm/build.sh`）、Desktop 前端、`fixtures/` 向量（`AIWS_REGEN=1`）、README 协议表、`aiworkspace_template_dev` 手册。
- 里程碑内部可并行；里程碑之间按依赖顺序（M0 → M1 → M2 → M3/M4 → M5），M3 与 M4 互不依赖。

**验证命令**（在 `buckyos/src`）：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1`；WASM 重建见第一期实施记录；Desktop e2e 在 `frame/desktop` 以 `playwright.aiworkspace.config.ts` 运行；DV 按 §12.2 的 kRPC 驱动法，在真实 Zone 上验收 §11.2 对应场景。

#### M0 契约落地（只改协议文件与局部缺陷，不改格式）

> 2026-10-23 M0 全部完成。落点：错误码 `core/src/error.rs` + `schemas/error-codes.json` + Desktop `api/session.ts` `errorCodeText`；方法分级 `server/src/lib.rs` `METHOD_LEVELS` + README 方法表 Level 列 + 契约测试 `server/tests/contract.rs`（`fixtures/protocol/stable-methods.json`，`AIWS_REGEN=1` 重录；`asset.import_object` 需 NDM，列为 DV 覆盖）；引用对象 `schemas/reference.schema.json` + `core/src/value.rs` `canonical_reference` / `reference_from_canonical`（`{epoch, seq}` 在 T1-12 前解析为 `VERSION_NOT_RETAINED`），`doc.resolve` 接受并回带 `canonical`；选择器目录写入 README（`table_view` / `table_query` 可由 `resolve_cell` 解析，`folder_member` 预留）；`proc.start` mock 幂等比较参数摘要，两条路径统一返回 `IDEMPOTENCY_MISMATCH` 顶层码；`table.add_option` 改为必须带 `expect`；校验器分类表与已知限制在 README Protocol 节。验证：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1` 全过；Desktop `pnpm check`、eslint 通过。

| 编号 | 内容 | 依据 | 落点 | 验收 |
| --- | --- | --- | --- | --- |
| T0-1 | **已实施（2026-10-23）** 现有 11 个类型的校验器分类表：原语结构约束 / 引用完整性 / 业务 Schema 字段约束，写入 README 协议节 | D01、D04 | `core/src/types.rs` 注释与 README | 表中每条校验器都能对应到 `types.rs` 的一个分支 |
| T0-2 | **已实施（2026-10-23）** `Code` 枚举与 `error-codes.json` 补齐预留码：`BASE_TOO_OLD` 启用说明、`QUOTA_EXCEEDED`（`retryable` 与 `data{needed_bytes, reclaimable_bytes}`）、`QUOTA_LIMIT`、`VERSION_NOT_RETAINED`、`RESOURCE_RECLAIMED`、`EXECUTION_CONTEXT_MISSING`、`DELEGATION_STALE`、`RUN_REVOKED` | D09 | `core/src/error.rs`、`schemas/error-codes.json` | `tests/schemas.rs` 通过；前端 `api/session.ts` 的错误映射补齐 |
| T0-3 | **已实施（2026-10-23）** kRPC 方法分级标注：README 方法表加“稳定 / 内部 / 实验”列；稳定级含 `ws.*`（含 `get_user_state` / `set_user_state`）、`doc.resolve/read/query/prepare/commit/get_submission/undo/get_changes/wait_changes`、`asset.*`、`lock.*`；`proc.*` 暂列实验，M3 结束升稳定 | D11 | README、`server/src/lib.rs` 方法表注释 | 稳定级每个方法有契约测试（请求/响应 fixture） |
| T0-4 | **已实施（2026-10-23）** 引用对象 JSON Schema：`{ zone?, workspace_id, entity_id, selector?, mode: live\|fixed, version?: {epoch, seq} \| {content_root, path?} }`；`doc.resolve` 接受该形状 | D10、D03 | `schemas/reference.schema.json`、`core/src/value.rs`、`store/src/reads.rs` | 现有三套版本词汇（`live_head/fixed_revision/published_channel`、`follow/fixed`、URL 源）映射到新形状的单元测试 |
| T0-5 | **已实施（2026-10-23，选“补进 resolve_cell”；`folder_member` 随 T1-2）** 冻结选择器目录：补 `folder_member`；`table_view` / `table_query` 从许愿输入声明中移除或补进 `resolve_cell`（二选一，建议移除，许愿改用 `table_members` + `table_field_values`） | D04 | `core/src/plan.rs` `resolve_cell`、`core/src/types.rs` 许愿输入校验、`store/src/wish/readset.rs` | readset 测试覆盖全部选择器 |
| T0-6 | **已实施（2026-10-23）** 修复 `merge_config_layers` 丢弃 `provider.on_behalf_of`；补一条断言实际 AICC 请求携带 `on_behalf_of` 的测试；服务侧无归属的 BuckyOS run 拒绝（`wish::llm::attribution`） | §12.2 | `agent_tool/src/xllm.rs`、`server/src/wish/{llm,stages,mod}.rs`、`server/tests/wish.rs` | DV 通过：AICC 用量记录中 run 的 `user_id` 为归属用户 `devtest` 而非 `ood1`/`system:aiworkspace`（§12.2 注） |
| T0-7 | **已实施（2026-10-23）** `proc.start` mock 路径幂等回放比较参数摘要；`expect` 可选性统一（`set_derived`、`tree.move`、`tree.place`、`add_option` 与 operations.json 一致） | 13.2、D04 | `store/src/proc.rs`、`core/src/plan.rs`、`schemas/operations.json` | 同 key 不同参数返回 `IDEMPOTENCY_MISMATCH` |
| T0-8 | **已实施（2026-10-23）** 协议文档写明已知限制：含 `richtext.apply_update` 的 Commit 不可撤销；import 模式容忍缺失资源；只读放映无固定基线 | 13.2、D17 | README | 文档核对 |

#### M1 格式 1.0：数据结构与日志

> 2026-10-23 M1 全部完成（FORMAT 1.0 / PROTOCOL 1.0 / DDL v4 已声明）。落点与决定：`model.rs` `supports_schema_version`/`opaque()`/`schema_id` 列（DDL v4）；`PlanEnv.schema_checks`（准入 true、replay false）把 `init_entity`/`validate_update` 的业务校验与原语形状（`MAX_PAYLOAD_BYTES`）分开；`REF_KINDS` 目录（`mode`/`integrity` 列由 kind 推导写入）；`object_link` 为弱引用；`owns` 由 planner 在提交末尾按“同一提交创建 + 位于 canvas-content + Block 未声明 `data_ownership: shared`”自动建边；级联删除 = “曾被 owns 且无存活 bind 的数据随最后一个 Block 删除”，展开写入规范化操作的 `cascade`；同一提交内已删目标的再次删除视为已满足；`data.pin`/`data.restore_owns`；`entity.clone` 作为复制语义的唯一实现（Desktop 同画布粘贴走它，跨画布引用，删除逻辑不再在前端算级联）；`$formula` 值：`FieldDef::normalize` 放行并校验形状，排序/过滤/unique 视为空；`Cap::Run=256`、`ALL=511`、`may_run()`，grants 增 `task_digest`/`outputs_json`，`proc.start` 需 run 或写权限，run 对持 run 者可见；user_state `revision` + `expect` 条件写 + `clear_block_state`；`base_seq`（恒 0）与 `check_base`，`retentions`（snapshot/export/clone/staged）与 `usage`（content/assets/exports/clones/staged）账本，`ws.get_info` 返回 `base_seq`/`planner_version`/`richtext_schema`/`usage`；`CommitOpts.as_principal` 让 run 结果以执行身份记幂等，origin 由 `derive_origin` 推导（Agent token → `agent`）。验证：三个 crate 全过，`store/tests/format1.rs`、`core/tests/phase2.rs` 新用例；契约 fixture 已重录。
> 其余项：T1-2/T1-3 `buckyos.dir`（`types.rs` `validate_dir`/`dir_refs`/`dir_payload_from_dir_object`，`plan.rs` `dir.set_member|remove_member|replace` 与 `folder_member` 选择器，`canonical.rs` `dir_object` 固定时间戳、`materialize.rs` `dir_object_of` 递归物化为 `cydir` 并写入 `content.dir_object`；`store/tests/kernel.rs` 与 ndn-lib `DirObject::gen_obj_id` 交叉验证；`member` 引用种类）；T1-6（`replica.rs` `changes_to_rows`/`rows_to_changes`/`state_digest`，`commit_deltas` 在 commits 行之后写入，`doc.get_changes` 按可读性过滤后带 `rows`，副本只应用 rows、无 rows 的 ops 事件报 `BASE_UNKNOWN`；`fixtures/vectors/apply-log.json` + `core/tests/apply_log.rs`；`PLANNER_VERSION`）；T1-12（`reads.rs` `resolve_retained`：同 epoch + `snapshots.seq` + `retentions(kind=snapshot)` 才解析，返回快照里该实体的内容对象）；T1-13（版本常量、测试与 Desktop 字面量、schemas、README；WASM 已重建；`aiworkspace_client.rs` 无版本常量无需改）。Desktop：`clipboard.ts` 同画布粘贴走 `entity.clone`，`CanvasView.tsx` 删除交给 planner 级联，`PermissionsPanel` 增 `run`，`client.ts`/`types.ts` 带 revisions/rows/`PROTOCOL_VERSION='1.0'`。验证：三 crate 全过，`pnpm check`/eslint/i18n 守卫通过；aiworkspace e2e 在后台跑（结果见 M2 进度注）。

| 编号 | 内容 | 依据 | 落点 | 验收 |
| --- | --- | --- | --- | --- |
| T1-1 | **已实施（2026-10-23）** 实体 `schema_version` 允许大于 1，并增加业务 Schema 标识字段；`degraded()` 对未知类型按不透明对象处理而非拒绝；`init_entity` / `validate_update` 拆为“原语校验”与“内置 Schema 校验”两个入口，后者仅在准入（非 replay）执行 | D01 | `core/src/model.rs`、`core/src/types.rs`、`core/src/plan.rs` | 副本重放含未知类型实体的日志成功；3D Scene 用 Table 实例表 + AssetRef 建模的测试不改原语、不升 FORMAT |
| T1-2 | **已实施（2026-10-23，命名为 `buckyos.dir` 资源目录）** 资源 Folder 原语（再核对 D25 命名）：成员名→AssetRef 或子 Folder 的映射，按成员名分版本格；操作 `folder.set_member` / `folder.remove_member` / `folder.replace`；成员数上限；物化为 DirObject 时时间字段固定；导入 DirObject 得到 Folder | D01、D25 | `core/src/types.rs`、`core/src/plan.rs`、`core/src/materialize.rs`、`core/src/canonical.rs` | 同成员两次物化 ObjId 相同；与 cyfs-ndn `DirObject::gen_obj_id` 交叉验证 |
| T1-3 | **已实施（2026-10-23：资源目录叫 `dir`，树容器保持 `folder`）** 树容器 `container(kind=folder)` 与资源 Folder 的命名区分（再核对：改名其一） | D25 | `core/src/types.rs`、前端 `app/aiworkspace`、i18n 表 | 两个名字在协议表、UI 文案中不再同名 |
| T1-4 | **已实施（2026-10-23）** 引用目录冻结：refs 表加 `mode`（live / fixed）与 `integrity`（strong / weak）列；新增 `owns` 种类；`object_link` 改为 weak；`anchor` 等列入目录；`blocks_delete` 改由 `integrity` 决定 | D05、§5.2 | `core/src/model.rs` `RefEdge`、`store/src/schema.rs`、`core/src/plan_richtext.rs`、`core/src/types.rs` | 删除被链接对象成功；删除被 bind 对象 `REFERENCE_BROKEN` |
| T1-5 | **已实施（2026-10-23，复制语义=planner 的 `entity.clone`）** 自有数据规则（再核对 D05 细则）：画布创建数据建 `owns`+`bind`；数据模式创建只建 `bind`；`entity.delete` 对 Block 由 planner 展开级联（目标曾被 `owns` 且无剩余 `bind` 则一并删除），展开结果写入规范化日志；新增 `data.pin`（删除 `owns`）；复制语义由共享库实现：同画布与“复制画布”克隆，跨画布引用，非自有声明总是引用 | D05 | `core/src/plan.rs`、`core/src/types.rs`（Block Schema 的非自有声明）、前端复制粘贴改走共享库 | §11.2“同时创建数据源、Block 和绑定中途失败”；删除/复制五种情形各一测试 |
| T1-6 | **已实施（2026-10-23）** 规范化日志改为纯应用型：`commit_ops` 保存完全解析的 cell 级写入与预计算的结构 hash；`replica.rs` 重放不再调用 `plan_commit`，改为 `apply_ops`；planner 版本号写入 `ws.get_info` | D20 | `core/src/plan.rs`（产出）、`core/src/replica.rs`、`store/src/docdb.rs` | 用旧版本 planner 生成的日志在新 planner 下重放一致（固定 fixtures 向量）；`AIWS_REGEN` 重生成向量 |
| T1-7 | **已实施（2026-10-23）** 公式值形状 `{ "$formula": { lang, version, expr } }` 作为带类型 key 的合法值；Schema 声明返回类型；v1 只保存不求值；`doc.read` 原样返回 | D18 | `core/src/value.rs`、`core/src/types.rs`、`schemas/` | 带公式的实体可提交、导出、重放 |
| T1-8 | **已实施（2026-10-23）** `run` 能力位（第 9 位）；`ALL` 常量、`write_policy` 导出、grants 校验同步；授权条目增加任务配置摘要与输出声明字段 | D16、D22 | `core/src/access.rs`、`store/src/workspace.rs` grants、`core/src/materialize.rs` | 仅 `run` 的主体不能 `doc.commit`，可 `proc.start` |
| T1-9 | **已实施（2026-10-23）** `user_state` 增加 `revision` 列与条件写；key 前缀 `block:<entity_id>/`；实体永久删除后服务端清理 | D13 | `store/src/schema.rs`、`store/src/workspace.rs` | 两客户端并发写同 key，后写者带旧 revision 得冲突 |
| T1-10 | **已实施（2026-10-23）** 历史下界 `base_seq`（恒为 0）进 `workspace_meta` 与 `ws.get_info`；低于下界的请求返回 `BASE_TOO_OLD`；保留根表 `retentions(kind, ref, holder, expires_at)` 替换 `snapshots.retained`；用量账本表 `usage(category, bytes)` 随事务更新 | D06、D19 | `store/src/schema.rs`、`store/src/workspace.rs`、`store/src/export.rs` | checkpoint / 导出 / 克隆 / staged 上传各写一条保留根；commit 后 usage 增量正确 |
| T1-11 | **已实施（2026-10-23）** 幂等按实际写入身份：run 结果提交以 `run.principal`（执行身份）查重，权限与可见性按调用者；origin 改服务端推导 | D08 | `store/src/workspace.rs` commit、`store/src/wish/runs.rs` | 两个用户 apply 同一 run 落到同一接受记录；客户端传入 origin 被忽略 |
| T1-12 | **已实施（2026-10-23）** 固定版本解析：`doc.resolve` 对 `{epoch, seq}` 查保留根，缺失返回 `VERSION_NOT_RETAINED`；建立 fixed 引用（`fixed_revision`、许愿 fixed 输入、演示基线）时必须同时登记 checkpoint 或克隆 | D03 | `store/src/reads.rs`、`store/src/export.rs`、`core/src/types.rs` | 删除 checkpoint 后解析该 seq 得明确错误 |
| T1-13 | **已实施（2026-10-23）** 版本常量升 FORMAT 1.0 / PROTOCOL 1.0 / DDL 版本 +1；`ws.get_info` 返回 PROTOCOL、FORMAT、planner、richtext Schema 版本矩阵；更新 README、fixtures、WASM、`aiworkspace_client.rs` | D07、D20 | `core/src/model.rs`、`store/src/schema.rs`、全部联动点 | 旧库 / 旧包 `UNSUPPORTED_VERSION`；全部测试通过 |

#### M2 内核闭环：保留、回收、配额、恢复

> 2026-10-23 M2 全部完成。落点：`store/src/service.rs` `Service::gc`（保留根 = 存活引用的 assets（`assets.ended_seq IS NULL`，由 `docdb.rs` 在每次提交维护）+ 保留期内的历史资产（`ended_seq` 对应 commit 的 `accepted_at` 距今 < `history_retention_days`，purge 置 0 立即可回收）+ `entity_versions(kind=generated)` + 有 `retentions` 行的快照 + 未过期 staged（retentions 与 local.staged_assets）+ `trash/` 30 天内；标记沿对象 JSON 内的 id 字符串传播；清扫跳过墙钟 1h 内的新对象；服务每小时跑一次，`diag.gc {dry_run}` 可手动），`mark_interrupted_runs`（启动扫描，`ACTIVE_STATES` 合一含 applying、`applying` 走幂等核对），`workspace.rs` `check_quota`（`QUOTA_EXCEEDED`/`QUOTA_LIMIT` + `data{needed_bytes, reclaimable_bytes, quota_bytes, used_bytes, owner}`，commit/上传/导出/克隆前检查，`CommitOpts.maintenance` 绕过）、`delete_checkpoint`/`clear_staged`/`purge_deleted`（内部操作 `entity.purge`，恢复报 `RESOURCE_RECLAIMED`）、`reclaimed_objects`（撤销与 `restore_version` 需要的对象已回收 → `RESOURCE_RECLAIMED`）、`show_source_grants`（克隆按源工作区当前 grants 判定）、事务回滚删除本次写入的结果对象、`set_base_seq`；`show.rs` 克隆上限 4 + 保留根与用量随 drop_clone 释放，放映锁“子树写权限也加锁”；核 `replica.rs` `epoch_mismatch`（活跃待发→conflict/EPOCH_MISMATCH，意图保留）+ WASM + Desktop `replicaSession.stop` 调用；设置 `AiWorkspaceSettings.quota_bytes`/`history_retention_days`；新 kRPC `ws.release_retention`/`ws.delete_checkpoint`/`ws.purge_deleted`/`ws.clear_staged`（stable，契约 fixture 已重录）、`diag.gc`。
> 2026-10-26 Named Bucket 迁移（`notepads/bucket-migration-todo.md` P1-1）取代了上面的对象存储与回收器：`FsObjectStore`、`retentions` 表、`Service::gc`/`diag.gc`、上传 staging 与本机路径下载已删除；对象在 zone 的 named store，保留由 `user/aiworkspace/<owner>/{assets,objects,exports}` 三个桶的绑定表达（快照与生成版本绑定一个列出闭包全部对象的保留集 DirObject），每小时的 `Service::sweep` 只解绑过期暂存、过保留期的历史资产与满 30 天的 trash。保留语义（D06）与配额（D19）不变。见 [BuckyOS AiWorkspace Durable Data Schema](<BuckyOS AiWorkspace Durable Data Schema.md>)。
>
> §11.2 场景 → 测试：对象改名后旧名称复用/离线命令迟到 → `store/tests/replica.rs::unknown_results_revoked_permission_and_epoch_change`；同时创建数据源/Block/绑定中途失败 → `store/tests/kernel.rs::v10_mixed_batch_atomic_with_fault_injection` + `core/tests/phase2.rs::own_data_follows_its_block_and_shared_data_never_does`；两人分别改 min/max、依赖期间新增记录 → `core/tests/planner.rs`（unique）+ `core/tests/phase2.rs::every_selector_of_the_frozen_catalogue_resolves_to_a_version_cell`；提交成功响应丢失、锁过期/权限变化 → `store/tests/kernel.rs::v12_idempotency_and_unknown_result` + `server/tests/process.rs`；去重历史过期后很旧的提交 → `store/tests/m2.rs::the_history_floor_refuses_what_lies_below_it`；备份与分享分别恢复 → `store/tests/packages.rs`、`store/tests/replica.rs`（epoch）；Source 三场景 → M4；演示期间原数据更新/授权失效 → `store/tests/m2.rs::a_show_clone_follows_the_source_workspaces_current_grants` + `store/tests/show.rs`；解除最后保留后回收 → `store/tests/m2.rs::the_collector_keeps_what_a_root_reaches_and_reclaims_the_rest`、`purge_reclaims_at_once_and_closes_undo`；满额后维护操作 → `store/tests/m2.rs::quota_admission_and_the_maintenance_channel`；多人多次读取公式 → `core/tests/phase2.rs::opaque_entities_and_formulas_are_kept_verbatim_and_schema_checks_run_only_at_admission`（读取不推进版本）；任务意图保存后崩溃/结果提交断线 → `store/tests/m2.rs::a_restart_marks_owned_runs_interrupted_before_anyone_asks` + `store/tests/wish.rs::an_accepted_application_is_reconciled_after_a_crash` + `server/tests/wish.rs::restart_reports_interrupted_and_reconciles_applying`；授权任务被修改/撤权/旧尝试迟到、缺执行上下文 → M3；AICC 归属 → T0-6 DV；表单草稿 → M5。

| 编号 | 内容 | 依据 | 落点 | 验收 |
| --- | --- | --- | --- | --- |
| T2-1 | **已实施（2026-10-23）** 服务级标记清除 GC：保留根 = 各 Workspace 当前状态 + `retentions` 表 + 未过期 `staged_assets` + 运行中 run 输入；`trash/` 纳入保留期；`diag.list_unretained` 改为按保留根计算 | D06 | `store/src/service.rs`、`store/src/objects.rs`、`store/src/export.rs` | §11.2“解除最后一份历史保留后回收资源”；并发上传期间 GC 不误删 |
| T2-2 | **已实施（2026-10-23）** 历史资源保留期：被替换或删除的 AssetRef 目标到期可回收；回收后相关撤销与版本恢复返回 `RESOURCE_RECLAIMED` | D06 | `store/src/workspace.rs` undo、`store/src/reads.rs` restore_version | 对应测试 |
| T2-3 | **已实施（2026-10-23）** 配额准入：commit、上传、克隆、checkpoint 前检查 `usage` + 预留；超额 `QUOTA_EXCEEDED` / `QUOTA_LIMIT`；结构化历史 v1 不计 | D19 | `store/src/workspace.rs`、`server/src/lib.rs` 上传 | §11.2“Source 刷新时达到配额”“满额后维护操作” |
| T2-4 | **已实施（2026-10-23）** 维护通道方法（manage）：`ws.release_retention`、`ws.delete_checkpoint`、`ws.purge_deleted`（立即回收并放弃撤销）、`proc.cancel`、`ws.clear_staged`；超额时仍可调用，有独立预算 | D19、§7.3 | `server/src/lib.rs`、`store/src/workspace.rs` | 满额 Workspace 走维护方法后可恢复写入 |
| T2-5 | **已实施（2026-10-23）** 服务启动扫描：活动 run 标 interrupted；两处 `ACTIVE_STATES` 合一；事务回滚后清理已写入对象库的生成结果对象 | D15、13.2 | `store/src/wish/runs.rs`、`server/src/wish/mod.rs`、`store/src/workspace.rs` | 重启后 `proc.list` 立即正确 |
| T2-6 | **已实施（2026-10-23）** epoch 变更：事件限定为恢复导入与截断重建；副本收到 `EPOCH_MISMATCH` 后保留待发意图，以冲突解决形式让用户重发；恢复导入在 UI 标为 manage 级破坏性操作 | D07 | `core/src/replica.rs`、前端 `offline/replicaSession.ts`、`store/src/service.rs` | §11.2“对象改名后旧名称被复用，离线命令迟到” |
| T2-7 | **已实施（2026-10-23）** 演示 ACL 查原 Workspace 当前 grants；放映锁判定改“有任何写权限即加锁”；克隆计 Workspace 额度，设并发上限与 TTL | D17 | `server/src/show.rs`、`store/src/show.rs` | §11.2“演示期间原数据更新 / 授权失效” |
| T2-8 | **已实施（2026-10-23，映射见下注；M3/M4/M5 的场景随其里程碑）** §11.2 场景逐条落为 store / server 测试，未覆盖项在验收报告中列明 | §11.2 | `store/tests/`、`server/tests/` | 每条场景对应测试名 |

#### M3 执行模型：本地与托管执行

> 2026-10-23 T3-1～T3-8 已实施（T3-9 按其条件等托管 DV）。**执行上下文**（T3-1）：`server/src/wish/exec.rs` `execution_context` 写入 `detail.context`（trigger / executor / compute / aicc / task.digest（`wish::basis_digest(payload)[..32]`）/ outputs / expires_at），缺项即 `EXECUTION_CONTEXT_MISSING`；`proc.start` 的 `params.execution.mode ∈ service|local|hosted`，缺省取 `WishConfig.default_execution`（buckyos-api `AiWorkspaceWishSettings.default_execution`，standalone `--wish-config` 同名键），`allow_service_execution=false` 时 service 模式被拒；hosted 模式只需 `run` 能力，其余需 `update`。**执行器协议**（T3-2/T3-3）：`proc.claim`（claim token `ct_`，每次领取一个 attempt + run token `rt_`，`store/src/wish/runs.rs` `wish_run_claim`，旧 attempt 的 token 保留以回 `RUN_REVOKED`）、`proc.heartbeat`（租约 `run_lease_ms()`，默认 90 s，`AIWS_RUN_LEASE_MS` 供测试缩短；`proc.get` 读到过期即 `interrupted`）、`proc.report`（`error` / `progress` / `finished` / Mock 形状的 `results` / `analysis`）；run token 只能调 `RUN_TOKEN_METHODS`；结果写入以 run 的 principal 提交（`CommitOpts.as_principal`），输出声明核对 `check_output_declaration`；`proc.apply` 校验 `DELEGATION_STALE`（task.digest 变化）与 attempt 过期（`RUN_REVOKED`）。
> **本地执行器**（T3-4）：采用“阶段宿主”分工——服务在 `proc.claim` 时打开阶段（`stages.rs` `host_open`：快照、工作目录、WORKSPACE.md、提示词与工具目录，`WishRuntime.hosted` 按 run id 保存），把 `stage{kind: model|program, system, user, tools, model, iterations, source}` 交给执行器；执行器托管模型循环与程序运行，每次工具调用回服务 `proc.tool`（`ws_*`、`submit_analysis`、`put_result`、`check_results`、`finish`，以及程序的 `llm_map`——由服务以 run 归属调模型并保留缓存），程序由执行器自己跑后以 `proc.program{source, results, log, files}` 交回（服务用同一 `judge_program` 校验），快照由 `proc.files` 取得（`context/`、`request.json`、`catalog/`、`lib/`），最后 `proc.report{finished:true}` 让服务从阶段收集的内容出候选并出计划（`drive_host`）。取消、租约过期、新 attempt 领取都关闭阶段（`drop_hosted`）。Desktop：`app/aiworkspace/executor/`——`localExecutor.ts`（claim → 心跳（租约/3）→ 循环 → report；claim token 存 `localStorage['aiworkspace.claims']`，页面离开时 `pagehide` 静默停止，让 run 变 `interrupted` 后可续跑）、`chat.ts`（AICC：逻辑模型走 `helper.llm_chat`、含 `@` 的精确模型走 `chat.completions.create`，用户 session 调用；开发覆盖 `aiworkspace.dev.llm.baseUrl` 走 OpenAI 格式）、`programHost.ts` + `program.worker.ts`（Worker 内用服务同一份 `aiws/aiws.js` 跑程序，`Deno` 全局由内存快照模拟，无 token、无 kRPC；`aiws.llm.map` 经 `globalThis.__aiwsLlmMap` 桥回主线程）。`wasm/` 未增加帮助函数：快照由服务物化、计划摘要由服务出，浏览器无需重算。e2e `wish.spec.ts`“T3-4: the local executor”通过：分析→执行→应用→只重跑程序→模型无响应时离开页面→租约过期转 `interrupted`→同一浏览器续跑为 attempt 2（mock-llm 增 CORS 并按工具目录给 `run_program{source}`，端口 `AIWS_E2E_LLM_PORT`，`start-backend.mjs` 设 `AIWS_RUN_LEASE_MS=6000`，浏览器心跳=租约/3；仅被停止的本地 run 保留 claim token，只有报告完成才忘记）；服务测试 `server/tests/wish.rs::an_outside_executor_works_on_the_services_stage_through_proc_tool_and_proc_program`。
> **托管执行**（T3-5）：`exec.rs` `dispatch_hosted` 以 Owner 指定 Agent 建 Goal（`goals.create` + `goals.activate`，目标文中给出 `buckyos aiws run-claim …` 与手册名）；无 Zone runtime 时记为 `dispatch.status=unavailable` 并把 claim token 留在 run 上供有权者转交。真实 Zone DV 未做（需 Agent 在 Zone 内就绪）。**Agent 侧**（T3-6）：websdk `src/aiworkspace_client.ts`（`getAiWorkspaceClient()`）与 CLI `cli/modules/aiws.ts`（`info/read/query/run-get/run-claim/run-heartbeat/run-files/run-tool/run-program/run-report`，`cli/tests/aiws_test.ts`）；手册 `src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_run.md`（AGENTS.md 手册表已加）。**Owner 托管配置**（T3-7）：`ws.set_hosted_exec{enabled, agent_id, allowed_wishes, triggerers, limits}`（`manage`），摘要进 `hosted_exec.digest`；Desktop 权限面板 `HostedExecSetting`（`aiws-hosted-*`，`permissions.spec.ts`）。**许愿 UI**（T3-8）：执行位置选择 `aiws-wish-exec-mode`（按 `ws.get_info.wish_execution` 与 `hosted_exec.enabled` 列出，记在 `localStorage['aiworkspace.execution']`）、仅 `run` 权限者显示托管提示并直接走 hosted、`interrupted` 显示续跑（`aiws-wish-resume`，只有持 claim 的浏览器能续）、`DELEGATION_STALE`/`RUN_REVOKED` 文案在 `core.ts`。验证：三 crate 全过（契约 fixture 因 `ws.get_info.wish_execution` 重录）；Desktop `pnpm check`/eslint；websdk `deno check`/lint/`cli/tests/aiws_test.ts`。**真实 Zone DV（2026-10-23，本机 test.buckyos.io，新二进制）**：以 kRPC 驱动本地模式全程——`proc.start{execution.mode=local}` → `proc.claim`（返回 stage 与 7/10 个工具）→ `proc.tool ws_profile/submit_analysis` → `proc.report{finished}` → `waiting_confirmation` → `proc.apply`；执行阶段 `proc.files` 取到快照（含 `lib/`）、用 Deno 跑 `lib/run.js` 后 `proc.program` 校验通过、`put_result`/`finish`、候选 2 个结果、apply accepted（驱动脚本见 memory）。AICC 适配器形状已对真实 AICC 核实：`helper.llm_chat{logical_model, messages[{role, content:[{type:text}]}], tools[{type:function,name,description,args_json_schema}], requirements.tool_call}` 返回 `message.content[{type:tool_use,call_id,name,args}, {type:provider_state…}]` 与 `tool_calls[]`，回传工具结果用 `{role:tool, content:[{type:tool_result, call_id, content:[{type:text}]}]}`；assistant 消息原样回传（含 provider_state，`chat.ts` `raw`）。由此修正：本地模式的 run 归属固定为发起者（`run_attribution(.., force_current_user)`），因为模型调用出自发起者自己的 session（用量记为 `user_id=<发起者>, app=system:control-panel, mode=direct`，无 run_ref；服务侧 `llm_map` 仍带 run_ref）。未验证：托管模式真实 Zone 派发（需 Zone 内就绪的 Agent）。

| 编号 | 内容 | 依据 | 落点 | 验收 |
| --- | --- | --- | --- | --- |
| T3-1 | **已实施（2026-10-23）** 执行上下文结构进 run 记录：触发者、执行身份、执行位置（local / hosted）、AI 归属、计算归属、绑定的任务配置摘要、输出声明、有效期；缺失即 `EXECUTION_CONTEXT_MISSING` | §9.6、K15 | `core/src/wish.rs`、`store/src/wish/runs.rs`、`store/src/schema.rs` | 不带执行上下文的 `proc.start` 被拒 |
| T3-2 | **已实施（2026-10-23）** `proc.*` 补齐：`proc.heartbeat`（本地模式租约）、`proc.report`（执行器回写状态与候选结果）、`proc.claim`（执行器领取 run 并取得 run 作用域凭据）；interrupted 由心跳超时判定 | D15 | `server/src/wish/mod.rs`、`store/src/wish/runs.rs` | 关闭浏览器后 run 在租约到期转 interrupted；§11.2“意图保存后服务崩溃 / 提交时断线” |
| T3-3 | **已实施（2026-10-23）** 结果写权限 = 执行身份当前权限 ∩ 已确认输出声明；配置摘要变化返回 `DELEGATION_STALE`；取消或撤权后旧尝试提交 `RUN_REVOKED` | D16、D22、§9.5 | `store/src/workspace.rs` commit、`store/src/wish/plan.rs` | §11.2“已授权任务被修改 / 运行中撤权 / 旧尝试迟到” |
| T3-4 | **已实施（2026-10-23）** 本地模式执行器：分析与执行阶段的 xllm 循环以浏览器为宿主（经 AICC 以用户 session 调用）；生成程序在 Worker 中运行，经 postMessage 拿受限 SDK（读输入快照、写候选结果），不持 token；复用 snapshot / readset / plan / 条件 Apply | D14、D21 | 前端 `app/aiworkspace/api`、新 `executor/` 目录、`wasm/` 增加输入快照与计划摘要帮助函数 | 本地模式跑通现有许愿 e2e；Worker 内无 token |
| T3-5 | **已实施（2026-10-23，派发经 OpenDAN goals.create/activate，真实 Zone DV 未做）** 托管执行模式（再核对 D23 派发接口）：Workspace 请求 AppLoader 以 Owner 指定 Agent 开 Work Session，携带 run 引用与 run 作用域凭据；Agent 以自身身份读写 Workspace；`WishExecutor::agent-work-session` 启用 | D14、D22、D23 | `server/src/wish/`、`buckyos-api`（AppLoader 入口）、`lib_opendan` Work Session 创建、`store/src/wish/runs.rs` | DV：协作者仅 `run` 权限触发，结果由 Owner 的 Agent 写回，AICC 归属为 Owner |
| T3-6 | **已实施（2026-10-23）** Agent 侧 Workspace 能力：websdk `aiworkspace` 客户端模块；恢复 `aiworkspace_template_dev` 手册或新增 `aiworkspace_run` 手册（读输入、提交候选结果、`proc.report`） | D14、AGENTS.md cmd_skills 规则 | `buckyos-websdk`、`src/apps/jarvis_agent/agent/cmd_skills/` | Agent 按手册完成一次 run 并写回 |
| T3-7 | **已实施（2026-10-23）** Owner 托管配置管理面：允许的任务、可触发的协作者、Agent 选择、能力与额度上限；配置摘要进授权条目 | D22 | `server/src/lib.rs`（`ws.set_hosted_exec`）、Desktop 分享/设置面板 | 配置变化后旧授权失效需重新确认 |
| T3-8 | **已实施（2026-10-23）** 许愿 UI：执行模式选择（本地 / 托管）、interrupted 续跑、`DELEGATION_STALE` 与 `RUN_REVOKED` 提示；仅 `run` 权限者引导走托管 | D14、D22 | `app/aiworkspace/ui`、`WishService.ts` | e2e |
| T3-9 | **未做（按本行条件待托管执行 DV 通过后执行）** 下线服务内执行路径：删除 `server/src/wish/stages.rs` 的进程内 xllm 与 Deno runner、`wish-host` HTTP 路由；`proc.*` 升稳定级 | D14、D11 | `server/src/wish/`、README | 托管执行 DV 通过后执行；全部测试通过 |

#### M4 Source、发布与域名

> 2026-10-23 T4-1～T4-7、T4-9 已实施，T4-8 部分实施。**Source**（T4-1）：数据实体 payload 的 `source{kind: url|workspace, url?, zone?, workspace_id?, entity_id?, mode: live|fixed, version?{content_root}, query?, refresh?{interval_s}, auth_mode: none|hosted, name?}`（`core/src/types.rs` `check_source`，表/富文本/记录/资源/目录都接受）；有 Source 的实体内容只读——`plan_table::source`（数据写）、`plan_richtext::state`、`dir_target`、`entity_set_keys`（`SOURCE_FREE_KEYS` 以外的键）统一走 `require_source_writable` → `INVALID_OPERATION/SOURCE_READ_ONLY`，只有 `PlanEnv.origin == "source"`（`CommitOpts.as_source`，`derive_origin` 出 `source`）或内部/导入路径可写；outline 信封带 `source` 投影（`read.rs` `outline_extras`，Desktop `state/outline.ts` 同步投影，WASM 已重建）。**刷新 = 一次 run**：`store/src/source.rs` `source_refresh_start`（runs 表 `source.refresh@1`，需实体 `update`）→ 服务 `server/src/source.rs::refresh` 取内容 → `source_apply` 按 diff 出操作（表：按 field_id/record_id 的增删改与 unset；富文本：按 block id 的 insert/replace/delete，文档不会中途为空；资源：staged 后 `entity.set_keys`）→ 以 run principal、`as_source` 提交一次 Commit（`origin: source`、带 `run_id`）；失败则 run `failed`、旧内容原样（§11.2 初次失败/刷新失败）；配额由 commit 的 `check_quota` 把关（QUOTA_EXCEEDED 预留可释放）。**服务侧抓取**（T4-2/D24）：`server/src/source.rs` 唯一的服务内 Source 执行器——同服务的发布直接以 `public` 主体读；否则一次无凭据 HTTP GET（30 s、32 MiB 上限；JSON 数组/CSV→表（列序保持、`id` 列作 record_id 否则按行号）、Markdown/文本→富文本、图片→资源；发布 JSON 信封原样接入）；`auth_mode: hosted` 的 Source 不在服务侧跑；每分钟定时器每 5 分钟扫 `due_sources`（`refresh.interval_s`、`local_meta source_refreshed:<id>`）以 Owner 身份发起；流量计入 `local_meta traffic_bytes`，`AiWorkspaceSettings.traffic_quota_bytes`（`Limits.traffic_quota_bytes`）超限 `QUOTA_LIMIT`，`ws.get_info.traffic`。**URL 查询表**（T4-3）：保留，`doc.source_capabilities` 已是 internal 级，能力按 §6.1 由 adapter 声明；未删除。**按路径发布**（T4-4）：local.sqlite 新表 `publications(name PK, entity_id, current)`（改名留旧名作别名）与 `referrers`；`ws.publish{entity_id, name?}`（manage；授予 `public` 读、登记名；返回 `path`、`url`、引用对象）、`ws.unpublish`、`ws.list_publications`、`ws.list_referrers`；HTTP `GET /kapi/aiworkspace/pub/{ws}/{name}`（无会话，JSON 信封 `buckyos.ai-workspace/pub/1.0`；`?fixed=1` 带 `content_root`，`?fixed=<root>` 读固定版本；`Accept: text/html` 出预渲染页=T4-9）、`POST …/referrers`。**消费方**（T4-5）：Desktop 数据树新建类型“链接”（`ui/sources/sourceLinks.ts` 解析发布链接或普通 URL → 创建带 `source` 的实体 → 立即刷新）；详情页 `SourcePanel.tsx`（来源/刷新/发布/引用者）；编辑器对有 `source` 的实体只读（`useWriteAccess`）。跨 Zone：`source.zone` + `url`，服务抓 `…/-/<entity>?format=json` 并 POST 登记引用；代码已写、未实测。**同 Zone 直读**：`kind: url` 的 Source 若指向本服务的发布（`aiws-<id>.<zonehost>/<workspace>/<name>` 或 `<zonehost>/kapi/aiworkspace/pub/<ws>/<name>`），以及 `kind: workspace` 带的 `zone` 恰是本 Zone，服务都直接读发布方，不走 HTTP（`server/src/source.rs` `own_publication` / `own_zone`）。2026-10-23 DV 修复：此前 `public_base` 代入完整 `ws_` ID，发布链接主机名带下划线，网关回 403、前端也认不出本 Zone 链接而存成 `kind: url`，服务转而 HTTPS 抓自己并因 DV 自签 CA 报 `fetch failed`；修后在本机 Zone 上原工作区的引用表刷新成功（`server/tests/source.rs` `own_publication_link_reads_directly`）。**引用登记**（T4-6）：同服务由刷新时服务直接写发布方 `referrers`（登记消费方实体 ID），跨 Zone 由消费方服务 POST；弱登记，不阻止撤回，撤回后刷新 `failed`。**导出分离**（T4-7）：现有 `doc.export` 的 `share`（无 collab/commits）与 `personal_backup`（含 Loro 历史，UI 已标“含协作历史”）即满足；克隆只在放映用（M2）。**独立域名**（T4-8）：scheduler `system_config_agent.rs` 为 aiworkspace 登记 `aiws` 主机项，`rootfs/etc/boot_gateway.yaml` 把 `aiws-<id>.<zone>` 折到 `aiws`；服务 `Limits.public_base = https://aiws-{workspace_id}.<zonehost>`（Zone 下由 zone document 得到；standalone `--public-base`），代入的是去掉 `ws_` 前缀的工作区 ID（DNS 标签不含 `_`，网关只折叠 `aiws-[a-z0-9]+`）；子域上 `/`、`/<workspace_name>` 重定向到 `<zonehost>/workspace/<id>`，`/<workspace_name>/<name>` 先按发布名解析（JSON/HTML），否则按 Surface 名（`surface_by_name`，名/标题 slug）重定向到带 `?surface=` 的 Desktop 页。未做：Desktop SPA 本身仍在 Zone 主机服务（源隔离要 SPA 在子域运行并解决登录态传递——混合 SSO 的 cookie/launch-token 策略待定）；改名别名只对发布名生效（Surface 改名无别名）。**验证**：`server/tests/source.rs`（发布→HTTP JSON/HTML/fixed→另一工作区 live/fixed/富文本插入与刷新→只读→引用者→发布方改动随刷新→CSV/JSON/Markdown URL→失败保留旧版→撤回后 404 且刷新失败不动旧数据）；Desktop e2e `tests/aiworkspace/sources.spec.ts`（发布、插入链接、跟随刷新、只读、引用者）；三 crate 全过（契约 fixture 因 `ws.get_info.public_base/traffic` 重录）；`pnpm check`/eslint。真实 Zone（2026-10-23 本机 DV）：部署服务二进制后在新工作区验过 `ws.publish` / `ws.list_publications` 与无会话 `GET /kapi/aiworkspace/pub/<ws>/orders`（JSON、HTML 预渲染、`?fixed=1` 带 `content_root`、未知名字 404），`public_base` 为 `https://aiws-<id>.test.buckyos.io`；`aiws-*` 子域本身要重新部署 scheduler 与 `boot_gateway.yaml`，未做。
> 2026-10-28 T4-8 续：`aiws-*` 子域已在 demo.buckyos.io 生效（`/`、`/<名>`、`/<名>/<surface id>` 均 307 到 `<zonehost>/workspace/<id>[?surface=]`）。Desktop 分享链接（`ShareDialog` → `shellContext.shareLink` → `links.ts publicWorkspaceUrl`）改为 `ws.get_info.public_base` + `/<pub_name(工作区标题) || workspace>/<surface id>`：Surface 段用 id 而非名字 slug，因为服务端 `surface_by_name` 接受 id、且 Surface 改名没有别名；工作区段服务端忽略，只是可读性。`public_base` 为空（standalone、e2e）时回退为 Zone 主机页面。「新标签打开」仍走 Zone 主机，避免一次跳转。剩余（SPA 上子域、登录态传递、Surface 改名别名）列为独立 TODO `notepads/aiworkspace-subdomain-todo.md`。
> 2026-10-28（续）T4-8 全部实施：aiworkspace 服务 `server/src/pages.rs` 在子域出 Desktop 构建的 `aiws.html`（`--web` / exe 旁 `web/` / `../control-panel/web/`）与静态文件，`/<ws>/<name>?id=` 出页、无 `id` 发布优先否则 302 到规范地址、`block=` 透传；`entry.describe`/`ws.join_entry` 的 `path` 改为子域绝对 URL；工作区 slug 避开网关保留前缀。登录态：buckyos-api `SYSTEM_LOGIN_TARGETS` 加 `aiworkspace`、`is_system_delegation_target`；verify-hub `delegate_user_session` 允许 `System(aiworkspace)`；control-panel `parse_sso_redirect_url` 把 `aiws-<label>` 折到 `aiws`，`prepare_app_launch` 接受 `service_id`；scheduler 保留 `aiws` 名。Desktop：第二入口 `aiws.html`/`src/aiws.tsx` + `standalone/StandalonePage.tsx`（host 解工作区、`?id/&block`）、`links.ts` 重写（`workspacePageUrl`/`zoneOrigin`/`loginUrl`/`entryUrl` 指 Zone 根）、`launch.ts`（`openWorkspacePage` 经 `prepare_app_launch{service_id}`）、`/workspace` 只剩列表、`LoginPage` 对指向 `aiws-*` 的 `redirect_url` 自动接力、`EntryPage` 进入工作区走委派、SW 按主机选入口、vite 双入口 + `aiws-*` 主机 dev/preview 中间件、提示器链接固定 Zone 根；e2e 的 Zone 主机改为 `localhost`、后端 `--public-base http://aiws-{workspace_id}.localhost:<port>`。测试：`server/tests/pages.rs`、control-panel/verify-hub 新用例、Desktop e2e 相关 spec 改走子域。未做：真实 Zone DV（子域 cookie、`/sso_refresh` 续期、离线 SW）。

| 编号 | 内容 | 依据 | 落点 | 验收 |
| --- | --- | --- | --- | --- |
| T4-1 | **已实施（2026-10-23）** Source 作为数据实体属性：`source{kind, url, query, refresh, auth_mode}`；有 Source 的实体对 Block 只读；刷新 = 一次 run（origin `source`，带 `run_id`），候选版本校验后普通 Commit 写入，每 Source 各自原子 | D12 | `core/src/types.rs`、`core/src/plan.rs`、`store/src/wish/`（复用 run 协议） | §11.2“Source 初次下载失败 / 刷新失败 / 刷新达配额” |
| T4-2 | **已实施（2026-10-23）** 服务侧无凭据公开 URL 抓取：唯一的服务内 Source 执行器；计流量额度；有凭据的 Source 只能经托管执行刷新 | D24 | `store/src/urlsource.rs` 重构为 fetcher、`server/src/lib.rs` 定时器 | 定时刷新在无浏览器、无 Agent 时生效 |
| T4-3 | **已实施（2026-10-23，保留为内部级未物化查询入口）** 现有 URL 查询表：声明能力（§6.1）后归入“未物化查询入口”，移出稳定面；或在 v1 删除（再核对） | D12 | `store/src/urlsource.rs`、`doc.source_capabilities` | 协议表更新 |
| T4-4 | **已实施（2026-10-23）** 按路径发布：`ws.publish(entity_id)` 对数据源授予 `public` 读权限并登记发布项；发布链接 = 引用对象的 URL 投影（D21 形态）；`ws.unpublish` 后消费方得 unavailable | D26、D10 | `store/src/workspace.rs` grants、`server/src/lib.rs`、`store/src/schema.rs` 发布表 | 另一 Workspace 以 live / fixed 两种方式插入成功 |
| T4-5 | **已实施（2026-10-23，同服务 e2e；跨 Zone 路径未实测）** 消费方插入发布链接：解析为 Source（同 Zone 直接读；跨 Zone 走 T4-2）；fixed 解析为实体 Content ID 或 ContentRoot | D26、D12 | `store/src/reads.rs` resolve、T4-1 | 跨 Workspace 插入 e2e |
| T4-6 | **已实施（2026-10-23）** 反向引用登记：同服务内由跨 Workspace 引用索引提供 `ws.list_referrers`；跨 Zone 由消费方插入时登记订阅记录 | D26 | `store/src/service.rs`、`store/src/schema.rs` | 发布方能列出引用者；撤回不受引用者阻止 |
| T4-7 | **已实施（2026-10-23，现有导出模式即满足，见进度注）** 导出分离：分享 / 发布只走 ContentRoot；`personal_backup` 带 Loro 历史并在 UI 标明；克隆只服务演示 | D26、D03 | `store/src/export.rs`、Desktop 导出面板 | 分享包中不含 `collab/`、`commits` |
| T4-8 | **已实施（2026-10-23 子域路由、发布页；2026-10-28 工作区页面上子域：服务出 `aiws.html`、`/<工作区 slug>/<surface slug>?id=`、子域自持登录态经 `prepare_app_launch{service_id}` / `/login` 接力、Zone 根 `/workspace/<id>` 删除；真实 Zone DV 待做，见 `notepads/aiworkspace-subdomain-todo.md`）** 独立域名（再核对 SSO cookie 策略）：cyfs-gateway 子域路由 `aiws-<id>.<zonehost>`；路径 `/<workspace_name>/<surface_name>`；名称→ID 解析与改名别名；`/workspace/<id>` 路由迁移；Desktop 入口改链接 | D21、D10 | `cyfs-gateway` 配置生成（scheduler `gateway_config`）、`app/aiworkspace/links.ts`、`AIWorkspaceRoute.tsx` | 子域打开、登录态有效、改名后旧链接行为符合决定 |
| T4-9 | **已实施（2026-10-23）** 公开内容预渲染（SEO）：发布的 Surface 服务端出静态 HTML 骨架 | D21、D26 | `server/src/lib.rs` HTTP 路由 | 无脚本抓取可见标题与正文 |

#### M5 界面、手册与收尾

> 2026-10-23 M5 全部完成。**草稿协议**（T5-1）：`app/aiworkspace/state/drafts.ts`——草稿存私有状态 `block:<id>/draft:<key>`（`{text, at, pending_key?, epoch}`），提交前先把 `idempotency_key` 记进草稿（`submitDraft`），提交走 `store.submit({ idempotencyKey })`（`SubmitOptions.idempotencyKey` → `session.commit` 的 `CommitOptions.idempotencyKey`，直连与副本两条路径都接受），成功后清草稿；打开 Block 时 `recoverDraft`：有 `pending_key` 先 `doc.get_submission`（副本模式 `submissionStatus`），`accepted` 则只清草稿、不重发，`unknown` 则回填编辑器并在下次保存用同一 key 重发；便笺编辑器（`ui/blocks/editors.tsx` `NoteEditor`）接入。**进入编辑即取锁**（T5-2）：`LockScope` 在捕获阶段的第一次 pointer-down 上为 `lock_required` 实体申请写锁（无人持有且本人可写；锁条自身的按钮除外），`state/locks.ts` 的 `acquire` 合并并发申请；`LOCK_*` 拒绝沿用 `noteLost` → 编辑器处 `aiws-lock-lost` 提示、修改保留在本窗口、再次进入取回锁后发出。**公式**（T5-3）：`ui/formula.ts` 识别 `{ $formula: { lang: "aiws", version: "1", expr } }`，前端按同记录的字段名/字段 id 求值（四则、括号、数值字段）只显示不写回；单元格“物化”按钮提交 `table.set_values{ derived: { program: "formula", formula } }`，内核 `plan_table::apply_meta` 对带 `program: formula` 的显式 derived 记为 `derived{program, formula, by, run_id: null}`（人工物化的公式是可复现计算，来源记公式本身；WASM 同步重建）。**复制画布与 pin**（T5-4）：`ui/canvas/surfaceOps.ts::duplicateSurfaceOps` 复制 Surface 时克隆其内容文件夹下的自有数据（Block 改指克隆）、共享数据只引用；数据详情页对自有数据显示“属于画布”并提供 `data.pin`（`tree.move` 到 `data`）。**手册**（T5-5）：`src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md` 覆盖新建 / 导出导入 / 引用 / 运行 / 发布（示例按协议手写，未在 DV 实跑）；AGENTS.md 手册表去掉“暂缓”。**验收报告**（T5-6）：`BuckyOS AiWorkspace 内核架构改进 验收报告.md`。**验证**：Desktop e2e `tests/aiworkspace/m5.spec.ts`（T5-1～T5-4 四个用例）、`features.spec.ts` 写锁用例与 `offline.spec.ts` 离线锁用例回归；`pnpm check` / eslint；三 crate 测试全过。

| 编号 | 内容 | 依据 | 落点 | 验收 |
| --- | --- | --- | --- | --- |
| T5-1 | **已实施（2026-10-23）** Block 草稿协议：表单 Block 用 `block:<id>/` 私有状态，提交前记 `idempotency_key`，恢复时先 `doc.get_submission` | D13 | `app/aiworkspace/state/userState.ts`、表单类 Block | §11.2“表单提交成功后清理草稿失败，再从另一设备打开” |
| T5-2 | **已实施（2026-10-23）** Mesh 类对象“进入编辑即取锁”约定与 `LOCK_REQUIRED` 界面 | D02 | 前端对象编辑器 | e2e |
| T5-3 | **已实施（2026-10-23）** 公式只读显示：前端对 `$formula` 值求值显示，不写回；物化按钮生成 Commit 与 derived 记录 | D18 | 前端 Table / 对象编辑器 | `doc.read` 返回公式文本，UI 显示结果 |
| T5-4 | **已实施（2026-10-23）** 复制画布 = 同画布语义；`data.pin` 入口（拖入数据面板） | D05 | 前端 | e2e |
| T5-5 | **已实施（2026-10-23，手册示例未在 DV 实跑）** cmd_skills 与文档：`aiworkspace_template_dev` 恢复并覆盖新建 / 导出导入 / 引用 / 运行 / 发布；README 协议表与 `doc/workspace` 各期文档回写“已实施”标注 | AGENTS.md | `src/apps/jarvis_agent/agent/cmd_skills/`、README、本文第十二节 | 手册示例在 DV 可执行 |
| T5-6 | **已实施（2026-10-23）** 验收报告：按 §11.2 列出每条场景的测试名或 DV 证据，未覆盖项明示 | §11.2 | `doc/workspace/…验收报告.md` | 报告与测试名一一对应 |

**不在本 TODO 范围**（§13.7）：当前用户 Agent 的托管执行、后端公式求值、公开演示房间持久化与回放、函数对象、配额数值与计费、Agent 与 XL Agent 分类、Guest 自带 Endpoint 与跨 Zone AICC。

## 十三、冻结前必须确定的设计决策

本文的目标是让 AiWorkspace 的内核数据结构、接口和运行模型基本稳定。前文多处写明“必须明确”的事项，本节把它们收敛为具体决策：每项列出需要决定的问题、当前实现、可选方案、建议和确认结论。实施 TODO 按这些决策推导，并据此修订 §12.3 的实施顺序。

“当前实现”依据 2026 年 10 月 22 日的源码核对，只描述现有行为，不表示已确认为契约。各项“建议”已于 2026 年 10 月 22 日确认；与建议不同或有补充之处，写在该项的“决定”中。

### 13.1 决策总表

| 编号 | 决策 | 结论 | 关联 | 状态 |
| --- | --- | --- | --- | --- |
| D01 | 内核原语清单与业务类型的声明方式 | 冻结少量存储原语（Folder 与 DirObject 同构）；业务类型分层，业务规则只在准入时校验 | K02、K14 | 已确认：采纳建议，子问题见 D01 决定（2026-10-22） |
| D02 | 集合的协作粒度 | 独立编辑的元素用子实体或表记录；key 内的值整体替换；v1 不做 JSON Patch | K02 | 已确认：采纳建议（2026-10-22） |
| D03 | 固定版本的实现方式 | 固定版本取物化快照或克隆，在线库只服务当前状态 | K05、K07、K13 | 已确认：采纳建议（2026-10-22） |
| D04 | 内核约束集合与读集合词汇 | 只保证值域、字段约束和引用完整性；冻结现有选择器目录 | K02、K04 | 已确认：采纳建议（2026-10-22） |
| D05 | 引用种类目录与删除、复制语义 | 每种引用标明版本选择和完整性；共享数据不随 Block 删除，自有数据跟随 Block；同画布复制克隆、跨画布复制引用 | K06 | 已确认：按用户规则（见 D05 决定）（2026-10-22） |
| D06 | 历史边界、幂等窗口与对象回收 | 提交历史 v1 不截断但预留下界；历史引用的资源设保留期；服务级 GC | K06、K14 | 已确认：采纳建议（2026-10-22） |
| D07 | epoch 变更与格式升级策略 | 列举更换 epoch 的事件；设定格式冻结点，此后只做显式转换 | K14 | 已确认：采纳建议（2026-10-22） |
| D08 | Commit 请求、幂等作用域与 origin | 确认现有字段；幂等按实际写入身份；origin 由服务端推导 | K04、K11 | 已确认：采纳建议（2026-10-22） |
| D09 | 结果与错误码 | 三态结果与错误码枚举列入协议；预留新契约需要的错误码 | K04、K09 | 已确认：采纳建议（2026-10-22） |
| D10 | 规范引用对象与名称、URL 投影 | 冻结 JSON 引用对象；名称、路径和 URL 是它的投影 | K01 | 已确认：采纳建议（2026-10-22） |
| D11 | 对外稳定接口面 | kRPC 方法分为稳定、内部、实验三级 | K14 | 已确认：采纳建议（2026-10-22） |
| D12 | Source 模型 | Source 是数据实体的属性；刷新是一次任务并以普通 Commit 写入 | K08 | 已确认：采纳建议（2026-10-22） |
| D13 | 用户私有状态 | 保留 KV；增加 Block 命名约定、条件写和草稿恢复协议 | §2.2 | 已确认：采纳建议（2026-10-22） |
| D14 | 用户任务的执行平台与执行身份 | v1 做本地与托管执行模式；原“用户后台模式”已由 D23 并入托管执行 | K15 | 已确认：采纳建议（2026-10-22）；形态按 D23 收敛为两种 |
| D15 | 派发可靠性与运行记录归属 | 意图持久保存；崩溃后显示 interrupted，显式续跑，不自动重做 | K11 | 已确认：采纳建议（2026-10-22） |
| D16 | 执行授权与任务输出边界 | 新增 run 能力；结果写权限为执行身份权限与已确认输出声明的交集 | K12 | 已确认：采纳建议（2026-10-22） |
| D17 | 演示基线与访问策略 | 固定基线统一用克隆；第三期放映锁保留为非公开放映的产品行为 | K13 | 已确认：采纳建议（2026-10-22） |
| D18 | 公式 v1 边界 | v1 后端不做读时求值；共享结果经 Commit 物化，复用 derived 记录 | K10 | 已确认：采纳建议（2026-10-22） |
| D19 | 配额与维护通道 | 先定计量口径、超额结果形状和维护方法清单；数值与计费延后 | K09 | 已确认：采纳建议（2026-10-22） |
| D20 | 规范化日志是否独立于 planner 可应用 | 日志改为纯应用型；K05 验收改为“旧日志在新 planner 下重放一致” | K05、D01、D07 | 已确认：采纳建议（2026-10-23） |
| D21 | 本地模式生成程序的执行形态 | sandboxed iframe / Worker + postMessage SDK，不持有 token | D14、K15 | 已确认：Worker + postMessage；不用 iframe，每个 Workspace 独立域名，见 D21 决定（2026-10-23） |
| D22 | 任务结果写权限的来源 | 托管执行模式按执行身份；本地模式按确认者，由服务签发限定输出的 run token | D16、D14 | 已确认：v1 托管执行用 Owner 指定配置，后续支持当前用户的 Agent，见 D22 决定（2026-10-23） |
| D23 | “用户后台模式”是否并入托管执行模式 | 并入；默认执行者为该用户的 Jarvis | D14 | 已确认：并入，但改称“托管执行模式”（Run Agent Work Session 是 AppLoader 的一种形式），见 D23 决定（2026-10-23） |
| D24 | v1 自动同步的宿主 | 服务侧只做无凭据公开 URL 抓取，计流量额度；有凭据的 Source 走托管执行模式 | D12、D14、D19 | 已确认：采纳建议（2026-10-23） |
| D25 | 资源 Folder 的命名与 DirObject 时间戳规范化 | 资源 Folder 与树容器 folder 二者改名其一；物化时时间字段固定 | D01、D03 | 已确认：以 DirObject 为准，不够时两者并存，见 D25 决定（2026-10-23） |
| D26 | 内容快照与协作快照的导出格式分离 | 分享与发布只走 ContentRoot；整库克隆只服务同 ACL 的演示 | D03、D17、§4.4 | 已确认：重点是按路径发布与引用追踪，见 D26 决定（2026-10-23） |

决策之间的依赖：

```text
D01 原语/Schema 分层 ──┬── D04 约束集合（哪些校验进内核）
                       ├── D05 owns 引用（级联删除要进规范化日志）
                       └── D20 纯应用型日志（K05 确定性）
D03 固定版本身份 ───────┬── D06 保留根（checkpoint / 克隆要登记）
                       ├── D10 version 判别联合
                       └── D26 克隆与 ContentRoot 分家
D14 执行形态 ───────────┬── D22 权限来源（本地模式需确认者派）
                       ├── D15 心跳 / report 新方法
                       ├── D24 自动同步宿主
                       ├── D21 本地沙箱形态
                       └── D08 幂等 principal 改按 run
D06 ⇄ D19 历史计量与回收
```

D01、D02、D03、D20 决定格式；D14、D16、D22、D23 决定执行模型；其余相对独立。推导 TODO 时按此顺序展开。

### 13.2 可直接确认为契约的现有实现

以下行为已符合前文原则，确认为契约，不再单独设计。2026-10-23 按“确认意见”列确认：其中“幂等先于状态检查”按改写后的顺序确认，“重放不重新准入”以 D20 为前提确认；各列中的“补充”与“需修正”事项进入 TODO。

| 契约 | 现有实现 | 确认意见（2026-10-22 review） |
| --- | --- | --- |
| 共享文档同一事务（K03） | Data Source 与 Block Tree 同在 `doc.sqlite`；一次 Commit 在单个 SQLite 事务中写入数据、引用、历史、幂等记录和 head | 可确认。补充：生成结果的内容对象在事务内写入对象库，回滚会留孤儿，属 D06 GC 范围 |
| 三态结果 | 服务端结果为 accepted、conflict、rejected；超时或断线由客户端副本记为 unknown | 可确认 |
| 幂等先于状态检查（§4.2） | 按 `(principal, idempotency_key)` 查到接受记录即返回原结果，先于放映锁、规划内的逐操作权限检查和写锁；同 key 不同请求返回 `IDEMPOTENCY_MISMATCH`。epoch 检查在幂等查找之前，写锁检查在规划之后（见 §4.1） | 按改写后的顺序确认。另需修正 `proc.start` 对 mock 程序的幂等回放不比较参数（[proc.rs](../../src/frame/aiworkspace/store/src/proc.rs)），与“同身份不同请求必须拒绝”相违 |
| 重放不重新准入（§4.3） | 副本以已接受的规范化操作、原作者和原接受时间，经同一 planner 重放，不按当前 ACL 判断（[replica.rs](../../src/frame/aiworkspace/core/src/replica.rs)） | 有条件确认：跳过 ACL 属实，但仍经 planner，确定性只在同 planner 版本内成立，见 D20 |
| 撤销是补偿提交（§5.3） | `doc.undo` 生成补偿 Commit；`partial` 模式先返回可执行范围和 `plan_digest`，调用方凭摘要确认执行 | 可确认。已知限制应写进协议：`richtext.apply_update` 无逆操作，含富文本编辑的 Commit 不可撤销（`NOT_UNDOABLE`）；migrate 逆操作超 64 MiB 也不可撤销 |
| 服务控制状态不随导出传播 | 授权、分享入口、锁、run 和暂存上传位于 `local.sqlite`，不进入导出包 | 可确认 |
| 取消后拒绝提交（§9.5） | run 进入 cancelled 后，带该 `run_id` 的 Commit 返回 `RUN_CANCELLED` | 可确认。取消先 `kill -9` 再写状态，提交端检查才是真正的闸门，与 §9.5 一致 |
| 资源先可用再引用（§4.1） | AssetRef 只能引用已上传并校验的对象，媒体类型和大小取自内容本身 | 可确认。补充：import 模式容忍缺失资源（[types.rs](../../src/frame/aiworkspace/core/src/types.rs) AssetRef 校验），导入包的引用完整性弱于在线提交，属 §6.3“未闭合依赖显式列出”范围 |

### 13.3 数据结构决策

#### D01 内核原语清单与业务类型的声明方式

**需要决定**：内核冻结哪些存储与协作原语；业务类型以什么形式声明、由谁校验；业务规则是否参与重放。

**当前实现**：[types.rs](../../src/frame/aiworkspace/core/src/types.rs) 内置 table-source、richtext、record、asset、annotation、wish、block-def、viewport、show-path、cell、container 等类型。连接线端点、Viewport 缩放范围、演讲步骤目标等业务规则都是 Rust 校验器。副本用同一个 `plan_commit` 重放已接受的提交，所以这些校验也属于重放语义。新增业务能力已多次提升版本：rotation 与 locked 使 FORMAT 升到 0.3，演示实体升到 0.4，PROTOCOL 为 0.5，旧库和旧包一律拒收。

**可选方案**：

- A：维持现状，所有业务类型继续内置，每次新增按协议升级处理。
- B：冻结少量原语；业务类型用声明式 Schema 描述（值域、引用声明、大小限制），由内核解释；Schema 带版本并随 Workspace 保存。
- C：B 的过渡形式。现有内置类型保留，但分成“原语”和“内置业务 Schema”两层：业务校验只在准入时执行，重放只检查原语形状。

**建议**：先做 C，以 B 为目标。原语候选为树节点（父子关系与 `order_key`）、按顶层 key 划分版本格的对象、Table、RichText、AssetRef。以下子问题需要一并确定：

- Folder（资源集合，§6.3）是否作为第六种原语。
- placement 目前是树边属性，由 `tree.place` 自动合并；rotation 等几何字段的形状建议由 Block Schema 校验，避免几何字段变化引起格式升级。
- 自由表格（单元格各自定型）是 Table 的一种配置，还是另一种原语；若 v1 不支持，也应确认 Table 原语以后能否在不改变语义的前提下扩展。
- RichText 的节点与标记集合（`richtext.basic.v1`）按富文本 Schema 版本演进，每个实体记录所用版本，不与协议版本绑定。

验收以 3D Scene 为准：新增 Scene 不应修改原语，也不应提升 FORMAT。

**决定（2026-10-22）**：采用方案 C，以 B 为目标。子问题的结论如下：

- **Folder** 作为原语，结构与 cyfs-ndn 的 DirObject（`cydir`，[CYFS 标准对象](<../../../cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md>) §11）同构：成员以名字为 key，同一 Folder 内唯一、不保存顺序；成员是文件（FileObject）或子 Folder；可以带 meta。Folder 的固定版本就是对应 DirObject 的 ObjId：物化或导出时生成 DirObject，导入 DirObject 得到 Folder。
- **自由表格** v1 不做；以后作为 Table 的列类型或配置扩展，不另立原语。
- placement 与 RichText Schema 按上文建议处理。

以下细则由 Folder 的决定推导，实施前需再核对：

- 成员修改以名字为寻址单位，与“按顶层 key 划分版本格的对象”一致；需要配套切换时，一次操作把整个 Folder 替换为另一个 DirObject。两种操作都提供。
- 需要顺序的应用（例如按顺序播放一组资源），在业务 Schema 中另行表达顺序，不改变 Folder 的成员结构。
- 数据树中的 folder 容器是组织实体的树节点，不是本条的资源 Folder；两者是否统一，另行确定（review 认为不能统一，见 D25）。

**Review 补充（2026-10-22）**

方案 C 的真正收益要写进契约。“重放只检查原语形状”的价值不在少做校验，而在**旧副本能重放它不认识的业务类型**：Scene 上线后，未升级的离线客户端仍能同步。这要求三件事：

- 未知类型的实体在副本中按不透明对象保存。现有 planner 已保留未知类型（[plan.rs](../../src/frame/aiworkspace/core/src/plan.rs) 创建路径），可直接确认为契约。
- 每个实体记录自己的业务 Schema 标识与版本。现有 `schema_version` 固定为 1，其他值一律拒绝（[model.rs](../../src/frame/aiworkspace/core/src/model.rs) `degraded`）；需要像 `richtext.basic.v1` 一样随实体演进。
- 对现有 11 个类型的 Rust 校验器做分类表：哪些是字段约束（如 Viewport zoom 范围）、哪些是引用完整性（如连接线端点存在且同 Surface）、哪些是 Table/RichText 原语自身的结构约束（如 title 字段必须为 text、record body 独占）。只有后两类进重放。这张表应是 TODO 的第一步。

类型目前以字符串常量加 `matches!` 列表声明，没有注册表或 Schema 结构（[model.rs](../../src/frame/aiworkspace/core/src/model.rs) `is_known_type`，[types.rs](../../src/frame/aiworkspace/core/src/types.rs) `init_entity` / `validate_update` 的 `match`）。方案 C 的最小实现是把这个 `match` 拆成“原语校验”和“内置 Schema 校验”两个入口，不要求先做声明式 Schema。

重放确定性依赖 planner 版本，这是 K05 的短板而不只是 D01 的问题，见 D20。

Folder 作为原语有四处需要补定义（命名与时间戳两项归入 D25）：

1. **命名冲突。** 数据树已有 `container(kind=folder)`，有序（`order_key`）、成员是任意实体；资源 Folder 无序、成员只能是文件或子目录。两者语义不同，不能统一。
2. **ObjId 不是纯内容寻址。** DirObject 的 `gen_obj_id` 把 `name`、`create_time`、`last_update_time` 与统计字段一起哈希（CYFS 标准对象 §11）。相同成员两次物化会得到不同 ObjId，除非规定物化时时间字段取固定值。core 已有 `file_object` 帮助函数采用固定时间戳（[canonical.rs](../../src/frame/aiworkspace/core/src/canonical.rs)），DirObject 应沿用同一规则。
3. **成员类型。** 冻结思路 §3.3 希望 Folder 能装“文件、流或 URL”，DirObject 成员只能是 `cyfile` 或 `cydir`。建议 v1 禁止外部 URL 成员，Folder 只装内部 AssetRef 和子 Folder。
4. **按名寻址与 K01 的张力。** Folder 成员没有稳定 ID，重命名等于删加；一人改名、一人替换内容会得到一个孤儿和一个新成员。可以接受，但应写明 Folder 是**值型**容器（像 Table 的 cell，不像 Table 的 record），并为成员数设上限（例如 10k，超出用子 Folder）。

Folder 是唯一能廉价随时算出实体级 Content ID 的原语（成员名到子 ObjId 映射的 hash），§6.3 的依赖闭合检查可优先利用它。

#### D02 集合的协作粒度

**需要决定**：需要多人并行编辑的集合用什么表达；嵌套值是否支持局部修改；大 JSON 是否引入 JSON Patch。

**当前实现**：对象只有顶层 key 是独立版本格，嵌套值整体替换。show-path 的 `steps` 整个数组是一个版本格，冲突时前端读取最新值、重放步骤命令后再提交。没有按元素 ID 寻址的列表原语。

**可选方案**：

- A：不新增列表原语。独立编辑的元素做成子实体或表记录，key 内的数组和对象整体替换。
- B：新增按元素 ID 寻址的列表原语，支持插入、删除、移动和修改元素。
- C：为大 JSON 引入 JSON Patch。

**建议**：A，v1 不做 JSON Patch。Scene 按“文件夹 + 每个实例一个对象实体”建模；Mesh 用 AssetRef，或用加锁、整体更新的对象。show-path 的 `steps` 保持整体替换与前端重放。

**Review 补充（2026-10-22）**

- **Table record 就是“按元素 ID 寻址的列表”。** record_id 由客户端提供、永不复用、每个 cell 是独立版本格（[plan_table.rs](../../src/frame/aiworkspace/core/src/plan_table.rs)）。方案 A 应明说：需要按元素并行编辑的集合首选单表 Table，“子实体”是次选。Scene 的建模相应改为“一个实例表（position、rotation、mesh AssetRef 列）+ Mesh 对象实体集合”，而不是“文件夹 + 每实例一个实体”。后者会让一万个实例变成一万个实体、一万条树边和一万次 `tree_children` hash 重算。
- **Mesh 的“锁定编辑、整体更新”已有基础设施。** 实体级写锁已存在（`lock.acquire(entity_ids)`，60 秒租约、10 分钟空闲，[workspace.rs](../../src/frame/aiworkspace/store/src/workspace.rs)）。需要补的只是前端“进入编辑即取锁”的约定和 `LOCK_REQUIRED` 的界面表达，不新造机制。
- show-path steps 的前端重放属于 §3.3 第二类（应用约定），不在内核确定性契约内，写明即可。

#### D03 固定版本的实现方式

**需要决定**：固定版本以什么形式存在和读取；固定引用使用什么身份；单个实体是否有 Content ID。

**当前实现**：存储是状态型 SQLite，保存当前行、commits 历史和 `commit_ops`（含逆操作），没有按任意 seq 读取旧状态的接口。可寻址的旧版本是 `entity_versions(entity_id, content_rev, object_id)` 与物化出的 ContentRoot ObjectId。实体级 Content ID 已经存在：物化时按 `{ws_type, schema_version, content}` 为每个实体生成 jobj id，写入 `entity_versions`，`doc.resolve` 的 `fixed_revision.object_id` 可以解析（[materialize.rs](../../src/frame/aiworkspace/core/src/materialize.rs)、[reads.rs](../../src/frame/aiworkspace/store/src/reads.rs)）。缺的是“任意时刻可取”，不是“没有”。

**可选方案**：

- A：固定版本取物化快照（ContentRoot）或数据库克隆，在线库只服务当前状态。
- B：在线库支持按 seq 读取（MVCC），保留旧行。

**建议**：A。协议中的固定版本写作 `{ workspace_id, epoch, seq }`，建立固定引用时必须同时生成 checkpoint 或克隆，并按 D06 登记保留；需要跨 Zone 或长期保存时，解析为 ContentRoot ObjectId。实体级 Content ID 按 `(entity_id, content_rev)` 懒生成并缓存，与现状一致。§5.2 的 fixed 引用、§6.3 的依赖闭合、§8.1 的固定依赖读取和第十节的演示基线都按此实现。

**Review 补充（2026-10-22）**

- **`{epoch, seq}` 是身份，checkpoint 是可读物，二者会脱节。** 没有 MVCC 时，seq 100 可读的前提是恰好存在 seq 100 的 checkpoint 或克隆。协议应把两件事分开：固定引用的身份写 `{epoch, seq}`，建立引用时**必须**同时登记一个保留项（checkpoint、克隆或 ContentRoot ObjId）；`doc.resolve` 找不到保留项时返回 `VERSION_NOT_RETAINED`（D09 预留）。
- **预留“从最近 checkpoint 重放到任意 seq”。** `commit_ops` 已保存规范化操作，副本已能重放；这条路不改格式、成本有界（重放距离）。v1 不实现，但协议不要禁止按任意 seq 读取。
- **整库克隆不是内容快照。** `VACUUM INTO` 带 `richtext_updates`、Loro 完整历史、commits 和 `entity_versions`；个人备份也带 `collab/*.loro`。作为同 ACL 的演示基线（D17）没有问题；作为分享或发布的固定版本，就是把已删除的文字一起交出去。见 D26。

#### D04 内核约束集合与读集合词汇

**需要决定**：内核强制哪些声明式约束；读前置条件的选择器目录是否冻结。

**当前实现**：内核检查值域、字段类型与选项、引用目标存在且可读。字段级约束中 `unique` 是跨记录约束，提交时对全部活记录检查；`required` / `nullable` 收紧时也回扫全表（[plan_table.rs](../../src/frame/aiworkspace/core/src/plan_table.rs)）。另有若干固定不变量：title 字段必须为 text 且不可删除、record body 必须是同表富文本且不被其他记录使用、连接线端点须在同一 Surface、Viewport 只能挂自由布局 Surface（[types.rs](../../src/frame/aiworkspace/core/src/types.rs)）。没有用户可定义的跨字段或跨记录规则。读前置条件由 `resolve_cell`（[plan.rs](../../src/frame/aiworkspace/core/src/plan.rs)）解析，支持 entity、doc_key、table_members、table_field、table_field_type、table_field_values、table_record、table_cell、tree_children、tree_edge、richtext_block。其中 table_members、tree_children 能发现成员增删，table_field_values 能发现某列任一值的变化。许愿输入声明在用的 `table_view`、`table_query` 不在该目录中，`resolve_cell` 无法解析。

**建议**：v1 内核只保证单值约束、字段级约束（含 `unique`）和同一 Workspace 内的强引用完整性。跨字段、跨记录的用户自定义不变量（例如 min ≤ max）不由内核保证；需要系统级保证时，使用写锁或受信写入路径（§3.3 第三类）。现有选择器目录冻结为读集合词汇，新增选择器按协议变更处理。

**Review 补充（2026-10-22）**

- `unique` 必须留在冻结集合里，它已是内核强制的跨记录约束，去掉等于静默取消现有数据的保证。
- 现有固定不变量按 D01 分类：title 字段类型、record body 独占属于 Table 原语自身结构约束，留在内核；连接线、Viewport 属于业务 Schema 校验，只在准入时执行。
- 选择器目录冻结前要补 **Folder 成员**选择器（D01 新原语），并处理 `table_view` / `table_query`：要么补进目录，要么从许愿输入声明里移除。
- `expect` 的可选性不一致：`set_derived`、`tree.move`、`tree.place`、`add_option` 缺 `expect` 时不检查，而 operations.json 声称结构操作必须带 `expect`（`add_option` 被归为结构操作却用可选检查）。冻结前统一。

#### D05 引用种类目录与删除、复制语义

**需要决定**：每种引用的版本选择与完整性要求；删除 Block 是否连带删除数据；复制 Block 时何时共享数据、何时复制内容。

**当前实现**：

- 引用种类在代码中逐个定义。bind、embed、value、body、def 阻止删除目标；input、derived、produced、asset、anchor、connector_endpoint、show_target、show_surface 不阻止。富文本的 `object_link` 在索引时被归为 `embed`（`ref_set` 丢弃了 `DocRef.kind`，[plan_richtext.rs](../../src/frame/aiworkspace/core/src/plan_richtext.rs)），因此行内链接也阻止删除目标。引用表已有 `dst_workspace_id`、`dst_object_id`、`dst_query_json` 字段，但没有统一的 live/fixed 标记；版本词汇有三套并存：引用对象的 `live_head / fixed_revision / published_channel`、许愿输入的 `follow / fixed`、URL 源的 `live_head / fixed_revision`。
- 删除：画布删除 Block 时，如果它绑定的数据位于 `canvas-content`、且没有其他 Block 显示，前端会一并删除该数据（代码注释标为待产品确认）。这与 §5.1“默认删除 Block 只删除视图”不一致。
- 复制：画布复制一律“复制视图、共享数据”，不区分独立可编辑内容；冻结思路 §10.2 期望独立内容复制出新的数据源。

**建议**：把引用种类整理成冻结目录，每种标明 live 或 fixed、强依赖或可失效、是否参与保留与新鲜度计算；新增种类按协议变更处理。删除与复制按数据归属决定：位于 `canvas-content` 且只被该 Block 使用的数据，视为 Block 的“自有内容”，随 Block 删除和复制；数据树中的共享数据只删除视图、共享引用。该规则需要产品确认；确认后由内核或共享库实现，不只放在单个前端组件中。§5.1 已按下面的决定改写。

**决定（2026-10-22）**：删除与复制按用户的心智模型规定，引用种类目录按建议整理。

| 情形 | 规则 |
| --- | --- |
| 数据归属 | 在数据模式中插入的数据是共享数据，不属于任何 Block；随 Block 在画布上创建的数据是该 Block 的自有数据；Block 定义可以显式声明其绑定的数据为非自有 |
| 删除 Block | 共享数据无论怎样删除 Block 都不删除；自有数据跟随 Block 删除 |
| 同一画布内复制 | 默认克隆自有数据，新 Block 绑定新数据 |
| 跨画布复制 | 默认引用同一数据源 |
| 声明为非自有的 Block | 无论同画布还是跨画布，复制时总是复制引用 |

以下细则由上表推导，实施前需再核对：

- 绑定共享数据的 Block，复制时总是复制引用，与 Block 显式声明为非自有相同。
- 跨画布复制后，同一份自有数据被多个 Block 引用，直到最后一个引用它的 Block 被删除时才随之删除。现有前端的判断（没有其他 Block 显示时才删除）与此一致。
- 归属判定需要数据结构支持：数据由哪种入口创建，以及 Block 定义中的“非自有”声明（D01 的业务 Schema）。规则由内核或共享库实现，不只放在单个前端组件中。现有复制一律共享数据，需要改为上述规则。
- 跨 Workspace 粘贴不在本决定范围内。

**Review 补充（2026-10-22）**

建议用**引用种类**而不是实体标志表达“自有”：新增 `owns` 引用（Block → 数据），与 `bind` 并存。用户规则随之变成纯引用计数：

- 删除 Block：移除它的 `owns` 与 `bind`；目标数据若曾被 `owns` 且不再有任何 `bind`，随之删除。数据模式插入的数据没有 `owns`，永不级联。
- 同画布复制：克隆数据，新 Block 建新 `owns` + `bind`。跨画布复制：只建 `bind`。声明非自有的 Block：复制只建 `bind`。
- “直到最后一个引用它的 Block 被删除才删”自然成立；`bind` 本就是阻止删除的强引用，不会出现数据被删而 Block 还绑着。

两个产品边界需要补裁决：

1. **复制整张画布**（“复制页面”）按跨画布规则会让两张画布共享可编辑内容，用户通常期望克隆。建议“复制画布”按同画布语义处理。
2. **把自有数据升格为共享**（拖进数据面板）需要显式操作（例如 `data.pin`，即删除 `owns`）；反向操作是否允许另定。

级联删除必须由 planner 展开成显式操作写进规范化日志，重放时不能再算一次“还有没有其他 bind”，否则违反 §4.3；这与 D20 是同一件事。

富文本 `object_link` 应改为可失效的弱引用，否则用户删不掉任何被链接过的对象。

#### D06 历史边界、幂等窗口与对象回收

**需要决定**：v1 是否截断提交历史与协作历史；幂等记录保留多久；对象回收的保留根是什么，空间由谁承担。

**当前实现**：commits 表没有截断逻辑，幂等记录在同一 epoch 内永久保留（epoch 检查先于幂等查找，换 epoch 后旧 key 得到 `EPOCH_MISMATCH`）；富文本每次 `apply_update` 都是一条 commit，压缩只折叠 `richtext_updates`，不折叠 commits，并保留完整的 Loro 历史。对象库是服务级的内容寻址目录，多个 Workspace 和演示克隆共享对象，没有回收路径；`diag.list_unretained` 实际只列出过期且未被 `assets` 引用的暂存上传，不检查快照等保留根。`snapshots.retained` 列恒为 1 且无人读取。`trash/` 目录不做物理删除。错误码 `BASE_TOO_OLD` 已定义，服务端没有使用，但前端副本已处理它。

**建议**：

- 结构化提交历史 v1 不截断，但协议先预留“历史下界 seq”（由 `ws.get_info` 返回）；低于下界的请求返回 `BASE_TOO_OLD`。幂等窗口等于历史下界，§4.2 的拒绝行为由此表达。
- 只被历史引用的资源（已删除或被替换的 AssetRef 所指对象）设保留期，到期后可以回收；此后涉及该资源的撤销或版本恢复明确失败，并报告资源已回收（§5.4）。
- 对象回收采用服务级标记清除。保留根为各 Workspace 的当前状态、保留期内的历史资源、checkpoint 与导出包、演示克隆、运行中 run 的输入和未过期的暂存上传。

**Review 补充（2026-10-22）**

- **历史下界的真实用户含义是“离线副本太旧要重新 bootstrap”。** `BASE_TOO_OLD` 的消费者是前端副本（`session.ts`、`replicaSession.ts`），不只是防旧请求。v1 不截断对离线用户最友好，这应作为“不截断”的理由写入。
- **结构化历史是否计入 Workspace 占用**要在 D19 定。不截断时每次编辑一行 commits 永不清理；在没有截断手段前计入会让用户陷入无法恢复的超额。建议 v1 不计，等“截断历史后重建”（D07 事件）实现后再计。
- 保留根清单要补：`entity_versions.generated`（许愿结果版本）、`snapshots` 表（替换恒为 1 的 `retained` 列为真正的保留登记）、导出包、副本 bootstrap 包、`trash/`。现状这些都是回收候选的漏网者。
- **标记清除与并发上传的竞态**：对象先写入、稍后才在业务提交中被引用，中间窗口会被误回收。建议把 `staged_assets` 记录视为保留根直到 24 小时 TTL 到期，不另加 mtime 宽限。

#### D07 epoch 变更与格式升级策略

**需要决定**：哪些事件更换 epoch，换 epoch 时未提交的本地修改如何处理；从哪个版本起，格式变化必须提供显式转换，而不是直接拒收。

**当前实现**：以 restore 语义导入时生成新 epoch，旧 epoch 的请求返回 `EPOCH_MISMATCH`。前端副本此时停止同步并保留待发修改，但没有把它们提交到新 epoch 的流程。库或包的格式、协议版本不一致时，一律以 `UNSUPPORTED_VERSION` 拒收，不做迁移。

**建议**：更换 epoch 的事件限定为“恢复导入”和“截断历史后重建”两类，并写入协议。待发修改在新 epoch 下作为新提交重新准入：保留原请求，由用户确认后重发，不自动续接。格式方面设定一个冻结点，例如本节决策落地后的 FORMAT 1.0。冻结点之前，继续遵守当前开发阶段规则（breaking change，全新安装）；冻结点之后，同一 FORMAT 内只增加旧实现可以忽略的内容，不兼容的变化必须提供显式转换。

**Review 补充（2026-10-22）**

- 版本号实际有五个：PROTOCOL、FORMAT、实体业务 Schema 版本、richtext Schema 版本，以及隐含的 planner / 规范化版本（D20）。`ws.get_info` 应返回完整矩阵，并为副本定义每种不匹配时的行为（继续只读、拒绝、重新 bootstrap）。
- “保留原请求，用户确认后重发”在换 epoch 后不成立于字节层面：请求里的 `expect {rev}` 必然全部过期。副本实际能保留的是**待发操作的意图**，重发应以冲突解决的形式呈现，而不是重放 CommitRequest。
- 恢复导入换 epoch 会让**所有**用户的离线副本失效，应明确为 manage 级破坏性操作并在界面提示影响范围。
- FORMAT 1.0 冻结前必须一次进入格式的内容见 §13.6 末尾清单。

### 13.4 接口决策

#### D08 Commit 请求、幂等作用域与 origin

**需要决定**：现有请求字段是否确认为冻结形状；幂等记录按什么身份隔离；origin 能否由客户端声明。

**当前实现**：CommitRequest 的字段为 `protocol_version`、`workspace_id`、`epoch`、`idempotency_key`、`session_id`、`origin`、`run_id`、`undo_group`、`message`、`preconditions`、`operations`。幂等按 `(principal, idempotency_key)` 查重。许愿格结果的 key 为 `run/<run_id>/<plan_digest>`，由调用 `proc.apply` 的用户提交。origin（human、agent、program、import、system）由客户端填写，服务端只检查枚举值并原样记入历史。

**建议**：确认现有字段。幂等按“实际写入身份”隔离：普通编辑是用户本人，任务结果是该运行的执行身份（D14、D16）。同一逻辑运行的结果提交使用确定的 key，无论由谁触发应用，都落到同一条接受记录。origin 改由服务端根据 Caller 类别和 run 推导，满足 §9.4 的审计要求。

**Review 补充（2026-10-22）**

- 任务结果按执行身份隔离，意味着 `proc.apply` 幂等查找所用的 principal **不是调用者**。现有 `wish_apply` 要求调用者就是 run 的 principal 且 key 落在调用者名下（[runs.rs](../../src/frame/aiworkspace/store/src/wish/runs.rs)）。改动点：查找改按 `run.principal`；权限与结果可见性仍按调用者判断（§4.2）。
- `proc.start` 的 mock 路径幂等回放不比较参数，需要与许愿路径一样做参数摘要比对。
- origin 推导需要 Caller 类别可区分。建议规则：Agent principal → `agent`；带 `run_id` → `program`；import API → `import`；内部提交（undo、压缩）→ `system`；其余 → `human`。SDK 脚本以用户 token 调用时归 `human`，不另设类别。

#### D09 结果与错误码

**需要决定**：错误码是否属于协议；新契约需要哪些错误码。

**当前实现**：业务结果统一放在 kRPC 的 `result` 中。Commit 返回三态结果，其他方法返回 `{ ok, error: { code, retryable, detail, sub_code?, data? } }`；错误码由 core 的 `Code` 枚举定义，拒绝类错误另带 `AccessDenial`。

**建议**：`Code` 枚举、retryable 语义和冲突详情列入协议，随协议版本演进，调用方只按枚举判断错误类别。现在预留以下错误码：历史下界（`BASE_TOO_OLD`，D06）、配额不足（区分释放空间后可重试与需要提高额度，D19）、执行资格失效（run 已取消或授权已撤销）、执行上下文缺失（§9.6）。

**Review 补充（2026-10-22）**：还需预留 `VERSION_NOT_RETAINED`（D03，固定版本身份存在但保留项已不在）、`RESOURCE_RECLAIMED`（D06，保留期后撤销或恢复失败）、`EXECUTION_CONTEXT_MISSING`（§9.6）、`DELEGATION_STALE`（D16，任务配置摘要已变化需重新确认）。配额错误的 `data` 应带 `needed_bytes` 与 `reclaimable_bytes`，否则前端无法引导“释放后重试”。

#### D10 规范引用对象与名称、URL 投影

**需要决定**：其他 Workspace、App 和 Agent 长期保存的引用采用什么形状；名称、路径和 URL 怎样与它换算。

**当前实现**：`doc.resolve` 接受 reference 或 path。Workspace 内部的持久引用不写 `workspace_id`，以便 Fork 前后 ContentRoot 保持相等。独立页面路由为 `/workspace/<id>`。没有统一的跨 Workspace 引用格式。

**建议**：冻结一个 JSON 引用对象 `{ zone?, workspace_id, entity_id, selector?, version? }`，其中 `version` 为 D03 的固定版本身份；Workspace 内部引用省略 `zone` 和 `workspace_id`。路径、URL 和展示名称只是它的投影；名称解析后的重试使用引用对象，不再按名称重新解析（§3.1）。URL 语法可以延后，但必须能与引用对象无损互转。

**Review 补充（2026-10-22）**

- `version` 应是**判别联合**：`{ epoch, seq }`（同 Zone、依赖保留）与 `{ content_root, path? }`（内容寻址、跨 Zone）。D05 列出的三套版本词汇统一到这里。
- 跨 Workspace 引用没有引用表维护，不可能是强引用（§5.2 已排除分布式事务）。引用对象应能表达 `mode: live | fixed`，而**强弱由位置决定**：Workspace 内由引用种类目录定，跨 Workspace 一律可失效。写明可省掉实现期的争论。
- Fork 后内部引用省略 `workspace_id` 使 ContentRoot 相等，但指向被 Fork 者的**外部**引用不会跟随到新 Workspace。可接受，写明。

#### D11 对外稳定接口面

**需要决定**：哪些 kRPC 方法作为其他 App、Agent 和 SDK 可以依赖的稳定接口，哪些只供 Desktop 使用或仍属实验。

**当前实现**：所有方法都在 `/kapi/aiworkspace` 下，没有分级；`doc.outline`、`diag.*`、`show.*` 等与界面或诊断强相关。`aiworkspace_template_dev` 手册暂缓，外部调用方还没有接口契约。

**建议**：分为三级。

- 稳定：`ws.*` 基础方法，`doc.resolve`、`doc.read`、`doc.query`、`doc.prepare`、`doc.commit`、`doc.get_submission`、`doc.undo`、`doc.get_changes`、`doc.wait_changes`，以及资源上传；`proc.*` 在 D14、D16 确定后进入稳定级。
- 内部：`doc.outline`、`diag.*`、`replica.*`、`show.*`。
- 实验：尚未冻结的新方法。

稳定方法遵守 K14，变化随协议版本发布；内部方法可以随 Desktop 一起修改。

**Review 补充（2026-10-22）**：稳定级还应包含 `ws.get_user_state` / `ws.set_user_state`（第三方 Block 必用）、`lock.*`（D02 Mesh 编辑路径的一部分）、`asset.begin_upload` / `asset.finish_upload` 及对应 HTTP 上传路由。稳定级要有可执行的契约测试（`schemas/` 下的 fixtures 已有雏形），否则“稳定”只是声明。

#### D12 Source 模型

**需要决定**：Source 表达在哪里；物化刷新走什么路径；多个 Source 的刷新是否需要整体原子；默认采用哪种同步身份和副本授权模式（§6.2）。

**当前实现**：只有“URL 查询表”一种。表格的 `source_ref` 指向服务中注册的 adapter，读取时实时查询，结果不进入本地版本；没有物化同步，也没有通用 HTTP 抓取。生产启动路径没有注册任何 adapter（只有 `--fixture-sources` 开关下的 `fixture` 方案），所以当前任何查询都返回 `DEPENDENCY_UNAVAILABLE`；传给 adapter 的调用者上下文只有 principal。

**可选方案**：

- 表达位置：A，任意数据实体上的 Source 属性，该实体对 Block 只读；B，独立的同步实体，产出普通数据实体。
- 刷新路径：经统一任务入口（第九节）形成普通 Commit；或者由内核提供专用同步路径。

**建议**：表达位置选 A，符合冻结思路 §2.2“类型、来源、求值方式分别理解”。刷新作为统一任务入口下的一次 run：先拉取候选版本，校验后以普通 Commit 写入（origin 为 source，带 `run_id`），复用 §9.3 的结果协议、配额准入和取消规则。v1 每个 Source 的刷新各自原子，不提供跨 Source 的原子刷新。默认模式为“已授权身份导入，作为 Workspace 内容按 Workspace ACL 共享”，刷新者身份记入运行记录；“按读取者权限访问远端”只用于未物化的查询入口。现有 URL 查询表归入后者，并按 §6.1 显式声明其能力。

**Review 补充（2026-10-22）**

- **v1 没有自动同步的宿主。** D14 v1 只有浏览器本地和 Agent 两种执行器，定时刷新没有地方跑。三个选项：(a) v1 只有手动刷新（浏览器）和 Agent 刷新；(b) 为“无凭据的公开 URL 抓取”保留服务侧窄路径，视作 Source 基础设施而非用户计算——它不运行用户代码，只消耗 Workspace 的流量额度（D19 已有口径），归属清晰；(c) 等后台模式。推荐 (b)，有凭据的 Source 留给托管执行模式。见 D24。
- **Source 在实体上意味着整实体对 Block 只读**，挡掉最常见的用法：“导入一张表，再自己加一列备注”。v1 没有公式和 lookup，用户无法用第二张表关联。建议补一句：Table 原语以后允许**列组级** Source（同步列只读、手动列可写），现在的实体级 Source 是其特例，不另立原语。
- 现有 URL 查询表生产环境不可用，归入“未物化查询入口”后要决定是留在 v1 协议里还是先移出稳定面。

#### D13 用户私有状态

**需要决定**：私有状态的命名空间与 Block 的关系；同一用户多设备、多 Session 写入时的冲突处理；草稿与业务提交的恢复协议；Block 删除后的清理。

**当前实现**：`local.sqlite` 的 `user_state` 按 `(subject, key)` 保存，每个 Workspace 每个用户一份自由 KV，单项不超过 256 KiB，最后写入者覆盖；与 Block 没有关联，Block 删除后不清理。

**建议**：保留 KV 结构，增加三项约定：

- key 以 `block:<entity_id>/` 开头表示 Block 的私有状态；实体被永久删除、无法恢复后，由服务端清理。
- `set_user_state` 支持可选的条件写（按修订号或 `updated_at`），由前端决定覆盖还是提示冲突。
- 草稿在提交前记下将使用的 `idempotency_key`；恢复时先用 `doc.get_submission` 查询，已接受则清理草稿，未接受才用同一 key 重发（§2.2，§11.2 最后一个场景）。

**Review 补充（2026-10-22）**

- 条件写用**修订号**，不用 `updated_at`：多设备时钟不可比。
- `user_state` 在 `local.sqlite`，不随 Fork、克隆和恢复导入传播（restore-replace 只复制 grants、entries、local_meta）。写明“恢复导入后用户私有状态归零”是预期行为。
- 草稿恢复依赖 `doc.get_submission` 按 `(principal, idempotency_key)` 查询，与 D08 一致；若表单提交改走 run，principal 会变成执行身份，前端查不到。约定表单直提交不走 run。

### 13.5 运行模型决策

#### D14 用户任务的执行平台与执行身份

**需要决定**：本地、用户后台和 Agent 三种模式分别由哪个组件承载；执行身份使用哪种凭据；v1 实现哪些模式。

**当前实现**：许愿格在 aiworkspace 服务进程内执行，xllm 是进程内组件，生成的程序由该服务启动 Deno 子进程运行（§12.2）。浏览器侧没有本地许愿执行器，只剩 Mock 的 `HtmlExecutor` 在页面主上下文直接执行 HTML（无沙箱）；WASM 核只导出 id、规范化、`Replica` 引擎和许愿配置摘要，没有执行或规划函数。`WishExecutor` 枚举已有 `agent-work-session` 占位（[types.rs](../../src/frame/aiworkspace/core/src/types.rs)），服务端和 UI 都拒绝它。BuckyOS 中可以承载执行的现有组件有：OpenDAN（一个进程托管一个 Agent，Agent 有独立身份和 AICC 归属）；AppLoader 按用户拉起的 AppInstance；Node Executor（[node_exector.md](../arch/node_exector.md) v1.2，P0 已实现托管 xllm，并在单 OOD 环境验证）。Node Executor 的 D-1“v1 消费方”已定为托管 xllm，D-2 凭据已定为现有 App session token，且只接受 AppDID 等于 AgentDID 的严格一对一映射；仍开放的 D-6、D-9 至 D-14 都是恢复通道与设备配置事项，与 Workspace 作为消费方无关。

**可选方案**：

- 本地模式：浏览器执行，以当前用户的 session 调用 AICC 和 Workspace。
- 用户后台模式：A，每个用户一个 aiworkspace 执行 App 实例，由 AppLoader 拉起，使用该 AppInstance 身份；B，经 Node Executor 托管 xllm，使用绑定到该用户的 App token；C，继续在 Workspace 服务内执行，只用 `on_behalf_of` 区分 AI 资源归属（不满足 §9.6，仅作对照）。
- 托管执行模式（Run Agent Work Session，AppLoader 的一种形式，D23）：经 AppLoader 启动一个 Agent Work Session 执行任务，以该 Agent 身份读写 Workspace。v1 使用 Workspace Owner 指定的 Agent 与配置，后续支持当前用户自己的 Agent（D22）。原“用户后台模式”并入此形态。

**建议**：v1 先做本地模式和托管执行模式。本地模式恢复浏览器执行；托管执行模式复用 OpenDAN 已有的身份、AICC 归属和容器。用户后台模式在 Node Executor 的剩余待定项确定后，在 A、B 中选择。在此之前，现有的服务内执行只作过渡，不再扩展。此项需要用户决定。

**Review 补充（2026-10-22）**

**本地模式的安全形态要先定（D21）。** 恢复“AI 生成程序在浏览器执行”意味着模型写的 JS 带着用户 session 跑在应用源下，等价于自我注入 XSS。按“先能力后安全”可以作为已知风险上线，但形态应现在定：生成程序跑在 sandboxed iframe 或 Worker，只通过 postMessage 拿到受限的 Workspace SDK（读输入快照、写候选结果），不直接持有 token。这决定本地执行器的接口形状，后改成本高。本地模式的运行特征还包括：标签页关闭即中断、长任务不可能、需要心跳或租约协议（D15）。

**托管执行模式的三个缺口：**

1. **派发传输未定。** Workspace 怎样把“执行这个 run”交给 Agent：建议复用 msg-center（Agent 已有收消息→开 Work session 的路径），消息携带 run 引用与 run token，不另造通道。
2. **Agent 侧的 Workspace 工具。** `aiworkspace_template_dev` 手册暂缓，Agent 没有读写 Workspace 的 cmd_skill 和 websdk 客户端；托管执行模式以这两者为前提。
3. `agent-work-session` 占位可直接沿用。

**“用户后台模式”可并入托管执行模式（D23）。** Node Executor 已定用 App session token 且要求 AppDID 等于 AgentDID，所以方案 B 的执行身份就是一个 Agent；方案 A（每用户一个 aiworkspace 执行 App）本质是再造一个没有对话能力的 Agent。每个用户都有 Jarvis，“后台执行等于交给我的默认 Agent”是新增代码最少的路线；Owner 委托（冻结思路 §8.7）等于“Owner 的 Agent 替协作者执行”，D16 的委托规则直接套用。代价是没有 Agent 的用户只有本地模式，这与“每人一个 Agent”的产品方向一致。

**现有服务内执行的下线条件要写明。** 它是今天唯一能跑的许愿格路径，“不再扩展”但不能在本地或托管执行模式可用前拆掉。建议：托管执行模式 DV 通过后下线（§12.3 T3-9）；在此之前只修 §12.2 指出的 `on_behalf_of` 合并缺陷（T0-6）。

#### D15 派发可靠性与运行记录归属

**需要决定**：执行意图的可靠性等级；服务或执行器崩溃后的行为；执行器外置后，逻辑运行记录的权威位置。

**当前实现**：`proc.start` 先在 `local.sqlite` 保存 run，再在服务内启动任务。服务重启后没有主动扫描：`proc.get` 发现活动状态而内存中没有 worker 时才把 run 标为 interrupted，不自动重跑；处于 applying 的 run 通过结果提交的幂等记录核对是否已写入（有则 `succeeded`，无则回到 `waiting_confirmation`）。终态是 `succeeded`，没有 `applied`。“活动状态”列表在 `runs.rs` 与 `show.rs` 两处定义不一致（前者不含 `applying`）。

**可选方案**：

- A：至少一次自动派发，崩溃后自动重新派发（outbox）。
- B：意图持久保存；崩溃后显示为 interrupted，由用户或策略显式续跑，不自动重做。

**建议**：B。它满足“意图不永久丢失”，又不会自动重做外部副作用，与执行器子进程既定原则一致：resume 不核验，只按 runtime 返回“被打断”的结果。执行器外置后，逻辑运行记录仍以 Workspace 的 `local.sqlite` 为准；执行器只持有执行尝试，结果经条件 Commit 写回。触发执行同时修改共享数据时（§9.1），run 意图在 `local.sqlite`、数据在 `doc.sqlite`，两者之间需要定义恢复衔接（例如先记意图，Commit 携带 `run_id`，启动时按 `run_id` 核对），不引入通用 outbox。

**Review 补充（2026-10-22）**

- 不要靠 SQLite `ATTACH` 跨 `doc.sqlite` 与 `local.sqlite` 做原子事务：WAL 模式下跨库事务不保证原子。“先记意图、Commit 带 `run_id`、启动时核对”是正确做法。
- 服务启动应有一次主动扫描把活动 run 标为 interrupted，否则 `proc.list` 在用户打开前仍显示 running；两处活动状态列表统一为一处。
- 本地模式的 interrupted 判定依赖浏览器心跳超时，`proc.*` 需增加心跳或租约方法。托管执行模式下 Agent 的 TaskMgr 任务与 Workspace run 是两份记录，权威在 Workspace run；Agent 任务失败或取消需回写 run 状态，建议新增 `proc.report` 由 Agent 调用，比 Workspace 轮询 TaskMgr 简单。

#### D16 执行授权与任务输出边界

**需要决定**：如何表达“可以执行预定义任务，但不能任意编辑数据”；结果写入的权限上限；任务能否创建新对象或新 Source（冻结思路 §8.5）。

**当前实现**：能力位只有 read、append、update、delete、structure、comment、export、manage，没有执行能力。许愿格结果以调用 `proc.apply` 的用户权限提交；候选计划允许 keep、replace、new，因此可以创建新的结果对象，但不能创建 Source。

**建议**：新增 run 能力，授予者可以在某个任务实体上触发执行。结果提交的权限上限为“执行身份的当前权限 ∩ 已确认任务配置声明的输出目标”（包括声明的新建位置）；触发者只需要 run，不需要对输出目标有 update。任务配置摘要改变后，旧授权不覆盖新配置，需要有编辑权限的人重新确认。v1 的任务可以在声明的位置创建结果对象，但不能创建或修改 Source；导入外部来源只在托管执行模式下、经单独授权提供。此项会修改能力位图和授权条目结构，需要在冻结前确定。

**Review 补充（2026-10-22）**

**权限来源要二选一，或按模式各取其一（D22）：**

- **执行身份派**（本项现方案）：写权限 = 执行身份当前权限 ∩ 声明输出。托管执行模式下成立（Owner 的 Agent 持有 Owner 权限）。本地模式下执行身份就是触发者，只有 `run` 的协作者写不回，等于本地模式只对有 update 的人可用，委托无从谈起。
- **确认者派**（能力委托）：写权限 = 声明输出 ∩ **确认者**在确认时的权限，apply 时复查确认者仍持有。触发者只需 `run`，执行身份可以是任何人（含触发者自己的浏览器）。服务为该 run 签发限定输出目标的 run token。这才是“允许执行预定义任务但不允许自由编辑”的完整实现。

两者可以并存：托管执行模式用前者（Agent 身份即边界），本地模式用后者。D22 的决定是 v1 统一用执行身份派，只有 `run` 的协作者改走 Owner 配置的托管执行；确认者派不在 v1 范围。

其他：配置摘要变化后“需要重新确认”要有界面状态和错误码（`DELEGATION_STALE`，D09）；`run` 能力位加入后 `Cap` 从 8 位变 9 位，`grants.capabilities` 整数列可容纳，但 `ALL=255` 常量与导出包中的 `write_policy` 要同步。

#### D17 演示基线与访问策略

**需要决定**：固定基线用写锁还是克隆实现；演示状态与事件是否持久保存；原 Workspace 撤权后，已有演示是否继续可访问。

**当前实现**：第三期的非公开放映按第三期规划 §8.1 实现。有写权限的用户开始放映时，整个 Workspace 被锁写，所有会话的提交返回 `SHOW_LOCKED`；live 放映另建临时克隆（`VACUUM INTO`，1 万行表约 45 ms），租约 60 秒，结束或过期即删除。“只读”按 Workspace 级 `any_write()` 判定，因此只有子树写权限的用户也被当作只读：不加锁、不克隆，直接读取实时内容，没有固定基线。克隆保留 grants 与 runs（活动 run 变为 interrupted），删除锁、暂存上传和 user_state，并沿用源 Workspace 的 epoch 与 seq。放映锁、状态和命令都只在内存中，服务重启结束所有放映。第十节描述的固定版本演示面向第四期的公开演示房间。

**可选方案**：

- A：维持写锁作为基线机制。
- B：所有演示都以克隆为基线，取消写锁。
- C：固定基线统一用克隆；放映锁保留为第三期非公开放映的产品行为，不作为基线契约。

**建议**：C。第四期的公开演示、回放和“结果带回原文档”都需要独立基线，克隆可以直接提供；放映锁继续服务“作者检验效果”的场景，并在协议中注明只读放映没有固定基线。克隆和事件的保留期按 D06 登记；只读用户发起的克隆由谁承担空间，按 D19 确定。演示访问在每次请求时按原 Workspace 的当前授权判断：原 Workspace 撤权或停止共享后，演示随之不可访问；需要独立于原 Workspace 的长期公开，走导出或发布，不由演示承担。

**Review 补充（2026-10-22）**

- “按原 Workspace 的当前授权判断”与代码不一致：克隆时复制了 grants，之后按克隆内的副本判定。需对齐为查原 Workspace 的当前 grants，否则撤权不会传导到正在进行的演示。
- 服务重启结束所有放映是内存态的结果；第四期公开演示房间若要跨重启，演示状态必须落盘。D17 应说明这属于第四期范围。
- 只读用户发起的克隆由谁承担空间：建议计 Workspace（owner）额度，并设每 Workspace 并发克隆上限与 TTL；Guest 没有额度账户，不能计个人。
- 放映锁判定建议改为“有任何写权限即加锁”，或接受现状并写明子树写权限者放映没有基线。
- 克隆带完整 Loro 历史，只能服务同 ACL 的演示，不能作为分享或发布的固定版本（D26）。

#### D18 公式 v1 边界

**需要决定**：v1 是否在数据结构中为公式预留位置；后端是否在读取时求值；公式结果如何保存。

**当前实现**：没有公式。生成结果通过 `entity.set_derived` 记录依赖（许愿格、run、执行器和读集合版本），由 `doc.freshness` 判断结果是否过期。

**建议**：v1 后端不做读时求值。公式定义作为实体的普通 key 保存，返回类型由 Schema 声明；需要共享或被其他应用读取的结果，显式物化为 Commit，依赖用现有 derived 记录表达，并增加求值器标识和求值语义版本。前端公式只在客户端显示求值结果，不写回。这样 Get 不会推进权威版本（K10），后端求值语言和缓存可以以后再定，不影响格式。

**Review 补充（2026-10-22）**

- “前端公式只在客户端求值不写回”意味着 **API 消费者（Agent、SDK）读到的是公式文本**，服务化承诺对公式字段不成立。写明：要被外部读取的结果必须物化。
- 公式的**值形状现在就要进 FORMAT 1.0**，否则以后加公式就是不兼容变化。建议预留 `{ "$formula": { "lang", "version", "expr" } }` 作为带类型 key 的合法值，由 Schema 声明返回类型；v1 只保存、不求值。
- 物化结果用 `set_derived` 与 `doc.freshness` 判过期，谁来重算？v1 只有浏览器与 Agent，答案是“有人打开时”，与电子表格打开重算一致，写明即可。

#### D19 配额与维护通道

**需要决定**：计量口径；超额时的结果形状；维护通道包含哪些方法。具体数值和计费可以延后。

**当前实现**：没有 Workspace 配额，只有单次请求上限（例如 Commit 请求 8 MiB、单个用户状态项 256 KiB）和 planner 的 Limits；AICC 侧已有按用户计的额度。

**建议**：

- 计量按 Workspace 的逻辑占用：当前内容、保留期内的历史资源、checkpoint 与导出包、演示克隆和暂存上传；与底层去重无关。
- 超额时以 rejected 返回专用错误码，区分“释放空间后可重试”和“需要提高额度”（D09）。
- 维护通道是一组明确的方法：解除保留、删除 checkpoint 与导出包、取消 run、清理暂存上传、立即回收已删除内容的资源（同时放弃对这些内容的撤销和版本恢复）。这些方法按 manage 权限开放，并有单独的资源预算。

D06 与本项相互制约：若提交历史 v1 不截断，删除内容本身不释放资源空间；必须依靠历史资源保留期或上面的“立即回收”，否则触限后无法恢复写入。

**Review 补充（2026-10-22）**

- 逻辑占用需要**随事务更新的用量账本**（commit 内按 assets 大小与结构化字节增量累计），不能靠事后扫表；“立即回收”同步减账。
- 结构化历史是否计入见 D06 补充（建议 v1 不计）。
- “维护通道按 manage 开放”意味着超额时普通协作者只能找 owner，可接受，但超额错误应告知前端联系谁。
- 流量计量的消费者目前只有 Source 抓取与上传下载；若 D24 采纳，流量额度就是服务侧抓取的唯一闸门。

### 13.6 Review 提出的新增决策

以下决策由 2026-10-22 的 review 从 D01–D19 的二阶后果中提出，各项的来龙去脉见对应决策下的“Review 补充”；2026-10-23 全部确认，与 D01–D19 同等纳入 TODO 推导。标为“由决定推导”的细则在实施前需再核对。

#### D20 规范化日志是否独立于 planner 可应用

**需要决定**：副本重放是继续经过 planner，还是只应用已完全解析的规范化日志。

**当前实现**：副本以 `Access::full(author)` 和 replay 标志调用同一个 `plan_commit`（[replica.rs](../../src/frame/aiworkspace/core/src/replica.rs)）。planner 内的归一化、`tree.place` 合并、`set_keys` 的 rev 推进等逻辑都参与重放。

**可选方案**：

- A：日志改为**纯应用型**——cell 级写入已完全解析、结构 hash 预计算，重放不再经 planner。日志变大，换来 planner 可自由升级。
- B：每条 commit 记录 planner 版本，副本携带多版本 planner。实现负担大，WASM 体积随之膨胀。

**建议**：A。K05 的验收改为“旧日志在新 planner 下重放一致”。D05 的级联删除、D01 的业务校验剥离都以此为前提。

**决定（2026-10-23）**：采纳方案 A。

#### D21 本地模式生成程序的执行形态

**需要决定**：AI 生成的程序在浏览器中以什么上下文运行。

**当前实现**：浏览器侧只有 Mock 的 `HtmlExecutor`，在页面主上下文执行，无沙箱。

**建议**：sandboxed iframe 或 Worker，只经 postMessage 获得受限 Workspace SDK（读输入快照、写候选结果），不持有 token。安全风险按“先能力后安全”作为已知风险记录，但接口形状现在定。

**决定（2026-10-23）**：不用 iframe。每个 Workspace 使用独立域名，URL 对 SEO 友好，形态为 `aiws-<id>.<zonehost>/<workspace_name>/<surface_name>?...`，Workspace 之间以及与 Desktop 之间由浏览器源隔离。生成程序用 Worker + postMessage。

以下细则由决定推导，实施前需再核对：

- 独立源隔离的是 Workspace 彼此及 Desktop，不隔离生成程序与本 Workspace 页面。Worker 与页面同源，但没有 DOM 和 cookie 访问；token 只留在页面主线程，SDK 调用经 postMessage 代理。建议中的“不持有 token”仍然成立。
- 子域名由 cyfs-gateway 的子域路由提供；现有 `/workspace/<id>` 独立页面路由迁到子域。登录态在 `<zonehost>` 父域还是各子域各自持有，要与混合 SSO 的 cookie 策略一起定。**→ 2026-10-28 定：各子域自持**（host-only cookie，`System(aiworkspace)` 目标，Desktop 经 `prepare_app_launch` 委派、Zone 根 `/login` 对已登录者自动接力）；Zone 根 `/workspace/<id>` 不迁移、直接删除（breaking），`/workspace` 列表保留。
- URL 中的 `workspace_name`、`surface_name` 是 D10 的名称投影，可变。改名后旧链接是否继续解析（保留名称到 ID 的历史别名）需要决定；D10 的“URL 与引用对象无损互转”要求 URL 里仍能带上 ID 或 ID 可由名字唯一解析。**→ 2026-10-28 定：`/<workspace slug>/<surface slug>?id=<surface id>`**——名字只为可读，`id` 决定导向；无 `id` 的链接由服务按名/标题 slug 解析后 302 到带 `id` 的规范地址，名字无别名。
- SEO 友好意味着公开内容需要服务端渲染或预渲染，属于 D26 发布链路的产品项，不属于内核。
- §13.7 的“URL 的具体语法”相应改为“URL 的完整语法”，基本形态已由本项决定。

#### D22 任务结果写权限的来源

**需要决定**：结果写回的权限上限以执行身份还是以确认者为准。

**建议**：按模式各取其一。托管执行模式按执行身份（执行身份权限 ∩ 声明输出）；本地模式按确认者（声明输出 ∩ 确认者在确认时的权限，apply 时复查），由服务签发限定输出目标的 run token。触发者只需 `run`。

**决定（2026-10-23）**：托管执行分两步。v1 使用 Workspace Owner 指定的配置执行（Owner 的 Agent，以及 Owner 管理的能力与额度配置）；后续再支持使用当前用户自己的 Agent。Owner 要承担配置管理。

以下细则由决定推导，实施前需再核对：

- v1 的权限来源统一为**执行身份派**：托管执行的执行身份是 Owner 配置的 Agent，写权限 = 该 Agent 的当前权限 ∩ 已确认的输出声明。只有 `run` 权限的协作者通过它执行，不需要“确认者派”的 run token。
- 本地模式写权限 = 触发者本人权限 ∩ 输出声明。没有 update 的人在本地模式写不回，界面应引导其改用托管执行。
- 冻结思路 §8.7 的触发者、执行环境所有者、执行身份三角色直接对应：触发者是点击者，环境所有者是 Owner，执行身份是 Owner 配置的 Agent。§9.4“委托绑定任务版本”落在 Owner 的配置项上：任务配置摘要变化后由 Owner 重新确认（`DELEGATION_STALE`）。
- 后续加入“当前用户的 Agent”时，权限来源仍是执行身份派，协议不变，只在 run 记录中增加执行身份的选择与审计字段。
- Owner 配置需要的管理面：允许哪些任务、哪些协作者可触发、能力与额度上限、Agent 选择。属 D16 的授权条目结构，冻结前一并确定。

#### D23 “用户后台模式”是否并入托管执行模式

**需要决定**：是否保留 D14 的第三种执行形态。

**建议**：并入。Node Executor 的凭据与映射规则使方案 B 的执行身份就是一个 Agent；方案 A 是再造一个无对话能力的 Agent。默认后台执行者为该用户的 Jarvis；Owner 委托等于“Owner 的 Agent 替协作者执行”。没有 Agent 的用户只有本地模式。

**决定（2026-10-23）**：原理上并入，但不叫“Agent 模式”：Run Agent Work Session 被视为 AppLoader 的一种形式，“Agent 模式”的叫法会误导。本文统一改称**托管执行模式**，定义为“经 AppLoader 启动一个 Agent Work Session 来执行任务”。

以下细则由决定推导，实施前需再核对：

- D14 的三形态收敛为两形态：本地（浏览器）与托管（AppLoader 启动 Agent Work Session）。Node Executor 方案 B 是托管形态的一种底层实现选择，不再是独立形态。
- 结合 D22：v1 托管执行的 Agent 由 Owner 指定；“默认执行者为该用户的 Jarvis”推迟到支持当前用户 Agent 的阶段。
- 派发传输（D14 Review 补充第 1 点）按 AppLoader 语义理解：Workspace 请求 AppLoader 以指定 Agent 开一个 Work Session，而不是向 Agent 发一条消息。两者的接口差异在实施前核对 OpenDAN 的 Work Session 创建入口。

#### D24 v1 自动同步的宿主

**需要决定**：定时刷新 Source 在 v1 由谁执行。

**建议**：服务侧保留“无凭据公开 URL 抓取”的窄路径，视为 Source 基础设施，不运行用户代码，只消耗 Workspace 流量额度；有凭据的 Source 走托管执行模式。否则 v1 只有手动刷新。

**决定（2026-10-23）**：采纳建议。

#### D25 资源 Folder 的命名与 DirObject 时间戳规范化

**需要决定**：资源 Folder 与数据树 `container(kind=folder)` 的命名；物化 DirObject 时时间字段取值；是否允许外部 URL 成员。

**建议**：二者改名其一（资源 Folder 改称 `dir` / “资源目录”，或树容器改称 `group`），不拖到实施期；物化时 `create_time` / `last_update_time` 取固定值，与 core 的 `file_object` 规则一致；v1 禁止外部 URL 成员，并为成员数设上限。

**决定（2026-10-23）**：以 DirObject 为准。若 DirObject 无法满足内部需求，则同时支持内部 Folder 形态与 DirObject，两者并存而不是放弃 DirObject：DirObject 很容易成为被 Workspace 引用的外部资源。命名与时间戳规范化按建议处理（未另表态）。

以下细则由决定推导，实施前需再核对：

- 并存时需要明确映射：内部 Folder 的成员若都能表达为 FileObject 或子 Folder，物化时直接生成 DirObject；含 DirObject 无法表达的成员（外部 URL、顺序以外的元信息）时，按 §6.3 规则在导出或固定演示时拒绝、物化或显式降级。
- 外部 DirObject 作为被引用资源进入 Workspace 时走 D12 的 Source 路径：ObjId 本身不变，所以天然是 fixed；“live”只能经名字入口或 PathObject 解析。
- 时间戳固定规则同时适用于物化出的 FileObject 与 DirObject。

#### D26 内容快照与协作快照的导出格式分离

**需要决定**：分享、发布、演示、备份各走哪种快照。

**建议**：分享与发布只走 ContentRoot（不含 CRDT 字节与 commits）；整库克隆只服务同 ACL 的演示基线；个人备份可带 `collab/*.loro`，但要在导出界面标明含完整编辑历史。

**决定（2026-10-23）**：重点是**按路径发布**：Workspace 在一个数据源上标记为公开后，得到一个链接，该链接可以在另一个 Workspace 里直接插入；发布方能跟踪自己的产物有没有被引用。快照格式分离的建议作为发布链路的实现约束保留。

以下细则由决定推导，实施前需再核对：

- 公开链接是 D10 引用对象的 URL 投影，形态按 D21（`aiws-<id>.<zonehost>/<workspace_name>/<data_name>`）。默认 live，对应现有引用对象的 `published_channel` 版本模式；消费方插入时可选 fixed，解析为该数据源的实体 Content ID 或 ContentRoot，不是整库克隆。
- 消费方以 D12 的 Source 接入：同 Zone 内由服务直接读取；跨 Zone 走 D24 的无凭据公开抓取。发布即对该数据源授予 `public` 主体读权限（现有 grants 的 `public` 主体可复用），其余 ACL 不变；§6.2 的“导入副本授权”由消费方 Workspace 的 ACL 决定。
- “跟踪被引用”需要反向引用登记。同一服务内由服务级跨 Workspace 引用索引实现（refs 已有 `dst_workspace_id`）；跨 Zone 需要消费方插入时向发布方登记订阅记录，否则只能退化为访问日志。登记是弱引用，不阻止发布方撤回；撤回后消费方按 §5.2 得到 unavailable。
- 发布的数据源可以是 Folder（D25）：产物文件夹（冻结思路 §3.5）就是“标记公开的 Folder”，解析为 DirObject ObjId。
- 关联：D10（引用对象）、D12（Source）、D21（URL）、D24（抓取）、D25（Folder）。

#### FORMAT 1.0 冻结前必须一次进入格式的内容

D07 规定冻结点之后不兼容变化必须显式转换。以下若不在 1.0 中，之后每项都是一次转换：

| 字段或结构 | 来源 |
| --- | --- |
| 实体 `schema_version` 允许大于 1，并记录业务 Schema 标识 | D01 |
| 资源 Folder 原语及成员表 | D01、D25 |
| `owns` 引用种类；引用边的 `mode: live / fixed` 与 `integrity: strong / weak` 列 | D05、§5.2 |
| 规范化日志改为纯应用型（或记录 planner 版本） | D20 |
| `version` 判别联合的两种形态 | D03、D10 |
| 公式值形状 `$formula` | D18 |
| `run` 能力位 | D16 |
| 任务配置摘要与委托记录（输出声明、确认者、摘要） | D16、D22 |
| 用量账本表；保留根表（替换恒为 1 的 `snapshots.retained`） | D19、D06 |
| `user_state` 修订号列 | D13 |
| 历史下界 `base_seq`（可先恒为 0） | D06 |

### 13.7 可以延后的事项

以下事项不影响内核数据结构、接口和运行模型的冻结，可以在相应能力开发时再定：Agent 与 XL Agent 的分类及差异；Guest 自带模型 Endpoint 与跨 Zone AICC；函数对象（Function Instance、Lambda Object）的格式、缓存和发布；公开演示房间的事件编码和回放界面；URL 的完整语法（基本形态已由 D21 决定）；配额数值和计费；后端公式语言和增量求值算法；托管执行使用当前用户自己的 Agent（D22 第二步）。

### 13.8 下一步

D01–D19 已于 2026-10-22 全部确认，D20–D26 已于 2026-10-23 全部确认，结论记录在 13.1 的状态栏；与建议不同或有补充的决定（D01、D05、D21、D22、D23、D25、D26），写在对应小节的“决定”中，其中标为“由决定推导”的细则在实施前需再核对。13.2 的八条契约已于 2026-10-23 按确认意见确认；各决策下的“Review 补充”与决策正文同等对待。冻结前的设计决策至此全部确定，实施 TODO 已推导为 §12.3（M0–M5，T0-1 至 T5-6），从 M0 开始实施。
