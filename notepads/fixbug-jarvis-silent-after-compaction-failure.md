# fixbug：Jarvis 收到消息后不回复（历史压缩失败卡死会话）

## 现象

2026-11-17 Windows DV（Zone `lzctest`）：对 Jarvis 说"你介绍一下你自己吧"，没有任何回复，连"收到，开始处理…"占位都没有。会话 `ui-20261117-3` 的 `last_error` 是 `llm context error: compress history: llm output parse failed: compress: summarizer returned empty text`，supervisor 每 30 秒重载一次，每次 70–130 秒、约 $0.0096，无上限。

## 为什么会有这个 Bug

三处叠加，缺一处都不会卡死：

1. **`read` 把二进制当文本。** 上一轮里 `llm_understand_media` 识图失败（见 3），Agent 改用 `read wechat_list.png`。`ReadFileTool` 直接 `String::from_utf8_lossy` 整个文件，155KB 的 PNG 变成约 3 万 token 的乱码进了 worklog。对象读取（`ObjectBackend`）早有 NUL 字节判断，本地文件没有。
2. **摘要器的输出额度被思考吃光。** 新消息到来时历史约 35.5K token，超过 24K 预算，触发 `llm.summarize`。AICC 按元数据把 `qwen-flash` 规格绑定为 `qwen3.8-flash:xhigh`；压缩请求 `max_completion_tokens = 6144`，`reasoning_tokens = 6144`、`finish_reason = max_output_tokens`，正文为空。经规格选中的思考档请求方改不了。
3. **压缩失败没有出路。** `history::compact` 把失败直接变成会话错误（不可恢复），轮次没开始、输入不 ack、用户收不到任何东西；supervisor 对出错会话按固定 30 秒 `recheck`，同样的历史同样失败，循环到天荒地老。
   - 同类问题：识图走 `llm.vision → kimi-k3.1-pro:max`，`llm_understand_media` 默认输出预算 2048，思考同样吃光，返回 `no JSON object found`——这正是 Agent 改用 `read` 读 PNG 的起因。

## 怎么修的

- **A. 本地文件也拒绝二进制**（`agent_tool/src/file_tools.rs`）：前 8192 字节有 NUL 即拒绝，提示用 `llm_understand_media` 或 shell；与 `read_tool.rs` 的对象读取共用 `looks_binary`。不拒绝非 UTF-8 文本（Windows 上的 GBK 文件照旧有损读出）。
- **B1. 压缩失败的一次性降级**（`llm_compress::trim_tool_results`，`lib_opendan/src/runner/history.rs`）：摘要器失败后，把最新一轮之前超过 `REPORT_CHARS`（4096 字）的工具结果从最老的轮起截短；装进目标就用于这次上下文（日志 `opendan.context.compaction.fallback`），不写 summary.json，下次照常先跑 LLM 摘要；仍装不下才报原错误。设计文档 `doc/llm_context/LLM Compress.md` §8 已同步。
- **B2. 出错会话的重载退避**（`lib_opendan/src/host/mod.rs`）：`HostedStatus.errors_in_a_row` 计连续出错次数，`recheck` 从 30 秒起每次翻倍，最长 8 分钟；新消息（`ensure_task`）不受影响，立即处理。
- **C. 廉价用途不走高思考档**：`qwen3.8-flash` / `qwen3.8-omni-flash` 在 `qwen-flash` 规格下改绑 `:low`（该规格只被 swift/summarize/translate 和回退 4.0 层引用，`default_effort` 不变，家族直选仍是 xhigh）；`qwen.model.json` `revision_seq` 11、`BUILTIN_CATALOG_REVISION_SEQ` 26；维护规程补了一条。`kimi-flagship` 被 chat/plan/code 共用，按规程不改它的档位，改为把 `llm_understand_media` 的默认及最低输出预算提到 16384。

## 验证

在 `src/` 下（Windows 11）：

- `cargo test -p agent_tool --lib`：新增 `read_file_refuses_binary_files`、`trim_clips_older_tool_results_until_the_target_is_met`、`media_output_budget_leaves_room_for_thinking` 通过；`xllm::vectors::compress` 通过（`trimmed_results` 为 0 时不序列化，与 TS 端共用的向量不变）。另有 10 个失败（`grep_tool`、`zone_file`、`read_tool` 两个、`xllm::tests` 两个、`xllm::vectors::runtime_identity`），改动前同样失败，都是 Windows 路径分隔符 / `\\?\` 前缀问题。
- `cargo test -p libopendan --lib host::`、`--test l1`（新增 `failed_summary_clips_older_tool_results_without_a_summary` 通过）、`--test context_limit`（`failed_compaction_logs_the_failure_and_preserves_the_error` 仍通过）。`l1::missing_tool_or_runtime_mismatch_fails_before_inference` 与 `context_limit::a_multi_turn_work_compacts_at_the_window_line_and_goes_on`（依赖 `/dev/zero`）改动前同样失败。
- `cargo test -p aicc --lib`：792 通过；`storage::*`、`service::cloud_update::*` 共 11 个失败，改动前同样失败（Windows 文件“拒绝访问”）。
- `cargo check -p opendan` 通过。

未验证：未部署到 DV 复现；Linux/macOS 未跑；SN 上 `qwen3.8-flash:reasoning-low` 的真实摘要质量与耗时未实测。部署前旧会话会继续失败（现在按退避重试）；部署后同一会话在下一次重载或用户发新消息时应能回复。

受影响的 skills：无需修改。手册没有写 `llm_understand_media` 的默认预算、`read` 对二进制的行为或 `qwen-flash` 的思考档；`read` 的报错本身指向 `llm_understand_media`。websdk 的 TS xllm 运行时（`src/xllm/hosts/deno/native_tools.ts` 的 `readFileTool`）有同样的二进制读取问题，本次未改。
