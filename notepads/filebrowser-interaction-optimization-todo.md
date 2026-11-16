# Files 交互优化 TODO

状态：待实施，供后续 Code Agent 执行。owner 已于 2026-11-16 答复 §5 的 Q1–Q12（原话保留在 §5），没有待确认项。第二批按 BuckyOS sudo 方案实施，可以和第一批一起做。

日期：2026-11-16。源码基线：buckyos `3036f52fb`，工作区另有与本任务无关的改动（Agent 浏览器等），不要动它们。

来源：owner 的“优化 File 交互”想法（原文见 §1.1）和 §5 的答复。本文件只记录任务；写 TODO 时没有改功能代码，也没有跑测试。

## 1. 目标与范围

Files 打开即落在当前用户自己的“我的文件”；侧边栏更干净：入口带图标，文件夹树默认收起，显示的内容以 nfs_server 返回的为准；图标模式能看到图片 / 视频 / PDF 等的缩略图，并可调大小；各类文件有可区分的图标。在 BuckyOS Desktop Lite 上，owner 通过 BuckyOS 的 sudo（系统公共的密码对话框）解锁后，可以像浏览另一棵目录树一样浏览这台电脑的全部目录。

按 owner 的产品原则：目标用户主要用手机，Zone 是主文件系统。每一项都要在手机宽度下可用，并在手机宽度下验收；不能只在桌面宽度下验收。

不在本轮范围：Files 以外 App 的布局调整；完整版（非 Lite）和跨节点的设备 / referral reader（[UI_DATAMODEL.md](../src/frame/desktop/src/app/filebrowser/UI_DATAMODEL.md) §9 第 10 项仍保留为远期）；电脑目录上的 ACL、分享、收藏集、AI 元数据。

### 1.1 owner 原文

> 最前面的 我的文件 最近使用 增加图标，首次使用并默认定位在我的文件（现在定位到 cyfs://home，应该是 cyfs://home/$current_user）
> 删除默认打开的 pictures tab
> 下面的文件夹默认都是收起的，非管理员只能看到“我的文件”
>
> 文件根据类型不同应该有不同的图标
> 图片/视频/pdf 等有缩略图的格式，应该在图标模式下显示缩略图，当选择图标模式的时候，可以选择图标的大小
>
> 设备分类只在 Desktop 版本上可见，点击开启后要求用户输入 sudo 密码，随后会展开当前 PC 的所有目录树。

### 1.2 对原文的理解（已按 §5 答复修正，实施以此为准）

| 原文 | 落地含义 |
| --- | --- |
| cyfs://home/$current_user | Files 内部 URL 是 `dfs:///home/<user_id>`，复制路径得到 `cyfs:///home/<user_id>`。`user_id` 取 NFSP `hello` 返回的 `identity.user_id`，不从页面别处猜 |
| 非管理员只能看到“我的文件” | 前端不按角色过滤，“nfs 给什么看什么”（Q8）。普通用户在 nfs_server 的 ACL 下本来就只能列出自己的 home，以及别人明确授权给他的目录 |
| 下面的文件夹默认都是收起的 | 只是初始 UI 状态：首次打开时整棵树收起，不能“全展开”（Q9） |
| Desktop 版本 | 只在 BuckyOS Desktop Lite 上有效（Q1），即产品 profile 的 `edition = lite`。Lite 只有一个用户，就是 admin（Q5） |
| sudo 密码 | BuckyOS 自己的 sudo：verify-hub `sudo_by_password` 加公共密码对话框，只当作门槛（Q2、Q3）。不是操作系统的 sudo，nfs_server 不接触任何密码。这个用途签发 30 分钟的 token（Q11） |
| 当前 PC 的所有目录树 | 运行 Lite 节点的这台电脑的文件系统，从 `/`（Windows 为各盘符）开始按需展开。“体验上就是另外一个目录树”（Q6）：交互与 DFS 目录树一致，实际能做什么取决于节点进程所用系统账户的权限。Lite 只有 Windows / macOS 版，节点以登录用户运行（Q12） |
| 设备文件的路径 | 复制路径得到标准的 `file:///` URI，例如 `file:///Users/alice/a.txt`、`file:///C:/Users/alice/a.txt`；不出现内部的 `device://` |

## 2. 当前实现与缺口

| 环节 | 当前实现 | 缺口 / 入口 |
| --- | --- | --- |
| 默认位置 | `DEFAULT_TABS` 有两个 tab：Home `/home` 和 Pictures `/home/Pictures`（[FileBrowserView.tsx:94](../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx#L94)）；新 tab（:219）、pane 兜底（[useBrowserPane.ts:87](../src/frame/desktop/src/app/filebrowser/data/useBrowserPane.ts#L87)）、上传选择起点（FileBrowserView.tsx:671）都写死 `/home` | 真实 Zone 里 `/home` 是所有用户 home 的父目录。要改成“我的 home”，并且只有一个 tab |
| 当前用户 | nfs_server `op_hello` 返回 `identity: {user_id, role}`，并在 hello 时 `ensure_home` 建好 `/home/<user>`（[server.rs:249](../src/frame/nfs_server/src/server.rs#L249)、[access.rs:214](../src/frame/nfs_server/src/access.rs#L214)）；前端 `ensureSession()` 拿到了，但只用来做缓存键（[nfs_browser_client.ts:325](../src/frame/desktop/src/api/nfs_browser_client.ts#L325)） | 暴露成 Files 数据层的“我的 home”，走 registry，组件里不能 import NFSP |
| 顶部入口 | `favorites` 里“我的文件”“最近使用”是纯文字按钮，没有图标和选中态，“我的文件”跳 `/home`；钉住的目录用 `★` 文本（[Sidebar.tsx:287](../src/frame/desktop/src/app/filebrowser/Sidebar.tsx#L287)） | 加图标和选中态，跳到我的 home |
| 文件夹树 | 根节点只要没被手动收起过就展开，当前位置的祖先也会展开（[Sidebar.tsx:75](../src/frame/desktop/src/app/filebrowser/Sidebar.tsx#L75)），所以首次打开是“全展开”，这是代码行为，不只是本地缓存的问题。`loadDfsRoots` 列出所有根，并为每个根预取一层子目录，所以树最多两层（[nfsp/sidebar.ts:24-60](../src/frame/desktop/src/app/filebrowser/data/nfsp/sidebar.ts#L24-L60)） | 首次打开全部收起；展开时再加载，可以任意深 |
| 设备分区 | 总是显示“设备”；没开高级模式时显示“启用高级模式”卡片，而高级模式只是一个前端状态；NFSP 的 devices 源恒为空；mock 里有假设备和 `view://demo/*` 诊断入口（[Sidebar.tsx:350](../src/frame/desktop/src/app/filebrowser/Sidebar.tsx#L350)）。NFSP 协议有 `at.realm`，但服务端只认 `dfs`（[namespace.rs:158](../src/frame/nfs_server/src/namespace.rs#L158)），hello 只通告 `realms: [dfs]`（server.rs:273） | 整套重做：只在 Lite 上、只对 owner 显示；用 BuckyOS sudo 解锁；后端新增 host realm |
| BuckyOS sudo | 公共对话框 `useSudoByPassword()`（[sudo.tsx](../src/frame/desktop/src/components/sudo.tsx)）调 verify-hub `sudo_by_password`，返回带 `sudo` 声明的 session token，受限账户不能申请。有效期写死为 3 分钟（[verify_hub main.rs:38](../src/kernel/verify_hub/src/main.rs#L38)、:1725）。请求结构 `SudoByPasswordRequest` 是 `deny_unknown_fields`，没有时长参数（[verify_hub_client.rs:199](../src/kernel/buckyos-api/src/verify_hub_client.rs#L199)）。websdk 也直接调用（`buckyos-websdk/src/verify-hub-client.ts:150`）。对话框的提示文字 `sudo.tokenHint` 写死“约 3 分钟”。现成用法：Storage 页以 `aud: 'nfs-server'` 申请，再把 token 交给 nfs_server 删除 bucket（[StoragePage.tsx:75](../src/frame/desktop/src/app/settings/pages/StoragePage.tsx#L75)）。nfs_server 从 Bearer 头或 cookie 取 token（[auth.rs](../src/frame/nfs_server/src/auth.rs)），`verify_nfs_session_token` 会带出 `sudo`，但不检查 `aud`（[nfs_copy.rs:111](../src/kernel/buckyos-api/src/nfs_copy.rs#L111)） | host realm 照这个模式做；要补 `aud` 校验 |
| Lite 判定 | `ProductProfile`（[product.rs](../src/kernel/buckyos-api/src/product.rs)）来自 `etc/product.json`，由 control-panel 发布到 `system/product`，并提供 `is_lite()`；前端已有 `fetchProductInfo` / `useProductInfo` | nfs_server 目前不读产品 profile |
| 文件图标 | `kindIcon(kind)` 只分 8 种 `FileKind`（[fileDisplay.tsx](../src/frame/desktop/src/app/filebrowser/fileDisplay.tsx)）；`FileKind` 由扩展名粗分，pdf / doc / xls / ppt / md / txt 都算 `document`（[fileKinds.ts](../src/frame/desktop/src/app/filebrowser/data/fileKinds.ts)）。Files、选择器、桌面图标、Users & Agents 都在用 `kindIcon` | 按更细的类型出图标；`FileKind` 本身还用于搜索过滤，不改它的语义 |
| 缩略图 | Files 图标模式只认列表里的 `thumbnailUrl`（[MainContent.tsx:409](../src/frame/desktop/src/app/filebrowser/MainContent.tsx#L409)），而 nfs_server 的 `access_urls` 没有 `thumbnail` 这一种，所以真实 Zone 上永远只显示类型图标。选择器已经用 `usePreviewThumbnail` 按需取缩略图，并用 IntersectionObserver 懒加载（[SelectDialog.tsx:66-91](../src/frame/desktop/src/sysdlg/select/SelectDialog.tsx#L66-L91)），但只取图片和视频 | 抽出共用组件给 Files 图标模式用，并扩到 PDF 和文档 |
| 缩略图后端 | `repr` 的 `purpose: thumbnail` 已支持：图片缩放、视频取帧、PDF 首页（poppler）、Office / iWork / epub 内嵌缩略图、已缓存的 `preview-document` PDF；不会为缩略图启动 LibreOffice；音频和其它 markup 明确返回 `unsupported`（[rpc.rs:180](../src/frame/nfs_server/src/preview/rpc.rs#L180)、[preview/mod.rs](../src/frame/nfs_server/src/preview/mod.rs)） | DFS 侧后端不用改。前端 [thumbnail.ts](../src/frame/desktop/src/components/preview/thumbnail.ts) 不缓存失败，同一个 `unsupported` 文件每次滚回来都会重新请求 |
| 图标大小 | 网格单元固定 `GRID_CELL_WIDTH = 172`、行高 160，图标框 64×64（[MainContent.tsx:634](../src/frame/desktop/src/app/filebrowser/MainContent.tsx#L634)、:808）；视图模式存在 `files.view`（useBrowserPane.ts:28/242）；切换入口在 TopBar 视图菜单（[TopBar.tsx:646](../src/frame/desktop/src/app/filebrowser/TopBar.tsx#L646)）和菜单 registry（[registry.ts:375](../src/frame/desktop/src/app/filebrowser/menu/registry.ts#L375)） | 加大小档位并持久化，网格尺寸随档位变 |
| 节点进程身份 | 桌面形态下 node_daemon 以登录用户身份运行，不是 root（[node_executor_host.rs:169](../src/kernel/node_daemon/src/node_executor_host.rs#L169)，macOS 用 LaunchAgent）。Lite 只有 Windows / macOS 版（Q12） | host realm 的实际权限就是这个登录用户的权限 |
| 复制路径 | `copyPathText` 把 DFS 路径转成 `cyfs:///…`，其它 scheme 原样复制（[urls.ts](../src/frame/desktop/src/app/filebrowser/data/urls.ts)）；`displayPath` 对非 DFS 的 URL 原样显示。调用方有状态栏、预览面板、多选复制（[FileBrowserView.tsx:885](../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx#L885)） | 设备 URL 要转成 `file:///`，否则会复制出、显示出内部的 `device://` |

## 3. 实施任务

### 第一批：前端

#### T1. “我的文件”定位到 `/home/<当前用户>`

- [ ] 在 Files 数据层加一个 home 提供者，沿用现有 registry 模式，由 [install.ts](../src/frame/desktop/src/app/filebrowser/data/install.ts) 注册，例如 `myHome(): Promise<{ path: string; userId: string | null }>` 加对应的 hook。
  - NFSP：`await ensureSession()` 后读 `identity`。有 `user_id` 时为 `/home/<user_id>`；没有时（单机无鉴权、系统身份）为 `/home`。
  - Mock：`/home`。现有 mock 夹具把 `/home` 当作用户 home，`/home/Pictures` 等测试依赖它。
- [ ] 改掉所有写死的 `/home`：`DEFAULT_TABS`、`handleNewTab`、`useBrowserPane` 的兜底、侧边栏“我的文件”、上传选择的起点（FileBrowserView.tsx:671），以及选择器的 `HOME`（[SelectDialog.tsx:25](../src/frame/desktop/src/sysdlg/select/SelectDialog.tsx#L25)）。选择器与 Files 共用侧边栏数据源，两边必须一致。
- [ ] 首次打开（没有 `openRequest`）落在我的 home；有 `openRequest` 时以它为准。`useBrowserPane` 只在首次渲染读 `initialTabs`，所以要么等 home 解析完再挂载 pane（期间显示骨架），要么先建 tab 再用 replace 方式改位置。两种做法都可以，但不能先闪一下 `/home` 的列表，也不能多出一条后退历史。
- [ ] 位于我的 home 时 tab 标题显示“我的文件”（用已有的 i18n `filebrowser.sidebar.myFiles`），不显示 user_id。面包屑保持真实路径。

#### T2. 顶部入口加图标，去掉默认的 Pictures tab

- [ ] “我的文件”“最近使用”改用 `navButton` 的样式，各加一个图标：“我的文件”用表示“我的”的图标（例如 `House`），“最近使用”用与“视图”分区一致的 `Clock`。两者都要有选中态，当前位置在我的 home 或 `view://recent` 时高亮。钉住的目录把 `★` 文本换成 `Star` 图标。桌面布局和手机 compact 布局用同一块。
- [ ] `DEFAULT_TABS` 只保留“我的文件”一个 tab。
- [ ] 不改 AI 主题分区里的“最近使用”。两处都保留（Q10），后续 AI 主题会基于最近文件做分析。

#### T3. 文件夹树首次打开全部收起，展开时再加载

- [ ] 首次打开时所有层级都收起，包括根节点，以及默认位置（我的 home）的祖先。展开状态只记用户手动展开的：`files.tree:<url>` 为 `'true'` 才展开。现在的判断是“不为 `'false'` 就展开”，要反过来。之后在主区域进入某个目录时，仍可以展开它的祖先来显示当前位置（Q9：“只是默认 UI 状态”）。AI 主题、收藏集分组保持现状。
- [ ] 树改成展开时才加载子目录：
  - 只列 `dir`，按名称排序，复用 `effectiveListLimit`；超过一页时显示“更多…”，不能静默截断；
  - 结果在会话内缓存；列出来为空就去掉展开箭头；
  - 单个节点出错时只在该节点上显示错误和重试，复用 `SectionError` 的样式；
  - 去掉 `loadDfsRoots` 为每个根预取一层子目录的逻辑。
- [ ] 树组件不绑定 DFS：接受 `loadChildren(url)`，T11 的电脑目录树直接复用。

#### T4. 侧边栏显示的内容以 nfs_server 为准（不按角色过滤）

- [ ] 前端不按管理员 / 普通用户过滤根目录或子目录，侧边栏、地址栏、选择器都显示 nfs_server 列出的内容（Q8）。普通用户在 ACL 下通常只看到 `home` → 自己的目录，以及别人授权给他的目录。
- [ ] 用一个普通用户在真实 Zone 上确认这一点。如果发现普通用户能列出不该看到的根，那是 nfs_server ACL 的问题，在 nfs_server 修，不在前端藏。

#### T5. 按文件类型区分图标

- [ ] 在 [fileDisplay.tsx](../src/frame/desktop/src/app/filebrowser/fileDisplay.tsx) 加 `fileIcon(entry: { kind; name; mime? }, size)`，按扩展名 / MIME 细分，至少包括：
  - 文件夹、图片、视频、音频、PDF；
  - 文字文档（doc/docx/odt/rtf/pages）、表格（xls/xlsx/ods/csv/numbers）、演示（ppt/pptx/odp/key）；
  - 纯文本 / 日志、Markdown、代码、压缩包、电子书（epub/mobi）、Visio / 绘图（vsd/vsdx）；
  - 安装包 / 可执行文件（exe/msi/dmg/pkg/apk/deb）；
  - 其余为通用文件。

  MIME 能用 `buckyos/content` 的 `mediaTypeFromExtension` 就复用，Office 家族用一张小表补。
- [ ] 颜色只用主题 token（`--cp-*`），浅色和深色主题下都要能辨认。图标用 lucide 现有的（`FileSpreadsheet`、`Presentation`、`FileArchive`、`FileCode`、`BookOpen` 等），不引入新图标库。
- [ ] `FileKind` 不变，它是搜索过滤用的分类。逐个检查 `kindIcon` 的调用方：Files 列表 / 网格 / 手机列表 / 预览面板 / 搜索结果、选择器、桌面上的文件图标（[DesktopRoute.tsx](../src/frame/desktop/src/desktop/DesktopRoute.tsx)）、[EntityAvatar.tsx](../src/frame/desktop/src/app/users-agents/components/shared/EntityAvatar.tsx)。手里有文件名的换成 `fileIcon`，只有 kind 的保留 `kindIcon`。

#### T6. 图标模式显示缩略图

- [ ] 把选择器的 `Thumb` 抽成共用组件（例如 `components/preview/FileThumb.tsx`），Files 图标网格和选择器都用它：
  - 列表自带 `thumbnailUrl` 时优先用；
  - 否则在接近视口时（IntersectionObserver，`rootMargin` 约 200px）调 `usePreviewThumbnail(source, { …, prefer: 'pipeline' })`；
  - 没拿到或失败时显示 T5 的类型图标。

  `source` 由调用方给出：DFS 条目是 `cyfs-path`，电脑上的条目用 T10 定的来源。
- [ ] 请求缩略图的类型：图片、视频、PDF，以及后端可能给出缩略图的文档（Office / iWork / epub；后端只取内嵌图或已缓存的 PDF，不会触发转换）。音频、文件夹和其它类型不请求。只在图标模式请求；列表模式（桌面行 16px、手机行 48px）保持类型图标。
- [ ] 负缓存：`UNSUPPORTED`（以及“没有内嵌缩略图”这类确定性失败）按 `path + modifiedAt` 在会话内记下，不再请求；超时、网络错误不记。可以放在共用组件里，也可以放在 `thumbnail.ts` 里只针对 `UNSUPPORTED`，但不能影响 Preview 自身的重试。
- [ ] 显示：缩略图 `object-cover` 填满图标框。视频叠一个小播放标记，PDF / 文档在角上叠一个小类型标记，让用户仍能分辨类型。引用项的 `LinkBadge`、剪切态的半透明、选中边框保持不变。
- [ ] 请求尺寸跟随 T7 的图标大小（`thumbnail.ts` 会按 DPR 分到 256 / 512 / 1024 三档）。只对虚拟网格实际渲染的单元请求，组件卸载时取消（`usePreviewThumbnail` 已在 effect 清理时 abort）。前端不要额外扇出并发。在几百个视频的大目录里滚动时，观察服务端预览并发槽和 CPU，把结果写进实施记录。

#### T7. 图标模式可选图标大小

- [ ] 三档：小 / 中 / 大，默认中。
  - 桌面按单元宽度定档，建议约 120 / 168 / 232px；图标框随之放大，建议约 72 / 112 / 176px（现在的 64px 偏小，放不下有意义的缩略图）。
  - 手机按列数定档，建议 4 / 3 / 2 列。

  具体数值以实际效果为准，写进实施记录。
- [ ] 入口：
  - 桌面：TopBar 视图菜单在“图标视图”下加大小单选，只在图标模式时可用；
  - 手机：走 menu registry / `MobileMenuSheet`，与“以图标查看”放在一处。

  设置存在 `files.iconSize`，与 `files.view` 一样是全局设置。
- [ ] `IconGridView` 的 `GRID_CELL_WIDTH` / `GRID_ROW_HEIGHT` 改为由档位计算。切换档位后虚拟列表要重新测量，`revealIndex` 的滚动定位和骨架单元的尺寸要同步。

### 第二批：设备分区（Lite 上用 BuckyOS sudo 解锁电脑目录树）

#### T8. 后端：在 Lite 上通告 host realm

- [ ] nfs_server 读产品 profile（buckyos-api 的 `ProductProfile`，与 control-panel 和 node_daemon 用同一个来源），只在 `is_lite()` 时提供 host realm。不另写推断逻辑；前端也不根据 UA、窗口宽度自己猜。
- [ ] `hello` 的 `realms` 在“Lite 且 actor 是 Zone owner（`Actor::Admin(Some(_))`）”时追加 `{"id": "host", "label": "<电脑名>"}`，其余情况不出现。前端只凭这一项决定是否显示“设备”分区。

#### T9. 后端：host realm 只认 BuckyOS sudo token

- [ ] host realm 上的每个请求都必须带 BuckyOS sudo token，并满足以下全部条件：
  - token 由 verify-hub 签发且未过期；
  - 带 `sudo` 声明；
  - principal 是 User，且是 Zone owner；
  - `aud` 为 `nfs-server`。

  `verify_nfs_session_token` 现在不返回也不检查 `aud`，要补上：加到 `NfsSessionClaims`，在 host realm 校验。
- [ ] token 的传递：参照 Storage 页的做法，前端拿到 sudo token 后，用它作为另一个 NFSP 客户端的 Bearer 并单独 `hello`，得到一个 sudo 会话；host realm 的请求只走这个会话。这个会话不能比 token 活得更久：nfs_server 记下 token 的 `exp`，过期后 host realm 请求一律返回明确的 `SUDO_REQUIRED` 错误码。普通 DFS 会话不能访问 host realm。
- [ ] 有效期跟随 sudo token（Q4），这个用途签发 30 分钟（Q11），nfs_server 不另设计时。长操作只在开始时检查授权：已经开始的下载、已经提交的复制任务跑完为止，不中途掐断。
- [ ] verify-hub 支持按请求指定 sudo token 时长：
  - `sudo_by_password` 增加可选参数（例如 `expires_in_secs`）。不传时仍为 3 分钟，现有调用方行为不变；上限 30 分钟，超过上限按上限签发（或拒绝，二选一，写进实施记录）。
  - 同步修改 buckyos-api 的 `SudoByPasswordRequest` 与 Rust 客户端、websdk 的 `verify-hub-client.ts`，以及 `sudo.tsx` 的 `SudoDialogOptions`（新增时长选项，传给请求）。
  - 对话框的 `sudo.tokenHint` 改为按实际过期时间显示（中英文都改），不再写死“约 3 分钟”。
  - verify-hub 测试覆盖：默认 3 分钟、30 分钟、超过上限。
- [ ] nfs_server 不接触密码，也不自己做口令重试限制，这些由 verify-hub 负责。host realm 的修改类操作要写审计日志（谁、何时、什么操作、哪个路径）。

#### T10. 后端：host realm 的文件操作

“体验上就是另外一个目录树”（Q6）：在电脑目录上的操作与 DFS 目录一致。实际权限是节点进程所用系统账户的权限，不提权，也不启动 root helper（Q2）。

- [ ] 读取：`list` / `stat` / `read`（含 reader 用到的分页）、下载、预览、缩略图（`repr` 接受 `at.realm = "host"`）。
  - 路径从电脑根开始：Unix 为 `/`；Windows 为一个虚拟根，列出各盘符。映射方式由实现决定，写进 [nfs_server.md](../product/bucky_file/nfs_server.md)。
  - 这个账户读不了的目录和文件返回 `PERMISSION_DENIED`，列表里显示为带锁、不可进入。
  - 设备文件、FIFO、socket 不可读，沿用现有规则；`..` 和路径规范化要有测试；符号链接照常显示，进入时按目标解析。
- [ ] 修改：新建目录、改名、删除、上传、覆盖写，由系统权限决定能否成功，失败如实映射成现有错误码。
- [ ] 跨 realm 复制 / 移动：电脑目录与 DFS 之间可以互相复制、移动（例如“复制到我的文件”），复用现有复制任务机制（[copy.rs](../src/frame/nfs_server/src/copy.rs)）。跨 realm 的移动 = 复制 + 删除源，失败时不删除源。
- [ ] 不适用 DFS 专属能力：不写 ACL、filedb、meta、recent，不建搜索索引，不支持分享链接和收藏集。watch 不是必需的，可以只提供手动刷新。
- [ ] 预览来源：电脑上的文件不能伪装成 `cyfs://` DFS 路径。需要给 Preview 一个能带上 realm 的来源（新的 ContentRef 种类，或者让 nfspProvider 识别 host realm），并让它走 sudo 会话。选新增代码最少的一种，写进实施记录。

#### T11. 前端：设备分区、sudo 解锁、电脑目录树

- [ ] 删除“高级模式”：`advancedMode` / `onToggleAdvanced`、“启用高级模式”卡片，以及 `DFS · Logical view` 的标签切换（分区标题固定为“文件夹”）。mock 的 `view://demo/*` 诊断入口只在 mock 运行时保留，供测试使用。
- [ ] “设备”分区只在 `hello.realms` 含 `host` 时显示；手机 compact 布局的分区按钮同理。未解锁时显示一行“<电脑名>”加锁图标。点击后调用公共对话框 `useSudoByPassword({ aud: 'nfs-server', title, reason, 时长: 30 分钟 })`，`reason` 写明“查看这台电脑上的全部目录”。不另做密码框。
- [ ] sudo token 只放在当前 Files 窗口的内存里，不写 localStorage / sessionStorage，也不与其他窗口共享。关闭窗口时丢弃 token 和 sudo 会话，立即失效（Q4）。
- [ ] 解锁后展开电脑的目录树，复用 T3 的树组件，从根开始按需加载。主区域用新注册的 reader（`device://<node>/<path>`；UI_DATAMODEL 已预留 `device://`），与 DFS 用同一套列表、网格、缩略图、菜单和拖放。按 host realm 的能力隐藏 DFS 专属项（权限对话框、分享、加入收藏集、AI 信息）。面包屑的根是电脑名。
- [ ] 设备文件的路径用标准 `file:///` URI：
  - `copyPathText` 遇到设备 URL 时生成 RFC 8089 的 file URI。各段按标准百分号编码（空格、非 ASCII、`%`、`?`、`#` 等），结果与 Node 的 `pathToFileURL` 一致。Unix 为 `file:///Users/alice/My%20Doc.txt`；Windows 为 `file:///C:/Users/alice/…`，用正斜杠，盘符保留冒号。注意这与 DFS 的复制规则不同：DFS 的 `cyfs://` 只转义 `%?#`，保持名字可读。
  - 状态栏、预览面板、多选复制等所有“复制路径”入口都走 `copyPathText`，不各自拼接。
  - 地址栏和状态栏显示设备位置时，用同一个 `file:///` 形式（解码后的可读形式即可），不显示内部的 `device://`。地址栏也接受粘贴进来的 `file:///` 路径（编码或未编码都可以），并导航到对应的设备位置；host realm 不可用时提示无法打开。
  - `data/urls.ts` 补单测：空格、中文、`#`、`%`、Windows 盘符、根目录。
- [ ] 收到 `SUDO_REQUIRED`（token 过期、服务重启）时：
  - 正在进行的交互弹出同一个 sudo 对话框，通过后自动重试原请求；
  - 用户取消时分区回到上锁状态；
  - 停在设备路径上的 tab 显示“已上锁”占位和“解锁”按钮。
- [ ] mock：加一个可开关的假 host realm（例如 `?fbHost=1`，或由 mock 注册参数控制），sudo 对话框通过 `requestPassword` 注入 mock。要覆盖这几种状态：未解锁、密码错误、解锁成功、token 过期后重新解锁、无权限目录、跨 realm 复制。
- [ ] 同步更新 [UI_DATAMODEL.md](../src/frame/desktop/src/app/filebrowser/UI_DATAMODEL.md)（`DeviceNode`、§9 第 10 项、设备 URL）和 [filebrowser_PRD.md](../product/bucky_file/filebrowser_PRD.md) §6.2 / §9.3：设备视图从“高级用户看集群各设备”改为“Lite 上 owner 用 BuckyOS sudo 看本机”。

### T12. 测试与 DV

- [ ] 先修改依赖旧默认值的 e2e：
  - [filebrowser-review.spec.ts](../src/frame/desktop/tests/e2e/pages/filebrowser-review.spec.ts) 多处点击侧边栏里的 “Pictures”，依赖树默认展开；
  - [filebrowser.copy.nfsp.spec.ts](../src/frame/desktop/tests/e2e/pages/filebrowser.copy.nfsp.spec.ts):333/363；
  - 所有依赖两个默认 tab、高级模式或 `Home` tab 标题的用例。

  全量跑一遍 filebrowser 与 sysdlg-select 的 e2e，先记下基线失败，再区分哪些是本任务引入的。
- [ ] 新增 mock e2e，每条至少跑桌面宽度和手机宽度（例如 390×844）两种：
  - 默认位置和单 tab；
  - 树首次打开全部收起，记住手动展开，能展开到第三层以下；
  - 三档图标大小切换后持久化；
  - 图标模式下图片 / 视频 / PDF 显示缩略图，`unsupported` 时显示类型图标且不重复请求；
  - 没有 host realm 时不显示设备分区；
  - sudo 解锁、密码错误、过期后重新解锁、跨 realm 复制；
  - 设备文件“复制路径”得到 `file:///`，在地址栏粘贴 `file:///` 能打开对应位置。
- [ ] NFSP e2e（单机 nfs_server 夹具）：无鉴权时“我的文件”是 `/home`。如果夹具的 TestKey 鉴权能签普通用户和 sudo token，再加两条：`/home/<user>`，以及 host realm 的接受 / 拒绝。
- [ ] nfs_server cargo 测试：
  - host realm 只在 Lite 加 owner 时通告；
  - 非 sudo token、`aud` 不对、已过期、非 owner、普通 DFS 会话都会被拒绝；
  - 未解锁或过期返回 `SUDO_REQUIRED`；
  - 路径穿越被拒；无权限的目录映射为 `PERMISSION_DENIED`；
  - 跨 realm 复制成功，失败时源文件不丢。
- [ ] 真实 Zone DV（本机 Zone，owner 和一个普通用户各一个）：默认位置；普通用户看到的内容与 nfs_server 一致；缩略图覆盖 JPEG / HEIC / MP4 / PDF / 带内嵌缩略图的 docx；手机宽度下的图标大小。
- [ ] Lite DV（Windows / macOS 测试 VM 上的 Lite 安装）：
  - 设备分区出现；
  - sudo 解锁，以及密码错误；
  - 浏览用户目录，系统目录显示为无权限；
  - 下载、预览，复制到我的文件，从我的文件复制到电脑；
  - token 的 `exp` 是 30 分钟；过期后重新解锁（不用真等 30 分钟，用 mock 或 cargo 测试里的短时长验过期，DV 只核对 `exp`）；关闭窗口后重新打开需要重新解锁；
  - 复制路径得到 `file:///` URI，在 Finder / 资源管理器或浏览器地址栏里能打开同一个文件；
  - 在手机宽度下完成一次解锁和复制。

## 4. 验收

- 打开 Files 只有一个“我的文件” tab，位置是 `/home/<当前用户>`（单机无鉴权时为 `/home`），过程中不闪 `/home` 列表；新 tab 也是我的文件。
- 侧边栏“我的文件”“最近使用”有图标和选中态；文件夹树首次打开全部收起，能展开到任意深度；侧边栏显示的内容与 nfs_server 返回的一致。
- 常见类型（PDF、Word、Excel、PPT、Markdown、代码、压缩包、电子书、安装包等）在列表和网格里图标可以区分，浅色和深色主题下都清楚。
- 图标模式下图片、视频、PDF（以及带内嵌缩略图的文档）显示缩略图，取不到时显示类型图标；切换小 / 中 / 大后布局正确、设置会保留；大目录滚动时没有重复请求，也不会卡住。
- 非 Lite 看不到设备分区。Lite 的 owner 通过 BuckyOS sudo 解锁后，可以像操作另一棵目录树一样浏览、预览、下载、复制电脑上的文件，权限不超过节点进程的系统账户；解锁 30 分钟内有效，过期或关闭窗口后要重新解锁；设备文件的复制路径是标准 `file:///` URI。
- 以上每一条都在手机宽度下验过；e2e 没有本任务引入的新失败；第二批有真实 Lite 安装上的 DV 记录。

## 5. owner 答复与待确认

Q1–Q10 的答复原话照录（11-16 owner 写在本文件中），“落实”列是据此做的修改。原来逐行插在表格中间的答复，挪到了表格的一列里，以免表格断开。

| # | 问题 | owner 答复 | 落实 |
| --- | --- | --- | --- |
| Q1 | “Desktop 版本”指 Zone 跑在装了 BuckyOS Desktop 的电脑上，还是指“在电脑上访问时”？ | 是的,这个功能只在Desktop Lite上有效 | T8 按产品 profile `is_lite()` 判定 |
| Q2 | sudo 密码的作用：只当门槛，还是真正提权？ | 只当门槛，sudo是buckyos的sudo(系统有公共的密码输入框) | T9 / T11 改用 verify-hub sudo 和公共对话框；访问权限是节点进程账户的权限 |
| Q3 | Windows 没有 sudo，用什么验证？ | 同上，是buckyos自己的提权要求 | 各平台一样，都走 BuckyOS sudo |
| Q4 | 解锁多久有效？ | 看buckyos sudo给的token有效期，我不记得。反正关闭窗口立刻失效 | 已查明：verify-hub 的 sudo token 有效期是 3 分钟。T9 跟随 token；T11 关闭窗口即丢弃。见 Q11 |
| Q5 | 管理员按什么判定？ | desktop lite版本下只有一个admin用户 | host realm 只对 Zone owner 通告 |
| Q6 | 设备视图本轮做到哪一步？ | 体验上就是另外一个目录树 | T10 读写、预览、跨 realm 复制都做；权限取决于系统账户。见 Q12 |
| Q7 | 设备分区是否只对 Zone owner 显示？ | 是 | T8 |
| Q8 | 非管理员看不到别人授权给他的目录，是否以后加入口？ | nfs给什么看什么 | T4 改为前端不按角色过滤 |
| Q9 | “默认收起”的范围？ | 只是默认UI状态，首次打开的时候全展开有点吓人（也可能是我本地的缓存数据的问题） | T3：首次打开全部收起；之后导航时仍可展开祖先。首次打开全展开是代码行为（根节点只要没被手动收起过就展开），不只是缓存的问题 |
| Q10 | “最近使用”同时出现在顶部和 AI 主题分区里，是否去掉后者？ | 都保留，后续AI主题分区会根据最近文件来分析。 | T2 两处都保留 |

第二轮（11-16）：

| # | 问题 | owner 答复 | 落实 |
| --- | --- | --- | --- |
| Q11 | sudo token 只有 3 分钟，跟随 token 意味着浏览时大约每 3 分钟要重输一次密码，是否接受？ | 可以签发30分钟的Token | T9：verify-hub 增加可选时长参数，上限 30 分钟、默认仍为 3 分钟；Files 申请 30 分钟 |
| Q12 | 电脑目录读写在 Linux 上节点以 root 运行时等于 root 读写，是否接受？ | Desktop Lite没有Linux版本 ，无需担心 | T10 按读写实施，不做 root 特判 |
| — | （owner 补充） | 设备文件树复制路径的时候，用的是file:/// 的标准路径 | T11：`copyPathText` 生成标准 `file:///` URI；地址栏显示和接受同一形式（显示和接受是我补的） |

## 6. 实施记录

（实施时填写：改动入口、偏差与理由、测试命令和结果、DV 环境与结果、未验证的平台。）
