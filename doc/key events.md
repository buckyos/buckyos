# BuckyOS System Events

kevent-bus 的基本模式介绍: 减少轮询操作
其设计意图类似传统的Windows的 WaitForMuiltObj(Vec<Path>)
由于objname是系统级去重的，所以

## 对象的R-Link 

二级实体（全网看来）
did:bns:$obj_name.zone_name

系统内全局对象:
/


## timer events

kevent-client内置，100%的进程内事件

## system config events

订阅路径 /system_config/$config_path

## kmsgqueue events

kevent和kmsgqueue都有id路径系统，因此经常组合使用 （IoT都会用）

先订阅 msg_queue_id 
/kmsg/$msg_queue_id 
在事件触发后，通过kmsgqueue的接口，去$msg_queue_id里pull msg

对象事件的 kevent key 统一为 `/obj/<host>/<inner-path...>/<event>`（规则见 [key url.md](key%20url.md) §7，Rust `buckyos_api::obj_event_key`，websdk `objectUrl.objEventKey`）；下面各服务的事件都按这个形式命名。

## task mgr events

订阅路径 /obj/taskmgr/$taskid/changed（整棵任务树：/obj/taskmgr/$root_id/tree）

当任务 `status` 发生变化时，task_mgr 会在该路径发布事件，避免使用者轮询 `get_task`

事件数据包含 `from_status` / `to_status` 以及 task 基础信息

不要订阅所有的

## msg center events

订阅路径 /obj/msg-center/owners/{owner_did}/sessions[/{session_id}]/{box}（box = inbox / sent / group_inbox / request_box，段名按 key url.md §7 编码）

当box里的消息发生变化时，会收到通知。联系人变化：/obj/msg-center/owners/{owner_did}/contacts/changed；群组变化：/obj/{group_did_host}/changed

使用例子（伪代码),用下面方法来取代定时轮询 msg_center.get_next()

```python

event_reader = kevent.sub_event("/obj/msg-center/owners/{owner_hex}/**")
event_reader.pull_event().await # 会超时返回，所以即使漏了消息也没关系
msg_center.get_next("$owner","inbox")

```


## opendan agent events

### todo events

订阅路径

/agent/$todo_list_id/$todo_id/status_changed

当 todo 的 `status` 发生变化时，opendan 会在该路径发布事件，避免轮询 todo 列表

可以得到特定todolist变更，todo变更（包含所有sub todo）
