
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

> **Beta 2.2 breaking-change 基线**：AICC 对外接口拆成控制面、typed inference 数据面和 Helper 三层，不保留 legacy all-in-one method、旧字段或兼容别名。Provider 渠道、线上协议和模型语义分别由 Provider Profile、Protocol Adapter 和 Model Driver 表达。最新协议以 `doc/aicc/aicc_api设计.md`、`doc/aicc/provider_profile_schema.md` 和 `doc/aicc/driver_metadata_schema.md` 为准。

---

## 2. 总体架构与数据流

### 2.1 关键组件（AICC 关注点）

AICC 内部逻辑可以抽象为 8 个核心子系统（这里按职责描述，而非目录/模块拆分）：

1. **API 层（入口）**

   * 接收调用请求，抽取租户上下文（user/app/tenant）
   * 做轻量校验与规范化（字段存在性、资源引用大小限制等）

2. **Catalog（静态真相源）**

   * Model Driver catalog 定义模型固有技术语义
   * Provider Rules catalog 定义渠道模型映射、operation 和请求规则
   * Pricing catalog 定义渠道价格，Known Provider catalog 为管理界面提供默认服务商信息

3. **Registry（实例池与能力声明）**

   * 保存当前可用 ProviderInstance（多 provider、多实例池化）
   * 暴露快照给 Router 使用，并维护必要的运行指标（in-flight、EWMA 延迟、错误率等）

4. **Model Resolver 与 Router（解析和选择策略）**

   * 将渠道原始 `provider_model_id` 唯一解析为 Model Driver 和 origin model
   * 硬过滤：结构化 capability、租户 allow/deny、逻辑目录是否可映射
   * 打分：成本/延迟/负载/错误率（权重可配置，支持租户 override）
   * 输出：primary + fallback 列表 + 完整模型和渠道身份

5. **Provider Profile（渠道策略）**

   * 定义 discovery、origin mapping、operation、请求限制和渠道价格
   * 内置主流 Provider 使用专用策略；小型兼容 Provider 使用受限配置

6. **Protocol Adapter（协议执行层）**

   * 只实现已注册 operation 的认证、endpoint、wire 编解码、stream 和异步状态机
   * 多个 Provider Profile 和 Model Driver 可以复用同一 Protocol Adapter

7. **Provider Call Resolver（调用解析）**

   * 合并模型语义、Provider rules、用户参数和价格，产生 `ResolvedProviderCall`
   * 执行层不得再根据模型名或 URL 猜测 operation

8. **Security & Observability（安全与可观测）**

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

/// 高层特性声明（如 plan/json_output/web_search/vision/asr 等）
pub type Feature = String;

pub mod features {
    pub const PLAN: &str = "plan";
    pub const TOOL_CALLING: &str = "tool_calling";
    pub const JSON_OUTPUT: &str = "json_output";
    pub const WEB_SEARCH: &str = "web_search";
    pub const VISION: &str = "vision";
    pub const ASR: &str = "asr";
    pub const VIDEO_UNDERSTAND: &str = "video_understand";
}
```

Router 使用结构化能力门限（逻辑模型定义的 `min_line`，见 `aicc_router.md` §6.7）做硬过滤。能力真相源是 Model Driver 静态能力、Protocol Adapter operation 能力和 Provider discovery 动态能力的交集。请求只使用结构化 `ModelRequirement` / `ModelDisable`；不保留 `Feature`、`must_features` 或 `ProviderInstance.features` 兼容判断。

> 注意：早期实现里 `llm.chat` 默认补 `web_search`、unknown model 乐观声明能力的做法已废弃。现在 unknown model 走 conservative fallback，不默认声明 `tool_call` / `web_search` / `vision` / `json_schema`，只能由 driver metadata 显式声明。

---

### 3.2 ResourceRef（非文本资源引用）

AICC 只定义“引用形态”，实际的校验/读取/鉴权走系统既有资源机制。

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ResourceRef {
    /// 推荐：cyfs://...（权限/校验由系统资源机制负责）
    Url { url: String, mime_hint: Option<String> },

    /// 小资源可内联；AICC 做大小和 MIME 强校验
    Base64 { mime: String, data_base64: String },

    /// 稳定资源引用，解析和读取必须携带租户上下文
    NamedObject { obj_id: ObjId },
}
```

AICC 侧的硬性原则：

* base64 必须强限制大小、mime 白名单
* 任何日志/metrics/tracing 不记录原始 base64 或资源原文
* 业务结果、Task Final event 和 ProviderState 必须精确保存，不得复用日志脱敏函数
* singular/array 数量、URL、Base64、MIME 和 NamedObject 必须经过公共 ResourceRef 校验层

---

### 3.3 模型与渠道身份

- `logical_model`：只用于 `route.resolve` 的逻辑目录名。
- `ModelUID`：模型可执行身份；同一基础模型通过不同协议访问时允许使用不同 ModelUID。
- `model_driver_id` / `origin_model_id`：模型固有语义和原厂身份。
- `provider_instance_name` / `provider_profile_id`：用户实例和渠道规则身份。
- `protocol_adapter_id`：实际使用的线上协议适配器。
- `provider_model_id`：Provider discovery 返回的原始模型名，实际调用必须原样保留。

Provider mapping 可以确定 ModelUID，但不能修改实际调用使用的 `provider_model_id`。Model Driver 的 `defaults` 只在 Driver 已经唯一确定后应用，不能参与 Driver 所有权判定。

---

## 4. 对外接口语义（AICC 视角）

> 这里不讨论 krpc 的通用机制与 JSON 传输细节，只描述 **AICC 的方法语义与字段含义**。

AICC 对外最核心的方法是控制面 `route.resolve` 与数据面 typed inference（`chat.completions.create` / `images.generate`），外加 `helper.*` 组合层；详见 `doc/aicc/aicc_api设计.md`。`cancel` 仍按下文语义工作。

公开调用必须使用以下入口之一：

* 普通 LLM / 图片调用：`helper.llm_chat` / `helper.text_to_image`
* 显式两阶段调用：`route.resolve` → `chat.completions.create` / `images.generate`
* 取消任务：`cancel`

### 4.1 请求/响应

请求与响应结构以 `doc/aicc/aicc_api设计.md` 和 `doc/aicc/maintenance/krpc_aicc_calling_guide.md` 为准。不要发送 `method: "complete"`；该名称只存在于内部执行链路和历史资料中。

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

### 5.1 ProviderInstance 声明

```rust
#[derive(Clone, Debug)]
pub struct ProviderInstance {
    pub provider_instance_name: String,
    pub provider_type: ProviderType,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub endpoint: String,
    pub credential_ref: CredentialRef,
    pub pricing_context: Option<PricingContext>,
}
```

Provider Instance 属于 system-config 管理的实例私有配置。Catalog 更新不能修改实例名称、endpoint、凭据、区域或协议选择。

### 5.2 Provider Trait（AICC 的“统一执行面”）

```rust
pub enum ProviderStartResult {
    /// 短任务：直接完成
    Immediate(AiResponse),

    /// 长任务：已开始/已提交（后续通过系统任务事件通道输出）
    Started,

    /// 请求已进入内部队列
    Queued { position: usize },
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn inventory(&self) -> ProviderInventory;

    /// 估算成本（供 Router 打分 / 预算/限额策略）
    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput;

    /// 核心：执行已经解析好的 operation，不重新解释模型家族
    async fn start(
        &self,
        ctx: InvokeCtx,
        call: ResolvedProviderCall,
        sink: TaskEventSink, // 事件写入接口（对接系统既有任务事件通道）
    ) -> Result<ProviderStartResult, ProviderError>;

    /// 只有真实发起上游取消或成功中止本地执行时才能返回 accepted
    async fn cancel(&self, ctx: InvokeCtx, task_id: &str) -> Result<CancelDisposition, ProviderError>;
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
    pub selected_model_uid: String,
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub provider_model_id: String,
    pub operation: String,
    pub fallback_attempts: Vec<RouteFallbackAttempt>,
}
```

#### 路由算法要点

1. **候选集**：从 RegistrySnapshot 中取支持 `capability` 的实例
2. **硬过滤**（必须满足才进入打分）：

   * 结构化 ModelRequirement 满足最终能力交集
   * tenant allow/deny provider_type
   * logical model 能映射到唯一的可执行 ModelUID
3. **打分**：

   * `cost_est` 由统一 Pricing Resolver 计算
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

## 7. 核心执行流程（AICC 视角）

公开 kRPC method 由 `AiccServerHandler::handle_rpc_call` 分发；能力请求进入 `AIComputeCenter::complete_with_method()` 内部执行链路。内部函数名不是公开 method 名，调用方不得据此构造 RPC 请求。

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
   * Provider 不支持取消时返回 `accepted=false`
   * 支持取消时调用上游 API或中止本地 polling，并屏蔽竞态产生的 late Final event

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
