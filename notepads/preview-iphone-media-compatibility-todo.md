# Preview：iPhone 图片与视频兼容 TODO

日期：2026-10-28（同日按源码 review 回写 §1、新增 §2 设计决策）

状态：P0–P2 已实施（2026-10-28，未提交）；自动化验证与 DV（本机 Zone 全新安装、headless Chromium/Linux）全链路见 §7；真实 iPhone / Safari 实机与 Windows/macOS 三平台验收未做。本文记录源码核对结果、开工前设计决策、开发顺序与验收范围。

目标：共享 `ContentPreview` 能可靠预览 iPhone 常见的 HEIC/HEIF 图片，以及 MOV/MP4 容器中的 H.264、HEVC 视频。浏览器能解码时直接显示；不能解码时生成并缓存兼容的预览副本，保留原文件。

关联：[HomeStation 视频播放问题 #692](https://github.com/buckyos/buckyos/issues/692)。该问题的播放器接入修复验证了 H.264/AAC MP4，不能据此认为 HEIC、HEVC 或 iPhone 实机兼容已通过。

## 1. 当前实现与缺口

以下链接相对本文位置，现状以对应源码为准。

| 范围 | 已确认的现状 | 缺口 |
| --- | --- | --- |
| 图片识别与能力判断 | [mediaTypes.ts](../src/frame/desktop/src/components/preview/mediaTypes.ts) `sniffMagic` 已识别 heic/heix/hevc/mif1/msf1 五个品牌，统一产出 `image/heic`；原生图片支持列表只有基础格式与异步探测的 AVIF | 没有 HEIC 解码探测，Safari 17+ 原生可显示的 HEIC 也被 `decideDirect` 判为 `runtime-unsupported`；缺 [imageNormalize.ts](../src/frame/desktop/src/app/messagehub/conversation/input/imageNormalize.ts) 已覆盖的 hevx/heim/heis/hevm/hevs 五个品牌；`image/heif` 从未产出 |
| 视频能力判断 | 同文件 `detectRuntimeProfile` 用裸容器 MIME 过 `canPlayType`，`decideDirect` 按 MIME 精确匹配 | **误拒**：Chromium 对 `video/quicktime` 返回空串，H.264 的 MOV 也在直放前被拦下；容器通过不代表能解其中的 HEVC |
| 直放失败回退 | [ContentPreview.tsx](../src/frame/desktop/src/components/ContentPreview.tsx) `handleRenderFailure` 已做 Direct 失败 → `skipDirect` → `ensurePreviewWork`；`<video>` 按 `MediaError` 区分编码不支持/损坏/网络 | `<img onError>` 一律报 `unsupported-encoding`，管线不可用且魔数已确认类型时落到"文件可能已损坏"（`failure && classification.confirmed`），HEIC 在 Chromium 上文案错误 |
| 转换回退 | [nfspProvider.ts](../src/frame/desktop/src/components/preview/nfspProvider.ts) 已预留 `repr/get_repr` 调用与临时 wire shape；[nfs_server/types.rs](../src/frame/nfs_server/src/types.rs) 明确未实现、未广告 `repr`；`/nfs/v1/read` 只服务 Native 节点，派生对象没有读取落点 | 真实环境返回 `no-pipeline`，没有图片解码或视频转换兜底；nfs_server 是 host 上的 kernel 服务，FFmpeg 只在 aios 容器镜像里，host 侧没有分发 |
| Mock 能力 | [mockProvider.ts](../src/frame/desktop/src/components/preview/mockProvider.ts) 有 `raster-decode` 和 `media-transmux` | 这是模拟管线，不能作为正式格式支持或转换成功的证据 |
| HomeStation 内容读取 | [media.ts](../src/frame/desktop/src/app/homestation/media.ts) 用 `response.blob()` 读完整文件后交给 Preview，`fetchContent` 未接 `signal`；服务端 [http.rs](../src/frame/homestation/src/http.rs) `object_content` 把整个文件读进内存返回，无 `Accept-Ranges`，上限 64 MiB | 大视频首播等待长；读取未接入取消信号；Range 缺口在服务端，不只在客户端。同一份内容上传时已绑定到 nfs-server 的 `scratch`/`published` 桶（[transport.ts](../src/frame/desktop/src/app/homestation/api/transport.ts) `importAttachment`），桶数据面 `bucket_store::serve_read` 已支持 Range 与 bkt_ 令牌 |
| HomeStation 业务页面 | [card/parts.tsx](../src/frame/desktop/src/app/homestation/card/parts.tsx)、[detail/ItemDetail.tsx](../src/frame/desktop/src/app/homestation/detail/ItemDetail.tsx) 封面、[ImmersiveMode.tsx](../src/frame/desktop/src/app/homestation/ImmersiveMode.tsx) 背景与媒体格直接 `<img src={mediaUrl()}>` | 这些入口绕过 Preview，共享 Preview 修好后信息流仍显示不兼容原件 |
| 上传与 MIME | [api/store.ts](../src/frame/desktop/src/app/homestation/api/store.ts) `uploadMeta` 要求 `file.type` 以 `image/` 开头且客户端能解码出宽高；[PublishComposer.tsx](../src/frame/desktop/src/app/homestation/publish/PublishComposer.tsx) 按 `File.type` 分类；[fsutil.rs](../src/frame/nfs_server/src/fsutil.rs) 有 `.heic/.mov` 映射，缺 `.heif`（只影响 `/nfs/v1/read`，桶数据面用上传时的 `content_type`）；websdk `content_mime.ts` 已有 heic/heif/mov | **硬阻断**：Chromium 下 HEIC 的 `File.type` 为空且不可解码，媒体上传两道都过不去，只能作为普通文件附件 |

[MessageHub 的 imageNormalize.ts](../src/frame/desktop/src/app/messagehub/conversation/input/imageNormalize.ts) 可在浏览器能解码 HEIC 时转成 JPEG，失败则返回原文件。可参考其识别与解码流程，但它不是通用 HEIC 解码器，也不能代替共享 Preview 的服务端兜底。

服务端已有的相关能力：[agent_tool image_fit.rs](../src/frame/agent_tool/src/llm_understand_media/image_fit.rs) `convert_with_ffmpeg` 按 PATH 裸名调用 ffmpeg，并用 `buckyos_api::hide_child_console_async` 处理 Windows 控制台，是"有则用"的现成先例；Rust `image` crate 0.25（agent_tool、control_panel 在用）解不了 HEIC/HEVC，不是替代；`bucket_store` 已有 Range、生命周期、GC、bkt_ 令牌。

## 2. 开工前设计决策（已定）

| 编号 | 决策 | 取值 |
| --- | --- | --- |
| D1 | 转换器位置与部署 | nfs_server 进程内直接 spawn `ffmpeg` / `ffprobe`，**有则使用，没有就失败**：启动时与每次 `repr` 前探测，探测到才在 `hello` 广告 `repr`，没有则 `repr` 返回 `unsupported`（reason `no-pipeline`，detail 注明缺哪个工具），不做任何隐式下载或安装。不走 TaskMgr、不派给容器（PRD §23.6 的 TaskMgr 形态留到跨节点需求出现时）。测试机先手动装好 ffmpeg/ffprobe。 |
| D2 | 工具探测的全平台适配 | 因为有 Lite 版本，进入 Agent 标准环境的 CLI 工具都按 Windows / macOS / Linux 三平台写：查找顺序 `$BUCKYOS_ROOT/libexec/buckyos-tool/bin/` → PATH，Windows 加 `.exe` 后缀并用 `hide_child_console_async`；路径含空格与非 ASCII 用 `Command::arg` 逐个传，不拼 shell 字符串；temp 目录放 nfs_server 数据根下；版本字符串用 `ffmpeg -version` 首行解析，进缓存 key。探测结果与版本在 `hello` 的 `repr` 能力描述里可见，便于 DV 判断走的是哪条路。 |
| D3 | 派生产物存储与鉴权 | 产物作为 nfs_server 自有系统桶（`preview-cache`）里的绑定，复用桶的 Range、生命周期、GC；`repr` 先按源对象 ACL 判权，通过后在结果里签短期、只读、绑定级的 bkt_ 令牌作为 `read_url`。缓存命中同样先判源对象权限再签令牌，不因命中绕过鉴权。原文件字节、对象 ID、"下载原文件"保持原样。 |
| D4 | `repr` 协议冻结 | 以 nfspProvider 现有临时 wire shape 为基础冻结：`work_key = hash(input_obj_id, converter_id@version, params_hash)`；runtime profile 从容器 MIME 列表扩成编码能力（`image: [...]`、`video_codecs: { h264, hevc }`、`heic: bool`）；`unsupported` 是规划结果、`failed` 是执行失败，按 PRD §23.7 不混用；状态记录与 attempt/retry CAS 按 PRD §23.6，用 nfs_server 自有 SQLite。 |
| D5 | Target Profile 固定档位 | 不把 viewport 宽高/dpr 原样进 key，服务端归并成两档：`preview-image`（JPEG q85、长边 ≤4096、sRGB、EXIF 方向烘进像素、透明度→白底时在 fidelity 注明）、`preview-video`（H.264 High ≤1080p、yuv420p、AAC 128k、`+faststart`、旋转元数据烘进像素）；HDR 一律 tonemap 到 SDR BT.709 并在 `fidelity` 注明"HDR 已映射为 SDR"。`purpose: thumbnail` 另一档（JPEG 长边 ≤640）。MOV 里编码已可播时只重封装（`-c copy`），HEVC 不可播时转码。 |
| D6 | 视频 Direct 判定 | 容器属于 mp4/mov/webm/ogg/mkv 时一律先直放，`canPlayType` 带 codecs 的结果只作为发给服务端的能力提示，不做门禁；`<video>` 的 `MEDIA_ERR_SRC_NOT_SUPPORTED` / `MEDIA_ERR_DECODE` 才是回退管线的信号。图片侧增加 2×2 HEIC 探针（与 AVIF 探针同款，带超时与释放），探测到才把 `image/heic`、`image/heif` 加进原生列表；管线不可用时错误文案改为"此浏览器不支持该图片格式"，不再报"损坏"。 |
| D7 | 上传契约 | `uploadMeta` 的宽高改为可选，客户端解码失败不阻断上传；HEIC 尺寸从 HEIF `ispe` box 读，不需解码；媒体分类按魔数（复用 `sniffMagic`），不依赖 `File.type`；`object.register` 允许缺省宽高，管线完成后由服务端回写或由 Preview 的 `onReady` 补。 |
| D8 | 信息流 `<img>` 走哪条路 | 卡片、封面、沉浸模式背景、详情页封面一律用 `purpose: thumbnail` 的派生 JPEG（复用 [thumbnail.ts](../src/frame/desktop/src/components/preview/thumbnail.ts) 的管线路径，或服务端按同一 work_key 预生成），只有详情页与沉浸模式的 `ContentPreview` 才拿原件。副作用：信息流流量减小。 |
| D9 | HomeStation 内容 URL | `fileSourceRef` 改为解析成 `bucket-object` 引用（服务端 `object.*` RPC 返回绑定的 bucket/key 与非 owner 读者所需的 bkt_ 令牌），Preview 走桶数据面的 Range；`object_content` 保留给旧链接与下载，但改为流式并加 `Accept-Ranges`。网关对 Range/206 的透传在 cyfs-gateway 仓，本仓无法核对，列入验收。 |
| D10 | 资源限制 | ffmpeg 并发 1–2（按 CPU 核数配置）、单任务超时 = max(60 s, 源时长 × 10)、同一 work_key 合并等待、客户端离开不杀进程（产物仍入缓存）；`preview-cache` 桶按最近未访问 30 天过期，总容量上限可配。失败记录负缓存 10 分钟，用户显式 Retry 立即新 attempt。 |

待实机确认：iOS Safari 的 `<input type=file accept="image/*">` 在"照片 → 传输到 Mac 或 PC：自动"的默认设置下会把 HEIC 转 JPEG、HEVC 转 H.264 再上传；真正的 HEIC/HEVC 原件多来自 Mac 照片导出、Files 应用或"保留原片"设置。验收矩阵按此写明用哪个设置。

## 3. P0：修正原生预览路径

- [x] D6：HEIC 解码探针进 `ensureRuntimeProfile`，有超时和资源释放；探测到才把 `image/heic`/`image/heif` 加进原生列表。
- [x] 补齐 HEIF 品牌（hevx/heim/heis/hevm/hevs）、`.heif` 扩展名（fsutil.rs）和 MIME 识别，覆盖大写后缀、空 `File.type`、`application/octet-stream`；复用 `sniffMagic` / websdk `content_mime.ts`。
- [x] D6：视频 Direct 改为按容器先试播，`canPlayType`（含 codecs）只作提示并进 runtime profile；保留 `MediaError` 驱动的回退。
- [x] 修正 `ContentPreview.tsx` 回退后的错误归类：图片直放失败且管线不可用 → `UNSUPPORTED_ENCODING`（"此浏览器不支持该图片格式"）而非 `CORRUPTED`；区分无访问权限、内容不存在、网络失败、编码不支持与内容损坏，并提供重试或下载入口。
- [x] D7：HomeStation 上传不再要求客户端解码出宽高、不按 `File.type` 分类；HEIC 在 Chromium 上能作为图片附件上传。
- [x] `media.ts` 的 `fetchContent` 接入 `resolvePreviewSource` 传入的 `signal`。

## 4. P1：补齐真实转换与缓存

- [x] D1/D2：nfs_server 的 ffmpeg/ffprobe 探测模块（三平台查找、版本解析、`hello` 能力广告、缺失即 `unsupported`）。新增 Rust 依赖按仓库规则确认；不引入 libheif / ffmpeg 绑定 crate。
- [x] D4：冻结 `repr/get_repr` 协议并同步服务端、websdk、nfspProvider 三处；状态记录、attempt、retry CAS、重启恢复按 PRD §23.6。
- [x] D3：`preview-cache` 系统桶、按源 ACL 判权后签短期 bkt_ 令牌、缓存命中仍判权。
- [x] D5：`preview-image` 档：HEIC/HEIF → JPEG，正确处理方向、尺寸、透明度与色彩；HDR → SDR 并注明。
- [x] D5：`preview-video` 档：先用 ffprobe 判编码，MOV 中编码已可播则重封装，HEVC 不可播则转 H.264/AAC MP4；旋转元数据烘进像素。
- [x] D10：并发、超时、合并等待、负缓存、桶过期与容量上限；失败后重试。
- [x] 原文件字节、对象 ID 与"下载原文件"保持原样。

## 5. P2：大文件与业务入口体验

- [x] D9：HomeStation 媒体 `fileSourceRef` 解析成 `bucket-object` 引用走桶数据面 Range；`object_content` 改流式并加 `Accept-Ranges`；验证网关对 Range/206 的透传（DV 已验：`/bucket/v1` 206 与视频拖动）。
- [x] 取消信号贯穿内容读取；切条或关闭后停止旧播放、释放资源，丢弃过期任务结果。
- [x] D8：信息流卡片、封面、沉浸模式背景、详情页封面改用 `thumbnail` 派生 JPEG；检查上传预览、文件浏览器和消息附件入口，避免共享 Preview 修好后业务页面仍直接用不兼容原件。
- [ ] `ndmSource.ts` 的 object-id 来源同样整读为 Blob（256 MiB 上限），MessageHub 附件视频有同样的首播等待，另列范围处理。

## 6. 验收

使用真实文件，记录浏览器、系统版本、iOS 照片传输设置，以及实际走的是原生还是转换路径（`hello` 的 `repr` 能力描述与 Preview 信息面板的 Direct/Pipeline 标记为证据）；至少覆盖 iOS Safari、macOS Safari、桌面 Chromium（macOS 有硬解 HEVC、Linux 无）。HEVC 需覆盖原生支持与不支持两种环境；ffmpeg 缺失的环境需验证 `repr` 不广告、Preview 给出"此浏览器不支持该格式"而非"损坏"。

| 样本 | 必须验证 |
| --- | --- |
| 真实 iPhone HEIC/HEIF，含竖拍照片 | 原生支持时直显；不支持时转换；方向、比例与颜色正确；Chromium 下能作为图片上传 |
| H.264 + AAC 的 MOV | Chromium 直放不再被 `video/quicktime` 误拒；画面与音轨正常；必要时重封装；播放、暂停、拖动进度可用 |
| HEVC + AAC 的 MOV / MP4 | 原生支持时直放；不支持时真实转换；时长、音画同步正确 |
| 竖拍、HDR 视频 | 旋转元数据正确，预览不横倒；HDR → SDR 映射效果可接受且 fidelity 注明 |
| JPEG、PNG、H.264/AAC MP4 | 既有图片与视频预览无回归 |

- [x] 在真实服务环境完成"iPhone 文件选择/上传 → Publish → 详情播放或显示 → 沉浸模式"全链路；补齐测试环境的上传代理，不能只用 mock feed 或拦截响应验收。（DV `tests/dv/preview-iphone-media.spec.ts`，样本为 libheif/x265 编码而非 iPhone 实拍，浏览器为 headless Chromium/Linux）
- [ ] 验证权限不足、文件不存在、网络中断、解码失败、转换器不可用（ffmpeg 缺失）、任务失败后重试的可见提示。
- [ ] 验证重复打开复用转换结果（同一 work_key）、非 owner 读者命中缓存仍需权限、切条后旧音视频停止、下载原文件字节未变。
- [ ] 验证 Windows / macOS / Linux 三平台的 ffmpeg 探测与调用（含路径含空格、非 ASCII 路径、Windows 无控制台弹窗）。
- [ ] 自动化使用真实 HEIC/HEVC 样本；PNG 改名为 `.HEIC` 或模拟媒体错误只能验证分支，不能证明格式兼容。

首轮不承诺 Live Photo 的图片/视频联动、ProRAW、ProRes 或无损 HDR 编辑；这些能力另行定义范围。

参考：[WebKit：Safari 17 的 HEIC 支持](https://webkit.org/blog/14445/webkit-features-in-safari-17-0/#heic)、[MDN：HEVC 编码与浏览器支持](https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/Video_codecs#hevc_h.265)、[BuckyOS Preview PRD §23](../product/bucky_file/BuckyOS%20Preview%20App-Component%20PRD.md)。

## 7. 实施记录（2026-10-28）

落点（均未提交）：

| 范围 | 文件 | 要点 |
| --- | --- | --- |
| 原生路径 | Desktop `components/preview/mediaTypes.ts`、`types.ts`、`io.ts`、`ContentPreview.tsx`、`thumbnail.ts`、i18n | HEIF 十个品牌 + `mif1`/`msf1` 看兼容品牌（`image/heif` 只给泛 HEIF）；2×2 HEIC 探针（3 s 超时、释放 `src`）；runtime profile 增 `videoCodecs {h264,hevc}`、`heic`；mp4/mov/webm/ogg/mkv 一律先直放；`loadedmetadata` 宽高为 0 也回退管线；图片直放失败且管线不可用 → `UNSUPPORTED_ENCODING`「此浏览器不支持该图片格式」，只有 Runtime 原生支持且魔数确认的类型才报损坏；`MEDIA_ERR_DECODE` 仍报损坏（不支持的编码 Chromium 报 SRC_NOT_SUPPORTED）；权限 / 不存在也给重试；探测时服务端返回的 HTTP 错误（`PreviewError.status`）直接显示；视频直放失败后向管线声明 `hevc:false`；`heifImageSize`（`ispe` + `irot`）。 |
| 上传 | HomeStation `api/fileKind.ts`、`api/store.ts`、`publish/PublishComposer.tsx` | 魔数判类型（Chromium 上 HEIC 的空 `File.type`），宽高 / 时长尽力而为，HEIC 尺寸读 `ispe`；文件选择器显式 accept `.heic,.heif,.mov`。服务端 `object.register` 本就不要求宽高，未改。 |
| 转换器 | nfs_server `src/preview/{tools,heif,media,convert,rpc,mod}.rs`、`server.rs`、`state.rs`、`filedb.rs`、`config.rs`、`main.rs` | 见 nfs_server README「预览管线」。ffmpeg 7.1 不处理 HEIF `irot`/`imir`、不自动拼 tile grid：自写最小 `meta` 解析 + ffprobe stream group 生成 `xstack`/`crop`/`transpose` 图。macOS 额外查 `/opt/homebrew/bin`、`/usr/local/bin`（launchd PATH 不含 Homebrew）。 |
| 缓存 | buckyos-api `nfs_share.rs`（`NFS_PREVIEW_CACHE_BUCKET`、`nfs_preview_cache_bucket()`）、`nfs_server_client.rs`（`preview_cache_mib`，默认 20480） | **偏离 D3/D10 的"cache 桶"写法**：桶的 `cache` 档不 pin，GC 宿主每 30 s 驱逐 class 0 对象，派生物会立刻丢；改用 pin 的 `app_data` zone 桶，30 天未访问过期与容量 LRU 由 nfs_server 按 `preview_works.accessed_at` 自己做。 |
| 协议 | websdk `src/nfsp.ts`（`ReprArgs/ReprRecord/ReprCapability`、`repr()`、`getRepr()`、`reprCapability`，dist 已重建）、Desktop `nfspProvider.ts` | 冻结 wire；`at` / `source.bucket` / `source.obj_id` 三种源；`target` 等旧临时字段删除（服务端拒绝未知参数）。 |
| 业务入口 | HomeStation 后端 `media_ref.rs`、`http.rs`、`objects.rs`；Desktop `homestation/media.ts`、`MediaThumb.tsx`、`card/parts.tsx`、`detail/ItemDetail.tsx`、`ImmersiveMode.tsx`、`api/transport.ts`、`components/preview/bucketSource.ts` | 新 `GET /home/<user>/objects/<id>/ref`（与 `/content` 同一读者鉴权）返回 published/following/scratch 中的绑定 + 6 h 只读 key 令牌（按列桶建索引，10 s 内不重建，命中再 `head` 复核，令牌复用）；`/content` 改流式 + Range + `Accept-Ranges`（去掉 64 MiB 整读）；Preview 的 HomeStation 源先解析成桶引用（`providerHandle`），没有绑定才读 `/content` URL（不再整读 Blob）；卡片 / 封面 / 沉浸背景 / 详情封面用 `MediaThumb`（`thumbnail` 派生 JPEG，`prefer:'pipeline'`，失败回原件）；沉浸模式图片改用 `ContentPreview`。 |

自动化验证（本机 Debian 13，ffmpeg 7.1.5 由 apt 安装）：

- `cargo test -p nfs_server`（含新 `tests/preview.rs` 6 例：真实 HEIC tile grid 竖拍方向与像素、缓存复用与同 work_key、非 owner 无权 ensure/查询、缩略图尺寸、alpha 白底、PQ HEIC 与 HLG 视频 tonemap、H.264 MOV 重封装、HEVC 转码 / 可播时重封装 hvc1、竖拍视频旋转烘焙、损坏输入负缓存与 Retry CAS、PDF/目录为规划 unsupported、未知参数拒绝、桶源无权 → 令牌放行后命中同一缓存、ffmpeg 缺失时 hello 不广告且 `repr` 返回 `no-pipeline`）、`-p homestation`、`-p buckyos-api` 通过；`-p scheduler` 只有既有失败 `test_create_pod_instance_no_suitable_node`（与本改动无关）。
- 独立模式二进制跑全尺寸样本（4032×3024 / 48 tile HEIC 竖拍 0.7 s，1280×720 HLG 与竖拍 HEVC 转码约 1 s）。
- Desktop：`tsc -b`、`pnpm build`、eslint（仅 `PublishComposer.tsx` 基线已有 1 条 react-hooks/refs）、`tests/datamodel/preview-media-types.test.ts`（真实夹具上的品牌 / `ispe` / 直放判定）、mock e2e：preview、preview-embed、homestation、homestation.video、messagehub-media 全过。
- HomeStation `tests/object_import.rs` 增补：`/content` 跨 chunk list 的 206/Content-Range/Accept-Ranges 与 416，`/ref` 返回 scratch 绑定 + 令牌且能读 nfs-server 桶数据面（206），无权读者 `/ref` 404。
- HomeStation 真实 devnet e2e（`playwright.homestation.config.ts`）12 例 11 过；`mute choices come from the HomeStation’s people` 失败，与媒体无关，未在基线上复核。
- Desktop 全量 mock e2e 447 过 / 21 失败，逐条核对均不在本次改动路径：8 例 ndm-upload-endpoint 的测试桩不认 10-26 加的 `object.upload_target`；app-launch / content-open / desktop / preview-embed 共 9 例写死 `localhost:4173`（临时换端口所致，preview-embed 用默认端口复跑 4/4 过）；ai-center、settings 集群、messagehub-header 标记已读、messagehub 超大附件（Playwright 50 MB buffer 上限）各 1 例。
- 夹具：`src/frame/nfs_server/tests/fixtures/preview/`（libheif `heif-enc` 与 x265 编码的真实 HEIC/HEVC，非改名）；**不是 iPhone 实拍**。

DV（2026-10-28 晚，`uv run start.py --all` 全新安装的本机 Zone test.buckyos.io，nfs-server 服务模式探测到 `/usr/bin/ffmpeg` 7.1.5）：

- `BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io npx playwright test --config=playwright.dv.config.ts tests/dv/preview-iphone-media.spec.ts` 通过：Chromium 选取空 `File.type` 的 HEIC 与 MOV → HomeStation 上传发布 → 信息流缩略图为管线 JPEG（竖拍竖显）→ 详情 HEIC 转 JPEG 原尺寸、`/bucket/v1` 经网关 206、"下载原文件"与原 HEIC 字节一致 → HEVC 转码、竖拍视频不横倒、拖动进度、HLG 信息面板注明「HDR 已映射为 SDR」、H.264 MOV 直放 → 好友 bob 的阅读流沉浸模式图片/视频都经管线就绪（非 owner 走自己 `following` 绑定的令牌）。全尺寸样本（`PREVIEW_DV_SAMPLES`）与仓库夹具各跑一遍，后者全冷转换。
- 挪走 `/usr/bin/ffmpeg` 后 `PREVIEW_DV_NO_FFMPEG=1` 用例通过：`hello` 不再广告 `repr`、`repr.detail` 列出搜索过的目录，HEIC 详情显示「此浏览器不支持该图片格式」并给出下载原文件；放回后 `hello` 立即恢复 `repr`。为此 `hello` 改为每次都做一次只 stat 候选路径的复核（原先只在 `repr` 前与每小时刷新）。
- 回归：DV `preview-named-object`、`storage-status` 与 share 其余用例通过；`share.spec` 的「HomeStation: the photo snapshot becomes a post attachment」失败——预填附件不经上传，即已记录的 10-23 起 `attachment_not_registered`；`homestation.spec` 第二例全部断言通过、只在测试收尾 `visitor.close()` 报 Target closed，单独复现正常关闭，与本改动无关。
- **DV 发现的既有缺陷（未修）**：websdk `ndm_client.ts` 秒传先算 qcid（文件头/中/尾各 4 KiB）查 `/ndm/v1/objects/lookup`，命中即复用对方内容 id、不做全量校验。两个同大小、只在采样区外不同的文件（例：只改了旋转矩阵的 MOV）会被挂成同一内容——DV 中 IMG_0004.MOV 的 FileObject 指向了 IMG_0003 的 chunk。建议命中后仍算全量 chunk id 比对一致才复用（只省上传、不省本地读）。仓库夹具 `portrait_hevc.mov` 已改为内容不同的片段，避免测试被它掩盖或干扰。

未做 / 未验证：

- iOS Safari / macOS Safari / macOS Chromium（硬解 HEVC）实机、iOS 照片传输设置矩阵、Live Photo；样本非 iPhone 实拍。
- Windows / macOS 上的 ffmpeg 探测与调用（代码按三平台写，路径含空格 / 非 ASCII 有单测，未在两平台实跑）；Windows 无控制台弹窗未实测。
- HEIC 的 ICC（Display P3）未转换到 sRGB，宽色域照片会略偏淡；Exif 方向只认 HEIF `irot`/`imir` 与 ffmpeg 自带的 display matrix。
- `ndmSource.ts` 的 object-id 整读（MessageHub 附件）未改，按 §5 另列。
- 结果没有写回 HomeStation 的宽高（D7「管线完成后回写」未做，前端卡片不依赖宽高）。
- HomeStation 视频帖卡片不显示正文、"我的发表"同一秒内的帖子顺序不稳定（既有展示行为，DV 用例按文件名对应）。
