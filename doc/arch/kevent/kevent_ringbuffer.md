# kevent ringbuffer 设计（v1.1）

## 1. 设计选择：共享内存里放"发布 Ring（每进程一个）"，而不是"全局多写单 Ring"

需求里共享内存路径的核心诉求是：发布端纯内存写入、跨进程微秒级通知、Sub 端读取并匹配，同时还要考虑多进程并发写入和崩溃清理。

如果做成**单个全局 Ring**，会变成"多生产者写同一 Ring"，要同时解决：

* 多生产者在 wrap-around 情况下写同一 slot 的互斥问题
* 生产者崩溃在写入中间态导致 slot 永久卡死的问题
* 还要兼顾"覆盖最旧"的语义
  这会把实现复杂度推高。

因此这里推荐一个更稳、更容易一次做对的结构：

### ✅ 方案：**每个发布者进程（以及 Node Daemon）在共享内存里各有一个 Publish Ring**

* **发布**：进程只写自己的 Publish Ring（单生产者，SP），写入完全无锁、纯内存顺序写。
* **消费**：订阅进程（以及 Node Daemon）读取"所有进程的 Publish Ring"，并在本进程内做 pattern 匹配，把命中的事件写入本进程的 reader ring（进程内 ring）供 `pull_event()` 消费。
* **best-effort 丢失**：任何消费方落后太多，都会因为 ring 覆盖而丢旧事件；消费方能检测丢失并"跳到最新窗口"。

这种设计很好地契合文档里的"广播 + 过滤"哲学（跨节点是广播+过滤，本机也可以用同样思路简化）。
同时，它把"并发写入"从**同一个 ring 的多写**变成了**多 ring 的单写**，极大降低实现风险。

---

## 2. 共享内存整体布局

共享内存由 Node Daemon 创建（`shm_open`/`memfd + fd 传递`均可），Full SDK 进程 mmap 该区域。

建议布局如下（对齐到 4KB page）：

```
+------------------------------+
| ShmHeader (4KB)              |
+------------------------------+
| RingDirectory (N entries)    |  例如 N=256/512
+------------------------------+
| GlobalNotify (cache line)    |  futex 等待用 + dirty_bitmap
+------------------------------+
| PublishRing #0 (fixed size)  |
+------------------------------+
| PublishRing #1               |
+------------------------------+
| ...                          |
+------------------------------+
| PublishRing #N-1             |
+------------------------------+
```

### 2.1 ShmHeader（关键字段）

* `magic` / `version`
* `epoch`：daemon 启动代数（重启 +1，u64）
* `daemon_pid` / `daemon_start_time_ns`（可选，用于诊断）
* `shm_size`
* `dir_version`：RingDirectory 结构变化时递增（seqlock 无锁读快照）
* `config`：slot_size、ring_capacity、max_rings 等参数
* `notify_seq`：futex 等待变量
* `dirty_mask[]`：bitset，长度 = `(max_rings + 63)/64`（见 6.1 扫描优化）

### 2.2 RingDirectory（每个 ring 一个 entry）

每个 entry 描述一个 Publish Ring 的归属与位置（固定大小，便于扫描）：

* `state`（原子 u8）：**三态状态机**
  * `0 (FREE)`：空闲，可分配
  * `1 (INIT)`：写方正在填字段（reader 不可使用）
  * `2 (READY)`：完整可用
* `owner_pid`
* `owner_boot_id`（可选，防 pid 复用；或用 start_time_ns）
* `ring_offset` / `ring_bytes`
* `generation`（entry 被复用时递增，用于消费者重置游标）
* `last_heartbeat_ns`（owner 进程更新，用于清理）

**entry 写入顺序**（写方持有目录 mutex）：

1. `state = INIT`（relaxed store）
2. 填写 `owner_pid / ring_offset / ring_bytes / generation / heartbeat` 等字段
3. `state = READY`（**release store**）

**读方扫描目录时只接受 `state == READY` 的 entry**。这样即使 writer 崩溃在中间态，也只会留下 INIT entry，reader 会忽略；恢复者可以将其回收。

> RingDirectory 的**写入**（新增/释放 ring）用一个 `pthread_mutex(PTHREAD_PROCESS_SHARED | PTHREAD_MUTEX_ROBUST)` 保护；
> RingDirectory 的**读取**走 `dir_version` 的无锁快照（见 5.1）。

---

## 3. Publish Ring 的数据结构（单生产者，多消费者）

### 3.1 固定 slot，避免可变长覆盖难题

为了严格支持"满则覆盖最旧"、并且让消费者能快速检测丢失，Publish Ring 使用**固定数量 slot**，每个 slot 有**固定最大 payload**。

* `capacity`：slot 数，建议 256～4096（按容量规划决定）
* `slot_size`：每条 event 的最大编码长度（建议 1KB～4KB；见容量规划）

> 需求文档建议 EventBus 消息体应尽量轻量，可对 data 设置上限（例如 64KB）作为待讨论事项。这里建议**默认 slot_size 不要做太大**，更符合"信号通道"的定位。

### 3.2 Ring 结构（概念）

* `head_seq`：当前已提交的最大序号（单调递增，u64）。保留 `head_seq` 的目的是让消费者**不用扫 slots 就知道是否有新数据**，作为 fast-path 优化。
* `slots[capacity]`：每个 slot 包含：

  * `seq`（原子 u64）：该 slot 当前存放的事件序号（提交后写入）
  * `len`（u32）：payload 实际长度
  * `payload[slot_size]`：事件二进制帧

**关键点**：

* 生产者写 slot 时，先写 payload/len，然后用 **release store** 写 `slot.seq`（提交点），最后再用 **release store** 写 `head_seq`（fast-path 指示器）。顺序严格为：数据 → slot.seq → head_seq。
* 消费者用 **acquire load** 读 `head_seq` 判断有无新数据，再用 **acquire load** 读 `slot.seq` 校验并做二次校验避免读到覆盖中的数据。即便消费者先看到 `head_seq` 增长，也能在 `slot.seq` 校验处正确等待/跳过，不会读到半条。

> **关于 u64 回绕**：`seq` 单调递增，`target + capacity != target`，双重检查能抓住"整圈覆盖"。唯一的理论漏洞是 u64 回绕（需写 2^64 条），工程上可视为不可能。实现中建议加注释或 assert 说明此假设。

---

## 4. 事件在共享内存里的编码（EventFrame）

文档里事件体是 JSON 样式，但共享内存内不建议直接存完整 JSON 字符串（解析成本高、大小不稳定）。
推荐一个**轻量二进制帧**（SDK 可在返回给用户前再组装为 JSON/对象）：

```
EventFrame := Header + eventid + source_node + ingress_node + data_json_bytes

Header:
- u16 version
- u16 flags              // bit0: reserved
- u32 total_len
- u64 timestamp_ms
- u32 source_pid         // 本机进程发布时填 pid；设备事件置 0
- u16 eventid_len        // <=256
- u16 source_node_len    // <=64
- u16 ingress_node_len   // <=64
- u16 reserved
- u32 data_len           // <= slot_size - header - strings
```

### 4.1 字段语义

* `eventid`、`source_node`、`ingress_node` 用 UTF-8。
* `data` 存 JSON bytes（或 CBOR/MessagePack，也可作为后续优化）。

### 4.2 `source_node` 与 `ingress_node` 的语义区分

* **`source_node`**：**原始发布者标识**
  * 本机进程发布：`source_node = 本节点 NodeId`
  * Light SDK 设备发布：`source_node = device_id / endpoint_id`（设备自身标识）
* **`source_pid`**：仅当发布者是**本机进程**时有意义；设备事件置 `0`
* **`ingress_node`**（v1.1 新增）：**将事件注入 mesh 的 daemon 所在节点 NodeId**
  * 本机进程事件：`ingress_node = 本节点 NodeId`
  * 设备事件：`ingress_node = 接入该设备的节点 NodeId`

> 分离 `source_node` 和 `ingress_node` 的目的是：不丢失"设备是谁"的信息，同时为诊断和归因提供"事件从哪个节点进入 mesh"的能力。防广播风暴的逻辑应基于传播规则（见第 7 节），而不是篡改 `source_node`。

**大小策略（建议默认）**：

* `eventid <= 256`
* `source_node <= 64`
* `ingress_node <= 64`
* `total_len <= slot_size`（超出直接丢弃或只保留摘要字段）

---

## 5. 并发与一致性设计

### 5.1 RingDirectory 的无锁读取快照

发布和消费路径都可能需要扫描当前有哪些 ring。

做法：

1. 写方（创建/释放 ring）在持有跨进程 robust mutex 时：

   * `dir_version += 1`（置为奇数表示更新中，**relaxed** 足够）
   * 修改目录 entry
   * `dir_version += 1`（变回偶数表示更新完成，**release**）
2. 读方：

   * `v1 = load_acquire(dir_version)`，若为奇数则重试
   * 扫描目录（**只取 `state == READY` 的 entry**）
   * `v2 = load_acquire(dir_version)`，`v1 == v2 && v2 为偶数` 才接受快照

这样发布路径无需拿锁，仍保持"纯内存快路径"。

#### 5.1.1 `dir_version` 奇数卡死的恢复逻辑

seqlock 风格的 `dir_version` 奇偶无锁读是正确的，但如果 writer 在"置奇数后"崩溃，reader 会永远看到 odd 而自旋。这与"系统可退化但不应卡死"的容错哲学相冲突。

**A. ROBUST mutex 恢复者逻辑**

目录写锁使用 `pthread_mutex(PTHREAD_PROCESS_SHARED | PTHREAD_MUTEX_ROBUST)`。任何拿锁返回 `EOWNERDEAD` 的线程/进程，必须做以下恢复动作：

1. **如果 `dir_version` 为奇数：`dir_version += 1` 变回偶数**（解除 reader 永久自旋）
2. 扫描目录：把所有 `state == INIT` 且 `owner_pid` 不存在/心跳超时的 entry 改回 `FREE`，并 `generation++`
3. 调 `pthread_mutex_consistent()` 再继续正常修改

伪代码：

```c
lock_dir(mtx):
  r = pthread_mutex_lock(mtx)
  if r == EOWNERDEAD:
    // 1) 修复 seqlock 奇数
    if (atomic_load_relaxed(dir_version) & 1) {
        atomic_fetch_add_relaxed(dir_version, 1);
    }
    // 2) 清理 INIT entry
    cleanup_init_entries();
    pthread_mutex_consistent(mtx);
    return RECOVERED;
  return r;
```

**B. Reader "看见奇数太久"走 slow-path 修复**

存在一种极端情况：崩溃后再也没有任何写目录的人（例如所有 ring 都已分配完），reader 仍会卡死。

因此在 reader 侧加一个"超时后尝试修复"的慢路径：

* 连续看到 `dir_version` 为 odd 超过阈值（例如 1~5ms 或 N 次循环）
* 尝试 `pthread_mutex_trylock` 获取目录锁：
  * 成功：如果仍 odd，则 `dir_version++` 修复，并清理 INIT entry
  * 返回 `EOWNERDEAD`：按恢复者流程修复

这条慢路径平时几乎不走，但能保证系统不会"永久自旋"。

### 5.2 Publish 写入算法（单生产者，无锁）

每个进程只写自己的 ring，因此写入不需要 CAS 争用。

伪代码：

```c
publish(bytes payload, u32 len):
    if len > slot_size: drop_and_stat(); return

    seq = head_seq + 1
    idx = seq & (capacity - 1)          // capacity 取 2^k，快速取模
    slot = slots[idx]

    slot.len = len
    memcpy(slot.payload, payload, len)

    atomic_store_release(slot.seq, seq)  // 提交点：先数据后 seq
    atomic_store_release(head_seq, seq)  // fast-path 指示器，必须在 slot.seq 之后

    // 通知消费者（配合 dirty_bitmap 优化）
    old = atomic_fetch_or(dirty_mask[ring_id / 64], 1ULL << (ring_id % 64))
    if !(old & (1ULL << (ring_id % 64))):
        // 首次置脏，才做 futex_wake（避免每条 event 都 wake）
        atomic_fetch_add(notify_seq, 1)
        futex_wake(&notify_seq, INT_MAX)
    else:
        // 已有脏位，仍需更新 notify_seq 以唤醒新等待者
        atomic_fetch_add(notify_seq, 1)
```

### 5.3 Consumer 读取算法（每个消费者对每个 ring 一份游标）

订阅进程会维护一张 `ring_id -> read_seq` 的表（存在进程私有内存即可）。

伪代码：

```c
consume_one_from_ring(ring):
    head = atomic_load_acquire(ring.head_seq)
    if read_seq == head: return NONE

    // 落后太多：丢弃最旧，跳到最新窗口（满足"保留最新"）
    if head - read_seq > capacity:
        read_seq = head - capacity
        stat_drop += ...

    target = read_seq + 1
    idx = target & (capacity - 1)
    slot = ring.slots[idx]

    s1 = atomic_load_acquire(slot.seq)
    if s1 != target:
        // 说明 target 已被覆盖，或 slot 还没提交（极短窗口）
        if s1 > target:
            // 覆盖：直接跳到 slot 当前序号附近
            read_seq = s1 - 1
        return NONE

    len = slot.len
    memcpy(local_buf, slot.payload, len)

    s2 = atomic_load_acquire(slot.seq)
    if s2 != target:
        // 读的过程中被覆盖，丢弃本条
        stat_race_drop += ...
        return NONE

    read_seq = target
    return decode(local_buf)
```

这套逻辑保证：

* **不会把被覆盖过程中的半条数据交给上层**
* **消费者慢会丢旧事件，并自动追到最新窗口**
* 单生产者无需任何互斥，发布延迟非常低

---

## 6. `pull_event()` 的阻塞与唤醒

共享内存 ring 本身解决"跨进程传输"，但 API 需要 `pull_event(timeout)` 阻塞。

推荐在 Full SDK 内做一个**ShmDispatch 线程**（每进程 1 个即可）：

* 等待 `notify_seq` 的 futex（带 timeout）
* 醒来后利用 `dirty_bitmap` 确定需要扫描的 ring（见 6.1）
* 依次从脏 ring 拉取新事件
* 对每条事件做 pattern 匹配（本进程内 local_bus）
* 命中则写入对应 `EventReader` 的进程内 ring，并唤醒 `pull_event`

这样：

* `pull_event` 只需要等待**自己 reader 的进程内 ring**（简单）
* 共享内存侧只负责高速"event stream 输送"
* 本机跨进程完全不需要 Unix Socket 往返，符合文档目标

> 如果你不想引入线程，也可以把"从 shm 拉取并分发"的动作放进 `pull_event` 内部（先 drain shm 再等），但会让等待逻辑更复杂（等待两个来源）。

### 6.1 ShmDispatch 扫描优化：`dirty_bitmap`

为减少"每次 wake 扫描所有 ring 的 head_seq"的开销（尤其在 ring 数量增长或 IoT/家庭节点资源有限时），引入 `dirty_bitmap`。

**结构**：在 `ShmHeader` 中放置：

* `notify_seq`：futex 等待变量
* `dirty_mask[]`：bitset，长度 = `(max_rings + 63) / 64`

**producer 行为**：发布者写入后 `fetch_or(dirty_mask[word], bit)`；仅从 0→1 时执行 `futex_wake`。

**dispatcher 行为**：被唤醒后：

* 对每个 word：`bits = exchange(dirty_mask[word], 0)`
* 只扫描 `bits` 里对应的 ring，并"drain 到没有新事件"

这能把扫描范围从"所有 ring"降低成"只有发生变化的 ring"，减少 CPU cache 污染。

---

## 7. Node Daemon 如何使用这些 Publish Rings

文档要求 daemon 隐式订阅本机所有全局事件，并广播到 peers；同时接收 peer 广播后写入共享内存供本机进程消费。

在本设计中：

* **本机进程发布的全局事件**：写到各自的 Publish Ring；daemon 的 ShmDispatch 扫描所有 ring，拿到事件后：

  * 若 `ingress_node == local_node`：广播给 peers
  * 写入本机侧无需额外动作（因为本机订阅进程也在读这些 rings）
* **peer 发来的事件**：daemon 收到网络包后：

  * 用 `local_subscriptions` trie 判断本机是否有人关心（文档要求"接收端过滤"）
  * 若关心：写入 **daemon 自己的 Publish Ring**（daemon 也是一个 producer）
  * 若不关心：丢弃
* **外部 Light SDK 设备发来的事件**：daemon 写入自己的 Publish Ring，`source_node` 填设备自身标识（device_id），`ingress_node` 填本节点 NodeId。

### 7.1 防广播风暴规则

结合需求文档的跨节点流程（只在"事件产生的节点"广播一次；接收端过滤后不再转发），防环路最清晰的规则是：

* **本地产生（含外部设备接入）的事件**（`ingress_node == local_node`）：daemon 广播给 peers
* **从 peer 收到的事件**（`ingress_node != local_node`）：daemon 只做本机过滤 + 投递，**不再二次广播**

这样既不丢"设备是谁"信息，也不会产生广播风暴。

> 如果后续需要多跳/分层拓扑，再引入 `hop_count/ttl` 或 `event_uuid + LRU seen-set`，但第一版全 mesh 并不需要。

这样做能把 daemon 的"输入统一入口"也变成 ring：逻辑非常直，符合"无状态、纯内存"的定位。

---

## 8. 进程崩溃清理（生产者 ring 回收）

文档明确提到"进程崩溃后共享内存 ring buffer 需要清理，daemon 负责检测并回收"。

这里的回收对象是 **RingDirectory entry + ring 空间**：

### 8.1 心跳与存活检测

* 每个 producer 进程（以及 daemon）定期更新自己 entry 的 `last_heartbeat_ns`
* daemon 周期性扫描目录：

  * `kill(pid, 0)` 或检查 `/proc/<pid>` 判断是否存活
  * 或 heartbeat 超时（例如 3s）视为异常

### 8.2 回收流程

* 将 entry 标记为 `state = FREE`
* `generation++`（让消费者发现 ring 被复用）
* `dir_version` 更新
* ring 内容不用清零（下次分配时会覆盖）

> 同时清理 `state == INIT` 且进程不存在/心跳超时的 entry（v1.1 新增）。

### 8.3 消费者如何处理 ring 被复用

* 消费者扫描目录时，如果发现某个 entry 的 `generation` 变化：

  * 将该 ring 的 `read_seq` 重置为当前 `head_seq`（从最新开始）
  * 避免把新进程的数据当成旧进程继续读

---

## 9. Daemon 自身崩溃/重启恢复

需求文档对 daemon 重启的预期是：纯净启动，重建共享内存区域，等待 client 重连并重新订阅。

### 9.1 ShmHeader 重启相关字段

* `epoch`：daemon 每次启动递增（u64，release store）
* `daemon_pid` / `daemon_start_time_ns`（可选，用于诊断）

### 9.2 Daemon 启动流程

1. 打开 shm：
   * 若存在且 `magic/version/size` 兼容：可选择复用（减少本机跨进程抖动）
   * 若不兼容：删除并重建
2. `epoch++`（写入 shm header，release store）
3. 扫描 RingDirectory：
   * 清理 dead pid / heartbeat 超时 entry → `state = FREE`（原第 8 节已有）
   * 清理 `state == INIT` 的 entry → `state = FREE`
   * 如果发现 `dir_version` 为奇数，修回偶数（见 5.1.1）
4. 启动 control-plane（unix socket / tcp）接收 SDK 的 global pattern 注册，重建 `local_subscriptions` trie（daemon 内存态）

### 9.3 SDK 行为

* SDK 在 `pull_event()` 或 ShmDispatch 主循环里周期性读取 `epoch`（或订阅 control socket 的重连事件）：
  * 发现 `epoch` 变化 ⇒ 重新把自己的 global patterns 注册给 daemon
  * shm 如果被重建（名字变化或 fd 失效）⇒ 重新 mmap 并重新分配自己的 Publish Ring

这套机制和"daemon 无状态、可随时重启"的目标一致：daemon 不需要从磁盘恢复，只需要等 SDK 重新声明订阅即可。

---

## 10. 容量规划建议（slot_size / capacity / ring 数）

文档提到需要做容量规划。
给出一个实用的 sizing 方法：

### 10.1 关键参数

* `R`：某 producer 的峰值事件率（events/s）
* `T`：消费者最大可能"读不到 ring"的时间窗口（s）

  * 例如进程 GC、调度抖动、短暂停顿等
* `capacity >= R * T * safety_factor`

  * `safety_factor` 建议 2～4

举例：

* 峰值 5k events/s，消费者可能 50ms 才醒一次
* `capacity >= 5000 * 0.05 * 4 = 1000`
  => 选 `capacity=1024`

### 10.2 slot_size 的建议

EventBus 是信号通道，不是数据通道；大数据应走 kMsgQueue。
因此默认建议：

* `slot_size = 1024`（默认）
* 超过直接 drop（或仅保留 eventid + 摘要）

### 10.3 默认配置（适合个人/家庭节点）

* `max_rings = 16`
* `capacity = 512`
* `slot_size = 1024`

粗略内存：

* `16 × 512 × 1024 ≈ 8MB`（再加少量 header/对齐/slot 元数据）

### 10.4 高负载环境配置

提供配置项让高负载环境按需调大：

* `max_rings = 64`
* `capacity = 1024`
* `slot_size = 2048`

粗略内存：

* `64 × 1024 × 2048 ≈ 128MB`

### 10.5 内存预算估算公式

总内存约为：

`total_bytes ≈ ring_count * capacity * (slot_overhead + slot_size) + directory`

其中 `slot_overhead ≈ 16~32B`。

---

## 11. 与需求文档逐条对齐（你可以据此落实现有接口）

* **低延迟、纯内存写**：发布只写本进程 ring（memcpy + 原子 store），不走 Unix Socket。
* **best-effort**：消费者落后会被覆盖并自动跳最新，丢失由 kMsgQueue 兜底。
* **满则丢弃最旧保留最新**：覆盖天然发生；消费者检测 `head-read > capacity` 直接跳到 `head-capacity`。
* **Daemon 无状态**：目录与 ring 都在共享内存；daemon 重启后 `epoch++` + 重新 mmap + 扫描目录即可继续（不依赖磁盘）；SDK 感知 epoch 变化后自动重新注册 global patterns。
* **进程崩溃清理**：daemon 通过 pid/heartbeat 回收 entry；ROBUST mutex 恢复者修复 seqlock 卡死；INIT 态 entry 被自动清理。
* **事件溯源清晰**：`source_node` 保留原始发布者身份，`ingress_node` 标识接入节点，防广播风暴基于传播规则而非篡改来源。

---

## 12. 可选增强（不影响 v1，但能明显提升可维护性/可观测性）

1. **统计与诊断**（共享内存里放 counters）

   * `dropped_overrun`（消费者落后导致的丢弃数）
   * `dropped_oversize`（payload 超 slot_size）
   * `dispatch_matched / dispatch_filtered`
   * `stat_race_drop`（读取过程中被覆盖的丢弃数）
2. **调试接口**

   * daemon 提供 debug 命令：列出 ring/进程、head/read 差值、丢弃计数
3. **分层 ring（按 namespace 分组）**

   * 事件量大时，可按 eventid 第一段（`/taskmgr`）拆多个 ring，减少扫描与缓存污染

---

## 附录：v1.1 变更清单

相对于 v1.0 的主要变更如下：

* **RingDirectory entry 状态机**：`in_use` 替换为三态 `state: FREE/INIT/READY`；writer 遵循 INIT → 填字段 → READY(release) 顺序；reader 只读 READY。
* **seqlock `dir_version` 恢复逻辑**：
  * writer 内存序明确化：odd(relaxed) → 改目录 → even(release)
  * reader 两次 acquire 快照
  * ROBUST mutex 恢复者：若 odd ⇒ +1 修偶，并清理 INIT entry
  * reader slow-path：odd 停留过久 ⇒ trylock 修复（防无 writer 场景）
* **Publish 写入内存序明确化**：数据 → `store_release(slot.seq)` → `store_release(head_seq)`
* **ShmDispatch 扫描优化**：header 增 `dirty_bitmap`；producer fetch_or 置位，仅从 0→1 时 futex_wake；dispatcher exchange 取位图只扫置位 ring。
* **事件来源字段分离**：
  * `source_node` = 原始发布者（进程：nodeid；设备：device_id）
  * `source_pid` = 进程 pid / 设备为 0
  * 新增 `ingress_node` = 接入 mesh 的节点 nodeid（用于诊断/归因）
  * 广播规则：仅 ingress daemon 广播；peer 收到不再二次广播
* **Daemon 重启恢复**：header 里 `epoch++`；SDK 发现 epoch 变化后重新注册 global patterns；shm 兼容可复用，不兼容则重建。
* **默认容量配置下调**：默认 max_rings=16, capacity=512, slot_size=1024（约 8MB），适合家庭节点；高负载可调大。