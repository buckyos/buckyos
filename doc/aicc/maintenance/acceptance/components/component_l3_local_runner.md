# AICC L3 本地 Runner 组件

定义本地 kRPC Mock 验收 runner 的 preflight、settings reload、models.list、L3 suite 执行、报告和清理。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 配置与重载验收

必须覆盖完整闭环：

```text
system_config 写入 settings
  -> service.reload_settings
  -> models.list
  -> route
  -> provider call
  -> usage / trace
```

用例：

1. 写入 Mock OpenAI settings，`base_url` 指向本地 Mock，reload 后 `models.list` 出现 `openai-mock-1`。
2. 禁用 Provider，reload 后候选消失，调用返回无候选或策略拒绝。
3. 修改 Provider capabilities，reload 后 `must_features` 硬过滤结果变化。
4. 修改 `provider_type` 为 `local_inference` / `cloud_api` / `proxy_unknown`，验证 `local_only` 过滤。
5. 全量覆盖 settings 和局部更新 settings 都能生效。
6. settings 非法时 reload 失败，不破坏上一版可用配置。
7. `provider.validate` 只做校验和脱敏诊断，不写入 system_config。
8. `provider.add` 写入后 reload，`models.list` / `provider.list` / `provider.health` 可见，且路由可命中新增 Provider。
9. `provider.refresh_models` 更新 inventory revision 后，`models.list` 中 exact model、`api_types`、`logical_mounts` 和 metadata resolver 结果同步变化。
10. `provider.delete` 后 reload，候选和 provider list 中删除目标消失；若仍被 locked policy 引用，必须返回明确错误或诊断。

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

## 3. 预检与清理流程

统一 runner 执行前应做 preflight：

1. 确认当前工作目录和仓库根目录。
2. 确认必要命令存在：`cargo`、`uv`、`pnpm`、`deno` 或 `node`。
3. L3 前确认 BuckyOS 已启动，`uv run src/check.py` 返回可用状态。
4. 检查 AICC 服务是否可访问。
5. 检查 task-manager 是否可访问。
6. 检查 Mock Provider 端口是否可用；如端口占用，自动选择新端口并写入临时 settings。
7. 校验 fixture manifest。
8. 创建本次 `run_id` 和报告目录。
9. 如果是 L4，确认 `allow_real_model_calls=true`，否则跳过真实调用。
10. 如果是 L4，确认 `buckyos-devkit`、Multipass 和临时 group template 可用。
11. 如果是 L4，创建或 clone 临时 group VM，启动后通过 gateway 完成登录和 `/kapi/aicc` 连通性检查。
12. 如果是 L4，调用 `models.list` 并读取最终生效逻辑目录，生成 `api_type × method × logical_path × Provider × model` 矩阵，并在真正执行前把矩阵摘要写入报告。

执行后应做 cleanup：

1. 停止 runner 启动的 Mock Provider。
2. 恢复或清理测试写入的 AICC settings。
3. 清理测试写入的 route overlay/settings。
4. 清理未完成的 Mock task 或记录到报告。
5. 保留报告、输入、输出和脱敏后的 Provider 请求摘要。
6. 如果是 L4，停止并清理本次 runner 新建的临时 group / VM。
7. 如果 `keep_on_failure=true`，保留临时 group，但必须在报告中写入 group 名、节点名和手工清理命令。

清理失败不能覆盖原始测试失败原因，应作为单独 warning 写入报告。

