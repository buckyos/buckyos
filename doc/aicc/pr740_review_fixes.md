# PR #740 review 修复（2026-11-17）

## 修复范围

- 同步 provider revision 断言；重新登记闲时 ASR 的 golden binding（132 条）及派生 adapter 的 operation 数（8 个），同步相关注释。
- 原厂 Seedream 5.0 Pro、Flash 和 5.5 声明 `capabilities.max_images: 1`，移除 Flash/5.5 的 `/n` omit 映射。缺省数量和 `n=1` 保持单图；`n>1` 明确失败，不能静默生成一张图。
- 豆包私有 image codec 仅在 `n>1` 时生成组图字段，`n=0` 拒绝。组图模型仍保留 `auto` 和 `max_images` 数量语义。
- Ark 数据面目录接受当前官方示例中的 PascalCase 和已核实真实响应中的 snake_case；未知类型记录诊断并保持不可路由，混合类型仅保留已识别能力。
- 增加标准渠道 Flash 的 inventory 原厂身份断言；补充两种任务类型写法、混合/未知值，以及单图和多图边界回归。
- 两条标准 Seedance 2.5 operation/request rules 合并为明确列举普通版与 Flash 两个已知 ID 的匹配规则，不对未来型号使用宽泛 pattern。
- 闲时 ASR operation ID 提取常量；删除重复官方来源 URL。
- T1.5 为两个渠道的五个单图物理 ID 增加 `n=4` 本地拒绝用例，要求稳定的 `invalid_request` 且 Mock 收到零条上游请求；Mock 对这些型号继续禁止全部组图参数。
- 保留统一 discovery 契约选择，并补验 qwen/typesafe 的网关发现路径。

## 公共组件修改与必要性

本轮确实修改了公共生产逻辑：`src/frame/aicc/src/call/mod.rs` 在 canonical 字段转换之前检查请求图片数量是否超过模型/渠道的有效 `max_images`；`src/frame/aicc/src/catalog/validation.rs` 校验原厂上限为正整数，并允许渠道收窄该上限。

原因是原先将 `n` 删除后，豆包 codec 已无法区分单图和被降级的多图请求。数量约束是原厂模型事实，应在请求语义仍完整、且原厂能力与渠道上限已经合并的位置统一校验。现有 CodecContext 不携带模型能力，不增加通用组件或协议字段、不复制原厂限制到 provider options 的最小实现是在已有 lowering 增加该校验。只有声明 `max_images` 的模型受影响；当前该事实仅对上述豆包单图模型新增。未声明上限的其他模型行为不变。渠道数值上限不能提高原厂上限。

协议字段转换仍在豆包私有 codec，不按模型名称硬编码选择程序分支。未修改路由算法、外部 RPC/SDK 请求结构、共享类型、存储结构或第三方依赖。

原厂约束由标准与 Agent Plan 的现有身份映射共同继承，无需在 provider 文件中重复原厂参数。两渠道都进入本轮 Mock 回归；本轮不重新执行真实 T2，原有账号未开通的两个模型继续保留为未通过，历史成功记录不扩展为新实现的真实验证结果。

## 官方依据

- [图片生成 API](https://docs.volcengine.com/docs/ark/image-generation-api?lang=zh)：Pro/Flash/5.5 不支持组图参数；支持组图型号的默认模式为 disabled。
- [Models API](https://docs.volcengine.com/docs/ark/models-api?lang=zh)：数据面同一 endpoint 的官方响应示例使用 TextGeneration、VisualQuestionAnswering。
- 已提交的 `test/aicc_test/acceptance/fixtures/doubao-models.json`：真实数据面响应使用 snake_case。

## 手册联动

检查 `aigc_image`、`aigc_audio`、`aigc_video`、`xllm`：无新增用户命令、参数名称或资源交付路径；单图数量限制由 metadata 声明并在 AICC 显式拒绝，原有手册通过实时模型查询发现能力。按用户要求不修改 Jarvis 提示词或手册，数量契约在 `driver_metadata_schema.md`、`provider_profile_schema.md` 同步说明。

## 本轮验证

- `cargo test --manifest-path src/Cargo.toml -p aicc --lib -- --test-threads=1`：800/800 通过，涵盖全部 AICC lib 单测。
- 验收自测：165/165 通过（包含健康恢复回归）。
- 静态 preflight：通过，完整 T1.5 manifest 1366 项。
- Deno 类型检查：run_t15_gateway、t15_mock_provider、provider_protocol_contracts 通过。
- `cargo build --manifest-path src/Cargo.toml -p aicc`：通过。
- 统一网关 T1.5 已完成：豆包标准与 Agent Plan 268/268、speech 48/48、qwen/typesafe 270/270；运行时包含独立预设等派生单元，合计 586/586 通过。
- 未运行全 workspace 测试或完整系统构建；本轮真实模型调用数为 0。


## 网关回归收集与测试 runner 修正

新进程首轮 qwen/typesafe 共 270/270 通过（qwen 263、typesafe 7），speech 48/48 通过，临时 Provider 清理均通过，真实模型调用为 0。

豆包首轮收集到两个测试 runner 健康恢复失败：多图拒绝用例本身已经返回预期 invalid_request、且没有发出上游请求，但自动健康恢复探测复制了负向用例的标记，因而再次提交 n=4。它错误地把正确的本地拒绝当作恢复失败，并提前跳过了剩余用例。

将已有恢复用例构造提取为 buildHealthRecoveryCase，恢复时同时清除官方错误、canonical 拒绝和多图拒绝标记，使用正常的 n=1 成功请求。新增回归断言验证恢复请求、错误预期和模型身份。非法请求的错误码与零上游请求断言保持不变；没有放宽 Mock，也没有修改产品实现，因此无需再次 build/deploy。该修正后，165 项自测、类型检查和 preflight 通过，再完整复跑相同的全部渠道范围。最终完整复跑结果见下，三组全部通过。


## 最终版 runner 的完整复跑

豆包标准与 Agent Plan：`t15-20261117075326-2208003`，268/268 通过（标准 109、Agent Plan 159）。五个 `reject-multiple-images` 单元全部通过，均记录 `captured_requests=0`；其后的合法单图健康恢复探测也通过。临时 Provider 清理通过，真实模型调用为 0。

安装二进制 SHA256：`ed1090474a3dafb6dcfedcd15f6e0691314ab9cbc797c8a40e55a841d7c22589`，备份为 `/opt/buckyos/bin/aicc/aicc.before-pr740-review-20261117`。本轮只执行一次产品 build/install/restart；后续健康恢复修正仅涉及测试 runner，没有再次修改部署的二进制。


speech 最终复跑：`t15-20261117075801-2212944`，48/48 通过，临时 Provider 清理通过，真实模型调用为 0。


qwen/typesafe 最终复跑：`t15-20261117080115-2216435`，270/270 通过（qwen 263、typesafe 7），临时 Provider 清理通过，真实模型调用为 0。三组最终总计 **586/586** 通过；从健康恢复修正到这三组完整通过之间未再修改测试实现或产品代码。

本次审查的十条行内意见均已落实到实现、回归或文档清理；原始详细报告保留在被 Git 忽略的本地目录中，不提交凭据、逐项原始报告或制品。


## 2026-11-18 rebase 最新 main

基于 `buckyos/buckyos:main` 的 `20da9c312a98c458c866381d3306ae240c9f4479` 重放本 PR 的四个提交。冲突来自上游模型目录升级：保留上游 Seed 模型原厂 reasoning 参数、渠道 reasoning 变体与新的 DeepSeek/Qwen provider revision，并保留本 PR 的 Economy 独立模型身份、库存变更和 review 修复。清理重叠的标准渠道映射；Agent Plan provider revision 递增为 14，内置目录 revision 递增为 26。

上游新增回归对 `deepseek-v4-1-flash-economy-261017` 的原厂身份预期同步为本 PR 新增的 `deepseek-v4.1-flash-economy`。首轮网关 T1.5 执行 322 项、286 项通过，36 项均被旧 Mock 的视觉 reasoning=minimal 固定断言拒绝。依据[官方深度思考文档](https://docs.volcengine.com/docs/ark/deep-thinking?lang=zh)（2026-11-18 核对），Responses 的 reasoning.effort 接受 none/minimal/low/medium/high/xhigh/max；Mock 改为校验官方枚举并继续拒绝未知值。runner 原有 variantWireDiagnostics 仍按每个变体的官方预期精确校验字段，未取消或放宽该断言。补充标准及 Agent Plan 两渠道 OCR/caption 的七档成功和非法档位拒绝自测。

此次冲突处理没有新增公共生产逻辑修改、依赖、RPC/SDK 字段或存储结构变化；未修改 Jarvis 提示词或手册。手册联动判断同上，模型事实仍通过实时目录查询获取。本轮仅构建并部署一次 AICC，备份为 `/opt/buckyos/bin/aicc/aicc.before-pr740-rebase-20261118`；Mock 修正只涉及测试代码，无需重新部署。

静态验证：AICC lib 809/809、验收自测 166/166、preflight、Deno 类型检查及 AICC build 通过。未执行全 workspace 测试或完整系统构建，未执行真实 T2。


最终网关 T1.5：标准与 Agent Plan `t15-20261118040345-3439565`，322/322 通过（标准 127、Agent Plan 195），包括合入上游后的新增 reasoning 变体；五个多图拒绝用例均记录 captured_requests=0。speech `t15-20261118040012-3435578`，48/48 通过。两组临时 Provider 清理均通过，真实模型调用为 0。


qwen/typesafe 首轮在 Qwen inventory 准备阶段停止：运行目录中 2026-10-30 的本地 `models/qwen.model.json`（revision 9）和 `providers/qwen.provider.json`（revision 13）优先于新内置目录，遮蔽了上游新增 `qwen4-max`。该轮 typesafe 7 项通过，Qwen 尚未执行协议用例，不能算作回归通过。临时将两份旧本地覆盖移出加载目录、重新加载设置后，对内置目录重新执行同一范围；测试后按 SHA256 核验并原样恢复文件及重新加载。未修改 Qwen 生产实现或扩大 PR 的模型升级范围。


最终 qwen/typesafe：`t15-20261118041253-3449589`，278/278 通过（qwen 271、typesafe 7），包含上游新增 Qwen 变体。临时 Provider 清理通过，两份本地 Qwen 覆盖已按原 SHA256 恢复并重新加载设置。本次 rebase 最终三组完整回归合计 **648/648** 通过，真实模型调用为 0；从 Mock 修正到最终完整通过之间未再修改产品或测试实现。
