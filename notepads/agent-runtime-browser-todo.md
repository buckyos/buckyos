# Agent 浏览器：实施 TODO

状态：已实施（2026-11-15），未提交。实施记录、和本文不一致的地方、没做和没验证的在 §12；实施中新出现、等 owner 定的问题在 §9；跨包的接口契约落在 [Agent Browser Protocol](<../doc/opendan/protocol/Agent Browser Protocol.md>)，它比 §5 细，两者不一致时以它为准（§5.8 列了调整）。

后续（2026-11-16）：用户试用发现 Google 登录被拒；以“像普通浏览器一样使用、保持原生浏览器指纹”为目标的改造见 [普通浏览与登录兼容性 TODO](agent-runtime-browser-compatibility-todo.md)。该文规划后续变更，本文保留第一期裁决与实施记录。

日期：2026-11-14 立项，同日四轮裁决；当晚做了代码落点核对和容器实测，升级为实施 TODO；随后 owner 答复了 Q22–Q26。源码基线：buckyos `3b75db8f1`。

来源：

- owner 2026-11-14 的要求（§1.1）和对本文的六轮答复（§2）。
- [Agent 浏览器 PRD 方向](<../product/BuckyOS Agent浏览器 PRD方向.md>)（§1.2）。
- [打印 TODO](aiws-print-pdf-export-todo.md)、[渲染管线方案讨论](<../doc/workspace/BuckyOS AI Workspace 渲染管线方案讨论.md>)：系统渲染那部分的需求方。

## 0. 怎么用这份文档

写给接手实施的 CodeAgent。

- §2 的裁决是约束，不重新讨论。实施中发现裁决和代码现实冲突，停下来写进 §12，问 owner。
- §3 的落点带行号，行号以基线 `3b75db8f1` 为准，动手前先核对。
- §4 是已经测过的事实，不用重测。标了“没测”的才需要验证。
- §5 是接口契约。名字和字段可以在实施中调整，调整后回写本文，并同步 §6 里引用它的任务。
- §6 的任务每条都有落点、做法、验证。做完一条，在 §12 记一行：改了什么、跑了什么、结果。
- 遵守仓库的 [AGENTS.md](../AGENTS.md)：不做兼容实现；改动范围小；新依赖要确认（已确认的是 D20、D28 里那几个）；手册和代码同一变更内更新；没跑的检查要写明。
- 不要提交，除非 owner 要求。
- 主体在 `buckyos` 仓库。RF5 可能要动 `cyfs-ndn`。

## 1. 需求

### 1.1 owner 的要求

1. 现在 Agent 做前端开发经常需要 Playwright。
2. 希望 Agent 的私有 runtime 里默认有浏览器可用。
3. 引入浏览器后还需要接线。
4. 用户有需要时，可以通过类似远程桌面的方法观察和操作 Agent 的浏览器。
5. 这个“Jarvis's browser”可以成为一个 App，用户可以直接使用。
6. 技术类似虚拟机。以后 Agent 有私有的 Win11、macOS，也可以用这个技术栈让用户用到。

### 1.2 PRD 方向的要点

| # | 要点 |
| --- | --- |
| R1 | 浏览器运行在 Personal Server 上，网页资源由服务器加载。Personal Agent 主用，Owner 可以直接用。只有手机的用户也能从 BuckyOS 打开它 |
| R2 | Desktop 版已经能经 Computer Use 借用 Owner 宿主上的浏览器。Kiosk（无显示器盒子）上、只用手机的用户没有浏览器可借，这是要补的场景 |
| R3 | 需求来源有三个：Kiosk 与手机用户、BuckyOS 应用开发、AIWS 导出 PDF |
| R4 | 原始页面模式：原网页，原布局和交互。用户直接操作，需要时交给 Agent，Agent 从当前页面接着做。弱网时降低帧率 |
| R5 | 重排模式：按用户为这个网站设定的偏好重新呈现，经常浏览的网站沿用设定。重排页直接在手机上显示，传输量少。依赖原网页交互的动作由 Agent 在原网页执行，也可以切回原始页面 |
| R6 | 对话模式：对话和网页属于同一件事。Agent 找入口、操作网页，期间网页对用户只读。Agent 先预填，缺项在聊天框里问 |
| R7 | 按事情组织：一件事可以跨页面、跨网站。用户可以在几件事之间切换，回来接着办 |
| R8 | 待定的产品选择：默认入口、首批场景、协作控制的文案和样式 |

### 1.3 交付物

| 交付物 | 一句话 | 任务组 |
| --- | --- | --- |
| ext-core 里的浏览器 | 一份 Chromium、`playwright-core`、三条命令，五个平台 | EX |
| 开发测试可用 | Agent 在私有 runtime 里跑 Playwright 测试，不用自己装浏览器和系统库 | DT |
| Agent 的常驻浏览器 | Agent Service 管理一个无头 Chromium，带登录状态，容器和原生运行都支持 | BR |
| 屏幕和原始页面模式 | 画面和输入经 WebSocket 到 Desktop，手机上能看能点能输中文 | SC、APP |
| 交接 | 一方操作，双向显式交接，Agent 能请用户接手 | HO |
| Agent 的操作命令 | `agent-browser`：连上常驻浏览器跑脚本、读页面、截图 | AB |
| 「Agent 浏览器」App | 选 Agent、地址栏、事情列表、三种模式、对话面板 | APP、DG |
| 重排 | 重排库、按网址触发、刷新、LLM 兜底、固定地址上的重排页 | RF |
| 系统渲染 | Workflow 函数把页面输出成 PDF 或图片 | SR |
| 文档和手册 | 表 B、协议文档、`browser` 手册、DV | DX |

## 2. 裁决记录（owner 2026-11-14）

| # | 原问题 | owner 的答复 | 落到设计上 |
| --- | --- | --- | --- |
| D1 | PRD 的“Work Session” | 是讨论时的口头说法：用户在浏览器里干一件事。参加讨论的很多人不是 BuckyOS 的开发者 | 不是 OpenDAN 的 Work Session，对应见 D15 |
| D2 | 重排怎么生成 | 重排就是一个选择器：优先用匹配好的代码重排，否则用兜底的 LLM 重排 | §5.6 |
| D3 | 安全范围 | 有些时候一定要把浏览器的控制权交回用户，安全要求就是原因。第一期原型的范围集中在密码上，其他的能 Agent 填就 Agent 填（老网站的很多交互太灾难了） | §5.5。不做补填框 |
| D4 | Q1 两种用途分开处理 | 从减少安装包大小的角度，有方案能尽量兼顾两者么？安装后可以是两个独立的软件 | 包里一份 Chromium，安装后是 Playwright 测试环境和 Agent 浏览器两个软件（§5.1） |
| D5 | Q2 常驻浏览器跑在哪里 | 在 Agent 自己的 private runtime 里 | §5.2 |
| D6 | Q3 怎么分发 | 实现后进 ext-core 包（paios 镜像） | 并入 ext-core，不另立扩展包 |
| D7 | Q4 远程画面用什么 | 按建议 | CDP 截屏流加输入注入。屏幕接口保留，RFB 留给以后的虚拟机 |
| D8 | Q5 谁在操作 | 按建议 | 一方操作，双向显式交接（§5.4） |
| D9 | Q6 登录状态和可见范围 | 按建议，登录态就是正常的跟浏览器，谁能用这个浏览器谁有权限 | 不做额外的归属或隔离。默认只有 owner 能用 |
| D10 | Q7 “用户可以直接使用”的范围 | Owner 直接用的就是某个 Agent 的浏览器 | 没有不属于 Agent 的浏览器 |
| D11 | Q8 系统服务用的浏览器是否纳入本文 | 纳入本文，这也是一个实际需要完成的需求 | §5.7、SR |
| D12 | Q9 原生运行的 Agent | 原生运行也支持，当成普通工具。用户使用的方法还是通过 BuckyOS App，不会直接点开 | 五个平台都带浏览器，都无头运行。原话写的是“原始运行”，按“原生运行”理解 |
| D13 | Q10 浏览器种类 | 只做 Chromium | |
| D14 | Q11 名称 | 好 | App 叫「Agent 浏览器」，接口叫“屏幕” |
| D15 | Q12 “一件事”怎么落到 OpenDAN 上 | 按你的建议 | 一件事 = 一个 MessageHub 会话（一个 UI Session）加一组标签页。没交代事情时归默认会话 |
| D16 | Q13 Agent 怎么操作常驻浏览器 | 用最直接高效的方法操作即可 | Playwright 经 CDP 直连。不做 ComputerUse 的浏览器驱动（§5.3） |
| D17 | Q14 重排代码从哪里来、存在哪里 | 重排代码存放和触发是单独的逻辑，谁完成都可以（Agent 是主要的实现者，但用户自己写也行） | 重排库和触发不问作者 |
| D18 | Q16 网页内容和提交前确认 | 填，先能力再安全 | 提交前不另行确认。风险见 §10 |
| D19 | Q17 视口 | 好，按你的建议 | 跟随正在看的设备，可切桌面版。Agent 不改视口 |
| D20 | Q18 ext-core 加浏览器 | 按设计，ext-core 里的东西，paios 里都有的。带浏览器能解决 3 个需求，这个空间付出就是值得的了 | Chromium 和 `playwright-core` 两个新依赖随之确认 |
| D21 | Q19 系统渲染的形态和运行身份 | 按你的建议 | Workflow Service 的内置函数。不用 root 加关沙箱。实测后选了降权（§5.7、D29） |
| D22 | Q20 直连后密码和接手只靠手册 | 接受该风险 | §10 |
| D23 | Q21 重排库的落点 | 这个目录最好是和浏览器绑定而不是和 Agent 绑定的。这样即使 Agent 删除了，这些重排代码还能跑。我理解重排代码的存在，有可能让会让用户访问 www.facebook.com 的时候，实际上加载的是 xxx.$zonehost/www-facebook-com/ 的代码？网站重排并不一定是要实时的，Agent 可以提前刷新得到结果，只需要用户在界面上能看到“基于什么时间的网站制作”即可 | 重排库挂在用户名下。重排页有 Zone 里的固定地址。结果可以提前制作，界面显示制作时间（§5.6） |
| D24 | Q21 的落点建议（应用对象桶加专用主机名） | 按你的建议 | 桶见 RF1。主机名的做法见 D27 |
| D25 | 下一步 | 把该文档升级成可以指导 CodeAgent 完成的整个需求的 TODO | 实施 TODO 的修订 |
| D26 | Q22 Chromium 用哪份构建 | 打包 Chrome for Testing | ext-core 直接带 Chrome for Testing，和 Playwright 的版本对齐。再分发许可没查到权威说法，记在 §10 |
| D27 | Q23 重排页的固定地址要不要分两步、要不要特殊隔离 | 重排页的代码都是我们生产的，安全边界可看作一致。按标准的 app 逻辑顶一个规则就好 | 不做沙箱加载那一步，也不做能力令牌。给重排定一个主机名规则 `reflow.<zone>`，和别的 App 一样用会话登录（§5.6、RF5）。“顶”按“定”理解 |
| D28 | Q24 opendan 引用 `tokio-tungstenite`、`hyper`、`hyper-util`、`futures` | 可以引入 | BR1 |
| D29 | Q25 系统渲染选降权到 `nobody`，不选容器 | 按你的建议，简单可靠的跑通就行 | §5.7、SR2。隔离只做必要的几样 |
| D30 | Q26 给 Agent 容器加 `--init` | 启动方式保持现状 | 不动容器的启动参数。孤儿进程在浏览器模块里处理（BR2） |
| D31 | Q27 Widevine 的再分发 | 这应该是一个通用机制：ext-tool 的基础机制要支持某个 item 是“用时下载”（安装完成触发 / 用户手工触发）。像 Agent 浏览器这种组件，对不能上网的环境没有意义（也不用进离线包），同时也能规避一些 CDM 的问题 | ext-tool 加“在线项”：包里只声明，装好以后在节点上从上游下载。Chromium 是第一个。我们不再分发 Chrome for Testing，Q27 和 D26 的再分发风险一起消掉（§5.9） |
| D32 | 在线项从哪里下载 | 下载地址就写官方。我生活在硅谷，BuckyOS 的早期客户也主要在美欧日，连不上 Google 不是问题 | 清单里每个平台一个官方地址加 SHA256，不配镜像，不做可覆盖的下载源 |
| D33 | 在线项怎么触发 | 装完自动下载；下载失败是可以接受的，可以在系统管理面板手工再次触发下载 | 包装好以后节点自动下载一次，失败就停在“失败”，不无限重试；“设置 > 扩展工具”和 `buckyos ext-tools` 里可以再次触发 |
| D34 | 哪些东西做成在线项 | 这次只做 Chromium。后续再专门讨论一次哪些包可以在线化 | LibreOffice 等照旧随包 |
| D35 | Q30 默认事情绑定哪个会话；Q34 “事情”的界面 | 每个 Agent 有一个专门处理“重排”的 chat session，不为每个事情建一个，所有和浏览器使用的都在这里。“事情”是逻辑上的，UI 上没必要做专门的支持，大部分普通用户压根做不好 topic 管理 | 每个 Agent 一个专用的浏览器会话（带 `ui.source` 标记的 MessageHub 会话，用 `bind_default` 绑定）；App 去掉事情列表和“新建一件事”；服务端每个 Agent 只有一个标签组（§5.10） |
| D36 | Q31 `browser` 手册进不进对话的默认集合 | 可以考虑让上面的专用 chat session 有这个手册 | 只有专用浏览器会话的 UI Session 带 `browser` 手册，别的对话照旧 |
| D37 | Q32 手机画面的清晰度 | 按建议做，最好能根据网速动态调整。真正解决网速问题的其实是重排 | 协议加 `pixel_ratio`（2 倍虚拟屏）；App 按实测的网速自动调像素密度、画质和帧率 |
| D38 | Q35 重排包的生命周期 | 现在内核跑通了，重排包的生命周期再单独设计。非实时重排的价值比想象中大：可以先缓存相关资源到本地，甚至对这些资源做处理（比如给无字幕的视频加字幕），甚至只是把看过的媒体从本地缓存变成组织好的本地存储都有价值。“重排”这个词可能把功能说窄了，实际上用户拥有了一段连续时间观察网站的能力，而不是只能看打开这一刻的网站。新网页 = 重排(网址, 当前时间, 历史记录) | 本期不改重排的格式和生命周期，另立文档设计 |
| D39 | Q38 重排脚本的运行身份 | 好，安全登记就是“Agent 为我定制的可信产物” | 记进已知风险的说法 |
| D40 | Q28、Q29、Q33、Q36、Q37、Q39–Q41、Q48 | Q28 已经发布了；Q29、Q36、Q37、Q39、Q40、Q41 按建议；Q33 现在还是高级功能，不放桌面；Q48 另行验证 | Q29：ext-core 里所有用 fontconfig 的工具都设 `FONTCONFIG_PATH`（ext-core 1.3.1）；其余照现在的做法 |

原 Q15 由 D3 定下，编号不复用。

## 3. 现状与落点

行号以基线为准。路径不带前缀的都在 `buckyos` 仓库里。

### 3.1 ext-core

| 环节 | 落点 |
| --- | --- |
| 版本常量、选项 | `publish/exttool/ext-core/build.sh:22-35`、`:47-55`。主循环 `:627-654` |
| 加一个官方归档 | `fetch`（`:71-89`，文件名必须在 `sources.sha256` 里）；解包到 `store/ext-core/<tool>/`（`install_official` `:342-411`）；`official-sources.json` 要有 `build_versions` 和 `components[]`（binaries 加 sources），由 `official_sources.py:23-34` 校验 |
| 加 conda-forge 包 | `conda-env.yml:12-22`，按 `:2-3` 的命令重新生成 `conda-lock.yml`；安装在 `install_conda`（`:315-325`） |
| 裁剪 | `prune_prefix`（`:186-313`）。`:211` 删掉了 `libatk-*`、`libatk-bridge-*`、`libatspi*`，Chromium 要用 |
| 字体 | Noto CJK 的四个 `.ttc` 放在 LibreOffice 的字体目录里（`:456`、`:501-506`）。`conda/etc/fonts/fonts.conf` 只列了 `../../share/fonts` 和 `../../fonts`，没有 Noto 所在的目录，所以只有 LibreOffice 自己看得见这些字体。`64-ext-core-cjk.conf` 在 `:457-466` |
| 启动器 | 是脚本，不是符号链接。Unix：`unix_launcher`（`:119-132`），环境文件 `unix_env_files`（`:135-167`），清单 `:590-598`。Windows：`windows_launcher`（`:169-181`），清单 `:601-625`。`soffice` 的环境文件在 `:162-166` |
| 包的布局规则 | 顶层只许 `ext-manifest.json`、`bin/`、`smoke/`、`store/<id>/`。`bin/` 里的每个文件都要在 `provides` 里，反之亦然。`bin/` 的文件里不能有构建机的绝对路径。校验在 `src/kernel/buckyos-api/src/ext_tools.rs:185-238` |
| 安装后的路径 | `store/ext-core` 是指向 `versions/ext-core/<版本>-<摘要>/store/ext-core` 的链接。稳定路径是 `$BUCKYOS_ROOT/tools/store/ext-core/<目录>`，容器里是 `/opt/buckyos/tools/store/ext-core/<目录>` |
| 清单 | `publish/exttool/ext-core/ext-manifest.json`（版本 1.2.0，24 条 `provides`，五个平台）。类型 `ExtToolsManifest`：`src/kernel/buckyos-api/src/ext_tools.rs:66-85` |
| 升版要改的地方 | `ext-manifest.json:4`、`dapp_meta/app.json:5`、`dapp_meta/pikg.json:4` |
| 表 B 和守卫 | `doc/app_service/paios容器需求.md:289`（表题里的“24 条命令”）、`:295`（表头）、`:302`（soffice 行，可照抄格式）。`.github/scripts/ext_tools.py:23-54`（`CONDA_TOOLS`、`OFFICIAL_TOOLS`、`check-table`），测试 `.github/scripts/test_ext_tools.py`。`src/frame/lib_opendan/src/baseline.rs:157-194`（`EXTENDED_TOOLS`），测试 `:744-770`。`ExtToolCategory`（`:111-120`）没有浏览器这一类 |
| 发布门禁 | 每个组件要有完整的对应源码和许可文本：`compliance.py:474-491`、`:510-513`，`release_manifest.py:91-93`、`:105-108` |
| 平台 | linux 两个平台任何机器都能构建；darwin 只能在 Mac 上构建；windows 可以用 `msiextract` 在 Linux 上构建。CI：`.github/workflows/ext-core.yml:62-67`。Linux 冒烟在裸的 `ubuntu:24.04` 容器里跑，`--network none`，root，64 MiB 的 `/dev/shm`（`:102-109`） |
| 现在的体积 | linux-x86_64 解压 1.8G（tar.gz 676 MiB），其中 conda 630M、libreoffice 510M。GitHub 的发布文件上限 2 GiB，超过只发 R2（`ext-core.yml:302`） |
| Node | `store/ext-core/node/`。没有给捆绑的 npm 包准备全局目录，全仓库没有设 `NODE_PATH`。`NPM_CONFIG_PREFIX` 指向每个 Agent 自己的目录（`baseline.rs:225`） |
| 本地迭代 | `build.sh --platform linux-x86_64` → `uv run src/tools/prepare_ext_tools.py --local <payload 目录>` → 删 `$BUCKYOS_ROOT/tools/installs/ext-core.json` → 重启 |

### 3.2 Agent 容器和会话环境

| 环节 | 落点 |
| --- | --- |
| 容器怎么起 | `src/kernel/node_daemon/src/app_loader.rs:1882-2015`（`run_worker_container`）。没有 `--user`（root）、没有 `--shm-size`（64 MiB）、没有 `--init`、没有能力和安全选项。测试用的预览 `preview_worker_start`（`:3441+`）要保持同步，断言在 `test_app_loader.rs:1143`、`:1262` |
| 容器里谁是 PID 1 | opendan：`publish/aios/entrypoint.sh:307-317` 直接 `exec` |
| PATH | 镜像 ENV（`build_aios:608`、`:624`、`:677-687`）和 `entrypoint.sh:70-74`。缓存类环境变量在 `entrypoint.sh:36-68` |
| 会话的环境变量 | `src/frame/lib_opendan/src/runtime/mod.rs:1658-1694`（`session_env_vars`），路径层 `:1574-1594`。shell 继承进程环境，不清空（`src/frame/agent_tool/src/llm_bash.rs:1195-1214`） |
| 找工具 | `baseline.rs:578-589`（`buckyos_root()`、`managed_tool_dirs()`），`resolve_locally`（`:521-534`） |
| 原生运行 | node_daemon 的 `try_start_native_agent`（`app_loader.rs:1699-1760`），环境 `:1665-1693`。没有实例卷，缓存落在 `<AgentRoot>/.cache/…`（`runtime/mod.rs:1557-1570`）。Windows、macOS 上以桌面用户身份运行 |
| AgentRoot 布局 | `doc/opendan/protocol/Agent State Protocol.md:11-33`。容器里实例卷是 `/opt/buckyos/instance` |

### 3.3 Agent Service（opendan）

| 环节 | 落点 |
| --- | --- |
| HTTP 服务和挂载 | `src/frame/opendan/src/loader.rs:431-513`。`/kapi/<agent_id>` 在 `:470-479` |
| 请求入口 | `src/frame/opendan/src/service.rs:717-764`（`serve_request`）。POST 走 kRPC；其他 GET 一律返回 `{"app_id":…}`（`:743-759`），WebSocket 路由要插在 `:743` 之前 |
| kRPC 分发 | `service.rs:343-523`（`call()` 的大 `match`）。`ui.*` 在 `:403-414`，`skills.*` 在 `:457-465` |
| 方法分类 | `is_owner_only`（`:559-571`，Agent 自己的 App 身份也会被拒）、`is_read`（`:573-639`）、`is_operation`（`:642-666`），由 `authorize_agent_method`（`:526-555`）使用。单测 `:779-875` 枚举这些清单 |
| 调用者身份 | `service.rs:144-187`（`caller()`）。现在的入参是 `&RPCRequest`，WebSocket 握手要复用它，得改成收 token、方法名、IP |
| 模块的写法 | 字段 `ui: Option<Arc<UiModule>>`（`service.rs:112-126`）；模块登记 `src/frame/opendan/src/lib.rs:15-27`；配置段 `src/frame/opendan/src/config.rs:49-64`；状态 `loader.rs:278-285`；后台循环 `loader.rs:523-597`，停机 `:618-631`；进程级环境变量 `loader.rs:311-320` |
| WebSocket 的现成写法 | opendan 里没有。同一套 HTTP 栈上的先例在 control_panel：分发 `src/frame/control_panel/src/main.rs:1448-1450`，升级 `src/frame/control_panel/src/agent_daemon.rs:732-819`，中继循环 `:1068-1096`，带真实升级的测试 `:1263-1342`。用到的 crate：`tokio-tungstenite`（workspace 里已有 0.29）、`hyper`、`hyper-util`、`futures`。opendan 的 `Cargo.toml` 现在没有引用它们 |
| 首帧鉴权的先例 | AIWS：`src/frame/aiworkspace/server/src/session.rs:558-577`（10 秒内发 `hello{token}`） |
| 子进程 | opendan 现在没有常驻子进程。按进程组杀：`src/frame/agent_tool/src/llm_bash.rs:760-797`、`:1202-1205`。Windows 的控制台标志：`src/kernel/buckyos-api/src/process_util.rs:13-60` |
| 给会话写系统记录 | `post_input(sid, &PostedInput::event(…))`：`src/frame/lib_opendan/src/protocol/input.rs:630-646`，例子 `src/frame/lib_opendan/src/todos.rs:516-530`。半订阅的事件在下一次输入前呈现，不单独触发推理；同一来源只保留最新一条（`protocol/state.rs:464-500`）。`_notify` 的隐式半订阅是可抄的先例：`src/frame/lib_opendan/src/template.rs:220-238`、`protocol/config.rs:934-947`、`:1041-1050` |
| 会话和 MessageHub | 一个 MessageHub 会话 = 一个 Agent 收件箱 = 一个 UI Session，可以同时有多个。UI Session 在第一条消息到达时才创建（`src/frame/opendan/src/ui.rs:707-801`）。`ui.conversation {peer_did, session_id?}` 在 `ui.rs:1053-1139` |
| 网关 | `/kapi/<agent_id>` 的路由：`src/kernel/scheduler/src/system_config_agent.rs:1278-1311`、`src/rootfs/etc/boot_gateway.yaml:366`。网关不对 `/kapi/*` 做鉴权，服务自己查。WebSocket 升级能过（AIWS 已在用） |
| 测试 | `src/frame/opendan/tests/loader.rs`（`rpc()` 在 `:639`，路由和权限的用例 `:936`，通知观察进 UI 会话 `:2618`） |

### 3.4 Desktop

路径都在 `src/frame/desktop/src/` 下。

| 环节 | 落点 |
| --- | --- |
| 加一个内置 App 要改六处 | `app/registry.tsx:31-52`；`mock/data.ts:21`（`appCatalog`，名字叫 mock 但是真的目录，My Content 的条目在 `:379-404`）；`app/backend-apps.ts:6-23`（白名单，漏了 App 会被丢掉）；`components/DesktopVisuals.tsx:32-57`（图标）；新增 `i18n/<app>.ts` 并在 `i18n/dictionaries.ts:1764-1766` 合并；`models/desktopFolder.ts:119-133`（桌面快捷方式，可选） |
| 模板 | 最新的 App 是 My Content（提交 `835b5bd44`）。带 store 的结构看 `app/workflow/`：`WorkflowAppPanel.tsx:8-19` |
| 启动参数和深链 | 每个 App 一个 `launch.ts`（`app/settings/launch.ts:6-12`）。Desktop 的 URL 深链：`desktop/DesktopRoute.tsx:279-305`（`?settings=`、`?agent_setup=`） |
| 手机 | 判定是 `useMediaQuery('(max-width:768px)')`（`DesktopRoute.tsx:259-261`），没有共用的 `useIsMobile`。全屏容器 `desktop/windows/MobileWindowSheet.tsx:16-95`，`contentPadding:'none'` 铺满。返回键 `desktop/windows/MobileNavContext.tsx:90` |
| 列出我的 Agent | `api/user_mgr.ts:868-871`（`fetchAgentList`），按 `owner_user_id` 自己过滤（`app/agent-setup/model.ts:43-49`） |
| 调 Agent Service | `app/messagehub/conversation/worklog/source.ts:10-25`（`agentServiceClient(agentDid)`），调用的例子 `app/messagehub/agentpanel/source.ts:33-70` |
| WebSocket | 唯一的先例是 AIWS 的 `SessionChannel`：`app/aiworkspace/session/channel.ts:39-263`（首帧发 token、断线重连、心跳）。token 取法 `app/aiworkspace/api/transport.ts:239-249` |
| 开发代理 | `src/frame/desktop/vite.config.ts:176-183` 的 `/kapi` 规则没有 `ws: true`，连真实 Zone 调试时要加 |
| 聊天组件 | 整块的 `app/messagehub/ConversationView.tsx:58-116`。窄面板的先例是 `app/messagehub/guest/GuestRoute.tsx:234-297`（历史加输入框，自己接 reader 和动作）。`ask_user` 的问题在输入框里回答（`conversation/input/ConversationComposer.tsx:726-760`）。嵌入要提供 `WindowDialogProvider` 等几个上下文，并引入 `messagehub/messagehub.css` |
| MessageHub 的 store | 接口 `app/messagehub/store/types.ts:58-188`：`sessions`（`:92`）、`ensureDefaultSession`（`:93-94`）、`create`（`:116`）、`reader`（`:96`）、`send`（`:127`）、`startSync`（`:77`）。实现 `app/messagehub/api/store.ts` |
| 选文件和保存 | 要 Zone 路径：`sysdlg/index.tsx:88-95`（`useSelectContent`）。要字节：`sysdlg/select/pickFiles.ts:46-63`（`usePickFiles`）。保存：`components/saveToZone.ts:32-65`（`useSaveToZone`，要 objId） |
| 网页应用的 iframe | Desktop 里跨源加载网页应用的做法：`app/web-frame/WebAppFramePanel.tsx:19-104`，登录 `app/web-frame/launch.ts:46-67`、`api/app_launch.ts:36-49`。带校验的消息桥可以参考 `app/aiworkspace/ui/extensions/htmlRuntime.ts:277-294` |
| 画布和输入 | 尺寸 `app/aiworkspace/ink/paint.ts:19-31`；触摸手势 `app/aiworkspace/ui/canvas/render/touch.ts:1-60`；解码帧用 `createImageBitmap`（`presentation/imageFile.ts:62-73`）；软键盘高度 `app/aiworkspace/ui/table/EditBar.tsx:18-29` |
| “查看浏览器”的入口 | `app/messagehub/agentpanel/AgentSessionPanel.tsx:126`、`:164`、`:196` |
| e2e | `tests/e2e/pages/<app>.spec.ts`，夹具 `tests/e2e/fixtures/desktop.ts`，手机视口 `tests/e2e/flows/mobile.spec.ts:4-8`。默认跑 mock（`.env.development` 的 `VITE_CP_USE_MOCK=true`） |

### 3.5 Workflow Service

路径都在 `src/kernel/workflow/src/` 下。

| 环节 | 落点 |
| --- | --- |
| 登记一个内置函数 | `functions/media/mod.rs:31-40`（名字）、`:98-181`（登记表，照抄 `TO_PDF` 的条目 `:164-171`）、`:212-245`（工具基线） |
| 模板 | `functions/media/document.rs`：函数对象 `:13-38`，`invoke` `:66-121` |
| 工具探测 | `functions/media/tools.rs:12-21`（名字和 `MEDIA_TOOLS`）、`:32-69`（查找目录）、`:148-158`（版本） |
| 车道 | `native.rs:25-55`（`ExecutionLane`，现在五条） |
| 运行工具 | `native.rs:481-576`（`run_tool`、`run_tool_env`）。不清空环境，不设进程组，只杀直接子进程 |
| 继承的凭据 | node_daemon 把服务的会话令牌放进环境变量 `WORKFLOW_SESSION_TOKEN`（`src/kernel/node_daemon/src/kernel_mgr.rs:225-228`），工具进程会继承 |
| 断网的先例 | 不在 workflow 里，在 nfs_server 的预览：`src/frame/nfs_server/src/preview/convert.rs:543-565`（`unshare --net`，不行就退到 `unshare --user --map-root-user --net`） |
| 降权的先例 | 只有 Node Executor：`src/kernel/node_executor/src/spawn.rs:150-203`（`drop_privileges`）、`:166`（进程组）、`:238`（按组发信号）、`:398-402`（最小环境） |
| 运行身份 | Linux 上是 root（node_daemon 拉起，`src/kernel/node_daemon/src/service_pkg.rs:414`） |
| 系统定义 | `defs/media.document.json`，嵌入和校验 `system_defs.rs:15-75`。`server/tests.rs:292` 断言一共 4 个 |
| 调用路径 | kRPC `/kapi/workflow`：`create_run`、`get_run_output`。Rust 客户端 `src/kernel/buckyos-api/src/workflow_service.rs:725`。调用的例子 `src/frame/agent_tool/src/llm_understand_media/pipeline.rs:810-870` |
| 输入鉴权 | 带 `obj_id` 的输入用调用者自己的令牌查读权限（`server.rs:103-152`）。不带 `obj_id` 的输入不做任何鉴权（`server.rs:651-653`） |
| 测试 | `functions/media/tests.rs:76-105` 断言正好 10 个函数和工具清单，都要改 |

### 3.6 桶和网关

| 环节 | 落点 |
| --- | --- |
| 系统服务声明桶 | `src/kernel/buckyos-api/src/control_panel.rs:23-69`（`control_panel_bucket_decls()`，现有 `wallpapers`）。scheduler 在 `src/kernel/scheduler/src/system_config_builder.rs:355` 写进规格 |
| 桶的规则 | bucket_id 是 `user/<appid>/<user>/<name>`。key 可以带 `/`。`app_data` 档要写配额 |
| 读写 | HTTP：`/bucket/v1/<bucket_id>/<key>`（`src/frame/nfs_server/src/bucket/http.rs:45-118`）。kRPC：`bucket.*`。websdk：`buckyos-websdk/src/bucket_client.ts`。CLI：`buckyos-websdk/cli/modules/bucket.ts`（一次一个文件，没有递归上传） |
| HTML 被强制下载 | `/bucket/v1` 对图片、音视频、PDF、纯文本以外的类型一律当附件下发，并加沙箱 CSP（`cyfs-ndn/src/ndn-lib/src/content_safety.rs:24-29`、`:66-79`）。这是桶的数据面的设计：能执行的内容不走这个入口。重排页走自己的主机名（D27、RF5） |
| Agent 能不能写 | 读写权限的 Agent 能读写 owner 名下任何 App 的用户桶；只读权限的 Agent 只能读（`src/kernel/buckyos-api/src/rbac_config.rs:92-93`、`:247-251`） |
| 主机名 | 系统服务的主机名是硬编码的：`src/kernel/scheduler/src/system_config_agent.rs:1209-1251`（nfs-server 有 `share` 和 `s3`）。保留名有两份：`system_config_agent.rs:2112-2120`、`src/kernel/scheduler/src/install_plan_executor.rs:176-187` |
| 按主机名分流的先例 | nfs-server：`src/frame/nfs_server/src/share/serving.rs:56-82`（`host_guard`）、`src/frame/nfs_server/src/bucket/s3.rs:101-114`（`host_route`），挂在 `server.rs:94-95`。网关里 `public.<zone>` 的提前返回：`src/rootfs/etc/boot_gateway.yaml:233-235` |

## 4. 实测记录

2026-11-14 晚，本机，一次性容器。镜像 `local/aios-test:latest-amd64`（Ubuntu 24.04，root，默认 seccomp，`/dev/shm` 64 MiB），工具卷只读挂载（本机装的 ext-core 是 1.1.0，没有 LibreOffice 和字体）。Playwright 1.65.1，它下载的是 Chrome for Testing 159.0.8154.6（revision 1255）。x86_64。

运行环境：

| # | 结论 |
| --- | --- |
| V1 | 工作镜像里 Chromium 缺 25 个系统库：libX11、libXcomposite、libXdamage、libXext、libXfixes、libXrandr、libasound、libatk-1.0、libatk-bridge-2.0、libatspi、libcairo、libcups、libdbus-1、libexpat、libgbm、libgio-2.0、libglib-2.0、libgobject-2.0、libnspr4、libnss3、libnssutil3、libpango-1.0、libsmime3、libxcb、libxkbcommon |
| V2 | ext-core 的 conda 前缀里已经有其中 21 个。缺的 4 个是 libatk-1.0、libatk-bridge-2.0、libatspi（在锁文件里，被 `build.sh:211` 删了）和 libgbm（不在锁文件里） |
| V3 | `LD_LIBRARY_PATH` 指到 `conda/lib`，再补上那 4 个库，Chromium 能启动、能渲染 |
| V4 | 以 root 运行不带 `--no-sandbox` 会拒绝启动。换成普通用户，在默认的 Docker 配置下报 `No usable sandbox`。所以容器里必须带 `--no-sandbox` |
| V5 | 环境里一个字体都没有时，渲染文字直接崩溃（`SkFontMgr_FontConfigInterface.cpp:163`）。装上字体后正常 |
| V6 | 解压后的体积：完整构建 414 MB，无头精简构建（headless shell）282 MB，ffmpeg 5 MB |
| V7 | Playwright 的无头测试默认用精简构建。只有完整构建时，默认的 `chromium.launch()` 报 `Executable doesn't exist … chromium_headless_shell-1255`；加 `channel: 'chromium'` 就能用完整构建。`PLAYWRIGHT_BROWSERS_PATH` 指向只读目录可以工作 |
| V8 | 无头运行、开 3 个简单标签页时，容器里所有进程的 PSS 合计约 430 MiB |

CDP（`--headless=new`，原始 WebSocket 客户端）：

| # | 结论 |
| --- | --- |
| V9 | 手机模拟生效：`Emulation.setDeviceMetricsOverride`（412×915、`mobile`）加触屏和 UA 之后，媒体查询走手机分支，`innerWidth` 是 412。能切回 1280 的桌面视口 |
| V10 | `Page.startScreencast`（jpeg、质量 50）：帧的尺寸是 CSS 像素（412×915），不随设备像素比变大。每帧 17–19 KB，不限速时 60 帧每秒。png 每帧约 38 KB |
| V11 | 流量控制靠不住单一手段：完全不回确认时，再发约 3 帧就停；每帧延迟 200 ms 再确认，仍有 14 帧每秒。限速要靠服务端丢帧或 `everyNthFrame` |
| V12 | `Input.insertText` 输入中文正确。`Input.imeSetComposition` 可用 |
| V13 | 文件对话框：`Page.setInterceptFileChooserDialog` 之后能收到 `Page.fileChooserOpened`，用 `DOM.setFileInputFiles` 填入文件。必须先 `Page.enable` |
| V14 | JS 弹框：能收到 `Page.javascriptDialogOpening`，没人处理时一直开着。触发弹框的那次输入命令在弹框关掉前不返回，不要等它 |
| V15 | 新窗口有 `Target.targetCreated`（带 `openerId`）。下载有 `Browser.downloadWillBegin`（先 `Browser.setDownloadBehavior`） |
| V16 | `<select>` 点开后，选项列表不在截屏帧里，也不在截图里。要由 App 自己显示 |
| V17 | 两个连接并存：Playwright 用 `connectOverCDP` 连上以后，截屏流继续，视口不变 |
| V18 | Playwright 连着、又没有注册弹框处理时，页面的弹框会在约 57 ms 内被它自动关掉。Agent 的连接不能在不操作时挂着 |
| V19 | Playwright 读得到密码框的值（D22 的风险属实）。`locator.ariaSnapshot()` 可用 |
| V20 | `Page.printToPDF`：A4、5 页，页眉、页脚、页码模板都生效，中文能用 `pdftotext` 抽出来 |

其他：

| # | 结论 |
| --- | --- |
| V21 | Playwright 从 1.57（2025-12）起下载的是 Google 的 Chrome for Testing，不再是它自己编译的 Chromium。Chrome for Testing 能不能再分发，没有查到权威说法；Chromium 的下载页说它带专有编解码器。owner 决定直接打包（D26） |
| V22 | 坑：Chromium 输出的 JPEG，分量编号是 0、1、2。有的看图工具会把它解成花屏，CodeAgent 的读图也会。看帧之前先用 `magick x.jpg x.png` 转一下 |

没测的：真机手机上的弱网体验；真实手机输入法的事件流；macOS、Windows；aarch64；conda 版的 atk 和 gbm；宿主机上降权后能不能开沙箱（Ubuntu 24.04 默认用 AppArmor 限制了非特权用户命名空间）。

## 5. 设计

### 5.1 ext-core 里的浏览器

（2026-11-15 改：Chromium 不在包里，是包的在线项，见 §5.9。下表的 `chromium/` 一行作废，其余照旧。）

包里放的东西（都在 `store/ext-core/` 下）：

| 目录 | 内容 |
| --- | --- |
| `chromium/` | 按 Playwright 的浏览器目录布局：`chromium-<revision>/<平台目录>/…`，带 `INSTALLATION_COMPLETE` 和 `DEPENDENCIES_VALIDATED` 两个标记文件 |
| `playwright/node_modules/playwright-core/` | 和这份浏览器配套的版本 |
| `browser/` | 我们自己的脚本：`agent-browser.mjs`、`page-render.mjs` 和共用的模块。源码放在 `publish/exttool/ext-core/browser/` |

三条命令（`bin/` 下的启动器，写进 `provides`）：

| 命令 | 是什么 | 谁用 |
| --- | --- | --- |
| `chromium` | 浏览器本体 | Agent Service 启动常驻浏览器；手册里的排查 |
| `agent-browser` | Agent 操作常驻浏览器的命令（§5.3） | Agent |
| `page-render` | 把一个页面输出成 PDF 或 PNG（§5.7） | Workflow 的系统渲染函数 |

要点：

- **只带完整构建，不带精简构建**（D4、D20）。测试要写 `channel: 'chromium'`（V7），由手册和项目模板写好。
- **Linux 的运行库复用 conda 前缀**（V2、V3）。不再删 atk 和 atspi，给 conda 环境加上 libgbm。
- **Linux 上浏览器可执行文件的位置放一个包装脚本。** Playwright 直接执行 `chromium-<revision>/chrome-linux64/chrome`，不经过 `bin/` 的启动器。所以把真正的二进制改名（比如 `chrome-bin`），原名放一个 sh 脚本：设好 `LD_LIBRARY_PATH`（`conda/lib`）和 `FONTCONFIG_FILE`（没设时指向前缀里的配置），再 `exec` 真正的二进制。脚本里不能有绝对路径。macOS、Windows 的构建自带依赖，不用包装。
- **字体要让 fontconfig 找得到**（V5）。现在 Noto CJK 只有 LibreOffice 看得见，要在前缀的字体配置里加上它的目录。
- **会话环境变量。** `PLAYWRIGHT_BROWSERS_PATH` 指到 `<工具目录>/store/ext-core/chromium`，`PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=1`。项目用配套版本的 `@playwright/test` 时不用下载浏览器。
- **项目用了别的 Playwright 版本。** 浏览器目录是只读的，它得把自己的浏览器下到别处，而且那份浏览器没有包装脚本。`agent-browser env` 打印出需要的环境变量（库路径、字体配置），手册教 Agent 这样做。手册建议新项目用系统的版本。
- **工具卷每个节点一份**，所有 Agent 共用。

### 5.2 常驻浏览器（Agent Service 的浏览器模块）

**进程。**

- Agent Service 按需启动一个 Chromium：`chromium --headless=new --user-data-dir=<profile> --remote-debugging-port=0 --no-first-run --no-default-browser-check --disable-dev-shm-usage`，再加缓存目录和下载目录。
- Unix 上有效用户是 root 时加 `--no-sandbox`（V4）。不是 root 时先不加；如果启动后立刻因为 `No usable sandbox` 退出，加上重试一次，并记日志。
- 端口从用户数据目录里的 `DevToolsActivePort` 文件读。调试端点只监听回环地址。
- 目录：容器里是 `${BUCKYOS_INSTANCE_VOLUME}/browser/{profile,cache,downloads}`；原生运行和宿主调试是 `<AgentRoot>/.cache/browser/…`。
- 停止时先让浏览器自己退出，不行再杀整个进程组，并回收它留下的子进程（BR2）。空闲回收：没有人看、没有 Agent 租约、超过配置的时间（默认 10 分钟）就停。停之前把各组标签页的网址存下来，下次按需重开。
- 崩溃后退避重启。

**分组。** 一件事一组标签页（D15）。

- 组的标识就是 MessageHub 会话的 id。每个 Agent 有一个默认组，对应它的默认会话。
- 新标签页归谁：页面自己弹出的（有 opener），归 opener 所在的组；Agent 持有租约时新开的，归租约的组；用户新开的，归他正在看的组。
- 各组的标签页列表存在 `<AgentRoot>/state/browser/groups.json`。
- 登录状态整个 Agent 共用，一个用户数据目录。

**kRPC**（挂在 `/kapi/<agent_id>`）：

| 方法 | 谁能调 | 作用 |
| --- | --- | --- |
| `browser.status {group_id?}` | owner、Agent | 是否启用、是否在运行、版本、各组的标签页、操作方、Agent 状态 |
| `browser.ensure {}` | owner、Agent | 没在运行就启动 |
| `browser.acquire {session_id}` | Agent | 取得所在组的操作租约。返回调试端点、组、标签页。用户正在操作时失败，错误码 `user_in_control` |
| `browser.renew {lease_id}`、`browser.release {lease_id, state?, note?}` | Agent | 续租、释放。`state` 可以是 `done` 或 `stuck` |
| `browser.request_handover {session_id, reason, note?}` | Agent | 请用户接手。`reason`：`password`、`captcha`、`other` |
| `browser.take_over {group_id}`、`browser.hand_back {group_id}`、`browser.hand_to_agent {group_id}` | owner | 接手、交还、交给 Agent |
| `browser.upload {request_id, zone_path}` | owner | 回应一次文件选择：服务端用调用者的令牌从 Zone 取文件，交给页面 |
| `browser.download_export {download_id}` | owner | 把一个下载好的文件登记成对象，返回 objId，供 App 存到 Zone |
| `browser.clear_profile {}` | owner | 停掉浏览器，清空用户数据目录 |
| `browser.reflow_refresh {url}`、`browser.reflow_state {url}` | owner、Agent | 重排的刷新和状态（§5.6） |

- “owner”的方法放进 `is_owner_only`。“Agent”的方法要保证 Agent 的 App 身份调得通，不受它对 owner 目录是只读还是读写的影响：用自己的浏览器是它自己的能力。
- `session_id` 是 Agent 所在会话的 id。服务端从它找到所属的 UI Session（Work 会话顺着来源往上找），再对应到 MessageHub 会话，也就是组。
- Agent 这边的入口：Agent Service 在进程环境里设 `OPENDAN_BROWSER_RPC`（本机回环上的 kRPC 地址），会话里的命令继承它，带着会话已有的 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 调用。跑在其他节点上的会话没有这个变量，用不了常驻浏览器（D5）。
- 标签页的打开、关闭、切换、导航走屏幕的 WebSocket（§5.4），不另设 kRPC。

**配置。** `[loader.browser]`：`enabled`（默认开；找不到 `chromium` 时模块报“不可用”，其他功能照常）、`idle_stop_secs`、`max_tabs`。

**给会话的记录。** 用一个隐式的半订阅 `_browser`，来源是 `system:browser:<group_id>`，同一组只保留最新一条。写进去的事件：用户已接手；用户已交还（带当前网址和标题，提示重新读页面）；用户请 Agent 接着做。它们不单独触发推理。需要叫醒 Agent 的时候，由 App 往会话里发一条普通消息（§5.5）。

### 5.3 Agent 怎么操作：`agent-browser`

D16：用最直接高效的方法。Agent 写一段 Playwright 脚本，一次跑完。

| 用法 | 作用 |
| --- | --- |
| `agent-browser status [--json]` | 浏览器和本组的状态：谁在操作、有哪些标签页 |
| `agent-browser run <脚本.mjs> [--arg k=v]…` | 取租约 → 连上 → 调脚本的默认导出 `async ({ page, context, args }) => 结果` → 把结果按 JSON 打印 → 断开 → 释放租约 |
| `agent-browser open <网址>` | 在本组打开或导航 |
| `agent-browser snapshot` | 当前页面的可访问性快照（文本） |
| `agent-browser screenshot <文件.png>` | 截图存成文件，用 `llm_understand_media` 看 |
| `agent-browser tabs` | 本组的标签页 |
| `agent-browser handover --reason password [--note …]` | 请用户接手 |
| `agent-browser done [--note …]`、`agent-browser stuck --note …` | 报告办完或卡住，控制回到用户 |
| `agent-browser env` | 打印别的版本的浏览器需要的环境变量（§5.1） |
| `agent-browser version` | 浏览器和 `playwright-core` 的版本 |

要点：

- 它是 Node 脚本，自带 `playwright-core`，所以 Agent 的脚本里不用解决模块查找的问题：`page` 直接传进来。
- **每次操作连上、做完就断开**（V18）。连着的时候，给不属于本组的页面注册空的弹框处理，免得把用户那边的弹框自动关掉。
- **操作前先取租约。** 用户在操作时命令直接失败，输出里说明“用户正在操作，等交还”。这是配合式的约束：Agent 绕开这条命令自己连调试端点，系统拦不住（D22）。
- 脚本运行期间自动续租。脚本抛错也要断开和释放。
- 不改视口（D19）。
- 手册规定：遇到密码框，调 `handover --reason password`，不读也不填密码框（D3）。

### 5.4 屏幕、交接和原始页面模式

**连接。** `GET /kapi/<agent_id>/browser/screen` 升级成 WebSocket。5 秒内的第一帧是 `hello`，带用户的会话令牌、要看的组、视口。服务端按 owner 校验，通过后回 `ready`。

**消息。** 文本帧是 JSON，二进制帧是一张 JPEG。

| 方向 | 类型 | 内容 |
| --- | --- | --- |
| 上行 | `hello` | `token`、`group_id`、`viewport {width, height, mobile}` |
| 上行 | `ack` | `seq`：收到并画完了这一帧 |
| 上行 | `input` | `touch`、`mouse`、`wheel`、`key`、`text`（整段文字）、`composition`（输入法组字） |
| 上行 | `nav` | `url`、`back`、`forward`、`reload` |
| 上行 | `tab` | `open`、`close`、`activate` |
| 上行 | `viewport` | 视口变了，或切到桌面版 |
| 上行 | `respond` | 回应一个 `request`：选了哪个选项、弹框点了什么、文件用哪个 |
| 上行 | `quality` | 帧率上限和画质 |
| 下行 | `ready` | 当前状态 |
| 下行 | `frame_meta` | 下一个二进制帧的 `seq` 和所属标签页 |
| 下行 | （二进制） | JPEG |
| 下行 | `state` | 操作方、Agent 状态、标签页列表、当前网址、标题、是否在加载 |
| 下行 | `request` | 页面之外的请求：`select`、`dialog`、`file_chooser`、`download`、`permission` |
| 下行 | `error` | 错误码和说明 |

**帧的节奏**（V10、V11）。服务端手里最多有一帧没被确认；新帧来了而上一帧还没确认，就只留最新的一帧，旧的丢掉。再加一个帧率上限（默认每秒 15 帧）。弱网时自然降到每秒几帧。

**视口**（D19、V9）。看的人是手机就按手机的尺寸、触屏和手机 UA 模拟；App 里可以切“桌面版”。视口由服务端这条连接设置，Agent 不改。

**页面之外的界面**（V13–V16）：

| 情况 | 做法 |
| --- | --- |
| `<select>` | 给每个页面注入一段脚本，拦下对下拉框的点按，把选项报上来；App 用自己的选择面板显示；选完后服务端设值并派发 `input`、`change` 事件 |
| JS 弹框 | 转成 `request`，App 显示，回应后服务端关掉它 |
| 文件对话框 | 转成 `request`。App 用系统文件选择器从 Zone 选，调 `browser.upload`；本机来源的文件走 `PUT /kapi/<agent_id>/browser/upload/<request_id>` 上传字节 |
| 下载 | 存在下载目录里，完成后发 `request`。App 用 `browser.download_export` 拿到 objId，再用 `useSaveToZone` 存到 Zone |
| 新窗口 | 在同一组里多一个标签页，`state` 里更新 |
| 权限请求（定位、摄像头等） | 一律拒绝，并发一个 `request` 让用户知道 |
| 日期、时间、颜色这类输入框 | 第一期按文本框处理，列为已知不足 |

**谁在操作**（D8）。按组算：

- Agent 持有有效租约、用户又没接手时，操作方是 Agent。其他时候是用户。
- 操作方是 Agent 时，用户的 `input`、`nav`、`tab` 不送达页面（服务端强制）。
- 用户“接手”：租约作废。Agent 之后的 `acquire` 失败，直到用户“交还”或“交给 Agent”。
- Agent 调 `request_handover`：租约作废，Agent 状态变成“等你接手”。
- 租约到期没续，或 Agent 调 `release`：操作方回到用户。
- Agent 状态：`idle`、`working`、`waiting_answer`（有没回答的 `ask_user`）、`waiting_takeover`、`done`、`stuck`。App 始终显示操作方和这个状态。
- 接手不暂停 Session。

### 5.5 「Agent 浏览器」App 和对话办理

**结构。** 先选一个自己的 Agent（D10），然后是：地址栏、事情列表、模式切换（原始页面、重排、对话）、状态条（谁在操作、Agent 在等什么）、对话面板。按手机设计，桌面上用同一套；例外是顶部（11-16 owner）：桌面只有一行，Agent 名字后面是地址栏，标签页按钮在右侧 ⋮ 的左边（App 宽度不到 640 px 时仍是两行）。

**一件事**（D15）。

- 事情列表就是和这个 Agent 的 MessageHub 会话。新建一件事 = 新建一个会话（创建时带上来源标记，以便只列出浏览器里建的）。默认会话对应“还没交代事情”时的浏览。
- 切换事情 = 切换会话，屏幕跟着切到那一组标签页。
- 对话面板复用 MessageHub 的历史和输入框组件，接 MessageHub 的 store。`ask_user` 的问题在输入框里回答，不另做。

**交给 Agent。** 用户在原始页面里点“交给 Agent”：App 调 `browser.hand_to_agent`，再往这个会话发一条消息，内容是“从当前页面接着做”加当前网址、标题和用户补的话。Agent 被这条消息叫醒。

**密码**（D3）。Agent 调 `agent-browser handover --reason password`，在对话里说明要用户填什么。App 的状态条显示“等你接手”，用户在原始页面里填完，点“交还”。App 调 `browser.hand_back`，并往会话发一条消息（默认文字“填好了，继续”，可以改），Agent 被叫醒后重新读页面。

**其他缺项**（D3、D18）。包括短信验证码、证件号，都用 `ask_user` 在对话里问，Agent 填进页面，提交前不另行确认。

**入口。** Desktop 的应用列表；MessageHub 的 Agent 面板里“Agent 正在使用浏览器 · 查看”；Desktop 的深链 `?agent_browser=<agent>&group=<会话>`，Agent 首页用它跳过来。

**默认入口**是 PRD 待定的选择（R8）。先按“打开 App 进默认会话的原始页面，地址栏为空，对话面板收起”做，等产品定了再调。

### 5.6 重排

D2、D17、D23、D27：选择器；存放和触发不问作者；和浏览器绑定；有固定地址，按标准 App 的逻辑提供；不必实时，显示制作时间。

**重排库。** 桶 `user/control-panel/<用户>/reflow`，声明在 control-panel 名下（Desktop 内置 App 的数据都这样放）。它属于用户，删除 Agent 不影响它。

| key | 内容 |
| --- | --- |
| `<站点>/manifest.json` | 匹配声明、偏好、作者、更新时间 |
| `<站点>/refresh.mjs` | 刷新脚本。在服务器的浏览器里对原网站运行，取回内容 |
| `<站点>/index.html` 和它引用的样式、脚本 | 页面代码。在用户设备上运行 |
| `<站点>/results/<页面>.json` | 一个页面最近一次制作的结果，带 `made_at`、`made_by`（哪个 Agent 的浏览器）、`source_url` |
| `<站点>/fallback/<页面>.html` 和同名的 `.json` | 没有页面代码时 LLM 兜底重排的结果和它的元信息 |

- key 和网址的路径一一对应：`reflow.<zone>/<key>`（见“显示”）。
- `<站点>` 是主机名把点换成短横线，如 `www-facebook-com`；换完没有短横线的（单段主机名）加后缀 `-host`，免得和网关按路径分流的前缀（`kapi`、`bucket`、`home` 等）撞上。`<页面>` 是规范化网址的散列。
- 包的格式第一版（`schema: "buckyos.reflow/1"`）：
  - `manifest.json`：`site`、`match[]`（`host` 加路径通配）、`preference`（用户的偏好，自然语言）、`author {kind: agent|user, id}`、`updated_at`、`refresh {script, timeout_s}`。
  - `refresh.mjs`：默认导出 `async ({ page, url, save }) => data`。`save.image(网址)` 把图片取回、压缩后放进结果，返回可内嵌的引用。
  - 页面代码按网址参数里的原网址找到 `results/<页面>.json`，把里面的数据画出来。数据来自第三方网站，按文本渲染，不要当 HTML 直接插进页面（§10）。
- RF0 试做之后可以改格式，改了回写这里。

**触发。** 用户在重排模式打开一个网址：

1. 按网址在库里找匹配的包。
2. 有包也有结果：直接显示，带制作时间。不调用模型，不需要浏览器在运行。
3. 有包没有结果：调 `browser.reflow_refresh`，显示“正在制作”，同时可以切到原始页面。
4. 没有包：看有没有兜底结果，有就显示；没有就由 App 往默认会话发一条消息，请 Agent 做兜底重排，显示“正在制作”。

**刷新。** `browser.reflow_refresh {url}`：Agent Service 在一个后台标签页里打开原网址，跑包里的 `refresh.mjs`，把结果写进桶。脚本失败就在状态里记下失败；用户在等的话，App 按第 4 步请 Agent 兜底，并让它修包。三种时机：用户点“刷新”；Agent 提前刷新（什么时候刷由 Agent 自己安排，命令是 `reflow refresh`）；用户打开一个还没有结果的页面。

**兜底和写包**是 Agent 的工作，写在手册里：读原页面，按偏好生成一份结果；对常访问的站点写出包放进库里；用户调整偏好时一并改包。Agent 在自己的工作目录里写和试，写好用 `reflow put` 放进库里。只读权限的 Agent 写不了别的 App 的桶，这时它把包留在自己的目录里，请用户在 App 里导入。

**`reflow` 命令**（Agent 的标准命令，TS，基于 websdk）：`reflow match <网址>`、`reflow ls`、`reflow get <站点> <目录>`、`reflow put <站点> <目录>`、`reflow rm <站点>`、`reflow result put <网址> <文件>`、`reflow refresh <网址>`。用户在 App 的管理页里也能查看、导入、删除。

**显示**（D27）。重排页的代码是我们自己生产的，按标准 App 的逻辑提供，不做特殊隔离：

- 主机名 `reflow.<zone>`，由 nfs-server 提供。它把路径直接当成重排桶里的 key：`reflow.<zone>/www-facebook-com/` 就是当前用户桶里的 `www-facebook-com/index.html`。目录补 `index.html`。HTML、脚本、样式按原类型下发，不强制下载。
- 用户是谁由会话决定，登录走系统服务自有源的现成做法（AIWS 的 `aiws-<id>.<zone>` 就是这样）：Desktop 调 `auth.prepare_app_launch`，经一次性的 `/sso_callback` 在这个主机上种下会话。只有登录的用户能打开，看到的是自己的桶。
- App 的重排模式用 iframe 加载 `https://reflow.<zone>/<站点>/?url=<原网址>`，做法同 Desktop 里别的网页应用。地址栏仍显示原网址，旁边显示制作时间。
- 页面和 App 之间用 `postMessage` 通信（校验来源是 `reflow.<zone>`）：页面把“打开链接”“执行原网页上的动作”“刷新”交给 App，App 去调 Agent Service。

**交互。** 普通链接：按“触发”处理。依赖原网页的按钮：结果里记下它对应原网页的哪个元素；点击后 App 请 Agent Service 在原网页上执行，再刷新这一页；处理不了的交给 Agent。切回原始页面：在浏览器里打开这个重排页的 `source_url`。

**不必实时带来的性质。** 重排模式不要求原标签页开着，也不要求浏览器在运行。

**以后的方向（D38，本期不做）。** owner 试用后认为非实时重排的价值比想象中大：可以先把相关资源缓存到本地，甚至对资源做处理（比如给没有字幕的视频加字幕），或者只是把看过的媒体从缓存变成组织好的本地存储。“重排”这个词把功能说窄了：用户得到的是在一段连续时间里观察一个网站的能力，而不只是打开那一刻的网站。新网页 = 重排(网址, 当前时间, 历史记录)。重排包的生命周期另立文档设计；本期的格式和触发不变。

### 5.7 系统渲染

（实施后的参数名、默认值和隔离做法见 §5.8 的“系统渲染”。）

D11、D21：Workflow Service 的内置函数。

**`page-render`**（ext-core 的命令，Node 脚本，用 `playwright-core` 起一个一次性的 Chromium）：

| 参数 | 含义 |
| --- | --- |
| `--html <文件或目录>` | 输入是本地的 HTML 文件，或带 `index.html` 的目录。渲染时不许访问网络 |
| `--url <网址>` | 输入是一个网址。只许访问 `--allow-host` 列出的主机 |
| `--allow-host <主机>` | 可重复 |
| `--pdf <输出>` 或 `--png <输出>` | 输出 |
| `--paper <A3\|A4\|A5\|Letter\|宽x高mm>`、`--landscape`、`--margin <mm>` | 纸张 |
| `--header-html <文件>`、`--footer-html <文件>`、`--background` | 页眉页脚模板和背景 |
| `--viewport <宽x高>`、`--scale <倍数>` | 截图用 |
| `--wait-ready <表达式>`、`--wait-ms <毫秒>` | 等页面给出“排好了”的信号 |
| `--timeout <秒>` | 总时限 |

结束时在标准输出打印一行 JSON：`{pages, bytes, sandbox, warnings}`。

**Workflow 的函数**：

| 函数 | 输入 | 输出 | 性质 |
| --- | --- | --- | --- |
| `media.page_to_pdf` | `content_id`（一份 HTML，或 zip 里带 `index.html`），加纸张参数 | `media.derived_pdf`（和 `media.to_pdf` 同一个结果类型） | 纯函数，可缓存 |
| `media.page_snapshot` | `content_id`，加视口参数 | PNG | 纯函数，可缓存 |

- 新车道 `Browser`，一次一个。
- 系统定义 `media.page`：`capabilities` → `page_to_pdf` 或 `page_snapshot`。触发参数里用 `obj_id`，读权限按调用者自己的令牌查，和 `media.document` 一样。
- **网址输入第一期不做成 Workflow 的入口。** `page-render` 支持 `--url`，但 Workflow 现在对不带 `obj_id` 的输入不做鉴权（§3.5），令牌也不能放进函数参数（会进缓存键和运行记录）。AIWS 导出 PDF 要用哪种输入，取决于渲染管线的裁决，在[打印 TODO](aiws-print-pdf-export-todo.md) 的 T7、T8 里定。

**隔离。** 渲染的内容可能包含扩展 Block 的 HTML，是第三方代码。Workflow 在 Linux 上是 root。新增一个隔离的启动方式，只给这条车道用：

- 清空环境，只留白名单（现在工具进程会继承 `WORKFLOW_SESSION_TOKEN`）。
- 单独的进程组，超时或取消时杀整组（Chromium 有一串子进程）。
- Linux 上有效用户是 root 时，降到 `nobody`。临时目录先交给这个账号。
- 断网：HTML 输入时用 `unshare --net`，做法抄 nfs_server 的预览；另外在浏览器里拦掉所有网络请求。
- Chromium 自身的沙箱：先带着沙箱起；宿主不支持时（V4 里的那种报错）去掉沙箱重试，结果里 `sandbox: false`，并记日志。
- Windows、macOS 上 Workflow 以桌面用户运行，不降权，沙箱照常。

和 AIWS 导出 PDF 的关系不变：页面模型、分页、PDF Frame、对话框都在打印 TODO；本文提供“把一个页面变成 PDF”的函数，哪种渲染管线的裁决都用得上。

### 5.8 实施中对本节的调整

接口的细节以 [Agent Browser Protocol](<../doc/opendan/protocol/Agent Browser Protocol.md>) 为准。下面只列和 §5.1–§5.7 的原文不同或原文没写的地方。

**ext-core（§5.1）**

- 版本：`playwright-core` 1.65.1，Chrome for Testing 159.0.8154.6（revision 1255）。`store/ext-core/browser/versions.json` 记这三个值，脚本和 Agent Service 都读它。
- Linux 的包装脚本除了 `LD_LIBRARY_PATH`、`FONTCONFIG_FILE`，还要设 `FONTCONFIG_PATH`：只设前者时 `fonts.conf` 对 `conf.d` 的相对包含找不到，渲染不出中文。`agent-browser env` 同样多导出这一项。
- Chrome for Testing 的归档里没有 Chromium 的许可文本，许可文本取自源码归档；对应源码登记的是 `-lite` 归档。

**常驻浏览器（§5.2）**

- 组的标识：默认组固定是 `default`。MessageHub 里和 Agent 的会话都是具名会话，没有稳定的“默认会话”，所以加了 `browser.bind_default {session_id}`：默认组第一次要用到会话时，App 取一个会话绑定上去，之后这个会话里 Agent 的操作都归 `default`。`_` 开头的组留给服务端自己用（重排刷新的后台标签页）。
- 每个标签页单独一个窗口，并开着焦点模拟：同一个窗口里排在后面的标签页不渲染，截屏流和 Playwright 的点击、截图都会停住。
- 一个组可以同时有几个租约（UI Session 和它的 Work 会话各跑一条命令）。`request_handover` 作废租约的同时也置“接手”，直到用户交还。
- kRPC：多了 `browser.bind_default`、`browser.reflow_action`；`browser.status` 多一个 `session_id` 参数；`browser.upload` 多一个 `done`；`browser.acquire` 在浏览器没运行时自己启动；错误都带稳定的 `code`。
- `waiting_answer` 不看会话登记表的 `needs_input`（每个空闲的对话它都是真），改查这个 Agent 发出的、还开着的提问通知。
- 没人看、操作方也不是 Agent 的组里出现 JS 弹框，服务端立刻关掉（`beforeunload` 按离开，其他按取消）：开着的弹框会卡住任何一个 Playwright 连接，不分组。有人看的组留给屏幕，60 秒没有回应也关掉。
- `_browser` 的隐式半订阅做在所有会话上（`SessionConfig::implicit_subscriptions()`），不是只写进 `ui` 模板：模板里的订阅在建会话时写死，已有的对话收不到。
- 启动参数多了 `--no-startup-window`、`--window-size=1280,800`、`--disk-cache-dir`，以及浏览器来自 ext-core 时的 `--user-agent`（无头模式默认的 UA 带 `HeadlessChrome`，很多网站把它当机器人）。Agent Service 自己的 `TMPDIR` 太长时浏览器进程改用 `/tmp`（Chromium 在 `TMPDIR` 下建套接字，超过约 62 字节起不来）。
- 崩溃处理进程（`chrome_crashpad_handler`）会脱离进程组，靠启动时带的一个环境变量标记找出来一并回收。

**`agent-browser`（§5.3）**

- `run` 的脚本多收到一个 `pages`（本组的标签页；`context.pages()` 里有别组的）。动作默认 30 秒超时。
- 本组页面上脚本没处理的弹框默认关掉。
- 内部子命令 `reflow-run`（重排刷新用）。
- 退出码：0 成功，1 失败，2 用法错误，3 浏览器不可用，4 用户正在操作，5 运行中丢了租约。

**屏幕（§5.4）**

- 多了下行的 `focus`（焦点落在可编辑元素上，App 据此弹出软键盘）、`request_closed`，上行的 `ping`；握手后的错误码多一个 `bad_message`。
- `touch` 的 `points` 是事件之后仍按着的全部触点。`viewport` 的 `mobile` 变了会重新加载当前标签页。
- 本机文件上传的临时文件要留到页面真正读它的时候（提交表单时才读），不是请求结束就删。

**App（§5.5）**

- “新建一件事”的来源标记放在 owner 自己的会话 UI 状态里（`ui_session.update_state`，键 `ui.source`），不改 msg-center 的协议。
- 交给 Agent、交还都先出一个确认面板（可以补一句话），确认后才调 kRPC 和发消息。
- 事情列表除了默认事情和带标记的会话，还列出 Agent 自己在用浏览器的会话。
- 桌面上没有加默认快捷方式，从“所有应用”进。

**重排（§5.6）**

- 只有 http(s) 的网址有重排；规范化按 WHATWG 的 URL 规则；共用的测试向量在 `doc/opendan/protocol/fixtures/reflow_vectors.json`，TS、Rust、Desktop 三份实现都对着它测。
- 多了 `<站点>/preference.json`：没有包的站点的偏好。放包时并进 manifest 并删掉它。
- `reflow put` 是整包替换；`reflow result put` 按扩展名分：`.json` 是包要画的结果，`.html` 是兜底页。
- 依赖原网页的动作走新的 `browser.reflow_action {url, selector}`。
- 一个 Agent 同时只跑一个刷新，其余排队。刷新的子进程以清空的环境启动，拿不到 Agent Service 的令牌。
- `reflow.<zone>` 上种下的会话，在 nfs-server 的文件、桶、NDM 接口上不算登录：否则受限账号能借这个新的登录目标用上文件接口。
- 改了偏好，App 会给 Agent 发一条消息请它改包并重做当前页。

**系统渲染（§5.7）**

- `media.page_to_pdf` 的参数：`paper`、`landscape`、`margin_mm`、`background`，可选 `header_html`、`footer_html`、`wait_ready`、`wait_ms`。`media.page_snapshot` 的参数：`viewport_width`、`viewport_height`、`scale`，可选 `wait_ready`、`wait_ms`；结果类型 `media.page_image`（`image_content_id`、`image_obj_id`、`width`、`height`）；截图是整页，超过 4096² 像素回 `resource_limit`。
- `media.page` 的触发参数默认值：`output` 是 pdf、A4、边距 10 mm、打印背景、视口 1280×800、倍数 1。
- 隔离：root 时断网用 `unshare(2)` 系统调用（降权在 `pre_exec` 里做，网络命名空间要在降权之前建）；`TMPDIR` 用 `/tmp` 下的短名私有目录（执行临时目录太深，Chromium 的套接字路径会超长）。
- `page-render` 的退出码：0 成功，2 参数错误，3 超时，4 渲染失败，5 浏览器没起来。`--url` 的主机自己也要在 `--allow-host` 里。

### 5.9 在线项（D31–D34）

ext-tool 的基础机制加了“在线项”：包的清单里声明，包本身不带，节点装好包以后从上游的官方地址下载。离线包里没有它；我们不分发它，它的再分发条款（Chrome for Testing 带的 Widevine CDM）不再是发布阻塞项。机制的说明在 `doc/app_service/paios容器需求.md` §6.4 的“在线项”一节。

- 清单：`online[]`（`id`、`version`、`desc`、`license`、`auto`，每个平台一条 `url`、`sha256`、`size_bytes`、`format`、`into`、`rename`、`overlay`），`provides[].needs` 写一条命令要哪个在线项。
- 节点上：`$BUCKYOS_ROOT/tools/online/<id>/` 和记录 `tools/online/<id>.json`（`state`: `missing` / `downloading` / `ready` / `failed`）。在包的版本目录之外：包升级时在线项的版本没变就不重新下载，只重盖 overlay。
- 触发：包装好以后自动下载一次；失败停在“失败”，不自动重试，重启也不重试（Q45）。手工再触发：Desktop“设置 > 扩展工具”、`buckyos ext-tools download <包> <在线项> [--node]`、kRPC `ext_tools.download`。对已就绪的在线项手工触发等于重新下载、整体替换（Q46）。手工请求记在 `nodes/<node>/ext_tools_online/<item>`。
- 下载期间已有一份在用时，记录保持 `ready`，新版本记在 `next` 里，换名之后才切过去，浏览器不中断。
- Chromium：`tools/online/chromium/chromium-1255/<平台目录>/…`，就是 Playwright 的浏览器目录；`PLAYWRIGHT_BROWSERS_PATH` 指这里。Linux 上的包装脚本、两个标记文件是 overlay，随包。`bin/chromium`、`agent-browser`、`page-render`、`playwright-core` 随包。
- 浏览器还没下载时：`chromium` 退 127，`page-render` 退 6；Agent Service 的 `browser.status` 报 `unavailable_code: not_downloaded`，下载好以后不用重启就可用；Workflow 的 `media.page` 报 `tools_missing`（`details.online_item: "chromium"`）；运行时的基线检查把它记成“等下载”（`ext.waiting`），不算缺失；手册的命令状态列在 `waiting_online`，不再推荐启用 ext-core；「Agent 浏览器」App 给管理员一个下载按钮。

## 6. 任务

每条：落点、做法、验证。没写落点的见 §3 对应小节。

### 5.10 一个 Agent 一个浏览器会话（D35、D36）和画面清晰度（D37）

- 每个 Agent 只有一个标签组 `default`（`_` 开头的内部组照旧，比如重排刷新的后台标签页），用户和 Agent 的所有会话共用这一组标签页。
- 浏览器会话：这个 Agent 名下带 `ui.source = agent-browser` 标记的 MessageHub 会话，App 第一次需要时找或建（标题“浏览器”）并用 `browser.bind_default` 绑定。`_browser` 记录、App 发的交接消息、重排的兜底请求都进这里；`waiting_answer` 只看这里的提问；它的 UI Session 带 `browser` 手册，别的对话不带。App 没有事情列表、没有“新建一件事”，深链只有 `agent` 和 `mode`。
- 清晰度：屏幕协议的视口带 `pixel_ratio`（1–2），大于 1 时服务端把标签页的窗口挪到 2 倍虚拟屏（启动参数 `--screen-info`），1 和 2 之间用截屏流的最大尺寸压。App 按实测的网速在四档之间自动换（清晰 / 标准 / 省流 / 极省：像素比、画质、帧率），降档快、升档慢；设备像素比低于 1.5 时“清晰”档也按 1；设置里可以手动选“自动 / 清晰 / 省流”。画质跟随生效视口的那条连接。细节在协议 §3。

### 6.1 EX：ext-core 加浏览器

- [x] **EX0 定版本。** 选一个当时稳定的 Playwright 版本，记下它配的 Chrome for Testing 的 revision 和各平台的下载地址（`npx playwright install --dry-run chromium` 能看到）。在 `build.sh:22-35` 加常量，在 `official-sources.json` 的 `build_versions` 里登记。打包的就是这份 Chrome for Testing（D26）。
- [x] **EX1 装进包。** `build.sh` 加 `install_chromium`：五个平台各取对应的归档，解到 `store/ext-core/chromium/chromium-<revision>/`，放上两个标记文件。`sources.sha256`、`official-sources.json` 的 `components[]`、`.github/scripts/ext_tools.py` 的 `OFFICIAL_TOOLS` 都加上。macOS 的在 Mac 上解，注意别弄坏签名。
  - 发布门禁要求每个组件有对应源码和许可文本（§3.1）。Chrome for Testing 是上游的二进制：`sources` 登记同版本的 Chromium 源码归档，许可文本用包里自带的。门禁因此过不去时，停下来问 owner，不要自己放宽门禁。
  - 验证：`build.sh --platform linux-x86_64` 成功；`test_ext_tools.py` 通过。
- [x] **EX2 Linux 运行库。** `build.sh:211` 不再删 `libatk-*`、`libatk-bridge-*`、`libatspi*`。`conda-env.yml` 加提供 `libgbm.so.1` 的包，重新生成锁文件，`CONDA_TOOLS` 同步。仿照 `assert_soffice_loads`（`:559-575`）加一个检查：用 `LD_LIBRARY_PATH=conda/lib` 对浏览器二进制跑 `ldd`，不许有 `not found`。
  - 验证：检查通过；V1 的 25 个库全部解析到 `conda/lib`。
- [x] **EX3 Linux 包装脚本。** 按 §5.1 做。
  - 验证：不设任何环境变量，直接执行 `chromium-<revision>/chrome-linux64/chrome --version` 成功；Playwright 的 `chromium.launch({ channel: 'chromium' })` 成功。
- [x] **EX4 字体。** 让前缀的 fontconfig 配置能找到 Noto CJK（改 `build.sh:303-311`、`:457-466` 附近）。
  - 验证：无头渲染一段中文输出成 PDF，`pdftotext` 能抽出中文；`soffice` 的冒烟照常通过。
- [x] **EX5 `playwright-core`。** 取 npm 的发布包，钉版本和 SHA256，解到 `store/ext-core/playwright/node_modules/playwright-core/`。登记来源和许可。
- [x] **EX6 我们的脚本和启动器。** 源码目录 `publish/exttool/ext-core/browser/`，构建时拷到 `store/ext-core/browser/`。在 Unix 和 Windows 的启动器清单里加 `chromium`、`agent-browser`、`page-render`。我们自己的文件在合规清单里怎么登记，照 `smoke/` 和 `env/` 的处理方式做。
- [x] **EX7 清单、基线、表 B、冒烟。**
  - `ext-manifest.json` 加三条 `provides`，升版到 1.3.0，连同 `dapp_meta/app.json`、`dapp_meta/pikg.json`。
  - `baseline.rs` 的 `EXTENDED_TOOLS` 同步。
  - 表 B 加三行，改表题里的条数。每行写明支持的场景。
  - 冒烟脚本 `smoke/chromium.sh`、`smoke/agent-browser.sh`、`smoke/page-render.sh` 和对应的 `.ps1`。要能在 root、断网、64 MiB 的 `/dev/shm`、180 秒内跑完。
  - 验证：`cargo test -p libopendan baseline`；`test_ext_tools.py`；按 `ext-core.yml:102-109` 的方式在裸容器里跑冒烟。
- [x] **EX8 会话环境变量。** 在 `baseline.rs` 的 `managed_tool_dirs` 旁边加一个函数，浏览器目录存在时给出 `PLAYWRIGHT_BROWSERS_PATH` 和 `PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS`，在 `session_env_vars` 里用上。容器、原生、宿主会话都要生效。
  - 验证：加单测；在容器的会话里 `echo` 这两个变量。
- [ ] **EX9 其他平台。**（部分：linux-aarch64、windows 的 payload 构建了但没在对应平台运行；darwin 没构建。见 §12） linux-aarch64、darwin 两个、windows。macOS 上确认包里的 Chromium 能直接运行（签名、隔离属性）；Windows、macOS 上确认由后台进程启动无头浏览器正常。
  - 验证：CI 的冒烟门禁在五个平台都过。
- [x] **EX10 记体积。** 把五个平台加完浏览器后的解压体积和压缩体积记进 §12。压缩包超过 2 GiB 的平台只发 R2。

### 6.2 DT：开发测试可用

- [x] **DT1 容器里跑通。** 新装的环境里，在私有 runtime 新建一个前端项目，装配套版本的 `@playwright/test`，配置里写 `channel: 'chromium'`，跑通一个测试。全程不下载浏览器，不装系统库。
- [x] **DT2 别的版本。** 项目用另一个 Playwright 版本时，按 `agent-browser env` 的输出设环境变量，下载它自己的浏览器并跑通。
- [ ] **DT3 原生运行。**（没做） macOS 或 Windows 上重复 DT1。
- [x] **DT4 手册。** `buckyos_app_dev` 等提到前端测试的手册里写明：系统的 Playwright 版本怎么查、配置怎么写、别的版本怎么办。

### 6.3 BR：Agent Service 的浏览器模块

- [x] **BR1 骨架。** 新模块 `src/frame/opendan/src/browser/`，在 `lib.rs` 登记，`StateService` 加字段，`[loader.browser]` 配置段，模块状态，后台循环和停机。opendan 的 `Cargo.toml` 加 `tokio-tungstenite`、`hyper`、`hyper-util`、`futures`（D28）。
- [x] **BR2 启动和停止。** 按 §5.2：找到 `chromium`，拼参数，读 `DevToolsActivePort`，进程组，空闲回收，崩溃退避，沙箱的判断。`entrypoint.sh` 里加浏览器目录的环境变量，原生运行走 `.cache` 的回退。Windows 用后台子进程的标志和 Job Object。
  - 容器的启动方式不变（D30）。容器里 opendan 是 1 号进程，Chromium 的子进程被杀后会挂到它名下变成僵尸。所以停止分三步：先用 CDP 的 `Browser.close` 让浏览器自己退出；等不到再给进程组发终止信号，最后才强杀。强杀之前记下这个进程组里的进程号，之后逐个 `waitpid` 回收。只回收记下的这些，不要回收任意子进程，否则会抢走 shell 命令的退出状态。
  - 验证：启动、停止后没有残留进程；杀掉 Chromium 后能自动重启；空闲到时间后停掉。
- [x] **BR3 CDP 客户端。** 用 `tokio-tungstenite` 连浏览器的调试端点。按 id 配对请求和回应，订阅事件，用扁平会话附着到每个标签页。只实现用得到的那些方法。
  - 验证：对着假的 CDP 服务端写单测；真浏览器的用例标 `#[ignore]`。
- [x] **BR4 分组和标签页。** 按 §5.2：组、归属规则、持久化、恢复、上限。
- [x] **BR5 kRPC。** 按 §5.2 的表在 `call()` 里加 `browser.*`。每个方法放进正确的分类清单，补 `service.rs:779-875` 的单测。更新 opendan README 的 kRPC 表。
- [x] **BR6 Agent 侧入口。** 进程环境里设 `OPENDAN_BROWSER_RPC`。确认 Agent 的 App 身份调得通 Agent 那一列的方法，只读权限的 Agent 也行。
- [x] **BR7 给会话的记录。** 加隐式半订阅 `_browser`，按 §5.2 写事件。
  - 验证：仿照 `tests/loader.rs:2618` 写用例：接手、交还后，UI 会话的下一次输入里能看到最新的一条。
- [x] **BR8 测试。** `cargo test -p opendan -- --test-threads=1`、`cargo test -p libopendan -- --test-threads=1`。

### 6.4 SC：屏幕和原始页面模式（服务端）

- [x] **SC1 WebSocket 端点。** 在 `serve_request` 的 `:743` 之前匹配 `GET …/browser/screen`，升级的写法抄 control_panel。把 `caller()` 改成能收 token、方法名、IP，握手时用它按 owner 校验。
  - 验证：仿照 `agent_daemon.rs:1263-1342` 写带真实升级的测试：没有 `hello`、令牌不对、不是 owner 都被拒。
- [x] **SC2 协议。** 按 §5.4 的表实现。
- [x] **SC3 截屏流和节奏。** `Page.startScreencast`；最多一帧在途、只留最新；帧率上限；切标签页；视口和 UA 的模拟。
  - 验证：客户端不回 `ack` 时服务端不再发帧；慢客户端收到的总是最新画面。
- [x] **SC4 输入。** 触摸、鼠标、滚轮、按键、整段文字、输入法组字，映射到 CDP 的输入命令。坐标按 CSS 像素。操作方是 Agent 时丢弃。
- [x] **SC5 页面之外的界面。** 按 §5.4 的表：下拉框的注入脚本、弹框、文件对话框、下载、新窗口、权限请求。注意 V13、V14。
- [x] **SC6 上传和下载。** `browser.upload`（服务端用调用者的令牌从 Zone 取文件，取法参考 Desktop 的 `sysdlg/select/zoneFiles.ts:23-31` 用的接口）；本机文件的 `PUT` 上传，带大小上限；`browser.download_export`（登记对象参考 `src/frame/opendan/src/objects.rs`）。

### 6.5 HO：交接

- [x] **HO1 状态。** 按 §5.4：每组的操作方、租约、Agent 状态、超时。
- [x] **HO2 用户这一侧。** `take_over`、`hand_back`、`hand_to_agent`；操作方是 Agent 时拦住用户的输入。
- [x] **HO3 Agent 这一侧。** `acquire`、`renew`、`release`、`request_handover`。
- [x] **HO4 可见。** 状态变化经 `state` 消息推给 App；交接写进会话（BR7）。
  - 验证：用例覆盖：Agent 持租约时用户输入被丢；用户接手后 `acquire` 失败；交还后成功；租约过期后操作方回到用户。

### 6.6 AB：`agent-browser`

- [x] **AB1 命令骨架。** `publish/exttool/ext-core/browser/agent-browser.mjs`：读 `OPENDAN_BROWSER_RPC` 和会话令牌，调 kRPC。没有这个变量或浏览器不可用时，给出明确的话。
- [x] **AB2 `run`。** 取租约、连接、调脚本、打印结果、断开、释放。运行中续租。给别组的页面挂空的弹框处理（V18）。脚本抛错时也要断开和释放。
- [x] **AB3 其余子命令。** 按 §5.3 的表。
- [x] **AB4 手册。** 新增 `src/apps/jarvis_agent/agent/cmd_skills/browser/SKILL.md`，`commands` 里写 `agent-browser`、`chromium`、`reflow`。内容：先 `status`；用 `run` 一次做完一段；被用户接手就停，等交还后重新 `snapshot`；遇到密码框调 `handover`，不读不填；不改视口；截图怎么看；兜底重排和写包（§5.6）。按 AGENTS.md“cmd_skills 联动维护”检查选择规则和受影响的手册。
  - 验证：`cargo test -p libopendan --lib skills`、`--test xagent_cli skills_are`。
- [x] **AB5 测试。** 在容器里用真浏览器跑：`open`、`snapshot`、`run` 一段填表脚本、`handover` 之后 `run` 失败。

### 6.7 APP：Desktop 的「Agent 浏览器」

目录 `src/frame/desktop/src/app/agent-browser/`。

- [x] **APP1 登记。** §3.4 的六处。id 用 `agent-browser`。手机上铺满。
- [x] **APP2 数据层。** store：我的 Agent 列表、当前 Agent、当前组、屏幕状态。调 Agent Service 复用 `agentServiceClient`。mock 模式下有一套假的屏幕和标签页，供 e2e 用。
- [x] **APP3 屏幕通道。** 仿照 AIWS 的 `SessionChannel` 写 `ScreenChannel`：首帧 `hello`、心跳、断线重连。`vite.config.ts` 的 `/kapi` 代理加 `ws: true`。
- [x] **APP4 屏幕视图。** 画布显示帧（`createImageBitmap`），画完回 `ack`。触摸、鼠标、滚轮转成 `input`。文字输入：用一个隐藏的输入框接住软键盘和输入法，组字过程发 `composition`，上屏的文字发 `text`。软键盘弹出时让出高度。
- [x] **APP5 外壳。** 选 Agent、地址栏、事情列表、标签页、模式切换、状态条、桌面版开关。
- [x] **APP6 页面之外的界面。** 下拉框的选择面板、弹框、文件选择（`useSelectContent` 拿 Zone 路径；本机文件用 `usePickFiles` 再上传）、下载后存到 Zone（`useSaveToZone`）。
- [x] **APP7 对话面板。** 按 `GuestRoute.tsx:234-297` 的做法嵌入历史和输入框，接 MessageHub 的 store。新建事情、切换事情。
- [x] **APP8 入口。** MessageHub 的 Agent 面板；Desktop 的深链；OpenDAN WebUI 的 Agent 首页加一个跳到 Desktop 的链接（仿照 `src/frame/opendan/web/src/pages/Runtimes.tsx:146`）。
- [x] **APP9 清除登录状态。** 设置入口，带确认，调 `browser.clear_profile`。
- [x] **APP10 e2e。** `tests/e2e/pages/agent-browser.spec.ts`：打开 App、选 Agent、看到画面、输入、切换事情、交接的状态条。再加一份手机视口的。
  - 验证：`pnpm check`、`pnpm lint`、`pnpm test:e2e`。基线就失败的用例见 §8.2。

### 6.8 DG：对话办理

- [x] **DG1 事情和会话。** 新建会话时带来源标记；事情列表只列带标记的和默认会话。
- [x] **DG2 交给 Agent。** 按 §5.5。
- [x] **DG3 密码接手。** 按 §5.5，App 和手册两边。
- [x] **DG4 状态。** `waiting_answer` 从会话里未回答的 `ask_user` 得出；`done`、`stuck` 来自 Agent 的 `release`。
- [ ] **DG5 DV。**（部分：真实模型下的填表、缺项提问、密码接手、接手与交还都验了；“跨两个网站的一件事”和短信验证码没验。见 §12） 真实 Zone、真实模型：交代一件跨两个网站的事，看预填、提问、填写、密码接手、结果说明。

### 6.9 RF：重排

- [ ] **RF0 试做。**（没做：没有挑真实的旧网站量耗时和 token。真实模型对一个测试站点做过一次兜底重排，从请求到显示 80 秒，约 41 万 token，其中含意图分析和建 Work 会话） 挑两三个旧网站（和 PRD 的首批场景一起选）。手工走一遍：LLM 兜底重排一页，量耗时和 token；为一个站写包，看覆盖多少页面、改版时怎么失效。结论写进 §12，需要的话改 §5.6 的格式。
- [x] **RF1 桶。** 在 `control_panel_bucket_decls()` 加 `reflow`：用户桶、`app_data`、写上配额。
  - 验证：`cargo test -p buckyos-api`；真实 Zone 上 Desktop 能读写，读写权限的 Agent 能写。
- [x] **RF2 `reflow` 命令。** `src/tools/buckyos-agent/commands/reflow.ts`，按 §5.6。它是标准命令，要同步 `baseline::STANDARD_TOOLS` 和表 A。websdk 的 `BucketClient` 没有递归上传，在命令里逐个文件放。
- [x] **RF3 触发和刷新。** Agent Service：`browser.reflow_state`（匹配包、查结果）、`browser.reflow_refresh`（后台标签页里跑 `refresh.mjs`，由 `agent-browser` 的内部子命令执行；写结果；记失败）。
- [x] **RF4 兜底。** App 在没有包或刷新失败时往默认会话发消息请 Agent 处理（§5.6）。手册里写清楚 Agent 该怎么做（AB4），包括页面代码要把取回的数据按文本渲染。
- [x] **RF5 主机名规则和提供。**
  - scheduler：`system_config_agent.rs:1237-1251` 给 nfs-server 的主机列表（现在是 `share`、`s3`）加 `reflow`；两份保留名都加（`system_config_agent.rs:2112-2120`、`install_plan_executor.rs:176-187`）。
  - nfs-server：在 `server.rs:94-95` 旁加一个按主机名分流的层（照 `share` 和 `s3` 的写法）。主机是 `reflow.<zone>` 时，把路径当成桶 `user/control-panel/<当前用户>/reflow` 的 key 读出来，目录补 `index.html`，按原类型下发。`/bucket/v1` 的读取路径会把 HTML 强制成下载（§3.6），这里不能直接复用它的响应头：要么在 nfs-server 里读出对象自己组响应，要么给 `cyfs-ndn` 的 `bucket_store/src/http.rs:174-190` 加一个不套内容策略的读取变体。选改动小的。
  - 登录：照 AIWS 的做法。`src/kernel/buckyos-api/src/app_availability.rs:81-84` 的两份清单（能登录的系统目标、能免密码委托的系统目标）现在只有 aiworkspace 能委托，要把提供重排的服务加进去。Desktop 一侧参考 `src/frame/desktop/src/api/app_launch.ts:36-49`。
  - 验证：`cargo test -p nfs_server`、`cargo test -p scheduler`、`cargo test -p buckyos-api`；网关的 `uv run src/test/test_boot_gatweay/run_debug_tests.py`；真实 Zone 上登录后能打开 `reflow.<zone>/<站点>/`，没登录的打不开，另一个用户看到的是他自己的桶。
- [x] **RF6 App 的重排视图。** iframe 加载重排页（参考 `src/frame/desktop/src/app/web-frame/WebAppFramePanel.tsx:19-104` 和 `web-frame/launch.ts:46-67`）；`postMessage` 的消息桥；制作时间；刷新按钮；“正在制作”；切回原始页面。
- [x] **RF7 管理页。** 偏好和包的列表、查看、删除、导入（上传一个 zip）。
- [ ] **RF8 DV。**（部分：真实 Zone 上除真机、“删掉制作它的 Agent 后仍能打开”之外都验了，用的是 Playwright 的手机视口。见 §12） 真实 Zone、手机：设偏好、兜底重排、写包、再次访问走代码、刷新、删掉制作它的 Agent 后仍能打开。

### 6.10 SR：系统渲染

- [x] **SR1 `page-render`。** `publish/exttool/ext-core/browser/page-render.mjs`，按 §5.7。HTML 输入时拦掉所有网络请求；网址输入时只放行 `--allow-host`。
  - 验证：冒烟（EX7）；带中文的多页 HTML 输出 A4 的 PDF，页眉页脚页码正确，`pdftotext` 能抽出文字。
- [x] **SR2 隔离的启动方式。** `native.rs` 里在 `run_tool_env` 旁加一个：清空环境、进程组、降权、断网，按 §5.7。降权和进程组的写法抄 Node Executor，断网抄 nfs_server 的预览。owner 的要求是简单可靠地跑通（D29）：就做这四样，不加别的机制；宿主上 `unshare` 不可用时记一条警告，靠浏览器里的拦截继续。
  - 验证：单测确认子进程的环境里没有 `WORKFLOW_SESSION_TOKEN`；超时后进程组里没有残留；Linux 上以 root 跑测试时子进程的有效用户不是 root。
- [x] **SR3 两个函数。** 新文件 `functions/media/page.rs`；`mod.rs` 登记；`tools.rs` 加 `page-render`；`native.rs` 加 `Browser` 车道；指纹包含 `page-render` 的版本、浏览器的版本和字体集。改 `tests.rs:76-105` 的断言。真工具的用例标 `#[ignore]`。
- [x] **SR4 系统定义。** `defs/media.page.json`，在 `system_defs.rs` 登记，改 `server/tests.rs:292`。
- [x] **SR5 DV。** `test/workflow_test` 加用例：用一份 HTML 对象跑 `media.page`，拿到 PDF；第二次命中缓存；没有读权限的调用者被拒。
  - 验证：`cargo test -p workflow -- --test-threads=1`；`cargo test -p workflow -- --ignored --test-threads=1`；`uv run test/run.py -p workflow_test`。

### 6.11 DX：文档和收尾

- [x] **DX1 ext-core 相关。** `doc/app_service/paios容器需求.md`（表 B，EX7 已做）、`doc/opendan/OpenDAN Agent-Session-Runtime.md:140`、`doc/pikg/format.md`。
- [x] **DX2 Agent Service 相关。** `src/frame/opendan/README.md`（kRPC、配置）、`doc/opendan/protocol/Agent State Protocol.md`（`state/browser/`）、`doc/path_usage.md`（浏览器目录、重排桶）。
- [x] **DX3 Workflow 相关。** `doc/workflow/media tool baseline.md`、`doc/workflow/executor list.md`、`doc/workflow/workflow service.md`、`doc/llm_context/LLM Understand Media.md` 里列函数的表。
- [x] **DX4 指回本文。** 打印 TODO 里“集成无头浏览器（另行讨论）”和渲染管线方案讨论里“等浏览器引擎”的地方，指到本文的 §5.7。
- [x] **DX5 手册回归。** 按 AGENTS.md“cmd_skills 联动维护”的表逐项检查并写明结论。
- [ ] **DX6 总的 DV。**（部分：§8.3 的容器、网关两项验了；真机手机、原生运行没验） §8.3。

## 7. 顺序

| 阶段 | 任务 | 说明 |
| --- | --- | --- |
| M1 浏览器进包 | EX0–EX8、DT1、DT4 | 做完后开发测试就可用。EX9、EX10、DT2、DT3 可以跟在后面 |
| M2 系统渲染 | SR1–SR5 | 只依赖 M1，可以和 M3 并行 |
| M3 原始页面模式 | BR1–BR8、SC1–SC6、HO1–HO4、APP1–APP6、APP9、APP10 | 另外两种模式的底座。服务端和前端可以并行，按 §5.4 的协议对接 |
| M4 Agent 能操作 | AB1–AB5、APP7、APP8、DG1–DG5 | 对话办理 |
| M5 重排 | RF0–RF8 | RF0 可以提前到 M1 之后任何时候做 |
| 收尾 | DX1–DX6 | 文档跟着各阶段写，最后统一检查 |

M3 之内的先后：BR1–BR3 → BR4、BR5 → SC1–SC3 →（APP3、APP4 可以开始联调）→ SC4–SC6、HO。

## 8. 验证

### 8.1 命令

Rust 的命令在 `src/` 下运行，Desktop 的在 `src/frame/desktop/` 下。包名以各自的 `Cargo.toml` 为准。

| 范围 | 命令 |
| --- | --- |
| ext-core | `publish/exttool/ext-core/build.sh --platform linux-x86_64`；`.github/scripts/test_ext_tools.py`（跑法见 `ext-core.yml:47-49`） |
| libopendan | `cargo test -p libopendan -- --test-threads=1` |
| opendan | `cargo test -p opendan -- --test-threads=1` |
| workflow | `cargo test -p workflow -- --test-threads=1`；真工具：加 `--ignored` |
| buckyos-api | `cargo test -p buckyos-api -- --test-threads=1` |
| nfs_server、scheduler（RF5） | `cargo test -p nfs_server -- --test-threads=1`、`cargo test -p scheduler -- --test-threads=1` |
| Desktop | `pnpm check`、`pnpm lint`、`pnpm test:e2e` |
| 镜像 | 仓库根目录：`./build_aios --local-test`，然后 `uv run src/start.py --skip-update` |
| DV | 仓库根目录：`uv run src/check.py`；`uv run test/run.py -p workflow_test` |

### 8.2 基线就失败的用例

动手前先在没改动的树上跑一遍，记下当时的失败。已知的记录在：

- [Agent UI 升级 TODO](done/agent-ui-upgrade-todo.md) §6.2：Desktop 的 `messagehub-agent-task.spec.ts`、`messagehub-group.spec.ts`、`messagehub-media.spec.ts` 等，以及 lint 的 4 个错误。
- `notepads/opendan-primary-runtime-tools-todo.md:339`：Desktop 的 `pnpm lint` 有 5 个 react-hooks 错误。
- mock 的一处不一致：`agent.list` 的 mock 返回的 Agent（`did:bns:assistant.alice`）在 MessageHub 的 mock 里不算“自己的 Agent”（那里只认 `did:buckyos:agent:codeassistant`）。写 e2e 时要处理。

### 8.3 真实环境

- **容器。** 重建镜像、重新播种 ext-core 后，在真实 Zone 上走一遍 §8.4。
- **手机。** 真机，用浏览器的限速或真实的弱网：看帧率、延迟、流量；真实输入法输入中文。
- **原生运行。** macOS 或 Windows 上的 Agent：同样从 App 使用，桌面上不出现浏览器窗口。
- **网关。** 确认屏幕的 WebSocket 在正式的 443 入口上能通。
- 看 Chromium 输出的 JPEG 帧之前先转成 PNG（V22）。

### 8.4 验收

开发测试：

- 新装的系统上，Agent 在私有 runtime 里新建一个前端项目并跑通 Playwright 测试，不需要自己安装浏览器或系统库。容器和原生运行各验一次。

原始页面模式与交接：

- 在手机上打开「Agent 浏览器」，选一个 Agent，打开一个网站：能看清、能点、能输入中文；限速的弱网下仍能用。
- 下拉框、弹框能用。
- 从 Zone 选一个文件上传；下载一个文件并存到 Zone。
- 用户浏览到某一页后交给 Agent，Agent 从这一页接着做。
- Agent 操作时页面对用户只读；用户接手后 Agent 的操作命令失败；交还后 Agent 重新读页面再继续，并且知道中间发生过接手。
- 用户接手，登录一个网站，交还；Agent 之后能以登录状态继续操作。
- 原生运行的 Agent 同样从 App 使用，桌面上不出现浏览器窗口。

重排：

- 给一个旧网站设定“只看标题、日期和正文，手机单栏”，手机上按此阅读。这个站在库里有包时不调用模型；没有包的站由 LLM 兜底重排。
- 重排页上能看到制作时间；点刷新后内容和时间都更新。
- 常驻浏览器没有运行时，已有结果的重排页照样能打开。
- 删除制作它的 Agent 后，重排页仍能打开；用户名下的另一个 Agent 能刷新它。
- 网站改版、刷新脚本失败时，页面仍能由兜底重排显示出来。
- 用户自己写的一个包导入后，打开匹配的网址时生效。
- 没登录的人打不开重排页；另一个用户打开同一个地址，看到的是他自己的重排库。

对话办理：

- 交代一件跨两个网站的事，Agent 预填已有信息，缺项（包括短信验证码）在对话里问，由 Agent 填进页面并办完。
- 需要密码时 Agent 请用户接手，用户在页面上填；密码不出现在对话记录里。
- 在两件事之间切换，回来时页面和进度都在。

系统渲染：

- 一份带中文、多页的 HTML 输出成 PDF：纸张尺寸正确，中文字体正常，文字可选可搜。
- 渲染进程拿不到 Workflow 的会话令牌，访问不了网络。
- AIWS 导出 PDF 的整体验收在打印 TODO §7。

通用：

- 不是 owner 的人连不上屏幕。
- 清除登录状态后，网站回到未登录。
- 常驻浏览器空闲后被回收，再用时能恢复各标签页和登录状态。
- 节点上浏览器不可用时，系统其他部分照常工作，Agent 和 App 都得到明确的“浏览器不可用”。

## 9. 待确认

Q1–Q26 都已答复（§2）。下面是实施和真实环境验证中新出现的问题，每条写了现在的做法；owner 没有另定之前按现在的做法走。

发布：

| # | 问题 | 现在的做法 |
| --- | --- | --- |
| Q27 | Chrome for Testing 的 Linux、macOS 归档里带着 Widevine CDM，它的 `LICENSE` 写明没有和 Google 的单独协议不得分发 | **已裁决（D31–D34）**：ext-tool 加“在线项”，Chromium 不再随包，节点装好包以后从 Google 的官方地址下载。我们不分发它，这条阻塞项和 D26 的再分发风险一起消掉 |
| Q28 | ext-core 升到 1.3.0 之后，`build-all.yml` 的 `resolve-ext-tools` 要求这个版本已经发布；发布被 pandoc（原有）拦着。另外 `ext-core.yml` 的 pikg 作业用 `npx buckyos@latest pikg doctor`，要等认得 `online` 字段的 websdk 发布后才能过 | 没处理。开发机不受影响（`buckyos-build.py` 忽略这一步的失败，沿用本地种子） |
> 已经发布了
| Q29 | ext-core 里用 fontconfig 的工具装到别的路径后读不到 `conf.d`（原有的缺陷：简体字回退对 `soffice` 实际不生效）。要不要在 `env/*.sh` 和 Windows 启动器里统一加 `FONTCONFIG_PATH`？表 B 写的“不读系统字体目录”也和现实不符 | 只给浏览器的包装脚本和 `agent-browser env` 加了。别的工具没动：加了会改变它们看到的字体 |
> 按你的建议

产品和交互：

| # | 问题 | 现在的做法 |
| --- | --- | --- |
| Q30 | 默认事情绑定哪个会话 | 第一次用到时取“最近活跃、且不是在浏览器里新建的”那个和 Agent 的对话，可能是用户在 MessageHub 里正在用的任意一个。另一种做法是给默认事情建一个专门的会话 |
> 每个agnet有一个专门处理“重排的chat session”, 不为每个事情建一个。所有和浏览器使用的都在这里。
| Q31 | `browser` 手册要不要进对话的默认手册集合 | 没进（进了每个对话的系统提示多约 11 KB）。靠意图分析给 Work 会话配手册，真实模型验证里 5 个 Work 会话都配上了；对话自己不直接操作浏览器 |
> 如上，可以考虑上面的重排专用chat session有这个手册?
| Q32 | 手机上画面的清晰度和流量怎么取舍 | 没做高清晰度。现在的帧是 CSS 像素（手机 412×780），在 2.6 倍的屏上放大显示，小字能认但发糊。实测只有一种办法不改页面布局和输入坐标：给浏览器加一块 2 倍的虚拟屏，有人要高清时把标签页的窗口移过去。代价：流量约 2.7 倍（全屏动画 309 → 839 KiB/s，静止一帧 20.8 → 56.2 KB），动画时 CPU 约 4 倍；这期间 Agent 自己的截图像素也翻倍；1.5 倍是 554 KiB/s。要做的话协议加一个 `pixel_ratio`（做法和数据见 §12 屏幕修复那一行） |
> 按你的建议，这个最好能根据网速动态调整。真正解决网速问题的其实是重排
| Q33 | 新账号的桌面要不要默认放「Agent 浏览器」的快捷方式 | 没放，从“所有应用”进。放了会改新账号的默认桌面 |
> 好，现在这个还是高级功能
| Q34 | 几处我定的默认：默认事情叫“日常浏览”；新事情默认名“浏览器里的事 月-日 时:分”；交给 Agent、交还都先出一个可以补话的确认面板；地址栏输入的不是网址时去 Bing 搜索；改了重排偏好自动给 Agent 发消息请它改包重做；用户刚按过页面时文件选择器直接弹出，否则只显示提示 | 如左。文案在 `src/frame/desktop/src/i18n/agent-browser*.ts` |
> 这个“事情” 是逻辑上的，我觉得UI上没必要做专门的支持。大部分普通用户压根做不好topic管理
| Q35 | 重排的 `<站点>` 和匹配规则都不含端口，同一主机不同端口共用一个包 | 如左。开发机上 `localhost:3000` 和 `localhost:8080` 会互相套用 |
> 现在内核跑通了，重排包的生命周期再单独设计。我今天试用了后，觉得非实时重排的价值比想象中大，比如可以先缓存相关资源到本地，甚至对这些资源做处理（比如无字幕的视频加个字幕）。甚至只是把看过的meida从本地缓存变成组织好的本地存储，都有价值。重排这个词可能把功能说窄了，实际上用户拥有了 一段连续时间观察网站的能力，而不是过去只是打开这一刻的网站。新网页 = 重排(网址，当前时间，历史记录)
| Q36 | `media.page` 的默认值和截图上限 | A4、边距 10 mm、打印背景；截图超过 4096² 像素失败（宽 1280、倍数 2 时页面高过约 6500 px 就超） |
> 按你的建议

权限和边界：

| # | 问题 | 现在的做法 |
| --- | --- | --- |
| Q37 | 下载存到 Zone：`browser.download_export` 登记对象后要给调用者登记持有，这要 `obj://ndm/grant`，普通用户 owner 和只读 Agent 没有 | 被拒只记日志，照常返回 objId；管理员 owner 实测能存，普通用户 owner 会在随后的提交对象那一步被拒 |
> 按你的建议
| Q38 | 重排的 `refresh.mjs` 以 Agent Service 的身份运行（容器里是 root），读得到服务的文件；能写 owner 重排桶的一方（owner、读写权限的 Agent）都能让它在这个 owner 的任一个 Agent 的容器里执行 | 子进程的环境是清空的，拿不到令牌；文件系统没有隔离（D27：重排的代码看作我们自己生产的） |
> 好，安全登记就是Agent为我定制的“可信产物”
| Q39 | 我加的一处收口：种在 `reflow.<zone>` 上的会话在 nfs-server 的文件、桶、NDM 接口上不算登录 | 已实现并验证。不加的话受限账号能借“nfs-server”这个新的登录目标用上文件接口 |
> 按你的建议
| Q40 | 原生运行的 private runtime 默认声明的基线是 standard，`browser` 手册是 extended | 节点声明成 extended 之后手册才生效，和其他 extended 手册一样。原生运行没有实测（DT3） |
> 按你的建议
| Q45 | 在线项下载到一半节点停了 | **owner 2026-11-15 认可。** 重启后记成失败（“下载时节点停了”），不自动重试，要手工再触发。照“失败不重试、重启也不重试”做的；另一种做法是中断后自动再试一次 |
| Q46 | 对已经就绪的在线项手工触发下载 | **owner 2026-11-15 认可。** 重新下载并整体换掉，当作修复用；界面上要二次确认 |
| Q47 | 在线项下载的超时和代理 | **owner 2026-11-15 认可。** 连接 30 秒、连续 60 秒没有数据、总共 2 小时；不做断点续传。走 HTTP 客户端的默认行为，会用环境变量里的系统代理（这不是可覆盖的下载源） |
| Q48 | macOS 离线包的签名脚本（`pve-pack-system/bin/pack/sign_macos_ext_tools.py`）只认 payload 顶层的 `ext-manifest.json`、`bin`、`store`、`smoke`，并且会跑清单里所有的冒烟 | 没改（另一个仓库）。要放行 `online/`，并跳过要浏览器的冒烟（`chromium`、`page-render`），否则 mac 包签名那一步会失败。owner 2026-11-15：当前环境做不了，由 owner 后续验证 |
> 另行验证
| Q41 | 只读权限的 Agent 发起的重排刷新 | 照常跑，写结果时被桶拒绝，状态是失败。没有事先拦。只读 Agent 的两条验收没验：权限模式只能在创建 Agent 时定 |
> 按你的建议

这次发现、不属于本需求、没有改的：

| # | 问题 |
| --- | --- |
| Q42 | scheduler 按 `app_index * 16 + 10000` 分配服务端口，第 5 号 App 得到 10080，它在 fetch 规范的禁用端口表里：任何用 `fetch` 直连这个端口的本机客户端都失败（Node 报 `bad port`，Deno 报 `Requests to port 10080 are blocked`）。这次只把 `agent-browser` 和 `reflow refresh` 改成了不用 `fetch` |
| Q43 | workflow 服务只在启动时探测一次媒体工具。ext-core 晚于它装好时（首次播种、重新播种），`media.*` 报 `tools_missing`，要重启服务才好 |
| Q44 | `behaviors/work_simple.toml` 里 `ask_user` 的用法行带 `[--allow-text]`，模型给纯文字问题也加上，被命令拒绝，每次多花一轮 |

PRD 待定的产品选择（R8）不在本文裁决：默认入口先按 §5.5 的写法做；首批场景和 RF0 一起选，避开要 U 盾、客户端控件、人脸识别的网站；协作控制的文案和样式归产品。

## 10. 已知风险

owner 定了先能力再安全（D18）。下面这些第一期不处理：

- **提交前不确认。** Agent 填完就提交，包括付款、对外发送、删除这类不可逆动作。
- **网页内容是不可信输入。** Agent 带着浏览器里的登录状态读第三方网页，网页里的文字可能诱导它做交代之外的事。
- **密码隔离和接手靠手册**（D22，V19）。Agent 绕开 `agent-browser` 直接连调试端点时，读得到密码框的值，也能在用户接手时继续操作。
- **Chrome for Testing 的再分发许可**没有查到权威说法（V21）。owner 决定直接打包（D26）。
- **重排页运行在带会话的源上**（D27：代码是我们自己生产的，安全边界看作一致）。但页面里的数据来自第三方网站，页面代码把它当 HTML 直接插入的话，对方的脚本就跑在这个源上了。手册要求按文本渲染。
- **容器里可能留下僵尸进程**（D30）。浏览器被强杀、又有没记到的子进程时，它们会一直挂在 1 号进程下，直到容器重启。只占进程号。真实环境验证里看到的另一个来源：Agent 在会话里跑项目的 Playwright 测试时自己起的浏览器，结束后留下的 `chrome-bin`、`chrome_crashpad_handler` 也挂到了 1 号进程下。
- **原生运行时的调试端点**在本机回环地址上，同一台机器上的其他进程也连得上。容器里没有这个问题。
- **容器里关着 Chromium 自身的沙箱**（V4）。隔离边界是容器。
- **Agent 的 Playwright 连接会自动关掉弹框**（V18）。`agent-browser` 做了防护；Agent 自己写连接代码时没有。
- **系统渲染在不支持沙箱的宿主上**以 `nobody` 身份、不带浏览器沙箱运行第三方 HTML。有断网和降权，没有文件系统隔离。
- **Playwright 版本不一致**时指到系统浏览器不保证能用。
- **日期、时间、颜色输入框**在原始页面模式下第一期只能当文本框用。
- **内存。** 常驻浏览器开几个标签页就要 400 MiB 以上（V8）。低配的盒子上要靠空闲回收和标签页上限。

实施和验证中补充的：

- **屏幕连接只在握手时校验令牌。** 令牌过期或 owner 被撤权之后，已经连上的屏幕继续可用，直到断开。
- **重排的刷新脚本以 Agent Service 的身份运行**（Q38）。owner 的说法（D39）：重排包是“Agent 为我定制的可信产物”，按这个边界登记。
- **下拉框的拦截一直开着。** 没人看、Agent 在操作时，点下拉框也不会弹出原生的选项列表（无头模式下本来也看不见）。Agent 用 Playwright 的 `selectOption` 不受影响。
- **页面环境里包了几个原生函数**（定位、摄像头、通知等的入口），用来告知用户“页面请求了权限、已被拒绝”。页面察觉得到。
- **有人用高清屏看的时候，页面的设备像素比是 2**（D37 的 2 倍虚拟屏）。没人看之后标签页留在原来的屏，Agent 操作期间也一样。`agent-browser screenshot` 已改成按 CSS 像素截（`scale: 'css'`），手册也要求脚本里的 `page.screenshot()` 带它；Agent 自己写的截图代码不带时，图的像素会翻倍。
- **Chrome 把崩溃转储写在浏览器目录之外**（`~/.config/google-chrome-for-testing/Crash Reports`）。
- **下载导出在 Agent 的 `artifacts` 桶里留下对象**，没有清理。
- **同一个还没做好的网址可能请 Agent 两次**：“已经请过”记在浏览器标签页的 `sessionStorage` 里，换设备或新窗口打开会再请一次。
- **`browser.status` 里的“办完 / 卡住”不随会话归档清掉**，要等这个组下一次租约、接手或交还。
- **Windows、macOS 的进程代码没有编译和运行过**（Job Object、`taskkill`、`.cmd` 启动器、没有 `/proc` 时的进程回收）。Windows 上重排刷新经 `.cmd` 启动器传带 `%`、`&` 的网址可能起不来。

## 11. 不在本期

- 虚拟机本身。屏幕接口的 RFB 实现等有虚拟机时再做。
- WebRTC 画面传输。
- 不属于任何 Agent 的浏览器。
- Firefox、WebKit。无头精简构建。
- 独立的浏览器扩展包（D6）。
- ComputerUse 的浏览器语义驱动（D16）。
- 提交前确认、对 Agent 操作的强制拦截、补填框、密码以外的敏感字段交回用户（D3、D18）。
- 系统渲染的网址输入作为 Workflow 的入口；AIWS 一侧的接入（在打印 TODO）。
- 重排页的沙箱隔离和能力令牌（D27）。给 Agent 容器加 `--init`（D30）。
- 外部网页由系统渲染转 PDF（由 Agent 的浏览器做）。
- 浏览器扩展、书签同步、从用户别的浏览器导入登录状态。
- 同一块屏幕给 owner 以外的人看或操作。
- 按固定周期自动刷新重排结果的调度。
- PRD“长期判断”里超出重排模式的部分。

## 12. 实施记录

实施时在这里记。每条写：日期、任务号、改了什么、跑了什么验证和结果、和本文不一致的地方。

| 日期 | 任务 | 记录 |
| --- | --- | --- |
| 2026-11-14 | 验证 | 容器实测，结果见 §4。测试脚本没有进仓库 |
| 2026-11-14 | 裁决 | owner 答复 Q22–Q26（D26–D30）：打包 Chrome for Testing；重排页按标准 App 的主机名规则提供；系统渲染降权；容器启动方式不变 |
| 2026-11-15 | 契约 | 动手前把跨包的接口写成 `doc/opendan/protocol/Agent Browser Protocol.md`（kRPC 的返回结构和错误码、屏幕 WebSocket 的消息、本机文件上传、重排的规则和消息）。各包按它对接，实施中的调整都回写在里面；和 §5 的差别列在 §5.8 |
| 2026-11-15 | EX0–EX8、EX10 | `publish/exttool/ext-core/`：`build.sh` 加 `install_chromium`（五个平台）、`install_playwright`、`assert_chromium_loads`、Linux 包装脚本、三个启动器、`versions.json`；不再删 atk / atspi，conda 加 `libgbm`（锁文件只并入这一项，没有全量重解：全量会让每个平台二三十个无关的包漂移）；字体配置加上 Noto CJK 的目录；清单、`dapp_meta` 升 1.3.0；`sources.sha256`、`official-sources.json`、`compliance.py`、`.github/scripts/ext_tools.py`、`ext-core.yml`；`baseline.rs`（`ExtToolCategory::Browser`、`EXTENDED_TOOLS`、`playwright_env`）和 `session_env_vars`；表 B（27 条）。验证：`build.sh --platform linux-x86_64` 构建出完整的 payload，但退出码 1，停在最后的 `compliance.py finalize`（pandoc 是原有的阻塞项，另加 Q27）；`test_ext_*.py` 11 过、ext-core 的 `test_*.py` 19 过、`check-table` 过；裸 `ubuntu:24.04`（root、断网、64 MiB `/dev/shm`）里 13 个冒烟全过；`env -i` 直接执行浏览器成功；V1 的 25 个库全部解析到 `conda/lib`；包里的 node 加 playwright-core 用 `channel: 'chromium'` 出中文 PDF；以 `nobody` 加清空的环境跑 `page-render` 通过 |
| 2026-11-15 | EX9 | linux-aarch64、windows-x86_64 的 payload 构建了：aarch64 只做了依赖的静态检查，没运行；Windows 的启动器和冒烟没在 Windows 上跑过。darwin 两个平台没构建（只能在 Mac 上） |
| 2026-11-15 | EX10 | 解压 / tar.gz：linux-x86_64 2.3 G / 874 MiB（1.2.0 是 676 MiB），linux-aarch64 2.3 G / 857 MiB，windows-x86_64 2.8 G / 971 MiB；darwin 估算约 800 MiB，没测。都在 2 GiB 以下。linux-x86_64 里 `chromium/` 414 M、`playwright/` 14 M、许可文本 138 M |
| 2026-11-15 | AB1–AB3、SR1 | `publish/exttool/ext-core/browser/`：`agent-browser.mjs`、`page-render.mjs`、`cli.mjs`、`runtime.mjs`、`krpc.mjs`、`test/`；`smoke/{agent-browser,page-render}.{sh,ps1}`。验证：`node --test "test/*.test.mjs"` 对着假的 Agent Service 加真 Chromium，宿主机和裸容器都全过（最后一次 27 条：26 过，1 条因本机 10080 端口被占而跳过，在隔离的网络命名空间里单独跑过）。`.ps1` 没执行过 |
| 2026-11-15 | BR1–BR8、HO1–HO4 | `src/frame/opendan/src/browser/`（`mod.rs`、`process.rs`、`cdp.rs`、`groups.rs`、`sessions.rs`、`rpc.rs`）、`tests/fake_browser/`；`service.rs`（`caller` 改成收令牌、方法名、IP）、`loader.rs`、`config.rs`、`Cargo.toml`（D28 的四个 crate，另加 unix 下的 `libc`）；`lib_opendan` 的 `_browser` 隐式半订阅；`publish/aios/entrypoint.sh`。验证：`cargo test -p opendan` 全过；真浏览器的 4 条 `#[ignore]` 用例（启动停止无残留、杀掉后重启、空闲回收后恢复标签页和 cookie、Playwright 连 `cdp_url`）通过；容器里以 1 号进程、root 跑确认孤儿进程被回收 |
| 2026-11-15 | SC1–SC6 | `browser/` 下 `screen.rs`、`input.rs`、`requests.rs`、`transfer.rs`、`http.rs`、两个注入脚本、`screen_tests.rs`。验证：带真实升级的握手用例；真浏览器 5 条（帧、手机视口、点按、中文、组字、按键、下拉框、弹框和超时、权限、上传、下载、租约期间输入被丢、杀浏览器后恢复）通过。实测到的事实：`Runtime.addBinding` 要先开 `Runtime` 域；CDP 的 `touchEnd` 带的是抬起的点；视口改动紧跟刷新时不出帧，要重开截屏流 |
| 2026-11-15 | SR2–SR4、DX3 | `src/kernel/workflow/`：`native.rs`（`run_tool_isolated`、`Browser` 车道）、`functions/media/page.rs`、`tools.rs`、`mod.rs`、`defs/media.page.json`、`system_defs.rs`；`Cargo.toml` 加 `zip`（workspace 已有）和 unix 下的 `libc`。验证：`cargo test -p workflow` 98 过；`--ignored` 的真工具用例通过（4 页中文 A4、页脚页码、截图、zip 输入、被拦的请求进 `warnings`）；容器里以 root 跑，子进程是 `nobody`、环境里没有令牌。Windows、macOS 分支没编译 |
| 2026-11-15 | RF1、RF2、RF5 | `control_panel_bucket_decls()` 加 `reflow`（用户桶、`app_data`、1024 MiB）；`src/tools/buckyos-agent/commands/reflow.ts`、`lib/reflow.ts`，`STANDARD_TOOLS`、`EXT_TOOLS_RESERVED_COMMANDS`（Rust 和 websdk 的 `cli/modules/pikg_protocol.ts` 两份）、表 A；scheduler 给 nfs-server 的主机加 `reflow`（保留名有三份，不是两份）；nfs-server 的 `bucket/reflow.rs`（没动 `cyfs-ndn`：读出对象后自己去掉下载和沙箱的响应头）；`app_availability.rs` 两份清单加 `nfs-server`。验证：`buckyos-api`、`scheduler`、`nfs_server` 的用例全过（含登录用户读到自己的桶、目录补 `index.html`、没登录 401、另一个用户、路径穿越）；`deno test` 52 过；网关调试用例 46 过 |
| 2026-11-15 | RF3 | `browser/reflow.rs`：三个方法、规则（对着共用向量测）、桶读写走 nfs-server 的 `/bucket/v1` 加调用者的令牌、刷新子进程。验证：13 条用例；真的 `agent-browser reflow-run` 加真 Chromium 通过。修了一个时序问题：刷新恰好结束时 `reflow_state` 可能返回“空闲但没有结果”，App 会误请 Agent 兜底 |
| 2026-11-15 | RF4、RF6、RF7 | Desktop `app/agent-browser/reflow/`（规则的移植、zip 和 Zone 目录两种导入、控制器、消息桥、地址栏、偏好、管理页）。验证：mock 上 e2e 20 条 |
| 2026-11-15 | APP1–APP10、DG1–DG4 | Desktop `app/agent-browser/`（外壳、store、屏幕通道和视图、页面之外的界面、对话面板、交接面板、MessageHub 入口）、`i18n/agent-browser*.ts`、登记五处、`vite.config.ts`（`ws: true`）、`DesktopRoute.tsx`（深链）、`messagehub/agentpanel/AgentSessionPanel.tsx`（入口）、OpenDAN WebUI 首页的链接。验证：`tsc` 通过；eslint 和基线相同（4 错误 33 警告，都不在本 App 里）；mock 上 `agent-browser*.spec.ts` 四个文件 49 条全过；`vite build` 通过 |
| 2026-11-15 | AB4、DT4、DX1–DX5 | 新增 `cmd_skills/browser/SKILL.md` 和 `references/reflow.md`；改 `ui_core`、`opendan_agent_dev`、`buckyos_app_dev`（Page tests）、`buckyos_dev_release`；打印 TODO、渲染管线讨论指回 §5.7；`doc/` 下 path_usage、Agent State Protocol、Session Input Protocol、xAgent、Agent-Session-Runtime、workflow 四份、SSO 两份、桶的两份、licensing。验证：`cargo test -p libopendan --lib skills`、`--test xagent_cli` 的两条手册用例、Jarvis 包加载用例通过；手册里的脚本和命令对着 payload 和真 Chromium 逐条跑过 |
| 2026-11-15 | DT1、DT2 | 裸容器里只设会话的两个环境变量：装配套版本的 `@playwright/test`、写 `channel: 'chromium'` 跑通，没有下载浏览器；去掉 `channel` 时报找不到 headless shell；换成 1.63.0 时按 `agent-browser env` 在子 shell 里下载它自己的浏览器并跑通。真实 Zone 的容器会话里由真实模型按手册又走了一遍（见 DV 的行） |
| 2026-11-15 | 集成 | 合并后 `cargo test -p opendan -p libopendan -p workflow -p nfs_server -p scheduler -p buckyos-api`：73 个 target、1454 条通过；失败的 7 条都在 libopendan，是临时构建目录的环境差异（缺工具目录），在 `src/target` 里重跑全过。`buckyos-build.py` 通过。`build_aios --local-test` 第一次失败，原因是 `/tmp` 下复用的 Cargo 缓存被系统清理删了一部分，和代码无关，清掉缓存后通过 |
| 2026-11-15 | 部署 | 本机 Zone（`test.buckyos.io`）：`prepare_ext_tools.py --local`、删 `tools/installs/ext-core.json`、`start.py`，node_daemon 装上 ext-core 1.3.0，三条新命令都探测到版本。现有的 Zone 没有 `reflow` 桶（`services/control-panel/spec` 只在激活时生成），手工给 spec 的 `spec_config.buckets` 加了一项。建了三个测试 Agent：webby、reflowy、agenty |
| 2026-11-15 | SR5 | `WF-DV-017`（HTML 对象经 `media.page` 出 PDF、第二次命中缓存、没有读权限的调用者被拒）和 `WF-DV-012`（5 个系统定义、12 个函数）在本机 Zone 通过。第一次跑时 `media.page` 报 `tools_missing`，原因见 Q43，重启后通过。`test/run.py -p workflow_test` 在它的环境里找不到 `deno`，是直接用 `deno run` 跑的。另补了隔离的证据：渲染期间 11 个进程属主都是 `nobody`，PDF 里的外部请求全部失败，测试站点收到 0 条请求（进程的环境没读到，只验了属主） |
| 2026-11-15 | DV：原始页面和交接 | 真实 Zone，Playwright 驱动 App（开发代理和部署版各一遍），Agent webby。通过：浏览和标签页；逐键、整段文字、粘贴、功能键、输入法组字（用 CDP 模拟到浏览器的输入法事件这一层）；手机视口下页面走手机分支、点到输入框后键盘输入框获得焦点、桌面版开关；下拉框、三种弹框、权限告知；从 Zone 选文件上传（中文名，内容一致）、本机文件上传、下载后存到 Zone 并读回；交接的全部状态；真实 Agent 从用户停下的页面接着做（没有重新加载）；新建事情和切换；`bob`、伪造令牌、无令牌连屏幕和调 kRPC 都被拒；清除登录状态；空闲 601 秒后回收，再连时标签页和 cookie 都在；杀掉浏览器后同一条连接 1.2 秒恢复；屏幕的 WebSocket 经 443 入口可用。数字：全屏动画默认 14.9 帧/秒、163 KiB/s；`max_fps 5` 加画质 30 是 39.5 KiB/s；静止页 0 帧；限速 400 kbit/s 加单向 150 ms 延迟时 3.3 帧/秒、点按到页面 224 ms；100 kbit/s 加 300 ms 时 0.9 帧/秒、新画面 1.7 秒；手机一分钟浏览下行 6.37 MiB。新增 `tests/dv/agent-browser.spec.ts`（8 条） |
| 2026-11-15 | DV：重排 | 真实 Zone，部署的 Desktop，Agent reflowy。通过：`reflow.<zone>` 上登录后按 `text/html` 下发、没登录 401、`bob` 看到自己的桶、重排源上的令牌调文件和桶接口被拒；三种放包方式；刷新、点“下一页”的动作、失败和排队；刷新脚本的环境里没有令牌；App 的四个触发分支，其中“没有包”真的请 Agent 做了一次兜底（80 秒做成并显示）；制作时间、刷新后更新；真等 16 分钟后会话自动续上；手机视口；浏览器停掉后已有结果照样打开；另一个 Agent 读到同一份结果；管理页。新增 `tests/dv/agent-browser.reflow.spec.ts`（10 条）。没验：`[loader.browser] enabled = false`、删除 Agent 后另一个 Agent 刷新、Agent 修包 |
| 2026-11-15 | DV：Agent 侧（DG5 的一部分、AB5） | 真实 Zone、真实模型（gpt-6.1-sol），Agent agenty，8 条消息，约 386 万输入 token。通过：容器会话里跑通 Playwright 测试；用 `agent-browser` 填表提交（站点收到的内容正确，屏幕上看得到，期间用户的点按被拒）；缺的电话用 `ask_user` 在对话里问，回答后填入并提交，不另确认；密码接手（Agent 请用户接手，用户经屏幕填密码并交还，Agent 重新读页面后继续；密码串在对话、工作记录、AgentRoot 里出现 0 次）；用户接手后 Agent 的命令以退出码 4 失败并停下；两件事各一组标签页；Agent 侧的 `reflow result put`、`reflow put`、`reflow refresh`；Agent 的令牌调 `browser.reflow_action` 被拒。新增 `test/agent_browser_dv/`。没验：跨两个网站的事、短信验证码、只读 Agent（Q41） |
| 2026-11-15 | DV 发现并修掉的 | ① 服务端口是 10080 的 Agent 用不了 `agent-browser` 和 `reflow refresh`（Q42）：两处改用 `node:http`。② `agent-browser status` 不标本会话的组：改成只显示本会话的组。③ 页面测试被意图分析选成了 `browser` 手册（那一条消息花了 111 万输入 token）：改了两本手册的描述。④ 重排刷新失败的原因一律是“超时”：页面动作的超时改成比整次刷新短，报错指出是哪个选择器。⑤ 用户换了网址后，对旧网址的兜底请求仍发给 Agent：App 发之前核对网址。⑥ 清除登录状态后浏览器一直停着：App 清除后接着启动。⑦ 等着的文件选择请求在重连时让 App 立刻弹出选择器：改成用户刚按过页面才自动弹 |
| 2026-11-15 | 屏幕修复（DV 之后） | `browser/screen.rs`、`process.rs`：① 一个标签页只有一个生效的视口，是正在看它的连接里最近设置的那一条的；它离开后换成剩下的里最近设置的，不重新加载；每条连接的 `state.viewport` 报的是生效的那个。原来两块屏幕同看一个标签页时互相覆盖，后来的走了先来的不恢复。② 启动参数加 `--disable-blink-features=CaretBlinking`：输入框有焦点时光标闪烁每秒出 2 帧整帧（约 43 KiB/s）。这个开关 Chromium 159 认、156 不认，换版本要重跑 `the_caret_of_a_focused_field_paints_no_frames`。③ Agent 的租约结束时给正在看的标签页重套视口：Agent 的 Playwright 截图之后，屏幕套的手机模拟会失效（原样重发同一条模拟命令不管用，要先发一条不同的）。Agent 操作期间截图之后到租约结束之前，页面不是手机的样子，这一段没处理。④ 换视口后约六分之一的概率不出新尺寸的帧：0.4 秒内没等到就重开截屏流。验证：`cargo test -p opendan` 全过；真浏览器用例（payload 的 Chromium 159）屏幕 8 条、核心 4 条、重排 1 条通过 |
| 2026-11-15 | 手机画面的清晰度（没做，Q32） | 实验数据（手机 412×780、画质 60）：现状帧 412×780，静止一帧 20.8 KB，全屏动画 309 KiB/s。`startScreencast` 的 `maxWidth/maxHeight` 调大不放大；模拟参数的 `scale` 会把 CDP 的输入坐标一起放大；`--force-device-scale-factor=2` 让所有标签页一直按 2 倍渲染；把视口放大会改掉页面布局。只有一种办法不改布局、设备像素比和输入坐标：启动参数 `--screen-info` 加一块 2 倍的屏，要高清时用 `Browser.setWindowBounds` 把标签页的窗口移过去，再重套一次视口；帧变成 824×1560，静止一帧 56.2 KB，动画 839 KiB/s，动画时浏览器的 CPU 从 0.29 核到 1.06 核；用 `maxWidth/maxHeight` 压到 1.5 倍是 618×1170、554 KiB/s。副作用：这期间 Agent 的 `page.screenshot()` 像素翻倍；每次切换页面收到一次 `resize`；`screen.isExtended` 恒为真。另外看到截屏流在服务端一直按每秒 60 帧编码，15 帧的上限只是丢帧，动画页面 1 倍下就占 0.3–0.5 核 |
| 2026-11-15 | 重新部署后复验 | 全部修复合入后重新构建、重新播种 ext-core、重建镜像、重启。① 服务端口是 10080 的 agenty 容器里，`agent-browser status`、`open`、`snapshot`、`done` 正常，`reflow refresh` 连得上服务（修复前是 `bad port`）；`status` 只显示本会话的组。② 浏览器进程带着普通 Chrome 的 UA 和关闭光标闪烁的开关。③ 屏幕（经 443 入口）：桌面视口先连，手机视口加入后两端都报手机视口，手机断开后桌面端 42 ms 内拿回自己尺寸的帧；点进输入框后静置 5 秒 0 帧。④ `tests/dv/agent-browser.spec.ts` 8 条全过，`tests/dv/agent-browser.reflow.spec.ts` 9 过 1 跳过（跳过的是默认不开的真实模型兜底那条），都在部署的 Desktop 上。⑤ `test/agent_browser_dv/main.py` 的三项不花 token 的检查通过。⑥ 回归：Desktop `tsc` 通过、eslint 和基线相同、mock 上四个 spec 49 条全过；Rust 见下一行 |
| 2026-11-15 | 最终回归（Rust） | 在 `src/target` 里 `cargo test -p opendan -p libopendan -p workflow -p nfs_server -p scheduler -p buckyos-api --no-fail-fast -- --test-threads=1`：73 个 target、1459 条通过、30 条 ignored、3 条失败，都在 libopendan，和这次改动无关：`concurrent_reservations_share_a_counter_and_reuse_the_same_key` 是负载下的偶发（当时同时在跑 Playwright），单独重跑通过；`xagent_cli` 的 2 条要求 `src/tools/bin` 和 `src/bin/agent-tools` 两个目录存在，临时建上后 15 条全过（建完已删） |
| 2026-11-15 | 裁决 | owner 答复 Q27（D31–D34）：ext-tool 支持“在线项”，Chromium 是第一个；下载地址只写官方的；装完自动下载，失败可以接受、在管理面板手工重新触发；这次只做 Chromium。随后认可 Q45–Q47 的默认做法，Q48（mac 离线包签名）由 owner 后续验证 |
| 2026-11-15 | 在线项：机制 | `buckyos-api`：`ext_tools.rs`（清单的 `online`、`needs` 和校验，payload 放行 `online/<id>/`，状态类型）、`ext_tools_install.rs` 和新的子模块 `ext_tools_install/online.rs`（下载、SHA256、zip 保留权限位和符号链接、`into` / `rename` / `overlay`、暂存后换名、原子写记录、`waiting`、包激活时重盖 overlay、孤儿清理、`opendan ext-tools online*` 子命令）；node_daemon 的 `plan_online`（下载在 reconciler 之外的任务里做）；control-panel 的 `ext_tools.download`、`ext_tools.status` 带在线项；websdk 的清单校验和 `buckyos ext-tools download`。顺带修了引擎原有的两处并发问题（探测的输出文件撞名、锁的误报）。验证：`buckyos-api`、`node_daemon`、`control_panel` 的用例全过，websdk 179 过；从 Google 官方地址真实下载一次（约 200 MB，下载 3.4 秒，解包换名探测 7 秒） |
| 2026-11-15 | 在线项：打包 | `publish/exttool/ext-core/`：`build.sh` 不再把 Chromium 打进 payload，只生成 overlay（两个标记文件和 Linux 的包装脚本）；构建时仍从官方地址取归档、摆好后检查依赖库和能否启动，检查完删掉；启动器按新位置找浏览器；新增 `place_online.py`（CI 和本地照清单摆浏览器）；清单声明 `online`；合规材料去掉 Chromium 和 Widevine 那条阻塞项（`build.sh` 只剩原有的 pandoc 一项）；`ext-core.yml` 的冒烟先摆浏览器再跑。官方地址和 Playwright CDN 是同一个文件。体积（tar.gz）：linux-x86_64 687 MiB（带浏览器时 874，1.2.0 是 676），linux-aarch64 670，windows-x86_64 761。验证：裸容器里按节点布局 13 个冒烟、29 条脚本测试全过 |
| 2026-11-15 | 在线项：使用方 | opendan 的浏览器模块区分 `no_package`、`not_downloaded`、`disabled`，每次现读记录、不缓存，下载好不用重启；Workflow 把 `page-render` 的退出码 6 映射成 `tools_missing`；协议文档同步。`baseline.rs` 和 `skills.rs`：浏览器没下载时 `chromium` 记成“等下载”（`ext.waiting`、`waiting_online`），不算缺失、不推荐启用 ext-core。Desktop“设置 > 扩展工具”显示在线项和进度，管理员可以下载、重试、重新下载；「Agent 浏览器」App 按三种原因说明，管理员能直接下载；OpenDAN WebUI 的 Skills 页、Runtimes 页显示“等下载”。验证：opendan、workflow、libopendan 的用例全过；Desktop mock e2e（Agent 浏览器和设置页）80 条、OpenDAN WebUI 58 条全过 |
| 2026-11-15 | 在线项：真实 Zone | 全部合入后重新构建、重新播种（不带浏览器的 ext-core，种子 686 MiB）、重建镜像、部署。ext-core 在启动后 21 秒装好，`installs/ext-core.json` 的 `waiting` 是 `chromium`、`page-render`；节点随即自动从 Google 下载，29 秒时 `ready`（下载约 6 秒）；`waiting` 清空，三条命令都探测到版本；目录 0755、记录 0644。Agent 的 `browser.status` 可用，容器里 `chromium --version`、`agent-browser open`、`done` 正常。`buckyos ext-tools download ext-core chromium`（经 `--endpoint http://127.0.0.1:3180`）手工重新下载：记录保持 `ready`、新版本在 `next` 里走完 7 秒后换上，期间 Agent 的浏览器一直可用。部署的 Desktop 设置页显示“在线项 chromium 159.0.8154.6 199 MB · ood1 已就绪 · 重新下载”。重启一次让 Workflow 重新探测（Q43）后：`WF-DV-017`、`WF-DV-012` 通过，`tests/dv/agent-browser.spec.ts` 8 过，`tests/dv/agent-browser.reflow.spec.ts` 9 过 1 跳过，`test/agent_browser_dv/main.py` 通过。没在真实 Zone 上验的：“还没下载”时 App 里管理员点下载的那条路（mock 上验过；真实 Zone 上浏览器在 Agent 起来之前就下载好了），下载失败后重试 |
| 2026-11-15 | 在线项之后的回归（Rust） | 在 `src/target` 里 `cargo test -p buckyos-api -p node_daemon -p control_panel -p opendan -p libopendan -p workflow --no-fail-fast -- --test-threads=1`（临时建了 `src/tools/bin`、`src/bin/agent-tools`，跑完已删）：61 个 target、1565 条通过、34 条 ignored、1 条失败。失败的是 `opendan` 的 `a_turn_is_a_task_and_a_slow_reply_replaces_its_placeholder`（期望回复已经替换占位，读到的还是“(working)”），是负载下的时序问题（TODO 之前的记忆里也记过它在负载下失败），单独连跑 3 次都通过 |
| 2026-11-15 | 裁决 | owner 在 §9 里答复了 Q28–Q41（记成 D35–D40）。要改代码的三件：一个 Agent 一个浏览器会话、去掉“事情”（D35、D36），画面清晰度按网速调整（D37），ext-core 的字体（Q29，D40） |
| 2026-11-15 | D35、D36 | opendan：每个 Agent 只有一个标签组 `default`；`bind_default` 只决定哪个 MessageHub 会话是浏览器会话；`_browser` 记录只投给它，前缀 `[browser]`；`waiting_answer` 只看它；建 UI Session 时如果对应的是浏览器会话就带上 `browser` 手册（libopendan `template.rs` 的 `own_cmd_skills`，提示词重新冻结时补回）；改绑时把那个会话空闲的 UI Session 归档。旧 `groups.json` 里的具名组丢弃。Desktop：去掉事情列表、“新建一件事”、深链的 `group`；第一次要用会话时找或建一个带 `ui.source = agent-browser` 标记、标题“浏览器”的会话并绑定；MessageHub 的入口在浏览器会话里总是“打开浏览器”，别的会话只在 Agent 操作浏览器时出现。协议 §1–§3、两本手册、Agent 侧 DV 脚本同步。验证：opendan 全过；Desktop e2e 76 条全过 |
| 2026-11-15 | D37 | opendan：启动参数 `--screen-info` 加一块 2 倍虚拟屏；视口带 `pixel_ratio`，大于 1 时把标签页窗口挪过去、1 和 2 之间用截屏流的最大尺寸压；画质跟随生效视口的连接；截屏流在第一次套完视口后才启动。屏幕握手只认 `default`。Desktop：四档自动换（降档快、升档慢），设备像素比低于 1.5 时清晰档也按 1，菜单里可选“自动 / 清晰 / 省流”。实测（手机 412×780，画质 60）：1 倍静止一帧 20.9 KB、动画 310 KiB/s、浏览器 0.29 核；1.5 倍 36.9 KB、540 KiB/s、0.73 核；2 倍 56.3 KB、840 KiB/s、1.07 核；没人看时 0.03 核，`--screen-info` 本身不增加 CPU。`agent-browser screenshot` 改成按 CSS 像素截（有人用高清屏看时页面的像素比是 2），新用例在 2 倍屏上复现并验证 |
| 2026-11-15 | Q29（D40） | ext-core 1.3.1：所有加载 fontconfig 的工具（含 `ffprobe`）的环境都设 `FONTCONFIG_PATH`，Windows 启动器同样；`imagemagick` 冒烟查中文字体；表 B 按实际改写（读系统和用户字体目录，这一条保留）。修复前后在裸容器里对比：`fc-match 'sans-serif:lang=zh-cn'` 从 Inconsolata 变成 Noto Sans CJK SC；`soffice` 转写了微软雅黑 / 宋体的 docx，嵌入的字体从 HK 变成 SC；ffmpeg 字幕、`magick pango:` 从方框变成正常。`magick label:` 不写 `-font` 时仍是空白，这是 ImageMagick 不经 fontconfig 的设计，表 B 写了正确写法 |
| 2026-11-15 | 部署和复验 | 重新播种 ext-core 1.3.1、全量构建、重建镜像、部署。部署时发现本机 Zone 在 12:14 被重新激活过（不是本次操作），原来的三个测试 Agent 的登记都没了（AgentRoot 还在 `data/home/devtest/agents/` 下）；新激活的 Zone 自动带上了 `reflow` 桶。新建测试 Agent `browsy` 后：`WF-DV-017`、`WF-DV-012` 通过；`tests/dv/agent-browser.spec.ts` 8 过；`tests/dv/agent-browser.reflow.spec.ts` 9 过 1 跳过（中途一条失败是用例拿 Agent 的名字去比显示名，已改成和 App 同样取显示名）；浏览器会话的 UI Session 冻结的手册是 `ui_core`、`browser`，`extensions.opendan.cmd_skills` 是 `browser`；经 443 的屏幕在 `pixel_ratio` 2 时帧是 824×1560、1 时 412×780；容器里 ext-core 是 1.3.1，`magick` 认得 Noto Sans CJK SC。没在真实 Zone 上验：按网速自动换档（只在 mock 的限速上验过）、两台设备同看时的规则、真机 |
| 2026-11-15 | 最后的回归（Rust） | `cargo test -p opendan --no-fail-fast -- --test-threads=1` 全过。`cargo test -p libopendan -p buckyos-api -p workflow --no-fail-fast -- --test-threads=1`：53 个 target、1061 条通过、2 条失败，都是时序用例（`retry` 的 `a_retryable_error_waits_for_its_backoff_and_the_same_run_goes_on`、`work_simple_async` 的 `parallel_tasks_tick_and_show_only_new_output`，当时机器同时在跑 Playwright），单独各跑两次都通过 |
| 2026-11-15 | ext-core 1.3.1 发布 CI | 发布运行 43793463467 的五个构建作业都失败，原因都在 11-12 加入、第一次在 CI 上跑的对应源码合规流程：① macOS 的 `_openmp_mutex` 以自己的配方目录为源码（构建机的绝对路径），被当成不安全路径拒绝：改为把包里带的配方当作它的源码；② runner 连不上 git.kernel.org（keyutils）：加 Debian 的同字节镜像；③ googlesource 的 `+archive` 每次请求字节都不同，按字节钉的 17 个归档永远对不上：改成按内容校验（`tree_sha256`），其他来源仍按字节；④ frippery.org 对 runner 返回 403（busybox）：`build.sh` 的下载失败时查 `source-mirrors.json`，经 owner 同意在 buckyos 仓库建了预发布 `ext-tools-upstream-mirror` 放两份同字节的文件；⑤ pandoc 的源码缺口按 owner 的决定改记为 `accepted_source_gaps`（写明缺口、接受人、日期），不再阻止发布、随对应源码清单发布。本机照 CI 的参数跑 `build.sh --platform linux-x86_64 --archive` 完整通过（运行包 722 MB、对应源码 3.7 GB），对应源码清单里 pandoc 的覆盖度是 `pinned-upstream-materials-with-accepted-gaps`。没在 runner 上重跑 |
| 2026-11-15 | ext-core 1.3.1 发布 CI（第二次） | 运行 43801194172：linux-aarch64、darwin-x86_64 通过；还有三处外部来源的问题：① LibreOffice 的下载地址经镜像重定向，分到的镜像证书链不完整：五个安装包和源码 tarball 加 TDF 自己归档的同字节镜像；② zlib.net 返回给 runner 的内容每次都不同：加作者在 GitHub release 的同字节副本；③ osx-arm64 的 libprotobuf 6.33.5 源码里的 Cargo.lock 有两个 git 依赖（googletest-rust）：新增 `cargo-git-sources.json`，把 git 依赖钉到 GitHub 对那个提交的归档（字节稳定）和它的 SHA256，没钉的 git 依赖仍然阻止发布。用真实的 protobuf 6.33.5 源码验证：15 个 crate 全部可收集，没有未解决项 |
