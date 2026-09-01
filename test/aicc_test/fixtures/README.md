# AICC acceptance fixtures

这里保存 `aicc_e2e_test_requirements.md` 使用的确定性输入，覆盖：

- 文本、Markdown、HTML、CSV/TSV、JSON/YAML/XML、RTF、源代码、PDF、DOCX、XLSX、PPTX、EPUB。
- JPEG、透明 PNG、mask、不同采样率/声道音频和字幕。
- 单文档、多文档、中文路径、同名文件、空目录、嵌套 ZIP。
- 空文件、损坏 ZIP、加密标志、路径穿越、深层目录、文件数和解压量边界、MIME 不匹配、文档 prompt injection。

运行以下命令可确定性重建，并同步写出带大小与 SHA-256 的 `acceptance/fixture_manifest.json`：

```bash
python test/aicc_test/acceptance/generate_fixtures.py
```

`video_fresh.mp4` 等已有大媒体继续复用 `test/jarvis_media_dv/assets`，不复制两份。
