# AgentTool独立

目的：现在AgentTool是独立的工具，应该从OpenDAN中独立出来，边界更清楚

## 边界

OpenDAN需要保留的工具：
- 4个元工具(Edit/Read/Write/ExecBash) 还是保留在OpenDAN中
- CreateSubAgent 暂时保留在OpenDAN中

其它工具的实现，移动到Agent Tool工程中


## 实现模型

**OpenDAN 依赖 AgentTool**

这意味着AgentTool更纯粹，是不依赖OpenDAN Runtime的工具.
如果OpenDAN内部需要用函数的方法操作 todo,应该使用Agent Tool的lib.

**AgentTool定义Tool的输出协议**
这个是Agent Tool与OpenDAN沟通的核心协议，OpenDAN内部怎么做AgentTool执行结果的提示词渲染，都依赖与此
将协议放入AgentTool,可以让边界更清楚

**opendan的Tool Policy**

只作用于4个元工具，后续的安全策略将以session sandbox为粒度设计（目前通过符号链接创建工具的方法，已经是一种可见性管理）
所以AgentTool的内部实现，可以完全不依赖OpenDAN的tool policy组件

## 调试支持
只构造一个agent_tool,删除现在的多bin支持
调试上，应提供一个tmux session创建脚本，模拟agent 实际使用的bash来进行手工调试。这里所有的命令可以独立执行都是依赖与符号链接的 



