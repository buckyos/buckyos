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
  - 都会执行 `workflow_complex_scenario_protocol_mix`（复杂 DAG + JSON 输出 + stream）

## 配置文件

默认读取当前目录 `aicc_remote_runner.toml`。  
示例文件：`test/aicc_test/aicc_remote_runner.toml`

关键规则：

- `sn-ai-provider` 始终执行（不需要 api-key）
- `openai/gemini/claude` 未提供 key 时，该 provider 的用例会显示为 `skipped`

## 运行

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
