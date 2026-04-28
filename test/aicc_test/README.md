# aicc_test

独立远程 AICC 用例执行器（非 `cargo test`），启动后读取 TOML 配置执行。

## 已迁移用例

- 基础远程链路：
  - `kRPC Direct`
  - `AiccClient (Rust)`
  - `TS SDK`
- 从 `aicc_gateway_workflow_remote_tests.rs` 迁移的复杂场景：
  - OpenAI
  - SN AI Provider
  - Gemini
  - Claude
  - 都会执行 `workflow_complex_scenario_protocol_mix`（复杂 DAG + JSON 输出 + v0.2 `llm.chat`）

## 配置文件

默认读取当前目录 `aicc_remote_runner.toml`。  
示例文件：`test/aicc_test/aicc_remote_runner.toml`

关键规则：

- `sn-ai-provider` 始终执行（不需要 api-key）
- `openai/gemini/claude` 未提供 key 时，该 provider 的用例会显示为 `skipped`

## 运行 smoke

```bash
cd test/aicc_test
pnpm test
```

smoke 会按 run 写入 `reports/aicc_smoke/<runId>/`，每个 case 一个子目录，包含 `input.json`、`response.json`、异步任务的 `task.json`（如有）和 `output.json`；总览写入 `summary.json`。图片链路会先运行 `image.txt2img`，所有会返回非结构化 artifact 的请求都会统一要求 `named_object` 输出。后续需要图片输入的用例只传生成图片的 ObjectId。报告阶段不会立即打开 `named_object` artifact，只在全部用例结束后通过 SDK `ndm_proxy.openReader` 打开一次生成图片并保存到 `generated-image/`。可用 `AICC_REPORT_DIR` 覆盖根目录。

## 打印模型目录树

`test_list_models.ts` 调用 aicc 服务的 `models.list` RPC，
拉取当前所有 provider inventory 与聚合后的逻辑路径树，并以 ASCII 目录树打印。
适合在排查路由问题、确认某个 logical path 是否被挂载时用。

```bash
cd test/aicc_test
pnpm run test:models
```

输出包含三段：

- `Providers`：每个 provider 实例下的 `exact_model` 与对应 `logical_mounts`（level-1 物理挂点）。
- `Catalog aliases`：`ModelCatalog` 里注册的 `alias → (provider_type, provider_model)` 映射。
  注：当前生产链路上 catalog 没人调 `set_mapping`，常态是空。
- `Logical directory tree`：按 `.` 分段的统一目录树，叶子节点合并三个来源（参考前缀以区分）：
  - 裸 `xxx@provider`：来自 provider inventory 的 level-1 mount（`logical_mounts`）。
  - `[ref: name → llm.xxx]  (w=权重)`：来自 `SessionConfig.logical_tree` 的 level-2 item，
    指向另一个 logical path。AICC 启动 / reload 时按
    `doc/aicc/aicc 逻辑模型目录.md` 第 4 节、由 `default_logical_tree.rs` **逐字**
    构造 `llm.plan` / `llm.code` / `llm.swift` / `llm.reason` / `llm.vision` /
    `llm.long` / `llm.fallback` 7 个 LLM level-2 节点（含 `opus` / `qwen_max` /
    `deepseek` / `kimi` / `glm` / `grok` 等 item，即便当前 zone 没装对应 provider 也保留）。
    路由时若某 item 解析不到底层模型，router 会按权重降级到下一个候选；item 列表
    本身始终反映设计意图，方便对照 doc 验证树形态。
  - `[alias→ provider_type/provider_model]`：catalog 别名（旧机制）。

## 运行 fal provider 用例

`test_fal.ts` 覆盖 fal provider 提供的 3 个 ai method（`image.upscale` / `image.bg_remove` / `video.upscale`）。
若远端 AICC 未配置 `settings.fal` 或 fal provider 不可用，相关用例会被自动标记为 SKIPPED，不会判失败。

```bash
cd test/aicc_test
pnpm run test:fal
```

可选环境变量：

- `FAL_TEST_IMAGE_URL` — image.upscale / image.bg_remove 输入图 URL
- `FAL_TEST_VIDEO_URL` — video.upscale 输入视频 URL
- `FAL_WAIT_TIMEOUT_MS` — 单用例超时（默认 240000）

退出码：`0` 全部通过；`1` 有失败；`2` 全部 skipped（未配置 fal）。

smoke 用例会通过 `../test_helpers/buckyos_client.ts` 的 `initTestRuntime()` 初始化标准 AppClient runtime，然后从 runtime 获取 AICC 和 task-manager client。

## 运行 remote runner

```bash
cd test/aicc_test
pnpm install
pnpm run remote
```

指定配置文件：

```bash
pnpm run remote -- --config ./aicc_remote_runner.toml
```

## TOML 示例

```toml
gateway_host = "http://192.168.100.136"

[auth]
token = ""
username = "zztestood5"
password = "your-password"
login_appid = "buckycli"

[runner]
model_alias = "llm.plan.default"
app_id = "aicc-tests"
output = "reports/aicc_remote_report.md"
rust_manifest_path = "rust_runner/Cargo.toml"

[api_keys]
openai = ""
gemini = ""
claude = ""
```

## 报告状态

- `✔` 通过
- `!` 部分通过
- `⏭` 跳过
- `✗` 失败

退出码：

- `0` 无失败
- `1` 有失败
- `2` 无失败但有 partial
