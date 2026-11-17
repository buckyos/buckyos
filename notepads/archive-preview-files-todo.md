# 压缩包浏览与 Preview 支持 TODO

状态：待实施。日期：2026-11-16。源码基线：buckyos `e2bad66d5`、buckyos-websdk `39f2127`，包含当前工作区；Files / Preview 最近升级为 `a90582731`、`51dc9a3a0`。本文是设计 review 与任务拆分；owner 已明确 Preview 内复用 Files 的文件列表显示（§1.1），其余建议项尚未视为 owner 裁决。

目标：在 Preview 中浏览压缩包目录并原地预览成员；每个成员有可传递、可重新打开的引用；Files 可选把压缩包作为目录浏览。NFS 提供共用的只读归档访问能力，按需解压，并在密码验证成功后按用户与归档内容身份记住密码。

建议首期完成 ZIP / ZIP64 的闭环，采用“只读 archive provider + 版本绑定引用 + 按需流式读取 + 有界缓存 + 必要时单成员物化”。不把通用 mount 系统作为前置项目。

## 1. Review 结论

### 1.1 Preview 复用 Files 的文件列表显示与交互

**owner 已明确（2026-11-16）：Preview 内部复用 Files 的“文件列表显示”，以保持体验一致。** 两个入口使用同一套展示组件与交互逻辑，包括文件行 / 图标视图、图标与缩略图、名称和元数据格式、选择、排序、打开及移动端触摸行为。复用应落实到共同维护的组件，不能只复用数据模型或照着 Files 另写一份相似 UI。

优先复用现有 `MainContent` 及其展示层；若存在 Files 宿主耦合，只做必要拆分，并让 Files 与 Preview 同时使用拆出的组件。共享列表接受条目数据、展示选项、选择状态、能力和操作回调；归档来源、目录导航栈、成员原地预览由 Preview 宿主负责。列表中的菜单与操作由只读 capabilities 裁剪。复用范围是文件列表显示与交互，Files 的窗口、侧栏、tab 和上传工作流仍由各自宿主管理。

现有 Preview session 只把叶子文件平铺成上一项 / 下一项；仍需补归档目录页、面包屑和导航栈，并将分页目录数据接入共享列表。

进入目录留在当前 Preview；打开成员复用图片、文本、PDF、表格、音视频和文档转换链。返回列表时恢复所在目录、选中项和滚动位置。深链接打开成员也应能返回其包内父目录。手机返回、返回包内列表、关闭 Preview 要有明确区别。

### 1.2 named object + inner_path 合适，但要区分路径与内容版本

外层路径可能被覆盖、移动，不能因为它是压缩包就当作 immutable。建议区分：

| 表达 | 语义 |
| --- | --- |
| `cyfs:///home/alice/a.zip` | 压缩包原文件，仍可按文件下载、复制、移动、删除 |
| `cyfs:///home/alice/a.zip/@/` | 建议的归档目录入口，指向当前路径的内容；不是固定版本 |
| `cyfs:///home/alice/a.zip/@/docs/a.txt` | 建议的成员浏览入口；实际目录是 `home`，不是 `homes` |
| 父归档对象 + 成员路径 | 固定内容的成员身份；复用 `ObjectRef { obj_id, inner_path }`，具体成员命名空间见 T1 |
| 本地文件来源 + expected version + 成员路径 | 尚未对象化时的浏览句柄；发现源变化即失效，不宣称 immutable |

现有 CYFS 已使用 `/@/` 遍历对象，而 FileObject 的 `/@/content` 表示内容对象。归档里恰好名为 `content` 的文件不能抢占这个含义。友好路径可以作为归档视图入口，但不能直接将其尾部当成普通 FileObject 的任意字段路径。

已有 NamedObject 可直接固定父对象；普通文件不应为了列出 ZIP 目录而强制先完整复制、算 hash、入 NamedStore。先按源版本浏览；需要长期稳定引用时，再复用或补齐按需 snapshot 能力，保证对象和内容实际保留、调用方可读。不能把 `sha256:<digest>` 当成可读取的 FileObject ID。

### 1.3 类似 mount，但首期只需要只读归档适配层

从体验看是惰性挂载；从实现看，在 NFS 的 `resolve/stat/list/read` 增加归档节点与读取分支即可。现有 bucket projection 已经有虚拟目录的先例，不必先引入挂载表、任意后端插件、系统级 mount 或 FUSE。

Files 与 Preview 共用这一层。外层文件的类型不变，进入归档视图后才显示为只读目录。Files 的配置只控制默认打开动作，不改变服务端路径含义，也不关闭 Preview 对压缩包的显式支持。

### 1.4 内存解压与临时目录不是二选一

赞同避免整包解压和重复 IO，但“只解压需要的部分”取决于格式与访问方式。建议先分清以下成本：

| 格式 / 场景 | 读取特点 | 建议 |
| --- | --- | --- |
| ZIP / ZIP64 列目录 | 通常可读尾部中央目录建立索引，不需解压所有成员 | 索引独立缓存，目录分页返回 |
| ZIP Stored 成员 | 未压缩；具备合适源读取和校验条件时可映射字节区间 | 按实际能力支持直接读取 |
| ZIP Deflate 成员 | 可定位成员；成员内偏后位置通常仍需从该压缩流前部解码 | 流式读取；对随机访问使用已解压缓存或物化 |
| solid 7z / RAR | 所需工作可能包括同一 solid block 的其他成员 | 按块计成本，后续支持，不能承诺只解压一个文件 |
| tar.gz 等顺序压缩流 | 无额外索引时，列目录和访问后部成员可能要扫描前部流 | 后续支持索引 / 顺序读取 / 缓存策略 |
| 文档转换、视频 seek、反复读取 | 消费者可能需要可 seek 的文件或本地路径 | 仅将所需成员物化到受控缓存，复用结果 |

ZIP 中央目录及成员定位依据 [PKWARE ZIP 规范](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT) §4.3.12、§4.4.16；Deflate 的随机访问需要额外解码状态，见 [zlib 的 inflateCopy 说明](https://www.zlib.net/manual.html)；solid block 的解压与缓存见 [7-Zip 解码文档](https://raw.githubusercontent.com/ip7z/7zip/main/DOC/7zC.txt)。

默认采用有背压的流式解压，小成员和热点块进有上限的内存缓存；需要本地路径或反复 seek 时再物化。内存预算包括索引、字典、解压输出和并发，不只是输出 buffer。临时文件也可能命中 OS 页缓存，不能预先断言内存方案总是更快；用冷 / 热缓存和真实 Preview 访问模式测量。

## 2. 当前代码与接入位置

下表是源码现状，不能把历史 TODO 中的计划当成已实现能力。跨仓库路径相对本文件所在目录。

| 环节 | 现状与缺口 | 入口 |
| --- | --- | --- |
| Files 分类与打开 | 已有 archive 图标分类；打开仍走 Content Open Router | [fileKinds.ts](../src/frame/desktop/src/app/filebrowser/data/fileKinds.ts)、[FileBrowserView.tsx](../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx) |
| 文件列表展示复用 | `MainContent` 已接收 `FileItemList`、视图 / 选择状态、打开与菜单回调；从此处复用或做最小拆分，Files 与 Preview 共用同一实现 | [MainContent.tsx](../src/frame/desktop/src/app/filebrowser/MainContent.tsx)、[FileItemList.ts](../src/frame/desktop/src/app/filebrowser/data/FileItemList.ts)、[fileDisplay.tsx](../src/frame/desktop/src/app/filebrowser/fileDisplay.tsx) |
| Files 设置 | settings 菜单目前返回 unsupported；开关需要可用入口与持久化 | [commands.ts](../src/frame/desktop/src/app/filebrowser/menu/commands.ts) |
| Preview 容器 | `ContentPreview` 拒绝 dir/container；`enumerateContainer` 是叶子列表 | [ContentPreview.tsx](../src/frame/desktop/src/components/ContentPreview.tsx)、[types.ts](../src/frame/desktop/src/components/preview/types.ts) |
| 协议引用 | Rust / TS 已有 object ref 的 `inner_path`；NFS `resolve_ref` 目前拒绝 object refs | [types.rs](../src/frame/nfs_server/src/types.rs)、[namespace.rs](../src/frame/nfs_server/src/namespace.rs)、[websdk nfsp.ts](../../buckyos-websdk/src/nfsp.ts) |
| 引用传递 | Preview ObjectIdRef、SDK TransferableContentRef 缺 inner_path；provider 枚举 object 条目时丢弃它，session 身份也只用 objectId | [Preview types](../src/frame/desktop/src/components/preview/types.ts)、[nfspProvider.ts](../src/frame/desktop/src/components/preview/nfspProvider.ts)、[session.ts](../src/frame/desktop/src/components/preview/session.ts)、[content.ts](../../buckyos-websdk/src/content.ts)、[app-frame.ts](../../buckyos-websdk/src/app-frame.ts) |
| 对象预览 | object-id 走 NDM 全量 Blob，限制 256 MiB，无 Range；不能据此让浏览器下载整个包 | [ndmSource.ts](../src/frame/desktop/src/components/preview/ndmSource.ts) |
| CYFS 解析 | 已有 `/@/`；FileObject inner path 只有 `content`；NFS 先整串解码，SDK 又会把 `%40` 归一回 `@` | [cyfs_http.rs](../../cyfs-ndn/src/ndn-lib/src/cyfs_http.rs)、[ndn_read.rs](../../cyfs-ndn/src/bucket_store/src/ndn_read.rs)、[namespace.rs](../src/frame/nfs_server/src/namespace.rs)、[object_url.ts](../../buckyos-websdk/src/object_url.ts) |
| NFS 读取与权限 | 数据面读取原生文件；归档需补节点、读分支、外层来源授权与只读能力 | [server.rs](../src/frame/nfs_server/src/server.rs)、[access.rs](../src/frame/nfs_server/src/access.rs)、[types.rs](../src/frame/nfs_server/src/types.rs) |
| 转换与依赖 | 已依赖 `zip` crate，能提取 Office/EPUB 内嵌图；已有 `repr` 管线，但没有通用归档成员输入 | [Cargo.toml](../src/frame/nfs_server/Cargo.toml)、[preview/convert.rs](../src/frame/nfs_server/src/preview/convert.rs)、[preview/rpc.rs](../src/frame/nfs_server/src/preview/rpc.rs)、[preview/mod.rs](../src/frame/nfs_server/src/preview/mod.rs) |
| 快照与 hash | 有分享快照及 Preview 输入 hash 缓存；普通文件的通用对象化另有 TODO | [share/snapshot.rs](../src/frame/nfs_server/src/share/snapshot.rs)、[nfs-server-file-objid-todo.md](nfs-server-file-objid-todo.md) |
| 分享与其他操作 | 当前会把普通 cyfs 路径交给 DFS 分享；归档成员必须按能力判断 | [content/share.ts](../src/frame/desktop/src/app/content/share.ts)、[menu/commands.ts](../src/frame/desktop/src/app/filebrowser/menu/commands.ts) |

## 3. 首期范围建议

- 源：本 Zone 的 DFS 文件、调用方有权读取的 NamedObject / bucket projection。来自聊天等入口的对象须保留既有授权上下文。
- 格式：ZIP / ZIP64，Stored / Deflate；密码 ZIP 的 ZipCrypto / AES 能力用当前依赖与真实样本核实，明确支持矩阵。格式支持必须同时说明压缩方法与加密方法，不能只看扩展名。
- 体验：分页浏览、原地预览成员、复制成员引用、下载单成员、记住 / 忘记密码、Files 可配置目录模式。
- 后续：7z / RAR / tar / tar.gz、嵌套包、分卷包、批量“解压到”、本机 host realm 归档。首期对这些有明确不支持状态；现有本机普通文件能力不变。
- 包内不支持原地写入、删除、改名、上传、修改 ACL。DOCX / XLSX / EPUB / VSDX 等 ZIP 容器文档保持现有预览优先级。

## 4. 实施 TODO

### T1 — 冻结归档路径、版本与引用协议（P0）

- [ ] 明确归档原文件、归档根、成员目录、成员文件四种定位和能力。优先复用 NFSP ObjectRef 与现有只读节点模式；增加的归档上下文必须贯穿权限与读取。
- [ ] 冻结成员空间与 FileObject `content` 的关系。建议保留 FileObject 原语义，通过显式归档视图 / 成员分支访问；例如保留 `/archive/` 分支后，`inner_path: '/archive/docs/a.txt'` 表示成员（这是候选协议，不是当前已支持语法）。`a.zip/@/docs/a.txt` 作为友好入口由 resolver 显式映射。若采用 ArchiveObject / DirObject 作为根，则同时说明构造、保留与解析成本；不要悄悄改写所有 FileObject 的 inner path 语义。
- [ ] 定义 literal `/@/`、真实 `@` 段、`%`、`#`、`?`、中文名的解析 / 格式化规则：先分语义边界，再逐段解码一次；formatter 必须保留真实 `%40` 与分隔符的区别。同步 Rust、SDK、NFSP 路径、权限键与网关实际转发行为。
- [ ] 定义空 inner path、归档根尾斜线、大小写、Unicode / 非 UTF-8 名称策略、隐式目录、同名 entry 与规范化碰撞。不能默默选择第一个 / 最后一个；首期可对歧义路径返回明确错误并标记条目，后续再扩稳定 entry 标识。
- [ ] live 来源固定 expected version。索引、读取和 hash 前后检查同一来源；外层替换 / 原地修改时返回 stale 并提示重新打开，不能把旧索引用于新字节。仅 size/mtime 的校验有盲区，不宣称强不可变；固定打开 fd 也不能阻止原地修改。
- [ ] 分开“复制当前位置”与“复制固定版本引用”。后者要求真实对象快照 / 已有不可变对象，并明确保留期限和权限；前者重新打开时可跟随路径的新内容。联动 [文件对象 ID TODO](nfs-server-file-objid-todo.md) 核实并完成所需 snapshot、对象保留和调用方读取授权，作为固定引用功能的交付前置；已有对象也需确保保留。未完成时不能把 live 地址标为永久引用。
- [ ] 把 source version、成员路径 / 标识纳入所有身份键：Preview session、窗口复用、缩略图、解压缓存、转换缓存、下载引用。密码和解锁状态不得出现在可复制引用里。

### T2 — NFS 只读归档节点与控制面（P0，依赖 T1）

- [ ] 在现有 namespace 中接入 archive root / entry，复用 `resolve/stat/list`，对真实目录和归档容器统一返回分页条目与 capabilities。为 ZIP 构造目录索引，不解压其他成员内容；限制中央目录大小与条目数，分页不能掩盖后台无界内存占用。
- [ ] 实现归档 ObjectRef 的解析与成员读取。首期候选 `/archive/` 分支由 NFS archive provider 处理，Preview / Files 从该 resolver 获取读取地址；仅增加 NFSP 字段不代表通用 CYFS / NDM / `/ndn/` 已支持。若冻结协议要求这些入口也直接读取成员，须将 `ndn_read.rs`、NamedDataMgr 的对应解析与鉴权联动列入同批交付。
- [ ] 实现 archive provider 的源读取、成员定位与流接口；优先复用已有 `zip` 依赖。CPU 密集解压进入有并发限制的 blocking worker，不能阻塞 async 服务线程。
- [ ] `hello` 通告实际归档格式、压缩 / 加密方法与限制；复用现有错误模型，区分不支持、损坏、需密码、密码无效、源失效、资源超限。无法区分密码错误与损坏时返回诚实的解密 / 完整性失败。
- [ ] 所有成员操作以外层来源授权为前提：list 也要求读压缩包权限，不能套用目录的 traverse 即可列举规则。NamedObject / bucket 来源沿用既有授权，不因知道 hash 就放行。
- [ ] 包内能力明确只读，`remove_semantics = none`，不接收文件 / 引用；服务端拒绝所有写操作。不要直接套用仍含 destroy 的 `Capabilities::for_kind("dir", false)`。外层原包保持原文件操作。
- [ ] 缓存命中、解锁、成员 read、缩略图、repr 都重新校验请求的访问上下文。权限撤销后的新请求不能继续取出已缓存明文。

### T3 — 数据面、缓存与现有 Preview Pipeline（P0，依赖 T2）

- [ ] 为成员返回服务端 `access_urls`，支持流式读取、取消与下载文件名；客户端下载使用返回地址，不自行按普通 nodeId 拼 URL，也不先将整个压缩包读成 Blob。
- [ ] 区分成员可定位与成员内可 seek。定义 Range / HEAD / Content-Length / ETag / 条件请求语义；206、Content-Range 和 416 必须正确，不把压缩字节偏移当作解压后偏移。
- [ ] 小文件顺序读取优先流式处理；大文件不整份放内存。针对 PDF 尾读、视频拖动等请求，按预算选择顺序解码、复用已解压数据或先物化目标成员；需等待时有进度和取消，不能每个 Range 都无界重解压。
- [ ] 分离目录索引缓存、解压成员 / 块缓存、转换缓存与凭据存储。配置全局和单用户内存预算、磁盘配额、并发、TTL / LRU；相同工作的并发请求合并，最后一个消费者离开后才取消共享工作。
- [ ] repr 接受归档成员，仍使用既有格式识别与转换器。需要本地文件的转换器只物化该成员，缓存键包含父归档版本与成员身份；临时路径由服务端生成，不能直接拼 entry 文件名。
- [ ] 解密明文与派生产物有独立访问控制；不能因内容去重而自动向其他用户开放。磁盘缓存采用受限权限、明确生命周期和崩溃清理；失败 / 校验未完成的产物不能标记为成功。

### T4 — 密码输入、记忆与忘记（P0，可与 T3 并行）

- [ ] 用统一解锁流程服务 Preview 和 Files：未输入、验证中、已验证、验证失败、取消。加密目录需要先解锁再 list；只加密成员数据时可以先列目录、读成员时再询问。
- [ ] 成功验证后服务端记忆密码，凭据键至少包含 `Zone + user + archive content hash`。改路径 / 改名仍可复用；换内容不能复用旧身份；不同用户不共享解锁结果。调用方 App 的可用范围沿用系统授权规则。
- [ ] 不能把 ZIP CRC、路径、size/mtime 或 FileObject ID 简单当作全内容 hash。已有可信内容身份时复用；否则当前浏览先用版本绑定会话，完整 hash 稳定后再持久化记忆，不让第一次列目录强制等待全包 hash。
- [ ] 持久密码加密保存，密钥放在服务端受保护的密钥设施；不进入 URL、日志、普通元数据或浏览器 localStorage。提供忘记单包密码 / 清除记忆的可用入口。
- [ ] 保存前完成所支持加密方案的密码 / 完整性验证，不能仅凭 header 检查通过或读出前几个字节就判定成功。部分读取未验证时只保留临时会话状态。
- [ ] 考虑 ZIP 不同成员不同密码：包 hash 下保存验证过的候选或成员范围；一个成员成功不等于全包解锁。失败不能删除其他成员已验证可用的凭据；多窗口对同一解锁请求去重。

### T5 — 复用 Files 列表、接入 Preview 目录浏览与引用传递（P0，依赖 T1–T4）

- [ ] **按 owner 要求复用 Files 的文件列表显示组件。** 优先直接使用 `MainContent`；仅在宿主耦合影响嵌入时拆出必要的共享展示层，Files 自身也使用同一实现。保留文件行 / 图标视图、手机列表、图标 / 缩略图、格式化、分页虚拟滚动和选择 / 打开行为的一致性，不复制一套归档专用列表。
- [ ] 共享组件通过数据、展示选项、状态和回调适配宿主：Preview 的打开目录回调进入包内目录，打开文件回调在当前 Preview 切换成员；菜单按成员 capabilities 提供。当前手机列表经 `onSelect` 处理点击，接入时沿用 Files 的普通点击打开 / 选择模式点击选择规则，不能只连接 `onOpenFile`。Files 与 Preview 各自持有所在位置、选择和滚动状态，避免互相覆盖；沿用共同的展示规则与可适用的用户偏好。
- [ ] 将 Files 专属的空态文案 / 动作改为按能力或由宿主提供：空归档不提示上传、拖入、同步；仅隐藏上传按钮不足以解决文案问题。加载、错误和重试复用列表样式，密码输入交给 Preview 宿主处理。
- [ ] 增加归档容器视图，将分页 list 适配到现有 `FileItemList` / Reader 模型后交给共享列表；不将现有最多 5,000 项的叶子枚举扩成一次性整包目录树。
- [ ] 在同一 Preview 内实现目录 → 成员预览 → 返回；桌面和手机保留导航栈、选中项与滚动。明确上级、后退、关闭的行为；归档根向上回到外部来源上下文，不把 `a.zip` 当作普通文件夹。
- [ ] 深链接打开成员时恢复父归档上下文；成员间上一项 / 下一项只遍历当前目录的可预览文件，并按实际分页能力提供后续项。
- [ ] 联动 Preview ObjectIdRef、SDK TransferableContentRef、NFSP 映射与 `contentRefString/isTransferableRef/parseSystemOpen`、App Frame 验证、Preview embed、session / parentSource。引用序列化、跨 App 打开和刷新后不得丢失成员路径与版本。
- [ ] 保持文档 ZIP 容器的识别优先级：DOCX / XLSX / EPUB / VSDX 等继续走现有预览；普通 archive 进入目录浏览。不把“是 ZIP”当作覆盖文档格式的最高优先级。
- [ ] 包内内容复用现有预览隔离。若渲染器支持相对资源，基准为包内父目录，禁止越出归档根；首期不支持时明确降级，不把相对地址交给 Desktop 根路径。HTML 不获得同源脚本权限。
- [ ] 下载 / 复制引用作用于当前成员；Open with、分享、导入等按成员能力开放。现有 DFS 分享和写入接口不接收未经支持的虚拟路径，避免错误分享整个包或尝试修改成员。提取成独立对象再分享是后续显式操作。

### T6 — Files 压缩包目录模式（P0，复用 T2 / T5）

- [ ] 增加实际可访问的设置项“将压缩包作为目录浏览”，建议默认开启，沿用 Files 偏好持久化方式。配置只影响 Files 默认打开动作。
- [ ] 开启时双击 / 点击受支持压缩包进入归档根；关闭时按内容打开规则进入 Preview；两种模式均保留显式“预览压缩包”和“下载原包”。
- [ ] 外层列表继续显示 archive 文件类型、压缩大小与原包操作；包内显示成员类型和解压后大小，使用只读 capabilities 控制菜单、拖放、粘贴与快捷键。
- [ ] 地址栏、复制路径、面包屑、前进 / 后退、tab 恢复识别归档边界。父级返回外部目录时选中原包；索引过期时刷新源，不静默跳到新版本同名成员。
- [ ] 首期普通目录搜索和侧栏展开不自动扫描所有压缩包；用户明确进入时才建索引。本机 host realm 后续接入时保留其 sudo 会话，不改写成 DFS 来源。

### T7 — 验证、文档与后续格式（P0 验收 / P1 扩展）

- [ ] 准备可再分发样本：空包、隐式目录、ZIP64、Stored / Deflate、中文 / 特殊名称、同名成员、损坏包、ZipCrypto / AES、不同成员密码、超大文件与大量小文件；记录每种能力的支持 / 不支持结果。
- [ ] 协议测试覆盖路径 parse/format round-trip，尤其真实 `@`、`%40`、`%2F`、双重编码、名为 `content` 的成员、绝对路径、`..`、反斜线、规范化冲突。符号链接 / 硬链接首期不跟随，物化不能逃逸缓存目录。
- [ ] 资源限制以实际读取 / 展开计数执行，不能信任头部声明：索引条目、路径深度、解压字节、字典内存、CPU 时间、磁盘预算、并发。覆盖压缩炸弹、取消、服务重启和缓存驱逐。
- [ ] 一致性 / 权限测试：索引后覆盖原包、hash 过程中修改、同路径新包、用户间同 hash、忘记密码、权限撤销、过期读取引用；固定对象引用和 live 入口的行为分别验收。
- [ ] 跑通 DFS 与 NamedObject 两种源，从 Files、Preview App、embed 深链接打开成员；验证密码交互、返回状态、配置关闭、文档转换、图片 / 文本 / PDF / 音视频、单成员下载。真实 Zone 与真机状态分别记录，mock 不能代替数据链路验收。
- [ ] 一致性验收：用同一组条目在 Files 与 Preview 共享列表中验证行 / 图标 / 手机视图、图标与缩略图、元数据格式、排序、选择、键盘 / 触摸打开和分页滚动；差异只来自宿主可用空间、只读能力和打开目标。共享组件调整同时回归 Files，确保原有文件浏览行为不受影响。
- [ ] 基准比较按需流式方案与临时物化：冷 / 热缓存、首次列目录、首次成员预览、重复打开、PDF 尾读、视频 seek、多用户并发。记录首屏 / 首字节时间、读取压缩字节、实际解码字节、临时写入字节、峰值 RSS / CPU；首次 hash / snapshot 单独计时。先取得基线再冻结性能目标。
- [ ] 更新 NFSP 文档、NFS / Preview README、Files 产品行为、websdk 类型与测试；按仓库约定同步 SDK 构建产物。检查受影响的系统文件 / SDK / App 开发手册。
- [ ] 验证入口：`cargo test -p nfs_server -- --test-threads=1`（在 `src/`）；Desktop 的 `pnpm check`、`pnpm lint` 与定向 datamodel / Playwright；websdk 按实际 package scripts 跑引用与 embed 测试。纯 TODO 阶段不运行这些功能测试。
- [ ] P1 按能力加入 7z / RAR / tar.gz、嵌套包与批量解压；新增格式前验证密码、solid、索引成本和跨平台依赖。需要新依赖或通用组件时按仓库规则先完成方案评审，不能为后续格式提前扩大首期实现。

## 5. 实施前需要定下来的选择

| 选择 | 建议 |
| --- | --- |
| 归档成员属于哪个对象命名空间 | 优先显式归档视图 / 分支，复用 ObjectRef；保留 FileObject `content`，冻结友好路径与稳定引用之间的转换 |
| 稳定引用的产生时机 | 浏览不强制整包 snapshot；复制固定版本引用 / 明确持久使用时对象化并保留，live 来源变化即 stale |
| 首期格式与范围 | ZIP / ZIP64 + Stored / Deflate + 经验证的密码方式；一层归档、DFS 与对象源；其他格式 / host realm 后续 |
| Files 默认行为 | 默认作为目录，设置可关闭；显式 Preview 始终可用 |

完成标准：用户能从一个压缩包进入目录、原地打开成员、返回原位置，复制的成员入口可重新打开；固定版本引用确实绑定可保留的内容；密码成功后同用户再次访问可复用，其他用户不获得解锁权限；整个过程不默认解压整包，写入边界、源版本变化、缓存和资源上限有可复现验证。

本次仅新增设计 TODO，未修改功能代码、协议或依赖，未运行功能测试；因此未改变现有内置 skill 的使用方式。实施时按 T7 检查相应手册联动。
