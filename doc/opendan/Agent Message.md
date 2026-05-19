# (Agent) Message Protocol

- NamedObject的定义参考ndn-lib,本质上是稳定编码的json
- DID基本遵循w3c语义
- MsgObject的实现在ndn-lib中
- 定义MsgCenter服务的功能
- 定义ContactMgr
- 实现Tg Tunnel,实现和MsgObject的转化


## 几个核心循环

一个典型的tunnel实现,可以同时为多个Agent/User服务
```python
def telgram_tunnel.recv_thread():
    while True:
        tg_update = get_next_update() # 从所有绑定的账号(tg_client)上读取信息
        msg_obj = tg_update_to_message_object(tg_update)
        msg_center.dispatch(msg_obj)

def telgram_tunnel.send_thread():
    while True:
        msg_obj = msg_center.get_outbox(self.tunnel_id).get_next()
        self.send_msg(msg_obj)
        msg_center.update_msg_state(msg_obj.id,SENDED)

def telgram_tunnel.send_msg(msg_obj):
    tg_client = get_client_from_sender(msg_obj.from)
    tg_msg = message_object_to_tg_msg(msg_obj)
    tg_client.send_message(tg_msg)
```

下面是Agent的处理，只关心自己的inbox和message_dispatcher提供的发送消息服务

```python
# 正常Agent Loop读消息，可以批量读
def agent_service.on_weakup()
    inbox = msg_center.get_inbox(self.did)
    msg = inbox.get()
    inbox.update_msg_state(msg.id,READING)
    self.llm_behavier["read_msg"].process(msg) # msg_center.outbox.put()
    update_msg_state(self.did,msg.id,READED)

    #处理群聊,如果多个Agent加入了一个群聊，那么reading状态时跟着agent id走的
    msg_list = self.get_group_chat_msgs()
    self.llm_behavier"read_msg"].process(msg_list)
    
    self.sleep(DEFAULT_SLEEP_TIME)

# 被特定的消息强制拉起进行LLM
def agent_service.on_msg(msg)
    if msg.state != UNREAD:
        return
    msg.state = READING
    self.llm_behavier["process_msg"].process(msg)
    msg.state = READED

# 被特定的事件拉起进行LLM
def agent_service.on_event(event)
    self.llm_behavier["on_event"].process(event)

```

```python
def message_center.dispatch(new_msg):
    is_block = contact_mgr.is_block(new_msg.sender)
    if is_block:
        return
    inbox = get_inbox(new_msg)
    inbox.put(new_msg)

def message_center.post_send(will_send_msg):
    is_block = contact_mgr.is_block(will_send_msg)
    if is_block:
        return
    outbox = get_outbox(will_send_msg)
    outbox.put(will_send_msg) #默认状态时WAIT

```

人也可以通过这个体系，在inbox里查看，来自所有tunnel的消息
系统内部，写inbox直接通过dispatcher中转，和tunnel没关系。

## MsgThread

从chat sessoin的角度来看，用户看到的message list包含自己发送的消息和
怎么整合inbox和outbox的messagelist,形成thread,是UI的工作
最常见的就是Message包含有ThreadId,然后对同一个session-id进行聚合

## MsgBox

msg_center提供了msgbox抽象.

```python
inbox = msg_center.get_inbox(my_did)
inbox.get_msg()
outbox = msg_center.get_outbox(my_did)
outbox.put_msg()
```

对tunnel来说，是往inbox的生产者，outbox的消费者

## 消息副本的问题

当内部的Agent1给Agent2发送消息时，是否需要添加2条消息

- 1条在Agent1的outbox
- 1条在Agent2的inbox

需要：因为大家对历史消息的删除策略不同

## 参考 MsgObject 定义

新的 `MsgObject` 协议定义见 `doc/message_hub/MsgObject.md`。

关键变化：

- `MsgObject` 是系统通用消息抽象，不绑定 Message Tunnel，也不只支持 BuckyOS DID。
- 作者只出现在 `from`；群、会话、外部账号、组件等都作为 `to` target 表达，不再使用 `source` 表达群消息作者。
- Message Tunnel 来源和投递通道放入 `via` / `to[].delivery`；平台无损信息放入 `ext["buckyos.message_tunnel"]`。
- 会话内或应用内排序放入顶层 `seq`；`created_at_ms` 只做时间事实和缺省兜底。
- 消息内容使用 `content.parts` 表达；附件使用 `MsgAttachment`，大附件通过 `Url` 或 `ObjId` 引用，不做内联编码。
- AI 流式交互使用多条不可变 `MsgObject` 帧，通过 `stream.stream_id + MsgObject.seq` 重建。

