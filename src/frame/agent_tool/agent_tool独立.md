# AgentTool独立

目的：现在AgentTool是独立的工具，应该从OpenDAN中独立出来，边界更清楚

## 边界

OpenDAN需要保留的工具：
- 4个元工具(Edit/Read/Write/ExecBash) 还是保留在OpenDAN中（但实现可以通过import agent tool实现）
- CreateSubAgent 暂时保留在OpenDAN中

其它工具的实现，从OpenDAN中删除并移动到Agent Tool工程中


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

## 开发调试支持
只构造一个agent_tool,删除现在的多bin支持
调试上，应提供一个tmux session创建脚本，模拟agent 实际使用的bash来进行手工调试。这里所有的命令可以独立执行都是依赖与符号链接的 
相关单元测试，能移过来的也移过来


## 独立后OpenDAN Runtime的结果

- runtime里的tool_calls和action都变得固定，不再需要扩展
- 所有的skills + tools的扩展都通过bash的实质扩展实现

### 固定的tool
虽然我们不鼓励使用tool_calls,但这是一个被LLM支持的很好的特性，我们还是在鼓励 小任务中简单使用。
下面的tool一般用于观测(读)操作

**重要:我们整体倾向少用或不用tool** 

- read_file
- load_memory (memory文件系统化了)
- glob
- grep
- exec_bash 通常不会带

**下面两个先不支持**
- create_sub_agent/delegate_task 依赖todo / Task系统，比较适合开一个并行的检索任务
- check_task

### 固定的action

action的意图已经退化成利用xml格式对大块文本写操作进行优化，相关的action prompt是固化在LLMResult提示词里的。
底层实现与agent_tool必须一致，并有一致的side-effect。系统目前只支持下面2个action
- edit_file
- write_file
- multi_edit (等待支持)

### 传统的Policy的问题

tool-policy: 注册逻辑相同，只要注册了就可用
action:在output_protocol中显性控制

exec_bash的权限控制集成到session级别。通过一个标准的tmux解决tool的可见性和可用性问题
权限的硬控制在opendan runtime sandbox

### 在agent-loop中记录执行结果

```python
def agent_loop_step:
    llm_output = llm(current_session_state)
    llm_result = parser(llm_output)
    if llm_result.is_error():
        current_session_state.append_error(llm_result.error())
    else:
        current_session_state.append_step_summary(llm_result)
        for cmd in llm_result.cmds:
            cmd_result = execute(cmd)
            current_session_state.append_cmd_result(cmd_result)
            if is_end(cmd_result):
                break            

def agent_loop:
    while True:
        step_result = agent_loop_step
        if is_end(step_result):
            break;
        if is_pending(step_result):
            current_session_state.set_pending(step_rsult)
            break;
        
```

cmd_result的结果有 OK/failed/pending 3种。
agent_loop本身在每个step后，都会判断是否做下面3个动作
- 进入下一个step
- session结束
- session进入等待状态，等待目标事件的发生

注意区分cmd_result和step_result. 







