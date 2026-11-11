# Preview：用系统自带的转换工具扩大默认可预览格式 TODO

> 状态：待实施。基线已定（owner，2026-11-10，见 §0.2）：
>
> - **paios 镜像里的工具都来自 ext-core。** LibreOffice 进了 paios 镜像，就属于核心扩展工具：`soffice` 命令一定可用，各环境只是调用方式不同。所以 LibreOffice 要进 ext-core 的五个平台。
> - pandoc 只用于它肯定能处理的格式。
>
> §4.1 是 owner 的裁决，§4.2 是待确认的推定。现状核实于 2026-11-10（buckyos `55e435bb2` 加工作区改动、buckyos-websdk `84501f0`、ext-core 1.1.0、本机 `/opt/buckyos/tools`、本机 aios 镜像 `local/aios-test:latest-amd64`），下文行号以此为准。
>
> 目标：按标准方式安装的 Zone 不需要用户另装软件，就能在 Files、聊天附件、文件选择器和 Preview App 里查看常见文档，**手机上也能看**（产品原则：目标用户是手机用户，Zone 是主文件系统）。
>
> 依据：[Preview PRD](../product/bucky_file/BuckyOS%20Preview%20App-Component%20PRD.md) §8、§9、§20 P1-7/P1-11、§23.3.1、§23.8；[iPhone 媒体兼容 TODO](done/preview-iphone-media-compatibility-todo.md) D1–D10；[Office 导入 TODO](done/aiworkspace-office-import-todo.md) §5；[表格导入 TODO](done/aiworkspace-table-import-todo.md)；[workflow 媒体工具基线](../doc/workflow/media%20tool%20baseline.md)；[paios 容器需求](../doc/app_service/paios容器需求.md) §6.4 表 B、表 C；[Node Executor](../doc/arch/node_exector.md) §8。

## 0. 前提与基线

### 0.1 现在 LibreOffice 在哪里

| 环境 | 有什么 | 出处 |
| --- | --- | --- |
| `ext-core` 扩展包（节点默认播种，五个平台，`$BUCKYOS_ROOT/tools/bin`） | ffmpeg / ffprobe 9.0、ImageMagick 7.1、poppler、tesseract、pandoc 3.12、python3、node、deno 等。**没有 soffice，也没有 CJK 字体** | 表 B |
| aios 工作镜像（Agent 容器） | 命令行工具全部来自运行时挂载的 `buckyos-tools` 卷（ext-core），镜像自己的 `/opt/buckyos/tools/bin` 是空的。**唯一的例外是 LibreOffice**：`build_aios` 用 apt 把 LibreOffice 24.2 的 core / writer / impress / calc（nogui，**没有 draw**）、DejaVu / Liberation / Noto / Noto CJK 字体和 locales 直接装进了镜像（`RUNTIME_PACKAGES` 第二组，注释写的是“规划中的 ext-office（表 C）”） | `build_aios:22-29`。本机核实：镜像里 `soffice` 是 `/usr/bin/soffice`，来自 `libreoffice-core-nogui`；`ffmpeg`、`magick`、`pdftoppm`、`tesseract`、`pandoc`、`python3`、`node`、`git`、`jq` 镜像本身都没有 |
| 原生宿主：Linux 上的 deb / rpm 安装，以及 macOS / Windows 上的 Desktop Lite | 安装器不装 LibreOffice。nfs_server 和 workflow 都是宿主进程（Linux 上是 static-pie），它们拉起的转换命令用的是宿主环境 | 本机实测：`/opt/buckyos/bin/nfs-server/nfs_server` 以 root 跑在宿主上，不在 docker 里 |

由此造成两个现象：

1. 标准安装的 Zone 上，Office 文档预览显示“不支持”。原因是宿主上没有 soffice，`preview-document` 不会出现在 `hello.repr.profiles` 里。llm_understand_media 的 `media.to_pdf` 也一样。
2. 本机开发 OOD 上 xlsx 预览是坏的。这台机器手工装的 LibreOffice 少了 calc：`program/` 下没有 `libsclo`，xlsx / csv 转换会报 `source file could not be loaded`。nfs_server 只看 `soffice` 这个程序在不在（`tools.rs:39`），照样广告能转，用户看到的就是“转换失败”，还会被负缓存 10 分钟。

### 0.2 基线（owner，2026-11-10）

- **paios 镜像里的工具都来自 ext-core。** LibreOffice 进了 paios 镜像，就属于核心扩展工具：`soffice` 命令一定可用，各环境的区别只是调用方式不同。缺了它算部署缺陷，不是正常降级（与媒体工具基线 R2 / R3 的口径一致）。
- **系统服务自己用扩展工具时，本地有就用本地的，本地没有就用容器里的。**
- **pandoc 只用于它肯定能处理的格式。** Office 文档一律交给 soffice。

推论：`build_aios` 里用 apt 装 LibreOffice 是一个待清理的例外。**LibreOffice 要进 ext-core 的五个平台**（与表 B 的其它工具同样处理），`build_aios` 随后去掉这一组 apt 包。这和 paios 容器需求 §6.4 现有的规则一致：“工作镜像里 Agent 可用的命令只随 ext-core 与表 C 的包变”，“新命令走扩展包，不单独往工作镜像加”。

落地后，各环境的调用方式：

| 环境 | soffice 从哪里来 |
| --- | --- |
| Agent 容器 | 工具卷里的 `/opt/buckyos/tools/bin/soffice` |
| Linux 宿主（nfs_server、workflow） | 宿主机上已有 LibreOffice 时优先用它；否则用 `$BUCKYOS_ROOT/tools/bin/soffice`，与容器的工具卷是同一份文件。前提是 ext-core 把 soffice 需要的系统库一起带上（§1.3） |
| macOS / Windows 宿主 | 宿主机上已有 LibreOffice 时优先用它；否则用 ext-core 原生 payload 里的 soffice |

所以“本地没有就用容器里的”在 soffice 上基本用不到：ext-core 落地后，每个环境本地就有。现有的查找顺序（`search_dirs()` 把 `tools/bin` 排在最后）不用改，就能找到它。

ext-core 带上 LibreOffice 之前，不拉起容器做过渡（Q9）：宿主上没有 LibreOffice 的 Zone，Office 预览仍然显示“不支持”。容器路线的本机实测结果留作参考（`local/aios-test:latest-amd64`，LibreOffice 24.2.7.2）：`docker run --rm --network none`，输入目录只读挂载、输出目录挂载，docx 和 xlsx 转 PDF 各用约 0.9 s，含容器启动时间。xlsx 转出的 PDF 里，日期列因为列宽不够显示成 `###`，这正是 Q4 选择用网页显示表格的原因。系统里目前没有“宿主服务把命令放进容器执行”的现成机制（Node Executor §8 列为待定）。

## 1. 现状

### 1.1 服务端（`src/frame/nfs_server/src/preview/`）

| 内容 | 识别方式（`media.rs:86 sniff`） | 转换 | 结果 |
| --- | --- | --- | --- |
| HEIC、AVIF、TIFF、PSD、BMP 等图片 | 魔数 | ffmpeg | `preview-image`（JPEG，长边 ≤ 4096），`thumbnail`（JPEG，长边 ≤ 640） |
| 视频（ftyp、QuickTime、MKV / WebM、AVI、MPEG-PS、TS） | 魔数 | ffmpeg 重封装或转码 | MP4 H.264；容器里没有画面时转成音频（`convert.rs:305`） |
| 音频（WAV、MP3、FLAC、Ogg、AMR、M4A） | 魔数 | ffmpeg | AAC，封装为 MP4 |
| Office / ODF / RTF：docx、pptx、xlsx、doc、ppt、xls、odt、odp、ods、odg、rtf | zip 条目名、OLE2 流名、`{\rtf`，最后用扩展名补判（`media.rs:55`） | 宿主上的 soffice → PDF，Office 通道并发 1 | `result_type: document`（PDF） |
| PDF 原件 | 不进管线 | — | 前端直接显示 |
| 文档和 PDF 的缩略图 | — | — | `unsupported`：“documents have no thumbnail yet”（`rpc.rs:263`） |
| 其他 | `sniff` 返回 `None` | — | `unsupported: no-pipeline` |

存在的问题：

- 探测只看 ffmpeg、ffprobe、soffice 三个程序（`tools.rs:39` 的 `NAMES`）。ext-core 里的 ImageMagick、poppler、pandoc 都没用上；soffice 也只在宿主上找。
- 图片一律交给 ffmpeg。RAW（DNG、CR2、NEF、ARW；iPhone 的 ProRAW 就是 DNG）的文件头和 TIFF 一样，会被交给 ffmpeg 的 tiff 解码器。这一点没有实测，预计要么失败，要么只能解出内嵌的小图。XCF、JXL、JP2、EXR 识别不出来。
- OLE2 格式只在文件前 1 MiB 里找目录。大文件的目录常常在 1 MiB 之后，这时退回按扩展名补判，而补判表里只有 `doc`、`ppt`、`xls`（`media.rs:66`）。WPS 的 `.wps` / `.et` / `.dps` 大文件因此认不出来。
- nfs_server 的 `preview/tools.rs` 和 workflow 的 `functions/media/tools.rs` 是两份复制的工具查找代码。
- `converter_id`（`mod.rs:190`）把转换器版本写进了 work_key。

### 1.2 前端（`src/frame/desktop/src/components/`）

- 遇到不认识的类型，前端照样会去问管线。所以服务端新增一种输入格式时，只要结果属于现有的结果族，前端就不用改。
- 新增结果族要改两处：`nfspProvider.ts:103 mapWireResult` 只把 `document` 映射成 `pdf`；websdk `nfsp.ts:252` 的 `ReprResult.result_type` 只写了 `image | video | audio`，连已经在用的 `document` 都没写进去。
- **手机上基本看不了 PDF。** PDF 原件和 Office 转出的 PDF 都由 `PDFIframeRenderer`（`ContentPreview.tsx:2344`）交给浏览器自带的查看器，`navigator.pdfViewerEnabled` 为假时直接降级成“下载 / 用其他应用打开”（`:2371`）。Android Chrome 一直没有内置查看器；iOS Safari 在 iframe 里显示多页 PDF 的效果还没实测。基线定下来以后，Office 文档全部转成 PDF，这个缺口就成了手机上的头号问题。
- `pdfjs-dist 6.4.299` 已经是 desktop 的依赖。`app/aiworkspace/ui/media/pdfDoc.ts` 封装好了加载、Worker、CMaps 和 standard fonts（构建产物在 `aiworkspace/pdfjs/`），目前只有 AI Workspace 在用。
- `HtmlRenderer` 用 `sandbox=""` 加 CSP（`:2151`，放行 `img-src data: blob:` 和 `font-src data:`），最多读 8 MiB。
- Markdown 文件按纯文本显示。MessageHub 有一个自己写的安全 Markdown 渲染器（`app/messagehub/conversation/history/MessageMarkdown.tsx`，180 行，不解析 HTML，链接只放行 http / https / mailto）。
- **有一个自研的 xlsx / csv 读取器**，在 `src/frame/aiworkspace/import-table/`（`xlsx.ts`、`format.ts`、`csv.ts`）。desktop 通过 `@table/*` 引用它（`tsconfig.node.json:28`）。它能处理 shared strings、数字格式（日期、货币、百分比）、1904 日期系统、公式的缓存值和合并单元格，浏览器和 Deno 都能跑；遇到 xls / ods / numbers 会拒绝读取。
- Files 列表的缩略图只认 NFSP `access_urls` 里的 `thumbnail`（`app/filebrowser/MainContent.tsx:409`），而 nfs_server 的普通文件不给这一项，所以 Files 里显示的都是类型图标。聊天附件、文件选择器和 HomeStation 用的是 `usePreviewThumbnail`。

### 1.3 实测（2026-11-10）

| 对象 | 结果 |
| --- | --- |
| pandoc 3.12.1 能读的格式 | docx、pptx、xlsx、odt、rtf、epub、ipynb、fb2、docbook、org、rst、latex、typst、csv / tsv。读不了 doc、xls、ppt、ods、odp、odg、wps、et、dps 和 iWork |
| pandoc 处理 Office 文档的效果 | docx / odt 转 HTML 只要 0.03 s。pptx 保留每页的文字、图片和表格，丢掉版式和讲者备注。**xlsx 丢数字格式**：日期输出成 `45231.0`，百分比输出成 `0.125`，只写成 `inlineStr` 的单元格被丢掉。这些都是“不能肯定处理好”的例子 |
| ImageMagick 7.1.2-31 | 能读 PSD、XCF、AI / EPS / PS（`gs` 在 store 里）、CR2 / NEF / ARW / DNG（libraw 0.22.3）、HEIC / AVIF、JXL、JP2、EXR、HDR、DDS、TGA、DCM、FITS、SVG。**policy 为空，即全部放开**；launcher 会无条件覆盖 `MAGICK_CONFIGURE_PATH`；`MAGICK_SECURITY_POLICY` 实测无效 |
| pdftoppm | PDF 首页转成 640px JPEG 用 0.03 s |
| aios 镜像里的 soffice | 见 §0.2：docx、xlsx 各约 0.9 s，`--network none` |
| LibreOffice 进 ext-core 的体积（官方 26.8.2） | **Linux x86_64**：无界面子集安装后 453 MiB，按 ext-core 的方式打成 tar.gz 是 156 MiB；Noto CJK 字体（Sans、Serif 的常规和粗体）安装后 89 MiB，tar.gz 72 MiB；还要加上 soffice 依赖的系统库（未测）。合计 payload 约 450 → 700 MiB，安装后约 1.25 → 1.85 GiB。aios 镜像去掉 apt 那一组后，约小 330 MiB。**macOS / Windows**：官方 dmg 是 285 / 295 MiB，msi 是 358 MiB，估计每个平台的 payload 增加 200–300 MiB（没有解包，未实测）。**五平台的 `.pikg`** 估计从 3.4 GiB 涨到约 4.6 GiB。以上只量了大小，没有运行（执行下载来的二进制被权限规则拦下了） |
| 官方 Linux 版依赖的系统库 | 即使只做无界面转换，也要用到 cairo、cups、dbus、fontconfig、freetype、glib、nss / nspr、X11 / Xext、avahi、krb5，并且要求 glibc ≥ 2.34（ext-core 现在的最低要求是 2.28，来自 node）。conda-forge 上没有 LibreOffice 的包。ext-core 的 launcher 原则上不依赖系统里的同名库，所以这些库要从 conda-forge 一起带上；cups、avahi 在 conda-forge 上有没有，要核实 |

## 2. 目标格式与路线

### 2.1 文档

| 格式 | 路线 | 结果 |
| --- | --- | --- |
| docx / docm / dotx、doc、wps、odt、rtf | soffice → PDF | pdf |
| pptx / ppsx / potx、ppt、dps、odp | soffice → PDF | pdf |
| xlsx / xlsm、csv / tsv | 前端表格渲染器直接读原件（推定 2）；csv / tsv 可以切回纯文本 | sheet |
| xls、et、ods | soffice → xlsx，再交给前端表格渲染器 | sheet |
| odg、vsdx、pub、hwp、wpd、iWork（pages / key / numbers） | soffice → PDF（numbers 走表格那条路）。odg、vsdx、pub 需要 draw 组件，C1 的 ext-core 子集里带了 draw；这些格式都要到 C2 才实测 | pdf / sheet |
| epub、ipynb、fb2、docbook、org、rst、tex、typst | pandoc → 自包含 HTML。这些 soffice 都读不了；进不进最终清单，由 B0 判断 pandoc 能不能“肯定处理好” | html |
| md | 前端渲染或 pandoc，B0 比较后二选一（Q5） | text / html |
| PDF 原件 | 桌面用 iframe，手机用 pdf.js（Q2） | pdf |

### 2.2 图片（顺带做，复用同一套机制）

| 格式 | 路线 |
| --- | --- |
| RAW：DNG、CR2、NEF、ARW、RAF、ORF、RW2 | magick（libraw）→ `preview-image` / `thumbnail` |
| XCF、JXL、JP2、EXR、HDR、DDS、TGA、DCM | magick → 同上 |
| PSD | 先比较现在的 ffmpeg 和 magick（`psd:file[0]` 取合成图）的效果，再决定交给谁 |
| HEIF / HEIC | 保持现在调好的 ffmpeg 路径，不动 |

### 2.3 音视频（顺带做，只需要补识别）

WMV / WMA（ASF）、FLV、RM / RMVB、AIFF、CAF、APE 由 ffmpeg 解码。容器里没有画面时，现有逻辑已经会转成音频。

### 2.4 不做

- EPS、AI、PS（Q6：这一期不开）。
- ZIP / 7z / RAR 内浏览（PRD §11.6）、CAD（`ext-cad`）、邮件文件（.eml / .msg）、加密文件的口令输入、编辑。
- 把 pandoc 当成 soffice 的降级路线：基线保证 soffice 一定存在，缺了就是部署缺陷。

## 3. 设计要点

### 3.1 转换器分组与探测

- `Probe` 从三个程序扩成五组：`media`（ffmpeg + ffprobe）、`office`（soffice）、`image`（magick）、`pdf`（pdftoppm + pdfinfo）、`markup`（pandoc）。`profiles()` 按组广告；`hello.repr` 写出每组的路径、版本和来源（宿主或容器）。`NAMES` 同步加入新增的程序，装上或升级后会重新探测。
- 查找顺序沿用 `search_dirs()`（`tools/bin` 排在最后）。ImageMagick 只认 `magick`（IM7）：系统里的 IM6 默认策略会挡掉一些格式，行为不一致。
- **soffice 和其它 ext-core 工具一样查找**（沿用 `search_dirs()`），另外要确认转换要用的组件都在。检查 `program/` 下的模块库：writer 是 `swlo`，calc 是 `sclo`，impress / draw 是 `sdlo`；Linux 上是 `lib*.so`，macOS 是 `lib*.dylib`，Windows 是 `*.dll`。宿主自己装的 LibreOffice 缺组件时（比如本机缺 calc），这一族要改用 `tools/bin` 里的 soffice，而不是直接判为不支持。所以查找要按组件分别进行，不能找到第一个就停。
- 都找不到时，`repr` 返回 `unsupported`，reason 是 `no-pipeline`，detail 写明“`soffice` 不可用（部署缺陷）”和已经查过的位置。启动日志和 `hello` 也要报出来，方便管理员排查。前端文案不提示用户自己去装软件。

### 3.2 结果族与 Profile

| Profile | 产出 | `result_type` | 前端渲染器 |
| --- | --- | --- | --- |
| `preview-document`（已有） | soffice 转出的 PDF | `document` | 桌面用 iframe，手机用 pdf.js |
| `preview-sheet`（新增） | soffice 把 xls / et / ods / numbers 转成的 xlsx | `sheet` | 新增的表格渲染器 |
| `preview-html`（新增） | pandoc 转出的自包含 HTML | `html` | `HtmlRenderer`（已有） |
| `thumbnail`（扩展） | 见 B4 | `image` | 已有 |

协议只增不改：`result_type` 新增 `sheet` 和 `html`，`runtime.accept` 新增 `sheet`；Runtime 不接受时，沿用现有规则返回 `no-renderer`。

### 3.3 安全

- **soffice**：落地后在宿主上直接运行。转换时会不会按文档里的远程图片、链接去联网，还没核实（B7）；如果会，Linux 上可以用 `unshare --net` 切断网络（nfs_server 和 workflow 是 root），macOS / Windows 的办法另找。
- **pandoc**：一律加 `--sandbox`（除了输入文件，不许做任何 IO），输出再经过前端 `sandbox=""` 加 CSP 的 iframe。
- **ImageMagick**：一律指定 coder（例如 `dng:input[0]`，coder 由服务端自己的 sniff 决定），不让它自己猜格式，所以不会调到 gs（Q6）；再用命令行 `-limit` 限制 memory、map、disk、width、height、area 和 time。服务端没法自带 policy.xml（§1.3）。
- **前端表格渲染器**：解析的是用户文件，要在 Worker 里跑，并限制解压后的大小（防 zip 炸弹）。`import-table` 现有的限制要核实一下。

### 3.4 资源

- pandoc 和 magick 走现有的 media 通道（按 CPU 核数给 1–2 个并发）。soffice 不论走宿主还是容器，都独占 Office 通道，并发 1，超时取 max(120 s, 10 s/MiB)；容器超时就 `docker kill`。
- HTML 结果超过 8 MiB（前端的读取上限）时，转换失败，返回 `TOO_LARGE`。
- 表格渲染器直接读原件的大小上限先定为 30 MiB，每个 sheet 最多显示 5,000 行，超出时提示已截断。

## 4. 裁决

### 4.1 owner 裁决（2026-11-10）

| # | 问题 | 裁决 |
| --- | --- | --- |
| 基线 | LibreOffice 的地位 | **进了 paios 镜像就是核心扩展工具，命令一定可用，各环境只是调用方式不同**（§0.2）。取代了 Q1、Q3 原来“pandoc 够用的话，LibreOffice 就选装”的方向 |
| 基线 | paios 镜像里的工具从哪里来 | **都来自 ext-core。** 所以 LibreOffice 要进 ext-core 的五个平台，`build_aios` 里用 apt 装的那一组要去掉。原来的 Q8（macOS / Windows 怎样单独提供 soffice）随之取消 |
| 原则 | 系统服务自己用扩展工具时从哪里取 | **本地有就用本地的，本地没有就用容器里的** |
| pandoc 的范围 | 什么时候用 pandoc | **只用于 pandoc 肯定能处理的格式**；Office 文档一律交给 soffice（原来的推定 1 就此定下） |
| Q2 | 手机上怎么看 PDF | **桌面保留 iframe**，手机改用 pdf.js |
| Q4 | 表格用 PDF 还是网页 | **网页** |
| Q5 | Markdown 由谁渲染 | **看效果，选效果好的**（B0 比较） |
| Q6 | EPS / AI / PS 开不开 | **先不开** |
| Q9 | 在 ext-core 带上 LibreOffice 之前，要不要先拉起 aios 容器、用镜像里 apt 装的 soffice 做过渡 | **不做**，直接做 C1。C1 之前，Office 预览保持现状 |
| Q10 | ext-core 里的 Noto CJK 字体是五个平台都带，还是只在 Linux 上带 | **五个平台都带**（每个平台约 +72 MiB）：字体集算在转换指纹里，各平台要一致 |

### 4.2 待 owner 确认的推定

| # | 推定 | 理由 |
| --- | --- | --- |
| 推定 2 | 表格的“网页”由前端渲染，不用 pandoc，也不用 soffice 导出的 HTML。xlsx / csv 由前端用 `import-table` 的读取器直接读原件；xls / et / ods / numbers 先由 soffice 转成 xlsx，再交给同一个渲染器 | pandoc 会丢数字格式（§1.3）；soffice 导出的 HTML 里图表是单独的文件，宽表也不好控制。自研读取器已经能处理格式和合并单元格，desktop 可以直接用，不需要新依赖。旧格式先转成 xlsx，就只需要维护一个渲染器 |
| 推定 3 | 缩略图：先取文件里内嵌的缩略图（ODF 都带 `Thumbnails/thumbnail.png`；OOXML 只有保存时写了才有 `docProps/thumbnail.jpeg`），没有的话，如果预览 PDF 已经缓存就对它跑 pdftoppm，再不行就显示类型图标。**不为缩略图单独启动 soffice** | 原来的 Q7 裁决“能用 pandoc 就用 pandoc”，前提是 LibreOffice 选装，这个前提已经变了；而且 pandoc 产不出图片。现在要考虑的是性能：Office 通道只有 1 个并发，聊天列表或文件网格一滚动，就会排起一长串 soffice 任务，把用户真正点开的那份预览压在后面。如果 owner 接受这个代价，可以改成“低优先级地为缩略图启动 soffice”，那样需要给 Office 通道加一个优先级队列 |

### 4.3 待裁决

暂无。

## 5. 实施项

### Phase A：前端（不依赖服务端工具）

- **A1 pdf.js 渲染器（Q2）**
  - 把 `pdfDoc.ts` 的加载部分（`loadPdfjs` 和资源路径）挪到 `components/preview/pdfjs.ts`，AI Workspace 改为从这里引用。还要确认 `/embed/preview` 页面也能拿到 `pdfjs/` 资源。
  - 按 URL 用 Range 加载：`getDocument({ url, rangeChunkSize, disableAutoFetch: true })`。桶数据面的 `read_url` 支持 Range；`/nfs/v1/read/` 支不支持要核实；对象 ID 来源（`ndmSource.ts`）本来就是 Blob。
  - 竖向连续滚动，只渲染可见页和前后各一页。canvas 设像素上限（iOS 单个 canvas 约 1670 万像素）。双指缩放先用 CSS 变换，松手后按新倍率重绘。显示页码，支持跳页。加密 PDF 显示“受密码保护”，并提供下载。
  - 选择规则：`pdfViewerEnabled` 为假，或者 `matchMedia('(pointer: coarse)')` 成立时用 pdf.js；其余情况（桌面）保持 iframe。组件 ref 上的方法（`zoomIn`、`fitToView` 等）两种渲染器都要能用。
  - 测试：Playwright 模拟移动设备（Pixel、iPhone），用一份多页 PDF 和一份依赖 CMaps 的中文 PDF。
- **A2 协议类型**：websdk `ReprResult.result_type` 补上 `document`、`sheet`、`html`；`mapWireResult` 把 `sheet` 映射到表格渲染器、`html` 映射到 HTML 渲染器；`detectRuntimeProfile` 的 `acceptTypes` 加上 `sheet`。
- **A3 表格渲染器（推定 2）**
  - 新增渲染器类型 `sheet`。在 `mediaTypes.ts` 里，xlsx / xlsm / csv / tsv 改为 `sheet` 直接显示，不再算 `isPipelineDocument`；xls / et / ods / numbers 仍然交给管线。
  - 在 Worker 里用 `@table/xlsx.ts`、`@table/csv.ts` 读取，用 `@table/format.ts` 格式化数字。
  - 界面：底部是 sheet 标签（隐藏的 sheet 不显示），行虚拟滚动，表头固定，支持合并单元格和横向滚动；手机上可以双指缩放。csv / tsv 可以切换到纯文本视图。
  - 超出 §3.4 的上限时提示已截断；文件加了密码时显示“受密码保护”。
- **A4 实机核对**：在 Android Chrome 和 iOS Safari 上分别打开 PDF 原件、Office 转出的 PDF 和表格，记录 `pdfViewerEnabled` 的值和实际效果，确认 A1 的选择规则。

### Phase B：服务端，用 ext-core 现有的工具

- **B0 pandoc 评估（决定 §2.1 的 pandoc 清单和 Q5）**
  - 样本：epub、ipynb、fb2、docbook、org、rst、tex、typst 各取几份真实文件，包括中文、表格、图片、公式（pandoc 输出 MathML）和脚注；Markdown 要覆盖 GFM 表格、任务列表、多层嵌套列表、代码块、脚注和中文长文。样本由 owner 提供，或者从公开来源收集，注意授权。
  - 做法：用命令行跑 `pandoc --sandbox -s --embed-resources`，加上 B2 计划用的 CSS；用 Playwright 在手机视口（390×844）下截图。Markdown 把 MessageMarkdown 的渲染和 pandoc（`-f gfm`）的输出放在一起比较。
  - 产出：评估报告（截图加结论）。只有每个样本都能完整、正确显示的格式，才进 pandoc 清单；tex、typst 这类 pandoc 只能读一部分的格式，宁可不进。Q5 由 owner 根据报告选定。
- **B1 探测分组**：按 §3.1 改 `tools.rs` 和 `mod.rs`（`Probe`、`Converter`、`converter_id`、`PROFILES`、`refresh` 的 identity），包括 soffice 的组件检查，组件按族分别查找。C1 之前，宿主上缺 soffice 或缺组件时，在规划阶段就返回 `unsupported`，不再等转换完才报 `failed`。测试：任意一组单独缺失时，其余组照常广告；本机缺 calc 时，xlsx 由前端直接显示，xls 返回 `unsupported`（C1 之后改用 `tools/bin/soffice`）。
- **B2 pandoc → HTML（`preview-html`）**：只处理 B0 确定的清单。识别 epub 靠 zip 的第一个条目 `mimetype` 等于 `application/epub+zip`；其余按扩展名提示，同时要求内容看起来像文本。参数是 `--sandbox -s --embed-resources --metadata title=<文件名>`，加一份 nfs_server 内置的 CSS（适配窄屏排版，表格可以横向滚动）；超过 8 MiB 按 §3.4 处理。fidelity 文案写“已转换为网页显示”。
- **B3 ImageMagick 图片**
  - 识别：
    - RAW：TIFF 头，再看 DNG 标签 0xC612、CR2 在偏移 8 处的 `CR`，或者扩展名提示
    - XCF：`gimp xcf`
    - JXL：`\xff\x0a`，或者 `JXL ` box
    - JP2：`jP  ` box
    - EXR：`v/1\x01`
    - HDR：`#?RADIANCE`
    - DDS：`DDS `
    - DCM：偏移 128 处的 `DICM`
    - TGA：只能按扩展名
  - 转换：`magick <coder>:<in>[0] -auto-orient -background white -flatten -resize 4096x4096> -colorspace sRGB -quality 85 jpg:<out>`，缩略图用 640，再加上 §3.3 的 `-limit`。PSD 先比较效果再决定。
  - 测试夹具尽量自己生成；DNG 用一张小样本，注意授权。
- **B4 缩略图（推定 3）**
  - 新增 `Kind::Pdf`（`%PDF-`），只用于 `purpose: thumbnail`；`purpose: preview` 仍由前端直接显示（PRD §23.3.1）。PDF 的缩略图用 `pdftoppm -f 1 -l 1 -scale-to 640 -jpeg` 生成；加密的 PDF 返回 `unsupported`。
  - Office、ODF 和 epub（取封面）：先读文件里内嵌的缩略图，缩放到 640 以内。读 zip 用 workspace 里已有的 `zip` crate。没有内嵌缩略图、但预览 PDF 已经缓存时，对缓存的 PDF 跑 pdftoppm（PRD §9.6 允许这种两步管线）。都没有就返回 `unsupported`，前端显示类型图标。
  - B4b（可选，可以单独排期）：Files 的网格视图里，能出缩略图的类型改用 `usePreviewThumbnail`，只给可见项请求，并限制并发。
- **B5 补音视频识别**：ASF（`30 26 B2 75 8E 66 CF 11`）、FLV（`FLV\x01`）、RM（`.RMF`）、AIFF / AIFC（`FORM....AIFF`）、CAF（`caff`）、APE（`MAC `）。测试样本用 ffmpeg 生成。
- **B6 修正文档识别**
  - 扩展名补判表加上 `wps / wpt → doc`、`et / ett → xls`、`dps / dpt → ppt`，以及 OOXML 的变体（`docm / dotx / dotm / xlsm / xltx / pptm / ppsx / potx`）、iWork（zip 里有 `Index/Document.iwa`）、`vsdx`、`pub`、`hwp`、`wpd`。前端的 `PIPELINE_DOCUMENT_TYPES`（`mediaTypes.ts:52`）同步修改。
  - OLE2 大文件：按文件头里的目录扇区号补读一次目录，不再只看前 1 MiB。
- **B7 宿主 soffice 的外链核查**：用带外链图片的 docx 加抓包，确认宿主上的 soffice 转换时会不会联网。结论写进本文；如果会，按 §3.3 断网。

### Phase C：落实 soffice 基线

- **C1 LibreOffice 进 ext-core（五平台）**
  - **取包**：Linux 用官方 deb tarball 解包（x86_64、aarch64）；macOS 用官方 dmg 里的 LibreOffice.app（arm64、x86_64 分别打）；Windows 用官方 msi 做管理安装（`msiexec /a`）解包。都只保留无界面转换需要的部分。§1.3 的 Linux 子集是 core、ure、writer、calc、impress、draw、math、graphicfilter、xsltfilter、自带字体加启动器。不要 pyuno、内置 Python、图标主题、数据库驱动、扩展和词典。
  - **系统库（Linux）**：soffice 依赖的系统库从 conda-forge 一起带上（§1.3）。用 `readelf` 列出全部 NEEDED，逐个落实来源。
  - **launcher**：soffice 的 launcher 按 ext-core 现有规范写（只指向 `store/` 里的文件，不回退到 PATH）。Linux 上还要设好库路径和 `FONTCONFIG_FILE`。Windows 的 launcher 指向 `soffice.com`。字体放进 LibreOffice 自己的 `share/fonts`，三个平台都会自动加载（Q10）。
  - **smoke**：用一份中英混排、带一张图的 docx，以及一份 xlsx，转成 PDF，再用 `pdftotext` 检查中文不是方块。Linux 的 smoke 要在不带 LibreOffice 依赖的干净 Ubuntu 24.04 容器里跑，证明自包含。
  - **清单和文档**：
    - `ext-manifest.json` 的 `provides` 加上 `soffice`。
    - paios 容器需求 §6.4：表 B 的 `soffice` 一行和过渡说明已于 2026-11-10 写入，`ext-office` 已从表 C 删除。实施完成后，去掉“待实施”标记，把表头的命令数改成 24。`OpenDAN Agent-Session-Runtime.md` §3（第 150 行）里“规划中的 ext-office”的说法同步修改。
    - `baseline::EXTENDED_TOOLS` 同步，一致性测试会检查。
    - ext-core 升版本（1.1.0 → 1.2.0）。
    - 发布流程不变：GitHub Release 单个资产不能超过 2 GiB，`.pikg` 只放 R2。
  - **镜像**：`build_aios` 去掉 `libreoffice-*-nogui`、四个字体包和 `locales`，只留 `bash`、`ca-certificates`、`rsync`。UTF-8 locale 改由 launcher 里的环境变量设置，`C.UTF-8` 本来就有。注释和 paios 容器需求 §6.3 同步改。镜像重建后跑 `xagent runtime baseline --check`。
- **C2 补测格式**：C1 落地后，实测 §2.1 里标“未实测”的格式（odg、vsdx、pub、hwp、wpd、iWork）。C1 的子集带了 draw，现在镜像里缺 draw 的问题也就不存在了。
- **C3 基线文档**：媒体工具基线 §1 和 §3 改成“soffice 和字体来自 ext-core”，删掉“宿主要另外装 LibreOffice 和字体”的要求；§6 里“宿主安装脚本没装 LibreOffice”这个缺口就此关闭。workflow 的 `media.to_pdf` 不用改代码，查找顺序相同，自然会找到 ext-core 里的 soffice。注意：它的指纹里有 soffice 版本和字体摘要，换成 ext-core 里的 soffice 之后，旧缓存会失效一次。

### Phase D：验收

- **样本矩阵**：§2 的每种格式至少放一个样本到 `tests/fixtures/preview/`，`tests/preview.rs` 按组跳过缺失的工具（沿用 `soffice_present()` 的写法）。容器路线加一组集成用例，没有 docker 时自动跳过。前端表格渲染器的用例放进 desktop 的 Playwright 测试。
- **工具缺失注入**：
  - 宿主机上没有 LibreOffice 时，docx 用 `tools/bin/soffice`；
  - 再把 `tools/bin/soffice` 也拿掉，docx 返回 `unsupported`，并说明是部署缺陷；
  - 宿主上的 LibreOffice 缺 calc 时，xls 改用 `tools/bin/soffice`；
  - 去掉 pandoc 时，epub 返回 `unsupported`。
- **真实 Zone**：用普通成员的账号，在真机手机上从聊天附件和 Files 打开 docx、doc、pptx、xlsx、xls、csv、PDF、epub、md、DNG，检查缩略图，并确认第二次打开命中缓存。
- **性能基线**：在一台 N100 级别的 OOD 上记录以下数据，写进本文：
  - 20 页 docx 转换耗时（宿主 soffice 和容器各一次）
  - 20 MB 的 DNG、5 MB 的 epub 转换耗时
  - 5 万行的 xlsx 在手机上首屏显示的时间

## 6. 要同步的文档

- PRD：§23.8 第 1、4 项（首批 Pipeline 清单、PDF 增强的顺序），§9.2 的结果族加上 `sheet`。
- `src/frame/desktop/src/app/preview/README.md`（Providers、Known gaps）。
- `src/frame/nfs_server/README.md` 的 preview 部分。
- 媒体工具基线（C3），paios 容器需求 §6.3、§6.4 表 B 和表 C（C1）。
