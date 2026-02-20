
# OpenDAN的类型

## Task相关
由系统的TaskMgr定义，OpenDAN根据自己的需要扩展了一些Task.data格式

## Msg相关

由系统的MsgCenter定义。用来表示在互联网上传播的通讯信息。
注意和kmsg模块的msg_queue里的msg定义不同

## InputEvent相关

来自系统event-bus的可扩展信息（比如一个IoT设备检测到了某个时间，系统检测到某个文件改变等），底层通常基于kmsg实现。


## Workshop

- Todo
- worklog
- Tool相关
  - ToolCall
  - ToolCallResult
  - ToolCallContext
- Skills相关

## Agent

- AgentSession
- memory相关

## Agent Loop

- behavior
  - behavior config
- Input
- Output (Output protocol)
  - BehaviorLLMResult
  - ResolveRouterResult
