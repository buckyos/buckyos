# TODO: FileBrowser 数据层重构 — Folder/View/Collection × FolderReader/FileItemList

> 状态：**已实现**（§6 的 PR1–PR4 全部落地；PR5 的后置项——拖拽 reorder、
> `device://`、搜索 view 化、Reader 级共享缓存、reorder 乐观更新——仍未做，
> 上移/下移菜单已保证顺序可调）。数据层代码在 `data/`，collection mock 在
> `mock/collections.ts`，e2e 覆盖在 `tests/e2e/pages/filebrowser.spec.ts`。
> 本文档保留为设计依据。
> 范围：`src/frame/desktop/src/app/filebrowser/` 下的 UI 数据层。不涉及真实 DFS 后端实现，
> 但要求重构后"接入真实后端 = 新写一个 Reader 实现"，UI 层零改动。
> Collection 的数据模型同样走 mock（集合持久化归后端——已有的 "Shares" 原理上
> 就是一个 Collection）；但成员操作（加/删/排序）要在 mock store 上**真实生效**，
> 让交互闭环可体验，而不是 toast。

## 1. 背景与问题

当前原型的数据假设是"全部目录内容已在内存、同步可得"：

- `FileBrowserSnapshot.entriesByPath: Record<string, FileEntry[]>` 是唯一数据源
  （[types.ts](./types.ts)），`entriesForPath()` 一行查表
  （[FileBrowserView.tsx](./FileBrowserView.tsx) `entriesForPath`）。
- 排序在 UI 层 `sortEntries()` 全量拷贝排序。
- [MainContent.tsx](./MainContent.tsx) 三种渲染路径（移动端列表 / 桌面表格 / 图标网格）
  全部 `entries.map(...)` 全量渲染真实 DOM，无虚拟化、无分页。
- 选中用 `selectedIds: string[]` + `includes()` 线性查找。
- `topic://` 视图、搜索各自走特例分支（`topicForPath` / `searchFiles`）。

这些假设在三个方向上会破：**大文件夹**（几千上万项）、**异步后端**（真实 DFS 列目录
必然是分页 RPC）、以及**位置类型不止"文件夹"一种**（见下节）。`FileEntry[]` 同步数组
签名已扩散到 MainContent、TopBar、StatusBar、context menu、selection 等多处，越晚改
动面越大。

## 2. 概念模型：三种位置类型（产品定案）

用户在浏览器里"打开"的任何位置，必属于以下三类之一。这是产品层定案，数据层抽象
必须把三者的差异编码进类型系统，方便后续 AI 驱动的文件管理：

| | **Folder 文件夹** | **View 视图** | **Collection 集合** |
|---|---|---|---|
| 本质 | 存储目的地（传统心智） | 条件筛选器的结果 | 手工管理的引用集合（全软链接） |
| 内容来源 | 真实存放在此 | 由查询条件派生（如"最近访问"、AI Topic） | 用户/AI 把任意已有文件或文件夹**放入** |
| 可写性 | 可写（上传/新建/粘贴 = 真实存储） | **只读**，不能手工增删任何元素 | 成员可管理（加/删**引用**），但**绝不是存储目的地** |
| 顺序 | 无固有顺序，按查询排序 | 按查询排序 | **有序**：用户可手工调整 item 顺序，该顺序决定默认 Preview 工具的浏览顺序 |
| 嵌套 | 子文件夹 | 无 | 内部可嵌套分组（组也是 Collection 结构的一部分，不是真实文件夹） |
| 删除语义 | 销毁数据 | 不可删 | 仅移除引用，原文件不受影响 |
| 磁盘影响 | 占用空间 | 无 | **零**——建立/整理 Collection 原理上不产生任何磁盘空间变化 |

关键推论（数据层必须承载）：

1. **Collection 内的每个文件有两个路径**：基于该 Collection 的路径（引用路径）+
   文件本身的原始路径。列表元素因此不能只是 `FileEntry`——同一个文件可以出现在
   多个 Collection、甚至同一 Collection 出现两次，**列表项身份 ≠ 文件身份**（见 3.4）。
2. **现有 `topic://` 按此分类是 View**（AI 按条件聚合的结果，只读），不是独立的
   第四类。搜索结果同理也是 View（本期不动，留口子）。
3. **写操作按 kind 分叉**：folder 的 paste/upload 是存储动作；collection 的
   "加入"是建引用；view 一律拒绝。UI 的 toolbar/menu/拖拽都要按 capability 裁剪，
   不能再硬编码。
4. **引用是 item 级概念，不是 Collection 专属**：folder 里也不排斥放引用 item
   （软链接）。Collection 的定义特征是"全部由引用组成 + 有序 + 必定不是存储目的地"，
   而不是"只有它能放引用"。引用能力建模在 item 上（`FileEntry.link`，见 3.4），
   双路径展示对任何容器里的引用项都适用。folder 里删除引用 item = 销毁链接对象本身
   （folder 删除语义"销毁所存之物"自然成立，无需特例）。
5. **引用类型可扩展**：引用 target 是规范 URL，类型由 scheme 决定、解析走注册表
   （`dfs://` 起步，share/远端等后续注册新 scheme 即可）。已有的 **"Shares" 原理上
   就是一个 Collection**——mock 要把它收编为种子集合，验证三分类能覆盖既有产品概念。

## 3. 目标架构

```
pane.url ──resolveReader()──▶ FolderReader（kind: folder|view|collection，按 scheme 注册）
FolderReader + ListQuery（排序/分页参数）──▶ FileItemList（有状态的已加载窗口）
FileItemList ──useFolderList() hook──▶ UI（虚拟化渲染，只取可见区间，按 capabilities 裁剪交互）
```

### 3.1 URL 即位置

每个 pane/tab 的"当前位置"统一为 URL 字符串。规范化为：

| scheme | kind | 例子 | Reader |
|---|---|---|---|
| `dfs://`（默认，裸路径 `/home/...` 等价于 `dfs:///home/...`） | folder | `/home/photos` | `DfsFolderReader`（现阶段 = mock） |
| `view://` | view | `view://recent`、`view://topic/trip-2025` | 各 view 类型一个 Reader |
| `collection://<id>/<groupPath?>` | collection | `collection://reading-list`、`collection://reading-list/papers` | `CollectionReader` |

- 现有 `topic://<id>` 迁移为 `view://topic/<id>`（保留旧 scheme 的解析兼容一行，
  规范化时重写）。
- 裸路径与 `dfs://` 的互转收敛到 `normalizeUrl()/displayPath()` 工具，
  面包屑/地址栏继续显示用户友好形式，内部一律存规范 URL。
- `device://`（advanced mode）预留 scheme，本期不实现。

注册表（沿用 menu registry 的数据驱动模式，见 [menu/registry.ts](./menu/registry.ts)）：

```ts
// data/readerRegistry.ts
export interface ReaderProvider {
  scheme: string                      // 'dfs' | 'view' | 'collection' | ...
  create(url: string): FolderReader
}
export function resolveReader(url: string): FolderReader
```

### 3.2 FolderReader 接口

新文件 `data/FolderReader.ts`：

```ts
export type LocationKind = 'folder' | 'view' | 'collection'

export interface LocationCapabilities {
  kind: LocationKind
  /** folder 专属：可作为存储目的地（上传/新建/粘贴实体）。 */
  acceptsContent: boolean
  /** collection 专属：可加入已有文件/文件夹的引用。 */
  acceptsReferences: boolean
  /** 删除语义：'destroy'（folder）| 'remove-ref'（collection）| null（view 不可删）。 */
  removal: 'destroy' | 'remove-ref' | null
  /** collection 专属：手工排序可用（sortKeys 含 'manual' 时为 true）。 */
  canReorder: boolean
  sortKeys: SortKey[]                 // collection 含 'manual' 且为其默认值
}

export interface ListQuery {
  sortKey: SortKey                    // SortKey 增加 'manual'
  sortDir: SortDir                    // sortKey='manual' 时忽略 dir（恒为集合定义顺序）
  /** 文件夹/分组排前面 — 查询参数下推，UI 不再自己排。manual 排序时忽略。 */
  foldersFirst: boolean
  offset: number
  limit: number
}

export interface FileItemPage {
  items: FileItem[]                   // 注意是 FileItem 不是 FileEntry，见 3.4
  totalCount?: number                 // 未知时 UI 退化为"加载更多"模式
  hasMore: boolean
}

export interface FolderReader {
  readonly url: string
  readonly capabilities: LocationCapabilities
  /** 位置自身的展示元信息（view 的条件描述、collection 的标题等），驱动顶栏 banner。 */
  readonly meta?: { title: string; description?: string }
  list(query: ListQuery): Promise<FileItemPage>
  /** 按 itemKey 取单项（PreviewPanel / 选中态恢复用）。 */
  getItem(key: string): Promise<FileItem | null>
  /**
   * 数据失效通知（上传完成、collection 被另一 pane 修改、refresh）。
   * mock folder 阶段可 no-op，collection 必须真实触发（双 pane 一致性靠它）。
   */
  watch(onInvalidate: () => void): () => void
  dispose(): void
}
```

排序语义从现 `sortEntries()` 原样下沉到 Reader（folders-first、name 为稳定
tie-breaker、`localeCompare(..., { numeric: true })`），mock Reader 内存排序，
真实后端将来服务端排序——**UI 层删除 sortEntries，不允许再对 items 排序**。

### 3.3 CollectionReader 扩展接口（成员管理）

在 FolderReader 之上扩展。集合的**数据模型走 mock**（与 folder mock 同级，
持久化将来归后端——Shares 本来就是后端概念），但成员操作在 mock store 上
**真实生效**，session 内交互闭环可体验：

```ts
// data/CollectionModel.ts —— 集合的数据模型与变更 API
export type CollectionNode =
  | { type: 'ref'; key: string; targetUrl: string }
  //  targetUrl 是任意规范 URL：引用类型由 scheme 决定（dfs:// 起步），
  //  解析走 target resolver 注册表，新增引用类型 = 注册新 scheme，模型不改。
  | { type: 'group'; key: string; name: string; children: CollectionNode[] }  // 集合内分组

export interface CollectionReader extends FolderReader {
  /** position 省略时追加到末尾。targets 是被引用项的规范 URL。 */
  addReferences(targets: string[], position?: number): Promise<void>
  removeItems(itemKeys: string[]): Promise<void>
  /** 手工排序：把 itemKeys 整体移动到 toIndex（决定默认 Preview 浏览顺序）。 */
  reorder(itemKeys: string[], toIndex: number): Promise<void>
  createGroup(name: string, position?: number): Promise<void>
  renameGroup(itemKey: string, name: string): Promise<void>
}
```

实现要点：
- 集合定义放进 mock 数据源（`mock/collections.ts` 的内存 store），变更方法
  就地修改内存数据并触发 watch——**接口形态即未来后端 RPC 形态**，换实现不换 UI。
  不做 localStorage/本地持久化：集合属于后端数据，UI 不自立门户。
- **集合只存 targetUrl 引用**，解析成 FileEntry 是 list 时通过 target resolver
  完成的，与文件数据源解耦。
- 引用悬空（目标被删/路径失效/scheme 无 resolver）时不崩：FileItem 标记
  `broken: true`，UI 灰显 + 菜单提供"移除失效引用"。
- 同一 `targetUrl` 允许在一个集合中出现多次（每次 ref 有独立 key）。
- `group` 打开 = 导航到 `collection://<id>/<groupPath>`；`ref` 指向真实文件夹时
  打开 = 导航到其**原始** `dfs://` URL（引用不改变文件夹的真实身份）。
- 变更后通过 `watch` 广播失效，另一 pane 打开同一集合时自动刷新。
- **Shares 收编**：现 Sidebar 的 Shared root（`DfsNode.kind === 'shared'`）在
  mock 中重构为一个种子 Collection（`collection://shares`），Sidebar 入口指过去。
  这是三分类对既有概念的回归验证，不是新功能。

### 3.4 FileItem：列表元素 ≠ 文件实体

新类型，列表/选中/菜单全链路用它替代裸 `FileEntry`：

```ts
export interface FileItem {
  /** 列表内唯一身份：folder/view = entry.id；collection = ref key（同文件可出现多次）。 */
  key: string
  entry: FileEntry
  /** 仅 collection 列表项存在 —— 双路径的"集合侧"上下文。 */
  ref?: {
    collectionUrl: string      // 所属集合（含 group 路径）
    refPath: string            // 基于该 Collection 的路径（路径 1）
    orderIndex: number         // 手工顺序
    broken?: boolean
  }
  // entry.path 始终是原始路径（路径 2），不因出现在 collection/view 中而改变
}
```

同时在 `FileEntry` 上新增 item 级引用字段（folder 里也能放引用 item，见 §2 推论 4）：

```ts
// types.ts 扩展
export interface FileEntry {
  // ...现有字段...
  /** 此 entry 本身是一个引用（软链接）。target 为规范 URL，scheme 可扩展。 */
  link?: { targetUrl: string; broken?: boolean }
}
```

两个概念不要混淆：`entry.link` 是"这个 item 是不是软链接"（item 级，任何容器都可能有）；
`item.ref` 是"这个列表项在 collection 里的成员上下文"（listing 级，仅 collection）。
collection 的列表项两者都有语义（成员即引用），folder 里的链接 item 只有 `link`。

约束：
- **选中、shift 范围、右键、虚拟行 key 一律用 `item.key`**，不再用 `entry.id`
  （collection 中同文件两次出现必须可被独立选中/移除）。
- PreviewPanel、"copy path"等文件级操作继续读 `entry`（原始路径）；
  带 `link`/`ref` 的项额外提供"跳转到原始位置"动作（导航到 target/原始路径的父目录
  并选中该项）。
- 渲染层对 `link`/`ref` 项加链接角标（Finder alias 式小箭头），与 kind 图标正交。

### 3.5 FileItemList（已加载窗口的视图模型）

新文件 `data/FileItemList.ts`。职责：对一个 `(reader, sortKey, sortDir)` 组合，
维护稀疏的已加载区间，供虚拟滚动按需取数。

```ts
export interface FileItemList {
  readonly url: string
  readonly capabilities: LocationCapabilities   // 透传 reader 的，UI 单一取用点
  readonly totalCount: number | undefined       // 未知时 UI 显示 "1,200+ items"
  readonly status: 'idle' | 'loading' | 'ready' | 'error'
  readonly error: Error | null
  itemAt(index: number): FileItem | undefined   // 未加载位置 undefined → 骨架占位
  ensureRange(start: number, end: number): void // 可见区间变化时调用，内部去重合并请求
  loadedItemByKey(key: string): FileItem | undefined
  loadedKeys(): string[]                        // 当前已加载的有序 key（shift 范围选择用）
  reload(): void
}
```

实现要点：
- 内部 `Map<number, FileItem>` 或分块数组存稀疏数据，按 `PAGE_SIZE = 200` 对齐
  请求边界，in-flight 请求去重。
- 排序参数变化 = 丢弃全部已加载数据重新拉。
- 请求带版本号，`reload()`/参数变化后到达的旧响应直接丢弃。
- collection 的 reorder 是**就地变更**：变更经 reader 落库后走 watch→reload；
  本期不做乐观更新（拖拽松手后短暂 loading 可接受），接口不阻碍将来加。

### 3.6 React 集成

新 hook `data/useFolderList.ts`：

```ts
function useFolderList(url: string, sortKey: SortKey, sortDir: SortDir): FileItemList
```

- 内部 `resolveReader(url)`，url 变化时 dispose 旧 reader。
- 订阅 `reader.watch()`，失效时 `reload()`。
- 通过 `useSyncExternalStore` 或内部版本号 state 触发重渲。
- 双 pane 各自一个实例互不共享；同一 collection 的跨 pane 一致性靠 watch 失效，
  不靠共享缓存（Reader 级缓存共享留作后续优化）。
- pane 的 `sortKey` 初值按 capabilities 定：collection 默认 `'manual'`，
  其余默认 `'name'`；切换位置时若当前 sortKey 不在 `capabilities.sortKeys` 内，
  重置为该位置默认值。

## 4. UI 层改动

### 4.1 MainContent 虚拟化

- 引入 `@tanstack/react-virtual`（新依赖，确认 pnpm workspace 安装到 desktop 包）。
- props 从 `entries: FileEntry[]` 改为 `list: FileItemList`。
- 三种渲染路径全部虚拟化：
  - **桌面表格**：`<table>` 布局与 row virtualizer 不好兼容，改成 CSS grid/flex 的
    div 行（表头保持 sticky，列宽用 `grid-template-columns` 对齐表头与行）。
  - **图标网格**：按"行 = Math.ceil(可视宽/单元宽) 个 item"折行虚拟化，
    容器 resize 时重算列数（`ResizeObserver`）。
  - **移动端列表**：普通 row virtualizer。
- 滚动时对可见区间调用 `list.ensureRange(start, end)`；`itemAt()` 返回 undefined 的
  位置渲染骨架行（灰条占位）。
- 空态判断改为 `list.status === 'ready' && list.totalCount === 0`；
  首屏 loading 渲染骨架列表。**空态文案按 kind 分**：folder = 现"上传"引导；
  collection = "把文件拖进来或从右键菜单加入"；view = "没有符合条件的内容"（无按钮）。

### 4.2 capability 驱动的交互裁剪

现在 toolbar/menu 的可用性是硬编码的（如 `canOrganize: !topic`），全部改为读
`list.capabilities`：

- **TopBar toolbar**：Upload/New folder/New file 仅 `acceptsContent`；
  Paste 在 folder = 真实粘贴（mock toast），在 collection = `addReferences`
  （真实生效）；Delete 文案按 `removal` 切换（"删除" vs "移出集合"），view 隐藏。
- **menu registry**：`FileMenuContext` 增加 `capabilities: LocationCapabilities`
  与 `item?: FileItem`，现有 `when` 条件改写；新增菜单项：
  - "加入 Collection ▸"（folder/view 中对任意选中项可用，二级列出现有集合 +
    "新建集合…"）——这是 AI 驱动整理的核心入口，必须真实生效。
  - "移出集合"（collection 内，`removeItems`）。
  - "跳转到原始位置"（collection/view 内）。
  - "移除失效引用"（`ref.broken` 时）。
- **排序菜单**：选项来自 `capabilities.sortKeys`；collection 的 'manual' 显示为
  "自定义顺序"。
- **拖拽 reorder**（collection + manual 排序时）：列表/网格行内拖拽调用
  `reorder()`。若 PR 体量过大可拆出，但"上移/下移"菜单项必须先有，
  保证顺序可调（顺序决定默认 Preview 浏览顺序，是 Collection 的核心语义）。

### 4.3 双路径/引用展示

- PreviewPanel：collection 项显示两行路径——集合路径（`ref.refPath`）+
  原始路径（`entry.path`，可点击跳转）；folder 里的 `link` 项显示链接 target。
- StatusBar：单选 collection 项/链接项时同样显示原始路径。
- "Copy path" 默认复制原始路径；collection 项的菜单里两个路径分别可复制。
- 所有 `link`/`ref` 项渲染链接角标；`broken` 项灰显。

### 4.4 选中模型

- `selectedIds: string[]` → `Set<string>`，**key 为 `FileItem.key`**（见 3.4）。
- 选中实体改为**捕获式**：选中时把 `FileItem` 存进 `Map<string, FileItem>`
  （从 `list.loadedItemByKey` 取），PreviewPanel / toolbar / StatusBar 用这份 Map，
  不再回查全局 `entriesById`（该索引随 snapshot 退役）。
- shift 范围选择基于 `list.loadedKeys()`；跨未加载空洞的范围选择不支持。
- select-all：本期语义 = "选中所有**已加载**项"；`totalCount` 与选中数不一致时
  StatusBar 显示 `selected 200 of 1,500`。全量 select-all（allExcept 模型）不在本期。

### 4.5 FileBrowserView 接线 / Sidebar

- `useBrowserPane` 增加 per-pane 的 `useFolderList(url, sortKey, sortDir)`；
  删除 `entriesForPath` / `sortEntries` / `topicForPath` 的数据职责
  （topic 标题等元信息走 `reader.meta`，现 topic banner 改为通用的
  "view/collection banner"：view 显示条件描述，collection 显示标题 + 引用计数）。
- StatusBar `totalCount` 改用 `list.totalCount`（undefined 时显示 `…`）。
- refresh 从 toast mock 改为真正调 `list.reload()`。
- Sidebar：topics 区块改为通用的 "Views" 区块（recent + topics）+ 新增
  "Collections" 区块（列出 CollectionStore 中的集合，支持"新建集合"）。
  DfsNode 树、devices 数据量小，本期仍读 snapshot。
- 搜索（`searchFiles`）本期**不动**，保持现状分支；留 TODO 注释：
  搜索可统一为 `view://search?q=...`（SearchHit 需要 reason/snippet，接口需扩展，后置）。

## 5. Mock 实现

- `mock/data.ts` 的 `fileBrowserSnapshot` 保留为 mock Reader 的内部数据源，
  但**不再从 FileBrowserView 直接 import entriesByPath/entriesById**。
- 新 `data/mockReader.ts`：
  - `MockDfsFolderReader`：内存排序/切片，**人为加 50–150ms 随机延迟**
    （暴露 loading 态、乱序响应等时序问题）。
  - `MockRecentViewReader`（`view://recent`）：按 modifiedAt 倒序取全库前 N。
  - `MockTopicViewReader`（`view://topic/<id>`）：现 topic 聚合逻辑迁入。
- `mock/collections.ts`：集合的内存 store + `CollectionReader` 实现（见 3.3），
  变更方法就地改内存数据并触发 watch；target 解析走 resolver 注册表
  （本期只注册 `dfs://` resolver，复用 mock 数据源的 `getItem`）。
- mock 数据新增：
  - 大文件夹 `/home/stress-10k`：程序化生成 10,000 个 FileEntry（混合 kind、
    随机 size/modifiedAt，名字带数字便于验证 numeric 排序）。生成用模块内函数，
    不要手写数据。
  - **Shares 收编为种子集合** `collection://shares`（成员 = 现 `/shared` 下内容的
    引用），Sidebar 的 Shared 入口改指此 URL。
  - 自定义种子集合 1 个（含嵌套 group、含一条指向真实文件夹的 ref、含一条
    broken ref、含同一文件的两条 ref）。
  - 某个普通 folder 里放 1 个 `link` 引用 item，验证"folder 也能放引用"的渲染与
    跳转路径。

## 6. 实施顺序（建议分 PR）

1. **PR1 — 抽象层 + mock 实现，行为不变**：
   FolderReader/FileItem/FileItemList/registry/useFolderList + mock readers
   （dfs + topic 迁 view://topic）；FileBrowserView 改为消费 hook；MainContent 暂时
   `toArray()` 全量渲染。验收：现有交互全部不回归（双 pane、tab、历史、topic 视图、
   右键菜单、排序、选中）。
2. **PR2 — 虚拟化三个渲染路径** + 骨架占位 + stress-10k；删除临时 `toArray()`。
3. **PR3 — 选中模型重构**（Set + FileItem.key + 捕获式 Map + StatusBar 计数语义）。
4. **PR4 — Collection 落地**：mock collection store + CollectionReader +
   Shares 收编 + Sidebar Collections 区块 + "加入 Collection/移出集合/跳转原始
   位置" 菜单 + 双路径/链接角标展示 + `FileEntry.link` + manual 排序与上移/下移
   （拖拽 reorder 可拆 PR4.5）+ `view://recent`。
5. **PR5（可选后置）** — 拖拽 reorder、`device://` scheme、搜索 view 化、
   Reader 级共享缓存、reorder 乐观更新。

## 7. 验收标准

- `/home/stress-10k` 打开：首屏 < 200ms 出骨架，滚动流畅（DOM 行数恒定在
  可视区 + overscan，~100 以内），列表/图标/移动端三模式均可用。
- 切排序键/方向：触发重新拉取（可见 loading 态），不在 UI 层排序。
- **三类位置语义正确**：view 内无任何增删入口；collection 内 Delete 显示为
  "移出集合"且原文件不受影响（去原 folder 验证仍在）；folder 行为不回归。
- **Collection 端到端**（session 内，mock store）：从 folder 多选 →"加入
  Collection"→ 集合内可见、顺序可调、双 pane 打开同一集合一侧改另一侧自动刷新、
  broken ref 灰显可清理、同一文件加两次可分别选中/移除。
- **Shares 收编**：Sidebar 的 Shared 入口打开的是 `collection://shares`，
  内容与原 `/shared` 一致，capability 表现为 collection（可移出引用、可调序）。
- folder 内的 `link` 引用 item：渲染链接角标、可"跳转到原始位置"、删除仅销毁链接。
- topic 视图（迁 `view://topic/`）、右键菜单、多选（click/ctrl/shift/select-all）、
  PreviewPanel 行为不回归；collection/view 项可"跳转到原始位置"。
- `grep -rn "entriesByPath\|entriesById" src/app/filebrowser --include='*.tsx'`
  在 PR3 后只剩 mock/data 目录内命中。
- 既有 playwright 测试通过；新增 e2e：stress 文件夹滚动 + 排序、collection
  加入/调序/移出 闭环。

## 8. 非目标（明确不做）

- 真实 DFS 后端对接、folder 写操作（上传/删除/重命名）的真实实现
  （仍是 toast mock，但完成后的刷新路径要走 `reader.watch` → `reload`，接口先立好）。
  注意 Collection 的成员管理**不在此列**——它在 mock store 上真实生效。
- Collection 的持久化（mock 是内存数据，刷新页面回到种子状态——持久化归后端，
  UI 不做 localStorage 自立门户）。
- `dfs://` 以外的引用 target scheme 实现（resolver 注册表立好即可，
  share/远端等类型后续注册）。
- 跨未加载区间的 select-all / allExcept 选中模型。
- Sidebar DfsNode 树与搜索的 Reader 化。
- 缩略图/预览懒加载；Preview 工具按 manual 顺序连播（顺序语义先立住，播放器后置）。
