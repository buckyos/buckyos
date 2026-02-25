High: 文档里的 wait_details 协议与当前输出协议不兼容
文档伪代码是 next_behavior == WAIT 时直接 set_wait_state(wait_details)，见 opendan.md:273 和 opendan.md:275。
现在 BehaviorLLMResult 没有 wait_details 字段，见 types.rs:94。
解析器对未知字段会报 schema 错误，wait_details 会被拒绝，见 parser.rs:108 和 parser.rs:192。
当前实际语义是通过 session_delta 改 state/wait_details，再由 transition 处理 WAIT，见 agent_session.rs:503 和 agent.rs:2212。
Medium: SLEEP 状态在 runtime 中基本未落地
文档把 SLEEP 定义为长期无输入进入休眠，见 opendan.md:174。
代码里无 input 时回落到 WAIT，未见自动转 SLEEP 路径，见 agent.rs:1411。


## 关于WAIT

在一个Behavior的开始(step0)无法获得输入，就会自动WAIT

- 没有new_msg
- 没有new_event
- 没有next_tod
- 没有last_step_summary(肯定没有)

Behavior可以在任何step主动的把自己进入WAIT_FOR_XXX状态，此时要进入下一个step,就必须


## Input的消费逻辑的

结论：不完全是。当前实现是“两层消费”，其中只有第二层接近你说的“step 后手工标记”。

pull 不是按 prompt 按需拉。session worker 在每轮 step 前都会先拉两类队列（msg+event），再塞进 session.new_msgs/new_events。见 agent.rs 第 1379、1384、1387、1111-1137 行。
“本 step 是否使用 new_msg/new_event”是按 behavior_cfg.input 占位符决定，不是按最终 prompt 内容决定。见 agent_session.rs 第 369-394、927-951 行。
LLM step 成功后，才会按 consumable_ids 从 new_msgs/new_events 里删除（手工消费）。见 agent.rs 第 1541 行 + agent_session.rs 第 426-447 行。
但系统层消息状态更早就变了：msg_center 的消息在 dispatch 时就被标 Readed，不是等 step 结束。见 agent.rs 第 432-468、710-727 行。
另外当前“消费”是从 new_* 删除，没有迁移到 history_*（测试里也验证了 history 为空）。见 agent_session.rs 第 1217-1225 行。

## 仔细Review Input的构造逻辑

先修 env key 对齐（最低风险、收益高）。
再加 msg_center history provider：按 session_id/thread_key 拉 list_box_by_time，裁剪后注入 <<History>>。
最后做 kevent resolver 抽象（通知与业务事件解耦）。



TODO：

- Behavior Loop的新流程 (带参数behavior),包括wait，正确实现模板替换
  - 摸板能依赖的变量就是
  - 要考虑恢复的问题（只能用session state)
- LastStep 的构造流程 等 新版本的魔板替换
- Review4个元工具（精品工具)的实现，
  - 注意bash的tmux化
- 正确实现Toolbox