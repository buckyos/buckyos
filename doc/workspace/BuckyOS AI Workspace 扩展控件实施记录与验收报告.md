# AI Workspace 扩展控件（P0、P1）：实施记录与验收报告

> 日期：2026-10-30。对应设计：[扩展实现思路](<BuckyOS AI Workspace 扩展实现思路.md>) v0.8（D1–D8、A1–A7、Q18 / Q19），任务单 [notepads/aiworkspace-extension-p0-todo.md](<../../notepads/aiworkspace-extension-p0-todo.md>)（M0–M6）。全部改动未提交（buckyos、buckyos-websdk 两仓）。
>
> 环境：Rust 测试用真实 SQLite 与进程内 nfs-server；Desktop e2e 是独立 `aiworkspace`（调试构建）+ 独立 nfs-server（`home` 原生导出、无鉴权）+ Vite 开发服务器（模拟运行时、`/nfs/v1` 代理到 nfs-server）+ Chromium；真实 Zone DV 在本机 `test.buckyos.io`（§7，全新安装，两例通过）。
>
> 本文只记录**已经运行过的事实**，每项结论后附产生它的命令或测试；没有运行过的写在 §6。§1–§7 是 P0；P1（组合控件与 P0 遗留）见 §8。

## 1. 结论

- **跑通的路径**：在 home 目录写 render 控件（`component.json` + 入口文件）→ `ext.build` 同步校验、一次 Commit 原子提交定义 / 源码快照 / `derived.build` → 实例（带命名绑定与属性覆盖）即时更新 → 改源码再 build 实例跟着变；源码坏了 build 被拒、实例保持上一版 → 多选导出为“对我所有工作区可用”（本机库）或 bundle / `.pikg` → 另一个工作区从本机库引入并实例化。两条实现路线中交付 render-block（declarative / html），契约为组合控件留位（`kind` 闭集未动，`deps` 必须为空）。
- **内核（M1）**：block-def 新键 `ident` / `manifest_version` / `source_dir` / `bindings` / `catalog` / `agent` / `provenance`；`source_dir` 是强引用边 `def_source`（快照目录在定义使用期间不能删）；`derived.build` 只能由 build / import / 撤销写；outline 投影 `ident`、`manifest_version`、`def_version`、`provenance_pkg`、`source_dir_id`；许愿格 HTML 定义补身份（作者 `did:aiws:<workspace_id>`）；`schemas/` 追到 PROTOCOL 1.1（selector 枚举 14 种与代码一致、去掉不存在的 `references.json`）。WASM 已重建。
- **服务（M2）**：`ext.build / export / install / list_library / uninstall / library_bundle / import`（均 experimental）；本机库在服务数据目录 `library.sqlite` + 每用户 `library` 桶；`aiws` PIKG 读取与校验（结构、AppDoc `app_type: aiws`、payload 大小与 SHA-256、每控件内容摘要、`component.json`）；错误码 `BUILD_FAILED`、`LIBRARY_PACKAGE_INVALID`。
- **websdk / CLI（M3）**：共享模块 `src/aiws_ext.ts`（Desktop 与 CLI 共用：收集源码、build / pull / watch、bundle 导出与校验、内容摘要、骨架）；`AiWorkspaceClient` 补 `getChanges / waitChanges / exportDoc / assetImportObject / controls* / ext*`；CLI `aiws build|pull|ext-export|library|ext-import`；PIKG 新 kind `aiws-controls`（`pikg init` 写骨架，`build / pack / info` 用与服务相同的规则校验 bundle）。
- **PIKG / Installer（M4）**：Rust `AppType::Aiws`（`pkg_list.aiws` 唯一、无其它子包）、product 准入（任何版本只接受用户提交进库）、`app_submit_route` 新路由 `RegisterAiwsPackage` → control-panel 校验 staged PIKG 后以 `mode: installer` 调 `ext.install`；不建 AppSpec、不起服务。
- **Desktop（M5）**：“工作区控件”对话框（主菜单“插入 → 扩展控件 → 工作区控件…”、插入目录扩展页签、数据源详情三个入口）：列表、详情（身份、版本、来源、最近构建、命名输入、实例）、编辑控件源码（无工作副本先 pull，再用“文件”打开）、构建、拉取、保存即构建（watch）、移除（被引用时禁用）、新建控件（home 骨架 + 立即构建）、从本机库添加、导出（全局可用 / bundle 写入 home）；插入目录对控件按 `config_schema`（A7 子集）与 `bindings` 生成初始化表单（只存覆盖，必填与 schema 校验）；declarative 解释器认 `<binding>.<expr>`（Q18）；手机只读。
- **手册（M6）**：`aiworkspace_template_dev.md` 新增 “Controls: develop, build, export, reuse”，`aiworkspace_run.md` 补 HTML 结果 / 控件的 `bindings`，`buckyos_app_dev.md` 加 `aiws-controls` kind，`buckyos_dev_release.md` 加 `aiws` 包的安装结果；PIKG 文档（buckyos `doc/pikg/{format,README}.md`、websdk `doc/modules/pikg.md`）加 `aiws` 类型。

## 2. 交付物

| 交付物 | 位置 |
| --- | --- |
| 契约（M0） | aiworkspace README “Extension controls”（身份、`component.json`、定义新键、方法表、build / 基线 / 诊断 / 兼容 / 摘要 / pull / import、AIWS 清单与 `aiws` 包、CLI）；`schemas/{component,aiws-manifest,block-def}.schema.json`；`schemas/commit-request.schema.json` 1.1；`schemas/error-codes.json` |
| 内核 | `core/src/types.rs`（`validate_block_def`、`BLOCK_DEF_KEYS`、`ident_string`、`check_def_bindings`、`check_catalog`、`check_build_record`、`def_source` 边、`is_icon_id`）、`model.rs`（`REF_KINDS` 加 `def_source`）、`plan.rs`（`SELECTOR_KINDS`）、`read.rs`（outline 投影）、`error.rs`（两个码、`Code::parse`）；`store/src/workspace.rs`（`CommitOpts.as_build` → origin `build`）；`store/src/wish/plan.rs`（许愿格定义身份） |
| 内核测试 | `core/tests/extension.rs`（4 个）、`core/tests/schemas.rs`（加 2 个：commit-request 与协议、扩展 schema 与校验器一致） |
| 服务 | `store/src/ext.rs`（组合与校验、兼容检查、build / import 规划与提交、导出、合成源码）、`store/src/library.rs`（库、`aiws` PIKG 读取、bundle 校验）、`server/src/ext.rs`（kRPC、网关鉴权、库归属、安装三种来源）、`server/src/lib.rs`（方法表、分派、`AppState.library`）；`buckyos-api` `aiworkspace_bucket_decls` 加 `library` 桶 |
| 服务测试 | `server/tests/ext.rs`（3 个，覆盖 TODO M2 列出的用例）；`store` 单测 3 个（摘要向量、schema 子集、收窄判断） |
| PIKG / Installer | `buckyos-api/src/app_doc.rs`（`AppType::Aiws`、`AIWS_SUB_PKG`、builder 规则、`aiws_pkg`）、`product.rs`（`AiwsControls`、`AiwsLibrary`、`AiwsOrigin`）；`control_panel/src/aiws_pkg.rs`、`app_installer.rs`（路由与单测）、`pikg.rs`（reader 单测） |
| websdk / CLI | `src/aiws_ext.ts`、`src/aiworkspace_client.ts`、`src/{index,node}.ts`、`package.json` / `vite.config.ts`（子路径 `buckyos/aiws_ext`）；`cli/modules/{aiws,pikg,pikg_protocol,share}.ts`；测试 `tests/aiws_ext.test.ts`（6）、`cli/tests/{aiws_test,pikg_test}.ts`（+4） |
| Desktop | `ui/extensions/{ExtensionsDialog,InstanceForm}.tsx`、`ui/extensions/{controlSources,controls,extEvents}.ts`、`ui/extensions/declarative.tsx`（命名绑定）、`ui/canvas/{InsertCatalog.tsx,catalog.ts,CanvasView.tsx}`、`ui/sources/DataDetail.tsx`、`ui/shell/{MainMenu.tsx,WorkspaceShell.tsx,shellContext.ts}`、`api/{client,session,types}.ts`、`offline/replicaSession.ts`、`state/outline.ts`；文案 `i18n/aiworkspace/ext.ts`（en + zh）及 `canvas/data/shell.ts` |
| Desktop 测试 | `tests/aiworkspace/extensions.spec.ts`（e2e）、`tests/dv/aiworkspace.extensions.spec.ts`（真实 Zone）、`tests/aiworkspace/start-backend.mjs`（`library` 桶）、`tests/datamodel/aiworkspace-i18n.test.ts`（新区域） |
| 手册与文档 | `src/apps/jarvis_agent/agent/cmd_skills/{aiworkspace_template_dev,aiworkspace_run,buckyos_app_dev,buckyos_dev_release}.md`；`doc/pikg/{format,README}.md`；websdk `doc/modules/pikg.md`；设计文档 §8 状态 |

## 3. 实现中的调整

见任务单 §12.1（10 条），要点：定义键 `source_dir` / 边 `def_source`（`source` 是 D12 Source）；CLI 用 `ext-export` / `ext-import` / `library`（`import` / `export` 已是脑图大纲动词）；源码目录平铺；**原生 home 文件没有 named-store 对象**（NFSP 的 `obj_id` 是 `sha256:` 摘要），build 时由客户端经 NFSP 读出内联提交（≤ 4 MiB），只有 `cyfile:` 对象按引用；`ext.install` 支持自己 `exports` 桶里的包（CLI）、Installer 的 staged 对象和“从工作区直接入库”（Desktop）；许愿格定义作者 `did:aiws:<workspace_id>`；build Commit 的 origin 为 `build`；Desktop 不在浏览器里打 PIKG；工作区独立域名上关闭需要 home 的操作。

## 4. 测试（已运行）

| 范围 | 命令 | 结果 |
| --- | --- | --- |
| 内核 / 存储 / 服务全量 | `cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace` | 45 个测试二进制全部通过（含许愿格真实 xllm 循环与 Deno、契约录制） |
| 服务扩展用例 | `cargo test -p aiworkspace --test ext` | 3 通过：build / 基线冲突 / 收窄列出实例 / dry_run / 内联 html / 快照删除保护；export 摘要 / 合成源码 / 入库与 satisfied / 同版本异内容拒绝 / import 两次 / 跨 Zone 中转（bundle → 内联 build + provenance）/ 卸载；PIKG 安装 / 篡改 payload / 文件与摘要不符 / 桶绑定安装并消费 / 他人桶拒绝 / installer 模式拒绝 |
| buckyos-api | `cargo test -p buckyos-api` | 全部通过（含 `aiws_packages_only_enter_the_library_from_a_user`） |
| control-panel | `cargo test -p control_panel` | 209 通过、1 失败（之后加的 `an_aiws_bundle_matches_every_target` 单独通过）：`sso_identity::tests::account_identity_carries_ceiling_method_and_password_state`（期望的 scopes 不含 `Aicc`，与本单无关的既有失败）；`pikg::tests::aiws_package_reads_and_tampering_fails`、路由单测通过 |
| WASM | `bash src/frame/aiworkspace/wasm/build.sh` | 重建成功 |
| websdk | `npx jest tests/aiws_ext.test.ts`；`cd cli && deno test --allow-read --allow-write --allow-env tests/` | 6 通过；CLI 128 通过 |
| Desktop 静态检查 | `npx tsc --noEmit -p tsconfig.app.json`；`pnpm exec eslint src/app/aiworkspace src/i18n/aiworkspace tests/aiworkspace/extensions.spec.ts tests/dv/aiworkspace.extensions.spec.ts` | 无错误 |
| Desktop i18n | `deno test --allow-read --allow-env tests/datamodel/aiworkspace-i18n.test.ts` | 15 通过、2 失败均为既有：未使用键 `canvas.catalog.sample / extension`，`store/src/wish/outline.rs:383` 的“字” |
| Desktop e2e（扩展） | `AIWS_BIN=… NFS_BIN=… pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/extensions.spec.ts` | 1 通过（7.6 s） |
| Desktop e2e（全量） | 同上不带文件 | 见 §4.1 |

### 4.1 e2e 全量

第一次全量（209 例，31.7 min）：205 通过、4 失败。

| 用例 | 结论 |
| --- | --- |
| `connectors.spec.ts:288` CN17 锚点 | 既有失败，与本单无关（单独重跑仍失败，失败点是锚点计数） |
| `i18n.spec.ts:133` 影片示例语言切换 | 本单引起：主菜单多了顶层项“工作区控件…”，小窗口里最后的“关闭工作区”落到窗口外点不到 |
| `offline.spec.ts:6` 快照离线冷启动 | 同上 |
| `offline.spec.ts:217` 离线撤销 / Mock / 写锁 | 同上 |

修正：入口挪到“插入 → 扩展控件”分组末尾（TODO §12.1 第 13 条），顶层项数不变。重跑 `extensions / i18n / offline / connectors:288` 共 13 例：11 通过；`offline` 两例与 `extensions` 通过；剩两例均为既有：
CN17 同上；`i18n.spec.ts:133` 现在越过关闭工作区，停在最后一步“刷新后保留 localStorage 里的 zh-CN”——Desktop 10-17 起外观设置跟随账号（`src/desktop/appearance/AccountAppearanceSync.tsx`），用例前面切回英文已写进账号，刷新后账号值覆盖 localStorage，用例的这一步与“设置跟着账号走”冲突，与本单无关。

## 5. 验收对照

| TODO 完成条件 | 结论 | 依据 |
| --- | --- | --- |
| M0：README + schemas 一次定稿 | 已写；名字在 M1–M5 中只按 §3 调整过一次 | README、`schemas/*`，`core/tests/schemas.rs` |
| M1：三包测试全绿，旧 fixture 可导入 | 通过 | §4 第 1 行 |
| M2：最小 declarative 与 html 控件 build 后能被 Desktop `resolve` 解析 | 通过：e2e 中 build 出的定义渲染为实例 | `extensions.spec.ts` |
| M3：只用 CLI 完成 build → export → pack → install(dev) → 另一工作区 import | 通过（真实 Zone） | §7 第二例 |
| M4：`buckyos app install <aiws.pikg>` 不产生 AppSpec、不起服务，`ext.list_library` 可见 | 通过（真实 Zone） | §7 第二例 |
| M5：甲乙各自从本机库引入、丙看到并能插入 | 部分：两个真实账号各自从自己的库引入同一工作区，未装包的一方插入另一方控件的实例；第三个账号未单独建 | §7 第一例 |

## 6. 未完成与已知限制

- Desktop 不在浏览器里生成 `.pikg`：导出写出 bundle 后给出 CLI 命令（§3）。
- 依赖者兼容检查只覆盖“同身份已有定义”的收窄（`kind`、`accepts`、`bindings`、`config_schema.required`）；定义 → 定义依赖（`deps`）是 P1。
- 引入只支持新定义与同来源精确复用，同身份异来源报 `EXT_IDENT_EXISTS`（P2 再做升级与迁移）。
- watch 依赖 NFSP watch（有损）：断线后 2 s 重连，`resync` 重列目录；在工作区独立域名上不可用。
- 新控件作者 DID 默认 `did:bns:<用户名>`，Desktop 无法取得用户真实 DID 时需手改。

## 7. 真实 Zone DV（2026-10-30，本机 `test.buckyos.io`）

部署：`uv run buckyos-build.py -s aiworkspace control_panel scheduler desktop jarvis_agent`，`uv run start.py --all`（全新安装：scheduler 重新种入带 `library` 桶的 aiworkspace 服务 spec；保留 `data/`），之后两次增量 `-s control_panel aiworkspace jarvis_agent`、`-s desktop aiworkspace` + `uv run start.py`。用例 `tests/dv/aiworkspace.extensions.spec.ts`：

```bash
cd src/frame/desktop
BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=bucky2025 \
  pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.extensions.spec.ts
# 2 passed (26.4s)
```

主菜单入口移到“插入 → 扩展控件”（§4.1）后重新 `-s desktop` + `uv run start.py` 部署，同一用例再跑：2 passed (26.6s)。

| 用例 | 内容 | 结果 |
| --- | --- | --- |
| Desktop + 两个账号 | devtest 在桌面的 AI Workspace 窗口里新建控件：骨架经 Zone 文件服务写进 `/home/devtest/AI Workspace/<工作区>/controls/<name>`，`ext.build` 提交，身份 `did:bns:devtest#<name>`；插入实例并把命名绑定 `data` 绑到 `tasks` 表，指标显示行数；再次构建、`derived.build.builder_version` 正确；“设为对我所有工作区可用”入库。临时账号用内联文件在自己的工作区构建 HTML 控件并入库。两人各自从自己的库把控件引入同一个共享工作区，双方 outline 都看到两个身份；临时账号没装 devtest 的包，仍在共享工作区插入其实例（Commit accepted）。浏览器控制台无错误。 | 通过 |
| 只用 CLI + Installer | `pikg init --kind aiws-controls` 生成骨架 → `aiws build <本地目录>` → 改文件再 build 为第 2 版 → 坏的 `component.json` 被拒（`BUILD_FAILED`）→ `aiws ext-export --install`（pack + 绑进自己的 exports 桶 + `ext.install`）→ `aiws library list` → `aiws ext-import` 进第二个工作区（provenance 为该包）→ 从库删除后 `app fetch --plan` + `app install --plan` 走 control-panel Installer：结果含 `pkg_id`，库里重新出现，`app list` 中没有这个包（未建 AppSpec）。 | 通过（M3、M4 完成条件） |

DV 中发现并已修正的问题（都已重新部署后通过）：

1. control-panel 检查 `aiws` 包时 Rust `derived_selector_for_key` 不认识 `aiws` 子包（“no package matches target”）→ 加入平台无关键，单测 `an_aiws_bundle_matches_every_target`。
2. Installer 调 `ext.install` 被拒：内核 / frame 服务以设备名为 subject 登录，aiworkspace 看到的是 `ood1` → 服务调用者改记 token 的 `appid`，按 `control-panel` 认 Installer。
3. 新建控件时浏览器控制台出现 5 条 404（NFSP 对尚不存在的目录、`.aiws-build.json` 做 `stat`）→ `ensureFolder` / `readBuildState` / `hasControlSource` 改为在父目录过滤列表，独立 nfs-server 上复测无失败请求。
4. CLI 的本地命令（`pikg`）拒绝显式 `--endpoint`：DV 用环境变量 `BUCKYOS_TOOL_ENDPOINT`；首次安装要先 `app fetch --plan`（与 `buckyos_dev_release.md` 一致）。

未在真实 Zone 上运行（TODO §9 DV 的其余项）：

- (3) 跨 Zone：用户 B 从自己 Zone 的库向 A 的工作区中转引入。服务侧路径（`ext.library_bundle` → 目标工作区 `ext.build` 内联文件 + `provenance`）只在 `server/tests/ext.rs` 单 Zone 内验证过；三 Zone VM 组未跑。
- (4) Agent 只凭手册在 home 建控件并 build：全新安装后没有 Agent，未跑；手册已写入本地目录 / Zone 目录两种 build 方式。DV 中 Agent 访问 nfs-server 的路径因此也未记录。
- (5) 旧 `manifest_version` 包：目前只有版本 1，读取路径（`compatManifest`、Rust `MANIFEST_VERSION` 上限）有单测，没有旧版本包可装。

## 8. P1：组合控件与 P0 遗留（2026-10-30）

> 对应任务单 [notepads/aiworkspace-extension-p1-todo.md](<../../notepads/aiworkspace-extension-p1-todo.md>)（M0–M6）与设计 v0.9 §8 P1 行、§7.2 Q15–Q17、§7.6 Q20–Q25。P0 已由用户提交（`63765d12d aiws support dev extensiion`），L6 不再单独做；P1 改动在 buckyos、buckyos-websdk 两仓未提交。本节同样只记已运行的事实。

### 8.1 结论

- **契约（M0）**：`component.json` `manifest_version: 2`（`kind: composite`、`entry: composition.json`、非空 `deps`；`events` 拒绝；文件名可为相对路径）；新 schema `schemas/composition.schema.json`（`layout` / `slots` / `chrome` 三件事）；block-def 新键 `deps`、`composition`；`aiws-manifest` 控件项加 `deps`；`operations.json` 加 `tree.clone`、`tree.move` 的 `slot`；错误码 `REFERENCE_CYCLE`；`controls/core.json` 的 `control_kinds` 加 `composite`。README “Extension controls / Composite controls / Validator classes” 已按实现写定。`hello-controls/controls/hello-board/` 去掉 `_comment` 与 `events` 成为冻结形态，`aiws-manifest.json` 升 2 并加 `hello-board`；三个控件经 websdk `checkComponent` / `checkComposition` 预检无诊断，摘要用 `aiwsContentDigest` 重算（`hello-card` 因 `behavior.js` 改用 `aiws.asset` 而变）。
- **内核（M2）**：`types.rs` 拆为 `validate_primitive`（原语 + 引用：容器 kind / layout / 系统节点 / `content_folder_id`、composite 的 `def_ref` / `bindings`、`slot`、AssetRef、目录、表元数据、block-def 的 `source_dir` 与 `deps`）与 `validate_schema`（其余业务校验，仅 `PlanEnv.schema_checks`）；`ws.import` 与副本重放不进后者，线程内计数器 `types::schema_validations()` 作哨兵。`kind: composite` 定义（`composition` 只查形状）、composite 容器（`def_ref` 指向 composite 定义且不可改、`config` / `bindings`、`layout { mode: def }`）、子节点必带定义里的 slot（同一 slot 可多个）、`fixed` 布局下子节点不得 `"auto"`、composite 内 Cell 不再挂 Block（`in_tree_control`）；`deps` 只能由 build / import / 内部 / 重放写，强边 `def_dep`，依赖环 `REFERENCE_CYCLE`；`tree.clone`（`plan.rs`，id = `c` + `sha256("<id_seed>\n<旧 id>")` 前 26 位）与 `tree.move { slot }`；目录成员名允许 `/`（Q23）；outline 投影 composite 定义的 `composition` / `config_defaults`、`deps`，容器的 `def_id` / `slot`；Commit 结果加 `cloned: [{ op_index, new_ids }]`；许愿格的 control 结果不接受 `composite`。
- **服务（M3）**：`store/src/ext.rs` `compose` 分 kind（composite 读 `composition.json`，`check_composition` 校验三件事与 `{{config.<key>}}`），`deps` 解析成同目录定义（缺失诊断 `code: DEP_MISSING`），slot 的 `cell:<ident>` 须已知且不是 composite；同一次 build / import 内按依赖拓扑排序；重建时收窄（含 composite 删 slot / 收窄 accepts）同时列实例与依赖者（`dependents`，诊断 `file` 为依赖者身份）；`ext.export` package 沿 `def_dep` 带依赖、去重，清单加 `deps`，`purpose: pull` 不带；`library.rs` `verify_bundle` 校验 `deps` 与 `component.json` 一致且闭包完整（否则 `LIBRARY_PACKAGE_INVALID`），库记录在 `info_json` 带 `deps`；`ext.import` 把依赖一起引入（依赖在前；工作区已有同身份定义时直接用它）。新 `store/src/pack.rs` + `server/src/ext.rs` `pack`：服务端 `ext.pack` 写 `aiws` PIKG 进调用者 `exports` 桶。
- **websdk / CLI（M4）**：`aiws_ext.ts` `AIWS_MANIFEST_VERSION = 2`、`checkComponent` v2（composite 分支、`deps`、`events`、相对路径文件名）、`checkComposition`、`slotCells`、`verifyBundle` 依赖闭包、递归 `collectSources`、`writeSourceFile`（pull 写子目录）、`packBuildFiles` / `buildFiles(…, upload)`（超 4 MiB 按大小上传成对象）、`sourcesDigest` / `unbuiltChanges`（L3）、`controlSkeleton({ kind: 'composite' })`；`aiworkspace_client.ts` 加 `extPack`、`extInstall` 收桶绑定；CLI 本地 build 递归收集并超限上传到自己的 `exports` 桶，`pull --local` 写子目录，`pikg init --kind aiws-controls --composite`，`library` / `ext-import` 说明带 `deps`。
- **Desktop（M5 与 M1 的 Desktop 项）**：通用 composite 宿主 `ui/controls/composite/`（一个 `ContainerDefinition` 服务所有组合控件，按 `def_ref` 读定义；布局用共享模块 `controls/composite.ts` `solveComposite`：`stack` 列 / 行、间距、内边距、slot 按 `top`→`stack`→`bottom`，`fixed` 保留 x / y；`Chrome.tsx` 画背景 / 圆角 / 边框 / 标题带 / 空态，`{{config.<key>}}` 取实例覆盖再取定义默认；`Inspector.tsx` 用定义的表单改属性与绑定）；插入目录把 composite 定义列为扩展条目（建容器 + 按 `min` 建初始 Block），裸宿主不出现；拖放进 composite（落在 Block 旁取其 slot，落在空白处追加，`accepts` / `max` 不收则提示），`tree.move` 带 `slot`；剪贴板：composite 实例经 `tree.clone` 粘贴（WASM 新增 `clone_id` 预知新 id），composite 的子 Block 粘到画布不再包成脑图，粘进 composite 取 slot；“定义为控件…”（`ui/extensions/DefineControlDialog.tsx`：每个选中 Block 一个 slot，`fixed` / `stack`，在 home 写三文件、build，可选一次 Commit 用实例替换选区）；流式页把 composite 当 group 显示；工作区控件对话框去掉 `onWorkspaceHost()` 门（Q24）、加“导入 .pikg 文件…”（Q21）与导出第三出口“生成 .pikg”→下载 / 直接入库（Q20）、未 build 改动提示（L3）、依赖者诊断、composite 新建；超限文件经 `uploadAsset` 走引用（Q22）；HTML 宿主 `aiws.asset(name)`（只给 `assets` 里列出的快照文件，iframe 内建 blob URL、卸载时 revoke）；关系面板认 `def_dep`；i18n en / zh。顺带修两处画布问题：outline 增量补丁重建定义的 extras 时 `kind` 取 `def_kind`（否则改 `composition` 后布局不更新），composite 的子 Block 按下时选中自己（脑图的“按根即按整图”不适用）。
- **手册与文档（M6）**：`aiworkspace_template_dev.md` “Controls” 加 composite、`deps`、`aiws.asset`、子目录、`pikg init --composite`、`ext.pack`；`aiworkspace_run.md` 加 composite 实例的 `doc.commit` 形态与 `tree.clone`；hello-controls README 第 6–9 步与总表；设计文档 §1.5 / §7.2 / §8 / §9 状态。

### 8.2 交付物

| 交付物 | 位置 |
| --- | --- |
| 契约 | README “Extension controls”“Composite controls”“HTML host API additions”“Validator classes”“Block-controls”；`schemas/{component,composition,block-def,aiws-manifest,operations,commit-request,error-codes}` |
| 内核 | `core/src/types.rs`（`validate_primitive` / `validate_schema`、`container_primitive` / `container_schema` / `composite_refs`、`check_slot`、`in_tree_control`、`composite_def` / `composite_algorithm`、`block_def_primitive` / `block_def_schema`、`check_composition_shape`、`deps_reach`、`check_member_name`、`entity_refs` 的 `def_dep` 与容器 `def` / `bind`）、`core/src/plan.rs`（`tree_clone`、`clone_id`、`tree_move` 的 `slot`、`PlaceRules` 的 fixed、`child_allowed_here`）、`model.rs`（`def_dep`）、`error.rs`（`REFERENCE_CYCLE`）、`read.rs`（outline 投影）、`wish.rs`；`controls/{core.json,contract.ts}`；`store/src/workspace.rs`（`ws.import` 关 schema、Commit 结果 `cloned`） |
| 内核测试 | `core/tests/composite.rs`（5）、`core/tests/clone.rs`（3）、`core/tests/apply_log.rs`（+1：业务违规但原语合法的日志能重放）、`core/tests/phase2.rs`（成员名用例改为相对路径规则） |
| 服务 | `store/src/ext.rs`（`check_composition`、`dependency_order`、`slot_cells_of`、`ext_dependents`、`narrowing` 扩到 slots、`ext_bundle_files`、合成 composite 源码）、`store/src/library.rs`、`store/src/pack.rs`（新）、`server/src/ext.rs`（`pack`、import 依赖闭包）、`server/src/lib.rs`（方法表 `ext.pack`） |
| 服务测试 | `server/tests/ext.rs` `composite_controls_with_dependencies`；`store` 单测 `pack::tests`（2） |
| websdk / CLI | `src/aiws_ext.ts`、`src/aiworkspace_client.ts`；`cli/modules/{aiws,pikg}.ts`；测试 `tests/aiws_ext.test.ts`（9）、`cli/tests/pikg_test.ts`（+1 composite 包） |
| 例子 | `doc/workspace/hello-controls/{aiws-manifest.json,README.md,controls/hello-board/*,controls/hello-card/behavior.js}` |
| Desktop | `src/frame/desktop/src/app/aiworkspace/`：`ui/controls/composite/{definition.ts,Chrome.tsx,Inspector.tsx,model.ts}`、`ui/controls/registerControls.ts`、`ui/canvas/{catalog.ts,InsertCatalog.tsx,CanvasView.tsx,clipboard.ts,FlowSurface.tsx,render/RenderHost.tsx}`、`ui/extensions/{ExtensionsDialog.tsx,DefineControlDialog.tsx,controlSources.ts,bridge.ts,htmlRuntime.ts}`、`ui/shell/{dialogs.tsx,download.ts}`、`ui/sources/RelationsPanel.tsx`、`state/outline.ts`、`api/{client,session,types}.ts`、`offline/replicaSession.ts`、`aiworkspace.css`、`wasm/*`（重建）；`src/i18n/aiworkspace/{ext,wish,data}.ts`；共享 `controls/{composite.ts,composite_test.ts,solve.ts,catalog.ts,mod.ts}` 与 `aiws/controls.json`（重生成，目录加 `composite` 宿主自述）；Desktop README “Extension controls / Composite controls” |

### 8.3 与任务单的出入

1. **slot 是“桶”，不是同父唯一**：任务单 §1 Q15 写内核校验“slot 同父唯一”，而 `hello-board` 的 `cards` 要放多张卡。实现：composite 子节点的 `slot` 必填且须为定义的 slot（引用类，写入时判定），同一 slot 可多个；`max` / `accepts` 由客户端执行，内核不计数；同父唯一（`SLOT_CONFLICT`）只对非 composite 的控件（脑图）保留。
2. **`tree.clone` 参数**：用 `order_key`（缺省排在父节点最后）而非 `index`；新增 `id_seed`，新 id 由 `id_seed` 与旧 id 哈希得出，保证服务、副本与离线客户端规划出同一组 id；规范化 op 记 `id_seed` + `new_ids`，Commit 结果给 `cloned`。
3. **`tree.move` 带 `slot`**：Block 移入 composite 必须有 slot、移出控件不能留 slot，单靠 `entity.set_keys` 无法在一步内满足，故 `tree.move` 加可选 `slot`（名字或 `null`），撤销一并还原。
4. **`ext.pack` 与 CLI 不逐字节相同**：tar 部分按 `pikg_archive.ts` 规则写，相同；gzip 层服务用 flate2（level 6），CLI 用 Node zlib（`Z_RLE`），压缩字节不同，于是 payload 摘要、包对象与 AppDoc 对象 id、`.pikg` 文件都不同。同一份 bundle 在服务端打两次字节相同；安装校验结果一致。AppDoc `show_name` 取 `package.title`（CLI 取包名），`create_time` 取这批控件最新的 build 时间（无则当前时间）。任务单 M6 DV(3)“Desktop 生成 .pikg 与 CLI pack 的 `aiws.tar.gz` digest 一致”按原文不可达，改为比对未压缩 tar（`ext.pack` 返回 `tar_digest`）。
5. **composite 布局只读 outline**：为了让 Desktop 同步求解，内核在 outline 里投影 composite 定义的 `composition` 与 `config_defaults`（`config_schema` 各属性的 `default`），与脑图“布局只读 outline”同一原则。
6. **`ws.import` 关闭业务 schema**：Q15 要求重放路径只走原语 / 引用，`ws.import` 现以 `schema_checks: false` 提交；schema 类校验器内部的引用检查（Cell 的 `source_ref` / `bindings` / `def_ref` 等）仍随其校验器只在准入时执行，没有搬进原语入口。
7. **`deps` 守卫**：与 `derived.build` 同思路，`deps` 的变化只接受 origin `build`、import、内部（撤销）与重放的 Commit，人的 Commit 改它是 `INVALID_OPERATION`。
8. **引入时的隐式依赖**：`ext.import` 只点名组合控件也会带上依赖；依赖若在工作区已有同身份定义（任何来源，包括本地 build），直接满足并让依赖者指向它，而不是报 `EXT_IDENT_EXISTS`（被点名的条目仍按 P0 规则）。
9. **composite 里的 Cell 不挂子 Block**：只有树形控件（脑图）的 Cell 可以有子 Block（`types::in_tree_control`）。
10. **库表不改 DDL**：控件的 `deps` 存在 `library_controls.info_json`，`ext.list_library` 每个控件带 `deps`。
11. **CLI 超限上传落点**：上传到调用者自己的 `user/aiworkspace/<user>/exports` 桶（key `aiws-src/<时间>-<名字>`），取响应里的 `obj_id`；服务经 NDM 网关以调用者 token 鉴权后读取。
12. **`pikg sign` 不存在**：设计与 hello-controls 原文写“签名仍是 CLI `pikg sign`”，CLI 现有动词只有 `doctor / init / build / pack / info / clean`；文档改为“服务端没有作者私钥，签名属于作者自己的 PIKG 流程”。
13. **L6**：P0 两仓改动已由用户提交（buckyos `63765d12d`），不再作为本单步骤。

### 8.4 测试（已运行）

| 范围 | 命令 | 结果 |
| --- | --- | --- |
| 内核 | `cargo test -p aiworkspace-core` | 全部通过；其中 `--test composite` 5、`--test clone` 3、`--test apply_log` 2（新增 1）、`--test schemas`（`operations.json`、block-def / component / aiws-manifest schema 与代码一致） |
| 内核改动后的存储 / 服务全量 | `cargo test -p aiworkspace-store -p aiworkspace` | 内核与 schema 改动完成后跑过一次：全部测试二进制通过（M3 改动前） |
| 服务扩展用例 | `cargo test -p aiworkspace --test ext` | 4 通过；新增 `composite_controls_with_dependencies`：缺依赖 `DEP_MISSING`、composition 的 9 类错误、composite 定义写 `deps` / `composition`、实例与 slotted 子节点、收窄 card 列出依赖者与实例、删 slot 列出实例、改布局重建成功且子节点不动、被依赖定义删除被拒、导出闭包去重 / pull 不带、入库 `deps`、新工作区 import 依赖在前、已有本地 card 时 satisfied、`ext.pack` 两次摘要相同、`ext.install { pikg: { bucket_id, key } }` 安装两控件、开放闭包的包被拒 |
| 打包单测 | `cargo test -p aiworkspace-store --lib pack` | 2 通过（tar 目录顺序 / 模式 / mtime、确定性；AppDoc DID 规则） |
| websdk | `npx jest tests/aiws_ext.test.ts` | 9 通过（composite 预检、依赖闭包、超限分流、递归收集） |
| CLI | `cd cli && deno task test` | 129 通过（含 `pikg init --composite` → build → pack → info） |
| 例子预检 | 用 `aiws_ext.ts` 的 `checkComponent`（带 composition 读取）与 `aiwsContentDigest` 扫 `hello-controls/controls/*` | 三个控件无诊断；`hello-kpi` 摘要不变，`hello-card` / `hello-board` 摘要写回清单 |
| 存储 / 服务全量（M3 之后） | `cargo test --no-fail-fast -p aiworkspace-core -p aiworkspace-store -p aiworkspace` | 48 个测试二进制全部通过（README 方法级别表补 `ext.pack` 后 `contract.rs` 通过） |
| WASM / 控件目录 | `bash src/frame/aiworkspace/wasm/build.sh`；`deno run -A tools/build_catalog.ts`；`cd controls && deno test -A` | WASM 重建（新增 `clone_id`）；目录重生成；共享模块 24 通过（新增 `composite_test.ts` 4：stack 列 / 行、标题带与 slot 顺序、fixed 与嵌套、accepts 规则） |
| Desktop 静态检查 | `pnpm check`；`pnpm exec eslint src/app/aiworkspace tests/aiworkspace tests/dv/aiworkspace.extensions.spec.ts`；`deno test --allow-read --allow-env tests/datamodel/aiworkspace-i18n.test.ts` | 通过；i18n 测试 16 / 18，剩下两项是 P0 时已记录的既有失败（`canvas.catalog.sample/extension` 未用键、`store/src/wish/outline.rs` 的“字”） |
| Desktop e2e | `AIWS_BIN=… pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/extensions.spec.ts` | 4 通过：P0 原例；composite（目录插入、标题与空态、两张卡 stack 间距 8、改 `composition.json` 为 row 后重排且内容不变、复制卡片到画布不成脑图、复制实例经 `tree.clone` 带两张卡、生成 .pikg → 直接入库两控件）；定义为控件（两形状 → composite 实例、原 Block 删除、fixed 保位）；Q24（在 `aiws-<id>.localhost` 上新建并 build 控件）。`--repeat-each=2` 下新例都通过（P0 原例因 home 里同名控件已存在而在第二遍失败，属重复运行造成） |
| 回归 e2e | 同上，`mindmap / canvas / objects / flow / blocks / extensions` 六个 spec | 54 / 56；失败两例：`blocks.spec.ts:138` 在并发构建时 Desktop 首屏超时（单跑通过），composite 例在并发下复制与粘贴竞速（测试改为等“已复制”提示后通过） |

### 8.5 验收对照（设计 §8 P1 行）

| 验收项 | 结论 | 依据 |
| --- | --- | --- |
| 两个卡片实例分别绑定数据，改 `composition.json` 并 build 后布局 / 外观更新，私有内容不串改或重建；复制子节点不变成脑图 | 通过：服务侧改布局重建、子节点 slot 与内容不变；内核 `tree.clone` 复制 composite 与子节点；画布上 stack 列 → 行重排、卡片内容不变、复制卡片到画布不包成脑图 | `server/tests/ext.rs`、`core/tests/clone.rs`、`tests/aiworkspace/extensions.spec.ts` |
| 混选 render 与组合定义导出，共享依赖去重，在新 Workspace 可分别实例化 | 服务侧通过（导出两控件、import 依赖在前） | `composite_controls_with_dependencies` |
| 被依赖控件收窄接口时 build 列出依赖定义并失败 | 通过 | 同上 |
| 重放不执行业务 schema | 通过（哨兵计数不变） | `core/tests/composite.rs` `replay_runs_no_business_schema` |
| P0 遗留 L1–L5、Q20–Q24 的 DV / e2e 证据 | 部分：Q20 e2e（生成 .pikg → 直接入库）+ DV（服务 `ext.pack` 入库、tar 与 CLI 一致）；Q22 单测；Q23 DV（hello-card 的 `icons/check.svg` 构建、pull 读回）；Q24 e2e（`aiws-<id>.localhost` 上新建并 build）；L1(3) DV（`manifest_version: 1` 源码构建 / 导出 / 引入）；L3 Desktop 提示；`aiws.asset` e2e。L1(1) 跨 Zone、L1(2) Agent 凭手册、L2 第三账号、真实子域 build 未做 | `tests/aiworkspace/extensions.spec.ts`、`tests/dv/aiworkspace.extensions.spec.ts` `[ext p1]` |

### 8.6 真实 Zone DV（2026-10-30，本机 test.buckyos.io）

`uv run buckyos-build.py -s aiworkspace -s jarvis_agent` 与 `-s desktop` 后 `uv run start.py --all`（保留 data），`BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=bucky2025 pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.extensions.spec.ts`：3 例通过。P0 两例（Desktop 内建控件与双账号库；纯 CLI 与 Installer）作回归通过；新例 `[ext p1]`：CLI 从本地目录 build hello-card（带子目录 `icons/check.svg`）与 hello-board（定义写入 `deps`）→ `ext-export <hello-board> --pikg --install` 打出的包含两控件 → 另一工作区 `ext-import …#did:bns:alice#hello-board` 先建 hello-card 再建 hello-board → 写入一个实例和两张卡，部署的 Desktop 画出标题 “DV board” 与纵向两卡（截图 `p1-composite-instance`）→ 服务 `ext.pack` 生成 `svc-<tag>.<owner>.bns.did` 并经 `ext.install { pikg: { bucket_id, key } }` 入库两控件 → 解压 CLI 的 `aiws.tar.gz` 得到的 tar 摘要等于 `ext.pack` 返回的 `tar_digest`。DV 中途修了测试本身两处（例子目录相对路径、清理要求工作区标题等于前缀）。

### 8.7 未完成与已知限制（截至文档写作时）

- 未做的 DV：L1(1) 跨 Zone 中转引入（ssotest 三 Zone VM）、L1(2) Agent 只凭手册建控件、L2 第三账号场景、真实 Zone 上用 Desktop“定义为控件”、真实子域（`aiws-<id>.<zone>`）上 build / watch、Agent 凭手册写 composite。手机端 composite 只读显示没有专门的用例。
- CLI `ext-export` 没有 `--dir` 比对未 build 改动（Desktop 有）；“导入 .pikg 文件…”与 slot 拒收没有单独的 e2e。

- `ext.pack` 产物与 CLI 打包不逐字节相同（§8.3 第 4 条）。
- composite 的 `max` / `accepts` 只由客户端执行；内核与服务都不数实例里的子节点。
- 跨 Zone 中转引入组合控件时，客户端须按依赖顺序逐个 `ext.build`（服务的 `ext.import` 只管本 Zone 库）。
