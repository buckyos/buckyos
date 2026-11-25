# Agent 浏览器二期：实施 TODO

日期：2026-11-24 · 源码基线 `1704812f8`（Goal 相关的事实以改进单合入后的代码为准，S0-0）· 状态：**owner 已答复（2026-11-24），未实施；还有两条等 owner 明确（§1.3）**

**2026-11-24：WG（Work 开发、Goal 运维）、GJ（Goal 作业）两组改由 [Goal Session 改进 TODO](opendan-goal-session-improve-todo.md)（下称“改进单”）完成，代码已实施未提交，实施记录在改进单 §11；真实 Zone 的 V10 没有跑。** 这两组的设计仍写在本文 §4.2–§4.4（和实现不同的地方在那里标注），契约以 [Agent State Protocol](<../doc/opendan/protocol/Agent State Protocol.md>) §9.4、§9.5、§9.9 为准。Q1、Q2、Q4、Q5、Q15、Q19、Q23 的答复在改进单的 Q16–Q19 和 §1.3，不用在这里再答。

**开发顺序（owner 2026-11-24）：改进单正在进行，本文排在它之后。** 入口条件、可以先做的部分和会冲突的文件见 §6。

需求来源：[二期需求](<../product/BuckyOS Agent浏览器 二期需求.md>)（下称 PRD）、架构设计 [§7.10](<../doc/opendan/OpenDAN Agent Session架构设计.md>)（Work 开发、Goal 运维），以及 2026-11-24 的三轮讨论（§2）。一期的实现与契约见 [一期 TODO](agent-runtime-browser-todo.md) 和 [Agent Browser Protocol](<../doc/opendan/protocol/Agent Browser Protocol.md>)。

## 0. 怎么用这份文档

**给 owner。** §1 的问题都已定，答复怎么落实见 §1.4。还要你明确的只有 §1.3 的两条，请直接写在“答复”后面。实施中出现的新问题写进 §11 再问你。

**给 CodeAgent。**

- §1 已定案：有答复的按答复和 §1.4 的落实，写着“未单独答复”的按建议。“另一种做法”都没有采纳，不要再加回来。
- §2 是已裁决的约束，不重新讨论。和代码现实冲突时停下来写进 §11，问 owner。
- §3 是查过、测过的事实，不用重查。标“没测”的才需要验证。行号以基线为准，动手前核对。
- §4 是设计。名字和字段可以调整，调整后回写本文和协议文档。**跨包契约先写协议文档再写代码**（S0-4）。
- §5 的任务做完一条，在 §11 记一行：改了什么、跑了什么、结果、没验什么。
- 遵守 [AGENTS.md](../AGENTS.md)：不做兼容实现；改动范围小；新依赖要 owner 确认（本文涉及的在 Q22）；手册和代码同一变更内更新。
- **机制越少越好**（owner 在 [Plan / work_simple TODO](jarvis-plan-work-simple-todo.md) D6、D9 的原则）。能用已有的命令、事件、提示词做到的，不加新机制。
- 不要提交，除非 owner 要求。

## 1. owner 决策（2026-11-24 已答复）

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
答复：（未单独答复。新依赖按 AGENTS.md 要明确确认，见 §1.3）

**Q23 和 Plan / work_simple TODO 的先后。** 它已裁决、未实施，会改 `analyze_intent`、`goal`、`work_simple`、`plan_simple` 的同一批文本。建议它先落地，本文的 WG3–WG5 在它之后改；别的任务不受影响。
答复：见改进单 Q16：Plan 单已先合入（a5831ec7d），改进单在它之后做完。

**Q24 有新内容时通不通知？** 建议例行更新不通知。只有三种通知：需要登录、升级请求、主人明确要跟踪的事件有了重要后续（每天最多一条）。影响 MN2。
答复：（未单独答复，按建议）

**Q25 评论收多少？** PRD 写“可见评论”。建议每篇最多 10 条，按原站的默认排序，一起翻译；接入时可以按来源关掉。影响 ST3。
答复：（未单独答复，按建议）

### 1.3 还要 owner 明确的两条

**Q22 新依赖 `linkedom`。** 内容见 §1.2。没有单独答复，但 AGENTS.md 要求新依赖明确确认，所以单列。不同意的话 `extract.ts` 只能用正则和字符串处理，改版时更容易取错。试做（S0-1）不进产品代码，先用着。影响 ST2。
答复：

**Q26 个人站和 HomeStation 是什么关系？**（由 Q3 的答复引出，事实见 §3.8）

- 现状：两边重叠。HomeStation 有自己的订阅源和抓取（RSS、网页），抓到的是“私人抓取”：没有入口、不进发表流的 FeedObject，经过候选、筛选进待读。它只存链接卡片和网页快照，不翻译、不整理正文。架构 §7.5 写明抓取可以由 Agent 实现，“协议只关心它能否产出合法 Feed Object 并写入候选缓存”。但服务端没有“收下一个外部抓取”的方法，Agent 的会话也只能操作 Agent 自己的 HomeStation，写不进主人的。
- **建议：本期只统一结构。** 每篇文章带一个合法的 FeedObject 和它的 ObjId（§4.5），HomeStation 不改。二期验收后再做第二步：HomeStation 加一个收下私人抓取的方法，主人在文章页点“分享 / 收藏 / 稍后读”，由 App 用主人的会话交过去，之后走它现有的候选、收藏和分享。两边的订阅列表不合并。
- 另一种做法：本期就让作业把新文章写进主人的 HomeStation 候选，HomeStation 成为同一批内容的另一个视图，它的 RSS / 网页来源逐步改由 Goal 抓取。要先定 Agent 写主人候选的授权、谁决定入列、两份订阅列表怎么合并，工作量大约再多一个里程碑。
- 影响：按建议本期只多 ST10；选另一种要加一组任务。
- 答复：

### 1.4 答复的落实（2026-11-24）

| 问题 | 你的答复 | 落实 |
| --- | --- | --- |
| Q1、Q2 | 见改进单 Q18、Q17 | 已实施。来源的例行更新就是 Goal 作业（§4.2、§4.6）；建站、接入、升级的 Work 由浏览器会话直接创建，不属于 Goal，因此能建立和改写 Goal（§4.3） |
| Q3 | 按建议，可以进一步考虑抓回来的文章封装成HomeStation的FeedObject的结构? | 数据和模板分开、放 `reflow.<zone>` 根，不变。**文章加一层 FeedObject 外壳**：形态和 HomeStation 的私人抓取一致，译文写成 Markdown 文件由它 `wraps`，封面和图片用桶里对象的 `cyfile` ID。站内页面仍读结构化的块，站内的主键仍是网址的哈希（§4.5、ST10）。我的理解：你说的是“结构”，所以本期不把文章送进 HomeStation，送不送是 Q26 |
| Q4、Q5、Q15、Q19 | 见改进单 Q19 | 通用约定已实施。本文只做站点专有的部分：来源页上的升级说明和 App 的按钮（AP8）、`site upgrade`（ST8）、运维与升级的分界在站点里的例子（MN2）、参数类小改动（MN3） |
| Q6 | 按建议 | 一个用户一个站，数据、模板、来源程序归用户；`site.json` 的 `maintainer` 记当前维护的 Agent 和 Goal；换 Agent 是一次升级（§4.5、AP2、MN1） |
| Q23 | 见改进单 Q16 | Plan 单已合入，改进单正在进行，本文排在最后（§6） |
| 其余 | 没有单独答复 | 按建议。Q12（浏览记录默认开）和 Q20（删一期的重排包）涉及隐私和删除已交付的功能，做到 M4、M5 时开工前先跟你确认一次 |

## 2. 已裁决的约束

### 2.1 本期（owner 2026-11-24）

| # | 原话 | 落实 |
| --- | --- | --- |
| D1 | goal session里会倾向于用脚本完成周期性工作。也就是说，在文档工作一段时间后，从原始站点->构造重排页的流程，主要是由脚本完成的（如果可以这些脚本甚至都不依赖无头浏览器） | “文档”按“稳定”理解。例行更新由作业完成，不推理；取内容的顺序是订阅源 / 接口 → 普通请求 → 浏览器 |
| D2 | 用户首次建立 “agent重排站”的时候，是一个普通的worksession, 后面的goal session主要是用来定期同步数据/分析用户的浏览行为。这个结构还是比较新的，相当于agent worksession可以对goal session的维护目标进行改写 | 建站、接入、升级都是 Work；Work 建立和改写 Goal |
| D3 | 通用规则。相当于传统的worksession基于workspace 维护一个项目，但需要用户说需求改了才会触发推理。而goal session的视角里，这个worksapce是活的（不是代码目录），需要定时完成运维任务。大问题请求用户开worksession做版本升级 | GJ、WG 两组按通用能力做，不写成只给重排站用 |
| D4 | 按建议，可以进一步考虑抓回来的文章封装成HomeStation的FeedObject的结构?（Q3） | 文章带 FeedObject 外壳（§4.5）。原话带问号，按“值得做，怎么做由整理者提”理解 |
| D5 | 此外开发顺序上，GoalSession的改进(buckyos/notepads/opendan-goal-session-improve-todo.md)任务已经在进行中了。 | 本文排在改进单之后（§6） |

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
| NHK、ITmedia、Ars Technica | RSS，有 ETag | HTML 里有正文 |
| Asahi | RSS，有 ETag | 只有约 800 字，部分付费 |
| BBC、DW、Spiegel、Le Monde、Guardian | RSS | HTML 里有正文；Spiegel、Ars Technica 的 RSS 带全文 |
| Hacker News | RSS | 链到各外站 |
| NYT、WSJ、Reuters | 前两个 RSS 可取 | 403 / 401，需要浏览器或只收列表 |
| 日经 | 首页可取，没有找到 RSS | 没测 |

- 解析：Deno 2.9.8 里 `linkedom@0.18.12` 解析 RSS 1–3 ms、300 KB 的页面 5–11 ms，进程约 83 MB，包缓存 12 MB。工作镜像的 Deno 是 2.9.2，没测。
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
- `buckyos-agent` 的共享模块靠 `_prepare_agent_tools`（`src/buckyos-build.py:209`）和 `build_aios:635` 复制进产物；命令以 `--allow-net --allow-read --allow-write --allow-env` 运行。

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
- 基线里 Desktop 没有任何地方调 `goals.*`，Goal 的页面只在 Agent 主页 WebUI（`src/frame/opendan/web`）。**改进单正在加（UX1–UX3，未提交）：** `components/conversation/goals/`（`GoalConversation`、`useGoalList`、`useGoal`、`source.ts`）和 Agent 面板的 `GoalList`。AP7 复用它们。作业的启停和立即运行有现成的 kRPC（`goals.job_set_enabled`、`goals.job_run`），owner 的会话可以直接调。
- Work 卡片：会话气泡里已有（`components/conversation/history/renderers.tsx:36`）。
- 没有书签解析。选文件用 `usePickFiles`（`sysdlg/select/pickFiles.ts:46`），照 `LibrarySheet.tsx:117` 的写法。
- 10 种语言：en、zh-CN 写在 `i18n/agent-browser*.ts`，其余 8 种在 `i18n/locales/*.json`；`desktop-i18n.spec.ts` 检查齐全。
- 默认模式变化会碰到 `tests/e2e/pages/agent-browser.spec.ts:105`、`:380`。

### 3.8 HomeStation 的 FeedObject（`src/frame/homestation/src`，2026-11-24 看代码，没有跑）

- 结构（`protocol.rs:144`）：`kind`、`publisher`（DID）、`iat`（秒）、`nonce?`、`entry?`、`content {type, title?, summary?, text?, cover?, media[{object, alt?}]}`、`wraps?`、`references[{relation, object_id}]`、`tags[]`、`source {kind, original_url, original_author?, captured_at_ms}`、`link?`、`base_on?`。不可变，ObjId = `cyfeed` + 规范化 JSON 的 SHA-256（`:314`）。没有语言和原文发布时间的字段。
- 限制（`validate_feed_object`，`:532`）：对象 ≤ 64 KiB；标题、摘要、正文合计 ≤ 8000 字符，更长的正文放进 `wraps` 的文件；媒体 ≤ 9；标签 ≤ 16；`wraps`、`cover`、`media` 只收 ObjId，不收网址；数字只能是整数（`:327`）。
- 私人抓取（`spider.rs:271`、`store_capture:299`）：`kind: post`，`publisher` 是主人的 DID，没有 `entry`，带 `source` 和 `link`，存成 `local_only`，进 `candidates`。选中后抓一份 HTML 快照，`wraps` 指向它，`content.type` 变成 `article`（`maybe_snapshot:339`）。架构 §5.6：私人抓取不进发表流，分享时才以主人自己的入口发表；cookie 一类的访问材料不进对象。
- 正文怎么显示：`item.wrapped_body`（`resources.rs:394`）读 `text/*` 或 `application/json`、单块、≤ 2 MiB 的文件（`evaluation.rs:655`），Desktop 当 Markdown 画（`ItemDetail.tsx:49`）。它用的 `MessageMarkdown` 没有图片分支。
- 按 ObjId 能从 Zone 的 named store 取到文件（`objects.rs:450`），所以桶里的图片和正文不用再传一份。
- 没有外部写入的入口：`handle_rpc` 要求调用方的 DID 就是这个 HomeStation 的主人（`api.rs:50`），Agent 的会话进的是 Agent 自己的 HomeStation（`host.rs:232`）；`publish.create` 不能带 `source`；`publish.share_capture` 只能分享已经在库里的私人抓取。
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

**FeedObject 外壳（Q3）。** `object` 是一个合法的 FeedObject，形态和 HomeStation 的私人抓取一致（§3.8），`object_id` 是它的 ObjId：

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
- 原站的评论不做成 FeedObject：评论者没有 DID。原文语言和发布时间没有对应的字段，留在文章里。
- **站内只用网址的哈希作主键**：路由、阅读记录、事件都用它，更正不会断开。`object_id` 是对外的身份，这一期没有消费方（Q26）。
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

- [ ] **S0-0 核对基线。** 改进单合入后重新核对 §3.3、§3.4、§3.7 的行号和说法，读一遍 `goal_handover`、`goal_check_script`、`goal_upkeep` 和改过的 `goal.toml`、`analyze_intent.toml`，确认 §4.2–§4.4、§4.12 和它们不冲突，冲突的写进 §11。
- [ ] **S0-1 试做。** 不接产品代码，在 Agent 容器（Deno 2.9.2）里手写雅虎日本、NHK、Hacker News 三个 `extract.ts` 和一个最小模板，数据手工 PUT 进测试 Zone 的桶，手机上看。
  - 记录：每篇翻译的 token 和耗时（`llm.translate`）、一次同步的耗时、图片大小、JSON 大小、首页在弱网下的体积。结果写进 §3，修正 Q9 的默认值。一篇文章顺带按 §4.5 组一个 FeedObject，量多出来的一次 PUT 和体积。
  - 本机 Zone 当天没有 Agent 和 aios 镜像，AICC 的别名都路由到 `qwen3.7-plus`：抓取、解析和页面可以先在本机的 Deno 上做，桶和翻译用主人的会话，容器里的部分等部署后补；记下翻译实际用的模型，成本和质量只对它成立。
- [ ] **S0-2 抓取稳定性。** 三个来源按 30 分钟跑 48 小时，记失败、限流、304 的比例。
- [x] **S0-3 读 Q1–Q6 的答复**，改 §4。2026-11-24 已按答复改（§1.4）；Q22、Q26 有答复时再改。
- [ ] **S0-4 冻结契约。** 新建 `doc/opendan/protocol/Personal Site Protocol.md`（§4.5 含 FeedObject 外壳、§4.6、页面消息、`/_read`），改 `Agent Browser Protocol`（`background_run`、`page_hint`、`visits`、§5 的变化）。（`Agent State Protocol` 的作业和权限表已随改进单写好。）三份 JSON 各一套 fixture，TS、Rust、Desktop 共用。

### 5.1 GJ：Goal 作业

**已由改进单 §5.9 与 RT1（GJ7）完成，代码未提交。下面的清单留作对照（改进单进行期间还在引用它），不再排期。**

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

**已由改进单 §5.4 完成，代码未提交；WG7 的真实 Zone DV 是改进单的 V10，没有跑。下面的清单留作对照（改进单进行期间还在引用它），不再排期。**

- [ ] **WG1 权限。** §4.3。`state/goals.rs`、`state/krpc.rs:157`、`protocol/goal.rs`、schema、`Agent State Protocol` §9.5。
  - 验证：改 `goal_session.rs:1248`；新增：UI 派的 Work 可以、Goal 派的不行、Todo 不行、群 UI 不行、`/new` 后的 owner UI 可以、Work 建的 Goal 通知能找到对话。
- [ ] **WG2 命令行。** `goal create / revise` 补字段，`--check-script` 在创建时装检查。`ui_core/references/sessions.md` 同步。
- [ ] **WG3 意图分析。** 先建设后维护的请求建 Work，任务书写明“做完建立 / 改写哪个 Goal”；纯观察和提醒仍直接建 Goal；小改动（Q15）直接 `goal revise` / `site config set`，结果用新的 `goal_revised` 或现有 `forwarded`（选改动小的）。`analyze_intent.toml`、`reports.rs:731`、`tests/notify.rs:269`。在 Q23 的那份 TODO 之后改。
- [ ] **WG4 Goal 提示词。** `goal.toml`：运维范围、先改工作区再安装、动手前看有没有主人的 Work、优先用交接来的工具。`goal_skills/goal_delivery` 同步。
- [ ] **WG5 升级请求。** §4.4 的约定写进 `goal.toml`、`analyze_intent.toml`、`ui_core`。不加机制。
- [ ] **WG6 文档。** 架构 §7.10 按实现更新，§15.7 回写，§8.3 补一句（Q19）。
- [ ] **WG7 DV。** 用一个和浏览器无关的例子（例如“做一个每小时检查我的几个网站是否可用的状态页，以后一直维护”）跑通：Work 建成 → Goal + 作业 → 故障唤醒 Goal 修好 → 请求升级 → 主人同意 → Work 升级。证明是通用能力。

### 5.3 ST：站点数据与 `site` 命令

- [ ] **ST1 数据层。** `lib/site/store.ts`：读写、If-Match、规范化网址和 id（复用 `lib/reflow.ts`，一期向量继续用）、schema 校验。PUT 要拿回 `obj_id`，照 `lib/bucket.ts` 的写法。
- [ ] **ST2 取数环境。** `lib/site/extract.ts`：`ctx`、条件请求、限速、`linkedom`。依赖随 `_prepare_agent_tools` 和 `build_aios` 打包，离线可用（Q22）。
  - 验证：断网的容器里 `site try` 用 fixture 能跑。
- [ ] **ST3 流水线。** §4.6 的七步。评论按 Q25。
  - 验证：每条核对规则一个 fixture 用例；半途超时后再跑能续上；两个来源同时跑互不影响。
- [ ] **ST4 翻译与归类。** §4.7、Q7、Q8。
  - 验证：mock AICC 覆盖长度不符、超额、无模型、标签不在清单里；用量写进 `OPENDAN_USAGE_LOG` 后 `goal stats` 的作业一项对得上；真实模型抽查日 → 中、英 → 中、德 → 英各 5 篇，人工看质量。
- [ ] **ST5 图片。** 缩放、内容寻址、无 ext-core 的降级、失败重试。
- [ ] **ST6 状态。** `feed.status` 的每种状态转换，`checked_at` 只在通过核对时更新。
- [ ] **ST7 发布与清理。** 先内容后 feed；保留期（Q9）；没有引用的图片。
- [ ] **ST8 命令。** `site` 的全部子命令、帮助、退出码、`site_test.ts`。
- [ ] **ST9 配额。** `control_panel.rs:57` 和它的测试、协议 §5.1；写明已有 Zone 怎么补（`services/control-panel/spec`）。
- [ ] **ST10 FeedObject 外壳**（Q3，§4.5）。`lib/site/feed.ts`：类型、译文转 Markdown、按 `validate_feed_object` 的规则校验、用 websdk 的 `buildNamedObjectByJson` 算 ObjId。不改 HomeStation 的代码，只在它的测试里加一个读共用向量的用例。
  - 验证：共用向量（最小、满 9 图、8000 字符边界、更正带 `base_on`、非法的各一份）在 Deno 和 `cargo test -p homestation` 两边的 ObjId 与校验结论一致；真实 Zone 上任取一篇，`wraps`、`cover` 的 ObjId 能从 named store 读到内容。

### 5.4 TP：默认模板

- [ ] **TP1 默认模板。** 放 `src/tools/buckyos-agent/site-template/`，`site init` 装它。手机单栏、深浅色、系统字体、不引外部资源、无构建步骤。总首页、来源页、文章页（译文 / 原文切换、看原文）、事件页；空站；各种来源状态。界面文字跟站点语言。
  - 文本一律 `textContent`；链接只放 `http(s)`；没收录的链接发 `site.open`。
- [ ] **TP2 体积与弱网。** 首屏 ≤ 100 KB（不含图）；feed 用 ETag；图片懒加载。
- [ ] **TP3 阅读上报。** §4.11，离线或失败不影响阅读。
- [ ] **TP4 版本与回退。** `site template put / use`。
- 验证：Playwright（手机视口）对着 fixture 数据测每个页面和状态；375 px 宽无横向滚动；标题里带 `<script>` 的 fixture 不执行。

### 5.5 RH：重排主机

- [ ] **RH1 登录跳转。** `nfs_server/src/bucket/reflow.rs:111`、`LoginPage.tsx:164`。验证：改 `tests/reflow.rs` 的 401 用例；真机浏览器直接打开、15 分钟后再打开。
- [ ] **RH2 `POST /_read`。** `reflow.rs:40`、`:109`，`auth.rs:65` 的注释，协议 §5.4。验证：Origin 不符、超大、限速、别的方法仍是 405、写不到别的前缀。

### 5.6 AP：App

- [ ] **AP1 首页模式。** `launch.ts`、`store.ts`、`AgentBrowserShell.tsx`；默认 `home`，不启动浏览器。验证：mock 断言打开 App 后没有 `browser.ensure`；改 `agent-browser.spec.ts:105`、`:380`。
- [ ] **AP2 外壳状态条。** 当前来源的同步时间、需要登录、升级、只读 Agent 说明、维护 Agent 不是当前 Agent 时的说明（Q6）。
- [ ] **AP3 重排按钮与选择。** 含 `browser.page_hint`（opendan `browser/rpc.rs`）。
- [ ] **AP4 加入来源。** 消息、等待显示、Work 卡片、完成后落到来源页；同一来源不重复请求。
- [ ] **AP5 起点与书签导入。** 空站首页三个起点；`usePickFiles` + `DOMParser` 解析书签 HTML；勾选、去重、排队（Q14）。
- [ ] **AP6 站内导航。** `site.*` 消息、返回、看原文、没收录的链接去原站、“只整理这一页”。
- [ ] **AP7 来源管理。** 替换 LibrarySheet / PreferenceSheet；维护目标与修订历史（复用 `components/conversation/goals/`，不另做视图）；重试、暂停直接调 `goals.job_*`（§4.9）；记录的查看、清除、开关。
- [ ] **AP8 升级请求。** 显示说明、升级 / 暂不、链接参数。
- [ ] **AP9 代办入口。** PRD §四：首页的地址栏和“交代任务”；从当前网页交给 Agent；进度和补充在同一会话。大部分一期已有，逐条核对缺什么再补，不重做。
- [ ] **AP10 多语言、mock、e2e。** 10 种语言；`mock/backend.ts` 加 `goals.*`、`site` 数据；手机视口用例。
- [ ] **AP11 手机客户端。** BuckyOS APP 里打开、通知点进来落到对应页（`openAgentBrowser`、`launchPath.ts`）。

### 5.7 MN：手册

- [ ] **MN1 work skill `reading_site`。** 收尾的通用做法是 `goal_handover`，这里指向它、不复制，只写重排站专有的。建站、接入、升级：怎么看懂原站、取数顺序（D1）、`site try` 的核对、在手机视口看效果、安装、建立 / 改写 Goal、交接说明、部分失败（Q16）、被拒和付费站（Q11）。`references/{extract,template,handover}.md`。
- [ ] **MN2 goal skill `reading_site_ops`。** 检查脚本范本（用 `ctx.jobs` 和 `ctx.slot_ms`）、诊断和修复、先改工作区再安装、运维与升级的分界（Q5）、升级请求、阅读分析（Q17）、事件、通知的节制（Q18、Q24）。
- [ ] **MN3 `browser` 手册。** 去掉包和库的部分，加个人站的入口、“加入阅读”消息怎么处理、小改动直接改（Q15）、“只整理这一页”。
- [ ] **MN4 相关手册与表。** `ui_core`、`opendan_agent_dev`、AGENTS.md 的联动表（加个人站一行：`site` 命令、来源包、数据契约、FeedObject 外壳）。`goal_check_script` 的作业部分改进单已写。
- 验证：`the_built_in_skills_pass_the_package_check` 等用例；手册里的命令逐条跑。

### 5.8 RB：记录与分析

- [ ] **RB1 阅读汇总。** `site reading --compact`，作业模板。
- [ ] **RB2 浏览记录。** `browser/visits.rs`、kRPC、配置。验证：密码交接期间、Agent 操作期间、`_` 组不记；查询串不落盘。
- [ ] **RB3 查看、清除、开关。** AP7 的后端。
- [ ] **RB4 分析。** 按来源和 `tags` 汇总停留；检查脚本的“有明显变化”判定、Goal 一天最多一次、写回 `site.json` 里 Q17 说的几个参数。`hide_topics` 只由主人的话改，Goal 不自己改。

### 5.9 BR：需要浏览器的来源

- [ ] **BR1 `background_run`。** `browser/reflow.rs` 的运行部分一般化，`agent-browser.mjs`，ext-core 升版。验证：用户正在操作、Agent 有租约、浏览器停着三种情况都能跑且互不影响。
- [ ] **BR2 启停。** 只为后台任务启动时不重开标签页、尽快停。
- [ ] **BR3 登录。** `needs_login` 全流程（§4.10）。
- [ ] **BR4 样板。** 一个要渲染的站、一个要登录的站。

### 5.10 EV：事件与补充

- [ ] **EV1 契约与命令。** `site events`。
- [ ] **EV2 补充文章。** 出处、增加了什么；查后续的预算写在 MN2。
- [ ] **EV3 页面。** 首页排序、事件页、文章页的“后续”。

### 5.11 CL：清理（Q20）

- [ ] **CL1** 删 `reflow_refresh`、`reflow_action`、包的读写与测试、Desktop 的包管理、手册的包部分、`reflow put / refresh`。保留：桶、规范化向量、兜底页、`reflow.<zone>` 的权限测试。
- [ ] **CL2** 一期 TODO、协议、PRD 方向文档标注被替代的部分。

## 6. 顺序与并行

**owner 定的顺序：Plan / work_simple（已合入 `a5831ec7d`）→ Goal Session 改进（进行中）→ 本文。**

| 里程碑 | 内容 | 入口 | 出口 |
| --- | --- | --- | --- |
| M0 | S0 | S0-1、S0-2 现在就可以做（不碰产品代码）；S0-0、S0-4 等改进单合入 | 契约文档、实测数字 |
| M1 能读 | ST、TP、RH、AP1 / AP2 / AP6 | 改进单的出口（它的 §6）：WG、GJ 完成并提交，V4、V10 在真实 Zone 上跑过；Q22 有答复 | 开发者手工建 Goal、装一个来源和作业，手机上能连读，定时更新 |
| M2 会建 | MN1 / MN3、AP3–AP5 | M1 | 主人点“重排”，Agent 自己建好并交给 Goal |
| M3 会运维 | MN2、AP7 / AP8 | M2；改进单的 Desktop Goal 视图（UX1–UX3）已合入 | 故障自修、升级请求闭环 |
| M4 懂主人 | RB、BR、AP9 / AP11 | M3；Q12 跟 owner 确认过 | 登录来源、按阅读调整 |
| M5 收口 | EV、CL、MN4、DV、验收 | M4；Q20 跟 owner 确认过 | §7 全过 |

**改进单的进度（它的 §11，2026-11-24 23:16）：** 代码全部写完，没有提交、没有部署；真实模型样例在测试进程里跑过一部分，SI4、SI6、V10 的回放结果还没填。真实 Zone 一条都没跑：本机 Zone 当天没有 Agent，也没有 aios 镜像。作业和 `xllm` 计量没有在 Agent 容器里跑过。本文 M1 真正依赖的是提交、部署、FX4 和 V10：FX4 证明 Work 收尾能交出 Goal，V10 证明作业在真实 Zone 能跑。UX1–UX3（Desktop 的 Goal 视图）只影响 M3。

**它合入之前不要动的文件：** `lib_opendan/`、`opendan/src/{loader,service,home,config,overview}.rs`、`opendan/web/`、`jarvis_agent/agent/`（behaviors 和全部手册）、`agent_tool/src/xllm.rs`、AGENTS.md、`Agent State Protocol.md`、架构设计、Desktop 的 `app/messagehub/`、`components/conversation/`、`i18n/locales/*.json`。因此 MN 全组、AP3 / AP7 / AP10、RB2、BR1 一定要等。ST（`src/tools/buckyos-agent`）、TP、RH（`nfs_server`、`LoginPage.tsx`）、AP1 / AP2 / AP6（`app/agent-browser`，翻译文案最后加）和它没有文件冲突，owner 要提前开工时从这几组开始；它们合在一起的 M1 出口仍要等改进单部署后才能跑。

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
- FeedObject 是跟着 HomeStation 的现有结构走的：它改字段或限制，ST10 的共用向量会失败，要一起改。
- 令牌 10 分钟：慢的来源要靠分批和续跑，单个来源一次处理不完 20 篇时会跨多次。
- 翻译质量和成本取决于 Zone 里有什么模型；没有 `llm.translate` 可用模型时整个站不能更新，要在建站时就查出来。
- 依赖 Agent 进程在线：Agent 停用期间不更新（已有内容可读）。
- 阅读记录是隐私数据，进入 Goal 的推理上下文。

## 10. 不在本期

离线阅读和 PWA；视频下载与字幕（E2 的方向，数据模型不挡）；多人共享一个站；全文搜索；任意网页的常驻问答侧栏（PRD §五）；绕过付费墙；按订阅源自动发现新来源；把作业放到别的节点上跑；把文章送进 HomeStation（候选、收藏、分享）和合并两边的订阅列表（Q26，有答复之前不做）。

## 11. 实施记录

| 日期 | 任务 | 记录 |
| --- | --- | --- |
