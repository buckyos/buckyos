# Issue #624 / PR #634 Review 修复验证报告

日期：2026-09-26。验证对象为合入提交 `704e81b2` 加本轮工作区修复，尚未生成新的提交或部署到 DV。
本报告替换旧报告；旧报告引用的外部分支提交和历史 T1/T1.5/T2 结果不能证明当前工作区通过验收。

## 本轮结果

| 验证 | 结果 | 范围 |
|---|---|---|
| `cd src && cargo test -p aicc` | 561 passed，0 failed | 单元测试；binary/doc targets 0 项 |
| `cd src && cargo check -p aicc --all-targets` | 通过 | 后端所有构建目标 |
| `cd src && cargo test -p buckyos-api` | 215 单元测试 + 10 集成测试通过 | 共享 DTO、typed API、decision |
| `cd src/frame/desktop && pnpm run build` | 通过 | TypeScript（含全部 e2e）及 Vite 生产构建 |
| `pnpm exec playwright test tests/e2e/pages/ai-center.spec.ts --project=chromium` | 1 passed | Provider 向导、端点继承、多币种展示 |
| `cd test/aicc_test && pnpm run acceptance:preflight` | 通过 | 13 Provider，130 static cases，681 T1.5 cases；这是 manifest 检查，非网关执行数量 |
| `pnpm run acceptance:self-test` | 87 passed，0 failed | Mock 协议、请求/响应 fixture、矩阵和报告自测 |
| `deno check --node-modules-dir=none acceptance/provider_protocol_contracts.ts acceptance/t15_mock_provider.ts` | 通过 | 本轮修改的协议契约和 Mock |
| `deno check --node-modules-dir=none acceptance/*.ts` | 未通过 | 30 处现有 WebSDK 类型错误，见下文 |
| `cd src && uv run check.py` | Running | 只读检查 DV 核心进程和端口，不能证明已部署本轮代码 |
| `git diff --check` | 通过 | 工作区 diff |

实际 Provider 调用：0；真实调用费用：0；本轮没有修改 DV 的设置、凭证、进程或部署文件，没有需要恢复的远端临时资源。
Rust 构建仍有 dead-code 等警告，不影响上述测试通过。

## 修复与回归证据

- **启动可用性**：`service/settings_runtime.rs` 隔离单实例启动/凭证/adapter 解析失败，管理 inventory 与事件日志保留错误。
  `persisted_reserved_custom_adapters_do_not_block_startup_or_reload` 覆盖四种旧 custom adapter、正常实例及重载后的再次收敛。
- **计费与路由**：HTTP 402 换候选而不原地重试；tiers-only 钉价及边界结算、非法 pricing 校验、全部静态价格结算维度遍历、可路由媒体单位与 API 对应校验均有测试。
  媒体 fixture 检查成功图片张数、视频秒数、Seedance completion tokens、TTS 字符数实际进入费用计算。
  Gemini 时长在最终编码请求中读取并随 task 保存；Sora 从状态响应读取，其他视频从结果响应读取。缺失必要用量时仍保持 unknown。
- **协议**：`protocol/review_tests.rs` 覆盖原生任务业务错误详情、默认/显式 Retry-After、MiniMax cancelled、GLM 缺失 id、TTS 四种格式/完成帧用量/HTTP 错误、整数时长、Seedream/Wan 尺寸与 Qwen 图片部分成功。
  原生轮询默认回退 3 秒，并限制每次驱动最多 30 分钟（包括等待中的单次请求）。进程恢复后重新计时，尚未改成跨重启累计截止。
- **元数据与连接**：国内 CNY 不匹配 global；MiniMax M2 文本/M3 vision；Qwen flash 精确项恢复阶梯价；Doubao 按量默认 URL、自定义代理和显式 operation 覆盖、plan 价格 unknown、policy_region 默认和 allowed_values 均有校验。
  无法复核的 `glm-4-32b-0414-128k` USD 价格已移除，不猜测币种或金额。
- **测试防漏**：恢复 MiniMax 音乐契约；加入 Qwen 图片生成、编辑和视频 Mock，并补齐三个对应官方模型的 Model Driver 身份；`qwen_media_protocol_fixtures_resolve_to_routable_inventory` 确认不会在路由前因 NoMatch 被丢弃；拒绝错误尺寸、浮点 duration、缺失输入和已移除字段；variant 基础模型缺失恢复失败；能力矩阵加入真实 vision 样本。
  T1 Docker 重启流程中的 PID 查找、cmdline 校验、kill、再次查找全部位于同一 host PID namespace；本轮没有实际执行进程重启。
- **其他**：无效可选 cost 遥测不再破坏成功响应（有效 cost 仍优先用于结算）；Claude/MiniMax 参数校验与 canonical 优先；buffered SSE namespace；trace warnings；system caller 过滤；ProtocolAdapterView 缺省/未知字段测试；向导仅保存显式 operation URL。

## 官方协议核对

Mock/fixture 修改依据下列官方资料，非当前 AICC 实现的反向推导：

| 协议点 | 依据 |
|---|---|
| GLM 成功查询可缺失 id | [MetaGLM 官方 CogVideoX cookbook](https://raw.githubusercontent.com/MetaGLM/glm-cookbook/main/vision/cogvideox_pysdk.ipynb)：成功输出为 `id=None`，查询继续使用提交的任务 ID |
| Doubao TTS usage header | [官方鉴权示例](https://docs.volcengine.com/docs/DoubaoVoice/old-version-console-authentication-reference-example?lang=zh)：`X-Control-Require-Usage-Tokens-Return` |
| Seedream 尺寸与 Seedance extension/用量 | [图片 API](https://docs.volcengine.com/docs/ark/image-generation-api?lang=en)、[视频创建 API](https://docs.volcengine.com/docs/ark/create-video-generation-task-api?lang=zh&redirect=1)、[视频结果 API](https://docs.volcengine.com/docs/82379/1521675) |
| Qwen 图片 W*H 与部分成功 | [Wan 图片 API](https://help.aliyun.com/en/model-studio/text-to-image-v2-api-reference)、[Qwen 编辑 API](https://help.aliyun.com/en/model-studio/qwen-image-edit-api) |
| Wan 旧版 size、整数 duration 与响应时长 | [旧版视频 API](https://help.aliyun.com/en/model-studio/legacy-wan-text-to-video-api-reference)、[新版视频 API](https://help.aliyun.com/zh/model-studio/text-to-video-api-reference) |
| MiniMax 视频任务、音乐、ASR language | [视频 V2 查询](https://platform.minimax.io/docs/api-reference/video-generation-v2-query)、[音乐 API](https://platform.minimax.io/docs/api-reference/music-generation)、[ASR API](https://platform.minimax.io/docs/api-reference/speech-to-text)；language 确为 header，未改动 |
| MiniMax 国内域名与模型能力 | [国内 Messages API](https://platform.minimax.cn/docs/api-reference/text-anthropic-api)、[模型说明](https://platform.minimax.io/docs/guides/models-intro)；合入代码的国内域名已正确 |
| Veo 缺省时长与 extension | [Veo API](https://ai.google.dev/gemini-api/docs/veo?hl=en)：普通生成缺省 8 秒，extension 新增 7 秒 |

## 未通过与未执行项

全量 Deno 检查先遇到本地 `node_modules` 无法解析 `npm:@types/node`；使用 `--node-modules-dir=none` 后可完成依赖解析，但报告 30 处 WebSDK 类型错误。
`test/aicc_test/deno.json` 把 `buckyos` 映射到 SDK 的 `dist/node.mjs`，Deno 从 JS 推断部分 nonce/token 参数只接受 `null | undefined`，与 runner 传入的 number/string 冲突。
已从 `704e81b2` 导出独立基线目录、使用同一本地 SDK 重跑，复现相同的 30 条错误（`/tmp/aicc-pr634-deno-baseline.log`）。
错误位于 `acceptance/gateway.ts`、`acceptance/run_t1_gateway.ts` 和 `test/jarvis_media_dv/jarvis_media_dv.ts` 的既有鉴权调用；本轮未改依赖或扩大到 SDK 修复。修改的协议契约/Mock 单独检查通过。

本轮未执行当前代码经 DV 网关的 T1/T1.5，因此不再保留旧报告“129/188 项通过”作为当前结果。
推送前须部署当前构建，按受影响 Provider 执行 T1/T1.5，并保存 manifest、逐 case evidence 与 cleanup 结果。
参考 [验收 README](README.md) 的配置保护和命令；T1.5 需同时提供环境变量及命令行的 config-mutation 开关。

T2/T3（含 GLM 真实视频）未执行。[AICC E2E Skill](../../../harness/SKILLS/aicc-e2e-test/SKILL.md) 明确要求：
“CodeAgent 开始执行任何 T2 或 T3 测试前，必须取得用户对本次执行的明确授权。”
当前请求授权代码修复，未选定真实 Provider 调用范围和预算；官方文档核对及本地 Mock 通过不能替代真实服务验收。

本机日志位于 `/tmp/aicc-pr634-{cargo,api,check,desktop-build,e2e,preflight,selftest,deno,deno-protocols,dvcheck}.log`，未将临时日志或本地配置提交到仓库。
