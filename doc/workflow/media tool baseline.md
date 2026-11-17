# Workflow 媒体工具基线（T2c）

日期：2026-11-05。来源：[notepads/llm-media-pipeline-workflow-todo.md](../../notepads/llm-media-pipeline-workflow-todo.md) §2.5 / T2c / T4b（用户裁决 P8：以 workflow 需求为先导定义 agent-runtime / paios 的工具基线）。

本文是 workflow 系统内置媒体管线（`media.image` / `media.pdf` / `media.video` / `media.document` / `media.page`）所需外部工具的**唯一需求清单**。清单从 `FunctionRegistration.requirements` 推导（`src/kernel/workflow/src/functions/media/mod.rs::media_tool_baseline()`），构建脚本、镜像文档与探测代码都对齐这里；改任一处先改本表。[paios 容器需求 §6.4](../app_service/paios容器需求.md) 仍是“官方发布的 Agent 可用命令”的维护入口，本文只约束其中的媒体部分。

## 0. 两个必须分别满足的环境

| 环境 | 谁用 | 为什么 |
| --- | --- | --- |
| **workflow 宿主**（运行 `workflow` 内核服务的 OOD / 节点，Linux / macOS / Windows） | `LocalThunkRunner` 在**宿主进程内**执行全部 `media.*` Native 函数（TODO D1） | 媒体转换 P0 不在 Agent 容器里跑；镜像里装了工具不等于宿主 runner 可用 |
| **paios 工作镜像**（Agent 容器：命令全部来自节点的扩展包 `ext-core`，`soffice` 和字体也是） | Agent 自己的 shell 步骤、`import_office` 等手册命令；未来 Scheduler / Node Runner 把 Native 执行调度到容器时也用它 | 保持 Agent 侧命令可用，并为后续分布式执行预置相同能力 |

缺基线项是**部署缺陷**，不是正常降级：对应转换步骤以 `tools_missing` 失败并列出缺项，Run `Failed`，不启用本地备用转换（裁决 R2 / R3）。镜像齐全、宿主缺工具的故障注入必须准确定位到宿主缺项（验收 21）。

## 1. 清单

| 工具 / 命令 | 最低版本 | 用到它的函数 | 用途 | 探测方式（`functions/media/tools.rs`） |
| --- | --- | --- | --- | --- |
| `pdfinfo` | poppler ≥ 22.0 | `media.probe`（pdf / document）、`media.extract_pdf_text`、`media.render_pdf_pages`、`media.to_pdf`（数派生 PDF 页数） | 页数、页尺寸 | `pdfinfo -v`，取 `version` 后的第一个 token |
| `pdftotext` | poppler ≥ 22.0 | `media.extract_pdf_text` | 分页文本（`-layout`，版面靠空格补齐，Agent 侧读入时规范空白；按页逐次调用） | `pdftotext -v` |
| `pdftoppm` | poppler ≥ 22.0 | `media.render_pdf_pages` | 页图（PNG，`-r <dpi>`，长边按 `max_long_edge` 缩放） | `pdftoppm -v` |
| `ffprobe` | ffmpeg ≥ 6.0 | `media.probe`（video）、`media.extract_frames`、`media.extract_audio` | 时长、流信息（有无音轨） | `ffprobe -hide_banner -version`，取 `ffprobe version <x>` |
| `ffmpeg` | ≥ 6.0，带 `libmp3lame`（音轨输出）、PNG 编码 | `media.extract_frames`、`media.extract_audio`；`media.fit_image` 的可选依赖（`image` crate 解不了的格式先转 PNG） | 均匀抽帧、提取音轨、非常见图片格式解码 | `ffmpeg -hide_banner -version` |
| `soffice`（LibreOffice headless：core、writer、impress、calc、draw；`ext-core` 带的是 26.8） | ≥ 7.0 | `media.to_pdf` | DOCX / PPTX / ODT / ODP / DOC / PPT / RTF / TXT → PDF；每次调用独立 `-env:UserInstallation` profile，Office lane 并发 1 | `soffice --version`（Windows 上 `soffice.com`），取第一个数字 token |
| `page-render`（`ext-core` 的命令：Node 脚本，用 `playwright-core` 起一个一次性的无头 Chromium；浏览器本身不在包里，是包的在线项，由节点下载，见 §3.2；参数和输出见 [Agent 浏览器 TODO §5.7](../../notepads/agent-runtime-browser-todo.md)） | — | `media.page_to_pdf`、`media.page_snapshot` | HTML 页面（一份 HTML，或带 `index.html` 的 zip）输出成 PDF 或 PNG；Browser lane 并发 1，隔离运行（§3.1） | `page-render --version` 退出码为 0 时取第一行（`page-render <版本> chromium <版本> playwright-core <版本>`），整行就是它的版本；这一步不启动浏览器，浏览器没下载时照样取得到 |
| 字体：DejaVu、Liberation、Noto CJK（`ext-core` 的 soffice 自带：LibreOffice 的字体加 Noto Sans / Serif CJK 常规体和粗体；1.3.1 起包里所有用 fontconfig 的工具都读得到这些字体和简体中文优先的回退规则，另外也读机器上的系统字体目录和用户字体目录（`/usr/share/fonts`、`~/.local/share/fonts`）；1.3.3 起在 Linux 上还读字体包 `ext-fonts` 已经下载的字体。见 [paios容器需求.md](../app_service/paios容器需求.md) 表 B 和“字体包”） | — | `media.to_pdf`（经 soffice）、`media.page_to_pdf`、`media.page_snapshot`（经 Chromium；一个字体都没有时渲染文字会崩溃） | 中西文文档的字形与度量；缺字体会改变页图内容 | 进 `to_pdf`、`page_to_pdf`、`page_snapshot` 的指纹：`fc-list` 的摘要（`env/fonts`，探测时算，没有 `fc-list` 时记为 `unknown`），加上每次取指纹时现读的 `ext-core` 版本（`env/ext-core`）和已经就绪的字体包在线项（`env/ext-fonts`），见 §2 |
| locale：`C.UTF-8` 可用（glibc 自带；`ext-core` 的 soffice launcher 在环境没有 UTF-8 locale 时自己设 `LC_ALL=C.UTF-8`） | — | `media.to_pdf` | soffice 的文件名、编码处理 | 不单独探测 |
| `tesseract`，带常用语言数据（`ext-core` 保留的 28 种加 `osd`，见 [paios容器需求.md](../app_service/paios容器需求.md) §6.4） | ≥ 5.0 | `media.ocr_pages`（软依赖：缺它时各页保留文字层，步骤输出 `status: tools_missing`，结果不进缓存） | 文字层缺失或乱码的页重新识别；按窗口抽样选语言（试各文字系统的代表模型比置信度，都不像时问 `osd`）；竖排中日韩用 `_vert` 模型 | `tesseract --version` 取第二个 token；`tesseract --list-langs` 的语言表进 `ocr_pages` 指纹（`env/ocr_langs`），增减语言包使旧结果失效 |

不在基线内：ImageMagick（服务于其它场景）；`media.capabilities` / `media.build_catalog` / `media.fit_image`（常规格式）只需要 workflow 自身与 `image` crate（编译依赖，随 workflow 发布）。

查找顺序（三平台相同，与 nfs_server preview 的 `tools.rs` 一致）：`$BUCKYOS_ROOT/libexec/buckyos-tool/bin/` → `PATH` → 固定位置（macOS Homebrew prefix、`/Applications/LibreOffice.app/Contents/MacOS`；Linux `/usr/lib/libreoffice/program`、`/opt/libreoffice*/program`；Windows `%ProgramFiles%\LibreOffice\program`）→ `$BUCKYOS_ROOT/tools/bin/`（扩展包，Windows 上是 `.cmd`）。扩展包排最后：机器上已有的工具不被替换。`ext-core` 的 ffmpeg（conda-forge）没有 libzimg，也就没有 `zscale`，只靠它的机器上 nfs_server 预览的 HDR 视频和 HEIF 不做色调映射，结果带“颜色可能失真”的说明。

## 2. 版本如何进入执行指纹

- 服务启动时 `probe_default()` 对上表每个命令定位并取版本，之后每隔 30 秒在后台重新探测并替换环境快照；安装、升级或删除工具后不需要重启 workflow，下一次探测完成后生效。快照为 `ToolEnvironment { tools, missing, searched, ext_tools, revision }`（`ext_tools` 是扩展包的工具根 `$BUCKYOS_ROOT/tools`，字体指纹从这里现读）；`revision` 是工具 (path, version)、字体集和 OCR 语言的 hash，`get_capabilities` RPC 与 `media.capabilities` 步骤都返回它。已经失败的 run 不会自动重试，确认工具就绪后重新发起。
- 每个函数只把**自己依赖**的工具放进指纹：`fingerprint = { "cmd/<tool>": <version> | "missing" }`；`step_cache_key = hash(thunk_obj_id, fingerprint)`。因此升级 poppler 使 PDF 步骤不命中旧缓存，但不影响 `fit_image`（验收 6）。
- `create_run(ensure=true)` 的合并键包含 `environment_revision`；执行前 runner 再核对指纹，不一致返回 `environment_changed`（retryable），不以旧 key 缓存新环境结果。
- `to_pdf` 的 `is_pure: true` 表示结果可复用，不表示字节相同：LibreOffice 每次写入不同的 `CreationDate`、`/ID`、`DocChecksum`。第一次的结果写入缓存，之后 `pdf_content_id` 稳定，后续页面步骤全部命中（WF-DV-015）。它的指纹是 soffice、pdfinfo 版本加下面的字体三项，字体变化使旧结果失效。
- `page_to_pdf`、`page_snapshot` 同样是“可复用”意义上的 pure（Chromium 把生成时间写进 PDF）。指纹是 `cmd/page-render`（`--version` 那一行，含脚本、Chromium、playwright-core 三个版本）加下面的字体三项：换浏览器、换脚本或字体变化都使旧结果失效。
- 字体三项（`tools::insert_font_fingerprint`）：`env/fonts` 是探测时宿主 `fc-list` 的摘要；`env/ext-core` 是 `$BUCKYOS_ROOT/tools/current.json` 里 `ext-core` 的 `<版本> <摘要>`（它带的字体和回退规则随包变），没有时是 `none`；`env/ext-fonts` 是 `tools/online/*.json` 里 `package` 为 `ext-fonts`、`state` 为 `ready` 且目录在的在线项，`<在线项>@<版本>` 排序后用逗号连接，没有时是 `none`。后两项在每次取指纹时现读，不等 30 秒的探测：管理员下载一个字体以后，下一次打印就换了缓存键，不用重启 workflow。调度时和执行前各取一次，中间刚好下载完时执行前的核对返回 `environment_changed`（retryable）。
- locale 目前不进指纹，由宿主预置固定。

## 3. 宿主预置与检查

- **三个平台相同**：宿主和容器用同一个扩展包 `ext-core`（`$BUCKYOS_ROOT/tools/`，节点默认播种）。poppler、ffmpeg、tesseract、`soffice` 和 soffice 用的字体都在里面，宿主不需要另装 LibreOffice 和字体包。机器上已经有的同名工具排在前面（§1 的查找顺序），所以宿主自己装了 LibreOffice 时用的是宿主那份，字体集也是宿主的。
- **Linux**：`ext-core` 的 soffice 要求 glibc ≥ 2.34（[paios 容器需求 §6.4 表 B](../app_service/paios容器需求.md)）。
- **检查**：
  - `check.py` 的 workflow（4070）一行只说明服务在线；能力诊断调 `workflow.get_capabilities`（返回 `environment.tools` / `missing` / `searched`），或看 workflow 启动日志 `workflow media tools revision=… present=[…] missing=[…]`。
  - 真实验证不是 `command -v`：对每个工具跑冒烟样例（§4），确认版本与真实转换都成功。

### 3.1 `page-render` 的隔离运行

页面里可能有扩展 Block 的 HTML，是第三方代码，而 workflow 在 Linux 上是 root。`page-render` 不走普通的 `run_tool`，走 `NativeContext::run_tool_isolated`（`src/kernel/workflow/src/native.rs`），只做四件事：

| 做法 | 宿主要满足的 |
| --- | --- |
| 清空环境，只留白名单：Unix 上是 `PATH`、`LANG`、`LC_ALL`、`LC_CTYPE`、`TZ`，另设 `HOME` 指到本次执行临时目录里的 `page/home`，`TMPDIR` 指到一个短名的私有目录 `/tmp/wf-<12 位随机十六进制>`（0700，调用结束时删除；没有语言环境时设 `LANG=C.UTF-8`）。`WORKFLOW_SESSION_TOKEN` 不在其中 | `page-render` 的启动器靠自身位置找到 `ext-core`，不依赖别的环境变量；`/tmp` 可写 |
| 单独的进程组；调用结束时（正常返回、超时、取消、调用被丢弃）整组 `SIGKILL` | — |
| Linux 上有效用户是 root 时降到 `nobody`（`setgroups` / `setgid` / `setuid`，降完核对）；之前把 `page/` 整个目录和上面那个 `TMPDIR` 目录交给这个账号 | 有 `nobody` 账号，没有则该步骤失败；`$BUCKYOS_ROOT/tools` 和 `data/cache/workflow/exec/<execution_id>` 一路对其他用户可进入（默认 umask 022 时成立，`tools` 目录现在是 0755） |
| 断网：root 时子进程在降权前 `unshare(CLONE_NEWNET)`；不是 root 时套 `unshare --net`，不行再套 `unshare --user --map-root-user --net`（同 nfs_server 预览） | 两种都不行时（容器里没有 `CAP_SYS_ADMIN`、没有 `unshare`）记一条警告继续，只剩 `page-render` 自己在浏览器里拦请求 |

**`TMPDIR` 必须短。** Chromium 在 `TMPDIR` 下建套接字（`<TMPDIR>/org.chromium.Chromium.XXXXXX/SingletonSocket`），套接字路径上限约 108 字节，`TMPDIR` 超过约 62 字节浏览器就起不来：`page-render` 退出码 5，stderr 里是 `Socket path too long`。服务的执行临时目录是 `$BUCKYOS_ROOT/data/cache/workflow/exec/<run_id>_<node_id>_<attempt>`，默认安装下光它自己就有 86 个字符左右，所以 `TMPDIR` 不放在里面。Unix 上固定用 `/tmp`，不跟随服务自己的 `TMPDIR`（它可能同样很长）；Windows 上没有这个限制，`TEMP`、`TMP` 仍指到 `page\tmp`。服务被强杀时 `/tmp/wf-*` 目录会留下，靠系统对 `/tmp` 的清理回收。

Chromium 自身的沙箱由 `page-render` 处理：先带沙箱起，宿主不支持时去掉重试，报告行里 `sandbox: false`，workflow 记一条警告，并写进 Thunk 的 `metrics.native`。macOS 上不降权、不建网络命名空间，清环境和进程组照做。Windows 上也不降权：白名单是 `PATH`、`PATHEXT`、`ComSpec`、`SystemRoot`、`SystemDrive`、`windir`、`ProgramData`、`ProgramFiles`、`ProgramFiles(x86)`，另设 `USERPROFILE`、`APPDATA`、`LOCALAPPDATA` 指到 `page\home` 之下，`TEMP`、`TMP` 指到 `page\tmp`；没有进程组，超时、取消或调用被丢弃时用 `taskkill /F /T` 结束进程树。

实测（2026-11-15，本机，Chrome for Testing 159.0.8154.6、playwright-core 1.65.1）：一次性容器里以 root 运行，默认配置下降到 `nobody`、没有网络命名空间、`sandbox: false`；加 `CAP_SYS_ADMIN` 并放开 seccomp / AppArmor（相当于宿主上的 root）时降到 `nobody`、有网络命名空间、`sandbox: true`。不是 root 的开发机上走 `unshare --user --map-root-user --net`，进程在命名空间里是 root，Chromium 因此不开沙箱（`sandbox: false`）。三种情况用的执行临时目录都和服务上一样深（超过 100 个字符）；`TMPDIR` 还放在里面时，同一条用例以 `Socket path too long` 失败。

### 3.2 `page-render` 的浏览器是在线项

Chrome for Testing 不随 `ext-core` 分发。包装好以后，节点从官方地址把它下载到 `$BUCKYOS_ROOT/tools/online/chromium/`，状态记在 `$BUCKYOS_ROOT/tools/online/chromium.json`；下载失败不自动重试，管理员在“设置 > 扩展工具”里或用 `buckyos ext-tools download ext-core chromium` 再次触发。`page-render` 的启动器和脚本仍然随包，所以有一段时间命令在、浏览器不在：

| 环节 | 浏览器没下载时 |
| --- | --- |
| 探测（服务启动时的 `page-render --version`） | 照常。三个版本取自随包的 `browser/versions.json`，不启动浏览器：`page-render` 不进 `environment.missing`，`cmd/page-render` 指纹和下载之后的一样 |
| 执行 `media.page_to_pdf`、`media.page_snapshot` | `page-render` 以退出码 6 结束（5 是浏览器在、没起来）。步骤以 `tools_missing` 失败，不重试，不进缓存：`message` 是 `this node has not downloaded the browser yet (the online item chromium of ext-core): download it in Settings › Extension tools`（`chromium` 带反引号），`details` 是 `{tools_missing: ["chromium"], online_item: "chromium", tool: "page-render", stderr_tail}`。和搜索路径上缺命令的 `tools_missing` 的区别：有 `online_item`，没有 `searched` |
| 下载好之后 | 不用重启 workflow：启动器的路径和指纹都没变，下一次执行直接成功 |

隔离运行降到 `nobody` 时（§3.1），`nobody` 要读得到 `tools/online/chromium/`，和读工具目录的要求相同。

## 4. 冒烟样例（宿主与镜像都要跑）

| 能力 | 样例 | 期望 |
| --- | --- | --- |
| PDF 文本 / 页图 | 两页以上、尾页含唯一答案句的文本 PDF；一页扫描（纯图）PDF | `media.pdf` Run Completed；manifest 尾页 `ok` 且文本含答案；扫描页 `no_text` + 页图；`summary.complete = true` |
| 视频抽帧 / 音轨 | 带音轨的 3 秒合成视频；无音轨视频 | `media.video` Completed；`frames` ≥ 1 且时间戳单调；有音轨 → `audio.status = ok`，无音轨 → `absent` 不失败 |
| DOCX / PPTX → PDF | 含中英文与一张图的 DOCX、三页 PPTX | `media.document` Completed；`to_pdf.pages` 正确；页图可见文字（非方块）；页码属于派生 PDF |
| HTML → PDF / PNG | 带中文、分成多页的 HTML；带样式表和图片的 zip；页面里引用一个外部地址 | `media.page` Completed；`page_to_pdf.pages` 正确，纸张是请求的尺寸，`pdftotext` 抽得出中文；外部地址没有被访问，出现在 `metrics.native.warnings` 里 |
| 缺工具故障注入 | 临时从搜索路径移除 `pdftoppm` 后建 `media.pdf` Run | `render_pdf_pages` Failed，`error.code = tools_missing`，`details.tools_missing = ["pdftoppm"]`，Run / Step / Thunk Task 与 RPC 一致；恢复工具后新 Run 的 ensure key 因 `environment_revision` 不同而不合并 |

对应的自动化：`cargo test -p workflow -- --ignored`（`functions/media/tests.rs` 的 poppler / ffmpeg / soffice / page-render 用例，依赖本机工具），`uv run test/run.py -p workflow_test` 的 WF-DV-009~ 用例（真实 Zone；`media.page` 是 WF-DV-017）。隔离运行的单测在 `native.rs`（不带 `--ignored` 就跑）；“降到 `nobody`”那一条只在以 root 运行测试时生效。

## 5. 与 paios 镜像的联动（T4b）

- 工作镜像不带媒体工具：`build_aios` 的 `RUNTIME_PACKAGES` 只有 `bash`、`ca-certificates`、`rsync`，`LANG=C.UTF-8`。容器里的 poppler、ffmpeg、`soffice`、字体都来自工具卷上的 `ext-core`（1.2.0 起含 LibreOffice）。
- [paios 容器需求 §6.4 表 B](../app_service/paios容器需求.md) 列了这几项并指回本文。
- runtime 的 `tools_check` / baseline 声明（`src/frame/lib_opendan/src/baseline.rs`，Agent 标准命令检查）**未**改为按本表探测媒体工具；标准命令测试通过不等于媒体基线满足。ExtTools / ext-core 的 `provides` 落地时使用同一清单。

## 6. 未完成项

- `ext-core` 1.2.0 的 soffice 只在 linux-x86_64 上实测（干净的 Ubuntu 24.04 / 22.04 容器，无网络）；linux-aarch64、macOS、Windows 的 payload 未运行过。
- macOS / Windows 上 soffice 的字体回退走系统字体（CoreText / GDI），没写字体的中日韩文字不一定落到随包的 Noto CJK；Chromium 也不读 fontconfig，`ext-core` 的字体和字体包都看不到。未处理。指纹里 `env/fonts` 仍取自宿主的 `fc-list`，`ext-core` 和字体包靠 `env/ext-core`、`env/ext-fonts` 两项跟上（§2）。
- `page-render` 的隔离运行只在 Linux x86_64 上验证过（§3.1）。macOS、Windows 的分支没有运行过。真实 Zone 上以 root 运行的 workflow 跑过 `media.page`：2026-11-15 本机 Zone 上 WF-DV-017 通过（HTML 对象出 PDF、第二次命中缓存、没有读权限的调用者被拒），渲染期间的进程属主都是 `nobody`，PDF 里的外部请求全部失败；进程的环境没有读到，只验了属主（[Agent 浏览器实施 TODO](../../notepads/agent-runtime-browser-todo.md) §12）。
