阶段 1：纯新模块，可独立验证
建议新增这些文件，先不接 AIComputeCenter.complete：

src/frame/aicc/src/model_types.rs
定义 ExactModelName、ApiType、ProviderInventory、ModelMetadata、ModelCandidate、RoutePolicy、RouteTrace、错误码枚举。
单测覆盖：最后一个 @ 解析、非法 provider instance、api_type/capability 匹配、serde fixture。

src/frame/aicc/src/model_registry.rs
维护 inventory snapshot，生成 exact model 索引和 logical mount default items。
单测覆盖：同一 logical mount 多 provider 保留、同 provider exact model 重复拒绝、inventory revision 替换、default items 纯函数生成。

src/frame/aicc/src/model_session.rs
实现 SessionConfig、逻辑目录树、items/item_overrides、继承合并、policy lock、revision、TTL 内存状态机。
单测覆盖：items 覆盖 default items、item_overrides patch、负权重拒绝、items 与 item_overrides 同时出现拒绝、revision conflict、expired revision。

src/frame/aicc/src/model_router.rs
实现纯路由：exact/logical 判断、逻辑目录展开、去重、权重优先级、fallback、硬过滤、trace。
单测覆盖：llm.plan -> llm.gpt5 -> gpt-5.2@openai_primary、逻辑树环、fallback 环、exact model 默认不 fallback、local_only fallback 后仍过滤。

src/frame/aicc/src/model_scheduler.rs
只做候选评分和 sticky binding 判断，不调用 provider。
单测覆盖：cost_first/latency_first/quality_first/local_first、session sticky 命中、绑定不可用后重新选择。


阶段 2：provider 修改需求整理，review 后再改
这一步只产出 review 文档或 checklist，不直接改 provider。

需要确认的 provider 改动点：

Provider trait 增加 inventory/metadata 能力，建议先是同步快照方法：fn inventory(&self) -> ProviderInventory，后续再加 async refresh。
ProviderInstance 需要区分 provider_instance_name、provider_type、provider_origin/provider_type_trusted_source，不能只信 provider 自称 local。
各 provider 注册时不再写 ModelCatalog alias；改为声明 models + logical_mounts + capabilities + pricing + health。
estimate_cost 长期应返回文档里的 CostEstimateOutput，短期可适配现有 CostEstimate。
provider 注册 clear 逻辑需要统一在 apply_provider_settings 做；当前 OpenAI 注册内部会 clear registry/catalog，见 openai.rs (line 2071)，这和多 provider 常态化不一致


阶段 3：系统集成