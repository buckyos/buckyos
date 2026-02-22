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

## 8. 待讨论事项

1. **共享内存 Ring Buffer 的具体实现**：进程间共享内存的 Ring Buffer 需要处理并发写入、进程崩溃清理、容量规划等细节。是否第一版先用 Unix Socket 经过 Daemon 中转作为简单实现，后续再优化为共享内存？

2. **Callback/Push 模式的 API 暴露**：当前 API 仅暴露 `pull_event()`。是否需要在 SDK 层提供 `on_event(callback)` 的 push 风格接口？

3. **节点规模与拓扑演进**：当前采用全 mesh 广播。若 BuckyOS 未来节点规模增长，可借鉴 NATS 的分层拓扑：核心节点全 mesh，边缘节点作为 Leaf Node 通过 hub-spoke 连接核心集群。这是一个自然的演进路径，不需要在第一版实现。

4. **Event 消息体大小限制**：建议对 `data` 字段设置大小上限（如 64KB）。EventBus 是信号通道，大数据应通过 kMsgQueue 引用传递。

5. **监控与调试**：是否需要提供查询接口，如列出当前活跃的 reader、各节点的连接状态等？

6. **Timer 精度保证**：SDK 层 Timer 的精度受进程调度影响，是否需要明确精度预期（如毫秒级尽力而为，不保证硬实时）？

7. **广播流量优化时机**：当前全量广播在小规模下没有问题。如果未来事件量增大且大部分节点不关心大部分事件，可以考虑引入轻量的订阅摘要交换（类似 NATS 的 interest graph pruning），但这应作为后续优化而非第一版需求。
