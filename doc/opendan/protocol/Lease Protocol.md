# Lease Protocol

版本 1 · 2026-09-29 · 由 `libopendan` 反写（`src/lock.rs`、`agent_tool::exec_tracking`）

## 1. 原语

协议只依赖一个存储原语：**可长期持有、持有者（进程 / 客户端会话）失效即释放的排他文件锁**。单节点是 `flock(LOCK_EX|LOCK_NB)`；多个容器共享 bind mount 时同样有效；DFS 上线后必须提供同一语义（并在失锁时通知持有者）。没有 TTL、续约和时钟依赖。

## 2. 资源与锁文件

| 资源 | 锁文件 | 持有期间 | 用途 |
|---|---|---|---|
| `session:<sid>` | `<sid>/.opendan_agent_session/lease.json` | 一次 drive 全程 | 推进权；持有者身份必须等于 `session.driver.principal` |
| `run:<run_id>` | `<sid>/.opendan_agent_session/runs/<run_id>/.lock` | 执行该 run 期间 | run 目录的执行互斥；libopendan 与接手的 xllm 共用 |
| `self_improve` | `<agent_root>/.locks/self_improve.lease` | 一个 self_improve session 的 drive 全程 | 全局只有一个认知整理 |
| `artifact:<aid>` | `<agent_root>/.locks/artifact/<aid>.lease` | 短临界区 | 串行化产物 head 与版本有效性变更 |

锁顺序固定：session → run → artifact；self_improve 在 session 之后获取。

## 3. 锁文件内容

```jsonc
{ "resource": "session:work-…", "epoch": 42,
  "holder": { "runner_id": "rn-…", "principal": "app:app2@alice", "host": "host:…", "pid": 1234, "runtime_id": "rt-…" },
  "acquired_at_ms": 0, "released_at_ms": null }
```

- `epoch` = 上一次内容的 epoch + 1（文件为空或不可解析时从 1 开始），写入 `state.json.writer.lock_epoch` 用于审计。
- 内容只由持有者经**同一文件描述符原地改写**（`pwrite(0)` + `ftruncate` + `fsync`）。锁文件**永不替换、永不删除**：flock 属于 inode，rename 之后新打开者会拿到新 inode 上的锁。其它状态文件照常原子替换。
- 其它参与方只为显示读取内容（“谁在推进”）；读到半写内容重读一次，仍失败视为未知。
- 正常释放时写入 `released_at_ms` 后解锁；崩溃时由操作系统释放，内容保留上一个持有者的信息。

## 4. 获取与使用

```text
acquire(resource, holder):
  session 资源：holder.principal != session.driver.principal → NotDriver
  open(lock_file, O_RDWR|O_CREAT|O_CLOEXEC)；进程内已持有同一 inode → Busy
  flock(LOCK_EX|LOCK_NB) 失败 → Busy(读出的持有者信息)
  epoch = prev.epoch + 1；原地改写内容 → Lease
held(lease): 锁文件路径当前的 (dev, inode) 仍等于加锁时 → 否则 LeaseLost（文件被替换 / 删除）
fenced(lease, write): held() 为真才执行受保护的写入与副作用
```

- 锁描述符必须带 `CLOEXEC`，否则 `exec` 启动的后台进程会在 runner 退出后继续持锁。
- 同一进程不重复加锁（flock 以打开文件描述为单位，两次打开会互相阻塞）；实现维护进程内已持有表。
- 卡死但存活的持有者不会释放锁；协议不提供强制抢锁，由运维按锁文件中的 host / pid 处理。

## 5. 锁保护的边界：执行核对

文件锁只保证协作 runner 之间状态写入互斥。runner 被 `kill -9` 时锁立即释放，但它启动的工具进程可能仍在运行。因此**接管方在启动新推理或工具之前**必须确认旧执行已停止：

1. **执行标识先持久化**：受管执行（native / tmux runtime 的 `exec`）在用户命令获准运行之前，把 `ExecutionRecord` 写入 run.json 的 `executions[]`（fsync）。native：启动包装进程（独立进程组），在收到 `go` 行之前不 `exec` 用户命令；登记失败或 runner 在放行前退出，管道关闭，命令不会执行。tmux：在专用 pane 启动等待 go 文件的包装进程，取得 PID/start ticks 后登记，登记成功才放行用户命令。
2. **标识**：`execution_id` 以环境变量 `OPENDAN_EXECUTION_ID` 注入命令（子进程继承），另记 `host`、`boot_id`、`pgid`、进程组长的 `start_ticks`。不得只凭可能复用的 PID / PGID 杀进程。
3. **探测**：boot_id 不同 → 已停止；host 不同 / 没有 `/proc` / 同进程组内有无法验证身份的进程 → **Unknown**；环境变量带该标记的存活进程 → **Alive**；否则 Stopped。
4. **停止**：Alive → 逐个终止已验证的进程并等待消失；超时或 Unknown → `RecoveryBlocked`，保留现场，不自动继续。
5. **记录的清除**：shell 返回后仍有后台子进程时记录保留；只有确认整个执行停止才移除。run 结束（及挂起）之前也必须核对并停止其执行。
6. **在途动作**：`inflight[]` 在工具开始前 fsync；只有包含其结果（或显式 `Unresolved`）的快照发布后才清除（快照 fsync 与 run.json 发布是同一次原子写入里清除）。异常、取消、runner 退出都不提前清除。恢复时，已有持久结果的调用不得再标为未知；没有结果的调用以“结果未知”物化到快照（function call：补 assistant 调用 + tool 结果；behavior：补齐或新增一个 step）并先持久化，再推理。**不重放工具。**

xllm 接手未结束的 run 时执行同样的检查（`XllmRun::resume`：先停旧执行，再物化 inflight），并在 `host_commit_pending` 非空时拒绝。

## 6. 写入范围

| 权限来源 | 可写范围 |
|---|---|
| session 锁（驱动者） | `.opendan_agent_session/` 除执行中的 run 目录外的全部；`.runtime/`；产物；绑定的 workspace；登记表中本 session 条目（status、location）；`state/perception/<sid>.jsonl`；输入源确认 |
| run 锁 | `runs/<run_id>/`（libopendan 执行时同时持有 session 与 run 锁；接手的 xllm 只持 run 锁，只写 run 目录） |
| 无需锁 | 向 session 的 kmsg 队列投递 |
| self_improve 锁 | `state/perception/.cursor.json`、`state/perception/.consolidations.jsonl`（`memory/` 与 `notebook/` 由 agent_tool 自身的锁串行） |
| artifact 锁（改自己的版本时还须持 session 锁） | `state/artifacts/<aid>/artifact.json` 与版本有效性变更 |

共享 AgentRuntime 在恢复前核验保存目标；remote_ssh 在目标 Linux 上核验执行标记、boot_id 与进程身份，无法证明已停止则 RecoveryBlocked。没有持久结果的调用保持结果未知，不重放副作用。SSH 暂用于独立 xllm；Session 的远端 helper 未部署时明确报 Capability。
