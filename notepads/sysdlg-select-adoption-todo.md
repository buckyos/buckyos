# 各 App 改用系统文件选择器（sysdlg/select）TODO

> 状态：全部已裁决（owner，2026-11-10；§8.2 的 N1–N4 按建议）。**2026-11-10 已实施除 §5.1 以外的全部条目（未提交）**，实施结果、偏差和未验证项见 §10。§5.1（D9、H1、D5、B6）仍等 nfs_server 的 `snapshot`，那份 TODO 的 §6 还有 4 个待裁决项。现状核实于 2026-11-10（buckyos `7e71bb20f`、buckyos-websdk `84501f0`），下文行号以这两个提交为准。
>
> 依据：[BuckyOS 系统对话框实现方案](../doc/desktop/BuckyOS%20系统对话框实现方案.md)。用到的几节：§4.3（`objId` 与 `ensureObjId`；Zone 条目不经浏览器中转）、§5（选择器界面）、§11.4（已实施与未做）。
>
> 范围：buckyos 仓库里所有用浏览器文件选择框的地方（`<input type="file">`、`webkitdirectory`、`showDirectoryPicker`），加上 websdk 里的一处。Desktop、OpenDAN WebUI、Text Editor、sys_test、node_active 都查过了。BuckyOSApp（Tauri 客户端）里没有这类代码。

## 0. 产品原则（owner，2026-11-10）

**目标用户是普通的手机用户，没有 PC。NFS（Zone）是用户的主文件系统，从 Zone 选文件是常规的高频流程；本机文件是次要来源。**

由此推出本 TODO 的几条做法：

1. 所有挑内容的入口都拉起系统选择器，用户先看到的是 Zone。本机来源放在选择器里的“本机”入口（`allowLocal: true`），不再在调用方界面上另放直达原生选择框的按钮。
2. 选择器本身要撑得起高频使用，尤其是在手机上：要有缩略图和搜索，起始位置要合适（§3.2）。
3. 最高频的两个入口——聊天附件和发动态——要的是对象 ID，都卡在 nfs_server 的 `snapshot` 上，所以 `snapshot` 的优先级随之提高（N1）。
4. 保存的方向默认也是 Zone：收到的附件“保存”，就是存到用户自己的 NFS 上（owner 已确认），不是下载到手机（§5.2、§7）。

设计稿 §5.2 末段（“以本机为主的入口……可以保留直达原生选择器的按钮”）和这条原则冲突，要改（C5）。

## 1. 结论

| 类别 | 数量 | 去向 |
|---|---|---|
| 已经换掉 | D6（AI Workspace 批量导入 4 个入口）、D11（File Browser）、T1/T2（Text Editor） | 只剩 AI Workspace 在自有源页面上的原生回退（A9） |
| 现在就能换 | AI Workspace 15 个输入框，Desktop 其他应用 5 处（`ImagePicker` 算一处，覆盖三个调用方），OpenDAN WebUI 3 处 | §4 |
| 等 nfs_server 的 `snapshot` | MessageHub（D9）、HomeStation 发布框（H1）、安装向导（D5）、AI Center Playground | §5.1 |
| 等附件对象的鉴权衔接 | 收到的附件存到用户的 NFS（V1）。落盘接口 `commit_object` 已经有了，缺的是让用户有权提交这个对象 | §5.2 |
| 保留原生 | File Browser 上传、MessageHub 访客页、websdk `ndm.pickupAndImport`（只作为“从本机上传”） | §6 |
| 不在本 TODO | 下载和导出（“另存为”方向），按原则默认应存进 Zone | §7，另立 |

## 2. 判断规则

1. **换**：用户挑的是“内容”（照片、文档、表格、安装包、备份）。按 §0，这些内容默认就在 Zone 里。
2. **不换**：语义就是“把本机文件传进 Zone”；页面没有 Zone 会话。
3. **入口形态只有一种**：原来的按钮改为拉起选择器；调用方能处理本机文件时传 `allowLocal: true`。桌面上的拖放保留（拖进来的一定是本机文件）。
4. **调用方怎么用结果**——这一点决定现在能不能做：
   - 要字节（解析、导入、转成 data URL、传进自己的桶）：Zone 条目用 `readZoneFiles` 读成 `File`，然后走原来的逻辑。这是 D6 的做法，**现在就能做**。
   - 要引用（预览、编辑）：直接用 `source`。**现在就能做**。
   - 要不可变的对象 ID（消息附件、动态附件、安装包）：要用 `ensureObjId`。它的 Zone 分支等 nfs_server 的 `snapshot`；按 §4.3，不做“先读到浏览器再登记”的过渡做法。这类入口在 `snapshot` 就绪之前**维持现状**，不换成一个选不了 Zone 文件的选择器。

## 3. 公共部分（先做）

### 3.1 调用方这一侧

- [x] **C1 通用的取文件 hook。** 把 AI Workspace 的 [`useImportPicker`](../src/frame/desktop/src/app/aiworkspace/ui/shell/importPicker.ts) 挪到 `sysdlg/select/`，参数加上 `accept`、`multiple`、`folder`，返回 `File[] | null`。行为和现在一样：没有窗口对话框层时 hook 本身是 null，调用方保留原生输入框。§4 里所有“要字节”的入口都用它，每处只改几行。
- [x] **C2 `accept` 换算。** 把原生 `accept` 字符串转成 `ContentSelector[]`（MIME 和扩展名两类），各入口继续用已有的常量：`TABULAR_ACCEPT`、`OFFICE_ACCEPT`、`ACCEPTED_IMAGE_TYPES`、`.pikg`、`.zip`。反方向的换算 [`SelectDialog.tsx:43`](../src/frame/desktop/src/sysdlg/select/SelectDialog.tsx#L43) `inputAccept` 已经有了。
- [x] **C3 mock 运行时的文件内容。** mock 的 [`readContent`](../src/frame/desktop/src/app/filebrowser/mock/folderOps.ts#L66) 只返回 `mock content of <path>`，所以“从 Zone 导入 PDF / 表格 / 图片”的用例没法在 mock 下写（§11.4 记的 AI Workspace 缺口就是这个原因）。需要让 mock 条目能挂夹具字节。按 §0，从 Zone 选是主路径，必须有用例覆盖。
- [x] **C4 用例改写。** 原生输入框挪进了选择器，测试要在选择器里多点一次“本机”。受影响的有：AI Workspace 的 `pdf-import`、`pdf-fidelity`、`office-import`、`drawio`、`table-import`（约 15 处 `waitForEvent('filechooser')`），`features.spec.ts`（`aiws-new-image`、`aiws-import-file`），`settings.spec.ts`（壁纸），`agent-setup.spec.ts`、`object-presentation.spec.ts`（头像），以及 `tests/dv/aiworkspace.*`。在 [`tests/e2e/fixtures/selectDialog.ts`](../src/frame/desktop/tests/e2e/fixtures/selectDialog.ts) 里加一个 `pickLocal(page, files)`。每个入口的主用例改成从 Zone 选，本机作为补充用例。
- [x] **C5 文档同步。**
  - 设计稿 §5.2 末段“什么时候用”改成 §0 的原则。
  - [`doc/sdk/context ext.md`](../doc/sdk/context%20ext.md) 第 442 行已经写明选文件由系统统一处理。补一句：`ndm.pickupAndImport` 只用于“从本机上传”，要选内容就用 `selectContent`，要对象 ID 再用 `ensureObjId`。
  - [`buckyos_app_dev`](../src/apps/jarvis_agent/agent/cmd_skills/buckyos_app_dev/SKILL.md) 第 31 行讲了 `buckyos/app-frame`，但没讲选文件。Agent 会照这份手册写 App，应补上“选文件默认用 `selectContent`，不放原生选择框”。这属于 cmd_skills 联动，要按 AGENTS 的要求记录和验证。

内存问题：`readZoneFiles` 会把整个文件读进内存，而本机的 `File` 是惰性读取的。不过 draw.io 和表格导入本来就对本机文件调用 `file.arrayBuffer()`（[CanvasView.tsx:1428](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx#L1428)、[:1434](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx#L1434)），峰值内存差别不大。只有工作区包导入（A7）原来是从 `File` 直接上传，大备份从 Zone 导入时会整个进内存，在手机上更要注意。PDF 和 Office 导入器对本机文件是否也整读，这次没有核实。

### 3.2 选择器本身（高频使用的前提）

现在的选择器是纯文本列表：图标、文件名、大小。没有缩略图，也没有搜索（§5.1 把搜索放在了第一阶段之外）。起始位置是“该调用方上次确认的目录”，再缺省是 home。

- [x] **S1 缩略图。** 图片和视频要有缩略图；`accept` 只收图片或视频时，用网格布局。File Browser 的数据层已经有 `thumbnailUrl`，组件可以参考 [MainContent.tsx:409](../src/frame/desktop/src/app/filebrowser/MainContent.tsx#L409) 的 `EntryThumbnail`。这是 B1、B2、D9、H1 等照片类入口换过去的前提，否则从 Zone 挑照片比手机相册难用。
- [x] **S2 搜索。** 复用 File Browser 的 [`useSearch`](../src/frame/desktop/src/app/filebrowser/data/search.ts#L63)（§5.1 原本就打算复用它）。
- [x] **S3 起始位置。** `open` 模式在没有上次目录、也没有给 `start` 时，先进“最近”（`view://recent`，已经在位置列表里），而不是 home。见 N4。

## 4. 现在就能换

### 4.1 AI Workspace（在桌面窗口里，路径 A）

| 编号 | 入口 | 位置 | 改成 | 优先级 |
|---|---|---|---|---|
| A1 | 主菜单“插入 → 导入 draw.io / PDF / Office / 表格” | [CanvasView.tsx:1427–1434](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx#L1427)，命令在 :856、:875、:895、:933 | `open` 单选，`accept` 用各自的常量 | P1 |
| A2 | 需要文件的对象（目录项 `needs: 'file'`）：流式页插入、工具栏、目录详情里的文件框 | [CanvasView.tsx:572](../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx#L572)、[ObjectToolbar.tsx:109](../src/frame/desktop/src/app/aiworkspace/ui/canvas/ObjectToolbar.tsx#L109) / [:173](../src/frame/desktop/src/app/aiworkspace/ui/canvas/ObjectToolbar.tsx#L173)、[InsertCatalog.tsx:224](../src/frame/desktop/src/app/aiworkspace/ui/canvas/InsertCatalog.tsx#L224) | `open` 单选。InsertCatalog 里那个是表单字段，改成“选择文件…”按钮加已选文件名 | P1 |
| A3 | 替换资源的文件 | [builtin.tsx:264](../src/frame/desktop/src/app/aiworkspace/ui/blocks/builtin.tsx#L264) `pickReplacement`、[editors.tsx:269](../src/frame/desktop/src/app/aiworkspace/ui/blocks/editors.tsx#L269) | `open` 单选，`accept` 由 `replaceAccept(payload)` 换算 | P1 |
| A4 | 数据树的“图片”“导入表格” | [DataTree.tsx:233](../src/frame/desktop/src/app/aiworkspace/ui/sources/DataTree.tsx#L233)、[:237](../src/frame/desktop/src/app/aiworkspace/ui/sources/DataTree.tsx#L237) | 同上。同一排的“导入文件 / 导入文件夹”已经由 D6 换掉 | P1 |
| A5 | 课堂受众名单导入 | [AudienceBlock.tsx:78](../src/frame/desktop/src/app/aiworkspace/ui/classroom/AudienceBlock.tsx#L78) | `accept` 用 `TABULAR_ACCEPT` | P2 |
| A6 | SVG 形状“从文件读” | [InsertCatalog.tsx:198](../src/frame/desktop/src/app/aiworkspace/ui/canvas/InsertCatalog.tsx#L198) | 读成文本填进文本框 | P2 |
| A7 | 导入工作区包（`.zip`，新建或恢复） | [shell/dialogs.tsx:305](../src/frame/desktop/src/app/aiworkspace/ui/shell/dialogs.tsx#L305) | 表单字段改成按钮。备份导出见 §7 | P2 |
| A8 | 扩展控件从 `.pikg` 导入 | [ExtensionsDialog.tsx:64](../src/frame/desktop/src/app/aiworkspace/ui/extensions/ExtensionsDialog.tsx#L64) | `accept` 为 `.pikg`。导入后照旧传进自己的桶 | P2 |
| A9 | 自有源页面（`aiws-<id>.<zone>`）：`useSelectContent()` 是 null，上面所有入口和 D6 的入口（[InsertCatalog.tsx:97–104](../src/frame/desktop/src/app/aiworkspace/ui/canvas/InsertCatalog.tsx#L97)、[DataTree.tsx:234–240](../src/frame/desktop/src/app/aiworkspace/ui/sources/DataTree.tsx#L234)）都回退成原生 | C1 的 hook | 没有窗口时走路径 C：用 websdk 已导出的 [`openSelectEmbed(systemOrigin, …)`](../../buckyos-websdk/src/app-frame.ts#L409)，系统入口按 `buckyos_app_dev` 的取法（`${location.protocol}//${buckyos.getZoneHostName()}`）。做完后删掉所有原生回退 | P1 之后 |

### 4.2 Desktop 其他内置应用

| 编号 | 入口 | 位置 | 改成 | Zone 条目怎么用 | 优先级 |
|---|---|---|---|---|---|
| B1 | 公共组件 `ImagePicker`。三个调用方：Agent 创建向导的头像（[AvatarPicker.tsx:21](../src/frame/desktop/src/app/agent-setup/AvatarPicker.tsx#L21)）、自己的头像（[SelfDetailPage.tsx:61](../src/frame/desktop/src/app/users-agents/components/detail/SelfDetailPage.tsx#L61)）、Zone 图标（[ZonePage.tsx:170](../src/frame/desktop/src/app/settings/pages/ZonePage.tsx#L170)） | [ImagePicker.tsx:89](../src/frame/desktop/src/presentation/ImagePicker.tsx#L89) | `open` 单选，`accept` 用 `ACCEPTED_IMAGE_TYPES`。改一个组件，三处都换掉 | 读成 `File` 后走 `imageFileToDataUrl` | P1，在 S1 之后 |
| B2 | 壁纸“选择图片…” | [WallpaperSection.tsx:348](../src/frame/desktop/src/app/settings/components/WallpaperSection.tsx#L348) | `accept` 为 `image/*`。拖放保留 | 读成 `File` 后走 `wallpaperStore.setImage` | P1，在 S1 之后 |
| B3 | Preview 落地页的“选择文件…” | [PreviewLanding.tsx:126–144](../src/frame/desktop/src/app/preview/PreviewLanding.tsx#L126) | `open` 多选。路径输入框保留，它还用来输入对象 ID 和 `obj://bucket/` | **不读字节**：直接用 `{ kind: 'cyfs-path' }` 引用；多选时和 `filesToRequest` 一样组成 list 会话 | P1 |
| B4 | 工作流导入定义文件（JSON / DSL） | [workflow/components/ImportDialog.tsx:201](../src/frame/desktop/src/app/workflow/components/ImportDialog.tsx#L201) | 拉起选择器 | 读成文本 | P2 |
| B5 | AI Center 导入提供方规则 JSON | [ProviderRulesPanel.tsx:180](../src/frame/desktop/src/app/ai-center/components/providers/ProviderRulesPanel.tsx#L180) | 拉起选择器。原先“几乎总是刚下载到本机”的理由在 §0 下不成立：Task Center 的下载本来就存进 Zone 的 Downloads | 读成文本 | P2 |

### 4.3 OpenDAN WebUI（各 Agent 子域上的独立页面）

前提：WebUI 现在没有接 `buckyos/app-frame`（只用了 `buckyos` 的 kRPC 和会话）。接入时，在桌面窗口里用 `AppFrameClient.selectContent`（路径 B），独立标签页用 `openSelectEmbed`（路径 C）。读 Zone 文件要一个 NFSP 客户端，Text Editor 的 [main.tsx](../src/apps/text_editor/src/main.tsx) 就是这样接的。和 Skills 页 SP2 改版（[opendan-skills-page-ui-todo](opendan-skills-page-ui-todo.md)）一起做。

| 编号 | 入口 | 位置 | Zone 条目怎么用 |
|---|---|---|---|
| O1 | Skills“从 `.pikg` 安装” | [Skills.tsx:366](../src/frame/opendan/web/src/pages/Skills.tsx#L366) | 读成 `File` 后走 `readSkillPikg`（在浏览器内解析） |
| O2 | Skills 文本导入的“从文件” | [Skills.tsx:421](../src/frame/opendan/web/src/pages/Skills.tsx#L421) | 读成文本 |
| O3 | Agent 资料“更换照片” | [Home.tsx:126](../src/frame/opendan/web/src/pages/Home.tsx#L126) | 读成 `File` |

## 5. 等服务端能力

### 5.1 等 nfs_server 的 `snapshot`

这几处要的是不可变的对象 ID。Zone 条目要经 `ensureObjId`，而它的 Zone 分支现在以 `unsupported` 拒绝，要等 [nfs-server-file-objid-todo](nfs-server-file-objid-todo.md) 第二步完成。按 §2 第 4 条，在那之前维持现状。

| 编号 | 入口 | 位置 | `snapshot` 就绪后 |
|---|---|---|---|
| D9（设计稿） | MessageHub 回形针菜单的“选择文件 / 选择文件夹” | [ConversationComposer.tsx:631–644](../src/frame/desktop/src/app/messagehub/conversation/input/ConversationComposer.tsx#L631)，菜单在 :657–677 | **替换**这两项（原计划是新增“从 Zone 选择”）：一个入口拉起选择器，`open` 多选，允许选文件夹（目录由 `snapshot` 返回 DirObject），`allowLocal: true`。访客页（[guest/GuestRoute.tsx](../src/frame/desktop/src/app/messagehub/guest/GuestRoute.tsx)）也用这个组件，但没有 Zone 会话，那里保留原生输入框 |
| H1（新） | HomeStation 发布框：媒体、文件、文件夹三个原生入口 | [PublishComposer.tsx:183–197](../src/frame/desktop/src/app/homestation/publish/PublishComposer.tsx#L183) | 同 D9，替换成选择器；媒体按钮带图片 / 视频的 `accept`。附件是 `store.uploadAttachment` 返回的对象 ID。要在设计稿 §11 补上这个编号 |
| D5（设计稿） | App Service 安装向导：“从本机上传”和“从 Personal Server 选择”（后者是写死三个示例文件的 mock） | [InstallWizard.tsx:167](../src/frame/desktop/src/app/app-service/pages/InstallWizard.tsx#L167)、[FilePickerDialog.tsx](../src/frame/desktop/src/app/app-service/components/installer/FilePickerDialog.tsx) | 合并成一个“选择安装包…”（`allowLocal`，`accept` 为 `.pikg`），拖放保留；删掉 `FilePickerDialog` 和示例数据 |
| B6 | AI Center Playground 的“上传”（结果是 `named_object` 的 obj_id） | [RequestFields.tsx:76](../src/frame/desktop/src/app/ai-center/components/playground/RequestFields.tsx#L76) | 开发者工具，P2 |

`snapshot` 就绪后的顺序：websdk `ensureObjId` 的 Zone 分支 → D9 → H1 → D5 → B6。D9、H1 还要等 S1（缩略图）。

### 5.2 收到的附件存到用户的 NFS（V1）

owner 已确认（2026-11-10）：用户“保存”附件，就是存到自己的 NFS 上。

**现状。** MessageHub 和 HomeStation 收到的附件只有两种处理方式：
- 单个文件用 `<a download>` 下载到本机：MessageHub 的 `DownloadButton`（[MediaAttachment.tsx:321](../src/frame/desktop/src/app/messagehub/conversation/media/MediaAttachment.tsx#L321)），HomeStation 的下载链接（[ObjectAttachment.tsx:46](../src/frame/desktop/src/app/homestation/card/ObjectAttachment.tsx#L46)）。
- 文件夹附件用 `showDirectoryPicker` 选一个本机目录，再逐个文件写进去：[MediaAttachment.tsx:380](../src/frame/desktop/src/app/messagehub/conversation/media/MediaAttachment.tsx#L380)、[ObjectAttachment.tsx:22](../src/frame/desktop/src/app/homestation/card/ObjectAttachment.tsx#L22)。

手机浏览器（iOS Safari、Android Chrome）都没有 `showDirectoryPicker`，所以手机用户现在根本看不到“保存文件夹”。MessageHub 现有的“收藏”存进的是收藏桶，不是目录。

**改成。** 主操作是“保存”：拉起选择器的 `save` 模式，`suggestedName` 填附件名，确认后由服务端把对象落到 `parent/name`，不经浏览器下载后再上传。下载到本机保留为桌面上的次要操作。

**落盘接口已经有了。** nfs_server 的 `commit_object { parent, name, obj_id }`（[mutate.rs:693](../src/frame/nfs_server/src/mutate.rs#L693)，README 的“Named-object destination commits”一节）在服务端把 NamedStore 里的 FileObject 或 DirObject 解到目标路径，并检查路径 ACL、同名冲突和写租约。File Browser 的上传已经在用它：websdk 的 [`commitObject`](../../buckyos-websdk/src/nfsp.ts#L867)，调用处在 [transfers.ts:73](../src/frame/desktop/src/app/filebrowser/data/nfsp/transfers.ts#L73)。上一版说“没找到接口”，是我查错了。

**缺口在鉴权。**
- `commit_object` 只在调用者满足 `may_read` 时放行（[ndm/mod.rs:302](../src/frame/nfs_server/src/ndm/mod.rs#L302)）：要么调用者持有这个对象（`ndm_holdings`，按“用户 + App”记），要么调用者有 `obj://ndm/objects read`。后者只有 admin、frame、agent 三组有，普通 `users` 没有。
- 附件的读权限由 msg_center 按消息上下文判定（[buckets.rs:375](../src/frame/msg_center/src/buckets.rs#L375) `collect_attachment` 里的 `authorize_attachment`），nfs_server 不认这个上下文。
- 远端附件可能还没取到本 Zone（`ensure_remote_attachment`）。

**建议做法。**
1. msg_center 新增一步（暂名 `msg.prepare_save_attachment { obj_id, context_path }`）：
   - 按消息上下文鉴权；
   - 确保附件已经取到本 Zone（和 `collect_attachment` 的前半段相同）；
   - 用已有的 `bucket.grant_holdings { user_id, app_id, obj_ids }`（[ndm/mod.rs:277](../src/frame/nfs_server/src/ndm/mod.rs#L277)，workflow 已经在用）把这个对象授予调用者，也就是用户加上发起请求的 App。
2. Desktop 再用用户自己的会话调 `commit_object`。同名冲突、ACL 都由 nfs_server 处理。
3. HomeStation 的做法相同，由 HomeStation 服务做第 1 步。

**要注意的几点。**
- `grant_holdings` 现在要求 `obj://ndm/manage`。frame 组只有 `uploads`、`objects read`、`retain` 这几项（[rbac_config.rs:255](../src/kernel/buckyos-api/src/rbac_config.rs#L255)）。按 N3，拆出只能授予 holding 的 `obj://ndm/grant` 给 frame 组。
- `grant_holdings` 只覆盖列出的对象，不覆盖下层对象。所以保存文件夹附件里的某一个文件时，要授予那个内部文件的对象 ID。
- 目录导入要求目标名字没被占用，不能覆盖。`save` 模式对文件夹返回 `overwrite: true` 时，要请用户换个名字。
- Zone owner 在 admin 组，现在直接调 `commit_object` 就能通过 `may_read`。用 owner 测通，不代表家庭成员也能用；用例必须用普通用户跑。

也考虑过另外两种做法，都不建议：
- 让 `commit_object` 接受 `context`，再回调上下文所属的服务鉴权：更通用，但要新建一条跨服务的鉴权链路。
- 先收藏到收藏桶，再从桶条目提交：会多留一份收藏副本，而且 `commit_object` 还不接受桶条目作为来源。

## 6. 保留原生

| 位置 | 原因 |
|---|---|
| File Browser“上传文件 / 上传文件夹”（[FileBrowserView.tsx:556](../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx#L556)） | 语义就是把本机文件传进 Zone，是手机内容进入 Zone 的入口。Zone 之间用复制和移动（D11 已换） |
| MessageHub 访客页 | 没有 Zone 会话 |
| websdk `ndm.pickupAndImport` 的浏览器 provider（[ndm_client.ts:575](../../buckyos-websdk/src/ndm_client.ts#L575)），以及调用它的 sys_test（[web/index.ts:528](../src/apps/sys_test/web/index.ts#L528)） | 只作为“从本机上传成对象”的原语，不加走系统选择器的 provider。文档引导开发者默认用 `selectContent`（C5） |
| node_active | 激活前还没有 Zone；那里只有下载 `did.json`，没有选择框 |

## 7. 不在本 TODO：下载和导出（“另存为”方向）

下面这些会触发浏览器下载，不是选择框。按 §0，默认应该存进 Zone（`save` 模式返回 `parent`、`name`、`overwrite`，再用 NFSP `open_write` 写入），下载到本机作为次要选项。另立 TODO，优先级比原先估计的高：

- AI Workspace：备份 / 分享包（[shell/dialogs.tsx:248](../src/frame/desktop/src/app/aiworkspace/ui/shell/dialogs.tsx#L248)）、模板包（[SaveTemplateDialog.tsx:66](../src/frame/desktop/src/app/aiworkspace/ui/extensions/SaveTemplateDialog.tsx#L66)、[ExtensionsDialog.tsx:384](../src/frame/desktop/src/app/aiworkspace/ui/extensions/ExtensionsDialog.tsx#L384)）、draw.io 导出（[drawioExport.ts:113](../src/frame/desktop/src/app/aiworkspace/ui/canvas/drawioExport.ts#L113)）。扩展控件的源码和导出写在 home 下固定的目录里（[controlSources.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/controlSources.ts) 的 `homeControlDir`、`homeExportDir`），可以让用户自己选目录（`folder` 模式）。
- Task Center 新建下载：已经存进 Zone，但固定在 home 下的 Downloads，只能填文件名（[NewDownloadDialog.tsx](../src/frame/desktop/src/app/task-center/components/NewDownloadDialog.tsx)）。可以改用 `save` 模式让用户选位置。
- 文件预览、File Browser、“打开方式”里的下载（[ContentPreview.tsx:887](../src/frame/desktop/src/components/ContentPreview.tsx#L887)、[FileBrowserView.tsx:716](../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx#L716)、[OpenWithSheet.tsx:56](../src/frame/desktop/src/app/content/OpenWithSheet.tsx#L56)）：内容本来就在 Zone 里，这里的“下载”确实是要拿到本机，不用改。
- AI Center 规则导出、Playground 结果下载、Settings 的导出（`settings/components/shared/format.ts`）：管理工具，价值低。

## 8. 裁决

### 8.1 已裁决（owner，2026-11-10）

| 编号 | 问题 | 结果 |
|---|---|---|
| Q1 | 入口形态 | 按 §0：所有入口都拉起选择器，本机来源放在选择器里。MessageHub、HomeStation 也一样，`snapshot` 就绪后**替换**原生按钮，不再保留本机直达 |
| Q2 | 先后顺序 | C1–C5、S1–S3 → A1–A4、B1–B3（P1）→ A9 → A5–A8、B4–B5、O1–O3（P2）；`snapshot` 就绪后 D9、H1、D5、B6；V1 不依赖 `snapshot`，N3 定了就可以做 |
| Q3 | AI Workspace 自有源页面走路径 C（A9） | 做，放在 P1 之后 |
| Q4 | OpenDAN WebUI（O1–O3） | 接，和 Skills 页 SP2 改版一起做 |
| Q5 | HomeStation 发布框 | 和 D9 一样等 `snapshot`，不做浏览器中转；设计稿 §11 补编号 H1 |
| Q6 | 安装向导的两个按钮 | 合并成一个“选择安装包…” |
| Q7 | 工作流定义导入（B4）、AI Center 规则导入（B5） | 都换（P2）。B5 原先建议不换，因 §0 改为换 |
| Q8 | websdk `ndm.pickupAndImport` | 不加系统选择器 provider，只作为“从本机上传”；文档引导开发者默认用 `selectContent`（C5） |

### 8.2 按原则推出的新问题（owner，2026-11-10：按建议）

| 编号 | 问题 | 结果 |
|---|---|---|
| N1 | nfs_server `snapshot`（[nfs-server-file-objid-todo](nfs-server-file-objid-todo.md) 第二步）现在是“待实施”，排在哪 | 提到 P0，和本 TODO 的 C、S 项并行。最高频的两个入口——聊天附件（D9）和发动态（H1）——都卡在它上面 |
| N2 | 选择器的缩略图（S1）和搜索（S2）原本不在第一阶段 | 提前做，作为照片类入口（B1、B2、D9、H1）换过去的前提 |
| N3 | 附件存到 NFS（V1）的鉴权衔接：方向已确认，落盘用已有的 `commit_object`。剩下的问题是：按 §5.2 的 `grant_holdings` 方案做，就要给 msg_center、HomeStation 服务加 `obj://ndm/manage`。这项权限还包括删除、GC、任意 owner 的 pin，比需要的宽 | 按 `grant_holdings` 方案做。nfs_server 拆出一项只允许授予 holding 的权限（例如 `obj://ndm/grant`），只给 frame 组，不给 `manage`。要同步 `rbac_config.rs`、nfs_server `ndm/mod.rs` 开头的权限表和 `grant_holdings` 的检查（`require(RES_MANAGE)` 改为接受新权限）。现有调用方 workflow 是 kernel 组，本来就有 `obj://*`，不受影响 |
| N4 | `open` 模式在没有上次目录时从哪里开始 | 从“最近”开始（S3）。`folder`、`save` 模式仍从 home 开始 |

## 9. 验证

- 每个改动的入口在 mock 运行时下，主用例从 Zone 选（依赖 C3），补充用例从本机选。
- 真实 Zone 上至少各走一次从 Zone 选：Preview（B3）、头像（B1）、AI Workspace 的 PDF 导入（A1）。设计稿 §11.4 记着，选择器本身也还没在真实 Zone 上跑过，可以一并验证。
- **手机实机必测。** 目标用户在手机上，而 §11.4 记着只跑过桌面 Chromium。每批改完都要在 iOS Safari 和 Android Chrome 上走一遍：sheet 布局、返回键、软键盘、缩略图加载，以及路径 C 的浮层（A9、O1–O3）。
- 文档：设计稿 §5.2（C5）和 §11.4 同步；产品文档 `sysdlg/select` 一节不用改（它本来就写的是“内容主要来自 Zone”）。
- cmd_skills：C1、C2 是 Desktop 内部改动，不涉及 SDK API。C5 要改 `buckyos_app_dev`，按 AGENTS 的 cmd_skills 联动要求记录受影响的手册和验证方式。

## 10. 实施状态（2026-11-10，未提交）

除 §5.1 外全部实施。路径相对 `src/frame/desktop/`，另注明的除外。

### 10.1 公共部分与选择器

| 编号 | 落点 | 说明 |
|---|---|---|
| C1 | `src/sysdlg/select/pickFiles.ts` 的 `usePickFiles(callerId)` | 返回 `({ accept, multiple, folder, title, signal }) => Promise<File[] \| null>`。**和原计划不同：hook 永远不为 null。** `src/sysdlg/index.tsx` 的 `useSelectContent()` 在没有窗口对话框层时走路径 C（`openSelectEmbed`），A9 因此并进了 C1。系统入口是 `${protocol}//${getZoneHostName()}`；mock 下是 `localhost` 同端口（`aiws-<id>.localhost` 去掉第一级） |
| C2 | 同上，`acceptToSelectors` | MIME 和扩展名拆成两个选择器（带 `mime` 的选择器不会退回比较 `ext`） |
| C3 | `src/app/filebrowser/mock/content.ts`、`thumbnail.ts` | 用例用 `window.__mockZoneFiles.put(path, base64, mime?)` 挂夹具（mock 启动时在 `main.tsx`、`aiws.tsx` 暴露）；种子图片读出来是生成的 PNG，其余是一行文本；mock 上传保留上传的字节；mock 图片带 SVG 缩略图 |
| C4 | `tests/e2e/fixtures/selectDialog.ts` | 新增 `pickZone`、`pickZoneFolder`、`pickLocal`、`putZoneFile`、`selectEmbedFrame`。AI Workspace 的用例跑在 zh-CN 下，`selectGoto` 写死了 “My files”，所以它在 `tests/aiworkspace/fixtures.ts` 里另有一套（`chooseFromZone` 等） |
| C5 | 设计稿 §5.1、§5.2、§11（D9 改为替换、补 H1）；`doc/sdk/context ext.md`；`buckyos_app_dev` 手册 | 手册补了“不放原生选择框、`allowLocal` 让本机成为选择器里的来源、对象 ID 用 `ensureObjId`（从 `buckyos` 导出）、`ndm.pickupAndImport` 只用于从本机上传”。手册是纯文字改动，核对了 `ensureObjId` 的导出位置 |
| S1 | `SelectDialog.tsx` 的 `Thumb` | 列表行首 28px 缩略图；`accept` 全是图片或视频时改为网格（`data-layout="grid"`，格子仍是 `select-row`）。缩略图优先用列表自带的 `thumbnailUrl`，没有时用 Preview Pipeline（`usePreviewThumbnail`，`prefer: 'pipeline'`，与 HomeStation 的 feed 相同），格子进入视野附近才请求。**真实 nfs_server 的列表从不带 `thumbnail` 访问地址**，所以真实环境全部走 Pipeline |
| S2 | 同上 | 搜索框在列表上方，复用 `useSearch`。**范围是用户能读的全部文件，不限当前目录**（手机上更常用，结果行带所在目录）。`folder` 模式只显示目录；点结果里的目录进入并退出搜索；`save` 模式点结果里的文件，进入它的目录并填文件名；Esc / 返回键先清空搜索 |
| S3 | 同上 `resolveStart` | `open` 没有 `start`、没有上次目录，或给的 `start` 不存在时，从“最近”开始；`folder`、`save` 仍从 home 开始 |

另修了一个触屏问题（B 组用例发现）：点目录后在同一位置点文件，浏览器会合成一次 `dblclick`，选择器把它当成“确认”提前关闭。现在只有两次点击落在同一项上才算双击确认。

### 10.2 各入口

| 编号 | 状态 | 偏差与说明 |
|---|---|---|
| A1–A8 | 已换 | 统一经 `app/aiworkspace/ui/shell/filePicker.ts`（包一层 `usePickFiles('aiworkspace')` 并报错）；`importPicker.ts` 已删，AI Workspace 里不再有原生文件输入。A3 的块操作不在 React 里，给 `RenderContext` 加了 `pickFile`。InsertCatalog 的 MUI 焦点陷阱会把焦点从选择器里抢走，选文件期间传 `disableEnforceFocus` |
| A9 | 已换 | 由 C1 的路径 C 覆盖，原生回退全部删除 |
| B1 | 已换 | `ImagePicker` 的 `inputTestId` / `inputLabel` 改为 `chooseTestId`，新增 `onPickingChange`。自己头像的对话框是 MUI `Dialog`，会盖住选择器，选文件期间先隐藏（`keepMounted`，草稿保留） |
| B2–B5 | 已换 | B3 的 Zone 条目直接用 `cyfs-path`，多选组成 list 会话；B4 的 accept 是 `.json,.dsl,.txt` |
| O1–O3 | 已换 | `src/frame/opendan/web/src/pickFile.ts`。WebUI 原来不回应 App Frame 握手，现在在页面加载时建立。路径 C 的系统入口取 profile 的 `desktop_url`（WebUI 把 SDK 的 Zone 主机设成了自己的主机）。没有 Zone（`--dev`）或 Shell 回 `unsupported` 时退回一次性的原生输入。**`buckyos` 依赖从 npm `^0.7.137` 改成 `git+…buckyos-websdk#main`**（npm 上的版本没有 `selectContent`），与 Desktop、Text Editor 一致，待 owner 确认。没有等 SP2，只动了这三个入口 |
| V1 | 已做 | 见 10.3 |
| D9、H1、D5、B6 | 未做 | 等 `snapshot` |

内存（§3.1 的问题）：PDF、Office、draw.io、表格导入对本机文件本来就整读（`file.arrayBuffer()`），从 Zone 选不增加峰值。只有资源导入（经 ndm，单文件上限 32 MiB）、工作区包（A7）和 `.pikg`（A8）原来是惰性读取，从 Zone 选时会先整个读进内存。

### 10.3 V1：收到的附件存到用户的 NFS

- nfs_server：新权限 `obj://ndm/grant`（write），只放行 `bucket.grant_holdings`；`grant_holdings` 接受它或 `obj://ndm/manage`。`rbac_config.rs` 给 frame 组加 `obj://ndm/grant`（msg-center、homestation 都在 frame 组），frame 仍没有 `manage`。
- msg_center：`msg.prepare_save_attachment { obj_id, context_path, inner_path? }` → `{ obj_id, kind: "file" | "dir" }`。鉴权与取回同 `collect_attachment`，然后把对象授予“调用者 + 调用 App”（`CallerIdentity` 新增 `app_key`，即 token target 的 `canonical_key()`，与 nfs_server 的 `Principal.app` 同源）。**和 §5.2 不同：文件夹附件里的文件由客户端传根附件 ID 加 `inner_path`**，服务端按消息鉴权根对象后逐级解析，只授予解析到的那个对象；单独的内部对象 ID 没法鉴权。非 Zone 用户（External、受限、App 会话）拒绝。
- HomeStation：`POST /home/<user>/objects/<cyfile|cydir id>/save`，受众检查同 GET，授予后返回 `{ obj_id, kind }`。每个对象本来就有自己的 `object_grants`，文件夹里的文件按自己的 ID 保存。
- Desktop：`src/components/saveToZone.ts` 的 `useSaveToZone`：`save` 模式选位置 → 授予 → 用户自己的会话 `commitObject(parent, name, obj_id, { overwrite })`。文件的覆盖由 `commit_object` 处理；文件夹遇到同名（选择时或提交时）重新拉起选择器请用户换名。MessageHub 文件卡片、缩略图、内联图片、文件夹及其中每个文件，HomeStation 的附件都有“存到我的文件”；`showDirectoryPicker` 路径已删，“下载”在触屏设备上隐藏。mock 下授予照常调用，落盘写进 mock Zone。
- 文档：nfs_server README、`product/bucky_file/nfs_server.md`、`doc/message_hub/Message Center.md`、MessageHub `UI_DATAMODEL.md`、`doc/homestation/HomeStation 协议与实现.md`。
- cmd_skills：不用改。`buckyos_admin` 只指向 `rbac_config.rs` 和 `system/rbac/policy`；`aigc_*` 讲的是 Agent 按对象 ID 读附件，没变，新方法本来就拒绝非 User 会话。

### 10.4 验证

- Rust：`cargo test -- --test-threads=1` 跑了 nfs_server、buckyos-api（397 通过）、msg_center（238 通过）、homestation，全部通过。新增：frame 服务能授予但不能 `remove_object`；文件、文件夹、文件夹内文件各自只授予自己；`../`、不存在的路径、不在消息里的对象、访客被拒；远端附件先取回再授予；HomeStation 的受众、未登录、他 Zone 读者、非文件。
- Rust：`cargo check --workspace --tests` 通过。
- Desktop：`tsc -b`、改动文件的 eslint 通过。
  - 默认配置全量 e2e（重启 dev server 后）：510 通过、22 失败、48 跳过。失败的 22 条：`ndm-upload-endpoint.spec.ts` 11 条（websdk 构建缺 `ndn` 导出、mock 不认 `object.upload_target`，与本次无关）、`ai-center.spec.ts:4` 和 `:119`、`desktop.spec.ts:330`、`messagehub-agent-task:575`、`messagehub-group:309`、`messagehub-media:173`、`messagehub.spec.ts:764`、`messagehub-actions:87`，都是改动前就记录过的失败或并行偶发；另外 `messagehub-agent-task:432`、`messagehub.spec.ts:59`、`object-presentation.spec.ts:81` 串行重跑通过（连同 `messagehub-upload`、`homestation` 两个 spec，38 条全过）。
  - AI Workspace（`playwright.aiworkspace.config.ts`，release 后端重新构建）：各导入 spec 与新的 `file-picker.spec.ts` 37 通过，2 条重跑通过（FP-02 是用例自身的问题，已改；DR-07 失败在粘贴文本导入，与本次无关）；features + offline 23 条全过，含自有源页面的路径 C。AI Workspace i18n 守卫 18 通过、2 失败，都是改动前就有的。
- OpenDAN WebUI：check、lint、build 通过；e2e 35 条中 34 条通过，失败的 `home.spec.ts:135` 是工作区对话框的 Runtime 字段改成了 `<select>`，与本次无关。新用例覆盖路径 B（假 Shell）、路径 C（假选择器页和假的 Zone 文件服务）。

**未验证：**
- 真实 Zone 一次都没跑：路径 B / C、Zone 文件的读取、Pipeline 缩略图、真实 nfs_server 的搜索、V1 全链路。V1 必须用**普通家庭成员**测（owner 在 admin 组，`may_read` 本来就能过）。
- 手机实机（iOS Safari、Android Chrome）都没测，只跑了桌面 Chromium 和它的手机模拟。
- DV 用例（`tests/dv/aiworkspace.*`、`tests/dv/object-presentation.spec.ts`）改了没跑；`pdf-fidelity` 需要 `AIWS_PDF_FILE` 和 `NFS_BIN`，没跑。
- A5 课堂名单导入没有 e2e（需要在线的课堂会话）；`.pikg` 从 Zone 导入只验到选择器，没有走完安装。
- V1：远端的文件夹附件 msg_center 本来就不支持；HomeStation 按需取回只对文件，关注的人发的文件夹必须已经完整在本 Zone。
- OpenDAN WebUI 上线要重建 paios aios 镜像。
