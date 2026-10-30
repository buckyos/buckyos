# AI Workspace 扩展控件 P0 TODO：编辑 → build、本机库、PIKG 控件包

日期：2026-10-29

状态：**2026-10-30 M0–M5 已实施（未提交），M6 手册已改、真实 Zone DV 见 §12**。实施中与本单的出入见 §12.1。原状态：未开工。依据 [doc/workspace/BuckyOS AI Workspace 扩展实现思路.md](<../doc/workspace/BuckyOS AI Workspace 扩展实现思路.md>) v0.8（D1–D8、§7.4 A1–A7 与 Q18 / Q19 全部已裁决，§7.5 影响评估）推导 §8 的 P0。第 2 节“现状”是 2026-10-29 对源码的核对（基线 `688df1577` + 未提交集），实施前重新核对当前分支。组合控件（composite）、HTML `parent`、事件绑定、来源包升级、模板包创建都不在本单（§8 P1–P3）。

目标：跑通“在 home 目录写一个 render 控件 → build 进自己的 Workspace 调试 → 多选导出为本机全局可用或 `.pikg` → 另一个用户在另一个 Workspace 引入并实例化”的完整路径；两条实现路线中只交付 render-block（declarative / html），但所有契约为组合控件留位。

## 1. 范围与已定决定

只复述实施需要的结论，不再讨论：

- **编辑 → build 原子部署**（D6 修订，A1 / A2）：源码事实来源是开发者 home 目录；`build` 把一次源码快照交服务**同步**校验，通过则一次 `doc.commit` 原子更新定义实体、源码快照 `buckyos.dir` 与实现资源，失败不提交。没有“已保存未生效”的共享中间状态，原 `source_revision` / `active_revision` 合一为 build 修订。Text Editor 与 websdk `content.ts` 零改动。
- **home 目录布局只是建议**：`/home/<owner>/AI Workspace/<workspace>/controls/<control>/`；build 以显式参数或个人 `user_state`（`aiws:src:<defId>`）定位目录；定义里不存绝对路径。
- **服务不读 NFSP 路径**：客户端用 NFSP `list` 收集每个文件的 `obj_id`（home 文件就是 NDN FileObject），服务以调用者 token 经 NDM 网关 `get_object` 取字节（复用 `authorize_object`）。
- **Workspace 快照是“已部署”状态**：`data/extensions/<def>/` 下定义实体 + 源码快照 dir + 资源；`pull` 把快照写回 home（客户端只读 Workspace、写 NFSP，不经 commit）。协作是 git 式：pull → 改 → build；旧基线 build 被 `expect` 拒绝。
- **身份**（A3）：控件身份 = 作者 DID + 控件名，字符串形式 `<author_did>#<name>`（不含 `/`，≤ 128 字节，可直接当实体 `name` 借内核“同级 name 唯一”去重）；新 payload 键 `ident { author_did, name }`；`def_id` 保留为宿主内短 id，正则不改。
- **PIKG 新类型 `aiws`**（A4）：Rust `AppType::Aiws`（wire `"aiws"`）、TS allowlist、product 准入、`app_submit_route` 新路由 → 调 aiworkspace 服务 `ext.install`；安装不建 AppSpec、不起服务。AIWS 清单在包 payload 内，控件集合为公共部分，`workspace_templates` 预留为空（Q13）。
- **本机库由 aiworkspace 系统服务持有**（A5）：自有 zone bucket + 服务 SQLite 表；`ext.list_library` 给 Desktop / CLI；“导出为全局可用”= bundle 后直接调 `ext.install`，不经 Installer。跨 Zone 引入由客户端中转（从自己 Zone 取 bundle，向目标 Workspace build）。
- **导出在客户端打包**（A6）：服务 `ext.export` 返回 bundle 清单（定义 + 源码快照 + 依赖闭包），客户端写到 home 临时目录（如 `AI Workspace/exports/<pkg>/`）再 `pikg pack`。
- **`config_schema` 子集**（A7）：`type / enum / default / required / title / description / minimum / maximum / minLength / maxLength`；前端用现有 `z.fromJSONSchema` 校验与生成表单，内核继续不校验值域（Q15）。
- **declarative 认命名绑定**（Q18）：解释器 `bind` 语法加 `<binding>.<expr>`；单 `source` 继续兼容。
- **向下兼容**（Q19）：`component.json` 与 AIWS 清单带 `manifest_version`；新版本只加可选字段；build / import 接受旧 `manifest_version`；`schemas/` 先追到 PROTOCOL 1.1；新增 block-def 键不升 PROTOCOL。
- **一个 Workspace 一身份一定义**（Q1）；引入落 `data/extensions/`（Q5）；权限复用能力集（Q9）：build / 引入 = 目标 folder `structure` + 定义 `update`，导出 = `export`。
- **Q14 收窄**：`derived.build { input_digest, builder_version, built_at }` 由服务在 build 的同一 commit 内以系统主体经 `entity.set_derived` 写；无异步任务。
- **原则**：所有系统服务对 Agent 可用，Agent 经网关用自己的 token 访问 nfs-server / aiworkspace，与其他客户端同路；DV 中“容器内 nfs-server 不可达”按 bug 修，不在设计里绕行。
- 遵守 [AGENTS.md](../AGENTS.md)：不新增第三方 JSON Schema / 打包库；复用 websdk `pikg_protocol.ts`。

## 2. 当前代码与接入点

| 范围 | 现状与差距 |
| --- | --- |
| [core/src/types.rs](../src/frame/aiworkspace/core/src/types.rs) `validate_block_def`（:1512-1574） | 允许键 `def_id, version, kind, title, accepts, allow_no_source, default_size, declarative, html, config_schema, actions, inspector, description`；`def_id` 只查格式不查唯一；`kind` 闭集 `declarative｜html`；`config_schema` 只要求是对象；≤ 512 KiB。需要加 `ident`、`manifest_version`、`source`（源码快照 dir 引用）、`bindings` 声明、`catalog`、`agent`、`provenance`（来源包 / 版本 / 摘要）；`kind` 不动。 |
| 同文件 `buckyos.dir`（:1076-1197）、[plan.rs](../src/frame/aiworkspace/core/src/plan.rs) `dir.set_member / remove_member / replace`（:862-916） | 成员 `{kind:file, object_id}` 必须已上传或 staged；每成员 `m:<name>` 修订；`dir.replace` 对 `content_rev` 带 `expect`。build 的 commit 直接复用，不加操作。 |
| plan.rs `def_ref` 强边 `def`（types.rs:2087；model.rs:238） | 删除被引用定义给 `REFERENCE_BROKEN`。需要定义 → 源码 dir 的引用边（B5），让导出闭包与删除保护覆盖源码快照。 |
| [core/src/read.rs](../src/frame/aiworkspace/core/src/read.rs) `outline_extras`（:316-318） | 投影 `def_id` / `def_kind`。加 `ident` 字符串与 `manifest_version`，目录与去重用。 |
| [schemas/](../src/frame/aiworkspace/schemas/) | `commit-request.schema.json` 仍是 `protocol_version 1.0`，selector 枚举缺五种；`model.rs:213` 引用不存在的 `references.json`；无 block-def / dir payload 定义。Q19：先追到 1.1。 |
| [wasm/build.sh](../src/frame/aiworkspace/wasm/build.sh) | core 改后重建；离线副本 `submit_local` 走同一 `plan_commit`。 |
| [server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs) `authorize_object`（:822-836）、`asset.import_object`（:875-904）、方法表（:478-490）、[store/src/export.rs](../src/frame/aiworkspace/store/src/export.rs) `closure`（:72-112）、`stage_object`（:316-331） | 已有：以调用者 token 经网关判定 FileObject 可读、`ws.stage_object` 入 staged（24 h TTL）、资源闭包计算。新 `ext.*` 方法进方法表为 `experimental`，并录 [fixtures/protocol/stable-methods.json](../src/frame/aiworkspace/fixtures/protocol/stable-methods.json)。现有命名空间 `ws / doc / control / proc / lock / diag / replica / asset / show`。 |
| [store/src/wish/plan.rs](../src/frame/aiworkspace/store/src/wish/plan.rs) `plan_html`（:1399-1440） | 许愿格已把 HTML 结果落成 block-def（`def_id = wish.<hash>`）+ Cell，`bindings` 写 Cell。P0 不改它的落点；给它补 `ident`（作者 = Workspace owner，name = `wish.<hash>`）与 `manifest_version`，使目录统一。 |
| [Desktop ui/blocks/registry.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/registry.ts) `resolve`（:269-320）、[useBlockContext.ts](../src/frame/desktop/src/app/aiworkspace/ui/blocks/useBlockContext.ts)（:62-74） | Cell 订阅 `e:<defId>` 重解析；`config_schema` 用 `z.fromJSONSchema` 校验（`snapshot` 键排除）。热更新路径现成。 |
| [ui/extensions/HtmlBlockHost.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/HtmlBlockHost.tsx)（:42-60）、[htmlRuntime.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/htmlRuntime.ts)、[bridge.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/bridge.ts)（:31-36） | remount key 是 `JSON.stringify(definition.html)`；实现内联在定义 payload。P0 实现仍内联（build 把源码文件内容写进 `html {html,css,js}`），源码快照另存 dir；remount key 不必改。`bindings` 由 bridge 消费。 |
| [ui/extensions/declarative.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/declarative.tsx)（:51-104） | 只读 `context.source`；`bind` 语法 `props.<key>` / `count` / `sum:<field>` / `<field>`。Q18：加 `<binding>.<expr>`。 |
| [ui/canvas/catalog.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/catalog.ts) `extensionEntries`（:78-89）、[InsertCatalog.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/InsertCatalog.tsx) `EntryDetail`（:138-219）、[DataDetail.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/DataDetail.tsx) `BlockDefDetail`（:93-116） | 扩展页签只列 block-def，详情只选单个 `source`，无 config / 命名绑定输入，空态文案“可在数据源中导入”无对应功能；block-def 详情只读 JSON。需要：实例初始化表单、从本机库添加、工作区控件管理页（构建 / watch / 导出 / 编辑源码 / 诊断）。 |
| Desktop [app/content/open.ts](../src/frame/desktop/src/app/content/open.ts)、[filebrowser/launch](../src/frame/desktop/src/app/filebrowser/launch.ts)（`filesOpenLaunch`） | 打开文件 / 文件夹已有；aiworkspace 目录下无任何调用。“编辑控件源码…”用它打开 home 目录。 |
| [buckyos-websdk/src/nfsp.ts](../../buckyos-websdk/src/nfsp.ts) | `list`（`EntryTarget.attrs.obj_id`）、`stat`（`NodeInfo.obj_id`）、`mkdir`、`uploadFile`、`watch`（SSE，lossy）。`ref.type === 'live'` 时无 `obj_id`。CLI 中只有 `share.ts` 用过 `NfspClient`。 |
| [buckyos-websdk/src/aiworkspace_client.ts](../../buckyos-websdk/src/aiworkspace_client.ts)、[cli/modules/aiws.ts](../../buckyos-websdk/cli/modules/aiws.ts) | 客户端无 `get_changes` / `asset` / `export` / `control` 封装；CLI 动词 `info / read / query / controls / import / export / run-*`。 |
| [buckyos-websdk/cli/modules/pikg_protocol.ts](../../buckyos-websdk/cli/modules/pikg_protocol.ts)（:905-959）、[pikg.ts](../../buckyos-websdk/cli/modules/pikg.ts)（:58-63, :888-907） | `app_type` 闭集 `service｜dapp｜web｜agent｜agent_skills`；AppDoc 键严格 allowlist；`init --kind` 四种；pack 不签名。 |
| [buckyos-api/src/app_doc.rs](../src/kernel/buckyos-api/src/app_doc.rs) `AppType`（:37-57）、`AppDocWire`（:581-625 `deny_unknown_fields`）、builder 规则（:1270-1374）；[product.rs](../src/kernel/buckyos-api/src/product.rs)（:364-409） | 加 `Aiws`；子包 key `aiws`（`SubPkgList.others` 已允许任意 key）；准入规则：只允许 `pkg_list.aiws`，无 docker / web / native。 |
| [control_panel/src/app_installer.rs](../src/frame/control_panel/src/app_installer.rs) `app_submit_route`（:2043-2051）、[agent_mgr.rs](../src/frame/control_panel/src/agent_mgr.rs) `register_agent_template`（:2498-2578）、[pikg.rs](../src/frame/control_panel/src/pikg.rs) reader | Agent + Rpc → 注册模板是现成的“资源型安装”样板。新路由 `Aiws + Rpc → RegisterAiwsPackage`：reader 校验后把 staged PIKG 交给 aiworkspace `ext.install`。 |
| [doc/pikg/format.md](../doc/pikg/format.md) §5 消费方表、[doc/pikg/README.md](../doc/pikg/README.md) | 要加 `aiws` 类型与 AIWS 清单的消费方说明（格式权威仍在 PIKG 文档）。 |
| 手册 [cmd_skills/aiworkspace_template_dev.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md)、`aiworkspace_run.md`、`buckyos_app_dev.md` | 无控件开发内容。 |
| 测试 | core `core/tests/*.rs`；server `server/tests/*.rs`；websdk 单测；Desktop e2e `tests/aiworkspace/*.spec.ts`；DV `tests/dv/aiworkspace.*.spec.ts`。 |

## 3. M0：契约冻结（只写文档与 schema，不写实现）

目标：下面每个名字在 M0 结束后不再变；写进 aiworkspace README 协议表与 `schemas/`。

- [x] **`component.json`**（`manifest_version: 1`）：`ident { author_did, name }`、`kind: declarative｜html`、`title`、`description`、`host_api`（最低宿主 API，html 对应 `api_version`）、`accepts`、`allow_no_source`、`default_size`、`config_schema`（A7 子集）、`bindings: { <name>: { accepts, required, description } }`、`entry`（html: `{ html, css?, js? }` 文件名；declarative: 文件名）、`assets[]`、`catalog { group, icon?, description }`、`agent`（Markdown 文件名）、`deps[]`（P0 必须为空，占位）。写 JSON Schema 到 `schemas/component.schema.json`，共享 TS 与 Rust 各一份校验（Rust 在 build 服务端，TS 在客户端预检与 `pikg` 校验）。
- [x] **block-def payload 新键**：`ident`、`manifest_version`、`source { entity_id }`（源码快照 dir）、`bindings`（声明）、`catalog`、`agent`、`provenance { pkg_id?, version?, digest? }`；`derived.build { input_digest, builder_version, built_at }`。`ident` 字符串形式 `<author_did>#<name>` 同时作实体 `name`。
- [x] **`ext.build`**：入参 `{ workspace_id, folder_id（默认 data/extensions）, files: [{ name, obj_id }], expect?: { def_rev?, dir_rev? }, provenance?, dry_run? }`；出参成功 `{ commit, def_entity_id, build: derived.build, diagnostics: [] }`，失败 `{ code: BUILD_FAILED, diagnostics: [{ file, line?, col?, severity, message }] }`，无部分提交。输入摘要 = 文件 `obj_id` 排序串接 + `manifest_version` + `builder_version`。
- [x] **`ext.export`**：入参 `{ workspace_id, entity_ids[] }`；出参 bundle 清单 `{ aiws_manifest, files: [{ path, obj_id, size }] }`，客户端经 `GET /kapi/aiworkspace/asset/<ws>/<obj_id>` 取字节。缺依赖 / 未 build 改动（客户端比对 home）各自报错，不生成半套。
- [x] **AIWS 清单 `aiws-manifest.json`**（`manifest_version: 1`，Q13）：`purpose: "aiws"`、`host_api`、`controls: [{ ident, kind, version, digest, dir: "controls/<name>/" }]`、`workspace_templates: []`（预留）。包内布局 `controls/<name>/component.json + 文件`。
- [x] **`ext.install` / `ext.list_library` / `ext.import`**：`install { pikg: { bucket_id, key } | { obj_id }, mode: "installer" | "dev" }` 校验 AppDoc(`aiws`) + 清单后登记；`list_library {}` → `[{ pkg_id, version, author, controls: [...] }]`；`import { workspace_id, folder_id, items: [{ pkg_id, ident }] }` 同 Zone 服务端复用 build 内部路径（文件已在本机库 bucket）。跨 Zone：客户端 `ext.library_bundle { pkg_id }` 取清单后对目标 Workspace 调 `ext.build`（带 `provenance`）。
- [x] **PIKG `aiws` 类型**：AppDoc `app_type: "aiws"`、`pkg_list.aiws` 单子包、`categories[0] = "aiws"`；Rust / TS 两端规则一致；准入只允许 Rpc 提交。
- [x] **CLI 动词**：`buckyos aiws build <dir> --workspace <ws> [--watch] [--dry-run]`、`aiws pull <def> --workspace <ws> --out <dir>`、`aiws export --workspace <ws> <def...> --out <dir> [--install]`、`aiws library list`、`aiws import --workspace <ws> <pkg_id>#<ident>...`、`pikg init --kind aiws-controls`。
- [x] **权限表**：build / import = 目标 folder `structure` + 已有定义 `update`；export = `export`；install(dev) = 调用者本人；list_library = 任意登录用户看自己的库。
- [x] `schemas/` 追到 PROTOCOL 1.1（selector 枚举、`protocol_version`、删 `references.json` 引用）；`core/tests/schemas.rs` 加对 `commit-request.schema.json` 的一致性检查。README 协议表加 `ext.*`。

完成条件：README + schemas 一次 review 通过；本单后续 M 不再改名字。

## 4. M1：内核

- [x] `validate_block_def` 新键与校验：`ident`（DID 格式 + name 正则 + 字符串 ≤ 128）、`manifest_version`（正整数，≤ 当前）、`source` 引用必须是活的 `buckyos.dir`、`bindings` 声明形状、`catalog` / `agent` 大小上限、`provenance` 形状；总大小仍 ≤ 512 KiB。
- [x] 引用边：`source` → dir 记为强边 `source`（阻止删 dir、进导出闭包）；`model.rs` 强边表加入；`check_ref_target` 覆盖。
- [x] `outline_extras` 投影 `ident`、`manifest_version`、`provenance.pkg_id`。
- [x] 许愿格 `plan_html` 补 `ident` 与 `manifest_version`（不改落点）。
- [x] 测试：`core/tests/extension.rs`（新键校验、source 边删除保护、ident 作 name 的同级唯一冲突、旧 payload（无新键）仍可读）；重建 WASM；`stable-methods.json` 重录。

完成条件：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace` 全绿；旧 fixture 全部仍能导入。

## 5. M2：服务

- [x] `server/src/ext.rs`：`ext.build`：按清单逐文件 `authorize_object` + 网关取字节（并发、总大小上限沿用 32 MiB/文件与 512 KiB 定义上限）→ 解析 `component.json`（Rust 校验器）→ 组装定义 payload（html 内联 / declarative 内联）→ `ws.stage_object` 每个文件 → 一次 commit：定义 create / set_keys（`expect`）、`dir.replace`（`expect`）、`entity.set_derived build`（系统主体，同一 commit）→ 返回。`dry_run` 只返回诊断。
- [x] 依赖者兼容检查钩子：P0 只对“同身份已有定义”检查 `accepts` / 必填 `bindings` / `config_schema.required` 的收窄，列出引用实例（`ws.list_referrers`）并失败；composite 依赖留接口。
- [x] `ext.export`：从定义 + `source` dir + `closure` 算文件清单，生成 `aiws-manifest.json` 内容（服务端算 digest），返回清单；不写任何桶。
- [x] 本机库：`ext.install`（读 PIKG：复用 `control_panel` 的 reader 逻辑能否 crate 化，否则在服务内按 `format.md` 解析 zip + `PACKAGE_META`；校验 AppDoc `aiws` + 清单 + 每控件 `component.json`；文件入服务自有 bucket `aiws_library`，索引进服务 SQLite 表 `library_packages / library_controls`）；`ext.list_library`；`ext.library_bundle`；`ext.import`（同 Zone：从库 bucket 取文件走 build 内部路径，带 `provenance`）。
- [x] 方法表登记 `experimental`；错误码 `BUILD_FAILED`、`LIBRARY_PACKAGE_INVALID` 进 `error-codes.json` 与 `Code::ALL`。
- [x] 测试：`server/tests/ext_build.rs`（成功 / 校验失败不提交 / 旧基线 `expect` 冲突 / 收窄 bindings 列出实例 / dry_run）、`ext_export.rs`（闭包与清单 digest）、`ext_library.rs`（install → list → import 同 Zone）。

完成条件：用测试里的最小 declarative 与 html 控件，build 后 `doc.read` 出的定义能被 Desktop 现有 `resolve` 解析。

## 6. M3：websdk 与 CLI

- [x] `AiWorkspaceClient` 补 `getChanges`、`waitChanges`、`assetImportObject`、`exportDoc`、`ext*` 全部封装；`controls*` 顺带补。
- [x] 共享模块 `buckyos-websdk/src/aiws_ext.ts`（Desktop 与 CLI 共用）：`collectSources(nfsp, dir)`（`list` 递归、取 `attrs.obj_id`，遇 live 节点等待 / `stat` 重试并报“文件尚未落对象”）、`build(client, ws, files, opts)`、`pull(client, nfsp, ws, defId, outDir)`（`doc.read` dir 成员 → asset GET → `uploadFile`）、`watch(nfsp, dir, onChange)`（SSE + debounce 500 ms + 内容摘要相同不 build）、`exportBundle(client, nfsp, ws, ids, outDir)`（取清单与字节、写 `aiws-manifest.json` 与 `controls/<name>/`）、`compatCheck(manifest)`（Q19：旧 `manifest_version` 升格读取）。
- [x] CLI `aiws.ts` 加 M0 动词；参照 `share.ts` 建 `NfspClient`；`export --install` = bundle → `pikg pack` → `ext.install { mode: dev }`。
- [x] `pikg_protocol.ts`：`app_type` 加 `aiws`；AppDoc 规则：`pkg_list.aiws` 单子包、无 docker / web；pack 时校验子包内 `aiws-manifest.json` 与每个 `component.json`（TS 校验器）。`pikg init --kind aiws-controls` 生成骨架（一个 declarative 样例）。
- [x] 单测：`collectSources` live 等待、`compatCheck`、pikg `aiws` 规则、argv。

完成条件：不开 Desktop，仅用 CLI 完成 build → export → pack → install(dev) → 在第二个 Workspace import。

## 7. M4：PIKG / Installer（Rust）

- [x] `app_doc.rs`：`AppType::Aiws`（`"aiws"`）；builder 规则 `pkg_list.aiws` 必有且唯一；`product.rs` 准入只接受 Rpc。
- [x] `app_installer.rs`：`app_submit_route` 加 `Aiws + Rpc → RegisterAiwsPackage`、`Aiws + Preinstall → Reject`；执行时 reader 校验后以安装者身份调 aiworkspace `ext.install { pikg: { obj_id }, mode: "installer" }`；安装记录只在 aiworkspace 库中，不写 `users/{owner}/apps/*`。
- [x] `doc/pikg/format.md` §5 加 `aiws` 行；`README.md` 指向本单与设计 §6.9。
- [x] 测试：reader 对 `aiws` 包通过 / 篡改清单失败；route 单测。

完成条件：`buckyos app install <aiws.pikg>` 不产生 AppSpec、不起服务，`ext.list_library` 能看到。

## 8. M5：Desktop

- [x] **工作区控件管理页**（数据源模式 `data/extensions/` 入口 + 主菜单“扩展”）：列表（ident、kind、版本、来源、引用实例数）、多选“导出控件…”（bundle → 写 home `exports/` → 调用 CLI 同一共享模块 pack → 可选“全局可用”）、“从本机库添加…”（`ext.list_library` → 多选 → `ext.import`；跨 Zone 时走客户端中转）、“新建控件…”（在 home 建骨架 + 立即 build + 插入测试实例）。
- [x] **定义详情**：“编辑控件源码…”（无工作副本先 `pull`，再 `filesOpenLaunch` 打开目录）、“构建”、watch 开关、最近 build 诊断与 `derived.build`、引用实例列表（`ws.list_referrers`）、移除（被引用拒绝）。`user_state` 键 `aiws:src:<defId>`。
- [x] **实例初始化表单**（B8）：`InsertCatalog` 对 def 条目按 `config_schema`（A7 子集，`z.fromJSONSchema`）生成字段，按 `bindings` 声明逐项选数据（必填校验），写 Cell `config` 与 `bindings`；“先只引入定义”仍可。`SelectionInspector` 用同一表单编辑 `config`。
- [x] **declarative 读 `bindings`**（Q18）：`bind` 语法 `<binding>.<expr>`，`watch` 每个绑定实体。
- [x] 扩展页签空态文案改为指向“从本机库添加 / 新建控件”；`BlockDefDetail` 显示 ident / 来源 / build 信息。
- [x] 手机：不提供引入与构建入口，列表只读。
- [x] i18n en + zh；e2e `tests/aiworkspace/extensions.spec.ts`（新建 → build → 实例更新；build 失败不改实例；初始化表单必填；从库添加；导出 bundle 文件存在）。

完成条件：甲乙两个账号在同一 Workspace 各自从本机库引入 A/B 与 C/D，丙（未装包）看到四个控件并能插入实例。

## 9. M6：手册、DV 与收尾

- [x] 手册：`aiworkspace_template_dev.md` 加“控件开发”一节（目录骨架、`component.json` 字段、build / pull / watch、导出与 install、从库引入；只列目录，自述按需 `describe`）；`aiworkspace_run.md` 加 `bindings` 用法；`buckyos_app_dev.md` 加 `aiws` 包类型一段；`buckyos_dev_release.md` 核对 install 动词。
- [ ] 真实 Zone DV（全新安装，不写兼容；**10-30 部分完成**：(1)(2) 与 Installer 通过，(3) 跨 Zone、(4) Agent 未跑、(5) 无旧版本可测，见实施记录 §7）：(1) 用户 A 在 home 写 declarative 控件 → CLI build → Desktop 实例出现；改错 → build 失败实例不变；(2) 导出 `--install` → 在 Workspace 2 从库添加 → 实例化；(3) 用户 B 从自己 Zone 库向 A 的 Workspace 引入（跨 Zone 中转）；(4) Agent 只凭手册在 home 建控件并 build；(5) 旧 `manifest_version` 包可 install / import（Q19）。记录 DV 中 Agent 访问 nfs-server 的路径，若不可达按 bug 修。
- [x] 文档：aiworkspace README 协议表与 `ext.*`；Desktop README；设计文档 §8 P0 改“已实施”并写《扩展控件实施记录与验收报告》（沿用 `doc/workspace/*实施记录与验收报告.md` 格式）。

## 10. 不在本单

- composite 宿主、`composition.json`、slots / Chrome、“定义为控件”（P1）；定义 → 定义依赖边与 `deps` 非空（P1）。
- HTML `parent`、事件 / 动作绑定、脚本行为、设计 / 运行态（P2 / P3）。
- 来源包显式升级与结构迁移（P2）；`.block.zip` 交换（P3）。
- Workspace 模板包的创建与实例化（预留字段已在 M0 清单）。
- PIKG 签名与发布闭环（沿统一信任体系另行补）。

## 11. 验证命令

- core：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace`；WASM：`PATH=/tmp/dev-cache-root/cargo/bin:$PATH CARGO_TARGET_DIR=/tmp/dev-cache-root/cargo-target bash src/frame/aiworkspace/wasm/build.sh`。
- buckyos-api / control_panel：`cargo test -p buckyos-api -p control-panel`（crate 名实施时核对）。
- websdk：仓库 `buckyos-websdk` 的 `deno test`；CLI 用 `deno run -A cli/main.ts aiws build ...` 对本地服务跑一遍。
- Desktop：`pnpm check && pnpm exec eslint src/app/aiworkspace tests/aiworkspace && pnpm build`；e2e：`PATH=/tmp/dev-cache-root/cargo/bin:$PATH AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts`。
- 真实 Zone DV：`BUCKYOS_UI_DV_BASE_URL=... BUCKYOS_TEST_ADMIN_PASSWORD=... pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.extensions.spec.ts` + CLI / Agent 手工步骤按 §9。

## 12. 实施记录（2026-10-30）

落点、测试与 DV 见 [扩展控件实施记录与验收报告](<../doc/workspace/BuckyOS AI Workspace 扩展控件实施记录与验收报告.md>)。M0 契约写在 aiworkspace README “Extension controls” 与 `schemas/{component,aiws-manifest,block-def}.schema.json`。

### 12.1 与本单的出入（实现时按代码事实定，已写进 README）

1. **定义的源码快照键叫 `source_dir`，引用边叫 `def_source`**（强、live）：`source` 是 D12 的 Source 键，带它的实体整体只读；`source` 这个引用边种类也已被 D12 占用。
2. **CLI 动词**：`aiws import` / `aiws export` 已是脑图大纲交换动词，扩展用 `aiws ext-export`、`aiws ext-import`、`aiws library list|install|remove`；`build`、`pull` 照旧。多个定义 / 条目用逗号分隔（CLI 没有可变长位置参数）。
3. **源码目录只支持平铺**（`buckyos.dir` 成员名不能含 `/`），≤ 64 个文件，`.` 开头的文件是客户端自己的（`.aiws-build.json` 记基线）。
4. **home 文件不是 FileObject**：原生 home 导出上 NFSP 报的 `obj_id`（只有算过全量哈希的节点才有）是 `sha256:` 内容摘要，不是 named store 里的 `cyfile:` 对象，服务取不到。`ext.build` 的文件因此两种：`cyfile:` 对象照旧按引用（网关鉴权 + stage），其余由客户端用自己的会话经 NFSP 读出、以 `base64` 内联（每次 ≤ 4 MiB）。设计 §7.5 W1 “零拷贝”只对对象成立。
5. **`ext.install` 三种来源**：`pikg: { bucket_id, key }`（调用者自己的 `exports` 桶，装完即消费，CLI 用这个）、`pikg: { obj_id }`（Installer 传 staged 对象，或网关判定可读的对象）、`from: { workspace_id, entity_ids } + package`（Desktop “设为全局可用”：服务端直接把导出 bundle 记入库，内容与 `.pikg` 相同）。另加 `ext.uninstall`。
6. **许愿格定义的作者**：规划时拿不到 owner 的 DID，`ident.author_did` 记 `did:aiws:<workspace_id>`（工作区自己生成的控件）。
7. **build Commit 的 origin 是 `build`**，message `build <ident>`；`derived.build` 只接受 build / import / 撤销路径写入（人的 Commit 不能冒充构建记录）。撤销 build Commit 时定义、快照与记录一起回到上一版。
8. **Desktop 不在浏览器里打 PIKG**：导出对话框提供“设为对我所有工作区可用”（`ext.install from`）和“把 bundle 写到我的 home”（再显示一条 `buckyos aiws ext-export … --pikg` 命令）。在工作区自己的域名（`aiws-<id>.<zone>`）上打开时，home 目录不可达，新建 / 编辑 / 构建 / 写 bundle 关闭并说明，库引入与“全局可用”照常。
9. 新控件的作者 DID 默认 `did:bns:<用户名>`（Desktop 拿不到用户 DID，表单可改；CLI `ext-export` 的 `--owner` 同此默认）。
10. `AppType::Aiws` 的准入在 `admit_app` 里先于“完整版不限制”判断：任何版本都只接受用户提交（进库），预装 / Agent / 已部署一律 `product_aiws_origin`。
11. **CLI `build` / `pull` 也接受本地目录**（文件内联）：CLI 没有往 Zone home 写文件的动词，Agent 容器和开发机的源码在本地；Zone 路径照旧（`--zone-path` 强制按 Zone 路径解释，`--watch` 只跟随 Zone 目录）。
12. **DV 发现并修正**：Rust `derived_selector_for_key` 没有 `aiws`（TS 有），Installer 检查会报 “no package matches target”；内核 / frame 服务以设备名（`ood1`）为 subject 登录，aiworkspace 改为对服务调用者记 token 的 `appid`（control-panel 据此被认作 Installer）；NFSP 对不存在路径的 `stat` 会让浏览器记 404 控制台错误，`ensureFolder` / `readBuildState` 改用父目录的过滤列表；首次 `buckyos app install` 要先 `app fetch --plan`。
13. **主菜单入口在“插入 → 扩展控件”分组末尾**，不是顶层项：顶层再多一项，小窗口里最后的“关闭工作区”会落到窗口外（全量 e2e 的 i18n / offline 三例因此点不到）；分组在没有定义时也可展开，只显示“还没有 Block 定义”与管理入口。
