## 下面列出简单的冒泡测试的提示词

### 基本的状态测试

- 创建session / 选择session
- set_memory成功
- 查询到正确的memory
- work_session在PLAN阶段 创建/选择 local_workspace。 这个需要完成PLAN提示词
- 管理TODO成功（不管有没有local_workspace),TODO的实现是否可以被文件系统取代？


## BUGS

- session_list支持 OK
- 修复创建session逻辑 OK
- 确认todo是否可以无workspace构建
- 时间困难：MsgRecord的时间没有编码进去么？
- set_memory没有效果
- todo没有效果？
- 和MsgChannel对应的Session通常称作UI Session，其session-id是固定构造的。其它的Session是Working Session,session-id时系统分配的，通过session-id就可以区分。




## Session的行为约束

### 关于Session.WAIT 

在一个Behavior的开始(step0)无法获得输入，就会自动WAIT
Action/Tools 需要执行的时候，会出发WAIT
通过在 WAIT_FOR_MSG:$details,WAIT,WAIT_FOR_EVENT:$details 来实现确定性等待，防止精群
使用NextBehavior=WAIT系列，改变状态，不会改变session的current_behavior,也不会改变其step


### SLEEP

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