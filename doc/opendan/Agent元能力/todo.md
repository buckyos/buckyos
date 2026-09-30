# Agent元能力框架的TODO

站在Agent完整元能力的视角，梳理Todo

## Agent Session Runtime

Agent Session是Agent活着的容器，其历史记录是Agent存在过的证明

> TODO 用process-chain 来重新实现hook point


### UI(Input Route) Session 

UI(Input Route) 都是long life-time session,期望基于历史记录理解Input的真正含义

UI session会积极的update-session-topic

> TODO: 提示词里优化对Notebook 的使用
> TODO: 群聊支持
> TODO: 看到的更多的历史消息记录（其它Session Agent发送给用户的消息）
> TODO: UI Session中用命令处理Approve

### Input Route  

Input本身带有Target Session最好，这样就不用Route
否则: Input -> UI Session -> LLM -Route-> WorkSession

根本性的痛苦之一:LLM Route错了，目前这类错误造成的伤害是最大的

> TODO: UI Session中识别到机械Forward标签
> TODO: 重点优化和Route相关的提示词

### Work(Task) Session

Work(Task) Session 通常专注于一个确定具体的任务(不是long life-time session)，open DAN的世界观里，Agent应该由大量的worksession和少量的UI Session组成。复杂工作放在Work Session里，能从基本架构上规避Context Windows的问题。

在Plan阶段倾向于“了解更多”状态（有长上下文），并会更新
在DO阶段，专注于完成特定TODO：假设信息已经在Plan阶段收集够，不探索，专注完成目标，快速失败
处理Report
处理workspace与产物交付

> TODO: **Plan阶段的提示词需要基于元能力框架大幅度的升级**
> TODO: Report的范式需要调整

### Agent Notebook + Self-Check

Agent Notebook有2个核心目的

1）处理延迟任务: 
直接处理：Input->LLM->CreateTaskSession->TaskSession完成
延迟处理：Input->LLM->记录Notebook --延迟--> Self-Check:创建计划任务--延迟-->执行计划任务->CraeteTaskSession->TaskSession完成

2）记录被声明的事实

> 分类学讨论结论（坑已踩过，避免重推）：
> - 这两个"目的"是按**消费者**分的（self-check 半 vs 召回半），不是按 itemKind 分的。写入侧其实只有一种 kind：**一条强信号原句**。fact/plan 不是写入时身份，是 self-check 后置、可演化、允许 `unresolved` 的 Review 标注（"做完X再做Y"记下那刻就判不了，必须容忍悬而未决）。
> - **强/弱信号 = 投递保证 = 资源授权，同一根轴**。所以推断（弱信号）不能直接建计划任务——best-effort 投递没资格授权资源开销。弱信号留 Memory，靠"提纯"升级：Memory 高召回候选 + 确认握手做精度门，用户"好"才写入 Notebook。中间态留 Memory，别在 Notebook 里堆未确认脏条目。确认摩擦 ∝ 解锁的下游承诺（同 Governance approval）。
> - 计划类 item 建完任务**标记不删**：防重建 + 当查询锚点（是否建任务/是否执行/成没成）。注意两条正交生命周期别合并——note 自身 `active/stale/superseded`（指令还作不作数） vs 派生任务 `pending/done/failed`（挂 Schedule-Task，查询时投影）。
> - 计划半**不上自动召回**（自寻址、有 self-check 心跳兜底），只保留显式查询；事实/偏好半才走广播式 Hint 召回。

> TODO: 架子已经完成了，需要优化实现

## Global Object (World State)

通过通用的方法(Global Object)定义世界状态,解决Agent主动探索更多信息
需要定义观察/探索/互动/订阅的标准动作
这里的核心是read （返回的必然是一个引导文本（提示词）），要考虑风险

> TODO 建立下面标准抽象： OK 

```text
read(object_id,params)
read(indexer_id, filter) //用read代替？
xcall(object_id, method, params)// 工具化？
subscribe(object_id, event_name, options)
unsubscribe(object_id, event_name)
read(url) //用read代替？
```
file:// (默认) : read
http:// or cyfs:// : read

原创：obj:// : read / call / subscribe，理解容器 + /@/ ,
原创：index:// : read only


### 为LLM定制的 read

>read 是开放世界解释器；xcall/event 是封闭世界执行器。

- agent的read 路由配置 （系统+agent+session overlay)
- **定义read adapat**
- 没有任何adapt 的情况下的read行为
    - 读取本地文件
    - 读取http/https
- read 对 cyfs:// 直接支持(更多的当数据），但要做一些翻译，核心就是对known object-type(schema) 进行定制化的翻译
- read 对 did-> Object（更多的当实体），核心是对known did-document schema 进行定制化的翻译

翻译的核心是简化

- 为LLM阅读简化语义（固定的json->text)
- 支持分级压缩
- session级别的可变检查（返回not changed)

read返回的多段式结构
- content， 注意Range（文本文件自带行号），裁剪和压缩
- Meta
- 提示词指导：Session级别的”逐步披露“和”未修改引导“，面向信用系统的可信性支持
  - 逐步披露是指相关工具的使用方法，进一步信息的获取的方法，Object Meta可以认为是一种通用的逐步披露协议
  - 未修改引导是指在一次session中，不重复返回已经返回的信息（包括逐步披露的内容）
  - 信用是说明数据的来源和可信度
- 错误引导： 如果读取错误，要把错误信息转换成LLM易于理解的类型

### 索引支持

Read(indexid) -> 说明查询方法
Read(indexid?query=xxx)->返回查询结果

### 实体支持

Read(obj_did)
Sub(objid,event_name) / Unsub(objid,event_name) ： /objid/event_name
Call(objid,function_name,params)


> TODO: 定义xcall协议 （本地或raw http) OK
    1）确定object的类型（本地特殊 / 标准 remote）
    2) 执行调用 ：本地trait 或 标准的json rpc (注意session对rpc client的初始化影响)

> TODO: 定义event协议。（本地或raw http) OK
    1） 确定object的类型（本地特殊 / 标准 remote）
    2） 订阅事件， remotes

### 数据(cyfs://)支持

Read(objid) -> 最简单，读取一个对象(如果是容器会返回分页访问的方法） objid是hash

objref: cyfs://$objid

> TODO 需要增加支持

### 工具支持(Agent Tool体系)

tmux session + 意图引擎

> TODO: 支持agent tool索引器，可以了解当前的环境（构造工具的环境+可安装的工具+已有的工具）

## Agent State & Hint Recall

定义Agent的内部状态

定义Recall的时机，Recall的结果是Hint 通过Hint的适当披露解决Agent Session里"不知道自己不知道“和塞入一大堆无用信息占领Context Window的问题.在Session中通过update-session-topic + 半订阅recall

### Hint Recall

Hint = 时间 + 一句话 + 对象ID

固定下现在支持的hint架构，并打通从hint->事实的路径

	
确认基于tag的机械召回路径
- 有tag的对象的匹配
- 传统的，基于FT5的倒排索引的召回（对象的那些属性会进？）
	
	
确认基于LLM的半订阅调用（目前缺失的一环），触发边界在哪？订阅太多触发？能否机械的判断当前session topic应该订阅哪些global object?

> TODO: 正确实现新的Hint


### Agent State Self Improve

Stage1: 从Session History中发现Attention Singal
发现事件 :
发现Object（主语或宾语）
探索Object之间的关系

Stage2：整理Attention 热度（世界参与度）
更新 Agent Memory Graph State (G_State): 当前G_State + Attention Signals => 新G_State 

Stage3:
寻找捷径（skills）： 独立流程


搭建框架的主要工作还是完成定时的触发，按照设计，定时触发主要分为以下两个机制：

1. 定时检测与触发
   第一步是用来解析 Session History。系统每 24 小时会进行一次检测，如果 Session History 有更新，那至少会触发一次。
   这类触发一般是凌晨3点进行检查


2. 阈值触发
   当 Session History 的累计量到达一个阈值的时候，也会触发更新。
   注意 Self-Improve所在的Session是特殊Session，其History不会触发Self-Improve


3. 触发后：
    UnImprove Session->Stage1 LLM->Attention Signals

4. 第二阶段是独立触发。也就是说，当第一阶段完成之后，只要满足以下两个条件，第二阶段就可以触发：

- 第一阶段已经完成了消费，并且明确标明它已经完成了一次处理。
- 存在第一阶段的结果。此时，Attention Signal 到 State Miner 的这个阶段就可以触发。
    Attention Signals->Stage2 LLM->Set Memory
    按时间逐个 分析这个时间窗口内的Session Hisotry,强调跨Session的总结


> TODO: 增加一个专门的组件来管理 Attention Signals OK 
> TODO: 补齐 Attention Signal / Session History 管理操作的 bash CLI 支持 OK 
> TODO: 如何重点观察skill的两种信号？ : 待实现，根本上是需要让session/session report与skill建立强结构关系 later
    1）使用了skill的session -> 看report
    2) 使用了skills mgr的selector,但没有选中任何skill
> TODO: 需要Agent视角的，对Object进行备注和状态管理的系统： 对象观察类memory OK 


## Skills 

> Skills是Agent与世界交互的捷径

安装Skills + 安装Agent Tool 是用户对Agent能力进行扩展的最主要的方法

### skill的标准定义 

skill的分类 
1）信息获取类（比如下载youtube video)
2) 交付类，对于内部系统，不给skill不可能被发现
3）流程类，做成一类事情的捷径
4）范式类：比如如何写一个好的BP，这类skill的存在主要是弥补LLM的不足，容易被LLM的发展给取代

一个（结晶出来的） skill 至少必须有：

- trigger / when_to_use
- procedure
- dependencies / required tools
- pitfalls
- verification
- source_event_ids
- risk_level
- owner_scope
- lifecycle_state
- verification_status

> TODO 需要重新定义skill的格式 OK

### skills的使用 

plan模式进行选择） + report汇报效果

> TODO 需要在标准定义的基础上优化提示词
> TODO 如何在UI Session使用skill?有必要么

### skills的安装 (llm_install_skill)

> TODO 需要实现 首先应该实现，这根本上是给一个结构后，让llm帮着完成“强化“的过程，

### skills的结晶和整理 (improve-skill)

> TODO 这个版本暂不实现（与价值观绑定太大）

## Agent的价值观

给自己主动布置什么任务？如何影响找捷径的方法？如何在多个同类事物中做选择
> TODO 暂时不支持

## Governance Runtime (统一信用管理？）

根据理论，似乎是Plan阶段对Do阶段的权限管理？

- 通过硬边界（沙盒）限制能力 （目前）
- 建立信用架构：如何定义？ Read里已经有相关的信用提示词了
- 根本矛盾是 LLM 自主决定的自由度 与 权限控制硬边界的矛盾 （这也许是产品问题？ 也许是一个核心问题）

identity
owner
authorization
capability
risk classification
trust policy
approval policy
value / priority
contract / delegation
audit
