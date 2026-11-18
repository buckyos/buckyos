# AICC 图片参数协议与路由错误码 TODO

日期：2026-11-18

状态：已在 `fix/aicc-image-contract-errors` 实施代码与手册修复；验证结果见文末。BuckyOS 基于上游 main `3ba922963`，WebSDK 基于上游 main `1c69b634a`。尚未部署到真实 Zone。

## 排查时的实现边界（修复前）

- 文生图 `TextToImageInvokeRequest` 同时有 `prompt`、`negative_prompt`、`quality`、`size`、`aspect_ratio`。
- 图生图 `ImageToImageRequest` 的业务字段只有 `images`、`prompt`、`strength`、`output`；`output.size` 已存在，没有专用负面提示词、质量和画幅比例字段。
- AIWorkspace 的 `mediaRequest` 复用图片参数组装，将顶层 `size`、`quality`、`negative_prompt` 等传给图生图，触发请求解析错误；错误发生在具体模型适配之前。
- Gemini 文生图编码传递 `aspect_ratio`；图生图编码没有比例传递路径。当前 Gemini 文生图编码也没有使用 `negative_prompt`、`quality`，不能把协议声明当作渠道能力已实现。
- AICC 逻辑模型路由把候选为空与候选被权限过滤为空都返回 `resource_not_permitted`。

## P0：分离路由错误码

- [x] 在权限过滤前区分空候选：逻辑路径存在但没有可用模型时使用现有 `no_candidate_model`，有候选但当前调用者均无权使用时才返回 `resource_not_permitted`。
- [x] 核对精确模型、不存在的模型、禁用 Provider、被规则排除的模型、能力不匹配、配额不足等路径，保持错误原因明确；优先复用已有错误码，不新增重复分类。
- [x] 对普通调用者避免披露不可见模型；管理员诊断提供候选缺失、渠道排除、禁用或权限过滤的依据。
- [x] 同步 `buckyos-api`、SDK、AIWorkspace、AI Center 和 Agent 的错误识别及中文提示，避免把模型缺失引导为授权问题。
- [x] 覆盖空候选、有候选但全部无权、部分有权、错误逻辑路径及精确模型场景。

## P0：对齐图片参数协议与调用方

- [x] AIWorkspace 按文生图/图生图分别组装请求；图生图尺寸使用已有 `output.size`，核对其值语义与各渠道编码，不能只移动字段后直接透传 `WxH`。
- [x] 避免 `format` 与尺寸分别赋值时互相覆盖 `output`；核对 `seed` 等同样由公共组装函数带入的字段。
- [x] 明确 `negative_prompt`、`quality` 的图生图需求：目前没有等价专用参数。若纳入统一协议，同步共享类型、序列化、SDK、前端和适配器；若暂不支持，调用方应提前给出明确能力提示。
- [x] 不把 `strength` 当作质量，不把在 `prompt` 中追加限制当作专用负面提示词的无损替代；不静默删除用户要求以掩盖能力缺失。
- [x] 增加桥接层请求与 AICC schema 的契约回归，覆盖有无参考图、尺寸、格式、负面提示词、质量和比例。

## P1：补齐图生图画幅能力

- [x] 为图生图设计显式画幅比例参数，复用已有文生图/视频的校验模式；定义比例与尺寸同时出现时的冲突规则。
- [x] 先检查已接入渠道的真实协议，再实现 Gemini 等适配映射；分别声明比例、尺寸和质量支持情况，不能以通用接口存在推断具体模型支持。
- [x] 将用户明确要求纳入候选能力筛选；精确模型无法满足时返回有字段位置和原因的错误，避免生成后才发现该要求没有传递。
- [x] 记录实际返回尺寸及要求是否满足；区分模型调用成功与结果满足画幅要求，不擅自拉伸、裁剪人物或降低检查要求。

## P1：参数支持情况可观察

- [x] 审计各图片适配器对统一参数的处理：实际映射、不支持、可选降级。重点核对 Gemini 文生图的 `negative_prompt`、`quality`、顶层 `size` 与 `output.size`。
- [x] 通过已有模型能力/诊断机制向 SDK、AIWorkspace 和编写模型提供可用选项，避免靠反复真实生成猜测参数。
- [x] 不支持参数的响应应指出字段和原因；明确选择拒绝或经调用者同意的降级，不能仅在服务日志警告或完全静默忽略。

## 联动与验收

- [x] 检查入口：`src/kernel/buckyos-api/src/aicc_client.rs`、`src/frame/aicc/src/service/inference.rs`、`src/frame/aicc/src/protocol/`、`src/frame/aiworkspace/aiws/host.js`、Desktop AI Center 和同级 `buckyos-websdk` 消费方。
- [x] 更新 `aigc_image`、`buckyos_admin`、`aiworkspace_run` 等受影响手册、示例和接口文档；未影响的手册说明依据。涉及媒体工具公共范本时同步检查派生工具示例。
- [x] 运行相关 Rust package 测试、`aiws` Deno 契约测试和前端检查；协议变化核对 SDK 编译与调用测试。新增测试针对实际故障边界，不只镜像实现。
- [ ] 在真实 Zone 验证文生图和带角色参考的横向图生图；核对请求、实际尺寸、错误码及页面提示。外部 Provider 未实测项单独列出，mock 不能代替真实生成验收。

## 独立工作流问题

角色设定、故事板和成片应在工作流层协调画幅约定。竖版角色资产与横版镜头可以有不同用途，但需明确转换/构图策略，不能让独立许愿格反复试错。已提交 [buckyos/buckyos#745](https://github.com/buckyos/buckyos/issues/745)。按用户明确要求，本分支不处理该 issue；仅对显式图片请求核验返回尺寸，不做跨步骤画幅协调。

本次修改共享协议、AICC 渠道映射/参数能力/路由、AIWorkspace 桥接和错误提示，并同步 WebSDK、CLI 与手册。复用已有参数、错误码和 Provider/路由组件；不新增项目依赖、不修改持久表结构、模块安装映射或运行配置。API 采用当前开发版本的直接变更，不加旧客户端兼容层。

新增 `requirements.image_parameters`，取值如 `image.img2img.aspect_ratio`；能力来自现有 operation binding，进入模型清单并在路由时筛选。具体参数值继续由对应渠道校验，不把支持参数名误认为支持任意值。

手册：更新 `aigc_image`、`aigc_audio`、`aigc_video`、`buckyos_admin`、`aiworkspace_run`、`xllm`。`opendan_agent_dev`、`ui_core` 及 plan skills 无需修改：本次未变更 AgentRootFS、技能发现/冻结、行为模板或会话执行协议。未新增/重构服务，无需修改构建模块、rootfs 种子或 scheduler 接入。


## 验证记录

- `cargo test --manifest-path src/Cargo.toml -p aicc -p buckyos-api -p aiworkspace-core --lib -- --test-threads=1`：AICC 813、API 413、core 47 项通过，API 3 项原有忽略；最终 AICC 全量再次通过 813 项。
- `cargo check --manifest-path src/Cargo.toml -p aicc --all-targets`：通过。
- `uv run --no-project --with cmake cargo test --manifest-path src/Cargo.toml -p aiworkspace --lib -- --test-threads=1`：8 项通过。
- 同一临时 CMake 环境执行 `cargo test ... -p aiworkspace --test wish --test wish_media --test contract -- --test-threads=1`：17 项通过。
- `cargo test ... -p agent_tool --lib xllm::vectors::provider_errors -- --test-threads=1`：1 项共享错误向量用例通过。
- `deno test --allow-read --allow-write --allow-run --allow-env src/frame/aiworkspace/aiws/`：12 项通过。
- Desktop `pnpm check`：通过；新增能力提示的 Deno 测试 2 项通过；Playwright 图片参数与 API 请求契约测试 2 项通过。
- WebSDK `pnpm exec tsc -p tsconfig.build.json --outDir /tmp/buckyos-sdk-image-types`：通过；`pnpm exec jest tests/aicc_client.test.ts tests/xllm/vectors.test.ts --runInBand`：209 项通过、11 项原有跳过。
- `git diff --check`：两个仓库通过。

尚未验证：完整 `buckyos-build.py`、全 workspace 测试、真实 Zone 部署和各 Provider 的付费生成。Playwright 浏览器交互用例已尝试，但 Chromium 缺少 `libatk-1.0.so.0`，未进入浏览器断言；不能当作通过。CMake 是仓库现有构建前置工具，本机原先缺失，使用临时 uv 工具环境完成相关测试，没有新增项目依赖。

API/DataModel 影响：新增可选图片参数与 `requirements.image_parameters`、模型参数能力标记和 `AI_NO_CANDIDATE_MODEL`，属于需要联动部署的接口变更。以前被静默忽略的未实现参数会明确失败；需按所选渠道调整宏的选项。参数映射单测不能替代真实模型对比例、分辨率和质量值的验收。

配套 WebSDK 本地提交：`b3875a9ac`（同名分支）。两个仓库尚未推送；部署时需同步 SDK 的错误分类与新增类型，Git 依赖声明仍保持 `main`。
