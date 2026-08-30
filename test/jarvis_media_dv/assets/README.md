# Jarvis Media DV 测试素材

这些文件专用于真实环境测试。图片和音频是项目生成的夹具；MP4 来自 MDN 的 CC0 测试媒体：

| 文件 | 内容与用途 |
|---|---|
| `image_primary.png` | 粉色花朵、岩石和绿叶，用于图片理解、编辑和图生视频 |
| `image_secondary.png` | 蓝天、山地和盘山公路，与主图明显不同，用于附件归属测试 |
| `image_ocr.png` | 仅包含 `BUCKYOS-DV-4827`，用于文字识别 |
| `audio_sfx.wav` | 合成蜂鸣、敲击和环境底噪，没有人声，用于非语音置信度测试 |
| `audio_speech.wav` | 中文系统语音朗读“今天的测试编号是四八二七”，用于 ASR/TTS 串联测试 |
| `video_fresh.mp4` | 花朵随风运动的 H.264 MP4，作为默认视频理解、转换和上传视频续写素材 |

PNG 和无人声音效可运行 `python generate_assets.py` 确定性重建。`audio_speech.wav` 使用 Windows SAPI 的 Microsoft Huihui 中文语音生成，因为可理解的 ASR 输入不适合用简单波形合成。

`video_fresh.mp4` 来源：[`MDN interactive examples / cc0-videos / flower.mp4`](https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4)，许可为 CC0 / Public Domain。它不由生成脚本重建，SHA-256 为 `0cd83d944a6ca7822b4a8306cecc60a36e859b041f6702c6a1ad9ead78924451`。

提交这些素材前应保持文件名和语义稳定；测试场景依赖素材之间的显著差异以及 OCR/语音中的固定编号。
