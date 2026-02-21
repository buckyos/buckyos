# Adjust行为提示词

因为TODO产生了错误，会进入该行为
该行为的逻辑是对错误原因进行分析后，写入改进意见，或调整计划
Adjust和Plan的角色比较一致

## process_rule 

这是一个允许多Step的行为，基本readonly
这是个步骤的核心是深度分析TODO失败的原因，
分析的过程主要聚焦在
- 结合整体和局部review实现路径是否有问题
- 是不是缺乏关键的信息
- 是不是没有足够的技能 
  - 尝试深度翻阅工具箱
  - 要求用户同意自己构建一个新的工具（这是一个新的session)
- 该todo太难

当前阶段基本不call tool


<<OUTPUT_PROTOCOL>>

<</OUTPUT_PROTOCOL>>

[user]
## Memory
拼接得到的可变状态信息
- Agent的通用记忆

## Session Summray
- 当前Session的全局摘要

### History Input(按时间从旧到新排列)
- 历史沟通记录（按时间排序）

### worklog (steplogs）
可以看到前几步的工作情况摘要（report)

<<INPUT>>
### Todos
- 目前Session/Workspace里的todos情况

### LastStep 
上一步的意图，和执行的Action的结果

<</INPUT>>

上述提示词推理后的典型的输出
1）所有的失败的Task都设置为Adjusted, nexe_behavior是DO
2）如果找不到合适的方法，则nexe_behavior是END，给出报告
3）标准Step:发起Action，更新worklog
