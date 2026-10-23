# AICC 统一视频生成协议

状态：**Frozen / Beta 2.3**（2026-10-22，Issue #651 P0）

本文冻结 `video.generate` / `helper.video_generate` / `video.edit` 的公共语义、服务端能力声明、素材事实与执行边界。字段级真相源是
`src/kernel/buckyos-api/src/aicc_client.rs`；服务端能力合同与计划在 `src/frame/aicc/src/video/`，素材探测在 `src/frame/aicc/src/media/`。正反例 `src/kernel/buckyos-api/tests/fixtures/video_generate_cases.json` 由 Rust 与 WebSDK 共用（WebSDK 保存同一份副本）。

## 1. 方法与迁移

| method | ApiType | 用途 |
| --- | --- | --- |
| `video.generate` | `video.generate` | 固定 `exact_model` 的生成：文本、关键帧、参考图/视频/音频及其组合 |
| `helper.video_generate` | `video.generate` | 逻辑模型生成；与 `video.generate` 共用业务字段、校验与需求推导 |
| `video.edit` | `video.edit` | 有真实源视频编辑合同的模型：源视频 + 编辑指令 |
| `video.extend` | `video.extend` | 续接（保持独立职责，续接不等于参考生成） |
| `video.upscale` | `video.upscale` | 超分 |

旧方法 `video.txt2video`、`video.img2video`、`video.video2video` 及其 ApiType、DTO、SDK 方法、逻辑挂载已删除，调用返回 `UnknownMethod`，不保留 alias 或兼容 dispatcher。

旧 operation 迁移表：

| 旧接口 / 分支 | 新归宿 | 依据 |
| --- | --- | --- |
| 各 adapter `txt2video` | `video.generate`（无条件） | 生成语义 |
| 各 adapter `img2video`（首帧） | `video.generate` + `keyframe{position:start}` | 首帧是强约束；Omni Interactions 的“图片 + 文本”没有首帧合同，改为 `reference_image` |
| GLM `vidu*-start-end` | `keyframe start + end` | 官方首尾帧模型 |
| GLM `vidu2-reference` | `reference_image` | 官方参考生图模型 |
| Doubao `video2video`（2.0 系列，未指定任务类型） | `video.generate` + `reference_video` | 原实现只发送 `reference_video`，属于参考生成 |
| Doubao 2.5 `video2video`（`omni_reference_task_type=edit`） | `video.edit` | 官方编辑任务类型 |
| OpenAI `videos/edits` | `video.edit` | 官方编辑接口 |
| Gemini Omni Interactions video→video | `video.edit` | 输入源视频并改写 |
| Veo `predictLongRunning` video→video（`preserveMotion/timeRange`） | 撤下 | Gemini API 无对应合同，原字段是自造 |
| 旧 `preserve_motion` / `time_range`（video2video） | 撤下 | 没有任何 adapter 精确实现；片段由 `reference_video.time_range` 表达 |

## 2. `video.generate` 请求

```json
{
  "exact_model": "minimax/hailuo-3@openrouter-main",
  "prompt": "@{hero} 在雨夜奔跑，镜头跟随，结尾停在 @{door} 前",
  "negative_prompt": null,
  "conditions": [
    {"type": "reference_image", "id": "hero", "resource": {"kind": "named_object", "obj_id": "..."}},
    {"type": "reference_image", "id": "door", "resource": {"kind": "url", "url": "https://..."}}
  ],
  "output": {"duration_seconds": 6, "aspect_ratio": "16:9", "size": {"short_edge": 1440}, "audio": "auto"},
  "seed": null,
  "idempotency_key": "film-42-shot-3"
}
```

- `prompt?`、`negative_prompt?`、`conditions`（默认空）、`output?`、`seed?`，以及通用的 `trace_id/execution_mode/idempotency_key/task_options/session_id`。
- `prompt` 为空白（或缺失）且 `conditions` 为空时报 `invalid_request`。模型要求 prompt 时由模型能力再检查。
- 未知字段在顶层与所有嵌套对象拒绝。
- helper 另有 `logical_model`、`requirements`、`disable`、`policy`、`session_overlay`；`exact_model` 不出现在 helper 中。

### 2.1 条件 `conditions[]`

| type | 字段 | 说明 |
| --- | --- | --- |
| `reference_image` | `id`, `resource` | 外观/风格/角色参考 |
| `reference_video` | `id`, `resource`, `time_range?`, `include_audio`（默认 `false`） | 动作/镜头参考 |
| `reference_audio` | `id`, `resource`, `time_range?` | 声音参考 |
| `keyframe` | `id`, `resource`, `position: start \| end` | 强制首帧/尾帧 |

规则：

- `id` 匹配 `^[A-Za-z][A-Za-z0-9_-]{0,63}$`，大小写敏感，但按 ASCII 小写判重（`img` 与 `IMG` 视为重复），不随数组重排或厂商编号变化。
- 同一 `position` 至多一个 keyframe；重复报错。
- `time_range` 复用 `TimeRange`，语义为半开区间 `[start_seconds, end_seconds)`：有限、非负、`start < end`、`end` 不超过实测时长；与输出时长相互独立。
- `include_audio=false` 时保证目标模型拿不到源音轨：有音轨的 MP4/MOV 由 AICC 生成去音轨派生素材；无法去音轨（分片 MP4、未知容器）时报错。`include_audio=true` 时源视频必须有音轨，片段时长计入音频总时长限制。
- 片段裁剪当前无法精确完成（不引入转码依赖），`time_range` 只接受覆盖整段素材（容差 0.05s）的值，其它值报 `unsupported_operation`。目标协议能原生表达片段时再开放。
- 未被 prompt 引用的条件仍然生效。条件总数（schema 级）至多 32 个，实际上限由模型合同决定。

### 2.2 prompt 引用

- `@{id}` 引用一个条件；`@@` 表示字面量 `@`；其它 `@`（如邮箱）保持原样。
- `@{` 未闭合、`id` 非法、引用不存在的 `id` 均报错。
- adapter 先按实际提交顺序建立 `id → 厂商标记` 映射，再按语法树替换，不做全局字符串替换。每种模态独立编号（1 起）：

| adapter | reference_image | reference_video | reference_audio | keyframe start / end |
| --- | --- | --- | --- | --- |
| MiniMax H3 / OpenRouter | `reference image N` | `reference video N` | `reference audio N` | `the first frame` / `the last frame` |
| Seedance (Ark) | `@图像N` | `@视频N` | `@音频N` | `首帧` / `尾帧` |
| 其它 adapter | `image N` | `video N` | `audio N` | `the first frame` / `the last frame` |

- 原始 prompt、提交 prompt、映射和预处理版本写入任务执行记录（native task 的 `resolved_parameters.video_plan`），普通日志只记录条件 id 与计数。

### 2.3 输出规格 `output`

| 字段 | 规则 |
| --- | --- |
| `duration_seconds` | 有限正数，≤ 600 |
| `aspect_ratio` | `W:H`，W/H 为 1–100 的整数；不接受 `adaptive`（省略即由模型决定） |
| `size` | `{short_edge}` 与 `{width,height}` 严格二选一；数值 16–8192；`{width,height}` 与 `aspect_ratio` 同时给出时约分后必须一致 |
| `fps` | 有限正数，≤ 240 |
| `audio` | `auto`（默认）/ `generate` / `none` |
| `media_type` | `video/*` |

省略采用部署默认值；显式但不支持的值必须拒绝，不能改写或静默丢弃。`audio=none` 只有模型能不生成音轨时才可满足；对总是生成音轨的模型拒绝（本轮不做输出去音轨后处理）。`negative_prompt`、`seed` 不支持时拒绝；`seed` 不承诺跨模型复现。

`short_edge` 与厂商分辨率档位一一对应：480P→480、720p→720、768P→768、1080p→1080、2K→1440、4K→2160。MiniMax/OpenRouter 的 `2K` 未给出像素，记为 1440（2026-10-22 OpenRouter H3 实测输出 2560×1440）。

### 2.4 结果 `videos[]`

`video.generate/edit/extend/upscale` 的响应统一为 `videos: GeneratedVideo[]`：

```json
{
  "resource": {"kind": "named_object", "obj_id": "..."},
  "media_type": "video/mp4",
  "provider_metadata": {"duration_seconds": 6, "width": null, "height": null, "fps": null, "has_audio": null},
  "measured_metadata": {"duration_seconds": 6.04, "width": 2560, "height": 1440, "fps": 24, "has_audio": true},
  "last_frame": null,
  "deviations": [{"field": "duration_seconds", "requested": 6, "actual": 6.04, "source": "measured"}]
}
```

- `provider_metadata` 只来自 Provider 响应；`measured_metadata` 只在 AICC 读到产物字节时（Base64 物化、需认证的下载）由 `media` 模块实测，否则为 `null`；不把请求规格回填为实际值。
- 与显式请求不一致时写入 `deviations`（时长容差 0.5s，比例容差 3%——编码器按 16/32/64 像素对齐，如 16:9 的 480p 常为 832×480；尺寸/fps/音轨精确比较），不伪报成功。
- pending/failed 状态 `videos` 可为空。第一版每次一个结果，不自动多次推理；Provider 返回多于一个结果时报错而不是截断。
- Base64 产物物化为 Named Object；Provider URL 保持 URL 并沿用现有登记、授权读取与过期行为，不宣称已持久化。

## 3. 路由需求

`ModelRequirement.video: VideoGenerationRequirement` 与 `HelperModelRequirement.video` 同构：

```json
{"prompt": true, "negative_prompt": false, "seed": false,
 "reference_images": 2, "reference_videos": 0, "reference_audios": 0,
 "keyframes": ["start"], "reference_video_audio": false, "time_ranges": false,
 "output": {"duration_seconds": 6}}
```

- `VideoGenerateRequest::requirement()`（WebSDK `videoGenerationRequirement()`）从请求推导；helper 与 `route.resolve` 共用同一判定。
- helper 取“请求推导 ∪ 调用方声明”：计数取大、布尔取或、关键帧取并集；调用方 `output` 与请求 `output` 冲突时报错。调用方声明不能降低请求本身的要求。
- 需求只包含调用方提出的内容；素材时长、音轨等服务端探测事实不进入公共需求，`route.resolve` 的预选不替代执行前的复核。

## 4. 模型能力合同（服务端）

Model Driver 的 `capabilities.video_generation` 声明厂商官方云 API 的合同；Provider Rules 的 `video_generation` 声明渠道 + adapter 的真实子集并替换前者；OpenRouter discovery 用 `/api/v1/videos/models` 的事实覆盖输出相关字段。合同经 catalog 校验，未知字段拒绝。

```json
{
  "prompt_required": true,
  "negative_prompt": false,
  "seed": false,
  "text_to_video": true,
  "keyframes": {"positions": ["start", "end"], "end_only": true},
  "references": {
    "image": {"max": 9},
    "video": {"max": 3, "min_seconds": 2, "max_seconds": 15, "max_total_seconds": 15, "include_audio": true},
    "audio": {"max": 3, "min_seconds": 2, "max_seconds": 15, "max_total_seconds": 15},
    "max_total": 12,
    "audio_requires_visual": false
  },
  "keyframes_with_references": false,
  "output": {
    "durations": {"min": 4, "max": 15, "integer": true},
    "default_duration_seconds": 4,
    "aspect_ratios": ["21:9", "16:9", "4:3", "1:1", "3:4", "9:16"],
    "default_aspect_ratio": "16:9",
    "keyframe_aspect_ratio": "input",
    "resolutions": [{"name": "768P", "short_edge": 768}, {"name": "2K", "short_edge": 1440}],
    "default_resolution": "768P",
    "sizes": [],
    "fps": [],
    "audio": "always",
    "media_types": ["video/mp4"],
    "rules": [{"when": {"resolutions": ["2K"]}, "durations": {"values": [5, 10]}}]
  }
}
```

- 缺省 = 不支持：`keyframes` 缺省不能用关键帧，`references.*` 缺省不能用对应参考，`durations/aspect_ratios/resolutions/sizes/fps` 为空时拒绝对应显式值。
- `keyframe_aspect_ratio=input` 表示有关键帧时比例取自输入图；此时显式 `aspect_ratio` 只在与实测关键帧比例一致（2% 容差）时接受，否则拒绝。
- `audio`: `always`（必定有音轨）/ `never` / `optional`（协议可开关）/ `unknown`（官方未说明：`auto` 可用，显式 `generate`/`none` 拒绝）。
- discovery 只给出一个分辨率档位时，该档位即默认档位（渠道只能产出该档）。
- `rules[]` 表达组合限制：`when` 内的条件全部成立时（`resolutions`、`inputs`、`scenarios`），再叠加 `durations/resolutions/aspect_ratios` 限制。
- 合同按具体模型与渠道登记，并记录官方协议依据；不从同名模型推断其它渠道的能力。

## 5. 执行顺序与素材事实

1. schema 与业务校验（`from_json` + `validate()`）。
2. 预选路由：只用请求推导的需求，落实 privacy / `local_only` / 可见性策略；无合格候选时在任何 URL 下载或探测前失败。
3. 素材准备：对 `reference_video` / `reference_audio` 读取字节（URL 走受控下载：HTTPS、公网地址、大小与超时限制），实测容器、时长、宽高、fps、音轨；Base64 / Named Object 图片同时读出尺寸。URL 视频/音频写入租户 Named Object 作为提交副本，保证“验证的内容 = 提交的内容”；去音轨派生素材同样落为 Named Object，并在计划中记录 `derived_from`。
4. 复核路由：helper 带素材事实重新路由；exact 用同一判定复核所选模型。fallback 候选必须满足同一需求。
5. lowering：按合同生成 `VideoPlan`（场景、提交顺序、厂商标记、时长/比例/档位/音频、默认值来源），写入 `resolved_parameters.video_plan`，并给定价提供 `/video/resolution`、`/video/duration_seconds`、`/video/audio`、`/video/reference_images` 维度。
6. 资源物化 → codec 编码提交。codec 只读计划，遇到计划之外的组合直接拒绝。

资源限额沿用默认（最多 16 项、单项 32 MiB、总量 64 MiB、inline 4 MiB）；原始与派生素材分别计数。URL 视频转存后按对象 URL（配置了 `public_named_object_base_url`）或 inline 字节交给 Provider，后者受请求体上限约束。

探测由 `media` 模块完成：ISO-BMFF（MP4/MOV：`mvhd/tkhd/mdhd/stts/hdlr`）、WAV、MP3（逐帧扫描）、PNG/JPEG/WEBP/GIF。去音轨只处理非分片 MP4/MOV：删除 `soun` trak 并修正 `stco/co64`；分片 MP4（含 `mvex`）报错。不依赖 ffmpeg/ffprobe。

## 6. 任务、幂等与费用

- 幂等指纹使用原始公共请求（含条件顺序、clip、`include_audio`、`output`、`seed`），素材准备、重排、去音轨与厂商 prompt 都不改写它。helper 以原始 helper 请求计算指纹，作用域是执行方法（`video.generate`）。
- 显式 `idempotency_key` 在路由前先查执行记录（`ExecutionEngine::replay`）：同 key 同请求直接返回原任务及其绑定模型，不重新路由、不重复下载素材，路由结果变化或此刻无候选都不影响；同 key 不同请求返回 `idempotency_conflict`。
- 持久边界：素材准备产物是租户 Named Object（可重做、无外部费用）；执行记录先以 `submitted` 落库，native submit 成功后才写入绑定（实际模型/operation、remote task id、`resolved_parameters.video_plan`、价格快照、凭据引用）。重启时有绑定的任务只查询/恢复；无绑定的任务直接失败，从不重新提交。恢复描述所需的资源上下文在提交前检查。
- 提交被拒（非 2xx，`native_submit_rejected`）表示上游没有任务，按通常策略重试/failover；提交结果不确定（超时、断连、成功响应无法解析，`native_submit_unknown`）或已接受后的失败一律不重试、不跨 Provider failover（视频 API 对 `after_accept` 同样生效），配额按 unknown 结算。
- 取消：未绑定前取消本地执行；已绑定且上游支持取消时调用上游；上游不支持（OpenRouter、Veo）时返回 `unsupported_operation`，不伪报已取消。
- 升级与旧任务：旧方法（`video.txt2video` 等）不再可读。启动恢复时无法解析的未完成记录被“退役”：去掉绑定与暂存输出、标记为 `failed`（`unsupported_operation`），向 TaskMgr 报失败，配额按 unknown 结算，且不阻断其它任务恢复。上游已接受的旧任务仍会在厂商侧完成并计费但不再回收结果，因此升级前应先排空进行中的视频任务。
- 上游终态 `usage.cost` 进入 `AiUsage.reported_cost`（OpenRouter 为 USD 总费用），与本地估价不重复累计；计费秒数（`video_seconds`）、产物实测时长、请求估值分开记录，小数秒不截断。
- 复合计价（按秒 + 按参考图）用 `Pricing.addons` 表达，`AiUsage.reference_images` 计量；未知 SKU 保持未知，不当作 0。

## 7. Provider 支持矩阵（2026-10-22）

✓ 支持；✗ 明确拒绝；? 按官方文档开放但未在线验证。

| 能力 | MiniMax H3 | H3-Max | OpenRouter H3 | OpenRouter H3-Max | Seedance 2.0 / fast / mini | Seedance 2.5 | Veo 3.1（Preview 已于 2026-10-22 停服，已从 `driver_metadata` 撤下；下表为该型号停服前的合同记录） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 纯文本 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ? |
| 首帧 / 尾帧 / 首尾帧 | ✓/✓/✓ | ✓/✓/✓ | ✓/✓/✓ | ✓/✓/✓ | ✓/✗/✓ | ✓/✗/✓ | ?/✗/? |
| 参考图 | ≤9 | ≤9 | ≤9 ? | ✗ | ≤9 | ≤30 | ≤3 ? |
| 参考视频 | ≤3，2–15s，总 15s | 同左 | ✗（渠道忽略） | ✗ | ≤3，2–15s，总 15s | ≤10，2–30s，总 30s | ✗ |
| 参考音频 | ≤3，2–15s，总 15s | 同左 | ✗ | ✗ | ≤3（需图或视频） | ≤10，可仅音频 | ✗ |
| 关键帧 + 参考混用 | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 时长 | 4–15 | 5–15 | 目录 5–15 | 目录 5–15 | 4–15 | 4–30 | 4/6/8 |
| 分辨率 | 768P/2K | 480P/768P | 目录 2K | 目录 480p/768p | 480p/720p/1080p(/4k 仅 2.0) | 480p/720p/1080p | 720p/1080p/4k |
| 输出音频 | always ? | never ? | 目录 true→always | 目录 false→unknown（T2 实测文件带有可听音轨） | optional | optional | always |
| seed / negative_prompt | ✗/✗ | ✗/✗ | 目录 / ✗ | 目录 / ✗ | ✓/✗ | ✓/✗ | ✓/✗ |
| 取消 | ✓ | ✓ | ✗ | ✗ | ✓（排队中） | ✓ | ✗ |

其它模型：Hailuo-02/2.3（MiniMax v1：文本、首帧 ?）、Sora 2（文本、首帧、`video.edit`）、Gemini Omni 1.1 Flash（文本、单张参考图、`video.generate`/`video.edit`/`video.extend`，是 Gemini API 上目前唯一在售的视频型号）、GLM CogVideoX / Vidu（文本、首帧、首尾帧、参考图 ?）、Qwen Wan（文本、首帧 ?）、fal（仅 `video.upscale`；通用 queue 不再声明生成能力）。完整合同以 `driver_metadata` 为准。

## 8. 错误

复用 `AiccError`：schema 与业务校验失败为 `invalid_request`；合同不支持的组合/规格为 `unsupported_operation`；无合格模型为 `no_candidate_model`；素材不可访问/损坏/缺音轨为 `resource_invalid`；Provider 故障为 `provider_error`。`details` 统一包含 `field`（JSON 指针）、`condition_id?`、`expected?`、`actual?`，不包含凭据或素材内容。
