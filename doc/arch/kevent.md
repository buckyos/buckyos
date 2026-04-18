# BuckyOS EventBus 技术需求文档



## 1. 概述

EventBus 是 BuckyOS 的基础事件通信设施，为分布式操作系统中的各组件提供轻量级的事件发布/订阅能力。

EventBus 的系统定位是**纯加速层**：一个完全无状态的、best-effort 的信号通道。它与 BuckyOS 的 kMsgQueue（有状态的 stream-based 消息队列）配合工作，构成分布式系统通信基础设施的完备组合：

- **EventBus**：低延迟异步通知，事件可丢失，无持久化，推模型
- **kMsgQueue**：可靠数据交付，消息不丢失，有持久化，拉模型（HTTP GET）

消费端的标准模式是：EventBus 通知驱动快路径处理，kMsgQueue 轮询兜底保证不丢。EventBus 完全故障时，系统自动退化为纯轮询，功能不受影响，仅延迟从毫秒级退化到秒级。

```
典型消费端模式:

loop:
    event = reader.pull_event(timeout=N)
    if event != null:
        // 快路径：event-bus 通知到了，立即去拿数据
        data = http_get(kmsgqueue, cursor)
        handle(data)
        update(cursor)
    else:
        // 慢路径：超时兜底轮询
        data = http_get(kmsgqueue, cursor)
        if data != null:
            handle(data)
            update(cursor)
```

EventBus 本身不是数据通道，而是信号通道。事件消息体应尽可能轻量（通知 + 轻量摘要），真正的数据交付通过 kMsgQueue 完成。

### 1.1 设计目标

- **轻量**：事件发布路径纯内存操作，不产生磁盘 I/O
- **低延迟**：进程内直接写入 Ring Buffer；本机跨进程走共享内存；跨节点走 TCP 长连接
- **完全无状态**：Daemon 无持久化存储，无注册表，无需恢复逻辑，随时可重启
- **Best-effort**：事件可丢失，不保证送达，可靠性由 kMsgQueue 兜底
- **轻量接入**：资源受限的设备（如 IoT）无需运行本地 Daemon，通过 Light SDK 直连远程 Daemon 即可发布事件

### 1.2 设计原则

- **事件可丢失**：事件发布时，若没有任何 reader 订阅匹配的 pattern，事件将被丢弃。EventBus 不提供事件回溯能力。丢失的事件由消费端的 kMsgQueue 轮询兜底找回。
- **无注册、无持久化**：全局事件无需 `create_event` 注册即可直接 pub/sub。Daemon 不维护任何磁盘状态。
- **双命名空间**：全局事件（`/` 开头）支持跨进程/跨节点分发；本地事件（非 `/` 开头）仅进程内分发。
- **统一消费模型**：无论全局事件、本地事件还是 Timer，消费端均通过 `pull_event` 统一接收，简化业务逻辑。
- **Ring Buffer 为终点**：Pub 的核心是找到所有匹配 Sub 的 Ring Buffer 并将 Event 写入。中间的所有拓扑结构本质上都是路由——解决"如何找到目标 Ring Buffer"的问题。
- **Daemon 为 Sub 服务**：Node Daemon 的存在是为了维护本机 Sub 的 Ring Buffer 并接收跨节点广播。纯 Pub 设备（如 IoT）无需运行本地 Daemon，只需连接任意远程 Daemon 投递事件即可。

---

## 2. 核心概念

### 2.1 EventID

EventID 分为**全局事件**和**本地事件**两种命名空间：

**全局事件**：以 `/` 开头，采用类似文件路径的层级命名。支持跨进程、跨节点分发。

```
格式: /<namespace>/<category>/[<subcategory>/...]<name>
示例:
  /taskmgr/new
  /taskmgr/done/task_001
  /filemgr/sync/completed
  /system/node/online
```

**本地事件**：不以 `/` 开头，作用域限定在当前进程内。纯进程内函数调用分发，不经过 Daemon。

```
格式: <name>
示例:
  task_done
  config_changed
  heartbeat_tick
```

命名规则：

- 全局事件必须以 `/` 开头，每级名称仅允许字母、数字、下划线、连字符，最大深度建议不超过 8 级，最大长度不超过 256 字符
- 本地事件不以 `/` 开头，不允许包含 `/`，仅允许字母、数字、下划线、连字符，最大长度不超过 128 字符

### 2.2 Pattern（订阅匹配模式）

订阅时使用 pattern 匹配 EventID，支持通配符。Pattern 同样遵循命名空间规则：

- 以 `/` 开头的 pattern 匹配全局事件
- 不以 `/` 开头的 pattern 匹配本地事件（精确匹配，本地事件不支持通配符）

全局事件 pattern 通配符：

| 通配符 | 含义 | 示例 |
|--------|------|------|
| `*` | 匹配单级任意名称 | `/taskmgr/*/task_001` 匹配 `/taskmgr/new/task_001`，不匹配 `/taskmgr/a/b/task_001` |
| `**` | 匹配零级或多级 | `/taskmgr/**` 匹配 `/taskmgr/new`、`/taskmgr/new/task_001` |

### 2.3 Event 消息体

```json
{
  "eventid": "/taskmgr/new/task_001",
  "source_node": "node_a",
  "source_pid": 1234,
  "timestamp": 1708588800000,
  "data": { /* 用户自定义 JSON，应尽量轻量 */ }
}
```

`eventid`、`source_node`、`timestamp` 由系统自动填充，`data` 由调用方提供。

---

## 3. API 设计

### 3.1 创建事件读取器

```
create_event_reader(patterns: string[]) -> Result<EventReader>
```

- patterns 可以混合包含全局 pattern（`/` 开头）和本地 pattern（非 `/` 开头），SDK 内部自动分流处理
- **全局 pattern**：注册到 Daemon，参与本机跨进程和跨节点分发
- **本地 pattern**：直接注册到 SDK 本地订阅表，仅进程内分发
- 不做 eventid 存在性校验（无注册机制）
- 返回 EventReader 实例，统一通过 `pull_event` 消费所有匹配事件

### 3.2 事件读取

```
EventReader.pull_event(timeout_ms?: number) -> Result<Event | null>
```

- 阻塞等待直到 Ring Buffer 中有匹配事件到达或超时
- `timeout_ms` 为 0 时非阻塞，立即返回
- `timeout_ms` 省略时无限等待

```
EventReader.close() -> void
```

- 关闭 reader，释放资源，取消订阅

### 3.3 事件发布

```
pub_event(eventid: string, data: JSON) -> Result<void>
```

- **全局事件**（`/` 开头）：写入本进程内匹配 reader 的 Ring Buffer；同时写入本机共享内存 Ring Buffer（本机其他进程 + Node Daemon 消费）
- **本地事件**（非 `/` 开头）：仅写入本进程内匹配 reader 的 Ring Buffer，不经过 Daemon
- 无匹配 reader 时事件静默丢弃
- 无权限校验（权限管控如需要，通过 BuckyOS 已有的身份/权限体系在 Daemon 层拦截）

### 3.4 定时器（Timer）

Timer 是 SDK 层的语法糖，底层复用本地事件机制。到期时 SDK 自动向指定的本地 eventid 发布事件，消费端通过统一的 `pull_event` 接收，无需引入额外的异步消费模型。

```
create_timer(eventid: string, options: TimerOptions) -> Result<TimerId>
```

- `eventid` 必须为本地事件（非 `/` 开头），否则返回错误
- Timer 完全在 SDK 进程内实现，Daemon 不感知 Timer 的存在
- 到期时 SDK 自动调用 `pub_event(eventid, { timer_id, tick_count, ... })`

```
TimerOptions {
  interval_ms: number        // 触发间隔（毫秒）
  repeat: boolean            // 是否重复，默认 true
  initial_delay_ms?: number  // 首次触发延迟，默认等于 interval_ms
  data?: JSON                // 每次触发时附带的自定义数据
}
```

```
cancel_timer(timer_id: TimerId) -> Result<void>
```

- 取消定时器，停止后续事件发布

**Timer 事件消息体：**

```json
{
  "eventid": "heartbeat_tick",
  "source_node": "node_a",
  "source_pid": 1234,
  "timestamp": 1708588800000,
  "data": {
    "_timer": {
      "timer_id": "t_001",
      "tick_count": 42
    }
  }
}
```

**使用示例：**

```
// 创建定时器，每秒触发
timer_id = create_timer("heartbeat_tick", { interval_ms: 1000 })

// 统一通过 EventReader 消费，与普通事件无异
reader = create_event_reader(["heartbeat_tick", "/taskmgr/**"])
loop:
    event = reader.pull_event()
    // event 可能是 timer 事件，也可能是 taskmgr 的全局事件
    handle(event)
```

---

## 4. 系统架构

### 4.1 核心模型：Ring Buffer 为终点

Pub/Sub 的本质是：Pub 找到所有匹配 Sub 的 Ring Buffer，将 Event 写入。三条投递路径的区别仅在于"Ring Buffer 在哪里、如何可达"：

| 路径 | Ring Buffer 位置 | 可达方式 | 延迟 |
|------|-----------------|----------|------|
| 进程内 | 本进程堆内存 | 直接指针访问 | 纳秒级 |
| 本机跨进程 | 共享内存 | mmap 同一块内存 | 微秒级 |
| 跨节点 | 远程进程内存 | TCP → 远程 Daemon → 写入 | 毫秒级 |

### 4.2 整体架构

```
  IoT Devices (Light SDK, pub only)
  ┌─────┐ ┌─────┐ ┌─────┐
  │Dev 1│ │Dev 2│ │Dev 3│
  └──┬──┘ └──┬──┘ └──┬──┘
     │       │       │
     └───────┼───────┘
             │ TCP / HTTP
             ▼
  Node A                                       Node B
  ┌────────────────────────────┐              ┌────────────────────────────┐
  │  ┌─────┐  ┌─────┐         │              │  ┌─────┐  ┌─────┐         │
  │  │Proc1│  │Proc2│         │              │  │Proc3│  │Proc4│         │
  │  │(pub)│  │(sub)│         │              │  │(sub)│  │(pub)│         │
  │  └──┬──┘  └──┬──┘         │              │  └──┬──┘  └──┬──┘         │
  │     │        │             │              │     │        │             │
  │     ├────────┤             │              │     ├────────┤             │
  │     │ Shared Memory        │              │     │ Shared Memory        │
  │     │ (Ring Buffers)       │              │     │ (Ring Buffers)       │
  │     ├────────┤             │              │     ├────────┤             │
  │     │        │             │              │     │        │             │
  │  ┌──▼────────▼──┐         │   TCP 长连接  │  ┌──▼────────▼──┐         │
  │  │  Node Daemon │◄────────┼──────────────►┼──│  Node Daemon │         │
  │  │  (无状态)     │         │              │  │  (无状态)     │         │
  │  └──────────────┘         │              │  └──────────────┘         │
  └────────────────────────────┘              └────────────────────────────┘
```

IoT 设备通过 Light SDK 直连任意 Node Daemon 投递事件，不参与 mesh 拓扑，不增加广播扇出。

### 4.3 三条投递路径

**路径一：进程内投递（最短路径）**

SDK 在进程内维护本地订阅表（Map<Pattern, List<RingBuffer>>）。`pub_event` 时直接遍历匹配的 reader，将 Event 写入其 Ring Buffer。纯函数调用，零系统调用开销。

**本地事件（非 `/` 开头）仅走此路径，到此结束。**

**路径二：本机跨进程投递（共享内存）**

全局事件在完成进程内投递后，还需通知本机其他进程。通过共享内存实现：

- 本机所有进程 mmap 同一块共享内存区域，其中包含 Ring Buffer
- Pub 端将 Event 写入共享内存 Ring Buffer
- 通过 eventfd 等机制通知 Sub 端有新事件到达
- Sub 端从共享内存 Ring Buffer 读取并匹配

避免了经过 Daemon 中转的两次 Unix Socket 往返。

**路径三：跨节点投递（Node Daemon + TCP 广播）**

Node Daemon 隐式 100% 订阅本机所有全局事件的 Pub 消息（通过共享内存 Ring Buffer 接收）。收到新 Event 后：

1. Node Daemon 通过全 mesh TCP 长连接，将 Event 广播给所有在线的 peer Node Daemon
2. 接收端 Node Daemon 检查本机是否有匹配的 reader，有则通过路径二（共享内存）写入对应进程的 Ring Buffer；无则丢弃
3. 不维护跨节点订阅同步表——广播 + 接收端过滤，简单可靠

由于 BuckyOS 节点规模较小（个位数到几十台），广播的网络开销远小于维护订阅同步协议的复杂度。

### 4.4 Node Daemon 内部结构

```
NodeDaemon {
    // 纯内存，无持久化
    local_subscriptions: Trie       // 本机所有进程的订阅关系，用于接收端过滤
    shared_mem: SharedMemoryRegion  // 本机共享内存 Ring Buffer 区域
    remote_peers: Map<NodeId, TcpConnection>  // 到其他节点的 TCP 长连接
    external_pub_listener: TcpListener        // 接受外部 Light SDK 设备的 pub 连接
}
```

Node Daemon 是纯无状态的：无持久化存储，无注册表，无跨节点订阅同步。重启后纯净启动，等 client 重连并重新订阅即可。

Node Daemon 同时接受两类事件来源：本机进程通过共享内存写入的事件，以及外部 Light SDK 设备通过 TCP/HTTP 投递的事件。两类来源的事件处理逻辑完全相同：本机匹配分发 + 广播给所有 peer。

### 4.5 Client SDK 结构

SDK 提供两种运行模式，适配不同的设备能力和角色：

#### 4.5.1 Full SDK（有本地 Node Daemon 的节点）

适用于运行了 Node Daemon 的完整 BuckyOS 节点，支持 pub + sub + timer 全部能力。

```
EventClient (Full Mode) {
    // 进程内
    local_bus: Map<Pattern, List<RingBuffer>>  // 进程内订阅
    timers: Map<TimerId, TimerState>            // 进程内定时器

    // 本机跨进程
    shared_mem: SharedMemoryRegion              // 共享内存映射

    pub_event(eventid, data):
        if eventid 不以 "/" 开头:
            // 本地事件：仅写入进程内匹配 reader 的 Ring Buffer
            write_to_matching_local_ringbuffers(eventid, data)
        else:
            // 全局事件：
            1. 写入进程内匹配 reader 的 Ring Buffer
            2. 写入共享内存 Ring Buffer（本机其他进程 + Node Daemon 消费）

    create_event_reader(patterns):
        local_patterns = patterns 中不以 "/" 开头的
        global_patterns = patterns 中以 "/" 开头的
        1. 为 reader 分配 Ring Buffer
        2. local_patterns 注册到 local_bus
        3. global_patterns 注册到 local_bus + 共享内存订阅表
        4. 返回 EventReader 实例

    create_timer(eventid, options):
        1. 校验 eventid 不以 "/" 开头
        2. 启动定时器线程/任务
        3. 到期时调用 pub_event(eventid, timer_data)
        4. 返回 TimerId
}
```

#### 4.5.2 Light SDK（无本地 Node Daemon 的设备）

适用于资源受限的设备（如 IoT 传感器、嵌入式终端），这些设备只需要发布事件，没有订阅需求，也没有能力运行本地 Node Daemon。

Light SDK 仅提供 `pub_event`，通过 TCP/HTTP 直连某个远程 Node Daemon 投递事件。实现极其轻量，几十行代码即可完成。

```
EventClient (Light Mode) {
    remote_daemon: TcpConnection    // 连到任意一个有 Daemon 的节点
    fallback_daemons: List<Endpoint> // 备选 Daemon 地址列表

    pub_event(eventid, data):
        // 直接发送给远程 Daemon，由其负责广播
        send(remote_daemon, event)
        // 发送失败时切换到备选 Daemon
}
```

**Light SDK 的特点：**

- 仅支持 `pub_event`，不支持 `create_event_reader` 和 `create_timer`
- 不需要共享内存、不需要本地 Daemon
- 连接哪个 Daemon 无所谓——每个 Daemon 收到 event 都会广播给所有 peer
- 连接断开时自动切换到备选 Daemon，丢失的事件由 kMsgQueue 兜底
- 不参与 mesh 拓扑，不增加广播扇出系数：多个 IoT 设备连到同一个 Daemon，对 mesh 来说只是一个 pub 源

```
部署拓扑示例:

  IoT Device 1 ──┐
  IoT Device 2 ──┼── TCP ──► Node A (Daemon) ◄──► Node B (Daemon)
  IoT Device 3 ──┘                    ▲                  ▲
                                      │                  │
                                   Proc1(sub)         Proc2(sub)
```

#### 4.5.3 模式选择指南

| 设备特征 | 推荐模式 | 说明 |
|----------|----------|------|
| 完整 BuckyOS 节点，有 pub 和 sub 需求 | Full SDK + 本地 Daemon | 三条路径全部可用 |
| 仅 pub 的设备（IoT 传感器等） | Light SDK | 无需本地 Daemon，直连远程 Daemon |
| 主要 sub，偶尔 pub 的设备 | Full SDK + 本地 Daemon | 需要本地 Ring Buffer 接收事件 |

---

## 5. 跨节点通信

### 5.1 节点发现

Node Daemon 复用 BuckyOS 已有的节点发现/membership 机制获取集群节点列表。节点规模较小时（几十台以内）采用全 mesh TCP 长连接。

### 5.2 事件广播（无订阅同步）

跨节点通信采用最简模型：**广播 + 接收端过滤**。

```
pub_event 跨节点流程:
  1. Node B 上某进程 pub_event("/taskmgr/new/123", data)
     （或：外部 IoT 设备通过 Light SDK 向 Node B Daemon 投递事件）
  2. Event 写入 Node B 共享内存 Ring Buffer（本机进程来源）
     或 Node B Daemon 直接收到 TCP 消息（外部设备来源）
  3. Node B Daemon 获取到新 Event
  4. Node B Daemon 通过 TCP 长连接广播给所有 peer Daemon
  5. Node A Daemon 收到后，检查本机订阅表
  6. 有匹配 reader → 写入对应进程的共享内存 Ring Buffer
     无匹配 reader → 丢弃
```

不维护跨节点订阅同步表，不交换 pattern 信息。每条全局事件都广播给所有 peer，由接收端决定是否需要。

这种设计在小规模集群下的优势：

- 零同步协议开销
- 新 reader 创建后立即生效，无需等待订阅信息传播
- Node Daemon 逻辑极简：收到就广播，收到就过滤
- 故障恢复无需重建订阅同步状态

### 5.3 连接管理

- Node Daemon 之间维护 TCP 长连接，连接断开后自动重连
- 重连期间的事件丢失，由消费端的 kMsgQueue 轮询兜底
- 不引入 UDP 补偿通知——TCP 长连接断了说明节点不可达，UDP 大概率也到不了

### 5.4 跨节点通信的协议设计

本节给出协议层设计，分为三类角色：

- **Peer Daemon 协议**：Node Daemon 与 Node Daemon 之间复制全局事件
- **Native Daemon API**：Full SDK / Light SDK / 其它本地 client 与 Daemon 之间的请求响应协议
- **HTTP Facade 协议**：穿过 gateway 给浏览器或轻量 HTTP client 使用的协议

当前 Rust 实现中，`src/kernel/kevent/src/lib.rs` 已经稳定了 Daemon 侧的核心语义：

- `register_reader(reader_id, patterns)`：只接受全局 pattern
- `unregister_reader(reader_id)`
- `publish_external_global(event)`：只接受全局 event
- `publish_from_peer(event)`：来自 peer 的事件只做本地分发，不再转发
- `pull_event(reader_id, timeout_ms)`

对应的协议对象定义在 `KEventDaemonRequest` / `KEventDaemonResponse` 中。

#### 5.4.1 Native Daemon API：请求响应协议

这一层是 Daemon 面向外部 client 的基础协议，适合：

- Light SDK 直连远程 Daemon 发布事件
- Full SDK 中的 daemon bridge
- 调试工具、测试工具
- 后续 HTTP 协议的直接映射

请求结构直接采用当前实现中的 `KEventDaemonRequest`：

```json
{ "op": "register_reader", "reader_id": "r1", "patterns": ["/taskmgr/**"] }
{ "op": "unregister_reader", "reader_id": "r1" }
{ "op": "publish_global", "event": { "eventid": "/taskmgr/new/task_001", "source_node": "node_a", "source_pid": 1234, "ingress_node": "node_a", "timestamp": 1708588800000, "data": { "ok": true } } }
{ "op": "pull_event", "reader_id": "r1", "timeout_ms": 15000 }
```

响应结构直接采用当前实现中的 `KEventDaemonResponse`：

```json
{ "status": "ok" }
{ "status": "ok", "event": { "eventid": "/taskmgr/new/task_001", "source_node": "node_a", "source_pid": 1234, "ingress_node": "node_a", "timestamp": 1708588800000, "data": { "ok": true } } }
{ "status": "err", "code": "INVALID_PATTERN", "message": "INVALID_PATTERN: daemon only supports global patterns" }
```

语义约束：

- `register_reader` 的 `patterns` 不能为空，且必须全部为全局 pattern
- `publish_global` 的 `event.eventid` 必须为全局 eventid
- `pull_event` 超时时返回 `{ "status": "ok" }`，即 `event` 字段缺失
- `timeout_ms == 0` 表示非阻塞读取
- `reader_id` 为空应视为协议错误

该层协议不规定底层一定是 TCP、Unix Socket、RTCP 或其它 native transport；只规定帧内 payload 语义。也就是说，**transport 可替换，request/response 结构应保持稳定**。

当前 Rust 节点侧实现额外约定了一种默认 TCP transport，便于调试工具或轻量 client 直接访问：

- 监听端口：`3183`
- 连接模型：单 TCP 连接上可连续发送多次 request/response
- frame 格式：`4-byte big-endian length prefix + JSON payload`
- payload：请求端写入 `KEventDaemonRequest` JSON，服务端返回 `KEventDaemonResponse` JSON

示意：

```text
+----------------------+------------------------------+
| u32 len (big-endian) | JSON payload bytes (len 个) |
+----------------------+------------------------------+
```

其中 JSON payload 本身仍然保持上面的 native 协议结构，不额外引入新的字段。

#### 5.4.2 Peer Daemon 协议：单向事件广播

Peer Daemon 之间的协议与上面的请求响应协议不同。当前实现中的 peer 抽象是：

```rust
trait KEventPeerPublisher {
    async fn broadcast(&self, event: &Event) -> KEventResult<()>;
}
```

这意味着 peer 复制链路只需要一件事：**把一个全局 Event 发给对端**。对端收到后调用 `publish_from_peer(event)`，只做本地分发，不再次广播。

因此 peer 协议推荐定义为：

- 长连接
- 单向 push
- 每个 frame 负载为一个完整 `Event`
- 不走 `register_reader / pull_event / unregister_reader`
- 不维护跨节点 reader 状态同步

示意：

```json
{ "eventid": "/taskmgr/new/task_001", "source_node": "node_a", "source_pid": 1234, "ingress_node": "node_a", "timestamp": 1708588800000, "data": { "ok": true } }
```

`ingress_node` 的作用是防环路：

- 本地 pub 进入 Daemon 时，如果没有 `ingress_node`，Daemon 会填成本地 `source_node`
- 只有 `ingress_node == local_node` 的事件才允许继续向 peer 广播
- 从 peer 收到的事件进入 `publish_from_peer()` 后，只做本地分发，不再外扩

因此：

- **Peer 复制协议的核心单位是 `Event`**
- **Client/Daemon 基础协议的核心单位是 `KEventDaemonRequest` / `KEventDaemonResponse`**

两者不要混用。

#### 5.4.3 HTTP Facade 协议：native 语义的 HTTP 映射

为了让 HTTP client 或经 gateway 转发的外部调用也能访问 kevent，推荐提供一个与 native 请求响应协议一一对应的 HTTP 端点：

```text
POST /kapi/kevent
```

请求体直接使用 `KEventDaemonRequest` JSON，响应体直接使用 `KEventDaemonResponse` JSON。

示例：

```http
POST /kapi/kevent
content-type: application/json

{ "op": "pull_event", "reader_id": "r1", "timeout_ms": 15000 }
```

```json
{ "status": "ok", "event": { "eventid": "/taskmgr/new/task_001", "source_node": "node_a", "source_pid": 1234, "ingress_node": "node_a", "timestamp": 1708588800000, "data": { "ok": true } } }
```

这个端点的价值是：

- 与当前 Rust 实现完全对齐
- 易于测试和调试
- Light SDK 若走 HTTP，也可以直接复用
- 后续 transport 从 native 切到 HTTP，不影响语义

但它**不是浏览器主推荐接口**，因为浏览器不适合自己管理 `reader_id + register/pull/unregister` 这整套生命周期。

#### 5.4.4 浏览器推荐协议：HTTP Stream Wrapper

对浏览器，推荐单独提供 stream wrapper：

```text
POST /kapi/kevent/stream
```

请求体：

```json
{
  "patterns": ["/msg_center/user1/box/in/**"],
  "keepalive_ms": 15000
}
```

语义：

1. 服务端收到请求
2. 内部生成 `reader_id`
3. 调用 native API：`register_reader(reader_id, patterns)`
4. 循环调用 native API：`pull_event(reader_id, keepalive_ms)`
5. 将结果写成 NDJSON stream
6. 连接关闭时调用 `unregister_reader(reader_id)`

推荐返回类型：

- `content-type: application/x-ndjson`

推荐 frame：

```json
{ "type": "ack", "connection_id": "c1", "keepalive_ms": 15000 }
{ "type": "event", "event": { "eventid": "/msg_center/user1/box/in/msg_001", "source_node": "ood1", "source_pid": 1234, "ingress_node": "ood1", "timestamp": 1708588800000, "data": { "record_id": "msg_001" } } }
{ "type": "keepalive", "at_ms": 1708588805000 }
{ "type": "error", "error": "INVALID_PATTERN: daemon only supports global patterns" }
```

该协议与 native API 的关系是：

- 浏览器不直接看到 `reader_id`
- `ack` 对应 reader 创建成功
- `event` 对应 native `pull_event` 返回了一个 Event
- `keepalive` 对应 native `pull_event` 超时但连接继续保持
- 断开连接等价于 `unregister_reader`

因此浏览器侧只需要维护一个长连接，不需要自己管理三段式 reader 生命周期。

#### 5.4.5 HTTP 发布协议

如果需要让浏览器或 HTTP client 也能发布全局事件，推荐提供：

```text
POST /kapi/kevent/publish
```

请求体：

```json
{
  "eventid": "/taskmgr/new/task_001",
  "data": { "ok": true }
}
```

服务端收到后应构造完整 `Event`，填充：

- `source_node`
- `source_pid`
- `timestamp`
- `ingress_node`

然后调用内部 native 语义 `publish_external_global(event)`。

成功响应：

```json
{ "status": "ok" }
```

这里不要求浏览器自己传完整 `Event`，原因是：

- `source_pid` 等字段应由服务端决定
- `ingress_node` 是协议控制字段，不应暴露给浏览器随意指定
- 对浏览器公开的 API 应尽量保持简单且安全

#### 5.4.6 协议分层建议

为了避免后续 Web SDK、Light SDK 和 native SDK 的语义漂移，建议固定如下分层：

- **内部核心语义层**：`register_reader / unregister_reader / publish_global / pull_event`
- **Native 协议层**：直接传 `KEventDaemonRequest` / `KEventDaemonResponse`
- **Peer 协议层**：直接传 `Event`
- **HTTP 兼容层**：`POST /kapi/kevent`，直接映射 native 协议
- **HTTP 浏览器层**：`POST /kapi/kevent/stream` 和可选的 `POST /kapi/kevent/publish`

这样一来：

- Rust core 语义稳定后，native SDK 不需要反复调整
- Web SDK 只依赖 HTTP facade，不影响底层实现
- `subscribe()`、`onEvent()`、React hook 等都只是 client 侧 helper，不需要 backend 再改协议

---

## 6. 容错与边界处理

### 6.1 设计哲学

EventBus 是 best-effort 的加速层，所有故障场景的兜底策略统一为：**事件丢失 → 消费端超时 → kMsgQueue 轮询找回**。因此容错设计追求简单，不做复杂补偿。

### 6.2 Daemon 重启

- 纯净启动，无需恢复任何状态
- 重建共享内存区域
- 等待 client SDK 重连并重新注册订阅
- 重连 peer 节点
- 重启期间的事件丢失，由 kMsgQueue 轮询兜底

### 6.3 节点离线

- Peer TCP 连接断开后，暂停向该节点广播事件
- 节点恢复后重新建立 TCP 连接，恢复广播
- 离线期间的事件丢失，由 kMsgQueue 轮询兜底

### 6.4 Reader 消费过慢

每个 reader 拥有独立的 Ring Buffer（固定大小）。当 Ring Buffer 满时，丢弃最旧的事件，保留最新的。消费者下次 `pull_event` 时从 Ring Buffer 当前位置开始读取，丢失的事件由 kMsgQueue 轮询兜底。

### 6.5 共享内存异常

进程崩溃后，其在共享内存中的 Ring Buffer 需要清理。Node Daemon 负责检测进程存活性并回收已死进程的资源。

### 6.6 错误码

| 错误码 | 含义 |
|--------|------|
| `INVALID_EVENTID` | eventid 格式不合法 |
| `INVALID_PATTERN` | 订阅 pattern 格式不合法 |
| `DAEMON_UNAVAILABLE` | 无法连接本机 Node Daemon / 共享内存不可用 |
| `TIMER_INVALID_TARGET` | Timer 的 eventid 不是本地事件（以 `/` 开头） |
| `TIMER_NOT_FOUND` | 取消的 timer_id 不存在 |

注意：不再有 `EVENT_NOT_FOUND`、`NO_MATCH`、`PERMISSION_DENIED`（无注册机制，无内置权限校验）。

---

## 7. 与 kMsgQueue 的协作模式

EventBus 和 kMsgQueue 构成分布式系统通信基础设施的完备组合。两者的关系类似于 Linux 的 epoll + read：EventBus 是信号通道（通知你有新数据），kMsgQueue 是数据通道（实际获取数据）。

| 维度 | EventBus | kMsgQueue |
|------|----------|-----------|
| 定位 | 信号通道（加速层） | 数据通道（可靠层） |
| 状态 | 完全无状态 | 有状态（持久化） |
| 交付语义 | Best-effort，可丢失 | At-least-once，不丢失 |
| 消费模型 | 推（写入 Ring Buffer） | 拉（HTTP GET） |
| 故障影响 | 退化为纯轮询，功能不受影响 | 系统不可用 |
| 消息体 | 轻量通知（信号） | 完整业务数据 |
| 分发方式 | 广播 + 过滤 | 按需拉取 |

### 7.1 为什么需要这个分离

分布式系统里因为 HTTP gateway 的存在，client 和 server 之间维持推流长连接很难做到可靠的分布式扩缩容和故障恢复。Pull 模型（HTTP GET）天然适配 gateway 的无状态路由，任何一次请求都可以被路由到任意后端实例。

EventBus 作为内部设施不经过 HTTP gateway，所以推模型没问题。面向外部或穿越 gateway 的可靠数据交付，由 kMsgQueue 的 HTTP GET 承担。

### 7.2 完整通信基础设施图景

```
BuckyOS 通信基础设施:

  RPC（同步调用）     ← 请求-响应，已有机制
  EventBus（异步通知） ← 本文档，信号通道，推模型
  kMsgQueue（可靠消息） ← 数据通道，拉模型（HTTP GET）

三者各司其职，不应合并。
```

---

## 8. 浏览器视角的 API 结构与使用逻辑

浏览器不是 Full SDK 运行环境，不能直接访问进程内 Ring Buffer、共享内存或 Node Daemon 内部结构。因此浏览器侧看到的 kevent 不应是"直连 Daemon"的协议，而应是**通过 cyfs-gateway 转发到某个 browser-safe HTTP wrapper service** 的能力。

该 wrapper service 可以按 path 或 hostname 被 gateway 路由到任意合适的后端服务；本文档不规定它必须挂在哪个具体服务上，只规定浏览器侧的抽象和语义。

### 8.1 浏览器侧定位

- 浏览器侧 kevent 仍然是**信号通道**，不是可靠数据通道
- 浏览器侧 kevent 只负责"有变化了"的通知；真正的数据仍通过业务 HTTP API 或 kMsgQueue 拉取
- 浏览器侧 kevent 是对服务端 `create_event_reader + pull_event + close` 的一层远程映射
- 浏览器侧只支持**全局 pattern**（以 `/` 开头）
- 浏览器侧不支持本地事件和 Timer，因为两者都是进程内语义

### 8.2 浏览器侧抽象 API

浏览器侧推荐暴露与本地 SDK 尽量一致的消费模型：

```ts
create_event_reader(patterns: string[], options?: BrowserReaderOptions)
  -> Promise<BrowserEventReader>

BrowserEventReader.pull_event(timeout_ms?: number)
  -> Promise<Event | null>

BrowserEventReader.close()
  -> void
```

其中：

- `patterns` 必须全部为全局 pattern
- `pull_event(timeout_ms)` 的语义与本地 SDK 一致
- `timeout_ms == 0` 时立即返回
- `timeout_ms` 省略时表示一直等待，直到收到事件或连接被关闭
- `close()` 用于主动断开 HTTP stream，并释放服务端 reader

建议的浏览器侧配置：

```ts
type BrowserReaderOptions = {
  keepalive_ms?: number
  signal?: AbortSignal
}
```

`keepalive_ms` 仅用于保持 HTTP stream 活性，避免中间网关或浏览器长时间空闲断开；它不改变 EventBus 的 best-effort 语义。

### 8.3 浏览器到服务端的传输映射

浏览器侧不感知 Ring Buffer，也不感知 Daemon。其实际传输可映射为：

1. 浏览器向某个 browser-safe wrapper 发起 HTTP streaming 请求
2. wrapper 在服务端内部调用 `create_event_reader(patterns)`
3. wrapper 循环执行 `pull_event(Some(keepalive_ms))`
4. 有事件时，将事件编码后持续写回浏览器
5. 超时时，写回 keepalive 或空闲帧
6. 浏览器断开连接时，wrapper 调用 `close()` 释放 reader

推荐传输形态：

- `POST` 建立订阅
- 响应为长连接 HTTP stream
- stream 编码推荐 `application/x-ndjson`
- 浏览器使用 `fetch()` + `ReadableStream` 消费

示意：

```text
Browser
  -> POST /kapi/kevent/stream
  -> body: { patterns, keepalive_ms }

Wrapper Service
  -> create_event_reader(patterns)
  -> loop { pull_event(timeout) }
  -> write NDJSON frames
```

返回给浏览器的 stream frame 可采用如下结构：

```json
{ "type": "ack", "connection_id": "..." }
{ "type": "event", "event": { "eventid": "/msg_center/u1/box/in/...", "source_node": "ood1", "source_pid": 1234, "timestamp": 1708588800000, "data": {} } }
{ "type": "keepalive", "at_ms": 1708588800000 }
{ "type": "error", "error": "..." }
```

其中真正对应 kevent 语义的是 `type=event`；其它 frame 只是浏览器长连接场景下的 transport 辅助帧。

### 8.4 浏览器侧标准使用逻辑

浏览器侧的推荐消费模式与普通消费端一致：**event 驱动快路径刷新，轮询负责兜底**。

```text
loop:
    event = browser_reader.pull_event(timeout=N)
    if event != null:
        // 快路径：收到 kevent 通知
        data = http_get(business_api or kmsgqueue, cursor)
        handle(data)
        update(cursor)
    else:
        // 慢路径：超时兜底轮询
        data = http_get(business_api or kmsgqueue, cursor)
        if data != null:
            handle(data)
            update(cursor)
```

关键原则：

- 不要把 kevent stream 当成可靠消息流
- 不要假设每次业务变化都一定能收到 kevent
- 不要要求 wrapper 持久化 cursor 或补发历史事件
- kevent 的职责只是让浏览器更快知道"可能有新数据了"
- 最终一致性由业务 HTTP API 或 kMsgQueue 保证

### 8.5 浏览器侧使用示例

以 `msg_center` 为例，浏览器订阅某个 inbox 的变化通知：

```ts
const reader = await kevent.create_event_reader([
  `/msg_center/${owner}/box/in/**`
]);

let cursor = "";

for (;;) {
  const event = await reader.pull_event(15000);

  if (event) {
    const data = await http_get_kmsgqueue(cursor);
    handle(data);
    cursor = data.next_cursor;
    continue;
  }

  const data = await http_get_kmsgqueue(cursor);
  if (data != null) {
    handle(data);
    cursor = data.next_cursor;
  }
}
```

对 `task_manager` 这类"变化后重新拉取当前状态"的场景，也应采用相同模式：

```ts
const reader = await kevent.create_event_reader([
  `/task_mgr/${task_id}`
]);

for (;;) {
  const event = await reader.pull_event(10000);
  if (event != null) {
    const task = await get_task(task_id);
    render(task);
  } else {
    const task = await get_task(task_id);
    render(task);
  }
}
```

### 8.6 浏览器侧能力边界

浏览器侧 kevent 第一版建议仅支持：

- 订阅全局事件
- 接收事件通知
- 主动关闭订阅

浏览器侧不建议直接支持：

- `pub_event`
- 本地事件订阅
- `create_timer`
- 服务端 reader 持久化恢复
- 历史事件回放

原因是浏览器侧的核心诉求是"通过 gateway 安全地接收通知并触发刷新"，而不是复刻 Full SDK 的全部本地能力。

### 8.7 与服务放置位置的关系

browser-safe wrapper 可以放在任意能够：

- 通过 gateway 暴露 HTTP stream
- 在服务端内部访问 kevent client
- 在同一业务上下文里完成鉴权和数据拉取

的服务中。

例如：

- 某个业务 service 内部自带 kevent wrapper
- 一个专用的 facade service
- 已有的 control-panel 类 service

是否按 path 还是 hostname 暴露，由 gateway 配置决定；这不影响浏览器侧 API 抽象。

## 9. 待讨论事项

**节点规模与拓扑演进**：当前采用全 mesh 广播。若 BuckyOS 未来节点规模增长，可借鉴 NATS 的分层拓扑：核心节点全 mesh，边缘节点作为 Leaf Node 通过 hub-spoke 连接核心集群。这是一个自然的演进路径，不需要在第一版实现。

**Event 消息体大小限制**：建议对 `data` 字段设置大小上限（如 64KB）。EventBus 是信号通道，大数据应通过 kMsgQueue 引用传递。

**监控与调试**：是否需要提供查询接口，如列出当前活跃的 reader、各节点的连接状态等？

**广播流量优化时机**：当前全量广播在小规模下没有问题。如果未来事件量增大且大部分节点不关心大部分事件，可以考虑引入轻量的订阅摘要交换（类似 NATS 的 interest graph pruning），但这应作为后续优化而非第一版需求。
