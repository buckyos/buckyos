# AI Workspace 手写笔支持 TODO

日期：2026-10-25

状态：待实现。本文是后续 code agent 的实施任务单；“现状”来自源码核对，所有复选框均为尚未完成的工作。

目标：在 aiws 自由画布中提供适合持续记笔记的手写体验：压感书写、可靠的笔与触摸协调、橡皮擦、撤销，以及可保存、重开、同步的矢量笔迹。以 OneNote 的常用书写交互为参考，优先保证不误触、不丢笔、跟手。

## 1. 实现范围与默认决定

- 第一版支持自由画布正式笔迹，包含笔工具、颜色、粗细、压感开关、整笔橡皮擦、现有选择/移动/删除、撤销/重做、保存/重开和工作区导出/导入。
- 默认交互：笔写字；书写模式下单指不操作画布或对象；无活动笔划时双指导航；提供“锁定画布触摸”。鼠标选择笔工具后可画等宽线。退出书写模式后恢复现有鼠标、触摸、文字编辑行为。
- 复用放映模式的笔迹采集/绘制基础，但保留其临时板书生命周期；不得因复用代码而把放映标注自动写入正式文档。
- 复用现有 WorkspaceSession、commit、撤销、权限和离线队列。第一版无需自建笔迹服务、实时传输协议或手掌分类模型。
- 流式页手写块、OCR/手写转文字、套索、局部擦除、荧光笔、多种倾斜笔刷、笔的特殊手势、演示录制/回放属于后续扩展。本 TODO 不替代第四期演示规划，也不代表这些能力已完成。
- 遵守 [AGENTS.md](../AGENTS.md)。优先使用现有依赖；确需新增第三方依赖或通用组件时，按仓库规则处理。当前任务单不预先引入笔刷库。

## 2. 当前代码与接入点

下列链接相对本文；实施前重新核对当前分支，不能把规划文件当作已实现能力。

| 范围 | 当前实现与需要处理的差距 |
| --- | --- |
| [前端 README](../src/frame/desktop/src/app/aiworkspace/README.md)、[服务 README](../src/frame/aiworkspace/README.md) | 前端 React/TypeScript，自研画布与 Block 注册表；所有文档写入经过 WorkspaceSession。 |
| [presentation/overlays.tsx](../src/frame/desktop/src/app/aiworkspace/presentation/overlays.tsx) | 已有 Canvas 2D InkLayer、`InkPoint {x,y,t,p?}`、压感线宽和整笔擦除；缺少活动 pointer 身份校验、取消收尾和增强采样。 |
| [presentation/StageView.tsx](../src/frame/desktop/src/app/aiworkspace/presentation/StageView.tsx) | 笔迹在本场 React 内存中，有颜色、撤销、清空；没有正式文档持久化。 |
| [RenderHost.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/render/RenderHost.tsx)、[touch.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/render/touch.ts)、[camera.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/render/camera.ts) | touch 进入 TouchGestures，pen 仍走鼠标式对象操作；touch 的 move/up/cancel 另由 window 监听。`reset()` 不等于完整取消：还需取消委托对象拖动、清除点击历史并停止相机动画。 |
| [CanvasView.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx)、[catalog.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/catalog.ts) | 书写工具、插入入口、选择与剪贴板接入现有画布 UI，不能只在放映模式提供入口。 |
| [registry.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/registry.ts)、[registerAll.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/registerAll.ts)、[ops.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/ops.ts) | 可新增内置 `ink` renderer；普通 `buckyos.cell` 已支持无 source_ref 的 config。复用 `cellOp`、`setCellConfig` 等现有操作。 |
| [state/store.ts](../src/frame/desktop/src/app/aiworkspace/state/store.ts)、[state/undo.ts](../src/frame/desktop/src/app/aiworkspace/state/undo.ts)、[api/ids.ts](../src/frame/desktop/src/app/aiworkspace/api/ids.ts) | 正式写入走 `store.submit`；复用统一撤销和随机 ID，不沿用原型里的 `Date.now()` 笔划 ID。 |
| [core/src/types.rs](../src/frame/aiworkspace/core/src/types.rs)、[core/src/plan.rs](../src/frame/aiworkspace/core/src/plan.rs) | 单个 Cell 的 config 按 canonical JSON 计算上限 64 KiB；默认单 commit 最多 10,000 ops，inverse 有大小上限。不能把整页无限增长的笔迹塞进一个 config。 |

## 3. D0：冻结笔迹数据契约与容量边界

推荐第一版使用普通 Cell 承载笔迹，`view: {type: 'ink', version: 1}`，每笔默认一个 Cell。这样可复用现有保存、权限、撤销、复制、导出和离线能力。若实测规模性能不达标，先记录证据和替代设计，再修改这一决定。

- [ ] 定义并验证 ink config：数据版本、逻辑 `stroke_id`、分段序号、笔样式、原始坐标范围、采样点 `{x,y,t,p?}`。`t` 使用相对笔划起点的毫秒时间；倾斜可作为可选字段，缺失不伪造。
- [ ] 采样时用 Surface 世界坐标；保存时转换为笔迹局部坐标，`placement` 负责位置与变换。明确原始坐标范围到 placement 的映射，覆盖零宽/零高笔迹、点按、旋转、父 group 变换与画布缩放，禁止双重缩放。
- [ ] 用现有随机 ID 机制标识笔划。复制笔迹时生成新的逻辑 ID，避免橡皮按 `stroke_id` 连带删除原件。
- [ ] 按真实序列化字节数控制 config 大小，留出样式与元数据余量；不得仅用固定点数估计 64 KiB。超长笔迹可分成多个 Cell，保存共同 stroke_id、段序与连续边界点，并作为一个逻辑笔划选择、擦除和撤销。
- [ ] 一笔所有分段原则上在一次 commit 中创建。明确单笔最大预算及超限行为：可见提示、保留草稿、禁止静默截断；批量删除也需遵守 ops/inverse 上限。
- [ ] 定义有界的点简化策略，保留起止点、明显转折和压力变化。数据仍可重新渲染；不能只留截图或丢弃压力的 SVG 路径。
- [ ] 校验有限坐标、压力范围、时间顺序、非空有效点和格式版本；复用现有 renderer config 校验。若需修改 core/协议，检查前后端类型与 WASM 联动，不能只让浏览器单边接受。

完成条件：确定数据格式和长笔分段规则；有覆盖直线、点按、压力变化、长笔的 fixture，可通过既有文档提交与读回链路。

## 4. D1：笔与触摸的统一输入处理

书写/擦除工具启用时的输入规则如下；普通选择模式保留原有行为。

| 输入/状态 | 预期行为 |
| --- | --- |
| pen 落下 | 开始一笔或一次擦除；当前操作绑定唯一 pointerId。 |
| pen 悬停 | 可显示光标，不产生笔迹；不依赖设备必须支持悬停。 |
| 单个 touch | 不写字，不移动/选择对象，不滚动 Block，不打开长按菜单。 |
| 双 touch，无活动笔划且未锁触摸 | 仅平移/缩放；从双指变成单指后停止导航，剩余触点离屏前不能变成点击或单指拖动。 |
| 活动笔划期间的 touch | 忽略；即使笔已抬起，也继续忽略这些触点直到各自 up/cancel。 |
| 画布触摸锁开启 | 画布内全部 touch 均不触发导航或对象操作；笔、鼠标及正常工具栏操作仍可用。 |
| 鼠标主键 | 笔/橡皮工具可用；无真实笔压时使用等宽笔画。 |
| 第二支笔、非活动 pointer 的 move/up/cancel | 不覆盖、不结束、不修改当前笔划。 |

默认双指导航存在设备边界：系统可能把手掌上报为两个触点，在笔尚未被检测到时，网页无法保证区分它与主动双指导航。严格防止这类画布移动使用“锁定画布触摸”；默认模式的防掌触效果按真机记录，不宣称所有设备上都能自动识别。

- [ ] 在画布输入入口统一协调，顺序早于对象句柄、富文本及可点击 Block；不能只在 InkLayer 冒泡阶段拦截。限定书写区域，避免劫持工具栏、菜单、弹窗或页面其他输入。
- [ ] 维护活动 pointer、当前操作和被忽略的 touch pointerId；不能只靠 `isPrimary` 或 `buttons` 判断笔划归属。每个 up/cancel 只清理对应 pointer，避免触点互相结束。
- [ ] 笔落下时先将所有已按下的 touch pointerId 加入忽略集合（包括先于笔落下的触点），再取消现有触摸手势和未提交对象拖动，停止惯性、相机动画、长按定时器、待执行 RAF 和双击历史；抬笔不能清空忽略集合。协调 window 触摸监听，防止旧手势继续执行，不要将 `touch.reset()` 当作完整取消。
- [ ] 写字期间冻结本地相机导航（包括滚轮/触控板）或提供同等可靠的逐点坐标变换；切换 Surface 等结构性变化走中断收尾，不能让一笔落进两张画布。
- [ ] 使用 pointer capture，支持出界继续采样/抬笔。正常 pointerup 只提交一次；随后触发的 lostpointercapture 不重复提交。
- [ ] 明确取消规则：pointercancel、异常 lostpointercapture、Esc、切换工具/Surface、失去写权限、页面隐藏或组件卸载，均取消当前未提交预览并释放状态；已有成功笔迹不受影响。可保留恢复草稿，但不得将取消当成正常提交。
- [ ] 屏蔽书写区域的文字选择、拖拽与误触 contextmenu；`touch-action` 在手势开始前配置。它只负责浏览器默认手势，不是手掌识别开关。
- [ ] 正确处理 touch 被系统过滤、先传入后取消、没有后续 move、pointerId 复用、页面重新获得焦点等情况；下一笔能正常开始。

完成条件：手掌先落、笔先落、掌抬笔不抬、笔抬掌不抬四种顺序均不会产生杂线、对象移动或笔划串接；触摸锁开启时还须保证任何掌触不移动画布。默认双指模式在落笔前的多触点掌触边界按上文验收。系统不提供的触摸数据不自行推测为“可用双指导航”。

## 5. D2：压感采样与低延迟绘制

- [ ] 抽取现有 InkLayer 的采样/几何/绘制逻辑供正式书写和临时板书复用，分别接入各自的保存生命周期。
- [ ] 压力按 `0..1` 处理，并提供合理的线宽曲线及等宽开关；不要用 `p || 0.5` 吞掉合法的零值。无压感设备的固定 0.5 不代表真实压感，不能根据单个事件断言设备能力。
- [ ] 笔划点按产生圆点，慢速曲线不呈明显折线，快速书写不丢尾；pointerup 的零压力不能导致末段突变或错误回弹。静止时的压力变化也需保留。
- [ ] 基线使用 Pointer Events。存在 `getCoalescedEvents()` 时读取合并点，接口不存在或返回空数组时使用当前事件；按时间顺序采样，避免重复计入派发事件的尾点。去重不能删掉同坐标但压力/时间有变化的点。
- [ ] 活动笔划使用独立即时绘制层，采样缓存与 React 文档状态分离；不在每个采样点触发全工作区重渲染、提交或网络请求。
- [ ] 已完成笔迹使用缓存、视口裁剪和合适的简化显示；禁止照搬原型在每个 pointermove 清屏并遍历所有历史点的做法。避免每笔一个持续运行的 RAF。
- [ ] 在此基础上评估预测点：`getPredictedEvents()` 只用于可替换的临时笔尖尾迹，真实点到达即覆盖；预测点绝不进入保存、撤销、命中测试和协同数据。
- [ ] `pointerrawupdate`、desynchronized canvas、Ink API 仅作为实测后启用的增强项，基线不依赖它们。使用 raw 输入时避免与 pointermove 重复采样；增强缺失仍应完整书写。
- [ ] DPR、窗口尺寸、画布缩放变化后清晰且坐标一致；正式 renderer 与实时层线宽/曲线一致，提交后不闪烁、不移位、不突然变形。

完成条件：关闭所有增强接口仍可正确书写；开启合并采样后曲线更完整；可证实预测点未入库。

## 6. D3：文档保存、编辑和 UI 闭环

- [ ] 注册 ink renderer 与工具入口，提供笔、整笔橡皮、颜色、粗细、压感开关和画布触摸锁。中英文文案接入现有 i18n；快捷键避开输入框和既有编辑器。
- [ ] 笔设置与触摸锁放入现有用户偏好；实际笔迹样式进入文档。书写模式只在允许编辑的自由画布开启，只读用户和锁定对象遵守现有 capability/mode 规则。
- [ ] 正常抬笔后通过 `store.submit` 提交一笔，并接入全局撤销/重做。下一笔不等待上笔网络确认；连续落笔仍保持稳定 ID、顺序、保存状态与可恢复的待提交内容。
- [ ] 提交结果未知、网络失败和权限拒绝走既有状态/重试机制；保留真实点与原提交标识，重试不重复创建笔迹。正式渲染接管后才移除待提交显示，既不闪空，也不重复绘制。
- [ ] 整笔橡皮按实际线段距离和笔宽命中，不能只检查离散点或矩形包围盒；考虑 zoom/旋转/父 group。一次擦除拖动形成一个撤销步骤，中断则回滚未提交擦除预览；多段长笔按逻辑笔划处理。
- [ ] 选择、移动、复制粘贴、删除、跨 Surface 移动、分组和缩放复用现有交互。至少保证默认笔划和多段笔划行为一致；不可留下“显示成一笔，操作时散成几段”的状态。
- [ ] 验证保存重开、版本恢复、工作区导出/导入、另一客户端可见与并发各自新增笔划；不得用整页数组覆盖导致远端笔迹丢失。同步完成笔划即可，实时远端笔尖可后置。
- [ ] 在已有离线能力支持的环境中验证新增/擦除/撤销、重载恢复和重连提交。Safari/Firefox 的在线书写不能被 Chromium 离线前提阻塞；当前未支持的平台不额外承诺离线能力。
- [ ] 保持放映临时板书的翻页保留、撤销、清空和退出释放语义；共享采样修复后对放映模式单独回归。

完成条件：用户可以持续写一页笔记，编辑、撤销、刷新重开、导出导入后仍保留相同的笔迹和压力效果。

## 7. D4：验证、规模与真机验收

自动化验证要断言用户可见结果、持久数据和副作用，不能仅测试实现内部状态。

| 编号 | 场景 | 必须满足 |
| --- | --- | --- |
| H01 | pen 的 down/move/up，压力递增，点按、快写、出界抬笔 | 线宽变化、圆点、起止点正确，每笔仅一次提交。 |
| H02 | 手掌先落/笔先落；手掌抬起但笔仍写；笔抬起后手掌继续移动 | 无杂线、截断、选择变化或对象提交；活动笔划及被隔离触点不移动画布；触摸锁模式下所有掌触均不移动画布。另记录默认双指模式中笔被检测前的多触点掌触表现。 |
| H03 | 双指手势/对象拖动/惯性未结束时开始书写 | 旧手势被取消，笔迹落点稳定，无延迟长按菜单或错误拖动提交。 |
| H04 | 另一 pointer 的 move/up/cancel，第二支笔，pointerId 复用 | 活动笔划只由所属 pointer 控制；结束后新笔划正常。 |
| H05 | cancel/lost capture/Esc/切工具/切页/失权/页面隐藏 | 取消无幽灵笔迹、重复提交、遗留 capture；下一笔可用。 |
| H06 | 无合并/预测接口、合并数组为空、预测后真实轨迹转向、无真实压感 | 正常降级；预测点不保存；没有零压力误处理。 |
| H07 | 压力简化、64 KiB 边界、超长分段、逻辑笔划复制和擦除 | 编码不超限、不丢关键点；一次逻辑操作对应一次撤销，副本独立。 |
| H08 | 缩放、DPR、旋转、父 group、移动/复制/跨页 | 笔迹与命中位置一致，擦除不会误删相邻线条。 |
| H09 | 保存失败/结果未知/重试、重载、导入导出、两客户端同时新增 | 可恢复、无重复、无覆盖；完成笔迹持久可读。 |
| H10 | 已准备离线副本：断网写字、撤销、重载、重连 | 本地恢复正确，重连后收敛，无重放重复。 |
| H11 | 退出书写、只读、放映临时标注、桌面鼠标和原触摸导航 | 原有编辑/导航/放映语义保持正常。 |

- [ ] 用现有 `tests/aiworkspace/fixtures.ts` 与 `offline-fixtures.ts`，新增 `handwriting.spec.ts`、`offline-handwriting.spec.ts`；触摸导航回归参考 `mobile.spec.ts`。纯状态机/几何测试放入现有合适的测试入口，避免另建测试框架。
- [ ] 合成 PointerEvent 只证明应用输入分派；使用浏览器真实输入路径补验 capture、滚动、默认行为。真机防掌触与端到端延迟不能用合成事件通过替代。
- [ ] 规模场景至少覆盖约 1,000 笔/200,000 采样点及一次超长连续笔划，记录硬件、浏览器、刷新率、数据大小、采样处理/绘制耗时、提交耗时、内存和交互表现。检查 BlockHost 挂载预算/LOD 不会让笔迹变成占位框或不可见。
- [ ] 在目标设备上连续书写至少 10 分钟，覆盖慢写小字、快速连笔、抬笔密集点划和掌触；目标是输入/绘制在目标刷新率的帧预算内，不随历史笔迹量持续恶化。记录真实笔尖延迟的测量方式，不把 JS handler 耗时当成端到端延迟。
- [ ] 真机矩阵：Windows + 有压感笔 + Edge/Chrome；iPad + 支持压感的 Apple Pencil + Safari；Android + 支持压感的笔 + Chrome。记录实际设备、系统与浏览器版本；无设备的组合明确列为未验证，不勾选通过。
- [ ] 如使用 Apple Pencil USB-C 等无压感硬件，验收等宽降级而非真实压感；hover、侧键/尾部橡皮仅在硬件实际提供时验收，始终保留 UI 操作入口。

实施后的检查命令（新增 spec 完成后执行；本 TODO 编写时未运行这些代码测试）：

```bash
# 在 buckyos/src/frame/desktop/ 下运行
pnpm check
pnpm exec eslint src/app/aiworkspace tests/aiworkspace
deno test --allow-read --allow-env tests/datamodel/aiworkspace-i18n.test.ts
pnpm exec playwright test --config=playwright.aiworkspace.config.ts handwriting.spec.ts offline-handwriting.spec.ts mobile.spec.ts presentation.spec.ts
pnpm build
```

Playwright 配置会启动隔离的真实 AIWorkspace/NDM 后端，离线用例使用生产构建。按实际改动补跑 objects/canvas/collab/permissions 等相关用例；若更改 Rust core/store，则在 `buckyos/src/` 运行 `cargo test -p aiworkspace-core -p aiworkspace-store`，并按实际影响验证服务及重建 WASM。不用无关全量测试掩盖缺失的真机验证。

## 8. D5：交付与实施回写

- [ ] 更新前后端相关 README，说明书写入口、触摸策略、数据格式、容量边界、离线限制和已验证平台。
- [ ] 检查 `aiworkspace_run`、`aiworkspace_template_dev` 手册与 `aiws/renderers.json` 等 renderer 消费入口，更新受影响的创建/渲染说明；无影响时记录判断依据。不得让 Agent 创建的笔迹与 UI 使用不同格式。
- [ ] 在本文末尾追加实施记录：实际落点、与默认方案的差异及原因、验证命令与结果、真机矩阵、性能数据、尚未完成项。按证据勾选任务，不把“浏览器支持 API”写成“真机书写已验收”。
- [ ] 交付时列出 schema/协议/WASM/依赖是否变化；本地用户无关改动保持原状。

推荐实施顺序：D0 数据契约 → D1 输入处理 → D2 即时绘制 → D3 保存与 UI → D4 验证 → D5 回写。D1/D2 可共享小型验证画布推进，但最终交付必须进入 aiws 正式自由画布。

## 9. 浏览器依据

这些是实现边界，不能当作跨设备效果保证；实现时继续使用特性检测。

- [W3C Pointer Events：primary pointer 与防掌触](https://www.w3.org/TR/pointerevents3/#the-primary-pointer)：设备/系统/浏览器可抑制并发触摸；不能依靠 `isPrimary` 排除手掌，没有通用 `isPalm` 标记。
- [MDN pressure](https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/pressure)：归一化压感；无压感设备激活时可返回固定 0.5。
- [MDN pointercancel](https://developer.mozilla.org/en-US/docs/Web/API/Element/pointercancel_event)、[touch-action](https://developer.mozilla.org/en-US/docs/Web/CSS/touch-action)：必须正确处理取消，并区分默认导航控制与手掌过滤。
- [WebKit Safari 18.2](https://webkit.org/blog/16301/webkit-features-in-safari-18-2/)：已加入合并/预测采样接口；不要沿用“Safari 不支持”的旧判断。
- [MDN getCoalescedEvents](https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/getCoalescedEvents)、[getPredictedEvents](https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/getPredictedEvents)、[Ink API](https://developer.mozilla.org/en-US/docs/Web/API/Ink_API)：增强能力及可用环境需检测，不能阻塞基线书写。
