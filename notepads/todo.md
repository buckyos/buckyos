文档要求 READY session 走可配置 worker 并发执行（opendan.md (line 183), opendan.md (line 195)）；实现仍是串行循环，代码里也写了 placeholder（agent.rs (line 252)）。

文档要求拉取 msg + event 并做事件已读标记（opendan.md (line 180), opendan.md (line 213)）；实现里 event 拉取是空实现（agent.rs (line 306)），也没有 set_event_readed 对应逻辑。

文档要求无 session_id 时走 resolve_router（可多 step）产出路由结果（opendan.md (line 181), opendan.md (line 310)）；实现是直接回落到 default session（agent.rs (line 366)）。

文档里的 reply 是对外发送消息（opendan.md (line 245), opendan.md (line 375)）；实现仅日志打印 reply，没有真正发送（agent.rs (line 816)）。

文档要求 action 可并发（opendan.md (line 75), opendan.md (line 437)）；实现按 for 循环串行执行，并明确“parallel hint ignored”（agent.rs (line 720), agent.rs (line 765)）。

文档要求 step 后处理 workspace/todo side effects（opendan.md (line 259)）；实现未处理 todo/todo_delta（BehaviorLLMResult 有 todo 字段：types.rs (line 104)，但 agent.rs 未消费）。

文档示例里 WAIT 可直接携带 wait_details（opendan.md (line 275)）；实现依赖 session_delta 改状态并在 next_behavior=WAIT 时“保留已有 wait 状态”，set_wait_state 在 agent.rs 未被调用（agent.rs (line 884), agent_session.rs (line 335)）。

文档强调 action 的 fs/network 等 policy gate（opendan.md (line 649), opendan.md (line 661)）；实现调用 exec_bash 时只传了 command/cwd/timeout/allow_network，未传 fs_scope（agent.rs (line 731)，对比字段定义 types.rs (line 165)）。

文档定义 SLEEP 为长期空闲态（opendan.md (line 174)）；实现有状态枚举但缺少主动进入 SLEEP 的调度逻辑（agent_session.rs (line 28), agent.rs (line 888)）。