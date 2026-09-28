# Canvas E2E 语义评估试验

本目录给 [Canvas 浏览器 E2E](../../doc/ai-canvas/e2e/execution-guide.md) 增加可选的语义复核。
范围是测试工具；不改变 Canvas 代码、Agent 适配器、AICC 配置或原有通过/失败结论。
先读 [评估约定与新增场景](../../doc/ai-canvas/e2e/semantic-evaluation.md)。

## 已有能力

`cases.mjs` 从本地浏览器导出与 UI 记录提取四个观察样本，并生成八个对照样本。
输入须位于 `doc/ai-canvas/e2e/runs/2026-09-28/browser-journey/`，该目录由 Git 忽略，
需要执行 Agent 的本地证据包；仅检出仓库代码时不会自动生成样本，也不会调用供应商。
加载时对照本地 manifest 检查 SHA-256 与文件大小。`reference.json` 存参考标签、名称、来源、
构造方式与理由；`request.json` 只包含观察和判据，不包含标签、参考理由或历史 PASS/FAIL。
参考标签由 Agent 起草，待人工评审，不能声称是已认可的人工金标。

`evaluator.mjs` 提供三种显式模式：`prepare` 离线准备、`live` 单次真实调用、`replay` 离线重放。
没有自动联网、重试或供应商回退。输出必须是 `/root/app/buckyos/.work` 内一个尚不存在的目录，
上层目录需先创建；拒绝覆盖既有结果。需要 Node 24，复用现有 TOML 解析器，没有新依赖。

从仓库根执行离线准备与工具测试。缺少本地证据包时，工具返回 `E_EVIDENCE_MISSING`，
证据依赖的校准检查标记跳过；这不等于 Jev 已验证：

```bash
node --test test/canvas_e2e/evaluator.test.mjs
node test/canvas_e2e/evaluator.mjs prepare --provider openrouter --out /root/app/buckyos/.work/canvas-jev-prepared
```

12 个样本共享一个 `state`，每个 question 明确指向对应样本；这是一批小型配对校准数据，
存在共享上下文与模板相似性，不是独立留出集，不能据其准确率发布质量保证或确定阈值。
源码中的伪响应仅验证解析器和统计逻辑，绝不能作为 Jev 实测输出。

## 凭据与一次真实调用

凭据放 `.work` 下本地 TOML，限制文件权限为当前操作者可读。可复用已有 AICC 验收 TOML 中
相同字段，但这里不会改写 AICC 设置。仅提供配置文件路径，不把 key 放命令参数或文档。
例如文件 `/root/app/buckyos/.work/canvas-jev.local.toml` 内容为：

```toml
[provider_credentials.openrouter]
api_token = ""
```

选择 TypeSafe 时改用 `[provider_credentials.typesafe]` 与 `--provider typesafe`。
操作者在本地填入 key；提交或展示证据前不得复制该配置。只有非空 key 且显式 `live` 才会调用。
当前仅接入两个固定官方 HTTPS 端点，禁止跟随重定向；模型版本变化会报 `E_MODEL`，需核对官方
文档后更新固定身份。provider 不能由被测页面或资料决定。

在已获当次真实调用授权、核对 `prepare` 输入后，将下列命令放入 `.work` 脚本，按项目规则
用 `setsid nohup bash ... > ... 2>&1 < /dev/null &` detached 启动并轮询：

```bash
node test/canvas_e2e/evaluator.mjs live --provider openrouter --config /root/app/buckyos/.work/canvas-jev.local.toml --out /root/app/buckyos/.work/canvas-jev-live
```

每次进程最多一次 HTTP 调用、12 个问题、请求 24 KiB、响应 64 KiB、45 秒超时、无重试。
超时不代表上游已取消或没有计费；次数与大小限制也不是美元硬限额。若需要费用硬上限，应使用
供应商侧受限额度凭据。报告保留用量与供应商返回的费用，未返回记 `null`，不记为零。
再次运行是另一次付费尝试，应使用新输出目录并纳入运行预算。

这是外部测试评估器的供应商直连，不是 AICC T2/T3 验收，也不验证 Canvas 的真实模型路径。
后续走 AICC 时复用 [decision.evaluate](../../doc/aicc/decision_api.md)、正常 ZoneGateway
和 AICC 验收技能，不直接访问内部服务端口；本轮不启动服务、部署或更改设置来补齐该路径。
现有 `acceptance/judge.ts` 面向生成式 LLM JSON 评分，因此本工具没有把 Jev 伪装成该接口。

## 结果复核

保存 `request.json`、`reference.json`、`started.json`、`report.json`；成功评估另存 `response.json`。
请求哈希绑定返回值与重放输入。仅保存已校验的回答、概率、置信度、模型与用量字段；供应商错误
正文和未知字段不入报告，避免泄露凭据。问题/选项缺失、概率非法、模型变化或超时统一为
`EVALUATOR_ERROR`，不得改成产品 FAIL，也不得猜测修补响应。

```bash
node test/canvas_e2e/evaluator.mjs replay --provider openrouter --response /root/app/buckyos/.work/canvas-jev-live/response.json --out /root/app/buckyos/.work/canvas-jev-replay
```

对照报告给出混淆矩阵、逐条名称和标签、不该支持却支持的样本清单。`productVerdict` 始终为
`UNCHANGED`；没有通过阈值，没有自动修改浏览器测试结论，也没有自动发布 issue。
执行 Agent 根据分歧回看原始证据，必要时补做浏览器动作；Jev 没有自由文本理由输出，报告中的
解释来自参考标注与证据复核，不能冒充模型解释。
