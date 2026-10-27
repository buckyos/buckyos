# BuckyOS Desktop Lite 调度器解耦与回退验收 TODO

日期：2026-10-26。

状态：待实施。本文只记录改造约束、任务与验收，不代表代码已修改或回退验证已通过。

## 1. 核心要求

用户明确要求：**调度器没有必须的理由，不应该随手修改任何代码。完成改造后，应能回退 Lite 相关的调度器代码，当前 Lite 仍正常运行。**

Lite 应通过安装配置、节点范围、应用创建入口和节点执行能力形成产品边界。调度器继续按应用需求与节点能力工作，不负责识别产品版本、执行产品配额或同步产品配置。

- Lite 本机设备保持 `support_container=false`；UI 阻止增加普通 BuckyOS 节点，节点加入接口保持同一约束。
- 允许增加 BuckyOS Agent Daemon。它作为 Executor 提供 Agent 的执行环境，不成为应用调度节点，也不向调度池提供容器能力。
- 调度器拿不到支持容器的可选节点，原有 `need_container && !node.support_container` 过滤自然阻止容器 App/Agent 的部署。
- 单 Agent、用户应用类型和预置应用许可等产品策略放在 control-panel 等产品入口。原生进程的加载与生命周期由 node-daemon/AppLoader 负责。
- 保持现有 Lite 能力：基础服务、Desktop、预置脚本应用、一个原生 Agent、static web 安装/更新/卸载，以及 Agent Daemon 接入；不能靠删掉当前功能实现调度器回退。

本文替代 [Lite 实施 TODO](buckyos-desktop-lite-todo.md) 中要求调度器重复校验产品策略、同步产品档位及按 Lite 强制运行方式的设计；其它产品行为继续沿用。通用原生 Agent 能力见 [原生 Agent Service TODO](opendan-native-agent-service-todo.md)。

## 2. 已核对的基线与依赖

本次源码核对 HEAD：`aa7b44bd8474d583c6d4cbe8326bc132c64fa1a7`。实施时重新检查工作树与提交历史，不覆盖其它在途改动。

- Lite 引入提交：`1df46176aa8eb966aa9badbdc6c7a2a22220b691`（`support buckyos-desktop-lite`）。
- Lite 前调度器基线：其父提交 `ee65f12f8d8d027df3c0abf55529ff4e37d4a32c`。
- 更早的 `5af93e6e6` 已实现通用原生 Agent 调度，以上基线包含它。回退 Lite 改动不等于撤销原生 Agent 支持。
- 后续 `4bb84f2d1` 又修改了调度器的 bucket_store/S3 相关逻辑。主线撤销 Lite 改动时不能用旧文件整体覆盖，误删这些无关变更。

Lite 提交触及以下调度器文件，实施时按 diff 逐项审计：

| 文件 | Lite 引入的职责 | 本次处理方向 |
| --- | --- | --- |
| [main.rs](../src/kernel/scheduler/src/main.rs) | boot 写入及每次启动同步 `system/product` | 将产品配置发布移到合适的现有初始化/产品管理入口，消除 scheduler 依赖 |
| [install_plan_executor.rs](../src/kernel/scheduler/src/install_plan_executor.rs) | 执行安装计划前读取产品并再次 `admit_app` | 产品准入在提交与任务入口完成，执行器保留通用执行职责 |
| [system_config_agent.rs](../src/kernel/scheduler/src/system_config_agent.rs) | 产品准入、唯一 Agent 槽位复核、产品副本上限、按产品清除 `need_container` | 从入口配置获得正确需求、实例数和 host 绑定，移除产品策略 |
| [app.rs](../src/kernel/scheduler/src/app.rs) | 根据产品把预置脚本/Agent 强制设为 Native，并增加原生脚本包分支 | 解决旧调度器输出与 AppLoader 的执行契约，使 Lite 不依赖这些新增分支 |
| [scheduler.rs](../src/kernel/scheduler/src/scheduler.rs) | 不可放置状态、单 spec 失败隔离、未放置 spec 重试、错误说明 | 区分通用修复与 Lite 策略；不能因为已在同一提交中就默认保留为 Lite 前提 |

不能直接删分支后宣称完成，当前至少存在这些回退障碍：

1. 旧调度器不发布 `system/product`；control-panel 读不到时按 Full 处理，产品限制可能丢失。
2. 旧调度器对非 Agent 返回 `run_mode=Container`；当前原生脚本加载依赖 Native 分支，需要在调度器之外解决契约，不能让脚本重新进入 worker 容器。
3. 旧的普通应用 `need_container` 推导还有特定 author 放行逻辑；当前 systest AppDoc 的 author 为 `did:bns:buckyos`，不在旧的 `did:web:buckyos.*` 列表中。即便 Loader 会原生执行，也可能在放置阶段被拦住。不得伪造 author、改签发身份或伪报容器能力来绕过。
4. 旧调度器遇到不可放置的 spec 可能中断整轮调度；不能依赖 Lite 提交新增的失败隔离来掩盖上游不断提交无效 spec 的问题。

## 3. 实施任务

### T0：确定最小改动与回退清单

- [ ] 对照上述基线列出 Lite 调度器改动及当前调用方，分别标注产品策略、运行契约、独立通用修复。
- [ ] 为每项依赖找到调度器之外的处理入口，先形成可验证的最小方案，再实施。优先使用已有配置、类型、初始化流程和 Loader 能力。
- [ ] 默认不新增调度器代码。确实无法由现有机制表达时，记录具体失败案例、必须改动的原因、最小范围及对回退验收的影响；不能以“顺手完善”扩大范围，也不能静默放宽回退要求。
- [ ] 通用修复单独评估、单独记录，不与 Lite 产品政策捆绑。是否保留通用修复不能替代旧基线运行验收。

### T1：在产品入口固定节点范围与配置

- [ ] Lite 的节点 UI 阻止增加普通节点，对应加入接口给出明确结果；不靠调度器读取 Lite 档位后隐藏节点。
- [ ] 激活、开发配置生成、安装与重启后的设备文档/设备上报都保持 `support_container=false`，宿主安装了 Docker/wslc 也不改变能力。
- [ ] Agent Daemon 继续沿现有 Executor 注册、连接和授权路径接入，不写成可放置 AppService 的普通节点。
- [ ] 为 `etc/product.json` 到 `system/product` 的发布确定调度器之外的单一责任入口。覆盖首次启动、待激活安装、已有 Lite 重启和产品更新；产品消费者运行前配置已就绪，避免短暂按 Full 放行。

### T2：产品准入与单 Agent 限制留在控制面

- [ ] 保留 control-panel 对新增服务应用、预置应用身份/版本及 static web 的现有产品策略，覆盖 UI、RPC/CLI、更新、重试和恢复任务。
- [ ] 唯一 Agent 槽位沿用现有语义：创建时原子取得，停止、停用或崩溃不释放；删除且确认运行时卸载完成后才释放。构造的 Agent spec 明确为一个实例，不要求 scheduler 再查槽位或裁剪数量。
- [ ] 创建原生 Agent 时满足已有通用契约：Zone owner、默认 Node host runtime、节点 executor/owner_agents 开放。不要靠 Lite 强制 Native 来修补错误配置。
- [ ] host 配置变更、Agent 删除和任务恢复不会留下反复要求容器运行的 spec；Agent Service 的宿主运行位置与额外 Agent Daemon 执行环境保持清晰。
- [ ] 明确安装失败反馈：无容器节点保证无法部署，不自动等于 inspect 立即失败。若需提前反馈，复用或补充产品入口/安装检查中的通用能力检查，不能为此修改核心调度算法。
- [ ] 正常产品入口不留下永远 Deploying 的不支持任务，不依赖 scheduler 的 Lite 准入分支完成清理。

### T3：预置脚本与原生 Agent 在旧调度器下运行

- [ ] 用旧基线实际生成 systest 的 spec 和实例配置，分别定位需求推导、包选择、运行方式三处障碍；先证明能表达，再决定配置/加载入口的最小改动。
- [ ] 在产品预装配置和 AppLoader 等现有机制中解决原生脚本执行，保持 App 身份、端口、服务发现、日志和状态上报契约；不得增加一个 Lite 专用调度器或后台持续改写调度结果。
- [ ] 不通过把脚本伪装成 Agent、伪造 author、宣称节点支持容器、手工启动脱管进程等方式通过验收。
- [ ] 原生 Agent 继续复用基线已有的 `native_agent_node` 与实例运行方式推导；验证重启、删除和 host 条件变化不会触发 Docker/wslc。
- [ ] node-daemon 保留无容器执行边界、原生进程管理与产品预置许可；调度器解耦不能扩大任意宿主程序的运行权限。

### T4：清理依赖与联动文档

- [ ] 撤销 Lite 引入的调度器产品配置读取/同步、产品准入、槽位与产品副本限制、按产品强制 Native 等依赖，保留无关业务改动。
- [ ] 清点共享类型和测试依赖；不能通过把产品判断搬进 scheduler 调用的公共 helper 来保留相同耦合。
- [ ] 更新旧 Lite TODO 的现状与相关手册：`buckyos_admin`、`buckyos_app_dev`、`opendan_agent_dev`；说明配置归属、安装失败反馈和原生运行判断。只更新实际受影响内容。

## 4. 回退验收是完成条件

以下验证必须实际执行并留证，不能只检查源码没有 `lite` 字符串，也不能只证明当前调度器能运行。

- [ ] 在隔离 checkout/worktree 中组合“改造后的其它模块 + 撤销 Lite 调度器改动后的 scheduler”，保留后续无关业务变更；记录精确提交与反向 patch。
- [ ] 再以 `ee65f12f8` 的调度器源码作为明确旧基线验证。为 bucket_store 等后续无关协议确需保留的差异逐项列明并单独验证；不得夹带 Lite 分支、原生脚本补丁或 Lite 提交新增的通用修复。无法运行旧基线时本项仍未完成，不能以选择性回退通过代替。
- [ ] 构建并安装真正回退后的 scheduler 可执行文件，记录源码版本、产物哈希、安装路径和实际运行进程，排除旧二进制未替换或后台仍运行新版的假通过。
- [ ] 同时覆盖全新 Lite 安装/激活和已有 Lite 重启。不得靠上次新版 scheduler 留下的 `system/product`、Native 实例配置或已启动进程通过验收。

| 场景 | 通过条件 |
| --- | --- |
| 基础系统 | Desktop、网关、system-config、控制面等健康；产品信息保持 Lite |
| 节点范围 | 普通节点不能加入；Agent Daemon 可加入并实际执行任务；调度器中没有支持容器的可选节点 |
| 预置脚本 | systest 由正式 AppLoader 原生托管，端口和网关访问正常；停止、崩溃恢复、重启可用 |
| 原生 Agent | 创建、对话、host 工具调用、产物交付及重启恢复正常；第二个 Agent 被产品入口拒绝，删除后可重新创建 |
| Static web | 安装、访问、更新、卸载完成，状态与实际路由一致 |
| 容器需求 | 容器 App/Agent 无法部署；正常入口反馈明确，不留下影响基础服务的无效调度任务 |
| 无容器依赖 | 无容器环境可运行；宿主已装 Docker/wslc 时仍无容器预载、启动或清理调用 |
| 完整版回归 | 原有容器应用、多 Agent 与节点扩展能力不受影响 |

- [ ] 按改动范围运行相关 Rust、配置生成、控制面/UI 检查，并用实际回退产物做 Lite DV；记录 Linux、Windows、macOS 的验证结果，未实测项明确保留。
- [ ] 无法放置的 spec 在旧基线下如何影响其它服务单独记录。若暴露独立调度器缺陷，单列问题与必要性证据，不将修复隐含为本次 Lite 回退验收的前提。
- [ ] 最终交付列出：非调度器改动、撤销的 Lite 调度器改动、保留的独立差异及理由、回退构建与运行证据、未完成项。

完成判定：**Lite 正常运行不再依赖这次 Lite 改造新增的调度器逻辑；实际回退相关调度器代码后，当前 Lite 功能仍成立。**
