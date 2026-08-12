# AICC L4 Gateway 环境组件

定义 gateway 真实模型验收、gateway TOML、buckyos-devkit 临时 group 生命周期和远程访问入口。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. Gateway 真实模型验收

真实模型成本受控：

- 每个真实 Provider 按 `api_type × method × 标准逻辑目录路径 × Provider × model` 展开用例；每个 planned 用例必须覆盖逻辑模型路径和精确物理模型路径。
- 每条 workflow 可以承载多个矩阵用例以控制成本，但报告必须逐项标记每个矩阵坐标的覆盖结果，不能用“代表性 workflow”替代未执行维度。
- 首次失败后只重跑同一个 `api_type × method × logical_path × Provider × model` 用例，最多累计 3 次 attempt。
- 任意 attempt 成功则该用例成功，所有 attempt 摘要都写入报告。
- 不断言自然语言全文，只断言协议事实。
- 未配置 API key 或 Provider 未启用时用例标记为 `skipped`，不算失败。
- `sn-ai-provider` 不需要普通 API key，缺 key 不得作为 skip 原因。
- 真实模型返回可理解错误时，报告记录为 `failed` 或 `partial`，保留错误码、Provider 摘要、trace id。

每条真实模型 workflow 至少断言：

1. 矩阵坐标中的 `api_type`、`method`、`logical_path`、`provider`、`exact_model` 被写入报告。
2. 逻辑模型段 route trace 正确，包含 `requested_model_type=logical`、`resolved_logical_path`、`selected_exact_model` 和 provider。
3. 物理模型段 response schema 正确，并确认 exact model 调用不发生隐式 fallback。
4. task 状态闭环。
5. artifact 可读取。
6. usage 存在。
7. route trace 存在且能关联逻辑段与物理段。
8. 错误被分类。
9. 成本调用次数受控。

建议 workflow：

| Provider | Workflow |
|---|---|
| OpenAI | 每个模型执行 `llm.chat` 多轮 + JSON schema + tool call + image/audio 或 embedding 子步骤 |
| Claude | 每个模型执行多模态 `llm.chat` + tool use + vision caption/OCR fallback |
| Google Gemini | 每个模型执行多模态 `llm.chat` + embedding/multimodal 或 image/video operation |
| fal | 每个模型执行 `image.upscale` / `image.bg_remove` / `audio.enhance` / `video.upscale` 中匹配能力的异步任务 + artifact 读取 |
| OpenRouter | 每个模型执行 `llm.chat` 复杂 JSON 输出 + OpenAI-compatible 兼容字段检查 |
| SN AI Provider | 每个模型执行无普通 API key 的 gateway 转发 workflow，验证 provider 归因、usage、trace 和 free credit 归因 |

## 2. 执行命令约定

推荐统一 runner 最终屏蔽底层命令，但文档仍保留基础命令，便于开发者单独定位问题。以下分为“当前可用命令”和“规划统一命令”。

### 2.1 Rust 单测

在仓库根目录执行：

```bash
cargo test -p aicc
cargo test -p buckyos-api --test aicc_client_test
```

如果按 AGENTS.md 建议在 `src` 目录运行，也可使用 workspace 相对命令；最终 runner 应固定工作目录，避免不同目录下路径解析不一致。

### 2.2 本地 kRPC Mock 验收

当前可用流程：

```bash
uv run src/check.py
cd test/aicc_test
pnpm install
pnpm test
pnpm run test:models
pnpm run test:fal
```

当前命令含义：

1. `pnpm test` 执行 `aicc_smoke.ts`，输出 `reports/aicc_smoke/<run_id>`。
2. `pnpm run test:models` 调用 `models.list`，打印 Provider inventory、legacy aliases 和逻辑目录树。
3. `pnpm run test:fal` 执行 fal provider 的 `image.upscale` / `image.bg_remove` / `video.upscale` 用例；未配置 fal 时按 skipped 处理。
4. 这些命令当前连接已启动的 BuckyOS / AICC，不负责自动启动 TS Mock Provider 或写入 Mock settings；统一 L3 runner 需要补齐该管理闭环。

规划统一命令：

```bash
cd test/aicc_test
pnpm run acceptance:local
```

`acceptance:local` 应完成：

1. 启动或连接 TS Mock Provider。
2. 写入 Mock settings。
3. 调用 `service.reload_settings`。
4. 调用 `models.list` 验证配置生效。
5. 运行 L3 用例。
6. 输出 `reports/acceptance/<run_id>`。

### 2.3 Gateway 真实模型验收

当前可用 remote runner：

```bash
cd test/aicc_test
pnpm install
pnpm run remote -- --config ./aicc_remote_runner.toml
```

规划统一 gateway 命令：

```bash
cd test/aicc_test
pnpm run acceptance:gateway -- --config ./aicc_acceptance.toml
```

真实模型验收必须显式传入配置文件；不应从开发者环境变量中隐式读取 key 后直接发起调用，避免误触发费用。

推荐最终提供一个全量自动化入口，用于发布前一次性执行 L1/L2/L3/L4 并输出报告：

```bash
cd test/aicc_test
pnpm run acceptance:all -- \
  --openai-key "<openai-api-key>" \
  --fal-key "<fal-api-key>" \
  --gemini-key "<google-gemini-api-key>" \
  --claude-key "<claude-api-key>"
```

`acceptance:all` 的职责：

1. 固定从仓库根目录或 `src` 目录解析路径，避免工作目录差异。
2. 执行 L1/L2 Rust 单测。
3. 执行 L3 本地 Mock 验收。
4. 使用 `buckyos-devkit` 创建并启动 L4 临时 group，通过 gateway 访问该 group。
5. 将传入的 4 个 key 写入临时 group 的 AICC settings；`sn-ai-provider` 不需要普通 API key。
6. 对 `openrouter`，runner 优先读取配置文件或临时 group settings 中的 `openrouter` key；如果发布验收要求强覆盖但缺 key，应在 preflight 阶段失败。普通开发验收可将 openrouter 矩阵标记为 `skipped`。
7. 动态读取 `models.list` 和最终生效逻辑目录，生成 `api_type × method × logical_path × Provider × model` 矩阵。
8. 每个 planned 矩阵用例执行逻辑模型段与精确物理模型段验证，失败后最多额外执行 2 次。
9. 输出 `summary.md`、`summary.json` 和脱敏后的 attempt 明细。
10. 清理 runner 新建的临时 group。

为了让参数尽可能少，`sn-ai-provider` 不设置普通 API key；OpenRouter key 不作为默认必填命令行参数，但发布验收若要求 OpenRouter 强覆盖，必须通过 `--openrouter-key` 或配置文件提供。

## 3. Gateway TOML 配置约定

真实模型验收通过 TOML 配置驱动。建议配置结构：

```toml
gateway_host = "https://example-zone.example"
report_dir = "reports/acceptance"
mode = "gateway"

[environment]
managed_by_devkit = true
group_name = "aicc-acceptance-${run_id}"
group_template = "2zone_sn"
blank_vm_template = "aicc-blank"
cleanup_on_exit = true
keep_on_failure = false

[auth]
token = ""
username = ""
password = ""
login_appid = "buckycli"

[runner]
app_id = "aicc-acceptance"
default_model_alias = "llm.plan"
timeout_ms = 300000
max_attempts_per_case = 3
allow_real_model_calls = false
fail_on_partial = false
matrix_mode = "full_cartesian"
providers = ["openai", "fal", "google-gemini", "claude", "openrouter", "sn-ai-provider"]

[providers.openai]
enabled = true
api_key = ""

[providers.claude]
enabled = true
api_key = ""

[providers.google-gemini]
enabled = true
api_key = ""

[providers.fal]
enabled = true
api_key = ""
image_url = ""
video_url = ""

[providers.openrouter]
enabled = true
api_key = ""

[providers.sn-ai-provider]
enabled = true
api_key = ""
requires_api_key = false
```

配置规则：

- `allow_real_model_calls` 默认为 `false`。只有显式设为 `true` 才允许发起真实模型调用。
- `matrix_mode=full_cartesian` 时，runner 必须按 `api_type × method × 标准逻辑目录路径 × Provider × model` 生成 L4 用例。
- 兼容旧配置 `matrix_mode=provider_model_cartesian` 时，runner 必须在报告中标记为降级模式，并明确列出未覆盖的 `api_type`、`method`、`logical_path` 维度；发布强覆盖不得使用该降级模式。
- `max_attempts_per_case` 默认为 `3`；只有首轮失败的用例才继续执行第 2 / 第 3 次 attempt。
- Provider `enabled=true` 但缺 key 时，用例标记 `skipped`；发布强覆盖模式下，缺 key 可在 preflight 直接失败。
- `google-gemini` 对应 AICC 配置中的 `settings.gemini` / `settings.google_gemini` 兼容入口，生效的 `provider_driver` 应归一为 `google-gemini`。
- `sn-ai-provider` 对应 AICC 配置中的 `settings.sn-ai-provider`，`requires_api_key=false`，缺普通 API key 不应导致 skipped。
- Provider key 不写入报告和日志。
- runner 应把最终生效配置的脱敏摘要写入报告。
- `managed_by_devkit=true` 时，runner 负责创建、启动、探测和清理 group；`keep_on_failure=true` 只用于人工排查，报告必须明确标注遗留环境名。

### 3.1 `buckyos-devkit` 临时 group 生命周期

L4 runner 应把被测环境视为一次性资源，推荐流程：

1. 生成唯一 `run_id` 和 `group_name`，例如 `aicc-acceptance-20260511-153000`。
2. 检查 `buckyos-devkit` / `buckyos-devtest`、Multipass、Python、`uv`、`cargo`、`pnpm` 是否可用。
3. 构造或复用空白 VM 模板；如果本次需要多个虚拟机，先构造一个空白虚拟机，再 clone 出 SN、OOD、普通节点等实例，然后按 group 配置修改 hostname、hosts、端口映射和 app 参数。
4. 使用 group template 生成临时 group 配置，最小建议为 `SN + alice-ood1`；需要多 Provider 节点或 gateway 冗余时再扩展节点。
5. 执行 `create_vms` / `install` / `start`，并等待 gateway、system-config、verify-hub、scheduler、task-manager、AICC 全部可访问。
6. 宿主机 runner 通过 gateway 登录并获取测试 token，后续所有 L4 调用都经 gateway 访问 `/kapi/aicc` 和相关 task / artifact 接口。
7. 写入真实 Provider settings，触发 `reload_settings`，调用 `models.list` 并读取最终生效逻辑目录，生成 `api_type × method × logical_path × Provider × model` 矩阵。
8. 运行 L4 矩阵用例并收集报告。
9. 默认执行 `stop` / `clean_vms` 清理临时 group；除非显式 `keep_on_failure=true`，失败环境也必须清理。

清理约束：

- runner 只能清理自己创建且带有本次 `run_id` 标签或命名前缀的 group / VM。
- 清理前应把必要日志、AICC settings 脱敏摘要、`models.list` 输出和失败 attempt 摘要复制到报告目录。
- 清理失败不能覆盖测试结论，应记录为 `cleanup_failed` warning，并列出残留 group / VM 名称。

