## 下面列出简单的冒泡测试的提示词

### 基本的状态测试

- 创建session / 选择session 
- set_memory成功 OK 
- 查询到正确的memory OK 

- work_session在PLAN阶段 创建/选择 local_workspace。 这个需要完成PLAN提示词
- 管理TODO成功（不管有没有local_workspace),TODO的实现是否可以被文件系统取代？
- 能正确加载skill （PLAN阶段能给更多信息）
- 能正确构造Action并执行
- 能正确完成一个TODO
- 使用文生图，正确得到一张图（用action方式)

## BUGS


- 大规模的简化提示词输出

 Session Summary是重要的（里面估计有WorkSpace Summary)
 Workspace Summary去掉
 LastStep最为重要
Action:
  bind_external_workspace $target_workspace_name
  bind_workspace $target_workspace_name
  create_local_workspace $workspace_name
  edit $path


- 完成数据区的整理（等搞sub agent的时候一起）
  扫盘，找到需要运行的Agent
- Agent Loop的强容错模式：当LLM推理发生后的任何错误，都可以构造成step_summary，等待一会后进入下一个step尝试修复
- Reply要更智能，在Session Summary中说明reply的效果
- session_list支持 OK 检查Render，需要看到状态
- 修复创建session逻辑 OK
- 和MsgChannel对应的Session通常称作UI Session，其session-id是固定构造的。其它的Session是Working Session,session-id时系统分配的，通过session-id就可以区分。 OK
- new_msg的bug OK
- 能正确加载msg history  OK
- new_msg在提示词中能正确显示 OK 
- 简单消息route的问题：如何查看历史记录？
  - Create session的时候多带几个message过去
  - 强制带上之前的8条消息（如果有的话）


- todo没有效果？
  - 确认todo是否可以无workspace构建 不能
  - 需要实现两个Result提示词，标准的和RouteResult还是不同的

- 时间困难：MsgRecord的时间没有编码进去么？

- set_memory没有效果
- 两种llm result模式


## Review Tool Use

选择Skills意味着 列出所有Skills,读取看看，以及进行选择，所以是个大活。只推进在Plan里配置
Plan 配置好后，会在TODO中设置好Skills, 使用next_behavior=DO:todo=T01

Behavior里不内置动态加载机制：配置是什么就是什么

推导toolbox依赖：
- 系统里的所有action,tool
- behavior 附加的 skills (为什么不是session当前的skills?)
- skills里会有一个列表
- 配置文件里item list(可以是deny模式)

最终，toolbox里要得到的是
- 可用的action列表
- 可用的tool列表
- 可用的cmd列表

机制很丰富，但也许对behavior的作者来说，配置一个skills最简单可控

- 当前加载了哪些Skills 是 Session的状态
- 切换behavior无非是3种结果
 - alone:替换成behavior配置的skill (清零逻辑) 
 - inherbit:合并到当前skill(追加逻辑)

```yaml
toolbox:
    mode: alone
    skills: ["coding/rust"] # 这个例子没有default_load_skills，纯拼
    default_allow_functions: ["read_file","bash"] # 在function里使用read tool
```

skills文件的定义
- process_rule定义
- process_rule中用到的cli的说明
- allow_cmds[""]

### 理想的提示词

<<process_rule>>



使用Linux Bash工具，在一个Linux Bash环境中完成工作，该环境中已经预装了主流的cli 工具可直接调用。


<<output_protoco>>


结构介绍
说明actions结构 ： 执行bash命令
针对write/edit文件的优化

remove <<toolbox>> section

### output_protocol修改
类型列表
- router
- 带acton的bash （现在的版本）
- 不带action的bash(文件系统只读)



### TODO Mgr 的简化 （OK）

TODO cli化


## Review OpenDAN的API Gateway和调试控制台


## Review Agent Loop

### Msg Input & Reply & History Record

- rouer dispathcer的时候，别说话。只有在创建的时候才说话
- route -> session 导致的double LLM的开销问题:
  - 当前topic驱动自动route？可以先用成本换效果
  - 效果真的好么
- 简化reply
  - 当前session有default reply对象 OK
  - 当不与default对象通信时，需要用action（此时会变成系统里的一个意图？）
- 简化Msg Prompt Render(Input和History是一样的)
  - 使用Nickname(did)+time的方法 说明消息的来源
  - 自己回复的消息用 Me + time 渲染
- 上述逻辑是否对群聊有效
  - 群聊获得Input的逻辑是被@，但一旦获得，就会往前读多条消息
  - 群session和其子session的default reply = groupid,但发送消息时，需要说明是否@某人
- 查看历史记录的问题：
  - ui session查看历史记录很简单：所有的消息都能看到
  - work session查看历史消息记录，只看和自己有关的(session是我们缩小状态范围的目标)
  - work session触发reply后，ui和自己都能看到？
  - Work session不再看到


#### 回复信息

核心问题：用哪个Channel? 如果 Agent<->用户 之间存在多个MsgTunnel,那么用哪个
1）原样回复: 如果session由一个msg_tunnle(UI session)创建，那么默认通过这个tunnel回复
2）主动回复：简单的使用contact_mgr提供的last active channel?
  升级成一个todo(意图)，会按完成todo的标准，通过多次step尝试来

- 肯定会写入自己的histroy
- 肯定会吸入ui session的history

### 强授权系统（和workflow结合）

强授权系统是被动的，是基于session的
当session操作一个action时，可能会被动触发一个 授权需求，并强制进入WAIT_FOR(Taskid状态+超时)
当该Taskid的状态改变，Action执行才会返回
用户会在自己的个人Task管理中心，看到所有等待确认的Task

Agent加入workflow:
session是这个workflow的 instance
Agent的llm_input，来自workflow的input
Agent的llm_result.action, 成为workflow的ouput

> 定制重点： 定制session, 定制behavior提示词，定制

### 失败处理
结果上就两种
1. 如果没有任何side-effect : 回滚当前step,相当于没发生
2. 产生正常的siede-effect，当前step被标记为失败事实，计入时间线，触发自动reply,并可以正常执行下一个step

### 正确处理文档中的Claim

`Agent打算做一个独占的写操作，才会触发Claim`
`claim 失败意味着另一个 session 已经在执行同类外部动作：`

目前来看没有实际需求（没有办法假设强UI的支持），先Later
要区分 Agent/Session/Worksapce这3个粒度的授权


### Prompt细节优化

- Input的前缀，能随着行为不同而不同

## 只用Worklog而不是MsgRecord来处理timeline?
 
- UI Session和WorkSession的侧重点不同
- 现在Worklog的内容还需要细化 : 很明显，是基于input和step_summary构造的提示词

```提示词例子
- 时间 收到来自xxx的信息:
- 时间 我想：xxxx
  - Reply: xxxxx
  - write_file index.html : 写完了主框架
- 时间 Step总结
  - 动作 结果简介
  - 动作 结果简介

## 上一步的结果

- read_file b.index 0:100 => OK  ，result:
```
xxxxx
xxx
xxxxxxx
xxx
xxxxxxxxxx
```
- write_file index.html => OK, write 435 bytes(+29 lines)
- edit_file b.js => OK,replace 23 lines
- git diff d1e711b7  -- agent_tool.rs => OK,result:
```
diff --git a/src/frame/opendan/src/agent_tool.rs b/src/frame/opendan/src/agent_tool.rs ==
index 6325c39e..4f1212b1 100644
--- a/src/frame/opendan/src/agent_tool.rs
+++ b/src/frame/opendan/src/agent_tool.rs
@@ -291,6 +291,13 @@ pub(crate) fn normalize_tool_name(name: &str) -> String {
         .to_string()
 }
 
+pub struct AgentToolResult {
+    pub exit_code: i32,
+    pub full_cmd_line:String,
+    pub result:Option<String>,
+    pub error:Option<String>,
+}
+
 #[async_trait]
 pub trait AgentTool: Send + Sync {
     fn spec(&self) -> ToolSpec;
```
### Loop 例子

核心点：
generaete_input() 该函数成功才会进入LLM,可以减少timer刷新状态->LLM判断没有新东西要处理的问题
系统要为generaete_input提供通用设计
- pull到msgbox
- taskmgr更新
- TODO有更新
- 文件有修改（最通用）
- 对象有修改：系统所有的子系统，都支持用 url查询状态


#### ui session,default_behavior = resolve_route

- session.current_behavior = DefaultBehavior ()
=> new msg
- resolve_route(0) : gen_input()->new_msg->llm_result.next_behavior=END, switch state to Sleep
  - session.current_behavior = DefaultBehavior (),step=0
=> new msg
- resolve_route(0) : gen_input()->new_msg->llm_result.next_behavior=None,
  - session.current_behavior=resolve_route,session.step=1
- resolve_route(1) : gen_input()->last_step_summary->llm_result.next_behavior=WAIT_FOR_MSG,switch state to WaitForMsg
  - session.current_behavior=resolve_route,session.step=2
=> new msg
- resovle_route(2) : gen_input()->last_step_summary,new_msg->llm_result.next_behavior=END,switch state to Sleep
  - session.current_behavior = DefaultBehavior (),step=0

#### PDCA Work Session default_behavior = plan

P-D-C-D-C-A-D-C-D-C-END
P后面总是D
D后面总是C
C后面可以是D也可以是A，也是是END（最复杂）
A后面总是D

get_next_ready_do 似乎没啥用了（这个是给下循环准备的
P-D-D-C-END (全部是序列化任务，挨个做完)

PDCA实例：

- session.current_behavior = DefaultBehavior ()
=> timeout
- plan(0) : gen_input->new_msgs->llm_result.next_behavior=None,
  - session.current_behavior=plan,session.step=1
- plan(1) : gen_ipput->last_step_summary->llm_result.next_behavior=Do,
  - workspace.todolis_init()
  - session.current_behavior=do:t1,session.step=0
- do:T1(0) : gen_input->do_item->llm_result.next_behavior = None,
  - session.current_behavior=do:t1,session.step=1
- do:T1(1) : gen_input->last_step_summary,new_msg->llm_result.next_behvior = WAIT_FOR_MSG (需要用户补充信息)
  - session.current_behavior=do:t1,sesioon.step=2
=> new msg （这是ui session route过来的）
- do:T1(2) : gen_input->last_step_summary,new_msg->llm_result.next_behvior = Check,
  - session.current_behavior=check,sesioon.step=0
- check:(0): gen_input->todolist->llm_result.next_behavior = do:T2
  - session.current_behavior=do:T2,session.step=0
- do:T2(0) : gen_input->do_item->llm_result.next_behvior = check,
  - session.current_behavior=check,sesioon.step=0



### 关于Session.WAIT 

当前behavior没有有效输入，需要等待有效输入。 
nex-behavior 的 LLM 结果
  None(不切换继续Step)
  END（SLEEP等待下次唤醒，唤醒后会变成Session Default Behavior）
  WAIT_FOR_MSG: LLM认为需要得到用户输入的二次确认才能继续。
  使用NextBehavior=WAIT系列，改变状态，不会改变session的current_behavior,也不会改变其step
   
如果gen-input失败，则会进入WAIT
如果call tool或call action触发授权请求，进入WAIT_FOR_EVENT 

* behavior因为step超过限制停止时，session该怎么办？
  本质上是behavior失败怎么办：构造一个失败的last_step_summary，根据behavior配置fall_behavior,强制切换
* input里用了last_step_summary,可以无限触发怎办？
  inpu里有last_step_summary的意义就是推进到结束或用户回复确认

### SLEEP

从SLEEP变成Ready，会重置到该session的default behavior (丢弃旧循环从头开始)
在StepSummary里引入Agent的 HP / TokenUsage信息
鼓励Agent做出简单的输出，并进行适当的休息
这个特性对SubAgent很重要




## 完成6个行为提示词的开发

- 默认行为:router_reslove,是否要把消息投递到新的session
- SelfImprove:
  - 测试是否能有效的通过LLM实现各种历史记录的压缩和Summary的整理
  - 能否构建自己的兵器库：编写常用py/js代码
  - 基于SubAgent Skill 增加新的SubAgent

- PDCA
  - Plan: 能够正确的给DO预选SKILL
  - Do:长Step，能真正完成
  - Check:如何触发？在Do里触发有点浪费提示词，应该是一个“找不到可用的Do后的自动触发，比较适合Plan里触发”
  - Adjust: 实际调整的主要手段有哪些？

## SubAgent

将一个todo 指派给subagent,是常见的范式.
此时subagent 的input来自 被指派todo的状态变化

- SubAgent有自己独立的Agent状态（但也可以很容易创建new instance)
  清空记忆有的时候有有清空记忆的美

- SubAgent通常在一个指定的Session工作（纯WorkSession)
  该WorkSession创建时，可以集成一些来源Session的信息，但后续互不影响
- SubAgent在自己的Local Workspace里工作，可以把工作结果update_todo 到父session的todo.item
  - Local Workspace绝对不会并行，并且只会别一个Agent使用
- SubAgent无法收到来自用户的直接指令和Msg，但会收到Pause的影响
- SubAgent不能使用reply来反馈信息，静默工作模式
- SubAgent所需要的授权来自系统，等待逻辑相同



## 构建OpenDAN的自动化集成测试框架


- 实现mock aicc （现有aicc通过判断配置文件是否存在即进入mock模式）
- 实现强类型的剧本生成器，里面有大量的类型的序列化反序列化代码
- 系统启动后，通过python给msg_cener的agent 消息端口发消息实现测试启动



## 实体协作

TODO中的一个item，分配给了其它的实体（通过SendMsg沟通）
PLAN的时候要能知道哪些实体可以给予哪些帮助，并在此基础上考虑任务分配
主Agent在WAIT状态时，能定期的通过SendMsg进行沟通，拿到并验证交付物后，将TODO标记为完成

- Agent需要能访问contact_mgr


## 多模态的支持

为文生图，文件传输等打通Message系列协议

- 先从单条非文本信息开始 
- 内部的MsgObject怎么定义，怎么把图片传给aicc
- Tg msg tunnel怎么实现，如何把图片放到msg object里,如何发送带有图片的msg object
- aicc如何使用text-to-img得到图片，并嵌入到msg object中 

## 最终于UI良好对接
