# OpenDAN 跨 run 历史重建导致 cache 前缀失效 TODO

状态：待实现。本次仅记录当前问题、修复建议与验收标准。

## 问题

同一 Session、同一 behavior 的正常续聊，应在 system、tools 和模型等条件不变时保留原始消息序列，只追加新的输入与输出。当前实现通常在一轮回答结束后关闭 run，下一次输入创建新 run，并将旧 worklog 重新渲染成一条 User 类型的 `<session_history>`。这改变了已发送的消息角色、内容和边界，破坏跨 run 的对话前缀连续性。

简化示例（实际请求还可能包含 behavior 消息、工具调用及结果）：

```text
第一轮请求：   system | user1
第一轮返回：   assistant1
期望下一轮：   system | user1 | assistant1 | user2
当前新 run：  system | user("<session_history>上一轮记录</session_history>") | user2
```

即使没有调用摘要模型，上述历史重建也已经改变了前缀。因此：

- 出现 `session_history` 不等于本次触发了 LLM 摘要压缩，但可能意味着原始消息序列被替换成了历史视图。
- 稳定的 system/tools 等共同前缀仍可能命中缓存；不能把这个问题描述成所有缓存必然全部失效。
- 同一 run 内的消息追加稳定，不能证明跨 run 也稳定。
- 新 run ID 本身不必然使缓存失效；关键是实际送给模型的内容前缀是否改变。
- 当前结论来自请求构造代码；实际缓存损失比例、延迟和费用变化仍需通过同一 provider/model 下的运行对照测量。

## 当前实现与原因

| 入口 | 当前行为 |
| --- | --- |
| [outcome.rs](../src/frame/lib_opendan/src/runner/outcome.rs) 的 `classify_done`、`finish_run` | 普通 Done 和允许等待的 `WAIT_USER_MSG` 通常结束 run；提交时清空 `state.live_run`，记录 `last_run`。 |
| [drive.rs](../src/frame/lib_opendan/src/runner/drive.rs) 的输入推进路径 | 没有 live run 时调用 `new_run_context`；正常下一轮通常不会延续上轮终态 snapshot 的历史。 |
| [live.rs](../src/frame/lib_opendan/src/runner/live.rs) 的 `own_run_context` | `inherit: recent_dialogue` 时调用 `build_history`，将返回的单条历史消息放在 system 后。 |
| [history.rs](../src/frame/lib_opendan/src/runner/history.rs) 的 `history_message`、`render_entry` | 将原始角色转为 `[input]`、`[assistant]`、`[result]` 等文本，装入一条 User 消息；还按记录年龄采用不同截断长度，旧记录可随新轮次出现而改变。 |
| [step_record.rs](../src/frame/llm_context/src/step_record.rs) 的 `XmlStepRenderer` | 同一 context 内当前 behavior 的 Step 以稳定的 assistant/user pair 追加；这部分已有的稳定性应保留并扩展到正常跨 run 续聊。 |

`history.rs` 的确定性渲染只保证“相同 summary + worklog 得到相同字节”，不保证“新增一轮后，之前请求的消息仍是新请求的原始前缀”。PendingTool、恢复未结束 run 等继续既有 snapshot 的路径，需要与普通结束后新建 run 的路径区分。

## 为什么不能直接从现有 worklog 无损还原

[flush.rs](../src/frame/lib_opendan/src/runner/flush.rs) 写入的是历史与审计投影，并非完整推理 transcript：

- assistant 使用 `text_content()` 和独立的 tool calls；文本块边界、与工具块的交错顺序、thinking/provider state 等未被完整保存。
- tool result 只提取文本并用换行合并，媒体等内容块丢失。
- 用户输入文本有记录，媒体块保留在 snapshot；原始消息结构不能仅靠 worklog 恢复。
- 初始 system/request.input 前缀不重复写入 worklog；Behavior Step 也不是完整的原始 `AiMessage`。

因此，将 worklog 文本重新分配成 user/assistant 角色只能近似恢复语义，不能保证请求前缀一致。修复需要使用已有完整 snapshot，或增加能够无损保留消息序列的持久记录。

## 修复建议

- [ ] **分离 run 生命周期与 LLM 历史生命周期。** 同一 Session 的兼容上下文可以跨正常 run 结束延续消息。run、Turn 的完成状态、统计和新一轮执行额度仍按现有语义推进，不应为了缓存把已完成任务伪装成未结束。
- [ ] **优先评估复用终态 snapshot 的完整历史。** 从 snapshot 提取原始消息、完整 Step pair 及必要的上下文信息，构造新 run 的历史输入；不能直接恢复旧终态执行状态，也不能重新执行旧工具或重复发送旧回答。若现有 snapshot 生命周期不适合承载跨 run 历史，再明确一个独立的持久 transcript 方案，避免两份可独立改写的历史真相源。
- [ ] **保持消息原样追加。** 保留角色、内容块顺序、call_id、tool_use/tool_result 配对及适用的 thinking/provider state。已发送的消息不得按年龄重新截断；时间、感知和 Goal 等新信息追加在本轮输入位置，不回写旧消息。
- [ ] **明确可延续的上下文条件。** 校验实际 system、tools、behavior 协议、模型和 provider 相关状态的兼容性；不能仅凭 Session ID 或相同模型 alias 认为前缀可复用。需要换上下文时明确建立新历史边界，并沿用已有 thinking/provider state 处理规则。保留 `inherit: none`、子上下文、fork/switch 的既有语义。
- [ ] **只在明确的历史重写边界生成摘要视图。** 正常续聊不重新生成 `session_history`。达到预算、显式压缩或显式导入其他上下文历史时才构建摘要/历史视图；新视图建立后作为稳定前缀继续追加。重新评估保留完整消息后的 token 预算和压缩触发条件，避免历史无限增长。
- [ ] **让历史状态跨 run、重启和清理保持一致。** 明确上下文历史版本与现有 `history_epoch` 的关系，保留输入 receipt/flush 游标的去重语义；历史引用与 Session 提交一致，压缩失败不发布半成品。审查 `after_run_end` 的旧 run 删除和 snapshot prune，不能清除下一轮仍需引用的唯一完整历史。
- [ ] **补充边界观测。** 记录正常历史延续、摘要压缩、上下文切换等不同原因，以及关联的 session/run/历史版本。复用已有 compaction/rewrite 日志，明确没有调用摘要模型的历史替换，避免只凭 `session_history` 标签判断发生了什么。

## 验收标准

- [ ] **跨轮前缀回归：** 同一 behavior、固定 system/tools/model，连续完成至少 3 轮普通对话；每轮即使创建新 run，下一次请求仍完整保留此前请求的消息前缀，并追加上轮结果与新输入，不出现正常换轮触发的 `session_history` 替换。
- [ ] **两种执行模式：** 覆盖传统 function-call 和 Behavior Loop；检查完整 `AiMessage`/内容块顺序及实际渲染结果，不能只比较 system 字符串或合并后的文本。
- [ ] **完整内容回放：** 覆盖原生工具调用、多内容块、媒体及适用的 thinking/provider state；工具结果关联正确，不丢块、不改变旧消息，也不重复执行已完成工具。
- [ ] **重启与清理：** 一轮结束后重启、旧 run 清理或 snapshot prune 后继续输入，仍能恢复应保留的完整历史；覆盖交接提交窗口的故障恢复，输入、回答和工具副作用不重复，历史用量不重复累计。
- [ ] **压缩边界：** 触发预算限制后允许一次显式历史重写，有对应原因和成功日志；重写后的后续请求继续追加。压缩失败保持可恢复状态，摘要与保留尾部之间无缺口、无重复。
- [ ] **上下文切换：** behavior/system/tools/model 等实际变化按既定规则处理；`inherit: none`、fork/switch 和挂起恢复不被普通续聊路径覆盖。
- [ ] **缓存对照：** 在同一 provider/model、相同前缀配置与可比缓存条件下，结合 AICC 请求/响应日志和 `read_aicc_log.py` 比较修复前后的请求公共前缀、cache usage、输入用量及延迟。前缀稳定作为确定性验收项；实际 cache 命中作为运行验证，不把外部缓存策略导致的未命中直接判成历史恢复失败。

## 实施范围

主要涉及 `libopendan` 的 `runner/live.rs`、`runner/outcome.rs`、`runner/drive.rs`、`runner/history.rs` 及历史持久化/清理路径；按需要调整 `llm_context` 的历史导出或初始化入口，复用现有消息类型和 Step renderer。实施时同步跨 run 恢复测试及历史稳定性文档。本 TODO 不代表相关修复已经完成。
