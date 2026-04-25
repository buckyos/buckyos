# AICC Provider 修改需求整理

阶段：阶段 2，provider 修改需求整理，review 后再改。  
范围：只整理 provider 接入层、registry 注册方式、成本估算接口的修改点；本文件不要求直接改代码。

本次 review 后决策更新：当前已在新开发分支，不考虑向前兼容。因此下面方案按“一次性切到新版模型路由”整理，不再保留旧 `ModelCatalog` alias 兼容层，不再保留旧 `ProviderInstance.instance_id` 命名，不再保留旧 `CostEstimate` 作为正式接口。

## 1. 结论摘要

`doc/aicc/aicc_router.md` 已经把新版 AICC 路由定义为“Provider inventory + logical_mounts + 多候选调度”的模型。当前代码里也已经有一部分新版基础结构：

- `src/frame/aicc/src/model_types.rs` 已有 `ProviderInventory`、`ModelMetadata`、`ModelCapabilities`、`ModelPricing`、`ModelHealth`、`ProviderType`。
- `src/frame/aicc/src/model_registry.rs` 已有 `ModelRegistry::apply_inventory()`，支持按 provider instance 全量替换 inventory，并从 `logical_mounts` 生成 default items。
- `src/frame/aicc/src/model_router.rs` / `model_scheduler.rs` 已有逻辑目录解析、硬过滤、fallback、session 粘性和 profile 调度的基础实现。

但现有运行中的 provider 接入仍主要走旧链路，这些旧链路在新开发分支中应直接移除或替换：

- provider trait 仍只有 `instance()`、`estimate_cost()`、`start()`、`cancel()`。
- 运行时 registry 仍保存 `ProviderInstance { instance_id, provider_type, capabilities, features, endpoint, plugin_key }`。
- provider 注册仍会直接写 `ModelCatalog` alias。
- 路由仍依赖 `ModelCatalog.resolve(capability, alias, provider_type)` 找 provider model。
- `openai.rs` 内部仍会 clear 全局 registry/catalog，和 `main.rs::apply_provider_settings()` 的统一 clear 重复且会破坏多 provider 常态。

所以阶段 2 的目标应是：把 provider 接入层直接迁移到已经存在的新版 inventory/metadata 结构上，并让实际注册与路由路径直接使用 `ModelRegistry + ModelRouter + ModelScheduler`。旧 `ModelCatalog` 不作为兼容层继续保留，provider 也不再负责写 alias。

## 2. 文档要求对照

来自 `doc/aicc/aicc_router.md` 的关键要求：

- Provider instance 以 `provider_instance_name` 为唯一 ID；精确模型名为 `<provider_model_id>@<provider_instance_name>`。
- Provider 通过 inventory/metadata 声明模型列表、`logical_mounts`、`capabilities`、`attributes`、`pricing`、`health`。
- inventory 是全量快照，每次成功返回完整替换该 provider instance 的模型列表。
- 同一逻辑模型名允许多个 provider instance 同时挂载，Registry 保留多候选。
- `local_only` 的可信判断必须基于 AICC 注册/系统配置确认的 `provider_type = local_inference`，不能只信 provider 自称 `local = true`。
- 动态成本估算应直接使用 `CostEstimateOutput`；inventory 的静态 `pricing` 只作为展示或 fallback。

## 3. 当前实现观察

### 3.1 新版结构已有但未贯通

`model_types.rs` 已定义：

- `ProviderInventory { provider_instance_name, provider_type, version, inventory_revision, models }`
- `ModelMetadata { provider_model_id, exact_model, api_types, logical_mounts, capabilities, attributes, pricing, health }`
- `ProviderType::{LocalInference, CloudApi, ProxyUnknown}`

`model_registry.rs` 已实现：

- `apply_inventory(inventory)`：按 `provider_instance_name` 替换快照。
- `default_items_for_path(logical_path)`：从所有 inventory 的 `logical_mounts` 生成 default items。
- 校验 exact model 必须属于 inventory 的 provider instance。
- 同一 logical mount 下多个 provider 会被保留。

这说明 provider 侧新增 inventory 能力时，应优先复用这些类型，而不是另起一套 metadata schema。

### 3.2 旧 provider trait 仍是实际接入入口

`src/frame/aicc/src/aicc.rs` 当前 provider trait：

```rust
pub trait Provider: Send + Sync {
    fn instance(&self) -> &ProviderInstance;
    fn estimate_cost(&self, req: &CompleteRequest, provider_model: &str) -> CostEstimate;
    async fn start(...);
    async fn cancel(...);
}
```

其中 `ProviderInstance` 仍是旧结构：

```rust
pub struct ProviderInstance {
    pub instance_id: String,
    pub provider_type: String,
    pub capabilities: Vec<Capability>,
    pub features: Vec<Feature>,
    pub endpoint: Option<String>,
    pub plugin_key: Option<String>,
}
```

它缺少新版文档要求的：

- `provider_instance_name` 和旧 `instance_id` 的命名统一关系。
- `provider_origin` / `provider_type_trusted_source` / revision。
- provider_type 的可信来源。
- 模型级 inventory/metadata。

### 3.3 provider 注册仍在写 ModelCatalog alias

`openai.rs`、`claude.rs`、`gimini.rs`、`minimax.rs` 都有 `register_default_aliases()` 和 `register_custom_aliases()`，直接写：

```rust
center.model_catalog().set_mapping(...)
```

这和新版要求冲突：新版 provider 注册不应写 alias，而应声明：

- models
- logical_mounts
- capabilities
- pricing
- health

逻辑目录 items 应由 Registry 根据 inventory 生成 default items，或者由 `SessionConfig` 显式配置覆盖。

### 3.4 OpenAI provider 内部 clear 需要移出

`src/frame/aicc/src/main.rs::apply_provider_settings()` 已经在开始处统一执行：

```rust
center.registry().clear();
center.model_catalog().clear();
```

但 `src/frame/aicc/src/openai.rs::register_openai_llm_providers()` 内部还会：

- settings 缺失或 disabled 时 clear registry/catalog。
- 注册前再次 clear registry/catalog。

这会导致多 provider 注册顺序中，OpenAI 的注册函数能清掉其它 provider 的注册结果。当前 `main.rs` 调用顺序是 OpenAI 先执行，问题暂时被顺序掩盖；但这不符合多 provider 常态化，后续一旦调整顺序或支持局部 reload，会直接出错。

## 4. 建议修改点

### P0. Provider trait 改为 inventory-first 接口

正式接口直接以 inventory 为 provider metadata 来源：

```rust
pub trait Provider: Send + Sync {
    fn inventory(&self) -> ProviderInventory;
    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput;
    async fn start(...);
    async fn cancel(...);
}
```

原因：

- 文档要求 inventory 是 provider 当前能力的声明式全量快照。
- 当前 provider 构造时已经有 `models/default_model/features/base_url/provider_type` 等配置，足够先生成静态快照。
- 新分支不需要保留旧 `instance()` 和旧 `estimate_cost()` 接口，避免 provider metadata 同时存在两套真相源。

后续再扩展：

```rust
async fn refresh_inventory(&self) -> Result<ProviderInventory, ProviderError>;
```

短期实现建议：

- `inventory()` 返回 `ProviderInventory`。
- `inventory_revision` 可先用配置 hash 或 `provider_instance_name + version` 的稳定字符串；如果暂时没有 hash，至少填一个可追踪的 revision。
- `exact_model` 统一由 `<provider_model_id>@<provider_instance_name>` 生成。
- provider runtime registry 从 `inventory.provider_instance_name` 建立执行入口，不再要求 provider 单独返回 `ProviderInstance`。

### P0. ProviderInstance 命名和类型语义一次性拆开

旧 `instance_id` 直接改名为 `provider_instance_name`，不保留 getter 或 alias。`provider_type` 明确表示可信部署类型，不再兼任厂商/驱动类型；厂商或实现类型另设字段。

```rust
pub struct ProviderInstance {
    pub provider_instance_name: String,
    pub provider_type: ProviderType,
    pub provider_driver: String,
    pub provider_origin: ProviderOrigin,
    pub provider_type_trusted_source: ProviderTypeTrustedSource,
    pub provider_type_revision: Option<String>,
    pub capabilities: Vec<Capability>,
    pub features: Vec<Feature>,
    pub endpoint: Option<String>,
    pub plugin_key: Option<String>,
}
```

字段决策：

- `provider_instance_name`：provider instance 唯一名，精确模型名后缀使用它。
- `provider_type`：使用 `model_types::ProviderType`，只表达 `local_inference`、`cloud_api`、`proxy_unknown` 这类可信部署类型。
- `provider_driver`：字符串，表达 OpenAI、Claude、Gemini、MiniMax、Ollama 等具体 provider 实现或厂商类型。旧 settings 里的 `"openai"`、`"claude"`、`"google-gimini"`、`"minimax"` 应迁移到这个字段。
- `provider_origin` 枚举建议至少区分 `SystemConfig`、`UserConfig`、`BuiltIn`、`ProviderClaimed`、`Unknown`。
- `provider_type_trusted_source` 用于 trace 和 `local_only` 硬过滤，例如 `system_config`、`admin_override`、`provider_inventory`、`default_unknown`。

关键原则：

- `provider_type = local_inference` 必须来自 AICC 注册过程或系统配置确认。
- Provider inventory 内的 `attributes.local = true` 只能展示或辅助，不能让候选通过 `local_only`。
- `proxy_unknown` 默认按非本地处理。

### P0. provider 注册不再写 ModelCatalog alias

目标注册流程应改为：

1. 构造 provider 实例。
2. `center.provider_registry().add_provider(provider)` 建立 provider 执行入口。
3. 调用 `provider.inventory()`。
4. 把 inventory 写入 `ModelRegistry::apply_inventory()`。
5. 由 `ModelRegistry` 根据 `logical_mounts` 生成 default items。

集成决策：

- `AIComputeCenter` 直接持有新版 `ModelRegistry`。
- 旧 `ModelCatalog` 从运行路径移除，不再生成 legacy alias。
- provider settings 中的 `alias_map`、`default_model` 如果还需要表达默认模型，应迁移为 global `SessionConfig` 的 logical tree/items，而不是 provider 注册副作用。
- `CompleteRequest.model.alias` 这类旧字段在本分支可直接迁移为新版 request model 字段；测试同步改，不做兼容解析。

### P0. OpenAI 内部 clear 移到 apply_provider_settings 统一管理

修改要求：

- `register_openai_llm_providers()` 不再调用 `center.registry().clear()`。
- `register_openai_llm_providers()` 不再调用 `center.model_registry().clear()`。
- provider disabled 时只返回 `Ok(0)`，不清全局状态。
- 全量 reload 的 clear 只保留在 `apply_provider_settings()`。
- `apply_provider_settings()` 统一清理 runtime provider registry、`ModelRegistry`、session/global route config 物化缓存；不再清理 `ModelCatalog`，因为该组件应退出运行路径。

后续如果支持单 provider reload，需要引入按 provider instance remove/replace：

- `registry.remove_instance(provider_instance_name)`
- `model_registry.remove_inventory(provider_instance_name)`

### P0. CostEstimate 直接替换为 CostEstimateOutput

当前 `CostEstimate` 只有：

```rust
pub struct CostEstimate {
    pub estimated_cost_usd: Option<f64>,
    pub estimated_latency_ms: Option<u64>,
}
```

文档长期要求：

```ts
interface CostEstimateOutput {
  estimated_cost_usd: number;
  pricing_mode: "per_token" | "subscription" | "free_quota" | "unknown";
  quota_state: "normal" | "near_limit" | "exhausted" | "unknown";
  confidence: number;
}
```

正式 Rust 结构建议：

```rust
pub struct CostEstimateInput {
    pub api_type: ApiType,
    pub exact_model: String,
    pub input_tokens: u64,
    pub estimated_output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub request_features: Vec<String>,
}

pub struct CostEstimateOutput {
    pub estimated_cost_usd: f64,
    pub pricing_mode: PricingMode,
    pub quota_state: QuotaState,
    pub confidence: f64,
    pub estimated_latency_ms: Option<u64>,
}
```

接口决策：

- 删除旧 `CostEstimate`，provider 只实现 `estimate_cost(&CostEstimateInput) -> CostEstimateOutput`。
- 成本评分只使用 `CostEstimateOutput`。
- inventory `pricing` 只做 UI 展示或动态 estimate 缺失时 fallback。
- 包月、免费额度、SN 免费额度等逻辑不能被压缩成简单 `estimated_cost_usd = 0`。
- 当 provider 无法可靠估算时，返回 `pricing_mode = Unknown`、`confidence` 低值，但仍需要给出可比较的 `estimated_cost_usd` 或由调度器按默认成本策略补齐。

### P1. 各 provider inventory 字段生成规则统一

各 provider 不应该各自随意决定 `logical_mounts` 和 `capabilities`。建议先定一组基础映射规则：

- LLM 模型：
  - `api_types`: `llm.chat`，需要时补 `llm.completion`。
  - `logical_mounts`: 至少挂到模型家族目录，例如 `llm.gpt5`、`llm.claude`、`llm.gemini`、`llm.minimax`。
  - 不建议直接挂到 `llm.plan`、`llm.code`、`llm.chat`，这些由 global session config 通过 items 配。
- Text2Image 模型：
  - `api_types`: `image.txt2image`。
  - `logical_mounts`: 例如 `image.txt2image.gpt_image` 或具体模型家族目录。
- capabilities：
  - 从 provider 支持能力和模型能力分开声明。
  - `tool_call`、`json_schema`、`vision`、`streaming`、`max_context_tokens` 尽量模型级填写。
- pricing：
  - 已有 provider 内部价格表时填静态价格字段。
  - 不确定时留空，不要虚构。
- health：
  - 启动时默认 `available/unknown quota`。
  - 运行时 metrics 或失败统计后续再回填。

## 5. 建议落地顺序

1. 在 `AIComputeCenter` 接入新版 `ModelRegistry`，并从运行路径移除 `ModelCatalog`。
2. 直接改 `Provider` trait：删除 `instance()` 和旧 `estimate_cost()`，新增 `inventory()` 与 `CostEstimateOutput` 版本成本估算。
3. 直接改 `ProviderInstance`/inventory 相关字段：`instance_id` 改为 `provider_instance_name`，`provider_type` 改为可信部署类型 enum，新增 `provider_driver`、`provider_origin`、`provider_type_trusted_source`。
4. 改 OpenAI/Claude/Gimini/MiniMax provider：注册时只 add provider + apply inventory，不写 alias。
5. 修掉 OpenAI 注册内部 clear，把全量 clear 收敛到 `apply_provider_settings()`。
6. 把实际 route 路径切到 `ModelRouter + ModelScheduler`，旧 `ModelCatalog.resolve()` 路径删除。
7. 用 global `SessionConfig` 表达默认角色目录和默认模型策略，替代 provider `default_model`/`alias_map` 副作用。

## 6. 已收敛决策与待确认点

已收敛决策：

- `ProviderInstance.instance_id` 本轮直接改名为 `provider_instance_name`。
- `provider_type` 直接使用 `model_types::ProviderType`，表示可信部署类型；厂商/实现类型另用 `provider_driver`。
- 旧 `ModelCatalog` 不继续保留，不做 alias 兼容层。
- provider 注册函数不再接触逻辑模型默认策略，只声明 inventory。
- 旧 `CostEstimate` 不保留，直接切到 `CostEstimateOutput`。
- 旧 tests 和旧 settings 格式同步迁移，不写兼容解析。

仍需实现前确认：

- global session config 的默认角色目录，例如 `llm.plan`、`llm.code`、`llm.chat`，首版默认权重怎么配。
- `inventory_revision` 首版生成规则：建议使用配置 hash；如果 provider 支持远端 inventory，则拼接 provider 返回 revision。
- SN/OpenAI 免费额度逻辑迁入 `CostEstimateOutput` 的字段表达方式：建议用 `pricing_mode = free_quota`，并在 trace extra 中记录 credit 使用情况。
- `provider_origin` 和 `provider_type_trusted_source` 的枚举值最终命名。

## 7. 验收 checklist

review 后进入实现阶段时，建议至少验证：

- [ ] `register_openai_llm_providers()` disabled 时不会清掉其它 provider。
- [ ] `apply_provider_settings()` 全量 reload 后只清一次 runtime provider registry 和 `ModelRegistry`。
- [ ] 每个 provider 都能返回 `ProviderInventory`。
- [ ] 同一 logical mount 下两个 provider instance 能同时出现在 `ModelRegistry::default_items_for_path()`。
- [ ] exact model 格式统一为 `<provider_model_id>@<provider_instance_name>`。
- [ ] provider instance name 包含 `@` 时会被拒绝。
- [ ] `local_only` 只信注册/系统配置确认的 provider type，不信 inventory 自称 local。
- [ ] provider 注册函数不写 alias，也不接触逻辑模型默认策略。
- [ ] 代码中不存在运行时依赖 `ModelCatalog.resolve()` 的路由路径。
- [ ] `estimate_cost()` 直接返回 `CostEstimateOutput`，且 quota/pricing_mode/confidence 有明确值。
- [ ] global `SessionConfig` 能表达 `llm.default`、`llm.plan`、`llm.code`、`llm.chat` 等默认逻辑目录。
- [ ] 现有 `cargo test` 通过。

## 8. 影响范围预估

主要代码入口：

- `src/frame/aicc/src/aicc.rs`
- `src/frame/aicc/src/main.rs`
- `src/frame/aicc/src/openai.rs`
- `src/frame/aicc/src/claude.rs`
- `src/frame/aicc/src/gimini.rs`
- `src/frame/aicc/src/minimax.rs`
- `src/frame/aicc/src/model_types.rs`
- `src/frame/aicc/src/model_registry.rs`
- `src/frame/aicc/src/model_router.rs`
- `src/frame/aicc/src/model_scheduler.rs`
- `src/frame/aicc/src/model_session.rs`

可能联动：

- `doc/aicc/how_to_add_provider.md`：新增 provider 指南需要从 alias 注册改成 inventory 声明。
- `doc/aicc/AICC.md`：旧 `ModelCatalog` alias 设计需要改为新版 inventory + logical tree。
- `src/frame/aicc/tests/*`：大量测试仍基于 `ModelCatalog` 和 string provider_type，需要直接迁移。
- `src/frame/desktop/src/app/ai-center/*`：如果 UI 展示 provider type、models、health，后续应对接 inventory/metadata。

## 9. 风险

- 一次性删除 `ModelCatalog` 会导致旧 RPC request 和旧测试全部需要同步迁移；这是本分支接受的破坏性变更。
- `provider_type` 当前同时承担“厂商类型”和“本地/云端类型”两种含义。新版必须拆成 `provider_driver` 和可信部署类型 `provider_type`，否则 `local_only` 无法做可靠硬过滤。
- settings 中旧 `"provider_type": "openai"` 这类字段需要改名，例如迁移为 `"provider_driver": "openai"`，同时新增可信 `"provider_type": "cloud_api"`。
- 静态 inventory 的 health/pricing 可能不准确，必须在 trace 或 UI 中区分静态声明和动态估算。
