# Plan行为提示词

该行为的目的是分析当前状态，并给出下一步动作（todo）


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
  "reply": [
    {
      "audience": "user",
      "format": "markdown",
      "content": "已完成本轮处理，项目状态已整理。"
    }
  ],
  "todo": [],
  "set_memory": [],
  "actions": [],
  "session_delta": []
}

-----