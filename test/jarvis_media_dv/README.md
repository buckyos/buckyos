# Jarvis Media DV Test

本目录用于在真实 BuckyOS 环境验证 Jarvis 的媒体任务闭环。测试关注实际用户行为，不替代 AICC、OpenDAN 或 msg-center 的单元测试。

自动模式的真实链路为：

```text
DV runner
  → Zone Gateway
  → msg-center.msg.post_send
  → MessageHub
  → Jarvis/OpenDAN
  → AICC/Provider
  → msg-center 会话与附件
  → DV runner 轮询终态
```

Telegram 模式由真实 owner 账号向 Jarvis bot 发送消息，覆盖完整 tunnel 入站和出站链路。

## 运行模式

### 1. Telegram tunnel 人工模式

这是完整 tunnel 验证的推荐模式。Telegram Bot API 只能让 bot 发消息，不能模拟 owner 用户给 bot 发消息，因此 runner 校验 bot token 后逐步显示素材、指令和判定条件，由测试人员在真实 Telegram 对话中操作。

```bash
cd test/jarvis_media_dv
export JARVIS_TELEGRAM_BOT_TOKEN='<telegram-bot-token>'
pnpm test -- --transport telegram-manual --suite all
```

也可以通过参数输入 token：

```bash
pnpm test -- \
  --transport telegram-manual \
  --telegram-bot-token '<telegram-bot-token>' \
  --case edit_then_animate
```

环境变量更安全；命令行参数可能被 shell 历史或进程列表记录。token 只用于运行时 `getMe` 校验，不写入报告。

### 2. MessageHub 自动模式

自动模式直接向 Jarvis 的原生 DID 发送消息，仍然经过 Zone Gateway、认证、msg-center 和 MessageHub，只跳过外部 Telegram adapter。

```bash
cd test/jarvis_media_dv

export BUCKYOS_TEST_GATEWAY_URL='https://your-zone.example.com'
export BUCKYOS_TEST_USERNAME='devtest'
export BUCKYOS_TEST_PASSWORD='<login-password>'

export JARVIS_DV_IMAGE_PRIMARY_ID='cyfile:...'
export JARVIS_DV_IMAGE_SECONDARY_ID='cyfile:...'
export JARVIS_DV_IMAGE_OCR_ID='cyfile:...'
export JARVIS_DV_AUDIO_SFX_ID='cyfile:...'
export JARVIS_DV_AUDIO_SPEECH_ID='cyfile:...'
export JARVIS_DV_VIDEO_FRESH_ID='cyfile:...'

pnpm test -- --suite all --interactive-review
```

参数形式：

```bash
pnpm test -- \
  --gateway-url 'https://your-zone.example.com' \
  --username 'devtest' \
  --password '<login-password>' \
  --image-primary-id 'cyfile:...' \
  --case image_edit \
  --interactive-review
```

自动模式默认要求用户通过 `--username`/`--password` 或 `BUCKYOS_TEST_USERNAME`/`BUCKYOS_TEST_PASSWORD` 提供登录凭据。runner 通过 Zone Gateway 的 `control-panel.auth.login` 获取临时 session token，并从登录响应取得用户 ID；密码和 token 都不会写进日志或报告。环境变量比命令行参数更安全。

调试认证链路时仍可通过 `--session-token` 或 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 显式覆盖用户名/密码登录；此时还需通过 `--user-id`/`BUCKYOS_TEST_USER_ID` 或 `--user-did` 指明发送者。

如 zone DID 无法从 `boot/config` 读取，可显式设置：

```bash
export JARVIS_DV_ZONE_DID='did:web:your-zone.example.com'
# 或直接指定
export JARVIS_DV_AGENT_DID='did:web:jarvis.your-zone.example.com'
```

## Provider API key

OpenAI、Gemini、FAL 等 Provider key 应通过 BuckyOS/AICC 正常配置渠道安装到测试 Zone。DV runner 不接受并转发 Provider key，也不会把 key 放进消息或测试报告。

需要脚本协助配置 tunnel 时，token 同样只通过参数或环境变量传入，例如：

```bash
export JARVIS_TELEGRAM_BOT_TOKEN='<telegram-bot-token>'
export JARVIS_TELEGRAM_ACCOUNT_ID='<owner-telegram-id>'
export BUCKYOS_APPCLIENT_SESSION_TOKEN='<session-token>'
./src/configure_jarvis_tunnel.sh
```

## 测试素材

仓库已提供一套原创测试素材，位于 [`assets/`](./assets/)。先将相应文件上传到测试 Zone 的 Named Store，再把完整对象 ID 配置到下列变量。为便于判断附件是否选错，各素材具有明显差异：

| 变量 | 仓库文件 | 素材内容 |
|---|---|---|
| `JARVIS_DV_IMAGE_PRIMARY_ID` | `assets/image_primary.png` | 粉色花朵、岩石和绿叶 |
| `JARVIS_DV_IMAGE_SECONDARY_ID` | `assets/image_secondary.png` | 与花朵明显不同的山地公路 |
| `JARVIS_DV_IMAGE_OCR_ID` | `assets/image_ocr.png` | 唯一文字 `BUCKYOS-DV-4827` |
| `JARVIS_DV_AUDIO_SFX_ID` | `assets/audio_sfx.wav` | 合成蜂鸣、敲击和环境底噪，没有人声 |
| `JARVIS_DV_AUDIO_SPEECH_ID` | `assets/audio_speech.wav` | 清晰朗读“今天的测试编号是四八二七” |
| `JARVIS_DV_VIDEO_FRESH_ID` | `assets/video_fresh.mp4` | 花朵随风运动的 CC0 H.264 视频 |

自动模式使用完整类型化 Named Object ID，例如 `cyfile:...` 或 `chunk:...`。可以先通过 Telegram/MessageHub 上传素材，再从消息对象或日志取得对应 ID。测试不会接受只剩十六进制摘要的 ID。

缺少某项素材时，依赖它的自动用例会标记为 `skipped`；Telegram 人工模式由操作者在对应步骤选择本地文件。

## 用例选择

```bash
# 查看全部用例
pnpm test -- --list

# 只显示计划，不连接真实环境
pnpm test -- --suite all --dry-run

# 基础用例
pnpm test -- --suite smoke --interactive-review

# 跨消息关联用例
pnpm test -- --suite linked --interactive-review

# 文本、图片、音频、视频的 4×4 转换矩阵（16 个用例）
pnpm test -- --suite matrix --interactive-review

# 指定一个或多个用例
pnpm test -- --case audio_sfx --case edit_then_animate --interactive-review
```

仓库统一入口也会自动发现该模块：

```bash
uv run test/run.py -p jarvis_media_dv
```

统一入口无法附加 runner 参数，适合全部自动模式；需要选择 suite/case 时直接在本目录执行。

## 判定与退出码

每一步包含两层判定：

1. 自动结构判定：是否收到回复、是否包含期望文字、是否返回正确 MIME 类型的 Named Object 附件、是否出现已知音频幻觉文本。
2. 人工语义判定：图片是否选对、编辑结果是否符合指令、视频是否连续、回复是否对用户友好。

建议真实验收始终使用 `--interactive-review`。非交互运行会把需要视觉或听觉判断的步骤标为 `review`。

退出码：

- `0`：无失败，且人工项已通过；使用 `--allow-review` 时允许遗留 review。
- `1`：存在自动或人工失败。
- `2`：没有失败，但仍有待人工确认项。

报告写入：

```text
reports/jarvis_media_dv/<run_id>/summary.json
```

可以通过 `JARVIS_DV_REPORT_DIR` 或 `--report-dir` 修改目录。报告保存指令、回复文本、附件引用、耗时和判定，不保存 session token 或 Telegram bot token。

## 前置检查

1. 当前分支已完整构建并安装到测试 Zone。
2. AICC 已配置实际要验证的 Provider 和模型权限。
3. 测试账号可通过用户名和密码登录，Jarvis、msg-center、AICC、task-manager 和 verify-hub 正常运行。
4. 清理或同步过持久化 Jarvis behavior 后已重启 Jarvis。
5. 每个 scenario 开始时 runner 会发送或提示发送 `/clean`，避免上一用例污染上下文。

## 已知边界

- LLM 输出和生成媒体存在随机性，视觉、听觉和跨消息语义仍需要人工判断。
- `--transport native` 验证 MessageHub 原生链路，不覆盖 Telegram adapter；完整 tunnel 验证使用 `telegram-manual`。
- 长视频任务可能超过十分钟。用例按媒体类别设置等待上限，超时报告会保留最后观察到的结构状态。
- Provider 原生视频续写通常只适用于该 Provider 自己生成且仍可恢复生成状态的视频；全新上传视频用例验证的是合理降级和用户说明。
