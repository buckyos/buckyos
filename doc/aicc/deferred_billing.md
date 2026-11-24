# AICC 延迟费用核对持久数据

## 1. Overview

AICC 在 fal 成功任务完成后，按原始 request_id 查询 Billing Events。生成结果立即交付，费用可先为明确标记的静态估算或未知，最终账单费用回填 Usage。协议明确返回的有效费用优先，不被账单查询或静态价格覆盖。参见 [Usage 成本](usage_costs.md)。

## 2. Data Classification

持久数据为原有平台 RDB 中的执行记录、用量记录，以及执行 binding 中新增的 billing 状态。内存轮询器、HTTP 连接和定时器可丢弃。没有新文件存储、媒体对象或第三方依赖。

## 3. Storage Strategy

复用 `aicc_execution_record.record_json`，不新增表或数据库列。`aicc_usage_event` 保持 event_id 唯一，费用补全是对同一条记录的修订，不新增调用或叠加金额。适用于 SQLite/PostgreSQL。SQLite 的核账写事务在读取前以 BEGIN IMMEDIATE 取得写锁，避免并发核账将读事务升级为写事务时产生 BUSY_SNAPSHOT；PostgreSQL 使用普通事务。依据：[SQLite 事务隔离](https://www.sqlite.org/isolation.html)。

## 4. Schema Definitions

`record_json.binding.billing` 的字段：

| 字段 | 类型 | 含义 |
|---|---|---|
| schema_version | u32 | 当前为 1 |
| status | pending / local_pending / applied / local / protocol_cost / unresolved | 待查账、待本地测量、已采用账单、已本地结算、已采用协议费用、核对结束但未能补全费用 |
| attempts | u32 | 已持久化的核对或测量尝试次数 |
| next_attempt_ms | u64 | 下一次可领取时间；领取时同时作为短期租约截止时间 |
| last_error | string? | 固定错误类别；不保存响应正文、令牌或密钥 |
| event | object? | request_id、endpoint_id、可空 quantity/unit、USD amount/currency |
| measurement | object? | 实际制品宽高、帧数、累计像素与探测方式 |

队列在 native binding 与上游 request_id 持久化时建立，早于任何账单查询。身份、凭证引用/指纹及 Provider 信息复用 binding/resume；不存原始密钥。

## 5. Schema Version

RDB 全局版本仍为 3；新增 JSON 子对象版本为 1。无 billing 子对象的执行不参与队列。

## 6. Upgrade Compatibility Strategy

当前开发阶段不增加旧版转换器或迁移流程。沿用原执行 JSON 的可扩展字段方式，历史记录不凭当前配置自动入队。未完成核对的记录不得因幂等窗口到期或普通任务清理而删除；现有执行存储没有该删除路径。

## 7. Extensibility Rules

event_id、tenant、Provider、模型和 request_id 的关联不可改变。领取、重试、成功状态通过旧 record_json 的 CAS 防止并发覆盖。账单金额、Usage JSON、执行输出缓存与 applied 状态在同一事务提交；崩溃发生在事务前则重查，事务后不再重复应用。协议费用记录只转为 protocol_cost，不查询或覆盖。

TaskMgr 的完成结果不可变，是完成时的快照；其中费用来源明确标记 pending。AICC 的查询/幂等重放和 Usage 汇总读取更新后的缓存/账本。令牌配额按原有实际用量结算，不因费用核对再次扣量。

## 8. Query Patterns

后台启动即扫描到期的成功执行，之后周期扫描。按终态、billing.status、next_attempt_ms 筛选，限批并发；到期扫描使用 `idx_aicc_billing_due` 部分表达式索引，仅索引成功且为 pending/local_pending 的执行，按 next_attempt_ms、created_at_ms、task_id 排序；SQLite 查询显式使用此索引，PostgreSQL 使用相同表达式与筛选条件的索引。索引在存储初始化事务中幂等创建，不新增表或列。领取使用 CAS 并保存短租约，重启后租约到期自动重查。空结果、网络错误、限流、临时错误均持久化退避重试，不重新生成媒体。

账单 HTTP 使用已解析连接配置的 `operation_base_urls["billing.events"]`，默认值由 fal Known Provider 声明为 `https://api.fal.ai/v1`；自定义端点必须显式配置，Mock 指向本机账单端点，不回退到公网。按 request_id 和 endpoint_id 过滤；显式传入覆盖任务日期的时间窗口，避免默认只查近 24 小时导致关机后漏账。重复、错误关联、非法金额或不完整分页不用于结算。quantity 可为空；有效最终金额仍可采用，不臆造计费像素量。

账单权限不足（401/403）、凭证不可用或该连接没有账单端点时，先将状态以 CAS 持久化为 `local_pending`，然后使用冻结价格和实际用量执行本地结算。完成状态为 `local`，费用来源为 `static_pricing`；普通网络错误、限流和账单暂未出账仍保留 `pending` 并退避重查。`local_pending` 的进程恢复只继续本地测量，不重复请求无权限的账单 API。

需要 `megapixel` 且结果没有提供时，测量生成制品的真实宽、高和视频帧数，按输出各帧像素之和除以 1,000,000 估算，不使用请求目标尺寸或时长乘假定帧率。先读受保护制品下载接口的有限头部并用现有 ext-core 的 ffprobe 提取帧数；头部不足时限量下载，必要时解码计帧。测量证据（制品名、宽高、帧数和读取方式）保存在 billing.measurement，Usage 用量、费用、执行缓存和 `local` 状态同一事务提交。下载/探测中断保留 `local_pending` 并重试，不能将失败测量记作零像素或零成本。

核对最多尝试 12 次，且不超过执行创建后的 24 小时。达到任一上限且未取得有效账单时，使用冻结价格尝试本地结算；本地测量仍失败则以 `unresolved` 结束，不再入队。已有静态估价保留，去掉 pending 来源；没有可靠金额时保持未知，记录 `billing_retries_exhausted:<固定错误类别>`，不伪造零费用。执行缓存与 Usage 费用来源在同一 CAS 事务中更新，重放不重复扣费。重启后的领取也受相同持久化次数和时间限制。

前端每 30 秒刷新可见待核账记录。后台刷新保留现有统计、展开状态和已加载分组，新分区成功后独立替换；切换身份范围或筛选条件才清空对应统计。移动端刷新已加载的事件页，相同 event_id 采用新记录；所有可见记录结束核账后停止轮询。
