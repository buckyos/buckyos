# AI Workspace 扩展控件 P2 + P3 TODO：事件与运行态、属性级协作、来源升级、HTML parent、Agent 组合结果、模板包、回放 / 手机 / 宿主能力验证

日期：2026-10-30

状态：**未开工；§1.2 Q26–Q33 已由用户 2026-10-30 裁决“全部按建议”，视为冻结。**依据 [扩展实现思路 v0.9 §8](<../doc/workspace/BuckyOS AI Workspace 扩展实现思路.md>) 的 P2 / P3 两行、§1.4 / §6.5 / §6.9 / §6.10、§7.2 Q1 / Q9 / Q10 / Q13、§7.4.3 C2 / C3，以及 P0 / P1 实施记录（[报告 §6 / §8.7](<../doc/workspace/BuckyOS AI Workspace 扩展控件实施记录与验收报告.md>)）的未完成项。P2 与 P3 合成一单：两者共享 `manifest_version: 3` 与 host API 3 的一次升版，事件 / parent / Agent 组合结果都落在同一个 HTML 宿主与同一个 `ext.build` 校验器上，拆开做要升两次版本。§1.2 的 **Q26–Q33 是本单新裁决项**，用户已确认全部按建议值执行。

目标：四段。**前段**（M1）给实例属性加属性级修订与撤销、离线 / 旧客户端写入的定义修订检查，这是事件 handler 与迁移写入实例的基础。**主体一（P2，M2–M3）**交付公共事件与动作绑定（`events` / `on` / `aiws.emit`、host API 3 保留 2）、设计 / 运行态、执行幂等与“重载不重放”，以及来源包显式升级与结构迁移（替换 `EXT_IDENT_EXISTS`）。**主体二（P3，M4–M5）**交付 HTML `parent`、`grid` 布局、Agent 组合结果、Workspace 模板包（导出 / 入库 / 从模板创建）。**收尾**（M6–M7）：回放 / 公共端 / 手机 / 宿主能力不足的验证与回退，P0 / P1 遗留 DV，手册、文档与 hello-controls 回写。

## 1. 范围与已定决定

### 1.1 已定（设计文档与 P0 / P1 实施已冻结，只复述）

- **事件在 P2、`parent` 在 P3，加事件接口时 host API 升 3 并保留 2 的读取路径**（§7.4.3 C2、Q19 向下兼容）；`component.json` / `composition.json` 里今天出现 `events` 即 `BUILD_FAILED`（`store/src/ext.rs` `EVENTS_LATER`），P2 放开。
- **设计 / 运行态与 `edit / view / show`、读写权限分别建模**（§1.4）；一次点击以当前操作者身份发起动作，需要共享的执行复用 command / run 记录与幂等；其他端观察结果，不因挂载或收到同步事件重复产生副作用；**重载不等于再次执行业务**（§6.7）。
- **属性级协作**：目标是不同属性可独立合并、同一属性检测冲突，个人撤销保留他人无关修改（§1.4）；依赖旧定义的写入携带版本条件，离线重放也检查（Q9）。
- **来源升级**：一 Workspace 一身份一定义（Q1）；同身份异来源今天一律 `EXT_IDENT_EXISTS`，P2 进入升级流程，列本地改动与受影响实例，不静默覆盖本地改动（§6.2 表第 3 步）；迁移是用户的事（README “Compatibility”），失败保留最后可用修订。
- **模板包**：复用控件包结构，`workspace_templates` 已预留且非空即拒绝（Q13、`library.rs` `verify_bundle`）；安装不建工作区；两次创建相互独立；“把模板添加到已有 Workspace”的合并规则后续单独设计（§6.9）。
- **Agent**：许愿格 control 结果不接受 `kind: composite`（P1 出入 12），P3 放开为按定义身份产出组合结果；Agent 的读写用自身授权。
- **公共端 / 回放**：v1 公共端与回放只渲染内置 Block 与 Block-control，HTML 扩展只发布受控状态 / 静态快照（第四期 Q06、P4-06）；不能把 live 界面按钮禁用后继续挂载会自动执行的扩展。
- **原则**：所有系统服务对 Agent 可用；DV 全新安装不写兼容；错误类别只看枚举；不新增第三方 JSON Schema / 打包库；主菜单顶层不再加项。

### 1.2 本单新裁决 Q26–Q33（用户 2026-10-30 裁决：全部按建议）

| # | 问题 | 建议 | 理由 / 代码依据 |
| --- | --- | --- | --- |
| Q26 | **事件与动作的数据契约** | 定义侧：`component.json`（`manifest_version: 3`）加 `events: { <name>: { title?, description?, params?: { <p>: { type: string \| number \| boolean \| object \| entity, description? } } } }`（名字 `[a-z][a-z0-9_]{0,31}`，≤ 16 个）；HTML 定义 `host_api: 3` 用 `aiws.emit(name, params)` 触发；declarative 新 item `button { label, emit, params? }`；composite 的 `composition.json` 加 `events: { <name>: { from: { slot, event } } }` 只做冒泡（build 校验该 slot 接受的 `cell:<ident>` 定义声明了那个事件）。实例侧：Cell / composite 容器新键 `on: { <event>: Handler \| Handler[] }`，Handler 三种：`{ kind: "write", target: "<binding>", what: "recordSet" \| "upsert" \| "setMarkdown", args }`、`{ kind: "wish", wish_id, inputs? }`、`{ kind: "open" \| "notify", target? \| text }`；字符串位置允许 `{{event.<p>}}` / `{{config.<k>}}` 模板。内核只查 `on` 的形状（原语类：对象、≤ 16 事件、每个 ≤ 4 handler、`kind` 枚举、≤ 16 KiB）；“事件存在于定义、binding 存在”是业务校验（Desktop 表单 + `ext.build` 依赖者 / 实例检查） | `write` 的词汇与 host API 的 `aiws.record / table / text` 完全一致（`bridge.ts` `write` 分支已有三种实现，handler 只是把调用者从 iframe 换成宿主）；`wish` 复用 `proc.start`；不引入第四种“command”对象 |
| Q27 | **设计 / 运行态怎么建模** | 画布级本地状态 `interactive`（不持久化、不进 `user_state`），工具栏一个 ▶ / ✎ 切换；缺省 edit → 设计、view → 运行、非公开 show → 只对 `presentation.live` 的 Block 运行、回放 / guest → 关且不可开。运行态：点 HTML Block 即激活（仍受 `html` 预算）、declarative `button` 可点、handler 执行；设计态：点击 = 选择 / 移动，`emit` 不执行而是在 Block 角标显示“事件 X（设计态，未执行）”作为调试信息，手动“运行一次”照旧。写权限：`write` / `wish` handler 与 HTML 自己的 `aiws.record().set` 一样，要求 `policy.writes`，**或** view 模式 + 运行态 + 调用者有 `update` 能力（view 的 `writes: false` 不再一刀切）。流式页取消“编辑即自动挂载”，与画布同规则 | `modePolicy` 在 `ui/blocks/registry.ts:337`，`bridge.ts:86 requireWrites` 只看 `policy.writes`，`HtmlBlockHost.tsx:86` 的 `run` affordance 只在 `edit`；§6.5 要求统一验收流式页例外 |
| Q28 | **属性级协作与撤销的内核形态** | 新 Operation `entity.set_props { entity_id, key: "config" \| "bindings" \| "on", props: { <name>: value \| null }, expect?: { <name>: rev } }`：每个属性一条修订 `key_revs["p:<key>.<name>"]`（`null` 删除），冲突按属性报 `REVISION_CONFLICT`（`conflicts[]` 列各属性当前 rev 与值），逆操作同形（撤销只还原自己改过的属性）；`entity.set_keys` 整键写入时把该键全部 `p:` 修订一并提升。`entity.set_keys config` 保留给 build / 迁移整体替换。Desktop 的 Inspector / `InstanceForm` / composite Inspector / HTML 的 config 写入改走 `set_props` | 完全照搬 `dir.set_member` 对 `m:<name>` 的做法（`plan.rs:896-945`，`dir.replace` 提升全部 `m:` 修订）；`ops.ts:45/75`、`composite/Inspector.tsx:31` 今天整键写 `config` 带 `expect.rev` 所以甲改颜色乙改文字必冲突 |
| Q29 | **离线 / 旧客户端写入的定义修订检查** | 不加新机制：客户端对实例的每次 `set_props` / `set_keys` Commit 带 `preconditions: [{ target: { entity_id: <def> }, expect: { rev: <本地 outline 的定义 content_rev> } }]`；定义已重建则 `REVISION_CONFLICT`，Desktop 提示“控件已更新，表单已按新定义刷新，请重试”；副本重放经同一规划器同样检查 | `plan.rs:86/626` 已有 read-set `preconditions`（`ext_build` 用它锁定义与目录修订），副本 `submit_local` 走同一 `plan` |
| Q30 | **来源升级与迁移的接口** | 新 RPC `ext.upgrade { workspace_id, items: [{ def_entity_id, pkg_id? }], migration?: { <ident>: Migration }, force?, dry_run? }`（同 Zone 从本机库取新版本；跨 Zone 走 `ext.build { provenance, upgrade: true, migration? }` 内联）。流程：库里该身份的版本 = `provenance.version` 且 digest 同 → `unchanged`；源码目录内容摘要 ≠ 库里 `provenance.version` 那个控件的 `digest` → `local_changes`，无 `force` 则 `INVALID_OPERATION / EXT_LOCAL_CHANGES`；否则走 `ext_build` 重建路径（`opts.upgrade` 跳过 `EXT_IDENT_EXISTS` 与 `expect`），narrowing 时套用 `Migration { config?: { rename?, drop?, fill? }, bindings?: { rename?, drop?, fill? }, slots?: { rename?, move? } }`——服务对每个受影响实例生成 `entity.set_props` / `tree.move { slot }` 放进同一 Commit，迁移后再跑兼容检查，仍不兼容整次失败；同包依赖按拓扑一并升级；origin `build`，message `upgrade <ident> <old>→<new>`，`provenance` 更新。`ext.build` 也接受 `migration`（本地开发收窄接口时用）。`dry_run` 回 `plan: [{ instance, ops }]`。依赖者（composite）升级后仍不兼容 → 失败，迁移依赖者不在本单 | `ext.rs:1099-1105` 的 `EXT_IDENT_EXISTS` 分支、`:1177` 本地重建保留 `provenance`、`:835 narrowing` 已列出全部收窄种类，正好是迁移表的键 |
| Q31 | **HTML `parent`** | host API 3 的 `aiws.createBlock({ renderer, parent?: "self" \| "<entity id>" \| { binding }, slot?, placement?, data?, config? })`：`parent` 是 composite 容器时必须给 `slot`（`child_allowed_here`），是树形控件节点时 placement `"auto"`，`"self"` 只在树形控件内允许（`in_tree_control`）；缺省仍建兄弟 | `bridge.ts:344` 今天固定 `target.cell.parent_id`；内核规则已在 P1 |
| Q32 | **模板包形态：快照式（A，建议）还是声明式 JSON（B，hello-controls 现提案）** | **A 快照式**：模板 = 一次 `materialize` 产出的内容闭包（`templates/<id>/snapshot.json` + `objects/`，随 `aiws` 子包进 `.pikg`，digest 盖住）+ 清单项 `{ id, title, description?, requires: { host_api, controls }, params: { <k>: { type, title, default } }, reset: { tables: [ids] } }`。导出 = 服务 `ext.export_template`（选中 Surface 子树 + 引用闭包 + `data/extensions` 依赖闭包 + 选中的数据实体，排除 runs / shows / session / personal）；创建 = `ext.create_from_template` 走 `ws.import new` 同一条 `load_workspace`（新 id、调用者 owner、富文本新 lineage），随后一次 `internal` Commit 做 `{{params.<k>}}` 替换与 `reset.tables` 清行。**不**用 Q17 重映射（新库无 id 冲突）；“加到已有 Workspace”仍不做。hello-controls 的 `team-board.template.json` 在 M0 改成清单项形态 | 第四期 Q02 已选“整库克隆”而非逐项载入，理由同样适用（富文本 / 表 / ink / 资产引用逐项声明必漏）；`service.rs:330 import` + `materialize.rs:353 load_ops` 现成；B 要再写一套物化器 + 重映射器，且表达不了富文本与 ink |
| Q33 | **回放 / 公共端 / 手机 / 宿主能力不足** | 只做验证与可读回退：`interactive` 在回放与 guest 锁 false；`html.api_version` 高于宿主时 Block 内显示快照 + “需要宿主 API n”而不是抛错；composite `layout.algorithm` 未知时按 group 排布并标注；`manifest_version` 高于宿主的包在库列表标“需要新版本”且不可引入；手机：composite / HTML 只读（不新增能力）。新增一个布局算法 `grid { columns, gap }`（K2，宿主自带） | `htmlRuntime.ts:161` 今天版本不等即 throw；`controls/composite.ts:27` 算法联合类型 |

**版本**：`manifest_version` 升 3（`events`、declarative `button`、composite `events`、清单 `workspace_templates` 非空）；HTML host API 升 3（`emit`、`createBlock.parent`、`context.interactive`），宿主同时接受 2 与 3；block-def 新键 `events`（定义）、`on`（实例，Cell 与 composite 容器）；PROTOCOL 不升。

## 2. 当前代码与接入点

| 范围 | 现状与差距 |
| --- | --- |
| [core/src/plan.rs](../src/frame/aiworkspace/core/src/plan.rs)（`preconditions` :86 / :626；`dir.set_member` / `dir.replace` 的 `m:` 修订 :896-945；`tree_clone`、`tree_move { slot }`）、[model.rs](../src/frame/aiworkspace/core/src/model.rs)（`key_revs` :118） | 无属性级修订。加 `entity.set_props`（M1）；`operations.json` / `commit-request.schema.json` 登记；WASM 重建。 |
| [core/src/types.rs](../src/frame/aiworkspace/core/src/types.rs)（`deps` 守卫 :1741、`derived.build` 守卫 :1988、`check_composition_shape`、`validate_primitive` / `validate_schema`） | block-def 无 `events`，Cell / 容器无 `on`。原语类：形状；业务类：不校验（Q15）。 |
| [core/src/wish.rs](../src/frame/aiworkspace/core/src/wish.rs)（`RESULT_TYPES` :32、control 结果拒 composite :194）、[store/src/wish/results.rs](../src/frame/aiworkspace/store/src/wish/results.rs)（`control` :527 / :656） | Agent 组合结果（M4）：`control` 结果加 `def` + `slots`。 |
| [store/src/ext.rs](../src/frame/aiworkspace/store/src/ext.rs)（`EVENTS_LATER` :186 / :358 / :561；`narrowing` :835；`BUILD_OWNED_EXTRA` :891；`ext_dependents` :969；`ext_build` :1004；`EXT_IDENT_EXISTS` :1099-1105；本地重建保留 `provenance` :1177；`html_api` :1335；清单 `workspace_templates: []` :1372） | 放开 `events`（定义 / composite 冒泡校验）、declarative `button`；`opts.upgrade` + `migration`；`ext.export_template`；清单模板项。 |
| [store/src/library.rs](../src/frame/aiworkspace/store/src/library.rs)（清单键 :62、模板拒绝 :73、DDL :245-253：`library_packages` / `library_controls`） | 加 `library_templates` 表（`owner, pkg_id, template_id, title, info_json, snapshot_obj, objects_json`）；`verify_bundle` 校验模板项与其对象闭包、`requires.controls` 在包内。 |
| [store/src/service.rs](../src/frame/aiworkspace/store/src/service.rs)（`import` :330：`new` / `restore` 语义；`fork` :439 用 `materialize_for` + `load_workspace`）、[core/src/materialize.rs](../src/frame/aiworkspace/core/src/materialize.rs)（`materialize_with` :236、`load_ops` :353）、[store/src/export.rs](../src/frame/aiworkspace/store/src/export.rs)（`closure` :72） | 模板导出 = 带过滤的 `materialize_with`；创建 = `load_workspace` 从库对象；都不新写格式。 |
| [server/src/ext.rs](../src/frame/aiworkspace/server/src/ext.rs)（`import` :160、`pack`、`dispatch` :190+）、[server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs) 方法表 :530-537 | 加 `ext.upgrade`、`ext.export_template`、`ext.create_from_template`（experimental）；`ext.pack` 带 `templates`。 |
| [schemas/](../src/frame/aiworkspace/schemas/)（`component` v2、`composition`、`block-def`、`aiws-manifest`（`workspace_templates` `maxItems: 0`）、`operations.json`、`error-codes.json`） | 升 3；新 schema `template.schema.json`（清单项）、`migration.schema.json`；错误码 `EXT_LOCAL_CHANGES`、`EVENT_UNDECLARED`（build 诊断码）。 |
| [controls/composite.ts](../src/frame/aiworkspace/controls/composite.ts)（`algorithm: 'stack' \| 'fixed'` :27、`solveComposite` :118） | 加 `grid`；未知算法回退。 |
| Desktop [ui/extensions/htmlRuntime.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/htmlRuntime.ts)（`HTML_API_VERSION = 2` :21、API 表 :80-140、版本相等检查 :161）、[bridge.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/bridge.ts)（`makeBridge` :74、`requireWrites` :86、`context.mode` :134、`write` 三分支 :271-342、`createBlock` :344）、[HtmlBlockHost.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/HtmlBlockHost.tsx)（`run` affordance :86） | host API 3：`emit`、`createBlock.parent`、`context.interactive`；接受 2 / 3；handler 执行器（新文件 `ui/extensions/events.ts`）；手势门。 |
| Desktop [ui/blocks/registry.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/registry.ts)（`CanvasMode` :23、`modePolicy` :337）、[ui/blocks/ops.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/ops.ts)（整键写 `config` :45 / :75 / :86）、[ui/controls/composite/Inspector.tsx](../src/frame/desktop/src/app/aiworkspace/ui/controls/composite/Inspector.tsx)（:31）、[ui/extensions/InstanceForm.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/InstanceForm.tsx)、[ui/canvas/tools.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/tools.tsx)、[ui/canvas/FlowSurface.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/FlowSurface.tsx) | `interactive` 状态与切换；`set_props` + 定义修订前置条件；事件绑定表单（`on`）；流式页去自动挂载。 |
| Desktop [ui/extensions/ExtensionsDialog.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/ExtensionsDialog.tsx)、[controlSources.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/controlSources.ts)、[ui/shell/dialogs.tsx](../src/frame/desktop/src/app/aiworkspace/ui/shell/dialogs.tsx)（模板页签 `TEMPLATES` :36-67 固定三项）、[api/demos.ts](../src/frame/desktop/src/app/aiworkspace/api/demos.ts) | 定义详情“有新版本 → 升级…”；新建对话框模板页签列本机库模板；“保存为模板…”。 |
| [buckyos-websdk `src/aiws_ext.ts`](../../buckyos-websdk/src/aiws_ext.ts)（`checkComponent` v2、`checkComposition`、`AIWS_MANIFEST_VERSION = 2`）、[cli/modules/aiws.ts](../../buckyos-websdk/cli/modules/aiws.ts)（`build` :512、`pull` :560、`ext-export` :618、`ext-import` :700）、`pikg_protocol.ts` | v3 预检（`events`、`button`、模板项）；CLI `ext-upgrade`、`template list \| create \| export`；`aiworkspace_client.ts` 加三个方法。 |
| 手册 [aiworkspace_template_dev.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md) “Controls”、[aiworkspace_run.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_run.md) | 无事件 / `on` / 升级 / 模板 / 组合结果。 |
| 测试 | core `tests/{composite,clone,apply_log,extension}.rs`；server `tests/ext.rs`；websdk jest `aiws_ext.test.ts`；CLI deno；Desktop e2e `tests/aiworkspace/{extensions,wish,session,mobile,offline}.spec.ts`；DV `tests/dv/aiworkspace.extensions.spec.ts`。 |

## 3. M0：契约冻结（只写 README / schemas / hello-controls，不写实现）

- [ ] **`component.json` v3**：`events`（Q26 形状）；HTML `host_api: 3`；declarative item `button { label, emit, params? }`（`emit` 必须在 `events` 里）；composite `composition.json` `events: { <name>: { from: { slot, event } } }`。`schemas/component.schema.json` / `composition.schema.json` 升 3，旧 1 / 2 照常读。
- [ ] **block-def 新键 `events`**（定义，build 写；形状同 `component.json`）；**实例键 `on`**（Cell 与 composite 容器；Q26 Handler 形状；原语类只查形状）。outline 投影定义的 `events` 名单与实例 `on` 的事件名（供 Desktop 角标与 Agent 查询）。
- [ ] **host API 3**：`aiws.emit(name, params) → Promise<{ handled }>`；`aiws.createBlock` 的 `parent` / `slot`；`aiws.context.interactive`；`aiws.version` 为定义声明的版本（2 或 3）。README host API 表加一节“API 3”，明确 2 的成员不变。
- [ ] **`entity.set_props`**（Q28）；`operations.json`、`commit-request.schema.json`；`conflicts[]` 的属性级形状。
- [ ] **`ext.upgrade`**（Q30）与 `Migration`（`schemas/migration.schema.json`）；`ext.build` 的 `upgrade` / `migration` 入参；错误码 `EXT_LOCAL_CHANGES`；`EXT_IDENT_EXISTS` 文案改为指向 `ext.upgrade`。
- [ ] **模板**（Q32）：清单项 schema（`schemas/template.schema.json`）、包内布局 `templates/<id>/{snapshot.json,objects/}`、`ext.export_template` / `ext.create_from_template` 入参出参、`ext.pack { templates }`、`ext.list_library.templates`；`aiws-manifest.schema.json` 放开 `workspace_templates`。
- [ ] **Agent 组合结果**（Q31 / M4）：`aiws.result.control(name, { def: "<ident>", config?, slots: { <slot>: [ "result:<n>" | "input:<n>" | { view, markdown } ] }, under? })`；写进 `catalog/controls.json` 的 composite 宿主自述与 task.md。
- [ ] **权限表**：`emit` 执行 handler = 调用者自己的能力（`write` 要 `update` 或 `policy.writes`，`wish` 要 `wish`）；`ext.upgrade` = `ext.build` 同；`ext.export_template` = 所选 Surface `export`；`ext.create_from_template` = 账号（`may_own`）。
- [ ] **hello-controls**：`hello-card/component.json` 加 `events.completed`、`behavior.js` 的注释改真代码（M0 只改形状，`host_api: 3`）；`hello-board/composition.json` 加 `events.card_done` 冒泡；`team-board.template.json` 改为清单项形态（去 `surfaces` / `blocks` 提案，加 `snapshot` / `reset` 说明）；README 第 6 / 7 / 10 / 11 / 12 步标记改“P2 / P3 形态已冻结”。

完成条件：README + schemas 一次 review；websdk `checkComponent` v3 分支对三个例子无诊断；本单后续 M 不再改名字。

## 4. M1：属性级协作与定义修订检查（前段，先于 M2）

- [ ] `plan.rs` `entity.set_props`：`p:<key>.<name>` 修订、按属性 `expect`、逆操作、`set_keys` 整键提升；`entity.unset_keys` 同步清 `p:`。`read.rs` / outline：`key_revs` 投影加 `p:`（Desktop 表单要用）。
- [ ] `types.rs` 原语类：`key` 枚举、属性名 `[a-z][a-z0-9_]{0,63}`、值 ≤ 64 KiB；业务类不动。
- [ ] Desktop：`ops.ts` / `InstanceForm` / composite `Inspector` / `bridge.ts` 的 config 与 bindings 写入改 `set_props`（每次只提交改动的属性）；撤销栈按属性；所有实例写入带定义修订 `preconditions`（Q29），`REVISION_CONFLICT` 且 `target` 是定义时提示并刷新表单。
- [ ] 离线：`replicaSession` 透传 `preconditions`；e2e `offline.spec.ts` 加“离线期间定义被重建，上线重放被拒并提示”。
- [ ] 测试：core `tests/props.rs`（并发两属性都保留、同属性冲突、撤销只还原自己、`set_keys` 后旧 `p:` 过期、前置条件拒绝）；Desktop e2e `collab.spec.ts` 加“甲改颜色、乙改文字均保留”。

完成条件：§8 P2 验收第一句“甲改颜色、乙改文字均保留”在 e2e 成立；重放路径不新增 schema 类校验（哨兵测试不变）。

## 5. M2：事件、动作绑定与设计 / 运行态（P2 主体一）

- [ ] **服务**：`ext.rs` `compose` 读 `events`（三种 kind）、declarative `button`、composite 冒泡校验（slot 的 `cell:<ident>` 定义有该事件，否则诊断 `EVENT_UNDECLARED`）；`narrowing` 加“事件被删 / 参数变必填”并列出绑定了它的实例（`on` 含该事件）；依赖者重校验覆盖冒泡来源。`html_api` 取 max 到 3。
- [ ] **内核**：block-def `events` 形状、实例 `on` 形状；`tree.clone` 复制 `on`（内部 `target` 指向子树内绑定时随重映射）。
- [ ] **Desktop host API 3**：`htmlRuntime.ts` 接受 `api_version` 2 / 3，bootstrap 按版本注入成员；`bridge.ts` 加 `emit`（设计态只记角标、运行态交给执行器）、`context.interactive`；**手势门**：宿主监听同源 iframe 的 `pointerdown` / `keydown`，挂载后首个手势之前的 `emit` 丢弃并 `notify`。
- [ ] **执行器** `ui/extensions/events.ts`：按实例 `on[event]` 顺序执行；全部 `write` handler 合成一个 Commit（`idempotency_key: evt/<instance>/<event>/<event_id>`，label “<控件>: <事件>”，可撤销）；`wish` → `proc.start` 同 key；`open` / `notify` 本地；模板替换 `{{event.*}}` / `{{config.*}}`；composite 冒泡：子实例事件触发后，沿 `slot` 向上找 composite 容器的定义 `events.from` 匹配项，再执行容器实例的 `on`。
- [ ] **设计 / 运行态**（Q27）：`store.ui.interactive` + 工具栏切换（`tools.tsx`，edit / view 可切，show / replay / guest 锁）；`modePolicy` 加 `events` 字段，`requireWrites` 改为 `policy.writes || (mode === 'view' && interactive && capabilities.update)`；运行态点击 HTML Block 即激活；declarative 解释器渲染 `button`；composite Chrome 在运行态不拦截子 Block 点击；`FlowSurface` 去编辑自动挂载。
- [ ] **事件绑定表单**：`InstanceForm` / Inspector 加“事件”页：每个定义事件一行，选 handler kind、目标绑定 / 许愿格、参数映射；实例 `on` 经 `set_props { key: "on" }` 写。
- [ ] **重载不重放**：build 后 HtmlActive 以新 `html` 重挂载（已有），执行器丢弃旧 runtime 的迟到 `emit`（runtime nonce 校验已有）；e2e：点一次 → 一条 Commit；改 `behavior.js` build 后再点 → 新实现、仍一条；刷新页面无新 Commit。
- [ ] **websdk / CLI**：`checkComponent` v3（`events`、`button`、composite `events`）；`controlSkeleton` 的 html 骨架带一个示例事件；CLI 动词不变。
- [ ] 测试：server `ext.rs` 加 events 成功 / 未声明冒泡 / 删事件列实例；core `on` 形状与 clone；Desktop e2e `extensions.spec.ts` 加：运行态点 Done → 记录 status=done 且 idempotent；设计态点击只选中并出角标；composite 冒泡 `card_done` → 容器 handler `notify`；view 模式有 `update` 的协作者可写、只读者被拒。

完成条件：§8 P2 验收第二句——一次点击只执行一次，改行为文件并 build 后下一次点击用新实现，重载不重放旧动作。

## 6. M3：来源升级与结构迁移（P2 主体二，可与 M2 并行）

- [ ] **服务** `ext.upgrade`（Q30）：库查找、`unchanged` / `local_changes` 判定（目录内容摘要 vs 库 `digest`）、`opts.upgrade` 进 `ext_build`、`Migration` 应用（`entity.set_props` / `tree.move { slot }` / 删除被 drop 的 binding）、迁移后复检、依赖拓扑、`dry_run` 计划；`ext.build` 收 `upgrade` / `migration`；`ext.import` 的 `EXT_IDENT_EXISTS` data 加 `library_version` 供客户端直接转入升级。
- [ ] **websdk / CLI**：`aiworkspace_client.extUpgrade`；CLI `aiws ext-upgrade <def> --workspace $WS [--pkg <pkg_id>] [--migration file.json] [--dry-run] [--force]`；跨 Zone：`buildFiles` 带 `upgrade: true`。
- [ ] **Desktop**：定义详情比较 `provenance.version` 与 `ext.list_library` → “有新版本 x.y.z → 升级…”；对话框列本地改动（`unbuiltChanges` 思路 + `local_changes`）、受影响实例、接口变化；迁移表单最小版（改名下拉、默认值、slot 映射）；`EXT_IDENT_EXISTS` 时“从本机库添加…”直接给“升级”按钮。
- [ ] 测试：server 加 升级 unchanged / 本地改动拒绝与 force / 收窄 + migration 一次 Commit 实例覆盖与内容保留 / dry_run 计划 / 依赖同升；Desktop e2e：装 0.2.0 → 升级 → 实例 `config` 覆盖与卡片内容不变。

完成条件：§8 P2 验收第三句——包升级 / 迁移保留实例覆盖与内容；旧客户端写入冲突可诊断（M1 已给）。

## 7. M4：HTML `parent`、`grid` 布局、Agent 组合结果（P3 前半）

- [ ] `bridge.ts` `createBlock` 的 `parent` / `slot`（Q31）；WASM 暴露 `child_allowed_here` 供预检（或由服务拒绝 + 可读错误，二选一，建议前者）。
- [ ] `controls/composite.ts` 加 `grid { columns, gap }`；`composition.schema.json` 算法枚举加 `grid`；未知算法回退 group（Q33）。
- [ ] 许愿格：`core/src/wish.rs:194` 放开 `def` 形式的 control 结果；`results.rs` 规划：解析 ident → 工作区定义（缺 → 错误“先 ext-import”）、建 composite 容器 + slot 子 Block（`result:<n>` 的 Cell 放进 slot，`input:<n>` 克隆引用，内联 richtext 新建）；`under` 支持 composite 的 slot。
- [ ] 手册 `aiworkspace_run.md` 与 `task.md`（catalog）加组合结果示例。
- [ ] 测试：server `wish` 测试加组合结果；e2e `wish.spec.ts`（mock LLM）加一例；Desktop e2e `parent` 建子 Block 进 slot。

## 8. M5：Workspace 模板包（P3 主体）

- [ ] **服务**：`ext.export_template`（`materialize_with` 过滤：选中 Surface 子树 + 引用闭包 + `data/extensions` 依赖闭包 + 选中数据实体；排除 `runs` / `shows` / `session` / personal；`params` / `reset` 写进清单项）；`ext.pack { templates }` 与 `ext.install` 入 `library_templates`；`verify_bundle` 校验模板项、对象闭包与 `requires.controls` 在包内；`ext.list_library` 列 templates；`ext.create_from_template`（`load_workspace` + `internal` Commit 参数替换与清行；两次创建独立；安装不建工作区）。
- [ ] **PIKG**：`aiws` 子包不变（模板在同一 bundle）；`pikg_protocol.ts` 校验模板项；Installer 路由不改。
- [ ] **websdk / CLI**：`aiws template list`、`template create <pkg_id>#<id> --title … --param k=v`、`template export --workspace $WS --surfaces s1,s2 --id team-board --title … [--param k:type:default] [--reset t1]`（产物进 `--out` bundle，随 `ext-export --pikg` 打包）。
- [ ] **Desktop**：新建对话框“模板”页签合并本机库模板（内置三项保留），参数表单；工作区菜单“导出 → 保存为模板…”（选 Surface、参数、重置表）→ 进本机库或写 bundle；库对话框显示模板。
- [ ] 测试：server `ext.rs` 加 模板导出 → pack → install → create 两次独立 / 清行 / 参数替换 / 缺控件拒绝；Desktop e2e：保存为模板 → 新建自模板 → KPI 实例绑定到新表。

完成条件：§8 P3 验收——安装不建工作区、两次创建内容独立、定义与实例引用正确（同一快照内无需重映射）。

## 9. M6：回放 / 公共端 / 手机 / 宿主能力不足验证 + 遗留 DV

- [ ] 回放与 guest：`interactive` 锁 false，HTML 只快照，事件不执行；e2e `session.spec.ts` 回放里 hello-card 不挂载、composite 正常排布。
- [ ] 宿主能力不足（Q33）：`api_version` 高于宿主 → 快照 + 文案；未知布局算法 → group + 标注；库列表 `manifest_version` 过高标记并禁引入；e2e 各一例（mock 定义）。
- [ ] 手机：`mobile.spec.ts` 加 composite / HTML 只读一例。
- [ ] **遗留 DV**（全新安装、真实 Zone）：L1(1) 跨 Zone 中转引入（ssotest 三 Zone VM）；L1(2) Agent 只凭手册建 declarative + composite 并 build；L2 第三账号四控件；真实子域 `aiws-<id>.<zone>` build / watch；Desktop “定义为控件”；本单新项各一条（事件、升级、模板、Agent 组合结果）。

## 10. M7：手册、文档与例子回写

- [ ] 手册 `aiworkspace_template_dev.md`（事件 / `on` / 升级 / 模板 / `parent`）、`aiworkspace_run.md`（组合结果、运行态写权限）。
- [ ] aiworkspace README（协议表、host API 3、Validator classes 新键归类）、Desktop README、设计文档 §1.5 / §7.2（Q1 / Q9 / Q10 / Q13 状态）/ §8 P2 P3 行改“已实施”、《实施记录与验收报告》追加 P2 / P3 节。
- [ ] hello-controls：M0 的形态改成真实可 build（`checkComponent` v3 无诊断、`aiws-manifest.json` 升 3 并加模板项与摘要）；README 第 6 / 7 / 10 / 11 / 12 步与 §2 总表 `[P2]` `[P3]` 改可用。

## 11. 不在本单

- 把模板添加到已有 Workspace（合并）；依赖者（composite）的自动迁移；任意新布局算法从包加载；React 插件动态加载（Q8）；服务端签名；`.block.*`；作者 DID 自动填充（Q25 已否）。
- 公共端 live HTML 扩展（第四期 Q06：等独立域名程序隔离）；session feed 事件进时间线之外的新事件通道。

## 12. 验证命令

- core / store / server：`cargo test --no-fail-fast -p aiworkspace-core -p aiworkspace-store -p aiworkspace`；WASM：`PATH=/tmp/dev-cache-root/cargo/bin:$PATH CARGO_TARGET_DIR=/tmp/dev-cache-root/cargo-target bash src/frame/aiworkspace/wasm/build.sh`；目录：`deno run -A src/frame/aiworkspace/controls/tools/build_catalog.ts`；`cd src/frame/aiworkspace/controls && deno test -A`。
- websdk：`npx jest tests/aiws_ext.test.ts`；CLI：`cd cli && deno task test`。
- Desktop：`pnpm check && pnpm exec eslint src/app/aiworkspace tests/aiworkspace && pnpm build`；e2e：`AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/{extensions,collab,offline,wish,session,mobile}.spec.ts`（跑 e2e 时不改 Desktop 源码）。
- 真实 Zone DV：`uv run start.py --all` 全新安装后 `BUCKYOS_UI_DV_BASE_URL=… BUCKYOS_TEST_ADMIN_PASSWORD=… pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.extensions.spec.ts`；跨 Zone 用 ssotest 三 Zone VM 组。

## 13. 验收（对照设计 §8 P2 / P3 行）

- P2：甲改颜色、乙改文字均保留；一次点击只执行一次，改行为文件并 build 后下一次点击用新实现，重载不重放旧动作；包升级 / 迁移保留实例覆盖与内容；旧客户端写入冲突可诊断。
- P3：Agent 通过手册发现 / 引入 / 实例化控件并产出组合结果；缺少宿主能力时可读回退且数据保留；公共端 / 回放不执行现场业务动作；模板：安装不建工作区、两次创建内容独立、定义与实例引用正确。
- 跨阶段回归（设计 §8 列表）不退化；P0 / P1 遗留 DV 全部勾掉。

## 14. 顺序与并行

M0 → M1 → { M2 ∥ M3 } → { M4 ∥ M5 } → M6 → M7。M1 必须先于 M2（handler 与迁移都用 `set_props`），M2 与 M3 只共享 M0 的 schema；M4 的 `parent` 依赖 M2 的 host API 3 骨架，M5 不依赖 M2–M4。建议一次冻结 M0 后按两条线并行推进。
