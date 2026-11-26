# BuckyOS 剪贴板管理与拖拽支持规范

**v1 草案 · 2026-11-21**

适用于 BuckyOS Web Desktop、桌面内置 App、通过 App Frame 接入的 Web App，以及它们与本机应用之间的复制、剪切、粘贴和拖放。下文的“必须”用于验收，“应”给出有理由可以偏离的默认行为，“可”表示可选能力。文中的 `transfer.*` 和第 2 节登记的格式是拟议协议，现有实现列在第 1 节。

剪贴板和拖放按同一套内容协议处理：发布方同时写入标准内容和自定义内容；接收方先处理自己认识的自定义内容，再按顺序处理标准内容；需要在 App 之间共用的结构，由本规范登记为 BuckyOS 标准内容。同一个文件拖入、粘贴或经选择器交给同一落点，应使用相同的权限检查和导入逻辑。**剪贴板以本次粘贴的内容为准，拖放以用户实际释放的目标为准。** 接收端依据意图选择一种表示执行一次操作。引用、文件字节和移动原件的权限分别处理。

## 1. 当前桌面的起点

| 现有行为 | v1 要解决的问题 |
|---|---|
| Files、AIWS、MessageHub 等内置 App 是同一桌面文档里的 React 面板。其他 Web App 通常位于各自的 App 子域。现有 App Frame 支持打开内容和系统选择器。 | 同一桌面内提供跨 App 内容交付；不同子域即使属于同一 Zone，也按不同 origin 处理。[App 注册表](../../src/frame/desktop/src/app/registry.tsx) · [Web App 宿主](../../src/frame/desktop/src/app/web-frame/WebAppFramePanel.tsx) |
| Files 拖拽时把随机 token 写入 `application/x-buckyos-file-items`，选区保存在 Files 模块内存，另一个 Files 窗口的落点能找回这些项。Ctrl+C/X 只记在本窗口的状态里，不写系统剪贴板，其他窗口和 App 粘贴不到。AIWS 和聊天的文件落点读取的却是浏览器 `File`。 | 把 Zone 引用登记为 BuckyOS 标准内容（§2.2），随剪贴板和拖放一起传递，目标直接消费有类型的引用；本机文件继续走现有文件导入链路。[Files 拖放与剪贴板](../../src/frame/desktop/src/app/filebrowser/FileBrowserView.tsx) · [聊天附件](../../src/frame/desktop/src/components/conversation/input/attachmentDraft.ts) |
| AIWS 能复制画布对象，但对象只保存在窗口内存，系统剪贴板里只写一个 `aiws-clipboard:` 开头的标记串；画布快捷键遇到未识别的纯文本，可能粘出上一次的内存对象。表格能同时写 HTML 和 TSV，HTML 用 `data-aiws-*` 属性携带字段定义和精确值。打开方式里的“导入到 AI Workspace”已有显式的文件交付入口。 | 保留对象复制的保真度和各类编辑器的焦点规则，把标准表示改为可读内容，修正旧对象回退；把表格标记升级为 BuckyOS 标准内容；统一拖入和显式导入的结果。[画布](../../src/frame/desktop/src/app/aiworkspace/ui/canvas/CanvasView.tsx) · [画布剪贴板](../../src/frame/desktop/src/app/aiworkspace/ui/canvas/clipboard.ts) · [表格序列化](../../src/frame/desktop/src/app/aiworkspace/ui/import/copyTable.ts) · [表格解析](../../src/frame/aiworkspace/import-table/html.ts) · [显式导入](../../src/frame/desktop/src/app/content/OpenWithSheet.tsx) |
| 复制路径、Block 或记录链接、DID 时只写文本，Files 给出的 `cyfs://` 路径没有接收端识别。文本编辑器拖动标签页时把 JSON 写进 `text/plain`，拖到输入框会插入这段 JSON。AIWS 的标签页、对象树、目录行、放映步骤等内部排序各用自定义 MIME。 | BuckyOS 地址在读端可还原为引用；`text/plain` 只放给人读的内容；App 内部排序不进格式登记。[路径文本](../../src/frame/desktop/src/app/filebrowser/data/urls.ts) · [文本编辑器](../../src/apps/text_editor/src/ui/App.tsx) |

## 2. 传什么

一次用户选择可以有多个**项**，每一项又可以有几种**表示**。选中三份文档是三个项；一张表同时提供 HTML 和 TSV，是一项的两种表示。接收端必须保留多选顺序，按项选取自己能处理且符合目标语义的表示。平台只给出一组并列格式时，先按一次选择处理，不能仅凭 MIME 数量推断项数。Windows 的多格式数据对象、Web 的 `ClipboardItem` 和 Android 的内容引用均采用了这一思路。[Windows Shell 数据交换](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios) · [W3C Clipboard API 工作草案](https://www.w3.org/TR/clipboard-apis/#clipboard-item) · [Android 剪贴板](https://developer.android.com/develop/ui/views/touch-and-input/copy-paste)

每种表示属于以下三层之一，接收端按 §2.3 的顺序在层间选择。

| 层 | 定义 | 示例 |
|---|---|---|
| Web 与系统标准内容 | 浏览器和本机应用通用的格式。每次复制或拖出至少提供一种给人读的表示。 | `text/plain`、`text/html`、`text/uri-list`、`image/png`、浏览器提供的 `Files` |
| BuckyOS 标准内容 | 由 §2.1 登记，类型、序列化器和解析器在 Web SDK 中提供。任何 BuckyOS App 都可以写和读；不认识它的 App 忽略它，使用同一次写入的标准内容。 | Zone 内容引用列表、带类型表格、BuckyOS 地址（§2.2） |
| App 自定义内容 | 某个 App 自己的结构，只有声明支持的目标才解释。可以是自包含数据，也可以经传输句柄指向桌面记录（§3.4）。 | AIWS Block、组和连接线的复制 |

| 用户选中的内容 | BuckyOS 内部交付 | 给系统剪贴板或非 BuckyOS App 的表示 |
|---|---|---|
| 文本、富文本、链接 | 保留文本、受支持的格式和链接语义。明确的“粘贴为纯文本”优先。 | 文本用 `text/plain`，富文本另给经清理的 `text/html`。复制链接应给可读 URL，并另给 `text/uri-list`。 |
| 表格 | 带类型表格（§2.2）：字段类型、选项和精确值随 HTML 传递。 | HTML 表格和纯文本 TSV。 |
| 图像 | 可携带图片表示或用户选中的文件。 | 截图优先给可用的 `image/png`。仅在有意义时附说明文本。 |
| Zone 文件、文件夹、对象 | Zone 内容引用列表（§2.2），复用 SDK 的 `ZoneContent`、`TransferableContentRef`、`ContentHints`。路径标识用户选中的位置；已知的 `objId` 或受支持的版本可标识选中时的内容。 | `text/plain` 给显示名称，`text/uri-list` 和 `text/html` 链接给 `cyfs://` 或 `obj://` 地址。用户明确“复制路径 / 链接”时，`text/plain` 给相应文本。“导出文件”才交付可用文件内容，无法拖出到本机时提供下载。 |
| 本机文件、目录 | 浏览器交付的 `File` 及相对目录结构，沿用 `LocalFile`、`LocalFolder`，由目标决定预览、导入或加入草稿。 | 保留本机原生文件通道。目录枚举不可用时给出选择文件夹入口；不能静默丢失层级或文件。 |
| AIWS Block、组、选区及其他专有对象 | App 自定义内容。源 App 声明版本和适用范围，如同一 Workspace。匹配的目标还原结构，其他目标可选择可读摘要、表格或已有链接。 | `text/plain` 给人能读的内容，例如标题、文字或选区内容，必要时另给 `text/html` 或 `image/png`。私有结构可留在桌面会话中（§3.4），不能写成文本标记。“复制链接”才输出相应链接。 |

### 2.1 格式登记与载体

- **命名。** BuckyOS 标准内容使用 `application/vnd.buckyos.<名称>+json`，名称由本节登记。系统 App 的自定义格式使用 `application/vnd.buckyos-<app>.<名称>+json`，第三方 App 使用自己的厂商前缀。不再使用 `x-` 前缀。格式名一律小写，浏览器会把 `DataTransfer` 的类型名转成小写。接收端只按登记表解释 BuckyOS 标准内容，不凭前缀猜测含义。[RFC 6838 媒体类型树](https://www.rfc-editor.org/rfc/rfc6838#section-3)
- **版本。** JSON 格式的顶层带 `"v"`，HTML 嵌入的标记同样带版本。接收端遇到不认识的主版本按不支持处理，降到下一层（§2.3），不尝试部分解析。
- **同名。** 同一格式在原生拖放的 `DataTransfer`、`copy`/`cut`/`paste` 事件的 `clipboardData`、异步剪贴板的 `ClipboardItem` 以及 `transfer.v1` 消息（第 5 节）中使用同一名字；只有异步剪贴板按浏览器要求加 `web ` 前缀。
- **载体。** 专用 MIME 是首选载体，但跨源、跨浏览器或浏览器不支持 Web 自定义格式时，它会被丢弃，例如 WebKit 只向同源页面提供自定义类型，而 App 位于各自的子域。因此 BuckyOS 标准内容在 `text/html` 里另有一份嵌入表示（链接或 `data-buckyos-*` 属性），带类型表格只用嵌入表示。已协商 `transfer.v1` 的不同源 App 之间还可以经 Shell 交付。接收端依次查找专用 MIME 和 HTML 嵌入表示，再按标准内容处理。[WebKit 自定义 MIME 的同源约束](https://webkit.org/blog/8170/clipboard-api-improvements/) · [Chromium Web 自定义格式](https://developer.chrome.com/blog/web-custom-formats-for-the-async-clipboard-api)
- **实现位置。** BuckyOS 标准内容的类型、序列化器和解析器放在 Web SDK，内置 App 和第三方 App 共用一份实现。新增格式时，先改本节登记表，再改 SDK。

| 格式 | 层 | 内容 | 写入方 / 接收方 |
|---|---|---|---|
| `application/vnd.buckyos.content-refs+json` | BuckyOS 标准 | Zone 内容引用列表 | Files、预览、压缩包浏览、HomeStation、聊天附件、AIWS 资源 Block / 聊天附件、AIWS 画布与目录、Files、打开方式 |
| `text/html` 中的 `data-buckyos-table`、`data-buckyos-value` | BuckyOS 标准 | 带类型表格 | AIWS 表格及其他表格视图 / AIWS 表格导入、电子表格类 App |
| `text/plain`、`text/uri-list` 中的 `cyfs://`、`obj://` | BuckyOS 标准（文本形式） | BuckyOS 地址 | 复制路径、Zone 引用的标准表示 / 接受引用的落点 |
| `text/html` 中的 `data-buckyos-ref` | BuckyOS 标准（嵌入表示） | Zone 内容引用列表的链接标记 `{"v":1,"kind":"file"\|"folder"}` | 同 Zone 内容引用列表 |
| `application/vnd.buckyos.transfer+json` | 传输句柄 | `{"v":1,"offerId":"…"}`，指向桌面传输记录；剪切另带 `"cut":true` | 剪切、体积大或延迟生成的表示、需要经 Shell 取回的私有选区（§3.4） |
| `text/html` 中的 `data-buckyos-transfer` | 传输句柄（嵌入表示） | 同上，写在包住本次 HTML 的元素上 | 同上；供只能读到 Web 标准格式的粘贴路径使用（§3.4） |
| `application/vnd.buckyos-aiws.clip+json` | App 自定义 | AIWS Block、组、连接线的复制 | AIWS 画布；体积可能较大且依赖源 Workspace，v1 经传输句柄取回 |

### 2.2 BuckyOS 标准内容 v1

**Zone 内容引用列表**（`application/vnd.buckyos.content-refs+json`）。值为 `{"v":1,"items":[…]}`，`items` 按用户选择的顺序排列，每项是 SDK 的 `ZoneContent`。引用只说明用户选了什么，不授予读取权；目标用自己的有效身份读取、写入或导入（第 6 节）。这个格式表达复制和添加引用；剪切还需要传输句柄（§3.4、第 6 节）。本机文件不进这个格式，继续使用浏览器的 `Files`，或经 `transfer.read` 结构化克隆。

例如，Files 交付已选 PDF 时写入以下内容。目标要跟随文件后续修改时使用路径，要导入选中时的内容时使用已知快照。缺少快照且源已变化时，导入方应重新确认版本或提示重选。名称、扩展名、MIME 和大小都是提示，读取时仍须验证。[系统选择器数据契约](BuckyOS%20系统对话框实现方案.md) · [Web SDK 内容类型](https://github.com/buckyos/buckyos-websdk/blob/main/src/content.ts)

```json
{
  "v": 1,
  "items": [
    {
      "origin": "zone",
      "kind": "file",
      "source": { "kind": "cyfs-path", "path": "cyfs:///home/alice/notes.pdf" },
      "hints": { "name": "notes.pdf", "mime": "application/pdf" }
    }
  ]
}
```

同一次写入的标准表示：`text/plain` 每行一个显示名称；`text/uri-list` 每行一个 `cyfs://` 或 `obj://` 地址（路径各段按 URI 转义）；`text/html` 是同序的 `<a href="cyfs://…" data-buckyos-ref='{"v":1,"kind":"file"}'>名称</a>` 列表，`data-buckyos-ref` 标明这是引用列表的嵌入表示并给出版本和种类（文件或文件夹）。专用 MIME 被丢弃时，BuckyOS 接收端可从带该标记的链接还原引用，此时只有地址、种类和作名称提示的链接文字，没有 `objId` 和其他提示；不带标记的 `cyfs://` 链接按 BuckyOS 地址处理。这些地址、名称和 `objId` 会进入系统剪贴板，可能被剪贴板历史、跨设备同步或其他本机应用读到。路径不是凭证，读取仍需权限，v1 接受这一点；凭证、签名地址和未选择的条目仍按第 6 节禁止写入。

**带类型表格。** 写在 `text/html` 的 `<table data-buckyos-table="…">` 上，属性值是 JSON `{"v":1,"fields":[…]}`，每个字段给 `name`、`type`，按类型另给 `scale`、`currency` 或 `options`（选项标签列表）。每个 `<td>` 可带 `data-buckyos-value`，值为该单元格的 JSON 线上值。字段类型取 AIWS 标准表的类型集合，作为系统级表格的字段类型标准：`text`、`boolean`、`number`、`decimal`、`date`、`datetime`、`select`、`multi_select`、`currency`、`json`。值的编码与 AIWS 核心一致，另有三条交付规则：
- 选项写标签而不写选项 ID。
- 引用字段（`object_ref`）离开源 Workspace 就没有意义，以 `text` 字段和显示文字交付。
- 公式不复制，单元格留空。

同一次写入的 `text/plain` 是带表头的 TSV。这个格式取代现有的 `data-aiws-table`、`data-aiws-value`。[核心值类型](../../src/frame/aiworkspace/core/src/value.rs)

**BuckyOS 地址。** `text/plain` 或 `text/uri-list` 中每行都是 `cyfs://<path>` 或 `obj://<objId>[/<inner path>]` 的文本，是 Zone 引用的文本形式，语法与 SDK 的 `contentRefString`、`parseObjRefString` 一致。文本输入焦点里仍按文本插入。接受引用的落点（画布、附件区、目录）可以让用户选择“插入为文件引用”或“作为文本”，未经选择不能读取或导入。地址无效、路径越界或无权读取时按文本处理，并说明原因。DID、Workspace 链接和分享链接暂不登记，按普通文本和 URL 处理。

### 2.3 写入与读取顺序

**写入。** 每次复制、剪切或拖出：

1. 至少写一种给人读的标准表示。`text/plain` 只放给人读的内容，不放标记串、JSON 或内部 ID。
2. 有对应的 BuckyOS 标准内容时一并写入，例如 Zone 选区写引用列表，表格写带类型的 HTML。
3. 按需追加 App 自定义内容。小而自包含的内容直接写入；剪切、体积大或延迟生成的内容，以及需要跨源取回的私有选区，写传输句柄（§3.4）。
4. 同一项的各层表示描述同一内容，在同一次写入中一并生成（§3.1）。

App 内部的拖拽，如排序、形状库到画布，只用 App 自定义 MIME，不写 `text/plain`，也不进登记表（第 4 节）。

**读取。**

1. 明确意图优先，例如“粘贴为纯文本”只取 `text/plain`。
2. 焦点和落点决定可接受的格式（§3.3、第 4 节）：文本输入框只接受文字和它认领的附件，对象视图接受对象和引用。
3. 在可接受的格式中依次尝试：自己认识的 App 自定义内容 → BuckyOS 标准内容（先专用 MIME，再 HTML 嵌入表示）→ Web 富格式（`Files`、`text/html`、图片）→ `text/plain`（再识别 BuckyOS 地址、TSV、SVG、draw.io 等）。
4. 某一层失败（版本不认识、范围不符、解析失败、句柄失效或权限被拒）时只降到下一层，不能回落到内存记录或上一次复制的内容。
5. 每项只按选中的一种表示执行一次操作，同一项的其他表示不再产生对象。

## 3. 剪贴板的读写与生命周期

1. **复制与剪切。** 快捷键和系统菜单走真实的 `copy`、`cut` 事件，处理器在事件期间写 `clipboardData`。工具栏和右键菜单复用同一序列化器，可在前台用户手势中调用 `navigator.clipboard.write()`。同一项的可用表示应一次写入一个 `ClipboardItem`；耗时的表示可用 Promise 生成。写入的内容按 §2.3。写入失败要告知用户实际复制到了哪里，不能报告系统复制成功。[WebKit Async Clipboard](https://webkit.org/blog/10855/async-clipboard-api/)
2. **普通粘贴。** 真实 `paste` 优先使用事件携带的内容，在返回事件循环前提取文本、文件或所需句柄。显式“粘贴”按钮可在安全上下文和用户操作中尝试异步读取；被拒绝时，保留焦点并给出系统粘贴快捷键或菜单入口。Shell 不在启动、聚焦、悬停时预读系统剪贴板，不向 iframe 提供“读取当前剪贴板”的宿主代理。[W3C 剪贴板事件](https://www.w3.org/TR/clipboard-apis/#clipboard-event-api) · [浏览器权限与 iframe 委派](https://web.dev/articles/async-clipboard#permissions_policy_integration)
3. **焦点优先。** 在输入框、富文本、表格单元格编辑器中，由最内层活动编辑器决定如何粘贴；弹窗打开时归弹窗。选中对象且没有文本插入点时，才由对象视图处理。处理器只在认领操作后调用 `preventDefault()`；嵌套容器还应停止或识别已处理事件。快捷键、工具栏和菜单对同一“粘贴”命令使用同一规则。
4. **自包含内容与传输句柄。** BuckyOS 标准内容和小而自包含的自定义内容直接写在剪贴板或拖放数据里。外部复制会把它们与标准表示一起覆盖，因此无需另外证明它们对应本次粘贴。只有以下几类内容由桌面保留一份会话记录：剪切、体积大或延迟生成的表示，以及需要经 Shell 跨源交付的私有选区。写入时附带传输句柄 `application/vnd.buckyos.transfer+json`，其值仅含版本和不透明的 `offerId`，如 `{"v":1,"offerId":"…"}`；剪切的句柄另带 `"cut":true`，记录已不在（刷新、另一个标签页）时接收端仍能按第 6 节提示原剪切已失效，与内容格式和标准表示在同一次写入中给出。同文档原生拖放或剪贴板事件能保留格式时使用原名；支持 Web 自定义异步剪贴板的浏览器使用带 `web ` 前缀的格式名。同一次写入有 `text/html` 时，句柄另以 `data-buckyos-transfer` 属性嵌入其中：Chromium 的 `copy` 事件写入的自定义类型无法被异步剪贴板读到，Web 自定义格式也读不到 `paste` 事件，工具栏“粘贴”和快捷键“复制”之间只能靠这份嵌入表示衔接。嵌入的句柄和专用 MIME 一样会被外部复制覆盖。异步写入前按能力选择类型；私有格式导致整次写入失败时，若仍有用户激活，可只写 Web 标准表示（含 HTML 嵌入表示），否则提示重新复制。同步事件需要的标识应预先备好，不能在 `await` 后补写原事件。[Chromium Web 自定义格式](https://developer.chrome.com/blog/web-custom-formats-for-the-async-clipboard-api) · [WebKit 自定义 MIME 的同源约束](https://webkit.org/blog/8170/clipboard-api-improvements/)
5. **外部复制一定覆盖原意。** 本条约束传输句柄；直接写入的内容随剪贴板一起被覆盖。只有本次粘贴携带有效句柄、范围匹配且权限允许时，才能用桌面记录还原对象。缺句柄、过期、纯文本碰巧相同、读权限被拒或只有 `keydown`，都不能证明这次粘贴指向记录。例：复制 Block，再从网页复制“周五发版”，回到 AIWS 必须处理这句话或提示当前落点不收文本。需要取回桌面私有选区时，可单列“粘贴桌面对象…”并显示待粘贴对象和目的地。
6. **保留范围。** v1 桌面记录限同一顶层桌面实例、登录用户和 Zone。普通复制可重复粘贴；对象剪切在移动成功后消费，取消剪切时解除标记。退出登录、切换用户或 Zone、桌面重载时清理；源文档关闭后使依赖该文档的延迟表示失效。直接写入的 BuckyOS 标准内容不受此限：刷新后、在另一个标签页或同 Zone 的另一个桌面实例里仍可粘贴，目标按自己的权限读取。系统剪贴板由系统管理，可能进入历史或跨设备同步；其中的 Zone 路径、`objId` 和名称按 §2.2 处理。桌面记录只保留当前操作所需数据，不默认记录历史或把内容同步到服务端。[Windows 剪贴板格式及历史控制](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)

## 4. 拖放通道与落点

| 来源 → 目标 | v1 通道 | 降级或边界 |
|---|---|---|
| 系统 App ↔ 系统 App | 同桌面文档可用原生 HTML 拖放，直接携带 BuckyOS 标准内容；剪切或私有选区使用同一桌面传输记录。双方使用同一内容契约。 | 识别并消费一次即可。只理解 `Files` 的旧目标需显式接入 Zone 引用。 |
| 系统 App ↔ iframe App | 已协商能力的窗口使用第 5 节的 Shell 路由。两个方向都必须支持。 | 未接入的 App 仅使用它本身能接收的标准格式或显式选择器。 |
| iframe App ↔ iframe App | 同源且约定格式时可用原生拖放。不同源、同一桌面内的协作 App 经 Shell 路由。 | 不把浏览器对跨源兄弟 iframe 的原生投递作为接入条件。 |
| 本机 / 外部网页 ↔ App、独立浏览器标签页 | 使用浏览器实际提供的 `Files`、文本、HTML、URL 或系统剪贴板。可导出的资源也可经下载、打开方式或文件选择器交付。 | 仅凭传输句柄无法在另一个桌面实例取回私有内容；自包含的 BuckyOS 标准内容可以，目标按自己的权限读取。本机文件落到 iframe 时由收到原生事件的文档处理。 |
| App 内部布局 | 画布移动、表格排序、窗口标题栏拖动等保留本地 Pointer Events 或已有的排序实现。使用原生拖放的内部排序只写 App 自定义 MIME。 | 本地移动只更新布局，拖到别的 App 不产生内容；对外拖出选区应提供可辨识的手柄或命令。 |

**为什么要区分通道。** Linux 有界面浏览器最小页面对照中，Chromium 156 的同源兄弟 iframe 能在 `drop` 读到纯文本和自定义字符串，跨源兄弟及跨源父子两个方向均未向目标投递拖放事件。Firefox 157 在跨源兄弟和父→子时能读到这两种值，子→父时可收到 `drop`，但读取值为空。这是同一顶层页面内的字符串实验，不能外推为 Safari、本机文件和所有部署组合的结论。[Chromium 落点判断](https://chromium.googlesource.com/chromium/src/+/b45f8019a5003cb6ddb0cbffe1034bd029f88809/third_party/blink/renderer/core/page/drag_controller.cc#456) · [Mozilla 关于子框架向父框架拖放的说明](https://bugzilla.mozilla.org/show_bug.cgi?id=1727176)

**原生事件。** 源在 `dragstart` 同步写入表示及 `effectAllowed`。目标在 `dragenter/dragover` 根据 `types`、当前落点和本地能力决定是否接收，对可接收的落点同步 `preventDefault()` 并设置 `dropEffect`。正文与文件应在 `drop` 读取。嵌套落点从内向外认领；例如可编辑目录认领文件后，外层画布必须跳过同一次事件。拖过窗口不会读取文件、建表、改布局或开始上传。[HTML 拖放数据模型](https://html.spec.whatwg.org/multipage/dnd.html#the-drag-data-store)

**操作反馈。** 可接收的落点显示目的地和业务动作，如“移动到项目目录”“添加到附件草稿”“预览”。浏览器的 copy、link 等光标只补充传输效果。无法接收时显示禁止态。多选须显示数量。释放时按最终坐标、最上层可见目标和当时的权限复核。聊天草稿、预览和未确认的导入向导均不能消费剪切记录；发送仍由用户决定。[Apple 拖放交互指南](https://developer.apple.com/design/human-interface-guidelines/drag-and-drop)

## 5. 桌面与 App Frame 的交付契约（提案）

Shell 管理一次操作的来源、目的地和生命周期；源 App 提供所选内容及可用操作；接收 App 判断落点并执行业务写入。内置 App 可直接调用宿主接口，iframe 复用现有 `buckyos.app-frame` v1 信封、握手 nonce、`id/replyTo` 及能力协商。拟议能力名为 `transfer.v1`，双方均声明后才启用。现有 `content.open` 继续表示“打开”，`sysdlg/select` 继续表示用户主动选择文件。[App Frame 宿主](../../src/frame/desktop/src/app/web-frame/WebAppFramePanel.tsx) · [App Frame 协议说明](../../src/apps/text_editor/readme.md) · [系统选择器](BuckyOS%20系统对话框实现方案.md)

| 拟议消息 | 方向 | 必要信息及接收规则 |
|---|---|---|
| `transfer.offer` | 源 → Shell | 登记高熵随机 `offerId`、用途（复制、剪切或拖拽）、选区代次、有序项、格式、允许的操作和适用范围。Shell 拒绝标识碰撞或覆盖，按真实来源绑定窗口、文档代次和作用域。登记只准备选区。 |
| `transfer.probe` | Shell → 候选目标 | 给 `operationId`、探测序号、有序项列表和窗口内 CSS 坐标或“当前焦点”。每项仅给本次选择内的序号 `itemKey`、种类、格式与允许的操作。应答给本地 `targetId`、接收的 `itemKey`、格式、操作和显示给用户的动作。悬停不交付名称、路径、业务对象 ID、正文或文件。 |
| `transfer.read` | Shell → 源 | 用户落下或在 Shell 的粘贴命令中选定目标后，按绑定的 `offerId`、项及格式请求数据。已在宿主保存的内容可直接使用。引用按 SDK 类型交付，本机文件可经结构化克隆，避免将文件编码为 JSON/Base64。 |
| `transfer.deliver` | Shell → 选定目标 | 给 `operationId`、`targetId`、操作、最终坐标或焦点，以及用户选中的项及表示。目标复核落点、版本与权限，按同一 `operationId` 幂等提交；回执须区分已完成、已接收待确认、取消和失败，并列出各项结果。 |
| `transfer.status`、`transfer.cancel` | Shell ↔ 参与方 | 根据 `operationId` 查询最终结果或撤回尚未提交的操作。回执超时进入待核对状态；查询与重发使用原操作 ID。已经提交的变更通过业务撤销处理。 |

消息中的格式使用 §2.1 登记表中的名字，不带 `web ` 前缀；目标在 `transfer.probe` 应答中按 §2.3 的顺序选择格式。

Shell 为每次新的粘贴或拖放生成 `operationId`，交付前固定其来源记录、最终目标文档、落点和操作。多次复制粘贴各有自己的 ID，重试同一次交付则复用原 ID，以 `(operationId, itemKey)` 对账。响应沿用 App Frame 的 `…Result` 形式。未知版本或没有共同表示返回 `unsupported`，过期或文档已换代返回 `stale`，权限拒绝返回 `denied`。正常取消单独记录。释放位置变化时必须按最终坐标重新探测；拒绝、超时或过期应答均不能触发向上一个落点交付。

**跨源手势必须有宿主可以确认的落点。** 一个可实施的 v1 路径，是在聚焦窗口的选区旁或窗口工具区提供由 Shell 绘制的“拖到其他 App”手柄。源预先登记选区；用户在手柄按下后，由接收按下事件的 Shell 元素捕获指针。移动时 Shell 根据实际窗口层叠、裁剪、滚动和缩放计算命中窗口及局部坐标，再让目标按自己的 DOM 定位落点。用户释放时 Shell 只向最终命中的目标交付。遮挡窗口不可穿透；遮罩只在本次手势中生效。内置源可把相同机制直接接到文件行或对象手柄上。[W3C Pointer Events 指针捕获](https://www.w3.org/TR/pointerevents3/#pointer-capture)

iframe 也可从内部手柄捕获指针并向 Shell 报告预览位置。此时指针流属于源文档；仅增加父页面遮罩不会把已捕获的事件移交给父页面。来自 App 的坐标、`pointerup` 或自报的 `isTrusted` 只能用作提示。若宿主没有独立观察到目标释放，须让用户在 Shell 中点击选定的目的地后再交付私有内容。同理，iframe 可自行粘贴浏览器交给它的标准内容；它发来的“我收到了粘贴”不能换取 Shell 当前选区。私有内容跨源交付使用用户明确操作的宿主落点或“粘贴桌面对象…”命令。

原生、Shell 和本地布局手势只能有一个拥有本次操作。Esc、`pointercancel`、释放前丢失捕获、候选目标消失、源或目标导航，都必须撤掉预览并放弃未提交交付。正常 `pointerup` 后自动发生的 `lostpointercapture` 只清理手势资源。独立标签页、未握手或不支持该能力的 App 可以使用标准粘贴、文件选择器或“打开方式”。

## 6. 写入、移动与权限

| 操作 | 可见结果与提交条件 |
|---|---|
| `copy` | 目标创建副本、附件草稿或导入结果，源继续可用。跨 App 默认使用复制；应用必须说清楚独立副本与共用数据源的差别。 |
| `link` | 目标保存对原内容的引用，例如向集合添加成员。显示“添加引用”；链接失效、权限变化或对象被删除时由目标显示对应状态。 |
| `move` | 同一数据服务能事务性移动时，保留对象身份并由该服务一次提交。不同服务只有在能持久化幂等操作、确认目标已提交、校验源版本并安全完成源端变更时才可提供移动。其他组合只能明确提供复制或引用。 |

最终操作从源允许、目标支持、当前身份可执行三者的交集中选取。修饰键依照所在平台表达用户的偏好，光标、目标提示和实际提交须一致。用户明确要求移动而落点只支持复制时，须显示可选动作，让用户选择复制或取消。将剪切项作为引用或附件加入草稿时，原件继续保留。

**对象剪切采用延迟移动。** Files 条目或 AIWS Block 在剪切时标记待移动。执行移动前，协调方须以 `(offerId, itemKey)` 原子保留该项，只允许一个操作在途。两个窗口同时粘贴时，第二个窗口应提示该项正在移动；回执不明时查询原操作。确定失败或取消后释放保留，逐项成功后才消费相应项。退出手势只移除预览；已提交操作仍按业务记录对账。剪切的句柄失效而剪贴板里仍有引用列表时，提示原剪切已失效，让用户选择复制或取消，不能静默改成复制。

失败、取消、权限撤销以及回执丢失都不能触发源删除。确认目标已写入且源端仍保留原件时显示“已复制，原件仍在”，按原操作 ID 恢复或核对。普通 iframe 回执、`dragend` 和 `dropEffect=move` 均不足以证明后端提交。富文本和输入框保留平台常规剪切行为，将选区写入剪贴板后移除文字，并由编辑器提供撤销。[Windows 延迟删除与移动确认](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios#handling-delete-on-paste-operations)

**批量与冲突。** 目标支持事务时，批量移动应整体成功或回滚。允许部分复制或导入时，先显示能接收的数量；完成后逐项标记成功、跳过、失败或待核对。重试只处理未完成项。同名文件按目标原有的重命名、替换或跳过规则处理；悬停、粘贴和修饰键都不能代替覆盖确认。异步导入应显示进度和可取消阶段。

**权限与数据最小化。**

- 源、目标 App 和窗口身份由 Shell 从注册表取得。每次握手由 iframe 文档创建独享的 `MessageChannel`，用精确的 `targetOrigin` 将一个端口交给 Shell。Shell 检查建连消息的 `event.origin`、`event.source` 和当前握手，随后绑定该端口与文档代次。传输消息在该端口上按 schema 校验，导航、关闭或重新握手时废弃旧端口和未交付请求。同 origin 重载也必须重新绑定，避免将已选私有内容发送到复用了同一 WindowProxy 的新文档。剪贴板标识和操作 ID 都不能单独授予读取权。[HTML 跨文档消息与端口](https://html.spec.whatwg.org/multipage/web-messaging.html#channel-messaging)
- 用户交付引用后，源端仍要有权提供选区，目标用自己的有效身份读取、写入或导入。NFSP 当前拒绝 App/Agent 委托身份直接访问文件面；此时应返回无权读取，可引导用户走现有的显式分享或导出流程。Shell 不代借桌面用户身份取回私有文件。创建分享、扩大受众或发送聊天消息需要各自的用户动作。[NFSP 鉴权](../../src/frame/nfs_server/src/auth.rs) · [NFSP 文件与分享能力](../../src/frame/nfs_server/README.md)
- 公开表示、剪贴板标识、日志和测量数据均不得夹带会话凭证、Cookie、签名下载地址或未选择的条目。公开表示也不得借 HTML 属性或 URL 查询参数暗藏这些信息。诊断记录格式、数量、结果码等必要信息，默认省略剪贴板正文和路径。
- 所有来源都校验格式和大小。HTML/SVG 经允许列表清理后呈现；链接检查协议；相对路径拒绝越界和非法名称；文件类型以可验证信息为准。大文件、目录、压缩数据在目标现有的读取与导入限额内按需处理。目标拒绝时不得悄悄换成权限更宽的读取方式。

## 7. 接入案例与可访问性

| 用户意图 | 接收方应呈现的结果 |
|---|---|
| 从 Files 将 Zone PDF 拖到 AIWS，随后把同一文件粘到同一画布 | 目标有权读取时，两条入口共用 PDF 的“作为文件 / 导入页面”选择。自由画布的拖入落点是鼠标位置，快捷键粘贴使用默认位置。取消向导不修改画布。 |
| 在 Files 复制文件，刷新桌面或换到另一个标签页，再粘到 AIWS | 粘贴使用剪贴板里的 Zone 引用列表，结果与拖入相同，目标按自己的权限读取。若原先是剪切，句柄已随刷新失效，按第 6 节提示复制或取消。 |
| 从 Files 将文件拖到 MessageHub；从 Finder 把同名文件拖到同一对话 | Zone 文件交付引用，本机文件交付 `File`。聊天按各自权限加入附件草稿；失败逐项提示，成功也不自动发送。 |
| 在 AIWS 复制表格，粘到电子表格、另一画布或正在编辑的单元格 | 电子表格可读 TSV/HTML；画布按带类型表格建表，字段类型和选项不丢失；单元格编辑时由编辑器处理。若将来支持矩形区域粘贴，应由网格声明并消费，画布不能同时建表。 |
| 从 iframe App 将对象交给另一个 iframe App，或将对象交给 Files | Shell 显示实际目标 App 和操作。双方有共同表示并具备权限时只交付一次；若目标不认识对象，可给已声明的文本或文件表示，也可表明无法放入。 |
| 复制普通文本、URL 后粘到 AIWS 空白画布 | 建议分别创建文本块和带链接的文本块。尚无相应插入能力的版本应说明该落点支持的格式，保证此前的对象不会出现。 |
| 把一段 `cyfs://` 路径文本粘到 AIWS 空白画布或聊天输入框 | 画布让用户选择“插入为文件引用”或“作为文本”；聊天输入框按文本插入。 |

本规范中的内容传输必须同时提供**键盘入口和通过点击或轻触即可完成的入口**。例如选中文件后用“复制 / 移动到…”选择目录，或在目标选择“添加附件 / 从 Files 选择”。只有快捷键不足以满足单指无拖动需求。保留平台常用的 Ctrl/Cmd+C、X、V；Esc 取消并恢复焦点。触屏入口不得只能靠悬停发现，拖动不得妨碍普通滚动。操作名称、目标、项数、完成或失败状态应可被读屏获取，图标、光标或颜色只作辅助。[WCAG 2.2 拖动动作](https://www.w3.org/WAI/WCAG22/Understanding/dragging-movements.html) · [W3C G219 可点击替代](https://www.w3.org/WAI/WCAG22/Techniques/general/G219.html)

## 8. 验收与接入顺序

| 验收场景 | 通过条件 |
|---|---|
| 系统 App ↔ 系统 App、系统 App ↔ iframe、iframe ↔ iframe | 两个方向均验证。使用真实 App 子域或等价不同 origin。在 Chromium 原生跨源 `drop` 不到达时，接入双方仍能通过 Shell 交付；未接入的目标有可解释的降级结果。 |
| 真实剪贴板 | 快捷键、浏览器菜单和 App 工具栏分别测。复制 Block 后外部复制文本，粘贴只使用本次文本；私有 MIME 被剥离、系统读取被拒、桌面刷新时结果也符合第 3 节。复制表格到系统外电子表格仍可读。Files 复制后，在另一个 Files 窗口、AIWS、聊天及刷新后的桌面都得到同样的引用；专用 MIME 被剥离时能从 HTML 链接还原引用。复制 Block 后系统剪贴板里是可读文本。 |
| 格式与版本 | 不认识的主版本、损坏的 JSON、越界或无效的地址都降到下一层，不产生对象。`text/plain` 中不出现标记串、JSON 或内部 ID。App 内部排序拖到输入框不插入内容。 |
| 焦点和嵌套落点 | 富文本、单元格、空白画布分别得到预期结果；拖文件到画布中的目录编辑器只增加一个目录项；HTML+TSV 或图片+文字说明不会产生重复对象。 |
| 真实文件与引用 | 分别从本机文件管理器和 BuckyOS Files 操作。验证单文件、多文件、目录、移动后失效的路径、受限引用及大文件。悬停期间无读取和上传。 |
| 失败恢复 | 目标拒绝、源版本变化、取消、冲突、目标已写入但回执超时、重复投递、两个窗口同时粘贴同一剪切项，均不丢源、不重复写。复制相同内容两次仍允许得到两个独立结果。 |
| 权限与窗口命中 | 其他窗口、其他用户、重用标识和失效会话不能领取记录；被遮挡的旧窗口、过期探测或源 App 自报的落点不能触发交付。验证目标 iframe 同源重载保留 windowId、缩放、快速跨窗松手及 Esc。 |
| 输入设备 | 在桌面 Chromium、Firefox、Safari，以及支持的移动端浏览器验证主要入口。键盘、单指点击、触屏滚动、焦点恢复和读屏状态均可用。对不支持的原生能力记录替代路径。 |

合成的 `DragEvent` 或 `ClipboardEvent` 适合单测解析、分发与幂等，不能证明系统剪贴板、跨源窗口或系统文件管理器互操作。端到端验收应使用系统剪贴板和有界面浏览器中的原生输入，记录 OS、浏览器版本、源和目标 origin、事件阶段、可见格式与最终业务结果。

建议先在 Web SDK 中实现 Zone 内容引用列表和带类型表格的类型、序列化器与解析器。接着把 Files、AIWS 和聊天接到这套内容类型与消费逻辑上，跑通 Zone PDF、真实文件及表格三个案例，同时修正 Files 复制不写系统剪贴板、画布写标记串和旧剪贴板回退三个问题。随后实现 App Frame 能力协商、Shell 跨窗手柄和非拖动入口，以两款不同子域的 App 验收。最后按各数据服务的事务和恢复能力逐项开放移动；仅支持复制或引用的目标应直接展示其实际能力。
