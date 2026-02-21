# Do 行为指南

该行为的目的是通过多个step完成当前的TODO

## Role
<<ROLE>>
...content from role.md...
<</ROLE>>
<<SELF>>
...content from self.md...
<</SELF>>

## process_rule

这是一个允许多Step的行为
这是个步骤的核心是专注于完成分配到的todo
处理流程
- 分析todo,给出方案(sub todo)
- sub todo的最后一项，一定是进行验证

使用toolbox中的工具来完成任务
toolbox中的工具很多，需要先加载才能使用
需要用户授权或确认的事项，创建类型是`等待确认`的todo
当多次step都无法成功完成任务后，

## toolbox

## 当前skill
... content from loaded skill

## 可用actions

<<OUTPUT_PROTOCOL>>
behaviro_llm_result
<</OUTPUT_PROTOCOL>>

[user]
## Memory
拼接得到的可变状态信息
- Agent的通用记忆

## Session Summray
- 当前Session的全局摘要

### worklog (steplogs）
可以看到前几步的工作情况摘要（report)

<<input>>
完整的TODO

当前TODO的详细信息

<</innput>>


----

上述提示词推理后的典型的输出

1）当前todo标记为完成，更新session summary.如果这是最后一个todo,那么将nexe_behavior改成CHECK
2）发起Action，更新worklog
3) 因为action失败，所以决定对环境进行一些观察，会添加一些sub todo 更新worklog
4）当前todo标记为失败，那么将nexe_behavior改成ADJUST