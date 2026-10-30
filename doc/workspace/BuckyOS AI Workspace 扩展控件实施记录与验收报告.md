# AI Workspace 扩展控件（P0）：实施记录与验收报告

> 日期：2026-10-30。对应设计：[扩展实现思路](<BuckyOS AI Workspace 扩展实现思路.md>) v0.8（D1–D8、A1–A7、Q18 / Q19），任务单 [notepads/aiworkspace-extension-p0-todo.md](<../../notepads/aiworkspace-extension-p0-todo.md>)（M0–M6）。全部改动未提交（buckyos、buckyos-websdk 两仓）。
>
> 环境：Rust 测试用真实 SQLite 与进程内 nfs-server；Desktop e2e 是独立 `aiworkspace`（调试构建）+ 独立 nfs-server（`home` 原生导出、无鉴权）+ Vite 开发服务器（模拟运行时、`/nfs/v1` 代理到 nfs-server）+ Chromium；真实 Zone DV 在本机 `test.buckyos.io`（§7，全新安装，两例通过）。
>
> 本文只记录**已经运行过的事实**，每项结论后附产生它的命令或测试；没有运行过的写在 §6。

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
