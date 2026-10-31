最新状态：[公共修复已部署及定向复测结果](../local-fix-retest/summary.md)。以下保留诊断时证据和原轮次结果；当前跨轮矩阵为 100 通过、64 失败、2 review。

# SN / AICC 逐项诊断结论

本轮没有新增真实模型推理。历史 T2 的 99/166 通过、67/166 失败保持原轮次记录；本页区分接口调用、内容质量及后续只读诊断。

## 1. 制品读取 405

调用链为模型返回 → AICC 写入 Named Store/outputs bucket → TaskMgr 成功结果引用 cyfile → runner 的 NDM Proxy `openReader` → Gateway `/ndm/proxy/v1/read/object/open` → Control Panel 405。405 发生在协议入口，尚未进入 NDN 对象内容读取。当前 Control Panel HTTP handler 没有该 NDM Proxy 路由。

33 项失败按 API 为 txt2img 16、inpaint 3、TTS 1、img2img 13。前 20 项到达结果读取阶段；13 项 img2img 是前置生成物读取失败，目标调用未执行。不能把 33 个失败用例等同于 33 个模型生成失败。

此前将 NDM Proxy 缺失等同于 NDN 不能提供下载不准确。现有 cyfs-dir 加临时配置即可读取原生 NDN 对象；[34 个对象回读报告](../ndn-readback/summary.md)全部成功。仍缺少 NDM Proxy 协议实现；原生 NDN GET 和该 Proxy API 是不同入口。需要使测试和实际提供的读取协议一致，不能仅凭 Proxy 405 改写模型能力。

## 2. Qwen Image 的实际阻断点

txt2img：BuckyOS 使用 SN `/images/generations`；SN/LiteLLM 的 OpenRouter 图片适配把请求送到上游 chat/completions，OpenRouter 404 明确要求 `/api/v1/images`。这是 SN 上游路由/适配缺口，已被现有 [sn-business #49](https://github.com/buckyos/sn-business/issues/49)记录。

img2img：失败的是同模型 txt2img 前置生成。runner 没有向 SN 发起目标 img2img，因此不是 SN 的 img2img 权限/策略主动阻断，也不能称为其目标编辑接口独立失败。

## 3. Pro OCR 是哪里超时

两项均已得到原生 remote task ID，实际 operation 为 responses.background。超时由 `run_gateway.ts::waitForTask` 在收到 running 后等待 TaskMgr 终态超过 900000ms 抛出；这不同于一次同步 HTTP 请求超时。快照中的单次 Provider HTTP 限额为 300000ms，AICC 原生轮询期限为 1800000ms。

保存的 active_poll_elapsed 分别停在 147622ms 和 29438ms。结合 runner 时间估算，两个任务最后推进均约在 09:04:59，与 09:05 的本地数据库锁争用吻合。源码中进度落库 `.await?` 可退出驱动而遗留 active 条目；恢复因 already_active 而跳过。故障注入回归确实复现此缺陷。不能再把这两项简单归为 SN 模型推理超过 15 分钟；历史日志未记录对应任务的原始数据库错误，尚不能证明它们与锁争用之间的唯一因果链，也不能确认上游最后结果/费用。

两任务测试收尾时 AICC 记录已 cancelled；TaskMgr Cancel pending 与 AICC delegated cancel 的 control_already_pending 造成跨服务终态不一致。本轮存储/驱动修复不包含这条 TaskMgr 收尾问题。

## 4. GPT-6.1-Terra 的结论

当前 SN → openrouter/openai/gpt-6.1-terra 路由不能完成本次请求的 background/computer 协议闭环，四个单元不可验收。LLM 和 caption 成功；这不是模型整体不可调用。

OCR、txt2img 返回状态可解析，但 background 并非 true；原始响应未保存，无法进一步区分缺失/null/false。img2img 同模型前置生成失败，目标未发起。computer-use 响应存在 output 数组，但没有可解析的 computer_call。它们均不是 runner 轮询超时。

结论只适用于该 SN 渠道/所选 operation，不能推导原厂不支持。AICC 按既有契约拒绝静默同步降级。需要 SN 核实并提供该实际路由的能力边界：支持则修正转发/响应；不支持则声明渠道限制或换上游，BuckyOS 再把限制落实到 provider 配置，而非修改原厂参数。本轮没有通过重新禁用掩盖失败；对应 issue 草稿单独保存。

## 5. GPT-5.5 制品归属写入

09:05:03 的 bucket bind 日志证明图片已写入；09:05:08，INSERT aicc_artifact_scope 等待 5.008269238 秒、rows_affected=0；同窗口 UPDATE/SELECT 也等待约 5 秒。对象 cyfile:690660ffcdb80fcf8cc8a8285ae4cd902188e4d8a85e11cb18af79e1e6d8f1fe 已通过 Gateway 回读 1,085,451 字节，但 aicc_artifact_scope 中无该对象记录。失败准确发生在内容落盘之后、归属登记阶段。

运行数据库 journal_mode=delete；SQLx SQLite 默认 busy_timeout 为 5 秒。使用相同 INSERT 的隔离 DELETE 模式复现为 5008ms 后 SQLITE_BUSY/code 5；WAL 模式相同读快照下写入成功。原日志通过 map_err(|_| ...) 丢失原始错误，无法历史还原锁持有者或严格证明那次错误代码；此限制保留。

本地回归在修复前分别复现 database is locked 和 active 标记残留；修复后 AICC 789/789 单测通过。WAL 能消除这类读快照阻塞写入，不保证消除所有并发写锁冲突。

## 6. Grok 拒答

任务为 Terminal/Succeeded、finish_reason=stop，输出为 `I can't do that.`。请求仅要求 `Reply with the exact marker BUCKYOS-AICC-4827.`；Provider 返回的 reasoning summary 把它标为 potential jailbreak。它是成功返回的拒答内容，不是 HTTP/JSON/传输失败。

[xAI moderation 文档](https://docs.x.ai/developers/advanced-api-usage/custom-moderation-controls)说明默认策略及可选 decline guidelines；[OpenRouter 模型说明](https://openrouter.ai/x-ai/grok-4.7)说明其防 jailbreak 机制。本次 AICC 请求没有 guidelines，官方文档也未规定禁止普通标记回显。当前证据更符合上游策略疑似误判，但无法区分 xAI/OpenRouter/SN 的具体策略来源；没有新增付费直连对照。不会通过改提示词或放宽断言将原失败改成通过。

## 7. 识别内容

通过认证 Gateway 回读 69 次复核所对应 TaskMgr 任务，全部 Terminal/Succeeded。19 个 OCR 和两个 caption 是内容质量问题；完整 OCR 目标为 `BUCKYOS-DV-4827`。原自动报告为 review，人工严格核验结果见下表。

| 实例 | 模型 | API | 预期与实际 |
|---|---|---|---|
| GLOBAL_OPEN | kimi-k3.1-pro | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKY05-0V-4827 |
| GLOBAL_OPEN | kimi-k3 | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKY05-OV-4827 |
| GLOBAL_OPEN | minimax-m3.1-flash | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKY05-04-4827 |
| GLOBAL_OPEN | qwen3.7-plus | vision.ocr | 预期 BUCKYOS-DV-4827；实际 Based on the image provided, the visible text is:  **ELCFTIDE-DLU-48B27** |
| GLOBAL_OPEN | qwen3.8-flash | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DU-4827 |
| GLOBAL_OPEN | ui-tars-1.5-7b | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BricKM05 - CV - 4827 |
| US_FULL | claude-haiku-5.5 | vision.caption | 预期五个椭圆；实际 four large, overlapping ellipses |
| US_FULL | gpt-5.3-codex | vision.ocr | 预期 BUCKYOS-DV-4827；实际 6IICKY0S-0V-4827 |
| US_FULL | gpt-5.4-mini | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKYNS-DV-4827 |
| US_FULL | gpt-5.4-nano | vision.ocr | 预期 BUCKYOS-DV-4827；实际 E I C K Y D T S E  O I H 4 E S F |
| US_FULL | gpt-5.4 | vision.ocr | 预期 BUCKYOS-DV-4827；实际 6ICKYOS-0U-4827 |
| US_FULL | gpt-5.5 | vision.caption | 预期两片叶子；实际 two pairs of green oval leaves |
| US_FULL | gpt-5.5 | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-0V-4827 |
| US_FULL | gpt-5.6-luna | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BLOCKY5 04-4827 |
| US_FULL | gpt-5.6-sol | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKY06-04-4827 |
| US_FULL | gpt-5.6-terra | vision.ocr | 预期 BUCKYOS-DV-4827；实际 FUCKTOS CV-4827 |
| US_FULL | gpt-6-luna | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BINARYS-04-4827 |
| US_FULL | gpt-6-sol | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BICKYNS-04-4827 |
| US_FULL | gpt-6.1-sol | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-0V-4827 |
| US_FULL | kimi-k3.1-pro | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-OV-4827 |
| US_FULL | kimi-k3 | vision.ocr | 预期 BUCKYOS-DV-4827；实际 BUCKY05-0V-4827 |

## 公共组件修复必要性与验证边界

本轮生产代码触及 AICC 公共存储/执行器，不能描述为“仅 SN 私有适配器修改”。Provider 配置无法控制本地 SQLite 事务，也不能清除执行器遗留的 active 标记，因此公共组件修复必要。没有新增第三方依赖或通用组件，没有更改对外协议、字段或存储 schema。

- `storage/mod.rs`：SQLite 连接启用 WAL，新增持有读快照时归属登记的回归。
- `execution/mod.rs`：驱动退出的错误路径统一清理 active，验证同一远程任务可恢复且不重新提交推理。
- `service/inference.rs`：后台驱动错误不再静默丢弃。
- `service/provider_execution.rs`：归属登记失败日志保留底层数据库错误。

验证：`cargo test --manifest-path src/Cargo.toml -p aicc --lib -- --test-threads=1` 789/789；`git diff --check` 通过。未部署本轮公共组件修复，未运行修复后 T1.5/T2、完整 workspace 测试或完整构建；不得用原有 1219+102 项 T1.5 结果冒充本轮公共修改已完成 E2E 验证。运行实例仍使用上一轮二进制和 DELETE 模式数据库，NDN 临时路由已恢复。

未来提交记录/PR 描述须明确标记：本变更修改 AICC 公共 SQLite 存储与原生异步驱动，原因是可复现的读写锁冲突和持久化失败后的 active 残留，不能仅靠 provider 参数修复。本轮未创建 Git commit 或 PR。

相关 cmd_skills 的命令、canonical API 和路径未变化；手册不需要复制 SQLite 实现细节，原动态发现示例保持有效。

## SN issue 状态

Qwen Image/Wan 已向 #49 追加本轮证据；Terra 已创建 #53。Grok 策略来源未定、内容识别质量、本地 Proxy/SQLite 问题没有冒充已确认的 SN 转发 bug。已提交的完整正文与元数据见 [issue 草稿](../issue-drafts/summary.md)。已提交 #49 补充回复及 Terra #53，均已回读核验。

本轮回归覆盖本地存储并发与执行器故障注入；这不是 Provider wire 协议问题，没有用修改 Mock 的上游协议预期掩盖本地故障。
