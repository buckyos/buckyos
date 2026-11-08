# llm_understand_media 长 PDF 实例复盘 TODO

日期：2026-11-07
来源：DV 本机 Zone，devtest 的 Jarvis 处理 `cyfs:///home/devtest/dk10000.pdf`（DK《English for Everyone Illustrated English Dictionary》，403 页，272,722,232 字节）。涉及 session `work-20261108-3`、`work-20261108-4`，worklog 在 `/opt/buckyos/data/home/devtest/agents/jarvis.test.buckyos.io/sessions/<id>/.opendan_agent_session/worklog.jsonl`。
设计依据：[LLM Understand Media](../doc/llm_context/LLM%20Understand%20Media.md) §4.6–§4.8。
状态：第四轮（2026-11-08）：M1–M11 已实施，未提交。实施结果、与本文的偏离、§3 的答案见 §5；DV 结果见 §5.4。

对照材料（临时目录，会被清理）：
- 这次建出的目录已导出到 `/tmp/claude-1000/-home-lzc-projects/29456eb0-8982-4fb8-a544-503c2657268e/scratchpad/dk10000-catalog`（`index.html` 可直接看），导出脚本是同一 scratchpad 下的 `scripts/export_catalog.py`。
- OCR 实验的输出在同一 scratchpad 的 `ocr/` 下。

## 0. 结论

管线本身没问题。3 个 Run 建出了完整目录：403 页都有页图，402 页有文本，`complete: true`，步骤缓存 30 天。问题出在入口和出口两头：

1. **入口**：cyfs 路径在容器里不被接受。修复已经写好，但没有部署，这一次多花了一个 session 和一轮用户追问。
2. **等待**：转换花了 6 分半钟，工具期间没有任何输出。Jarvis 等了两分钟，就自己用 pdftotext / pdftoppm 重做了一遍，还多调了 3 次模型。结果回来时，它的总结已经写好了。
3. **结果**：只给模型看了 7 页。原因有三：文本额度被 `-layout` 补的空格吃掉；页图只跟着已选文本走；403 页的索引退化成了一行。
4. **文字层是乱码**：这本书自带的文字层像是用中文模型做的 OCR（英文书里出现 `矗阳市`、`『`），按词检索命不中，送进去的文本也几乎没用。这是核心问题（Q2）。
5. **补读**：Agent 想按页补读，只能再走一次模型。目录也没有以 Agent 能浏览的形式交给它。

### 0.1 用户裁决（2026-11-07）

| # | 裁决 | 落点 |
| --- | --- | --- |
| Q1 | `read` 原则上尽量什么都能读，但"index + 文本 / 图片"这个结构的本意是利用 LLM 成熟的目录检索能力，所以应该还原成一个本地目录交给 LLM | M9，不给 `read` 加 `mmanifest:` 模式 |
| Q2 | OCR 乱码是核心问题，要给出改进办法 | M10：用 tesseract 重新识别，按页在文字层和 OCR 之间选。设计文档 §9 原定"OCR 不进本期，workflow 不依赖 tesseract"，相应改掉 |
| Q3 | 有 LLM 可用时，把 goal 相关的关键词翻译成资料的主语言 | M11 |
| Q4 | 窗口先不并行 | 不做 |
| Q5 | 用一个可以清除、Agent 能访问的本地目录；长期缓存在 named store 里。目录结构是为 Q7 服务的：Agent 可以在目录里挑出具体页图，再做 LLM 分析 | M9 |
| Q6 | 常见语言都要带上 | M10 的语言包和选语言 |
| Q7 | 由 Q5 定：目录交给 Agent，由它用自己的工具浏览，再对具体页图调 `llm_understand_media`；旁路模型仍然不开工具 | M9 |

## 1. 经过

| 时间（HST） | 事件 |
| --- | --- |
| 21:14:13 | 用户："总结一下cyfs:///home/dk10000.pdf 的内容"，Jarvis 开 work-20261108-3 |
| 21:14 | `llm_understand_media 'cyfs:///home/dk10000.pdf'` 被拒（容器里是旧二进制）。Jarvis 查了本地 `/home/dk10000.pdf` 和 `/opt/buckyos/data/home/dk10000.pdf`，都不存在，报告"读到 0 页"后结束 |
| 21:17:47 | 用户："cyfs:///home/devtest/dk10000.pdf 这个路径你能访问么"，Jarvis 开 work-20261108-4（21:18:41） |
| 21:18 | cyfs 路径再次被拒。改用 `/opt/buckyos/data/home/devtest/dk10000.pdf`，25 秒后转为后台任务 |
| 21:19:07 / 21:21:30 / 21:24:14 | 依次创建三个 `media.pdf` Run，分别处理第 1–200、201–400、401–403 页。窗口串行，前两个分别用了 2 分 23 秒和 2 分 44 秒 |
| 21:20:18、21:20:52、21:23:55 | runtime 的 task_watch 每 30 秒唤醒一次，提示 "Go on with something that does not depend on it" |
| 21:20:52 起 | Jarvis 自己用 `pdftotext -layout` 抽了全书（1.8 MB），按页列出首行，用 `pdftoppm` 渲染了 18 页，拼成 3 张图，每张调一次 `llm_understand_media`（走 `media.image`） |
| 21:23:24 | 根据自己的抽取写好总结 |
| 21:25:16 | PDF 结果回来：模型只看了第 1、34、68、101、135、383、403 页，提示 396 页未展示 |
| 21:26:02 | Jarvis 把覆盖说明补进报告，结束 |

多出来的成本：
- 1 个失败的 session 和 1 轮用户追问；
- 3 次唤醒推理；
- 3 次图片模型调用；
- session 目录里 1.8 MB 文本和 18 张渲染图。

## 2. 修复项

### M1 部署 Zone 路径支持

- **现状**：[zone_file.rs](../src/frame/agent_tool/src/llm_understand_media/zone_file.rs) 已于 2026-11-07 实现，还没提交。容器里的 `/opt/buckyos/bin/opendan/agent_tool` 是旧版本。
- **做**：提交，重建 aios 镜像并安装。
- **验收**：按 §4 重跑，两个路径都能直接读。`cyfs:///home/dk10000.pdf` 应经 owner home 回退找到 `/home/devtest/dk10000.pdf`，输出里有一行说明。
- **顺带观察**：work-3 里 UI session 明确交代了"不改为所有者 home 下的其他路径"。看 Jarvis 拿到工具的回退说明后怎么处理。

### M2 文本先规范空白，再计数和发送

- **现状**：
  - `media.extract_pdf_text` 用 `pdftotext -layout`（[pdf.rs:120](../src/kernel/workflow/src/functions/media/pdf.rs#L120)），版面靠空格补齐。
  - 全书 1,863,227 字，其中只有 320,308 个非空白字符。
  - 第 101 页 47,930 字，其中 46,378 个是空格。
  - [plan_pages](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L1086) 按原始字数算预算，并原样把文本送给模型。
- **做**：在 agent 侧读入页文本时先规范，计数、发送、写入 M9 目录都用规范后的文本：
  - 去掉行尾空白；
  - 连续空行合成一行；
  - 行内 2 个以上的空白合成两个空格（保留分栏的提示）。
- **实测**：按这个规则，全书变为 430,473 字（全部合成单空格是 411,996），第 101 页变为 1,570 字。
- **不改 workflow 函数的原因**：改 `extract_page` 的输出要动全局的 `IMPLEMENTATION_REVISION`（[mod.rs:45](../src/kernel/workflow/src/functions/media/mod.rs#L45)），所有媒体缓存会一起失效。改在 agent 侧，对已经建好的目录立即生效。
- **验收**：单测。本书 60,000 字的额度应能装下大约 56 页文本（平均每页 1,068 字）。

### M3 单页文本设上限，均匀取页不留大洞

- **现状**：
  - goal 没有命中任何页时，按平均页长算出取 12 页（[pipeline.rs:1163](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L1163)），然后按顺序装入，装不下的跳过（[pipeline.rs:1173](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L1173)）。
  - 第 1、34、68、101、135 页合计 59,974 字，其中第 101 页就占了八成。后面只有 5 个字的第 403 页挤了进去，第 136–382 页整段没有送。
  - M2 之后仍有大页：第 389 页规范后还有 7,465 字（多为索引页）。
- **做**：
  - 文本不能全送时，每页截到 `额度 / 份数`，设一个下限（例如 1,500 字）。截断处标注 `[page N text truncated: X of Y chars]`。
  - 份数按截断后的长度算，保证均匀选中的页都装得下。
  - goal 点名的页和命中的页放宽上限。
  - `details.coverage` 增加 `text_truncated` 页码列表。
- **验收**：单测：在 402 页里放一页 48k 字。选中的文本页要分布全书，相邻两页的间隔不超过平均间隔的 2 倍。

### M4 页图独立铺满额度

- **现状**：没有点名页、也没有命中页时，页图只从"已选文本的页"和"只有页图的页"里取（[pipeline.rs:1188-1204](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L1188-L1204)）。16 张的额度只用了 7 张：6 张对应选中的文本页，1 张是唯一没有文本的第 383 页。
- **做**：按现有优先级取完以后，剩余额度在所有有页图的页里均匀取，跳过已选的页。
- **验收**：本书送出 16 张页图，分布在全书。

### M5 页数多时，给旁路模型的索引不要整份退化

- **现状**：
  - 索引预算 `INDEX_CHARS = 6,000`（[pipeline.rs:40](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L40)），403 页摊下来每页不到 16 字。
  - 于是 [render_index](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L1047) 改为按类别列页码范围。模型拿到的只有 `text+image: 1-382, 384-403` 和 `image: 383` 两行，等于没有索引。
  - 模型回答里也写了"没有展示完整目录"。
- **做**：放不下每页开头时，均匀取一部分页列出开头（例如每页 40 字，放得下多少页就取多少页），其余页按范围列，不要整份退化。
- 这一项只管旁路模型的输入。给 Agent 的完整索引在 M9 的目录里。

### M6 识别更多页码写法，并加 `--pages`

- **现状**：[goal_pages](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L1009) 只认标记词后面的第一个数字或一个区间：
  - `page 14, 50, 70` 只得到 14；
  - `第14、50、70页` 一页都得不到；
  - `物理页14`、`p14`、`pp. 3-5` 都不认。
  - Jarvis 的实际写法是"PDF物理页14、50、70、104、148、162"。
- **做**：
  1. `goal_pages` 支持列表（逗号、顿号、"和"、"and"）以及 `页N`、`pN`、`pp.` 这几种写法。
  2. 新增 `--pages <1,5,10-20>` 参数，Agent 不必把页码藏在 goal 里。与 goal 里识别出的页码取并集。
- **验收**：单测覆盖以上写法。

### M7 长转换输出进度

- **现状**：helper 等 Run 期间什么都不输出（[wait_terminal](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L704)）。6 分半钟里，Agent 只看到"仍在运行"。runtime 唤醒时提示它去做不依赖这个任务的事，Jarvis 就自己把转换重做了一遍。
- **做**：在 stderr 输出进度行。stdout 的最后是 JSON 结果，不要混进去。
  - 开始：`converting 403 pages in 3 runs (first time for this file; later calls reuse the result)`
  - 每个窗口结束：`pages 1-200 ready (2m23s)`，命中缓存时为 `pages 1-200 from cache`
  - 目录写好：`catalog written to <目录路径>`（M9）
  - 进入模型分析：`reading 16 page images and 56 pages of text with <model>`
- **验收**：在容器 session 里，task_watch 唤醒时的 `<<background_context>>` 能看到这些行。如果看不到，要改 runtime 展示后台任务输出的方式，不要改到 stdout。

### M8 帮助文本和结尾提示

- **现状**：两个 session 里，Jarvis 都先跑了 `llm_understand_media --help`。这是告诉它"长文档第一次转换要几分钟，之后可以复用"的最好位置。
- **做**：
  - USAGE（[llm_understand_media.rs:2756](../src/frame/agent_tool/src/llm_understand_media.rs#L2756)）加一段：
    - 长文档第一次转换要几分钟，期间会输出进度；
    - 长文档会展开成一个本地目录（M9），可以直接 `grep` 文本、按页补看；
    - 转换结果会缓存，换个问题再问，或用 `--pages` 补读，都不会重新转换，只多一次模型调用；
    - 不需要自己渲染页面。
  - 覆盖说明（[pipeline.rs:908](../src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L908)）改为给出目录路径和可用的写法，例如 `llm_understand_media <目录> --pages 2-33` 或 `llm_understand_media <目录>/pages/0101.png`。
- Jarvis 的 cmd_skills 不改，由工具自己说清楚。

### M9 把目录还原成 Agent 可访问的本地目录（Q1、Q5、Q7）

- **定位**：
  - 长期缓存在 named store 里，即 workflow 的步骤缓存（30 天），加上交付时给请求者登记的持有记录（§4.8.2）。
  - 本地目录只是这份缓存的展开，随时可以删除。删了以后，下次调用会重新展开（转换命中缓存，只是复制文件）。
  - 目录结构为 Q7 服务：Agent 用 `ls`、`grep`、`cat` 找到要看的页，再对那一页的页图调 `llm_understand_media` 做分析。本例中 Jarvis 自己做的"按页列首行"、用 pdftotext 抽全文、用 pdftoppm 渲染页面，都不再需要。
- **位置**：放在 Agent 的工具缓存根目录下，与 `DENO_DIR` 等同级，由 [`tool_cache_env`](../src/frame/lib_opendan/src/baseline.rs#L217) 新增一项 `LLM_UNDERSTAND_MEDIA_CATALOG_DIR` 注入。三种运行形态的路径沿用 [path_usage.md](../doc/path_usage.md) 的约定，不进用户 home，不随 AgentRoot 同步：

  | 形态 | 目录 |
  | --- | --- |
  | 容器 | `/opt/buckyos/instance/media-catalog/`（app 私有的 instance volume） |
  | 原生私有 runtime（Desktop Lite） | `<AgentRoot>/.cache/media-catalog/` |
  | 节点 runtime（host） | `<账户 home>/.opendan/<agent_id>/cache/media-catalog/` |

  没有这个变量时（例如单独运行 helper），退到系统临时目录，与 Run 账本的做法一致（§4.8.1）。path_usage.md 的那张表要同步加上这一项。
- **每份文档一个目录**：目录名是工具收到的源文件对象 id，例如 `media-catalog/cyfile-25b7a541…/`（完整 id）。
  - 路径本身就能说明来源。目录被清掉之后，Agent 仍拿旧路径来调用，工具可以从路径取出 obj_id，用它重新跑管线（命中缓存）、重新展开，再继续处理。
  - 同一文件重复冻结得到的 obj_id 相同（时间字段为 0），所以同一 Agent 处理同一个文件，只会有一个目录。
- **结构**：

  ```text
  media-catalog/<obj_id>/
    README.md        来源（原路径、名字、obj_id）、页数、文本来源统计（文字层 / OCR）、用法示例
    index.tsv        page  status  text_source  chars  image_wh  first_line
    manifest.json    合并后的 MediaManifest，附各窗口的 manifest id
    pages/0001.txt   选定的文本：M2 规范后；按 M10 选用 OCR 时就是 OCR 文本
    pages/0001.png   页图
  ```

- **Agent 在目录里做 LLM 分析（Q7）**：
  - `llm_understand_media <目录>/pages/0101.png --goal …`：工具认出这是某个目录里的页图后：
    - 直接用 manifest 里的 `image_obj_id`，不再冻结、上传一份新文件；
    - 告诉模型这是"《原文件名》第 101 页"，并附上这一页选定的文本；
    - `details.media_source` 记为 `{catalog: <obj_id>, page: 101}`。
  - `llm_understand_media <目录> --pages 14,50,70 --goal …`：一次看几页（M6），直接读 `manifest.json`，不调 workflow。
  - Agent 也可以先用 ImageMagick 裁出局部，再把裁出的文件当普通图片传进来。README 写明页图的长边是 2048 像素，裁剪不能增加细节。需要更高分辨率时单页重渲染（另一组参数，单独缓存），这件事以后再做。
  - 页图的路径被清理掉时，按上面"每份文档一个目录"的规则先重建，再处理。
- **写入与清理**：
  - 先写到同级的临时目录，写完再改名到正式位置。最后写 `.complete`，记录 manifest id 和文本选择规则的版本。
  - 再次调用时，`.complete` 一致就不重写，只更新它的 mtime；规则版本变了，只重写 `pages/*.txt` 和 `index.tsv`。
  - 总量超过上限（`LLM_UNDERSTAND_MEDIA_CATALOG_MAX_BYTES`，默认 5 GiB）时，工具按 `.complete` 的 mtime 删掉最久没用的目录。runtime 或用户也可以随时删整个 `media-catalog/`。
  - 写入只用工具已经能读到的对象，不需要新的权限。
- **短文档不展开**：直传或一次全送得下的文档不建目录，门槛沿用"长文档"的定义（`LONG_DOC_PAGES`，默认 20 页）。
- **体积**：本例 445 MB，几乎全是页图；文本约 0.4 MB。
- **设计文档要改的地方**：
  - §3 的 fork-and-discard 只约束主干 history；目录是这次调用有意留下的缓存，不随工作目录一起删除。
  - §4.7 加"Agent 侧目录"。
  - §9 的"目录产物与 `read` 的边界"改为已定：不走 `read`，走本地目录。
- **验收**：
  - 本例调用后，在目录里 `grep -il colosseum pages/*.txt` 能找到第 101 页（依赖 M10 的 OCR 文本）。
  - `llm_understand_media <目录>/pages/0101.png --goal "列出这一页的建筑"`：答案用到了页码和该页文本，Named Store 里没有新增文件对象。
  - 删掉整个目录后，用同一路径再调用：目录自动重建，转换步骤全部来自缓存。
  - 清理：单测覆盖上限触发时按 mtime 删除。

### M10 OCR：文字层乱码时重新识别（Q2）

**实测**（tesseract 5.5.3，ext-core 自带，只有 `eng` 和 `osd` 两个语言包；输入是管线已渲染的 1713×2048 页图，`--psm 11`，丢掉置信度低于 60 的词）：

| 页 | 文字层：词典词比例 / 中日韩字符数 | tesseract：词典词比例 | 耗时 |
| --- | --- | --- | --- |
| 14 | 35% / 12 | 91% | 0.24 s |
| 70 | 41% / 7 | 86% | 0.55 s |
| 101 | 17% / 320 | 76% | 0.52 s |
| 194 | 27% / 33 | 81% | 0.45 s |
| 300 | 16% / 24 | 93% | 0.31 s |

第 101 页对比：
- 文字层：`44.2 FAM01US. BUILDIIINGS AND M10NUMENTS1`
- tesseract：`FAMOUS BUILDINGS AND MONUMENTS … the Colosseum the pyramids the Great Mosque of Djenné … the Taj Mahal the Forbidden City … the Eiffel Tower … Burj Khalifa`

`--psm 3` 与 `--psm 11` 相比：在本书和一份原生 PDF 上，`psm 11` 识别出的词都不少于 `psm 3`，耗时相近。

**判断文字层坏没坏**（不依赖词典的两个指标）：
- **干净词比例**：只由字母组成（可带一个首尾标点）的词占全部词的比例。
- **一致率**：OCR 的词里，也出现在文字层里的比例。

| 样本 | 文字层干净词比例 | OCR 干净词比例 | 一致率 |
| --- | --- | --- | --- |
| 本书 5 页 | 0.26–0.49 | 0.70–0.80 | 0.21–0.52 |
| 原生 PDF（`/usr/share/pve-docs/pve-admin-guide.pdf`）4 页 | 0.91–0.95；目录页因点线只有 0.04 | 0.49–0.95 | 0.48–0.98 |

候选规则：一致率 < 0.6，且 OCR 干净词比例比文字层高 0.2 以上时，用 OCR 文本，否则用文字层。这 9 页全部判对。样本太少，阈值要在 §4 的语料上再定。

**做**：

1. **workflow：新函数 `media.ocr_pages`**（Native、pure，工具依赖 `tesseract`，放在 Pdf lane）。
   - 输入：`render_pdf_pages` 和 `extract_pdf_text` 的输出、语言。
   - 只识别两类页：没有文本的页，以及文字层干净词比例 < 0.6 的页。原生文档几乎不触发。
   - 每页输出 OCR 文本对象、平均置信度、两个指标。
   - `media.pdf` 与 `media.document` 的定义都加这一步，`build_catalog` 多接一个输入。
   - 已有的抽取和渲染步骤的 fun_id 不变，所以已缓存的结果仍然命中，只有 OCR 和 build_catalog 需要重跑。
2. **manifest**：`MediaEntry` 增加 `ocr_text_obj_id` 和 `text_quality`（两个指标、OCR 置信度）。同步 [media_manifest.rs](../src/kernel/buckyos-api/src/media_manifest.rs)；目前没有 TS 侧的类型需要同步。
3. **用哪份文本由 agent 侧决定**。workflow 只产出事实（两份文本和指标），选用规则放在 agent_tool。这样调规则不会让缓存失效。`plan_pages`、M9 的目录、M11 的检索都用选定的文本。`index.tsv` 和 `details.coverage` 记录每页用的是哪份。
4. **扫描件**（整页没有文字层）这次也有了文本。设计文档 §4.6.5 里"扫描页靠页图做视觉覆盖，本期不做 OCR"要改。
5. **选语言**：按文档选，不按页选。
   1. 均匀抽约 5 页，用 `osd`（`--psm 0`）判断文字系统和文字方向。
   2. 按文字系统选语言：
      - 汉字：`chi_sim` 和 `chi_tra` 各试 2 页，取平均置信度高的；
      - 日文：`jpn`；韩文：`kor`；西里尔字母：`rus` / `ukr`；阿拉伯字母：`ara` / `fas`；其余文字系统各取对应的语言；
      - 拉丁字母：先用 `eng` 识别，再用内置的常用词表判断是哪种语言，不是英语就换成该语言重新识别。
   3. 除英语外，都组合上 `+eng`，因为中英混排很常见。竖排文字用 `_vert` 版本。
   4. 选定的语言和依据写进函数输出和 manifest（`ocr.lang`、`ocr.script`）。M11 直接用它作为资料的主语言。
   5. 缺少对应语言包时不识别，在条目里记 `ocr: lang_unavailable`。
6. **语言包（Q6：常见语言都带上）**：
   - conda-forge 的 tesseract 包本来就带 126 种语言（tessdata_fast，共 340 MB）。现在是 [build.sh:198-202](../publish/exttool/ext-core/build.sh#L198-L202) 在打包时删到只剩 `eng` 和 `osd`。所以只要放宽这份保留清单，不需要新的下载源。
   - 建议保留的 28 种，合计 63 MB（另有 `osd` 10.5 MB）：
     - 系统界面语言（zh / zh-TW / en / es / fr / de / ko / ja / ru）：`chi_sim`、`chi_tra`、`eng`、`spa`、`fra`、`deu`、`kor`、`jpn`、`rus`；
     - 竖排：`chi_sim_vert`、`chi_tra_vert`、`jpn_vert`、`kor_vert`；
     - 其他常用：`por`、`ita`、`nld`、`pol`、`tur`、`ukr`、`ara`、`fas`、`heb`、`hin`、`ben`、`tha`、`vie`、`ind`、`msa`。
   - 同步修改：
     - [paios容器需求.md](../doc/app_service/paios容器需求.md) §6.4 的 tesseract 一行。它现在写着"只留 eng、osd；中文数据包不在 ext-core；主路径是 `ocr_image`；不是 workflow 媒体基线的必需项"，都要改。这一行是扩展包清单的事实来源。
     - [media tool baseline.md](../doc/workflow/media%20tool%20baseline.md) 第 29 行"不在基线内：…tesseract"，要改为在基线内。
     - [smoke/tesseract.sh](../publish/exttool/ext-core/smoke/tesseract.sh)，检查 `--list-langs` 包含全部保留的语言。
   - 上线按扩展包的流程：在 build.sh 本地构建 payload，用 `prepare_ext_tools.py --local` 导入，重新播种（见 AGENTS.md "paios 镜像的本地开发迭代"）。

**成本**：本书 402 页，每页约 0.45 秒，约 3 分钟 CPU；分到 Pdf lane 的 2 个槽，第一次转换大约多 1.5–3 分钟，之后走缓存。

**设计文档要改的地方**：
- §4.6.5 和 §9 的 OCR 一行，以及"workflow 不依赖 tesseract"的说法；
- 工具清单要加 `tesseract`（[tools.rs](../src/kernel/workflow/src/functions/media/tools.rs)），capabilities 的单测要跟着改。

**验收**：
- 本书各页选中的都是 OCR 文本；
- 一组原生 PDF 上，98% 以上的页保留文字层；
- 扫描件有 OCR 文本；
- 中文、日文和中英混排的资料选对了语言（有 `ocr.lang` 记录）；
- 缺语言包时有明确记录。

### M11 goal 关键词翻译成资料的主语言（Q3）

- **现状**：goal 是中文、资料是英文时，词法打分永远是 0，只能退到均匀取页。
- **触发条件**：文本一次送不完，需要检索，而且 goal 没有点名页。
- **做**：
  - 资料的主语言优先用 M10 记在 manifest 里的 `ocr.lang`；没做 OCR 的文档（文字层正常）由下面这次调用一并判断。
  - 发一次带 `json_schema` 的小模型调用，输入是 goal 和资料的抽样（均匀取约 20 页的开头，合计不超过 2,000 字，用 M10 选定的文本），输出 `{doc_language, terms: [...]}`：资料主语言，以及该语言的关键词和常见同义词。
  - goal 和资料是同一种语言时，不发这次调用。
  - 翻译出的关键词和 goal 原有的关键词一起参与打分。
  - goal 原有的关键词已经命中足够多的页时（例如 ≥ 3 页），不发这次调用。
- **模型**：环境变量 `LLM_UNDERSTAND_MEDIA_TERMS_MODEL`，默认 `llm.chat`。调用失败时退回只用原关键词，并在 details 里记录。
- **记录**：`details.retrieval = {doc_language, terms, source: "translated" | "goal"}`。
- **与 M9 的关系**：Agent 自己在目录里 `grep` 时会用原文的语言，不经过这一步；这一步只服务旁路模型的选页。
- **验收**：用中文 goal "这本书里关于烹饪的部分"读本书，`terms` 里有 `cooking` 之类的词，并命中第 70 页（`29.1 COOKING VERBS`）。依赖 M10 先把文字层换成 OCR 文本。

## 3. 实现时要确认的事实

没有待裁决项，Q1–Q7 见 §0.1。下面这些要在实现时查清楚，不需要再拍板：

- `tool_cache_env` 的三种形态里，容器的 instance volume 什么时候会被清空（重建容器、safe mode、卸载），确保与"目录可以随时删除"的前提一致。
- 宿主 runtime 的 `<账户 home>/.opendan/<agent_id>/cache/` 由降权后的账号所有，工具能写入（Node Executor 的账号见 AGENTS.md）。
- tesseract 用组合语言（如 `chi_sim+eng`）的耗时，和单一语言比慢多少；本书只测了 `eng`。
- M10 的阈值，要在 §4 第 5 条的语料上定。
- 保留 28 种语言后，ext-core 各平台 payload 的实际增量（conda-forge 各平台的 tessdata 应该相同，以实测为准）。

## 4. 验收：重跑本例

1. 全部修复项部署后，在 devtest 的 Jarvis 里发"总结一下cyfs:///home/dk10000.pdf 的内容"。
2. 期望：
   - 路径直接可读，输出里说明回退到了 `/home/devtest/dk10000.pdf`；
   - 抽取和渲染命中缓存（本书的缓存 2026-12-07 前有效），只有 OCR 和 build_catalog 要重跑，期间有进度输出；
   - 目录展开到 `media-catalog/<obj_id>/`，Agent 能直接 `grep` OCR 文本，并能对其中某张页图调 `llm_understand_media`；
   - 模型拿到分布全书的 16 张页图和约 56 页文本，中文 goal 能命中英文页；
   - Jarvis 不自己跑 pdftoppm / pdftotext。
3. 追问"第 14、50、70 页讲什么"：模型看到的正是这三页的页图和文本。
4. 删掉 `media-catalog/` 后，用旧路径追问某一页：目录自动重建，然后正常回答。
5. 第一次转换的进度展示，要换一份没转过的长 PDF 来测，因为本书已经在缓存里。
6. M10 的阈值和选语言语料：本书 403 页、几份原生 PDF（中英文各至少一份）、一份没有文字层的中文扫描件、一份中英混排的资料。

## 5. 实施记录（第四轮，2026-11-08，未提交）

### 5.1 落点

| 项 | 落点 |
| --- | --- |
| M1 | `zone_file.rs` 已随 `18218399d` 提交；部署见 §5.4 |
| M2 | `pipeline.rs` `normalize_text`：行首行尾空白去掉、行内两个以上空白合成两个空格、连续空行合成一行。全书正好 430,473 字，与 §2 的实测一致（行首空白要去掉，保留两个空格是 460,770） |
| M3 | `plan_pages` 重写：点名页 → 命中页（最多剩余预算一半、每页八分之一）→ 其余页 `spread_fitting` 均匀取，每页截到 `max(剩余 / 份数, 1500)`；`coverage.text_truncated`、`text_ocr`。词法分加了 IDF，出现在一半以上页面的词不计分 |
| M4 | 页图：点名 → 命中（一半）→ 只有页图的页 → 在已选文本的页里均匀取 → 在所有页里均匀取 |
| M5 | `render_index`：放不下每页时均匀列约 90 页（每行 40 字），其余按类别列范围；类别区分 `text` / `ocr` |
| M6 | `goal_pages` 认列表和 `p14`、`pp.`、`页14`；`--pages`（CLI）/ `pages`（工具参数），与 goal 取并集 |
| M7 | `Watcher`（`run_pipeline` 等待时每 5 秒看一次 run graph）：首窗口 probe 报出页数且有转换步骤在执行时输出 `converting N pages in K runs (…)`；每分钟一行当前步骤；窗口结束 `pages a-b ready (…)` / `from cache`；`reading …`、`catalog written to …`。只在命令入口开（`enable_progress`），stdout 不变 |
| M8 | `USAGE` 加"Long documents"一节；覆盖说明给出目录路径、`--pages` 和单页图的写法 |
| M9 | 新 `catalog.rs`；`tool_cache_env` 加 `LLM_UNDERSTAND_MEDIA_CATALOG_DIR`；`publish/aios/entrypoint.sh` 导出 `/opt/buckyos/instance/media-catalog`；`run_catalog` 处理目录与页图输入 |
| M10 | workflow 新函数 `media.ocr_pages`（`functions/media/ocr.rs`），两条定义加步骤与 `ocr_lang` 输入；`MediaEntry.ocr_text_obj_id` / `text_quality`、`MediaManifest.ocr`；`tools.rs` 加 tesseract 与语言表探测；ext-core 保留 28 种语言加 `osd`，两个 smoke 同步 |
| M11 | 新 `terms.rs`：`need_translation` 判断要不要调用，`search_terms` 调 `LLM_UNDERSTAND_MEDIA_TERMS_MODEL`（默认 `llm.chat`），`details.retrieval` |

文档：设计文档 §4.1、§4.5.2–§4.5.3、§4.6.1–§4.6.2、§4.6.5（OCR 一段）、§4.7（读目录、Agent 侧目录、等待时的输出）、§9；`path_usage.md`、`11_env_contract.md`、`media tool baseline.md`、`paios容器需求.md` §6.4。`ui_core.md` 不改（"在 goal 里点名页码"仍然成立，`--pages` 与目录由工具自己说明，与 M8 一致）。

### 5.2 与本文的偏离

1. **OCR 的触发不只看干净词比例。** 按本文的口径（只由字母组成、可带一个首尾标点，CJK 标点也算分隔）实测，dk10000 的文字层是 42%–60%，原生中文 / 中英混排是 49%–76%，两者重叠；一致率分得很开（dk10000 21%–53%，原生 64 页全部 ≥ 77%）。所以改为按窗口抽样：干净词 < 60% 的页是可疑页，先识别其中 4 页，一致率中位数 < 70% 判这个窗口的文字层乱码，整个窗口都识别；否则只有抽到的页带 OCR。没有文字层的页总是识别。阈值起初定 60%，DV 里第 3 个窗口唯一的抽样页（索引页 401）一致率 61% 被判成"不乱码"，词多的索引页在 61%–65%，所以提到 70%（原生页最低 77%）；`ocr_pages` 为此有了自己的版本号（`impl-3+ocr-2`），不动全局版本，抽取和渲染的缓存不受影响。
2. **选用规则去掉"干净词高 0.2"。** 用 `一致率 < 70% 且（OCR 干净词 ≥ 文字层 或 OCR 置信度 ≥ 60）`。按本文的规则，dk10000 的 36 个可疑页只有 14 页会换成 OCR；第一版用 `< 60% 且干净词不低于文字层`，DV 里仍有 10 页留在文字层：索引页一致率 61%–65%，满页数字的页（第 356 页基数词）OCR 干净词反而少。文本规则版本因此是 2。
3. **选语言不靠 OSD。** OSD 在语料上错得很多（德文 → 西里尔、韩文 → 天城文、中文扫描件 → 拉丁、竖排中文 → 韩文）。改成用 `eng`、`chi_sim`、`jpn`、`kor`、`rus` 各识别抽样页，比加权平均置信度（每份语料正确的模型都明显最高）；都低于 60 才问 OSD（阿拉伯文、希伯来文上 OSD 判断准确）。
4. **竖排看版面几何。** 横排模型读竖排页置信度仍有 86，但读出来是跨列的乱序；`jpn_vert` 必须配 `--psm 5`。判据是单字框"下方最近邻间距 < 2 × 右侧最近邻间距"（竖排两者约 0.6，横排下方是右侧的 5 倍以上）。
5. **tesseract 是软依赖。** 缺 tesseract 时 `ocr_pages` 输出 `tools_missing`、各页保留文字层、结果不缓存，PDF 转换不因此失败；所以它不在 registration 的 `cmd/*` requirements 里，基线文档按"可选依赖"列出。拉丁文字缺专门的语言包时用 `eng`（`basis.wanted` 记下），其余文字系统缺包按本文记 `lang_unavailable`。
6. **后续窗口沿用第一个窗口的语言**（`ocr_lang` 由 agent 传入），不在每个窗口重新选。
7. **点名页时只送点名页**（文本和页图，其余页只在索引里）。§4 第 3 条要求"模型看到的正是这三页"；第一版把剩余预算继续均匀铺满全书，DV 里模型拿到 102 页、回答在讲 16 页。
8. **目录与模型分析并行写入**，`catalog written to` 在 `reading …` 之后出现。Agent 改过的页图（mtime 晚于 `.complete`）按普通图片处理，便于原地裁剪。
9. `LONG_DOC_PAGES` 的代码默认值原来是 30，与设计文档和本文的 20 不符，已改成 20；设计文档里 `MAX_PAGES` 的默认值改成代码里的 2000。
11. **（本文没有的一项）借给命令的 token 至少还剩 10 分钟。** 第二次容器端到端（120 页扫描件）转换跑完后读 manifest 失败：`named store authentication: Invalid token: refresh token is missing`。原因是命令拿到的是 Agent 当前的 session token（Verify Hub 签发，15 分钟），Agent 只在剩 30 秒时才续，命令自己没有 refresh token 续不了，所以命令能用的时间是 30 秒到 15 分钟之间的随机值；事故那次 6.5 分钟没撞上是碰巧。改法：buckyos-api runtime 加 `renew_token_ahead(margin)`（`renew_token_from_verify_hub` 仍是 30 秒），`agent_tool::runtime_context::refresh_appclient_session_env` 用 600 秒，即给命令注入 token 前剩不到 10 分钟就先续。容器日志里续期间隔从约 14.5 分钟变成有命令时 5 分钟。超过 10 分钟的单次调用仍可能撞上；彻底的办法是命令经 `session.service_token` 自己换新 token（runtime 需要一个替换 session token 的入口），没做。
12. ext-core 的版本号没动（仍是 1.1.0）。payload 内容变了，正式发布前要升版本（例如 1.2.0）再走发布流程；本机用 `prepare_ext_tools.py --local` 播种。

### 5.3 §3 的答案

- 容器的 instance volume 是 `buckyos-instance-<runtime_key>`：重建容器复用，safe mode 用一次性卷（`-safe-<时间戳>`），node_daemon 的卸载路径不删它。目录只靠工具自己的总量上限清理，与"随时可删"的前提一致。
- 节点 runtime 的 `<账户 home>/.opendan/<agent_id>/cache/` 与 `DENO_DIR` 同一处，命令以降权后的账号执行，写得进去。
- 组合语言的耗时（OMP_THREAD_LIMIT=1）：`eng` 约 0.4 秒一页，`chi_sim+eng` 1.5–2.5 秒一页；`OMP_THREAD_LIMIT=1` 比默认的 OpenMP 快（0.40 秒 vs 0.51 秒），所以单进程一线程、进程内最多 4 个并行。
- 阈值在 30 份语料上定（脚本与语料在本轮 scratchpad，会被清理；workflow 里留了 `ocr_pages_on_a_corpus` 这个 ignored 用例，`MEDIA_OCR_CORPUS=<目录>` 指向 `pNNN.png` / `pNNN.txt` 即可重跑）：dk10000 46 页、PVE 管理手册 40 页、中文 PRD 12 页、中英混排设计文档 12 页、中文与中英混排扫描件、12 种语言的原生与扫描页、竖排中日文各一页。结果：语言全部选对；原生 64 页 100% 保留文字层；dk10000 有字的 44 页全部改用 OCR；扫描件都有 OCR 文本。
- linux-x86_64 payload 从 1.2G 到 1.3G，tar 从 421 MiB 到 450 MiB；tessdata 71 MB。其他平台没构建（darwin 只能在 Mac 上建）。

### 5.4 DV（2026-11-08，本机 Zone，devtest）

部署：`prepare_ext_tools.py --local` 播种新 payload（删掉 `tools/installs/ext-core.json` 后重启重装，tesseract 29 个语言包），`buckyos-build.py --skip-web`、`./build_aios --local-test`、`start.py`。workflow 的 `get_capabilities` 报出 tesseract 5.5.3 与全部 `ocr_langs`。

| # | 做法 | 结果 |
| --- | --- | --- |
| A | 宿主 helper（devtest），`dk10000.pdf`，goal "总结一下这本书的内容" | stderr：`converting 403 pages in 3 runs (…)`、`pages 1-200 ready (28s)`、`pages 201-400 ready (41s)`、`pages 401-403 ready (5s)`、`reading 16 page images and 65 pages of text`、`catalog written to …`。抽取、渲染来自缓存，OCR 新跑；`manifest.ocr = {lang: eng, garbled_layer: true}`。文本 65 页、每隔约 7 页分布全书，页图 16 张分布全书（事故时是 7 页、7 张）；3 个索引页被截断。总结准确（10,000 词、180 个单元、七大板块） |
| A′ | 同上，阈值改为 70% 后 | 400 页用 OCR，留在文字层的只有第 5 页（一致率 90%，文字层本来就对）和空白的第 403 页 |
| B | `grep -il colosseum pages/*.txt` | 第 101 页（和索引页 377） |
| C | `llm_understand_media <目录>/pages/0101.png --goal "列出这一页的建筑"` | `media_decision.kind = catalog`，`media_source = {catalog, page: 101, dir}`，没有冻结上传；14 处建筑，Observed |
| D | `llm_understand_media <目录> --goal "第14、50、70页讲什么"` | 第一版把剩余预算铺满全书（102 页文本），改后模型恰好拿到 14、50、70 三页的文本和页图（§5.2 第 7 条） |
| E | goal "这本书里关于烹饪的部分" | `retrieval.source = translated`，词里有 `cooking`、`bake`、`kitchen`…；第 70 页文本和页图都送到；答案定位到"第 29 单元 Cooking，PDF 第 70–71 页"，Observed |
| F | 删掉整个目录，用旧页图路径问第 70 页 | `the catalog … is gone; rebuilding it from the conversion cache`，三个窗口全部 `from cache`（step source 全是 cache，capabilities 除外），目录重建后正常回答 |
| H | 首次转换：PVE 管理手册 685 页（原生） | `converting 685 pages in 4 runs`，四个窗口 50s / 53s / 53s / 23s；只识别了 15 页抽样页，一致率 93%–100%，685 页全部保留文字层；英文 goal 不调翻译 |
| G1 | 容器 Jarvis："总结一下cyfs:///home/dk10000.pdf 的内容" | Jarvis 在 UI session 里直接调用；路径被接受，输出末尾有回退到 `cyfs:///home/devtest/dk10000.pdf` 的说明；25 秒后转后台，90 秒出结果；目录在 `/opt/buckyos/instance/media-catalog/`（442 MB）；没有自己跑 pdftoppm / pdftotext，总结正确 |
| G2 | 容器 Jarvis，120 页中英混排扫描件 | 失败于读 manifest（token 过期，§5.2 第 11 条），修复后重新部署 |
| G3 | 容器 Jarvis："开一个工作任务，读完 cyfs:///home/devtest/scan-notes2.pdf 后给我一份内容总结"（60 页扫描件，全新） | 进度：`converting 60 pages in 1 run`、`the first pages: recognizing text in page images (OCR) (1m07s)`、`pages 1-60 ready (1m35s)`。Todo 子会话先看 `--help`，第一次调用期间 task_watch 唤醒 4 次，它都只是等待；结果回来后用 `--pages 1-10` … `51-60` 分 6 批补读（全部 `from cache`），复制了目录的 `index.tsv` 作证据，没有自己跑 pdftoppm / pdftotext / tesseract |

没验到的：

- **task_watch 唤醒时 `<<background_context>>` 里的进度行。** G3 里 Jarvis 把命令的 stderr 重定向进了证据文件（`2> evidence/01-initial.stderr.txt`），任务本身没有新输出，唤醒消息里也就没有这个块；进度行在那个文件里，内容正确。work / plan 的模板都放了 `{{ background_context }}`，`observe.rs` 读的就是任务的 stderr，所以没改 runtime。事故当天的 worklog 里也没有带内容的块（那时 helper 等待期间什么都不输出）。
- 竖排、阿拉伯文等只在语料上测过，没走 Zone。
- darwin / windows 的 ext-core payload 没构建；ext-core 没发布。
- 超过 10 分钟的单次调用（§5.2 第 11 条）。

测试：`cargo test -p workflow`（88 通过，`--ignored` 的 `tesseract_reads_scanned_pages_and_replaces_a_garbled_layer` 通过，`ocr_pages_on_a_corpus` 跑了 30 份语料）、`-p buckyos-api --lib`（392）、`-p agent_tool --lib`（355 通过，3 个 `grep_tool` 失败是本机没有 `rg`）、`-p libopendan --lib`（79）及 `session_runtime`、`tool_session_token`、`runtimes`、`auth_recovery`、`xagent_lib work_sessions_take`、`xagent_cli skills_are`；`cargo check --workspace --tests`。测试文件 `scan-notes.pdf`、`scan-notes2.pdf` 留在 devtest 的 home。
