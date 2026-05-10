# Workflow Idempotency Review

## 结论

P0: Workflow 当前把 `idempotent` 建模成默认 true 的弱声明，这对外部 service 节点不安全。外部 service 是否幂等经常不可证明，甚至同一个接口在不同参数、不同 provider、不同后端版本下语义都可能变化。幂等性必须是强声明：只有明确标注为幂等的节点，才允许自动 retry、跨 Run cache 命中、失败后自动重新调度；未声明或不确定时应按非幂等处理。

## 当前代码事实

- DSL 层 `StepDefinition.idempotent` 默认 true：`src/kernel/buckyos-api/src/workflow_dsl.rs:47`。这意味着 DSL 作者不写字段时，引擎会乐观认为节点幂等。
- 编译层把 step 的 `idempotent` 原样放进 `CompiledNode` 和 `Expr::Apply`：`src/kernel/workflow/src/compiler.rs:388`、`src/kernel/workflow/src/compiler.rs:471`。
- 执行层在 `schedule_apply` 里用 `compiled.idempotent` 查 cache：`src/kernel/workflow/src/orchestrator.rs:548`。
- 执行成功后用 `is_idempotent(compiled)` 写 cache：`src/kernel/workflow/src/orchestrator.rs:702`。
- Thunk 失败后的自动 retry 只看 `retry.max_attempts`，不看 `idempotent`：`src/kernel/workflow/src/orchestrator.rs:275`。
- direct adapter 失败后的自动 retry 也只看 `retry.max_attempts`，不看 `idempotent`：`src/kernel/workflow/src/orchestrator.rs:737`。
- 人工 `retry` 只检查节点状态是 `WaitingHuman | Failed`，不检查非幂等：`src/kernel/workflow/src/orchestrator.rs:403`。
- rollback 已经有一处非幂等保护，会阻止越过已经完成的非幂等下游节点：`src/kernel/workflow/src/orchestrator.rs:452`。这说明代码已经承认非幂等节点是恢复边界，但 retry 路径没有跟上。
- `ThunkObject` 没有 `idempotent` 字段：`src/kernel/buckyos-api/src/thunk_object.rs:84`。`build_thunk` 还显式忽略了 Apply 里的 `idempotent`：`src/kernel/workflow/src/orchestrator.rs:2138`。因此调度器侧无法基于幂等性做保护。
- AICC adapter 的方法 schema 有 `idempotent`，但注释说明最终是否 cache 仍由 Step `idempotent` 字段决定：`src/kernel/workflow/src/adapters/aicc.rs:62`。因此方法级 schema 目前只是作者参考，不是执行保护。

## 风险

P0: 非幂等 service 节点失败后被自动 retry，可能重复提交外部任务、重复扣费、重复发消息、重复写入状态，甚至在服务抖动时放大成雪崩。

P0: 默认 true 会把“不知道是否幂等”的节点误分类为幂等。对外部 service 来说，不知道不等于安全；正确默认值应该是 false 或 unknown，只有强声明才能进入 retry/cache 路径。

P1: AICC 这类 provider 调用尤其敏感。某些能力看起来输入相同，但 provider 可能生成不同资源、产生新 task、消耗额度或触发外部副作用。方法表里的默认幂等性不能替代 workflow step 的显式声明，更不能让缺省 step 自动获得幂等待遇。

P1: 人工 retry 也不能无保护地重放非幂等节点。人类可以决定补偿、提交手工结果、跳过或重建后续计划，但“重新执行同一个外部调用”应要求额外确认或专门的 force 语义。

## 建议

1. 把 DSL 默认改成非幂等：`idempotent` 不写时应按 false 或 unknown 处理。更理想是拆成 `idempotency: explicit_idempotent | non_idempotent | unknown`，避免 bool 表达不出“未声明”。
2. 自动 retry 必须 gated by strong idempotency：`retry.max_attempts > 1` 只对显式幂等节点生效。非幂等或 unknown 节点第一次失败后应进入 `WaitingHuman` 或 `Failed`，不能自动置回 `Pending`。
3. cache 只允许显式幂等节点使用。当前 cache 逻辑应从 `compiled.idempotent` 改成“强声明 idempotent”，避免默认 true 带来的误命中。
4. 人工 `retry` 对非幂等/unknown 节点应被拒绝，或要求一个单独的高风险动作，例如 `force_retry_non_idempotent`，并记录审计事件。
5. `ThunkObject` 或 thunk metadata 应携带幂等性声明，让 scheduler/runner 侧也能做同样判断，避免 workflow 层和调度层语义断裂。
6. AICC method schema 的 `idempotent` 可以作为 UI/DSL lint 提示，但不应自动覆盖 step 声明。若要自动补全，也必须在生成 workflow 时显式写入，不能靠运行时默认。
7. 静态分析增加 warning/error：外部 executor（`service::` / `http::` / `appservice::`）如果缺少强幂等声明却配置了 retry，应直接报错或至少 P0 warning。

## 期望行为

- 纯幂等 thunk：显式声明后允许 cache 和 retry。
- 外部 service 节点：默认非幂等，不自动 retry，不跨 Run cache。
- 非幂等节点失败：进入人工处理，让人选择 submit output、skip、abort、amend plan 或显式高风险重试。
- 已完成的非幂等节点：继续作为 rollback / replay / resume 的恢复边界。

## Summary

Workflow 现在已经有 `idempotent` 字段和部分使用点，但语义是“乐观默认 + 局部生效”。这对外部 service 不够安全。幂等性应该变成强契约：不声明就不重试、不缓存、不自动重放。否则一次 provider 抖动或外部服务 5xx，就可能把 workflow 引擎变成重复调用放大器。
