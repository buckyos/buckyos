# 提示词需求

## 整体逻辑

new_msg + new_event
 |
resolve-router -> 快速回应
 |
Plan-Do-Check-Adjust Loop

new_event
 | 
self-improve

### Session 与 Workspace

(task) Session是一个逻辑上的 topic, 用来收拾必要的上下文
workspace则是用来交付成果的地方。worksapce被创建后，其生命周期不与session绑定
查询TODO的时候，是以workspace.load_todo(session_id) 的方式来获取的

### 元工具

OpenDAN的Agent Runtime，有一些tool总是可以用的（不考虑权限问题)
因此可以在process_rule的提示词中稳定的依赖这些tool并指导如何使用 （比如使用文件编辑工具+pyton指令可以非常强大）

- bash工具
- 文件编辑工具
- git工具
- opendan的基础工具

有几个sub agent是已经构建好的，也可以依赖

- 使用浏览器的Agent
- 操作windows的Agent

### 确定Sessionid, 做快速应答

根据Input msg 是否带session_id，走
- resolve_router （可以有多step) 
- router (单step)

next_behavior: PLAN 或 END


### PLAN

PLAN阶段对Workspace是只读的。

- Plan的Input是新的 Session/Message/Event ，这些Input会触发Plan构造新的Todo
- 收集必要的信息（可能需要用户确认）
- 直接作答（利用chat history作为上下文来源）
- 初始化workspace,并将worksapce记录到session中
- 构建TODO List,给每个todo 初始的skills
- 激活SubAgent,开始必要的并行处理（有一些todo被标记为可以直接开始），这个规则要说清楚


PLAN是一个多step的行为，因此提示词里要自然支持对step的状态跟踪
next_behavior: DO 或 END


### DO 

Do阶段的主要是消灭TODO，将TODO的状态变成 Complete或Failed
如果需要在DO阶段展开成PDCA循环，那么就需要把任务指派给sub agent

- Do的Input是有当前可执行Task的todo
  - 如果当前todo的前置任务都是sub agent负责的并行任务，那么会进入等待状态（`无input就会跳过`）
- Do是系统里最常发生的多step行为，会反复迭代“根据Last Step Action的结果决定当前Step的Action
- Do的最后几个Step里，一定会包含简单的自检
- Do内部会做一次自修复的尝试，多次失败后，才会将状态改成Failed
- Do的过程中允许求助外部帮助（虽然我们希望这个行为发生在PLAN阶段）

next_behavior: Check 或 Adjuest


### Check

Check阶段的目标是将TODO的状态从Complete改成Done
Check会将类型是"Bench"的TODO，从 WAIT变成Done (集成测试只在Check阶段做)

- Check的Input是Complete的TODO
- Check也是多Step的
- Review实现的整体结构是否合理，是否于TODO里的文档一致
- Review Workspace里的交付产物，是否覆盖要求和规范
- 遵循 整体验证、基于实现的边缘验证的逻辑
- Check不会做修复，检查到失败立刻标记Task状态为 CHECK_FAILED,然后进入Adjust环节
- Check全部通过后，一般会主动Reply用户

next_behavior: Adjuest 或 END

### Adjuest

因为TODO产生了错误，会进入该行为
该行为的逻辑是对错误原因进行分析后，写入改进意见，或调整计划
Adjust和Plan的角色比较接近

这是一个允许多Step的行为，基本readonly
这是个步骤的核心是深度分析TODO失败的原因，
分析的过程主要聚焦在
- 结合整体和局部review实现路径是否有问题
- 是不是缺乏关键的信息
- 是不是没有足够的技能 
  - 尝试深度翻阅工具箱
  - 要求用户同意自己构建一个新的工具（这是一个新的session)
- 该todo太难

next_behavior: DO 或 END （彻底失败)

## Self-Improve

从结果上来说，Self-Improve的目的是为了让下一个session工作的更好，通常会做下面的工作

- 对Memory进行整理
- 对Session进行整理（各种History）
- 对Workshop进行整理 

这里的整理的本质是对 Memory相关的提示词进行压缩，这个压缩的行为也可能在Plan阶段触发

###  扩展 tool,skill,subagent

5）如果有成本空间，则尝试自己打造新的工具
6) 如果有成本空间，则尝试自己构建新的SubAgent

### 升级认知

4）对agent的self.md进行修改，调整自己的工作方式

### 整理Knowledge Base

这个工作是系统的一个专门维护Knowledge Base的Agent负责，普通Agent只需要在Self-Improve阶段给其发信息说需求就好了

## 如何实现等待用户确认?(Agent心智模型)

等待用户补充信息:使用send_msg主动和用户沟通，请求补充帮助。随后用户通过msg补充信息
等待用户授权:使用send_msg主动和用户沟通，请求授权。 用户可以用系统命令浏览等待授权的行为，并通过相关授权

Agent的心智模型如下

```python
def behavior_step()
    input = generate_input(session_state)
    llm_reuslt = llm(input)
    if llm_result.is_wait():
        wait_for_input(llm_result.wait_events)
    else:
        do_next_step(llm_result.next)
```

Agent在一个session里是串行处理的，也就是说，同时只会在一个Behavior的一个Step中。
一次推流后，可以设置为挂起。如果事件到了，但是并没有获得足够的信息，那么会浪费一次推理（推理结果又是wait)
上述挂起不会挂起并行的SubAgent的流程，SubAgent可以继续推进

## Agent运行收到新的User Message

按上面的心智模型，每一个Step在收集input的时候，都会看到新的message,会影响下一个step的处理
有时也可以在系统的事件处理里，强制将Agent的 behavior切换到route上去

## 系统中断后的恢复

一次LLM推理是昂贵的，因此每次推理完成后，都会立刻保存状态，这样系统故障后可以继续充上一次step中恢复。
唯一不能恢复的是tool call,此时被视为系统级别的支持llm 推理中断，并没有完成推理

按该要求，Agent Loop里，Agent的Current Session,Current Behavior,以及每次generate_input(session_state)，都能在系统重启后恢复


## 系统构造提示词的逻辑

### System Prompt (在Step中是静态的)

- <<role>>段，加载agent的role.md和self.md
- <<process_rules>>段，加载当前behavior的process_rule段
- <<policy>>段，加载当前behavior的policy段
- <<toolbox>>段，加载当前behavior的 可用tool+Skill配置
- <<output_protocol>>段，behavior只需要配置2种模式就好了(BehaviorLLMResult和RouerResult),系统会自动填充

### User Prompt
- <<Memory>>段， 这根据配置进行编译的核心段落，系统会按优先级和预算自适应的编入记录的状态信息
  - AgentMemory
  - Session Summray
  - History Message
  - Worksapce Summary
  - Workspace Worklog
  - Workspace Todo
- <<Input>>段，非常关键，不同的模式，对Input是不同的，如果无法得到Input，那么这个Step会被跳过
  - new msg (处理过后，除非手工掉Action，否则这些msg会被设置为readed) 
  - new event
  - Current Todo Details
  - LastStep Summray
  

## 完成各个行为配置的重点

- 编写process_rule （固定提示词）
- 编写policy （固定提示词）

- 配置可用toolbox （tool + skills) （目录)
- 已装载的tool/skills (tool名列表,skill名)

- 配置Memory提示词的结构，需要包含哪些段落，段落对token使用的限制
- 配置input提示词的结构(模板组合)

