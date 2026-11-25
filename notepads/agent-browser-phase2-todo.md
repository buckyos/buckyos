# Agent 浏览器二期：实施 TODO

日期：2026-11-24，11-25 更新 · 源码基线 `1704812f8`（Goal 相关的事实以 `5b197b11e` 为准，S0-0）· 状态：**代码已全部实施（2026-11-25），未提交、未部署；真实 Zone 只验了“能读”这一段（§11 最后几行），§7 的验收一条都没有跑**

**2026-11-24：WG（Work 开发、Goal 运维）、GJ（Goal 作业）两组改由 [Goal Session 改进 TODO](opendan-goal-session-improve-todo.md)（下称“改进单”）完成，代码已提交（`5b197b11e`，2026-11-25），实施记录在改进单 §11；真实 Zone 的 V4、V10 没有跑。** 这两组的设计仍写在本文 §4.2–§4.4（和实现不同的地方在那里标注），契约以 [Agent State Protocol](<../doc/opendan/protocol/Agent State Protocol.md>) §9.4、§9.5、§9.9 为准。Q1、Q2、Q4、Q5、Q15、Q19、Q23 的答复在改进单的 Q16–Q19 和 §1.3，不用在这里再答。

**开发顺序（owner 2026-11-24）：本文排在改进单之后。** 改进单的代码已提交，还差真实 Zone 的验收。入口条件和可以先做的部分见 §6。

需求来源：[二期需求](<../product/BuckyOS Agent浏览器 二期需求.md>)（下称 PRD）、架构设计 [§7.10](<../doc/opendan/OpenDAN Agent Session架构设计.md>)（Work 开发、Goal 运维），以及 2026-11-24 的三轮讨论（§2）。一期的实现与契约见 [一期 TODO](agent-runtime-browser-todo.md) 和 [Agent Browser Protocol](<../doc/opendan/protocol/Agent Browser Protocol.md>)。

## 0. 怎么用这份文档

**给 owner。** §1 的问题都已答复，怎么落实见 §1.4。现在没有等你回答的问题。实施中出现的新问题写进 §11 再问你。

**给 CodeAgent。**

- §1 已定案：有答复的按答复和 §1.4 的落实，写着“未单独答复”的按建议。“另一种做法”都没有采纳，不要再加回来。
- §2 是已裁决的约束，不重新讨论。和代码现实冲突时停下来写进 §11，问 owner。
- §3 是查过、测过的事实，不用重查。标“没测”的才需要验证。行号以基线为准，动手前核对。
- §4 是设计。名字和字段可以调整，调整后回写本文和协议文档。**跨包契约先写协议文档再写代码**（S0-4）。
- §5 的任务做完一条，在 §11 记一行：改了什么、跑了什么、结果、没验什么。
- 遵守 [AGENTS.md](../AGENTS.md)：不做兼容实现；改动范围小；新依赖要 owner 确认（已同意的只有 Q22 的 `linkedom`）；手册和代码同一变更内更新。
- **机制越少越好**（owner 在 [Plan / work_simple TODO](jarvis-plan-work-simple-todo.md) D6、D9 的原则）。能用已有的命令、事件、提示词做到的，不加新机制。
- 不要提交，除非 owner 要求。

## 1. owner 决策（2026-11-24、11-25 已答复）

### 1.1 影响架构

**Q1 例行脚本放在哪里跑？**

- 现状：没有“按时间表跑、不推理、能做事”的地方。Goal 检查脚本按契约只观察（30 秒，不给写和 run）。AIWS 的同步任务（`agent-session sync`）能按时间表跑，但是沙箱：只有声明的域名、没有令牌、不能写文件、不能调模型、120 秒、最短 15 分钟、失败通知 owner。
- **建议：新增 Goal 作业（`agent-session goal job`）**。作业属于一个 Goal，是安装在 Goal 目录里的一份程序，宿主按时间表在 Agent 的私有 runtime 里以 Agent 的身份运行，不推理。Goal 休眠或归档，作业跟着停。检查脚本维持只读，它读作业的运行记录来决定要不要推理。
- 这和我在讨论里的建议（扩展同步任务）不同，改的理由有两条。一是你定了运维是通用规则，别的活工作区（备份、监控、报表）也需要跑脚本，AIWS 的数据源沙箱承载不了。二是一期你定过“重排页的代码都是我们生产的，安全边界可看作一致”，作业的代码同样是 Agent 写的，Agent 调试它时本来就不在沙箱里。
- 另一种做法：扩展同步任务（加模型调用、多输出、通配域名、长超时，落点已查清，见 §3.4）。隔离更好，但只服务于抓取，要开的口子多。
- 影响：GJ 全组。
- 答复：见改进单 Q18（按建议，Goal 作业；已实施）。

**Q2 Work 凭什么能建立和改写 Goal？**

- 现状：建 Goal 只有 owner 和 UI 会话能做；UI 只能修订自己建的 Goal，`/new` 之后新的 UI 会话就不能改了；Work 一律拒绝；Goal 不能改自己的定义。另外有一个已有的缺口：会话身份是自报的（`OPENDAN_SESSION_ID` 或 kRPC 的 `session` 参数），Agent 的 shell 去掉它就被当成 owner。所以这套权限现在只防误用，不防故意绕过。
- **建议：规则放宽到够用，不加授权字段。** 直接由 UI 会话创建、且不属于某个 Goal 的 Work，对 Goal 的权限等同它的父 UI。对端是 owner 的 UI 会话可以修订这个 Agent 的任何 Goal。Plan 的 Todo、Goal 派的 Work 不行。身份自报的缺口这一期不补，按“先能力再安全”记为已知风险。
- 另一种做法：创建 Work 时带结构化授权（指明哪个 Goal、能不能新建），并堵住身份自报。更严，要改会话协议、登记表和 kRPC 鉴权。
- 影响：WG1、WG2。
- 答复：见改进单 Q17（按建议，不加授权字段；已实施）。

**Q3 个人站用什么形式？**

- **建议：数据和模板分开，放在 `reflow.<zone>` 的根。** 文章、列表、状态是 JSON 和图片，存在现有的重排桶（`user/control-panel/<owner>/reflow`）。页面是一套模板（HTML/CSS/JS），在手机上读 JSON 画出来。改版式只换模板，不用重抓。根首页不需要改服务端（`/` 已经映射到桶根的 `index.html`）。文章正文存成结构化的块，按文本渲染，不存原站的 HTML。
- 另一种做法一：脚本直接生成静态 HTML。不依赖页面脚本，但改版式要重新生成全部文章，存储也大。
- 另一种做法二：做成一个静态 App。登录跳转是现成的，但每次改模板要升版本、打包、安装，还多一个 App 身份。
- 影响：ST、TP、RH 全组。
- 答复：按建议，可以进一步考虑抓回来的文章封装成HomeStation的FeedObject的结构?

**Q4 “请求用户开 Work 做版本升级”走什么通路？**

- 现状：Goal 用 `questions` 提问，回答只会唤醒 Goal。通知没有“操作”按钮，只有回答和忽略。
- **建议：用现有的通知和对话，不加新机制。** Goal 把升级建议写成工作区里的一份文件（问题、影响、建议、任务书草稿），用 `notify send --key --url` 发一条通知。链接打开 Agent 浏览器的升级说明，主人点“升级”后，App 往浏览器会话发一条消息，意图分析据此建 Work。这条消息是主人自己发的，所以 Work 自然是“主人发起的”。没有专用 App 的 Goal，主人在对话里回一句“升级吧”也走同一条路。
- 另一种做法：给 Goal 的结果加一个结构化的升级请求，通知带“升级 / 暂不”选项，点“升级”由宿主直接建 Work。不经过模型，更确定，但要改 Goal 结果、通知和宿主。
- 影响：WG5、AP8。
- 答复：见改进单 Q19（按建议，通知加对话；通用约定已实施，站点专有的部分在 AP8）。

**Q5 运维和升级的分界，这样写行不行？**

- **建议：** 用户看到的东西、系统的权限和成本都不变，是运维；其余是升级。按这条，换选择器、重试、把频率调低是运维；新增抓取域名、加来源、重做版式、改成要登录或要开浏览器的取法、把频率调高一倍以上，是升级。
- 影响：WG4、MN2 的提示词，验收 V5、V6。
- 答复：见改进单 Q19（按建议；已实施）。

**Q6 用户有多个 Agent 时，站归谁？**

- **建议：** 一个用户一个个人站，数据、模板和来源程序都归用户（存在用户的桶里）。同一时刻由一个 Agent 的一个 Goal 维护，记在站点定义的 `maintainer` 里。换 Agent 是一次升级：新 Agent 从桶里取回来源程序，建立自己的 Goal，旧 Goal 归档。App 里换 Agent 看到的是同一个站。
- 影响：ST1、AP2、MN1。
- 答复：按建议

### 1.2 产品与参数

没有单独答复的，按建议定案。

**Q7 脚本里的模型步骤用哪个模型，用量算在谁头上？** 建议走现有路由 `llm.translate`，请求里关掉自动换模型（和 [显式 fallback](aicc-explicit-fallback-todo.md) 的原则一致），Zone 里没有可用模型时作为配置问题报给主人。用量记在 Goal 名下，计入 Goal 的周期预算和 Agent 后台总量，另设每日上限（默认 100 万 token）。影响 GJ5、ST4。
答复：（未单独答复，按建议）

**Q8 翻译额度用完或翻译一直失败，新文章怎么办？** 建议不发布，来源页说明“今日翻译额度已用完”，第二天继续。另一种做法是先发原文并标“未翻译”。影响 ST4、ST6。
答复：（未单独答复，按建议）

**Q9 频率、数量、保留和空间。** 建议：新闻站 30 分钟检查一次，博客 6 小时，论坛 1 小时；首次接入取最近 20 篇，每次最多 20 篇新文章；列表保留 100 条；文章保留 30 天，主人读过或归入事件的保留 180 天；桶配额从 1 GiB 提到 8 GiB（粗估 10 个来源、每天各 30 篇带图约 120 MB/天）。影响 ST7、ST9。
答复：（未单独答复，按建议）

**Q10 需要登录的来源怎么取？** 建议用主人在 Agent 浏览器里的登录态，在后台标签页里取，不把 cookie 导出给脚本。代价是这类来源每次更新都要启动浏览器，所以默认频率降为 6 小时。影响 BR 全组。
答复：（未单独答复，按建议）

**Q11 拒绝非浏览器请求的站和付费站怎么办？** 实测 NYT、WSJ、Reuters 返回 403 / 401。建议：先用浏览器后台标签页取；仍取不到正文的，只收列表（标题、摘要），点开直接去原站，接入时就向主人说明。不做绕过付费墙和反爬的手段。影响 MN1、BR。
答复：（未单独答复，按建议）

**Q12 阅读记录和浏览记录记什么？** 建议两种记录分开。阅读记录是个人站里每篇文章的打开时间、停留、读到的位置。浏览记录是 Agent 浏览器里主人操作期间的网址（去掉查询串和片段）、标题、停留；交接密码期间不记，Agent 操作期间不记。都保留 90 天，只给维护这个站的 Agent 用，App 里能查看、清除、关闭。默认开。影响 RB 全组。
答复：（未单独答复，按建议）

**Q13 点“重排”之后。** 建议立即弹出选择：加入阅读（长期）、请 Agent 办事、只整理这一页。已收录的网址直接打开站内页面，不弹。哪一项排在最前由一个不用模型的判断决定（页面上有没有密码框、多字段表单、文章标记）。影响 AP3。
答复：（未单独答复，按建议）

**Q14 导入书签。** 建议解析后列出来让主人勾选，默认不全选；逐个来源排队接入，同时只有一个在接入。空站首页除了“导入书签”和“打开网站”，再加一个“在聊天里说想看哪些站”。影响 AP5。
答复：（未单独答复，按建议）

**Q15 “以后别看体育”这类小改动，要不要开 Work？** 建议不用，主人所在的 UI 会话直接修订 Goal 或改站点定义里的参数。要动脚本、模板或加来源才开 Work。影响 WG3、MN3。
答复：见改进单 Q19（按建议；已实施）。

**Q16 接入失败一部分怎么交接？** 建议接好的照常交给 Goal，没接好的写进 Work 报告并在站点定义里标为 `failed`，主人可以在来源管理里重试。影响 MN1、AP7。
答复：（未单独答复，按建议）

**Q17 “Goal 每天可调选题和排版”里的排版指什么？** 建议指模板参数：栏目顺序、每栏条数、默认排序、折叠。改模板代码算升级。每天由检查脚本给一次机会，阅读记录没有明显变化就不推理。影响 MN2、RB4。
答复：（未单独答复，按建议）

**Q18 主人一直不理会升级请求。** 建议只通知一次，来源页一直标着，停掉这一个来源的作业，其他照常；主人忽略后，除非问题变了，不再提。影响 WG5、MN2。
答复：（未单独答复，按建议）

**Q19 架构 §8.3 的软件维护 Goal（确认后自己派 Work 改代码）要不要改？** 建议保留：维护目标里写明“允许自己派 Work 修改代码”才可以，默认不写。这一期不动 §8.3 的行为，只在 §7.10 补一句。影响 WG6。
答复：见改进单 Q19（按建议；§7.10、§8.3 已写）。

**Q20 一期的重排包怎么处理？** 一期按网址的包（`manifest.json`、`refresh.mjs`、`index.html`、`reflow_refresh`、`reflow_action`）和个人站是两套东西，都留着 Agent 会混淆。建议 M5 删掉包这一套，只留“只整理这一页”（兜底页，`reflow result put`）。影响 CL 全组。
答复：（未单独答复，按建议）

**Q21 手机浏览器直接打开 `https://reflow.<zone>/`。** 现在没有会话时是纯文本 401。建议这一期做：未登录跳登录页，登录后回来；不做 PWA 和离线。影响 RH1。
答复：（未单独答复，按建议）

**Q22 新依赖。** 建议 `linkedom`（纯 JS 的 HTML / XML 解析，随 `buckyos-agent` 命令一起打包，不在运行时下载）。图片缩放用 ext-core 已有的 `magick`，没装 ext-core 时存原图（不超过 2 MiB）。没有别的新依赖。影响 ST2、ST5。
答复：同意引入（2026-11-25，见 §1.3）

**Q23 和 Plan / work_simple TODO 的先后。** 它已裁决、未实施，会改 `analyze_intent`、`goal`、`work_simple`、`plan_simple` 的同一批文本。建议它先落地，本文的 WG3–WG5 在它之后改；别的任务不受影响。
答复：见改进单 Q16：Plan 单已先合入（a5831ec7d），改进单在它之后做完。

**Q24 有新内容时通不通知？** 建议例行更新不通知。只有三种通知：需要登录、升级请求、主人明确要跟踪的事件有了重要后续（每天最多一条）。影响 MN2。
答复：（未单独答复，按建议）

**Q25 评论收多少？** PRD 写“可见评论”。建议每篇最多 10 条，按原站的默认排序，一起翻译；接入时可以按来源关掉。影响 ST3。
答复：（未单独答复，按建议）

### 1.3 后答的两条（2026-11-25）

owner 的原话：

> Q22同意引入，Q26按建议，HomeStation那边的spider也是结构，实际上并没开始做。Agent Browser这边的需求更真实具体一些。

**Q22 新依赖 `linkedom`。** 内容见 §1.2。单列是因为 AGENTS.md 要求新依赖明确确认。影响 ST2。
答复：同意引入。

**Q26 个人站和 HomeStation 是什么关系？**（由 Q3 的答复引出，事实见 §3.8）

- **建议：本期只统一结构。** 每篇文章带一个合法的 FeedObject 和它的 ObjId（§4.5），HomeStation 不改。二期验收后再做第二步：HomeStation 加一个收下私人抓取的方法，主人在文章页点“分享 / 收藏 / 稍后读”，由 App 用主人的会话交过去，之后走它现有的候选、收藏和分享。两边的订阅列表不合并。
- 另一种做法：本期就让作业把新文章写进主人的 HomeStation 候选，HomeStation 成为同一批内容的另一个视图，它的 RSS / 网页来源逐步改由 Goal 抓取。要先定 Agent 写主人候选的授权、谁决定入列、两份订阅列表怎么合并，工作量大约再多一个里程碑。
- 影响：本期只多 ST10。
- 答复：按建议。HomeStation 的 spider 只是结构，没有真正开始做；Agent 浏览器这边的需求更真实具体。
- 更正：这条问题原来写“两边重叠”，说重了。HomeStation 有订阅源列表和一份最小的抓取代码，但抓取不是做好的功能，没有一套成熟的抓取要合并。

### 1.4 答复的落实（2026-11-24、11-25）

| 问题 | 你的答复 | 落实 |
| --- | --- | --- |
| Q1、Q2 | 见改进单 Q18、Q17 | 已实施。来源的例行更新就是 Goal 作业（§4.2、§4.6）；建站、接入、升级的 Work 由浏览器会话直接创建，不属于 Goal，因此能建立和改写 Goal（§4.3） |
| Q3 | 按建议，可以进一步考虑抓回来的文章封装成HomeStation的FeedObject的结构? | 数据和模板分开、放 `reflow.<zone>` 根，不变。**文章加一层 FeedObject 外壳**：按 HomeStation 架构 §5.6 的私人抓取约定，译文写成 Markdown 文件由它 `wraps`，封面和图片用桶里对象的 `cyfile` ID。站内页面仍读结构化的块，站内的主键仍是网址的哈希（§4.5、ST10）。只统一结构，本期不把文章送进 HomeStation（Q26 已答） |
| Q4、Q5、Q15、Q19 | 见改进单 Q19 | 通用约定已实施。本文只做站点专有的部分：来源页上的升级说明和 App 的按钮（AP8）、`site upgrade`（ST8）、运维与升级的分界在站点里的例子（MN2）、参数类小改动（MN3） |
| Q6 | 按建议 | 一个用户一个站，数据、模板、来源程序归用户；`site.json` 的 `maintainer` 记当前维护的 Agent 和 Goal；换 Agent 是一次升级（§4.5、AP2、MN1） |
| Q22 | 同意引入 | `linkedom` 在构建时打成一个文件随命令分发，运行时不下载（ST2）。打包、断网运行、类型检查已试通（§3.2） |
| Q23 | 见改进单 Q16 | Plan 单、改进单的代码都已提交，本文排在最后（§6） |
| Q26 | 按建议，HomeStation那边的spider也是结构，实际上并没开始做。Agent Browser这边的需求更真实具体一些。 | 本期只统一结构，HomeStation 的产品代码不改。**抓取以个人站的需求为准**（我的理解）：取数、核对、翻译、重试都按本文做，不参照、不复用、不迁就 `spider.rs`；FeedObject 装不下的内容留在文章 JSON 里，不为了装进去而删减，装不下的地方记在 §3.8，留给 HomeStation 以后做抓取时参考 |
| 其余 | 没有单独答复 | 按建议。Q12（浏览记录默认开）和 Q20（删一期的重排包）涉及隐私和删除已交付的功能，开工前另行确认：**2026-11-25 owner 确认 Q12 两种记录都默认开、Q20 这次就删** |

## 2. 已裁决的约束

### 2.1 本期（owner 2026-11-24）

| # | 原话 | 落实 |
| --- | --- | --- |
| D1 | goal session里会倾向于用脚本完成周期性工作。也就是说，在文档工作一段时间后，从原始站点->构造重排页的流程，主要是由脚本完成的（如果可以这些脚本甚至都不依赖无头浏览器） | “文档”按“稳定”理解。例行更新由作业完成，不推理；取内容的顺序是订阅源 / 接口 → 普通请求 → 浏览器 |
| D2 | 用户首次建立 “agent重排站”的时候，是一个普通的worksession, 后面的goal session主要是用来定期同步数据/分析用户的浏览行为。这个结构还是比较新的，相当于agent worksession可以对goal session的维护目标进行改写 | 建站、接入、升级都是 Work；Work 建立和改写 Goal |
| D3 | 通用规则。相当于传统的worksession基于workspace 维护一个项目，但需要用户说需求改了才会触发推理。而goal session的视角里，这个worksapce是活的（不是代码目录），需要定时完成运维任务。大问题请求用户开worksession做版本升级 | GJ、WG 两组按通用能力做，不写成只给重排站用 |
| D4 | 按建议，可以进一步考虑抓回来的文章封装成HomeStation的FeedObject的结构?（Q3） | 文章带 FeedObject 外壳（§4.5）。原话带问号，按“值得做，怎么做由整理者提”理解 |
| D5 | 此外开发顺序上，GoalSession的改进(buckyos/notepads/opendan-goal-session-improve-todo.md)任务已经在进行中了。 | 本文排在改进单之后（§6） |
| D6 | Q22同意引入，Q26按建议 | 引入 `linkedom`（ST2）；本期只统一 FeedObject 结构，HomeStation 不改（§4.5、ST10） |
| D7 | HomeStation那边的spider也是结构，实际上并没开始做。Agent Browser这边的需求更真实具体一些。 | 抓取的需求和实现以本文为准，`spider.rs` 不作参考（§1.4 Q26、§3.8） |

### 2.2 之前定过、对本期有效的

| # | 内容 | 出处 |
| --- | --- | --- |
| E1 | 重排页的代码都是我们生产的，安全边界可看作一致，不做沙箱加载 | 一期 TODO D27 |
| E2 | 新网页 = 重排(网址, 当前时间, 历史记录)；缓存资源、组织成本地存储都有价值 | 一期 TODO D38 |
| E3 | 重排库和浏览器绑定，不和 Agent 绑定 | 一期 TODO §2 |
| E4 | 一个 Agent 一个浏览器会话（`ui.source = agent-browser`） | 一期 TODO D35、D36 |
| E5 | 密码交回用户，其余 Agent 代填；提交前不另行确认 | 一期 TODO §2 |
| E6 | 机制越少越好，优先用已有的事件和提示词 | Plan / work_simple TODO D6、D9 |
| E7 | 检查脚本不给 run 和写权限；不为“Agent 把脚本写坏”另加机制，兜底是硬保障 | Goal TODO §8 |
| E8 | 预算是“背地里就给你这么多 token 干活”，用量要让 Agent 看得见 | Goal TODO §8 |
| E9 | Goal 是珍贵的资源，不靠多建 Goal 分摊职责；内置配置只是初始状态，可以自我演化 | Goal TODO §8 |
| E10 | AICC 不自动换模型，fallback 由管理员手工配置 | [aicc_router](../doc/aicc/aicc_router.md) §9 |
| E11 | 先能力再安全，缺口记成已知风险 | 多处 |
| E12 | 用户没有 PC，体验按手机评估；选文件默认面向 Zone | 产品原则 |
| E13 | skill 是纯文本，不带命令和安装脚本；命令放 `buckyos-agent` 或扩展包 | AGENTS.md |

## 3. 事实

### 3.1 成本与资源

- 一次什么都不做的 Goal Activation 是 2.5–7.8 万 token，结晶 Goal 一次 `no_action` 18.2 万。Goal 缺省周期预算 24 小时 200 万，硬间隔 900 秒，Agent 后台总量 500 万。
- 一期真实模型做一张兜底重排：80 秒，约 41 万 token（含意图分析和建 Work）。8 条消息的 DV 共 386 万输入 token。
- 浏览器内存（有窗口、2 倍屏）：1 / 3 / 6 / 12 个真实网站 468 / 1393 / 2124 / 3458 MB（[内存 TODO](agent-browser-memory-todo.md)）。

### 3.2 不开浏览器取内容（2026-11-24 实测，未登录，普通 UA）

| 站 | 列表 | 正文 |
| --- | --- | --- |
| 雅虎日本新闻 | RSS 有 ETag，`max-age=60`；RSS 指向 `/pickup/`，再指向 `/articles/` | 5 篇都是 200。抽看 2 篇，正文和前 10 条评论都在 HTML 内嵌的 `__PRELOADED_STATE__`（`articleDetail.paragraphs`、`commentFull.userCommentList`） |
| NHK、ITmedia、Ars Technica | RSS，有 ETag | HTML 里有正文。**2026-11-25 更正：NHK 变了**，`www3.nhk.or.jp/rss/news/cat0.xml` 从 2026-08-08 起不再更新，链接跳到 `news.web.nhk`，那里不确认使用提示或不登录只给导语；样板 `news-web-nhk` 按“只有导语”做 |
| Asahi | RSS，有 ETag | 只有约 800 字，部分付费 |
| BBC、DW、Spiegel、Le Monde、Guardian | RSS | HTML 里有正文；Spiegel、Ars Technica 的 RSS 带全文 |
| Hacker News | RSS | 链到各外站 |
| NYT、WSJ、Reuters | 前两个 RSS 可取 | 403 / 401，需要浏览器或只收列表 |
| 日经 | 首页可取，没有找到 RSS | 没测 |

- 解析：Deno 2.9.8 里 `linkedom@0.18.12` 解析 RSS 1–3 ms、300 KB 的页面 5–11 ms，进程约 83 MB，包缓存 12 MB。
- `deno bundle` 能把 `npm:linkedom@0.18.12` 打成一个 466 KB 的 `.mjs`（2.9.8，2026-11-25）。空缓存、断网时在本机 2.9.8 和工作镜像的 2.9.2（`local/aios-test`，`--network none`）都能解析 RSS（CDATA、中日文）和 HTML（取正文、取内嵌脚本的内容）；镜像里 `deno check` 也过。可选的 `canvas` 没有带上，也没有用到。许可证 ISC，直接依赖是 MIT、ISC、BSD-2-Clause。npm 上的最新版是 0.18.15，没测。
- 没测：连续抓取几天会不会被限流；不同地区出口的差异。

### 3.3 Goal 与 Work（`src/frame/lib_opendan/src`）

**2026-11-24：本节是 `1704812f8` 的事实。改进单实施后变了的：** 由 UI 会话直接创建、不属于某个 Goal 的 Work 权限等同父 UI，`created_by.work` 记下它；对端是 owner 的 UI 会话可以改写任何非内置 Goal；`goal create / revise` 的命令行字段已补齐，创建时可装检查脚本，没有 Goal 也能 `goal check run`；`scheduled` 输入没被消费之前不再投第二次；意图分析按请求判断先建 Work 还是直接建 Goal，小改动直接 `goal revise`（结果用 `forwarded`，没有新增 outcome）；`goal.toml` 写明运维与升级的分界和升级请求；`questions` 可以带选项。`goal post` / `activate` 仍没有权限检查；`behavior`、`scope.workspace` 仍要 restart 才生效。

- 权限：`state/goals.rs:436` 按自报的 `session` 判定调用方；create `:617`、revise `:731`、set_check `:993`、set_status `:838`。`created_by.via` 对非 UI 一律写 `owner`（`:651`）。创建时有 `check_script` 字段（`:100`、`:706`）。Goal 不能以休眠状态创建（`:682`）。`goal post` / `activate` 没有权限检查。
- Work 创建：`api.rs:1053 create_sub_session`，`Origin` 每次新建（`:1128`），不继承。`origin.goal` 的用处：预算（`goal.rs:91`）、预算用完暂停（`host/goals.rs:684`）、归档停止（`host/mod.rs:140`）、吸收（`runner/children.rs:216`）、Goal 头（`runner/goal.rs:431`）。所以升级 Work 不能带它。
- revise 不唤醒 Goal，下一次 Activation 的头里有 `changed_since_last_activation`（`runner/goal.rs:843`）。`scope.workspace`、`behavior` 冻结在 Goal Session 里，改它们要 restart（`goal.rs:517-535`）。
- 起步：带检查脚本且不是 `--start` 的 Goal，epoch 开始先跑检查，不推理（`host/goals.rs:886`）。
- Workspace 写入权：每次 drive 取、drive 结束放（`runner/drive.rs:415`、`:1518`）。取不到就 Busy，宿主 5–30 秒退避重试（`host/mod.rs:615`），不报错；这期间归档、休眠、回答也都等着。Plan 根会话不持有写入权（`:421`），Todo 之间 Goal 可能插进来。
- **Busy 期间每个通过检查的时间槽都会投一条 `scheduled` 输入，它绕过硬间隔**（`protocol/goal.rs:545`）。Work 占用几小时后放行，会连续打开多次 Activation。
- 提问到通知：`runner/goal.rs:1314`，只有自由文本；同一文本的问题答过之后再问，返回的是同一条已关闭的通知（`:1292`）。通知 `notify send`：`xagent_cli.rs:3130`，`--key` 去重，`--url` 是点开落到哪。`normal` 级的 Goal 通知投到**默认会话**（聊天路由 TODO §6.2）。
- 意图分析只会在 Work 和 Goal 之间二选一，持续的事直接建 Goal（`analyze_intent.toml:34-38`、`:71-77`、`:101-103`）。结果的 schema 是 `additionalProperties: false`（`:124-191`），宿主校验在 `runner/reports.rs:731-800`。
- `goal.toml:50` 写着 Goal 不能改定义。`work_simple.toml` 没有提到 Goal。
- 一定要改的用例：`tests/goal_session.rs:1248 callers_are_kept_to_their_kind`（断言 Work 建 Goal 被拒）。

### 3.4 脚本运行与模型

**2026-11-24：** Goal 作业已有（`goal_jobs.rs`，契约在 Agent State Protocol §9.9），`xllm` 的用量经 `OPENDAN_USAGE_LOG` 计入 Goal 预算；脚本绕过 `xllm` 直接调 AICC 的用量仍然看不见，所以站点脚本的翻译要走 `xllm` 或自己往 `OPENDAN_USAGE_LOG` 写记录。

- 同步任务 `sync.rs` 的契约、限额、失败分类、凭据、重启行为（Q1 没有选它，只作参考）：`TaskDef:79`，限额 `:35-48`，Deno 参数 `:274-296`，判定 `:359`，`finish:950`，`check_host:488`。Deno 2.9.8 支持 `--allow-net=*.yimg.jp`。宿主重启时正在跑的任务没有运行记录，那一槽也不补。
- 调度可复用：`goal_time.rs` 的 `Timetable`（`every:<秒>` 和 cron + 时区），检查脚本也用它。
- 借给命令的会话令牌至少还有 600 秒有效期，命令自己不能续（`agent_tool/src/runtime_context.rs:16`）。**作业一次运行不能超过 600 秒。**
- 读写权限的 Agent 用这个令牌可以直接 `PUT /bucket/v1`，只读 Agent 不行（`nfs_server/tests/reflow.rs:331-353`）。只读 Agent 能装作业（改进单 §11），但写不进桶，所以不能维护个人站，AP2 要说明。`reflow` 命令就是这么写的（`src/tools/buckyos-agent/lib/reflow.ts` 的 `putBytes`）。
- `llm.translate` 路由已有，成本优先（`aicc/src/service/model_defaults.rs:686`、`:1104`）。`helper.llm_chat` 的 `policy.allow_fallback`、`runtime_failover` 默认都是 true（`aicc_client.rs:2267`），要显式关。
- Goal 预算只统计会话的 `usage.jsonl`（`goal.rs:77`、`:91`、`:159`），脚本直接调 AICC 的用量看不见。
- `buckyos-agent` 的共享模块靠 `_prepare_agent_tools`（`src/buckyos-build.py:208`）和 `build_aios:517` 复制进产物，并改写 `deno.json` 的 `imports`。它现在没有任何 npm 依赖，也没有打包 npm 依赖的步骤；镜像构建末尾会删掉 Deno 缓存，所以写 `npm:` 说明符会在运行时联网下载。命令以 `--allow-net --allow-read --allow-write --allow-env` 运行。

### 3.5 重排主机与桶

- `reflow.<zone>`：只有 GET / HEAD（`nfs_server/src/bucket/reflow.rs:106`），只认目标为 `system:nfs-server` 的用户会话（`:51`、`auth.rs:63`），没有会话回纯文本 401（`:111`）。`/` 是桶根的 `index.html`（`:77`）。PUT 时带的 `Cache-Control` 原样下发，图片可以设成长期不变。
- 这台主机上网关先分走 `/home/*`、`/kapi/*`、`/bucket/*`、`/ndn/*`、`/ndm/*`、`/nfs/v1/*`、`/s3/*`、`/sso*`、`/.well-known/*`、`/1.0/identifiers/*`（`src/rootfs/etc/boot_gateway.yaml:338-400`）。`/_x` 这类路径能到 nfs-server，而且不会和一期的 `<site>`（必含 `-`）冲突。
- 会话 15 分钟过期，刷新令牌 7 天，页面脚本能 `POST /sso_refresh`，但首次打开文档不能。登录页对这个主机的交互登录后端已允许（`app_availability.rs:81`、`sys_auth_backend.rs:1394`），静默跳转目前只给 AI Workspace（`src/frame/desktop/src/auth/LoginPage.tsx:164`、`:213-221`）。
- 桶：单次 PUT 32 MiB；支持 If-Match / If-None-Match；`bucket.batch` 一次最多 100 个、全成或全败；LIST 是 kRPC，一页 1000；`x-bucky-expires-at` 可以给单个对象设过期；没有按前缀删除。**`ensure_bucket` 每次同步都会按声明改写 lifecycle 和配额**，运行时改的会被覆盖。配额声明在 `buckyos-api/src/control_panel.rs:57`。声明只在激活时写进 `services/control-panel/spec`，已有 Zone 要手工补（一期 DV 的坑）。
- PUT 的返回里 `obj_id` 就是这份内容的 `cyfile:` FileObject ID（`cyfs-ndn/src/bucket_store/src/content.rs:16`），GET 的响应头 `x-bucky-obj-id` 也是它。ID 含文件名（key 的最后一段）和服务端探测的媒体元数据，所以用返回值，不在脚本里自己算。
- 图片可以作为独立对象存（带 Content-Type）。一期放 `data:` 只是脚本的做法。
- 同一主机上所有页面同源，任何页面脚本都能读写阅读记录。

### 3.6 浏览器

- `agent-browser run` 取租约、用当前标签页，用户占着时拒绝（`groups.rs:410`）。一期的 `reflow-run` 不取租约，开自己的后台标签页（`_reflow` 组），不计入 `max_tabs`，同时只跑一个（`browser/reflow.rs:598`）。它只由 Agent Service 启动。
- 用浏览器的 cookie 发请求而不开标签页：没有命令，`page.context().request` 可以做到。cookie 只能经运行中的浏览器取。
- 空闲 600 秒停（`mod.rs:1199`）。再启动会重开保存的标签页。
- 没有任何浏览历史和停留记录。可以挂的事件入口只有一个：`mod.rs:896 on_event`；看的人 `mod.rs:227`，当前标签页 `groups.rs:343`，谁在操作 `groups.rs:391`。

### 3.7 Desktop App（`src/frame/desktop/src/app/agent-browser`，下称 AB）

- 三个模式 `page | reflow | chat`（`launch.ts:6`），默认 `page`（`store/store.ts:82`），没有首页。`page` 和 `chat` 都会连屏幕并启动浏览器（`store.ts:303-321`）；`reflow` 不连，但进入时会自动刷新包（启动浏览器）或自动发消息请 Agent（花 token）（`reflow/controller.ts:260-278`）。
- 没有独立的“重排”动作，切到 reflow 标签就是。页面里的 `reflow.open` 会再走一次 `evaluate`，没收录的链接就自动请 Agent（`controller.ts:524-531`），这正是 PRD 的第一个痛点。
- iframe 没有 sandbox（`ReflowFrame.tsx:55`），按来源校验消息（`:36`）。登录走 `requestServiceLaunch('nfs-server', …)`（`reflow/source.ts:37`）。
- 往浏览器会话发消息有现成接口：`sendToBrowserSession`（`conversation/session.ts:67`）。往输入框预填文字没有接口；可以在挂载前 `hub.saveDraft`。
- 启动参数只有 `agent` 和 `mode`（`launch.ts:13-16`）。新的 `requestId` 会重新挂载整个 App（`AgentBrowserAppPanel.tsx:27`）。通知卡片的 `source.link` 是普通链接（`messagehub/notify/NotifyCard.tsx:125`）。
- 基线里 Desktop 没有任何地方调 `goals.*`，Goal 的页面只在 Agent 主页 WebUI（`src/frame/opendan/web`）。**改进单已加（UX1–UX3，`5b197b11e`）：** `components/conversation/goals/`（`GoalConversation`、`useGoalList`、`useGoal`、`source.ts`）和 Agent 面板的 `GoalList`。AP7 复用它们。作业的启停和立即运行有现成的 kRPC（`goals.job_set_enabled`、`goals.job_run`），owner 的会话可以直接调。
- Work 卡片：会话气泡里已有（`components/conversation/history/renderers.tsx:36`）。
- 没有书签解析。选文件用 `usePickFiles`（`sysdlg/select/pickFiles.ts:46`），照 `LibrarySheet.tsx:117` 的写法。
- 10 种语言：en、zh-CN 写在 `i18n/agent-browser*.ts`，其余 8 种在 `i18n/locales/*.json`；`desktop-i18n.spec.ts` 检查齐全。
- 默认模式变化会碰到 `tests/e2e/pages/agent-browser.spec.ts:105`、`:380`。

### 3.8 HomeStation 的 FeedObject（`src/frame/homestation/src`，2026-11-24、11-25 看代码，没有跑）

**两部分的成熟度不同。** FeedObject 的结构、校验和对象身份是 HomeStation 发表、投递都在用的，按它做。抓取（`spider.rs`）owner 明确只是结构、没有真正开始做（D7）：代码里有最小的一份（正则解析 RSS / Atom、链接卡片、整页 HTML 快照，由 `pull.rs:107` 定时调用，缺省开着，`tests/sources.rs` 有一个用例），没有正文提取、翻译、核对、条件请求、限速和需要登录的处理。**不要拿它当实现参考，也不要改它。**

- 结构（`protocol.rs:144`）：`kind`、`publisher`（DID）、`iat`（秒）、`nonce?`、`entry?`、`content {type, title?, summary?, text?, cover?, media[{object, alt?}]}`、`wraps?`、`references[{relation, object_id}]`、`tags[]`、`source {kind, original_url, original_author?, captured_at_ms}`、`link?`、`base_on?`。不可变，ObjId = `cyfeed` + 规范化 JSON 的 SHA-256（`:314`）。没有语言和原文发布时间的字段。
- 限制（`validate_feed_object`，`:532`）：对象 ≤ 64 KiB；标题、摘要、正文合计 ≤ 8000 字符，更长的正文放进 `wraps` 的文件；媒体 ≤ 9；标签 ≤ 16；`wraps`、`cover`、`media` 只收 ObjId，不收网址；数字只能是整数（`:327`）。
- 私人抓取在代码里的样子（`spider.rs:271`、`store_capture:299`，只用来说明形态）：`kind: post`，`publisher` 是主人的 DID，没有 `entry`，带 `source` 和 `link`，存成 `local_only`，进 `candidates`。选中后抓一份 HTML 快照，`wraps` 指向它，`content.type` 变成 `article`（`maybe_snapshot:339`）。架构 §5.6：私人抓取不进发表流，分享时才以主人自己的入口发表；cookie 一类的访问材料不进对象。
- 正文怎么显示：`item.wrapped_body`（`resources.rs:394`）读 `text/*` 或 `application/json`、单块、≤ 2 MiB 的文件（`evaluation.rs:655`），Desktop 当 Markdown 画（`ItemDetail.tsx:49`）。它用的 `MessageMarkdown` 没有图片分支。
- 按 ObjId 能从 Zone 的 named store 取到文件（`objects.rs:450`），所以桶里的图片和正文不用再传一份。
- 没有外部写入的入口：`handle_rpc` 要求调用方的 DID 就是这个 HomeStation 的主人（`api.rs:50`），Agent 的会话进的是 Agent 自己的 HomeStation（`host.rs:232`）；`publish.create` 不能带 `source`；`publish.share_capture` 只能分享已经在库里的私人抓取。
- 装不下个人站需求的地方，留给 HomeStation 以后做抓取时参考（本期不改结构）：没有原文语言和原文发布时间；一篇只有一种语言的标题和摘要，装不下译文和原文对照；媒体最多 9 个；评论者没有 DID，原站评论没有位置；没有“来源要登录、不可再分发”的标记；网址不变的更正和事件的后续只能靠 `base_on`、`references` 表达，没有按网址查最新版本的约定。
- TS：websdk 的 `buildNamedObjectByJson('cyfeed', …)`（`src/ndn_types.ts:810`）和 ndn-lib 同一算法，不用新写。FeedObject 的类型只在 Desktop（`app/homestation/protocol/feed.ts`），websdk 没有。

## 4. 设计

### 4.1 全貌

```text
主人点「重排」/ 聊天 ─► 浏览器会话（UI）─► 意图分析 ─► Work：建站 / 接入来源 / 升级
                                                       │ 在工作区写来源程序和模板，用真实页面核对
                                                       ├─ 安装：site template put、goal job put
                                                       └─ 建立或改写 Goal（维护目标）
                                                             │
宿主按时间表 ─► Goal 作业（site sync，不推理）─► 用户的重排桶 ─► reflow.<zone> 首页 / 来源页 / 文章页
                     │ 运行记录、用量                              │ 阅读记录 POST /_read
                     ▼                                            ▼
              Goal 检查脚本（只读）── run: true ──► Goal Session：修小故障、分析阅读、请求升级
```

三层东西分开：**工作区**是源码，Work 和 Goal 都能改；**安装版本**是作业和模板的运行副本，安装就是发布；**内容**归用户，在桶里，和源码无关。Goal 修小故障也要先改工作区再安装，否则下次升级会把修复覆盖掉。

### 4.2 Goal 作业（通用能力，Q1）

**已实施（改进单 GJ）。和下文不同：** `usage.jsonl` 每个 Goal 一份，在 `jobs/usage.jsonl`，记录带 `run_id`；命令不经过 shell，`{dir}` 换成安装版本的目录；`trigger` 另有 `install`；另有 `goals.job_runs`、`goals.job_log` 两个读方法；预算用完时 `OPENDAN_MODEL_BUDGET` 是剩余额度（`0` 才拒绝），不是只有 0。

存放：`state/goals/<goal_id>/jobs/<job_id>/`

| 文件 | 内容 |
| --- | --- |
| `job.json` | `{job_id, title, version, command: [..], timetable, timeout_secs, enabled, updated_at_ms, updated_by}`；`timetable` 是 `every_secs`（≥ 300）或 cron + 时区，用 `goal_time::Timetable`；`timeout_secs` 缺省 300，最大 600 |
| `versions/<n>/` | 安装进来的程序目录，只读，保留最近 5 个 |
| `data/` | 作业自己的可写目录，也是运行时的 cwd；升级不动它 |
| `runs.jsonl` | `{run_id, version, at_ms, ended_at_ms, status: ok \| failed \| timeout \| killed, kind?: transient \| auth \| program \| budget, summary, metrics?, trigger: schedule \| manual}`，保留最近 200–400 条 |
| `schedule.json` | 宿主的进度：上一个槽、连续失败数、正在跑的 run |
| `usage.jsonl` | 模型用量，`UsageRecord` 的格式 |
| `logs/<run_id>.log` | stdout + stderr 的末尾 64 KiB，保留最近 20 份 |

- 命令：`agent-session goal job put <job_id> --goal <goal_id> --dir <目录> --command "<命令>" (--every <秒> | --cron ".." [--timezone ..]) [--timeout <秒>] [--title ..]`，以及 `list | show | run [--wait] | log | enable | disable | rollback [--to <n>] | rm`。kRPC：`goals.jobs`（读）、`goals.job_put / job_run / job_set_enabled / job_rollback / job_delete`。
- 运行：宿主（Agent Service 进程）在私有 runtime 里起子进程，`cwd = data/`，进程组可整体结束。环境变量：`PATH` 同会话的 shell，`OPENDAN_AGENT_DID / AGENT_ROOT / RUNTIME_ID`，刷新过的会话令牌，`OPENDAN_GOAL_ID`、`OPENDAN_JOB_ID`、`OPENDAN_JOB_RUN_ID`、`OPENDAN_JOB_DIR`（当前版本目录）、`OPENDAN_USAGE_LOG`。**不设 `OPENDAN_SESSION_ID`**，作业不是会话。
- 结果：退出码 0 是 ok；stdout 最后一行如果是 JSON `{summary?, kind?, metrics?}` 就取它。`transient` 隔 30 秒、120 秒重试。作业不发通知：要告诉主人的事由检查脚本的 `notify` 发。
- 调度：同一作业同时一个运行；每个 Agent 最多 2 个作业同时跑；错过的槽合并成一次；新装的作业立即跑一次。宿主启动时，把 `schedule.json` 里还标着在跑的运行记成 `killed`，下一轮补跑。
- 生命周期：Goal 休眠或归档，不再启动，正在跑的结束掉；唤醒后恢复。Agent 停用时停。
- 权限同检查脚本：owner；这个 Goal 自己；Q2 定的 UI 和 Work。
- 预算：周期预算或 Agent 总量用完时照跑，但带 `OPENDAN_MODEL_BUDGET=0`，模型步骤拒绝执行并以 `budget` 结束。
- 检查脚本的 `ctx` 加 `jobs`：每个作业的 `{job_id, version, enabled, last_run, failures, next_at_ms}`。Goal 头加 `<jobs>`，列同样的内容。

### 4.3 Work 建立和改写 Goal（Q2）

**已实施（改进单 WG1、WG2）。** 收尾的通用做法是 work skill `goal_handover`，重排站专有的交接说明仍在 MN1。

- `CallerKind::Work` 带上它的 UI 父会话：登记项 `origin.parent_session` 是 UI、`origin.goal` 为空时有，否则没有。有的 Work 可以 create、revise、set_check、job_*；`created_by.via` 写 `ui_session:<父会话>`（通知的去处和后续权限都靠它），另记 `created_by.work = <sid>`。
- UI 修订：对端是 Agent owner 的 UI 会话可以修订这个 Agent 的任何非内置 Goal；群会话和对端是别人的 UI 仍只能动自己建的。对端的判定复用给 Memory 绑定主体的那段（`memory::session_caller`）。
- `goal create` / `goal revise` 的命令行补齐 `healthy_state`、`achieved_when`、`constraints`、`workspace` 和检查脚本（创建时 `--check`，改用 `goal check set`），不再要求写 `--json`。
- Work 交接的内容：建 Goal（带检查脚本，不带 `--start`，所以起步不推理）、装作业、把维护说明作为 goal skill 加进 `initial_skills`，在 `report` 的 `result` 里写 `goal_id`。
- 改写时先 `goal show` 取修订号再 `revise`，冲突就重读再改。Goal 下次 Activation 会看到哪些字段变了、作业换到了哪个版本。
- 不加“升级中”的标记。Work 占着工作区时 Goal 的推理自然等待；Goal 要动手修复前先看有没有主人发起的 Work 正在用这个工作区，有就不修（提示词）。

### 4.4 升级请求（Q4）

**通用约定（第 1、3、5 条和“不再提”）已写进 `goal.toml`、`goal_delivery`、`analyze_intent.toml`、`ui_core`（改进单 WG5）；站点专有的第 2、4 条和 `site upgrade close` 仍是本文的事。**

1. Goal 写 `upgrades/<日期>-<短名>.md`（在工作区里）：发生了什么、影响哪些内容、建议、任务书草稿、不升级的后果。
2. 站点场景：`site upgrade open <source> --file ..`，把摘要写进这个来源的状态，来源页和 App 能看到。
3. `agent-session notify send --title .. --message .. --key "upgrade:<goal>:<短名>" --url "/?agent_browser=<agent>&mode=home&upgrade=<短名>"`。
4. 主人点“升级”，App 用 `sendToBrowserSession` 发一条固定格式的消息（带文件路径、goal_id）。没有专用 App 时，主人在对话里答应也一样。
5. 意图分析读那份文件，建 Work。做完 `site upgrade close <短名>`。
6. 主人点“暂不”：记进状态，Goal 不再提；问题变了用新的短名。

### 4.5 个人站的数据（Q3）

桶 `user/control-panel/<owner>/reflow`，下面是 key：

| key | 写的人 | 内容 |
| --- | --- | --- |
| `index.html` | `site template put` | 很小的入口：读 `_data/site.json`，加载当前模板版本 |
| `_app/<version>/**` | 同上 | 模板，按版本，不再改；`Cache-Control: immutable` |
| `_data/site.json` | Work、主人的 UI、Goal（只改参数） | 站点定义，写入带 If-Match |
| `_data/feed/<source>.json` | 这个来源的作业 | 状态 + 最近 100 条摘要。**最后写它**，写完新内容才可见 |
| `_data/a/<article>.json` | 同上 | 文章，含 FeedObject（见下）。`<article>` 是规范化网址 SHA-256 的前 16 位（沿用一期算法和测试向量）。更正时覆盖，`rev` 加一 |
| `_data/md/<article>.md` | 同上 | 译文的 Markdown，FeedObject 的 `wraps` 指向它。页面不读它 |
| `_data/img/<sha256>.<ext>` | 同上 | 图片，内容寻址，长期不变 |
| `_data/events.json` | Goal | 事件分组（§4.11） |
| `_data/reading.json` | 汇总作业 | 每篇的最近阅读、累计停留，每个来源的汇总 |
| `_read/<yyyymmdd>/<ulid>.json` | nfs-server | 页面上报的一批阅读记录，90 天过期 |
| `_src/<source>/<version>/**` | `site source put` | 来源程序的副本（Q6：换 Agent 时取回） |

`site.json`：

```json
{ "schema": "buckyos.site/1", "title": "…", "language": "zh-CN", "owner_did": "did:…",
  "topics": ["国际", "科技", "体育"],
  "template": { "version": "t3", "params": { "home_order": "time", "per_column": 20, "hide_topics": ["体育"] } },
  "columns": [ { "id": "jp", "title": "日本", "sources": ["news-yahoo-co-jp"] } ],
  "sources": [ { "id": "news-yahoo-co-jp", "title": "雅虎日本", "home_url": "https://news.yahoo.co.jp/",
      "match": [ { "host": "news.yahoo.co.jp" }, { "host": "www.yahoo.co.jp" } ], "lang": "ja", "kind": "news",
      "needs": "http", "job": "src-news-yahoo-co-jp", "every_secs": 1800, "max_new": 20, "keep_days": 30, "comments": 10,
      "state": "preparing | active | paused | failed", "added_at": 0, "added_by": "work:<sid>" } ],
  "maintainer": { "agent": "did:…", "goal_id": "g-…" }, "updated_at": 0, "updated_by": "…" }
```

`feed/<source>.json` 的 `status`：`{ state: preparing | ok | failed | needs_login | needs_upgrade | paused, checked_at, published_at, error?: {kind, message}, note?, login_url?, upgrade?: {id, title, summary} }`。`checked_at` 是最近一次**通过核对**的检查时间，没有新文章也算；脚本跑完但没通过核对不更新它。`items[]`：`{id, object_id, url, title, summary, image, tags, published_at, fetched_at}`，标题和摘要是译文，原文在文章里。列表只放索引，不放整个 FeedObject，首屏体积见 TP2。

文章：`{ schema, id, source, url, lang, published_at, fetched_at, rev, object_id, object, title: {o, t}, byline, blocks: [...], comments: [...], links: [{url, text, article?}], images: {<cyfile>: {key, w, h}}, supplement?: {event, adds} }`。`blocks` 的类型：`p`、`h2`、`h3`、`quote`、`li`、`code`（都带 `text: {o, t}`），`img`（`src, alt: {o, t}, w, h, missing?`），`table`（`rows`）。正文里的链接收在 `links`，不做行内链接。

**FeedObject 外壳（Q3、Q26）。** `object` 是一个合法的 FeedObject，按 HomeStation 架构 §5.6 的私人抓取约定（主人名义、没有 `entry`、只在本地），`object_id` 是它的 ObjId。合不合法只看 `validate_feed_object`，不看 `spider.rs` 怎么写：

```json
{ "kind": "post", "publisher": "<site.json 的 owner_did>", "iat": 1764000000,
  "content": { "type": "article", "title": "<译文标题>", "summary": "<译文摘要>",
               "cover": "cyfile:…", "media": [ { "object": "cyfile:…", "alt": "…" } ] },
  "wraps": "cyfile:…", "tags": [ "科技" ], "link": "<原文网址>",
  "source": { "kind": "web", "original_url": "<原文网址>", "original_author": "<byline>", "captured_at_ms": 1764000000000 },
  "base_on": "cyfeed:…" }
```

- `wraps` 是 `_data/md/<article>.md` 的 `cyfile`，`cover`、`media` 是 `_data/img/` 里图片的 `cyfile`，都取 PUT 的返回值。`media` 最多 9 张，其余只在块里。没有配图就没有 `cover`。
- `iat` 取抓取时刻，不带 `nonce`，没有 `entry`。同一次抓取算出来的 ObjId 是确定的。更正（`rev` 加一）是新的对象，`base_on` 指前一个。
- `tags` 取自 `site.json` 的 `topics`，由翻译那一步给出（§4.7）。主人说“别看体育”，改的是模板参数 `hide_topics`。
- 原站的评论不做成 FeedObject：评论者没有 DID。原文、原文语言和发布时间没有对应的字段，留在文章里。文章 JSON 是完整的一份，外壳是它的一个视图，两者冲突时不为外壳删减文章。
- **站内只用网址的哈希作主键**：路由、阅读记录、事件都用它，更正不会断开。`object_id` 是对外的身份，这一期没有消费方（Q26 按建议，第二步在 §10）。
- 私人抓取不等于可以再分发：对象不签名、不发表。要登录才能看到的来源（`needs: login`）照样有 FeedObject，以后做分享时要默认排除。

页面路由用查询串：`/` 总首页，`/?s=<source>` 来源页，`/?a=<article>` 文章页，`/?e=<event>` 事件页。

### 4.6 来源程序和 `site` 命令

来源包是一个目录：`source.json`（id、匹配、语言、频率、核对阈值、`needs: http | browser | login`）、`extract.ts`、`fixtures/`（几份真实页面，给测试用）、`NOTES.md`（怎么取、怎么核对、有什么坑；给以后修它的 Goal 和 Work 看）。

```ts
export default async function extract(ctx: ExtractCtx): Promise<Extracted>;
// ctx: source, state, limit, known(urlOrId), log
//   fetch(url, { conditional }) → { status, text, notModified }   // 统一 UA、同一主机 1 次/秒、条件请求、重定向、超时
//   html(text), xml(text)                                          // linkedom
//   browser(url, script)                                           // 仅 needs 为 browser / login，见 §4.10
// Extracted: { list: [{url, title, published_at?, summary?, image?}], items: [RawArticle], state? }
// 失败：throw new SiteError("auth" | "transient" | "program", message)
```

取数的部分由 Agent 写，其余是产品代码（`src/tools/buckyos-agent/lib/site/`），所有来源共用：

| 步骤 | 做法 |
| --- | --- |
| 1 取数 | 跑 `extract.ts` |
| 2 核对 | 列表原来有现在空；正文少于 `max(200, 最近 30 篇中位数 × 0.3)`；替换字符多于 1%；3 篇以上正文相同；标题为空。单篇不过就不发这一篇，一次里超过一半不过整批失败（`program`）。阈值可以在 `source.json` 里按来源改 |
| 3 图片 | 下载到临时目录，`magick` 缩到长边 1280、去元数据；失败的留到下次，连续两次失败按 `missing` 发布 |
| 4 翻译与归类 | §4.7 |
| 5 封装与写入 | 图片、译文 Markdown 先写，拿到 `cyfile`；组 FeedObject、校验、算 ObjId；再写文章；每满 10 篇翻一次 `feed`。校验不过的这一篇不发。超时时已翻的已经可见，其余下次继续 |
| 6 清理 | 过保留期的文章和没有引用的图片，每天一次，每次最多删 500 个 |
| 7 记录 | 最后一行 JSON：`{summary, kind?, metrics: {listed, new, published, rejected, images, tokens, ms}}` |

命令（`src/tools/buckyos-agent/commands/site.ts`，输出和退出码照 `reflow` 的约定）：

- `site init`：没有 `site.json` 就建，装默认模板。`owner_did` 从 Agent 文档的 `owner` 或 control-panel 的 `get_user_did` 取，Agent 的令牌能不能读没验，ST8 时先试。
- `site status [--json]`：站点、每个来源的状态、最近几次作业。
- `site match <网址>`：已收录的文章、属于哪个来源、还是没有。
- `site config get | set`：改标题、语言、栏目、模板参数、来源的频率和开关，带 If-Match。
- `site source put <dir> | list | rm | get <id> <dir>`：写 `_src/` 和 `site.json` 里的这一项。
- `site try <dir> [--limit 3] [--no-translate]`：跑一遍前三步，把结果写到本地目录，打印核对结论，不写桶。
- `site sync <source> [--dir <来源包>]`：完整跑一遍。作业就是它：`agent-session goal job put src-<source> --goal <goal_id> --dir <来源包> --command "site sync <source> --dir {dir}" --every 1800 --timeout 600`。新版本装上会立即跑一次。
- `site template put <dir> | list | use <version>`：`use` 就是回退。
- `site preview <dir>`：本地起一个静态服务，用 `site try` 的输出或桶里的数据看页面（Agent 打不开 `reflow.<zone>`）。
- `site upgrade open | close | list`、`site events get | put`、`site reading`。

### 4.7 翻译与归类

- 一篇一次请求：标题、摘要、各块、评论的原文数组进去，等长的译文数组出来，用 `json_schema` 约束；同一次请求再给出至多 3 个 `topics` 里的标签，原文没有摘要时补一条。数组长度对不上就重试一次，再不行这篇留到下次。标签不在 `topics` 里就丢掉，不为它重试。
- 路由 `llm.translate`，`policy: {allow_fallback: false, runtime_failover: false}`。没有可用模型、额度、权限的拒绝都按配置问题处理，写进状态，不重试。
- 按原文块的 SHA-256 缓存在作业的 `data/`，更正文章时只译变了的块。
- 原文语言和站点语言相同时不译，只做归类和补摘要。
- `trace_id = goal-job:<goal>:<job>:<run>`，每次调用往 `OPENDAN_USAGE_LOG` 追加一行。作业环境没有这个变量时（Work 里试跑）不记，会话自己的用量照旧。
- 公用实现放 `src/tools/buckyos-agent/lib/model.ts`，往 `OPENDAN_USAGE_LOG` 写的记录和 `xllm` 同一格式（`at_ms, model, run_id, input_tokens, output_tokens, total_tokens`，`run_id` 是 `OPENDAN_JOB_RUN_ID`）。`xllm` 已经认这两个变量，别的作业直接用它。动手前先看 `xllm` 能不能满足这里的要求（`json_schema`、指定 `llm.translate`、关掉自动换模型），能就不写 `model.ts`。
- **每日上限取 100 万和 Goal 周期预算一半的较小值。** 作业的用量计入周期预算（缺省 24 小时 200 万），预算用完 Goal 就不能推理，翻译不能把修故障的额度吃光。`OPENDAN_MODEL_BUDGET` 是本次运行还剩的额度，低于它就停。

### 4.8 重排主机（RH）

- 未登录（Q21）：对 `Accept: text/html` 的导航请求，先回一小段 HTML，尝试 `POST /sso_refresh` 后重载，不行就跳登录页（`redirect_url` 回到原地址）；别的请求仍是 401。LoginPage 的静默跳转扩到这个主机。
- `POST /_read`：同一种会话；校验 `Origin`；正文 ≤ 16 KiB 的 JSON 数组；写成 `_read/<yyyymmdd>/<ulid>.json`，带 90 天过期；每个会话每分钟最多 30 次（进程内计数）。别的写入都不开。

### 4.9 App

- 模式改成 `home | page | chat`，默认 `home`。`home` 不连屏幕、不 `ensure`，浏览器停着也能看。原来的 `reflow` 模式去掉，“只整理这一页”的兜底页在 `home` 里显示。
- **外壳和页面的边界**：地址栏、代办入口、同步时间、需要登录、升级提示、“看原文”都在外壳里，数据来自 `_data`，模板怎么改都不影响。页面自己也可以显示这些。
- 页面和 App 的消息（同源校验照旧）：页面 → App `site.ready`、`site.location {route, url?}`、`site.open {url}`（没收录的链接）、`site.add {url?}`；App → 页面 `site.navigate {route}`、`site.updated`。没收录的链接直接在 `page` 模式打开原站，不请 Agent。
- “重排”按钮在 `page` 模式的地址栏旁（Q13）。已收录就切到 `home` 的对应页。判断默认项用新的 `browser.page_hint {}`：服务端在当前标签页里数密码框、可见表单字段和文章标记，不用模型，返回 `reading | task | unknown`。
- 加入阅读：App 往浏览器会话发固定格式的消息（网址、标题、主人写的一句要求），站点首页显示“已请 Agent 接入”；Work 开始后由 Work 把来源写成 `preparing`。
- 来源管理（替换 LibrarySheet）：来源列表、状态、最近同步、暂停 / 恢复 / 重试 / 移除；“维护目标”一页复用改进单的 Goal 会话视图（`components/conversation/goals/`），再加最近的修订（`goals.changes`）和作业（`goals.jobs`）；阅读记录和浏览记录的开关与清除。
- 重试和立即同步直接调 `goals.job_run`，暂停和恢复调 `goals.job_set_enabled` 并改 `site.json` 里这一项的 `state`，都用主人的会话，不经过 Agent、不花 token。移除来源要动来源程序和维护目标，发消息请 Agent 做。
- 启动参数加 `route`、`url`、`upgrade`。

### 4.10 需要浏览器或登录的来源（Q10、Q11）

- 新 kRPC `browser.background_run {script_path, url, timeout_s}`，由一期的 `reflow-run` 一般化而来：服务端起 `agent-browser background-run`，开自己的后台标签页（`_job` 组），不取租约，不受用户占用影响，同时只跑一个，结束就关。脚本拿到 `{page, request, save}`，`request` 带浏览器 cookie，能不开页面取到就不渲染。
- 只为后台任务启动浏览器时不重开保存的标签页，跑完 60 秒没有别的事就停（不等 600 秒）。和 [内存 TODO](agent-browser-memory-todo.md) M1 一起考虑。
- 需要登录：`extract` 抛 `auth`，状态变 `needs_login`，作业暂停到主人处理；检查脚本发一条通知，链接打开 `page` 模式的登录页。主人登录后在来源页点“已登录，重试”。

### 4.11 记录与分析

- 阅读：模板在文章页上报 `{article, source, opened_at, dwell_ms, max_scroll}`（页面隐藏时停表，离开时 `sendBeacon`）。汇总作业 `site reading --compact` 每小时把 `_read/` 汇成 `reading.json`，并写一份给检查脚本看的摘要到作业的 `data/`。
- 浏览：`browser/visits.rs` 挂在 `on_event`，只在“用户在看、用户在操作、不在密码交接、不是 `_` 组”时累计，按网址（去查询串和片段）合并，存 AgentRoot 的 `browser/visits.jsonl`，90 天。kRPC `browser.visits {since}`（owner、Agent）、`browser.visits_clear`（owner）、开关 `[loader.browser] visits`。
- Goal 用它们做三件事：调模板参数和栏目顺序（Q17），给主人加来源的建议，决定深挖哪件事。
- 事件：`events.json` 是 `[{id, title, summary, articles: [{id, adds}], supplements: [{article, source_url, adds}], updated_at}]`。Goal 把已收录的文章归到事件，需要时去查后续和原始材料，作为 `supplement` 文章收进来，每条写清来源和“为之前的报道增加了什么”。总首页把有后续的事件排在主人上次读过的那篇下面。

### 4.12 Goal 的默认配置

- `objective`：让个人重排站保持站点定义规定的样子，并按主人的阅读情况改进选题。`healthy_state`：每个 `active` 来源的 `checked_at` 不超过 3 个周期；主人读到的文章都有译文和配图；没有未处理的失败。
- 检查时间表 300 秒，脚本由 Work 从手册里的范本装上（Goal 之后可以改）。返回 `run: true` 的条件：某作业连续两次 `program` 失败且没报过；某来源 `checked_at` 过期而作业没有失败记录；每天一次且阅读摘要有明显变化。`needs_login` 只发通知。
- goal skill `reading_site_ops`，`scope.workspace` 是站点工作区。
- 建立的命令照 work skill `goal_handover`：`goal create --lifecycle maintain … --workspace <id> --skill reading_site_ops=… --check check.ts --every 300`，不带 `--start`，再逐个 `goal job put`。历史用缺省的 `fresh`。

## 5. 任务

### 5.0 S0：试做与契约（先做，决定后面的数字）

- [x] **S0-0 核对基线。** 改进单已提交（`5b197b11e`），现在就可以做：重新核对 §3.3、§3.4、§3.7 的行号和说法，读一遍 `goal_handover`、`goal_check_script`、`goal_upkeep` 和改过的 `goal.toml`、`analyze_intent.toml`，确认 §4.2–§4.4、§4.12 和它们不冲突，冲突的写进 §11。
- [ ] **S0-1 试做。**（2026-11-25：没有按原样做，改用产品代码实测，翻译的数字仍没有；见 §11） 不接产品代码，在 Agent 容器（Deno 2.9.2）里手写雅虎日本、NHK、Hacker News 三个 `extract.ts` 和一个最小模板，数据手工 PUT 进测试 Zone 的桶，手机上看。
  - 记录：每篇翻译的 token 和耗时（`llm.translate`）、一次同步的耗时、图片大小、JSON 大小、首页在弱网下的体积。结果写进 §3，修正 Q9 的默认值。一篇文章顺带按 §4.5 组一个 FeedObject，量多出来的一次 PUT 和体积。
  - 2026-11-24 本机 Zone 没有 Agent 和 aios 镜像，AICC 的别名都路由到 `qwen3.7-plus`（11-25 看到 Docker 里有 11-23 构建的 `local/aios-test`，Deno 2.9.2 的部分可以直接用它）：抓取、解析和页面可以先在本机的 Deno 上做，桶和翻译用主人的会话，容器里的部分等部署后补；记下翻译实际用的模型，成本和质量只对它成立。
- [ ] **S0-2 抓取稳定性。**（没有跑） 三个来源按 30 分钟跑 48 小时，记失败、限流、304 的比例。
- [x] **S0-3 读 owner 的答复**，改 §4。2026-11-24、11-25 已按答复改（§1.4），没有剩下的。
- [x] **S0-4 冻结契约。** 新建 `doc/opendan/protocol/Personal Site Protocol.md`（§4.5 含 FeedObject 外壳、§4.6、页面消息、`/_read`），改 `Agent Browser Protocol`（`background_run`、`page_hint`、`visits`、§5 的变化）。（`Agent State Protocol` 的作业和权限表已随改进单写好。）三份 JSON 各一套 fixture，TS、Rust、Desktop 共用。现在就可以做。

### 5.1 GJ：Goal 作业

**已由改进单 §5.9 与 RT1（GJ7）完成，已提交（`5b197b11e`）。下面的清单留作对照（改进单还在引用它），不再排期。**

- [ ] **GJ1 存储与安装。** `lib_opendan/src/goal_jobs.rs`（新）：定义、版本目录（`fsutil::publish_dir`）、回退、删除、限额（目录 ≤ 20 MiB、单 Goal ≤ 64 个作业）。
  - 验证：单测覆盖安装、重装、回退、并发安装、损坏的 `job.json`。
- [ ] **GJ2 运行器与调度。** 起进程、环境、超时、重试、并发上限、重启恢复、生命周期。能和 `sync.rs` 共用的（槽计算、记录裁剪）提出来共用，不复制。
  - 落点：`loader.rs:462` 附近启动；`host/goals.rs` 在休眠、归档处结束在跑的作业。
  - 验证：可控时钟的单测；`kill -9` 宿主后重启能看到 `killed` 并补跑；休眠时在跑的作业被结束。
- [ ] **GJ3 命令、kRPC、权限。** `xagent_cli.rs` 的 `goal job`，`state/krpc.rs`、`opendan/src/service.rs`；只读 Agent 不能写。
  - 验证：权限表逐格测；帮助文字；`agent-session goal job put` 从真实会话装一个作业并跑通。
- [ ] **GJ4 检查脚本与 Goal 头。** `ctx.jobs`（`goal_check.rs:415`）、`<jobs>`（`runner/goal.rs`）。
- [ ] **GJ5 用量与预算。** `OPENDAN_USAGE_LOG`、`OPENDAN_MODEL_BUDGET`；`goal_tokens_since`（`goal.rs:91`）把作业的 `usage.jsonl` 加进来，自动进入闸门、准入、Agent 总量和 Goal 头；`agent_tool/src/xllm.rs` 认这两个变量。
  - 验证：作业用量让周期预算用完，Goal 不再推理，作业的模型步骤以 `budget` 结束、其余照跑。
- [ ] **GJ6 Agent 主页。** `src/frame/opendan/web` 的 GoalDetail 加作业面板：状态、最近运行、日志、启停、立即运行、回退。
- [ ] **GJ7 定时输入不堆积。** Goal Session 还有没消费的 `scheduled` 输入时，`host/goals.rs:989` 不再投新的，只累加 `missed`。
  - 验证：Work 占用工作区 3 小时，放行后只打开一次 Activation。

### 5.2 WG：Work 开发、Goal 运维

**已由改进单 §5.4 完成，已提交（`5b197b11e`）；WG7 的真实 Zone DV 是改进单的 V10，没有跑。下面的清单留作对照（改进单还在引用它），不再排期。**

- [ ] **WG1 权限。** §4.3。`state/goals.rs`、`state/krpc.rs:157`、`protocol/goal.rs`、schema、`Agent State Protocol` §9.5。
  - 验证：改 `goal_session.rs:1248`；新增：UI 派的 Work 可以、Goal 派的不行、Todo 不行、群 UI 不行、`/new` 后的 owner UI 可以、Work 建的 Goal 通知能找到对话。
- [ ] **WG2 命令行。** `goal create / revise` 补字段，`--check-script` 在创建时装检查。`ui_core/references/sessions.md` 同步。
- [ ] **WG3 意图分析。** 先建设后维护的请求建 Work，任务书写明“做完建立 / 改写哪个 Goal”；纯观察和提醒仍直接建 Goal；小改动（Q15）直接 `goal revise` / `site config set`，结果用新的 `goal_revised` 或现有 `forwarded`（选改动小的）。`analyze_intent.toml`、`reports.rs:731`、`tests/notify.rs:269`。在 Q23 的那份 TODO 之后改。
- [ ] **WG4 Goal 提示词。** `goal.toml`：运维范围、先改工作区再安装、动手前看有没有主人的 Work、优先用交接来的工具。`goal_skills/goal_delivery` 同步。
- [ ] **WG5 升级请求。** §4.4 的约定写进 `goal.toml`、`analyze_intent.toml`、`ui_core`。不加机制。
- [ ] **WG6 文档。** 架构 §7.10 按实现更新，§15.7 回写，§8.3 补一句（Q19）。
- [ ] **WG7 DV。** 用一个和浏览器无关的例子（例如“做一个每小时检查我的几个网站是否可用的状态页，以后一直维护”）跑通：Work 建成 → Goal + 作业 → 故障唤醒 Goal 修好 → 请求升级 → 主人同意 → Work 升级。证明是通用能力。

### 5.3 ST：站点数据与 `site` 命令

- [x] **ST1 数据层。** `lib/site/store.ts`：读写、If-Match、规范化网址和 id（复用 `lib/reflow.ts`，一期向量继续用）、schema 校验。PUT 要拿回 `obj_id`，照 `lib/bucket.ts` 的写法。
- [x] **ST2 取数环境。** `lib/site/extract.ts`：`ctx`、条件请求、限速、`linkedom`（Q22 已同意）。
  - 打包：源码的 `deno.json` 写 `"linkedom": "npm:linkedom@0.18.12"`；构建时用 `deno bundle` 打成 `vendor/linkedom.mjs`，并像 `../../frame/` 那几项一样改写 `imports`。`_prepare_agent_tools`（宿主副本）和 `build_aios`（镜像）都要做，共用一段，不各写一份。来源包经 `ctx.html / ctx.xml` 用它，不自己 import。`linkedom` 和它依赖的许可证文本放在 `vendor/` 里，随产物带上。
  - 没验：`deno bundle` 在 macOS、Windows 的构建机上；`aicc-tool.ts` 的启动时间有没有变（只有 `site` 用到它，按需 import）。
  - 验证：断网的容器里 `site try` 用 fixture 能跑；产物里没有 `npm:` 说明符。
- [x] **ST3 流水线。** §4.6 的七步。评论按 Q25。
  - 验证：每条核对规则一个 fixture 用例；半途超时后再跑能续上；两个来源同时跑互不影响。
- [x] **ST4 翻译与归类。** §4.7、Q7、Q8。
  - 验证：mock AICC 覆盖长度不符、超额、无模型、标签不在清单里；用量写进 `OPENDAN_USAGE_LOG` 后 `goal stats` 的作业一项对得上；真实模型抽查日 → 中、英 → 中、德 → 英各 5 篇，人工看质量。
- [x] **ST5 图片。** 缩放、内容寻址、无 ext-core 的降级、失败重试。
- [x] **ST6 状态。** `feed.status` 的每种状态转换，`checked_at` 只在通过核对时更新。
- [x] **ST7 发布与清理。** 先内容后 feed；保留期（Q9）；没有引用的图片。
- [x] **ST8 命令。** `site` 的全部子命令、帮助、退出码、`site_test.ts`。
- [x] **ST9 配额。** `control_panel.rs:57` 和它的测试、协议 §5.1；写明已有 Zone 怎么补（`services/control-panel/spec`）。
- [x] **ST10 FeedObject 外壳**（Q3、Q26，§4.5）。`lib/site/feed.ts`：类型、译文转 Markdown、按 `validate_feed_object` 的规则校验、用 websdk 的 `buildNamedObjectByJson` 算 ObjId。不改 HomeStation 的产品代码，只在它的测试里加一个读共用向量的用例；不读、不复用 `spider.rs`。
  - 验证：共用向量（最小、满 9 图、8000 字符边界、更正带 `base_on`、非法的各一份）在 Deno 和 `cargo test -p homestation` 两边的 ObjId 与校验结论一致；真实 Zone 上任取一篇，`wraps`、`cover` 的 ObjId 能从 named store 读到内容。

### 5.4 TP：默认模板

- [x] **TP1 默认模板。** 放 `src/tools/buckyos-agent/site-template/`，`site init` 装它。手机单栏、深浅色、系统字体、不引外部资源、无构建步骤。总首页、来源页、文章页（译文 / 原文切换、看原文）、事件页；空站；各种来源状态。界面文字跟站点语言。
  - 文本一律 `textContent`；链接只放 `http(s)`；没收录的链接发 `site.open`。
- [x] **TP2 体积与弱网。** 首屏 ≤ 100 KB（不含图）；feed 用 ETag；图片懒加载。
- [x] **TP3 阅读上报。** §4.11，离线或失败不影响阅读。
- [x] **TP4 版本与回退。** `site template put / use`。
- 验证：Playwright（手机视口）对着 fixture 数据测每个页面和状态；375 px 宽无横向滚动；标题里带 `<script>` 的 fixture 不执行。

### 5.5 RH：重排主机

- [x] **RH1 登录跳转。** `nfs_server/src/bucket/reflow.rs:111`、`LoginPage.tsx:164`。验证：改 `tests/reflow.rs` 的 401 用例；真机浏览器直接打开、15 分钟后再打开。
- [x] **RH2 `POST /_read`。** `reflow.rs:40`、`:109`，`auth.rs:65` 的注释，协议 §5.4。验证：Origin 不符、超大、限速、别的方法仍是 405、写不到别的前缀。

### 5.6 AP：App

- [x] **AP1 首页模式。** `launch.ts`、`store.ts`、`AgentBrowserShell.tsx`；默认 `home`，不启动浏览器。验证：mock 断言打开 App 后没有 `browser.ensure`；改 `agent-browser.spec.ts:105`、`:380`。
- [x] **AP2 外壳状态条。** 当前来源的同步时间、需要登录、升级、只读 Agent 说明、维护 Agent 不是当前 Agent 时的说明（Q6）。
- [x] **AP3 重排按钮与选择。** 含 `browser.page_hint`（opendan `browser/rpc.rs`）。
- [x] **AP4 加入来源。** 消息、等待显示、Work 卡片、完成后落到来源页；同一来源不重复请求。
- [x] **AP5 起点与书签导入。** 空站首页三个起点；`usePickFiles` + `DOMParser` 解析书签 HTML；勾选、去重、排队（Q14）。
- [x] **AP6 站内导航。** `site.*` 消息、返回、看原文、没收录的链接去原站、“只整理这一页”。
- [x] **AP7 来源管理。** 替换 LibrarySheet / PreferenceSheet；维护目标与修订历史（复用 `components/conversation/goals/`，不另做视图）；重试、暂停直接调 `goals.job_*`（§4.9）；记录的查看、清除、开关。
- [x] **AP8 升级请求。** 显示说明、升级 / 暂不、链接参数。
- [x] **AP9 代办入口。** PRD §四：首页的地址栏和“交代任务”；从当前网页交给 Agent；进度和补充在同一会话。大部分一期已有，逐条核对缺什么再补，不重做。
- [x] **AP10 多语言、mock、e2e。** 10 种语言；`mock/backend.ts` 加 `goals.*`、`site` 数据；手机视口用例。
- [x] **AP11 手机客户端。** BuckyOS APP 里打开、通知点进来落到对应页（`openAgentBrowser`、`launchPath.ts`）。

### 5.7 MN：手册

- [x] **MN1 work skill `reading_site`。** 收尾的通用做法是 `goal_handover`，这里指向它、不复制，只写重排站专有的。建站、接入、升级：怎么看懂原站、取数顺序（D1）、`site try` 的核对、在手机视口看效果、安装、建立 / 改写 Goal、交接说明、部分失败（Q16）、被拒和付费站（Q11）。`references/{extract,template,handover}.md`。
- [x] **MN2 goal skill `reading_site_ops`。** 检查脚本范本（用 `ctx.jobs` 和 `ctx.slot_ms`）、诊断和修复、先改工作区再安装、运维与升级的分界（Q5）、升级请求、阅读分析（Q17）、事件、通知的节制（Q18、Q24）。
- [x] **MN3 `browser` 手册。** 去掉包和库的部分，加个人站的入口、“加入阅读”消息怎么处理、小改动直接改（Q15）、“只整理这一页”。
- [x] **MN4 相关手册与表。** `ui_core`、`opendan_agent_dev`、AGENTS.md 的联动表（加个人站一行：`site` 命令、来源包、数据契约、FeedObject 外壳）。`goal_check_script` 的作业部分改进单已写。
- 验证：`the_built_in_skills_pass_the_package_check` 等用例；手册里的命令逐条跑。

### 5.8 RB：记录与分析

- [x] **RB1 阅读汇总。** `site reading --compact`，作业模板。
- [x] **RB2 浏览记录。** `browser/visits.rs`、kRPC、配置。验证：密码交接期间、Agent 操作期间、`_` 组不记；查询串不落盘。
- [x] **RB3 查看、清除、开关。** AP7 的后端。
- [x] **RB4 分析。** 按来源和 `tags` 汇总停留；检查脚本的“有明显变化”判定、Goal 一天最多一次、写回 `site.json` 里 Q17 说的几个参数。`hide_topics` 只由主人的话改，Goal 不自己改。

### 5.9 BR：需要浏览器的来源

- [x] **BR1 `background_run`。** `browser/reflow.rs` 的运行部分一般化，`agent-browser.mjs`，ext-core 升版。验证：用户正在操作、Agent 有租约、浏览器停着三种情况都能跑且互不影响。
- [x] **BR2 启停。** 只为后台任务启动时不重开标签页、尽快停。
- [x] **BR3 登录。** `needs_login` 全流程（§4.10）。
- [x] **BR4 样板。** 一个要渲染的站、一个要登录的站。

### 5.10 EV：事件与补充

- [x] **EV1 契约与命令。** `site events`。
- [x] **EV2 补充文章。** 出处、增加了什么；查后续的预算写在 MN2。
- [x] **EV3 页面。** 首页排序、事件页、文章页的“后续”。

### 5.11 CL：清理（Q20）

- [x] **CL1** 删 `reflow_refresh`、`reflow_action`、包的读写与测试、Desktop 的包管理、手册的包部分、`reflow put / refresh`。保留：桶、规范化向量、兜底页、`reflow.<zone>` 的权限测试。
- [x] **CL2** 一期 TODO、协议、PRD 方向文档标注被替代的部分。

## 6. 顺序与并行

**owner 定的顺序：Plan / work_simple（已提交 `a5831ec7d`）→ Goal Session 改进（代码已提交 `5b197b11e`，真实 Zone 没有验收）→ 本文。**

| 里程碑 | 内容 | 入口 | 出口 |
| --- | --- | --- | --- |
| M0 | S0 | 没有入口条件，现在就可以做 | 契约文档、实测数字 |
| M1 能读 | ST、TP、RH、AP1 / AP2 / AP6 | M0 的契约冻结；改进单的出口（它的 §6）：WG、GJ 完成并提交（已满足），V4、V10 在真实 Zone 上跑过（没有） | 开发者手工建 Goal、装一个来源和作业，手机上能连读，定时更新 |
| M2 会建 | MN1 / MN3、AP3–AP5 | M1 | 主人点“重排”，Agent 自己建好并交给 Goal |
| M3 会运维 | MN2、AP7 / AP8 | M2（改进单的 Desktop Goal 视图 UX1–UX3 已提交） | 故障自修、升级请求闭环 |
| M4 懂主人 | RB、BR、AP9 / AP11 | M3；Q12 跟 owner 确认过 | 登录来源、按阅读调整 |
| M5 收口 | EV、CL、MN4、DV、验收 | M4；Q20 跟 owner 确认过 | §7 全过 |

**改进单的进度（2026-11-25 看提交和它的 §11）：** 代码和文档在 `5b197b11e`。真实模型样例在测试进程里跑过（含 FX4、SI4、SI6 和 V10 的前半），**真实 Zone 一条都没跑**：要先用这次提交重建 Jarvis 包和 aios 镜像并部署（本机 Docker 里最新的 aios 镜像是 11-23 的），再在有 Agent 的 Zone 上跑 V4、V10。作业和 `xllm` 计量也没有在 Agent 容器里跑过，这两样是本文 M1 真正依赖的。

**可以先写、不能先算完成。** 改进单已提交，原来列的“合入之前不要动的文件”不再有限制。M0 全部可以开工；ST、TP、RH、AP1 / AP2 / AP6 不依赖 Goal 的真实行为，owner 要提前开工时从这几组开始。M1 的出口（定时更新）要等改进单部署并跑过 V4、V10；V10 在容器里暴露问题时，先回改进单修，不在本文绕开。

契约冻结后，ST（TS）、TP（页面）、RH（nfs_server）、AP（Desktop）可以各派一个子 Agent 并行。Rust 子 Agent 各用自己的 `CARGO_TARGET_DIR`；libopendan 的 `xagent_cli` 等用例依赖 `src/tools/bin`、`src/bin/agent-tools`，临时目录里失败不算。同机别的会话也在改同一棵树，开工前先看 `git status`，只动自己的文件，不要用 `git stash` 做基线。

## 7. 验收

| # | PRD 场景 | 通过条件 | 怎么验 |
| --- | --- | --- | --- |
| V1 | 首次使用，导入书签后再添加雅虎日本 | 首页可见多个来源；建站和接入各是一个 Work；同一个 Goal 维护；维护目标里看得到新来源；Goal 起步没有推理 | 真实 Zone + 真实模型 |
| V2 | 手机上连续点开两篇已收录的日文新闻 | 中文正文、配图、看原文；第二篇 2 秒内可读；没有新的会话推理 | 真机 |
| V3 | 后台刷新时打开首页 | 旧列表可读；新文章出现在列表时正文已可打开；同步时间可见 | 人为放慢第 5 步的 e2e + 真实 Zone |
| V4 | 来源稳定运行，主人没提新要求 | 连续 72 小时：新文章按频率出现；Goal 没有因例行更新而推理；`needs: http` 的来源没有启动浏览器；用量与预算页对得上 | 真实 Zone 长跑 |
| V5 | 原站小改 | 用可控的测试站改选择器：旧内容可读、同步时间停住、Goal 修好、工作区和安装版本一致、主人没有操作 | `test/agent_browser_dv` 的测试站 |
| V6 | 原站彻底改版 | 收到一条升级请求，看得懂；同意后 Work 升级，期间旧版本可读、别的来源照常；暂不处理时没有第二条通知 | 同上 |
| V7 | 按事件组织 | 首页按约定变化；后续有出处和“增加了什么” | 真实 Zone，人工看 |
| V8 | 办事网站 | 从当前页面交代、沿用登录态、看得见进度、同一会话补充 | 一期 DV 的表单站 + 真机 |
| V9 | 断网与故障 | 原站不可达、浏览器停着、Agent 停用，已收录内容都能读 | e2e + 真实 Zone |
| V10 | 通用性 | 移到改进单的 V10 | — |
| V11 | 低配 | 4 GiB VM 上 10 个 `http` 来源长跑，内存增量 ≤ 300 MB | 测量脚本 |

## 8. 验证命令与 DV 注意

- Rust（`src/`）：`cargo test -p libopendan -- --test-threads=1`、`-p opendan`、`-p opendan --lib browser::`、`-p nfs_server`、`-p buckyos-api`、`-p homestation`（ST10）。Goal 与作业：`-p libopendan --test goal_session --test goal_jobs`。手册：`-p libopendan --lib skills`、`--test xagent_cli skills_are`、`--test xagent_lib work_sessions_take`、`-p opendan --test loader the_jarvis_package_loads_and_answers`。
- TS：`src/tools/buckyos-agent` 下 `deno test -A`。ext-core：`node --test publish/exttool/ext-core/browser/test/`。
- Desktop（`src/frame/desktop`）：`pnpm check`、`pnpm lint`、`pnpm exec playwright test tests/e2e/pages/agent-browser`、`desktop-i18n.spec.ts`；DV 用 `playwright.dv.config.ts` 和 `tests/dv/agent-browser*.spec.ts`。
- 部署：容器内的改动要 `./build_aios --local-test` 再 `uv run src/start.py --skip-update`；`agent-browser.mjs` 的改动要重建并重新播种 ext-core；先发 aios 镜像再发 opendan。
- 真实模型 DV：用专门的测试 Agent（重排 DV 会清浏览器配置目录）；测试 Goal 由脚本在拿到结果后自动归档，不要留着短硬保障过夜（一期烧过约 220 万 token）；手册描述会影响意图分流，改完先用便宜的消息试分流。
- 已知坑：新桶或改配额在已有 Zone 上不会自动生效；按端口分配第 5 号 App 是 10080，`fetch` 会拒；workflow 只在启动时探测媒体工具；本机 Zone 被重新激活过，测试 Agent 可能不在。

## 9. 已知风险

- 会话身份自报（Q2）：Agent 绕过权限在技术上可行，靠提示词约束。
- 作业以 Agent 身份运行、没有沙箱（Q1）：来源程序处理的是第三方内容，程序本身由 Agent 写；被诱导写出恶意程序的风险和 Agent 平时写代码相同。
- 站内所有页面同源：模板出问题可能读到或伪造阅读记录，`/sso_refresh` 会把会话令牌交给页面脚本（一期已有）。
- 原站条款和版权：内容只存在用户自己的 Zone、只给用户看；不绕过付费和反爬；频率保守。
- 翻译用量和 Goal 的推理共用周期预算：§4.7 的每日上限给推理留了一半，主人改小 Goal 预算时上限跟着变小，新文章会更晚出现。
- FeedObject 是跟着 HomeStation 的现有结构走的：它改字段或限制，ST10 的共用向量会失败，要一起改。私人抓取的约定还没有真正的使用者（D7），HomeStation 以后做抓取时可能改；影响只在外壳，站内主键和页面不受影响，重新组一遍即可。
- 令牌 10 分钟：慢的来源要靠分批和续跑，单个来源一次处理不完 20 篇时会跨多次。
- 翻译质量和成本取决于 Zone 里有什么模型；没有 `llm.translate` 可用模型时整个站不能更新，要在建站时就查出来。
- 依赖 Agent 进程在线：Agent 停用期间不更新（已有内容可读）。
- 阅读记录是隐私数据，进入 Goal 的推理上下文。

## 10. 不在本期

离线阅读和 PWA；视频下载与字幕（E2 的方向，数据模型不挡）；多人共享一个站；全文搜索；任意网页的常驻问答侧栏（PRD §五）；绕过付费墙；按订阅源自动发现新来源；把作业放到别的节点上跑；把文章送进 HomeStation（候选、收藏、分享）、合并两边的订阅列表、改 `spider.rs`（Q26 按建议，二期验收后的第二步再定）。

第二步要做时先看 §3.8 列的“装不下的地方”。HomeStation 真正做抓取时，建议直接用本文的来源包和作业产出的 FeedObject，不另写一套抓取（我的建议，没有裁决）。

## 11. 实施记录

| 日期 | 任务 | 记录 |
| --- | --- | --- |
| 2026-11-25 | S0-0 | 核对了 §3.3、§3.4、§3.7 和 `goal_handover`、`goal_check_script`、`goal_upkeep`、`goal.toml`、`analyze_intent.toml`：§4.2–§4.4、§4.12 和它们不冲突。新发现：`xllm` 没有关掉自动换模型的入口（`agent_tool/src/xllm.rs` 不带 `policy`），所以按 §4.7 写了 `lib/model.ts`，没有复用 `xllm` |
| 2026-11-25 | S0-4 | 契约冻结：新建 [Personal Site Protocol](<../doc/opendan/protocol/Personal Site Protocol.md>)、`doc/opendan/protocol/fixtures/site/`（6 份样例 + FeedObject 向量），Agent Browser Protocol 加 §7（`background_run`、`page_hint`、visits）和 §2.2 的五行。共用类型在 `src/tools/buckyos-agent/lib/site/types.ts`。**和 §4 不同的地方**（都已写进协议）：① `feed.status` 只由作业写，没有 `needs_upgrade`、`paused`；升级请求放 `site.json` 的 `upgrades[]`，来源的显示状态由 `site.json`、升级请求、feed 三处合成（一个文件一个写的人）。② 作业不暂停自己，`needs_login`、`failed` 的来源照时间表重试，主人登录后自动恢复，“已登录，重试”就是立即运行（少一个机制）。③ `ctx` 多了 `feed()`（RSS / Atom）和 `blocks()`（正文 DOM 变块），是产品代码，来源包不用各写一遍。④ 列表另出长边 480 的小图（`thumb`、`items[].image`）。⑤ `site.json` 多了 `reading.enabled`、`limits.translate_tokens_per_day`。⑥ `events.json` 最外层是带 `schema` 的对象；条目里的 `title`、`source`、`at` 由 `site events put` 填。⑦ 作为作业运行时 `site sync` 的最后一行是作业结果行，不是 AgentToolResult。⑧ 给检查脚本看的阅读摘要不写文件，放在汇总作业结果行的 `metrics.profile`（`ctx.jobs[].last_run.metrics` 直接读到）。⑨ 浏览记录的开关要能在 App 里改，多了 `browser.visits_set_enabled` 和 `state/browser/visits.json`；`[loader.browser]` 多了 `background_idle_stop_secs`。⑩ Markdown 里的图片写 `cyfile` ID（文章里不存原图网址）。⑪ 文章、Markdown、图片写入时带 181 天过期作兜底，来源被移除后留下的对象靠它清掉。⑫ 页面和 App 的消息多了 `site.session`（会话续不上，由 App 重新登录）和 `site.theme` |
| 2026-11-25 | S0-1、S0-2 | **没有按原样做。** S0-1 改成用产品代码实测：三个样板来源（雅虎日本、NHK、Hacker News）用 `site try --limit 3 --no-translate` 对真实网站跑通（见 ST 一行的数字）。翻译的 token 和耗时、桶里的对象大小没有数字：本机 Zone 是 `waterflier3.web3.buckyos.ai`，这台机器上没有它的 AppClient 私钥，`initRuntime` 起不来。S0-2 的 48 小时稳定性没有跑 |
| 2026-11-25 | ST1–ST8、RB1、EV1 | `lib/site/{store,extract,check,images,translate,pipeline,packages,reading,local,testing}.ts`、`lib/model.ts`、`lib/agent_service.ts`（从 `commands/reflow.ts` 提出来的 kRPC 调用）、`commands/site.ts`（全部子命令，含 `supplement put`）、`site-sources/{news-yahoo-co-jp,news-web-nhk,news-ycombinator-com}/`。`deno test -A lib/site/ lib/model_test.ts commands/site_test.ts` 86 过；`deno check aicc-tool.ts site-sources/*/extract.ts`、`deno lint` 干净；一期向量 20 过。实测（未翻译、有 `magick`）：雅虎 10 次请求 8.7 秒，文章 JSON 22–27 KB（含 10 条评论），6 张图共 381 KB；NHK 7 次 5.3 秒，3.3–3.7 KB；HN 12 次 7.2 秒，22–47 KB。实施里加的：`fixtures/index.json` 和 `site try --record`、`data/unlisted.json`（文章写了 feed 没写时不丢条目）、过期文章的墓碑、被拒的网址 24 小时内不重取、`_src/<id>/` 留最近三版。**没验**：真实桶（`bucket.put` 带 `http_headers`、If-Match 的 ETag、真实 `cyfile` ID）、真实翻译、`owner_did`（按 `control_panel/src/owner.rs` 读代码写的：先 `owner.context`，再 `user.get`，否则要 `--owner-did`）、`ctx.browser` 只对着模拟的 kRPC 测过。已知缺口：两个来源引用同一张图时，先过期的那个会把图删掉；websdk 的 `putBytes` 带不了 `Cache-Control`，不变的对象要多调一次 `bucket.put` |
| 2026-11-25 | ST2 打包、`magick` | `src/tools/finish_agent_tools_copy.py`（新）：把分发副本 `deno.json` 里的 `npm:` 依赖用 `deno bundle` 打成 `vendor/<名字>.mjs`，旁边放依赖的许可证文本（15 个包），去掉 `deno.lock` 和 `site-template/test`，源码里直接写 `npm:` 的拒绝。`_prepare_agent_tools` 和 `build_aios` 都调它。验证：`uv run python -m unittest discover -s tools/tests`（141 过，新增 3 条）；打出来的副本在 `local/aios-test`（Deno 2.9.2、`--network none`）里 `site try … --fixtures` 跑通、`deno check aicc-tool.ts` 通过（为此 `extract.ts` 里给 linkedom 的两个入口写了类型，打包后的 `.mjs` 没有类型）。`magick` 要运行权限：`publish/aios/aicc-tool` 和 `lib_opendan/src/baseline.rs` 的启动器在 `PATH` 上找得到 `magick` 时才加 `--allow-run=<路径>`（找不到时 Deno 每次启动都往 stderr 写一行提示，所以不能无条件加）。**没验**：`./build_aios --local-test` 整个镜像构建、macOS / Windows 的构建机 |
| 2026-11-25 | ST9、ST10 | 配额 1024 → 8192 MiB（`buckyos-api/src/control_panel.rs`，`cargo test -p buckyos-api control_panel` 4 过）。`lib/site/feed.ts`（外壳、Markdown、校验、ObjId）+ 向量 7 合法 13 非法，`deno test` 10 过，`cargo test -p homestation --test site_feed_vectors` 1 过，两边的结论和 ObjId 一致。ObjId 的字符串是 `<类型>:<64 位小写十六进制>`。`feedObjectProblems` 只收外壳的形状，比 HomeStation 严。**已有 Zone 怎么得到新配额**：没有迁移；改 system-config 的 `services/control-panel/spec`，把 `spec_config.buckets.reflow.quota_mib` 改成 `8192`（没有这一项就加 `"reflow": {"kind":"data","data_policy":"app_data","quota_mib":8192}`），nfs-server 60 秒内重读，下次访问那个用户的桶时生效，不用重启（读代码得出，没在真实 Zone 上做） |
| 2026-11-25 | TP1–TP4、EV3 | `site-template/`：`index.html`（4 KB）、`app.js`、`article.js`（打开文章才加载）、`app.css`、`i18n.js`、`lang/` 9 种语言。Playwright 77 过（Chromium，375×667），命令：`pnpm --dir src/frame/desktop exec playwright test -c ../../tools/buckyos-agent/site-template/test/playwright.config.ts`。**TP2 的预算只在一个条件下成立**：10 个来源分 4 栏时首屏 83.8 KB（不含图），10 个来源挤在一栏是 156 KB；真实 feed 有 100 条（约 52 KB），第一栏三个来源就超。原因是网关不压缩（`boot_gateway.yaml` 的 `node_gateway` 配了 `gzip_types` 但没有 `gzip: true`，cyfs-gateway 缺省关）：本机 gzip 后代码 48 → 17 KB，20 条的 feed 10.4 → 1.9 KB。**建议 owner 定要不要开网关压缩**（一行配置，影响全 Zone 的流量，没有改）。没测 WebKit；9 种语言的文案没有母语者看过 |
| 2026-11-25 | RH1、RH2 | `nfs_server/src/bucket/reflow.rs`：未登录的导航请求回一段固定的 HTML（先 `POST /sso_refresh`，不成跳 `https://<zone>/login?redirect_url=…`，同一地址 10 秒内再来就停下显示登录链接，防止循环）；`POST /_read`（顺序：会话 401 → `Origin` 的主机名 403 → 每用户每 60 秒 30 次 429 → 超过 16 KiB 413 → 形状 400 → 写入 204）。`LoginPage.tsx` 的静默跳转扩到这个主机。`cargo test -p nfs_server --test reflow` 9 过，`--lib reflow` 7 过，全 crate 过；`sso-login.spec.ts` 10 过。和 §4.8 不同：限速按用户不按会话；空数组、字段不对一律 400。实测发现 Firefox 上 `sendBeacon` 传字符串会带 `Origin: null` 被拒，模板要用 `application/json` 的 Blob（已写进协议 §7、模板第二轮改）。网关不用改（只读探测过运行中的 Zone）。**没验**：用新构建在真实 Zone 上直接打开 `https://reflow.<zone>/`、15 分钟后再打开、真实的 `POST /_read`、Safari、BuckyOS APP 客户端 |
| 2026-11-25 | AP1–AP6、AP8–AP10 | Desktop：`app/agent-browser/site/`（新，`controller.ts`、`data.ts`、`messages.ts`、书签解析和各个面板）、`panels/HomePanel.tsx`、`mock/site.ts`、`i18n/agent-browser-site.ts`，模式改成 `home \| page \| chat`、默认 `home`。`pnpm check` 过；`playwright test tests/e2e/pages/agent-browser` 69 过（含“打开 App 后没有 `browser.ensure`”）。固定格式的消息在 `site/messages.ts`（英文，不随界面语言）。基线就失败、不是这次引入的：`desktop-i18n.spec.ts` 8 条（`taskCenter.tasks.*` 三个键在 8 个 locale 里本来就缺）、`pnpm lint` 5 处（`RequestFields.tsx`、`SharePage.tsx`、`tests/dv/aiworkspace.integrity.spec.ts`）。**没验**：真实 Zone、真实模板（mock 里是一张说同样消息的替身页） |
| 2026-11-25 | BR1、BR2、RB2、`page_hint` | opendan：`browser/background.rs`（新，和重排刷新共用的启动器和名额、`browser.background_run`）、`browser/hint.rs` + `hint_page.js`、`browser/visits.rs`，`agent-browser.mjs` 加内部子命令 `background-run`，ext-core 1.3.3 → 1.3.4。`cargo test -p opendan --lib browser::` 86 过；全 crate 过；真实 Chromium（无头）18 过，另 3 条 `screen_tests::headed::*` 要 `Xvnc`，这台机器没有；Node 32 过（要写成 `node --test "…/test/*.test.mjs"`，Node 24 不认目录）。三种情况（用户在操作、Agent 有租约、浏览器停着）在真实浏览器上都跑通、互不影响。浏览记录在模块关着、没有浏览器时也能看和清除。**没验**：有窗口（虚拟显示器）模式、ext-core 1.3.4 的 payload 没有构建、真实 Zone。两条用例在 HEAD 上本来就不稳定：`screen_tests::what_pages_show_outside_themselves_is_asked_of_the_screen`、真实浏览器的 `a_headless_browser_is_what_it_is_and_refuses_passkeys` |
| 2026-11-25 | Q12、Q20 | owner 确认：阅读记录和浏览记录都默认开；一期的重排包这次就删 |
| 2026-11-25 | 第二轮 ST、RB1、RB4、EV1、EV2、CL1（TS） | `pending.reason` 加 `translate_failed`；文章、Markdown、图片带 181 天过期（图片再次被引用时顺延）；`site events put` 从文章里填 `title`、`source`、`at`（文章不在桶里是校验错误）；`site reading --compact` 同一次打开只算一次（作业在 `reading_state.json` 里留 90 天的打开记录），作业 `metrics` 是 `{records, opens_7d, dwell_ms_7d, articles_7d, profile}`。**`reflow` 精简**：`put`、`get`、`refresh`、manifest、包、结果、`preference.json` 都删了，剩 `match`、`ls`、`rm`、`result put <网址> <page.html>`；缺本地文件现在退出码是 5（原来误报 2）。`deno test -A lib/site/ lib/model_test.ts commands/site_test.ts` 89 过，`lib/reflow_test.ts lib/bucket_test.ts` 12 过，`deno check`、`deno lint` 干净。`magick` 被拒绝运行时和没装一样存原图（实测真 `magick`：出 1280 的大图和 480 的小图） |
| 2026-11-25 | BR3、BR4 | 测试站 `test/agent_browser_dv/site.mjs` 加了 `/live`（脚本跑完才有内容）、`/members`（没登录 303 到 `/login`）、`/__markup?v=1\|2\|3`（2 改类名、3 改结构和地址，给 V5、V6 用）。两个样板来源在 `test/agent_browser_dv/sources/{dv-live-news,dv-members}`。`browser_stub.mjs` 起无头 Chromium、用仓库里的 `agent-browser background-run` 应答 `browser.background_run`（本机没有 Agent Service 时的替身）。`node test/agent_browser_dv/sources_check.mjs` 10 步全过：渲染的来源发布 3 篇；会员来源没登录时是 `auth`、状态 `needs_login` 带 `login_url`，登录后发布 3 篇；改版 2、3 都让两个来源以 `program` 失败，改回 1 恢复。`ctx.browser` 的失败分类：`auth: ` 开头是 `auth`；没有浏览器、连不上、`net::ERR_…`、超时是 `transient`；其余（含等不到选择器）是 `program`。**没验**：真实的 Agent Service（租约、排队、令牌都没经过） |
| 2026-11-25 | 第二轮 TP | 上报用 `application/json` 的 Blob，只报主机会收的记录；页面可见时每 10 分钟 `POST /sso_refresh`；嵌在 App 里会话续不上时发 `site.session`、不重载；`site.theme` 覆盖系统明暗；事件页不再逐篇取文章；`translate_failed` 十种语言；测试服务器的 `/_read` 照 `reflow.rs` 的顺序和状态码。Playwright 85 过。首屏 85.9 KB（四栏）/ 158 KB（一栏）。已知：计时从页面加载算起，加载时会话已接近 15 分钟的话，第一次续期前的那次上报可能丢 |
| 2026-11-25 | CL1（opendan）、浏览记录权限 | 删 `browser.reflow_refresh`、`browser.reflow_action`、`reflow_no_package`、`agent-browser reflow-run`、`_reflow` 组；`browser/reflow.rs` 1550 → 460 行，`browser.reflow_state` 只回 `{url, site, page, fallback, view}`，只读 `fallback/<page>.json`。`browser.visits` 加了 `is_own_only`：只有 owner 和这个 Agent 自己能读（同一个 owner 的别的 Agent 不行；`background_run` 的那一类拦不住读写权限的别的 Agent）。`cargo test -p opendan` 全过（lib 109、loader 33 等）；真实 Chromium 17 过、3 条要 `Xvnc`；Node 31 过；ext-core 的 Python 检查过。`opendan/README.md`、Agent State Protocol §1（`visits.jsonl`、`visits.json`）已更新。`browser.background_run` 和 `acquire` 一样，同一个 owner 的读写 Agent 可以调别的 Agent 的（现有的通用规则，没有改） |
| 2026-11-25 | MN1–MN4 | 新手册：work skill `cmd_skills/reading_site/`（`references/{extract,template,handover}.md`）、goal skill `goal_skills/reading_site_ops/`（检查脚本范本是 `references/check.ts`，交接时复制、填两个常量）。`browser` 手册去掉了包（删 `references/site-package.md`、`preview.md`，`reflow.md` 只剩兜底页），加“主人的阅读站”一节和 `references/reading-site.md`：四条固定消息的第一行逐字引用（和 `site/messages.ts` 逐字节相同），小改动直接在对话里做。`ui_core`、`opendan_agent_dev` 各补一处，AGENTS.md 联动表改“Agent 浏览器”一行、加“个人重排站”一行。`behaviors/*.toml`、`agent.toml` 没有改：`reading_site` 能被意图分析选到，`--skill reading_site_ops` 能解析到内置手册。验证：`cargo test -p libopendan --lib skills` 14 过、`--test xagent_cli skills` 2 过、`--test xagent_lib work_sessions_take` 1 过、`-p opendan --test loader the_jarvis_package_loads_and_answers` 1 过；检查脚本 `deno check` 过、44 个手工 `ctx` 用例过，并经真实的宿主运行器（`xagent goal check run`）跑过；手册里 41 条 `site` / `reflow` 命令的参数都被接受（停在没有 Zone 身份）；交接的 `goal create / revise / job put / check set` 在开发用的 AgentRoot 上跑通。**没验**：真实模型的回放（意图分流没有被模型读过）、真实 Zone。手册作者定的、等 owner 过目：新来源的建议走升级请求（每周最多一条，不用 `questions`）；移除来源在对话里直接做、不开 Work；站点工作区固定叫 `reading-site`；Q24 的三种通知之外多一种“Zone 里没有翻译模型”（Q7 说要报给主人），由检查脚本发、每次故障只发一次 |
| 2026-11-25 | 写手册暴露的产品缺口 | 已补：`site` 没列进 `baseline::STANDARD_TOOLS` 和 `EXT_TOOLS_RESERVED_COMMANDS`（一条 libopendan 用例失败），并且 `commands/site_test.ts` 会被当成命令、镜像里多出一个 `site_test` 启动器 → 用例移到 `lib/site/command_test.ts`，两处清单加 `site`，`paios容器需求.md` 和 `OpenDAN Agent-Session-Runtime.md` 的命令表同步；汇总作业的 `--dir` 不能是空目录（`goal job put` 拒绝）→ 帮助和协议改成“有一个文件的目录”；通过的运行宿主不留 `kind`，检查脚本看不到“翻译被拒” → `site sync` 的 `metrics` 加 `pending`、`pending_reason`；Agent 读不到浏览记录 → `site visits [--days 7]`；新维护者取不回别人改过的模板 → `site template get <version> <dir>`；主人口头拒绝升级没有命令 → `site upgrade dismiss <id>`；`site events get` 的输出不能直接喂给 `events put` → `--out <file>`。`deno test -A lib/ commands/` 136 过。**没补，记为已知**：`ctx.jobs[]` 没有时间表和标题，检查脚本范本去读 `state/goals/<goal>/jobs/<job>/job.json` 和 `versions/<n>/source.json`（布局在 Agent State Protocol §9.9 里有）；`site try --fixtures` 不回放 `ctx.browser`，要浏览器的来源没有离线回归；来源的 `every_secs` 和作业的时间表是两个值，改频率要重新 `goal job put`；换维护者之后没有东西拦着两个 Goal 同时维护一个站，新 Agent 也归档不了旧 Agent 的 Goal（手册让它告诉主人）；`goal job put --dir` 会带上点开头的文件 |
| 2026-11-25 | CL2 | Agent Browser Protocol §5 重写成“兜底页 + 共用的桶、网址规则、主机”，包的内容删掉并注明由个人重排站接替；一期 TODO 和 PRD 方向文档各加一段说明 |
| 2026-11-25 | AP7、AP11、CL1（Desktop）、`site.session` / `site.theme` | 删掉包管理（`LibrarySheet`、`PreferenceSheet`、`packageFiles.ts`、站点偏好、`reflow_refresh` / `reflow_action` 的调用和界面、57 个不用的文案键）。新的“管理我的阅读”页（`site/ManagePanel.tsx`、`GoalTab.tsx`、`RecordsTab.tsx`、`manage.ts`）：来源列表与每个来源的操作（立即同步 / 已登录重试调 `goals.job_run`；暂停 / 恢复先调 `goals.job_set_enabled` 再带 If-Match 改 `site.json`；移除发固定格式的消息）；维护目标一页复用 `GoalConversation`，下面是 `goals.jobs` 和 `goals.changes`；记录一页写明记什么、只给维护的 Agent 用，两种记录各有开关、查看、清除。`maintainer` 为空或是别的 Agent 时操作置灰并说明。“只整理这一页”先弹一张可写一句要求的确认单（消息里是 `Note:`，不再有 `Preference:`）。`site.theme` 在 `site.ready` 的处理里先发再显示 iframe；`site.session` 重新走登录再回到当前路由，15 秒内再丢一次就显示重试而不是循环。AP11 缺的只有通知卡片的链接（原来会把整个 Desktop 载进 MessageHub 的页面），改成经 `openAgentBrowser` 打开。`pnpm check` 过；`playwright test tests/e2e/pages/agent-browser` 71 过；i18n 331 个键在 8 个 locale 里齐全。四条固定消息的原文在 `site/messages.ts`（加入、升级、只整理这一页、移除）。基线就失败：`desktop-i18n.spec.ts` 那 8 条、`messagehub-notify.spec.ts:259`（期望 “Agent”，zh-CN 词典是“智能体”，两个文件都没改过）。**没验**：真实 Zone（`goals.*`、visits 的形状是读 Rust 源码写的）、BuckyOS APP 客户端里打开（只读了代码）、`tests/dv/agent-browser.reflow.spec.ts` 重写了但没跑。App 作者定的、等 owner 过目：接入失败的来源点“再试一次”是重发加入的消息；删掉包管理后没有列出 / 删除兜底页的界面，桶里一期留下的包数据不显示也不删 |
| 2026-11-25 | 全量验证 | TS：`deno test -A lib/ commands/`（`src/tools/buckyos-agent`）136 过，`deno check aicc-tool.ts site-sources/*/extract.ts` 过；模板 Playwright 85 过；`node test/agent_browser_dv/sources_check.mjs` 全过。Rust：`cargo test -p opendan` 全过（lib 109、loader 33 等）；`-p nfs_server -p buckyos-api -p homestation` 35 组 331 过；`-p libopendan --no-fail-fast` 34 组 301 过，失败的两组是 HEAD 上就失败的（`fixtures` 5 条、`memory_replay a_jarvis_conversation_is_told_what_to_perceive`，后者的断言文本在 HEAD 的 `ui_core` 里就没有）；`state::registry::id_tests::concurrent_reservations_…` 在机器满载时失败过一次，单独重跑和空闲时整套都过。Desktop：`pnpm check`、`pnpm build` 过，`agent-browser` e2e 71 过。Python：`tools/tests` 141 过。**镜像**：`./build_aios --image local/phase2-check --arch amd64 --cargo-target-dir <新目录>` 通过（镜像里的 `deno check` 过；有 `site` 启动器、没有 `site_test`；`vendor/linkedom.mjs` 和许可证在、没有 `deno.lock` 和模板的 `test/`；挂上宿主的 SDK、`--network none` 时 `site try … --fixtures` 能写出数据）。第一次用 `--reuse-cargo-target` 失败是这台机器 `/tmp` 10 天清理删了缓存里的构建脚本产物，不是代码问题。**没跑**：`uv run buckyos-build.py` 整体构建、ext-core 1.3.4 的 payload、macOS / Windows |
| 2026-11-25 | 真实 Zone（ssotest 的 alice，`alice.web3.devtests.org`，主人的会话，不经过 Agent） | 把打包后的命令副本和 SDK 送进 VM，用 `auth.login` 拿主人的令牌注入运行。**已有 Zone 补桶声明的办法验证有效**：`buckyos --yes system-config get / set-file services/control-panel/spec` 加 `buckets.reflow` 后 20 秒内 `site` 就能用，不用重启。`site init`：`owner_did` 取到 `did:bns:alice`（主人的令牌；Agent 的令牌仍没验），模板装成 `t1`（13 个文件）。`site source put`、`site sync news-ycombinator-com --no-translate`：30 条列表、10 篇新的、6 篇发布，35 秒；`site.json` 的 If-Match 生效（ETag 是 `"cyfile:…"`）；文章的 `wraps`、`cover` 和 Markdown、图片对象的 `x-bucky-obj-id` 对得上，FeedObject 外壳是用真实 ID 组的。`reflow.<zone>` 上（页面会话）：`/` 是 `text/html`，`_data/*.json` 是 `application/json`、`private, no-cache`，`_app/t1/*` 是 `text/javascript` / `text/css`、`public, max-age=31536000, immutable`（`/bucket/v1` 上同样的对象是 `application/octet-stream`，那是数据面自己的规则）。**没有模型时**（这个 Zone 没有 provider）：`site sync news-yahoo-co-jp` 取到 8 篇、发布 0 篇，AICC 回 `no permitted model is available for this request (resource_not_permitted)`，归为 `translate_config`，退出码 0，作业结果行是 `{"kind":"budget","metrics":{…,"pending":8,"pending_reason":"translate_config"}}`，20 秒。**发现并修的**：正文里的 SVG 图标被当成“没取到的图”，文章要晚一轮才发布还带缺图占位 → 改成不收的格式当场丢掉。**没验**：翻译的 token 和耗时（没有模型）、Agent 的令牌、Goal 作业的调度、App（这个 Zone 没有 Agent） |
| 2026-11-25 | 真实 Zone 续（alice，换上新的 `nfs_server` 和 Desktop 之后） | 用无头 Chromium（390×844）经 SN 直接打开 `https://reflow.alice.web3.devtests.org/`，10 步全过：没有会话的导航请求得到登录用的 HTML → 跳到 `https://alice.web3.devtests.org/login?redirect_url=…` → 输入密码后回到个人站（RH1 和 `LoginPage` 的交接）；首页显示真实的来源和文章，没有横向滚动；文章页配图加载；离开文章时 `POST /_read` 回 204、`Content-Type: application/json`；带着会话在另一个标签页直接打开来源页；`[]`、对象回 400，超大回 413，`PUT` 回 405；页面没有脚本错误。之后 `site reading --compact` 把这条上报折进 `reading.json`（1 次打开、4.7 秒），作业结果行的 `metrics.profile` 是 `tags:;sources:news-ycombinator-com`。修过图片规则的命令再同步一次：之前等图的 4 篇连同新的 6 篇都发布，SVG 当场丢掉，`pending` 归零；`site config set`（If-Match）、`site events get --out` 也在真实的桶上跑过。截图在会话的临时目录（`reflow-dv/`），没有进仓库。**VM 的现状**：四台都已关机；301（alice）上 `bin/nfs-server/nfs_server` 和 `bin/control-panel/web` 是这次工作树的构建（旧的在旁边的 `nfs_server.bak-phase2`、`web.bak-phase2`），`services/control-panel/spec` 加了 `buckets.reflow`，alice 的重排桶里有一个带两个来源的测试站，`/root/phase2/` 是测试用的命令副本。**仍然没验**：有模型时的翻译、Agent 的令牌和只读 Agent、Goal 作业按时间表跑 `site sync`、检查脚本和通知、App 的 `home` / 管理页对着真实的 Agent Service、`browser.background_run` / `page_hint` / `visits` 经真实的 kRPC、有窗口的浏览器、真机和 Safari |
| 2026-11-25 | 等 owner 定或过目 | ① **网关要不要开压缩**（`boot_gateway.yaml` 的 `node_gateway` 加 `gzip: true`，并把 `text/javascript` 加进 `gzip_types`）：不开的话 TP2 的首屏预算只在第一栏不超过四个 20 条的 feed 时成立；影响全 Zone 的流量，没有改。② 手册作者定的四件事（MN 一行末尾）和 App 作者定的两件事（AP7 一行末尾）。③ 页面发给 App 的消息用 `postMessage(…, '*')`：任何能把 `reflow.<zone>` 嵌进去、又带得上会话的页面都读得到 `site.location`（标题和网址）；没有改，因为 App 的来源不止一种（Desktop、客户端壳），记作已知风险。④ `browser.background_run` 的脚本以 Agent Service 的系统身份运行（和一期的 `reflow-run` 一样），宿主运行时在 Linux 上是 root，不是 Node Executor 的账号。⑤ 登录页对这个主机显示的是 “Requested by nfs-server”，已登录而交接失败时表单上没有提示（要加文案键，没有做）。⑥ 实施过程里一个子任务为了找身份文件，把 `~/.buckycli/buckyos_boot.toml` 的一部分打印进了它自己的工具输出，里面有 AICC provider 的 API key；没有使用、复制或发往别处，只是进了那次会话的本地记录 |
