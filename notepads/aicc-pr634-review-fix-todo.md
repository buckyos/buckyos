# AICC PR #634 Review 修复 TODO

日期：2026-09-26。基于合入提交 `704e81b2`（Merge PR #634 `fix/624-aicc-provider-protocols`，对比基线 `ef1bb164`）的 Review。合入时 `cargo test -p aicc` 541 个测试全过，desktop `tsc` 干净；下列问题都没有被现有测试覆盖。

源码路径相对于 `src/frame/aicc/src/`，`providers/*.json` 相对于 `src/frame/aicc/driver_metadata/`。行号以 Review 时为准，动手前请重新定位。标 **[已核实]** 的条目已对照代码确认；其余来自分模块 Review，修之前需先确认。

完成状态（2026-09-26）：以下代码修复、元数据修正和回归测试已完成；勾选表示实现或核实完成，不表示真实 Provider 验收已执行。修复基于 `704e81b2` 工作区，未提交或部署。验证结果见 [本轮报告](../test/aicc_test/acceptance/ISSUE_624_TEST_REPORT.md)。

---

## P0：可用性 / 计费

- [x] **P0-1 一个旧 custom provider 让整个 AICC 起不来** [已核实]
  - `provider/builtin/registry.rs:220`：本 PR 新增检查，custom profile 只能用 `custom_provider_adapters` 白名单内的 adapter，否则报 "reserved for a built-in provider"。
  - `service/settings_runtime.rs:86-94`：`prepare` 对每个 provider 的 `resolve` 用 `?`，一个失败 → `Runtime::new` 失败、每次 settings reload 也失败。
  - 场景：用户之前保存过 `protocol_adapter_id = glm-chat / kimi-chat / deepseek-responses / openrouter-responses` 的 custom provider，升级后所有 provider 都不可用。云端元数据收紧白名单时同样会触发。
  - 修法：单个 provider 解析失败时禁用该 provider 并记告警（管理接口可见），不要让整个 runtime 失败。补一个"持久化了旧 adapter 的 custom provider 仍能启动"的测试。

- [x] **P0-2 HTTP 402 不再重试或切换 provider** [已核实]
  - `error/mod.rs:274`：402 从 `Transport` 改成 `ProviderRejected`，只有消息命中 `requires_candidate_change` 关键字才会换候选。
  - 场景：OpenRouter 402 "Insufficient credits" / "requires more credits" 匹配不到 → 请求直接失败，不降级到下一个 provider。
  - 修法：402 一律允许切换候选（不重试同一个），或补齐关键字；加测试。

- [x] **P0-3 只写了 `tiers` 的计价条目算不出费用** [已核实]
  - `execution/mod.rs:382-415` `PinnedPricingSnapshot::from_pricing` 只看顶层 `*_token` 和 `unit`，两者都没有时返回 `Ok(None)`，`tiers` 被忽略。
  - 受影响：`doubao.provider.json` 的 `doubao-seed-2-0-*`、`minimax.provider.json` 的 `MiniMax-M3` / `minimax-m3`、`qwen.provider.json` 约 35 条 qwen3.x / qwen3.5 / qwen3-max（含 ap-southeast-1）。
  - 修法：代码里把"有 token 维度的 tiers"视为 token 计费；同时在 `catalog/validation.rs` 拒绝"既无 token 价、又无 unit、tiers 也无效"的条目。

- [x] **P0-4 计价单位与 adapter 上报用量不匹配**
  - [x] 所有 adapter 都不设 `usage.video_seconds` → `unit: video_second` 全部失效（Gemini Veo、OpenAI sora-2、MiniMax H3/H3-Max、Qwen wan）。
  - [x] Seedream（`doubao_media.rs:~556`）、Qwen 图片（`qwen_media.rs:~199`）只报 `request_units`，但计价 `unit: image`。
  - [x] Seedance 按 `output_token` 计价，`decode_video_result`（`doubao_media.rs:~602`）只报 `request_units(1)`。
  - [x] `glm.provider.json` 的 `glm-tts`（两条）有 `amount: 0.0002` 但没有 `unit` [已核实]，应为 `"unit": "character"`。
  - 修法：adapter 补齐对应 usage 字段；加一个测试：每条 pricing 的计费维度必须是对应 operation 的 adapter 会上报的维度；`amount` 无 `unit` 在校验时报错。

## P1：媒体协议

- [x] **Doubao TTS mime 写死 `audio/mpeg`** [已核实]：`protocol/doubao_speech.rs:~230`，请求 wav / pcm / ogg_opus 时标错类型。按 `audio_format()` 结果映射 mime。
- [x] **Doubao TTS 用量丢失** [已核实]：`doubao_speech.rs:~200` 完成帧（code 20000000）先 `continue`，没读 `usage.text_words`，`characters` 永远 None。另核对请求头名 `x-control-request-usage-tokens`（文档为 `X-Control-Require-Usage-Tokens-Return`）。
- [x] **Doubao TTS 先校验 content-type 再看 HTTP 状态**：`doubao_speech.rs:151-169`，401/429/5xx 变成 "invalid content type"，重试/降级判断错误。
- [x] **任务失败丢掉 provider 错误详情** [已核实]：`doubao_media.rs:386-396`、`qwen_media.rs:269-278`、`minimax_media.rs:~438`、`glm_media.rs` Status 只返回 `Failed`，最终变成 `service/provider_execution.rs:1187` 的通用 "native Provider task reported failure"。内容审核（`OutputVideoSensitiveContentDetected`、`DataInspectionFailed`）被归为协议错误。参照 `openai_responses.rs:~2727` 返回带 provider message/code 的 `Err`。
- [x] **Seedream 把 `aspect_ratio` 当 `size` 发** [已核实]：`doubao_media.rs:290-291`，"16:9" → 400。需要换算成 `WxH` 或 1K/2K/4K。
- [x] **首次之后每 250ms 轮询**：新 codec 的 Status 只透传 `Retry-After`，`poll_after` 默认值只在 Submit 生效，`execution/mod.rs:~1404` 回退 250ms 且无截止时间。3 分钟视频约 700 次查询，易触发限流。Status 也要带上默认间隔；考虑加总超时。
- [x] **video `duration` 以浮点发出**：`doubao_media.rs:~486`、`qwen_media.rs:~361`（`glm_media.rs` 旧代码同样）发 `5.0`，Ark 要求整数。参照 MiniMax V2（`minimax_media.rs:~580`）取整。
- [x] **Qwen 图片**：`qwen_media.rs:386-393` n>1 时一张被审核拦截就整体失败（应保留成功的图）；`:306` `size` 透传，DashScope 要 `W*H`。
- [x] **MiniMax V2 视频没有映射 `cancelled`**：`minimax_media.rs:434-443`。
- [x] 待确认：`doubao_media.rs:453-455` extend 发送的 `operation` / `continuation_handle` 非 Ark 字段；`qwen_media.rs:363-368` 发 `ratio`（Wan 用 `size`）；`minimax_media.rs:~211` ASR language 放在 HTTP header；`qwen_media.rs:565-585` `ensure_success` 未带 `provider_code` / `http_status`。
- [x] 待真机确认：`glm_media.rs` Status 成功时要求 `id`，Zhipu `async-result` 可能只有 `request_id`，会导致 GLM 视频全部 invalid_response。

## P1：元数据

- [x] **不带 `region` 的 CNY 价格漏到默认 global 区域**：`glm.provider.json` `model_pricing[39..101]`、`minimax.provider.json` `model_pricing[12..22]` 是国内价格但没有 `region`，global（api.z.ai / 海外 MiniMax）会命中 CNY 价格。Kimi 默认已改 global（api.moonshot.ai），但只有 CNY 价格。补 region 或补 global USD 价格。
- [x] **`glm-4-32b-0414-128k` 用 USD**（`glm.provider.json:~1509`）：文件里唯一的 USD 条目、source 是 bigmodel.cn、无 region。
- [x] **MiniMax vision 标反**：`minimax.model.json` 给 M2.x 标 vision（`minimax.provider.json:~65` `MiniMax-M2*` 绑了 vision operation），M3 只标 llm；验收契约 `provider_protocol_contracts.json:644-646` 写的是只有 M3 支持 vision。确认后二选一改齐。
- [x] **待确认 URL**：MiniMax 国内域名 `api.minimaxi.com` → `api.minimax.cn`（`minimax.known-provider.json:~25`、`registry.rs:~1285`）；Doubao 默认切到 `/api/plan/v3` 但价格仍是按量付费，与 `doc/aicc/provider_pricing_sources.md` "plan 计价保持 unknown" 矛盾，已有按量 key 可能不可用。
- [x] **Doubao 自定义 `base_url` 只作用于 LLM**：`provider/inventory.rs:173-176` 总是先用 profile 的 `operation_base_urls`，图像/视频/TTS 绕过用户配置的代理或按量端点。
- [x] Qwen `qwen3.5-flash` / `qwen3.6-flash` 精确条目是平价，对应 `*` 模式是阶梯价，精确匹配优先 → 长上下文低估。

## P2：测试与报告

- [x] **e2e spec 必然失败** [已核实]：`src/frame/desktop/tests/e2e/pages/ai-center.spec.ts:25-26` 用了 Playwright 不存在的 `page.getByDisplayValue`；`:17` 仍找 `豆包（火山方舟）`，mock（`src/api/aicc_mgr.ts:132`）已改名 `Doubao (Volcengine Ark)`。另外 Doubao region 选项、Qwen Base URL 两个断言与 mock catalog 不符。把 `tests/e2e` 纳入 typecheck。
- [x] **MiniMax 音乐生成的 T1.5 覆盖被删**（`minimax.music-generation.v1` 契约及测试），runtime 仍路由该功能。恢复。
- [x] **`variantCells` 缺基础模型时 `throw` → `continue`**（`test/aicc_test/acceptance/run_t15_gateway.ts:~1905`），模型从 inventory 消失时测试仍绿。改回失败，或至少计入报告的失败项。
- [x] **"distinguish text-only and vision" 测试没有 vision 样本**（`acceptance.test.ts:~2355`）。
- [x] **`ISSUE_624_TEST_REPORT.md` 过期**：写于 `0d9ea34b`，之后 4 个 commit（约 3.3k 行）未重跑；引用的 `e8cd0607`、`ed84d1c4` 不在仓库。按合入代码重跑并更新。
- [x] `run_t1_gateway.ts:309-330` 的 `restart_via_docker` 用 runner 命名空间里的 PID 去 `--pid=host` kill，runner 在容器内时会杀错进程；且未写入 `aicc_acceptance.example.toml`。
- [x] 元数据测试缺口：currency 与 region 一致性、`amount` 无 `unit`、tiers-only 可计费、计价维度与 adapter usage 对应、vision 能力与验收契约一致。

## 次要

- [x] `usage.cost` 异常（负数、空 currency）现在会让已计费成功的响应失败：`protocol/openai_responses.rs:1307-1341`、`openai_chat_completions.rs:1071-1101`。计费不读该字段，改为忽略即可。
- [x] Claude/MiniMax 允许 provider option 里的 `temperature` / `top_p` / `stop_sequences` 覆盖 canonical 值且跳过校验（`claude_messages.rs:797-833`、`minimax_messages.rs:122-144`）。
- [x] `derived_responses.rs` 的 buffered SSE 分支没做 provider namespace 改写（事件里仍是 `provider:"openai"`）。
- [x] 路由追踪的 `warning_count` 永远为 0：`storage/mod.rs:1184-1190` 读 `warnings`，`RoutingTrace`（`routing/mod.rs:146-179`）没有该字段；desktop "Warnings" 分段永远为空。
- [x] `ProtocolAdapterView`（`src/kernel/buckyos-api/src/aicc_client.rs:~4355`）带 `deny_unknown_fields`，新增 `custom_provider_selectable` 无 `serde(default)`，新旧版本互相解析失败。
- [x] `storage/mod.rs:1875-1900` 按 `system` 过滤 caller app 时不再匹配字面值 `'system'`，与 `caller_app_query` 不一致。
- [x] 向导（`WizardShell.tsx:~566`、`aicc_mgr.ts:~1201`）把 catalog 的 `operation_base_urls` 作为显式覆盖写进实例，之后云端修正的 URL 到不了已有实例。
- [x] `policy_region` 未按 `allowed_values` 校验、默认值未生效（目前无 `access_rules`，潜在问题）。


## 本轮处理结果

| 范围 | 落地与验证 |
|---|---|
| P0-1 | 单实例凭证、adapter、连接或启动错误被隔离；事件日志和管理 inventory 暴露失败原因。四种旧 custom adapter + 健康实例的启动、重载、再次收敛测试通过。 |
| P0-2 | 402 无条件切换候选，不重试同一个；补足 credits 消息匹配及回归测试。 |
| P0-3 | 顶层或 tiers token 单价均可钉住并结算；拒绝无计费维度、amount 无 unit 和 token/unit 混合条目。 |
| P0-4 | Seedream/Qwen 成功图片张数、Seedance completion tokens、Qwen/MiniMax/Sora 视频秒数、Veo 提交时钉住的秒数、TTS 字符数已接入结算。静态价格全遍历、可路由单位与 API 对应校验和实际响应解码计费测试通过。second/megapixel 仍按既有规则为 unknown。 |
| 媒体协议 | TTS 格式、完成帧用量、请求头、HTTP 错误优先级；原生任务错误码/消息、默认轮询间隔、单次驱动 30 分钟截止；整数时长、Seedream/Wan 尺寸、图片部分成功、MiniMax cancelled 均已修正。 |
| 待确认协议 | Ark 不再发送 operation/continuation_handle；Seedance 2.5 extension 使用官方字段。Wan 旧版按 size 下发，2.7 保留 ratio/resolution。MiniMax ASR language 在官方文档中确为请求头，保留。Qwen HTTP 错误保留 code/status。 |
| GLM 查询结果 | 官方 MetaGLM cookbook 的成功输出包含 `id=None` 和 request_id；状态解析不再要求 id，结果查询复用提交的 task ID。已补 mock 和 Rust 回归，未执行真机调用。 |
| 元数据 | CNY 限国内 region；删除无法核实的 glm-4-32b USD 价格；MiniMax M2 文本/M3 vision 对齐；Qwen flash 精确条目使用阶梯价。MiniMax 国内域名在合入代码中已是 api.minimax.cn，无须重复修改。 |
| Doubao 连接 | 默认恢复按量端点；plan 端点禁用按量价格。自定义 Base URL 覆盖继承的 operation 端点；显式覆盖优先。向导仅保存用户覆盖，并显示实际继承的 URL。 |
| 测试 | 修复并执行 Chromium e2e；全部 e2e 纳入 TypeScript 检查，连带修正两个既有 spec 类型错误。恢复 MiniMax 音乐契约、补 Qwen 媒体严格 mock 及对应三个模型身份/可路由测试；variant 基础模型缺失恢复报错，补 vision 样本。Docker PID 查找、校验、kill、轮询均使用同一 host namespace。 |
| 次要项 | 忽略无效可选 cost（有效 cost 仍参与实际费用）；canonical 采样参数优先且 provider option 校验；buffered SSE namespace 改写；trace warnings；ProtocolAdapterView 可选字段及未知字段；system 精确过滤；policy_region 默认与白名单校验。 |

验证：AICC 561 项通过；buckyos-api 215 项单元测试及 10 项集成测试通过；验收自测 87 项通过；preflight、协议 Mock 类型检查、桌面生产构建、AI Center Chromium e2e 和 `git diff --check` 通过。

边界：本轮没有部署当前 AICC、运行经 DV 网关的 T1/T1.5 或发出 T2/T3 真实调用。全量 Deno 检查仍受现有本地 WebSDK 类型推断错误影响，详情及复测命令见报告。30 分钟截止按每次原生任务驱动计时，进程重启后的恢复会重新计时。
