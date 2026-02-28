

## 关于WAIT 

在一个Behavior的开始(step0)无法获得输入，就会自动WAIT
Action/Tools 需要执行的时候，会出发WAIT
通过在 WAIT_FOR_MSG:$details,WAIT,WAIT_FOR_EVENT:$details 来实现确定性等待，防止精群


## SLEEP

在StepSummary里引入Agent的 HP / TokenUsage信息
鼓励Agent做出简单的输出，并进行适当的休息


## 完成6个提示词

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


## 多协作主题

一个平行的DO任务，分配给了其它的实体（通过SendMsg沟通）
主Agent在WAIT状态时，能定期的通过SendMsg进行沟通，拿到并验证交付物后，将TODO标记为完成

## 多模态的支持

- 为文生图，文件传输等打通Message系列协议
- 