# BuckyOS E2E 入口与验证边界

本页用于选择当前仓库中实际存在的测试入口。需求、测试源码、历史报告与本次执行结果分别阅读；本页不发布新的通过率。执行前记录被测提交、相关未提交差异、后端地址、浏览器、身份、模型或夹具，以及选中的用例范围。

## 浏览器测试

Desktop 的 [package.json](../../src/frame/desktop/package.json) 中 `test:e2e` 只使用默认 [playwright.config.ts](../../src/frame/desktop/playwright.config.ts)，不会遍历其他配置。默认测试目录为 `tests/e2e`；开发环境的 `.env.development` 启用 Mock runtime。部分 spec 自带真实环境开关或服务夹具，必须逐项核对，不能把整套结果统称为真实 Zone E2E。

以下命令均从 `src/frame/desktop` 执行，要求已有对应依赖与浏览器；目录、端口、启动和跳过条件以所链接的配置及 spec 为准。

| 入口 | 验证对象与环境 | 不由此证明的范围 |
| --- | --- | --- |
| `pnpm run test:e2e` | 默认 Desktop 页面测试；目录 `tests/e2e`，Vite 端口 4173 | 所有独立套件已运行、真实身份和所有服务均可用 |
| `pnpm exec playwright test --config=playwright.aicc.config.ts` | AI Center 模型与路由页面，明确开启 Mock runtime | AICC T1/T1.5/T2/T3、真实 Provider 推理 |
| `pnpm exec playwright test --config=playwright.aiworkspace.config.ts` | [AI Workspace](../../src/frame/desktop/src/app/aiworkspace/README.md) 的真实独立后台、文档提交、权限、协作、离线、许愿格等 UI；专用目录 `tests/aiworkspace` | Zone 登录、Gateway/RBAC、真实 AICC 或 Provider 质量；测试使用静态身份、fixture 数据源和脚本化模型 |
| `pnpm exec playwright test --config=playwright.homestation.config.ts` | HomeStation 的本地 `devnet` 服务、静态身份和 RSS 夹具；需先构建 devnet 或提供 `HS_DEVNET_BIN` | 已部署 Zone、外部 RSS 服务或真实用户间联邦链路 |
| `pnpm exec playwright test --config=playwright.real.config.ts` | [MessageHub 真实环境 spec](../../src/frame/desktop/tests/e2e/real)；需已有部署或代理开发服务，及 `MESSAGEHUB_REAL_E2E=1` 等各 spec 前置 | 未开启或缺少锚点的用例；通过 API 登录并注入会话的测试不等于登录页 SSO 已验证 |
| `pnpm exec playwright test --config=playwright.dv.config.ts` | `tests/dv` 的真实 Zone UI 用例；入口由 `BUCKYOS_UI_DV_BASE_URL` / `BUCKYOS_TEST_ZONE_HOST` 配置 | 全部 Desktop 用户旅程；独立后台测试不能代替这一层 |

OpenDAN WebUI 在 `src/frame/opendan/web` 下另有三套配置：默认 [playwright.config.ts](../../src/frame/opendan/web/playwright.config.ts) 使用本地开发 UI；[playwright.real.config.ts](../../src/frame/opendan/web/playwright.real.config.ts) 面向已运行的 OpenDAN；[playwright.zone.config.ts](../../src/frame/opendan/web/playwright.zone.config.ts) 面向 Agent 的真实应用域名及 Zone 登录。运行时从当前 AgentSpec 确认应用域名和 owner，并通过环境变量配置，不把示例的 `jarvis.test.buckyos.io` 当作实际 Agent 身份。

有依赖时可以用 `pnpm exec playwright test --config=<配置文件> --list` 核对选例；列出用例不代表执行通过。Playwright 脚本验证固定流程，执行 Agent 也可按冻结的用户目标观察 UI、完成操作并留证；两种方法都需要实际路径、关键截图或 trace、导出物及独立结果检查。

## 服务与模型链路

[AICC E2E 需求](../aicc/aicc_e2e_test_requirements.md) 与 [当前验收工具](../../test/aicc_test/acceptance/README.md) 是 T1/T1.5/T2 的入口：T1 测路由和系统行为，T1.5 测独立官方协议 Mock，T2 测获准真实 Provider 的矩阵。`acceptance:preflight`、`acceptance:self-test` 和 Deno 类型检查验证工具及契约，不等于执行真实 Gateway 用例。`doc/aicc/maintenance/acceptance` 保留旧 L1–L4 拆分资料，发生冲突时按当前规范和 runner 处理。

[Jarvis Media DV](../../test/jarvis_media_dv/README.md) 是 T3 消息与多模态任务入口，默认 msg-center，Telegram 需要额外配置。真实模型或外部消息发送的前置和授权按该套件及 [AICC 执行技能](../../harness/SKILLS/aicc-e2e-test/SKILL.md) 处理；缺失能力或凭据应明确跳过，不能变成通过。

仓库根目录的 `uv run test/run.py --list` 只发现 runner 支持的模块；`-p <模块>` 会按检测到的入口执行。它不自动启动 Zone，不遍历所有嵌套测试，也不统一调度 Desktop Playwright 或 Canvas 评估器。Node 模块会安装依赖，依赖 `buckyos` 时还会更新该包，可能改变锁文件；执行前读对应 README。真实本机 DV 还需核对 `uv run src/check.py` 的激活和服务状态。

## CI、产物与结论

[Dev Tests](../../.github/workflows/dev-tests.yml) 当前运行 Cargo 测试、AICC 静态契约门禁、多平台构建及单节点 `test_rbac`。配置中没有上述 Playwright 套件或 AICC 完整 T1–T3 执行命令；不能根据 CI 绿灯推断所有 E2E 完成。每次仍需核对实际工作流 revision 和运行日志。

按项目 AGENTS 的工作区规则安排临时数据与日志，长远程任务 detached 启动再轮询。AI Workspace runner 的后台数据使用 Node `tmpdir()`，可通过 `TMPDIR` 指向项目 `.work`；当前配置的 Playwright 输出和离线预览构建仍在 Desktop `test-results/`，HomeStation 的数据也位于该目录。要求全部产物进入 `.work` 时，应先解决这些 runner 输出路径；本页不把文档建议写成已实现的配置能力。

报告同时说明用户目标完成范围、运行环境和证据边界。`PASS` 需要所有选定判据的证据；`FAIL` 需要实际违反判据的证据；已尝试但证据不足用 `INCONCLUSIVE`；未运行用 `NOT_RUN`。静态检查、脚本列表、历史报告与真实模型调用分别记录。纯文档维护不生成新的 E2E 通过结论。
