# Jarvis 默认聊天与会话路由 TODO

待 Review · 2026-11-18 · 源码基线 `623b4d196`

目标是让用户有一个随时能回来的主聊天。能在对话内回答的事当场完成，需要执行的事交给合适的工作会话。用户补充要求、点了通知卡或隔了很久再问时，Agent 能找到原工作、用户的决定和依据。

建议先做 **主聊天入口、意图报告、回复回退**。这三项分别决定用户回到哪个会话、Agent 能否准确说明做了什么，以及回复格式出错时用户能否收到结果。下文的「待定」供 Review 时批注。

## 1. 已接通的路径

| 场景 | 当前行为 |
| --- | --- |
| 直接问答 | 短查询、读文本可在当前聊天用 `read` 完成。图片、文档、音视频可通过 `llm_understand_media` 处理，较长的调用以任务结果返回。[聊天规则](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml#L16) |
| 判断是否开工作 | `try_analyze_user_intent` 以 fork 继承聊天上下文，先查环境，再用 `ask_user` 澄清会改变选择的信息。单项可自验的交付走 `work_simple`，需拆 Todo 或独立验收的走 `plan_simple`，长期达成、维持或观察某种状态走 `goal`。[意图分析](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L30) |
| 放置工作 | 分析器在私有 runtime 和 owner 默认 runtime 之间选，按运行环境选 `cmd_skills`。workspace 可选已有、创建、导入，或对一次性任务用 `none`。给 plan 的 skills 是其 Todo 的候选。[创建规则](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L72) |
| 聊天入口 | 日常私聊、用户自建专题、群聊和 App 专用聊天都已有。通用 App embed 按 App 与 Agent 复用最近会话，也可指定本 App 创建的会话。Agent Browser 有绑定的浏览器会话，AIWorkspace 面板会跨 workspace 复用同一会话。[App embed](../src/frame/desktop/src/components/conversation/embed/ConversationEmbedApp.tsx#L90) · [浏览器会话](../src/frame/desktop/src/app/agent-browser/conversation/session.ts#L1) · [AIWorkspace 面板](../src/frame/desktop/src/app/aiworkspace/ui/jarvis/session.ts#L27) |
| 记忆与输入 | `reply.topic` 更新主题，宿主将一批输入的消息正文与主题用于 recall。模型可主动 `recall --read` 和 `ctl perceive`。常规未压缩聊天复用原生消息前缀，同批默认可收 16 个有效输入，群聊背景随请求附入。[批量召回](../src/frame/lib_opendan/src/runner/memory_batch.rs#L144) · [输入批次](../src/frame/lib_opendan/src/runner/inputs.rs#L146) · [前缀回归用例](../src/frame/lib_opendan/tests/ui_chat.rs#L99) |

## 2. 优先处理

- [ ] **主聊天有稳定入口。** 无显式会话的私信会回落到 `dm:<peer>`，Loader 会续接该地址的未结束 UI session。Desktop 点击 Agent 却会打开最近活跃的 native 会话。
  - **待定**　主聊天绑定现有默认私信地址，还是允许用户指定已有聊天为主聊天？
  - **验收**　先在主聊天 A 发言，再使专题 B 或 App 会话 C 成为最近活跃项。重新从 Agent 主入口进入仍到 A，显式打开 B、C 各自续接。主聊天归档或用户执行 `/new` 时，应有明确的后继行为。[私信地址](../src/frame/msg_center/src/msg_center.rs#L928) · [Loader 续接](../src/frame/opendan/src/ui.rs#L729) · [桌面默认选择](../src/frame/desktop/src/app/messagehub/sessionModel.ts#L33)

- [ ] **让下一次意图分析能用上本次报告。** fork 的结果已进入父聊天历史，但当前 schema 接受 `outcome="work_created", report="", work=null`，宿主对澄清问答只核对条数。收紧报告正文与 outcome 所需字段，核对问答内容，给“已转给现有 goal”一个能携带目标 ID 的结果。修正分析器查找刚结束工作的指引，`--active` 会过滤所有 `Finished`。
  - **验收**　空报告、缺工作 ID、问答条数正确但内容不符，均触发纠正。已有 goal 收到消息后能返回其原 ID。上一份工作失败后再分析，能找回报告里的缺失条件，无需再问已回答的问题。[分析输入与历史查找](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L43) · [返回契约](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml#L115) · [宿主校验](../src/frame/lib_opendan/src/runner/reports.rs#L602) · [CLI 过滤](../src/frame/lib_opendan/src/xagent_cli.rs#L1323)

- [ ] **给聊天的结构化回复接上 `report()` 回退。** 正常路径输出 `{reply, topic, artifacts}`。当前宿主允许一次格式纠正，再错就以 `output_format` 结束本轮。通用 UI 已能用 final report 发正文和附件。顶层 Work 的 final report 另有 `succeeded / failed / blocked`，聊天回退使用 UI 的结束语义。建议保留正常路径，在纠正耗尽时给模型一次等价的 report 提交机会。
  - **待定**　已知 provider 不支持原生 schema 时，是否直接选 report？
  - **验收**　强制两次无效 JSON 后，聊天仍能交付一次有效回复。正文、topic、附件沿用同一套校验，原始坏 JSON 不发给用户，下一条消息可在同一聊天继续。[此前的回复契约决定](done/jarvis-chat-prompt-todo.md) · [失败用例](../src/frame/lib_opendan/tests/ui_chat.rs#L280) · [UI report 投递](../src/frame/lib_opendan/tests/outbound.rs#L280) · [Work outcome 适用范围](../src/frame/lib_opendan/src/runner/reports.rs#L258)

## 3. 后续消息与群聊

- [ ] **统一已有工作的续接规则。** 私聊会把与未结束 plan 有关的补充发回原 plan，暂停时排队。群聊应给被激活的 owner 请求同样的路由指引。普通 `work_simple` 目前拒收追加。
  - **待定**　继续采用停止后重开，还是允许它在下一次执行前消费补充？
  - **验收**　同一用户并行跑两个 plan 时，带明确指代的补充进入对应 plan。歧义会澄清。催促暂停的 plan 不会创建重复工作。群成员的背景发言不会成为任务。同步修正交接说明，`--context recent:6` 实际还会附近期对话摘录。[私聊路由](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml#L20) · [群聊路由](../src/apps/jarvis_agent/agent/behaviors/groupchat_route.toml#L18) · [简单工作约束](../src/apps/jarvis_agent/agent/cmd_skills/ui_core/SKILL.md#L17) · [交接摘录](../src/frame/lib_opendan/src/api.rs#L1145)

- [ ] **把用户对通知的处理结果带回相关聊天。** `ask_user` 的答案已回到提问分支，再由分析报告带给父聊天。普通通知当前向聊天提供“已投递”背景，用户在卡片上处理后，宿主会唤醒发起会话，聊天可用 `notify show` 主动查。建议在原聊天的下一批输入带上通知 ID、处理状态及对该聊天可见的答案。
  - **验收**　用户在通知卡上选择 B 后回到聊天问“按刚才选的继续”，父聊天能关联到 B。重复投递只出现一次，无关聊天看不到该选择。[通知背景](../src/frame/lib_opendan/src/notify.rs#L309) · [唤醒目标](../src/frame/opendan/src/notify.rs#L82)

- [ ] **配置群聊回看条数与激活条件。** 当前群聊默认关闭。开启后，只有 owner 结构化 @ Agent 才触发，每次从同一 mailbox 最近 20 条已读记录取背景，保留说话人并去重。建议先开放回看条数。
  - **待定**　触发条件只需要指定成员的 @，还是已有需要支持的其他条件？
  - **验收**　配置为 3 后，触发请求只带最近范围内的有效背景，普通群聊不单独开轮。被允许者、其他成员、未点名三种消息分别验证。若开放其他触发人，一并明确其可调用的工作和可见记忆。[触发者判断](../src/frame/lib_opendan/src/bridge/msg.rs#L197) · [上限](../src/frame/opendan/src/ui.rs#L48) · [群聊回归用例](../src/frame/opendan/tests/loader.rs#L2122)

## 4. 记忆与长聊天

- [ ] **用户或 App 创建聊天时可选择无记忆。** 目前创建请求没有对应字段，模板 `load_hints=false` 只关闭自动注入。把选择传到宿主，同时约束自动 recall、主动 recall/read 与写感知。
  - **待定**　该聊天创建的 work/plan 是否继承此选择？
  - **验收**　预先在普通聊天留下一个可召回事实，无记忆聊天自动和主动读取都拿不到它。在无记忆聊天给出的新偏好不会出现在其他聊天。重开该会话后选择仍有效。[创建请求](../src/kernel/buckyos-api/src/msg_center_client.rs#L1871) · [自动注入条件](../src/frame/lib_opendan/src/runner/memory_batch.rs#L73) · [记忆访问身份](../src/frame/lib_opendan/src/memory/mod.rs#L238)

- [ ] **明确什么值得写成感知。** 现行手册除长期偏好、纠正与选择理由外，还允许记录环境现象、项目惯例和确切指针。
  - **待定**　能重新观察的事实有时也有历史价值，是否只保留来源及当时的判断？
  - **验收**　分别给模型三个例子——用户说“今后的周报限一页”、项目配置中可随时读取的当前端口、同一工具在某版本失败而改一项参数后成功。检查每例记了什么、出处和适用条件，以及下次怎样找到。[感知写入手册](../src/apps/jarvis_agent/agent/cmd_skills/ui_core/SKILL.md#L30)

- [ ] **让跨轮聊天的历史预算与模型窗口一致。** 运行中已有约 80% 触发、约 30% 重写目标。下一轮重建聊天历史时另有默认 24,000 token 预算，大窗口可能提前丢掉原生前缀。建议默认按实际模型窗口推导预算，并明确显式配置的覆盖行为。
  - **验收**　用相同长对话分别跑小、大窗口，记录触发原因、重写前后 token 与下一轮历史前缀。有足够窗口时保留原消息，逼近触发线时仍能压缩并继续。相关工作已有运行中压缩与摘要失败恢复的记录。[跨轮默认值](../src/frame/lib_opendan/src/runner/mod.rs#L255) · [历史重建](../src/frame/lib_opendan/src/runner/history.rs#L60) · [窗口压缩 TODO](opendan-context-window-compaction-todo.md) · [摘要失败修复记录](fixbug-jarvis-silent-after-compaction-failure.md)

- [ ] **压缩后保留本会话感知线索，给出稳定的原文回读入口。** 摘要保留开头、摘要、精确片段和尾部，worklog 仍在。自动召回会过滤本会话的感知，固定压缩提示也只让模型重读会话目录。建议让摘要携带可定位的感知引用，提示中给出可执行的历史读取方式。现有 `agent-session read-session <sid> --worklog N` 读最近 N 条，底层 `session.worklog` 已支持按 turn 和游标分页。
  - **验收**　压缩前记录用户的关键纠正，连续压缩两次后仍能定位那条感知和原话。之后追加新消息，旧引用仍指向同一条记录。[摘要与固定提示](../src/frame/lib_opendan/src/runner/history.rs#L39) · [本会话过滤](../src/frame/lib_opendan/src/memory/observation.rs#L721) · [历史分页](../src/frame/opendan/src/service.rs#L272)

## 5. 验证基线

上次 Review 在 `4c91b3d0c` 跑过 47 个 Rust 定向测试和 9 个 Desktop 会话测试，全部通过。它们覆盖包加载、群聊、默认会话选择、输入批次、fork、召回、通知、历史压缩与回复失败路径。本稿已按 `623b4d196` 核对相关源码变化。真实模型选择工作形态的准确性、跨轮缓存命中率仍待 DV。可用同一条“直接问答 → 澄清 → 创建工作 → 追加要求 → 读回交付”的对话记录对照改动前后表现。
