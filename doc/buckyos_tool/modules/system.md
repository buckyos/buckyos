# System 模块需求

> 状态：Draft  
> 对应 modules：`system`、`node`、`service`

## 1. 目标与边界

管理 BuckyOS Zone、当前节点和系统服务的状态、生命周期、维护与升级。cyfs-gateway 专属管理
不在本模块；这里只消费统一系统健康和服务状态。

## 2. 资源模型

- system/zone overall desired and observed state；
- node id、平台、capabilities、maintenance state；
- service spec、instance 和 readiness；
- update/rollback operation、目标版本、backup gate 和 rollback point。

node-daemon 生命周期与 workload 生命周期必须分开，不能把 controller restart 等同于停止全部
App/Service。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `system status` | read | sync | Zone overview、健康和版本 |
| `system capabilities` | read | sync | 返回当前平台/部署可用操作 |
| `system metrics` | read | sync | 获取受权限控制的关键指标 |
| `system dry-run --operation <op>` | privileged | task | 对 update/rollback 检查版本、节点顺序和 backup gate |
| `system apply <operation-id>` | operation-defined | task | 按 update/rollback 风险级别执行 operation |
| `node list` | read | sync | 列出 Zone 节点 |
| `node check [node-id]` | read | sync | 标准探测或黑盒检查 |
| `node start [node-id]` | privileged | task/either | ensure-running |
| `node stop [node-id]` | destructive | task/either | 正常或明确 blackbox 模式 |
| `node restart [node-id]` | privileged | task | stop/start 编排，不默认杀 workloads |
| `node maintenance-set <node-id>` | privileged | task | cordon/drain/uncordon |
| `service list` | read | sync | 列出系统服务和实例 |
| `service status <service-id>` | read | sync | desired/observed/readiness |
| `service restart <service-id>` | privileged | task | 按服务策略重启 |

## 4. 在线与故障模式

- 系统在线时使用正式 system/control-panel/node-daemon API。
- 标准入口不可用时，只允许 node check 和显式 HostControl blackbox 操作。
- 远程多节点顺序、屏障和 drain 属于 system-control，不由本地 CLI 自己循环实现。
- Windows Desktop 开发者可从本机 Deno 入口调用 HostControlClient；普通用户优先在 Jarvis
  容器中调用，没有 Jarvis 时使用 paios 临时容器。

## 5. 实现基础

当前已有本地 `node check/start/ensure-running/stop/restart`，Control Panel 已有 system overview、
status、metrics 和日志入口；system update check/apply 仍为占位。新 CLI 应复用 node-control
抽象，不能把现有 Rust shell/进程扫描代码直接翻译到各 TS command。
