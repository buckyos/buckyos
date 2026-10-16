# system_config / msg-center CPU 优化 TODO

状态：**已完成线上只读分析，待实现修复和验证**。本次只新增任务记录，不包含代码修复或上线。

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

- [ ] 在共享 RBAC 层复用固定模式；对动态编译模式建立有界缓存，或在策略加载时预编译。优先复用已有机制，避免直接改第三方缓存目录中的源码。
- [ ] 缓存键覆盖实际模式及编译选项；缓存容量有上限，避免以每个请求资源路径形成无界条目。核对并发首次构造和缓存锁竞争。
- [ ] 减少同一规则中重复的 `keyGet3` 提取，保持短路、捕获和匹配结果等价。
- [ ] 保留 Unicode、通配符、捕获组、锚点、动作正则、策略优先级及 app/user 双侧检查语义。缓存编译后的正则，仍逐请求验签并按当前策略判权。
- [ ] 若修改同级 buckyos-base，按仓库依赖流程联动调用方和锁文件，构建 system_config、msg-center 及其它受影响的消费者。

### P1：仅在策略实际变化时重建 Enforcer

- [ ] 将“已应用的策略版本/内容”与读取缓存 TTL 分离；读取过期仍要刷新，同内容刷新不重建。
- [ ] 保留现有周期刷新和拒绝后的强制刷新；处理策略新增、修改、撤销、删除以及并发刷新，避免旧请求覆盖新策略。
- [ ] 评估在 RBAC 层记录已应用版本/内容是否足够，避免为此改坏 SystemConfigClient 其它消费者的 `is_changed` 语义。

### P2：减少身份查询和同步中的重复工作

- [ ] 在单次 RPC 内复用已经验证的 caller identity，避免后续步骤重复验签和读取用户 profile；不得信任未验签的 claims。
- [ ] 为只需确认 Agent 归属的调用提供精简查询，保留 spec 身份一致性验证，避免额外读取 settings/profile。
- [ ] 区分需要 `allow_group` 等设置的调用，不能用精简归属结果替换其完整检查。
- [ ] 评估维护归属索引或短期有界缓存，明确用户/Agent 新增、删除、归属变更的失效规则；周期同步优先按变化更新，保留变化感知能力。

## 验证与验收

- [ ] 正则实现与现有实现做等价性验证：允许、拒绝、跨用户访问、Unicode、特殊字符、通配符、捕获、动作集合和策略优先级。
- [ ] 验证不同 principal 类型、过期 token、错误 issuer/audience；确认权限撤销和策略删除按现有刷新约定生效。
- [ ] 同一策略内容反复经历 TTL 到期时，Enforcer 重建次数不增加；真实变化能更新，并发刷新不会回退已应用版本。
- [ ] 建立重复资源/策略匹配微基准，记录正则构造次数、分配量和耗时；缓存预热后，同一模式不再随请求数线性重新编译，且高基数输入下缓存大小受控。
- [ ] 运行共享 RBAC、`buckyos-api` 与 msg-center 的相关测试和构建检查，按实际修改范围补充测试；报告实际执行结果。
- [ ] 在相同机器规格、Agent 数量、策略和请求率下，分别测空闲轮询及消息活动场景；先预热，再至少采样 60 秒，比较 CPU、minor faults、约 320 KB 分配次数、RPC 延迟、下游 GET/LIST 数以及 Enforcer 重建次数。
- [ ] 优化前后保持相同日志级别；不能通过关闭鉴权、减少服务实例或降低测试请求率制造性能改善。记录各阶段改善幅度，区分正则优化与查询减少的收益。
- [ ] 修复完成并通过验证后，再按部署流程安排上线和复测；更新此 TODO 的完成状态。

## 诊断资料

以下资料位于当前工作区的同级部署目录，不随本仓库单独分发：

- [结构化分析记录](../../demo.buckyos.io/prepared/cpu-analysis-20261017-0008/findings.json)：CPU、缺页、分配、版本和判断边界。
- [后续 5 分钟脱敏日志统计](../../demo.buckyos.io/prepared/cpu-analysis-20261017-0008/recent-log-summary.jsonl)：截至 00:17:04 的另一窗口，勿与上文截至 00:12:07 的窗口混算。
- 目标机采样文件：`/root/demo-deploy/cpu-analysis-20261017.perf`，权限 `0600`。诊断结束时两个进程的 `TracerPid` 均为 0，服务仍为 active，未修改配置或重启。
