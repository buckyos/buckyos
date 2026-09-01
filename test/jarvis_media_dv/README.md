# Jarvis Media DV Test

本目录在真实 BuckyOS 环境中验证 Jarvis 的文本、图片、音频和视频任务。唯一测试入口默认只走 msg-center；显式启用 Telegram 后可依次覆盖两条完整消息链路。所有通道共用 [`scenarios.ts`](./scenarios.ts) 中的场景与判定规则。

## 消息链路

```text
DV runner → Zone Gateway → msg-center → MessageHub → Jarvis/OpenDAN
          → AICC/Provider → msg-center session → DV runner

DV runner / GramJS user client → Telegram → Jarvis Bot
  → msg-center Telegram tunnel → MessageHub → Jarvis/OpenDAN
  → AICC/Provider → Telegram tunnel → Jarvis Bot reply → DV runner
```

GramJS `telegram` 包只属于本测试工具，不进入 BuckyOS、msg-center 或 Jarvis 的运行时依赖。

## 唯一入口

```bash
cd test/jarvis_media_dv
pnpm install
cp jarvis_media_dv.example.toml jarvis_media_dv.local.toml
pnpm test
```

正式发送消息前，runner 会先按优先级合并参数，并交互补齐可提前输入的必需值，然后展示最终环境清单，包括：

- 即将使用的消息出入口及 Gateway/Bot，以及每项参数的来源；
- BuckyOS 与 Telegram 登录参数的最终取值状态，敏感值只显示是否已经配置；
- 每一个 Named Store ID、本地媒体路径及其就绪状态；
- 期望覆盖的 Provider；
- 场景、步骤、人工判定设置和报告目录。

清单展示后进入 10 秒倒计时：输入 `c` 并回车可以取消，直接回车可立即开始，超时则自动开始。无人值守环境使用 `pnpm test -- --yes`：它同时跳过等待并禁止所有交互输入。`--yes` 不代表 CodeAgent 已获得运行 T2/T3 的人工授权。此时缺少消息通道的登录/API 等基础参数会直接失败；缺少媒体素材只跳过依赖该素材的场景。Telegram 在登录过程中需要验证码或 2FA 时，也必须提前通过参数、配置或环境变量提供。`--dry-run` 不交互补参，只展示当前已收集的参数和步骤，不发送消息。

消息通道选择本身不是必须参数，完全不配置时只启用 msg-center。普通模式只会交互询问已启用通道启动所必需的参数，例如 BuckyOS 登录信息，或显式启用 Telegram 后所需的 API 凭据和首次登录信息。图片、音频、视频等场景素材不是整套测试的必须参数，runner 不会为它们弹出输入提示；缺少素材时只把依赖它的场景记为 `skipped`，其它场景继续执行。

仓库统一 DV 入口仍可发现并执行该套件：

```bash
uv run test/run.py -p jarvis_media_dv
```

## 配置优先级

参数按命令行、本地 TOML、环境变量、交互输入的顺序解析。默认配置文件是 `jarvis_media_dv.local.toml`，也可以使用：

```bash
pnpm test -- --config /secure/path/jarvis-media.toml --yes
```

未配置消息通道时默认只执行 `msg-center`，不会询问 Telegram 参数。需要覆盖 Telegram 时可重复指定消息通道；Provider 参数同理：

```bash
pnpm test -- \
  --transport msg-center \
  --transport telegram \
  --provider openai \
  --provider fal \
  --suite smoke
```

TOML 中的对应配置为：

```toml
[common]
transports = ["msg-center"]
suite = "all"
scenario_concurrency = 2
yes = false

[environment]
providers = ["openai", "gemini", "fal", "minimax", "claude"]
```

同时测试两条消息链路时，将 `transports` 改为 `["msg-center", "telegram"]`。

Provider 列表是本轮期望覆盖目标，不锁定模型或篡改 AICC 路由；实际命中的 Provider 以 AICC 运行日志为准。环境变量可使用逗号分隔的 `JARVIS_DV_TRANSPORTS`、`JARVIS_DV_PROVIDERS`，以及 `JARVIS_DV_YES`。

带语义 rubric 的步骤默认通过 AICC 的 `llm.plan.default` 执行 LLM Judge，可用 `[judge].model`、`JARVIS_DV_JUDGE_MODEL` 或 `--judge-model` 参数化，也可用 `--no-judge` 关闭。Judge 返回的 task ID、结论和理由写入步骤报告；Judge 调用按 task ID 合并进财务总账，不会被误算成 Jarvis workflow Provider 覆盖。结构、消息类型、附件数量和 MIME 仍由确定性断言判定，Judge 不能放宽这些断言。

msg-center 会在不同 Jarvis session 间并行执行场景，默认并发为 2，可用 `common.scenario_concurrency`、`JARVIS_DV_SCENARIO_CONCURRENCY` 或 `--scenario-concurrency` 调整；同一场景内的历史依赖步骤仍严格串行。Telegram 为避免外部测试账号风控保持串行。T3 runner 不实现全局/Provider 并发和最小请求间隔门禁。

## msg-center

runner 通过 Zone Gateway 的 `control-panel.auth.login` 获取临时 session token，再调用 msg-center。密码和 token 不写入报告。

Gateway URL 没有默认值，是所有运行模式的必填参数。可通过 `--gateway-url`、配置文件中的 `msg_center.gateway_url` 或 `BUCKYOS_TEST_GATEWAY_URL` 提供；普通模式缺失时 runner 会提示输入，`--yes` 自动模式缺失时直接失败。复制 [`jarvis_media_dv.example.toml`](./jarvis_media_dv.example.toml) 可创建本地配置模板。

| 参数 | 环境变量 |
|---|---|
| Gateway | `BUCKYOS_TEST_GATEWAY_URL` |
| 用户名 | `BUCKYOS_TEST_USERNAME` |
| 密码 | `BUCKYOS_TEST_PASSWORD` |
| Session override | `BUCKYOS_APPCLIENT_SESSION_TOKEN` |
| 用户 ID | `BUCKYOS_TEST_USER_ID` |
| 用户 DID | `JARVIS_DV_USER_DID` |
| Zone DID | `JARVIS_DV_ZONE_DID` |
| Jarvis DID | `JARVIS_DV_AGENT_DID` |
| 可选 Group DID | `JARVIS_DV_GROUP_DID` |

msg-center 默认使用测试工程 `assets/` 中已提交的六类素材。runner 登录后会把当前场景需要的文件上传到 Named Object Store，并使用生成的 `chunk:` ID 发送附件。也可以通过 `--image-primary-id`、`--image-secondary-id`、`--image-ocr-id`、`--audio-sfx-id`、`--audio-speech-id`、`--video-fresh-id` 或示例 TOML 中的对应字段提供已有的完整类型化 Named Object ID（例如 `cyfile:...` 或 `chunk:...`）；显式 ID 优先于工程素材。

## Telegram

在 [my.telegram.org](https://my.telegram.org) 创建 Telegram application，取得 `api_id` 和 `api_hash`。测试需要 Telegram API 凭据、Jarvis Bot 用户名，以及首次登录所需的手机号、验证码和可能存在的 2FA 密码。

| 参数 | 环境变量 |
|---|---|
| API ID | `TELEGRAM_API_ID` |
| API hash | `TELEGRAM_API_HASH` |
| 手机号 | `TELEGRAM_PHONE` |
| 一次性登录码 | `TELEGRAM_CODE` |
| 2FA 密码 | `TELEGRAM_PASSWORD` |
| Jarvis Bot 用户名 | `JARVIS_TELEGRAM_BOT_USERNAME` |
| StringSession | `TELEGRAM_SESSION` |
| Session 文件 | `TELEGRAM_SESSION_FILE` |

首次登录成功后，StringSession 默认写入 `.jarvis_media_dv.telegram.session`。Telegram 直接发送 `assets/` 下的本地文件，也可使用 `--image-primary-file`、`--audio-sfx-file` 等参数或示例 TOML 覆盖。

API hash、2FA 密码和 StringSession 都是敏感信息，建议保存在权限受限的本地配置、session 文件或环境变量中。

## 用例、报告与退出码

- `smoke`：媒体理解、OCR、图片编辑、音效、ASR/TTS、图生视频，以及合法、空、损坏、加密、路径穿越、文件数、解压量和嵌套深度 ZIP；
- `linked`：历史生成物、连续附件、引用纠错、视频续写；
- `matrix`：文本、图片、音频、视频之间完整的 4×4 转换矩阵。

完整说明见 [`TEST_CASES.md`](./TEST_CASES.md)，素材说明见 [`assets/README.md`](./assets/README.md)。每个场景从 `/clean` 开始。

主报告写入 `reports/jarvis_media_dv/<run_id>/summary.md`，按 transport 和场景汇总结果。场景内与 Jarvis 的详细对话默认不在主报告中展开，每行提供“查看对话”链接，指向 `conversations/<transport>-<scenario>.md`。同目录保留 `summary.json` 作为机器可读数据。单个 transport 初始化失败时会记录 `_environment` 失败并继续测试下一个 transport。

报告还会逐入口列出文本、图片、视频、音频、文档、压缩包和多附件的入站/出站覆盖状态。全量 suite 若没有声明某一覆盖单元会生成 `_entry_coverage` 失败；缺素材的 `skipped`、平台限制和待复核项不会被当成通过。断言、附件或 usage 审计失败会写入结构化 `product_defects`，包含预期行为、实际行为和证据路径。财务报告同时列出 Jarvis workflow 与 Judge usage，按 caller/time window 和 Judge task ID 去重汇总，并调用 AICC `trace.query` 审计每个执行步骤的 `run_id:transport:scenario_id:step_id`。若 Jarvis/AICC 没有传播该 ID，报告会失败并明确标为产品缺陷，不会用时间邻近关系伪造逐步骤关联。

- `0`：没有失败，且人工项已通过；使用 `--allow-review` 时允许遗留 review；
- `1`：存在自动、人工或环境失败；
- `2`：没有失败，但仍有待人工确认项。

运行前应确保 BuckyOS 服务可经 Zone Gateway 访问、Telegram tunnel 已绑定、AICC 已配置真实 Provider、Telegram 测试账号已打开过 Jarvis Bot 私聊。Provider token 可通过本地 TOML 的 `[provider_credentials.<driver>]` 参数化；只有配置与 `--allow-credential-mutation` 同时允许时才临时写入 AICC，并在 `finally` 恢复原设置。Minimax、OpenRouter 和 SN 可保留在覆盖清单中；没有账号或明确禁止执行时不发起真实调用。
