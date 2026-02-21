# Check行为提示词

该行为的主要目的，是对已经比较为完成的任务进行整体性的确认

## process_rule

这是一个允许多Step的行为。
Review实现的整体结构是否合理，是否于TODO里的文档一致
Review Workspace里的交付产物，是否覆盖要求和规范
遵循 整体验证、基于实现的边缘验证的逻辑

<<OUTPUT_PROTOCOL>>

<</OUTPUT_PROTOCOL>>

[user]
## Memory
拼接得到的可变状态信息
- Agent的通用记忆

## Session Summray
- 当前Session的全局摘要

### Todo
- 目前Session/Workspace里的todos情况
- 展开的Checklist todo

### History Input(按时间从旧到新排列)
- 历史沟通记录（按时间排序）

### worklog (steplogs）
可以看到前几步的工作情况摘要（report)


<<INPUT>>


### LastStep 
上一步的意图，和执行的Action的结果

<</INPUT>>

[user] (逻辑上的，这个阶段尽量不用tools?)
<<TOOLS>>
...tool specs filtered by allow_list...
<</TOOLS>>

----
1）当前TODO标记为Done，如果所有TODO的状态都是DONE了，：reply完成报告，更新session summary，nexe_behavior是END
2）发起Action，更新worklog
3）根据Action的结果，判断当前check是否通过
4）所有TODO都完成标记后，或则Check失败的任务超过3个后，将nexe_behavior改成Adjust
