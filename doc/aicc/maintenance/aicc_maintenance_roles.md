# AICC 新模型适配维护角色与支持状态

本文用于回答一个具体运维问题：当某个模型厂商上线新模型，或者市场上出现新的有竞争力模型厂商时，BuckyOS / 商业服务商 / 产品用户 / 模型服务商分别应该怎样让 AICC 尽快支持它。

本文重点是运维维护动作：改哪个配置、期望得到什么效果、如何验收。只有在必须发布新版本时，才简要说明需要实现哪些 AICC 组件，不展开源码清单。

状态定义：

- **已实现**：代码或文档中已有明确机制，并且当前实现中可用。
- **规划中 / 未完全实现**：文档或代码中已有入口、字段、P1/P2 条目或明确说明，但不能按完整可用理解。
- **畅想**：未在当前文档或代码中出现，属于本文为了完整性补充的建议，不应视为近期规划。

重要边界：

- API Key、OAuth token、device credential 等私有授权材料属于用户私有数据，默认应保存在用户自己的 Zone / system_config / 本地运行环境中。
- BuckyOS 项目方不应保存普通用户自己的 API Key。
- 如果商业服务商提供托管 Key，那是服务商与用户之间的商业托管能力，必须单独明示，不属于 BuckyOS 开源默认机制。
- Provider 自声明的 `local`、`privacy` 等信息不能直接作为安全真相源；`local_inference` 这类安全语义必须由本地管理员或可信 system_config 确认。

## 0. 新模型适配的基本判断

上线新模型或新厂商时，先按下面顺序判断需要哪类维护：

1. **只是已有 Provider 新增模型，且协议兼容**：优先维护 metadata / routing 配置，不需要发版。目标是让 AICC 知道模型 ID、能力、成本估算、逻辑目录挂载和路由偏好。
2. **新厂商兼容 OpenAI-compatible 协议**：用户或服务商可以新增 Provider instance，配置 `base_url`、授权信息和 models；必要时再补 metadata override。通常不需要 BuckyOS 发版。
3. **新厂商协议不兼容现有 adapter**：需要 BuckyOS 项目方或商业服务商发布新版本，增加 Provider adapter，再配套发布 metadata 基线和验收用例。
4. **模型能力类型本身是新的**：例如新增 API type、输入输出形态或调度语义，通常需要发版更新 AICC 协议、inventory schema、路由策略和测试。

云更新配置按确定程度分成两个维护面：

- **模型事实 / 逻辑确定服务**：维护相对稳定、可由模型能力事实决定的信息，例如模型 ID、provider driver、支持的 `api_types`、上下文长度、多模态能力、是否支持 tool/function calling、是否支持 streaming、默认 `logical_mounts`、弃用状态、替代模型建议等。这部分变更应尽量可审计、可缓存、可随版本内置。
- **运营策略服务**：维护变化更频繁、带运营判断的信息，例如价格估算、套餐/免费额度/超额价格、健康度、可用性、推荐权重、限流、熔断、灰度、区域偏好、成本优先或质量优先策略等。这部分可以独立于模型事实更快更新，也应允许服务商提供自己的策略。

metadata 基线和运营策略都有两种分发形态：

- **云端更新**：面向已经安装并运行的用户。AICC 从配置的 HTTPS 发布源通过 NDN 验证链拉取 index、manifest 和不可变 provider metadata 对象，提交完整 activation 后生效；失败时保持最新有效 activation 或回退 builtin metadata。
- **随版本内置缓存更新**：面向新安装用户或离线安装场景。新版本应携带发布时最新的模型事实基线和默认运营策略基线，使新用户即使没有云端更新，也能直接体验发布时已知的新模型。

### 0.1 统一更新验收流程

无论更新通过云端配置分发，还是通过发布新版本分发，都应按同一套验收流程执行。区别只在于交付物不同：云端更新交付模型事实配置 / 运营策略配置 / provider settings，发版更新交付版本包和随版本内置缓存。

1. **准备更新内容**：明确本次更新的 provider、model、api type、逻辑目录、模型事实变更、运营策略变更、routing 变更、是否需要 adapter 发版。
2. **新增或更新测试用例**：每次支持新模型、新 Provider、新逻辑目录挂载或新 fallback 策略，都应补充对应用例；同时标记本次更新会影响的旧用例。
3. **测试环境配置更新**：先把云端配置或版本包发布到测试环境，不直接进入发布环境。
4. **测试环境相关用例验收**：先执行本次新增用例和相关旧用例，覆盖 inventory、metadata 解析、exact model、logical model、fallback、成本估算、禁用策略和错误返回。
5. **测试环境全量验收**：相关用例通过后，再执行 AICC 全量用例，确认没有破坏旧 Provider、旧模型和旧路由策略。
6. **发布环境上线**：测试环境全量通过后，才把云端配置或版本包发布到发布环境。
7. **发布环境相关用例验收**：上线后先执行本次更新相关用例，确认发布环境配置、授权、网络和实际 Provider 状态正确。
8. **发布环境全量验收**：最后执行一次 AICC 全量用例，作为本次更新完成的验收点。

测试用例命名应能支持“按需求找相关用例”。建议命名中包含：`aicc`、更新类型、provider driver、provider instance 或厂商名、model id 或 model family、api type、逻辑目录、场景。例如：

- `aicc.metadata.openai.gpt-4.1.logical-llm-chat`
- `aicc.provider.openai-compatible.new-provider.models-list`
- `aicc.route.llm-code.exact-model-fallback`
- `aicc.cost.openai.gpt-4.1.estimate`
- `aicc.regression.routing.global-exact-model-weights`

如果测试框架支持 tags，应同时维护 tags，例如 `provider:openai`、`model:gpt-4.1`、`api_type:llm.chat`、`logical:llm.chat`、`update:metadata`、`update:routing`。如果 tags 机制尚未落地，至少要用命名约定保证可以通过关键字筛选相关用例。

## 1. BuckyOS 项目方

BuckyOS 项目方维护公共协议、默认模型事实基线、默认运营策略基线、默认逻辑目录、官方可分发的模型信息，以及需要随版本发布的 AICC 能力。

### 1.1 metadata、能力、成本、健康度、逻辑目录挂载

#### 已实现

- **维护模型事实基线**：当已支持的厂商发布新模型时，项目方可以更新随版本携带的 driver metadata。应补充或修正模型 ID、`api_types`、`capabilities`、上下文长度、`logical_mounts`、是否弃用、替代模型建议等逻辑上较确定的信息。
- **维护默认运营策略基线**：项目方可以维护默认价格估算、估算延迟、基础健康度、默认推荐权重和 fallback 建议。这些信息确定性弱于模型事实，应允许云端策略或服务商策略覆盖。
- **维护云端发布内容**：同一次更新应同时更新新版本携带的 builtin metadata，并按 index + manifest + 不可变 provider 对象发布到可信源。客户端只通过完整 activation 使用云端内容，不接受手工写入旧 `etc/.../remote_cache/<driver>.json`；人工覆盖仍使用 `local/` 或 `system-config/`。
- **维护对应测试用例**：模型事实或运营策略更新都必须同步新增或更新验收用例，并明确会影响哪些旧用例。用例应覆盖新模型出现在 inventory、能力字段正确、逻辑目录挂载正确、成本/健康度/权重策略生效、fallback 行为正确。
- **期望效果**：AICC 重新加载后，新模型出现在 `models.list` 的 inventory 中；如果 metadata 配置了 `logical_mounts`，模型还应出现在对应逻辑目录下，例如 `llm.chat`、`llm.code`、`llm.plan`。
- **验收方法**：在测试环境更新模型事实配置和运营策略配置后触发 `reload_settings`，先跑本次新增和相关旧用例，再调用 `models.list` 检查模型 ID、能力字段、逻辑目录、成本/健康度字段和 route trace；相关用例通过后执行全量用例。发布环境上线后重复相关用例和全量用例。
- **保底回滚**：如果模型事实配置导致错误能力或错误挂载，应回滚事实配置；如果运营策略导致错误路由，应优先回滚策略配置。回滚后重新 `reload_settings`，确认 `models.list` 和 route trace 恢复预期。

#### 规划中 / 未完全实现

- **官方云端自动更新服务**：文档已提到 per-driver URL 拉取、cache TTL、签名验证、revision 回滚等方向，但当前不能按完整可用理解；模型事实服务和运营策略服务的独立分发、独立回滚也不能按完整实现理解。
- **metadata 签名和可信来源展示**：schema 中已有 `signature` 字段，但当前未强制校验。
- **动态成本、套餐、免费额度、超额价格**：当前已有成本估算和 quota 字段入口，但多数直连 Provider 仍主要依赖静态 metadata 或 adapter 本地估算，不能完整获取真实套餐与余额。
- **真实健康度和熔断恢复**：inventory 和 route 中已有 health / quota / error 相关字段，但完整云端健康度采集、熔断、恢复策略不能按已完成理解。
- **产品化 Provider 管理 UI**：AI Center PRD / 原型中已有 Provider 管理、Usage / Balance、Routing UI、health / quota 展示等内容，但不能按当前完整产品能力理解。

#### 畅想

- 官方维护全球主流模型健康度、价格、推荐路由的实时服务。
- 官方提供模型能力认证或兼容性认证。
- 官方提供稳定的 BuckyOS Provider marketplace。
- 官方作为公共 metadata 信任根，支持第三方 metadata 签名、撤销和审计。
- 官方维护模型弃用 / 替代建议库，辅助逻辑目录自动迁移。

### 1.2 版本发布：新协议、新能力、新 Provider adapter

#### 已实现

- **何时必须发版**：当新厂商不是 OpenAI-compatible，或者已有 adapter 无法正确调用新协议时，需要发布新版本；当新模型引入 AICC 尚不认识的新 API type / method / 输入输出形态时，也需要发布新版本。
- **发版组件清单**：需要实现或更新 Provider adapter、Provider settings 解析、inventory 生成、成本估算、协议转换、错误映射、metadata 基线、默认逻辑目录挂载、route fallback 规则和验收测试。
- **随版本携带本地缓存**：版本包应包含发布时最新的模型事实基线、默认逻辑目录基线和默认运营策略基线。这样新安装用户在没有云端更新的情况下，也能识别该版本已支持的新模型，并获得可用的默认路由和成本/健康度策略。
- **随版本携带验收用例**：版本发布必须带上新增或更新的测试用例，并保证用例命名能识别 provider、model、api type、逻辑目录和更新类型，方便后续只跑相关用例。
- **期望效果**：升级版本后，用户只需配置 Provider 授权和必要的 `base_url`，AICC 即可从 Provider 获取模型列表，生成 inventory，并按逻辑目录完成路由。
- **验收方法**：先在测试环境安装版本包，用真实或 mock Provider 验证 `/models` 拉取、`models.list` 输出、exact model 调用、logical model 路由、fallback、错误返回和成本估算；相关用例通过后执行全量用例。发布环境上线后重复相关用例和全量用例。对新 API type 还应验证 helper / typed inference 调用链。

#### 规划中 / 未完全实现

- 新 API Type / method 的扩展流程已在文档中定义：每新增一个 method，需要同步 API schema、标准 method 集合、Provider inventory、Router fallback / policy、Provider adapter、task-manager 状态和测试。
- `aicc 逻辑模型目录.md` 中有 v0.4 待办：多语种 TTS、图像模型 tier、`agent_runtime` fallback 语义、多模态 any-to-any schema 收敛。
- P1 中还有 UI 模型树展示、Agent 多角色模型映射、应用侧 route overlay 组合基础设施、详细 score breakdown。
- Acceptance test 文档中列出了真实 Provider 矩阵、mock provider、L3/L4 验收等，但部分工具和测试链路仍是待实现或未完全迁移状态。

#### 畅想

- 官方为每类新能力提供独立认证测试套件和兼容性徽章。
- 官方提供长期维护的 Provider plugin API 和插件市场。
- 官方提供模型迁移助手，在厂商下线模型时自动生成版本升级建议。

### 1.3 BuckyOS 项目方不应维护的内容

#### 已实现 / 当前原则

- 普通用户 API Key 不应由 BuckyOS 官方云端保存。用户自己的授权配置应保存在自己的 Zone / system_config 中，主要是 `services/aicc/settings`。
- AICC usage log 不是最终财务账本；最终账务应由对应服务商或官方 SN 服务端负责。AICC 本地 usage log 只能作为本地记录和调试依据。
- `local_only` / `local_inference` 的安全判断不能只信 Provider inventory 自声明。是否可信应由本地管理员、产品服务商或可信 system_config 决定。

#### 规划中 / 未完全实现

- metadata 签名、来源展示、可信更新链还未完整落地。
- 更细的隐私、合规、密钥托管 UI 需要产品化实现。

#### 畅想

- 官方提供端到端密钥硬件保护和跨设备私钥托管方案。

## 2. 使用 BuckyOS 开源项目开发商业产品的服务商

服务商可以完全跟随 BuckyOS 项目更新，也可以为了稳定性、SLA 和商业能力提供自己的更新服务。

### 2.1 跟随 BuckyOS 项目更新

#### 已实现

- **接收 BuckyOS 的模型事实和运营策略基线**：服务商可以直接使用 BuckyOS 发布的 builtin metadata 和默认策略，也可以配置可信云更新源获取经过 NDN 验证并原子提交的 metadata activation，再叠加自己的运营策略。这适合只想跟随官方节奏、但仍希望控制产品默认路由的产品。
- **维护产品默认 Provider settings**：服务商可以预置或引导用户配置 `services/aicc/settings`，包括 Provider instance、`provider_driver`、`provider_type`、`base_url`、是否启用、models 列表等。
- **维护产品默认 routing_config**：服务商可以管理系统级 `services/aicc/settings.routing_config`，设置默认逻辑目录、Provider 权重、禁用列表、exact model 权重和 fallback 策略。
- **维护服务商相关用例集合**：服务商跟随 BuckyOS 更新时，应把本产品启用的 Provider、模型、逻辑目录和路由策略映射到测试用例命名或 tags 上，确保能筛选出本次更新相关用例。
- **期望效果**：产品用户配置自己的 API Key 后，就能看到服务商认可的新模型；逻辑模型调用会按服务商的默认策略路由到新模型或保留旧模型 fallback。
- **验收方法**：先在服务商测试环境写入 settings / routing_config，调用 `reload_settings` 后执行相关用例，再执行全量用例；测试环境通过后上线发布环境，并再次执行相关用例和全量用例。相关用例至少检查 Provider 是否启用、模型是否出现、逻辑目录是否符合预期、exact model 和 logical model 是否能成功返回。

#### 规划中 / 未完全实现

- AI Center PRD / 原型中已有更完整的 Provider 管理、Routing、Usage / Balance UI，但不能按当前完整产品能力理解。
- 更完整的 L4 真实 Provider 矩阵验收、动态模型矩阵生成仍需要产品化和持续维护。

#### 畅想

- 服务商提供上游 BuckyOS 版本兼容性认证报告。
- 服务商按行业场景维护“推荐 BuckyOS 版本 + 推荐模型栈”的组合包。

### 2.2 服务商自己的更新服务

#### 已实现

- **提供自营 Provider 网关**：如果服务商希望统一接入多个上游模型，可以提供 OpenAI-compatible endpoint，然后在产品侧把它配置成一个 Provider instance，维护自己的 `base_url`、授权策略和 models 列表。
- **发布服务商模型事实包**：服务商可以把自家确认过的模型能力、上下文长度、api type、逻辑挂载和弃用状态按云更新协议发布为 index、manifest 和不可变 provider 对象；仅供本 Zone 人工维护的覆盖可写入 `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/<driver>.json`。
- **发布服务商运营策略包**：服务商可以独立维护成本估算、额度策略、健康度、推荐权重、灰度和熔断策略。这类策略可以比模型事实更新更频繁，也应能单独回滚。
- **发布服务商默认路由策略**：服务商可以更新 `services/aicc/settings.routing_config`，例如让 `llm.chat` 优先走新模型，让 `llm.code` 保持旧模型，或为不同 Provider 设置权重。路由策略应优先引用模型事实中的逻辑目录，再叠加运营策略中的权重和健康度判断。
- **新增或更新服务商测试用例**：服务商自己的 Provider 网关、metadata 包和默认路由策略都应有对应测试用例。用例命名应能反映服务商网关、上游 Provider、模型族、逻辑目录、成本或 quota 场景。
- **期望效果**：服务商可以在不等待 BuckyOS 发版的情况下，把协议兼容的新模型推给产品用户；如果出现质量或稳定性问题，也可以通过 metadata / routing_config 回滚。
- **验收方法**：先在服务商测试环境发布模型事实包 / 运营策略包 / routing / provider settings，执行本次新增用例和相关旧用例，再执行全量用例；测试环境通过后再发布到正式环境，重复相关用例和全量用例。相关用例至少验证 Provider 列表、模型列表、逻辑目录挂载、exact model 调用、logical model 调用、fallback、禁用策略和错误返回。若服务商维护价格或额度字段，还应验证 route 输出中的成本估算和 quota 字段是否符合产品预期。

#### 规划中 / 未完全实现

- 官方发布源、发布工具和灰度运营流程仍需独立交付；AICC 客户端的定时同步、NDN 验证、增量下载、原子 activation、LKGS 与回滚保护已经实现。
- 租户级 quota、套餐、动态 cost estimate：AICC 文档已有 `CostEstimateOutput`、`quota_state`、P1 条目，但完整商业账务不是 AICC 当前完成项。
- 应用侧合成 app / agent / conversation overlay：AICC 明确只接收最终 `session_overlay`，文档列为 P1 的应用侧 overlay 组合基础设施。
- metadata 签名和可信分发链尚未完整实现。

#### 畅想

- 服务商建立自己的模型认证矩阵和 SLA。
- 服务商提供企业级模型市场、私有模型仓库、审计和合规报表。
- 服务商对不同客户提供定制模型路由策略包。
- 服务商维护跨厂商统一账务和成本优化服务。
- 服务商提供行业模板：代码助手、客服、文档总结、私有知识库、本地优先等。
- 服务商提供托管式 Provider 网关，并对模型能力、成本、健康度做统一治理。

### 2.3 服务商的授权与安全责任

#### 已实现 / 当前原则

- 用户自带 API Key 默认不应上传给 BuckyOS 官方云端。服务商如果只是分发 BuckyOS 产品，应让用户把 API Key 保存在自己的 Zone / system_config 中。
- 服务商可以在自己的产品中实现托管 Key，但这必须作为明确商业托管能力处理，并与 BuckyOS 开源默认机制区分。
- 服务商不能把不可信远程代理标成 `local_inference`。如果服务商提供托管网关，应明确标记其远程属性、隐私边界和数据处理责任。

#### 规划中 / 未完全实现

- 更完整的密钥托管 UI、来源标记、审计记录、租户隔离需要产品化实现。
- metadata 来源、签名、回滚、灰度策略需要服务商自行补齐，AICC 只提供部分基础结构。

#### 畅想

- 服务商提供密钥托管的硬件级保护、合规证明和跨区域灾备。

## 3. 产品用户

产品用户维护自己的 Provider 授权、路由偏好、预算、隐私和临时新模型接入。用户的核心目标是：在官方或服务商还没有发版适配前，尽量通过配置先把协议兼容的新模型用起来。

### 3.1 API Key 或其他授权策略

#### 已实现

- **新增或修改 Provider 授权**：用户可以在自己的 system_config / AICC settings 中维护 `services/aicc/settings`，配置 API Key、`base_url`、Provider instance、`provider_driver`、`provider_type`、启用状态和 models 列表。
- **使用 OpenAI-compatible 新厂商**：如果新厂商兼容 OpenAI-compatible 协议，用户可以新增一个 Provider instance，填入厂商提供的 `base_url` 和 API Key，然后调用 `reload_settings`。
- **使用官方 SN Provider**：`sn-ai-provider` 使用设备签名登录 SN，并缓存短期 `sn-sso` session，不使用普通 API Key，也不透传 BuckyOS `verify-hub` session。用户不应把自己的第三方 API Key 交给 BuckyOS 官方云端保存。
- **期望效果**：`models.list` 能看到该 Provider，并列出用户配置或 Provider `/models` 返回的新模型。
- **验收方法**：配置后调用 `reload_settings`，再调用 `models.list`。如果 Provider 不出现，优先检查启用状态、`provider_driver`、`base_url` 和授权；如果 Provider 出现但模型不出现，检查 models 列表或 Provider `/models` 兼容性。

#### 规划中 / 未完全实现

- AI Center UI 管理 Provider 授权、余额、Provider 状态属于 PRD / 原型内容，不能按当前完整实现理解。
- `quota.query` 查询额度和预算状态已有协议形态，但完整实现未完成。
- 用户级 metadata 信任管理和签名展示只能算预留，因为 metadata `signature` 字段已有，但校验未完成。

#### 畅想

- 用户一键导入第三方 Provider 包并完成授权。
- 用户在本地自动检测 API Key 权限、套餐类型和可用模型。
- 用户端提供授权风险评分，提示某 Provider 是否可信。

### 3.2 自定义路由策略

#### 已实现

- **指定新模型直接使用**：用户可以用 exact model 形式调用新模型，例如 `new-model@provider-instance`。这适合先验证新模型是否可用。
- **把新模型挂到逻辑目录**：用户可以修改系统级 `services/aicc/settings.routing_config`，把 exact model 挂到 `llm.chat`、`llm.code`、`llm.plan` 等逻辑目录，或调整 Provider 权重、禁用旧 Provider、设置 fallback。
- **临时覆盖路由**：用户可以通过 request `session_overlay` 做单次或应用侧临时路由偏好，不需要永久修改系统级路由。
- **补充本地 metadata**：如果新模型已经能调用，但能力、成本或逻辑目录识别不准确，用户可以添加 `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/<driver>.json` 或 system-config override，补充 `api_types`、`capabilities`、`logical_mounts`、成本估算等。
- **期望效果**：logical model 调用不需要写死新模型 ID，也能按配置路由到新模型；fallback 仍能在新模型不可用时回到旧模型。
- **验收方法**：调用 `models.list` 检查 exact model 和 `logical_mounts`；发起 logical model 请求，检查 route trace 是否命中新模型；禁用或降低新模型权重后再次调用，确认 fallback 符合预期。

#### 规划中 / 未完全实现

- per-user routing config 当前明确不支持；系统级 routing 配置持久化在 `services/aicc/settings.routing_config`，临时偏好走 request `session_overlay`。
- AI Center UI 管理模型目录、Routing、Usage / Balance 属于 PRD / 原型内容。
- Agent 多角色模型映射列为 P1，当前可以通过 overlay 表达，但完整 UI / SDK 基础设施不能按完成理解。
- 详细 score breakdown 列为 P1。

#### 畅想

- 用户可视化 route trace 和成本模拟。
- 用户自定义模型角色模板，如 coding、summary、private-local。
- 用户对不同 Agent / App 使用图形化策略编排。
- 用户在本地自动比较多个模型的质量、速度和成本，然后生成推荐路由。

### 3.3 官方未适配前的保底方法

#### 已实现

- **保底方法一：协议兼容时新增 Provider instance**。如果新厂商提供 OpenAI-compatible endpoint，用户配置 `base_url`、API Key、`provider_driver`、`provider_type` 和 models，`reload_settings` 后即可尝试使用。
- **保底方法二：直接用 exact model 调用**。如果只是已支持 Provider 发布新模型，且 adapter 能调用该模型，用户可以先用 `new-model@provider-instance` 直接测试，不必等待逻辑目录挂载。
- **保底方法三：写 local metadata override**。当新模型可调用但 AICC 不知道它的能力或逻辑目录时，用户写本地 metadata override，补充 `api_types`、`capabilities`、`logical_mounts`。
- **保底方法四：写 routing_config**。用户可以把新模型临时挂到逻辑目录，并保留旧模型 fallback。这样应用侧继续使用 `llm.chat` 等逻辑名，不需要立刻改业务代码。
- **验收方法**：先 exact model 调通，再 `models.list` 看 inventory 和逻辑挂载，最后用 logical model 调用验证路由。验收顺序应从“能调用”到“能被发现”再到“能被逻辑路由选中”。

#### 规划中 / 未完全实现

- 对非 OpenAI-compatible 的全新 Provider，通常仍需要 adapter 代码支持，用户无法只靠配置完整接入。
- metadata 签名和第三方包可信管理未完整实现。
- 对新模型自动测试能力并生成 override 目前没有明确已实现机制。

#### 畅想

- 一键测试新模型能力并生成 local override。
- 一键导入第三方 Provider 包。
- 用户端自动推荐新模型应挂载到哪些逻辑目录。

## 4. 模型服务商 / 中间商

目前模型服务商 / 中间商一般不会对 BuckyOS 提供定制服务。乐观情况下，如果 BuckyOS 成为有影响力的项目，它们可能主动适配 BuckyOS。由于当前没有明确实现或近期规划，本节整体按“畅想”处理。

### 4.1 提供 BuckyOS 兼容接口

#### 已实现

- 无。当前通常是 BuckyOS、商业服务商或用户通过已有厂商协议或 OpenAI-compatible 协议主动适配模型服务商，而不是模型服务商主动提供 BuckyOS 专用接口。

#### 规划中 / 未完全实现

- 无明确项目内规划。

#### 畅想

- 提供 BuckyOS AICC-compatible API。
- `/models` 直接返回 AICC `ProviderInventory`，包含模型 ID、能力、成本、健康度、quota、逻辑目录建议和版本号。
- 提供 BuckyOS 专用接入文档、mock endpoint 和测试 key。

### 4.2 维护 BuckyOS metadata 和 inventory

#### 已实现

- 无。当前主要由 BuckyOS 项目方、商业服务商或用户侧 metadata override 维护；模型服务商没有已实现的主动维护通道。

#### 规划中 / 未完全实现

- 无明确项目内规划。

#### 畅想

- 主动维护 BuckyOS driver metadata，并在发布新模型、下线旧模型、调整价格时同步更新。
- 提供 `inventory_revision`、全量 replace、`inventory_changed` 事件。
- 提供模型弃用 / 替代建议，让 AICC 自动迁移逻辑挂点。
- 提供 metadata 签名、版本、回滚。

### 4.3 提供动态成本、额度和健康信息

#### 已实现

- 无针对 BuckyOS 的模型服务商主动支持机制。AICC 当前能承接相关字段，但真实成本、额度、健康度主要仍由 BuckyOS adapter、本地配置或服务商自建网关转换后提供。

#### 规划中 / 未完全实现

- 无明确项目内规划。

#### 畅想

- 提供动态 `CostEstimateOutput`。
- 提供 BuckyOS 原生 quota、billing、health、route trace 扩展。
- 提供包月、免费额度、超额价格、缓存价格、阶梯价格等可供调度器使用的信息。
- 提供 BuckyOS 官方认证 Provider 插件。
