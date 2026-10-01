我们现在会发现，有一些 Agent tool 或者说 action 其实是不关心返回过程的。

因为从某些意义上讲，它们表达的是某一种状态。也就是说，这里讨论的“具体需不需要返回值”，其实隐含的意思是：下一条新消息（下一次输入）到来之后，它的结果还要不要交给 LLM 作为输入。同一次输入的工具循环里，后续推理（Round）总能看到它的结果。
我们希望工具的实现者可以对这个事情进行控制。

最常见的例子，比如我们现在的 update_session_topic 调用：
1. 如果它在执行过程中触发了一些机械性的逻辑，得到了某些 hint。
2. 然后通过内部逻辑判断，认为 update_session_topic 的结果是需要大语言模型（LLM）处理的。
3. 那么它就需要进行返回。

这相当于是一种明确的意图。这个意图说明 agent tool 希望大模型把它的返回结果用到后续输入的处理中去。

这里的单元是 Message Pair，也就是 hot tail 中的单元：一条 user message 到对应的最终 assistant message 之间，中间可以有多次推理（Round）和多次工具调用。它是消息层的单元，不等于 Session Turn。

相当于说，当这个 Message Pair 正常完成之后，如果不希望它的结果继续作为 LLM 的输入，那么在下一条新消息输入时，它其实会从消息列表中被删除。

原本的消息序列可能是：
- user message
- tool call1
- tool result1
- tool call2
- tool result2
- assistant message

在处理后，中间可能会少掉一些 tool call 和 tool result。但这不会影响到对上一个 Message Pair 中 user message 和 assistant message 语义的判断。比如变成
- user message
- tool call1
- tool result1
- assistant message
- user message2 <-- 新消息

这种裁剪只会发生在一个新消息到来的处理过程中。也就是说，某种意义上，这是一种工具实现者可以做的故意的压缩（机械压缩）。这种压缩的核心就是它判断，只要用户拿到历史的 user message 和最后结果的 assistant message 后，这一段 tool code 即使丢掉，也不会对后续新消息的处理产生什么影响。

