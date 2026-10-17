# system_config / msg-center CPU 优化 TODO

状态：**代码修复与本地验证已完成（2026-10-16），待推送 buckyos-base、部署 demo.buckyos.io 后线上复测**。ZoneSync 按变化更新需要 system_config 提供变化信号，评估后未实施。实施细节见文末“实施记录”。

分析时间：2026-10-17 00:08–00:17（香港时间）。目标环境：`demo.buckyos.io`，2 个逻辑 CPU。
部署版本为 `0.7.0+build261015.main.c9f4febeeedf.dirty`，部署源码参考提交为 `63378d78cbba723e80cd40d76ad0bef71445cc7c`；两个线上二进制摘要均与该次交付清单一致。

## 分析结论

高置信指向两个服务共用的 **RBAC 匹配反复编译正则**，引发大量临时内存分配、触页和释放。msg-center 的身份解析和周期同步进一步增加 system_config 的请求量。另有策略读取缓存到期后冗余重建 Enforcer 的问题。

发布二进制已去除调试符号；当前结论来自 CPU 采样、系统调用统计、二进制控制流及依赖源码的交叉核对，尚未得到完整符号调用栈，也未精确分摊各项 CPU 成本。没有旧版同负载对照，不能断言问题由最近一次升级引入。

## 线上证据

00:12:33 开始的 30 秒 CPU 采样如下。**100% 表示占满一个逻辑 CPU**，两项合计约占整机 CPU 容量的 33%。CPU 指标在附加 strace 之前采集。

| 指标 | system_config | msg-center |
| --- | ---: | ---: |
| 平均 CPU | 42.27% | 24.03% |
| 5 秒采样峰值 | 89.8% | 31.0% |
| minor faults / 秒 | 35,185.90 | 20,280.57 |
| major faults / 秒 | 0 | 0 |
| 平均 RSS | 31,924 KiB | 38,692 KiB |
| 独立 5 秒 strace 采样的 mmap 次数 | 2,271 | 1,258 |

- 线程采样中，开销集中在 Tokio worker；Sled、SQLite 和 kevent 分发线程接近空闲。观察窗口内 RSS 稳定，未见换页或数据库线程成为主要瓶颈的证据。
- system_config 的另一次 3 秒采样记录到 **2,078 次、每次 320,020 字节的 mmap**，对应 2,077 次按页取整为 323,584 字节的 munmap。
- 49 Hz、30 秒 perf 采样共 843 个样本，无丢失。两个程序的热点分别为 `system_config:0x210e69` 和 `msg_center:0x8db7d4`，均落在相同的 32 字节元素初始化循环；同时出现明显的缺页处理、清页开销。
- strace 会增加开销，其 futex 等待时间不是 CPU 使用时间；不可直接拿该汇总推断锁消耗了大部分 CPU。

截至 00:12:07 的 5 分钟日志窗口：

| 指标 | 数量 |
| --- | ---: |
| system_config HTTP 请求 | 2,813，约 9.4 次/秒 |
| system_config GET / LIST / SET | 1,633 / 546 / 473 |
| 来自 msg-center 的 GET | 699，占全部 GET 的 42.8% |
| 来自 control-panel 的 GET | 570 |
| 来自 node-daemon 的 GET | 156 |
| msg-center HTTP 请求 | 281，约 0.94 次/秒 |

GET 来源由日志中的 appid 统计。后台同步也会产生 GET，因此不能用 `699 / 281` 当作每次消息请求的固定放大倍数；现有 msg-center 访问日志也不足以精确拆分每个 RPC 方法的贡献。

## 根因与代码入口

### 1. RBAC 每次匹配反复编译正则：优先处理

- [rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs) 的 `DEFAULT_RBAC_MODEL` 对同一资源和 policy 最多重复调用 4 次 `keyGet3`，之后还会调用 `keyMatch3`，部分规则再调用 `regexMatch`。
- 部署锁定的 Casbin `2.20.0` 在 `src/model/function_map.rs::key_get3` 中每次构造 3 个 Regex，其中两个是固定模式。匹配一条适用规则最多产生 14 次 Regex 构造；实际次数取决于短路和命中的规则。
- 共享实现位于同级仓库 [buckyos-base/src/rbac/src/lib.rs](../../buckyos-base/src/rbac/src/lib.rs)，使用普通 Enforcer，常规鉴权分别检查 app 和 user。
- 部署依赖 `regex-automata 0.4.18` 的 `src/nfa/thompson/compiler.rs::Utf8State::new` 创建容量 10,000 的 `Utf8BoundedMap`；`map.rs::Utf8BoundedMap::clear` 首次初始化 10,000 个 32 字节表项，约 320 KB。
- 固定模式 `\{[^/]+?\}` 中的 `[^/]` 是 Unicode 补集，即使资源路径全是 ASCII，也会进入该构造路径。二进制循环的数据布局、调用方的空表检查和版本递增、线上分配尺寸均与此吻合。

这是对主要热点的高置信归因，不代表已经证明全部 CPU 都花在正则编译上。

### 2. 身份解析与周期同步放大配置查询

- [notify.rs](../src/frame/msg_center/src/notify.rs) 的 `notify_caller` 对 App 身份读取用户 profile，再通过 `agent_of_app` 查询 Agent 归属。
- [zone_agent.rs](../src/frame/msg_center/src/zone_agent.rs) 的 `find_zone_agent` 遍历用户并读取 Agent spec，找到后还会读取 settings/profile。
- [main.rs](../src/frame/msg_center/src/main.rs) 中 ZoneSync 每 30 秒读取用户和 Agent 资料；通知 watcher 也会周期轮询。业务消息少时仍存在这些请求。
- [system_config.rs](../src/kernel/buckyos-api/src/system_config.rs) 默认缓存范围为 `services/` 和 `system/rbac/`；上述 `users/` 读取以及 LIST 不受该缓存保护。

没有发现足以解释本次采样的空队列忙循环。`get_next` 读取邮箱历史记录再过滤的潜在成本可另行分析，本次 SQLite 线程数据不支持将其列为主要原因。

### 3. 缓存到期被当作策略变化

- [system_config.rs](../src/kernel/buckyos-api/src/system_config.rs) 的过期处理先删除旧值，重新插入时因没有旧条目而返回 `is_changed=true`，即使内容没有变化。
- [runtime.rs](../src/kernel/buckyos-api/src/runtime.rs) 的 `enforce` 据此调用 `rbac::update_enforcer`。缓存命中返回 false，**并非每个 RPC 都重建整个 Enforcer**。
- 活跃调用下，约 10 秒 TTL 到期后的下一次读取会导致冗余重建；拒绝后的强制刷新也可能触发。其线上成本尚未单独量化。

## 修复任务

### P0：消除 RBAC 热路径中的重复正则构造

- [x] 在共享 RBAC 层复用固定模式；对动态编译模式建立有界缓存，或在策略加载时预编译。优先复用已有机制，避免直接改第三方缓存目录中的源码。——用 Casbin `add_function` 覆盖 `keyMatch3`/`keyGet3`/`regexMatch`，未改第三方源码。
- [x] 缓存键覆盖实际模式及编译选项；缓存容量有上限，避免以每个请求资源路径形成无界条目。核对并发首次构造和缓存锁竞争。——每个函数一张表，改写规则与编译选项固定，键为原始策略模式；每表上限 1024，满则清空；锁只护查找/插入，编译在锁外，并发首次构造最多重复编译一次并保留先插入者。
- [x] 减少同一规则中重复的 `keyGet3` 提取，保持短路、捕获和匹配结果等价。——线程内复用上一次 `keyGet3` 调用结果（纯函数，结果完全相同），模型表达式不变。
- [x] 保留 Unicode、通配符、捕获组、锚点、动作正则、策略优先级及 app/user 双侧检查语义。缓存编译后的正则，仍逐请求验签并按当前策略判权。——非法模式由 Casbin 的 panic 改为不匹配（fail closed）并告警一次。
- [ ] 若修改同级 buckyos-base，按仓库依赖流程联动调用方和锁文件，构建 system_config、msg-center 及其它受影响的消费者。——已用本地 patch 构建并测试全部消费者；待推送 buckyos-base main 后更新 buckyos 锁。

### P1：仅在策略实际变化时重建 Enforcer

- [x] 将“已应用的策略版本/内容”与读取缓存 TTL 分离；读取过期仍要刷新，同内容刷新不重建。
- [x] 保留现有周期刷新和拒绝后的强制刷新；处理策略新增、修改、撤销、删除以及并发刷新，避免旧请求覆盖新策略。——策略键被删除时按“仅默认策略”生效（原实现不更新）；读取与安装按轮次串行，较早的读取不会在较晚之后安装。
- [x] 评估在 RBAC 层记录已应用版本/内容是否足够，避免为此改坏 SystemConfigClient 其它消费者的 `is_changed` 语义。——RBAC 层记录已安装的 model/policy 全文即可，`is_changed` 未改。未用 revision 判新旧：键不存在时没有 revision，数据库回滚还会使 revision 变小，按版本拒绝会挡住撤销。

### P2：减少身份查询和同步中的重复工作

- [x] 在单次 RPC 内复用已经验证的 caller identity，避免后续步骤重复验签和读取用户 profile；不得信任未验签的 claims。——请求作用域只记成功结果，键为完整 token 字符串。
- [x] 为只需确认 Agent 归属的调用提供精简查询，保留 spec 身份一致性验证，避免额外读取 settings/profile。
- [x] 区分需要 `allow_group` 等设置的调用，不能用精简归属结果替换其完整检查。——群相关 `zone_agent` 仍读 spec + settings；只有归属/存在判断走精简查询。
- [ ] 评估维护归属索引或短期有界缓存，明确用户/Agent 新增、删除、归属变更的失效规则；周期同步优先按变化更新，保留变化感知能力。——已做归属位置 hint（每次仍读并校验 spec，失效即回退全量扫描）；ZoneSync 按变化更新未做，原因见实施记录。

## 验证与验收

- [x] 正则实现与现有实现做等价性验证：允许、拒绝、跨用户访问、Unicode、特殊字符、通配符、捕获、动作集合和策略优先级。
- [ ] 验证不同 principal 类型、过期 token、错误 issuer/audience；确认权限撤销和策略删除按现有刷新约定生效。——principal 类型、撤销、删除，以及同一请求内过期/篡改 token 仍被拒，均已测；错误 issuer/audience 未单独补测（该校验代码未改动）。
- [x] 同一策略内容反复经历 TTL 到期时，Enforcer 重建次数不增加；真实变化能更新，并发刷新不会回退已应用版本。
- [x] 建立重复资源/策略匹配微基准，记录正则构造次数、分配量和耗时；缓存预热后，同一模式不再随请求数线性重新编译，且高基数输入下缓存大小受控。
- [x] 运行共享 RBAC、`buckyos-api` 与 msg-center 的相关测试和构建检查，按实际修改范围补充测试；报告实际执行结果。
- [ ] 在相同机器规格、Agent 数量、策略和请求率下，分别测空闲轮询及消息活动场景；先预热，再至少采样 60 秒，比较 CPU、minor faults、约 320 KB 分配次数、RPC 延迟、下游 GET/LIST 数以及 Enforcer 重建次数。
- [ ] 优化前后保持相同日志级别；不能通过关闭鉴权、减少服务实例或降低测试请求率制造性能改善。记录各阶段改善幅度，区分正则优化与查询减少的收益。
- [ ] 修复完成并通过验证后，再按部署流程安排上线和复测；更新此 TODO 的完成状态。

## 实施记录（2026-10-16）

### 改动

| 仓库 | 文件 | 内容 |
| --- | --- | --- |
| buckyos-base | `src/rbac/src/matcher.rs`（新） | `keyMatch3`/`keyGet3`/`regexMatch` 的缓存版：沿用 Casbin 2.20 的模式改写与捕获编号，只复用每个策略模式的编译结果；同线程复用上一次 `keyGet3` 结果；`build_enforcer` 中覆盖内置函数。 |
| buckyos-base | `src/rbac/src/lib.rs` | 记录已安装 Enforcer 的 model/policy 全文；新增 `apply_enforcer_policy`（文本相同不重建，返回是否重建；构建串行，并发同内容只构建一次）、`enforcer_build_count`、`matcher_regex_compile_count`、`matcher_cached_pattern_count`；删除已无调用方的 `update_enforcer`；`create_enforcer` 仍无条件重建。重建时打 info 日志 `rbac enforcer rebuilt for changed policy`。 |
| buckyos-base | `src/Cargo.toml`、`src/rbac/Cargo.toml` | rbac 直接依赖 `regex = "1"`：它本来就是 casbin 的传递依赖（锁定 1.13.1），锁文件不新增 crate。casbin 版本要求由 `2.9` 收紧为 `2.20`（`matcher.rs` 需要 2.15 起才有的 `Dynamic` 参数 `OperatorFunction` 和 `dynamic_to_str`；2.20.0 是当前最新版，锁定版本不变）。 |
| buckyos-base | `src/rbac/tests/policy_cache.rs`、`tests/fixtures/` | 集成测试与微基准；fixtures 是 buckyos 当前 `DEFAULT_RBAC_MODEL`/`DEFAULT_RBAC_POLICY` 的快照。 |
| buckyos | `kernel/buckyos-api/src/rbac_config.rs` | `sync_rbac_enforcer`：“读策略 + apply”按轮次串行；等待中的调用方直接采用刚结束那一轮的结果（成功或错误），system_config 变慢时不会逐个叠加超时。 |
| buckyos | `kernel/buckyos-api/src/runtime.rs` | `enforce` 不再依赖 `is_changed`，改走 `sync_rbac_enforcer`；拒绝后的强制刷新（每秒至多一次）在轮次内 invalidate 后重读，仅在策略文本变化时重新判权。 |
| buckyos | `frame/msg_center/src/owner_session.rs` | `RuntimeSessionTokenVerifier`：`zone_agent` 只读 spec + settings，`agent_owner`/`is_zone_agent` 只读 spec，均带位置 hint。`TokenVerifierSlot` 外包 `RequestScopedVerifier`：请求作用域（tokio task_local）内复用 `verify`/`resolve_user_did`/`zone_agent`/`agent_owner` 的成功结果；作用域外直通。 |
| buckyos | `frame/msg_center/src/zone_agent.rs` | `find_zone_agent_spec(source, did, hints)`：只读 spec（`parse_agent_spec` 校验 + DID 一致）；先查 hint 用户，否则按 `list("users")` 顺序扫描；找到则更新 hint，找不到则删除。删除生产已不用的 `find_zone_agent`。 |
| buckyos | `frame/msg_center/src/main.rs`、`notify.rs` | `serve_request` 整体放进请求作用域（覆盖 kRPC、group_http、object_access、cyfs_dispatch）；`agent_of_app` 只需归属，改用 `agent_owner`。 |

### 验证结果

- 函数级等价：缓存版与 `casbin::function_map` 原函数逐一比对，覆盖 37 个模式 × 34 个键，以及每个模式的各变量和缺失变量。用例含 Unicode、正则元字符、字面 `{`、同段多变量、捕获组偏移和非法模式；Casbin 会 panic 的输入，缓存版返回不匹配或空串。
- Enforcer 级等价：BuckyOS 默认模型 + 默认策略 + 典型 zone tail 下，与不覆盖函数的 Casbin Enforcer 对比，20 主体 × 32 资源 × 7 动作共 4480 组合全部一致（release，`--include-ignored`；debug 默认只跑抽样子集）。sudo 与 app/user 双侧检查用例通过。
- 重建：同一文本 apply 50 次，构建数不增；授权、撤销、删除 tail 各重建一次，判权立即生效；非法模型 apply 失败时保留原 Enforcer；16 个并发同内容 apply 只构建并安装一次。buckyos-api 侧：同文本 TTL 到期（`is_changed=true`）20 次不重建；慢的旧读取进行中发起的强制刷新排在其后读取、最后安装；等待者共享失败轮次的错误，不重复读取。
- 微基准（release，7 种典型请求 × 20 轮，含 app/user 双侧）：

| 指标（每请求） | Casbin 内置函数 | 缓存版 |
| --- | ---: | ---: |
| 耗时 | 3612 µs | 143 µs |
| 分配次数 | 34,852 | 2,442 |
| 分配量 | 44,284 KiB | 85 KiB |

- 预热后正则编译次数不再增长；再跑 4000 个不同请求路径，缓存模式数不变（只缓存策略侧模式）。
- msg-center：App 身份（Agent 构造 App）调用 `notify.send` 时，一次请求内 `verify`/`resolve_user_did`/`zone_agent` 各调用 1 次；作用域外分别 ≥2/≥2/≥1。不同请求不共享；同一请求内的过期或篡改 token 仍逐次验签并被拒。Agent 查找：首次按用户顺序扫描且只读 spec；之后只读 hint 位置的 spec 一次；Agent 迁移、spec 无效、删除时回退扫描并更新或删除 hint。
- 测试套件（`--test-threads=1`，buckyos 用本地 patch 指向 buckyos-base 的 rbac）：rbac 11 通过（全矩阵用例默认 ignored，已在 release 下跑过）；buckyos-api 241、msg_center 146、task_manager 40、system_config 34 全部通过；`cargo check --workspace --all-targets` 通过。buckyos-api 的 `app_service_uses_node_gateway_for_verify_hub_before_zone_bootstrap` 在并行全量运行中失败过一次（环境变量竞争；原代码单跑也通过），重跑和串行均通过。

### 预期效果（尚未线上测量）

- 预热后每次 enforce 不再编译正则，每次约 320 KB 的 Unicode 表初始化随之消失；这是两个服务共同的主要热点。
- 同文本 TTL 到期不再重建 Enforcer。
- msg-center 每请求对 system_config 的读取：User 身份读 owner 资源时，profile GET 由 2 降为 1；Agent 构造 App 调用 `notify.*` 时，profile GET 由 2 降为 1，Agent 归属由“1 次 LIST + 最多 N 个 spec GET + settings + profile”降为 hint 命中后的 1 次 spec GET；Agent 身份调用同理。ZoneSync（每 30 秒一轮）的读取量不变。

### 未完成与风险

1. **发布顺序**：buckyos 依赖新增的 `rbac::apply_enforcer_policy`。须先推 buckyos-base main，再在 buckyos 执行 `cargo update -p rbac`（两仓库的 Cargo.lock 都不在 git 中）；顺序反了 buckyos 会编译失败。本地联调：`cargo test -p <pkg> --config 'patch."https://github.com/buckyos/buckyos-base.git".rbac.path="/home/lzc/project/buckyos-base/src/rbac"'`，跑完还原 Cargo.lock。
2. **线上未验证**：未部署 demo.buckyos.io，也未在同规格、同负载下复测 CPU、minor faults、320 KB mmap 次数、RPC 延迟、GET/LIST 数和 Enforcer 重建次数。重建次数可 grep 上述日志。
3. **ZoneSync 按变化更新未做**：system_config 没有子树 revision，也不发变更 kevent；只有逐键 GET 才拿得到 revision，不读就无法感知变化。需要 system_config 先提供 `users/` 子树 revision 或 watch 能力。不以拉长 30 秒间隔换取指标。
4. 对非 Agent 的 DID 调用 `is_zone_agent` 仍会全量扫描。短 TTL 负缓存会推迟新建 Agent 的识别，未做。
5. **发现的既有问题（未处理、未实测）**：默认 RBAC（`users`/`admin` 的 `obj://config/users/{users}/agents/{agent}/{key}` 读写规则）允许用户写 `users/<自己>/agents/<任意 agent_id>/spec`，而 `AgentSpec::validate` 不核对 spec 所在的用户路径。归属查找按 `list("users")` 顺序取第一个合法 spec，因此从代码看，排序更靠前的用户可能伪造他人 Agent 的归属，再通过 `user_may_observe` 读取该 Agent 的邮箱。hint 会让已找到的位置保持不变，但冷启动仍按扫描顺序。需要另行设计 spec 写入权限或归属校验。

## 诊断资料

以下资料位于当前工作区的同级部署目录，不随本仓库单独分发：

- [结构化分析记录](../../demo.buckyos.io/prepared/cpu-analysis-20261017-0008/findings.json)：CPU、缺页、分配、版本和判断边界。
- [后续 5 分钟脱敏日志统计](../../demo.buckyos.io/prepared/cpu-analysis-20261017-0008/recent-log-summary.jsonl)：截至 00:17:04 的另一窗口，勿与上文截至 00:12:07 的窗口混算。
- 目标机采样文件：`/root/demo-deploy/cpu-analysis-20261017.perf`，权限 `0600`。诊断结束时两个进程的 `TracerPid` 均为 0，服务仍为 active，未修改配置或重启。
