# kevent / kmsg 测试方案

## 目录

- [1. 任务确认](#1-任务确认)
- [2. 当前实现边界](#2-当前实现边界)
  - [kevent](#kevent)
  - [kmsg](#kmsg)
  - [kevent 与 kmsg 的协作](#kevent-与-kmsg-的协作)
  - [调用方边界](#调用方边界)
- [3. 测试代码存放目录](#3-测试代码存放目录)
  - [模块级自动化测试](#模块级自动化测试)
  - [真实环境链路测试](#真实环境链路测试)
  - [路由配置测试](#路由配置测试)
  - [测试报告和证据](#测试报告和证据)
- [4. 已有覆盖与补缺范围](#4-已有覆盖与补缺范围)
  - [kmsg 已有覆盖](#kmsg-已有覆盖)
  - [kmsg 补缺范围](#kmsg-补缺范围)
  - [kevent 已有覆盖与补缺重点](#kevent-已有覆盖与补缺重点)
- [5. 测试分层](#5-测试分层)
  - [L1：模块级功能约定测试](#l1模块级功能约定测试)
  - [L2：系统配置与路由测试](#l2系统配置与路由测试)
  - [L3：真实环境链路测试](#l3真实环境链路测试)
  - [L4：性能和压力测试](#l4性能和压力测试)
- [6. 详细测试项](#6-详细测试项)
  - [6.0 编号说明与状态判断](#60-编号说明与状态判断)
  - [A. kevent 模块功能测试](#a-kevent-模块功能测试)
  - [B. kmsg 已有覆盖与补缺测试](#b-kmsg-已有覆盖与补缺测试)
  - [C. kevent + kmsg 联动测试](#c-kevent--kmsg-联动测试)
  - [D. 使用场景归纳后的功能测试](#d-使用场景归纳后的功能测试)
  - [E. 路由和真实环境测试](#e-路由和真实环境测试)
- [7. 性能测试设计](#7-性能测试设计)
  - [kevent performance](#kevent-performance)
  - [kmsg performance](#kmsg-performance)
- [8. 环境构建](#8-环境构建)
  - [本地模块测试环境](#本地模块测试环境)
  - [真实环境测试环境](#真实环境测试环境)
- [9. 测试复现指南](#9-测试复现指南)
- [10. 推进步骤](#10-推进步骤)
- [11. 报告和进度规则](#11-报告和进度规则)
- [12. 风险和取舍](#12-风险和取舍)
- [13. 测试视角：以设计和生产语义为准](#13-测试视角以设计和生产语义为准)
  - [当前已发现的设计 / 实现偏差](#当前已发现的设计--实现偏差)
- [14. 附录：测试方案维护原则](#14-附录测试方案维护原则)

## 1. 任务确认

本方案只基于当前工作区实际存在的文档和代码设计测试，不引用已删除或工作区不存在的资料。重点参考：

- `doc/arch/kevent.md`
- `doc/arch/kmsg.md`
- `src/kernel/buckyos-api/src/kevent_client.rs`
- `src/kernel/buckyos-api/src/kevent_ringbuffer.rs`
- `src/kernel/buckyos-api/src/msg_queue.rs`
- `src/kernel/kevent/src/*.rs`
- `src/kernel/kmsg/src/*.rs`
- `src/kernel/node_daemon/src/kevent_server.rs`
- `src/test/test_boot_gatweay/*`
- `test/run.py`
- `doc/arch/system_events.md`
- `doc/arch/task_mgr.md`
- `doc/control_panel/ARCHITECTURE.context.md`
- `doc/message_hub/Message Tunnel Design.md`
- `doc/opendan/Agent Task Executor.md`
- `doc/agent_tool/build-in agent-tool手册.md`
- `src/frame/msg_center/src/msg_center.rs`
- `src/frame/control_panel/src/message_hub.rs`
- `src/frame/opendan/src/msg_center_pump.rs`
- `src/frame/opendan/src/session_event_pump.rs`

目标是提供一套可落地、可重复执行、不过度设计的测试体系，覆盖 kevent 和 kmsg 作为生产基础通信能力必须满足的功能、可靠性、性能和真实链路行为。仓库内只维护测试计划、测试入口和当前测试结论；当前结论记录在 `notepads/kevent_kmsg_test_report.md`，每轮详细执行日志和原始输出可在本地归档到 `test/kevent_kmsg/reports/`，该目录不纳入版本控制。

## 2. 当前实现边界

### kevent

当前 kevent 由几层组成：

- SDK/client 层：`KEventClient`、`EventReader`、Timer、Local/Full/Light/LocalPubOnly 模式。
- shared ringbuffer 层：`SharedKEventRingBuffer`，用于本机跨 client / 跨进程快速通知。
- daemon/service 层：`KEventService`，负责 global reader 注册、事件分发、peer broadcast、HTTP/native 协议处理。
- HTTP wrapper 层：`/kapi/kevent`、`/kapi/kevent/stream`、`/kapi/kevent/publish`。
- node-daemon native TCP 层：`KEVENT_SERVICE_NATIVE_PORT` 上的 framed JSON 协议。

核心语义：

- EventBus 是“尽力通知”通道，不保证事件一定送达，也不保存历史事件。
- local event 只在进程内投递。
- global event 通过 daemon / shared ring / peer 路径投递。
- reader 使用 pattern 匹配，支持 global `*` / `**`，local pattern 是精确匹配。
- Timer 是 SDK 本地能力，只能发布 local event。
- 浏览器侧只应通过 HTTP stream wrapper 消费 global event。

### kmsg

当前 kmsg 是 sled-backed kRPC/HTTP 消息队列服务：

- API 类型和客户端封装在 `buckyos-api/src/msg_queue.rs`。
- 服务实现是 `src/kernel/kmsg/src/sled_msg_queue.rs`。
- 进程入口是 `src/kernel/kmsg/src/main.rs`，挂载到 `/kapi/kmsg`。

核心语义：

- kmsg 是可靠数据通道，承担持久化消息交付。
- 支持 queue 创建/删除/配置/统计。
- 支持 post/read/fetch/subscribe/unsubscribe/commit_ack/seek/delete_message_before。
- 支持 `SubPosition::Earliest`、`Latest`、`At(index)`。
- `fetch_messages(auto_commit=false)` 应保持 at-least-once 能力，必须显式 `commit_ack`。
- `read_message` 不应改变订阅 cursor。

### kevent 与 kmsg 的协作

文档定义的生产模式是：

- kevent 只做“有变化”的低延迟通知。
- kmsg 保存完整业务数据。
- 消费端收到 kevent 后快速拉 kmsg；收不到 kevent 时也通过轮询 kmsg 兜底。

因此必须测试两者的组合语义，而不是只分别测各自 API。

### 调用方边界

除 kevent/kmsg 模块本身外，测试还需要覆盖当前代码中已经形成的调用方契约：

- `msg_center` 在消息 box 变化时发布 `/msg_center/{owner}/box_{box}_{owner}/changed` 事件，payload 包含 `operation`、`owner`、`box_kind`、`box_id`、`record_id`、`msg_id`、`state`、`updated_at_ms`。
- `control_panel` 的 chat stream 订阅 `/msg_center/{owner}/box_in_{owner}/**` 和 `/msg_center/{owner}/box_out_{owner}/**`，收到事件后必须重新读取 `MsgCenterClient.get_record(...)`，事件只作为加速信号。
- `OpenDAN msg_center_pump` 订阅 `/msg_center/{owner}/box_{in,group_in,request}_{owner}/**`；无法识别的 `/msg_center/` 事件应防御性 sweep 全部 inbox 类 box。
- `OpenDAN session_event_pump` 聚合各 session 的 kevent 订阅 pattern，动态重建 reader，并把匹配事件路由成目标 session 的 `Inbound::Event`。
- `TaskManagerClient::wait_for_task_end_kevent` 明确把 kevent 当作加速层：订阅后立即回读 `get_task`，event 或 timeout 后都回读真相源，订阅失败时退化为轮询。
- `agent_tool` 文档提供 `subscribe_event` / `unsubscribe_event`，说明 Agent Session 可以订阅 KEvent pattern；这依赖 `session_event_pump` 的 pattern 聚合、路由和 reader 重建行为。

## 3. 测试代码存放目录

### 模块级自动化测试

测试代码不放入模块源码文件，不扩展现有 `src/*.rs` 内部 `mod tests`。

- `src/kernel/kevent/tests/`
  - `client_contract.rs`
  - `service_contract.rs`
  - `http_contract.rs`
  - `shared_ring_contract.rs`
  - `usage_contract.rs`
  - `performance.rs`
- `src/kernel/kmsg/tests/`
  - `sled_contract.rs`
  - `rpc_contract.rs`

`kmsg` 当前是 binary crate，没有 `src/lib.rs`。只允许 `sled_contract.rs` 用一次 `#[path]` 引入底层实现，用于验证持久化、cursor、retention、权限、错误行为、并发和性能参考数据；不得写面向私有分支覆盖率的白盒测试。

```rust
#[path = "../src/sled_msg_queue.rs"]
mod sled_msg_queue;
```

注意：被引入文件里的 `#[cfg(test)]` 内容可能会在该测试中再次参与编译或运行；如果造成重复或冲突，应改为拆出 `lib.rs` 或调整测试结构。`rpc_contract.rs` 不引入 sled，只用测试内 fake `MsgQueueHandler` 验证 RPC/handler 接口行为。真实 HTTP `/kapi/kmsg` 行为放到 DV 测试。

### 真实环境链路测试

统一目录为 `test/kevent_kmsg/`：

| 目录 | 目的 | 环境要求 |
| --- | --- | --- |
| `dv/` | gateway 入口下的 kevent/kmsg 最小闭环、POST/GET 行为记录、event 唤醒后回读 kmsg。 | 标准 devtest 环境；Deno、pnpm、bash。 |
| `restart/` | BuckyOS 服务重启后 kmsg 消息可读、kevent stream 可恢复，subscription 丢失作为偏差记录。 | 独立 Linux 测试机或可重建 devtest 环境。 |
| `peer_container/` | Docker 网络命名空间下 native framed 单向 peer 投递。 | Linux 测试机、Docker。 |
| `peer_vm/` | QEMU/KVM VM 隔离下 native framed 单向 peer 投递。 | Linux 测试机、`/dev/kvm`、`qemu-system-x86_64`、`qemu-img`、`genisoimage`。 |
| `reports/` | 本地每轮详细测试数据和当轮结论，git 忽略，不作为仓库交付内容。 | 无。 |

执行入口：

```bash
uv run test/run.py -p kevent_kmsg
uv run test/run.py -p kevent_kmsg/dv
uv run test/run.py -p kevent_kmsg/restart
uv run test/run.py -p kevent_kmsg/peer_container
uv run test/run.py -p kevent_kmsg/peer_vm
```

`test/run.py --list` 通过 `test/kevent_kmsg/main.py` 发现分组入口。分组入口默认只打印子项说明，不直接运行重启、Docker 或 VM 测试。`dv/` 和 `restart/` 的 `package.json` 必须包含 `scripts.test`，runner 会在对应目录执行 `bash -lc "pnpm install && pnpm test"`。

真实环境测试使用 Deno runner，并固定 WebSDK 依赖：

```json
{
  "name": "kevent-kmsg-test",
  "private": true,
  "type": "module",
  "scripts": {
    "test": "deno run --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors kevent_kmsg_dv.ts"
  },
  "dependencies": {
    "buckyos": "github:buckyos/buckyos-websdk#beta2.2"
  }
}
```

### 路由配置测试

路由测试复用并必要时补充 `src/test/test_boot_gatweay/`。该目录覆盖 `/kapi/kevent/*` 早转发和 `/kapi/kmsg/*` service 路由，不另建重复路由测试。

### 测试报告和证据

每轮详细证据可在执行者本地保存到 `test/kevent_kmsg/reports/<yyyyMMdd-HHmmss>/`，该目录被 git 忽略。建议至少包含：

- `commands.log`
- `cargo-kevent.log`
- `cargo-kmsg.log`
- `boot-gateway.log`
- `dv.log`
- `performance.json`
- `report.md`

仓库内的当前结论和进度索引只写入 `notepads/kevent_kmsg_test_report.md`。
## 4. 已有覆盖与补缺范围

### kmsg 已有覆盖

`src/kernel/kmsg/src/sled_msg_queue.rs` 已有较完整的内部测试，实施本计划时应复用这些测试，不重复搬到新测试文件里：

- `test_msg_queue_end_to_end`：覆盖 create/update/post/stats/subscribe/fetch/commit_ack/read/seek/delete/unsubscribe/delete_queue 的主流程。
- `test_multiple_subscribers_and_messages`：覆盖多订阅者、不同起始位置、独立 cursor、追加消息和裁剪后的读取。
- `test_path_queue_name_roundtrip`：覆盖绝对路径型 QueueUrn 和路径型 subscription id。

`src/kernel/buckyos-api/src/msg_queue.rs` 也已有 mock client 层测试，覆盖 `MsgQueueClient` 的 in-process 行为、`read_message` 无副作用、`commit_ack`、`seek` 和路径 QueueUrn。

### kmsg 补缺范围

补缺测试不重复 KM-01 到 KM-10 的 happy path。默认补缺范围收敛为：

- 持久化 reopen：确认 drop/reopen 后 queue、message、subscription cursor 仍符合预期。
- config 生效：`max_messages`、`retention_seconds`、权限字段目前疑似未实现，需要用测试确认并输出偏差。
- RPC/HTTP 行为：模块级测试验证公开 RPC/handler 的方法名、参数解析和错误返回；真实 HTTP `/kapi/kmsg` 的 POST/GET 当前行为放到真实环境链路测试。
- 并发和性能：验证 index 唯一、stats 正确，记录性能参考数据。
- 真实服务链路：通过 gateway 的最小验证确认生产入口可用。

kmsg 补缺测试放在 `src/kernel/kmsg/tests/*` 或 DV 目录。现有内部 `#[cfg(test)]` 测试继续保留并运行，但不作为补缺测试的默认落点。

### kevent 已有覆盖与补缺重点

`kevent` 和 `buckyos-api` 已有较多内部单元测试，覆盖 local pub/sub、pattern、timer、light mode、shared ring、HTTP wrapper、native transport 等。`src/kernel/kevent/tests/*` 的重点不是重复内部断言，而是把公开契约和设计偏差固化为独立 integration tests，尤其是：

- HTTP/native 对外协议。
- shared ring 数据大小和 overflow 边界。
- reader 生命周期和错误码一致性。
- peer 防环路和跨节点能力缺口。

## 5. 测试分层

### L1：模块级功能约定测试

目的：快速、稳定地验证模块承诺的基本行为。默认纳入日常 `cargo test`。

环境：

- 在 `src/` 目录运行。
- 不依赖完整 BuckyOS 运行环境。
- 使用 `tempfile` 或临时文件隔离 sled DB 和 shared ringbuffer。

命令：

```bash
cd src
cargo test -p kevent
cargo test -p kmsg
```

说明：

- `cargo test -p kevent` 包含现有单元测试和 kevent integration tests。
- `cargo test -p kmsg` 运行现有 kmsg 内部测试，以及 `src/kernel/kmsg/tests/*` 测试。

### L2：系统配置与路由测试

目的：验证 rootfs / boot gateway 配置能把 kevent 和 kmsg 暴露到预期入口。

环境：

- 仓库根目录。
- 可执行 `cyfs_gateway debug`，脚本会自动查找常见路径。

命令：

```bash
uv run src/test/test_boot_gatweay/run_debug_tests.py
```

### L3：真实环境链路测试

目的：验证接近生产形态的完整链路：gateway -> service -> storage/event。

环境：

- 仓库根目录。
- 先启动 devtest 环境：

```bash
uv run src/start.py --all
uv run src/check.py
```

命令：

```bash
uv run test/run.py -p kevent_kmsg/dv
```

真实环境测试必须通过 gateway 暴露入口访问 `/kapi/kevent` 和 `/kapi/kmsg`，不直接绕过 gateway 调服务进程端口。若调试阶段需要直连端口，只能作为临时定位手段，不计入最终通过证据。

### L4：性能和压力测试

目的：覆盖基础吞吐、延迟、并发和资源退化风险。

性能测试默认标记为 ignored，避免拖慢普通开发循环。确认后实现为 integration test 中的 ignored tests。

命令：

```bash
cd src
cargo test -p kevent --test performance -- --ignored --nocapture
cargo test -p kmsg --test sled_contract -- --ignored --nocapture
```

`kmsg` 性能参考数据放在 `src/kernel/kmsg/tests/sled_contract.rs` 的 ignored case 中，用唯一一次 `#[path]` 引入方式验证底层写读性能；公开 RPC 和真实环境性能参考数据仍放在 `test/kevent_kmsg/dv`。

性能测试不直接写死产品 SLO。默认只做正确性检查和性能参考记录：测试失败条件主要是数据错误、panic、死锁、明显卡死或资源耗尽；吞吐和延迟写入 `performance.json` 作为基线。产品定义明确 SLO 后，再把基线升级为硬门槛。

## 6. 详细测试项

### 6.0 编号说明与状态判断

测试项编号只用于追踪计划和报告，不代表源码模块名，也不代表实现层级。编号前缀含义如下：

| 前缀 | 中文含义 | 说明 |
| --- | --- | --- |
| `KE-*` | kevent 模块测试 | 验证 kevent 自身能力，例如订阅、发布、HTTP 接口、共享环形队列。 |
| `KM-EXIST-*` | kmsg 已有测试 | 当前代码里已经存在的 kmsg 测试，纳入计划是为了复用和追踪，避免重复写同类测试。 |
| `KM-GAP-*` | kmsg 补缺测试 | kmsg 还需要补充的测试，例如重启恢复、配置生效、权限、RPC 行为、并发。 |
| `KK-*` | kevent 与 kmsg 联动测试 | 验证 kevent 负责通知、kmsg 保存可靠数据这套组合模式。 |
| `UF-*` | 使用功能测试 | 从真实调用方中提炼出的共性功能测试，例如“收到通知后回读真相源”。 |
| `DV-*` | 真实环境测试 | 通过 devtest 或真实 gateway/service 入口执行的端到端测试。 |
| `KP-*` | kevent 性能测试 | 记录 kevent 的吞吐、延迟和压力表现。 |
| `MP-*` | kmsg 性能测试 | 记录 kmsg 的写入、读取、并发和可靠写入成本。 |
| `R-*` | 审视方向 | 用来指导测试要重点检查什么，不一定是一条独立测试。 |
| `D-*` | 设计/实现偏差 | 已发现或待验证的设计与当前实现不一致之处。 |
| `B-*` | 阻塞项 | 测试推进中遇到的环境或依赖问题。 |

测试结论不使用笼统的“好了”。统一按下面口径判断：

- `方案可推进`：测试项已经对应到设计要求、公开 API 或真实使用方式，测试放在哪里、怎么跑、需要什么环境、怎么留证据、怎么更新报告都已经明确，且没有已知结构性阻塞。
- `测试已完成`：`notepads/kevent_kmsg_test_report.md` 中对应测试项状态为 `已完成`，并且有可复现命令、测试文件、关键输出或运行态探针证据支撑。
- `部分完成`：已有核心正确性证据，但还缺计划要求的真实环境、规模、路径或故障注入验证。
- `未执行` / `阻塞`：没有执行证据，或当前环境问题会导致执行结果只反映环境失败。

`方案可推进` 不等于 `测试已完成`。实际进度以 `notepads/kevent_kmsg_test_report.md` 中的当前状态为准。

### A. kevent 模块功能测试

| 编号 | 测试项 | 测试目的 | 重要性 | 测试方法 | 命令 | 验证和证据 |
| --- | --- | --- | --- | --- | --- | --- |
| KE-01 | eventid / pattern 校验 | 防止非法 topic、非法 wildcard 和 local/global 混用进入生产链路。 | 高 | 覆盖合法 global/local eventid、非法空路径、非法字符、local 含 `/`、`*` 非完整 segment、超长名称。 | `cargo test -p kevent --test client_contract` | 断言返回 `INVALID_EVENTID` / `INVALID_PATTERN`，日志保存到 `cargo-kevent.log`。 |
| KE-02 | pattern 匹配和 normalize | pattern 是事件路由核心，错误会导致漏收或误收。 | 高 | 测 `/a/*/c`、`/a/**`、`/**/done`、重复 pattern、宽 pattern 覆盖窄 pattern。 | 同上 | 断言匹配集合和 normalized pattern 精确一致。 |
| KE-03 | local pub/sub | 验证进程内最短路径能力。 | 高 | `KEventClient::new_local` 创建 reader，发布 local event，验证 timeout、非匹配事件不投递、匹配事件投递。 | 同上 | 收到事件字段正确；timeout 返回 `None` 而不是卡死。 |
| KE-04 | reader add/remove pattern | 验证订阅动态更新不会丢已排队事件。 | 中高 | 发布后不 pull，先 add pattern，再 pull；remove 后验证旧 pattern 不再投递；禁止移除最后一个 pattern。 | 同上 | 断言已排队事件仍在；最后 pattern remove 返回 `INVALID_PATTERN`。 |
| KE-05 | Timer | Timer 是 SDK 本地能力，常用于心跳/周期任务。 | 中 | 创建一次性 timer 和重复 timer；验证 `_timer.timer_id`、`tick_count`、取消后不再触发。 | 同上 | 事件到达时间在合理窗口内；取消后 2 个 interval 内无新事件。 |
| KE-06 | mode 边界 | 防止 Light/LocalPubOnly 被误用。 | 中高 | Light 只能发布 global event；LocalPubOnly 不能 create reader/timer；Full 无 daemon/shared ring 时 global 能力应给出明确错误。 | 同上 | 断言 `NOT_SUPPORTED` / `DAEMON_UNAVAILABLE`，不 panic。 |
| KE-07 | daemon service register/publish/pull | 验证 daemon 内核心分发语义。 | 高 | `KEventService` 注册 global reader，发布 global event，pull；测试不存在 reader、空 reader_id、local pattern 被拒绝。 | `cargo test -p kevent --test service_contract` | `pull_event` 正确返回事件或 `None`；非法输入返回明确错误。 |
| KE-08 | reader queue overflow | kevent 是尽力通知通道，满队列时应丢旧保新。 | 高 | 用小 capacity service/client，发布超过 capacity 的事件，验证只保留最新 N 条。 | 同上 | 断言旧事件被丢，新事件顺序正确。 |
| KE-09 | peer broadcast 与防环路 | 跨节点广播不能无限转发。 | 高 | 两个 `KEventService` + `InProcessPeerPublisher`；验证 A 到 B；再验证 peer 收到的事件不会二次 broadcast。 | 同上 | B 收到一次；可用计数 publisher 证明无重复广播。 |
| KE-10 | HTTP native endpoint | `/kapi/kevent` 是 native JSON facade，必须与核心协议一致。 | 高 | 调 `RegisterReader`、`PublishGlobal`、`PullEvent` JSON；非法 JSON 返回 400。 | `cargo test -p kevent --test http_contract` | 响应结构为 `KEventDaemonResponse`；错误码和 HTTP status 符合映射。 |
| KE-11 | HTTP publish endpoint | 浏览器/HTTP client 可能通过 publish 投递 global event。 | 中 | POST `/kapi/kevent/publish`，验证服务端补齐 `source_node`、`source_pid`、`timestamp`、`ingress_node`。 | 同上 | reader 收到事件；非法 local eventid 返回 400。 |
| KE-12 | HTTP stream endpoint | 浏览器推荐消费路径，生产风险高。 | 高 | POST `/kapi/kevent/stream`，读取 NDJSON ack、event、keepalive；断开后 reader 被清理。 | 同上 | frame 顺序正确；content-type 是 `application/x-ndjson`；无泄漏。 |
| KE-13 | native TCP framed 协议 | node-daemon native port 是低层生产入口。 | 高 | 使用 duplex stream 或本地 listener 测 framed register/pull；非法 frame length 被拒绝。 | 可放在 `node_daemon` 现有测试，或作为 DV 补充 | 断言 response frame 可解码；非法 frame 不导致服务 panic。 |
| KE-14 | shared ringbuffer 多 client | 本机跨 client 快路径关系到低延迟。 | 高 | 使用唯一临时 `BUCKYOS_KEVENT_RINGBUFFER_PATH`；publisher/subscriber 两个 Full client；订阅后新 producer 第一条事件必须可见。因该环境变量是进程级状态，`shared_ring_contract.rs` 必须用全局锁串行执行，或运行命令加 `--test-threads=1`。 | `cargo test -p kevent --test shared_ring_contract -- --test-threads=1` | 事件到达；late producer 第一条不丢。 |
| KE-15 | shared ringbuffer overrun | 慢 reader 时应按尽力通知语义丢旧保新，不读坏数据。 | 高 | 发布超过 ring capacity 的事件；consumer drain；验证无反序、无坏 JSON、cursor 前进。测试同样必须隔离临时 ringbuffer path 并串行执行。 | 同上 | 不 panic；返回事件可解码且 index 单调。 |

### B. kmsg 已有覆盖与补缺测试

| 编号 | 测试项 | 测试目的 | 重要性 | 测试方法 | 命令 | 验证和证据 |
| --- | --- | --- | --- | --- | --- | --- |
| KM-EXIST-01 | 现有 queue 主流程 | 复用已有内部测试，避免重写 happy path。 | 高 | 保留并运行 `test_msg_queue_end_to_end`。 | `cd src && cargo test -p kmsg` | 主流程通过；日志保存到 `cargo-kmsg.log`。 |
| KM-EXIST-02 | 现有多订阅者流程 | 复用已有多 subscriber / cursor 测试。 | 高 | 保留并运行 `test_multiple_subscribers_and_messages`。 | 同上 | 各订阅者 cursor 独立，裁剪后读取正确。 |
| KM-EXIST-03 | 现有路径 QueueUrn 流程 | 复用已有路径型 queue name 测试。 | 高 | 保留并运行 `test_path_queue_name_roundtrip`。 | 同上 | 绝对路径 QueueUrn 可 create/post/subscribe/fetch。 |
| KM-GAP-01 | persistence reopen | 补足现有测试未覆盖的重启恢复场景。 | 高 | 在 `src/kernel/kmsg/tests/sled_contract.rs` 用唯一一次 `#[path]` 引入方式和临时目录调用 `SledMsgQueue::new_in_dir`，创建 queue/post/sub/fetch/ack 后关闭并重新打开。 | `cd src && cargo test -p kmsg --test sled_contract` | 重新打开后 stats、read/fetch、subscription cursor 保持一致。 |
| KM-GAP-02 | config 生效 | 验证 `max_messages`、`retention_seconds` 是否实现。 | 高 | 在 `sled_contract.rs` 创建带限制的 queue，写入超过限制并等待过期窗口，验证 stats/read/fetch。 | 同上 | 若未生效，报告标注为“实现与设计不符”。 |
| KM-GAP-03 | 权限配置 | 验证 `other_app_can_*`、`other_user_can_*` 和 `RPCContext` 是否生效。 | 高 | 分两层：`sled_contract.rs` 构造不同 `RPCContext` 验证 handler 是否使用权限字段；该测试目标是暴露是否忽略 `RPCContext`，不能为了适配当前实现而把权限忽略当作通过。多身份真实环境测试仅在 devtest 能稳定构造多 session 时执行。 | `cd src && cargo test -p kmsg --test sled_contract`；真实环境条件具备时再跑 `uv run test/run.py -p kevent_kmsg/dv` | 模块级测试若确认全部忽略权限字段，标注偏差；多身份真实环境测试不可用时记录为未验证风险，不作为默认自动门槛。 |
| KM-GAP-04 | RPC 接口行为 | 验证公开 RPC/handler 行为，不引入 sled 私有实现。 | 高 | `rpc_contract.rs` 使用测试内最小 fake `MsgQueueHandler`，挂到 `MsgQueueServerHandler` 上，专门验证方法名、参数解析、未知方法、缺字段、错类型和错误映射；不启动完整 HTTP 服务，也不 include `sled_msg_queue.rs`。真实 `/kapi/kmsg` HTTP POST/GET 行为放到真实环境测试。 | `cd src && cargo test -p kmsg --test rpc_contract` | 正常方法返回可解析结果；异常返回明确错误，不 panic。 |
| KM-GAP-05 | 并发 post | 补充并发生产者下 index 唯一性。 | 高 | `sled_contract.rs` 并发 post 1000 条；真实环境测试可补公开 RPC 并发小样本。 | `cd src && cargo test -p kmsg --test sled_contract` | index 唯一、连续，stats count 正确。 |
| KM-GAP-06 | sync_write cursor 可靠性 | 验证 `sync_write` 是否覆盖 cursor 更新。 | 中高 | `sled_contract.rs` 中 sync_write=true 队列 fetch/ack/seek 后 drop/reopen。 | `cd src && cargo test -p kmsg --test sled_contract` | cursor 不回退；若回退，标注生产风险。 |

### C. kevent + kmsg 联动测试

| 编号 | 测试项 | 测试目的 | 重要性 | 测试方法 | 命令 | 验证和证据 |
| --- | --- | --- | --- | --- | --- | --- |
| KK-01 | event 驱动拉取 kmsg | 验证推荐生产模式。 | 高 | 创建 queue 和 subscription；post kmsg 后发布 kevent `{ queue_urn, index }`；consumer 收到 event 后 fetch kmsg。 | `uv run test/run.py -p kevent_kmsg/dv`；若需要 Rust 独立测试，则命名为 `kevent_kmsg_contract` | 收到的 kmsg payload 与 event index 对应。 |
| KK-02 | kevent 丢失时 kmsg 轮询兜底 | 验证尽力通知通道丢事件时，可靠数据层仍能保证业务可恢复。 | 高 | 只 post kmsg，不发布 kevent；consumer `pull_event(timeout)` 返回 None 后按 cursor fetch kmsg。 | 同上 | 即使无 event，消息仍能被 fetch 到。 |
| KK-03 | 重复 kevent 不导致重复处理已 ack 消息 | kevent 可重复/无序场景下业务消费应靠 kmsg cursor 收敛。 | 中高 | 对同一 kmsg index 发布两次 event；第一次 fetch+ack，第二次 event 后 fetch 为空。 | 同上 | kmsg cursor 保证不重复处理。 |

### D. 使用场景归纳后的功能测试

该层不按每个业务使用场景无限增加测试项，而是从现有调用方中提炼共性能力，再针对这些能力做稳定的功能测试。当前使用方包括 `TaskManagerClient::wait_for_task_end_kevent`、`msg_center` box changed event、`control_panel` chat stream、`OpenDAN msg_center_pump`、`OpenDAN session_event_pump`、`agent_tool subscribe_event` 和 AgentRuntime / workflow task 事件处理。它们共同使用的不是某个业务流程，而是以下 kevent/kmsg 能力：

- kevent 作为尽力通知信号，业务必须回读真相源。
- event path / pattern 是事件生产方和消费方的共同约定。
- reader 支持动态订阅、重建、取消和 fanout。
- kevent timeout、重复、丢失和 reader 关闭都不应破坏业务收敛。
- kmsg 或业务 API 承担可靠数据读取，kevent payload 只作为轻量提示。

| 编号 | 功能契约 | 测试目的 | 重要性 | 测试方法 | 命令 | 验证和证据 |
| --- | --- | --- | --- | --- | --- | --- |
| UF-01 | kevent 唤醒后回读真相源 | 覆盖 TaskMgr、AgentRuntime、control_panel、OpenDAN 这类“event 只唤醒，状态靠回读”的共同模式。 | 高 | 用 mock 构造 event payload 与真相源不一致、event timeout、重复 event、订阅失败四种场景；业务处理必须调用 `get_task`、`get_record`、kmsg fetch 或对应真相源。 | 默认落点：对应模块的最小验证测试；若 mock 成本高，先做静态检查并记录未验证风险。 | 状态变化只来自真相源读取；无 event 时仍能通过 timeout/poll 收敛。 |
| UF-02 | event path 与 pattern 兼容 | 覆盖 msg_center -> control_panel/OpenDAN、task_mgr -> wait/AgentRuntime 等 path 约定，防止 path 改动导致静默失效。 | 高 | 静态检查 + 小型匹配测试：用事件生产方实际 event id 样本验证消费方 pattern 能匹配；包含 `/msg_center/{owner}/box_{box}_{owner}/changed`、`/task_mgr/{id}` 和 `/task_mgr/{root_id}`。 | `cargo test -p kevent --test usage_contract`；`cargo test -p task_manager event_id -- --nocapture` | 所有事件样本均能被目标消费方 pattern 匹配；TaskMgr root event id 只接受合法 kevent path segment。 |
| UF-03 | event payload 最小可用字段 | 覆盖“event 轻量提示，完整数据回读”的共同约束，防止业务把 payload 当可靠数据源。 | 中高 | 对 msg_center box changed、task_mgr task-changed event、kmsg notification 样本检查 payload 只包含定位和摘要字段；消费侧用 payload 定位后必须回读 record/task/message。TaskMgr 大 data 事件需避免超过 shared ring slot，具体省略策略作为源码风险跟踪。 | `cargo test -p kevent --test usage_contract`；TaskMgr 通过源码审视与 `kevent_kmsg/task_mgr` DV smoke 覆盖当前路径 | payload 包含定位字段，例如 `record_id`、`msg_id`、`queue_urn/index` 或 `task_id`；消费侧不直接信任业务状态字段；过大 task data 不内联。 |
| UF-04 | reader 生命周期和动态订阅 | 覆盖 session_event_pump、agent_tool subscribe_event、chat stream 这类动态 reader 使用方式。 | 中高 | mock 测试多 session 重叠 pattern、取消订阅、pattern 去重排序、reader close 后重建、匹配事件 fanout。 | `cargo test -p kevent --test usage_contract`；目标证据还应包含 `cargo test -p opendan route_event_targets_matching_sessions -- --nocapture`，若 OpenDAN crate 编译被其它 workspace 依赖阻塞则记录为该项未完整验证。 | 匹配事件只投递到目标订阅；取消后不再投递；reader 关闭后可重建；重复 pattern 不导致重复投递。 |
| UF-05 | kevent 失败后的兜底路径 | 覆盖 kevent daemon 不可用、reader 创建失败、pull timeout、stream 断开、重复/丢失 event 的共同退化行为。 | 高 | 用 mock 或真实环境测试注入失败：create reader 失败、pull 返回 timeout/ReaderClosed、重复 event、无 event；消费侧必须 fallback 到 poll/sweep/fetch。TaskMgr / AgentRuntime task inbox 必须在任务变化 event 丢失时通过 `list_tasks` 轮询兜底。 | 模块最小验证 + 真实环境最小验证；完整服务重启作为手工验证；目标证据还应包含 OpenDAN task inbox 定向测试。 | 失败路径不 panic、不卡死；最终通过真相源读取恢复；错误可解释并有日志证据。 |

### E. 路由和真实环境测试

| 编号 | 测试项 | 测试目的 | 重要性 | 测试方法 | 命令 | 验证和证据 |
| --- | --- | --- | --- | --- | --- | --- |
| DV-01 | boot gateway kevent route | 确认 `/kapi/kevent/*` 走预期早转发。 | 高 | 复用或补充 `req_kevent_direct_ok.json`。 | `uv run src/test/test_boot_gatweay/run_debug_tests.py` | debug 输出 PASS，保存 `boot-gateway.log`。 |
| DV-02 | boot gateway kmsg route | 确认 `/kapi/kmsg/*` 可路由到 service。 | 高 | 复用或补充 `req_service_kmsg_via_routes_ok.json`。 | 同上 | debug 输出 PASS。 |
| DV-03 | kmsg gateway 最小闭环 | 验证真实 BuckyOS 环境中 `/kapi/kmsg` 可完成最小队列闭环。 | 高 | TS 测试通过 gateway 调 create/post/subscribe/fetch/ack/delete。 | `uv run test/run.py -p kevent_kmsg/dv` | 所有 RPC 返回成功；测试数据清理；保存 `dv.log`。 |
| DV-03B | kmsg HTTP 当前行为记录 | 记录真实 `/kapi/kmsg` 当前只接受 kRPC over HTTP POST，GET 拉模型与文档不一致。 | 中 | TS 测试对 `/kapi/kmsg` 执行一个 POST 最小验证，再执行 GET 探测并记录 status。 | 同上 | POST 可用；GET 当前若不可用，不作为失败项，报告标为文档/实现偏差。 |
| DV-04 | kevent gateway stream 最小验证 | 验证浏览器推荐消费路径在真实环境可用。 | 高 | TS 测试建 `/kapi/kevent/stream`，另一路 publish，读取 ack/event/keepalive。 | 同上 | NDJSON frame 正确；断开后进程无异常日志。 |
| DV-05 | kevent + kmsg gateway 协作 | 验证生产链路：kmsg 持久化 + kevent 通知。 | 高 | TS 测试 create queue、subscribe、post message、publish event、stream 收 event 后 fetch。 | 同上 | event 加速路径可用；轮询兜底路径也可用。 |
| DV-06 | kmsg 持久化自动验证 | 验证真实服务入口下消息在测试进程重连后仍可读。 | 高 | TS 测试创建 queue/post/read，关闭 client 并重新创建 client，再 read/fetch。该项不重启服务，只验证公开入口和持久数据可重复读取。 | `uv run test/run.py -p kevent_kmsg/dv` | 重新连接后消息仍可读；记录 queue_urn/index。 |
| DV-07 | TaskMgr task-changed 真实链路 smoke | 验证 TaskMgr 作为 kevent 生产方的真实 gateway 链路（beta2.2 起 runner task_ready inbox 已删除）。 | 中高 | 通过 gateway 调 TaskMgr 创建 Pending task，建立 `/kapi/kevent/stream` 订阅 `/task_mgr/{task_id}`，驱动 `update_task_status` 后等待 task-changed event；收到 event 后必须通过 TaskMgr `get_task` 回读真相源；另验证 `list_tasks` 轮询兜底可发现 Pending task。 | `uv run test/run.py -p kevent_kmsg/task_mgr` | 收到 task-changed event（payload 无 runner 字段）；回读到同一 task；无 event 时轮询仍可发现 Pending task。 |
| DV-MANUAL-01 | 服务重启后 kmsg 持久化 | 验证完整 BuckyOS 服务重启后不丢数据。 | 高 | `kevent_kmsg/restart` 创建 queue/post/read/sub/ack，执行 `uv run src/start.py --skip-update` 重启服务，`uv run src/check.py` 恢复后通过 gateway 重新 read 重启前消息。 | `uv run test/run.py -p kevent_kmsg/restart` | 重启后消息仍可读；subscription 若丢失，记录为 D-09 相关偏差，不作为该项消息持久化失败。 |
| DV-MANUAL-02 | kevent daemon restart 退化行为 | 验证尽力通知通道的故障语义：不会卡死，功能靠 kmsg 恢复。 | 中高 | `kevent_kmsg/restart` 在重启前建立 stream，重启期间观察旧 stream 有界关闭或超时；恢复后重建 stream，post kmsg 并 publish kevent。 | `uv run test/run.py -p kevent_kmsg/restart` | 旧 stream 行为可解释；恢复后 kmsg fetch 和 kevent stream 均可用。 |

## 7. 性能测试设计

性能测试不追求模拟全量生产流量，只覆盖会导致生产不可用的明显退化。首轮性能测试的通过标准以正确性为主，时间阈值只作为宽松保护，防止测试永久卡住，不作为产品 SLO。

### kevent performance

| 编号 | 测试项 | 测试目的 | 重要性 | 测试方法 | 初始通过标准 | 证据 |
| --- | --- | --- | --- | --- | --- | --- |
| KP-01 | local pub/sub throughput | 记录进程内事件通道性能参考数据。 | 高 | 单 client、单 reader、发布 10k local events 并 pull 完。 | 全部收到；无 panic/死锁；超宽松 timeout 内完成。 | throughput、p50/p95/p99 latency 写入 `performance.json`。 |
| KP-02 | service publish/pull throughput | 记录 daemon service 分发路径性能参考数据。 | 高 | `KEventService` 单 reader，发布 10k global events。 | 全部收到；无 panic/死锁；超宽松 timeout 内完成。 | 同上。 |
| KP-03 | shared ring latency | 记录本机 Full client 快路径性能参考数据。 | 高 | 两个 Full client，发布 2k global events。 | 无坏数据；无 panic/死锁；记录 p50/p95/p99，不以固定 p95 作为硬失败。 | 同上。 |
| KP-04 | slow reader overflow | 验证慢消费者场景符合尽力通知的丢旧保新语义。 | 高 | capacity 小于发布量，发布 10k，慢 reader 最终 drain。 | 不 panic；保留最新事件，旧事件允许丢。 | 记录收到数量和最新 seq。 |
| KP-05 | HTTP stream sustained | 验证浏览器推荐消费路径能保持长连接并持续收帧。 | 中高 | stream 保持 30s，周期 publish。 | stream 不断开，keepalive/event 正常。 | DV/perf 日志。 |

### kmsg performance

| 编号 | 测试项 | 测试目的 | 重要性 | 测试方法 | 初始通过标准 | 证据 |
| --- | --- | --- | --- | --- | --- | --- |
| MP-01 | post throughput | 记录持久化写入路径性能参考数据。 | 高 | `src/kernel/kmsg/tests/sled_contract.rs` 的 ignored case 单 queue post 10k 条 1KB payload；真实环境测试可补公开 RPC 小样本参考数据。 | index 连续；无 panic/死锁；超宽松 timeout 内完成。 | ops/s、p95 写入 `performance.json`。 |
| MP-02 | fetch throughput | 验证批量消费路径具备生产可用的基础吞吐。 | 高 | 预置 10k 条，fetch batch=100，auto_commit=true。 | 全部读完；无重复、无缺失。 | batch latency。 |
| MP-03 | at-least-once overhead | 验证可靠消费路径在显式 ack 下不破坏 cursor。 | 高 | fetch auto_commit=false + commit_ack。 | 全部读完；无 cursor 错乱。 | ops/s。 |
| MP-04 | concurrent producers | 验证多生产者并发写入时 index 唯一且连续。 | 高 | 10 个 task 并发 post，总 10k 条。 | index 唯一连续，stats 正确。 | index 校验摘要。 |
| MP-05 | sync_write 性能成本 | 记录可靠写入开关的性能成本，作为退化排查基线。 | 中 | sync_write=false 和 true 各 post 1k 条。 | 两者都成功；记录差异，不以差异作为失败条件。 | sync_write 对比数据。 |

## 8. 环境构建

### 本地模块测试环境

不需要完整 BuckyOS 运行环境。

```bash
cd src
cargo test -p kevent
cargo test -p kmsg
```

如果依赖未下载或网络失败，需要先解决 cargo 依赖环境。测试实现不引入生产依赖；只使用 workspace 已有依赖。

### 真实环境测试环境

在仓库根目录执行：

```bash
uv run src/start.py --all
uv run src/check.py
```

通过标准后再运行：

```bash
uv run test/run.py -p kevent_kmsg/dv
```

真实环境测试中的 URL 和凭证通过环境变量配置，默认使用 devtest 本地环境：

- `BUCKYOS_GATEWAY_BASE_URL`
- `BUCKYOS_TEST_APP_ID`
- `BUCKYOS_TEST_USER_ID`

如果需要登录态，优先复用现有 `test/aicc_test` 和 `test/test_helpers` 的登录方式。测试包只增加测试依赖，不引入模块生产依赖。

## 9. 测试复现指南

执行者按本节准备环境和运行命令，再用 `notepads/kevent_kmsg_test_report.md` 核对当前状态。每轮详细证据可保存到本地 `test/kevent_kmsg/reports/`，但不提交到仓库。

| 分类 | 命令 | 环境 | 覆盖 | 通过判断 |
| --- | --- | --- | --- | --- |
| 模块级功能 | `cd src && cargo test -p kevent && cargo test -p kmsg` | Rust workspace 可解析；Linux 如需 native 依赖设置 `LIBCLANG_PATH`。 | `KE-01`~`KE-15`、`KM-EXIST-*`、`KM-GAP-*`、`D-01/D-04/D-05/D-06/D-07/D-09` | 退出码 0；config/权限等当前偏差必须记录为偏差，不伪装成设计通过。 |
| 使用功能契约 | `cd src && cargo test -p kevent --test usage_contract && cargo test -p task_manager event_id -- --nocapture` | 不需要完整 BuckyOS；OpenDAN 定向证据另跑 `cargo test -p opendan route_event_targets_matching_sessions -- --nocapture`，若 workspace 其它依赖编译失败则记录为阻塞。 | `UF-01`~`UF-05`、`D-11`、TaskMgr task event producer contract | event id 与 consumer pattern 匹配；payload 只用于定位；TaskMgr runner/root path 已由现有单测和 DV smoke 覆盖；timeout/重复 event 不破坏真相源收敛。 |
| 真实环境 DV | `uv run src/start.py --all && uv run src/check.py && uv run test/run.py -p kevent_kmsg/dv && uv run test/run.py -p kevent_kmsg/task_mgr` | 仓库根目录；标准 devtest；Deno、pnpm、bash 可用；TaskMgr smoke 还要求 TaskMgr 成功启动并上报，gateway route/service_info 包含 `task-manager`；旧环境若存在旧 `task-mgr-main.db` schema，需要先迁移或备份重建，本轮按测试环境 workaround 处理，代码修复需单独任务确认。 | `KK-01`~`KK-03`、`DV-03`~`DV-07`、`DV-03B`、`D-08` | kmsg POST/kRPC 闭环通过；kevent stream 可 ack/receive；GET `/kapi/kmsg` 当前行为只记录；TaskMgr task-changed event 必须通过 gateway 验证。 |
| 服务重启恢复 | `uv run test/run.py -p kevent_kmsg/restart` | 可重建 devtest；测试会执行 `uv run src/start.py --skip-update`。 | `DV-MANUAL-01`、`DV-MANUAL-02`、`D-09` | 重启前消息重启后可读；旧 stream 有界关闭或超时；恢复后新 stream/kmsg 可用；subscription 丢失记录为 D-09 偏差。 |
| peer container | `uv run test/run.py -p kevent_kmsg/peer_container` | Linux 测试机；Docker 可运行 `ubuntu:24.04`。 | `D-02`、`D-03` | 外部 client 发布到 `node_a` 后，`node_b` 能收到同一 event。 |
| peer VM | `uv run test/run.py -p kevent_kmsg/peer_vm` | Linux 测试机；KVM、`qemu-system-x86_64`、`qemu-img`、`genisoimage` 可用。 | `D-02`、`D-03` | 与 container 同；只证明手工配置 peer 后 native framed 单向投递可达。 |
| 性能基线 | `cd src && cargo test -p kevent --test performance -- --ignored --nocapture --test-threads=1 && cargo test -p kmsg --test sled_contract -- --ignored --nocapture` | Rust workspace；性能测试默认 ignored。 | `KP-01`~`KP-05`、`MP-01`~`MP-05` | 退出码 0；输出 baseline JSON；数值作为基线，不作为产品 SLO。 |
| 路由 debug | `uv run src/test/test_boot_gatweay/run_debug_tests.py` | `cyfs_gateway debug` 能加载当前 `boot_gateway.yaml`。 | `DV-01`、`DV-02` | route case 输出 PASS；若 debug binary 不支持当前 `--backup-map` 语法，记录为工具链阻塞，不代表 runtime gateway DV 失败。 |

单独复现 kevent slow reader baseline：

```bash
cd src
cargo test -p kevent --test performance slow_reader_overflow_10k_baseline -- --ignored --nocapture
```

每轮执行后：

- `test/kevent_kmsg/reports/` 可在本地保存该轮实际命令、环境、退出码、关键输出、日志路径和当轮结论；该目录不提交。
- `notepads/kevent_kmsg_test_report.md` 更新当前状态表、复现索引、关键证据、阻塞项和剩余风险。
- 实现与设计不符时，在当前报告的偏差和修补建议中记录依据、实际行为和建议处理方式。

## 10. 推进步骤

1. 确认本测试方案。
2. 实现 `src/kernel/kevent/tests/*` integration tests。
3. 实现 `src/kernel/kmsg/tests/sled_contract.rs` 和 `src/kernel/kmsg/tests/rpc_contract.rs`。其中 `sled_contract.rs` 是唯一使用 `#[path = "../src/sled_msg_queue.rs"]` 引入实现文件的测试，验证持久化、config、权限、cursor、并发、性能等对外行为；`rpc_contract.rs` 使用 fake `MsgQueueHandler` 验证 RPC/handler 接口行为。
4. 实现 `test/kevent_kmsg/dv`、`restart`、`peer_container`、`peer_vm` 测试。
5. 运行模块级测试：

```bash
cd src
cargo test -p kevent
cargo test -p kmsg
```

6. 修正测试或实现中暴露的问题。若需改协议、字段、存储结构，同时检查前后端和文档联动。
7. 运行路由测试：

```bash
uv run src/test/test_boot_gatweay/run_debug_tests.py
```

8. 启动 DV 环境并运行：

```bash
uv run src/start.py --all
uv run src/check.py
uv run test/run.py -p kevent_kmsg/dv
```

9. 在合适环境运行 restart、peer container、peer VM 测试：

```bash
uv run test/run.py -p kevent_kmsg/restart
uv run test/run.py -p kevent_kmsg/peer_container
uv run test/run.py -p kevent_kmsg/peer_vm
```

10. 运行 ignored 性能测试和真实环境性能参考测试：

```bash
cd src
cargo test -p kevent --test performance -- --ignored --nocapture
cargo test -p kmsg --test sled_contract -- --ignored --nocapture
cd ..
uv run test/run.py -p kevent_kmsg/dv
```

11. 可在本地汇总 `test/kevent_kmsg/reports/<timestamp>/report.md`，并更新仓库内的 `notepads/kevent_kmsg_test_report.md`。

## 11. 报告和进度规则

报告分两层维护：

| 文件 | 职责 |
| --- | --- |
| `notepads/kevent_kmsg_test_report.md` | 当前测试报告和进度索引，记录每个计划项的状态、结论、证据索引和剩余风险。 |
| `test/kevent_kmsg/reports/<timestamp>/` 或 `test/kevent_kmsg/reports/<date>-<name>.md` | 执行者本地每一轮测试的详细记录，包含命令、环境、原始输出、日志路径、性能数据和当轮结论；该目录 git 忽略，不提交。 |

每执行一轮测试后：

1. 可将该轮命令、环境、退出码、关键输出、日志路径和当轮结论归档到本地 `test/kevent_kmsg/reports/`。
2. 更新 `notepads/kevent_kmsg_test_report.md` 中的当前结论、统计口径、测试项状态、阻塞项和证据索引。
3. 如增加测试项、拆分测试项或调整测试目录，同时更新本测试计划和当前报告。
4. 不在 notepad 当前报告里长期保留多轮历史数据；历史明细由执行者本地 `test/kevent_kmsg/reports/` 保留。

当前报告中的每个测试项必须使用以下状态之一：

| 状态 | 含义 |
| --- | --- |
| `已完成` | 已有自动化测试或明确运行态探针证据。 |
| `部分完成` | 已覆盖核心正确性，但未完全按计划中的环境、规模或路径执行。 |
| `未执行` | 计划项尚无执行证据。 |
| `阻塞` | 受环境或依赖问题阻塞，当前执行只会得到环境失败。 |

每个测试项至少维护：`当前状态`、`测试结果`、`通过证据 / 当前证据`、`剩余工作`。

报告完整性检查：

- 当前报告覆盖 test plan 中所有测试项。
- `已完成` 项有可复现命令或明确探针证据。
- `阻塞` 项写明阻塞原因、影响范围和下一步。
- 当前报告中的状态变化要有可复现命令和关键证据；本地 `test/kevent_kmsg/reports/` 可作为执行者自留的详细证据。
## 12. 风险和取舍

- 不把所有行为都放到真实环境测试里。大多数语义用模块级测试快速覆盖，真实环境测试只验证关键链路，避免测试慢且不稳定。
- kevent 补充测试优先放在 integration test 或根目录 test 模块，避免侵入模块源码。
- kmsg 当前是 binary crate，仅 `src/kernel/kmsg/tests/sled_contract.rs` 通过 `#[path]` 引入实现文件，不改生产源码。该方式只能验证设计要求和对外行为，不能写面向私有分支覆盖率的白盒测试。公开链路测试仍放在真实环境/RPC/HTTP 层。
- kevent 是尽力通知通道，测试不会要求事件永不丢；只要求错误可解释、不会卡死、丢事件时 kmsg 兜底有效。
- kmsg 是可靠层，测试会严格要求持久化、index 单调、cursor 正确、重启后数据仍可读。
- 性能阈值先作为防明显退化的 guardrail，首轮报告记录基线；没有产品 SLO 前不做过度性能门槛。

## 13. 测试视角：以设计和生产语义为准

本测试方案不能面向源码实现细节来设计，也不能把“当前代码怎么写的”直接当成正确行为。测试的判断基准按优先级排列如下：

1. `doc/arch/kevent.md` 和 `doc/arch/kmsg.md` 中定义的定位、语义、协议和边界。
2. BuckyOS 生产环境中 kevent / kmsg 应承担的角色：kevent 是尽力通知通道，kmsg 是可靠数据通道。
3. 模块公开 API、服务入口、调用方使用方式所体现的契约。
4. 当前源码实现。

实施测试时，需要把测试分成两类：

- **功能约定测试**：验证当前实现是否满足设计文档和公开接口语义。例如 kmsg 的 index 单调、cursor 正确、重启后数据仍可读；kevent 的 local/global 边界、pattern 匹配、尽力通知下的丢旧保新、HTTP stream frame 语义。
- **实现审视测试**：通过测试暴露当前实现中不合理、过度耦合、与设计不符或生产风险较高的地方。例如文档要求的能力未实现、错误码与文档不一致、HTTP facade 与 native 协议不一致、kmsg 配置字段没有生效、kevent 事件大小限制与 shared ring slot 限制冲突、异常路径返回 500 或 panic。

测试代码可以调用公开类型和本 crate 暴露的测试可用 API，但不能为了迎合当前私有实现而写“白盒脚本”。如果测试失败且失败原因是设计契约和当前实现冲突，应在测试报告里明确标注为：

- `实现与设计不符`
- `设计未明确但生产风险存在`
- `当前实现合理，文档需要补充`
- `测试假设错误，需要修正`

这类结论需要保留证据：失败命令、日志、输入参数、实际返回、预期依据，以及引用的文档章节或公开 API。

当前已识别的重点审视方向：

| 编号 | 主题 | 审视目标 | 重要性 | 验证方式 |
| --- | --- | --- | --- | --- |
| R-01 | kevent 文档语义与实现能力对齐 | 检查 local/global event、pattern、Timer、Light/Full mode、HTTP stream 是否符合文档边界。 | 高 | 用契约测试验证设计语义，不以内部函数分支为覆盖目标。 |
| R-02 | kevent 错误码与 HTTP status | 检查非法 eventid/pattern、reader 关闭、daemon 不可用、非法 JSON 是否返回可解释错误，而不是 panic 或 500。 | 高 | HTTP/native 测试断言错误码、status 和响应体。 |
| R-03 | kevent 数据大小限制一致性 | 文档建议 event data 可到 64KB，但 shared ring slot 当前约 2KB，需要确认生产路径是否可能丢失或失败。 | 高 | 构造 1KB、2KB 附近、超过 slot、接近 64KB 的事件，分别验证 local/service/shared ring/HTTP 路径行为。 |
| R-04 | kevent 尽力通知行为是否可解释 | 事件允许丢，但不能表现为卡死、坏数据、重复广播风暴或资源泄漏。 | 高 | overflow、peer 防环路、stream 断开清理、daemon restart 场景验证。 |
| R-05 | kmsg 文档接口与实际 API 对齐 | `doc/arch/kmsg.md` 是基础接口说明，当前 `buckyos-api/src/msg_queue.rs` 增加了权限字段和 `read_message`，需要确认这些行为是否应成为正式对外约定。 | 中高 | 模块测试和报告中列出文档缺口或实现扩展。 |
| R-06 | kmsg 配置字段是否真正生效 | `max_messages`、`retention_seconds`、权限字段等配置若当前未实现，需明确是缺陷、未完成还是文档未定义。 | 高 | 创建带配置的 queue 后写入超过限制、跨 app/user 场景检查；若未实现，在报告中标记设计/实现偏差。 |
| R-07 | kmsg 可靠性边界 | kmsg 是可靠数据通道，持久化、cursor、ack、重启恢复不能只按当前 happy path 测试。 | 高 | reopen、delete/compact 后 fetch、auto_commit=false 重复读取、commit_ack 后推进。 |
| R-08 | kevent + kmsg 协作退化路径 | 不能只验证收到 kevent 的快路径，还要验证 kevent 丢失/重启/断流时 kmsg 轮询兜底仍能保证功能。 | 高 | 组合测试中同时覆盖 event-driven 和 polling fallback。 |
| R-09 | msg_center / control_panel / OpenDAN event path 兼容 | msg_center 是当前 kevent 真实生产调用方之一，事件生产方和消费方的 path/payload 约定必须一致。 | 高 | 静态检查和最小验证覆盖 `/msg_center/{owner}/box_{box}_{owner}/changed`、OpenDAN inbox pattern、control_panel in/out 订阅和回读 record 语义。 |
| R-10 | TaskMgr task event producer / AgentRuntime task inbox 兼容 | beta2.2 后 TaskMgr 只发布 task changed 与 root fanout event（runner task_ready inbox 已删除）；AgentRuntime task inbox 订阅 `/task_mgr/**`。这些路径和 payload 是 kevent 生产/消费契约。 | 高 | TaskMgr 单元测试固定 event id、payload、data inline limit；`kevent_kmsg/task_mgr` DV 通过 gateway 验证 task-changed event 后回读 TaskMgr 真相源，若 gateway 未暴露 `task-manager` 则按配置阻塞记录。 |

### 当前已发现的设计 / 实现偏差

以下是基于当前文档和代码静态阅读已经能确认或需要重点验证的偏差。实施测试时应把这些作为优先验证目标；如果测试证明偏差存在，需要在测试报告中明确标注“实现与设计不符”或“文档需要补充”。

| 编号 | 模块 | 偏差说明 | 依据 | 状态 | 初步判断 | 验证方式 / 剩余验证 |
| --- | --- | --- | --- | --- | --- | --- |
| D-01 | kevent | 文档建议 `data` 字段大小上限可为 64KB，`KEventClient` 也按 64KB 校验；但 shared ringbuffer 单 slot 只有 2048 bytes，且写入的是序列化后的完整 `Event`，因此 2KB 左右以上的 global event 在 shared-ring 路径可能失败。 | `doc/arch/kevent.md` 提到 64KB；`kevent_client.rs` 有 `MAX_EVENT_DATA_SIZE_BYTES = 64 * 1024`；`kevent_ringbuffer.rs` 有 `SLOT_DATA_SIZE = 2048`。 | 已静态确认 | 明确的设计/实现冲突。需要决定是降低文档/API 上限，还是调整 shared ring 传输策略。 | 增加 1KB、2KB、4KB、64KB event 的 local/service/shared ring/HTTP publish 测试，记录各路径行为。 |
| D-02 | kevent | 文档描述 Node Daemon 通过全 mesh TCP 长连接向所有 peer 广播 global event；当前代码有 `KEventPeerPublisher` 抽象、in-process 测试、container 级和 QEMU/KVM VM 级 native framed 单向投递测试，但 `node_daemon/src/kevent_server.rs` 未看到 peer 连接建立、维护和配置加载。 | `doc/arch/kevent.md` 的 peer daemon 协议和 `remote_peers` 设计；当前 node_daemon kevent server 只启动 HTTP、native TCP 和 shared-ring importer。`test/kevent_kmsg/peer_container` 与 `test/kevent_kmsg/peer_vm` 可验证手工配置 peer 后的单向 framed 投递。 | 部分验证，仍疑似未完整实现 | 生产跨节点自动 peer mesh 能力疑似未落地。 | 已有 container 和 QEMU/KVM VM harness 验证单向 framed peer 投递；剩余验证是完整 `ood1 + node1` BuckyOS DV，确认 node-daemon 是否自动建立和维护 peer 连接。 |
| D-03 | kevent | 文档说外部 Light SDK 连接任意 daemon 发布事件后应广播到所有 peer；container 和 QEMU/KVM VM harness 已验证“外部 client -> node_a -> node_b”单向可达，但完整任意 daemon / 全 mesh 语义仍依赖 D-02。 | `doc/arch/kevent.md` 说明 Light SDK 只需连接任意 Daemon；当前只有本地 service/HTTP/native publish 路径。harness 显示 framed `PublishGlobal` 到接收端后 `ingress_node` 会变成接收端。 | 部分验证，仍疑似设计语义未完整落地 | 依赖 D-02；当前 framed 协议还不能清晰表达 peer 转发语义。 | 使用 `uv run test/run.py -p kevent_kmsg/peer_container` 和 `uv run test/run.py -p kevent_kmsg/peer_vm` 作为单向可达证据；剩余验证是 peer publish 协议确认和完整双节点 BuckyOS DV。 |
| D-04 | kevent | 文档的错误码列表只有 `INVALID_EVENTID`、`INVALID_PATTERN`、`DAEMON_UNAVAILABLE`、`TIMER_INVALID_TARGET`、`TIMER_NOT_FOUND`；当前实现额外有 `NOT_SUPPORTED`、`READER_CLOSED`，且 `pull_event` 对不存在 reader 返回 `Ok(None)`，`update_reader` 对不存在 reader 返回 `READER_CLOSED`，生命周期错误语义不一致。 | `doc/arch/kevent.md` 错误码章节；`kevent_client.rs` 的 `KEventError`；`service.rs` 的 `pull_event` / `update_reader`。 | 已静态确认 | 文档与实现不一致，且 API 行为需要统一。 | 模块测试固定关闭/不存在 reader 的 pull/update/remove 行为；报告中建议统一错误语义或补文档。 |
| D-05 | kevent | 文档强调 EventBus 是尽力通知、无匹配 reader 时静默丢弃；当前 service 在 mirror 到 shared ring 失败时可能让 HTTP publish / external publish 返回错误。这对超大事件是好事还是违反尽力通知语义，需要设计确认。 | `doc/arch/kevent.md` 的尽力通知和静默丢弃语义；`service.rs` 的 `mirror_to_shared_ring` 返回错误链路。 | 待测试验证 | 设计语义不够精确，尤其是“非法/过大事件”是否应失败。 | 构造过大事件，分别验证 publish 返回、reader 接收、日志。 |
| D-06 | kmsg | 文档和 API 都定义了 `max_messages`、`retention_seconds`，但当前 sled 实现仅保存 config，未在 post/fetch/read 或后台维护中执行最大条数和过期清理。 | `doc/arch/kmsg.md` 的 `QueueConfig`；`msg_queue.rs` 的字段；`sled_msg_queue.rs` 只读取 `sync_write`，未使用 `max_messages` / `retention_seconds`。 | 已静态确认 | 明确的设计/实现冲突。 | 创建 `max_messages=3` / `retention_seconds=1` 的队列，写入和等待后验证 stats/read 是否裁剪。 |
| D-07 | kmsg | 文档包含 `PermissionDenied` 和权限控制说明；当前 API 增加 `other_app_can_read/write`、`other_user_can_read/write`，但 sled 实现所有 handler 都忽略 `RPCContext`，没有权限校验。 | `doc/arch/kmsg.md` 的权限说明；`msg_queue.rs` 的权限字段；`sled_msg_queue.rs` handler 参数均为 `_ctx`。 | 已静态确认 | 明确的设计/实现冲突或未完成项。 | DV 或 handler test 构造不同 user/app context，验证是否按 config 拒绝读写。 |
| D-08 | kmsg | kevent 文档把 kmsg 描述为“拉模型（HTTP GET）”，但当前 kmsg HTTP 服务只接受 POST，并通过 kRPC handler 提供 `fetch_messages` / `read_message`。 | `doc/arch/kevent.md` 多处写 kMsgQueue 拉模型（HTTP GET）；`sled_msg_queue.rs` 的 `HttpServer` 只接受 `Method::POST`。 | 已静态确认 | 文档和现实现状不一致。该项先作为当前行为记录和文档偏差，不作为默认测试失败项。 | DV 中记录 POST 可用和 GET 当前 status；报告建议确认最终协议是 HTTP GET 还是 kRPC POST。 |
| D-09 | kmsg | `sync_write` 当前只在 create/update/post/delete_message_before 等路径触发 flush；订阅状态变化如 subscribe、unsubscribe、fetch auto_commit、commit_ack、seek 没有按 queue config flush。若 `sync_write` 代表 WAL/可靠队列状态，则 cursor 可靠性不足。 | `doc/arch/kmsg.md` 将 `sync_write` 描述为 Write-Ahead-Log 语义；`sled_msg_queue.rs` 中 cursor 更新路径无 flush。 | 已静态确认 | 需要设计确认；对 at-least-once 重启恢复有生产风险。 | sync_write=true 队列中 fetch/ack 后 reopen，验证 cursor 是否稳定；进一步需要 crash 级验证。 |
| D-10 | kmsg | `doc/arch/kmsg.md` 描述的是基础 trait，没有包含当前公开 API 中的 `read_message`、权限 bool、绝对路径 QueueUrn 透传规则等扩展。 | `doc/arch/kmsg.md` 与 `msg_queue.rs` 差异。 | 文档需补充 | 文档落后于实现，不一定是代码错误，但测试计划和报告要明确当前契约来源。 | 在测试报告里列出“文档未覆盖但当前 API 暴露”的能力，建议补文档。 |
| D-11 | kevent 调用方 | `msg_center` 当前发布 `/msg_center/{owner}/box_{box}_{owner}/changed`，OpenDAN 订阅 inbox 类 box id，control_panel 订阅 inbox/outbox box id。如果未来 msg_center path 或 box id 变化，两个消费方容易静默失效。 | `msg_center.rs` 的 `build_box_changed_event_id`；`message_hub.rs` 的 chat stream patterns；`msg_center_pump.rs` 的 `build_msg_center_event_patterns`。 | 已静态确认风险 | 不是当前实现 bug，但属于调用方约定脆弱点，应固化测试。 | 增加事件生产方和消费方 pattern 兼容性最小验证，验证所有消费方 pattern 能匹配 msg_center 当前发布路径。 |

## 14. 附录：测试方案维护原则

- 只依据当前工作区实际存在的 kevent/kmsg 文档、模块代码和调用方代码设计测试。
- 测试以设计文档、公开 API、生产语义和调用方契约为判断基准，不能把当前私有实现当作正确性来源。
- 测试覆盖要全面但不冗余；调用方测试先归纳共性功能，只有出现新的共同能力时才增加测试项。
- 测试代码不侵入模块源码；kevent 使用 integration tests，kmsg 底层契约只允许 `sled_contract.rs` 一处 `#[path]` harness。
- 每项测试必须写清目的、重要性、方法、命令、环境要求、验证方式和证据。
- 真实性能阈值只用于防明显卡死、panic、死锁或资源耗尽；没有产品 SLO 前只记录 baseline。
- 如果实现与设计不符，不改写测试目标来适配实现；在报告中标注偏差并保留命令、输入、实际返回和日志证据。
- 当前结论统一看 `notepads/kevent_kmsg_test_report.md`；每轮具体数据和当轮结论可在本地归档到 `test/kevent_kmsg/reports/`，不提交到仓库。
- 维护本文件时必须同步更新目录。
