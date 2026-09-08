# TODO：FB-04-COPY — 文件复制端到端支持

状态：已完成（2026-09-07，Linux 本地导出 FS 范围）。实现、真实服务 E2E、Mock/Files/Preview 回归及交付证据见 §6。优先级：P1。

用户反馈：Copy/Paste 是文件管理器的常用功能，要求后续 Code Agent 完成可实际使用的端到端支持。此前 [UI 评审 FB-04](./filebrowser_UI_review_2026-09-07.md) 只交付移动和复制禁用说明，本任务负责补齐文件复制；必要的后端实现和真实服务 E2E 验收均在任务范围内。

## 1. 目标与当前缺口

用户在 Files 选择文件或目录，执行 Copy，再到目标目录 Paste，得到内容一致、可以独立修改和删除的副本，源对象保持不变。支持单项、多项和递归目录复制。

### 1.1 本轮范围与会话语义

- 本轮只实现 `nfs_server` 导出目录内的本地文件系统复制。named object 级别的复制待接入真正的 DFS 后实现；不以 NamedStore、对象克隆或内容去重能力为交付前置条件。
- Copy/Paste 属于同一用户会话，不实现跨用户、跨登录会话的剪贴板共享。提交后的服务端任务有独立生命周期，页面刷新、关闭或 NFSP 会话重建不等于取消任务；原用户可重新查询任务。
- Copy 记录源身份，不冻结内容快照。Paste 提交时校验源身份并固定源、目标和复制选项；执行器读取每个文件前捕获可用的文件状态基线，提交副本前复检，检测到源被替换或内容变化时报告失败。目录复制不承诺整棵树的同一时刻快照；本地 FS 的状态复检也不承诺检测所有本机进程的并发修改。不得只凭相同路径复制已被替换的对象。
- 普通文件副本必须是独立文件实体，内容哈希允许相同；不得用与源共享可变内容的硬链接代替副本。保留目录结构、文件 mtime 和普通权限位；属主及 ACL 按目标创建规则处理，不复制 filedb 标签、分享授权和集合关系等附加数据。

### 1.2 复用系统 task-mgr

Paste 创建系统 task-mgr 任务并返回 `task_id`，由 `nfs_server` 作为执行者完成实际文件复制；长任务的状态、进度、取消和结果使用同一个 Task。优先复用服务创建并执行自身任务的现有接入方式，不另建通用任务系统；需要分发时再使用 Task Dispatch Center。

- 每次用户主动 Paste 生成新的请求幂等键。同一次提交的网络重试保留原键和不可变 Input，返回原 Task；不能只以源路径和目标路径去重，否则会吞掉用户主动再次粘贴。沿用 TaskMgr 的创建者身份与幂等键约束，任务创建、查询和控制保留同一用户上下文。
- TaskMgr 保证任务创建去重、状态写入的 runner epoch/revision 校验和 Result 一次性提交；实际文件系统副作用由复制执行器保证幂等。执行器保存每个子项的源身份、选定目标名称、临时文件、提交状态及副本身份，防止重复提交或恢复扫描启动并发复制；覆盖“文件已落盘、任务进度尚未写入”窗口。优先复用 Task 的 progress 承载恢复检查点；大目录的明细按需分页或分块持久化，不向单个 Task Input/progress/Result 填入无界清单。
- 非终态任务因执行器重启而恢复时沿用原 Task，先对账已提交子项，再继续未完成部分；不能确认的子项明确报告待核对或失败，不盲目再生成一个副本。Task 已进入终态后，用户选择“重试失败项”使用新幂等键、新 Task 和 `retry_of`，仅处理失败或未完成部分；不把原 Task 改回 Running，也不重复创建已成功子项。
- 取消通过 TaskMgr control 请求传给执行器。UI 在执行器确认前显示“正在取消”；执行器停止后续工作、清理未提交的临时文件并保留已提交子项，准确报告取消结果。目录部分完成不表示整目录回滚。
- 当前 NFSP `hello` session 还未绑定已认证用户，不能直接充当 Task creator。接入时复用现有可信认证入口传递实际调用者，使原用户能查询和控制自己的任务；完整 NFSP SSO/cap 和跨用户复制不随本任务扩展。

### 1.3 实施前缺口（本轮已补齐）

实施前 `data/folderOps.ts` 没有复制操作，Mock/NFSP 适配器均固定 `supportsCopy: false`。现已补齐真实复制执行器、任务、认证和两套适配；真实模式按已认证服务能力开放 Copy。

本任务要求检查现有存储/NFSP 能力，复用可满足语义的操作；缺少能力时补齐服务端、客户端和适配器。只启用菜单、将 `supportsCopy` 改为 `true`、修改提示文案、添加 Mock 成功反馈，均不能视为完成。

## 2. 必须交付的行为

- [x] 打通右键 Copy/Paste、工具栏共用命令、Ctrl/Cmd+C/V、“复制到”目标选择、“复制到另一栏”、复制修饰键拖放及手机菜单；各入口使用同一套能力与执行规则。
- [x] 复用深层目标浏览、路径输入、面包屑、最近目标和新建目标文件夹。允许在同目录复制，通过冲突流程产生新名称；不能复用移动操作的“同目录跳过”逻辑。
- [x] 复制成功后保留源文件、源目录层级与文件内容；目录复制包含空目录和未加载分页中的实体条目。普通文件副本拥有独立的文件实体身份，修改/删除任意一份不影响另一份。若底层使用写时复制，仍须验证独立性，不要求专门实现去重优化。
- [x] 文件系统软链接复制链接本身（包括失效链接），保留链接文本，不跟随目标；相对链接在新位置按文件系统规则解析，不承诺仍指向原目标。软链接不适用普通文件的内容独立性保证。硬链接按普通文件复制成独立文件；设备文件、FIFO、socket 等特殊文件明确报告不支持，不作为普通字节流读取。
- [x] Collection/View/Group 本身及 named object 目标不做内容复制；实体目录中的虚拟 binding 不递归跟随、不复制，并逐项报告跳过。集合成员能够通过真实目标 Ref 解析到本地文件或目录时，允许复制该实体；失效引用报告不可复制。不能把集合展示路径当成文件路径。普通文件副本、复制路径、复制集合引用保留不同剪贴板意图；集合添加/移除引用仍不复制或删除原件。
- [x] Copy 只记录待复制对象，Paste 才提交 task-mgr 任务。成功后剪贴板仍可用于再次粘贴；失败后仍可重试。任务遵循 §1.1 的源状态规则，校验文件系统源可读及目标可写；切换标签/窗口不能改变已提交的对象，也不能清除用户的新选择。页面刷新后可以重新查询同一用户的已提交任务，不要求恢复未提交的剪贴板。
- [x] 禁止目录复制到自身或后代。同名文件/目录进入统一冲突对话框，支持保留两者、跳过、取消和应用到后续同类冲突；保留扩展名并遵守 UTF-8 名称长度限制。默认不覆盖、不合并已有目录；安全替换/合并仍是独立任务。
- [x] 提供逐项成功、失败、跳过、取消、目标路径和失败原因；成功后立即刷新目标，并可定位副本。递归复制部分失败必须披露已创建的子项，不能把未完整复制的目录标为全部成功。
- [x] 按 §1.2 实现任务创建去重、执行互斥、取消、故障恢复和失败项重试。响应丢失后的同一请求重放不得重复创建任务或副本；用户主动再次 Paste 属于新任务，正常触发名称冲突流程。重试部分完成的目录只能复用经原任务记录及身份核验确认的副本目录，不扩展为任意目录合并。
- [x] 实际能力就绪后开放 Copy；真实无权限/不支持/加载状态保留明确原因。采用服务端本地文件系统复制，避免让浏览器整文件下载后再上传来完成服务器内部复制。

## 3. 实施入口与联动要求

路径相对于仓库根目录：

| 层 | 入口及要求 |
| --- | --- |
| UI/命令 | `src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx`、`menu/commands.ts`、`menu/registry.ts`、`dialogs/MoveTargetDialog.tsx`；复用现有命令和对话框，补齐复制意图、目标与执行分支。 |
| 操作/任务 | `src/frame/desktop/src/app/filebrowser/data/folderOps.ts`、`data/operationFeedback.ts`、`types.ts`、`data/schemas.ts`；提供复制接口、结果和取消/重试契约，检查与移动逻辑不同的同目录、剪贴板和源选择语义。 |
| 适配/客户端 | `src/frame/desktop/src/app/filebrowser/mock/folderOps.ts`、`data/nfsp/folderOps.ts`；客户端为 `src/frame/desktop/src/api/nfsp_client.ts`、`nfs_browser_client.ts`。Mock 与真实模式采用一致的复制语义。 |
| 系统任务 | `src/kernel/buckyos-api/src/task_mgr.rs`、`src/kernel/task_manager/`、`src/frame/desktop/src/api/task_mgr.ts`；复用 TaskMgr 2.0 共享类型、schema、幂等创建、progress/result 和 control 契约。参考 `src/frame/control_panel/src/app_install_engine.rs` 的服务执行及 progress 恢复模式；如使用 Dispatcher，遵循 `task_dispatcher.rs` 的 offer/bind/activate 协议。 |
| 服务端 | `src/frame/nfs_server/`；补齐本地复制执行器、TaskMgr 接入、可信调用者传递、源状态复检、逐项持久记录和执行幂等。任务 schema 与 Input/Result 同步共享类型；明确取消清理、重启对账及文件提交与任务进度之间的故障恢复规则。 |
| 文档/协议 | 同步 `src/frame/desktop/src/app/filebrowser/UI_DATAMODEL.md`、相关协议/共享类型、[nfs_server.md](./nfs_server.md) 和原 UI 评审实施状态；记录权限、元数据、时间戳及链接的复制规则。 |

不顺带扩展 named object 复制、回收站、分享、完整 NFSP 鉴权或目录合并。遵守仓库依赖与构建约束。

## 4. E2E 验收矩阵

关键成功路径必须从实际浏览器操作发起，经真实 NFSP 服务创建真实 TaskMgr 任务并由复制执行器完成，通过读回文件内容/哈希和目录结构断言验证。按钮可点击、出现成功提示、Mock/请求 stub 通过，均不足以证明真实复制完成。

| 场景 | 必须断言 |
| --- | --- |
| 单文件跨目录复制 | 从 Documents 经 Copy/Paste 到三级子目录；源与副本均存在，内容/哈希一致，实体身份不同；刷新页面及重启测试服务后仍存在。 |
| 副本独立性 | 分别修改、删除普通文件的源与副本，另一份内容与存在性不受影响；若底层采用写时复制，同样覆盖。 |
| 同目录与重复粘贴 | 同目录选择保留两者后产生有效新名称；成功后再次 Paste 仍可工作，源文件始终保留。 |
| 多项与递归目录 | 覆盖空目录、三级嵌套、超过一页的目录、零字节、二进制、中文/长文件名及真实大文件；结构和字节内容完整，记录大文件规模与耗时。 |
| 冲突 | 文件重名、目录重名、文件与目录同名；保留两者/跳过/取消/同类批量决策结果准确，原有目标不被覆盖。 |
| 部分失败与重试 | A 成功、B 冲突、C 文件系统无权限或 IO 失败；逐项结果与实际存储一致。终态后的失败项重试创建带 `retry_of` 的新 Task，不重复创建 A，不修改原任务终态。 |
| 任务去重与恢复 | 提交响应丢失、同键并发重投均返回同一 `task_id`，只有一个有效执行过程；用户主动再次 Paste 则产生新键、新任务。页面刷新后原用户仍可查询任务；重启服务并覆盖“副本已落盘、进度未写入”的故障窗口，恢复后不重复生成副本，无法确认的结果明确报告。 |
| 取消与源变化 | 复制目录期间通过 TaskMgr 取消；确认前显示正在取消，确认后未提交临时文件已清理、已提交子项保留。覆盖 Copy 后同一源文件被修改（读取执行时内容）、源移动/替换、复制期间检测到修改、目标失效；不会复制错误对象，已提交子项和剩余项可追溯，不产生伪成功。 |
| 上下文与入口 | 右键、快捷键、双栏目标/复制拖放及 375px 手机菜单可完成复制；慢复制期间切换标签或窗口并另选对象，新选择不受旧任务影响；输入框 Ctrl/Cmd+C/V 保留文本编辑行为。 |
| 链接、引用与既有流程 | 覆盖有效/失效/相对软链接复制自身、硬链接源产生独立普通文件、特殊文件拒绝、虚拟 binding 显式跳过、集合成员解析真实本地目标复制、named object/集合组不支持；添加/移除集合引用不改变原件。剪切移动、删除、上传和 Preview 回归保持通过。 |

真实测试使用独立临时目录和测试服务，自动准备数据并清理本次创建的对象。复用 `tests/e2e/pages/filebrowser.nfsp.spec.ts` 的真实服务路径（`FB_NFSP_E2E=1`、`VITE_NFS_PROXY`），补齐真实 TaskMgr、其 RDB 依赖及可信测试用户上下文的启动/fixture；原独立 NFSP fixture 不视为已经包含这些能力。保留 Mock 的确定性故障覆盖。权限拒绝验收指真实文件系统读写权限或只读目标拒绝，不要求完成用户间授权矩阵；成功数据路径、任务去重/恢复和文件系统权限拒绝不能全部被 stub 替代。

## 5. 完成标准与交付证据

- [x] UI → 操作接口 → NFSP 客户端 → 服务端/TaskMgr → 本地文件复制与持久化/读回全链路完成，上述验收有可复现的自动化覆盖；named object 复制不在本轮完成标准中。
- [x] Mock 回归和真实 NFSP E2E 实际执行通过；真实测试未被 skip，记录命令、环境、通过数和失败数。
- [x] Desktop 生产构建、相关静态检查及原 Files/Preview 回归通过；服务端有改动时运行相关 Rust 测试和构建。
- [x] 附桌面与手机操作截图，以及真实源/副本的内容一致性、身份独立性、持久化和故障恢复证据；更新本 TODO 与原评审的完成状态。

真实环境不可用时，记录具体阻塞、已完成工作和复现方式，并保持本 TODO 未完成。不得再次以“后端暂不支持”作为文件复制已经交付的结论。

## 6. 交付与验收记录（2026-09-07）

### 6.1 已交付

- 所有复制入口复用 `copyEntries`、共享能力/冲突/目标选择器；源选择和剪贴板保持不变，文件内容、集合引用和路径文本使用独立意图。目标成功后刷新，可定位副本；结果明细每页 100 项，可加载完整递归结果。
- 新增 `nfs.copy/v1` 共享 schema、签名本地 `copy_ref` 和六个 `copy_*` NFSP 方法。集合长期 Ref 与复制时原生身份分离，避免同路径替换后的锚点重绑误复制。真实用户来自可信 Bearer token，hello session 不能充当 creator。
- `nfs_server` 通过现有 TaskMgr 代理用户建任务并执行；不可变 Input/幂等键、执行文件锁、逐项 SQLite WAL 日志、临时文件原子不覆盖提交、读取前后状态复检、取消对账及终态 `retry_of` 均落地。
- 内容复制使用服务端 1 MiB 缓冲；目录枚举实际 FS，包含空目录和未加载条目；软链接复制文本，硬链接源生成独立文件，特殊文件拒绝，虚拟 binding 披露跳过。普通权限/mtime 保留，附加关系/分享元数据不复制。
- UI DataModel v1.4、NFSP 产品协议 §10、服务 README 及原评审实施状态同步。未新增第三方依赖，未部署或改动正在运行的开发 Zone。

主要新增入口：

| 层 | 实现 |
|---|---|
| 共享类型/schema/可信用户 | `src/kernel/buckyos-api/src/nfs_copy.rs`；`task_mgr.rs` 注册内置 schema |
| 复制执行及恢复 | `src/frame/nfs_server/src/copy.rs`；namespace/containers/search 发放原生 copy_ref |
| UI 任务适配/结果映射 | `src/frame/desktop/src/app/filebrowser/data/nfsp/copy.ts`、`data/copyResult.ts` |
| 刷新后查看任务 | `src/frame/desktop/src/app/filebrowser/dialogs/CopyTasks.tsx` |
| 确定性 Mock | `src/frame/desktop/src/app/filebrowser/mock/copy.ts` |
| 真实测试服务 | `src/kernel/task_manager/examples/nfs_copy_fixture.rs`、`tests/e2e/fixtures/nfs-copy.ts` |
| 自动化 | `filebrowser.copy.nfsp.spec.ts`、`filebrowser.copy.mock.spec.ts`、`tests/e2e/data/copy-model.spec.ts` |

### 6.2 实际执行结果

环境：Linux x86_64，Node 24.18.1，pnpm 11.18.0，Rust 1.97.1，Playwright Chromium。
NFSP/TaskMgr 使用独立临时目录，分别监听 3262/3382；复用真实 TaskManagerService、
TaskStore 及其 SQLite RDB 用户/系统分区，未替换任务创建、运行、控制或存储逻辑。
root 环境下以 uid/gid 65534 执行服务，chmod 000 源与 0555 目标实际触发权限拒绝。
签名测试用户只替代可信密钥来源，仍执行真实签名/claims 校验；生产使用 runtime 认证。

| 验证 | 结果 |
|---|---|
| 完整真实服务 E2E | **19 通过，0 失败，0 跳过**（18 项复制 + 原真实 NFSP 综合流程） |
| 原 Files/Preview + Mock 复制 + 映射 | **34 通过，0 失败，0 跳过**（原 30 项 + Mock 2 项 + 映射/性能 2 项） |
| 补充证据复跑 | 原有 2 项再次通过，补存源/副本身份及哈希，并验证结果面板分页加载全部 243 项 |
| Rust | **295 通过，0 失败**：buckyos-api 195、NFS 单元 42 + 集成 18、TaskMgr 40；最终 namespace/搜索联动后另复跑 NFS 60 项通过 |
| 构建 | nfs_server、TaskMgr 测试服务构建通过；Desktop TypeScript + Vite 生产构建通过 |
| 静态 | Desktop 相关 ESLint 0 错误，保留 3 条已有 TanStack Virtual/React Compiler 警告；新增测试 TypeScript 检查通过；`git diff --check` 通过 |

真实矩阵覆盖：三级目标、双向修改/删除独立性、同目录重复 Paste、243 项递归分页、
空/零字节/中文长名/二进制/大文件、软链接/失效链接/硬链接/FIFO、虚拟 binding、集合实体
与组/named object 边界、四种文件/目录冲突组合、同类批量决策、A 成功/B 冲突/C 权限拒绝、
部分目录重试身份复用、一次及全部提交响应丢失、8 路同键并发、刷新后续查原任务、
进程读取中退出和落盘未记进度窗口、无法确认的目标身份、取消临时清理、源修改/移动/替换、
目标消失、双栏及修饰键拖放、慢复制时另开标签并选择新对象、375px 手机复制、目标新建目录/
面包屑/最近目标，以及原剪切移动、删除、上传、搜索和 Preview 流程。

重启测试中的 ECONNRESET/socket hang up 是主动杀死测试执行器产生的预期故障；客户端
恢复后仍使用原 Task，不代表成功路径被 stub。最终通过结论以列出的完整执行及补充证据为准。

### 6.3 可复现命令

从仓库 `src/` 执行，构建和测试保留相同的 `CARGO_TARGET_DIR`（未设置则使用 `src/target`）：

```bash
cargo build -p nfs_server
cargo build -p task_manager --example nfs_copy_fixture
cargo test -p nfs_server -p task_manager -p buckyos-api

FB_NFSP_E2E=1 FB_COPY_AUTO=1 VITE_NFS_PROXY=http://127.0.0.1:3262 \
  pnpm --dir frame/desktop exec playwright test \
  tests/e2e/pages/filebrowser.copy.nfsp.spec.ts \
  tests/e2e/pages/filebrowser.nfsp.spec.ts --workers=1 --reporter=list

pnpm --dir frame/desktop exec playwright test \
  tests/e2e/pages/filebrowser.spec.ts tests/e2e/pages/filebrowser-review.spec.ts \
  tests/e2e/pages/filebrowser.copy.mock.spec.ts tests/e2e/pages/preview.spec.ts \
  tests/e2e/data/copy-model.spec.ts --workers=1 --reporter=list

pnpm --dir frame/desktop run build
pnpm --dir frame/desktop exec eslint src/app/filebrowser src/api/nfsp_client.ts \
  src/api/nfs_browser_client.ts src/api/nfs_copy.ts src/i18n/filebrowser-review.ts
```

新增测试静态检查，在 `src/frame/desktop` 执行：

```bash
pnpm exec tsc --noEmit --allowImportingTsExtensions --module ESNext \
  --moduleResolution bundler --target es2023 --jsx react-jsx --skipLibCheck \
  --types node,vite/client tests/e2e/fixtures/nfs-copy.ts \
  tests/e2e/pages/filebrowser.copy.nfsp.spec.ts tests/e2e/pages/filebrowser.copy.mock.spec.ts \
  tests/e2e/data/copy-model.spec.ts
```

### 6.4 截图、内容与恢复证据

- [桌面实际复制结果](./artifacts/FB-04-COPY/copy-desktop.png)、[375px 手机实际复制结果](./artifacts/FB-04-COPY/copy-mobile-375.png)。
- [完整测试结果及环境](./artifacts/FB-04-COPY/verification.json)、[真实 E2E 日志](./artifacts/FB-04-COPY/real-e2e.log)、[补充证据复跑](./artifacts/FB-04-COPY/evidence-e2e.log)。
- [源/副本哈希、inode、刷新/重启与独立性](./artifacts/FB-04-COPY/independence-and-persistence.json)。
- [128 MiB 与递归规模/耗时](./artifacts/FB-04-COPY/copy-size-and-time.json)、[响应丢失与落盘窗口恢复](./artifacts/FB-04-COPY/recovery.json)、[异身份目标明确失败](./artifacts/FB-04-COPY/ambiguous-recovery.json)。
- [数据映射性能](./artifacts/FB-04-COPY/copy-model-performance.json)、[Rust 测试](./artifacts/FB-04-COPY/rust-tests.log)、[生产构建](./artifacts/FB-04-COPY/desktop-build.log)、[相关静态检查](./artifacts/FB-04-COPY/desktop-eslint.log)。

实际记录：源 inode `2787061`，副本 inode `2787299`，SHA-256 均为
`aff343f3ce4030720fc5ea58e0673a96349fcd08d89d2d76951e7473bfee30eb`。落盘窗口恢复前后 inode 均为 `2788000`。
128 MiB 文件 SHA-256 为 `b0c10952e9d16ab904fe7b94eacfe57309b249dc622e52e90482bf375e0d72e0`；
包含其在内的 243 项目录复制及读回校验记录耗时 **7.778 秒**（本机单次观测，不是吞吐承诺）。

映射测试流式处理一百万条合成记录约 1.01 秒，Map 窗口最多 100 项；第 1、70、4321 页
各 100 项纯映射约 0.10–0.12 ms。网络 100 ms 是模型假设，非实测；不以该结果宣称百万
文件实际磁盘复制性能。服务端目前逐次聚合 journal，超大目录的汇总工作仍是线性的。

### 6.5 本轮边界

交付范围是 Unix 稳定原生身份的本地导出 FS，实际 E2E 在 Linux 执行。named object 复制、
任意目录合并、覆盖替换、完整 NFSP SSO/cap、回收站和分享不在本轮。普通权限/mtime
保留，属主/ACL 按目标创建规则，软链接保留文本。目录非整树快照，不能承诺检测所有本机
并发修改。测试验证进程故障恢复，未验证硬件断电；未运行整仓部署或完整 buckyos-build。
复制日志必须随服务数据持久保存。当前 UI 任务入口展示最近 50 个任务，协议支持游标继续查询。
