# AICC 测试规划与模拟环境方案（cargo test）

## 1. 目标与发布门槛

本方案用于 AICC 模块发布前质量验收，核心目标：

1. 覆盖高风险语义：路由、fallback、长短任务边界、多租户隔离、错误码一致性
2. 用低成本模拟环境替代真实大模型调用
3. 形成可重复执行的 `cargo test` 用例体系

发布门槛建议：

- L1 核心语义层：100% 通过（必须）
- L2 Provider 适配器协议层：100% 通过（必须）
- L3 稳定性与并发层：>= 90% 通过（允许登记已知风险后发布）

---

## 2. 测试分层设计

### L1：核心语义层（必须全绿）

覆盖以下语义：

- Router：硬过滤（capability / must_features / allow-deny / alias映射）
- 打分：成本/延迟/负载/错误率
- 执行：`Immediate` / `Started` / `Queued`
- fallback：仅启动阶段可重试错误可触发；`Started` 后必须停止重试
- 多租户：跨租户 cancel 拒绝
- 资源校验：base64 大小、mime 白名单、URL 合法性
- 错误码：`bad_request` / `no_provider_available` / `model_alias_not_mapped` / `provider_start_failed` / `resource_invalid`

### L2：Provider 协议层（必须全绿）

对 `openai.rs` / `gimini.rs` 的请求-响应语义做协议测试：

- 2xx 正常返回
- 4xx 不可重试错误
- 429/5xx 可重试错误
- 响应体缺失关键字段
- 超时/网络抖动
- 不支持 capability 的错误路径

### L3：稳定性层（建议）

- 并发 complete 的 task_id 唯一性
- Registry 热更新并发下 route 稳定性
- 指标更新（EWMA/error/in_flight）收敛与不越界

---

## 3. 模拟测试环境（低成本）

### 3.1 组件构成

1. `MockProvider`（脚本化）
   - 输入：预置 `VecDeque<Result<ProviderStartResult, ProviderError>>`
   - 输出：按序返回 `Immediate/Started/Queued/retryable/fatal`
   - 记录：start 调用次数、cancel 记录

2. `MockTaskMgrHandler`（内存任务管理）
   - 已存在基础实现，可继续扩展断言字段（status/progress/data/events）

3. `FakeResourceResolver`
   - 可配置成功、权限失败、解析失败、超时

4. `TaskEventSink`（内存事件收集）
   - 用于断言事件序列：Queued -> Started -> Final/Error/CancelRequested

5. `Fake HTTP Server`（用于 adapter 测试）
   - 返回可编排 HTTP 响应：200/400/429/503/无效JSON/超时
   - 不访问外部 API，无模型成本

### 3.2 环境原则

- 不依赖真实 OpenAI/Gemini
- 所有测试默认离线可跑
- 测试数据固定，避免随机波动
- 单元测试优先，集成测试补充

---

## 4. 典型测试用例清单（建议最小集）

### 4.1 路由与映射（8）

- `route_01_mapped_primary_with_fallback`
- `route_02_alias_unmapped_returns_model_alias_not_mapped`
- `route_03_must_features_filtered_out`
- `route_04_tenant_allow_provider_types`
- `route_05_tenant_deny_provider_types`
- `route_06_max_cost_filter`
- `route_07_max_latency_filter`
- `route_08_tenant_mapping_override_global`

### 4.2 启动与 fallback（6）

- `start_01_retryable_error_then_fallback_success`
- `start_02_fatal_error_no_fallback`
- `start_03_started_must_stop_fallback`
- `start_04_queued_no_fallback`
- `start_05_all_candidates_failed_provider_start_failed`
- `start_06_fallback_respects_limit`

### 4.3 生命周期与事件（4）

- `task_01_immediate_persists_completed`
- `task_02_started_persists_running_and_binding`
- `task_03_queued_persists_pending_and_position`
- `task_04_emit_error_event_with_code`

### 4.4 多租户与安全（4）

- `sec_01_cancel_reject_cross_tenant`
- `sec_02_cancel_accept_same_tenant`
- `sec_03_resource_invalid_from_resolver`
- `sec_04_base64_policy_enforced`

### 4.5 可观测（2）

- `obs_01_error_code_mapping_consistent`
- `obs_02_log_redaction_no_prompt_or_base64`

### 4.6 并发稳定性（2）

- `conc_01_task_id_uniqueness_under_concurrency`
- `conc_02_registry_hot_update_route_consistency`

### 4.7 Adapter 协议测试（OpenAI/Gimini 各 6）

- `adapter_xx_01_http_200_success`
- `adapter_xx_02_http_429_retryable`
- `adapter_xx_03_http_503_retryable`
- `adapter_xx_04_http_400_fatal`
- `adapter_xx_05_invalid_json_fatal`
- `adapter_xx_06_timeout_or_network_error_classified`

---

## 5. 执行命令建议（cargo test）

在 `src/` 目录执行：

```bash
cargo test -p aicc -- --test-threads=1
cargo test -p aicc route_
cargo test -p aicc start_
cargo test -p aicc task_
cargo test -p aicc sec_
cargo test -p aicc obs_
cargo test -p aicc conc_
cargo test -p aicc adapter_openai_
cargo test -p aicc adapter_gimini_
```

说明：

- 初始阶段建议 `--test-threads=1` 保证稳定
- 并发类测试可单独启用多线程执行

---

## 6. 与其他模块的协作依赖（需联调验收）

1. TaskMgr
   - 任务状态机语义一致（Pending/Running/Completed/Canceled/Failed）
   - task data 字段持久化一致

2. 事件通道（TaskMgr/MsgQueue）
   - 事件 schema 和顺序语义对齐（Started/Queued/Final/Error/CancelRequested）

3. IAM/Auth
   - tenant 身份来源可信，跨租户校验可生效

4. Resource 服务
   - 资源权限校验与审计落地

5. 配置中心
   - alias 映射与策略更新原子性，避免漂移窗口

---

## 7. PR 验收证据要求（发布卡点）

提交实现时需附：

- 测试清单与风险映射表（case -> risk）
- `cargo test -p aicc` 全量通过结果
- 安全场景通过证据：
  - 跨租户 cancel 拒绝
  - base64 超限/非法 mime 拒绝
- 协议测试证据：
  - openai/gimini 各至少 6 条
- 失败路径证据：
  - `model_alias_not_mapped`
  - `provider_start_failed`

无证据按未实现处理，不建议发布。

---

## 8. 当前代码基线观察（便于增量实现）

当前 `src/frame/aicc/src/aicc.rs` 已有基础单测（约 7 条）：

- immediate 成功
- retryable fallback
- queued 状态持久化
- parent task 透传
- rootid/session_id 透传
- no provider 不建任务
- cross-tenant cancel 拒绝

建议在此基础上补齐上述 L1/L2/L3 用例矩阵。
