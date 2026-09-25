# AICC 逻辑目录路由逻辑 Review

- 评审对象：`src/frame/aicc/src/model/mod.rs`（Registry 展开）、`src/frame/aicc/src/routing/mod.rs`（实例选择）、`src/frame/aicc/src/service/inference.rs`（运行时状态、成本估算及 failover）
- 对照设计：`doc/aicc/aicc_router.md` §6.2.1、§6.5、§10.3–10.4；`doc/aicc/aicc 逻辑模型目录.md` §3.1
- 日期：2026-09-25
- 评审依据：以本次确认的“逐层最高权重展开 → 候选 instance 按策略选择”为目标语义。下文 §1–§6 为评审时的目标与差距分析；实施结果见文末 §7（2026-09-25 已实现）。

## 0. 结论

路由应分成两个阶段：

1. **展开候选列表**：每到一个目录，只展开该目录内权重最大的**可用** item；最大值并列时全部展开。进入每个子目录后独立重复同样的选择，直到得到 exact instance（`$modelid@$instanceid`）候选。最高权重组有可用候选时，低权重 item 不展开；最高组不可用（为空、未通过 admission、被硬过滤）时才尝试同层下一权重组，直到有候选为止；全部不可用则候选为空、路由失败（2026-09-25 确认，取代本文原先“为空也不尝试低权重”的表述）。
2. **选择 instance**：将展开得到、通过硬过滤的 instance 合并成一个候选池，按当前策略选择最合适的一个，通常成本优先。所有策略比较项相同时，按固定的默认顺序选择；延迟等易变运行指标不参与软排序或最终平局裁决。

这里的第一阶段不是“选出唯一胜出目录”，也不是“把整棵树展开以后再按路径权重排序”。**item 权重只在同一父节点下比较；不同分支里的子级权重、版本号不能跨分支比较。** 同权重分支不按规格 ID、家族 ID 提前淘汰，而是各自展开后共同进入阶段二。

**所有影响优先级的软性要素都必须通过配置表达为权重，用户能够查看和覆盖。** 版本、稳定性偏好、厂商或家族偏好在默认配置中体现为权重；其中版本对应的默认权重可以直接由 model-driver metadata 声明，例如本例的 `56/55`。Registry 将声明值写入对应 item，展开时只使用配置覆盖后的最终权重，不再解析版本或按其它 metadata 字段增加隐藏优先级。阶段二的成本等偏好也必须由公开的策略权重与规则表达，不能增加用户无法调整的比较键。两阶段的权重作用域保持分离；能力、权限、健康等硬门槛以及最终平局的默认顺序仍按各自规则处理。

当前实现不满足上述规则：Registry 展开所有非零权重 item；Router 再用 `llm_order` 或 `priority_path` 全局排序。原 Review 建议中的“高权重组为空就继续下一权重组”“p95 → error rate 作为稳定 tie-break”也不符合此次确认的语义，现予以修正。

## 1. 用示例明确目标语义

下面沿用本次讨论的名称与权重，仅表示路由结构，不是可直接提交给当前 Registry 的配置；家族下面的 Provider instance 暂时省略。

```text
llm.chat
├── gpt-mini (2.0)
│   ├── gpt-5.6 (56)
│   └── gpt-5.5 (55)
├── claude-opuse (2.0)
│   ├── opus-5.5 (55)
│   └── opus-4.8 (48)
└── gimini-flash (1.8)
    ├── gimini-3.1 (31)
    └── gimini-2.5 (25)
```

展开过程：

1. `llm.chat` 的最高权重是 `2.0`，保留 `gpt-mini` 和 `claude-opuse`；不进入 `gimini-flash`。
2. `gpt-mini` 内只展开 `gpt-5.6 (56)`；不展开 `gpt-5.5 (55)`。
3. `claude-opuse` 内只展开 `opus-5.5 (55)`；不展开 `opus-4.8 (48)`。
4. 两个家族继续按相同规则展开 Provider instance。若各家族的 instance item 权重相同，则它们全部进入候选池，再执行能力、运行状态及策略硬过滤。
5. 在 `gpt-5.6` 和 `opus-5.5` 的合格 instance 中按策略选择。成本优先且其它显式策略条件相同时，选成本最低者；成本及其它策略项都相同，则按默认文档顺序，示例中 `gpt-5.6` 在前。

**`56 > 55` 不会使 `gpt-5.6` 在阶段二天然胜出**：这两个数属于不同父节点。把路径 `[2.0, 56]` 和 `[2.0, 55]` 做字典序比较，会错误地排除跨分支比价。

这里的 `56/55`、`55/48`、`31/25` 已经给出了按版本安排的默认权重，定义可以直接来自各自的 model-driver metadata。同一系列内按这些声明值表达版本优先级即可，不需要设计跨厂商、跨系列统一且不冲突的版本编码。`gpt-5.5` 和 `opus-5.5` 都使用 `55` 没有冲突；路由只比较同一父节点下的最终权重，不负责从模型名推导这些数值。

若一个家族有多个 Provider，最终比较的是具体 instance 的成本，不是家族名称。比较单价需要统一币种与计费单位；输入/输出分别计费时，应使用同一请求用量口径估算成本。当前实现使用 `estimated_cost`（见 §2.3），不能直接比较不同计费单位的原始数值。

## 2. 当前实现核对

### 2.1 Registry 展开了全部非零权重分支

`ModelRegistry::expand` / `expand_path`（`model/mod.rs`）：

- 遍历 `node.items`，只跳过 `item.weight == 0`，没有在本层取最大权重。
- 目录引用继续递归，exact target 经 API、能力 admission、精确模型禁用检查后成为候选。
- 每条路径记录 `CandidatePath.priority`；最后一位是指向叶子的 item 权重。
- 按 exact model 去重，保留多条 `paths`；候选经 `BTreeMap` 汇集，之后又按 `llm_order` 排序。
- `resolve_candidates` 在 admission 后无候选时执行 Registry 的 fallback；此时尚未检查运行时可用性。

另一个关键细节：**当前 LLM 规格到家族的 item 权重并不是示例中的版本值。** `materialize_families` 将规格到家族、家族到 instance 的 item 权重都写为 `1.0`；`validate_llm_tree` 还要求规格到家族的权重为 `1.0`。版本顺序由 `LlmOrder.version` 单独表达。

因此仅在现有 `expand_path` 增加 `max(item.weight)` 不足以实现目标：规格到家族仍会全部同权展开。目标应直接读取 **model-driver metadata 声明的默认权重**，写入对应 item 后再合并用户配置。无需在路由代码中新增版本解析或版本到数值的转换规则，也不能临时计算一个用户无法修改的“有效版本权重”。

当前 `catalog/schema.rs` 的 `LlmSemantics` 尚无对应默认权重字段，`ModelVersion::from_model_id` 仍按厂商模型名解析版本；`driver_metadata_schema.md` 也描述了版本元组排序。这是当前实现与目标的差距，后续需同步 metadata 字段、读取与校验、内置 driver 文档，以及 Registry 的 item 构造；不能将上面的目标当作现有 schema 已支持的能力。

手工调整还受到两处校验阻碍：

- 规格到家族的 item 经 overlay 改成任何非 `1.0` 权重，都会被 `validate_llm_tree` 拒绝。
- 家族内 instance 即使只改权重，`apply_item_patches` 也会更新整个 item 的 `source`；随后 `validate_llm_tree` 要求来源必须为 `DriverMetadataMount`，因此用户/session 的权重覆盖也会被拒绝。

现有 `build` 已按“生成目录 → factory → system → user → session → 校验”处理，覆盖顺序可以复用。需要把成员身份/归属的事实来源与权重的配置来源分开校验，允许合法成员的权重被覆盖；不能用“成员来自 inventory”禁止用户修改调度偏好。

### 2.2 Router 过滤后全局排序

`route_logical` → `evaluate_candidates` → `finish`（`routing/mod.rs`）：

- 硬过滤覆盖能力、operation、experimental 权限、Provider enabled/凭证/可用性/health/熔断、配置的延迟上限，以及隐私、信任、本地、预算、quota 等条件。
- 只有当前路径的全部候选被过滤，才执行 `next_fallback_target`。
- `score_candidates` 计算综合分，`compare_ranked` 全局排序，第一名成为 selected，其余全部进入 `fallback_candidates`。

`compare_ranked` 的实际顺序：

| 候选类型 | 比较顺序（从前到后） |
|---|---|
| 双方都有 `llm_order` | 规格权重降序 → 规格 ID 升序 → 稳定性 → 版本降序 → 家族 ID 升序 → USD 估算成本升序（已知优先）→ p95 延迟 → 近期错误率 → canonical_quality → exact_model_weight → final_score → exact model 名 |
| 其它情况 | canonical_quality → `priority_path` 字典序降序 → exact_model_weight → final_score → exact model 名 |

其中：

- `llm_order` 只提取紧邻规格之前的那一跳权重；双方都有此字段时跳过完整的 `priority_path` 比较。
- `compare_priority` 在共同前缀相等时偏好更长路径。
- `provider_weight` 进入 `final_score` 的 preference 分量，不是与 `exact_model_weight` 相同的独立排序键。
- `final_score` 混合 cost、latency、reliability、quality、preference、cache、local。默认 CostFirst 的 cost 系数也只有 `0.55`，仍可能由其它项抵消，不能保证严格成本优先；Balanced 的 cost 系数为 `0.25`。

### 2.3 成本和默认顺序的现状

`service/inference.rs` 的运行状态构建会估算请求成本，再通过 `catalog.cost_in_usd` 换算成 USD；缺少价格或换算失败时为 `None`。LLM 比较器将已知成本排在未知之前；通用 `normalize` 将未知值记为 `1.0`。免费必须明确表示为 `0`，不能用“未定价”代替。

运行时 `quality_score`、`cache_hit_probability` 目前均为 `None`，但 p50/p95、近期错误率、近期失败次数与 degraded 状态确实会影响现有软排序。

公共 DTO `LogicalItems`（`buckyos-api/src/aicc_client.rs`）和 Registry 的 `EffectiveLogicalNode.items` 都使用 `BTreeMap`。它们按 key 排序，**不保留配置文档的书写顺序**；后续候选汇集与排序也不能还原该顺序。现有“最后按 exact model 名”虽然确定，但不等于用户要求的默认文档顺序。

## 3. 主要偏差与原 Review 修正

| # | 当前问题 | 对目标语义的影响 / 修正方向 |
|---|---|---|
| D1 | `expand_path` 展开全部非零权重 item | 低权重分支进入候选和 failover。应在每个父节点先取最大权重，只递归最高权重组。 |
| D2 | LLM 在成本前比较规格 ID、家族 ID、版本 | 同权重规格没有合并比价。ID 只能用于约定的最终稳定顺序，不能提前选定唯一规格或家族。版本只参与所在父节点的展开选择。 |
| D3 | 通用路由比较完整 `priority_path` | 错把不同分支内部权重放在一起比较；共同前缀相等时还偏好深路径。阶段二应移除路径权重比较。 |
| D4 | LLM 版本优先级独立于配置；规格到家族锁定权重 `1.0`，家族 instance 的权重覆盖又被来源校验拒绝 | 将 model-driver metadata 声明的默认权重写入可覆盖的 item；分离成员事实与偏好配置的校验，运行时只使用合并后的最终权重。 |
| D5 | p95、error rate 与动态综合分参与排序 | 同配置、同成本的合格 instance 会随运行观测抖动。易变指标不参与策略软排序或平局裁决。 |
| D6 | `BTreeMap` 和后续排序丢失文档顺序 | 需要在配置/生成目录、overlay、展开、去重的完整链路保存稳定顺序，最终平局沿用此顺序。 |
| D7 | `fallback_candidates = ranked[1..]` 包含低权重分支 | 首选失败后可能调用本不应展开的模型。默认只能保留本次最高权重展开池内的剩余 instance。 |
| D8 | 未知成本与免费成本的表达可能混淆 | 保留“免费 = 0，未知 = None”；成本策略应明确未知成本位置，不把缺价格当成免费。 |
| D9 | trace 只有实例过滤/排序，缺少逐层展开决策 | 无法直接说明某个分支为何没展开。应记录父节点、最大权重、选中/跳过 item、候选来源与最终稳定顺序。 |

原 Review 的两点判断也需要纠正：

- “叶子 item 权重压过成本”本身不是问题。**叶子 item 同样属于阶段一**；低权重叶子不进池，只有最高权重叶子才交给阶段二比价。问题在于当前通过全局排序模拟这一过程。
- `compare_ranked` 对“双方有 llm_order”和“至少一方没有”的候选使用不同键，存在比较关系不一致的条件性风险，但不能直接声称普通 LLM 配置可复现。当前 `validate_llm_tree` 禁止功能目录直接挂 exact model，也限制家族成员和规格引用；原 Review 用“LLM 功能目录混挂实例”作为现成例子不成立。重构后所有进入同一候选池的 instance 应使用同一组比较规则。

## 4. 目标算法与边界

### 4.0 配置权重是软性优先级的唯一来源

默认权重的读取、覆盖与消费顺序为：

```text
model-driver metadata 声明版本对应的默认权重；功能目录配置声明分支偏好权重
    → Registry 将声明值写入对应 item
    → factory / system / user / session 配置依次覆盖
    → 得到可查看的最终 item 权重及其来源
    → 按每层最大权重展开
    → 池内按公开配置的策略权重与规则选择 instance
    → 所有策略项相同才使用默认顺序
```

具体要求：

1. **版本偏好直接表达为 metadata 默认权重。** model-driver metadata 可以声明 `gpt-5.6 = 56`、`gpt-5.5 = 55`；Registry 原样采用权重值，路由层不解析模型名、不比较版本元组，也不另算一个版本权重。权重只需表达所在父节点内的偏好，不要求全局唯一。用户将后者改为 `60` 后，该规格必须展开 `gpt-5.5`；将二者都设为 `56` 时必须同时展开，设为 `0` 则禁用对应 item。
2. **其它软性偏好也归入配置权重。** 如果保留“稳定版更优”等默认偏好，应由 metadata/配置声明的权重体现，并允许用户覆盖。`allow_experimental` 表示能否准入；在许可且满足其它硬约束后，实验版不能再被隐藏的 stability 比较键压后。规格/家族/厂商名称不携带隐式优先级。
3. **手工覆盖优先于声明的默认值。** 用户可在原有 `item_overrides` 入口只改权重，无须修改模型名称、版本 metadata、归属或代码。metadata/inventory 刷新与 Registry 重建时重新应用覆盖；移除覆盖才恢复 metadata/配置当前声明的默认值。实例暂时消失期间，持久化覆盖不应丢失，也不能凭覆盖创造可执行实例。
4. **配置必须可解释。** 配置读写和 UI 应能查看默认权重、覆盖值、最终权重及来源，trace 记录实际使用值。不能只展示 `1.0`，内部却按版本或其它字段决定优先级。
5. **两阶段分别配置。** item 权重决定展开资格；成本等策略权重决定池内选择，不把价格混入目录版本权重，也不把路径权重跨分支相乘。策略项的权重、方向、组合规则及相对优先级必须公开且可调整；不允许另外叠加固定的版本、稳定性、canonical_quality 或实例偏好排序键。
6. **硬约束独立校验。** 能力、权限、固定 effort、模型归属和库存真实性等继续约束候选资格；提高权重不能绕过它们。权重本身仍须为有限非负数。最终平局顺序按 §4.3 保存，不用暗加小数权重破坏原本的并列关系。

沿用 §1 的语义名称，用户只需覆盖一项权重即可改变本层选择：

```yaml
llm.gpt-mini:
  item_overrides:
    gpt-5.5:
      weight: 60
```

这是目标语义示意；实际 item key 以生成目录为准，当前校验尚不支持上述调整。覆盖后，`gpt-mini` 分支展开 `gpt-5.5`，与另一分支的 `opus-5.5` 一起进入策略选择；用户权重 `60` 不与另一分支的 `55` 比较。

### 4.1 阶段一：按本层最大权重展开

以下为语义伪代码，省略已有的环检测、命名空间校验和 trace 参数：

```text
expand(node, inherited_constraints):
    items = 默认权重与配置覆盖合并后、符合所选预设的 items
    items = items 中 weight > 0 的项，保持默认文档顺序
    if items 为空:
        return []

    max_weight = max(item.weight for item in items)
    pool = []
    for item in items:
        if item.weight != max_weight:
            continue
        if item 指向目录:
            pool += expand(item.target, 累积后的约束)
        else if item.target 通过静态 admission 且未被禁用:
            pool += [item.target 及来源路径、默认顺序]
    return pool

paths = expand(requested_directory, request_constraints)
candidates = 按 exact instance 去重(paths)，保留首次合格路径的顺序和全部合格来源
allowed = 对 candidates 执行能力、运行状态、权限、预算等硬过滤
```

约束：

- 最大权重在**当前父节点**内求值，所有并列项均展开。子目录分别递归，结果合并；权重不相乘、不跨分支比、不因路径更深而优先。
- 目录引用与直挂 instance 使用相同规则。二者同权时都保留，之后按策略选 instance。
- 所选预设、API 类型、能力与禁用规则仍然生效，不能因权重高而绕过；同一 instance 只有经合格路径到达才可入池。
- 规格到家族的默认 item 权重直接取自 model-driver metadata 的权重声明，如示例中的 `56/55`，按 §4.0 接受配置覆盖后才参与展开。路由不再依赖现有版本元组或十进制版本编码；比较只发生在同一父节点内。
- 固定 effort、experimental 许可等准入约束需要保留。“稳定版优先”是可覆盖的默认权重偏好，不能伪装成硬过滤；“旧版兜底”如需保留，应通过明确的 fallback 表达。两者都不能隐藏在阶段二的 LLM 专用比较器里。
- **~~不循环尝试次高权重组~~（已由 2026-09-25 决定取代）。** 最高权重组的 item 不可用（为空、admission 不满足、被运行状态或策略硬过滤）时，不进入候选，继续尝试同层下一权重组，直到有候选为止；若所有 item 都不可用，则该目录候选为空（路由失败）。只要最高组中还有任一并列分支可用，就只在最高组的合格 instance 中选择。

因此“低权重不展开”只在最高组有可用候选时成立；可用性包含运行期硬过滤，所以硬过滤需要在展开时逐个叶子求值，而不是展开后再统一过滤。

### 4.2 阶段二：候选池内按策略选择

阶段二只看已经入池并通过硬过滤的 instance，不重新比较目录权重、路径深度、规格/家族优先级。

- 通常使用成本优先策略。通过公开配置的策略权重与组合规则表达成本优先；该配置若要求成本为首要项，就应保证按可比较的成本升序，不能使用可能被延迟、错误率等抵消的混合分。成本到策略分值的映射也须明确，不能藏在额外比较器中。
- `exact_model_weights` / `provider_weights` 的正值若作为显式实例偏好保留，属于阶段二的配置权重，应明确其与成本权重的组合及优先级，不能额外硬编码“实例权重一律覆盖成本”。精确模型权重为零的禁用语义仍需保留。
- `canonical_quality` 等非硬性偏好若保留，必须转换为可配置的策略权重/分量，不得作为独立固定比较键；硬性能力要求仍由 admission/min_line/request 约束处理。
- **p50/p95 延迟、近期错误率、近期失败次数等易变运行指标不进入软排序，也不作为 tie-break。** 健康状态、熔断、显式最大延迟等仍可作为硬性可用门槛；门槛改变候选资格与平局顺序抖动是不同问题。
- 所有策略键相等时按 §4.3 的默认顺序选择。

因此现有 `LatencyFirst`、Balanced、CostFirst 等 profile 及自定义分量不能原样接入目标算法，需要审查其动态评分项；保留“按当前策略选择”不等于保留当前所有评分公式。

对未知成本的处理建议明确为：已知成本优先，未知成本排后；免费按已知 `0` 处理。若全是未知成本，则按其它已声明策略项及默认顺序决定，不伪造价格。

### 4.3 默认顺序必须作为数据保留

按本次示例，默认顺序是文档/配置明确的 item 顺序，经深度优先展开形成稳定的 instance 顺序；相同策略下，先出现的合格 instance 优先。

落地时需满足：

1. 从源配置到 Registry 显式保留 item 顺序；不能依赖 JSON 对象键顺序或现有 `BTreeMap` 迭代结果来声称保留了书写顺序。具体采用顺序列表还是独立顺序字段，留给实现选择。
2. 动态生成的家族/instance 也需要确定且文档化的默认顺序；不能由 Provider discovery 返回先后、并发完成先后或运行时统计决定。
3. overlay 合并、去重、过滤均保留顺序。多路径到达同一 instance 时保留最早合格路径的顺序，trace 可保留全部来源；被过滤的路径不影响候选位置。
4. 将展开顺序传到阶段二；所有策略键相同才使用它。规格 ID、家族 ID、exact model 名不得在成本之前提前截断候选。

实现这一点会涉及公共 DTO、配置读写、前端编辑与展示，以及 Registry 的联动，不能仅将 `sort_by` 换成稳定排序：源顺序一旦丢失，稳定排序也无法恢复。

### 4.4 无候选与 failover

- 展开池只包含选中权重组的 instance；运行时 failover 列表默认只是**本池剩余 instance 按阶段二排序后的结果**，调用失败不会再把低权重分支加入执行列表。
- 无合格候选或本池耗尽时，根据允许的 fallback 规则处理；没有可用 fallback 则返回无候选/执行失败，不暗中展开旧版本或下一规格。
- 配置允许的 fallback 到逻辑目录后，对新目标重新执行两阶段算法；到 exact instance 则继续执行必要的硬过滤。仍保留原请求/原任务约束、fallback 权限、深度与环检测。
- LLM 无隐式 Parent fallback 的限制继续保留。若要支持“降到下一权重组”，需作为明确的 fallback 扩展记录与测试，而不是预先把这些实例放入正常候选列表。

## 5. 实现与设计文档需要同步的内容

以下文件已按本文同步（实施详情见 §7）：

| 位置 | 需要同步的内容 |
|---|---|
| `src/frame/aicc/src/catalog/schema.rs`、metadata 读取/校验及 `src/frame/aicc/driver_metadata/models/` | 支持并填写 model-driver 默认权重声明；不再从模型名解析版本来决定路由优先级。 |
| `src/frame/aicc/src/model/mod.rs` | 将 metadata 声明值写入默认 item 权重，复用 overlay 合并；分离成员事实与权重来源校验；逐层按最终权重展开；顺序传递与多路径去重。 |
| `src/frame/aicc/src/routing/mod.rs` | 统一按配置权重与规则进行池内策略比较；移除隐藏的路径/LLM 优先级及动态软排序；限制 failover 池；trace 展示实际权重及来源。 |
| `src/frame/aicc/src/service/inference.rs` | 核对成本口径及未知成本；执行层只接收本轮允许的 failover 候选。 |
| `src/kernel/buckyos-api/src/aicc_client.rs` 及对应配置/前端 | 保证权重可查看、可覆盖、可恢复默认；联动权重来源、顺序表达与策略配置的 DTO、序列化、编辑及展示。 |
| `doc/aicc/aicc_router.md` §6.2.1、§10.3–10.4 | 更新同权重规格合并、局部版本选择、动态评分移除、默认顺序、低权重不自动兜底等规则。 |
| `doc/aicc/aicc 逻辑模型目录.md` §3.1 | 更新“同权重按规格 ID 先选一个”“旧版兜底”“规格耗尽后尝试下一规格”等旧规则。 |
| `doc/aicc/driver_metadata_schema.md` | 定义默认权重字段与作用域，增加 `56/55` 这类声明示例，替换“解析官方 ID 并按版本元组排序”的路由契约。 |

这些旧设计与本次确认的语义存在差异，已在 §7 的实施中一并更新。

## 6. 建议验收项

验证重点既包括“最后选中了谁”，也包括“哪些分支根本没有展开”。

1. **完整示例**：只展开 `gpt-5.6` 与 `opus-5.5`；其它四个模型不进入候选或默认 failover。交换二者成本后应分别选中较便宜的一方；同价且其它策略项相同时选默认顺序靠前者。
2. **局部权重**：`[2.0, 56]` 与 `[2.0, 55]` 都进入池；同一父节点的 `56/55` 才会淘汰后者。不能使用路径字典序或乘积代替逐层选择。
3. **同权重全部展开**：同权重规格、同权重家族、同权重 Provider 均合并入池；直挂 instance 与子目录并列时不因路径深度改变资格。
4. **低权重按可用性展开**：最高权重组有可用候选时低权重分支不触发递归；最高组为空、全部 unhealthy、能力不满足、被预算过滤时才尝试同层下一权重组；所有组都不可用时返回无候选，有显式 fallback 则验证新目标及原约束保留。
5. **稳定顺序**：同策略、同成本时，改变 p95/error rate（均未触发硬门槛）不改变选择；重载配置、打乱 discovery 返回顺序仍保持默认顺序；显式调整文档顺序后平局结果随之改变。
6. **多路径去重**：同一 instance 经多个最高权重路径到达只出现一次，保留最早合格路径的顺序；被裁剪或 admission 拒绝的路径不参与决定顺序。
7. **成本策略**：免费 `0`、未知价格、非 USD 换算、缺汇率按既定规则处理；如果保留显式实例偏好，分别测试其与成本的优先级。
8. **运行时 failover**：首选执行失败后，只在本轮池内按同一策略顺序尝试剩余 instance；低权重分支不能因 `ranked[1..]` 自动进入执行列表。
9. **LLM 准入与统一比较**：固定 effort、experimental 权限和禁用仍有效；单独验证 metadata 默认权重的读取、校验及 item 构造。所有合格 instance 使用统一配置规则比较，覆盖传递性与平局确定性。
10. **手工权重覆盖**：将旧版权重从 `55` 改为 `60` 后只展开旧版；改成与新版相同则都展开，改成 `0` 则禁用。验证规格到家族、家族到 instance 两层都接受只改权重的 overlay，且不改变成员事实。
11. **配置持久性与可解释性**：metadata/inventory 刷新、Registry 重建和实例消失后恢复，均保留手工覆盖；移除覆盖恢复默认。配置视图与 trace 的最终权重一致，能区分默认来源及 factory/system/user/session 覆盖来源。
12. **无隐藏优先级**：最终权重相同时，版本、稳定性、厂商/家族名称不能提前淘汰候选；实验版许可开启后，提高其配置权重应生效，关闭许可时仍不得绕过准入。池内改变策略权重应按公开规则改变选择，配置之外的比较键不得覆盖结果。
13. **metadata 直接定义权重**：以 metadata 声明的 `56/55` 构造 item 并正确选择；修改声明值且无用户覆盖时，结果随权重变化。模型名无法解析版本仍可按声明权重参与路由；不同系列重复使用 `55` 不发生冲突，也不触发跨分支版本比较。

现有 `logical_route_honors_branch_weight_before_scheduler`、`llm_spec_preference_precedes_versions_and_preserves_instance_scheduling` 等测试明确期待低权重分支/旧版进入 failover，必须更新这些旧断言。`scheduler_profiles_choose_expected_candidate`、`degraded_provider_is_ranked_after_healthy_provider`、`same_family_fixed_effort_uses_converted_price_before_latency_and_unknowns` 等涉及动态评分或延迟顺序的测试也需要随目标语义审查。

## 7. 实施记录（2026-09-25）

实施按 §4 执行，并采用用户确认的可用性语义：**不可用的高权重 item 不进入候选，继续尝试同层下一权重组，直到有候选；全部不可用则路由失败。**

### 7.1 主要改动

| 位置 | 改动 |
|---|---|
| `buckyos-api/src/aicc_client.rs` | `LogicalItems` 改为有序列表 `Vec<LogicalItem{name,target,weight}>`，列表顺序即默认顺序；删除 `ModelItem`。`AiccSchedulerProfileWeights` 删除 `latency`、`reliability`。 |
| `catalog/schema.rs`、`validation.rs`、`catalog/mod.rs` | `llm.weight` 为必填有限非负数；删除 `ModelVersion`、`LlmModel.version` 及按模型名解析版本。 |
| `driver_metadata/models/*.model.json` | 93 个 LLM 条目补 `weight`，取官方版本 `major*10+minor`（如 gpt-5.6=56、opus-4-8=48、gemini-3.1=31），无版本号的 `emohaa` 为 1。未 bump `revision_seq`。 |
| `model/mod.rs` | item 改为有序 `Vec<EffectiveItem>`，记录 `default_weight`、`source`、`weight_source`。规格到家族默认权重取 `llm.weight`；删除规格权重必须为 1.0 的校验；家族成员校验改为“成员归属是事实（name==target 且属于 inventory 物化成员）”，允许只改权重。`item_overrides` 仅含 weight 且目标不存在时忽略（持久覆盖在实例消失期间不报错），缺少库存的家族上仅有覆盖时不再构建失败。`expand_path` 按节点求权重组：降序逐组展开，首个非空组即返回；叶子在展开时调用 Router 传入的可用性判定。候选按首次到达顺序去重。新增 `reachable_models`（全量可达，供 quota 预备）、`ExpansionStep` 记录。删除 `LlmOrder`，stability 只剩 `experimental` 准入标志。 |
| `routing/mod.rs` | `route_logical` 通过 `resolve_available_candidates` 把硬过滤（模型能力、实验版、运行状态、策略/预算/quota）下推到展开阶段；Registry 负责 fallback 链。阶段二统一为 `final_score` 升序 → 默认顺序；删除 `llm_order`、`priority_path`、p95/error rate 比较器。评分分量只剩 cost / quality（规范字段匹配质量）/ preference（exact_model_weight×provider_weight 与 session 历史）/ cache / local。默认权重：cost_first = cost 1.0；latency_first = cost 0.5 + local 0.5；quality_first = quality 0.7 + cost 0.3；balanced/strict_local = cost 0.5 + quality 0.3 + preference 0.2；local_first = local 0.7 + cost 0.3。trace 新增 `logical_expansion`（每目录选中权重、各 item 权重/来源/状态 expanded|unavailable|not_expanded），`ranked_candidates` 以 `default_order`、`item_weights` 取代 `priority_path`。 |
| `service/mod.rs`、`inference.rs`、`model_defaults.rs` | 目录 JSON 的 items 改为有序数组，含 `default_weight`、`source`、`weight_source`；模型页规格成员默认权重取 metadata。quota 预备改用 `reachable_models`。`CandidateRuntimeState.quality_score` 删除。 |
| `opendan/src/session_model.rs` | Session profile 镜像类型的 `items` 同步为有序列表。 |
| `desktop` AI Center | 解析数组形式的目录 items（保留顺序，带默认权重与来源）；trace 评分项删除 latency/reliability。 |
| `test/aicc_test` | overlay fixture 改为列表；failover 归因用例的 backup 权重与 primary 相同（否则按新语义不会进入 failover 列表）。 |
| 文档 | `aicc_router.md` §6.2–6.5、§10.1–10.7、§13.2、§15、`aicc 逻辑模型目录.md` §3.1/§四、`driver_metadata_schema.md`、`aicc-models-mgr.md`。 |

### 7.2 验证

- `cargo test -p aicc`：493 passed。新增/改写：`each_directory_expands_only_its_highest_available_weight_group`、`weight_only_overrides_keep_position_and_record_sources`、`duplicate_item_names_are_rejected`、`d04_metadata_weights_select_families_without_parsing_model_names`、`family_instance_weights_accept_overrides_but_not_new_members`、`logical_route_expands_only_the_highest_available_weight_group`、`equal_weight_branches_compare_cost_then_keep_document_order`、`volatile_runtime_metrics_do_not_change_equal_cost_selection`、`exact_model_weight_is_a_configurable_preference_not_an_override`、`llm_lower_weights_are_tried_only_when_higher_groups_are_unavailable`、`llm_stability_is_admission_only_and_fallback_keeps_requirements`。
- `cargo test -p buckyos-api`：217 passed；`cargo test -p opendan`：全部通过。
- `cargo check --workspace --tests --exclude scheduler` 通过；`scheduler/tests/sn_ai_routing.rs` 的语法错误是既有问题，与本次无关。
- desktop：`tsc -b` 与相关文件 eslint 通过；`test/aicc_test`：`deno check test_list_models.ts acceptance/mock_settings.ts` 通过（`run_t1_gateway.ts` 的 deno check 因依赖类型解析崩溃，未能验证）。

### 7.3 遗留与风险

- **未做 DV 验证**：未重新构建部署，也未在 DV 环境跑 `test/aicc_test` 验收。
- **配置格式 breaking**：已持久化在 `services/aicc/settings.routing_config` 中、使用旧 map 形式 `items` 的配置会解析失败，需要改写成列表。
- **latency_first 语义**：由于运行观测不再参与排序，`latency_first` 退化为“本地 + 成本”偏好，延迟要求只能通过 `max_latency_ms` 硬门槛表达；如需静态延迟偏好，应在 metadata 增加静态延迟字段后作为公开分量接入。
- **UI 未提供编辑**：Ai Center 目前只解析并携带 `default_weight`/`source`/`weight_source`，尚未展示“默认值/覆盖值/来源”，也没有编辑权重的入口。
- 同一目录被不同约束栈多次访问时，`logical_expansion` 只记录第一次的决策。

