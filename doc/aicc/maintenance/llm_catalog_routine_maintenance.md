# AICC LLM 模型目录例行维护

本文是可重复执行的维护手册：核对主流厂商的型号、能力、价格与下线信息，更新 Model Driver / Provider Rules，验证默认用途覆盖，并留下记录。首次修复（2026-11-16）的过程与结论见 [driver_metadata_review_20261116.md](../driver_metadata_review_20261116.md)。

> **2026-11-27 路由变更**：目录之间没有 fallback，系统也不自动换模型。换模型只来自用途的“不可用时”策略（用户手工打开，[aicc_router.md §9](../aicc_router.md#9-fallback-与失败处理)）。`llm.fallback` 保留为兜底 LLM：默认关闭，管理员在用途表打开后，只为静态上没有模型的 LLM 用途兜底（[§9.5](../aicc_router.md#9-fallback-与失败处理)）。维护目录时：新规格放进合适的用途目录，或标记 `direct_only`；不要靠扩充 `llm.fallback` 提高用途覆盖率，用途覆盖以用途表（`routing.usecases`）为准。

触发时机：厂商发布新模型、调价、能力变化或下线时追加维护；没有变化也可按需复查。本文不创建定时任务。当前规则以 `src/frame/aicc` 源码为准，本文与源码冲突时更新本文。

## 1. 当前目录规则（维护前先确认仍成立）

入口在 `src/frame/aicc/src/service/model_defaults.rs` 头部契约与 `builtin_logical_tree_overlay()`。

- 结构：`用途（llm.chat 等）→ 规格（llm.{vendor}-{spec}）→ 家族:固定预设 → exact instance`。家族、规格、effort 绑定全部来自 Model Driver 的 `llm.spec` / `additional_specs`；用途表只给规格权重。
- 用途集合以 `builtin_logical_model_definitions()` 中 `api_type = llm` 的 `llm.*` 定义为准（当前 chat、plan、code、swift、summarize、translate、vision），`llm.fallback` 单独验收。用途的能力门槛：plan/code：tools + JSON Schema + 32K；summarize：16K；vision：视觉 + 32K。
- 目录之间没有回退：用途、规格、家族静态上没有合格候选就返回无候选，不回退到父目录或其他目录；规格、家族直选不升级到其他家族。
- 兜底 LLM：管理员层 `usecases."llm.fallback".enabled = true` 时，七个 LLM 用途中静态上没有候选的（只因上下文长度不够的不算）改用 `llm.fallback`。这一步在用途表求值（`routing/usecase.rs`）里做，注册表不含回退边；改道时只带请求自身的硬要求，不带原用途的门槛，用 `llm.fallback` 这一行的配置。默认关闭时这些用途直接报无候选。
- `llm.fallback` 只引用非 `direct_only` 规格，分四层：4.0 各系列廉价快速档、3.0 性价比档、2.0 标准档、1.0 高档与特化档。每个目录只展开第一个仍有合格候选的权重组；组内由 scheduler 排序（默认 `balanced`）。未知价格按最差成本处理，不按零。`direct_only` 规格不能出现在任何用途或 `llm.fallback` 中（`validate_llm_tree` 拒绝）。
- 边界：路由只用静态条件（库存、能力、启用、凭据、策略），健康、熔断、冷却不参与。选中的模型执行失败时不换模型；只有用途配置了“不可用时”策略才换一次，Provider 故障也不会转到 `llm.fallback`。

## 2. 每轮步骤

### 2.1 建立基线

1. 记录日期、`git log -1` 提交号、本轮范围（至少 OpenAI、Anthropic、Google Gemini；其余仓库已有厂商逐项标记“已检查 / 需要更新 / 本轮未覆盖 + 原因”）。
2. 列出现有 metadata：每个 `driver_metadata/models/*.model.json` 的规格、家族、`effort`/`default_effort`/`supported_efforts`、能力、`stability`、`weight`；每个 `providers/*.provider.json` 的 variants（effort lowering）、`capability_limits`、`model_pricing`。

### 2.2 核对官方变化

只采信原厂官方页面；第三方资料单独标注。至少查：

| 厂商 | 页面 |
| --- | --- |
| OpenAI | `developers.openai.com/api/docs/models`（含每个模型页、`/models/compare`）、`/api/docs/pricing`、`/api/docs/deprecations`、`/changelog/` |
| Anthropic | `platform.claude.com/docs/en/about-claude/models/overview`、各模型页、`/about-claude/pricing`、`/about-claude/model-deprecations`、`/release-notes/overview`、`/build-with-claude/effort` |
| Google | `ai.google.dev/gemini-api/docs/models`（现跳转到 `aistudio.google.com/docs/models`）、各模型页、`/pricing`、`/deprecations`、`/changelog`、`/thinking`、`/openai` |
| 其他 | Qwen `help.aliyun.com/zh/model-studio/models`（另查 `/model-pricing`、`/newly-released-models`、`/model-depreciation` 及各模型页；页面由 JS 渲染）、GLM `docs.bigmodel.cn` / `docs.z.ai`、DeepSeek `api-docs.deepseek.com/updates`、`/quick_start/pricing`（页面有 `.md` 导出）、Kimi `platform.kimi.com/docs/models`、`/docs/pricing/chat`、`/docs/api/models-overview`（国际站 `platform.kimi.ai`；价格表由 JS 填充）、MiniMax `platform.minimax.io/docs/faq/models`、`/docs/guides/pricing-paygo`（国内站 `platform.minimax.cn`，`platform.minimaxi.com` 已跳转）、Doubao `docs.volcengine.com/docs/ark/model-list`、`/model-pricing`、`/model-deprecation-notice`（Markdown 导出 `docs.volcengine.com/docs/zh/ark/<页面>.md`）、xAI `docs.x.ai/docs/models` |

每个结论记录准确 URL 和核对日期，分为：已知不变 / 新增 / 变更 / 下线 / 渠道未开放（受限访问、仅部分组织）。未声明的事实写“未知”，不从名称或通配规则推断能力。

### 2.3 更新配置

- **原厂事实**写 Model Driver：canonical ID、别名、稳定性、上下文与输出上限、输入模态、tools、结构化输出、`supported_efforts`、`default_effort`、所属规格及固定 `effort`、同规格内的版本 `weight`。
- **渠道事实**写 Provider Rules：operation、effort lowering（variants 的 `origin_model_id` 列表）、`capability_limits`、请求改写、`model_pricing`（`source_url`、`verified_at`、币种、输入/输出/缓存读写、长上下文 `tiers`）。只有 endpoint/adapter/发现方式变化时才改 Known Provider。
- 新型号先归入已有规格；只有厂商发布新产品线或新用途时才新增规格。新增规格应加入合适的用途，否则标记 `direct_only`。单模型覆盖测试如果还要求非 `direct_only` 规格出现在 `llm.fallback`，照加并在记录里写明。
- 低档预设只能通过合法的 `spec` / `additional_specs` 绑定表达；不要为降低兜底 LLM 的成本改写所有用途共用的规格 effort 或 `default_effort`，也不要伪造 `none`。
- 只被 swift、summarize、translate 和 `llm.fallback` 引用的廉价规格（如 `qwen-flash`、`gpt-mini`、`minimax-highspeed`）绑定 `none`/`low` 档：这些调用方的输出上限只有几千 token（会话压缩 6144），`xhigh`/`max` 档会把额度全部用在思考上、返回空正文。
- 下线：原厂停供只记录日期并调整对应原生渠道的价格/可用性；第三方或本地部署仍可提供时保留技术定义。停止默认推荐（调权重）与删除定义分开处理；退役规格前先清理用途、`llm.fallback` 和管理员 `routing` 配置里的引用。用途表各层选中的精确模型从库存消失时配置保留、这一格跟随下一层（管理员层的还会记 `usecase_model_missing` 事件），记录里写明受影响的用途。
- 未来日期生效的调价无法用 `time_windows`（按一天内时段/星期计价）表达：记录生效日期，到期后再更新价格并改 `verified_at`。
- 修改过的每个 catalog 文件 `revision_seq` 加 1；`src/frame/aicc/src/settings/mod.rs` 的 `BUILTIN_CATALOG_REVISION_SEQ` 加 1。同步更新断言固定计数/版本的单测（`provider_pricing_contains_all_rebased_model_defaults`、`every_builtin_provider_price_has_provenance`、各 provider 的 `revision_seq` 断言）。
- 同步独立验收契约：`test/aicc_test/acceptance/provider_protocol_contracts.json`（官方模型 ID 列表、variant 规则），必要时 `provider_capability_baseline.json`、`logical_entrypoint_baseline.json`、`route_exposure_contracts.json`。按官方文档手写，不从产品 metadata 生成。

### 2.4 兜底 LLM 选型

- 每个纳入范围的系列在 4.0 层保留其廉价快速规格；没有独立廉价档的系列（如 Kimi）以最低通用规格作为最低档并在记录中说明。
- 选型表列：系列、规格 → 当前首选家族:预设、适用渠道、费用口径（渠道、币种、输入/输出/缓存、阈值）、速度依据（官方定位或实测；没有数字就写“无官方延迟数字”）、后备顺序、来源、核对日期。原厂价不代表 SN/OpenRouter 等渠道的实际报价。
- 同一规格内按版本 `weight` 选最新家族，不按价格；最新版更贵时在记录中写明取舍。

### 2.5 验证

在 `buckyos/src/` 下：

```bash
cargo test -p aicc --lib -- --test-threads=1
```

重点用例：

| 用例 | 覆盖 |
| --- | --- |
| `service::tests::builtin_single_llm_serves_every_usecase_it_is_capable_of` | 每个内置 LLM 单独接入时，所有用途和 `llm.fallback` 要么有候选，要么有明确能力拒绝；`direct_only` 家族不泄漏 |
| `routing::tests::fallback_llm_serves_uncovered_llm_usecases_only_when_enabled` | 兜底 LLM：未打开时无候选即报错；打开后只为静态无候选的 LLM 用途提供，trace 带 `fallback.kind = coverage` |
| `service::tests::builtin_logical_tree_is_not_an_inventory_snapshot` | `model_defaults.rs` 注释树与实际目录逐项一致 |
| `provider::builtin::upgrade_tests::*` | 发现 → 身份匹配 → 预设/价格 → 路由的闭环（新增型号在此补用例） |

另外确认这几类用例仍在并通过（用例名随路由改造可能已变，按内容找）：`llm.fallback` 廉价档优先、廉价档缺失时用现存档、全部缺失时无候选；用途的能力门槛对直接展开有效、`direct_only` 不泄漏；规格、家族直选不升级到其他家族。

在 `buckyos/test/aicc_test/` 下：

```bash
pnpm install --frozen-lockfile
pnpm run acceptance:preflight
pnpm run acceptance:self-test
```

Windows 且 `core.autocrlf=true` 时，`doc/aicc/aicc_e2e_test_requirements.md` 以 CRLF 检出，preflight 的表格正则会报 “cannot find canonical API type table”。临时把该文件转成 LF 运行，结束后 `git checkout -- doc/aicc/aicc_e2e_test_requirements.md` 恢复。

T1 / T1.5 / T2 按 [验收 README](acceptance/README.md) 执行；需要 Zone 与真实上游，未执行时在记录中标为未验证，不能用 Mock 通过代替。

### 2.6 回读生效

- builtin metadata 在构建时嵌入二进制：需 `uv run buckyos-build.py -s aicc` 并按部署流程安装、重启后才生效。
- 有效来源按身份整文件选择 `system-config > local > cloud > builtin`，不逐模型合并。旧的 local / system-config 文件会整体遮住新的 builtin 文件：回读 AICC 的 metadata 来源视图确认实际来源与 revision；发现遮蔽时记录来源与差异，不直接清空用户配置。
- 回读用途表（`routing.usecases`，或 `buckyos ai routing`）确认每个用途的当前模型、来源和覆盖状态（已覆盖 / 由兜底 LLM 提供 / 未覆盖及原因）；需要看条目和权重时再读逻辑目录（model directory / route preview）与 `llm.fallback`；回读 inventory 的 `unmatched_models`、`unavailable_presets`。

### 2.7 记录

在 `doc/aicc/driver_metadata_review_<YYYYMMDD>.md` 记录：日期 / 提交、核对范围与官方结论表（含 URL）、新增 / 修改 / 退役项目、用途覆盖变化、兜底 LLM 选型表与依据、有效配置来源与 revision、验证命令与结果、未验证渠道、回退办法（回滚对应 JSON 与 revision）。无变更也记录已检查范围。

## 3. 消费方与手册

- Jarvis：`agent.toml` 默认 `llm.chat`；behaviors 使用 `llm.chat`、`llm.plan`（plan_simple）、`llm.code`（explore）；会话压缩使用 `llm.summarize`。目录规则变化后确认这些用途仍有候选。
- 手册：模型路由、“不可用时”策略或兜底 LLM 语义变化时检查 `src/apps/jarvis_agent/agent/cmd_skills/xllm/SKILL.md` 与 `opendan_agent_dev/SKILL.md`；涉及媒体能力再查 `aigc_*`。
