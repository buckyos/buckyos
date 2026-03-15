# Agent 持久化

- Agent Loop,需要提供哪些状态持久化设施？
- Agent Loop，如果构造Prompt(提示词工程)

## Agent Enviroment 

Agent的所有状态数据，都保存在Agent的Enviriment中

### Agent Memory

Agent专用的文件夹，保存Agent的所有记忆
- memory.md 最小加载内容（通常在self-improve环节更新）
- Memory的搜索机制（因为是文件系统，所以鼓励使用 bash的find命令去搜索)
- 插入机制：
  - things(包含fact) 的插入机制，该机制带有执行度查询和来源查询
  - 通过新建文件保存任意内容
- Memory的整理机制: 在self-improve环节，会对memory目录的文件进行整理

### Agent (Task) Session

session是LLM时代的基础产品抽象，用户已经基本接受了这种产品形体
session通常代表用户希望AI完成的一项工作。对用户来说是公开的，可浏览的.
session可以用于协作的：比如在一个session中增加一个sub agent,这个sub agent就可以访问这个session里的内容

- 用户给Agent发送消息时，可以显式的声明属于哪个session
- 用户可以手工删除一个session
  - Session被删除时，系统会尽力抹去这个session造成的影响
  - Agent Memory中可以保存和session相关的私有记忆,当session被删除时，哪些记忆也可以被删除

> 如何超越传统的，基于chat history的session持久化？

- 从传统编程的视角来看，session更像一个project
- 支持session over-view
- 支持载入所有的history(包括chat history)
- 如果session 使用了Agent Workshop,那么通过workshop接口可以
  - 管理session的todo list
  - 管理session的worklog
  - 提供对已有“实现/信息”的阅读和检索能力（观察）, 这通常使用文件系统实现
- 如果session 里和多个已有session有关，可以互相建立关联
  - 网状关系比较难管理，更推荐父子关系
  - 例子：父Session是“构造自己的音乐播放器”，子Session是：“修复对mp3格式的兼容性支持"
- 注意session没有完成这个状态，todo才有

> 外界输入 --> LLM Router --> 得到session 这个过程的正确率非常关键！

按上面的流程，大量的记忆都来自session,一旦搞错正确率就会大幅度下降！

### Agent Workshop

这是Agent私有（独占）的工作坊，支持Agent完成被分配到的todo
workshop提供了对对Agent的PDCA循环的支持
workshop的实现和agent runtime高度相关，定义了”Agent搞定任务的手段集合“
workshop是Agent和其sub agent共享的



- workshop有基础能力（Bash+文件系统+Git+浏览器）
- workshop支持Agent在Self-Improve环境打造自己的工具+skills
- workshop支持Agent安装skills
- OpenDAN提供了Agent Workshop的观察UI


### Local Worksapce

用来协同工作和交付结果的地方（对代码任务来说，这是类似一个git repo）
Agent总是有一个私有的Workspace(不会因为协作冲突)

> 独占的workspace又被称作workshop

#### Agent如何在Workspace中恢复`工作进度`

- 对Workspace进行观察，得到一些全局的经验并保存
- 结合worklog和session信息，得到一些session相关(当前task)相关的经验(这些经验通常是动手前的准备，任务完成后就没有了保存的价值)

从经验来看，Agent并不太擅长修改一个复杂的系统，而是擅长构造一个新系统（从头开始）.
如果所有的历史工作都是Agent完成的，那么worklog就足够作为线索了展开了。

## 提示词的构造中使用session状态

### Agent Router 提示词(快速响应提示词)
<TODO>
要求必须能准确的router到一个session,如果不能判断的话，反问用户并收集足够的信息，确认选择后才会离开Router behavior

### behavior提示词

- Agent 角色配置
- 全局的Memory 
- workspace的全局信息(如有)
- 当前behavior的信息 （日后主要开发）

- Session相关 
  - Session Over-view (可能会包括parent session的overview)
  - 从workshop中载入的session相关信息
    - worklogs
    - todo
  - 从workspace中摘入session相关 （如有协作需求）
- Input (Message / Event) History


## 一些结论

- Agent Memory是私有的，穿越Agent全部声明周期的 （用户尽量观察不干预）
- Workshop是属于Agent自己的，最小的工作台（用户尽量观察不干预）
- Session是focus(topic)相关的，偏局部的，偏一次任务的 （用户可以整体性的删除）
- Workspace是 协作相关的，是跨越多个实体（人和Agent），每次工作前都可能变化的（很明显是高级特性） （用户经常日常干预)


### Agent的磁盘结构

agents/$agent_name agent根目录,readonly,安装时的目录，性质类似与 bin/$appname
- behaviors 目录
- skills 目录

data/$agent_name agent的数据目录
- behaviors 目录，自演进后修改的行为，会覆盖安装目录的同名behaviors
- skills 目录，自演进后添加的skills,会覆盖安装目录的同名skill
- enviroment 
  - workspace (准备好了以主机身份进行协作) 
    - $project_name 
  - todo  
  - tools 
  - worklog
- memory
  - calendar.db 
- sessions
  - $sessionid
    - summary.json
- sub-agents









