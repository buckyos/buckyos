# OpenDAN Agent Runtime 设计介绍

本文结合opendan默认的jarvis agent的实现，说明整个OpenDAN Agent Runtime的设计

## 默认Agent Jarvis的逻辑

new_msg + new_event
 |
`resolve-router` -> 快速回应
 |
`Plan-Do-Check-Adjust Loop`

new_event
 | 
`self-improve`

Jarvis虽然是默认的Agent，但也是在runtime的支持下，通过定义各个behavior的配置，实现了预定的逻辑
对Runtime来说，主要就是提供运行机制和运行容器

- 运行容器和机制(Agent Loop + Agent Session)
  - Sub Agent是高级的运行机制
- Behavior 配置和 LLM Input / Ouput 标准化
- 状态管理
  - Agent Session （系统管理更新,Agent通常是读)
  - Workspace (Agent可以自由创作的文件系统）
    - Worklog
    - Todo
    - git
    - filesystem
- 元工具集合 即使没有Workspace,Agent也可以通过元工具完成一些任务


## Session & Workspace & Workshop

(task) Session是一个逻辑上的 topic, 用来归并必要的上下文
workspace 是用来交付成果的地方。不被runtime管辖，有时是公网的服务。可以通过skills扩展
wokrshop 是Agent私有的工作区，在workshop内，可以创建多个私有的workspace(一般称作code_workspace,或local_workspace)
local_workspace 如果是在session运行中，agent创建的，通常是其私有的，如果是用户先创建local workspace再让Agent来来完成任务，那么这个local workspace通常是属于用户

***session 可以绑定 0 个 local_workspace 和 0..n个 workspace***

- 当session需要一个确定的，用来运行各种元工具的worksapce时，总是可以在agent的workshop里创建local_workspace成功
- session在需要local_workspace时，应仔细考虑是创建还是使用已有的local_workspace
- session的任务里如果包含外部协作（比如修复github上的bug),那么有机会通过skills，绑定(remote) workspace

查询TODO List的时候，

- 有local_worksapce 是以workspace.load_todo(session_id) 的方式来获取的
- 没有则是session.load_todo() 的方式来获取
  **这个路径是典型的基础设施支持，但逻辑上不会走到，因为需要todo的时候通常已经在PLAN阶段选择workspace了**

agent_session也是agent执行的主要逻辑容器。如果把执行llm看成一次AI时代的CPU调用，那么

- agent_session相当于传统的thread,是系统运行llm的容器。session内的llm调用总是顺序执行的
- agent_session的一次执行的最小粒度是 behavior step, 每次执行后都会保存session.state
- 如果agent_session的执行不依赖任何外部环境，那么给定behavior_name + step,就可以在系统重启后继续运行,也可以回退到上一个step重新执行
- 每个behavior在执行的时候，通常都会读取new_msg和new_input，因此外界对当前session的影响，最慢会在下一个step产生。
- local_workspace锁 agent_session可以并行执行，但如果两个agent_session使用同一个local_worksapce,则这两个agent_session只有一个能处于Running状态
  **这个约束可以更强:需要等待另一个session的所有todo都完成/失败** , 非顺序性的修改local_workspace也会带来负面

### Sub Agent

agent于agent之间相当于进程级别的隔离，因此系统总是允许agent独立运行,这也是系统目前确定性的并行来源
目前sub agent是系统构造新agent的主要手段，但为了防止复杂的状态冲突,sub agent的状态共享逻辑要非常细腻

- session
  - subagent对agent session是只读的（但通常不读）
  - subagent会创建自己的subagent session,在这个session里专注完成自己的工作
- workspace
  - subagent可以读取agent的todo,可以更新自己的todo状态
  - worklog subagent总是可以append worklog
  
### Sub Agent Session

Sub Agent session简称 Sub Session

- Sub Session可以得到Parent Session的信息
- 在Pause Agent Session的时候，会Pause其所有的Sub Session
- Resume的时候也一样,会Resume所有处于Pause状态的所有Sub Session


### 元工具

OpenDAN的Agent Runtime，有一些tool总是可以用的（不考虑权限问题)
因此可以在process_rule的提示词中稳定的依赖这些tool并指导如何使用 （比如使用文件编辑工具+pyton指令可以非常强大）

- bash工具 为了用好bash工具，session有cwd的概念，并且包含了一些环境变量，比如访问workspace目录)
- 文件编辑工具 用文件编辑工具可以更好的
- git工具 
- opendan的基础工具


有几个sub agent是已经构建好的，也可以依赖

- 使用浏览器的Agent
- 操作windows的Agent

## Jarvis的Behavior详解

### resolve_router

> 目标 确定Sessionid, 做快速应答

根据Input msg 是否带session_id，走
- resolve_router （可以有多step)
- router (单step)

next_behavior: PLAN 或 END


### PLAN

> 目标 收集信息，制定可行的计划

PLAN阶段对Workspace是只读的。

- Plan的Input是新的 Session/Message/Event ，这些Input会触发Plan构造新的Todo
- 收集必要的信息（可能需要用户确认）
- 直接作答（利用chat history作为上下文来源）
- 创建/选择 workspace,并将worksapce记录到session中
- 构建TODO List,给每个todo 初始的skills
  - 将Todo分配给SubAgent,开始必要的并行处理（有一些todo被标记为可以直接开始）
  - 将Todo分配给外部（求助外部），尝试等待Msg
  - Task的前置任务是前一个Task，但是DelegateTask的前置任务不能是DelegateTask（让DelegateTask能并行运行）

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

### Self-Improve

从结果上来说，Self-Improve的目的是为了让下一个session工作的更好，通常会做下面的工作

- 对Memory进行整理
- 对Session进行整理（各种History）
- 对Workshop进行整理 

这里的整理的本质是对 Memory相关的提示词进行压缩，这个压缩的行为也可能在Plan阶段触发

####  扩展 tool,skill,subagent

- 如果有成本空间，则尝试自己打造新的工具
- 如果有成本空间，则尝试自己构建新的SubAgent

#### 升级认知

- 对agent的self.md进行修改，调整自己的工作方式

#### 整理Knowledge Base

这个工作是系统的一个专门维护Knowledge Base的Agent负责，普通Agent只需要在Self-Improve阶段给其发信息说需求就好了

## 如何实现等待用户确认?

等待用户补充信息:使用send_msg主动和用户沟通，请求补充帮助。随后用户通过msg补充信息
等待用户授权:使用send_msg主动和用户沟通，请求授权。 用户可以用系统命令浏览等待授权的行为，授权或deny后，会产生event
因为有整体沙盒的缘故，OpenDAN需要用户授权的操作相对会更少一些，让整个处理流程更流畅

## Agent Session运行收到新的User Message

Agent 每一个Step在收集input，因此都会看到新的message,会影响下一个step的处理
有时也可以在系统的事件处理里，强制将Agent的 behavior切换到route上去
注意对Message的状态管理

- Agent Loop收到信息，处理后分派到session : msg的状态是readed,但是对session来说是new_msg
- session在一次LLM中看过msg了，此时msg变成history_msg(不会在input中)

## Agent是如何管理用户日程的

Agent注册了Timer Event,因此每过3分钟，就会被Event唤醒一次
此时Agent会读取自己的Memory,发现有“需要提醒主人的事项” ，此时会触发Agent SendMsg,并根据该提醒是一次性的还是多次的，绝对是否需要通过set_memory删除该记录。

## 系统中断后的恢复

一次LLM推理是昂贵的，因此每次推理完成后，都会立刻保存状态，这样系统故障后可以继续充上一次step中恢复。
唯一不能恢复的是tool call,此时被视为系统级别的支持llm 推理中断，（并没有完成推理，不是一个正确的step)
在构造哦BehaviorExecInput的时候，只有last_step_summary是直接恢复的，在每个step中，Agent都不应该通过worklog来推workspace的状态，而是需要自己先观察再确定。


## Agent Loop & Session Loop

> 虽然我们计划使用PDCA 作为主循环逻辑，但对系统来说，没有任何behavior的逻辑是被假设存在的

Session的状态图:

- PAUSE 用户手工暂停，
- WAIT 标准WAIT，任何事件都可以唤醒
- WAIT_FOR_MSG, 等待一个特定msg，超时后会变成READY
- WAIT_FOR_EVENT,等待一个特定的event，超时后会变成READY
- READY 就绪，等待执行
- RUNNING 正在执行中
- SLEEP 长期没有获得过有效输入，就会变成SLEEP状态


Agent Loop 的心智模型如下

```python

class AIAgent:
    def dispatch_msg_to_session(self,msg_pack):
        session = self.get_session(msg_pack.session_id)
        session.push_msg(msg_pack)
        if session.state == "WAIT":
            session.update_state("READY")
        if session.state == "WAIT_USER_MSG":
            # 有某个人的消息
            if session.waits.have_msg(msg_pack):
                session.update_state("READY")

    def dispatch_event_to_session(self,event_pack):
        session = self.get_session(event_pack.session_id)
        session.push_event(event_pack)
        if session.state == "WAIT":
            session.update_state("READY")
        if session.state == "WAIT_EVENT":
            # 有等待的事件
            if session.waits.have_event(event_pack)
                session.update_state("READY")

    def session_run_thread(self):
        #标准调度，将session的状态从READY变成RUNNING，一个session只会被分配到一个run thread
        while self.running:
            next_run_session = self.wait_next_ready_session()
            next_run_session.update_state("RUNNING")
            if next_run_session.current_behavior == None:
                # 通常默认的dehavior 是  router_pass,期望能快速处理input
                next_run_session.current_behavior = self.default_behavior
                next_run_session.step_index = 0
            behavior_cfg = load_behavior(next_run_session.current_behavior)

            while True:
                self.run_behavior_step(behavior_cfg,next_run_session)
                next_run_session.save()
                if not next_run_session.state == "RUNNING":
                    break

    def schedule_sessions(self):
        # 把处在在WAIT_MSG,WAIT_EVENT很长时间的session状态，设置为READY
        update_wait_timeout_sessions()
        

    def run_agent_loop(self):
        for i in self.max_session_parallel:
            thread.spawn(self.session_run_thread)

        while self.running:
            msg_pack,event_pack = self.pull_msg_and_events()
            if msg_pack.session_id is None:
                # pull_msg的实现已经做了精细的分pack
                route_result = self.llm_route_resolve(msg_pack) 
                msg_pack.session_id = route_result.session_id
            # 会根据session的WAIT的细节，将session状态变成READY
            dispatch_msg_to_session(msg_pack)
            self.set_msg_readed(msg_pack)
            
            if event_pack.session_id is None:
                route_result = self.llm_route_resolve(event_pack)
            # 会根据session的WAIT的细节，将session状态变成READY
            dispatch_event_to_session(event_pack)
            self.set_event_readed(event_pack)

            #系统也可以根据优先级，将run_session的状态从RUNNING变成READY,让出执行空间
            self.schedule_sessions()


    def run_behavior_step(behavior_cfg,session)
        behavior_exec_input = session.generate_input(behavior_cfg)
        # 没有有效的输入，没必要触发llm
        if behavior_exec_input == None:
            session.state = "WAIT"
            return

        prompt = behavior_cfg.build_prompt(behavior_exec_input)

        # 执行LLM,此时所有的状态信息，都编码到了prompt中
        behavior_llm_reuslt = behavior_cfg.do_llm_inference(prompt)
        
        # 仔细的处理behavior_llm_reuslt
        # 对外回复
        for msg in behavior_llm_reuslt.reply
            # 处理时，有默认默认"收件人的概念“，大部分session里，都有default_send_msg_info
            self.do_reply_msg(msg)

        # Action调用 (这里绝对不处理tool_calls,这是do_llm_inference内部处理的)
        action_result = self.do_action(behavior_cfg,behavior_llm_reuslt.actions)
        # 创建当前step的worklog
        worklog = self.create_step_worklog(action_result,behavior_llm_reuslt)
        session.append_worklog(worklog)
        session.last_step_summary = build_step_summary(behavior_llm_reuslt.thinking,worklog)
        # 消费input
        session.update_input_used(behavior_exec_input)

        if session.workspace_info:
            workspace = get_workspace(session.workspace_info)
            workspace.append_worklog(worklog)
            # TODO patch
            if behavior_llm_reuslt.todo_delta:
                workspace.apply_todo_delta(behavior_llm_reuslt.todo_delta,session)

        # Memory 
        if behavior_llm_reuslt.set_memory:
            self.apply_set_memory(behavior_llm_reuslt.set_memory)

        # Session meta patch
        if behavior_llm_reuslt.session_delta:
            session.update(behavior_llm_reuslt.session_delta)

        if behavior_llm_reuslt.next_behavior:
            # WAIT 处理
            if behavior_llm_reuslt.next_behavior = "WAIT":
                session.set_wait_state(behavior_llm_reuslt.wait_details)
            # 让session让出执行的调度逻辑:
            elif behavior_llm_reuslt.next_behavior == "END"
                session.state = "WAIT"
            else:
                session.current_behavior = behavior_llm_reuslt.next_behavior
                session.step_index = 0
        else:
            # 未设置就进入下一个step
            session.step_index += 1
            if session.step_index > behavior_cfg.step_limit:
                session.current_behavior = self.defulat_behavior
                session.step_index = 0 
                session.state = "WAIT"

    # 在apply_todo_delta内部，有可能把todo分配给sub_agent
    def dispatch_todo_to_sub_agent(sub_agent_id,todo_item,session):
        sub_agent = get_sub_agent(sub_agent_id)
        #刚刚创建出来的session默认状态为WAIT
        sub_session = sub_agent.create_sub_session(session.id)
        event_pack = create_todo_changed_event(todo_item)
        event_pack.session_id = sub_session
        sub_agent.dispatch_event_to_session(event_pack)
        sub_agent.running = True
```

Agent在一个session里是串行处理的，也就是说，同时只会在一个Behavior的一个Step中。
一次推理后，可以设置为挂起(WAIT)。如果期待事件到了，但是并没有获得足够的信息，那么会浪费一次推理（推理结果又是WAIT)
上述挂起不会挂起并行的SubAgent的流程，SubAgent可以继续推进



## 从BehaviorCfg中构造提示词的逻辑

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
  - LastStep Summray,包括现在已经已经运行了多少个step的成本信息
  

## 提示词工程师完成各个行为配置的重点

- 编写process_rule 
- 编写policy 

- 配置可用toolbox （tool + skills) （目录)
- 默认装载的tool/skills (tool名列表,skill名)

- 配置Memory提示词的结构，需要包含哪些段落，段落对token使用的限制
- 配置input提示词的结构(模板组合)

### 一个典型的配置

下面是一个典型的route behavior配置，此时input已经有了明确的session_id

```yaml
process_rule: | # 最后使用模版替换，载入了当前session的worksapce 目录里的一个特定文件和session current dir的一个特定文件
  ### next_behavior 决策流程

  1. **琐碎 / 无实质内容的输入**（问候、确认、表情符号、无实质内容的闲聊）：
  * 立即给出一个简短、自然的回复。
  * 将 `next_behavior` 设置为 `END`。


  2. **非琐碎输入**（包含请求、问题、任务或有意义的内容）：
  * 将 `next_behavior` 设置为 `PLAN`。

  3. **记忆查询：**
  * 如果所选行为能从检索相关的过去背景中受益，请在 `memory_queries` 中填充简明扼要的搜索字符串。
  * 否则使用 `[]`。

  {{workspace/to_agent.md}}
  {{cwd/to_agent.md}}


policy: |
  * 仅输出 JSON 对象——不得包含其他内容。
  * 必须包含所有三个字段。
  * `memory_queries` 必须是一个数组；为空时使用 `[]`

output_protocol:
  mode: RouteResult

# 不允许是用任何工具，限制了该behavior只能快速的完成
# toolbox: 
#   skills:
#     - opendan.default 

memory: # build提示词时，会对该部分进行动态调整
  total_limt: 12000
  global_memory:
    limit: 0  # 0代表无限制
  agent_memory:
    limit: 3000
  history_messages:
    limit: 3000
    max_percent: 0.3 # 当有两个限制时，取实际上最小的
  session_summaries:
    limit: 6000
  
input: | # 这里使用模版替换，如果两个模版替换得到的都是Null，则无input
  {{new_event}}
  {{new_msg}}
  

limits: # 该behavior的run limit,不会进入提示词控制
  max_tool_rounds: 1
  max_tool_calls_per_round: 4
  deadline_ms: 45000
```

