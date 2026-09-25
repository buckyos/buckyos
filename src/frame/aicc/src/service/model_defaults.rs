// 内置逻辑模型树的正确性契约（未叠加用户/session 配置时的目标结构，尚待实现）。
//
// 路径按 `.` 分隔；`item 名 -> 完整逻辑路径 (weight)` 表示带权重的引用。
// 例如 chat 下的 gpt_standard 指向 llm.gpt-standard，不会创建 llm.chat.gpt_standard 子目录。
// 本设计只调整 LLM；图片、视频继续保留下面按家族评分的设计。
//
// 背景：本注释描述的是 LLM 路由的新设计，用来替换下方代码中 builtin-aicc-router-v4
// 的旧结构。旧结构中功能到规格的权重表已经可用，问题集中在“规格”没有被正式声明：
// - 规格目录只是 version_rules.current_mount 的副产品，零 inventory 时 llm.gpt-standard
//   等目录不存在，功能目录里的引用悬空（service/tests.rs 目前就在断言这种状态）。
// - 厂商新增规格后没有人会发现它未接入：claude-fable 挂到 llm.fable，但没有任何功能引用它。
// - openai 的 version_rules.auto_mounts 把 nano/mini/standard/pro 全部直挂到 llm.plan、
//   llm.reason、llm.code 等功能目录；功能目录本身是 Hybrid，也会按能力门槛自动吸入模型。
//   两者都绕过了功能到规格的权重，例如 gpt-nano 会出现在 llm.plan 里。
// - 规格内各版本统一按 1.0 挂载，版本先后只靠 version_rank 前缀推断 current。
// - 命名混用：llm.sonnet、llm.kimi 与 llm.gpt-standard 并存，llm.glm 同时是厂商目录
//   （llm.glm.{model}）和规格目录。
// - 仓库列出的模型没有全部归档：gpt-6-astra、gpt-5.3-codex、Qwen 开放权重系列、charglm-4、
//   emohaa、codegeex-4 不属于任何规格；glm-4.5v/4.6v、glm-4-long、glm-ocr 被 glm-standard
//   的兜底匹配吸入，与对话主力模型争抢 current。
// 曾考虑过统一的 lv1～lv5 档位，没有采用：功能表本来就按厂商逐个规格配置权重，统一档位
// 带不来额外信息，反而让 `plan -> claude_lv4` 需要查表才能看懂，也放不下 code 这类特化规格。
// 新设计不改核心架构：仍然是 LogicalModelDefinition + 带权重 item 的 overlay + metadata
// metadata 驱动动态目录 + RegistryLayers 分层叠加。改动集中在 metadata 中的规格、
// 家族与 effort 声明、从官方模型 ID 推导版本顺序，以及 builtin overlay 引用的规格名。
//
// LLM 预定义结构与动态结构的边界：
// 1. Model driver 维护者根据厂商产品线、用途和部署规模，在 metadata 中声明规格；
//    可以有 code/highspeed 等特化规格，AICC 不预设统一档位。规格路径统一为
//    llm.{vendor}-{spec}，例如 llm.claude-opus、llm.kimi-code、llm.glm-standard。
//    规格名是稳定 ID，不带版本号；可包含架构和规模，如 llm.qwen-dense-27b。
//    同一规格内是可替代的产品迭代；同代的不同规模、不同专用任务不能当作前后版本。
// 2. 预定义树 = 功能目录 + 功能到规格的偏好权重（本文件）+ metadata 声明的规格目录。
//    规格目录的存在只取决于已加载的 metadata，不取决于 inventory，内容初始为空。
// 3. Metadata 定义模型所属家族、所属规格和固定思考 effort；版本顺序从官方模型 ID 推导。
//    AICC 根据 metadata 与有效 Provider inventory 的交集，动态创建家族/预设目录，
//    填充“规格 -> 家族预设 -> model:variant@provider”的关系。
//    LLM 模型无需重复声明 logical_mounts；功能路径及其规格引用由通用逻辑树维护。
//    只有 metadata、没有对应 inventory 模型时，不生成该家族，规格保持为空。
// 4. 零 Provider 时，功能目录、规格目录和功能到规格的权重仍须对外可见。
//    按功能名可得到规格候选；规格的家族候选和 exact model 候选均为空。
//    空规格是合法状态，不表示定义缺失，也不需要预建不存在的家族目录。
// 5. Inventory 更新后按同一规则重建动态部分；家族在所有有效 inventory 中都不存在时，
//    移除该家族、预设及指向它的动态引用，保留功能目录、规格目录及其偏好权重。
//    多个 Provider 提供同一家族时，共用家族/预设节点并分别挂载 exact model，
//    不因 Provider 数量增加而重复累加家族权重。
// 6. 规格只接收 metadata 明确归档的家族/固定预设；功能目录只通过规格获得模型。
//    不能用通用 Auto/Hybrid 或 version_rules.auto_mounts 把模型直接挂入功能目录，
//    也不能把所有符合能力门槛的 LLM 塞入规格。
//    家族候选是否可执行还须检查能力、库存、Provider 状态及调用策略。
//    llm 根只作命名空间，不使用 Auto 收集模型；通用请求须明确选 llm.chat 等任务。
//    任务、规格、家族均禁止隐式 Parent fallback。任务无合格候选时，只执行显式配置的
//    fallback，否则返回无候选；回退仍保留原请求及原任务的能力约束，不绕过任务编排。
//    llm.fallback 默认为空，不自动吸入模型；家族直选默认 strict，不升级到其他家族。
// 7. 契约校验（替代固定档位提供的结构保证）：
//    - 功能目录引用的每个规格都必须由某个内置 metadata 声明，否则构建失败，
//      防止厂商改名后留下悬空引用。
//    - 每个声明的规格至少被一个功能目录引用，或在 metadata 中显式标记 direct_only
//      （默认只按规格名或家族名直选），防止新增规格无人接入；显式配置将其接入任务时，
//      须同时解除 direct_only，且不能通过 fallback 暗中接入。
//    - 内置 metadata 中每个 api_types 含 llm 的有效模型规则恰好声明一个规格。
//      按现有精确匹配优先、pattern 首个匹配的语义解析，再校验最终归属；允许规则覆盖，
//      不允许最终结果未归档或同时归入多个规格；零库存时也检查每条 LLM 规则的声明。
//    - 家族 ID 不得与功能名、规格名重名。
// 8. 显式 overlay 可以调整路由偏好；默认契约测试应验证零 Provider 时的空规格、
//    接入后的动态填充、最后一个对应模型移除后的清理、静态结构保留，以及第 7 条校验。
//    还须验证高权重空规格被跳过、任务无候选不回根、direct_only 不被自动选中，
//    以及官方 Provider 移除模型后，其他 Provider 的同一家族仍保留。
//
// LLM 的选择原则：
// - 使用者优先按功能选择，其次按确定版本的家族名选择，最后直接选择规格。
//   规格之间没有跨厂商的统一强弱刻度，由功能表按厂商逐个给权重。
// - 功能到规格的权重表达跨厂商偏好；规格到家族预设的顺序表达同一规格内的版本先后。
//   先按请求及任务约束筛选可执行的家族预设，跳过没有合格候选的规格；再按功能权重
//   选规格，在规格内选合格的最新稳定版本，旧版用于兜底；该规格耗尽再尝试下一规格。
//   两层权重不相乘，也不跨层比较；同权重规格以规格 ID 稳定排序，不拿版本序号决胜。
//   不填写 version_order；按厂商命名约定提取官方模型 ID 中的版本号，缺失分量补零。
//   单位数 minor/patch 时按 major * 100 + minor * 10 + patch 转整数，如 5.6 -> 560、
//   5.5 -> 550、6 -> 600；多位数分量按数值元组比较，避免整数槽位冲突。
//   同版本允许并存，以规范化家族 ID 升序决胜；参数量、日期和产品后缀不作为版本。
//   无可识别版本的模型在同稳定性类别的已识别版本之后兜底，彼此按家族 ID 稳定排序。
//   不把大参数模型与小参数模型、普通版与专用版按发布日期串成升级链。
// - 家族名尽量沿用厂商命名，路径段中的版本小数点改用连字符，例如
//   gpt-5.6 Terra 对应 llm.gpt-5-6-terra，避免将版本号误当作逻辑子目录。
//   家族直选使用 metadata 的 default_effort；规格引用使用 effort，两者都须属于
//   supported_efforts，不自动升级到别的家族版本。
// - :low/:high 等表示家族的固定思考预设，不是子目录；metadata 只声明 supported_efforts，
//   AICC 据此派生思考 variant，无需重复 variants 表；参数转换由 Adapter/Provider 负责。
//   经规格选中的预设不再由请求覆盖思考强度。none 表示关闭思考，thinking 表示仅支持
//   开关时的开启状态，native 表示无法调节的原生模式；不得生成厂商不支持的预设。
//   例如 Qwen3.8-2.4T-A95B 只支持思考，强度为 low/medium/xhigh，不能伪造 none/high；
//   不能将某个家族的参数模板直接套到整个厂商，托管版与开放权重版也须分别核验。
// - 当前 logical_mounts 已能触发动态建目录；规格声明、带版本顺序的规格到家族引用、
//   固定预设绑定、空规格可见性及第 7 条校验仍需落实到实现和测试，
//   不能把现有 current_mount 加统一 1.0 的挂载当作完成。
//
// 规格划分与持续运营规范：
// - 主流厂商的规格按厂商自己的产品线划分，名称沿用厂商叫法（nano/mini/pro、haiku/
//   sonnet/opus、flash/plus/max 等），不为对齐其他厂商而合并或拆分。
// - 特化用途用 code、vision、long、character 等描述，可组合成 vision-code；厂商单独
//   售卖的加速版沿用 x、flashx、highspeed，但加速版不自动代表能力更强。
// - 开放权重是模型属性，不是统一规格。没有产品线名的权重模型按用途、架构、部署规模
//   划分；下方 qwen-moe-*/qwen-dense-* 是 AICC 的部署分组，不是厂商官方档位。
//   B 表示十亿参数；35B-A3B 表示总参数 35B、每 token 激活约 3B，不能按 3B 估算权重
//   存储，也不能把 MoE 的激活参数与稠密模型总参数直接换算成质量或显存等级。
// - 新版只在用途及部署定位仍可替代时归入同一规格，版本值自动推导。preview/exp/beta 可以入规格，
//   同规格有满足约束的稳定版时优先稳定版；否则只在调用策略允许时使用实验版。
// - 不以对话为用途的专用模型（例如 glm-ocr）应从 metadata 中去掉 llm api_type，
//   只保留专用 API，不占用 LLM 规格。
// - 新增模型：先归入已有规格；只有厂商发布了新产品线或新用途时才新增规格。
//   新增规格必须同时决定功能引用（给出初始权重）或标记 direct_only，第 7 条校验兜底。
// - 调整偏好只改功能表的权重，不通过改规格归档来实现。
// - 厂商停止托管只改变对应 Provider 的库存；保留模型 metadata，供第三方和本地部署
//   继续归档。停止默认推荐与删除模型定义分开处理，不能把官方下线传播成全局不可用。
//   只有显式维护性清理才删除定义；退役规格也不能在功能/overlay 仍引用它时直接删除。
// - 浮动 API 名必须按 Provider 已确认的底层模型映射到家族；不能仅因调用 ID 保持不变
//   就继续宣称它是旧版本，也不能将官方的别名切换套到其他 Provider 的固定旧版本上。
//
// 分档依据（官方资料核验于 2026-09-24；产品事实与 AICC 路由判断分开）：
// - Qwen 托管服务：Max 面向复杂推理/Agent，Plus 为通用主力，Flash 偏吞吐和成本。
//   Qwen3.8-2.4T-A95B 是首次开放的 Max 级模型，但权重版仅文本且强制思考；托管 Max
//   增加视觉、非思考等能力。因此分别归 qwen-moe-2t 与 qwen-max，不能视为同一预设。[Q1]
//   Qwen3.5-397B-A17B 对应 Plus 级开放权重；122B-A10B、35B-A3B 和 27B 是并列规模，
//   不是它的后续版本。27B 是稠密模型，35B-A3B 是低激活参数 MoE，部署取舍不同。[Q2,Q3]
//   官方 Qwen3.5 同表自测中，122B/27B/35B 的 SWE-bench Verified 为 72.0/72.4/69.2，
//   GPQA Diamond 为 86.6/85.5/84.2；这支持按任务取舍，不支持“总参数越大必然越强”。
//   这些是厂商特定评测设置下的结果，不作为跨厂商权重公式。[Q3]
//   27B 系列已有 3.5/3.6/3.8 迭代，35B-A3B 系列已有 3.5/3.6；只在各自规格内排序。[Q4,Q5]
// - GLM：standard 是通用旗舰，Air 是较小的通用 MoE 产品线，Flash 是低成本服务线。
//   “Flash”不保证小型本地部署：4.7-Flash 为 30B-A3B，5.3-Flash 已是 320B-A18B，且
//   原生多模态；Provider 是否实际提供模型及能力必须单独判断，不能由后缀推断。[G1,G2]
//   4.6V 为 106B 视觉主力，4.6V-Flash 为 9B 轻量视觉模型，拆为 vision/vision-flash；
//   FlashX 保留独立加速线。5V-Turbo 官方定位视觉编程，独立归 vision-code，可同时
//   被 code、vision 任务引用；4-32B 单列 dense-32b，不当作旗舰 MoE 的旧版本。[G3,G4,G5]
// - DeepSeek：Pro/Flash 保留厂商产品线；Flash 同样能做推理、编程和 Agent，不能仅因
//   名称把它当作短文本小模型。V4 的 Vision-Exp 单列 vision，避免文本版与视觉实验版
//   按发布时间相互替代；后续通用 Flash 增加视觉能力时，可让 vision 任务引用 flash。[D1]
// - Kimi：K2.6 是通用多模态 Agent 模型，本身可编程；Code/Code-highspeed 为独立服务线。
//   通用模型擅长 code 不要求重复归档，由 code 任务同时引用 general 与 code。[K1]
// - MiniMax：M2 系列侧重编程和 Agent；M2.7 与 highspeed 官方说明结果一致、速度不同，
//   保留 standard/highspeed 两条服务线，code 可引用 standard，swift 可引用 highspeed。[M1]
// - Doubao：Pro 为复杂 Agent 主力，Lite 为均衡服务，Mini 偏低成本/吞吐，Code 为编程
//   专用；这些是产品定位，不承诺跨代始终 Pro 胜过 Lite，也不表示全部开放权重。[B1]
//
// 当前仓库模型的目标归档（下方树覆盖本批模型；尚待写入 metadata 并落实第 7 条校验）：
// - gpt：nano = gpt-5.6-luna、gpt-5.4-nano；mini = gpt-5.6-terra、gpt-5.4-mini；
//   standard = gpt-5.5、gpt-5.4；pro = gpt-5.6/gpt-5.6-sol、gpt-5.5-pro、gpt-5.4-pro；
//   max = gpt-6-astra（新的旗舰规格）；codex = gpt-5.3-codex。
// - claude：haiku = claude-haiku-4-5；sonnet = claude-sonnet-5；opus = claude-opus-5；
//   fable = claude-fable-5-1、claude-fable-5。
// - gemini：flash-lite = 3.5/3.1/2.5-flash-lite；flash = 3.8/3.7/3.6/3.5-flash、
//   3-flash-preview、2.5-flash；pro = 3.1-pro-preview、2.5-pro。
// - qwen：flash = qwen3.8/3.7/3.6/3.5-flash；plus = qwen3.7/3.6/3.5-plus；
//   max = qwen3.8/3.7-max、qwen3.6-max-preview、qwen3-max；
//   moe-2t = qwen3.8-2.4t-a95b（集群级，文本复杂推理/代码）；
//   moe-400b = qwen3.5-397b-a17b（大型通用多模态）；
//   moe-120b = qwen3.5-122b-a10b（中型通用多模态）；
//   moe-35b = qwen3.6/3.5-35b-a3b（轻量稀疏，批处理/Agent/多模态）；
//   dense-27b = qwen3.8/3.5-27b（中型稠密，代码/通用多模态）。
//   这些规模 ID 表达各自部署线，不是 lv1～lv5；精确内存、速度由 Provider 实测决定。
// - glm：standard = glm-5.3/5.2/5.1/5、glm-4.7/4.6/4.5；
//   dense-32b = glm-4-32b-0414-128k（独立稠密线，direct_only）；
//   x = glm-4.5-x；turbo = glm-5-turbo；air = glm-4.5-air；airx = glm-4.5-airx；
//   flash = glm-5.3/4.7-flash、glm-4-flash-250414；flashx = glm-4.7-flashx、
//   glm-4-flashx-250414；vision = glm-4.6v/4.5v；vision-code = glm-5v-turbo；
//   vision-flash = glm-4.6v-flash、glm-4.1v-thinking-flash、glm-4v-flash；
//   vision-flashx = glm-4.6v-flashx、glm-4.1v-thinking-flashx；
//   long = glm-4-long；code = codegeex-4（代码补全，不等同完整代码 Agent，direct_only）。[G6]
//   character = charglm-4；emohaa = emohaa（分别 direct_only，无依据将二者排成版本链）；
//   glm-ocr 待按上面的规范去掉 llm api_type。
// - kimi：general = kimi-k2.6、kimi-k2.5；code = kimi-k*-code；
//   code-highspeed = kimi-k*-code-highspeed。
// - deepseek：flash = deepseek-v4-flash；pro = deepseek-v4-pro；
//   vision = deepseek-v4-flash-vision-exp（实验分支，受稳定性策略限制）。
// - doubao：mini/lite/pro/code = doubao-seed-2-0-{mini,lite,pro,code}-*。
// - minimax：standard = MiniMax-M2.7/M2.5/M2.1/M2；highspeed = 对应 -highspeed。
//
// 官方已发布、当前 metadata 尚待补齐的接入清单（不伪装成下方树已覆盖的库存）：
// - Qwen3.6-27B 可接 dense-27b；Qwen3.8-Flash-Next 是独立开放权重发布，不能被
//   qwen3.8-flash* 自动当作托管 Flash 的新版本，须先补精确规则及独立部署规格。[Q7]
// - Qwen3-Coder 有 480B-A35B、30B-A3B、Next 80B-A3B 等并列部署线；接入时分别声明
//   code-large/code-small/code-next 并编排进 code，不能并入通用模型或共用版本链。
//   Coder-Next 只支持非思考输出，不因它是代码 Agent 模型就生成 high 预设。[Q6,Q8]
// - Kimi K3 是与 K2.6 并列提供的更大旗舰，接入时新增 kimi-flagship 并给 plan/code/vision
//   权重；K2.7-Code 及 highspeed 可归既有 code 线。K2.5 官方停服不删除其权重定义。[K1]
// - DeepSeek 2026-09-10 已用 deepseek-flash 提供 V4.1，并将官方旧 V4 Flash / Vision-Exp
//   API 名暂时重定向到 V4.1；须补版本绑定后归 flash，不能把官方旧名挂回旧家族。[D1]
// - GLM-5.3-FlashX、Doubao Seed 2.1 等新服务需补各自 metadata；名称相近不代表已接入。[G2,B2]
//
// 资料来源（模型规模/能力据官方资料；上述规格拆分及下方任务偏好属于 AICC 设计判断）：
// [Q1] https://huggingface.co/Qwen/Qwen3.8-2.4T-A95B
// [Q2] https://huggingface.co/Qwen/Qwen3.5-397B-A17B
// [Q3] https://huggingface.co/Qwen/Qwen3.5-122B-A10B
// [Q4] https://huggingface.co/Qwen/Qwen3.8-27B
// [Q5] https://huggingface.co/Qwen/Qwen3.6-35B-A3B
// [Q6] https://qwenlm.github.io/blog/qwen3-coder/ ; https://huggingface.co/Qwen/Qwen3-Coder-Next
// [Q7] https://huggingface.co/collections/Qwen/qwen38 ; https://huggingface.co/Qwen/models
// [Q8] https://huggingface.co/Qwen/Qwen3-Coder-30B-A3B-Instruct
// [G1] https://docs.bigmodel.cn/cn/guide/models/text/glm-4.5 ; https://huggingface.co/zai-org/GLM-4.7-Flash
// [G2] https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5.3-flash
// [G3] https://huggingface.co/zai-org/GLM-4.6V
// [G4] https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5v-turbo
// [G5] https://huggingface.co/zai-org/GLM-4-32B-0414
// [G6] https://docs.bigmodel.cn/cn/guide/start/model-overview
// [K1] https://platform.kimi.com/docs/models
// [D1] https://api-docs.deepseek.com/updates/
// [M1] https://www.minimax.io/models/text/m27
// [B1] https://seed.bytedance.com/zh/seed2/
// [B2] https://docs.volcengine.com/docs/ark/model-list?lang=zh
//
// 任务权重说明：下列 LLM 数值均为初始路由偏好，不是官方评分或跨榜单换算结果。
// 开放权重规格按上述用途接入任务，默认作为候补；部署方可按质量、延迟、成本实测调整。
// 新一代 Flash 可能超过旧一代 Pro，应调整任务权重，不通过改名或跨规格版本排序表达。
// 未确认的能力不能从厂商整体宣传继承：如当前 Qwen 小模型 metadata 未声明 vision，
// 则 vision 引用虽可见，运行时仍须拒绝不满足要求的候选；须补实测能力后才能执行。
//
// logical root（预定义结构；LLM 规格目录初始为空）
// ├── llm
// │   ├── chat
// │   │   ├── gpt_standard -> llm.gpt-standard (2.2)
// │   │   ├── claude_sonnet -> llm.claude-sonnet (2.1)
// │   │   ├── gemini_flash -> llm.gemini-flash (1.9)
// │   │   ├── gpt_mini -> llm.gpt-mini (1.4)
// │   │   ├── qwen_plus -> llm.qwen-plus (1.3)
// │   │   ├── glm_standard -> llm.glm-standard (1.2)
// │   │   ├── kimi_general -> llm.kimi-general (1.2)
// │   │   ├── qwen_moe_400b -> llm.qwen-moe-400b (1.15)
// │   │   ├── deepseek_flash -> llm.deepseek-flash (1.1)
// │   │   ├── glm_flash -> llm.glm-flash (1.1)
// │   │   ├── qwen_dense_27b -> llm.qwen-dense-27b (1.1)
// │   │   ├── qwen_moe_120b -> llm.qwen-moe-120b (1.05)
// │   │   ├── doubao_lite -> llm.doubao-lite (1.0)
// │   │   ├── qwen_moe_35b -> llm.qwen-moe-35b (1.0)
// │   │   └── minimax_standard -> llm.minimax-standard (0.9)
// │   ├── plan
// │   │   ├── claude_fable -> llm.claude-fable (2.6) [初始值]
// │   │   ├── gpt_max -> llm.gpt-max (2.6) [初始值]
// │   │   ├── claude_opus -> llm.claude-opus (2.5)
// │   │   ├── gemini_pro -> llm.gemini-pro (2.4)
// │   │   ├── gpt_pro -> llm.gpt-pro (2.3)
// │   │   ├── gpt_standard -> llm.gpt-standard (2.0)
// │   │   ├── qwen_max -> llm.qwen-max (1.8)
// │   │   ├── qwen_moe_2t -> llm.qwen-moe-2t (1.7)
// │   │   ├── deepseek_pro -> llm.deepseek-pro (1.5)
// │   │   ├── doubao_pro -> llm.doubao-pro (1.4)
// │   │   ├── glm_standard -> llm.glm-standard (1.3)
// │   │   ├── glm_flash -> llm.glm-flash (1.2)
// │   │   ├── kimi_general -> llm.kimi-general (1.2)
// │   │   ├── qwen_moe_400b -> llm.qwen-moe-400b (1.15)
// │   │   ├── minimax_standard -> llm.minimax-standard (1.1)
// │   │   └── qwen_moe_120b -> llm.qwen-moe-120b (1.05)
// │   ├── code
// │   │   ├── claude_sonnet -> llm.claude-sonnet (2.4)
// │   │   ├── gpt_codex -> llm.gpt-codex (2.2) [初始值]
// │   │   ├── gpt_standard -> llm.gpt-standard (2.1)
// │   │   ├── qwen_max -> llm.qwen-max (1.9)
// │   │   ├── deepseek_pro -> llm.deepseek-pro (1.8)
// │   │   ├── glm_standard -> llm.glm-standard (1.8)
// │   │   ├── qwen_moe_2t -> llm.qwen-moe-2t (1.8)
// │   │   ├── deepseek_flash -> llm.deepseek-flash (1.7)
// │   │   ├── glm_flash -> llm.glm-flash (1.7)
// │   │   ├── kimi_code -> llm.kimi-code (1.7)
// │   │   ├── kimi_general -> llm.kimi-general (1.7)
// │   │   ├── doubao_code -> llm.doubao-code (1.6)
// │   │   ├── minimax_standard -> llm.minimax-standard (1.6)
// │   │   ├── glm_vision_code -> llm.glm-vision-code (1.5)
// │   │   ├── claude_opus -> llm.claude-opus (1.4)
// │   │   ├── qwen_dense_27b -> llm.qwen-dense-27b (1.3)
// │   │   ├── qwen_moe_400b -> llm.qwen-moe-400b (1.3)
// │   │   ├── qwen_moe_120b -> llm.qwen-moe-120b (1.2)
// │   │   └── qwen_moe_35b -> llm.qwen-moe-35b (1.1)
// │   ├── swift
// │   │   ├── gpt_mini -> llm.gpt-mini (2.0)
// │   │   ├── claude_haiku -> llm.claude-haiku (1.9)
// │   │   ├── gemini_flash -> llm.gemini-flash (1.8)
// │   │   ├── gemini_flash_lite -> llm.gemini-flash-lite (1.6)
// │   │   ├── qwen_flash -> llm.qwen-flash (1.5)
// │   │   ├── glm_flash -> llm.glm-flash (1.4)
// │   │   ├── doubao_mini -> llm.doubao-mini (1.3)
// │   │   ├── minimax_highspeed -> llm.minimax-highspeed (1.2)
// │   │   ├── qwen_moe_35b -> llm.qwen-moe-35b (1.2)
// │   │   └── gpt_nano -> llm.gpt-nano (1.1) [初始值]
// │   ├── summarize
// │   │   ├── gpt_mini -> llm.gpt-mini (2.0)
// │   │   ├── gemini_flash -> llm.gemini-flash (1.8)
// │   │   ├── claude_haiku -> llm.claude-haiku (1.6)
// │   │   ├── qwen_flash -> llm.qwen-flash (1.5)
// │   │   ├── glm_flash -> llm.glm-flash (1.4)
// │   │   ├── doubao_lite -> llm.doubao-lite (1.3)
// │   │   ├── minimax_highspeed -> llm.minimax-highspeed (1.2)
// │   │   ├── qwen_moe_35b -> llm.qwen-moe-35b (1.2)
// │   │   └── qwen_dense_27b -> llm.qwen-dense-27b (1.1)
// │   ├── translate
// │   │   ├── gemini_flash -> llm.gemini-flash (1.9)
// │   │   ├── gpt_mini -> llm.gpt-mini (1.8)
// │   │   ├── claude_haiku -> llm.claude-haiku (1.6)
// │   │   ├── qwen_flash -> llm.qwen-flash (1.5)
// │   │   ├── glm_flash -> llm.glm-flash (1.4)
// │   │   ├── doubao_lite -> llm.doubao-lite (1.3)
// │   │   ├── minimax_highspeed -> llm.minimax-highspeed (1.2)
// │   │   ├── qwen_dense_27b -> llm.qwen-dense-27b (1.1)
// │   │   └── qwen_moe_35b -> llm.qwen-moe-35b (1.1)
// │   ├── vision
// │   │   ├── gpt_standard -> llm.gpt-standard (2.2)
// │   │   ├── gemini_pro -> llm.gemini-pro (2.1)
// │   │   ├── claude_opus -> llm.claude-opus (1.9)
// │   │   ├── claude_sonnet -> llm.claude-sonnet (1.8)
// │   │   ├── qwen_max -> llm.qwen-max (1.6)
// │   │   ├── doubao_pro -> llm.doubao-pro (1.5)
// │   │   ├── deepseek_flash -> llm.deepseek-flash (1.4)
// │   │   ├── glm_flash -> llm.glm-flash (1.4)
// │   │   ├── kimi_general -> llm.kimi-general (1.4)
// │   │   ├── glm_vision_code -> llm.glm-vision-code (1.35)
// │   │   ├── glm_vision -> llm.glm-vision (1.3)
// │   │   ├── deepseek_vision -> llm.deepseek-vision (1.2)
// │   │   ├── qwen_moe_400b -> llm.qwen-moe-400b (1.2)
// │   │   ├── qwen_dense_27b -> llm.qwen-dense-27b (1.15)
// │   │   ├── glm_vision_flash -> llm.glm-vision-flash (1.1)
// │   │   ├── qwen_moe_120b -> llm.qwen-moe-120b (1.1)
// │   │   └── qwen_moe_35b -> llm.qwen-moe-35b (1.0)
// │   ├── fallback [空；只接收显式配置，无 Parent fallback]
// │   │
// │   │   以下规格目录由各厂商 metadata 声明（顺序无含义；均为空，由 metadata + inventory
// │   │   动态填充；direct_only 表示未被功能引用、只能按规格名或家族名直选）
// │   ├── gpt-nano
// │   ├── gpt-mini
// │   ├── gpt-standard
// │   ├── gpt-pro
// │   ├── gpt-max
// │   ├── gpt-codex
// │   ├── claude-haiku
// │   ├── claude-sonnet
// │   ├── claude-opus
// │   ├── claude-fable
// │   ├── gemini-flash-lite
// │   ├── gemini-flash
// │   ├── gemini-pro
// │   ├── qwen-flash
// │   ├── qwen-plus
// │   ├── qwen-max
// │   ├── qwen-moe-2t [集群级文本推理/代码；2.4T-A95B]
// │   ├── qwen-moe-400b [大型通用多模态；397B-A17B]
// │   ├── qwen-moe-120b [中型通用多模态；122B-A10B]
// │   ├── qwen-moe-35b [轻量稀疏多模态；35B-A3B]
// │   ├── qwen-dense-27b [中型稠密多模态；27B]
// │   ├── glm-flash
// │   ├── glm-flashx [direct_only]
// │   ├── glm-air [direct_only]
// │   ├── glm-airx [direct_only]
// │   ├── glm-standard
// │   ├── glm-dense-32b [direct_only]
// │   ├── glm-x [direct_only]
// │   ├── glm-turbo [direct_only]
// │   ├── glm-vision
// │   ├── glm-vision-code [视觉编程/Agent]
// │   ├── glm-vision-flash [轻量视觉]
// │   ├── glm-vision-flashx [direct_only]
// │   ├── glm-long [direct_only]
// │   ├── glm-code [direct_only]
// │   ├── glm-character [direct_only]
// │   ├── glm-emohaa [direct_only]
// │   ├── kimi-general
// │   ├── kimi-code
// │   ├── kimi-code-highspeed [direct_only]
// │   ├── deepseek-flash
// │   ├── deepseek-pro
// │   ├── deepseek-vision [视觉实验分支]
// │   ├── doubao-mini
// │   ├── doubao-lite
// │   ├── doubao-pro
// │   ├── doubao-code
// │   ├── minimax-highspeed
// │   └── minimax-standard
// ├── embedding
// │   ├── text
// │   └── multimodal
// ├── rerank
// ├── image
// │   ├── txt2img
// │   │   ├── gpt_image -> image.txt2img.gpt_image (3.0) [1381; gpt-image-2 (medium)]
// │   │   ├── mai_image -> image.txt2img.mai_image (2.8) [1334; mai-image-2.6]
// │   │   ├── grok_image -> image.txt2img.grok_image (2.6) [1302; grok-imagine-image-2.0 (low)]
// │   │   ├── reve -> image.txt2img.reve (2.6) [1301; reve-2.1]
// │   │   ├── muse_image -> image.txt2img.muse_image (2.5) [1276; muse-image]
// │   │   ├── seedream -> image.txt2img.seedream (2.4) [1256; seedream-5.0-pro]
// │   │   ├── qwen_image -> image.txt2img.qwen_image (2.4) [1254; qwen-image-3.0-pro]
// │   │   ├── gemini -> image.txt2img.gemini (2.3) [1246; gemini-3-pro-image-2k]
// │   │   ├── ideogram -> image.txt2img.ideogram (2.1) [1204; ideogram-4.0-quality]
// │   │   ├── recraft -> image.txt2img.recraft (1.9) [1169; recraft-v4.1-utility-pro]
// │   │   ├── flux -> image.txt2img.flux (1.9) [1162; flux-2-max]
// │   │   ├── imagen -> image.txt2img.imagen (1.8) [1148; imagen-ultra-4.0-generate-001]
// │   │   ├── glm -> image.txt2img.glm (1.1) [1010; glm-image]
// │   │   └── sd -> image.txt2img.sd (1.0) [938; stable-diffusion-v35-large]
// │   ├── img2img
// │   │   ├── gpt_image -> image.img2img.gpt_image (3.0) [1461; gpt-image-2 (medium)]
// │   │   ├── grok_image -> image.img2img.grok_image (2.8) [1430; grok-imagine-image-2.0 (low)]
// │   │   ├── mai_image -> image.img2img.mai_image (2.8) [1429; mai-image-2.6]
// │   │   ├── muse_image -> image.img2img.muse_image (2.7) [1402; muse-image]
// │   │   ├── seedream -> image.img2img.seedream (2.7) [1394; seedream-5.0-pro]
// │   │   ├── gemini -> image.img2img.gemini (2.6) [1390; gemini-3-pro-image-2k]
// │   │   ├── reve -> image.img2img.reve (2.6) [1375; reve-2.1]
// │   │   ├── qwen_image -> image.img2img.qwen_image (2.2) [1304; qwen-image-2.0-pro-2026-06-22]
// │   │   ├── hunyuan_image -> image.img2img.hunyuan_image (2.2) [1302; hunyuan-image-3.0-instruct]
// │   │   ├── wan_image -> image.img2img.wan_image (2.2) [1302; wan2.7-image-pro]
// │   │   └── flux -> image.img2img.flux (2.0) [1262; flux-2-max]
// │   ├── inpaint
// │   ├── upscale
// │   └── bg_remove
// ├── vision
// │   ├── ocr
// │   ├── caption
// │   ├── detect
// │   └── segment
// ├── audio
// │   ├── tts
// │   ├── asr
// │   ├── music
// │   └── enhance
// ├── video
// │   ├── txt2video
// │   │   ├── gemini_omni -> video.txt2video.gemini_omni (3.0) [1516; gemini-omni-1.1-flash]
// │   │   ├── seedance -> video.txt2video.seedance (2.8) [1479; dreamina-seedance-2.0-720p]
// │   │   ├── wan -> video.txt2video.wan (2.8) [1476; wan3.0]
// │   │   ├── minimax_h3 -> video.txt2video.minimax_h3 (2.7) [1460; minimax-h3]
// │   │   ├── muse_video -> video.txt2video.muse_video (2.7) [1456; muse-video]
// │   │   ├── happyhorse -> video.txt2video.happyhorse (2.6) [1427; happyhorse-1.0]
// │   │   ├── sora -> video.txt2video.sora (2.3) [1368; sora-2-pro]
// │   │   ├── veo -> video.txt2video.veo (2.2) [1364; veo-3.1-audio]
// │   │   ├── grok_imagine -> video.txt2video.grok_imagine (2.1) [1342; grok-imagine-video-720p]
// │   │   ├── pixverse -> video.txt2video.pixverse (1.6) [1240; pixverse-v5.6]
// │   │   ├── runway -> video.txt2video.runway (1.5) [1225; runway-gen-4.5]
// │   │   ├── kling -> video.txt2video.kling (1.5) [1216; kling-2.6-pro]
// │   │   ├── hailuo -> video.txt2video.hailuo (1.5) [1206; hailuo-2.3]
// │   │   ├── hunyuan_video -> video.txt2video.hunyuan_video (1.3) [1169; hunyuan-video-1.5]
// │   │   └── ltx -> video.txt2video.ltx (1.2) [1154; ltx-2-19b]
// │   ├── img2video
// │   │   ├── minimax_h3 -> video.img2video.minimax_h3 (3.0) [1495; minimax-h3]
// │   │   ├── gemini_omni -> video.img2video.gemini_omni (3.0) [1488; gemini-omni-1.1-flash]
// │   │   ├── wan -> video.img2video.wan (2.9) [1480; wan3.0]
// │   │   ├── seedance -> video.img2video.seedance (2.9) [1477; dreamina-seedance-2.5-720p]
// │   │   ├── grok_imagine -> video.img2video.grok_imagine (2.8) [1456; grok-imagine-video-1.5-720p]
// │   │   ├── flux_video -> video.img2video.flux_video (2.8) [1449; flux-3-video-20260811]
// │   │   ├── happyhorse -> video.img2video.happyhorse (2.7) [1442; happyhorse-1.0]
// │   │   ├── veo -> video.img2video.veo (2.5) [1398; veo-3.1-audio]
// │   │   ├── vidu -> video.img2video.vidu (2.3) [1363; vidu-q3-pro]
// │   │   ├── kling -> video.img2video.kling (2.3) [1354; kling-v3-pro]
// │   │   ├── pixverse -> video.img2video.pixverse (2.0) [1299; pixverse-v5.6]
// │   │   ├── hailuo -> video.img2video.hailuo (1.8) [1262; hailuo-2.3]
// │   │   ├── hunyuan_video -> video.img2video.hunyuan_video (1.5) [1198; hunyuan-video-1.5]
// │   │   ├── ltx -> video.img2video.ltx (1.3) [1159; ltx-2-19b]
// │   │   └── runway -> video.img2video.runway (1.0) [1052; runway-gen4-turbo]
// │   ├── video2video
// │   ├── extend
// │   └── upscale
// └── agent_runtime
//     └── computer_use

// Provider inventory 接入后的动态展开示例（以下家族及引用不属于预定义结构）：
// llm
// ├── gpt-pro [规格；openai metadata 声明]
// │   ├── gpt_5_6_sol -> llm.gpt-5-6-sol:high (560，推导版本值)
// │   └── gpt_5_5_pro -> llm.gpt-5-5-pro:high (550，推导版本值)
// ├── gpt-5-6-sol [动态家族；厂商家族名 gpt-5.6 Sol；默认预设由 metadata 定义]
// │   └── :high [固定思考预设]
// │       ├── provider_a -> gpt-5.6-sol:reasoning-high@provider-a
// │       └── provider_b -> gpt-5.6-sol:reasoning-high@provider-b
// └── gpt-5-5-pro [动态家族；厂商家族名 gpt-5.5 Pro]
//     └── :high [固定思考预设]
//         └── provider_a -> gpt-5.5-pro:reasoning-high@provider-a
//
// 此时 llm.plan 跳过其他空规格，选择 llm.gpt-pro -> llm.gpt-5-6-sol:high，版本值 560 优先。
// 若只剩旧版满足约束，则选择版本值 550 的家族预设；两者都不在有效 inventory 时，
// llm.gpt-pro 恢复为空，llm.plan 到 llm.gpt-pro 的引用及其权重 2.3 继续保留。
// 若 plan 的所有规格都无合格候选且没有显式 fallback，则返回无候选，不回退到 llm 根。

use async_trait::async_trait;
use buckyos_api::{
    AiccFallbackMode, AiccFallbackRule, AiccLogicalNodeOverlay, AiccRouteOverlay,
    AiccSchedulerProfile, ApiType, ModelDisable, ModelItem, ModelRequirement,
};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::catalog::CatalogSnapshot;
use crate::error::RuntimeError;
use crate::model::{
    LogicalModelDefinition, ModelRegistry, MountMode, ProviderInventory as ModelProviderInventory,
    RegistryLayers,
};
use crate::runtime::ModelRegistryAssembler;

pub(super) struct ServiceModelAssembler {
    pub(super) session: Option<buckyos_api::AiccRouteOverlay>,
}

#[async_trait]
impl ModelRegistryAssembler for ServiceModelAssembler {
    async fn build(
        &self,
        catalog: Arc<CatalogSnapshot>,
        inventories: Vec<ModelProviderInventory>,
    ) -> Result<Arc<ModelRegistry>, RuntimeError> {
        ModelRegistry::build(
            catalog.as_ref(),
            &inventories,
            builtin_logical_model_definitions(),
            RegistryLayers {
                factory: Some(&builtin_logical_tree_overlay()),
                session: self.session.as_ref(),
                ..RegistryLayers::default()
            },
        )
        .map(Arc::new)
        .map_err(|error| RuntimeError::Backend(error.to_string()))
    }
}

pub(super) fn builtin_logical_model_definitions() -> Vec<LogicalModelDefinition> {
    let mut definitions = vec![
        llm_logical_definition(
            "llm",
            ModelRequirement::default(),
            MountMode::Auto,
            AiccSchedulerProfile::Balanced,
            Some(AiccFallbackRule {
                mode: AiccFallbackMode::Strict,
                target: None,
            }),
            Some("general"),
        ),
        logical_definition(
            "embedding.text",
            ApiType::EmbeddingText,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("general"),
        ),
        logical_definition(
            "embedding.multimodal",
            ApiType::EmbeddingMultimodal,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("multimodal"),
        ),
        logical_definition(
            "rerank",
            ApiType::Rerank,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("general"),
        ),
    ];
    definitions.extend([
        llm_logical_definition(
            "llm.chat",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("general"),
        ),
        llm_logical_definition(
            "llm.plan",
            tool_json_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("pro"),
        ),
        llm_logical_definition(
            "llm.code",
            tool_json_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("pro"),
        ),
        llm_logical_definition(
            "llm.swift",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            parent_fallback(),
            Some("fast"),
        ),
        llm_logical_definition(
            "llm.summarize",
            context_requirement(16_384),
            MountMode::Hybrid,
            AiccSchedulerProfile::CostFirst,
            parent_fallback(),
            Some("utility"),
        ),
        llm_logical_definition(
            "llm.summary",
            context_requirement(16_384),
            MountMode::Hybrid,
            AiccSchedulerProfile::CostFirst,
            parent_fallback(),
            Some("utility"),
        ),
        llm_logical_definition(
            "llm.translate",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::CostFirst,
            parent_fallback(),
            Some("utility"),
        ),
        llm_logical_definition(
            "llm.reason",
            context_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            disabled_fallback(),
            Some("reasoning"),
        ),
        llm_logical_definition(
            "llm.vision",
            vision_requirement(32_768),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("multimodal"),
        ),
        llm_logical_definition(
            "llm.long",
            context_requirement(128_000),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("long_context"),
        ),
        llm_logical_definition(
            "llm.fallback",
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            disabled_fallback(),
            Some("fallback"),
        ),
    ]);
    definitions.extend([
        logical_definition(
            "image.txt2img",
            ApiType::ImageTextToImage,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("general"),
        ),
        logical_definition(
            "image.img2img",
            ApiType::ImageImageToImage,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("general"),
        ),
        logical_definition(
            "image.inpaint",
            ApiType::ImageInpaint,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("edit"),
        ),
        logical_definition(
            "image.upscale",
            ApiType::ImageUpscale,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("edit"),
        ),
        logical_definition(
            "image.bg_remove",
            ApiType::ImageBackgroundRemove,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            parent_fallback(),
            Some("utility"),
        ),
        logical_definition(
            "vision.ocr",
            ApiType::VisionOcr,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "vision.caption",
            ApiType::VisionCaption,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "vision.detect",
            ApiType::VisionDetect,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "vision.segment",
            ApiType::VisionSegment,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("vision"),
        ),
        logical_definition(
            "audio.tts",
            ApiType::AudioTextToSpeech,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "audio.asr",
            ApiType::AudioSpeechRecognition,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::LatencyFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "audio.music",
            ApiType::AudioMusic,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "audio.enhance",
            ApiType::AudioEnhance,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            strict_fallback(),
            Some("audio"),
        ),
        logical_definition(
            "video.txt2video",
            ApiType::VideoTextToVideo,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.img2video",
            ApiType::VideoImageToVideo,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.video2video",
            ApiType::VideoToVideo,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.extend",
            ApiType::VideoExtend,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "video.upscale",
            ApiType::VideoUpscale,
            ModelRequirement::default(),
            MountMode::Hybrid,
            AiccSchedulerProfile::QualityFirst,
            parent_fallback(),
            Some("video"),
        ),
        logical_definition(
            "agent_runtime.computer_use",
            ApiType::AgentComputerUse,
            vision_requirement(8_192),
            MountMode::Hybrid,
            AiccSchedulerProfile::Balanced,
            parent_fallback(),
            Some("agent"),
        ),
    ]);
    definitions
}

fn llm_logical_definition(
    path: &str,
    min_line: ModelRequirement,
    mount_mode: MountMode,
    scheduler_profile: AiccSchedulerProfile,
    fallback: Option<AiccFallbackRule>,
    tier: Option<&str>,
) -> LogicalModelDefinition {
    logical_definition(
        path,
        ApiType::Llm,
        min_line,
        mount_mode,
        scheduler_profile,
        fallback,
        tier,
    )
}

fn logical_definition(
    path: &str,
    api_type: ApiType,
    min_line: ModelRequirement,
    mount_mode: MountMode,
    scheduler_profile: AiccSchedulerProfile,
    fallback: Option<AiccFallbackRule>,
    tier: Option<&str>,
) -> LogicalModelDefinition {
    LogicalModelDefinition {
        path: path.to_string(),
        api_type,
        min_line,
        disable_line: ModelDisable::default(),
        default_options: BTreeMap::new(),
        mount_mode,
        scheduler_profile,
        fallback,
        route_policy: buckyos_api::AiccPolicyConfig::default(),
        user_visible_tier: tier.map(str::to_owned),
    }
}

fn parent_fallback() -> Option<AiccFallbackRule> {
    Some(AiccFallbackRule {
        mode: AiccFallbackMode::Parent,
        target: None,
    })
}

fn strict_fallback() -> Option<AiccFallbackRule> {
    Some(AiccFallbackRule {
        mode: AiccFallbackMode::Strict,
        target: None,
    })
}

fn disabled_fallback() -> Option<AiccFallbackRule> {
    Some(AiccFallbackRule {
        mode: AiccFallbackMode::Disabled,
        target: None,
    })
}

fn tool_json_requirement(min_context_tokens: u64) -> ModelRequirement {
    ModelRequirement {
        tool_call: true,
        json_schema: true,
        min_context_tokens: Some(min_context_tokens),
        ..ModelRequirement::default()
    }
}

fn context_requirement(min_context_tokens: u64) -> ModelRequirement {
    ModelRequirement {
        min_context_tokens: Some(min_context_tokens),
        ..ModelRequirement::default()
    }
}

fn vision_requirement(min_context_tokens: u64) -> ModelRequirement {
    ModelRequirement {
        vision: true,
        min_context_tokens: Some(min_context_tokens),
        ..ModelRequirement::default()
    }
}

pub(super) fn builtin_logical_tree_overlay() -> AiccRouteOverlay {
    AiccRouteOverlay {
        revision: Some("builtin-aicc-router-v4".to_string()),
        logical_tree: BTreeMap::from([(
            "llm".to_string(),
            AiccLogicalNodeOverlay {
                children: BTreeMap::from([
                    (
                        "chat".to_string(),
                        logical_node(&[
                            ("gpt", "llm.gpt-standard", 2.2),
                            ("sonnet", "llm.sonnet", 2.1),
                            ("gemini", "llm.gemini-flash", 1.9),
                            ("mini", "llm.gpt-mini", 1.4),
                            ("qwen_plus", "llm.qwen-plus", 1.3),
                            ("glm", "llm.glm", 1.2),
                            ("kimi", "llm.kimi", 1.2),
                            ("deepseek_flash", "llm.deepseek-flash", 1.1),
                            ("doubao_lite", "llm.doubao-lite", 1.0),
                            ("minimax", "llm.minimax", 0.9),
                        ]),
                    ),
                    (
                        "plan".to_string(),
                        logical_node(&[
                            ("opus", "llm.opus", 2.5),
                            ("gemini", "llm.gemini-pro", 2.4),
                            ("gpt_pro", "llm.gpt-pro", 2.3),
                            ("gpt", "llm.gpt-standard", 2.0),
                            ("qwen_max", "llm.qwen-max", 1.8),
                            ("deepseek", "llm.deepseek-pro", 1.5),
                            ("doubao_pro", "llm.doubao-pro", 1.4),
                            ("glm", "llm.glm", 1.3),
                            ("kimi", "llm.kimi", 1.2),
                            ("minimax", "llm.minimax", 1.1),
                        ]),
                    ),
                    (
                        "code".to_string(),
                        logical_node(&[
                            ("sonnet", "llm.sonnet", 2.4),
                            ("gpt", "llm.gpt-standard", 2.1),
                            ("qwen", "llm.qwen-max", 1.9),
                            ("deepseek", "llm.deepseek-pro", 1.8),
                            ("kimi_code", "llm.kimi-code", 1.7),
                            ("doubao_code", "llm.doubao-code", 1.6),
                            ("opus", "llm.opus", 1.4),
                        ]),
                    ),
                    (
                        "swift".to_string(),
                        logical_node(&[
                            ("mini", "llm.gpt-mini", 2.0),
                            ("haiku", "llm.haiku", 1.9),
                            ("gemini_flash", "llm.gemini-flash", 1.8),
                            ("gemini_flash_lite", "llm.gemini-flash-lite", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_mini", "llm.doubao-mini", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "summarize".to_string(),
                        logical_node(&[
                            ("mini", "llm.gpt-mini", 2.0),
                            ("gemini_flash", "llm.gemini-flash", 1.8),
                            ("haiku", "llm.haiku", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_lite", "llm.doubao-lite", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "summary".to_string(),
                        logical_node(&[
                            ("mini", "llm.gpt-mini", 2.0),
                            ("gemini_flash", "llm.gemini-flash", 1.8),
                            ("haiku", "llm.haiku", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_lite", "llm.doubao-lite", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "translate".to_string(),
                        logical_node(&[
                            ("gemini_flash", "llm.gemini-flash", 1.9),
                            ("mini", "llm.gpt-mini", 1.8),
                            ("haiku", "llm.haiku", 1.6),
                            ("qwen_flash", "llm.qwen-flash", 1.5),
                            ("glm_flash", "llm.glm-flash", 1.4),
                            ("doubao_lite", "llm.doubao-lite", 1.3),
                            ("minimax_highspeed", "llm.minimax-highspeed", 1.2),
                        ]),
                    ),
                    (
                        "reason".to_string(),
                        logical_node(&[
                            ("gpt_pro", "llm.gpt-pro", 2.5),
                            ("opus", "llm.opus", 2.4),
                            ("gemini", "llm.gemini-pro", 2.2),
                            ("qwen_max", "llm.qwen-max", 1.9),
                            ("deepseek", "llm.deepseek-pro", 1.8),
                            ("doubao_pro", "llm.doubao-pro", 1.6),
                            ("glm", "llm.glm", 1.5),
                            ("kimi", "llm.kimi", 1.4),
                            ("minimax", "llm.minimax", 1.3),
                        ]),
                    ),
                    (
                        "vision".to_string(),
                        logical_node(&[
                            ("gpt", "llm.gpt-standard", 2.2),
                            ("gemini", "llm.gemini-pro", 2.1),
                            ("opus", "llm.opus", 1.9),
                            ("sonnet", "llm.sonnet", 1.8),
                            ("qwen_max", "llm.qwen-max", 1.6),
                            ("doubao_pro", "llm.doubao-pro", 1.5),
                            ("kimi", "llm.kimi", 1.4),
                        ]),
                    ),
                    (
                        "long".to_string(),
                        logical_node(&[
                            ("gemini", "llm.gemini-pro", 2.3),
                            ("opus", "llm.opus", 2.1),
                            ("gpt_pro", "llm.gpt-pro", 2.0),
                            ("gpt", "llm.gpt-standard", 1.8),
                            ("qwen_max", "llm.qwen-max", 1.6),
                            ("deepseek", "llm.deepseek-pro", 1.5),
                            ("kimi", "llm.kimi", 1.4),
                            ("minimax", "llm.minimax", 1.3),
                        ]),
                    ),
                ]),
                ..AiccLogicalNodeOverlay::default()
            },
        )]),
        ..AiccRouteOverlay::default()
    }
}

fn logical_node(items: &[(&str, &str, f64)]) -> AiccLogicalNodeOverlay {
    AiccLogicalNodeOverlay {
        items: Some(
            items
                .iter()
                .map(|(name, target, weight)| {
                    ((*name).to_string(), ModelItem::new(*target, *weight))
                })
                .collect(),
        ),
        ..AiccLogicalNodeOverlay::default()
    }
}
