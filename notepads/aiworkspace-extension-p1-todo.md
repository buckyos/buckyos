# AI Workspace 扩展控件 P1 TODO：P0 遗留 + 组合控件

日期：2026-10-30

状态：未开工。依据 [doc/workspace/BuckyOS AI Workspace 扩展实现思路.md](<../doc/workspace/BuckyOS AI Workspace 扩展实现思路.md>) v0.9（§8 P1 行与“P0 遗留”表、§7.6 Q20–Q25 用户裁决、§7.2 Q15–Q17、§7.4.3 C1、§4.3 K1–K8）与 [P0 任务单](aiworkspace-extension-p0-todo.md) §12.1 的实现事实推导。第 2 节“现状”是 2026-10-30 对源码的核对（P0 未提交集之上），实施前重新核对当前分支。事件 / 动作绑定、HTML host API 3、来源包升级、HTML `parent`、Workspace 模板包都不在本单（§8 P2 / P3）。例子目录 [doc/workspace/hello-controls/](<../doc/workspace/hello-controls/README.md>) 的 `hello-board` 是本单 M0 要冻结的形态起点，M6 回写。

目标：两段。**前段**把 P0 遗留收口（提交两仓、三项 DV、第三账号场景、未 build 改动提示、服务端打包 `.pikg`、Desktop 导入 `.pikg`、超限资源对象引用、独立域名放开开发操作、HTML 控件取用 `assets`）。**主体**交付组合控件：先按 Q15 把内核校验拆成“原语 / 引用”与“业务 Schema”两层，再放开 `kind: composite`、`composition.json` 双端校验、定义 → 定义依赖边、内核级子图克隆（Q17）、Desktop 通用 composite 宿主（Chrome / slots / `stack` + `fixed` 布局）、“定义为控件…”，最后源码子目录（Q23）。顺序不能反（C1）：composite 的业务 schema 不能进重放路径。

## 1. 范围与已定决定

只复述实施需要的结论，不再讨论：

- **C1 顺序**：Q15 拆层 → 放开 `kind` → `deps` 非空与依赖边 → Q17 子图克隆 → composite 宿主 → Q23 子目录。前段遗留项与内核工作无依赖，可并行。
- **Q15（D01 方案 C）**：内核只校验结构与引用（容器 kind、`def_ref` 存在、slot 同父唯一、placement 分量、依赖边），业务规则（config 值域、composition 内容）由定义的 schema 在服务准入（`ext.build`）与 Desktop 校验，重放不执行。`types.rs` 按 README “Validator classes” 分类表拆成两个入口；每个校验器的 doc comment 已标 `Validator class:`，拆分以它为准。
- **K4：用户控件类型不进 `control_kinds`**。内核只多一个宿主 kind `composite`（进 `core.json` 的 `control_kinds`，让子节点可用 `"auto"` 分量）；具体是哪个控件由容器上的 `def_ref` 决定。composite 实例是 `buckyos.container { kind: "composite", def_ref, config, bindings?, layout: { mode: "def" } }`，子节点带 `slot`，仍是真实 Block（R23：皮肤由 Chrome 画，内容是实体）。
- **composition.json 只说三件事**（K3 / hello-board 提案）：`layout`（算法与参数）、`slots`（名字、接受类型、数量、放置）、`chrome`（背景、圆角、标题、空态），属性映射用 `{{config.<key>}}`。它不生成、不删除用户 Block。`events` 键是 P2，本单 build 继续拒绝。
- **布局算法只复用宿主能力**（K2）：P1 新增 `stack`（列 / 行、间距、内边距）与 `fixed`（子节点保留 x / y），`tree` 继续归脑图；任意新算法随宿主发布，不从包加载。求解仍是 `controls/solve.ts` 的纯函数、单向、只填 `"auto"` 分量（脑图 §5.7）。
- **Q16 依赖边**：block-def 新键 `deps: [{ ident, entity_id }]`（build 时把 `component.json` 的 `deps: ["<ident>"]` 解析成同 Workspace `data/extensions` 下的定义），引用边 `def_dep`（live、strong）：阻止删除被依赖定义、进导出 / Fork 闭包、依赖环在 prepare 拒绝；被依赖定义 build 时同 Workspace 内依赖者一并校验，不兼容整次失败并列出依赖者。
- **Q17 子图克隆为内核级操作**：一个 commit Operation（规划在 `plan.rs`，WASM 共享，离线可用），Desktop 复制 / 粘贴 / 组合实例复制、P3 模板实例化复用；整包 `ws.import` 继续保留 ID 不走它。
- **Q20 服务端打包**：新 RPC `ext.pack`，服务在 `ext.export` 的 bundle 上组装 APPDOC、按 CLI `pikg_archive.ts` 的确定性规则写 `aiws.tar.gz` 与 ZIP 外壳（store / server 已依赖 `tar`、`flate2`、`zip`），产物落调用者 `exports` 桶；不签名（私钥在用户手里，仍是 CLI `pikg sign`）。Desktop 给“下载 `.pikg`”与“直接入库”（复用 `ext.install { pikg: { bucket_id, key } }`）。
- **Q21**：不做 `.block.*`；Desktop “导入 `.pikg` 文件”= 上传到自己的 `exports` 桶 → `ext.install`，零新 RPC。
- **Q22**：一次 build 内联累计 ≤ 4 MiB 不变；超限时客户端先把大文件经 NDM 上传成 `cyfile:` 对象（P0 服务已按引用 + 网关鉴权处理），入参不变；hello-controls 第 6 步已写开发者视角。
- **Q23**：`buckyos.dir` 成员名允许 `/`（成员名编码相对路径，不做嵌套 dir），≤ 64 文件不变。
- **Q24 放开**：网关 `get_service_info_from_req` 按路径把 `/nfs/v1/*` 交给 nfs-server，不看 host；子域会话由 `prepare_app_launch { service_id }` 委托、必须是 `principal_kind: user`，nfs-server 文件面当 `Actor::User`。去掉 Desktop `onWorkspaceHost()` 门，真实子域 DV。
- **Q25 不做**：作者 DID 工作区内只是标签，对外靠 PIKG 签名；默认值不改、不迁移。
- **HTML 控件取用 `assets`**：host API 2 加 `aiws.asset(name) → blob URL`（可加性成员，控件用 `typeof aiws.asset === 'function'` 判断），不升版本号；事件与 `parent` 留给 host API 3（P2 / P3）。
- **`manifest_version` 升到 2**：新增键 `deps` 非空、`kind: composite`、`entry: composition.json`；旧 1 照常读（Q19）。build 对 `manifest_version: 1` 的 composite 报错指向升版。
- **原则**：所有系统服务对 Agent 可用（nfs-server 经网关）；DV 先全新安装不写兼容；错误类别只看枚举；遵守 [AGENTS.md](../AGENTS.md)：不新增第三方 JSON Schema / 打包库，Rust 复用已有 `tar` / `flate2` / `zip`。

## 2. 当前代码与接入点

| 范围 | 现状与差距 |
| --- | --- |
| [core/src/types.rs](../src/frame/aiworkspace/core/src/types.rs) `validate_block_def`（:1609，键表 :1603，`kind` 闭集 :1654）、`validate_container`（:814；kind `folder｜surface｜group` + control；`locked` 规则 :857）、`is_control_kind`（:912，读 `core.json`）、`enclosing_container` / `laid_out_by_control`（:969 / :988）、slot 同父唯一（:995-1010）、`check_member_name`（:1084，拒绝 `/`） | 三类校验同路径执行（README “Validator classes” 明说 today every class still runs on both paths）。要：拆两个入口；`kind` 加 `composite`；容器 kind `composite` 接受 `def_ref` / `config` / `bindings`，`layout.mode: "def"`；block-def 新键 `deps`、`composition`；成员名允许 `/`。 |
| [core/src/model.rs](../src/frame/aiworkspace/core/src/model.rs) 边表（:233-240：`bind / owns / embed / value / body` + P0 `def_source`） | 加 `def_dep`（live、strong）；`check_ref_target` 与删除阻止覆盖。 |
| [core/src/plan.rs](../src/frame/aiworkspace/core/src/plan.rs)（`dir.*` :862-916、`def_ref` 强边） | 无子图克隆原语；Desktop 复制在前端重映射（`clipboard.ts`）。加 `tree.clone`。 |
| [controls/core.json](../src/frame/aiworkspace/controls/core.json)（`control_kinds: ["mindmap"]`）、[controls/solve.ts](../src/frame/aiworkspace/controls/solve.ts)、`tree.ts`、`placement.ts` | 只有 tree。加 `composite` kind、`stack` / `fixed` 求解、按定义分派；重生成 `aiws/controls.json`（`deno run -A tools/build_catalog.ts`）。 |
| [store/src/ext.rs](../src/frame/aiworkspace/store/src/ext.rs) `compose`（:316；`deps` 必须为空 :458-461）、`check_declarative`（:219）、`narrowing`（:561）、`ext_build`（:694）、`ext_export`（:880，闭包只含 `def_source` dir）、`synthesize_sources`（:989） | 加 `composition.json` 解析与校验、`deps` 解析成实体 id、依赖者重校验、导出闭包带 `def_dep`；`ext.pack`。 |
| [store/src/library.rs](../src/frame/aiworkspace/store/src/library.rs)（`verify_bundle` :59、`untar_bundle` :114 读、`read_pikg` :171 用 `zip` 读） | 只有读。写 tar.gz / ZIP 要新模块 `store/src/pack.rs`；清单与库表加 `deps`；`ext.import` 按依赖顺序引入。 |
| [server/src/ext.rs](../src/frame/aiworkspace/server/src/ext.rs)（`ext.install` 三来源 :100-150、`import` :160、方法分派 :190-206） | 加 `ext.pack`；方法表 `experimental`；`stable-methods.json` 重录。 |
| [store/src/export.rs](../src/frame/aiworkspace/store/src/export.rs) `closure`（:72）、`TempFile`；[server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs) `own_binding`（`exports` 桶） | `ext.pack` 的产物写法与 `ext.install { pikg: { bucket_id, key } }` 的读法对称。 |
| [buckyos-api/src/app_doc.rs](../src/kernel/buckyos-api/src/app_doc.rs)（`AppType::Aiws`、builder 规则） | Rust 侧组装 `aiws` AppDoc 要能从 `{ name, version, owner, title, 子包 digest }` 构造并算 `app_doc_object_id`（与 TS `appDocObjectId` 一致）。核对是否已有 builder；没有则补最小构造。 |
| [buckyos-websdk `src/aiws_ext.ts`](../../buckyos-websdk/src/aiws_ext.ts)（`checkComponent` :145、`buildFiles` :113 累计 4 MiB、`collectSources` :297 平铺、`exportBundle` :238、`pullControl` :368、`controlSkeleton` :418） | 加 composition 预检、`deps`、`manifest_version 2`、递归目录、超限上传、`kind: composite` 骨架。 |
| [cli/modules/pikg_archive.ts](../../buckyos-websdk/cli/modules/pikg_archive.ts) `createDeterministicTarGz`（:100）、`tarHeader`（:757）、`writeStoredZip`（:222）；[pikg.ts](../../buckyos-websdk/cli/modules/pikg.ts) pack（APPDOC :758-800） | 服务端写入的对照物；两端字节一致是 M3 验收。 |
| [cli/modules/aiws.ts](../../buckyos-websdk/cli/modules/aiws.ts)（`build` :492、`pull` :540、`ext-export` :597、`library` :657、`ext-import` :679） | `library list` 显示 `deps`；`ext-import` 多条目按依赖排序；其余动词不变。 |
| Desktop [ui/controls/registry.ts](../src/frame/desktop/src/app/aiworkspace/ui/controls/registry.ts)、`registerControls.ts`、`mindmap/{definition,Chrome,ops}.tsx`；[canvas/clipboard.ts](../src/frame/desktop/src/app/aiworkspace/ui/canvas/clipboard.ts)（:197 粘贴固定包装 `mindmap`）、[CanvasView.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx)（:51-53 直接引 mindmap `addNode` / `convert` / `ops`；:615 duplicate） | 无通用 composite 宿主。加 `ui/controls/composite/`（`definition.tsx` 由 def 的 `composition` 生成 `ContainerDefinition`、`Chrome.tsx`、slots 放置）；剪贴板 / duplicate 对 composite 走 `tree.clone`；按注册表分派替换 mindmap 硬编码。 |
| Desktop [ui/extensions/ExtensionsDialog.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/ExtensionsDialog.tsx)（`homeReachable = !onWorkspaceHost()` :39；`ExportControls` :275，两出口 `library｜bundle`）、[controlSources.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/controlSources.ts)（`exportToHome` :79、watch :111）、[links.ts](../src/frame/desktop/src/app/aiworkspace/links.ts) `onWorkspaceHost`（:40） | 去门；加 `.pikg` 出口（`ext.pack` → 下载 / 入库）、“导入 `.pikg` 文件…”、未 build 改动提示、“定义为控件…”。 |
| Desktop [ui/extensions/htmlRuntime.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/htmlRuntime.ts)（`srcdoc` :179，API 表 :90-110）、[bridge.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/bridge.ts)（`fetchAsset` :218）、[AssetBlobImage.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/AssetBlobImage.tsx) | iframe 拿不到 `assets`。加 `aiws.asset(name)`：宿主读定义 `source_dir` 成员 → `fetchAsset` → blob URL，卸载时 revoke。 |
| Desktop [ui/extensions/InstanceForm.tsx](../src/frame/desktop/src/app/aiworkspace/ui/extensions/InstanceForm.tsx)、[canvas/InsertCatalog.tsx](../src/frame/desktop/src/app/aiworkspace/ui/canvas/InsertCatalog.tsx) | 插入 composite 实例 = 建容器（`def_ref` + config）+ 按 slots 的 `min` 建初始子 Block（可为空）。 |
| 手册 [cmd_skills/aiworkspace_template_dev.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev.md) “Controls”、`aiworkspace_run.md` | 无 composite、`deps`、`aiws.asset`。 |
| 测试 | core `core/tests/{extension,apply_log,mindmap,schemas}.rs`；server `server/tests/{ext,control}.rs`；websdk `deno test`；Desktop e2e `tests/aiworkspace/extensions.spec.ts`；DV `tests/dv/aiworkspace.extensions.spec.ts`。 |

## 3. M0：契约冻结（只写文档与 schema，不写实现）

目标：下面每个名字在 M0 结束后不再变；写进 aiworkspace README “Extension controls” 协议表与 `schemas/`，并把 hello-controls `hello-board` 改成冻结形态（去 `_comment`，`events` 暂留注释说明 P2）。

- [ ] **`component.json` `manifest_version: 2`**：`kind` 加 `composite`；composite 的 `entry` 是 `composition.json` 文件名；`deps: ["<author_did>#<name>", …]`（≤ 32，去重，不能含自己）；composite 不得有 `accepts` / `allow_no_source`（容器不是 Cell），`bindings` 允许（供 `{{bindings.<name>}}` 以后用，P1 只校验形状）。`schemas/component.schema.json` 升 2，TS / Rust 校验同步。
- [ ] **`composition.json`**（`schemas/composition.schema.json`）：`layout: { algorithm: "stack" | "fixed", direction?: "column" | "row", spacing?: number | "{{config.<key>}}", padding?: number | {t,r,b,l} }`；`slots: { <name>: { accepts: ["cell:<ident>" | "<view type>" | "container"], min?: int, max?: int, placement: "stack" | "top" | "bottom" | "free" } }`（名字 `[a-z][a-z0-9_]{0,31}`，≤ 16 个；`fixed` 布局只允许 `free`）；`chrome: { background?, radius?, border?, title?: { text, slot? }, empty? }`，字符串值允许 `{{config.<key>}}`，键必须在 `config_schema.properties` 里（build 校验）。无 `events`（出现即 `BUILD_FAILED` 指向 P2）。
- [ ] **block-def 新键**：`deps: [{ ident, entity_id }]`（服务写，客户端不得直接写：`ext.build` 之外的 commit 带 `deps` 变更由内核拒绝，与 `derived.build` 同思路）；`composition`（校验通过的 `composition.json` 内容，内联）；`kind: composite`。outline 投影 `deps` 的 ident 列表。
- [ ] **composite 实例**：`buckyos.container { kind: "composite", def_ref: { entity_id }, config, bindings?, layout: { mode: "def" }, title? }`；子节点 `slot` 必填且在定义 slots 里；`core.json` `control_kinds` 加 `composite`；`check_placement` 对 `fixed` 布局的子节点不接受 `"auto"`。复制实例 = `tree.clone`。
- [ ] **引用边 `def_dep`**：live、strong；删除被依赖定义 → `REFERENCE_BROKEN`；环 → 新错误码 `REFERENCE_CYCLE`（进 `error-codes.json` 与 `Code::ALL`）；导出 / Fork 闭包随包。
- [ ] **`tree.clone`**（Q17）：`{ op: "tree.clone", source_id, parent_id, index?, placement?, slot?, content: "private" }` → 深拷贝子树（容器 / Cell / 连接线），新 id；父子、`slot`、placement 重写；`def_ref` 保留（共享定义）；`source_ref` / `bindings` 指向子树内 `owns` 的数据时克隆数据并重映射，指向外部时保留（共享绑定）；两端都在子树内的连接线随克隆重映射，跨界连接线不复制；富文本 body 经 `richtext::fork`。结果 `new_ids: { <old>: <new> }` 进 commit outcome。离线 WASM 同一实现。
- [ ] **`ext.pack`**：`{ workspace_id, entity_ids[], package: { name, version, title? } }` → `{ bucket_id, key, obj_id, size, digest, app_doc_object_id, pkg_id }`；权限同 `ext.export`（`export`）+ 调用者本人的 `exports` 桶；产物文件名 `<name>-<version>.pikg`；AppDoc owner = 调用者（`library.rs app_id_from_did` 的反向规则，与 CLI `--owner did:bns:<user>` 默认一致）；不签名。
- [ ] **`ext.export` / `aiws-manifest.json`**：清单 `controls[]` 加 `deps: ["<ident>"]`；导出闭包按 `def_dep` 自动带齐依赖并去重；`purpose: pull` 不带依赖。`schemas/aiws-manifest.schema.json` 加 `deps`。`ext.install` / `ext.import` 校验包内依赖闭包完整（缺依赖 → `LIBRARY_PACKAGE_INVALID`）；`ext.import` 多条目按依赖拓扑引入。
- [ ] **`ext.build` 依赖者重校验**：build 定义 X 时，对 `def_dep → X` 的每个定义重跑 composition 校验（slot `accepts` 里的 `cell:<ident>` 仍存在、不检查实例）；任一失败 → 整次 `BUILD_FAILED`，诊断 `file` 为依赖者 ident。
- [ ] **host API 2 加 `aiws.asset(name) → Promise<string>`**：`name` 必须在定义的 `assets` 里；返回 blob URL；卸载时 revoke。README host API 表加一行，不升版本。
- [ ] **目录成员名**：`check_member_name` 允许 `/` 分隔的相对路径：段非空、不是 `.` / `..`、无 `//`、不以 `/` 开头或结尾，总长 ≤ 255。`entry` / `assets` / `agent` 文件名同规则；bundle 写 `controls/<dir>/<relative>`。
- [ ] **CLI**：动词不变。`library list` 列 `deps`；`ext-import` 多条目按依赖排序；`pikg init --kind aiws-controls --composite` 生成 composite 骨架。
- [ ] **Desktop 新入口名**：工作区控件对话框 **导出控件… → 生成 .pikg**（下载 / 直接入库）、**导入 .pikg 文件…**、选区右键 / 对象工具栏 **定义为控件…**；详情里“home 有未 build 改动”提示。
- [ ] **权限表**：`tree.clone` = 目标 `structure` + 源 `read`；`ext.pack` = `export`；`定义为控件` = build 权限 + 目标 Surface `structure`。

完成条件：README + schemas 一次 review 通过；hello-controls `hello-board` 两个文件按冻结形态改写并通过新的 `checkComponent`（composite 分支）；本单后续 M 不再改名字。

## 4. M1：P0 遗留（前段，与 M2 并行）

- [ ] **L6 提交两仓 P0 改动**（先于一切；既有无关失败：control_panel `sso_identity` scopes、Desktop i18n 两项、e2e CN17 与 `i18n.spec.ts:133` 单独处理，提交说明列出）。
- [ ] **L1 三项 DV**：(1) 跨 Zone 中转引入（三 Zone VM 组 ssotest：bob 从自己库向 alice 的 Workspace `ext.build` 内联 + `provenance`）；(2) Agent 只凭手册在本地目录建 declarative 控件并 `build <本地目录>`，记录它访问 aiworkspace 的路径，不可达按 bug 修；(3) 旧 `manifest_version: 1` 包在升到 2 之后仍可 `library install` / `ext-import`（本单 M0 升版后这一项有了真实夹具）。
- [ ] **L2 第三账号**：甲乙各从库引入 A/B 与 C/D，丙（未装包）看到四个控件并插入实例；DV spec 加该场景。
- [ ] **L3 未 build 改动提示**：导出 / 入库 / 生成 `.pikg` 前，对有工作副本（`user_state aiws:src:<def>` 或目录 `.aiws-build.json`）的定义，客户端 `collectSources` + `aiwsContentDigest` 与 `ext.export(purpose: pull)` 返回的 `digest` 比对，不同则提示“home 有未 build 的改动”并可继续；CLI `ext-export` 同（`--dir` 可选，给了才比）。
- [ ] **L4 `ext.pack` 与 Desktop 导入**：服务 `store/src/pack.rs`（确定性 tar.gz：路径排序、mode 0644、mtime 0、uid/gid 0、ustar，无链接；ZIP stored 外壳 + `PACKAGE_META` + APPDOC，与 `pikg_archive.ts` 规则逐项对照）→ 写 `exports` 桶；Desktop 导出对话框第三出口“生成 .pikg”（成功后“下载”经 `/bucket/v1/<bucket_id>/<key>` 带会话 token 取 blob；“直接入库”调 `ext.install`）；“导入 .pikg 文件…”（NDM 上传 bind 进 `exports` 桶 → `ext.install { mode: dev }` → 库列表刷新）。
- [ ] **Q22 超限上传**：`aiws_ext.buildFiles` 累计超过 4 MiB 时，按大小降序把文件经 NDM 上传成对象（Desktop `uploadAsset`、CLI 现有 NDM 上传），入参改 `{ name, obj_id }`；仍超限才报错。单测用 5 MiB 夹具。
- [ ] **Q24 放开**：删 `homeReachable = !onWorkspaceHost()` 与 `desktopOnly` 文案；`homeFiles()` 的 NFSP base 用当前 origin；watch（NFSP SSE）在子域验证；DV 在 `aiws-<id>.<zone>` 上新建 / build / watch / 写 bundle。
- [ ] **`aiws.asset`**：`htmlRuntime.ts` API 表加 `asset`；`bridge.ts` 解析定义 `source_dir` 成员 → `fetchAsset` → `URL.createObjectURL`，按实例缓存、卸载 revoke；e2e 用 hello-card 的 `check.svg`。
- [ ] 测试：server `ext.rs` 加 pack → install 往返（同一 bundle 两次 pack 字节相同；`read_pikg` 读回与 `verify_bundle` 一致）；websdk 单测 `buildFiles` 超限分流；Desktop e2e 导入 `.pikg`、未 build 提示、子域构建（mock 运行时模拟子域 host）。

完成条件：P0 遗留表 L1–L6 全部勾掉；hello-card 的图标在实例里显示；在工作区独立域名上完成一次 build。

## 5. M2：内核

- [ ] **Q15 拆层**：`types.rs` 的 `init_entity` / `validate_update` 拆成 `validate_primitive`（原语 + 引用）与 `validate_schema`（业务）两个入口，按 README 分类表逐个校验器归位；重放（`apply_log`、`ws.import`、副本）只走前者，准入两者都走。`core/tests/apply_log.rs` 加“业务违规但原语合法的日志能重放”用例；现有全部测试不变绿。
- [ ] **`kind: composite`**：`validate_block_def` 放开 `kind`，composite 必须有 `composition`（对象 ≤ 64 KiB，内核只查形状：对象、slots 名字正则、无 `events`）；`deps` 形状与“只能由 build 写”的守卫。
- [ ] **composite 容器**：`validate_container` 接受 `kind: composite` + `def_ref`（引用类：目标是 block-def 且 `kind: composite`）+ `config` / `bindings` / `layout.mode: def`；子节点 `slot` 必填并在定义 slots 内（引用类，定义变化时不回溯已有子节点）；`fixed` 时子节点不得 `"auto"`；`core.json` 加 `composite`。
- [ ] **`def_dep` 边**：`model.rs` 边表、`entity_refs`、删除阻止、环检测（prepare 阶段 DFS，`REFERENCE_CYCLE`）、闭包。
- [ ] **`tree.clone`**：`plan.rs` 新 Operation；`operations.json` / `commit-request.schema.json` 登记；outcome 带 `new_ids`。
- [ ] **成员名含 `/`**（Q23）：`check_member_name` 新规则；`dir.replace` / `set_member` 不变。
- [ ] 测试：`core/tests/extension.rs`（composite 定义 / 容器 / slot / 依赖边 / 环 / 删除阻止 / `deps` 守卫）、`core/tests/clone.rs`（私有内容克隆、外部绑定保留、连接线两端规则、富文本 fork、离线一致）、成员名用例；重建 WASM；`stable-methods.json`、`aiws/controls.json` 重录。

完成条件：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace` 全绿；旧 fixture 全部仍能导入；重放路径不再调用任何 schema 类校验器（用一个 panic 哨兵测试证明）。

## 6. M3：服务

- [ ] `store/src/ext.rs`：`compose` 分 `kind`：composite 读 `composition.json`，Rust 校验（M0 规则，含 `{{config.<key>}}` 键存在、slot `accepts` 的 `cell:<ident>` 在同 Workspace 已有定义里或在本次 `deps` 里）；`deps` ident → entity id 解析（缺 → `BUILD_FAILED` 诊断 `deps`）；写 `deps` / `composition` 键。
- [ ] 依赖者重校验（M0 规则）；`narrowing` 扩到定义级（被依赖定义删掉 slot 接受的类型 / 必填 binding 变化时列依赖者）。
- [ ] `ext_export`：闭包加 `def_dep`，清单 `deps`，`purpose: pull` 不带；`library.rs verify_bundle` 校验闭包完整；库表加 `deps`；`ext.import` 拓扑顺序、已满足的依赖 `satisfied`。
- [ ] `ext.pack`（见 M1 L4，若 M1 先做则此处只补 composite 闭包）。
- [ ] 方法表登记；错误码 `REFERENCE_CYCLE`、`DEP_MISSING`（build 诊断码）。
- [ ] 测试：`server/tests/ext.rs` 加 composite build 成功 / `events` 拒绝 / 缺依赖 / 环 / 依赖者不兼容整次失败并列出 / 导出闭包去重 / import 拓扑；pack 往返。

完成条件：用测试里的 `hello-card` + `hello-board` 两个控件，build 后 `doc.read` 出的 composite 定义能被 Desktop 宿主解析并插入实例。

## 7. M4：websdk 与 CLI

- [ ] `aiws_ext.ts`：`checkComponent` v2（composite 分支、`deps`、`manifest_version ≤ 2`）、`checkComposition`、`collectSources` 递归（成员名用相对路径，`.` 开头文件 / 目录仍是客户端私有）、`pullControl` / `exportBundle` 写子目录、`buildFiles` 超限分流（M1）、`controlSkeleton({ kind: 'composite' })`。
- [ ] `aiworkspace_client.ts` 加 `extPack`；CLI `library list` 显示 `deps`、`ext-import` 排序、`pikg init --composite`；`pikg_protocol.ts` 校验子包清单 `deps` 闭包。
- [ ] 单测：composite 预检、递归收集、超限分流、清单闭包。

完成条件：不开 Desktop，仅用 CLI 把 hello-controls 三个控件 build → `ext-export --pikg --install` → 第二个 Workspace `ext-import hello-board`（自动带 hello-card）。

## 8. M5：Desktop

- [ ] **通用 composite 宿主** `ui/controls/composite/`：`definition.tsx` 由定义的 `composition` 生成 `ContainerDefinition`（`configSchema` 来自定义、`children.accepts` 来自 slots、`layout.solve` 分派 `stack` / `fixed`）；`Chrome.tsx` 按 `chrome` 画背景 / 圆角 / 边框 / 标题 / 空态，`{{config.<key>}}` 取实例 `config` 回退定义默认值；Inspector 复用 `InstanceForm`；流式页缺省按 group。注册表按 `def_ref` 解析而不是按 kind 查表。
- [ ] **插入实例**：`InsertCatalog` 对 composite 定义 → 建容器 + 按 slots `min` 建初始子 Block（空 slot 显示 `chrome.empty`）；拖入 slot 时校验 `accepts`，不接受则提示。
- [ ] **“定义为控件…”**：选区 → 对话框（名字、作者 DID、标题、每个选中 Block 一个 slot 名与 `accepts` 预填、布局 `fixed`，可切 `stack`）→ 在 home 生成 `component.json` + `composition.json` + `AGENT.md` → build → 可选“把选区替换为实例”（新建容器 + `tree.clone` 选区进 slots + 删除原选区，一次 commit；不勾则只建定义与一个空实例）。
- [ ] **复制 / 粘贴 / duplicate**：composite 实例走 `tree.clone`；`clipboard.ts` :197 的 `mindmap` 固定包装与 `CanvasView.tsx` 的 mindmap 直引改为按注册表分派（最少覆盖 composite 与 mindmap 两种）。
- [ ] **源码子目录**：“编辑控件源码…” / pull / 新建骨架支持子目录；`exportToHome` 写子目录。
- [ ] M1 的 Desktop 项（`.pikg` 出口与导入、去门、`aiws.asset`、未 build 提示）若未在 M1 做完，在此收口。
- [ ] 手机：composite 实例只读显示（Chrome + 子 Block），不提供“定义为控件”。
- [ ] i18n en + zh；e2e `tests/aiworkspace/extensions.spec.ts` 加：composite 插入 → 改 `composition.json` 并 build 后布局 / 外观更新、子 Block 内容不变；复制实例子节点不变成脑图；“定义为控件”替换选区；slot 拒收。

完成条件：§8 P1 验收第一句——两个客户卡片实例分别绑定数据，改 `composition.json` 并 build 后布局 / 外观更新，私有内容不串改或重建；复制子节点不变成脑图。

## 9. M6：手册、DV、文档与例子回写

- [ ] 手册 `aiworkspace_template_dev.md` “Controls”加 composite（`composition.json` 三件事、`deps`、`aiws.asset`、子目录）；`aiworkspace_run.md` 加 composite 实例的 `doc.commit` 形态与 `tree.clone`。
- [ ] 真实 Zone DV（全新安装）：(1) Alice 在 Desktop 用“定义为控件”把两张 hello-card 做成 hello-board，改 `gap` build 后间距变化、卡片内容不变；(2) `ext-export --pikg` 得到的包 `pikg info` 可读、`app install` 入库、Bob 的 Workspace `ext-import hello-board` 自动带 hello-card；(3) Desktop “生成 .pikg → 直接入库”与 CLI pack 的 `aiws.tar.gz` digest 一致；(4) 子域上 build；(5) Agent 凭手册写一个 composite 并 build。
- [ ] 文档：aiworkspace README 协议表与 host API 表；Desktop README；设计文档 §8 P1 改“已实施”、§7.2 Q15–Q17 状态列更新；《扩展控件实施记录与验收报告》追加 P1 一节（沿用现有格式）。
- [ ] **hello-controls 回写**：`hello-board/{component.json, composition.json}` 去 `_comment` 成为 P0/P1 真实契约（`events` 移到 README 第 7 步的 P2 说明）；`aiws-manifest.json` 加 hello-board 与 `deps`；README 第 6 / 7 / 8 / 9 步与 §2 总表把 `[P1]` 标记改为可用；`behavior.js` 的 `aiws.asset` 注释改成真实代码。

## 10. 不在本单

- 事件 / 动作绑定（`events` / `on` / `aiws.emit`）、HTML host API 3、设计 / 运行态边界、属性级协作与撤销、来源包显式升级（替换 `EXT_IDENT_EXISTS`）——P2。
- HTML `parent`、新布局算法的包加载、Agent 组合结果、移动端编辑、公共端 / 回放、Workspace 模板包——P3。
- `.block.*` 交换格式（Q21 已否）；作者 DID 自动填充与迁移（Q25 已否）；服务端签名。

## 11. 验证命令

- core：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace`；WASM：`PATH=/tmp/dev-cache-root/cargo/bin:$PATH CARGO_TARGET_DIR=/tmp/dev-cache-root/cargo-target bash src/frame/aiworkspace/wasm/build.sh`；目录：`deno run -A src/frame/aiworkspace/controls/tools/build_catalog.ts`。
- buckyos-api：`cargo test -p buckyos-api`（AppDoc `aiws` 构造）。
- websdk：仓库 `buckyos-websdk` 的 `deno test`；CLI 对本地服务：`deno run -A cli/main.ts aiws build ./doc/workspace/hello-controls/controls/hello-board --workspace <ws>`。
- Desktop：`pnpm check && pnpm exec eslint src/app/aiworkspace tests/aiworkspace && pnpm build`；e2e：`PATH=/tmp/dev-cache-root/cargo/bin:$PATH AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/extensions.spec.ts`。
- 真实 Zone DV：`BUCKYOS_UI_DV_BASE_URL=... BUCKYOS_TEST_ADMIN_PASSWORD=... pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.extensions.spec.ts` + §9 手工步骤；跨 Zone 用 ssotest 三 Zone VM 组。

## 12. 验收（对照设计 §8 P1 行）

- 两个客户卡片实例分别绑定数据，改 `composition.json` 并 build 后布局 / 外观更新，私有内容不串改或重建；复制子节点不变成脑图。
- 混选 render 与组合定义导出，共享依赖去重，在新 Workspace 可分别实例化；被依赖控件收窄接口时 build 列出依赖定义并失败。
- P0 遗留 L1–L6 全部完成；Q20–Q24 的裁决各有一条 DV 或 e2e 证据。
- 跨阶段回归（设计 §8 列表）不退化：build 只改本工作区；`unchanged` 不空提交；`ext.install` 三来源同一记录；重放不执行业务 schema。
