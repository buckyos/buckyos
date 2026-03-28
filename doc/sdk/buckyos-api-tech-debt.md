# buckyos-api 技术债记录

更新时间: 2026-03-20

## 背景

`buckyos-api` 是一个跨度很大的基础组件，目前同时承担了这些职责:

- runtime 初始化与全局状态管理
- 身份、token、trust key、RBAC 相关逻辑
- system_config 访问与缓存
- 各类系统服务 client
- 一部分通用数据模型与服务文档生成

这次记录的目标不是“把所有大文件都拆掉”，而是优先偿还那些已经影响运行时语义、故障隔离和维护成本的债务。

## 判断原则

本轮 review 对技术债做了一个区分:

- 代码量大但模式稳定、心智负担低的 kRPC client，不是当前最高优先级
- 会导致 panic、语义污染、状态不一致、失败扩散的核心逻辑，是当前最高优先级

因此，优先关注 `runtime.rs`、`system_config.rs`、`control_panel.rs`、`lib.rs` 这几个边界层文件。

## 优先级排序

### P0: 失败模型与重试策略过粗

当前问题:

- 一次 RPC 失败在部分路径上可能被放大为整个流程失败
- 后台 keep-alive、token 刷新、service info 上报、trust key 刷新之间缺少明确的故障隔离
- 缺少“哪些错误可重试、哪些错误应立即暴露”的统一规则
- 公共 API 中仍有 `panic!` / `unimplemented!`，调用方无法只通过 `Result` 完成错误处理

典型风险:

- 某个依赖服务短暂抖动，触发整个 service 初始化失败
- 后台定时任务中的单点异常导致进程异常退出或状态长期失真
- 调用方只能靠 `unwrap()` 或进程级兜底处理错误

建议动作:

1. 定义统一的错误分类
   - `Retryable`: 超时、连接失败、503、leader 切换、瞬时不可达
   - `NonRetryable`: 参数错误、权限错误、数据格式错误、契约不匹配
   - `Degraded`: 可降级继续运行，但需要记录告警

2. 为关键路径定义不同重试策略
   - `login()`: 小次数、指数退避、带 jitter
   - `renew_token_from_verify_hub()`: 重试但不 panic，不阻塞主业务
   - `update_service_instance_info()`: 失败只告警，下一轮继续
   - `refresh_trust_keys()`: 失败进入降级态，不应拖垮整个进程
   - 普通业务 RPC: 由调用方决定是否重试，不在 client 层一刀切

3. 把“失败隔离”作为 runtime contract 的一部分
   - 后台任务失败不能直接演变成整个系统 panic
   - 周期性任务应记录失败次数、最近失败原因、下一次重试时间
   - 达到阈值后进入 degraded 状态，而不是无限打印相同错误

4. 清理公共 API 中的 panic/unimplemented
   - 不支持的能力返回显式错误
   - 尚未实现的接口返回 `NotSupported` / `NotImplemented`

验收标准:

- 单次 RPC 失败不会直接导致进程 panic
- 周期任务的失败是局部的、可观测的、可恢复的
- 调用方能从错误类型判断是否应该重试

### P0: SystemConfigClient 的缓存语义不可靠

当前问题:

- 缓存是进程级全局静态，但过期时钟是 client 私有的
- runtime 每次获取 system config client 都会新建实例
- 缓存没有按 `service_url`、`session_token`、`RPCContext` 做隔离
- `set_by_json_path()` 会把局部 patch 值写回完整 key 的缓存

这会带来:

- 不同 zone / 不同身份上下文之间理论上可能串缓存
- 新 client 复用旧缓存，但没有正确继承失效语义
- 局部更新后，后续 `get()` 可能读到残缺 JSON

建议动作:

1. 先修正语义错误
   - `set_by_json_path()` 成功后不要把 patch value 直接写回 key 缓存
   - 更安全的策略是直接删除该 key 缓存

2. 重新定义缓存边界
   - 缓存 key 至少包含 `service_url + auth_scope + rpc_context + key`
   - 或者干脆把缓存收敛到 runtime 持有的单一 client 实例中

3. 明确缓存的用途
   - 只缓存读多写少的稳定配置
   - 对高频变化数据或带权限差异的数据禁用共享缓存

4. 为缓存加测试
   - patch 后立即 get
   - 不同 token 访问同一 key
   - 新 client 与旧 client 的过期行为

验收标准:

- patch 更新不会污染完整文档读取
- 不同访问上下文之间不共享不安全缓存
- 缓存是否生效与失效具备可预测性

### P1: runtime 的前置条件、后台任务与全局注册契约不清晰

当前问题:

- `AppService` 的登录前置条件与后台 keep-alive 所需条件不一致
- `keep_alive` 中会调用依赖 `device_config` 的逻辑，但 `login()` 没有对所有 service 类型做同等校验
- 全局 runtime 通过 `OnceCell` 保存，但 `set_buckyos_api_runtime()` 静默吞掉重复注册失败
- `login()` 每次调用都会创建新的永久后台任务

这类问题的代价不一定是立刻崩溃，但会导致:

- 初始化阶段没有失败，运行几秒后才在后台炸掉
- 二次初始化、重试或测试环境里出现隐性状态污染
- 运行时行为依赖调用顺序，而不是显式 contract

建议动作:

1. 明确每种 `BuckyOSRuntimeType` 的最小前置条件
   - 哪些字段必须在 `init` 后存在
   - 哪些字段必须在 `login` 前存在
   - 哪些后台任务只对哪些 runtime type 生效

2. 收敛后台任务生命周期
   - 避免重复 spawn keep-alive
   - 为 keep-alive 提供显式启动状态或句柄

3. 调整全局 runtime 接口
   - `set_buckyos_api_runtime()` 返回 `Result`
   - 如果重复注册，调用方必须显式处理

4. 把“初始化失败”前移
   - 能在 `init/login` 阶段确认的问题，不要留给后台任务 `unwrap()`

验收标准:

- 相同输入一定得到相同的初始化和登录结果
- 后台任务不会因为缺少前置状态而 panic
- 重复注册与重复启动会被显式拒绝

### P1: 公共 API 中仍存在 panic 和未完成接口

当前问题:

- `runtime.rs` 中多个目录访问接口对不支持的 runtime 直接 `panic!`
- `control_panel.rs` 中 `install_app_service/remove_app/stop_app/start_app` 仍是 `unimplemented!()`
- `app_mgr.rs` 中 `FrameServiceInstanceConfig::new()` 仍未实现

这会导致:

- 作为基础库时，调用方无法只靠类型系统和 `Result` 理解边界
- 未实现接口在编译期不可见，在运行时才炸

建议动作:

1. 清理所有导出 API 中的 `panic!` / `unimplemented!`
2. 不成熟接口先收窄可见性，或明确标记为实验性
3. 如果短期不实现，就返回明确错误而不是中断进程

验收标准:

- 导出的稳定 API 不再通过 panic 表达业务边界
- 未实现能力不会在运行期意外中断调用方

### P2: crate 边界过宽，核心与生成式 client 混在一起

当前问题:

- `lib.rs` 对很多模块做了 `pub use *`
- runtime、identity、system_config、service client、测试辅助混在一个 crate 里
- 结果是 crate 的表面积过大，依赖和职责边界不清晰

说明:

- 这不是当前最急的风险
- 但它是后续可维护性和演进成本的来源

建议动作:

1. 先逻辑拆层，再考虑物理拆 crate
   - `runtime_core`
   - `identity_auth`
   - `system_config_client`
   - `service_clients_generated`

2. 优先减少通配 re-export
   - 让依赖方按模块引用
   - 逐步收敛公共 API 面

3. 保留生成式 client，但把它们标记为“低心智负担层”
   - 方便后续集中生成、校验、替换

验收标准:

- 调用方更容易判断自己依赖的是核心运行时还是普通 client
- 修改 runtime 不会无关波及大批 service client

## 本轮建议的落地顺序

建议按下面顺序偿还:

1. 修复 `SystemConfigClient` 的缓存污染问题
2. 清理导出 API 中的 `panic!` / `unimplemented!`
3. 给 runtime 增加明确的前置条件校验和全局注册返回值
4. 引入统一的重试与故障隔离策略
5. 最后再考虑模块边界和物理拆分

## 本轮不建议优先处理的内容

以下内容暂不建议作为这一版主目标:

- 单纯因为文件长而拆分生成式 kRPC client
- 只做表面格式统一、不改变失败模型的重构
- 没有测试补充的“大规模命名整理”

原因是这些工作对认知负担有帮助，但对线上风险和基础能力提升不如前面的几项直接。

## 直接关联的代码位置

- `src/kernel/buckyos-api/src/lib.rs`
- `src/kernel/buckyos-api/src/runtime.rs`
- `src/kernel/buckyos-api/src/system_config.rs`
- `src/kernel/buckyos-api/src/control_panel.rs`
- `src/kernel/buckyos-api/src/app_mgr.rs`

## 后续建议

如果下一步继续推进，建议直接拆成 3 个小任务:

1. `system_config` 缓存语义修正
2. `runtime` 失败模型与 keep-alive 安全化
3. `panic/unimplemented` 清理与错误类型补全
