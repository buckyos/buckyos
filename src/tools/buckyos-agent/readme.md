# BuckyOS Agent

> 待buckyos-web sdk文档后，基于TS 实现

buckyos-agent tools 是用 TypeScript 编写、运行在 Deno 环境下的 BuckyOS 命令行工具。
会默认打入opendan的docker镜像(paios/aios), 是opendan agent runtime为Agent提供的，访问buckyos的基础工具
是用ts编写是方便开放源代码给Agent,Agent可以在此代码基础上，自行升级合组合

## MVP功能需求

1. 通过control_panel的接口发布static-web-app
应为jarvis的权限问题，只能执行到store,需要传一个页面url让用户自己点开，确认上线
发布到哪里的问题？
- 一个通用的publish目录，放在里面的所有文件夹立刻就能得到预览
- 打包成app发布，需要用户手工鉴权

2. aicc命令组

## 表格数据导入 `import_table`

`commands/import_table.ts`：把 CSV / TSV / xlsx / SQLite 文件导入 AI Workspace 成为 TableSource，推断规则与 Desktop
导入向导相同（共享模块 `src/frame/aiworkspace/import-table/`，deno.json 里的 `@table/`；宿主副本与 aios 镜像里随命令
放在 `vendor/frame/aiworkspace/`，由 `buckyos-build.py` 的 `_prepare_agent_tools` 与 `build_aios` 复制）。先
`import_table <file> --dry-run` 看计划（字段、类型、行数、报告），再去掉 `--dry-run` 带 `--workspace` 提交。

## Office 文档导入 `import_office`

`commands/import_office.ts`：把 .docx / .pptx 导入 AI Workspace 成为可编辑内容，转换器与 Desktop 的导入对话框相同
（共享模块 `src/frame/aiworkspace/import-office/`，deno.json 里的 `@office/`；它还引用 import-drawio / import-pdf /
import-table 的部件，四个模块一起由 `_prepare_agent_tools` 与 `build_aios` 复制到 `vendor/frame/aiworkspace/`）。docx →
新的流式页（富文本 Block + 图片 Block，原文件放最后），pptx → 新的自由画布（每张幻灯片一页 + 放映路径）。Deno 没有
canvas、不连 LibreOffice：裁剪保留整图、图表 / EMF 等成占位框或略去，都写进报告。先 `--dry-run` 看计划再提交。

## 向用户提问 `ask_user`

`commands/ask_user.ts`（逻辑在 `lib/ask.ts`）：向 Agent 所服务的用户提一个问题，是一次标准的异步工具调用。

```bash
ask_user "<question>" [--option <id>=<label>]... [--allow-text] [--expires <seconds>]
```

- 不带 `--option` 是文字问题；带 `--option` 是选择题（至少两个，id 与 label 各自不重复、非空），`--allow-text`
  只能和 `--option` 一起用（也可以用文字回答）。`--expires` 默认 10800 秒（3 小时）。
- 发一条 System Notify 的 `input` 通知（msg-center `notify.send`，`level: timely`）。msg-center 为它建一个 Zone
  TaskMgr 任务（`human.input/v1`），回答就是任务结果。stdout 一行 AgentToolResult：`status: "pending"`、`task_id` =
  通知的任务（`t-…`）、`pending_reason: "user_input"`，`detail` 带 `notify_id`、`question`、`options`、
  `expires_at_ms`、`url`。命令不等待；runtime 在预算内等该任务，之后 Agent Session 自己跟进、回答以 `<<task_result>>` 输入回来（xllm 单跑时模型用 `wait_task` / `get_task_state` 跟进）。
- 收件人：会话 token 的 `sub`。Agent 的工具拿到的是其 App 的 verify-hub 会话（`principal_kind: app`），`sub` 就是 App
  实例的 owner（verify-hub 只签发这样的 token，msg-center 也只允许它通知这个用户）；token 读不出用户（如 `agent`
  主体）时报错，不猜。
- 幂等：`idempotency_key = ask:$XLLM_RUN_ID:$XLLM_CALL_ID`（两者由 `shell` 工具注入，缺一个就报错：要作为单独一条
  shell 命令运行）。msg-center 按“调用方 + 键”去重，崩溃后重跑同一调用找回的是已发出的那条通知；它已关闭时直接返回
  终态：已回答为 success（`output` 是选项的 label 和/或文字，`detail.answer` 带 `option/label/text`），
  dismissed / canceled / expired 为 error。
- 退出码：0 已提问（pending）或已回答；1 参数错误或不在 run 的工具调用里；2 发送失败、找不到收件人、被拒绝或撤销；
  4 已过期。

## 兜底页 `reflow`

`commands/reflow.ts`（规则与读写在 `lib/reflow.ts`）：单个网页“只整理这一页”的兜底页，存在用户的重排桶
`user/control-panel/<owner>/reflow` 里（`<site>/fallback/<页面>.html` 和同名的 `.json`），在 `https://reflow.<zone>/` 上显示。协议见
[`doc/opendan/protocol/Agent Browser Protocol.md`](../../../doc/opendan/protocol/Agent%20Browser%20Protocol.md) §5。主人长期阅读的网站不在这里做：
那是个人站的来源，见下一节的 `site`。

```bash
reflow match <url>                    # 站点、页面、已有的兜底页、该显示哪一页
reflow ls                             # 有兜底页的站点
reflow rm <site>                      # 删除这个站点下的全部内容
reflow result put <url> <page.html>   # 把这份 .html 存成这个网址的兜底页
```

- 站点名、规范化网址、页面散列、匹配规则和 Agent Service（Rust）共用一份测试向量：
  `doc/opendan/protocol/fixtures/reflow_vectors.json`，`lib/reflow_test.ts` 用它。`site.json` 里来源的 `match` 用的是同一套匹配规则
  （`rulesMatch`），文章 id 就是这里的页面散列。
- `reflow match` 的 `detail` 是 `{ url, site, page, fallback, view }`，和 Agent Service 的 `browser.reflow_state` 同一形状；`view` 是相对
  `https://reflow.<zone>/` 的路径，没有兜底页时是 `null`。
- 只读权限的 Agent 写不了这个桶：`rm`、`result put` 以 `permission_denied` 失败。
- 退出码：0 完成；1 参数错误；2 服务拒绝或失败；5 本地文件错误（如 `result put` 的文件不存在）。

## 个人重排站 `site`

`commands/site.ts`（数据、流水线在 `lib/site/`，翻译用的模型调用在 `lib/model.ts`）：一个用户一个个人站，数据在用户的重排桶
`user/control-panel/<owner>/reflow` 里（`_data/`、`_app/`、`_src/`、`index.html`），页面在 `https://reflow.<zone>/`。契约是
[`doc/opendan/protocol/Personal Site Protocol.md`](../../../doc/opendan/protocol/Personal%20Site%20Protocol.md)，共用样例在
`doc/opendan/protocol/fixtures/site/`。`site --help` 列出全部子命令、来源包的格式和退出码。

```bash
site init --title 我的阅读 --language zh-CN --topics 国际,科技,体育   # 建 site.json，装默认模板（site-template/）
site try site-sources/news-yahoo-co-jp --limit 3 --no-translate      # 不碰桶：取数、核对、图片，写到本地目录
site try <dir> --fixtures --no-translate                             # 只用包里的 fixtures/，不联网
site source put <dir> --state active                                 # 校验来源包，存成 _src/<id>/s<n>/，写进 site.json
site sync <source> --dir <dir>                                       # 完整同步一次；作业就是它
site preview site-template --data <site try 的输出目录>               # 本机看模板
site status | match <url> | config get|set | source list|get|rm | template put|list|use|get
site upgrade open|close|dismiss|list | events get [--out <file>]|put | supplement put
site reading [--compact] | visits [--days 7]
```

- **来源包**（`source.json`、`extract.ts`、`fixtures/`、`NOTES.md`）由 Agent 写，其余是这里的产品代码。三个样板在 `site-sources/`：
  `news-yahoo-co-jp`（RSS → 文章页内嵌的 `__PRELOADED_STATE__`，评论另取一页）、`news-ycombinator-com`（RSS → 讨论页的评论 +
  外链页面经 `ctx.blocks` 取正文）、`news-web-nhk`（只能取到导语的站：标题、导语、配图，`check.min_chars` 调低）。各自的
  `NOTES.md` 写了怎么取、怎么核对、有什么坑。`extract.ts` 不 import 包外的东西：解析用 `ctx.html / ctx.xml / ctx.feed / ctx.blocks`，
  失败用全局的 `SiteError`。
- **取数环境**（`lib/site/extract.ts`）：统一的 UA、同一主机 1 次/秒、30 秒超时、条件请求（`ETag` / `Last-Modified` 存在作业的
  `state.json`）。`linkedom` 只在 `site` 的代码路径里动态 import（`deno.json` 的 `"linkedom": "npm:linkedom@0.18.12"`；分发时由构建打成
  `vendor/` 里的一个文件），别的命令不加载它。`ctx.browser` 调本 Agent 的 Agent Service `browser.background_run`
  （`lib/agent_service.ts`）；要渲染、要登录的来源的样板和不依赖 Agent Service 的本机跑法在 `test/agent_browser_dv/`。
- **一次同步**（`lib/site/pipeline.ts`，协议 §5.1）：取数 → 核对（`check.ts`，§5.3 一条规则一个用例）→ 图片（`images.ts`）→ 翻译与归类
  （`translate.ts`）→ FeedObject 外壳（`feed.ts`）→ 写文章 → 最后写 `feed/<source>.json` → 每天一次清理。没发布完的留在作业的
  `data/pending.json`，下一次接着做；作为 Goal 作业运行时（有 `OPENDAN_JOB_RUN_ID`）状态放在工作目录，stdout 最后一行是作业的结果行，
  否则放在当前目录的 `.site-job/<source>/`。
- **图片**：`PATH` 上有 `magick`、进程又被允许运行它（`--allow-run=magick`，`deno task site` 和安装态的 launcher 都只放行这一个程序）时，
  缩到长边 1280、去元数据，第一张另出 480 的小图；没装、或不许运行（`NotCapable` / `PermissionDenied`）时一样处理：存原图（至多 2 MiB）。
  `site` 的代码不启动别的程序。取不到的图片（网络错误、超时、4xx / 5xx）等下一次同步再试一次，还取不到就按 `missing` 发布；取到了但不是
  站里收的图片（SVG 图标、ICO、回 200 的错误页……）当场去掉这一块，不重试、不留占位，文章在这一次同步里照常发布；地址以 `.svg` 结尾的
  根本不去取（`ctx.blocks` 也不把它们和 `data:` 图片变成块）；取到了、是图片但存不下的（太大、转换失败）当场按 `missing` 发布。每去掉一张，
  日志里有一行。
- **翻译**：一篇一次 `helper.llm_chat`，模型 `llm.translate`，`json_schema`，`policy: { allow_fallback: false, runtime_failover: false }`。
  译文按原文块的 SHA-256 缓存在 `data/tr/<article>.json`。用量按 `xllm` 的格式追加到 `$OPENDAN_USAGE_LOG`，受 `$OPENDAN_MODEL_BUDGET` 和
  `site.json` 的 `limits.translate_tokens_per_day` 约束；被拒绝时不发布未翻译的文章，`feed` 的 `status.pending` 说明原因
  （`translate_budget` / `translate_config`；译文长度对不上、模型临时出错是 `translate_failed`，连续三次同步都失败就放弃这一篇），退出码仍是 0。
- **桶**：`lib/site/store.ts` 只用 websdk `BucketClient` 的一小部分（测试用内存里的桶）。`site.json` 带 `If-Match` 写，冲突时重读再改，三次不成
  报错。不变的对象（图片、模板文件）先 `PUT`（`If-None-Match: *`）再用 `bucket.put` 补上 `Cache-Control: immutable`：数据面的 `putBytes`
  没有带这个头的参数。
- **保留与清理**：作业每天按自己的 `data/index.json` 删过保留期的文章、Markdown 和不再被它引用的图片。另有桶自己的过期时间兜底
  （`_data/a/`、`_data/md/`、`_data/img/` 写入时带 `fetched_at` 加 181 天；图片每次被再次引用时顺延）：来源被移除、换了维护者之后留下的
  对象靠它清掉。**已知的限制**：图片按内容命名，两个来源收到同一张图时共用一个对象，而清理只看本作业的索引，先过期的那个来源会把它删掉，
  另一个来源的文章里这张图就没有了（转载同一个页面的来源之间会遇到，比如 Hacker News 的外链和那个站自己的来源）。
- **阅读汇总**（`lib/site/reading.ts`）：`site reading --compact` 把 `_read/` 的上报汇成 `reading.json`；`article` 和 `opened_at` 相同的几条算
  一次打开，停留相加。作业结果行的 `metrics` 是 `{ records, opens_7d, dwell_ms_7d, articles_7d, profile }`，`profile` 是最近 7 天按停留排的
  前 3 个标签和前 3 个来源（`tags:a,b,c;sources:x,y,z`，停留相同按名字排），同样的阅读得到同样的一行，检查脚本拿它比较。
- **事件**：`site events put <file>` 校验后把每个条目的 `title`、`source`、`at` 从文章里取来填上；条目指的文章不在桶里就报错并指明是哪一条。
  `site events get --out <file>` 把现有的 `events.json` 写成 `put` 收的文件（还没有时是空的一份）。
- **作业结果行**：`site sync` 的 `metrics` 是 `{ listed, new, published, rejected, images, tokens, ms, pending, pending_reason }`。宿主不保留退出码
  为 0 的运行的 `kind`，所以“翻译被拒”（`kind: "budget"`）到不了检查脚本，它看 `pending`（取到了还没发布的篇数）和 `pending_reason`
  （`null` 或 `feed` 的 `pending.reason`）。汇总作业 `site reading --compact` 的程序目录不能是空的（`goal job put` 不收空目录），放一份说明即可。
- **浏览记录**：`site visits [--days 7]`（`lib/site/visits.ts`）经 `lib/agent_service.ts` 调本 Agent 的 `browser.visits`，按主机和按页面列出主人在
  Agent 浏览器里看过什么；记录被主人关掉时写明；不在 Agent 自己的 runtime 上（没有 `$OPENDAN_BROWSER_RPC`）或服务连不上时是 `no_browser`。
- **换维护者**：来源包用 `site source get` 取回，改过的模板用 `site template get <version> <dir>` 取回（连同 `index.html`，取到的目录 `template put` 照收）。
  升级请求做完了是 `site upgrade close`，主人说不做是 `site upgrade dismiss`。
- 只读权限的 Agent 写不了这个桶：写操作以 `permission_denied` 失败。
- 测试：`deno test -A lib/site/ lib/model_test.ts`（内存桶、假的模型、三个样板包对着自己的 fixtures）。要渲染、要登录的
  来源：`node test/agent_browser_dv/sources_check.mjs`（测试站点 + 代替 Agent Service 的 `browser_stub.mjs`，要本机有 ext-core 的 Chromium）。

## AICC Agent CLI Tools

实现见 `aicc-tool.ts` + `commands/*.ts`，需求文档：[`doc/aicc/aicc_agent_cli_tools.md`](../../../doc/aicc/aicc_agent_cli_tools.md)。

### 给 Agent: 为什么源码长这样

Agent 应当能直接读 `commands/*.ts` 学到怎么调 AICC，然后自己写新的 deno
工具（这比把若干 CLI 用管道串起来更可控）。所以每个命令文件刻意写成一份
**从上到下的配方**，固定 6 步：

```
1) 解析 argv → 位置参数 + 选项
2) 按 SDK 类型构造 canonical method request
3) initRuntime() 拿到登录后的 BuckyOS 会话
4) 逻辑模型先 route.resolve，再调用 typed inference；文生图直接调用 Helper
5) 把产物交到本 Agent 的 artifacts 桶（命令返回 obj://bucket/...；给了输出路径才另存本地），或读取文本
6) emit() 一行 AgentToolResult JSON 给上游
```

**`gen_image.ts` 是带详细注释的范本**，其他命令文件用同样的骨架。改变化点
只在 (1) (2) (5)；(3)(4)(6) 是机械的。

`lib/` 是这套工具的"标准库"，Agent 写自己的新命令时同样从这里拿原件：

| 文件 | 用途 |
|---|---|
| `lib/runtime.ts` | `initRuntime()` — 登录 BuckyOS |
| `lib/aicc.ts` | `callAicc()` canonical 路由与 typed inference + task-manager 轮询；`textToImage()`、`commonPolicyOptions()`、`describeFailure()` |
| `lib/io.ts` | `resolveInputResource()` 把字符串映射成 ResourceRef；`pickArtifact()`；`deliverArtifact()` 把产物交到 artifacts 桶（可选另存本地）；`saveResourceToPath()` / `saveBucketObjectToPath()` 落盘；`suffixPathByMime()` |
| `lib/bucket.ts` | 本 Agent 的桶：`uploadNamed()`（字节直传 `artifacts` 或 `scratch`，≤32 MiB 走 `PUT /bucket/v1`，更大走 NDM 上传 + `bind`）、`copyToArtifacts()`（`bucket.copy`）、`readBucketObject()`、`obj://bucket/...` 解析 |
| `lib/cli.ts` | `parseArgvOrExit()`、`requireString/flagInt/flagFloat/flagBool`、`takeRepeatedFlags()`（可重复的 `--x v`）、`COMMON_OPTIONS_HELP`、`bailArgError()` |
| `lib/ask.ts` | `ask_user` 的部件：`askUser()` 发 `input` 通知并把通知状态映射成 AgentToolResult；`askRecipient()` 从会话 token 取收件人 |
| `lib/model.ts` | 程序里的一步模型调用：`aiccModel()`（`helper.llm_chat`，`json_schema`，不换模型）、`MeteredModel`（记 `$OPENDAN_USAGE_LOG`、守 `$OPENDAN_MODEL_BUDGET`） |
| `lib/agent_service.ts` | `callAgentService()`：调本 Agent 的 Agent Service（`$OPENDAN_BROWSER_RPC`） |
| `lib/result.ts` | AgentToolResult builders + `emit/emitAndExit`、各种 `bail*` 错误收敛、退出码常量 |
| `lib/types.ts` | Agent 输出类型，以及从发布版 `buckyos` SDK 派生的 AICC response / `ResourceRef` 视图 |

写一个新命令的最短模板：复制 `gen_image.ts`，把 `TOOL` / `METHOD` /
`capability` / `HELP` / step 2 的 typed request builder / step 5 的产物处理换成
你要的就行。


### 调用方式

```bash
# 单一 binary + 子命令
deno run --allow-net --allow-read --allow-write --allow-env --unsafely-ignore-certificate-errors aicc-tool.ts <command> [args...]

# 或 deno task
deno task gen_image "A matte black desk lamp" result.png --quality high

# 也可以把 aicc-tool 当 binary 安装后，给每个 command 建 symlink
# (file 名匹配 commands/*.ts 文件名时，自动作为子命令)：
ln -s aicc-tool gen_image
gen_image "prompt" out.png
```

AIOS 镜像中的只读安装目录是 `/opt/buckyos/bin/opendan/buckyos-agent/`。镜像不带 SDK：
安装态 `deno.json` 的 `buckyos` 指向 `/opt/buckyos/libexec/buckyos-tool/dist/node.mjs`，即
node-daemon 从宿主 `$BUCKYOS_ROOT/libexec/buckyos-tool/` 只读挂入 Agent 容器的 SDK，
与容器内 `buckyos` 命令用的是同一版（随宿主系统升级）。该挂载缺失时 `aicc-tool` 及各命令直接报错退出。
仓库内开发态 `deno.json` 指向构建产物 `src/rootfs/libexec/buckyos-tool/dist/node.mjs`。
在 Agent 容器内可以直接运行 task：

```bash
cd /opt/buckyos/bin/opendan/buckyos-agent
deno task gen_image --help
```

Agent 组合新工具时，可以读取该目录下的 `commands/*.ts` 和 `lib/*.ts`，SDK 类型见
`/opt/buckyos/libexec/buckyos-tool/dist/*.d.ts`，但应把派生脚本写入 Agent 或当前 Session
自己的 `tools/` 目录，不要修改只读安装副本。运行派生脚本时显式复用安装态配置：

```bash
deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json \
  --allow-net --allow-read --allow-write --allow-env \
  --unsafely-ignore-certificate-errors \
  ./tools/my-aicc-tool.ts
```

### 命令清单

`gen_image` / `edit_image` / `inpaint_image` / `upscale_image` / `remove_bg`
`ocr_image` / `caption_image` / `detect_image` / `segment_image`
`text_to_speech` / `speech_to_text` / `gen_music` / `enhance_audio`
`gen_video` / `edit_video` / `extend_video` / `upscale_video`
`ai_provider list|health` / `ai_quota` / `materialize_resource`

每个命令的 `--help` 列出参数。

`gen_video` 对应统一的 `video.generate`（协议见 `doc/aicc/video_generation.md`）：文本、首尾帧
（`--first-frame/--last-frame`）、参考图/视频/音频（可重复的 `--ref-image/--ref-video/--ref-audio <id>=<素材>`，
`--clip`、`--with-audio` 细化参考片段）都是同一请求的 `conditions`，提示词用 `@{id}` 引用素材；输出规格
（`--duration/--aspect/--short-edge|--size/--fps/--audio/--format`）进入 `output`。逻辑模型走
`helper.video_generate`，由 AICC 按实测素材选模；模型不支持的条件或规格直接报错，不会被丢弃。
`edit_video` 对应 `video.edit`。视频结果解析 `videos[]`，多于一个视频时报错而不是截断，
`detail.video` 带 Provider 声明值、AICC 实测值和偏差。

### 输出协议

CLI 始终将 `AgentToolResult` JSON 写到 stdout（与 `src/frame/agent_tool` 的协议一致，`agent_tool_protocol: "1"`），stderr 用于进度日志。退出码遵循 doc §2.5：

异步 AICC 任务等待期间，CLI 每 5 秒向 stderr 写一行以
`__BUCKYOS_AGENT_PROGRESS__` 开头的结构化心跳。OpenDAN 会消费该心跳以更新会话状态并续租 `shell` 的空闲超时；它不属于最终 `AgentToolResult`。

- `0` 成功
- `1` 参数错误
- `2` AICC 调用失败
- `3` provider/route 失败
- `4` 等待超时
- `5` 本地 I/O 失败

### 输入/输出处理

- 输入文件：本地路径 → base64；`http(s)://...` → URL；`named_object:<obj_id>` → NamedObject；`obj://bucket/<bucket_id>/<key>` → 它绑定的 NamedObject
- 输出 artifact：一律进本 Agent 的 `artifacts` 桶（`user/<agent app>/<owner>/artifacts`，`documents` 档，归用户、备份），结果 `detail.files[].url` 是 `obj://bucket/...`，直接列进报告的 artifacts。AICC 已存进它的 `outputs` 桶（`user/aicc/<owner>/outputs`，7 天）的结果用 `bucket.copy` 复制过来，不再传字节；URL/base64/raw PCM 读出字节后上传一次
- 中间文件：`uploadNamed(runtime, bytes, name, { bucket: "scratch" })` 进 `scratch` 桶（`temp`，7 天）
- 资源落盘：`materialize_resource obj://bucket/... | named_object:<obj_id> <output_path>`（本地工具需要路径时的显式降级）；目标文件已存在时拒绝覆盖

### 环境变量

在 OpenDAN 中，工具使用运行时注入的受限 AppClient session token，并通过宿主机 NodeGateway 访问系统服务。本地独立运行时仍需私钥（默认搜索路径 `~/.buckyos`、`/opt/buckyos/etc` 等）。容器内 `buckyos` 命令另读 node-daemon 注入的 `BUCKYOS_TOOL_ENDPOINT`（见 `doc/arch/11_env_contract.md`），本工具不读该变量。

- `BUCKYOS_APP_ID`（默认 `buckyos-agent`，兼容 `BUCKYOS_TEST_APP_ID`）
- `BUCKYOS_APPCLIENT_SESSION_TOKEN`：OpenDAN 注入的 AppClient session token；设置后启用容器内访问方式
- `BUCKYOS_HOST_GATEWAY`：容器访问宿主机的地址（Docker 默认 `host.docker.internal`，Windows WSLC 由 node-daemon 注入 `host.wslc.internal`）
- `BUCKYOS_NODE_GATEWAY_PORT`：NodeGateway 端口（默认 `3180`）
- `BUCKYOS_AICC_TOOL_ROOT`：仅供 AIOS launcher 调试覆盖安装目录（默认 `/opt/buckyos/bin/opendan/buckyos-agent`）
- `BUCKYOS_OWNER_USER_ID`：当前 Zone owner 用户 ID（默认 `devtest`）
- `BUCKYOS_ZONE_HOST`（默认 `test.buckyos.io`，兼容 `BUCKYOS_TEST_ZONE_HOST`）
- `BUCKYOS_APP_CLIENT_DIR` / `BUCKYOS_TEST_APP_CLIENT_DIR`：额外私钥搜索目录
- `AICC_DEFAULT_PROFILE`：`balanced|cheap|fast|quality`（默认 `balanced`）
- `AICC_DEFAULT_TIMEOUT`：等待 task 完成的毫秒上限（默认 `900000`）
