
# AI Compute Center（AICC）服务设计文档


## 1. 设计目标与定位

**AI Compute Center（AICC，服务名建议：`aicc`）**是 BuckyOS 内核体系中的 AI 调度与执行入口服务，核心职责是：

1. **统一多类 AI 能力入口**：LLM、T2I、T2V、T2Voice、I2T、V2T、Video2Text 等（能力集合可扩展）。
2. **在多 Provider / 多实例之间做“选择与启动”**：

   * 根据 capability + feature + 模型别名（alias）映射，找到可用实例
   * 在候选实例中按**成本/速度/负载/错误率 + 租户策略**进行打分选择
   * 在“启动阶段”支持实例级 fallback（避免重复提交长任务）
3. **结果归一（Result Normalization）**：

   * 对不同 Provider 的输出做最小必要的结构归一（文本 / JSON / artifact 引用 / usage & cost 等）
   * 对敏感内容做日志与观测层面的最小泄露原则（不落 prompt、原始资源字节等）

> 边界声明
>
> * AICC **不重新设计**系统已有的：RPC 框架（krpc）、任务生命周期管理（TaskMgr）、事件/日志队列（MsgQueue）。
> * AICC 只需要：在长任务场景下**生成/关联 task_id**，并将进度与输出写入系统既有的任务事件通道（具体格式/存储/订阅语义以系统组件为准）。

---

## 2. 总体架构与数据流

### 2.1 关键组件（AICC 关注点）

AICC 内部逻辑可以抽象为 6 个核心子系统（这里按“职责”描述，而非目录/模块拆分）：

1. **API 层（入口）**

   * 接收调用请求，抽取租户上下文（user/app/tenant）
   * 做轻量校验与规范化（字段存在性、资源引用大小限制等）

2. **ModelCatalog（模型别名与映射）**

   * 将 `(capability, alias)` 映射到不同 provider_type/instance 可用的真实模型名
   * 支持租户覆盖策略（例如租户 A 强制用某个 vendor 的模型）

3. **Registry（实例池与能力声明）**

   * 保存当前可用 ProviderInstance（多 provider、多实例池化）
   * 暴露快照给 Router 使用，并维护必要的运行指标（in-flight、EWMA 延迟、错误率等）

4. **Router（选择策略）**

   * 硬过滤：capability、must_features、租户 allow/deny、alias 是否可映射
   * 打分：成本/延迟/负载/错误率（权重可配置，支持租户 override）
   * 输出：primary + fallback 列表 + 映射后的 provider_model

5. **Provider Adapter（执行适配层）**

   * 统一 Provider 抽象：inproc / outproc / vendor API
   * Provider 负责执行与（必要时）判定长任务（见 5.2）

6. **Security & Observability（安全与可观测）**

   * 多租户隔离：路由、限流/预算（若启用）、资源权限、任务可见性
   * 观测：指标、追踪、错误码；严格限制敏感字段进入日志/metrics

---

### 2.2 数据流（短任务 / 长任务）

1. **短任务（Provider 判定为可直接完成）**

   * 调用方 → AICC → Router 选实例 → Provider 执行 → AICC 归一结果 → 立即返回 result

2. **长任务（Provider 判定为异步/耗时任务）**

   * 调用方 → AICC → 生成/关联 `task_id` → Router 选实例 → Provider 提交任务
   * AICC 立即返回 `task_id`（以及可选的事件引用 `event_ref`）
   * 后续进度/增量输出/最终结果：由 Provider/AICC 写入系统既有的任务事件通道（TaskMgr/MsgQueue），调用方按系统既定方式消费

> 关键约束：
> **AICC 不使用网络流式协议（SSE/WebSocket）作为核心机制**；长任务输出依赖系统既有任务事件通道。

---

## 3. 核心概念与数据模型

### 3.1 Capability 与 Feature

```rust
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub enum Capability {
    LlmRouter,
    Text2Image,
    Text2Video,
    Text2Voice,
    Image2Text,
    Voice2Text,
    Video2Text,
}

/// 高层特性声明（如 plan/json_output/vision/asr 等）
pub type Feature = String;

pub mod features {
    pub const PLAN: &str = "plan";
    pub const TOOL_CALLING: &str = "tool_calling";
    pub const JSON_OUTPUT: &str = "json_output";
    pub const VISION: &str = "vision";
    pub const ASR: &str = "asr";
    pub const VIDEO_UNDERSTAND: &str = "video_understand";
}
```

Router 会用 `must_features` 做硬过滤（例如 “要做 Plan” 必须选择声明支持 `plan` 的实例）。

---

### 3.2 ResourceRef（非文本资源引用）

AICC 只定义“引用形态”，实际的校验/读取/鉴权走系统既有资源机制。

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ResourceRef {
    /// 推荐：cyfs://...（权限/校验由系统资源机制负责）
    Url { url: String, mime_hint: Option<String> },

    /// 兼容：base64（AICC 仅做强限制 + 严禁日志落地）
    Base64 { mime: String, data_base64: String },
}
```

AICC 侧的硬性原则：

* base64 必须强限制大小、mime 白名单
* 任何日志/metrics/tracing 不记录原始 base64 或资源原文

---

### 3.3 模型抽象名（Model Alias）与 ModelCatalog

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSpec {
    /// 稳定抽象名，例如:
    /// - "llm.plan.default"
    /// - "video2text.general"
    /// - "t2i.fast"
    pub alias: String,

    /// 可选：调用方指定真实模型名（一般不建议对普通调用方开放）
    pub provider_model_hint: Option<String>,
}
```

**ModelCatalog 的职责**：把 alias 落到不同 provider 的“真实模型名”，并支持租户覆盖。

典型映射键：

* `(capability, alias, provider_type)` → `provider_model_name`

---

## 4. 对外接口语义（AICC 视角）

> 这里不讨论 krpc 的通用机制与 JSON 传输细节，只描述 **AICC 的方法语义与字段含义**。

AICC 对外最核心的方法通常是：

* `complete`：发起一次 AI 计算（短任务直接返回结果；长任务返回 task_id）
* `cancel`：best-effort 取消指定 task（是否可取消由系统任务机制与 provider 能力决定）

### 4.1 complete 请求/响应（语义）

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompleteRequest {
    pub capability: Capability,
    pub model: ModelSpec,
    pub requirements: Requirements,
    pub payload: AiPayload,

    /// 业务侧可传；AICC 默认不做去重，仅可透传给 provider（可选）
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompleteResponse {
    /// AICC 侧生成或关联的 compute task id
    pub task_id: String,

    /// 短任务：Succeeded + result != None
    /// 长任务：Running + result == None
    pub status: CompleteStatus,
    pub result: Option<AiResponseSummary>,

    /// 可选：事件通道引用（opaque string）
    /// 由系统任务/事件组件定义其格式；AICC 仅透传/生成
    pub event_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CompleteStatus {
    Succeeded,
    Running,
    Failed, // 启动阶段失败（路由失败/参数不合法/资源不可用等）
}
```

**要点**：

* **是否长任务由 Provider 判定**（AICC 不用耗时阈值猜测）
* AICC 的 fallback 只发生在**启动阶段失败**，一旦某实例成功启动（尤其返回 Started/Running），就不再跨实例重试，避免重复提交产生多份费用/输出

---

### 4.2 cancel（语义）

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CancelRequest {
    pub task_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CancelResponse {
    pub task_id: String,
    /// best-effort：是否接受并触发取消流程
    pub accepted: bool,
}
```

---

## 5. Provider 抽象与执行边界

### 5.1 ProviderInstance 声明（供路由过滤）

```rust
#[derive(Clone, Debug)]
pub struct ProviderInstance {
    pub instance_id: String,
    pub provider_type: String, // e.g. "vendor-a", "local", "inproc-x"
    pub capabilities: Vec<Capability>,
    pub features: Vec<Feature>,

    /// outproc: endpoint；inproc: plugin_key（具体语义由实现决定）
    pub endpoint: Option<String>,
    pub plugin_key: Option<String>,
}
```

### 5.2 Provider Trait（AICC 的“统一执行面”）

```rust
pub enum ProviderStartResult {
    /// 短任务：直接完成
    Immediate(AiResponseSummary),

    /// 长任务：已开始/已提交（后续通过系统任务事件通道输出）
    Started,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn instance(&self) -> &ProviderInstance;

    /// 估算成本（供 Router 打分 / 预算/限额策略）
    fn estimate_cost(&self, req: &CompleteRequest, provider_model: &str) -> CostEstimate;

    /// 核心：启动执行，由 provider 自行判定长/短任务并返回 Started/Immediate
    async fn start(
        &self,
        ctx: InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        sink: TaskEventSink, // 事件写入接口（对接系统既有任务事件通道）
    ) -> Result<ProviderStartResult, ProviderError>;

    /// best-effort 取消
    async fn cancel(&self, ctx: InvokeCtx, task_id: &str) -> Result<(), ProviderError>;
}
```

> 注意：`TaskEventSink` 在此仅作为“写事件的抽象接口”，不规定事件结构与队列/存储实现（由系统既有组件定义）。AICC 只关心：Started/Progress/Delta/Final/Error/Canceled 等语义是否可表达。

---

## 6. Registry 与 Router 设计（AICC 核心）

### 6.1 Registry：实例池快照与指标

Registry 需要满足：

* 支持动态 add/remove ProviderInstance（热更新）
* 路由时获取快照，避免路由过程被并发修改影响一致性
* 维护 Router 所需的最小指标集合（例如）：

  * `in_flight`
  * `ewma_latency_ms`
  * `ewma_error_rate`
  * （可选）历史成本均值 / 成功率分能力维度统计

接口形态示意：

```rust
pub struct Registry {
    // instances + provider handles + metrics
}

impl Registry {
    pub fn add_instance(&self, inst: ProviderInstance, provider: Box<dyn Provider>);
    pub fn remove_instance(&self, instance_id: &str);

    pub fn snapshot(&self, capability: Capability) -> RegistrySnapshot;
    pub fn get_provider(&self, instance_id: &str) -> Option<std::sync::Arc<dyn Provider>>;
}
```

---

### 6.2 Router：硬过滤 + 打分 + fallback

你提出的约束是：**成本/速度/负载结合自动统计 + 用户配置**，并按租户隔离。

#### 路由配置模型（示意）

```rust
pub struct RouteWeights {
    pub w_cost: f64,
    pub w_latency: f64,
    pub w_load: f64,
    pub w_error: f64,
}

pub struct TenantRouteConfig {
    pub allow_provider_types: Option<Vec<String>>,
    pub deny_provider_types: Option<Vec<String>>,
    pub weights: Option<RouteWeights>,
}

pub struct RouteConfig {
    pub global_weights: RouteWeights,
    pub tenant_overrides: std::collections::HashMap<String, TenantRouteConfig>,
}
```

#### Router 输出

```rust
pub struct RouteDecision {
    pub primary_instance_id: String,
    pub fallback_instance_ids: Vec<String>,
    pub provider_model: String, // alias 映射后的真实模型名
}
```

#### 路由算法要点

1. **候选集**：从 RegistrySnapshot 中取支持 `capability` 的实例
2. **硬过滤**（必须满足才进入打分）：

   * `must_features ⊆ instance.features`
   * tenant allow/deny provider_type
   * ModelCatalog 能映射 `(capability, alias, provider_type)` → `provider_model`
3. **打分**：

   * `cost_est = provider.estimate_cost(req, provider_model)`
   * `latency/load/error` 来自 Registry 指标
   * 归一化后按权重线性组合
4. **选择**：

   * primary = 最低分
   * fallback = 后续若干候选（用于“启动阶段失败”的重试）

> 关键执行约束：
>
> * fallback 只用于 **启动阶段失败**（连接失败/瞬时 5xx/鉴权失败等）。
> * 一旦某个 provider 返回 `Started`，AICC 视为任务已提交，**停止 fallback**，避免重复提交多个长任务。

---

## 7. 核心执行流程（AICC 视角伪代码）

下面伪代码刻意避免展开 TaskMgr/MsgQueue 的内部语义，只保留 AICC 的决策与调用边界：

```rust
pub async fn complete(ctx: InvokeCtx, req: CompleteRequest, state: AppState) -> CompleteResponse {
    // 1) 轻校验（字段存在性、base64 限制、url 语法等）
    validate_req(&req)?;

    // 2) 生成/关联 task_id（长短任务都可统一生成，便于观测与追踪）
    let task_id = gen_task_id();

    // 3) 路由：快照 + 过滤 + 打分 + 得到 decision
    let snap = state.registry.snapshot(req.capability.clone());
    let decision = state.router.route(&ctx.tenant_id, &req, &snap, &state.route_cfg, &state.model_catalog)?;

    // 4) 解析资源引用（不在此定义 cyfs 权限细节）
    let resolved = state.resource_resolver.resolve(&ctx, &req).await?;

    // 5) 构造任务事件 sink（对接系统既有任务事件通道；不在 AICC 定义其细节）
    let sink = state.task_event_sink_factory.build(&ctx, &task_id);

    // 6) 启动 provider（启动失败才尝试 fallback）
    let result = start_with_fallback(&ctx, &req, &resolved, &decision, &sink, &state).await;

    match result {
        Ok(ProviderStartResult::Immediate(r)) => CompleteResponse {
            task_id,
            status: CompleteStatus::Succeeded,
            result: Some(r),
            event_ref: sink.event_ref(), // 可选
        },
        Ok(ProviderStartResult::Started) => CompleteResponse {
            task_id,
            status: CompleteStatus::Running,
            result: None,
            event_ref: sink.event_ref(), // 可选
        },
        Err(_) => CompleteResponse {
            task_id,
            status: CompleteStatus::Failed,
            result: None,
            event_ref: sink.event_ref(), // 可选
        }
    }
}
```

---

## 8. 多租户隔离与安全策略（AICC 必做）

AICC 的隔离点应覆盖：

1. **路由隔离**

   * tenant 级 allow/deny provider_type
   * tenant 级权重覆盖（成本优先/速度优先等）
   * tenant 级模型 alias 覆盖（强制用某 vendor 或某 region 实例）

2. **资源隔离**

   * 资源引用解析必须带租户上下文
   * 权限校验与审计由系统既有机制完成，但 AICC 需要正确传递 ctx

3. **任务可见性与取消**

   * cancel 必须校验 task 所属 tenant，防跨租户操作
   * cancel 是 best-effort：AICC 只负责触发与传播，不承诺立即终止

4. **敏感信息最小暴露**

   * logs/metrics/tracing：只记录 task_id、tenant_id、instance_id、错误码、耗时等
   * 严禁记录：prompt 原文、资源原文/base64、生成物字节

---

## 9. 可观测性（建议的最小集）

* **指标（按 capability / provider_instance 维度）**

  * `start_success`, `start_fail`
  * `immediate_success`, `started_long_task`
  * `cancel_requests`
  * `route_no_candidate`, `route_alias_unmapped`
* **延迟**

  * 路由耗时、启动耗时、短任务总耗时
* **日志**

  * 只打结构化字段：`task_id / tenant_id / instance_id / capability / alias / status / error_code`

---

## 10. 错误码建议（AICC 输出层面）

* `bad_request`：字段缺失/格式错误/base64 超限
* `no_provider_available`：硬过滤后无候选
* `model_alias_not_mapped`：alias 无法映射到任何实例/模型
* `provider_start_failed`：启动阶段失败（可带 retryable）
* `resource_invalid`：资源引用无权限/不可用/校验失败
* `canceled`：任务被取消（终态）
* `internal_error`：未分类异常

---

## 11. AICC 实现避坑清单（仅保留与 AICC 强相关）

1. **长任务边界必须由 Provider 显式返回**：`Immediate` vs `Started`，AICC 不做耗时阈值猜测。
2. **fallback 只发生在启动失败**：一旦返回 `Started`，立刻停止重试，避免重复提交。
3. **alias 映射要可观测**：alias 未映射是高频运维问题，必须清晰报错与打点。
4. **Registry 快照化**：路由时用快照，避免实例热更新造成路由过程不一致。
5. **资源处理安全第一**：base64 严控、绝不落日志；资源权限校验必须绑定租户 ctx。
6. **多租户配置覆盖要有优先级规则**：global → tenant →（可选）app/user，避免策略“叠加失控”。

---

如果你希望我继续做进一步“聚焦化”，我还能基于这版再帮你做两件事（都不涉及 TaskMgr/MsgQueue/krpc 的内部设计）：

1. 给出一份 **ModelCatalog + RouteConfig** 的示例配置（YAML/JSON），专门服务于 alias 映射与租户策略。
2. 把 `AiPayload / AiResponseSummary / Requirements` 进一步抽象成“能力无关的统一骨架 + capability 专用扩展”，让 AICC 更像“调度内核”，能力扩展成本更低。
