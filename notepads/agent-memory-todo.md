Findings
[P1] 缺少 resolve-by-default 生命周期 API。
Store 只有通用 update_lifecycle_status，CLI/工具只暴露 MarkAttentionSignalConsumed，没有 mark_converted / mark_watching / mark_dropped，也没有合法状态跃迁校验、watching TTL 重算或 hint 回指。见 agent_attention_signal.rs (line 1167)、buildin_tool.rs (line 647)、agent_tool_cli_dev/src/lib.rs (line 5120)。

[P1] target_consumer 没有落库，Skill gap 没有独立 pending 队列。
AttentionSignal 结构和表结构都没有 target_consumer 字段；ListPendingAttentionSignals 返回所有 pending signal，包括 skill_coverage_gap，然后靠 Stage2 prompt 让 LLM 跳过。当前只有一个 helper 用 signal_type <> skill_coverage_gap 过滤 memory signals，但没有 Candidate Skill Miner 的查询/消费通道。见 agent_attention_signal.rs (line 553)、agent_attention_signal.rs (line 1095)、agent_attention_signal.rs (line 1123)。

[P1] expires_at 已写入但完全不生效。
Store 创建 signal 时设置了 72h expires_at，但 pending 查询没有过滤过期项，也没有 sweep 把 pending/watching 转成 expired。这正是新文档 §32.6 明确要求补的点。见 agent_attention_signal.rs (line 995)、agent_attention_signal.rs (line 1108)。

[P1] 实际 OpenDAN Stage-1 输入不是 HistoryView.Raw / Full，且游标不是 (round_index, entry_seq)。
read_session_history 只返回 message 视图：没有 entry_kind，没有 raw entry，也不会返回 event entry；step 被压成 assistant text。实际提交进度只保存 committed_round_index，没有 entry_seq，因此不满足“未处理范围由 (session_id, round_index, entry_seq) 判定”。见 buildin_tool.rs (line 71)、buildin_tool.rs (line 327)、session_model.rs (line 349)、round_history.rs (line 1088)。

[P2] SkillCoverageGap 的 schema 和新文档不一致。
文档要求 target_consumer = candidate_skill_miner，stage2_hints.suggested_action = route_to_skill_candidate_miner；代码没有 target_consumer，并定义了 consider_skill_coverage_gap。见 agent_attention_signal.rs (line 271)、agent_attention_signal.rs (line 2003)。

[P2] Stage1 完成标记和“成功后推进游标”的边界偏弱。
self-improve session 只要以 self_improve_signals 行为 Done 结束，就会标记 Stage1 completed；没有检查 Begin/Discover/Complete/commit 是否全部成功。若 LLM 中途写入 signal 后失败但仍 Done，可能提前唤醒 Stage2。见 agent_session.rs (line 4352)。