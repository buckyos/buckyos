# BuckyOS AI Workspace 多媒体对象

> 状态：已实施（2026-10-26），未提交，未在 DV 部署。
> 范围：AI Workspace 画布插入并显示图片以外的媒体与文件——视频、音频、PDF 及任意文件。

## 1. 原则

1. 任何媒体文件尽可能得到一张图，插入结果与插入图片一样：Block 按这张图的比例放下（长边最多 480），画布上看到的就是这张图（视频尤其如此）。
2. 能在画布里播放的，就在画布里播放，播放用内嵌的系统 Preview 组件（`components/ContentPreview.tsx`）。
3. 不适合在画布里播放的，显示图（或图标），双击弹出 Preview。

数据模型不新增实体类型：所有文件仍是 `buckyos.asset-ref`，仍由 `asset` Block 显示；“一张图”是 AssetRef 的 `poster`。

## 2. 种类

前端按媒体类型分五类（`ui/media/kinds.ts`）。媒体类型以后端从字节嗅探的为准；后端给出 `application/octet-stream`（旧上传、后端不认识的格式）或 ZIP 容器时，用文件扩展名的类型。

| 种类 | 条件 | 画布上 | 打开方式 |
| --- | --- | --- | --- |
| image | 浏览器能直接显示的图片（含 SVG、AVIF） | 图片本身 | 双击 / 预览：Preview 弹窗 |
| video | `video/*` | 帧作 poster + 播放钮 + 时长 | 浏览器能播放：画布内播放；否则 Preview 弹窗 |
| audio | `audio/*`、`application/ogg` | 有封面：封面 + 播放钮 + 时长；无封面：图标卡片 + 播放钮 | 同上 |
| pdf | `application/pdf` | 首页作 poster + “N 页” | Preview 弹窗 |
| file | 其它（含 HEIC/TIFF 等浏览器不能直接显示的图片） | 图标卡片（类型徽标、文件名） | Preview 弹窗（文本、HTML 等能预览；其余显示不支持 + 下载） |

## 3. 插入：一张图与内容事实

插入（拖放、粘贴、对象工具栏、插入目录、流式页插入、替换文件）都经 `ui/media/stage.ts` 的 `stageMediaFile`：先在浏览器里得出内容事实（`ui/media/poster.ts`，15 秒超时，失败只是没有 poster），再把文件和 poster **并行**上传，提交 AssetRef。poster 上传失败不影响文件本身。

| 种类 | poster | 其它事实 |
| --- | --- | --- |
| image | 不需要（图片本身） | `image`（解码后的尺寸，含 EXIF 方向） |
| video | 视频 10% 处（最多 1 秒）的帧，JPEG，长边 ≤ 1280 | `image`（帧尺寸）、`duration_ms` |
| audio | 内嵌封面（§3.2），重编码为 JPEG | `duration_ms`；有封面时 `image` |
| pdf | 首页（§3.3），JPEG，长边 1280 | `image`、`page_count` |
| file | 无 | 无 |

### 3.1 尺寸

有 poster（或是图片）时 Block 取其比例，长边最多 480；无 poster 的视频 320×180；卡片 300×88。插入目录给了固定尺寸的入口不改尺寸。Block 仍是 `resize: locked`。

### 3.2 音频封面

`ui/media/audioCover.ts` 只解析字节：ID3v2.2 `PIC`、ID3v2.3/2.4 `APIC`（含解同步、v2.4 数据长度指示）、FLAC `PICTURE` 块、MP4 `moov/udta/meta/ilst/covr`。前封面（picture type 3）优先。Ogg 注释里的 `METADATA_BLOCK_PICTURE` 不读。

### 3.3 PDF 首页

用 pdf.js（`pdfjs-dist@6.4.299`，只在插入 PDF 时动态加载，worker 单独打包）渲染首页。未内嵌字体回退到浏览器字体；未打包预置 CJK CMap，依赖预置 CMap 的老式中文 PDF 首页文字可能缺失（已知缺口）。

## 4. 画布上的显示与交互

显示与播放在 `ui/media/AssetMedia.tsx`，`AssetCell`（`ui/blocks/editors.tsx`）负责读取与可用性，`asset` Block 定义在 `ui/blocks/builtin.tsx`。

### 4.1 操作

| 场景 | 单击 | 播放钮 | 双击 | 工具栏 |
| --- | --- | --- | --- | --- |
| 自由画布·编辑 | 选中 / 拖动 | 激活 Block（Editor 挂载即播放） | 激活 Block：可播放的就地播放，其余打开 Preview 弹窗 | 播放、替换、适应方式（图/视频/PDF）、预览 |
| 自由画布·查看 / 演示 | 选中 | 就地播放 | 同上（`RenderHost` 发 `open` 意图） | 播放、预览 |
| 流式页 | — | 就地播放 | — | 无（文件信息与“替换…”在图下方） |
| 数据源详情 | — | 就地播放 | — | — |

编辑模式下播放走 Block 的激活：`Static` 不播放（BlockRegistry 约定），点别处结束激活即停止播放。Enter 同双击。工具栏“预览”在任何模式都打开弹窗。

### 4.2 画布内播放

播放时在 Block 里挂 `ContentPreview`，`variant="inline"`、`autoPlay`：不显示 Preview 的工具栏、翻页和信息面板，保留原生媒体控件；音频用紧凑舞台（文件名 + 控件）。外层标 `data-role="editor"`，画布不抢指针与按键；右上角“停止播放”按钮，Esc 也停止。视频播放时盖住 poster；有封面的音频把控件放在封面底部。

为此 Preview 组件新增两个宿主属性：`variant: 'default' | 'inline'`、`autoPlay`（`app/preview/README.md`）。

### 4.3 Preview 来源与弹窗

资产以宿主 Source Resolver `aiworkspace-asset` 交给 Preview（`ui/media/source.ts`）：来源值带工作区 id、`object_id`、媒体类型、名称，以及该工作区会话的读取函数（函数不进身份），解析结果是带正确类型的 blob。弹窗挂在 `.aiws-root` 上（画布世界有 transform，不能用 fixed），会话是同一 Surface 上所有可用资产（大纲顺序），可前后翻页。弹窗与内嵌播放器外层有 `aiws-preview-host`，Workspace 的基础 `button` 样式不作用于 Preview 组件内部。

## 5. 其它消费方

- **draw.io 导出**：图片导出本身，其它文件导出 poster，没有 poster 只导出标题。
- **包导出 / 导入、离线副本、保留与回收**：poster 是 AssetRef 引用的第二个对象，和 `object_id` 一样进入引用边、保留集、导出包与副本资产表（`types::asset_object_ids`）。
- **读权限**：能读 AssetRef 的人能读它的 poster（`asset` 引用边）。

## 6. 后端

- **AssetRef 新键**（`core/src/types.rs` `validate_asset`）：`poster`（已上传的 PNG/JPEG/WebP/GIF 资产的 object id；SVG 不行，它是活动内容）、`duration_ms`、`page_count`（正整数）。`object_id` 改变而没有同时给出新值时，`image`、`poster`、`duration_ms`、`page_count` 一并删除（`ASSET_CONTENT_KEYS`）。
- **嗅探**（`store/src/objects.rs` `sniff`）：新增 MP4/QuickTime/3GP/M4A（按 `ftyp` 品牌，HEIC/AVIF 也是 `ftyp`）、WebM/Matroska、Ogg（Theora 为视频）、WAV、AVI、FLAC、MP3（ID3 或帧头，排除 UTF-16 BOM）、AAC ADTS、BMP、TIFF。
- **asset 路由**：图片之外，`audio/*`、`video/*` 也按真实类型返回（被动内容）；SVG、HTML 等仍是 `octet-stream`。

## 7. 限制与未做

- 单个资产仍限 32 MiB（`max_asset_bytes`），较长的视频放不进；asset 路由整段读入内存、不支持 Range，前端也整段下载成 blob 再播放。放宽上限需要流式读取 + Range（以及 `<video>` 能带凭据的地址），未做。
- 数据树导入（文件夹导入、数据树“导入图片”）不生成 poster：这些视频在画布上是无 poster 的 320×180 黑框 / 卡片。
- 浏览器不能播放的视频（多数 MKV/AVI、H.265）抽不到帧，显示为卡片，弹窗里是“不支持 + 下载”（真实模式下 nfs_server 没有 repr 管线）。
- 音频没有封面时不生成波形图。
- 触屏：没有双击打开，用播放钮和工具栏。
- Agent 侧（`aiworkspace_run` 程序宿主、许愿格上下文）不读 poster。

## 8. 验证

- Rust：`cargo test -p aiworkspace-store --lib objects`（嗅探）、`cargo test -p aiworkspace-store --test packages media_assets`（poster 校验、读权限、保留、副本、导出导入、替换清键）；三 crate 全量通过。
- 前端单测：`deno test --no-check --allow-env --allow-read --node-modules-dir=manual --unstable-sloppy-imports tests/datamodel/aiworkspace-media.test.ts`（封面解析、种类、时长与徽标）。
- e2e：`tests/aiworkspace/media.spec.ts` MD-01～MD-03（拖入 WebM 得帧 poster 并就地播放；WAV 卡片播放；PDF 首页 poster 与弹窗；文本文件卡片、弹窗翻页；查看模式双击播放）。headless Chromium 没有 PDF 插件，弹窗里的 PDF 是降级态。
