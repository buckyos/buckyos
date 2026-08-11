我们现在会发现，有一些 Agent tool 或者说 action 其实是不关心返回过程的。

因为从某些意义上讲，它们表达的是某一种状态。也就是说，这里讨论的“具体需不需要返回值”，其实隐含的意思是：它的结果要不要交给下一个轮次的 LLM 作为输入。
我们希望工具的实现者可以对这个事情进行控制。

最常见的例子，比如我们现在的 update_session_topic 调用：
1. 如果它在执行过程中触发了一些机械性的逻辑，得到了某些 hint。
2. 然后通过内部逻辑判断，认为 update_session_topic 的结果是需要大语言模型（LLM）处理的。
3. 那么它就需要进行返回。

这相当于是一种明确的意图。这个意图说明 agent tool 希望大模型把它的返回结果用到下一轮对话中去。

所谓下一轮对话，就是指一个所谓的 hot tail，即一个 user message 和一个 assistant message 之间。

相当于说，当正常的一轮调用完成之后，如果不希望它的结果作为下一轮 LLM 的输入，那么当下一轮进行输入时，它其实会从消息列表中被删除。

原本的消息序列可能是：
- user message
- tool call1
- tool result1
- tool call2
- tool result2
- assistant message

在处理后，中间可能会少掉一些 tool call 和 tool result 的轮次。但这不会影响到对上一轮 user message 和 assistant message 语义的判断。比如变成
- user message
- tool call1
- tool result1
- assistant message
- user message2 <-- 新消息

这种裁剪只会发生在一个新消息到来的处理过程中。也就是说，某种意义上，这是一种工具实现者可以做的故意的压缩（机械压缩）。这种压缩的核心就是它判断，只要用户拿到历史的 user message 和最后结果的 assistant message 后，这一段 tool code 即使丢掉，也不会对后续新消息的处理产生什么影响。

