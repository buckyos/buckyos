# BuckyOS 需求规划列表

基于改列表，可以使用AI显著的提高进度

## 内核

### Boot流程里涉及到cyfs-gateway的boot config构造
尤其是在oods[ood,gateway]这个典型的2节点网络场景下

### boot + system_config 支持多节点
- 1个OOD -> 2个OOD -> 3个OOD -> 2n + 1个 OOD，n最大为3
- system-config能更有效的支持本地cache(特别是buckyos selector）场景，减少系统访问的数量
核心逻辑是缓存时知道系统的 version,根据请求里带的version进行自动更新，或则基于时间自动更新（至少可以在30秒内不发起 get请求）

### 调度器逻辑完善
这个已经被抽象的非常独立，完全可以进行一波AI研究实践
当调度器所在的OOD不可用时，如何让另一个OOD自动启动调度器（调度器的自动迁移，本质上是boot逻辑）

#### OPTask机制完善


### klog成为新的系统服务
为有分布式需求的其他内核服务，提供更高性能的raft协议支持
并与system-config隔离开来，保证内核的关键能力不会受到影响

## Named Data Manage(DCFS)
- 接口语义和存储backend的语义确定 
- 与cyfs:// 协议设计理念统一，强调更面向吞吐，面向AI的声明式数据处理： new_url = 算子(url1,url2 ... )的概念，而不是手工的 open,read,write,close
- 在上述语义确定的情况下，支持DFS
考虑到我们的规模，我们的核心逻辑是：
所有的Client节点都有完整的MetaDB，单只有OOD节点的MetaDB 可写
chunk storage可以以标准文件系统的形式，保存在多个目录下。在小规模下，chunk storage主要目的是组成存储池，重点保障可用性和性能释放，可靠性依靠system-backup
在大规模下，才会基于chunk storage 桶的概念提高本地可靠性
复杂调度工作由buckyos调度器协助完成（边界很重要），能让DCFS保持简单

### 接口完全FS化，目标是基于现有核心设计，未来能扩展成DFS
- 现在的Path接口，能与did-resove对接上
- 基于NDM原始接口，与FUSE接口对接，实现广泛的mount支持

### 基于klog实现分布式fs-meta-db

### 存储池扩展管理，通过添加不同类型的chunk storage来

### 基于OPTask的存储运维
- 数据完整性扫描
- 存储池改变后的强制数据平衡操作（数据迁移）

## cyfs:// 与 Content Network

## BuckyOS & OpenDAN 基础服务
### TaskMgr
设计完善

### MessageBus(MessageQueue)
这个传统是个内核组件， 我现在为了管理复杂度，刻意不允许在内核使用pub/sub(事件)机制。
消息总线，buckyos的 分布式的pub/sub基础设施

### InBox
用户InBox,通知管理中心的的底层设施

## Computer Center Service (OpenDAN)
关键基础服务，分离了“AI推理能力使用需求”和“AI能力提供者”
Router逻辑能根据规则，选择最合适能力提供者

- LLM Router
- Text2Image Router
- Text2Video Router
- Text2Vocie Router
- Image2Text Rouer(现在已经合并到LLM多模态中)
- Vocie2Text Router(现在已经合并到LLM多模态中)
- Video2Text Router

### Workflow (OpenDAN)
工作流定义了输入输出和一些步骤，这些步骤里的操作会依赖特定软件）。
这是一种可安装的App类型，安装后会同时安装上依赖的软件。
用户可以手工替换一些步骤的实现，只要符合这个步骤的输入输出即可。
是否可以使用N8N的底层设施？


### Agent Runtime(OpenDAN)
Agent 为基于自然语言提示词的Agent提供了在BuckyOS上运行的环境。除了LLM能力是必须支持的外，还包括下面可选基础功能
- 与其他Agent通信
- Code Interpreter （
- MCP
- Agent Memory管理
- 文件访问
- 知识图谱访问

### Agent Code Interpreter(OpenDAN)
核心功能，平凡的支持Agent构造Code并执行的能力
支持两个关键功能
- 构造用户可使用可发布的软件服务
- 构造可以给Agent反复使用的工具

其基本内核是一个python沙盒，和一个专门写代码的agent,这比调用MCP更有效的节约token

### Agent Message Tunnel (OpenDAN)

解决如何发送信息给Agent,Agent 有did
可以给Agent注册各种传统的账号(比如Telegram bot,slack bot)
Agent和Agent之间互相发送信息


## BuckyOS系统管理（Control Panel)
TODO：需要大规模的细化
- “添加” 入口
- 系统的基本信息，和关键信息的实时使用情况
- 任务管理器：从资源使用的角度，看到系统里正在进行的Task(尤其是AI在干什么)
- 网络管理（名字管理？访问链路，内部局域网，网络行为管理）
- 设备管理
- 用户管理（名字管理）
  - 用户数据管理

- 存储管理 （这个是否应该和备份一起，独立在存储管理里？）

### dApp使用的系统集成功能
- 身份管理
- 内容分享（发布）已发布内容在内容管理里

## dApp & Agent管理

## BuckyOS的默认应用

### Jarvis 
默认的入口AI Agent（统一入口）
可以根据任务类型，自动选择更适合的Agent， 用户也可以手工切换到特定Agent。
一些重要的内置Agent:
- 写软件的Agent: 利用BuckyOS的基础设施，快速完成软件开发并发布到当前Zone
- Agent的UI基本上都是chatbox,产品的逻辑是“立刻响应的AI”，对于AI驱动的非即使响应（），应该引导到别的系统App进行跟踪
- 因为Agent可以写软件，所以我们鼓励Agent为当前任务构造临时的AI

### Home Station(Content Network 入口)
AI时代的Content Feedlist，更好的满足“刷刷刷”的需求
想看到内容/发布内容时的入口
内容也包含商品，所以传统逛taobao的需求也是在这里得到满足的（发布商品对普通人来说，就是发起一个二手交易）
内容也包含浏览历史记录，如果在cyfs-gateway中配置了https内容拦截保存，那么保存的历史内容也在这（通过浏览器插件也可以实现同样目的）

### Message Hub 
AI时代的信息中心(Outlook)，可以看到实时的通信信息，并立刻回复
用AI整合了今天的 Messages,Email, Calender（日程管理）,通知中心，TODO List
- 不会漏掉关键信息，并能整合小TODO
- 能合并不同来源同一个人的信息
- 更好的过滤垃圾信息
- AI辅助回复

还可以看到Agent与Agent之间的RAW通信记录

### My Home (Home Assistant的AI进化版)
物理的家在虚拟世界的投影
- 看到设备的状态并进行直接管理（智能家居）
- 设备行为管理（比如对小孩的上网行为进行家长控制)
- 基于AI分析的 Home Logs,可以看到一些重要的事件和相关的所有信息
安防事件的AI升级版，通过摄像头的原始数据可分析出 “Bob来访"，“儿子出去玩了”这种信息
有影视级设备时，如果家里开Party,还可以自动拍摄精彩瞬间

- 故障管理，issue处理的跟踪
- 看到能源使用和一些日常损耗品（卫生纸、瓶装水）的库存状态，分析消耗和开销、自动补充（自动花钱）

### Workspace 
已经有基础原型
AI时代的工作桌面，整合了TODO管理，团队，相关目标文件，相关资料文件（来自各个管道），相关历史信息（可能来自团队的知识库）
产品核心是帮助人关注只有人才能解决的问题
- 灵感：通过浏览各种可能的相关信息，获得灵感（想做什么，比如一个游戏的原始创意）。灵感是所有事务的源头。
- 责任：需要我完成的事务，这些事务可以进一步指示给AI完成
- 责任：需要我确认的事务，一些事务已经被标注为完成，我需要确认其完成质量，并决定后续安排
- 学习：通过一个合理的路径阅读资料，完成对某个知识的学习（未来哪些知识是一定要学习的？）
- 补充信息：为事务的开展，持续补充相关信息。 

### 面向AI,取代Filebrowser的 新文件浏览器
看到拥有的所有数据，数据是支持所有高级功能的基础
支持基于对话查找内容
支持Agent用不同的逻辑构建知识图谱后，在这里通过可视化访问
关注点是存储管理、可靠性管理（备份回复）、版本等

## BuckyOS App (钱包)
与BuckyOS面向Zone不同，钱包是以人为主体的软件

## Backup Suite (独立产品)
Backup Suite的核心功能，是尽量帮助用户把现有数据迁移到BuckyOS上
 
 ## SDK
 




