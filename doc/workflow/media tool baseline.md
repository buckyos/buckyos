# Workflow 媒体工具基线（T2c）

日期：2026-11-05。来源：[notepads/llm-media-pipeline-workflow-todo.md](../../notepads/llm-media-pipeline-workflow-todo.md) §2.5 / T2c / T4b（用户裁决 P8：以 workflow 需求为先导定义 agent-runtime / paios 的工具基线）。

本文是 workflow 系统内置媒体管线（`media.image` / `media.pdf` / `media.video` / `media.document`）所需外部工具的**唯一需求清单**。清单从 `FunctionRegistration.requirements` 推导（`src/kernel/workflow/src/functions/media/mod.rs::media_tool_baseline()`），构建脚本、镜像文档与探测代码都对齐这里；改任一处先改本表。[paios 容器需求 §6.4](../app_service/paios容器需求.md) 仍是“官方发布的 Agent 可用命令”的维护入口，本文只约束其中的媒体部分。

## 0. 两个必须分别满足的环境

| 环境 | 谁用 | 为什么 |
| --- | --- | --- |
| **workflow 宿主**（运行 `workflow` 内核服务的 OOD / 节点，Linux / macOS / Windows） | `LocalThunkRunner` 在**宿主进程内**执行全部 `media.*` Native 函数（TODO D1） | 媒体转换 P0 不在 Agent 容器里跑；镜像里装了工具不等于宿主 runner 可用 |
| **paios 工作镜像**（Agent 容器：`soffice` 与字体在镜像的 `RUNTIME_PACKAGES`，其余命令来自节点的扩展包 `ext-core`） | Agent 自己的 shell 步骤、`import_office` 等手册命令；未来 Scheduler / Node Runner 把 Native 执行调度到容器时也用它 | 保持 Agent 侧命令可用，并为后续分布式执行预置相同能力 |

缺基线项是**部署缺陷**，不是正常降级：对应转换步骤以 `tools_missing` 失败并列出缺项，Run `Failed`，不启用本地备用转换（裁决 R2 / R3）。镜像齐全、宿主缺工具的故障注入必须准确定位到宿主缺项（验收 21）。

## 1. 清单

| 工具 / 命令 | 最低版本 | 用到它的函数 | 用途 | 探测方式（`functions/media/tools.rs`） |
| --- | --- | --- | --- | --- |
| `pdfinfo` | poppler ≥ 22.0 | `media.probe`（pdf / document）、`media.extract_pdf_text`、`media.render_pdf_pages`、`media.to_pdf`（数派生 PDF 页数） | 页数、页尺寸 | `pdfinfo -v`，取 `version` 后的第一个 token |
| `pdftotext` | poppler ≥ 22.0 | `media.extract_pdf_text` | 分页文本（`-layout`，版面靠空格补齐，Agent 侧读入时规范空白；按页逐次调用） | `pdftotext -v` |
| `pdftoppm` | poppler ≥ 22.0 | `media.render_pdf_pages` | 页图（PNG，`-r <dpi>`，长边按 `max_long_edge` 缩放） | `pdftoppm -v` |
| `ffprobe` | ffmpeg ≥ 6.0 | `media.probe`（video）、`media.extract_frames`、`media.extract_audio` | 时长、流信息（有无音轨） | `ffprobe -hide_banner -version`，取 `ffprobe version <x>` |
| `ffmpeg` | ≥ 6.0，带 `libmp3lame`（音轨输出）、PNG 编码 | `media.extract_frames`、`media.extract_audio`；`media.fit_image` 的可选依赖（`image` crate 解不了的格式先转 PNG） | 均匀抽帧、提取音轨、非常见图片格式解码 | `ffmpeg -hide_banner -version` |
| `soffice`（LibreOffice headless：core、writer、impress、calc） | ≥ 7.0 | `media.to_pdf` | DOCX / PPTX / ODT / ODP / DOC / PPT / RTF / TXT → PDF；每次调用独立 `-env:UserInstallation` profile，Office lane 并发 1 | `soffice --version`（Windows 上 `soffice.com`），取第一个数字 token |
| 字体：DejaVu、Liberation、Noto（含 CJK） | — | `media.to_pdf`（经 soffice） | 中西文文档的字形与度量；缺字体会改变页图内容 | `fc-list` 的摘要进 `to_pdf` 指纹（`env/fonts`）；没有 `fc-list` 时记为 `unknown` |
| locale：`C.UTF-8` 可用，建议生成 `en_US.UTF-8` | — | `media.to_pdf` | soffice 的文件名、编码处理 | 不单独探测 |
| `tesseract`，带常用语言数据（`ext-core` 保留的 28 种加 `osd`，见 [paios容器需求.md](../app_service/paios容器需求.md) §6.4） | ≥ 5.0 | `media.ocr_pages`（软依赖：缺它时各页保留文字层，步骤输出 `status: tools_missing`，结果不进缓存） | 文字层缺失或乱码的页重新识别；按窗口抽样选语言（试各文字系统的代表模型比置信度，都不像时问 `osd`）；竖排中日韩用 `_vert` 模型 | `tesseract --version` 取第二个 token；`tesseract --list-langs` 的语言表进 `ocr_pages` 指纹（`env/ocr_langs`），增减语言包使旧结果失效 |

不在基线内：ImageMagick（服务于其它场景）；`media.capabilities` / `media.build_catalog` / `media.fit_image`（常规格式）只需要 workflow 自身与 `image` crate（编译依赖，随 workflow 发布）。

查找顺序（三平台相同，与 nfs_server preview 的 `tools.rs` 一致）：`$BUCKYOS_ROOT/libexec/buckyos-tool/bin/` → `PATH` → 固定位置（macOS Homebrew prefix、`/Applications/LibreOffice.app/Contents/MacOS`；Linux `/usr/lib/libreoffice/program`、`/opt/libreoffice*/program`；Windows `%ProgramFiles%\LibreOffice\program`）→ `$BUCKYOS_ROOT/tools/bin/`（扩展包，Windows 上是 `.cmd`）。扩展包排最后：机器上已有的工具不被替换。`ext-core` 的 ffmpeg（conda-forge）没有 libzimg，也就没有 `zscale`，只靠它的机器上 nfs_server 预览的 HDR 视频和 HEIF 不做色调映射，结果带“颜色可能失真”的说明。

## 2. 版本如何进入执行指纹

- 服务启动时 `probe_default()` 对上表每个命令定位并取版本，得到 `ToolEnvironment { tools, missing, searched, revision }`；`revision` 是全部工具 (path, version) 的 hash，`get_capabilities` RPC 与 `media.capabilities` 步骤都返回它。
- 每个函数只把**自己依赖**的工具放进指纹：`fingerprint = { "cmd/<tool>": <version> | "missing" }`；`step_cache_key = hash(thunk_obj_id, fingerprint)`。因此升级 poppler 使 PDF 步骤不命中旧缓存，但不影响 `fit_image`（验收 6）。
- `create_run(ensure=true)` 的合并键包含 `environment_revision`；执行前 runner 再核对指纹，不一致返回 `environment_changed`（retryable），不以旧 key 缓存新环境结果。
- `to_pdf` 的 `is_pure: true` 表示结果可复用，不表示字节相同：LibreOffice 每次写入不同的 `CreationDate`、`/ID`、`DocChecksum`。第一次的结果写入缓存，之后 `pdf_content_id` 稳定，后续页面步骤全部命中（WF-DV-015）。它的指纹是 soffice、pdfinfo 版本加字体集摘要，字体变化使旧结果失效。
- locale 目前不进指纹，由宿主预置固定。

## 3. 宿主预置与检查

- **Linux 宿主（OOD）**：与镜像同一份 apt 清单：`poppler-utils ffmpeg libreoffice-core-nogui libreoffice-writer-nogui libreoffice-impress-nogui libreoffice-calc-nogui fonts-dejavu-core fonts-liberation fonts-noto-core fonts-noto-cjk locales`。安装脚本 / 安装文档应把这组包列为 workflow 服务的运行依赖（本轮未改安装脚本，见 §6）。
- **macOS**：Homebrew `poppler ffmpeg` + LibreOffice.app；字体依赖系统自带 CJK 字体。
- **Windows**：poppler（如 `poppler-windows` 发行）、ffmpeg、LibreOffice 安装到 `%ProgramFiles%`，或把二进制放进 `$BUCKYOS_ROOT/libexec/buckyos-tool/bin/`。
- **检查**：
  - `check.py` 的 workflow（4070）一行只说明服务在线；能力诊断调 `workflow.get_capabilities`（返回 `environment.tools` / `missing` / `searched`），或看 workflow 启动日志 `workflow media tools revision=… present=[…] missing=[…]`。
  - 真实验证不是 `command -v`：对每个工具跑冒烟样例（§4），确认版本与真实转换都成功。

## 4. 冒烟样例（宿主与镜像都要跑）

| 能力 | 样例 | 期望 |
| --- | --- | --- |
| PDF 文本 / 页图 | 两页以上、尾页含唯一答案句的文本 PDF；一页扫描（纯图）PDF | `media.pdf` Run Completed；manifest 尾页 `ok` 且文本含答案；扫描页 `no_text` + 页图；`summary.complete = true` |
| 视频抽帧 / 音轨 | 带音轨的 3 秒合成视频；无音轨视频 | `media.video` Completed；`frames` ≥ 1 且时间戳单调；有音轨 → `audio.status = ok`，无音轨 → `absent` 不失败 |
| DOCX / PPTX → PDF | 含中英文与一张图的 DOCX、三页 PPTX | `media.document` Completed；`to_pdf.pages` 正确；页图可见文字（非方块）；页码属于派生 PDF |
| 缺工具故障注入 | 临时从搜索路径移除 `pdftoppm` 后建 `media.pdf` Run | `render_pdf_pages` Failed，`error.code = tools_missing`，`details.tools_missing = ["pdftoppm"]`，Run / Step / Thunk Task 与 RPC 一致；恢复工具后新 Run 的 ensure key 因 `environment_revision` 不同而不合并 |

对应的自动化：`cargo test -p workflow -- --ignored`（`functions/media/tests.rs` 的 poppler / ffmpeg / soffice 用例，依赖本机工具），`uv run test/run.py -p workflow_test` 的 WF-DV-009~ 用例（真实 Zone）。

## 5. 与 paios 镜像的联动（T4b）

- `build_aios` 的 `RUNTIME_PACKAGES` 已加入 LibreOffice nogui 组件、字体与 `locales`（生成 `en_US.UTF-8`，`LANG=C.UTF-8`）；poppler-utils、ffmpeg 原已包含。镜像未在本轮重新构建，amd64 / aarch64 的真实探测与样例运行**未验证**。
- [paios 容器需求 §6.4 表 B](../app_service/paios容器需求.md) 同步了这四项并指回本文；§6.3 的“不含 libreoffice”已改。
- runtime 的 `tools_check` / baseline 声明（`src/frame/lib_opendan/src/baseline.rs`，Agent 标准命令检查）**未**改为按本表探测媒体工具；标准命令测试通过不等于媒体基线满足。ExtTools / ext-core 的 `provides` 落地时使用同一清单。

## 6. 未完成项

- 宿主安装脚本 / 安装文档未加入 LibreOffice 与字体包；本轮只改了镜像清单与文档。
- macOS / Windows 宿主的字体与 locale 要求未实测。
