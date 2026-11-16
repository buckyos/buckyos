# Preview 组件优化 TODO

状态：2026-11-16 已实施（未提交），记录见 §6。未完成：真实 Zone 与真机验收、其余平台的 payload 实测；嵌入页分享的做法与原文不同，待 owner 确认（T10 第 4 项）。包含 VSD / VSDX（Microsoft Visio）预览补齐，以及 App / 嵌入模式的工具栏与标准操作优化。

日期：2026-11-16。源码基线：buckyos `6c0e2eef8`，工作区另有与本任务无关的改动。

来源：owner 询问 Preview 是否具备支持 VSD / VSDX 的条件，并要求将后续工作写入 `notepads`；随后结合两张截图补充 UI 优化要求（§1.1、T7–T11）。本文件只记录任务；建立和更新 TODO 时未修改功能代码、未运行转换或 UI 测试。

## 1. 目标与范围

用户从 Files 或其他复用 Preview 的入口打开 `.vsd` / `.vsdx` 后，能够查看文档的各页、缩放和适应窗口；转换不可用与文件转换失败有明确且不同的反馈。

Preview App 不再呈现系统窗口标题栏叠加组件标题栏的双栏效果；预览操作集中在顶部居中的半透明工具栏。嵌入模式使用一致、简洁的视觉，并补齐新窗口打开、导入 AI Workspace、分享等标准操作。

复用现有链路：文件识别 → `preview-document` → 节点上的 LibreOffice Draw 导入并转 PDF → 现有 PDF 查看器。桌面继续复用浏览器 PDF 查看器，手机继续复用 pdf.js。

本轮不包含 Visio 编辑、导出回 Visio、新增 Visio 到 AI Workspace 的可编辑图形导入器，也不扩展到 `.vst` / `.vss` / `.vsdm` 等其他格式。AI Workspace 部分只接入其已有 import 能力支持的格式；Preview 能查看一种格式，不代表 AI Workspace 能导入该格式。无需引入另一套图形渲染引擎。其他格式扩展的历史工作见 [preview-more-formats-todo.md](preview-more-formats-todo.md)，其中的历史现状不能代替当前源码。

### 1.1 owner 补充的 UI 要求

两张截图分别指出：Preview App 在系统窗口标题栏下又画了一条标题 / 操作栏；嵌入预览在内容顶部覆盖了通栏渐变标题背景。以下是明确要求，T7–T11 给出实现与验收细则。

| 项目 | 要求 |
| --- | --- |
| App 模式 | 将组件内部的第二条 titlebar 改为顶部居中的工具栏，使用系统色半透明背景，可选择 pin 住或自动消失 |
| 嵌入模式 | 优化标题与操作区域，去掉当前渐变色 title 的视觉 |
| 在新窗口中打开 | 换成更能表达新窗口的图标，与“使用其它应用打开”区分 |
| 使用其它应用打开 | 对 AI Workspace 已支持 import 的格式，增加“导入到 AI Workspace”操作 |
| 分享 | 添加分享按钮 |

下文的默认模式、窄屏布局和具体交互沿用现有机制或作为实施建议，不额外扩大到通用窗口系统改造。

## 2. 当前实现与缺口

| 环节 | 当前实现 | 本轮缺口 / 入口 |
| --- | --- | --- |
| 前端分类 | `PIPELINE_DOCUMENTS` 已将 `vsdx` 映射为 `application/vnd.ms-visio.drawing`、`preview-document`；未列出 `vsd` | [mediaTypes.ts](../src/frame/desktop/src/components/preview/mediaTypes.ts)：补 VSD 扩展名和 MIME 识别，核对实际 Content-Type 对分类的影响 |
| 后端内容识别 | ZIP 条目 `visio/document.xml` 或 ZIP + `vsdx` 文件名可识别为 `Kind::Document("vsdx")`；OLE2 流名与扩展名表未包含 VSD | [media.rs](../src/frame/nfs_server/src/preview/media.rs)：补 VSD 内容识别、受约束的文件名补判及测试 |
| 转换器选择 | `Office` 仅有 Writer / Calc / Impress；`office_of` 把 `vsdx`、`odg`、`pub` 归入 Impress | 同上及 [tools.rs](../src/frame/nfs_server/src/preview/tools.rs)：正确区分 Draw 与 Impress 的需求 |
| 能力探测 | 通过 `sw` / `sc` / `sd` 模块库判断组件；不在 LibreOffice program 目录内的 launcher 被视为组件齐全 | Draw / Impress 共用 `sd`，仅检测该库不能证明具备 Draw 和 Visio 导入过滤器；需验证实际启动的安装 |
| PDF 转换与显示 | 已有独立 LibreOffice 用户配置、转换超时、Office 并发槽、缓存和 PDF 结果映射 | 复用 [convert.rs](../src/frame/nfs_server/src/preview/convert.rs)、[mod.rs](../src/frame/nfs_server/src/preview/mod.rs)、[nfspProvider.ts](../src/frame/desktop/src/components/preview/nfspProvider.ts)，无需新增结果族 |
| 工具分发 | ext-core 构建已包含 Draw 与导入过滤器；当前 manifest 为 1.3.1，LibreOffice 为 26.8.2 | [build.sh](../publish/exttool/ext-core/build.sh)、[ext-manifest.json](../publish/exttool/ext-core/ext-manifest.json)：核验实际 payload，不重复实施“加入 LibreOffice” |
| 测试 | 前端有 VSDX 分类测试，后端有合成 ZIP 的 VSDX 识别测试；现有 soffice smoke 验证 DOCX / XLSX | [preview-media-types.test.ts](../src/frame/desktop/tests/datamodel/preview-media-types.test.ts)、[preview.rs](../src/frame/nfs_server/tests/preview.rs)、[soffice.sh](../publish/exttool/ext-core/smoke/soffice.sh)：缺真实 VSD / VSDX 转换和显示验收 |
| 工具栏外观 | [ContentPreview.tsx](../src/frame/desktop/src/components/ContentPreview.tsx) 使用 `absolute inset-x-0 top-0` 通栏布局与 `linear-gradient` 背景，包含关闭、文件名和操作；App 外层也显示窗口标题 | 收拢为顶部居中的工具栏，区分 App 与嵌入宿主的标题 / 退出职责 |
| 工具栏显示与 Pin | 组件已有 `auto / visible / silent`、自动隐藏计时与悬停 / 菜单临时保显；[PreviewAppPanel.tsx](../src/frame/desktop/src/app/preview/PreviewAppPanel.tsx) 的 Pin 是“不自动复用此窗口” | 新增用户可切换的工具栏固定 / 自动隐藏状态；不能把现有窗口 Pin 当作已实现 |
| 嵌入入口 | 宿主可直接嵌入 ContentPreview；[PreviewEmbedApp.tsx](../src/frame/desktop/src/app/preview/embed/PreviewEmbedApp.tsx) 当前固定 `uiMode="visible"`；`variant="inline"` 本身无工具栏 | 同步优化带工具栏的嵌入入口，保留 inline 的精简行为 |
| 新窗口与其他应用 | 新窗口使用 `SquareArrowOutUpRight`，Open with 使用外观相近的 `ExternalLink`；[OpenWithSheet.tsx](../src/frame/desktop/src/app/content/OpenWithSheet.tsx) 展示系统内容 handler、下载和复制引用 | 更换新窗口图标，并接入 AI Workspace 已有导入能力与目标选择流程 |
| 分享 | Preview 尚无专用分享操作；系统已有 [ShareDialog.tsx](../src/frame/desktop/src/sysdlg/ShareDialog.tsx)，当前参数是文件 / 文件夹的 DFS 路径 | 复用 `sysdlg/share`，按原件的来源和权限决定是否能分享，不假设所有 ContentRef 都可直接传入 |

源码包含格式名、系统存在 `soffice`、转换命令返回 0，都不能单独作为 Visio 预览验收通过的依据。执行时重新核对实际服务版本、工具路径、组件与字体；不沿用此前开发机的安装状态作为部署保证。

## 3. 实施任务

### T1. 准备真实样本和基线

- [ ] 为 VSD、VSDX 各准备至少一份真实、可再分发的小文件，放入现有 [preview fixtures](../src/frame/nfs_server/tests/fixtures/preview/)；记录来源、许可、生成工具 / 版本及预期内容。 **进展**：VSDX 已有 `src/frame/nfs_server/tests/fixtures/preview/drawing.vsdx`（本仓库按 MS-VSDX 生成，`publish/exttool/ext-core/smoke/make_visio.py`，随仓库许可）；VSD 是 `tests/fixtures/preview/drawing.vsd`：owner 同意后取自 LibreOffice core 测试数据 `writerperfect/qa/unit/data/draw/libvisio/pass/fdo57117-1.vsd`（11264 字节，sha256 `0be3bdd8b576ff28c280ac744a9538cee96cbc4b984cc796e8e12a155c9c6b77`，MPL-2.0，LibreOffice 的 libvisio 回归样本；预期一页 A4，一个带“test”字样的矩形）。
- [ ] 样本集覆盖多页、中英文文字、连接线 / 箭头、组合形状和嵌入图片。保存原始 Visio 显示或可信导出的对照结果，供人工核对版式。 **进展**：VSDX 样本覆盖两页、中英文、两条带箭头连接线（Connects 粘连）、组合形状、嵌入 PNG；预期内容写在生成脚本里。VSD 样本只有一页、一个带英文字的框，多页 / 中文 / 连接线 / 组合 / 图片只由 VSDX 覆盖。没有 Visio，只能用生成定义和样本说明对照 LibreOffice 渲染（§6），没有 Visio 原生显示的对照图。
- [ ] 在独立临时目录与独立 LibreOffice 用户配置下，先验证两种文件都能转换成可解析、页数正确且包含预期内容的 PDF，再接入业务回归。不要用修改扩展名或仅含文件头的假文件代替真实转换样本。 **进展**：VSDX 已在独立临时目录、独立 profile 下用 ext-core soffice 26.8.2.2 转出 2 页 PDF，文字与图片可解析；VSD 转出 1 页 A4，文字“test”可提取，原件不变。

### T2. 补齐 VSD 识别和预览路由

- [x] 在 `mediaTypes.ts` 补 VSD 的扩展名、实际使用的 MIME 及必要的别名映射到 `preview-document`；保持 VSDX 的现有行为。
- [x] 验证扩展名大小写、缺失 / 通用 Content-Type、明确 Visio Content-Type 等输入；前端不能因 MIME 差异把可转换文档错误分到文本或直接渲染路径。
- [x] 在 `media.rs` 核对真实 VSD 的 OLE2 流结构并增加识别，沿用现有有界读取和内容优先原则。扩展名补判以容器签名为前提，不能将任意 `.vsd` 字节当作有效文档。
- [x] 覆盖无扩展名但内容足以识别、OLE 目录不在初始探测窗口、伪装扩展名和截断文件；无关 OLE 文件继续不支持，已识别但损坏的文档按转换失败处理。
- [x] 核对 Files 的 Content Open Router、Preview App 和嵌入组件入口，保持它们走同一套分类与 provider；只修复本次格式接入涉及的映射。

### T3. 准确探测 Draw / Visio 能力并选择转换器

- [x] 让 VSD / VSDX 明确要求 Draw 及 Visio 导入能力；修正 `office_of` 中同样依赖 Draw 的 `odg` / `pub` 分类，回归 Impress 文档不受影响。
- [x] 选定能区分 Draw / Impress 并验证 Visio 导入过滤器的探测依据；不能仅新增 `Draw → sd` 映射。检查原生安装及 ext-core launcher 对应的真实安装，避免未知 launcher 无条件被标记为能力齐全。
- [x] 保持现有查找顺序，选择第一个真正能处理该格式的安装。宿主 LibreOffice 有 Impress 但没有 Draw / Visio filter 时，应能选择具备能力的 ext-core 安装。
- [x] 没有可用转换器时，在规划阶段返回准确的 `unsupported/no-pipeline` 原因；不要等到启动转换后才报通用转换失败。用户提示应能理解，具体路径和组件信息留在能力诊断 / 日志里。
- [x] 补齐探测缓存的失效条件，使组件 / 过滤器安装变化可被重新发现，避免每次预览都启动重型转换探测。
- [x] 若调整 `hello.repr` 的组件字段或能力结构，同步检查 `describe()`、`missing_office()`、前端 provider、buckyos-websdk 的 `src/nfsp.ts` 和协议文档；不新增与既有机制重复的能力协议。

### T4. 转换、缓存和失败处理

- [x] 让两种格式都进入既有 `Kind::Document` → `preview-document` 路径，输出保持 `result_type: document`、`media_type: application/pdf`。
- [ ] 先验证当前 `--convert-to pdf` 对两种格式的实际行为；只有样本证明需要时才增加 Draw 专用导出过滤器选择，避免影响 Writer / Calc / Impress。 **进展**：VSDX 实测 `--convert-to pdf` 自动选 `draw_pdf_Export`（“as a Draw document”），不需要另指定过滤器；VSD 同样“as a Draw document”经 `draw_pdf_Export`。
- [x] 保持独立用户配置、Office 并发限制、超时和已有外部链接限制。转换不修改原件；失败不留下可被当成成功结果读取的半成品。
- [x] 验证重复打开复用缓存、多个入口共享转换，以及失败后显式 Retry 的行为。如果本次改变了转换输出或转换器选择语义，核对 `converter_id` / work key，按现有机制处理旧缓存。
- [x] 保留 LibreOffice 字体 / 版式可能有差异的提示；真实样本中出现的缺字、连接线错位或嵌入图像丢失要记录，不将“生成了 PDF”等同于效果合格。
- [x] 缩略图沿用内嵌图 / 已缓存 PDF 的现有策略，不为了列表缩略图额外启动一次完整 Office 转换。

### T5. 回归与实际部署验收

- [x] 前端分类测试覆盖 VSD / VSDX、MIME 与扩展名组合，并确认交给 PDF 结果渲染器。
- [x] 后端测试覆盖 VSD 识别、Draw / Impress 区分、过滤器缺失、宿主缺组件而 ext-core 可用、全部不可用，以及真实样本 `repr` / `get_repr` 的完成与失败状态。 真实 VSD / VSDX 样本的嗅探与 `repr` 完成、构造的 OLE2 目录、截断 VSDX 的失败与 Retry 都有用例。
- [x] 给 ext-core 的 Unix [soffice.sh](../publish/exttool/ext-core/smoke/soffice.sh) 和 Windows [soffice.ps1](../publish/exttool/ext-core/smoke/soffice.ps1) smoke 增加真实 VSD / VSDX 转 PDF 检查，使用 payload 自身的工具与字体，防止宿主机完整 LibreOffice 掩盖打包缺件。 **进展**：两份脚本都加了 VSDX（`make_visio.py` 随 payload 的 `smoke/` 分发，`build.sh` 复制）；`soffice.sh` 在本机已安装的 ext-core 1.3.1 darwin-arm64 payload 上用 payload 自身工具跑通；VSD 用 `smoke/visio-fdo57117.vsd`（与测试 fixture 同一文件），随 `smoke/THIRD_PARTY.md`（来源、git blob、sha256、MPL-2.0 声明）一起由 `build.sh` 放进 payload；`compliance.py` 写的 `buckyos-packaging/README.txt`、`SOURCE_CODE.md` 和 ext-core README 都注明这些样本不属于 AGPL 的首方文件。按 `build.sh` 的复制清单摆出的 smoke 目录里跑通（DOCX、XLSX、VSDX、VSD），`test_compliance` 17 项通过。`soffice.ps1` 未在 Windows 上运行。
- [ ] 在真实 Zone 从 Files 打开两种文件，核对页数、可见内容、缩放和再次打开的缓存行为；再验证至少一个复用 Preview 的入口（聊天附件或 `/embed/preview`）。Mock 测试只证明 UI 行为，不算转换链路通过。 **未做**：没有把新 nfs_server / Desktop 部署到真实 Zone 验证（需要 `start.py` 重装运行环境）。
- [ ] 分别验证桌面 PDF 查看器和手机 pdf.js 的显示、翻页与缩放。真机 iOS Safari / Android Chrome 未测时单独记录；桌面浏览器模拟手机不能计作真机验证。 **未做**：只在 Playwright Chromium（mock runtime）里跑过；真机 iOS Safari / Android Chrome 未测。
- [ ] 为 Linux x86_64 / aarch64、macOS x86_64 / arm64、Windows x86_64 记录 payload 测试状态、LibreOffice 版本、样本与结果。没有环境的平台标为未验证，不能将构建通过写成 Visio 预览通过。 **进展**：只有 macOS arm64（本机已装 ext-core 1.3.1，LibreOffice 26.8.2.2）实测；Linux x86_64 / aarch64、macOS x86_64、Windows x86_64 未验证。

### T6. 文档与交付

- [x] 更新 [Preview README](../src/frame/desktop/src/app/preview/README.md) 和 [nfs_server README](../src/frame/nfs_server/README.md)，写清格式支持、转换路径、能力缺失和已知保真限制；同步 Preview 的工具栏显隐、宿主行为、AI Workspace 导入和分享说明。
- [x] 在 [已有格式扩展 TODO](preview-more-formats-todo.md) 的实施记录中关联本任务，保留历史记录与本次验收的区别。
- [x] 按仓库 `AGENTS.md` 检查受影响的内置手册；涉及部署诊断 / 工具分发时核对 `buckyos_admin`、`buckyos_dev_release`，没有受影响项时说明依据。
- [x] 回填本文件的完成项、验证命令、平台结果和仍待解决的问题。提交说明列出代码、协议 / SDK、打包、文档是否发生变化。

### T7. App 模式：居中顶部工具栏与显隐

- [x] 保留系统窗口标题栏管理窗口标题、拖动和窗口按钮；将 Preview 内部通栏标题 / 操作栏改为内容区域顶部居中的紧凑浮动工具栏，不再重复画一条完整文件名 titlebar。
- [x] 使用现有 `--cp-*` 系统主题色形成均匀半透明背景，去掉渐变到底部透明的通栏色带；圆角、边框、间距与系统控件一致，可适量使用背景模糊。亮 / 暗主题及明暗复杂图片上均能辨认按钮。
- [x] 工具栏提供明确的“固定工具栏 / 自动隐藏”切换。复用现有 `uiMode` 与设置机制，默认沿用 `auto`，尊重已有用户选择；固定后闲置不隐藏，解除固定恢复自动隐藏。宿主明确指定 `silent` 的场景继续无工具栏。
- [x] 区分“固定工具栏”和现有“固定窗口，保留用于对比”的状态、文案与图标；前者只影响显隐，后者只影响窗口复用。实施建议将窗口固定放入更多菜单，避免并列的两个 Pin 按钮难以区分。
- [x] 自动隐藏支持鼠标移动、触摸唤出和键盘进入；悬停工具栏、焦点处于工具栏或其弹层、菜单 / 对话框展开时保持可见。用户固定状态与这些临时保显条件组合计算，不能被一次 pointer leave 覆盖。
- [x] 长文件名放在系统标题 / 信息面板等合适位置，不撑宽工具栏；窄窗口 / 手机将次要操作收入更多菜单，保留退出或返回的既有入口。工具栏不覆盖视频底部控制条，不因显隐造成内容尺寸或缩放跳变。

### T8. 嵌入模式：去掉渐变标题，统一操作区域

- [x] 对宿主弹层 / 容器中的 ContentPreview 和 `/embed/preview` 同步替换渐变 title，复用 T7 的主题色、半透明背景与紧凑布局；不能只修 Preview App 而留下截图中的嵌入样式。
- [x] 嵌入宿主没有系统标题栏时，仍提供可识别的文件名与退出入口，文件名可在紧凑工具栏内截断显示并提供完整提示；已有宿主标题时避免重复。关闭嵌入预览只退出该预览，不关闭宿主 App。
- [x] 支持固定 / 自动隐藏工具栏，并处理 iframe 当前强制 `visible` 与宿主配置的关系；按宿主能力提供操作，不直接依赖 Preview App 的 window manager。
- [x] `variant="inline"` 的小尺寸内容块保持无工具栏；普通嵌入和全屏使用同一套操作语义。若需要扩展 embed 消息或配置，同步 buckyos-websdk 的 `app-frame` 协议类型、宿主消费方、示例和测试。

### T9. 标准操作：新窗口图标与 AI Workspace 导入

- [x] 将“在新窗口中打开”的图标换成明确的窗口 / 叠窗形状，优先选用已有图标库，避免继续使用与“使用其它应用打开”近似的外链箭头。保留操作名称、tooltip、键盘可访问性及新建独立窗口的行为。
- [x] 在“使用其它应用打开”的选择列表内，对 AI Workspace 实际支持 import 的原始格式增加“导入到 AI Workspace”。复用现有导入格式判断与入口，避免在 Preview 维护另一份会漂移的格式白名单。
- [x] 核对 [AI Workspace 导入入口](../src/frame/desktop/src/app/aiworkspace/ui/import/) 与 [Content Open Router](../src/frame/desktop/src/app/content/open.ts) 的连接方式。当前普通内容打开路由不等于已有跨应用导入 handler，需明确接入导入动作：选择 / 打开目标工作区，按既有导入流程确定可写的目标位置；用户取消不产生导入结果，成功后能定位到导入内容，失败有明确反馈。
- [x] 判断格式和执行导入时使用原件的引用、名称、MIME 和读取能力；当前 `ContentPreview.requestOpenWith` 优先传转换结果 MIME，实施时须修正导入支持判断的数据来源，不能把 renderer 的 PDF / HTML 等预览衍生物当成原始输入。文件切换后操作必须对应当前项，不能持有上一文件的导入请求。
- [x] 不支持的格式不显示可执行的导入选项；权限不足、原件不可读或来源无法交给 AI Workspace 时按既有能力反馈处理。对 Blob / 对象引用 / 路径来源逐一核对可传递性，不默认为路径；VSD / VSDX 预览接入本身不意味着新增 AI Workspace Visio importer。

### T10. 标准操作：分享按钮

- [x] 在带工具栏的 Preview 中添加有明确图标和“分享”提示的按钮，App 和嵌入模式使用一致语义；窄屏下调整排列，确保分享入口仍可发现。
- [x] 对可分享的原始文件调用系统 `sysdlg/share`，复用现有分享规划、链接和应用转交流程；分享对象是原件，不是预览缓存 PDF、临时 Blob URL 或带访问凭据的读取 URL。
- [x] 将 ContentRef / resolved source 转成分享服务支持的来源并检查权限。当前系统分享接收 DFS 路径；对象引用、聊天附件和本地 Blob 只有在能经既有流程取得合法分享来源时才启用，否则明确提示原因，不伪造路径或点击后静默失败。
- [ ] 嵌入场景通过现有宿主操作机制发起系统分享；需要新增回调或消息时同步类型、消费方与测试。打开 / 取消分享对话框不直接创建链接或发布内容，按现有流程执行用户选择；关闭后返回原预览状态。 **偏差，待 owner 确认**：`/embed/preview` 页在 Zone 根源上、用自己的会话，Share 直接在嵌入页里打开系统分享对话框（`ShareDialog shell={false}`，只有链接；转交 MessageHub / HomeStation、My Content 要 Desktop shell，隐藏），没有扩展 `buckyos.preview-embed` 协议，也没有走 App → shell。宿主弹层（MessageHub 媒体查看、AI Workspace 媒体 / 目录预览、任务中心）没有接 Share：它们的来源是对象 / 附件，当前分享来源只能是 DFS 路径。

### T11. UI 与操作验收

- [ ] 以用户两张截图的问题为对照，补 App / 宿主弹层 / iframe embed 在亮暗主题下的截图：App 只有一条系统 titlebar 加居中浮动工具栏；嵌入没有通栏渐变 title；长文件名、窄窗口和手机布局无挤压或溢出。 **进展**：App（亮 / 暗）、iframe embed（亮 / 暗）、窄窗口、手机宽度的截图在 `notepads/evidence/preview-component-optimization/2026-11-16/`（mock runtime）；宿主弹层没有单独截图（同一组件、同一工具栏）。
- [x] 覆盖工具栏固定、解除固定、闲置隐藏、鼠标 / 触摸 / 键盘唤出、菜单与焦点保显；确认工具栏固定与窗口固定互不改变。隐藏工具栏时按钮不截获内容点击，也不留下不可见的键盘焦点入口。
- [ ] 在图片、视频和多页 PDF 上验证缩放、翻页、播放控制、退出 / 全屏行为及内容布局稳定；复用现有 Preview E2E，避免为纯样式细节编写镜像实现的测试。 **进展**：现有 `preview.spec.ts` / `preview-formats.spec.ts` / `preview-embed.spec.ts` / `content-open.spec.ts` / `share.spec.ts` 在新工具栏下全部通过（39 项）；浏览器自带 PDF 查看器顶部保留固定 52px，工具栏不再盖住它的页码 / 缩放；视频控制条在底部，工具栏和保真提示都不在底部。没有针对视频单独新增用例。
- [x] 验证新窗口图标与 Open with 可区分且各自行为正确；覆盖至少一种 AI Workspace 支持和一种不支持的格式、目标权限不足、取消、成功定位，以及已转为 PDF 的 Office 原件仍按原格式导入。
- [ ] 验证分享可用 / 不可用来源、权限失败、取消与成功；切换当前文件后导入和分享都作用于新文件。App 与嵌入至少各验证一条真实操作链路，不以 Mock 按钮点击代替后端操作成功。 **进展**：可分享（Zone 文件）、不可分享（本设备文件，提示原因）、取消（不产生链接）、嵌入页里创建链接都有 E2E；分享服务是 mock，权限失败和真实链接创建没有在真实 Zone 验证。
- [x] 所有新增文案加入现有中英文 i18n；图标按钮有可访问名称，焦点可见，触摸区域适合手机，自动显隐尊重系统减少动画设置。

## 4. 建议执行顺序与验证入口

Visio 路线先做 T1，再做 T2 / T3，随后完成 T4 和 T5；UI 路线可独立并行，先做 T7 / T8，再接入 T9 / T10 并执行 T11。两条路线完成后统一回填 T6。若缺少真实样本或目标平台环境，可继续独立的实现任务，但相关验收项保持未完成。

以下是后续实施时的验证入口，建立本文时未执行。先核对运行环境和仓库指引；不为本任务清空 Zone 或覆盖无关工作区改动。

在 `buckyos/src/` 下：

```bash
cargo test -p nfs_server --lib preview -- --test-threads=1
cargo test -p nfs_server --test preview -- --test-threads=1
```

在 `buckyos/src/frame/desktop/` 下：

```bash
pnpm check
node --experimental-strip-types tests/datamodel/preview-media-types.test.ts
pnpm exec playwright test tests/e2e/pages/preview.spec.ts tests/e2e/pages/preview-formats.spec.ts tests/e2e/pages/preview-embed.spec.ts tests/e2e/pages/content-open.spec.ts
```

真实 Zone 验证前，从仓库根目录执行 `uv run src/check.py` 并确认已激活；沿用 `src/frame/desktop/playwright.dv.config.ts` 和 `tests/dv/` 的现有认证 / 样本准备方式补 Visio 用例。ext-core smoke 按其构建流程在目标 payload 环境中运行，不能直接使用开发机默认 PATH 代替。现有转换测试可能在工具缺失时跳过，须检查实际执行和跳过项。

UI 与操作回归补入上述 Preview / content-open 用例；AI Workspace 导入复用 `tests/aiworkspace/office-import.spec.ts`、`table-import.spec.ts` 等相应格式用例及其专用配置，分享参考 `tests/e2e/pages/share.spec.ts`、`share.nfsp.spec.ts` 和 `tests/dv/share.spec.ts`。按实际改动选择相关用例，并为 T11 留下截图和真实操作结果。

## 5. 完成标准

- [ ] VSD / VSDX 都能通过真实后端转换并在 Preview 查看全部预期页面，原件保持不变。 **进展**：VSD、VSDX 都经真实 nfs_server + ext-core soffice 转换（集成测试，原件不变）；没有在真实 Zone 的 Preview 界面里看。
- [ ] 真样本的文字、图形、连接线和图片完成对照检查，差异有记录。 **进展**：VSDX、VSD 都已对照（§6）。
- [ ] 缺 Draw / Visio filter、损坏文件、转换超时的行为经过验证，界面无永久加载状态。 **进展**：缺 Draw / Visio filter（规划阶段 `unsupported/no-pipeline`）、损坏文件（`failed` + 显式 Retry）已测；转换超时沿用既有机制，没有为 Visio 单独测。
- [ ] 至少一个实际平台完成 payload → nfs_server → Preview 的完整验收，其余平台与真机覆盖状态明确列出。 **未完成**：ext-core payload（本机已装）→ nfs_server（集成测试）这一段已通过，Preview 界面在真实 Zone 的那一段没做。
- [x] 现有 Office / PDF 预览相关回归通过，文档和受影响接口同步完成。
- [x] App 双 titlebar 与嵌入渐变 title 问题消除，居中半透明工具栏在亮暗主题和窄屏下可用，固定 / 自动隐藏及键盘 / 触摸交互验收通过。
- [x] 新窗口图标完成替换；AI Workspace 支持的 import 格式可从 Open with 导入；分享按钮接通系统流程，操作对象与权限正确。

## 6. 实施记录（2026-11-16）

| 项目 | 记录 |
| --- | --- |
| 实施提交 / 工作区基线 | 基线 `3036f52fb`（`6c0e2eef8` 之后只多了本文），开始时工作区干净；改动未提交。buckyos-websdk 未改。 |
| 改动入口 | 后端 `src/frame/nfs_server/src/preview/{media,tools,rpc,mod}.rs`；前端 `src/frame/desktop/src/components/ContentPreview.tsx`、`components/preview/{mediaTypes,types}.ts`、`app/preview/{PreviewAppPanel.tsx,embed/PreviewEmbedApp.tsx}`、`app/content/{OpenWithSheet.tsx,share.ts}`、`app/aiworkspace/{AIWorkspaceAppPanel.tsx,ui/canvas/CanvasView.tsx,ui/import/externalImport.ts}`、`sysdlg/ShareDialog.tsx`、i18n（`dictionaries.ts`、`i18n/aiworkspace/core.ts`）；打包 `publish/exttool/ext-core/{build.sh,compliance.py,README.md,smoke/soffice.sh,smoke/soffice.ps1,smoke/make_visio.py,smoke/visio-fdo57117.vsd,smoke/THIRD_PARTY.md}`；手册 `buckyos_admin/SKILL.md`；文档 Preview README、nfs_server README、`preview-more-formats-todo.md` §7.7。 |
| 协议 / 共享类型 / SDK / 依赖 | NFSP `hello.repr`：`soffice[].components` 多了 `draw`，新增 `soffice[].filters`（`visio`）、`soffice_missing_filters`、`soffice_unresolved`；`repr` 规划失败的 `detail` 去掉了路径（诊断仍在 `hello.repr.detail` 和日志）。消费方核对：前端 provider 不读这些字段，websdk `src/nfsp.ts` 没有 soffice 字段，只有 nfs_server README 与 `buckyos_admin` 手册写到，已同步。`buckyos.preview-embed` 协议未改。前端 `PreviewCapabilities` 多 `share`，`ContentPreview` 新增 `titleInHost` / `onUiModeChange` / `onRequestShare`。无新依赖。ext-core manifest 版本未升（payload 只多 smoke 文件，其中 `visio-fdo57117.vsd` 是第三方 MPL-2.0 样本，发版时再升）。 |
| 样本与对照结果 | `tests/fixtures/preview/drawing.vsdx`（5337 字节）：`smoke/make_visio.py` 按 MS-VSDX 生成，输出确定（固定时间戳），随仓库许可再分发。VSD：`tests/fixtures/preview/drawing.vsd`（来源、许可、sha256 见 T1），OLE2 流 `VisioDocument`、`SummaryInformation`、`DocumentSummaryInformation`，转出 1 页 A4，一个细黑边框里居中“test”，与样本说明一致。VSDX 预期：第 1 页“流程 Flow”三个框（开始 Start / 检查 Check / 完成 Done）、两条红色带箭头连接线、一个虚线框内带“组合 Group”的组合；第 2 页一张 96×72 PNG 和“图片说明 Picture caption”。ext-core soffice 26.8.2.2（darwin-arm64）结果：2 页 Letter 横向，文字全部可提取，箭头、组合、图片都在，图片位置与尺寸符合定义；差异：Calibri → Liberation Sans，中文落到 Noto Sans CJK HK（macOS 上不走 fontconfig 的简体优先），未见缺字、连接线错位或图片丢失。 |
| Draw / Visio 探测依据 | 读 soffice 实际启动的安装（launcher 脚本跟到它 `exec` 的 soffice，ext-core 的 sh 与 cmd 两种都认；跟不到的不当作齐全，进 `soffice_unresolved`）。应用 = 模块库 + 该应用的 registry：Writer `sw`+`writer.xcd`、Calc `sc`+`calc.xcd`、Impress `sd`+`impress.xcd`、Draw `sd`+`draw.xcd`。Visio = 有 Draw、`draw.xcd` 登记 `draw_Visio_Document` 与 `com.sun.star.comp.Draw.VisioImportFilter`、服务注册表 `*.rdb` 中实现它的库存在（本机 ext-core 是 `libmergedlo.dylib`）。全部是文件检查，不启动转换；探测缓存的身份加入了真实二进制、模块库、xcd、rdb。`office_of`：odg / pub / vsd / vsdx → Draw，`filter_of`：vsd / vsdx → Visio。 |
| 已运行命令、通过 / 失败 / 跳过 | `cargo test -p nfs_server --lib -- --test-threads=1`：88 通过。`cargo test -p nfs_server --test preview -- --test-threads=1`：18 通过、2 失败——`stills_go_through_imagemagick`（本机 ImageMagick 读 DNG 要 `darktable-cli`）与 `videos_remux_or_transcode_by_runtime_hint`（ext-core ffmpeg 没有 `zscale`），都是环境问题，与本次无关；新增 `visio_drawings_become_pdf`（VSDX 两页、VSD 一页）、`an_install_without_an_application_or_filter_leaves_it_to_the_next_one` 通过（后者原是 Linux 专用，改为 Unix 都跑）。`node --experimental-strip-types tests/datamodel/preview-media-types.test.ts` 通过。`pnpm check`（`tsc -b`）通过；改动文件 `eslint` 无问题（全量 lint 仍有 `PdfJsRenderer.tsx` 一条原有错误，未改）。`npx playwright test tests/e2e/pages/{preview,preview-formats,preview-embed,content-open,share}.spec.ts`：39 通过。`AIWS_BIN=… NFS_BIN=… pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/preview-import.spec.ts`：2 通过（真实 aiworkspace + nfs_server 后端）。`PATH=/opt/buckyos/tools/bin:/usr/bin:/bin sh <按 build.sh 清单复制的 smoke 目录>/soffice.sh`：通过（DOCX、XLSX、VSDX、VSD）。`uv run --project ../../.. python -m unittest test_compliance`（ext-core 目录下）：17 通过。未运行：`soffice.ps1`、真实 Zone DV、`cargo test` 全 workspace（只改了 nfs_server）。 |
| 平台、浏览器与真实 Zone 结果 | macOS 15（Darwin 24.6）arm64 + 本机已装 ext-core 1.3.1（LibreOffice 26.8.2.2）：后端转换与 smoke 通过。浏览器只有 Playwright Chromium（桌面 1440×900、窄窗口 520–560 px、390 px 宽手机布局）。真实 Zone、iOS Safari / Android Chrome、Linux / Windows / macOS x86_64 payload 未验证。 |
| App / 嵌入工具栏截图与显隐验收 | 截图：`notepads/evidence/preview-component-optimization/2026-11-16/`（`app-image-{light,dark}`、`app-narrow-light`、`app-phone-dark`、`embed-image-{light,dark}`）。App 只剩窗口标题栏，工具栏居中半透明（`--cp-surface-opaque` 80% + 模糊 + 细边框），不带文件名与关闭；嵌入页没有渐变，工具栏里有关闭和截断的文件名。E2E 覆盖：闲置隐藏且 `inert`、Tab 唤出并进入、焦点保显、固定后闲置不隐藏并写入 Preview 设置的默认 UI 模式、解除固定后隐藏、窗口保留在“更多”且不影响工具栏固定、窄窗口把次要操作收进“更多”而保留分享。实现中修了一个计时问题：开发模式 StrictMode 重放 effect 时会清掉隐藏计时器而不置空，旧代码靠菜单 effect 碰巧重排计时；现在 effect 发现“显示中但没有计时”会重新开始。触摸唤出沿用 `pointerdown` / `touchstart`，未在真机验证。 |
| AI Workspace 导入与分享真实链路 | 导入：Files → Preview App → Open with… → “导入到 AI Workspace” → AI Workspace 窗口 → 画布走拖放同一路径（PDF 先问“文件 / 页面”）→ 选“文件”后真实后端大纲里出现该 PDF（PV-01）；Markdown 不提供导入；DOCX 在 Preview 里显示为转换后的网页，交给 AI Workspace 的是原件（名称与 zip 原件字节，后端记录的类型不是 `text/html`，PV-02；mock 的 DOCX 不是真 Office 包，所以画布按文件收下）；没有打开工作区时显示等待提示，可“不导入了”。目标只读 / 演示中时等待提示显示原因（实现了，没有 E2E）。分享：App 对 Zone 文件打开 `sysdlg/share`（mock 分享服务），本设备文件给出原因；嵌入页创建链接（mock 服务）。真实 Zone 上的分享未验证。 |
| 已知限制与剩余待办 | 1）VSD 样本内容很简单（一页、一个框、英文），复杂 VSD 的版式没有样本验证。2）真实 Zone、真机、其余平台 payload 验收。3）嵌入页分享没有走宿主（T10 第 4 项偏差，待确认是否要扩展 `buckyos.preview-embed`）；宿主弹层未接分享。4）iframe 渲染的结果（HTML、浏览器 PDF 查看器）内部移动鼠标仍唤不出自动隐藏的工具栏，只在指针进入 iframe 时唤出；需要时用工具栏的固定按钮，或在 Preview 设置里把默认 UI 模式设为常显。5）窗口对话框层不接管焦点也不处理 Esc（既有行为）：分享前先让触发按钮失焦，避免 Esc 关掉背后的预览；Open with 面板同样问题未处理。6）ext-core 发版需升 manifest 版本以带上新的 smoke 文件。 |
