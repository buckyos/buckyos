# AICC 测试规划与模拟环境方案（cargo test）

## 1. 目标与发布门槛

本方案用于 AICC 模块发布前质量验收，核心目标：

1. 覆盖高风险语义：路由、fallback、长短任务边界、多租户隔离、错误码一致性
2. 用低成本模拟环境替代真实大模型调用
3. 形成可重复执行的 `cargo test` 用例体系

发布门槛建议：

- L1 核心语义层：100% 通过（必须）
- L2 Provider 适配器协议层：100% 通过（必须）
- L3 协议规范层：100% 通过（必须）
- L4 稳定性与并发层：>= 90% 通过（允许登记已知风险后发布）
- L5 复杂任务编排层：>= 80% 通过（允许登记已知风险后发布）

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

### L3：协议规范层（必须全绿）

覆盖各 capability 的输入输出协议规范：

- **LLM（llm_router）**
  - `payload.text/messages/input_json/options` 组合规则
  - tool calling 字段合法性（`tool_specs`）
  - temperature/top_p/max_tokens 参数边界
- **Text2Image（text2image）**
  - prompt 来源优先级（text/messages/input_json/options）
  - 图片输出 artifact 引用格式（URL/Base64）
  - size/quality/style 参数映射
- **Voice2Text / Video2Text / Image2Text（voice2text / video2text / image2text）**
  - `ResourceRef::Base64`：mime 白名单 + 大小上限 + 可解码
  - `ResourceRef::Url`：scheme 必须存在 + URL 合法性
  - language/hotword 等配置参数
- **Text2Voice（text2voice）**
  - 文本输入长度限制
  - voice/model 配置参数格式
  - 音频输出格式（URL 引用）
- **通用协议规范**
  - 敏感字段不落日志（prompt/base64 原文）
  - `idempotency_key` 透传
  - `session_token` 与 tenant 绑定

### L4：稳定性层（建议）

- 并发 complete 的 task_id 唯一性
- Registry 热更新并发下 route 稳定性
- 指标更新（EWMA/error/in_flight）收敛与不越界

### L5：复杂任务编排层（建议）

- 任务拆解：LLM 生成 DAG 计划
- 串行执行：依赖步骤阻塞
- 并行执行：同 parallel_group 步骤并发
- 循环重规划：质量不达标时触发二次拆解
- fallback 执行：子任务失败时使用备选 alias

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

### 4.8 协议规范层 - Base64 资源（8）

- `proto_b64_01_image_valid_png`
- `proto_b64_02_image_valid_jpeg`
- `proto_b64_03_audio_valid_wav`
- `proto_b64_04_audio_valid_mp3`
- `proto_b64_05_video_valid_mp4`
- `proto_b64_06_invalid_mime_rejected`
- `proto_b64_07_size_limit_exceeded_rejected`
- `proto_b64_08_malformed_base64_rejected`

### 4.9 协议规范层 - URL 资源（6）

- `proto_url_01_https_valid`
- `proto_url_02_http_allowed`
- `proto_url_03_missing_scheme_rejected`
- `proto_url_04_empty_url_rejected`
- `proto_url_05_invalid_url_format_rejected`
- `proto_url_06_resource_unreachable_simulated`

### 4.10 协议规范层 - 各模型特定格式（10）

**LLM 特定：**
- `proto_llm_01_messages_format_valid`
- `proto_llm_02_input_json_format_valid`
- `proto_llm_03_tool_specs_format_valid`
- `proto_llm_04_temperature_boundary_valid`

**Text2Image 特定：**
- `proto_t2i_01_prompt_from_text`
- `proto_t2i_02_prompt_from_messages`
- `proto_t2i_03_prompt_from_options`
- `proto_t2i_04_artifact_url_format`

**Voice2Text/Video2Text 特定：**
- `proto_v2t_01_language_param_respected`
- `proto_v2t_02_hotword_param_respected`

**Text2Voice 特定：**
- `proto_t2v_01_voice_param_format_valid`
- `proto_t2v_02_output_artifact_url_format`

### 4.11 协议规范层 - 混合资源模式（4）

- `proto_mix_01_url_and_base64_in_same_task`
- `proto_mix_02_multiple_images_mixed`
- `proto_mix_03_workflow_mixed_resource_modes`
- `proto_mix_04_cross_capability_resource_passthrough`

### 4.12 协议规范层 - 安全与脱敏（4）

- `proto_sec_01_no_base64_in_logs`
- `proto_sec_02_no_prompt_in_logs`
- `proto_sec_03_no_artifact_bytes_in_events`
- `proto_sec_04_idempotency_key_preserved`

### 4.13 复杂任务编排（8）

- `workflow_01_plan_generates_valid_dag`
- `workflow_02_serial_dependency_blocks_until_ready`
- `workflow_03_parallel_group_executes_concurrently`
- `workflow_04_replan_triggered_on_quality_threshold`
- `workflow_05_retryable_subtask_uses_fallback_alias`
- `workflow_06_started_subtask_never_retries_cross_instance`
- `workflow_07_each_step_routes_to_correct_capability`
- `workflow_08_event_sequence_reflects_dag_structure`

---

## 5. 执行命令建议（cargo test）

在 `src/` 目录执行：

```bash
# 全量测试
cargo test -p aicc -- --test-threads=1

# 按组执行
cargo test -p aicc route_
cargo test -p aicc start_
cargo test -p aicc task_
cargo test -p aicc sec_
cargo test -p aicc obs_
cargo test -p aicc conc_
cargo test -p aicc adapter_openai_
cargo test -p aicc adapter_gimini_

# 协议规范层
cargo test -p aicc proto_b64_
cargo test -p aicc proto_url_
cargo test -p aicc proto_llm_
cargo test -p aicc proto_t2i_
cargo test -p aicc proto_v2t_
cargo test -p aicc proto_t2v_
cargo test -p aicc proto_mix_
cargo test -p aicc proto_sec_

# 复杂任务编排
cargo test -p aicc workflow_
```

说明：

- 初始阶段建议 `--test-threads=1` 保证稳定
- 并发类测试可单独启用多线程执行
- 协议层测试建议在 MockProvider 中增加"协议校验器"模式

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

---

## 9. 复杂任务规划提示词示例（生产可用版）

### 9.1 Planner 系统提示词

```
你是任务编排规划器。请把用户目标拆解为可执行子任务图（DAG），并严格输出 JSON。

必须遵守：
1. 每个步骤必须包含 id、title、capability、model_alias、depends_on、parallel_group、acceptance_criteria。
2. 仅允许以下 capability：
   - llm_router（文本生成/分析）
   - text2image（文生图）
   - text2voice（文生语音）
   - image2text（图像识别）
   - voice2text（语音转写）
   - video2text（视频理解）
3. 能并行的步骤必须放同一 parallel_group；有依赖的步骤必须在 depends_on 声明目标步骤的 id。
4. 若存在高风险步骤，给出 fallback_alias 和 retry_policy。
5. 若任务可能失败或质量不足，必须给出 replan_trigger 条件。
6. 若涉及二进制资源，明确指定资源传递方式：url 或 base64。
7. 输出仅 JSON，不要解释文字。
```

### 9.2 Planner 输出 JSON Schema

```json
{
  "plan_id": "string",
  "goal": "string",
  "max_replan_rounds": 3,
  "steps": [
    {
      "id": "step_1",
      "title": "string",
      "capability": "llm_router | text2image | text2voice | image2text | voice2text | video2text",
      "model_alias": "string",
      "must_features": ["string"],
      "inputs": {
        "key": "${step_x.output.key}"
      },
      "depends_on": ["step_x"],
      "parallel_group": "group_a",
      "resource_mode": "url | base64",
      "retry_policy": {
        "max_retries": 2,
        "backoff_ms": 1000,
        "fallback_alias": "string"
      },
      "acceptance_criteria": [
        "score >= 0.8",
        "output contains 'xyz'"
      ],
      "replan_trigger": {
        "condition": "score < 0.6",
        "target_step": "step_1"
      }
    }
  ],
  "global_acceptance": [
    "all_steps_completed",
    "total_cost < 1.0"
  ]
}
```

### 9.3 典型复杂任务示例：产品发布多媒体包

**输入：**
- 产品 PRD 文档（文本）
- 演示视频文件（video）
- 采访音频文件（audio）
- 截图素材（images）

**目标产出：**
- 发布文案（LLM）
- 1 张主视觉海报（T2I）
- 30 秒旁白（T2Voice）
- FAQ（LLM）
- 视频要点摘要（V2T + LLM）

**拆解后的 DAG：**

| Step | Capability | Model Alias | Mode | Depends On | Parallel Group |
|------|------------|-------------|------|------------|----------------|
| S1 | llm_router | llm.plan.default | - | - | - |
| S2 | video2text | v2t.default | url | - | group_input |
| S3 | voice2text | asr.default | base64 | - | group_input |
| S4 | image2text | i2t.default | url | - | group_input |
| S5 | llm_router | llm.default | - | S2,S3,S4 | - |
| S6 | text2image | t2i.default | - | S5 | - |
| S7 | text2voice | t2v.default | - | S5 | - |
| S8 | llm_router | llm.default | - | S5 | group_qa |
| S9 | llm_router | llm.qa.default | - | S8 | - |

**特点：**
- 并行：S2/S3/S4 同时处理输入媒体
- 串行：S5 依赖所有输入处理完成
- 并行：S6/S7/S8 依赖 S5 结果
- 循环：S8 质量评分 < 0.6 时触发 replan回到 S5

---

## 10. 模拟环境增强：协议校验器

### 10.1 SimProvider 协议校验模式

```rust
enum ProtocolMode {
    Normal,                    // 正常执行
    ValidateOnly,              // 仅校验输入协议，不真正执行
    InjectError(ProviderError), // 注入指定错误
}

struct SimProvider {
    // ... 现有字段 ...
    protocol_mode: ProtocolMode,
    input_validator: Option<Box<dyn Fn(&CompleteRequest) -> Result<(), ProviderError>>>,
}
```

### 10.2 协议校验器示例

```rust
// Base64 校验器
fn base64_validator(mime_whitelist: &[&str], max_bytes: usize) -> impl Fn(&CompleteRequest) -> Result<(), ProviderError> {
    move |req| {
        for resource in &req.payload.resources {
            match resource {
                ResourceRef::Base64 { mime, data_base64 } => {
                    if !mime_whitelist.contains(&mime.as_str()) {
                        return Err(ProviderError::fatal(format!("mime '{}' not allowed", mime)));
                    }
                    let decoded = general_purpose::STANDARD.decode(data_base64)
                        .map_err(|_| ProviderError::fatal("invalid base64"))?;
                    if decoded.len() > max_bytes {
                        return Err(ProviderError::fatal(format!("size {} exceeds limit {}", decoded.len(), max_bytes)));
                    }
                }
                ResourceRef::Url { url, .. } => {
                    if !url.contains("://") {
                        return Err(ProviderError::fatal("url must contain scheme"));
                    }
                }
                // ...
            }
        }
        Ok(())
    }
}
```

### 10.3 模拟资源解析器

```rust
enum ResourceResolverMode {
    Success,
    PermissionDenied,
    NotFound,
    Timeout,
    Malformed,
}

struct FakeResourceResolver {
    mode: ResourceResolverMode,
    // 模拟资源映射表
    resource_map: HashMap<String, Vec<u8>>,
}
```

### 10.4 模拟存储（用于 artifact URL）

```rust
struct FakeArtifactStorage {
    artifacts: Mutex<HashMap<String, Vec<u8>>>,
}

impl FakeArtifactStorage {
    fn store(&self, data: Vec<u8>, mime: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.artifacts.lock().unwrap().insert(id.clone(), data);
        format!("sim://artifact/{}", id)
    }

    fn get(&self, url: &str) -> Option<Vec<u8>> {
        url.strip_prefix("sim://artifact/")
            .and_then(|id| self.artifacts.lock().unwrap().get(id).cloned())
    }
}
```

---

## 11. PR 验收证据要求（发布卡点）- 更新版

提交实现时需附：

- 测试清单与风险映射表（case -> risk）
- `cargo test -p aicc` 全量通过结果
- 安全场景通过证据：
  - 跨租户 cancel 拒绝
  - base64 超限/非法 mime 拒绝
  - URL 无 scheme 拒绝
- 协议测试证据：
  - openai/gimini 各至少 6 条
  - Base64 协议测试至少 8 条
  - URL 协议测试至少 6 条
  - 各模型特定格式测试至少 10 条
- 失败路径证据：
  - `model_alias_not_mapped`
  - `provider_start_failed`
  - `resource_invalid` 各场景
- 复杂任务编排证据（如实现）：
  - DAG 生成与解析正常
  - 并行步骤确实并发执行
  - 串行步骤正确阻塞
  - 循环重规划触发正常

无证据按未实现处理，不建议发布。
