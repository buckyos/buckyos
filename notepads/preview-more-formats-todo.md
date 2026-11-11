# Preview：用系统自带的转换工具扩大默认可预览格式 TODO

> 状态：待实施。owner 已于 2026-11-10 裁决 Q1–Q7（§4.1）；由裁决推出的 4 条推定待 owner 确认（§4.2）。现状核实于 2026-11-10（buckyos `55e435bb2` 加工作区改动、buckyos-websdk `84501f0`、ext-core 1.1.0，本机 `/opt/buckyos/tools`），下文行号以此为准。
>
> 目标：按标准方式安装的 Zone，不需要用户另装软件，就能在 Files、聊天附件、文件选择器和 Preview App 里查看常见文档，**手机上也能看**（产品原则：目标用户是手机用户，Zone 是主文件系统）。默认路线只用 ext-core 自带的工具和前端；LibreOffice 是选装件，装了就用它。
>
> 依据：[Preview PRD](../product/bucky_file/BuckyOS%20Preview%20App-Component%20PRD.md) §8、§9、§20 P1-7/P1-11、§23.3.1、§23.8；[iPhone 媒体兼容 TODO](done/preview-iphone-media-compatibility-todo.md) D1–D10；[Office 导入 TODO](done/aiworkspace-office-import-todo.md) §5；[表格导入 TODO](done/aiworkspace-table-import-todo.md)；[workflow 媒体工具基线](../doc/workflow/media%20tool%20baseline.md)；[paios 容器需求](../doc/app_service/paios容器需求.md) §6.4 表 B、表 C。

## 0. 前提：LibreOffice 不是默认组件，这一期也不把它变成默认组件

“为了 llm_understand_media 默认带了 LibreOffice”这个说法，只在 Agent 容器里成立：

| 环境 | 有什么 | 出处 |
| --- | --- | --- |
| `ext-core` 扩展包（节点默认播种，五平台，`$BUCKYOS_ROOT/tools/bin`） | ffmpeg / ffprobe 9.0、ImageMagick 7.1（带 gs、libraw、rsvg、libheif、libjxl、openjpeg、OpenEXR）、poppler（`pdfinfo` / `pdftotext` / `pdftoppm`）、tesseract（28 种语言）、pandoc 3.12、python3、node、deno 等。**没有 soffice，也没有 CJK 字体** | 表 B，`publish/exttool/ext-core/` |
| aios 工作镜像（Agent 容器） | LibreOffice 的 core / writer / impress / calc nogui，DejaVu / Liberation / Noto / Noto CJK 字体，locales | `build_aios` 的 `RUNTIME_PACKAGES`。`ext-office` 在表 C 里还是“规划中” |
| 原生宿主（nfs_server 和 workflow 都跑在这里） | 安装器不装 LibreOffice，也不装 CJK 字体 | 媒体工具基线 §6 的未完成项 |

后果：

1. **标准安装的 Zone 上 Office 文档现在预览不了。** 预览管线在 nfs_server 进程里直接 spawn `soffice`。宿主上没有 soffice，`preview-document` 就不在 `hello.repr.profiles` 里，Office 文档全部显示“不支持预览”。
2. **本机开发 OOD 上 xlsx 预览是坏的。** 这台机器的 LibreOffice 是手工用 apt 装的，没装 `libreoffice-calc-nogui`（`program/` 下有 `libswlo`、`libsdlo`、`libsmlo`、`libhwplo`，没有 `libsclo`）。实测 xlsx、csv 执行 `--convert-to pdf` 都报 `source file could not be loaded`。nfs_server 只检查 `soffice` 程序在不在（`tools.rs:39`），照样广告 `preview-document`，结果用户看到的是“转换失败”，还会被负缓存 10 分钟，而不是“不支持”。

owner 的裁决（Q1）：**pandoc 够用的话，LibreOffice 就继续作为选装件。** 所以默认路线是 ext-core 的 pandoc 加上前端渲染；“pandoc 够不够用”由 B0 的评估报告来回答。走默认路线还有一个附带好处：HTML 和表格由浏览器排版，服务端不需要 CJK 字体。

## 1. 现状

### 1.1 服务端（`src/frame/nfs_server/src/preview/`）

| 内容 | 识别方式（`media.rs:86 sniff`） | 转换 | 结果 |
| --- | --- | --- | --- |
| HEIC、AVIF、TIFF、PSD、BMP 等图片 | 魔数 | ffmpeg | `preview-image`（JPEG，长边 ≤ 4096），`thumbnail`（JPEG，长边 ≤ 640） |
| 视频（ftyp、QuickTime、MKV / WebM、AVI、MPEG-PS、TS） | 魔数 | ffmpeg 重封装或转码 | MP4 H.264；容器里没有画面时转成音频（`convert.rs:305`） |
| 音频（WAV、MP3、FLAC、Ogg、AMR、M4A） | 魔数 | ffmpeg | AAC，封装为 MP4 |
| Office / ODF / RTF：docx、pptx、xlsx、doc、ppt、xls、odt、odp、ods、odg、rtf | zip 条目名、OLE2 流名、`{\rtf`，最后用扩展名补判（`media.rs:55`） | soffice → PDF，Office 通道并发 1 | `result_type: document`（PDF） |
| PDF 原件 | 不进管线 | — | 前端直接显示 |
| 文档和 PDF 的缩略图 | — | — | `unsupported`：“documents have no thumbnail yet”（`rpc.rs:263`） |
| 其他 | `sniff` 返回 `None` | — | `unsupported: no-pipeline` |

存在的问题：

- 探测只看 ffmpeg、ffprobe、soffice 三个程序（`tools.rs:39` 的 `NAMES`）。ext-core 里的 ImageMagick、poppler、pandoc，预览管线都没用上。
- 图片一律交给 ffmpeg。RAW（DNG、CR2、NEF、ARW；iPhone 的 ProRAW 就是 DNG）的文件头和 TIFF 一样，会被判成 `Image` 交给 ffmpeg 的 tiff 解码器。这一点没有实测，预计要么失败，要么只解出内嵌的小图。XCF、JXL、JP2、EXR 识别不出来。
- OLE2 格式只在文件前 1 MiB 里找目录。大文件的目录常在 1 MiB 之后，这时退回按扩展名补判，而补判表里只有 `doc`、`ppt`、`xls`（`media.rs:66`）。
- `converter_id`（`mod.rs:190`）把转换器的版本写进 work_key，所以换转换器或升级版本时，旧缓存会自然失效。

### 1.2 前端（`src/frame/desktop/src/components/`）

- 遇到不认识的类型，前端照样会去问管线。所以服务端新增一种输入格式时，只要输出属于现有的结果族，前端就不用改。
- 新增结果族要改两处：`nfspProvider.ts:103 mapWireResult` 只把 `document` 映射成 `pdf`；websdk 的 `nfsp.ts:252` 里，`ReprResult.result_type` 只写了 `image | video | audio`，连已经在用的 `document` 都没写进去。
- **手机上基本看不了 PDF。** PDF 原件和 Office 转出来的 PDF，都由 `PDFIframeRenderer`（`ContentPreview.tsx:2344`）交给浏览器自带的查看器显示；`navigator.pdfViewerEnabled` 为假时，直接降级成“下载 / 用其他应用打开”（`:2371`）。Android Chrome 一直没有内置 PDF 查看器（`pdfViewerEnabled` 为假）；iOS Safari 在 iframe 里显示多页 PDF 的效果没有实测。
- `pdfjs-dist 6.4.299` 已经是 desktop 的依赖。`app/aiworkspace/ui/media/pdfDoc.ts` 封装好了加载、Worker、CMaps 和 standard fonts（构建产物在 `aiworkspace/pdfjs/`），目前只给 AI Workspace 的导入和海报用。
- `HtmlRenderer` 用 `sandbox=""` 加 CSP（`:2151`，放行 `img-src data: blob:` 和 `font-src data:`），最多读 8 MiB。图片以 data URI 内嵌的自包含 HTML 可以直接显示。
- Markdown 按纯文本显示。MessageHub 有一个自己写的安全 Markdown 渲染器（`app/messagehub/conversation/history/MessageMarkdown.tsx`，180 行，不解析 HTML，链接只放行 http / https / mailto）。
- **已有自研的 xlsx / csv 读取器**：`src/frame/aiworkspace/import-table/`（`xlsx.ts` 260 行，加上 `format.ts`、`csv.ts`），desktop 可以通过 `@table/*` 别名引用（`tsconfig.node.json:28`）。它能处理 shared strings、单元格样式的数字格式（日期、货币、百分比）、1904 日期系统、公式的缓存值和合并单元格，浏览器和 Deno 都能跑，不依赖 SheetJS。xls、ods、numbers 能识别出来，但会拒绝读取。
- Files 列表的缩略图只认 NFSP `access_urls` 里的 `thumbnail`（`app/filebrowser/MainContent.tsx:409`），nfs_server 的普通文件不给这一项，所以显示的都是类型图标。聊天附件、文件选择器和 HomeStation 用的是 `usePreviewThumbnail`。

### 1.3 实测（2026-11-10，本机 `/opt/buckyos/tools`）

| 工具 | 结果 |
| --- | --- |
| pandoc 3.12.1：能读哪些格式 | 能读 docx、pptx、xlsx、odt、rtf、epub、ipynb、fb2、docbook、org、rst、latex、typst、csv / tsv。**读不了** doc、xls、ppt、ods、odp、odg、wps、et、dps 和 iWork |
| pandoc：docx / odt | nfs_server 测试夹具 `office.docx` / `office.odt` 转成自包含 HTML，各用 0.03 s |
| pandoc：pptx | 用 pandoc 自己生成的 pptx 测试，每页的标题、正文、图片（内嵌为 data URI）、表格都保留了，讲者备注丢了。真实 PowerPoint 文件（文本框带位置、图表、SmartArt）要在 B0 评估 |
| pandoc：xlsx | **数字格式全丢**：日期单元格（`yyyy-mm-dd`，值 45231）输出 `45231.0`，百分比（值 0.125）输出 `0.125`；只写了 `inlineStr` 的单元格会被丢掉。中文 shared strings 正常 |
| ImageMagick 7.1.2-31 | 能读 PSD、XCF、AI / EPS / PS（`gs` 在 store 里）、CR2 / NEF / ARW / DNG（libraw 0.22.3）、HEIC / AVIF、JXL、JP2、EXR、HDR、DDS、TGA、DCM、FITS、SVG。**policy 为空，即全部放开**；launcher 会无条件覆盖 `MAGICK_CONFIGURE_PATH`；`MAGICK_SECURITY_POLICY` 实测无效 |
| pdftoppm | PDF 首页转成 640px JPEG，用 0.03 s |
| ffmpeg 9.0.2 | 有 libx264、aac、libmp3lame、libopus、libvpx；没有 zscale（已知） |

## 2. 目标格式与路线

### 2.1 文档

| 格式 | 没装 LibreOffice（默认） | 装了 LibreOffice | 结果 |
| --- | --- | --- | --- |
| docx / docm / dotx、odt、rtf | pandoc → HTML（重新排版，保留文字、表格、图片、基本格式） | soffice → PDF（推定 1） | html / pdf |
| pptx / ppsx / potx | pandoc → HTML（每页一节，保留文字、图片、表格，版式丢失） | soffice → PDF（推定 1） | html / pdf |
| xlsx / xlsm | 前端表格渲染器直接读原件（推定 2） | 同左 | sheet |
| csv / tsv | 前端表格渲染器直接读原件，可切回纯文本 | 同左 | sheet |
| epub、ipynb、fb2、docbook、org、rst、tex、typst | pandoc → HTML | 同左 | html |
| md | 前端渲染或 pandoc，按 B0 的比较结果二选一（Q5） | 同左 | text / html |
| PDF 原件 | 桌面用 iframe，手机用 pdf.js（Q2） | 同左 | pdf |
| doc、wps、ppt、dps、odp、odg | 不支持，提示可安装 LibreOffice（推定 4） | soffice → PDF | pdf |
| xls、et、ods | 不支持，提示可安装 LibreOffice | soffice → xlsx，再交给前端表格渲染器（推定 2） | sheet |
| iWork（pages / key / numbers）、hwp、vsdx、pub、wpd | 不支持 | soffice → PDF（未实测） | pdf |

### 2.2 图片（顺带做，复用同一套机制）

| 格式 | 路线 |
| --- | --- |
| RAW：DNG、CR2、NEF、ARW、RAF、ORF、RW2 | magick（libraw）→ `preview-image` / `thumbnail` |
| XCF、JXL、JP2、EXR、HDR、DDS、TGA、DCM | magick → 同上 |
| PSD | 先比较现在的 ffmpeg 和 magick（`psd:file[0]` 取合成图）的效果，再决定交给谁 |
| HEIF / HEIC | 保持现在调好的 ffmpeg 路径，不动 |

### 2.3 音视频（顺带做，只需补识别）

WMV / WMA（ASF）、FLV、RM / RMVB、AIFF、CAF、APE 由 ffmpeg 解码。容器里没有画面时，现有逻辑已经会转成音频。

### 2.4 不做

- EPS、AI、PS（Q6：这一期不开）。
- ZIP / 7z / RAR 内浏览（PRD §11.6）、CAD（`ext-cad`）、邮件文件（.eml / .msg）、加密文件的口令输入、编辑。
- 把 LibreOffice 做成默认播种的组件（Q1）。要不要把 `ext-office` 做成可选装的扩展包，留在表 C 里，不在本 TODO 的范围内。

## 3. 设计要点

### 3.1 转换器分组与探测

- `tools.rs` 的 `Probe` 从三个程序扩成五组：`media`（ffmpeg + ffprobe）、`office`（soffice 及其组件）、`image`（magick）、`pdf`（pdftoppm + pdfinfo）、`markup`（pandoc）。每组都是可选的，`profiles()` 按组广告；`hello.repr` 列出每组的路径和版本。`NAMES` 同步加入这些程序，装上或升级后会重新探测。
- 查找顺序沿用 `search_dirs()`（`tools/bin` 排最后）。ImageMagick 只认 `magick`（IM7），不用 `convert`：系统里的 IM6 默认策略会挡掉一部分格式，行为不一致。
- **soffice 按组件探测。** 规划阶段检查 `program/` 下的模块库：writer 对应 `swlo`，calc 对应 `sclo`，impress / draw 对应 `sdlo`（Linux 上是 `lib*.so`，macOS 上是 `lib*.dylib`，Windows 上是 `*.dll`）。缺某个组件时，这一族文档按“没装 LibreOffice”的那一列处理。

### 3.2 结果族与 Profile

| Profile | 产出 | `result_type` | 前端渲染器 |
| --- | --- | --- | --- |
| `preview-html`（新增） | pandoc 生成的自包含 HTML | `html` | `HtmlRenderer`（已有） |
| `preview-sheet`（新增，只在装了 LibreOffice 时有） | soffice 把 xls / et / ods 转成的 xlsx | `sheet` | 新增的表格渲染器 |
| `preview-document`（已有） | soffice 转出的 PDF | `document` | 桌面用 iframe，手机用 pdf.js |
| `thumbnail`（扩展） | 见 B4 | `image` | 已有 |

- 同一种格式在不同节点上可能走不同的转换器。work_key 里本来就有 `converter@version`，各自的缓存天然分开；后来装上了 LibreOffice，规划会自动换路线，旧缓存按“30 天未访问”的规则过期。
- 协议只增不改：`result_type` 新增 `html` 和 `sheet`，`runtime.accept` 新增 `sheet`；Runtime 不接受某种结果时，按现有规则返回 `no-renderer`。

### 3.3 安全

- **pandoc**：一律加 `--sandbox`（除了输入文件，禁止一切 IO）。输出再经过前端 `sandbox=""` 加 CSP 的 iframe。
- **ImageMagick**：一律用显式 coder（`dng:input[0]`，coder 由服务端自己的 sniff 决定），不让 IM 自己猜格式，这样 PS / EPS / PDF 等交给 gs 的 coder 永远不会被调起来（Q6）。另外用命令行 `-limit` 限制 memory、map、disk、width、height、area 和 time。服务端没法自带 policy.xml（见 §1.3）。
- **前端表格渲染器**：解析的是用户文件，要限制解压后的大小（防 zip 炸弹），并在 Worker 里解析。`import-table` 现有的限制要核实。
- **soffice**：已经做到每次转换用独立的 profile。文档里的远程图片和链接在转换时会不会发起网络请求，没有核实（B7）。

### 3.4 资源

- pandoc 和 magick 走现有的 media 通道（按核数 1–2 个并发），soffice 保持独占 1 个并发。超时：pandoc 取 max(30 s, 2 s/MiB)，magick 取 max(60 s, 5 s/MiB)，按实测再调。
- HTML 结果超过 8 MiB（前端读取上限）时转换失败，错误码 `TOO_LARGE`，不让前端截断 HTML。
- 表格渲染器直接读原件的大小上限先定 30 MiB，每个 sheet 最多显示 5,000 行，超出时提示已截断。

## 4. 裁决

### 4.1 owner 裁决（2026-11-10）

| # | 问题 | 裁决 |
| --- | --- | --- |
| Q1 | LibreOffice 怎样成为默认组件 | **pandoc 够用（能预览常见办公文件）的话，LibreOffice 继续选装。** 够不够用以 B0 的评估报告为准 |
| Q2 | 手机上怎么看 PDF | **桌面保留 iframe**，手机改用 pdf.js |
| Q3 | 没有 soffice 时降不降级成 pandoc HTML | 同 Q1，pandoc 够用最好 |
| Q4 | 表格用 PDF 还是网页 | **网页** |
| Q5 | Markdown 由谁渲染 | **看效果，选效果好的那个**（B0 比较前端渲染与 pandoc） |
| Q6 | EPS / AI / PS 默认开不开 | **先不开** |
| Q7 | 为了出缩略图，要不要单独启动 soffice | 同 Q1，能用 pandoc 就用 pandoc |

### 4.2 由裁决推出、待 owner 确认的推定

| # | 推定 | 理由 |
| --- | --- | --- |
| 推定 1 | 节点上同时有 pandoc 和 LibreOffice 时，Word 类和演示文稿用 soffice 转 PDF；表格不论装没装都按网页显示（Q4） | 用户选装 LibreOffice，就是为了看到原版式。手机上的 PDF 靠 pdf.js 显示（Q2） |
| 推定 2 | Q4 的“网页”由前端渲染，**不走 pandoc**：xlsx / csv 由前端用 `import-table` 的读取器直接读原件；xls / et / ods 只有装了 LibreOffice 才能看，先由 soffice 转成 xlsx，再交给同一个渲染器 | 实测 pandoc 把日期输出成 `45231.0`、百分比输出成 `0.125`（§1.3），对表格来说不算“够用”。自研读取器已经能处理数字格式和合并单元格，desktop 可以直接引用，不需要新依赖 |
| 推定 3 | Q7 的“用 pandoc”做不到：pandoc 只能产出文本格式，产不出图片（转 PDF 要额外的 PDF 引擎，ext-core 里没有）。改成三级：先取文件里内嵌的缩略图（ODF 都带 `Thumbnails/thumbnail.png`；OOXML 只有保存时写了才有 `docProps/thumbnail.jpeg`，各软件的默认行为要用样本核实）；没有的话，如果预览 PDF 已经缓存，就对它跑 pdftoppm；再不行就显示类型图标。**任何情况下都不为缩略图启动 soffice** | 内嵌缩略图零成本，读 zip 用 workspace 现成的 `zip` crate，不需要外部工具；同时满足 Q7“不为缩略图动 LibreOffice”的本意 |
| 推定 4 | 没装 LibreOffice 时，pandoc 读不了的格式（doc、xls、ppt、wps、et、dps、ods、odp、odg、iWork）显示“不支持”，并按 PRD Level 0 提示“安装 LibreOffice 后可以预览”，同时保留“下载原件”和“用其他应用打开” | 这些是 pandoc 的硬缺口（§1.3）。旧版 Office 和 WPS 文件在国内仍然常见，B0 报告里要列出这部分占比，作为 Q1 的判断依据之一 |

## 5. 实施项

### Phase A：前端（不依赖服务端工具）

- **A1 pdf.js 渲染器（Q2）**
  - 把 `pdfDoc.ts` 的加载部分（`loadPdfjs` 和资源路径）挪到 `components/preview/pdfjs.ts`，AI Workspace 改为从这里引用。确认 `/embed/preview` 页面也能拿到 `pdfjs/` 资源。
  - 用 URL 加 Range 加载（`getDocument({ url, rangeChunkSize, disableAutoFetch: true })`）。桶数据面的 `read_url` 支持 Range；`/nfs/v1/read/` 支不支持要核实；对象 ID 来源（`ndmSource.ts`）本来就是 Blob。
  - 竖向连续滚动，只渲染可见页及其前后各一页；canvas 像素设上限（iOS 单个 canvas 约 1670 万像素）；双指缩放时先用 CSS 变换，松手后按新倍率重绘；显示页码并支持跳页；遇到加密 PDF 显示“受密码保护”并提供下载。
  - 选择规则：`pdfViewerEnabled` 为假，或者 `matchMedia('(pointer: coarse)')` 成立时用 pdf.js，其余情况（桌面）保持 iframe。组件 ref 上的方法（`zoomIn`、`fitToView` 等）在两种渲染器下都要可用。
  - 测试：Playwright 模拟移动设备（Pixel、iPhone），用一份多页 PDF 和一份依赖 CMaps 的中文 PDF。
- **A2 协议类型**：websdk 的 `ReprResult.result_type` 补上 `document`、`html`、`sheet`；`mapWireResult` 把 `html` 映射到 HTML 渲染器、`sheet` 映射到表格渲染器；`detectRuntimeProfile` 的 `acceptTypes` 加上 `sheet`。
- **A3 表格渲染器（推定 2）**
  - 新增渲染器类型 `sheet`。`mediaTypes.ts` 把 xlsx / xlsm / csv / tsv 改成 `sheet` 并直接显示，不再算 `isPipelineDocument`；xls / et / ods 仍然交给管线。
  - 在 Worker 里用 `@table/xlsx.ts`、`@table/csv.ts` 读取，数字格式用 `@table/format.ts`。
  - 界面：底部 sheet 标签（隐藏的 sheet 不显示）、行虚拟滚动、表头固定、合并单元格、横向滚动；手机上支持双指缩放。csv / tsv 可以切换到纯文本视图。
  - 超出 §3.4 的上限时提示已截断；加密的文件显示“受密码保护”。
- **A4 实机核对**：在 Android Chrome 和 iOS Safari 上分别打开 PDF 原件、HTML 结果和表格，记录 `pdfViewerEnabled` 的取值和实际效果，确认 A1 的选择规则。

### Phase B：服务端，以及 pandoc 的评估

- **B0 pandoc 评估（Q1、Q3、Q5 的判断依据，先做）**
  - 样本：真实的 docx（中文公文，带表格、图片、页眉页脚、脚注、批注、修订、公式）、pptx（带图片、图表、SmartArt、文本框排版、讲者备注）、odt、rtf、epub、ipynb；Markdown（GFM 表格、任务列表、多层嵌套列表、代码块、脚注、中文长文）。样本由 owner 提供，或者从公开来源收集，注意授权。另外统计一下手头样本里 doc / xls / ppt / wps 这类 pandoc 读不了的格式占多少（推定 4）。
  - 做法：直接用 CLI 跑 `pandoc --sandbox -s --embed-resources`，加上 B2 计划用的 CSS；用 Playwright 在手机视口（390×844）下截图，和 soffice 转出的 PDF 并排对比。Markdown 拿 MessageMarkdown 的渲染结果和 pandoc（`-f gfm`）的结果并排对比。
  - 每个格式都看这几项：正文是否完整、顺序是否正确；表格结构；图片；标题和列表；中文；脚注、批注；公式（pandoc 输出 MathML）；修订（`--track-changes=accept`）；转换耗时和输出大小。
  - 产出：一份评估报告（截图加结论），每个格式标成“够用 / 勉强 / 不够”，交 owner 定 Q1、Q5。被判“不够”的格式，没装 LibreOffice 时按推定 4 处理。pptx 如果不够，备选是复用 AI Workspace 的 pptx 解析器（`import-office/pptx.ts`，能拿到文本框位置和图片），这需要另行立项。
- **B1 探测分组**：按 §3.1 改 `tools.rs` 和 `mod.rs`（`Probe`、`Converter`、`converter_id`、`PROFILES`、`refresh` 的 identity），并实现 soffice 的组件探测。测试：任意一组单独缺失时，其余组照常广告；本机缺 calc 时，xlsx 不再进 `preview-document`，xls 返回 `unsupported`，不再是 `failed`。
- **B2 pandoc → HTML（`preview-html`）**
  - 输入：epub（zip 的第一个条目是 `mimetype`，内容为 `application/epub+zip`）、ipynb、fb2、docbook、org、rst、tex、typst（按扩展名提示，同时内容要像文本）。docx / odt / rtf / pptx 在没有 writer / impress 组件时走 pandoc（推定 1）。B0 判为“不够”的格式不走 pandoc。
  - 参数：`--sandbox -s --embed-resources --metadata title=<文件名>`，加一份 nfs_server 内置的 CSS（窄屏排版、表格可以横向滚动）；docx 加 `--track-changes=accept`。超过 8 MiB 按 §3.4 处理。
  - fidelity 文案：Word 类写“已转换为网页，保留文字、表格和图片，版式与原件不同”；演示文稿写“已转换为网页，每页依次显示，版式与原件不同”。
- **B3 ImageMagick 图片**
  - 识别：RAW（TIFF 头，再看 DNG 标签 0xC612、CR2 偏移 8 处的 `CR`，或者扩展名提示）、XCF（`gimp xcf`）、JXL（`\xff\x0a` 或 `JXL ` box）、JP2（`jP  ` box）、EXR（`v/1\x01`）、HDR（`#?RADIANCE`）、DDS（`DDS `）、DCM（偏移 128 处的 `DICM`）、TGA（只能按扩展名）。
  - 转换：`magick <coder>:<in>[0] -auto-orient -background white -flatten -resize 4096x4096> -colorspace sRGB -quality 85 jpg:<out>`，缩略图用 640；加上 §3.3 的 `-limit`。PSD 先比较效果再决定交给谁。
  - 测试夹具尽量自己生成，DNG 用一张小样本，注意授权。
- **B4 缩略图（推定 3）**
  - 新增 `Kind::Pdf`（`%PDF-`），只用于 `purpose: thumbnail`；`purpose: preview` 仍由前端直接显示（PRD §23.3.1）。PDF 缩略图用 `pdftoppm -f 1 -l 1 -scale-to 640 -jpeg`；加密的 PDF 返回 `unsupported`。
  - Office、ODF 和 epub（封面）：先读文件里内嵌的缩略图或封面，缩放到 640 以内；没有的话，如果预览 PDF 已经缓存，就对它跑 pdftoppm（PRD §9.6 允许的两步管线）；都没有就返回 `unsupported`，前端显示类型图标。
  - B4b（可选，可以单独排期）：Files 的网格视图对能出缩略图的类型改用 `usePreviewThumbnail`，只对可见项请求，并限制并发。这是手机上的 Files 里认出文档的主要手段。
- **B5 补音视频识别**：ASF（`30 26 B2 75 8E 66 CF 11`）、FLV（`FLV\x01`）、RM（`.RMF`）、AIFF / AIFC（`FORM....AIFF`）、CAF（`caff`）、APE（`MAC `）。测试样本用 ffmpeg 生成。
- **B6 修正文档识别**
  - 扩展名补判表加上 `wps / wpt → doc`、`et / ett → xls`、`dps / dpt → ppt`，以及 OOXML 的各种变体（`docm / dotx / dotm / xlsm / xltx / pptm / ppsx / potx`）。前端的 `PIPELINE_DOCUMENT_TYPES`（`mediaTypes.ts:52`）同步修改。
  - OLE2 的大文件：按文件头里的目录扇区号补读一次目录，不再只看前 1 MiB。
- **B7 远程资源核查**：用带外链图片的 docx 加抓包，确认 soffice 和 pandoc 转换时会不会发起网络请求，结论写进本文。
- **B8 Markdown（Q5）**：按 B0 的比较结果实施。选前端渲染的话，把 MessageMarkdown 提成共享组件，`TextRenderer` 遇到 `text/markdown` 时默认显示渲染效果，可以切到源码；选 pandoc 的话，md 改为走 `preview-html`。不论选哪个，这一期相对路径的图片都不解析。

### Phase C：LibreOffice 选装

- **C1 有就用**：按推定 1 排定转换器的优先级，组件探测见 B1。实现 `preview-sheet`（soffice `--convert-to xlsx`，需要 calc 组件）。
- **C2 没有时怎么提示**：推定 4 的“不支持”状态里，给出“安装 LibreOffice 后可以预览”的说明和安装文档的链接（各平台的安装方式见媒体工具基线 §3）。只有内容能被识别出是这类格式时才提示，识别不出的格式不提示。
- **C3 文档同步**：媒体工具基线写明 LibreOffice 对预览来说是选装件；workflow 的 `media.to_pdf` 仍然把它列为基线（Agent 容器里有）。

### Phase D：验收

- **样本矩阵**：§2 的每种格式至少一个样本，放进 `tests/fixtures/preview/`；`tests/preview.rs` 按组跳过缺失的工具（沿用现在 `soffice_present()` 的写法）。前端表格渲染器的用例放进 desktop 的 Playwright 测试。
- **工具缺失注入**：去掉 pandoc 时，epub 显示“不支持”并给出原因；去掉 soffice 时，docx 走 pandoc HTML、doc 显示“不支持”并提示安装；缺 calc 组件时，xlsx 照常显示（由前端渲染），xls 显示“不支持”。
- **真实 Zone**：用普通成员账号，在手机实机上从聊天附件和 Files 打开 docx、pptx、xlsx、csv、PDF、epub、md、DNG，检查缩略图，以及第二次打开时命中缓存。
- **性能基线**：在一台 N100 级别的 OOD 上，记录 20 页 docx（pandoc 和 soffice 各一次）、20 MB DNG、5 MB epub 的转换时间，以及一个 5 万行 xlsx 在手机上首屏显示的时间，写进本文。

## 6. 要同步的文档

- PRD §23.8 第 1、4 项（首批 Pipeline 清单、PDF 增强的顺序）。PRD §9.2 的结果族要加上 `sheet`。
- `src/frame/desktop/src/app/preview/README.md`（Providers、Known gaps）。
- `src/frame/nfs_server/README.md` 的 preview 部分。
- 媒体工具基线（C3）。
