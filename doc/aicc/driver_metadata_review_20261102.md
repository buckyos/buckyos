# SN 新增原厂模型参数复核（2026-11-02）

本次新增原厂定义为 5 个；SN 新增目录条目还包含别名和推理预设，不能按目录条目数算原厂模型数。复核基于以下原厂公开说明。原厂事实放在 Model Driver；SN/OpenRouter 输出额度继续保留在各自 Provider Rules，不回写原厂文件。

| 原厂模型 | 核实参数 | 配置结论 |
|---|---|---|
| GPT Image 2.5 Flare、Sunburst | 文本/图像输入，图片输出；无流式、工具调用、结构化输出；quality=auto/low/medium/high/xhigh/max；size=auto 或自定义，边长为 16 倍数且 ≤3840，长宽比 ≤3，像素数 655360–8294400，超过 3686400 为实验规格；n=1–10；背景 auto/opaque/transparent；PNG/JPEG/WebP | 补齐 `image_generation_parameters`，显式声明 tool_call/json_schema=false。保留 txt2img/img2img/inpaint；原厂 OpenAI 的 generate/edit 映射和 SN 映射无需改变 |
| Kimi K3.1 Pro | 上下文 1048576，最大 completion 1048576（默认 131072），推理 low/high/max、默认 max；temperature=1、top_p=.95、n=1、两种 penalty=0；K3 不使用 K2 的 thinking 参数 | 保留长度和推理档位，补齐固定采样描述；native Kimi、SN、OpenRouter 同步移除固定参数和 K2 thinking，让上游使用官方固定默认值。SN/OpenRouter 943718 输出限制不改 |
| GLM-5.3-FlashX（及同源 Flash） | 总参数 320B、激活 18B；上下文 1M、输出 128K；必须开启思考；low/high/max，默认 max | 修正 parameter_scale=320B，补齐激活规模和 thinking_required；已有 native GLM、SN、OpenRouter 操作/推理 lowering 保留。Coding Plan 尚未开放 FlashX，不据此宣称所有 GLM 渠道可调用 |
| Grok 4.7 | 上下文 500000；官方明确无文本输出硬上限；low/medium/high/xhigh、默认 high；temperature 推荐 .7，API 接受 0–2，实际采样不高于 1；top_p 推荐 .95、实际采样不高于 .95 | 原厂不填写人为输出硬上限，补齐采样范围；128000 是默认输出预算，不能写成原厂最大输出。SN/OpenRouter 的 450000 仍仅属于渠道配置 |

来源：

- [OpenAI Flare](https://developers.openai.com/api/docs/models/gpt-image-2.5-flare)、[Sunburst](https://developers.openai.com/api/docs/models/gpt-image-2.5-sunburst)、[图片参数指南](https://developers.openai.com/api/docs/guides/image-prompting)、[Generate API](https://developers.openai.com/api/reference/resources/images/methods/generate)。
- [Kimi K3.1 Pro 快速开始](https://platform.kimi.com/docs/guide/kimi-k3-1-pro-quickstart)、[模型参数比较](https://platform.kimi.com/docs/api/models-overview)、[Chat API](https://platform.kimi.com/docs/api/chat)。
- [GLM Flash/FlashX 原厂说明](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5.3-flash)、[Z.ai 对应说明](https://docs.z.ai/guides/vlm/glm-5.3-flash)。
- [xAI Grok 4.7](https://docs.x.ai/developers/grok-4-7)、[采样与输出预算说明](https://docs.x.ai/developers/model-capabilities/text/best-practices)。

图片与采样参数的新增 capability 子树是可展示、可查询的技术描述，不是新增参数校验器。现有请求参数模型不能完整表达 background/compression 等所有原厂选项；本轮未扩展公共请求协议，也未承诺这些选项已端到端适配。T1.5 默认成功请求不等于所有尺寸边界及质量档位均已实测。

检查了引用这些模型的 openai、kimi、glm、openrouter、sn provider：native OpenAI 和 GLM 没有新增渠道收窄项，配置保持；Kimi/SN/OpenRouter 更新固定参数规则和相应严格 Mock。T2 baseline 保留每个有效 API 的覆盖，FlashX 保留独立 T2 单元（已撤销依据不足的 Flash 别名去重），本轮没有真实调用。

受影响手册 `aigc_image`、`aigc_audio`、`aigc_video` 已检查：本轮没有改变 CLI、任务结果结构、下载入口或手册命令，故不修改。没有修改公共运行组件、共享类型、存储或依赖；只更新 metadata、测试及文档。

测试报告仅在本地保存，不随源码提交。

## PR #727 review 修正（2026-11-12）

- GLM-5.3-Flash 保留原厂 low/high/max、默认 max；相较旧配置的默认 high，这是有意的默认行为变更。Doubao 与 Doubao Agent Plan 删除无效的 medium 变体，不保留旧选择别名。FlashX 使用独立 `glm-flashx` spec，不进入 `llm.glm-flash` 候选。
- [Kimi 官方定价](https://platform.kimi.com/docs/pricing/chat)已核实：K3.1 Pro 输入/输出为 13/68 CNY 每百万 token，缓存命中 1.3，5 分钟/1 小时缓存写入为 13/26。原生 Kimi 增加独立定价，不沿用 K3 单价。
- 渠道 `capability_limits` 在原厂未声明硬上限时也生效；已有原厂上限时取最小值。请求 lowering 和额度预算使用有效渠道输出上限，超限请求在发送前拒绝。SN/OpenRouter 的渠道数值不写入原厂模型文件；OpenRouter 精确规则保持现有替换语义。
- native driver 复用 `ExecutionLease` 处理取消和异常退出，可重试存储错误会自动重新驱动已提交任务；恢复优先提交已暂存结果，不重新提交上游推理。SQLite 锁竞争按可重试错误分类。制品归属登记在本地有限退避重试，耗尽后按临时错误报告，不再按配置错误报告。
- SQLite 在创建连接池前启用并核实 WAL，切换期间锁竞争有限退避重试；池连接仅核实模式，不执行模式切换。内存数据库保留 memory 模式。
- Wan 3.0 使用原厂文件中的视频契约，Qwen 精确规则显式指定视频 operation；删除 SN 的无效 native 变体、Sora 残留映射及音频协议契约中的 Kimi 死配置。报告敏感信息检查只豁免具体的 `task-result-artifact` 标记，不再用通用前缀边界排除密钥。

检查了 `aigc_image`、`aigc_audio`、`aigc_video`：现有发现、`resume_aicc_task`、同幂等键重接和下载命令保持有效，服务端恢复增强不改变手册操作，故无需修改手册。未新增依赖或变更公共 RPC、SDK、共享类型、存储 schema。

验证：`cd src && cargo test -p aicc --lib -- --test-threads=1` 通过 788 项；最终补强 rebound 存储重试后，`cargo test -p aicc --lib execution::tests:: -- --test-threads=1` 通过 68 项。`test/aicc_test/` 下的 `node --experimental-strip-types --test acceptance/acceptance.test.ts` 通过 153 项，`node --experimental-strip-types acceptance/preflight.ts` 通过。未运行完整 workspace 测试、完整构建、在线 T1/T1.5 或真实 Provider T2/T3；WAL 在其他文件系统及 PostgreSQL 未实测。

### 完整 T1/T1.5 重跑（2026-11-12）

`uv run buckyos-build.py -s aicc` 构建通过，安装并重启 AICC 后经 Zone Gateway 执行。安装二进制 SHA256 为 `cd9374dcde02041ea79321bfe625fb3971abd800654710bfc062bda487ea648c`。

- T1：184 通过、1 失败；129 个静态 manifest 单元均有结果，另有 56 个路由暴露单元全部通过。失败为 `t1.config.cloud_update_dynamic_catalog` 发布 fixture 时 NDM Proxy 返回 405。本地 run 为 `aicc-t1-2026-11-12T06-26-11-296Z-0bdfbe61`。
- 最终 T1.5：2603 通过、29 失败、392 跳过。覆盖全部 15 个 provider driver，计划中的 3021 个 manifest 单元均有结果，另有 3 个 provider 准备失败记录。392 个跳过包含 70 个渠道策略禁用及 322 个准备失败导致的未执行，不能视为全部协议单元实测通过。SN 为 1149 通过、70 策略跳过、0 失败。
- T1.5 失败分类：19 个错误码断言失败；4 个 Doubao/Agent Plan 视频编辑 duration 契约失败；1 个 Qwen 图片任务缺少轮询 wire；1 个跨渠道制品用例引用已禁用的 SN `wan-3.0`；1 个云 fixture 发布 405；3 个 inventory 契约准备失败。准备失败分别为 Gemini 暴露未声明的 `gemini-2.5-flash-lite:reasoning-none`、OpenRouter 缺少 `openai/gpt-5.6:reasoning-none`、GLM 缺少 `glm-5.3:reasoning-thinking`。这些失败未在本轮修改产品代码或放宽协议断言。
- 两次早期 T1.5 分别被云 fixture 发布和 Gemini inventory 校验中断，报告均保留。runner 调整为保留云 fixture case 的失败并继续，以及隔离 provider 准备失败、明确标记受阻单元和切换用例后继续其他 provider。调整后的自测仍为 153/153。最终 run 为 `t15-20261112063734-3072936`，本地报告在 `test/aicc_test/reports/acceptance/pr727-review-full-t15/`。
- 两轮清理均通过；独立回读确认 settings 无本轮 run ID、runtime 无残留测试 inventory。真实模型调用及费用均为 0。结束后 `uv run src/check.py` 确认 Zone 仍已激活且核心服务可用，保留测试前已有告警。

完整 workspace 测试、完整系统构建、真实 T2/T3、其他文件系统及 PostgreSQL 仍未验证。所有逐 case 报告保留在本地忽略目录。

### 失败归因与测试修正（2026-11-12）

405 来自本机旧版 Gateway 配置：安装文件将 `/ndm/*` 转发到 control-panel，POST 落入静态页面处理器，返回 `Allow: GET, HEAD`，GET 则返回 Desktop HTML。无 token、无效 token、普通用户和 sudo token 都返回同样的 405。仓库当前模板已转发到 nfs-server；临时修正路由后，无效身份返回 401，有效身份的非法对象 ID 返回 400，fixture 发布成功。因此这是安装配置未更新，不是 NDM 权限拒绝，也不是 AICC 更新 metadata 的接口返回 405。诊断临时路由在回归结束时删除，未永久改写安装配置。

19 项错误码断言修正为 canonical `invalid_request`，对应上游 HTTP 400/404/422 的参数校验或模型不存在错误；没有修改 Provider 错误响应或 AICC 映射。Doubao 原生及 Agent Plan 的视频编辑 Mock 改为校验 `duration=-1`、`ratio=adaptive`，生成请求仍要求正 duration，依据 [Seedance 2.5 文档](https://docs.volcengine.com/docs/ark/seedance-2-5?lang=zh)。

独立契约依据官方声明修正，没有修改这三个 Provider 的产品配置：

- Gemini Flash-Lite 默认关闭思考，补充无显式选项的 `reasoning-none`，依据 [Google thinking 文档](https://ai.google.dev/gemini-api/docs/thinking)。
- GLM-5.3 始终思考，支持 low/high/max；删除测试额外要求的 `reasoning-thinking` 别名，依据 [GLM-5.3 文档](https://docs.z.ai/guides/llm/glm-5.3)。
- OpenRouter 测试默认模型由已不在公开列表中的裸 `openai/gpt-5.6` 改为 `openai/gpt-5.6-luna`，补齐 40 个模型共 152 个已暴露预设的独立预期。各档位逐项与 [官方模型列表](https://openrouter.ai/api/v1/models) 的 `reasoning.supported_efforts` 对照，冻结最小元数据于 `test/aicc_test/fixtures/openrouter-reasoning-20261112.json`，wire 依据 [reasoning 参数文档](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens) 校验；不改变产品精确规则和通配规则。

Qwen 的失败来自 runner 等待上限与首次轮询均为 5 秒的竞争。轮询超时用例改用 15 秒等待上限，区分 runner 等待超时与 TaskMgr 失败，断言已经发起轮询；结束后先隔离旧 Mock instance，再等待遗留任务终止，避免后续场景串扰。这里测试的是 runner 等待超时，不是假装覆盖产品的长任务最终期限。

按用户选择，跨渠道制品用例使用启用的 SN `gpt-image-2` 图片生成，再送入 OpenAI 图片编辑，明确绑定源模型并验证两端 wire。原 Wan 视频组合按渠道禁用策略跳过。

自测 159/159、契约及 T1.5 runner 的 `deno check`、静态 preflight 均通过。临时修正 NDM 路由后的云更新回归仍暴露独立问题：T1 未出现 V1 logical mount，T1.5 未下发 cloud Provider Rules 的 `service_tier=default`；保留失败断言，不能将发布恢复视为云更新通过。本轮修正集中在测试及其 fixture，不改变公共接口、依赖或命令；`aigc_image`、`aigc_audio`、`aigc_video` 手册无需修改。

最终分组回归 `t15-20261112081339-3173685`：186 项中 184 通过、1 失败、1 策略跳过，清理通过，真实调用和费用均为 0。19 项错误码、4 项 duration、Qwen 等待超时及后续成功场景、152 项补充 OpenRouter 预设、三个 Provider 的基础及重点预设、新 SN 图片跨渠道用例均通过。唯一失败为上述云更新 Provider Rules；策略跳过为原 Wan 视频跨渠道组合。T1 定向云更新仍失败于上述 logical mount 断言。本轮未再次运行完整 T1/T1.5；历史完整结果与本轮定向结果分别保留，不能合并宣称完整套件通过。

### Gateway 更新后的完整重跑（2026-11-12）

按用户要求在当前环境直接运行全量 T1/T1.5，没有增加临时 NDM 路由，也没有构建或重启服务。安装的 Gateway 可执行文件已更新，但 `/opt/buckyos/etc/boot_gateway.yaml` 仍将 `/ndm/*` 指向 control-panel；通过 Zone Gateway 探测仍得到 405 及 `Allow: GET, HEAD`。

- T1 `aicc-t1-2026-11-12T09-20-43-187Z-cc0a56c9`：184 通过、1 失败，清理通过。唯一失败为 `t1.config.cloud_update_dynamic_catalog` 在 `publish_v1` 阶段收到 NDM 405。
- T1.5 `t15-20261112092232-3315820`：3806 项全部有结果，3673 通过、61 失败、72 渠道策略跳过。覆盖全部 15 个 Provider，没有 inventory 准备失败。真实调用及费用均为 0，临时实例清理通过。云更新仍失败于 fixture 发布 405，因此本轮没有重新验证此前 logical mount 和 service_tier 的后续断言。
- 另 60 项失败是上一轮测试修正引入的回归：按 HTTP 400/404/422 统一推断 `invalid_request` 过宽，同步接口仍按现有契约返回 `provider_error`，并保留供应商原始 code。撤销状态码默认推断，使用现有 `aicc_error_code_by_scenario` 为已核实的 19 个 native/queue 场景显式声明 `invalid_request`；同步协议的原有预期保持 `provider_error`。补充同时覆盖两类的回归断言，自测仍为 159/159，类型检查及 preflight 通过。该修正没有改产品代码或供应商 wire fixture。
- 全量中的四项 duration、Qwen 轮询时序、三个 Provider inventory、补充预设及新的 SN 图片跨渠道用例通过；72 项跳过均为渠道策略禁用，包含两个跨渠道组合。

本地报告目录为 `test/aicc_test/reports/acceptance/pr727-gateway-updated-full-t1/` 和 `pr727-gateway-updated-full-t15/`。结束后的 `uv run src/check.py` 显示 Zone 已激活、核心服务可达，仍为 Running With Warnings。错误码预期修正后的定向结果单独记录，不改写本轮原始全量报告。

定向重测 `t15-20261112101010-3365354` 的 60 项错误码用例全部通过，清理通过，真实调用和费用均为 0，报告位于 `pr727-gateway-updated-error-retest/`。最终代码之后没有再次运行完整 T1.5，因此全量原始结果仍为 3673/61/72，不能将定向结果合并表述为一次全量通过。

### 同步 boot_gateway.yaml 后的完整重跑（2026-11-12）

按用户要求，将仓库 `src/rootfs/etc/boot_gateway.yaml` 原样同步至 `/opt/buckyos/etc/boot_gateway.yaml`，旧文件备份在 `/tmp/buckyos-pr727-gateway-config-0nxlwk4u/`。新模板依赖的 `cross_zone_socks.json` 和 `cross_zone_socks_params.json` 原先不存在，按当前 `node_daemon/src/boot.rs::write_cross_zone_socks_credentials` 的格式生成本机随机凭据及 params，权限均为 0600，未写入仓库或报告。

Gateway 拒绝热加载 RTCP 安全策略变更，因此仅终止旧 Gateway，由 node-daemon 重新拉起。旧进程使用的是已被替换的可执行文件，新进程 PID 为 3376618，使用已安装的新二进制。启动阶段曾因端口暂未释放出现 bind 失败，随后恢复正常。更新后经 Zone Gateway 的空 POST 探测返回 401 `unauthenticated`，405 消失；安装模板与仓库文件逐字节相同。

- T1 `aicc-t1-2026-11-12T10-19-54-336Z-f0e14794`：184 通过、1 失败。云 fixture 发布成功，失败阶段从 `publish_v1` 推进至 `converge_v1`，仍未出现 V1 logical mount。129 个静态 manifest 单元全部有结果，56 个额外路由暴露单元通过；settings 原样恢复，清理通过。
- T1.5 `t15-20261112102129-3382477`：3806 项全部有结果，3731 通过、3 失败、72 渠道策略跳过。全部 15 个 Provider 完成准备，无未执行单元；真实调用和费用均为 0，临时 Provider 和 Mock 清理通过。
- T1.5 云 fixture 已发布并发出实际 Mock 请求，但 cloud Provider Rules 的 `service_tier=default` 仍未出现在请求中。另两项失败为 `openrouter.videos.v1` 的 `invalid_request` 和 `glm.videos.generations.v4` 的 `invalid_parameters` 测试预期遗漏：两者都是 native 视频路径，TaskMgr 记录实际 canonical code 为 `invalid_request`，原始供应商 code 分别为 `400` 和 `1210`。补充两项显式 scenario 预期及自测，未改产品实现或 Provider wire fixture；159 项自测、类型检查、preflight 均通过。

完整报告保留于 `pr727-config-synced-full-t1/` 和 `pr727-config-synced-full-t15/`，不将后续定向重测合并改写为全量通过。更新及测试未改变公共 CLI、RPC、共享类型或依赖；既有手册操作保持有效。

视频错误码定向重测 `t15-20261112110753-3433349` 为 2/2 通过，清理通过，真实调用和费用均为 0。最终独立回读确认本轮 T1、T1.5 和定向重测的 run ID 均未残留在 settings 或 runtime inventory 中；系统检查仍为已激活、核心服务可达、Running With Warnings。补齐这两项测试预期后未再次运行全量 T1.5，原始全量失败数仍保留为 3。

### 云 metadata 用例隔离修正（2026-11-12）

进一步诊断确认，上述两个云生效断言失败来自现有本地覆盖：`local/models/openai.model.json` 和 `local/providers/openai.provider.json` 的优先级高于云端同身份文件。缓存中已包含测试挂载和 `service_tier`，但生效文件仍是本地版本。AICC 按 `(catalog_kind, catalog_id)` 选择整份配置，优先级 system-config > local > cloud > builtin；较高云 revision 不越过该优先级。这是测试准备未隔离覆盖，不是已确认的云加载实现缺陷。

在现有 `cloud_update_transaction.ts` 增加整 catalog 隔离及恢复：按文件内容中的 catalog 身份识别本地文件（不依赖文件名），重命名为同目录隐藏备份；system-config metadata 文档仅移除目标身份的整份 catalog，保留其他 catalog，不对模型条目进行合并或修改。准备失败回滚，正常及异常退出均恢复原始文件及 system-config 原始字符串，再刷新 AICC。恢复支持刷新后的鉴权客户端，遇到新建同名文件时拒绝覆盖并保留备份。T1 按其四份发布文件隔离冲突；T1.5 按 OpenAI Provider Rules 文件隔离。产品文件优先级及整文件替换实现均未修改，无依赖、公共 CLI、RPC、共享类型变更；现有 `aigc_image`、`aigc_audio`、`aigc_video` 操作不变，无需修改手册。

验证：自测 162/162；`deno check` 覆盖 transaction、T1/T1.5 runner；静态 preflight 和 diff 检查通过。定向 T1 `aicc-t1-2026-11-12T14-11-12-989Z-00fbfa43` 的云更新用例通过，覆盖 V1/V2 挂载切换、模型删除恢复、动态 Provider 路由和 Mock 请求；定向 T1.5 `t15-20261112141132-3620196` 的 cloud-update 通过，捕获请求包含 `service_tier=default`。两轮清理通过，真实模型调用及费用为 0；本轮未再次运行完整 T1/T1.5 或真实 T2/T3。

独立核验原有 11 份本地 metadata 的 SHA256、权限、属主、inode 完全一致，无隐藏备份遗留，system-config metadata 原为空且仍为空，settings/runtime 无本轮 run ID 或测试挂载。回归时本机 root 属主目录临时交给普通执行用户，结束后目录原属主也已恢复；运行时核心服务保持可达。

### 修复后的最终完整回归

按用户要求在当前代码上不加 case 或 Provider 过滤，全量顺序重跑 T1/T1.5。本轮没有再修改测试实现或产品代码。

- T1 `aicc-t1-2026-11-12T15-20-24-579Z-be08460a`：185 通过、0 失败；129 个静态 manifest 单元和 56 个额外路由暴露单元全部通过，云更新 V1/V2 用例通过，settings 原样恢复，清理通过。
- T1.5 `t15-20261112152233-3697334`：3734 通过、0 失败、72 渠道策略跳过；全部 15 个 Provider 完成准备，3806 个计划单元全部有结果，没有准备失败或遗漏单元。72 项跳过均有明确的渠道禁用策略，不能算作协议实测通过。云更新 `service_tier`、错误码、duration、Qwen 轮询、inventory、独立预设及跨 Provider 用例均通过，临时实例与 Mock 清理通过。
- 两层真实模型调用和费用均为 0。本地报告分别位于 `pr727-final-full-t1/` 和 `pr727-final-full-t15/`。
- 独立比较确认原有 11 份本地 metadata 的 SHA256、权限、属主、inode 与回归前一致，目录原属主恢复，无隐藏备份；settings/runtime 无本轮 run ID 或测试挂载，system-config metadata 覆盖仍为空。最终系统检查保持已激活、核心服务可达，保留原有 Running With Warnings 告警。

这是修复后的完整运行结果，先前各轮失败及定向重测报告原样保留。未运行真实 Provider T2/T3 或完整 workspace 测试。
