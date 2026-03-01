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

- Agent Loop的强容错模式：当LLM推理发生后的任何错误，都可以构造成step_summary，等待一会后进入下一个step尝试修复

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




## Review Agent Loop



### ui session,default_behavior = resolve_route

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

### PDCA Work Session default_behavior = plan

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



## 回复信息

核心问题：用哪个Channel? 如果 Agent<->用户 之间存在多个MsgTunnel,那么用哪个
1）原样回复: 如果session由一个msg_tunnle(UI session)创建，那么默认通过这个tunnel回复
2）主动回复：简单的使用contact_mgr提供的last active channel?
  升级成一个todo(意图)，会按完成todo的标准，通过多次step尝试来

- 肯定会写入自己的histroy
- 肯定会吸入ui session的history


## 只用Worklog而不是MsgRecord来处理timeline?


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

SubAgent验证，SubAgent可以在自己的独立容器里并行工作，并且能正确的激活/SLEEP

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