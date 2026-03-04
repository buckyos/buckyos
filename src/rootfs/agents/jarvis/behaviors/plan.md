# Plan行为提示词

该行为的目的是分析当前状态，并给出下一步动作（todo）

> 在提示词里每一次要求Agent做重大决策，就是在重大的增加系统的不稳定性，这是提示词开发的坏味道。

## Role
<<ROLE>>
...content from role.md...
<</ROLE>>
<<SELF>>
...content from self.md...
<</SELF>>

## process_rule

这是一个允许多Step的行为
这是个步骤的核心是构建todo,并为todo收集足够的信息来完善session summary
这一步，通常会在session中创建workspace，用来做后续的结果交付
构造todo的时候要理解目前有哪些能力可以使用（重点）
    - PLAN阶段的注意力是关注 现状分析+能力分析（扫描tools/skills)，并给出TODO（或者失败）
    - 当能力不够时，可以启用能力扩展模式，此时会创建一个新的session来专注于该工具（能力的主动构建），这个行为需要用户的授权
当前阶段基本不call tool



<<OUTPUT_PROTOCOL>>

<</OUTPUT_PROTOCOL>>

[user]
下面的字段，是编译的，不写就会用编译构造
## Memory
{{agent_memory}}

{{memory_entry}}

## Session Summray
{{session_summary}}

### Todos
- 目前Session/Workspace里的todos情况
{{session_todos}}

### History Input(按时间从旧到新排列)
- 历史沟通记录（按时间排序）
{{chat_history}}

### worklog (steplogs）
可以看到前几步的工作情况摘要（report)


<<INPUT>>


### LastStep 
{{last_step_result}}
<</INPUT>>

[user] (逻辑上的，这个阶段尽量不用tools?)
<<TOOLS>>
...tool specs filtered by allow_list...
<</TOOLS>>


----

上述提示词推理后的典型的输出
1）没有可以做的了：reply报告，更新session summary，nexe_behavior是END
2）一个新session的首次Plan：创建TODO，可能包含PLAN阶段的sub-todo，最终目标是能给出完整的TODO
3) 标准 Plan: 发起Action，更新worklog
4) 收集到了信息后的标准 Plan: 更新worklog,更新session的信息 并将一个sub-todo标记为完成
5) TODO都完成了：切换到check模式
=
Typical LLM return example (JSON only):
{
  "next_behavior": "END",
  "thinking": "work completed for this wakeup",
  "reply": "已完成本轮处理，项目状态已整理。",
  "set_memory": {},
  "shell_commands": []
}

-----

## 本次实现的基本思路（对应 plan.yaml）

这次 `PLAN` 行为配置的核心目标，是把“分析问题并形成可执行计划”做成默认路径，而不是直接进入执行。

1) 阶段定位：先计划，后执行  
- `PLAN` 阶段默认只读，优先做信息收集、任务拆解、风险识别。  
- 非必要不发起 `actions`，避免在计划阶段产生副作用。  

2) 输入组织：围绕“新输入 + 上一步 + 当前待办”  
- 通过 `{{new_msg}}` 感知本轮新需求。  
- 通过 `{{last_step_summary}}` 延续上一步上下文。  
- 通过 `{{workspace.todolist.next_ready_todo}}` 锚定当前最应推进的任务。  

3) 产出形式：以 TODO 和行为切换为主  
- 主输出是可执行的 TODO（目标、依赖、验收标准尽量明确）。  
- 当计划已充分时，切换到 `do`。  
- 可直接收敛时，切换到 `END`。  
- 需用户补充信息或授权时，切换到 `WAIT_FOR_MSG`。  

4) 上下文预算：给计划足够信息密度  
- memory 预算优先覆盖历史消息、TODO、session/workspace 摘要与 worklog。  
- 目的是提升“计划质量”和“可执行性”，减少空泛建议。  

5) 工具策略：默认禁用工具箱  
- 通过 `toolbox.mode: none` 约束 PLAN 阶段偏思考与编排。  
- 需要动作时优先通过 `todo` / `reply` 驱动后续阶段处理。  

6) 与文档对齐  
- 与 `opendanv2.md` 中 8.2 PLAN 定义保持一致：  
  收集必要信息、构建 TODO、必要时请求确认/授权、再进入 `DO` 或 `END`。  
