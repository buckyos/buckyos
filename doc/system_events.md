# BuckyOS System Events

kevent-bus 的基本模式介绍: 减少轮询操作

## timer events

kevent-client内置，100%的进程内事件

## system config events

订阅路径 /system_config/$config_path

## kmsgqueue events

kevent和kmsgqueue都有id路径系统，因此经常组合使用 （IoT都会用）

先订阅 msg_queue_id 
/kmsg/$msg_queue_id 
在事件触发后，通过kmsgqueue的接口，去$msg_queue_id里pull msg

## task mgr events

订阅路径 /task_mgr/$taskid

当任务 `status` 发生变化时，task_mgr 会在该路径发布事件，避免使用者轮询 `get_task`

事件数据包含 `from_status` / `to_status` 以及 task 基础信息

不要订阅所有的

## msg center events

订阅路径 /msg_center/$owner/box/$box_name/$event_name

当box里的消息发生变化时，会收到通知

使用例子（伪代码),用下面方法来取代定时轮询 msg_center.get_next()

```python

event_reader = kevent.sub_event("/msg_center/$owner/box/in/*")
event_reader.pull_event().await # 会超时返回，所以即使漏了消息也没关系
msg_center.get_next("$owner","inbox")

```


## opendan agent events

### todo events

订阅路径

/agent/$todo_list_id/$todo_id/status_changed

当 todo 的 `status` 发生变化时，opendan 会在该路径发布事件，避免轮询 todo 列表

可以得到特定todolist变更，todo变更（包含所有sub todo）
