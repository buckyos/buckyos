# Preview：iPhone 图片与视频兼容 TODO

日期：2026-10-28

状态：待实现。本文记录源码核对结果、开发顺序与验收范围。

目标：共享 `ContentPreview` 能可靠预览 iPhone 常见的 HEIC/HEIF 图片，以及 MOV/MP4 容器中的 H.264、HEVC 视频。浏览器能解码时直接显示；不能解码时生成并缓存兼容的预览副本，保留原文件。

关联：[HomeStation 视频播放问题 #692](https://github.com/buckyos/buckyos/issues/692)。该问题的播放器接入修复验证了 H.264/AAC MP4，不能据此认为 HEIC、HEVC 或 iPhone 实机兼容已通过。

## 1. 当前实现与缺口

以下链接相对本文位置，现状以对应源码为准。

| 范围 | 已确认的现状 | 缺口 |
| --- | --- | --- |
| 图片识别与能力判断 | [mediaTypes.ts](../src/frame/desktop/src/components/preview/mediaTypes.ts) 能识别部分 HEIC 品牌；原生图片支持列表只有基础格式与异步探测的 AVIF | 没有 HEIC/HEIF 解码探测，原生支持的浏览器也会被判为不能直接预览；品牌识别需补全 |
| 视频能力判断 | 同文件通过 `canPlayType` 检测 `video/mp4`、`video/quicktime` 等 MIME | 没有区分容器内部的 H.264、HEVC 及编码参数，支持 MP4 容器不代表能解码其中的 HEVC |
| 转换回退 | [nfspProvider.ts](../src/frame/desktop/src/components/preview/nfspProvider.ts) 已预留 `repr/get_repr` 调用；[nfs_server/types.rs](../src/frame/nfs_server/src/types.rs) 明确未实现、未广告 `repr` | 真实环境返回 `no-pipeline`，没有图片解码或视频转换兜底 |
| Mock 能力 | [mockProvider.ts](../src/frame/desktop/src/components/preview/mockProvider.ts) 有 `raster-decode` 和 `media-transmux` | 这是模拟管线，不能作为正式格式支持或转换成功的证据 |
| HomeStation 内容读取 | [media.ts](../src/frame/desktop/src/app/homestation/media.ts) 使用 `response.blob()` 读完整文件后交给 Preview | 大视频首播等待长；读取未接入取消信号 |
| 上传与 MIME | [PublishComposer.tsx](../src/frame/desktop/src/app/homestation/publish/PublishComposer.tsx) 依赖 `File.type` 分类；[fsutil.rs](../src/frame/nfs_server/src/fsutil.rs) 有 `.heic/.mov` 映射，缺 `.heif` | 空 MIME 或通用 MIME 可能影响媒体分类与后续预览 |

[MessageHub 的 imageNormalize.ts](../src/frame/desktop/src/app/messagehub/conversation/input/imageNormalize.ts) 可在浏览器能解码 HEIC 时转成 JPEG，失败则返回原文件。可参考其识别与解码流程，但它不是通用 HEIC 解码器，也不能代替共享 Preview 的服务端兜底。

## 2. P0：修正原生预览路径

- [ ] 为 HEIC/HEIF 增加真实解码探测或按当前文件试解码，避免仅凭固定支持列表拒绝原生可显示的内容；探测有超时和资源释放。
- [ ] 补齐 HEIF 品牌、扩展名和 MIME 识别，覆盖大写后缀、空 `File.type`、`application/octet-stream`；复用已有内容识别能力。
- [ ] 区分视频容器与实际编码，补充 H.264/HEVC 能力判断；保留真实媒体加载失败后的回退，不能把 `canPlayType` 的结果当作播放成功。
- [ ] 能直接显示的图片与视频优先使用原文件；失败时区分无访问权限、内容不存在、网络失败、编码不支持与内容损坏，并提供适当的重试或下载入口。

## 3. P1：补齐真实转换与缓存

- [ ] 明确转换执行器的位置和部署方式，优先复用已有媒体工具；新增依赖按仓库规则确认。AIOS 容器安装了 FFmpeg 不代表 `nfs_server` 已能执行转换。
- [ ] 落实 `repr/get_repr` 的共享协议、服务能力发现、任务状态与错误映射；同步检查服务端、SDK 与 Preview provider，能力可用后再广告。
- [ ] HEIC/HEIF 生成 JPEG、PNG 或 WebP 预览，正确处理方向、尺寸、透明度与色彩；明确 HDR 转 SDR 的策略和信息损失。
- [ ] MOV 中的编码已被浏览器支持时，按需优先重封装；HEVC 无法原生解码时，生成 H.264/AAC MP4 等目标环境可播放的预览副本。
- [ ] 派生预览按源对象/版本、转换器版本及输出参数缓存，合并重复任务；支持失败后重试，并限制任务耗时、并发和资源占用。
- [ ] 原文件字节、对象 ID 与“下载原文件”保持原样；派生内容沿用原文件访问权限，不能因缓存命中绕过鉴权。

## 4. P2：大文件与业务入口体验

- [ ] 让 HomeStation 视频复用支持鉴权与 Range 的内容 URL，避免必须下载完整 Blob 后才能首播；同时验证服务与网关的实际 Range 行为。
- [ ] 将取消信号贯穿内容读取；切条或关闭后停止旧播放、释放资源，丢弃过期任务结果。
- [ ] 检查 HomeStation 的上传预览、信息流图片与封面、详情、沉浸模式，以及文件浏览器和消息附件入口，避免共享 Preview 修好后业务页面仍直接用不兼容原文件。

## 5. 验收

使用真实文件，记录浏览器、系统版本及实际走的是原生还是转换路径；至少覆盖 iOS Safari 和桌面 Chromium。HEVC 需覆盖原生支持与不支持两种环境。

| 样本 | 必须验证 |
| --- | --- |
| 真实 iPhone HEIC/HEIF，含竖拍照片 | 原生支持时直显；不支持时转换；方向、比例与颜色正确 |
| H.264 + AAC 的 MOV | 画面与音轨正常；必要时重封装；播放、暂停、拖动进度可用 |
| HEVC + AAC 的 MOV / MP4 | 原生支持时直放；不支持时真实转换；时长、音画同步正确 |
| 竖拍、HDR 视频 | 旋转元数据正确，预览不横倒；明确并验证 HDR 保留或 SDR 映射效果 |
| JPEG、PNG、H.264/AAC MP4 | 既有图片与视频预览无回归 |

- [ ] 在真实服务环境完成“iPhone 文件选择/上传 → Publish → 详情播放或显示 → 沉浸模式”全链路；补齐测试环境的上传代理，不能只用 mock feed 或拦截响应验收。
- [ ] 验证权限不足、文件不存在、网络中断、解码失败、转换器不可用，以及任务失败后重试的可见提示。
- [ ] 验证重复打开复用转换结果、切条后旧音视频停止、下载原文件字节未变。
- [ ] 自动化使用真实 HEIC/HEVC 样本；PNG 改名为 `.HEIC` 或模拟媒体错误只能验证分支，不能证明格式兼容。

首轮不承诺 Live Photo 的图片/视频联动、ProRAW、ProRes 或无损 HDR 编辑；这些能力另行定义范围。

参考：[WebKit：Safari 17 的 HEIC 支持](https://webkit.org/blog/14445/webkit-features-in-safari-17-0/#heic)、[MDN：HEVC 编码与浏览器支持](https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/Video_codecs#hevc_h.265)。
