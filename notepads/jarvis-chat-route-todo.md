# Jarvis 默认聊天与会话路由 TODO

D1–D11、Q1–Q6 已裁决，没有待你回答的问题 · 2026-11-23（原稿 2026-11-18）· 源码基线 `c1940beeb`

目标是让用户有一个随时能回来的主聊天。能在对话内回答的事当场完成，需要执行的事交给合适的工作会话。用户补充要求、点了通知卡或隔了很久再问时，Agent 能找到原工作、用户的决定和依据。

各条目已按五轮裁决改写，原话和落实方式在“裁决记录”一节。现在没有待你回答的问题，有 4 处是我按理解先写的，在下一节。

## 待确认

### 我按理解先写了，不对再改

- **Q6**　有两处是我定的。
  - 默认会话记的是对话，不是某一代 UI session。所以通道换绑到新的 UI 会话之后，默认仍在这个通道上，通知照常到通道。
  - “用户要知道影响”我做成改默认之前的确认：先说明哪个通道将收不到系统通知，用户确认后才改。另一种做法是照常改，改完再提示。
- **Q5**　通知怎样跟随默认会话，有三处是我定的。
  - 通知只认用户明确设过的默认会话。没设过时投递规则不变。
  - 设了默认之后，`timely` 原来加投“最近活跃的会话”，改成加投默认会话。`normal` 的通知没有来源会话时投到默认会话，比如 goal 发的。`quiet` 和 `urgent` 不变。
  - 非 Agent 来源的通知这一期仍不走 Telegram，比如 App 通知。它们现在只有分类会话里的主记录。默认会话是按 Agent 记的，这类通知不属于哪个 Agent。
- **Q2**　有三处是从你的答复推出来的。
  - Agent 不能写用户的信箱，所以新建聊天的 `/new` 由 Desktop 处理，不经过 Agent。
  - 旧聊天和它的工作都留着，所以这种 `/new` 不再因为有未结束的工作被拒绝。
  - 在通道和 App 创建的会话里，`/new` 是换绑到新的 UI 会话，旧的存档，也就是现在的行为。`/clean` 不变，在哪里都是这种换法。
- **Q3**　“结构化恢复”我读作“结构化回复”。兼容模式下，最后一段文字直接作为正文发给用户，只有要更新主题或发附件时才用一次工具调用提交。另一种读法是兼容模式下每次回复都必须走工具调用。

## 裁决记录

D5、D6、D8、D10、D11 的落实是我按理解写的，第二轮你没有改动，按此执行。

**D1 主聊天用哪个会话。**（§2 第一项）

> 从tg的流程来说，应该允许用户把1个chat session设置为默认会话。有的用户喜欢新建session,那么新建chat session的时候也允许新建后设置默认会话（相当于用户主动做了context window管理）

第二轮（Q1，Telegram 的消息进不进默认会话）：

> 不同的tunnel默认会话不同，我的意思是tg-tunnel这条路已经试过只有一个session的感觉了。习惯用tg tunnel的用户可以吧tg tunnel的会话设置成默认会话

第三轮（Q5，把 Telegram 会话设为默认之后 Desktop 上看到什么）：

> Q5的含义是，是让系统通知能走Tgtunnel到到用户。 当默认会话到Tg后，意味着用户在webui里这个默认会话是只读的（这是预期内的产品体验）

落实：用户可以把一个聊天设为默认会话，新建时也可以直接设为默认。没设过时取最早创建的那个。原建议（固定用 `dm:<peer>`）不采用，“已经在用的人主入口变成空会话”的后果也就不存在了。每个通道有它绑定的会话，Telegram 入站不改。默认会话同时是 Agent 通知的投递目标。把 Telegram 会话设为默认后，通知经 Telegram 到用户，Desktop 里这个会话只读。我在 Q5 里的建议（在用户信箱登记同 ID 的会话、从 Desktop 写入）不采用。

**D2 `/new` 之后还要不要能找到之前的工作。**（§2 第一、二项）

> 同上逻辑

第二轮（Q2，“同上逻辑”的含义）：

> `/new` 直接变成“新建一个聊天并设为默认”

第四轮（Q6，默认会话是 Telegram 会话时在 Desktop 里发 `/new` 要不要改默认）：

> Q6 涉及到的几个概念： 1）每个tunnel必须绑定一个ui session,这个ui session可能是default ui session 2) webui能看到所有的ui session,但有的ui session是readonly的 2）tunnel 可以更换其绑定的ui session, 通常旧的session会处于“存档”状态 。 系统新建session并设置默认这个行为，对已经存在tunnel并且tunnel之前的uisession是默认uisession的情况，造成的后果是tunnel无法收到系统通知

落实：在 WebUI 里用户自己建的聊天中，`/new` 新建一个聊天并设为默认，旧聊天连同它的工作留在列表里。新聊天查不到旧聊天的工作。在通道和 App 创建的会话里，`/new` 是换绑到新的 UI 会话，旧的存档，默认仍跟着通道。三个概念写进了 §2 第一项。

第五轮（Q6，这个后果是接受还是要避免）：

> Q6 按接受写。 用户要知道这个行为对通道的影响

落实：在 WebUI 里新建并设为默认，会让原来是默认的通道收不到系统通知，这个后果接受。改默认之前 Desktop 向用户说明对通道的影响。

**D3 澄清问答由谁写进分析报告。**（§2 第二项）

> 按建议

落实：宿主直接填，模型不再抄一遍。

**D4 回复格式不合格时怎么办。**（§2 第三项）

> 我理解这是一个llm可以主动控制的ui session模式开关。默认是开json schema,当处理失败的时候，在失败的返回值里给一个hint,让llm有机会设置这个模式为“兼容模式”

第二轮（Q3，兼容模式下回复怎么交）：

> 用一次工具调用提交（这个是支持schema之前的老方法），agent只有在需要的时候才用结构化恢复

落实：UI session 增加回复模式，默认按 schema，模型可以自己切到兼容模式，格式纠正消息里给出提示。原建议里的宿主补救和按 AICC 特性位判断都不做。兼容模式下，最后一段文字作为正文，需要主题或附件时用一次工具调用提交。

**D5 回复回退排不排进第一批。**（§2 第三项）

> 同上

落实：不再按模型区分，由模型自己切换。这一项仍排在第一批的第三位。

**D6 `work_simple` 运行中收到补充怎么办。**（§3 第一项）

> 反复讨论后，机制上还是允许work_simple有inbox吧 （这个不需要开kevent/kmsg 的开销不大）

落实：`work_simple` 可以收消息，补充直接投给它，在下一次推理前读入。还有未读的补充时，工作不允许结束。原建议（聊天停掉它再重新分析）不采用。

**D7 群聊这一期开放到哪里。**（§3 第三项）

> 一组条件吧，按goal session的逻辑，估计最终会脚本化（脚本可以根据现在的预算更仔细的分级决定唤醒时机）

第二轮（Q4，这一期有哪些条件，非 owner 触发时能做什么）：

> 按你建议，这里主要是做可扩展的结构。

落实：触发改为一个条件列表，结构可扩展。判断接口按 goal 检查脚本的合同留形状，以后换成脚本。这一期的条件分“谁”和“怎样”两维，默认值等于现在的行为。非 owner 触发的那一轮只在对话内回答，这一期只靠提示词约束，记为已知风险。

**D8 “无记忆”指什么。**（§4 第一项）

> 无记忆有两种 1：绝对不写 2：不读也不写

落实：创建聊天时三选一：正常、不写、不读不写。两种无记忆模式下，聊天的主题都不显示给其他会话。其他会话仍能用 `read-session` 读它的记录再自己写感知，这条路这一期不堵。

**D9 无记忆聊天创建的工作是否继承这个选择。**（§4 第一项）

> 继承

**D10 “什么值得写成感知”这一项留不留。**

> memroy的结果优化另立TODO，这个就是不断的根据长期使用结果改进。我现在倾向于让LLM自己理解机制：你记下的东西要对未来的你有用。。

落实：本项移出，方向记在 §4 末尾。新 TODO 还没建。

**D11 跨轮保留多少原始历史。**（§4 第二项）

> 这里有个细节，如果Agent的提示词升级了，那么在压缩和“缓存失效”后回来的时候，应该有机会做一次llm context重建（含压缩）

落实：按建议的线（约 80% 触发，压到约 30%），加上提示词升级后的重建时机。提示词没变时，隔很久回来不重建。

## 1. 已接通的路径

| 场景 | 当前行为 |
| --- | --- |
| 直接问答 | 短查询、读文本可在当前聊天用 `read` 完成。图片、文档、音视频可通过 `llm_understand_media` 处理，较长的调用以任务结果返回。[聊天规则](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml#L16) |
| 判断是否开工作 | `try_analyze_user_intent` 以 fork 继承聊天上下文，先查环境，再用 `ask_user` 澄清会改变选择的信息。单项可自验的交付走 `work_simple`，需拆 Todo 或独立验收的走 `plan_simple`，长期达成、维持或观察某种状态走 `goal`。[意图分析](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L30) · [工作形态](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L72) |
| 放置工作 | 分析器在私有 runtime 和 owner 默认 runtime 之间选，按运行环境选 `cmd_skills`。workspace 可选已有、创建、导入，或对一次性任务用 `none`。给 plan 的 skills 是其 Todo 的候选。[workspace 与 runtime](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L78) · [创建命令](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L88) |
| 聊天入口 | 日常私聊、用户自建专题、群聊和 App 专用聊天都已有。通用 App embed 按 App 与 Agent 复用最近会话，也可指定本 App 创建的会话。Agent Browser 有绑定的浏览器会话，AIWorkspace 面板会跨 workspace 复用同一会话。[App embed](../src/frame/desktop/src/components/conversation/embed/ConversationEmbedApp.tsx#L90) · [浏览器会话](../src/frame/desktop/src/app/agent-browser/conversation/session.ts#L1) · [AIWorkspace 面板](../src/frame/desktop/src/app/aiworkspace/ui/jarvis/session.ts#L27) |
| 记忆与输入 | `reply.topic` 更新主题，宿主将一批输入的消息正文与主题用于 recall。模型可主动 `recall --read` 和 `ctl perceive`。常规未压缩聊天复用原生消息前缀，同批默认可收 16 个有效输入，群聊背景随请求附入。[批量召回](../src/frame/lib_opendan/src/runner/memory_batch.rs#L144) · [输入批次](../src/frame/lib_opendan/src/runner/inputs.rs#L146) · [前缀回归用例](../src/frame/lib_opendan/tests/ui_chat.rs#L99) |

## 2. 优先处理

- [ ] **主聊天有稳定入口，通知跟随默认会话。**（D1、D2）
  - **概念**（owner 2026-11-23）
    - 每个通道绑定一个 UI 会话，它可以就是默认会话。现在的实现是，通道发来的消息落在一个固定的信箱地址上，Loader 把这个地址绑到一个 UI session。[Loader 的绑定](../src/frame/opendan/src/ui.rs#L1)
    - WebUI 能看到全部 UI 会话，其中通道绑定的是只读。现在用户自己的会话列表里没有通道的会话，要补。
    - 通道可以更换它绑定的 UI 会话，旧的通常处于存档状态。现在的实现是在通道里发 `/new`：Loader 把当前的 UI session 存档，下一条消息在同一个地址上开新的。
  - **现状**
    - 无显式会话的私信落到 `dm:<peer>`，Loader 续接该地址上未结束的 UI session。Desktop 点击 Agent 打开的是最近活跃的 native 会话，App 创建的会话也参与比较。Agent 还没有会话时，Desktop 新建一个随机 ID 的会话。没有“默认会话”这个概念：会话偏好只有置顶和免打扰，创建请求里也没有对应字段。[私信地址](../src/frame/msg_center/src/msg_center.rs#L928) · [Loader 续接](../src/frame/opendan/src/ui.rs#L729) · [桌面默认选择](../src/frame/desktop/src/app/messagehub/sessionModel.ts#L33) · [桌面首次创建](../src/frame/desktop/src/app/messagehub/store/defaultSession.ts#L19) · [会话偏好](../src/frame/desktop/src/app/messagehub/sessionModel.ts#L15) · [创建请求](../src/kernel/buckyos-api/src/msg_center_client.rs#L1871)
    - Telegram 入站的消息自带会话 ID `tg:<bot>:<chat>`，记在 Agent 的信箱里，对方是那个 Telegram 账号。用户自己和 Agent 的会话列表里没有它，Desktop 只能用旁观模式看，不能写。[入站时填的会话](../src/frame/msg_center/src/tg_tunnel.rs#L860) · [ID 的构成](../src/kernel/buckyos-api/src/msg_center_client.rs#L42) · [私聊的收件人](../src/frame/msg_center/src/tg_tunnel.rs#L800) · [旁观模式只读](../src/frame/desktop/src/app/messagehub/api/store.ts#L887)
    - 来自 Agent 的通知按等级另投到用户和这个 Agent 已有的对话里，原生和 Telegram 都算。`quiet` 不另投，`normal` 投到来源会话，`timely` 再加最近活跃的会话，`urgent` 投到全部。候选对话取自 Agent 的信箱，是用户说过话的那些会话。非 Agent 来源的通知只有分类会话里的主记录。[副本计划](../src/frame/msg_center/src/notify.rs#L1066) · [候选对话](../src/frame/msg_center/src/notify.rs#L1007) · [投递规则文档](../doc/message_hub/System%20Notify.md)
    - `/new` 由 Agent 的 Loader 处理，原地换 Agent 侧的 UI session，MessageHub 里的会话不变。有未结束的工作时拒绝。`/clean` 是同一个命令。Desktop 不识别斜杠命令，原样发出。Agent 只能读用户的信箱，不能替用户建会话。[命令处理](../src/frame/opendan/src/ui.rs#L677) · [原地换代](../src/frame/opendan/src/ui.rs#L569) · [换代的 session ID](../src/frame/opendan/src/ui.rs#L778) · [只能写自己的信箱](../src/frame/msg_center/src/owner_session.rs#L928)
  - **做法**
    - msg_center 为每对 owner 和对方记一个默认会话。`msg.create_session` 可以带“设为默认”，另加一个把已有会话设为默认的调用。同一个对方只有一个默认，设了新的就取代旧的。[创建时的会话 ID](../src/frame/msg_center/src/owner_session.rs#L1266)
    - 默认会话的候选是用户和这个 Agent 已有的对话，原生和 Telegram 都算。这份列表通知投递已经在用，设默认时按它校验。记录的写法和通知的来源会话一致，是用户端点加会话 ID。它记的是对话，不是某一代 UI session，所以通道换绑到新的 UI 会话之后，默认仍在这个通道上。[对话的写法](../src/frame/msg_center/src/notify.rs#L187)
    - 没设过时，Desktop 的主入口取最早创建的那个 native 会话。已经在用的人不用迁移，新用户第一次建出的会话就是默认。这条只用于主入口，通知不看它。
    - Desktop 的 `selectDefaultSession` 对 Agent 返回默认会话。会话详情里加“设为默认”，新建会话时可以勾选“设为默认会话”，列表里标出哪个是默认。
    - Desktop 在 Agent 的会话列表里列出它的 Telegram 对话，读的是 Agent 的信箱，只读，可以设为默认。默认会话是 Telegram 会话时，主入口打开的就是这个只读会话，这是预期的体验。用户要在网页上聊，就打开或新建别的聊天。
    - 通知跟随默认会话。用户设了默认会话后，`timely` 加投的“最近活跃的会话”改成默认会话，`normal` 的通知没有来源会话时投到默认会话，`quiet` 和 `urgent` 不变。默认会话是 Telegram 会话时，这些通知经 Telegram 到用户。没设过默认时规则不变，非 Agent 来源的通知不变。投递规则文档同步改。
    - 每个通道有它绑定的会话。Telegram 入站不改，仍落在它绑定的会话。其他无显式会话的私信仍落到 `dm:<peer>`。
    - `/new` 改成新建聊天。Desktop 的会话组件在发送前识别 `/new`。在用户自己建的、和 Agent 的聊天里，它等同于“新建会话并设为默认”，然后切到新聊天，这条文字不发给 Agent。旧聊天留在列表里，它的工作继续跑，结果仍在旧聊天里转述，所以不需要拒绝。用户自己建的聊天按会话来源 `manual` 认。[会话来源](../src/frame/desktop/src/app/messagehub/types.ts#L77)
    - 在通道里发 `/new` 不变。它是通道换绑到新的 UI 会话，旧的存档，仍发给 Agent 由 Loader 处理，有未结束的工作时仍拒绝。App 创建的会话同样处理。`/clean` 在哪里都是这种换法。默认会话是这个通道的会话时，换绑之后通知仍到这个通道。
    - 默认离开通道时要让用户知道影响（Q6）。默认会话原来是某个通道的会话时，在 WebUI 里新建会话并设为默认，默认就离开了通道，通道不再收到系统通知。这个后果接受。勾选“设为默认”、在会话详情里设为默认和发 `/new` 都算。Desktop 在改之前说明哪个通道将收不到系统通知，用户确认后才改，取消则只新建聊天、不动默认。会话列表里标出默认会话，用户随时能把通道的会话重新设为默认。
    - 手册里对 `/new` 的说法要改，`ui_core` 和 `opendan_agent_dev` 各有一处。[ui_core 的会话手册](../src/apps/jarvis_agent/agent/cmd_skills/ui_core/references/sessions.md#L22) · [opendan_agent_dev 的会话手册](../src/apps/jarvis_agent/agent/cmd_skills/opendan_agent_dev/references/sessions.md#L18)
  - **已有的行为**　归档的会话收到新消息会自动恢复。不用新做，只需验收。[归档后恢复](../src/frame/msg_center/src/owner_session.rs#L1456)
  - **验收**
    - 把 A 设为默认后，使专题 B 或 App 会话 C 成为最近活跃项。重新从 Agent 主入口进入仍到 A，显式打开 B、C 各自续接。
    - 新装的 Zone 第一次打开 Agent，建出的会话就是默认。没设过默认的老用户，主入口是他最早的那个会话。
    - 新建会话 D 并勾选设为默认，主入口随即到 D，A 留在列表里可以打开。
    - 在默认会话 A 里发 `/new`，出现新聊天 E 并成为默认，Desktop 切到 E，Agent 从新上下文开始。A 留在列表里。A 里还在跑的工作结束后，结果出现在 A。
    - 在 Telegram 里和在 App 创建的会话里发 `/new`，仍在同一个会话，Agent 从新上下文开始。有未结束的工作时被拒绝。默认会话是这个 Telegram 会话时，`/new` 之后通知仍到 Telegram。
    - 默认会话是 Telegram 会话时，在 WebUI 里新建会话并设为默认，或在别的聊天里发 `/new`，Desktop 先说明 Telegram 将收不到系统通知。确认后默认改到新会话，之后的通知出现在新会话里，Telegram 收不到。取消后新聊天照常建出，默认仍是 Telegram 会话。把 Telegram 会话重新设为默认后，通知回到 Telegram。
    - 从 Telegram 发来的消息仍进它自己的会话，改默认会话不影响它。
    - 把 Telegram 会话设为默认后，从 Desktop 主入口进入的是这个会话，能看到完整记录，不能输入。
    - 之后 goal 发出的 `normal` 通知和任意一条 `timely` 通知，用户在 Telegram 里收到。把默认改回 Desktop 的聊天后，同样的通知出现在那个聊天里。
    - 没设过默认时，通知的投递和现在一致。
    - 归档默认会话后从主入口进入，回到它并恢复，不另建会话。

- [ ] **让下一次意图分析能用上本次报告。**（D2、D3）
  - **现状**　fork 的结果已进入父聊天历史。当前 schema 接受 `outcome="work_created", report="", work=null`，宿主对澄清问答只核对条数。请求被转给现有 goal 或运行中的 plan 时，结果里带不出它们的 ID。分析器被指引用 `sessions --children --active` 找刚结束的工作，而 `--active` 会过滤所有 `Finished`。[分析输入与历史查找](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L43) · [转给 plan 时的结果](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L112) · [返回契约](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L115) · [宿主校验](../src/frame/lib_opendan/src/runner/reports.rs#L602) · [CLI 过滤](../src/frame/lib_opendan/src/xagent_cli.rs#L1313)
  - **做法**
    - 跨字段条件放进宿主的 `ReplyValidator`，原生 schema 表达不了这类条件。`report` 不能为空。`work_created` 必须带 `work`，其 `session_id` 是本分支创建的子会话。`goal_created` 必须带 `goal`，其 `goal_id` 存在。[ReplyValidator](../src/frame/lib_opendan/src/runner/reports.rs#L550)
    - 按 D3，`clarifications` 由宿主填。
    - 增加一种结果，表示请求已转交给现有的 goal 或 plan，带其 ID。
    - `sessions` 增加一个有上限的“最近结束”查询，分析器改用它。只去掉 `--active` 不够，主聊天长期存在，`--children` 会列出它创建过的全部工作。按 D2，只查这个聊天当前这一代。`/new` 新建的聊天查不到旧聊天的工作，原地换代的也不跨代。
    - 修正交接说明。提示里写的是“工作会话看不到这段对话”，实际 `--context recent:6` 会附上父会话最近对话的摘录。[提示原文](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L91) · [交接摘录](../src/frame/lib_opendan/src/api.rs#L1145)
    - CLI 变了，同步 `ui_core`、它的 `references/sessions.md` 和 `opendan_agent_dev`。
  - **验收**　空报告、缺工作 ID、ID 不是本分支创建的，均触发纠正。报告里的问答与用户在通知卡上的原话一致。已有 goal 或运行中的 plan 收到消息后，结果带回其原 ID。上一份工作失败后再分析，能找回报告里的缺失条件，无需再问已回答的问题。主聊天创建过很多工作时，查询结果仍有上限。`/new` 之后，新聊天里的查询不带出旧聊天的工作。原地换代之后的查询不带出上一代的工作。

- [ ] **回复格式不合格时，模型可以切到兼容模式。**（D4、D5）
  - **现状**　正常路径输出 `{reply, topic, artifacts}`。宿主允许一次格式纠正，再错就以 `output_format` 结束本轮，用户看到失败提示，原始输出不发给用户。纠正消息由通用的 llm_context 层生成，只重述格式要求。[一次纠正](../src/frame/lib_opendan/src/runner/live.rs#L354) · [纠正消息](../src/frame/llm_context/src/context_loop.rs#L2528) · [失败处理](../src/frame/lib_opendan/src/runner/outcome.rs#L251) · [失败用例](../src/frame/lib_opendan/tests/ui_chat.rs#L281)
  - **不采用的方案**　原稿的 `report()` 回退。聊天有任务未结束时 final report 会被拒，附件无效时照样失败，UI 的工具白名单里也没有 `report`。[final 校验](../src/frame/lib_opendan/src/runner/reports.rs#L267) · [工具白名单](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml#L64) · [上一份 TODO](done/jarvis-chat-prompt-todo.md) §13 第 8 条
  - **做法**
    - UI session 增加回复模式，记在会话状态里。默认按 schema，另一个值是兼容模式。跨 Turn 保持。换了上下文就回到默认，新聊天和原地换代都算。
    - 模型用一条 `agent-session ctl` 命令切换，可以切回。[ctl 命令表](../src/frame/lib_opendan/src/xagent_cli.rs#L895)
    - 格式纠正消息里加一句提示，说明可以切到兼容模式和对应的命令。这句提示由宿主附加，不写进通用层。
    - 兼容模式下，最后一段文字原样作为正文发给用户。这样的一轮主题不变，没有附件。
    - 需要更新主题或发附件时，用一次工具调用提交 `{reply, topic, artifacts}`。校验用同一套，提交后本轮结束，不检查任务。
    - 兼容模式的说明里写清什么时候用这个工具：要发附件时，或话题变了时。主题长期不更新会影响召回。
    - 切换当轮生效。模型在纠正那一轮里切了模式，接下来给出的文字就按正文交付，不用等下一条消息。要在同一轮发附件，这个工具得一开始就在 UI 的工具表里，没切模式时调用它会被拒并提示先切换。
    - 没切模式、纠正后仍不合格时维持现状，本轮失败并显示失败提示。
    - `ui_core` 里“纠正后仍失败就查 run 和 worklog”那句要改，同步 `references/sessions.md` 和 `opendan_agent_dev`。[手册原文](../src/apps/jarvis_agent/agent/cmd_skills/ui_core/SKILL.md#L26)
  - **验收**
    - 强制第一次回复带多余字段。模型在纠正那一轮切到兼容模式，用户在同一轮收到正文，第一次那份不合格的输出不发给用户。
    - 之后几轮保持兼容模式，纯文字的回复直接送达。带附件或换了话题的那一轮通过工具调用交付，附件送达，主题更新。聊天有任务在跑时也能交付。
    - 切回 schema 模式后恢复原路径。
    - 没切模式且两次都不合格时，本轮失败并显示失败提示。
    - 以上各种情况下，下一条消息可在同一聊天继续。

## 3. 后续消息与群聊

- [ ] **统一已有工作的续接规则。**（D6）
  - **现状**　私聊会把与未结束 plan 有关的补充发回原 plan，暂停时排队。群聊的路由提示没有这一节。`work` 类会话的队列只收控制，消息在投递前和消费时各拦一次，而这个队列在 Zone 里本来就存在。创建子会话时已有一个“接收父会话的消息、但不等待输入”的选项，plan 派发 Todo 用的就是它，CLI 创建时固定为关。运行中到达的消息留到下一个 Turn 才读入。`work_simple` 的输入模板已经能渲染消息，但提示里写的是“没有人读这个会话”。[私聊路由](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml#L20) · [群聊路由](../src/apps/jarvis_agent/agent/behaviors/groupchat_route.toml#L18) · [work 类的输入通道](../src/frame/lib_opendan/src/template.rs#L226) · [投递前的拦截](../src/frame/lib_opendan/src/state/registry.rs#L725) · [只收控制的队列](../src/frame/lib_opendan/src/api.rs#L379) · [子会话选项](../src/frame/lib_opendan/src/api.rs#L698) · [plan 派发](../src/frame/lib_opendan/src/todos.rs#L159) · [CLI 创建](../src/frame/lib_opendan/src/xagent_cli.rs#L1450) · [轮间检查点](../src/frame/lib_opendan/src/runner/hook.rs#L85) · [work_simple 提示](../src/apps/jarvis_agent/agent/behaviors/work_simple.toml#L15)
  - **做法**
    - 群聊的路由提示补上与私聊相同的“进行中的工作”一节。
    - 聊天创建的 `work_simple` 打开上面那个选项，`post <work sid> --msg` 就能投进去。队列是现成的，不用新开 kmsg 或 kevent。
    - 读入时机。补充要在下一次推理前进入上下文。现在的轮间检查点只处理控制，要么在这里读入消息，要么收到消息时结束当前 Turn、带着消息开下一个。取改动小的一种。
    - 还有未读的补充时不允许结束，和“任务未结束不能结束”在同一个位置拦。[final 校验](../src/frame/lib_opendan/src/runner/reports.rs#L267)
    - 工作结束后才到的补充，`post` 返回已结束，聊天告诉用户并重新分析。
    - 提示与手册。`work_simple` 写清中途收到补充时以最新的要求为准，并在报告里说明采纳了哪些补充。私聊和群聊的路由提示、`ui_core`、`references/sessions.md` 里“普通工作不收消息、只能停掉重开”的说法改掉，同步 `opendan_agent_dev`。plan 的 worker 不变，补充仍发给 plan。[简单工作约束](../src/apps/jarvis_agent/agent/cmd_skills/ui_core/SKILL.md#L20) · [会话命令手册](../src/apps/jarvis_agent/agent/cmd_skills/ui_core/references/sessions.md#L5)
  - **验收**
    - 同一用户并行跑两个 plan 时，带明确指代的补充进入对应 plan。歧义会澄清。催促暂停的 plan 不会创建重复工作。群成员的背景发言不会成为任务。
    - 对运行中的 `work_simple` 提出补充后，同一个工作会话在下一次推理前读到它，最终报告里说明了这条补充，没有创建第二个工作。
    - 补充和结束报告同时发生时，工作不会带着未读的补充结束。
    - 工作结束后才到的补充，聊天会告诉用户并重新分析。

- [ ] **把用户对通知的处理结果带回相关聊天。**
  - **现状**　通知有两类。告知类（`Notice`）没有可回答的内容。提问类（`Input`）被用户回答后，监听器唤醒的是提问的会话，通常是 work 或 goal。显示卡片的聊天只有一行“已投递”背景，之后可以用 `notify show` 主动查，不会得知用户选了什么。`ask_user` 的答案回到提问分支，再由分析报告带给父聊天。[通知背景](../src/frame/lib_opendan/src/notify.rs#L309) · [只跟踪提问类](../src/frame/opendan/src/notify.rs#L33) · [唤醒目标](../src/frame/opendan/src/notify.rs#L82)
  - **做法**　Agent 侧的监听器在提问类通知关闭时，向显示卡片的那个聊天（`source.conversation` 对应的 UI session）投一条背景事件，带通知 ID、处理状态和答案，不主动开一轮。不改 msg_center 协议。事件的 key 要和“已投递”的 `notify:<id>:<session>` 区分开，同 key 的输入会被当作重复去掉。[按 key 去重](../src/frame/lib_opendan/src/runner/inputs.rs#L299)
  - **验收**　用户在通知卡上选择 B 后回到聊天问“按刚才选的继续”，聊天能关联到 B。重复投递只出现一次，无关聊天看不到该选择。

- [ ] **群聊的触发条件和回看条数可配置。**（D7）
  - **现状**　群聊默认关闭。开启后，只有 owner 结构化 @ Agent 才触发，这条规则写死在宿主里，在任何推理之前判断。每次从同一 mailbox 最近 20 条已读记录取背景，保留说话人并去重。群聊的路由提示通篇以“回答 owner 的请求”为前提，其他成员的发言只是材料。goal 的做法是，宿主在一次运行机会变成推理之前先跑检查脚本，脚本返回 `{run, reason}`，不花 token，能看到当前是被间隔还是被预算按住。[触发者判断](../src/frame/lib_opendan/src/bridge/msg.rs#L197) · [上限](../src/frame/opendan/src/ui.rs#L48) · [群聊路由提示](../src/apps/jarvis_agent/agent/behaviors/groupchat_route.toml#L9) · [goal 检查脚本](../src/frame/lib_opendan/src/goal_check.rs#L1) · [群聊回归用例](../src/frame/opendan/tests/loader.rs#L2122)
  - **做法**
    - 这一项的重点是结构可扩展，这一期的条件本身只做最少的。
    - 触发改为 owner 的 Agent 设置里的一个条件列表，和 `allow_group` 放在一起，运行中重读。宿主仍在推理前判断，任一条满足就开一轮。回看条数也放在这里。
    - 每条条件写“谁”和“怎样”。谁可以触发：owner、指定成员、所有成员。怎样触发：结构化 @ Agent、回复 Agent 的消息。默认只有一条，owner 的 @，等于现在的行为。
    - 判断做成一个接口，输入是这条消息、最近的背景和预算状态，输出是 `{run, reason}`，和 goal 检查脚本的合同一致。这一期的内置条件是它的一个固定实现。
    - 以后加新的条件类型或换成脚本，配置的形状和调用点都不变。按消息量、时间、预算分级的条件留到那时。配置里出现不认识的条件类型时，这一条按不满足处理并记日志。
    - 非 owner 触发时，输入里标明是谁触发的。路由提示写清这一轮只在对话内回答，不创建工作，记忆的可见范围沿用群聊现有的范围。这一期宿主不硬拦，记为已知风险。
  - **验收**
    - 回看配置为 3 后，触发请求只带最近范围内的有效背景，普通群聊不单独开轮。
    - 默认配置下行为和现在一致：owner 的 @ 触发，其他成员的 @ 和 owner 未点名的消息不触发。
    - 加一条“所有成员的 @”后，其他成员的 @ 触发一轮，这一轮不创建工作。
    - 加一条“owner 回复 Agent 的消息”后，owner 不点名、直接回复 Agent 上一条消息也触发一轮。
    - 配置里有一条不认识的条件类型时，其他条件照常生效。
    - 每次触发在 worklog 里记下是哪条条件放行的。

## 4. 记忆与长聊天

- [ ] **用户或 App 创建聊天时可选择记忆模式。**（D8、D9）
  - **现状**　创建请求没有对应字段，模板 `load_hints=false` 只关闭自动注入。记忆层的写入侧已有 `RecordPolicy::DoNotRecord`，目前没有任何地方设置它。聊天的主题会以“conversation … is about”的形式出现在其他会话的背景里。[创建请求](../src/kernel/buckyos-api/src/msg_center_client.rs#L1871) · [自动注入条件](../src/frame/lib_opendan/src/runner/memory_batch.rs#L73) · [记忆访问身份](../src/frame/lib_opendan/src/memory/mod.rs#L238) · [不记录策略](../src/frame/lib_opendan/src/memory/mod.rs#L219) · [主题进入其他会话](../src/frame/lib_opendan/src/runner/background.rs#L231)
  - **做法**
    - 创建请求增加记忆模式，三个值：正常、不写、不读不写。从创建请求传到宿主的会话配置。Desktop 新建会话时可选，App 创建会话时可以带这个字段。
    - 不写。接上 `DoNotRecord`，`ctl perceive` 不落任何东西。主题不显示给其他会话。自动召回和主动 recall 照常。实施时确认后台的记忆整理不会从这个会话的记录里提取内容。
    - 不读不写。在“不写”之上再关掉自动召回，主动 recall 和 read 返回空并说明原因。
    - 按 D9，它创建的 work 和 plan 继承同一个模式。
  - **验收**
    - 预先在普通聊天留下一个可召回事实。“不写”的聊天能召回到它。“不读不写”的聊天自动和主动读取都拿不到它，它创建的工作也拿不到。
    - 在这两种聊天里给出的新偏好都不会进入长期记忆，它们创建的工作也不写，其他聊天召回不到。
    - 其他会话的背景里不出现这两种聊天的主题。
    - 重开该会话后模式仍有效。

- [ ] **让跨轮聊天的历史预算与模型窗口一致，提示词升级后有机会重建上下文。**（D11）
  - **现状**
    - 三处各有一条线。运行中约 80% 窗口触发，重写到约 30%。run 结束后，历史超过 0.75 × 24,000 = 18,000 token 就压缩到 12,000，与窗口无关。下一轮重建时，上一轮的原生消息不超过 24,000 才原样复用，而且压缩过一次之后就不再走这条路径。所以大窗口下，原生前缀在 18,000 token 时就丢了。[运行中的触发线](../src/frame/agent_tool/src/xllm.rs#L146) · [默认值](../src/frame/lib_opendan/src/runner/mod.rs#L255) · [run 结束后的压缩](../src/frame/lib_opendan/src/runner/outcome.rs#L1528) · [压缩阈值](../src/frame/lib_opendan/src/runner/history.rs#L931) · [历史重建](../src/frame/lib_opendan/src/runner/history.rs#L60)
    - 聊天的历史被压缩过之后，下一次新开 run 会重新冻结提示词，Agent 包升级后的提示词从这里生效。没压缩过的聊天一直用创建时冻结的那份。会话状态里已经能比较出冻结的提示词和当前的有没有差别。[压缩后重新冻结](../src/frame/lib_opendan/src/runner/drive.rs#L625) · [调用位置](../src/frame/lib_opendan/src/runner/drive.rs#L2120) · [提示词是否最新](../src/frame/opendan/src/ui.rs#L1127)
    - 宿主不知道上游的缓存是否还有效。用量里有缓存读取的计数，但没有用它做判断。[缓存用量](../src/kernel/buckyos-api/src/aicc_client.rs#L2797)
  - **做法**
    - 后两处改用运行中的那套线，由模型窗口推导。会话显式配置了 `history_budget_tokens` 时按配置，不按窗口推导。这会改 run 结束后的压缩触发，[窗口压缩 TODO](opendan-context-window-compaction-todo.md) 当时定的是不动它。
    - 这样改之后压缩会少很多，提示词升级只靠“压缩后重新冻结”就更难生效。按 D11 的补充再加一个时机：隔了很久回来、缓存已经失效，而且提示词有变化时，在这一轮开始前重建一次上下文，重新冻结提示词，历史超过目标线就压到目标线。
    - 提示词没变时不重建，回来的第一条消息按原价重读一次，最多接近窗口的 80%。
    - “缓存已失效”按空闲时长判断，新增一个阈值。默认值在实施时按 AICC 实际请求的缓存档位定。
  - **验收**
    - 用相同长对话分别跑小、大窗口，记录触发原因、重写前后 token 与下一轮历史前缀。大窗口下，超过 18,000 token 但远低于触发线的对话，run 结束后不压缩，下一轮前缀与上一轮一致。逼近触发线时仍能压缩并继续。
    - 升级 Agent 包之后，在阈值内继续聊，提示词和前缀都不变。超过阈值回来的第一轮，worklog 里有重新冻结和压缩的记录，之后用的是新提示词。
    - 提示词没变时超过阈值回来，不重建。
    - 相关工作已有运行中压缩与摘要失败恢复的记录。[窗口压缩 TODO](opendan-context-window-compaction-todo.md) · [摘要失败修复记录](fixbug-jarvis-silent-after-compaction-failure.md)

- [ ] **压缩后保留本会话感知线索，给出稳定的原文回读入口。**
  - **现状**　摘要保留开头、摘要、精确片段和尾部，worklog 仍在。自动召回会过滤本会话的感知，理由是它们还在上下文里，压缩后这个前提不成立。固定压缩提示只让模型重读会话目录。CLI 的 `read-session <sid> --worklog N` 只能读最近 N 条。按 turn 和游标分页只在 kRPC `session.worklog` 上，Agent 用不到。[摘要与固定提示](../src/frame/lib_opendan/src/runner/history.rs#L39) · [本会话过滤](../src/frame/lib_opendan/src/memory/observation.rs#L721) · [CLI 读取](../src/frame/lib_opendan/src/xagent_cli.rs#L1362) · [历史分页](../src/frame/opendan/src/service.rs#L272)
  - **做法**　感知线索由宿主负责，不依赖摘要模型保留引用。两种做法取改动小的一种：在摘要块后固定附上本会话感知的引用，或者对压缩边界之前写下的感知取消“本会话”过滤。`read-session` 增加按 turn 和游标读取，固定压缩提示改为给出这条命令。CLI 变了，同步 `ui_core`、它的 `references/sessions.md` 和 `opendan_agent_dev`。
  - **验收**　压缩前记录用户的关键纠正，连续压缩两次后仍能定位那条感知和原话。之后追加新消息，旧引用仍指向同一条记录。

**移出本 TODO**　“什么值得写成感知”不在这里做（D10）。记忆效果的优化另立 TODO，按长期使用的结果持续改进。方向是让模型自己理解机制：记下的东西要对未来的自己有用。新 TODO 还没建。

## 5. 验证基线

2026-11-23 按 `c1940beeb` 重新核对了文中的源码引用，行号已更新。同日三轮按裁决改写时，新增的引用也在这个基线上核对过。每次都没有跑测试。上次 Review 在 `4c91b3d0c` 跑过 47 个 Rust 定向测试和 9 个 Desktop 会话测试，全部通过。它们覆盖包加载、群聊、默认会话选择、输入批次、fork、召回、通知、历史压缩与回复失败路径。真实模型选择工作形态的准确性、跨轮缓存命中率仍待 DV。可用同一条“直接问答 → 澄清 → 创建工作 → 追加要求 → 读回交付”的对话记录对照改动前后表现。
