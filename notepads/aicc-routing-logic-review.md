# AICC 逻辑目录路由逻辑 Review

- 评审对象：`src/frame/aicc/src/routing/mod.rs`、`src/frame/aicc/src/model/mod.rs`（Registry 展开）、`src/frame/aicc/src/service/inference.rs`（运行时状态/成本估算）
- 对照设计：`doc/aicc/aicc_router.md` §6.2.1、§6.5、§10.3–10.4；`doc/aicc/aicc 逻辑模型目录.md` §3.1
- 日期：2026-09-25
- 评审出发点（期望语义）：**先选出胜出目录（权重最高、且目录下有可用实例），再在该目录内选成本最低的实例（`$modelid@$instanceid`）。**

## 0. 结论

当前实现**不是**“先选目录、再选实例”的两阶段算法，而是：

> 把整棵子树摊平成 exact model 候选列表 → 硬过滤 → 用一个多键比较器 `compare_ranked` 全局排序一次 → 取第一名，其余作为 runtime failover 列表。

目录权重通过“候选携带的路径权重向量 `priority_path` 做字典序比较”间接体现，所以**大多数简单场景结果与期望一致**，但有几处系统性偏差：

1. **LLM：成本不是在“胜出规格目录”内比较，而是在“同规格 + 同稳定性 + 同版本 + 同家族”内比较。** 规格内先选最新版本家族，再在该家族的实例里比价。这与设计文档 §6.2.1 一致，但和“目录内选最便宜”的直观理解不同。
2. **非 LLM：叶子 item 权重、`exact_model_weight` 都排在成本前面**，成本只作为 `final_score` 的一部分在最后参与，而 `final_score` 是多因素加权分。只要目录内实例的 item 权重不同，成本就不起作用。
3. **LLM 规格目录以下的所有 item 权重被忽略**；`canonical_quality` 排在目录权重之前；混合候选时比较器可能不满足全序。

建议：把排序改写成显式的**两阶段（目录选择 → 实例选择）**，并让 failover 列表也按“先同目录、再下一目录”生成。详见 §4。

## 1. 当前实现流程

### 1.1 展开（Registry）

`ModelRegistry::expand` / `expand_path`（`model/mod.rs:1208`、`:1238`）：

- 从请求路径递归展开 items；指向目录的 item 继续展开，指向 `x@y` 的 item 成为叶子候选。
- 每条到达叶子的路径记录 `CandidatePath.priority: Vec<f64>`，第 i 个元素 = `logical_paths[i]` 这一层所选 item 的 weight。**最后一个元素是指向叶子的 item 的权重。**
- 同一 exact model 经多条路径到达时按 exact model 去重，`paths` 保留全部路径。
- `item.weight == 0` 与 `exact_model_weight == 0` 的在此处直接剔除。
- LLM 家族候选额外带 `llm_order`（`model/mod.rs:826`）：`spec_weight`（指向 `llm.<spec>` 的那个 item 的权重）、spec、stability、version、family。

`resolve_candidates`（`model/mod.rs:481`）若展开后为空（**admission 层面**为空，与运行时可用性无关），按 Registry 的 fallback 规则换目录。

### 1.2 过滤（Router）

`evaluate_candidates`（`routing/mod.rs:587`）对每个候选做：model 硬过滤（min_line/disable_line/operation）、experimental 开关、`hard_filter_runtime`（enabled/凭证/模型可用/health/熔断/最大延迟）、策略层（隐私、信任、本地、成本上限、quota/budget）。

`route_logical`（`routing/mod.rs:449`）：**只有当前路径下候选被全部过滤时**才走 `next_fallback_target`（parent / target_logical / target_exact）。

### 1.3 排序（Router）

`finish`（`routing/mod.rs:669`）→ `score_candidates`（`:1030`）计算 `final_score` → `ranked.sort_by(compare_ranked)`（`:1168`）→ 取第一名；`fallback_candidates = ranked[1..]`。

`compare_ranked` 的实际比较顺序：

**A. 两个候选都有 `llm_order`（LLM 家族）**

| 顺序 | 键 | 方向 |
|---|---|---|
| 1 | `spec_weight` | 高优先 |
| 2 | spec id | 升序 |
| 3 | stability | Stable 优先 |
| 4 | version | 新优先 |
| 5 | family id | 升序 |
| 6 | `estimated_cost`（USD） | 低优先；有值优于无值 |
| 7 | p95 latency | 低优先；有值优于无值 |
| 8 | error_rate_5m | 低优先；有值优于无值 |
| 9 | canonical_quality | 高优先 |
| 10 | exact_model_weight | 高优先 |
| 11 | final_score | 低优先 |
| 12 | exact model 名 | 升序 |

**B. 其它情况（非 LLM，或任一方无 `llm_order`）**

| 顺序 | 键 | 方向 |
|---|---|---|
| 1 | canonical_quality | 高优先 |
| 2 | `priority_path`（多路径取最大者）逐层字典序 | 高优先 |
| 3 | exact_model_weight | 高优先 |
| 4 | final_score（cost/latency/reliability/quality/preference/cache/local 加权，越低越好） | 低优先 |
| 5 | exact model 名 | 升序 |

`final_score` 的权重由 scheduler profile 决定（`default_weights`，`routing/mod.rs:1010`）。默认 Balanced 下 cost 只占 0.25，CostFirst 下占 0.55。

## 2. 与期望语义（先目录、后最便宜实例）的差异

### 2.1 LLM：比价范围是“家族”，不是“目录”

规格目录下挂的是多个家族（如 `llm.gpt-pro` → `gpt-5-6-sol`(560)、`gpt-5-5-pro`(550)），家族下才是各 Provider 实例。比较器在第 4 步就按版本拉开，所以：

```text
llm.gpt-pro
├── gpt-5-6-sol  → a@p1 ($1.00)
└── gpt-5-5-pro  → b@p2 ($0.10)
```

结果是 `a@p1`。“目录内最便宜”只在**同一家族的多个 Provider 实例**之间成立。

> 这与设计文档一致（`aicc_router.md` §6.2.1：“规格内优先最新合格稳定版本，旧版兜底”）。**这里需要确认期望：规格目录内是“最新版本优先”，还是“最便宜优先”？** 如果是后者，需要改设计文档。

另外：设计 §10.4 写的是家族内“最后才比较同家族的实例权重/调度分”，但代码在家族内**先比价格、p95、错误率，再比 exact_model_weight**。也就是说，用户用 `exact_model_weights` 给某个 Provider 实例加权，在 LLM 家族内**会被价格压过**。这一点代码与设计不一致。

### 2.2 LLM：规格目录以下的 item 权重全部失效

`compare_ranked` 在双方都有 `llm_order` 时跳过 `compare_priority`（`routing/mod.rs:1223-1230`），只用 `spec_weight`（功能目录 → 规格目录那一跳的权重）。因此：

- 规格目录里指向家族的 item 权重、家族目录里指向实例的 item 权重，都不参与排序（版本号替代了规格内的权重，这符合设计）。
- 如果家族目录内想用 item weight 偏好某个 Provider（例如 `provider_a: 2.0, provider_b: 1.0`），**不生效**，只能靠 `exact_model_weights` / `provider_weights`，而它们又排在价格之后（见 2.1）。
- 如果请求直接指向规格目录本身（spec 在路径第 0 位），`spec_weight` 恒为 1.0（`checked_sub(1)` 失败）。单一规格内部不受影响，但语义上值得注明。
- 功能目录下若再嵌一层功能子目录（`llm.plan` → `llm.plan.deep` → `llm.gpt-pro`），只取紧挨规格的那一跳，上层权重被忽略。

### 2.3 非 LLM：叶子 item 权重和 exact_model_weight 都压过成本

`priority_path` 最后一位就是指向叶子的 item 权重，因此同一目录下的实例**先按 item 权重排**，再按 `exact_model_weight`，最后才是 `final_score`：

```yaml
image.generate:
  items:
    flux_a: { target: flux@p1, weight: 2.0 }   # $0.05
    flux_b: { target: flux@p2, weight: 1.0 }   # $0.01
```

结果是 `flux@p1`，成本不起作用。

> 这同样与设计文档 §10.4 一致（第 3–6 步：逐层取最高权重 → exact_model_weight → profile 评分）。设计本身把“目录内实例”也当作带权重的分支，与“目录内选最便宜”的理解不同。**这也需要确认：目录下直接挂的叶子 item 的 weight 算“目录选择”还是“实例偏好”？**

即使权重全部相同，`final_score` 也不等于成本：Balanced profile 下成本只占 0.25，延迟/可靠性/质量合计 0.65。另外 `normalize` 对 None 给 1.0（最差），所以**一个没有定价的实例在 cost 维度被当成最贵**。

### 2.4 `canonical_quality` 排在目录权重之前

B 分支的第一键是 `canonical_quality`（请求的 canonical_fields 与模型的匹配程度：Exact > Fuzzy > Default > Prompt > Unsupported）。如果请求或目录 min_line 带 canonical_fields，匹配更好的低权重目录实例会胜过高权重目录实例。设计文档没有规定这个顺序，需要确认是否符合预期：它应该是目录选择前的硬门槛、目录内的排序键，还是跨目录的最高键？

### 2.5 “目录可用”的判定口径有两层

- Registry 层 `resolve_candidates` 的 fallback 看的是 **admission 后是否为空**（静态能力）。
- Router 层 `route_logical` 的 fallback 看的是 **过滤后是否为空**（运行时 + 策略）。
- 同一请求路径内，高权重兄弟目录下实例全挂时，不走 fallback，而是在全局排序里自然落到下一个兄弟目录。这符合“权重最高**且可用**”，但 trace 里没有“跳过了哪个目录、为什么”的目录级记录，只有逐实例的 filtered 列表。Review 时很难直接看出“胜出目录是谁”。

## 3. 实现层面的缺陷

| # | 问题 | 位置 | 影响 |
|---|---|---|---|
| D1 | **比较器可能非全序**：同一候选集里既有带 `llm_order` 的，也有不带的（如 LLM 树里直接挂了一个非家族 exact model，或功能目录直接挂实例），两两比较时用不同的键集合，可能违反传递性，`sort_by` 结果不可预测 | `routing/mod.rs:1168-1254` | 选中结果依赖输入顺序；Rust 新版 sort 遇到非全序可能 panic（1.81+ 的 sort 实现会检测并可能 panic） |
| D2 | **前缀路径更长者胜**：`compare_priority` 逐位相等后 `left.len().cmp(&right.len())`，`[2.0]` vs `[2.0, 0.1]` 选更深的。同一目录下“直挂实例(2.0)”与“子目录(2.0)”并列时，子目录永远赢 | `routing/mod.rs:964-972` | 与“同权重全部保留”（§10.4 第 3 步）不符 |
| D3 | **无定价被视为最贵 / 排后**：`normalize` 给 None 1.0；LLM 分支里有值优于无值。本地模型、未配价格的 Provider 会被系统性压后 | `routing/mod.rs:1126`、`:1188-1193` | 本地/免费实例若没写 pricing 反而不被选；应区分“免费=0”与“未知” |
| D4 | 成本只认 USD：`inference.rs` 会用 `catalog.cost_in_usd` 换算，**换算失败即为 None**，然后按 D3 处理 | `service/inference.rs:632-651` | 缺汇率的币种等同“未知成本” |
| D5 | LLM 家族内价格排在 `exact_model_weight` 之前 | `routing/mod.rs:1177-1198` | 与设计 §10.4 不一致；用户权重配置被价格覆盖 |
| D6 | `quality_score` 生产路径恒为 None，`cache_hit_probability` 恒为 None | `service/inference.rs:657-658` | `final_score` 里这两项对所有候选都是常数，不影响排序但会误导 trace 阅读 |
| D7 | trace 没有目录级决策记录 | `RoutingTrace` | 无法直接回答“为什么选了这个目录” |

## 4. 建议：显式两阶段路由

基于“先选胜出目录，再选目录内实例”的思路，建议把 `compare_ranked` 这个单一比较器拆成**两阶段、每阶段只有一组键**：

### 4.1 阶段一：目录选择（逐层下降）

```text
select_dir(node):
    groups = node.items 按 weight 分组，降序；weight==0 已剔除
    for group in groups:                          # 同权重为一组
        pool = []
        for item in group:
            if item 指向目录:  pool += select_dir(item.target)   # 递归，返回该子目录的胜出实例池
            else:             pool += [item.target] if 可用
        if pool 非空: return pool                 # 第一个有可用实例的权重组胜出
    return []
```

- “可用” = 通过 §1.2 的全部硬过滤；判定在目录选择之前做完。
- 同权重兄弟**合并成一个池**，不按 ID、不按路径深度拉开（修 D2）。
- LLM：功能目录 → 规格目录这一层照常按 weight 选；规格目录 → 家族这一层**用版本值当 weight**（这正是设计里 `560/550` 的含义），stable/experimental 作为分组前置条件。这样 LLM 与非 LLM 走同一个算法，`llm_order` 的特殊比较器可以删掉（修 D1）。
- 递归下降到叶子后，得到的实例池就是“胜出目录”的实例集合。trace 里逐层记录“本层候选分支、各自权重、跳过原因、胜出分支”（修 D7）。

### 4.2 阶段二：目录内实例选择

对阶段一返回的实例池排序：

1. `exact_model_weight` / `provider_weights`：高优先（用户显式偏好应能覆盖成本，修 D5）。**待定：** 是否要求严格“最便宜优先”，把这一步挪到成本之后。
2. `estimated_cost`：低优先；**免费（0）与未知分开**，未知排在已知之后但不当作最贵参与归一化（修 D3）。
3. p95 latency → error rate → 实例名，作为稳定的 tie-break。
4. 可选：保留 scheduler profile，但只在**实例池内部**生效，不能跨目录翻盘。

### 4.3 failover 列表

`fallback_candidates` 按“同池剩余实例（阶段二顺序）→ 阶段一下一个权重组的池 → …”生成，与当前“全局排序的剩余部分”在简单场景下等价，但顺序由目录结构决定，更容易解释。

### 4.4 需要你拍板的问题

1. **LLM 规格目录内：最新版本优先，还是最便宜优先？** 现设计与代码都是最新版本优先（2.1）。
2. **目录下直挂实例的 item weight 算什么？** 算“目录选择”（参与阶段一分组，现状）还是“实例偏好”（并入阶段二，排在成本前或后）？
3. **`exact_model_weights` / `provider_weights` 与成本谁先？** 设计说权重先；代码在 LLM 家族内是价格先。
4. **`canonical_quality` 放哪一层？** 建议作为阶段二的第一键（目录内挑匹配最好的），不跨目录翻盘；如需硬性要求应走 min_line。
5. **scheduler profile 是否保留？** 如果目标是“目录内最便宜”，CostFirst 之外的 profile 实际上会改变语义。建议 profile 只在阶段二生效。

## 5. 建议补的测试

现有测试多是单一目录或单一家族，建议补：

- 同权重兄弟：一个直挂实例 + 一个子目录（覆盖 D2）。
- LLM 树中混挂非家族 exact model（覆盖 D1）。
- 高权重目录实例全部 unhealthy → 选到下一目录，并检查 trace 的目录级记录。
- 同目录两个实例：一个无定价、一个有定价；一个 `pricing.estimated_cost = 0` 的本地实例（覆盖 D3）。
- LLM 家族内 `exact_model_weights` 偏好较贵实例（覆盖 D5，明确期望结果）。
- 非 USD 定价、缺汇率（覆盖 D4）。
