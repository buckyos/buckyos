# AICC Fixture 与测试资源组件

定义固定资源、ResourceRef 输入、artifact 输出、fixture manifest、digest/media type 校验和资源生成约束。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 测试数据与资源 Fixture

`test/aicc_test/fixtures` 和 Rust test fixture 中应维护固定数据：

| 类型 | Fixture | 用途 |
|---|---|---|
| 图片 | 小 PNG、大 PNG、透明 PNG、JPEG、mask PNG | `image.*`、`vision.*`、多模态 `llm.chat` |
| 音频 | 短 wav/mp3、长音频、噪声音频 | `audio.tts`、`audio.asr`、`audio.enhance` |
| 视频 | 短 mp4 或 mock video object | `video.*` |
| 文档 | text chunk、PDF/image page mock、OCR 样例 | `embedding.text`、`vision.ocr`、`rerank` |
| 结构化数据 | JSON schema、tool schema、rerank docs | `llm.chat`、`rerank` |
| 大批量数据 | 101 条 embedding items 或超过 1MB 预估响应 | artifact 输出策略 |

每个 fixture 应有固定 digest、media type、size、必要 metadata，便于验证 FileObject meta 和 artifact 输出。

## 2. Fixture Manifest 约定

固定资源应有 manifest，runner 开始时校验存在性、大小、digest 和 media type。

推荐文件：

```text
test/aicc_test/fixtures/manifest.toml
```

示例：

```toml
[[fixtures]]
id = "image_png_small"
path = "images/small.png"
media_type = "image/png"
size_bytes = 1024
sha256 = "..."
used_by = ["image.img2img", "vision.caption", "llm.chat"]

[[fixtures]]
id = "mask_png_alpha"
path = "images/mask_alpha.png"
media_type = "image/png"
sha256 = "..."
attributes = { width = 512, height = 512, has_alpha = true }
used_by = ["image.inpaint", "vision.segment"]

[[fixtures]]
id = "audio_wav_short"
path = "audio/short.wav"
media_type = "audio/wav"
sha256 = "..."
attributes = { duration_seconds = 2.0, sample_rate = 16000 }
used_by = ["audio.asr", "audio.enhance"]
```

Fixture 要求：

- 小文件可以直接进入仓库。
- 大文件应尽量使用可生成的 deterministic fixture，或在 runner 中按脚本生成。
- L4 真实模型使用外部 URL 时，必须在 TOML 中显式配置，不应默认访问不受控 URL。
- fixture 内容不应包含真实用户数据。

