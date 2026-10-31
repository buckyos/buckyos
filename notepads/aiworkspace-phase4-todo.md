# AI Workspace 第四期 TODO：公开演示房间、记录与回放

日期：2026-10-30

依据：[第四期规划 v0.3](<../doc/workspace/BuckyOS AI Workspace 第四期规划.md>)（§12 Q01–Q24 全部已定）。本单按 §9 的 P0→M4 推导，按“内核原语 / 课堂互动扩展”分列；§1 是实施层面的自定裁决（规划未细到的地方），出入记 §6。

## 1. 实施裁决（规划未细到，按“新增代码最少、复用现有机制”定）

- **I1 Session 注册表**：服务级 `sessions.sqlite`（数据目录下，同 `library.sqlite`）：`sessions`（状态机、Owner／Presenter、策略、基线、权威舞台状态、归档 manifest）、`session_invites`（`at_`／`dt_`／`pt_`）、`session_participants`（出席）、`session_posts`（课后评论／评分／时间点感受）。状态 `prepared → live → ended → archived`，另有 `expired`；访问 `open｜closed`、课后发帖 `open｜closed`、删除，三者独立。
- **I2 本场 Workspace**：`Service::clone_for_session` 沿用 `VACUUM INTO`，`purpose=session`、`session_source`、`session_id`；不被 `drop_stale_clones` 清扫、不计放映克隆上限、Owner 为本场 Owner（配额计 Owner，Q12）。克隆后一次性处理：授权表重置（Owner 全部 + Presenter 编辑能力；分享入口、发布、referrers、用户状态、锁、暂存清空）；`base_seq`＝克隆点并删除更早的 `commits／commit_ops／commit_deltas／entity_versions`，富文本压成 shallow snapshot（不带编辑历史）；一次“备注剥离”内部 Commit 把 Frame `presentation.notes`／Viewport `notes` 移到 Presenter 作用域表 `session/notes`（按 step 索引）。
- **I3 作用域**：`access.rs` 新增 `presenter` 与 `participant:<principal>` 两种 scope。`Access.reviewer`＝本 Workspace 任一写能力或 manage（Presenter／Owner）。`presenter` 仅 reviewer 可见；`participant:<p>` 对 p 与 reviewer 可见，另可由实体 `readers` 显式授权列表扩展。读取、变化流、副本 bootstrap、导出都经 `sees_scope`，不另写过滤。
- **I4 实体类型**：`buckyos.task`／`buckyos.response`／`buckyos.result`，只能放在 `session` 系统文件夹，只由 origin `session`（新 origin：Session 服务代参与者写入）写；核心只校验形状与上限，玩法字段放 `ext`（扩展自管）。
- **I5 `session` 系统文件夹**：不加进 seq 0 的系统节点（会改变所有 Workspace 的初始状态与 fixture 摘要），由内部操作 `system.ensure { id: "session" }` 按需创建，固定 id `session`、`system: "session"`，不可删除／移动／改名；许愿格可见（Q21），`shows` 的排除不变。
- **I6 feed**：Source `{ kind: "session", feed, mode: "live" }`，七个 feed 全部是 Table（`stage` 也用单行表，便于表格视图绑定；见 §6）。字段固定（`store/src/session/feeds.rs`），刷新复用 `source_ops` 的表格差分，Commit origin `source`。样例实体由 `session.feed_sample` 创建；开场时清空样例行；`my_responses` 每个参与者一张 `participant:` 作用域表（id 由样例 id 与 principal 派生），客户端按 `session.join` 返回的 `my_feeds` 重定向绑定。高频 feed（participants／reactions／responses）每秒至多一次。
- **I7 事件**：本场 Workspace 的 `doc.sqlite` 里建 `session_events`（克隆时创建，普通 Workspace 没有这张表），自增 `seq` 即本场服务端事件序号。Commit 在同一事务里写一条 `commit` 事件（outbox 直接就是事件表，P4-09 无窗口）；舞台、采样、出席、媒体等事件直接写。事件头按 §6.2，作用域 `public｜presenter｜participant:<p>｜diag`。
- **I8 权威**：Q01-C。服务端按路径步骤执行 `next/prev/first/last/goto/black/back/free` 并确认；转场按“同 Surface 用 fly、否则 fade、步骤自带的优先”在服务端决定；取景矩形由控制端附在命令里，服务端记录。指针／相机／笔迹是采样事件。
- **I9 凭据**：`at_`（观众邀请，可多人共用；`session.join` 时绑定调用者自己的身份：Zone 账号／External／guest，写一条 `via_entry=<邀请 id>` 的读授权，撤销邀请即删）、`dt_`（大屏，主体 `display:<邀请 id>`，只读公共投影，带“是否播语音”）、`pt_`（提示器，代 Presenter 发命令／看备注）。令牌由服务生成、存原值以便 Owner／Presenter 再次复制。
- **I10 传输**：`GET /kapi/aiworkspace/session/ws`（axum `ws` 特性，无新依赖）；消息 `hello / welcome / event / cmd / ack / sample / ping / pong / rtc / control`；长轮询降级 `session.watch`。每场一个内存 hub（`tokio::sync::broadcast`），订阅端按作用域过滤。
- **I11 语音**：str0m（用户 2026-10-30 确认）`default-features=false, features=["aws-lc-rs"]`，单 UDP 端口 ICE-lite；Opus 包按演示时间写 10 s Ogg Opus 分段（手写 Ogg 页与 CRC，不加 ogg crate），分段经 `stage_asset` 成为本场资产并记事件。媒体端口 `--media-port`／`AIWS_MEDIA_PORT`（默认 4122/udp），公网地址 `AIWS_MEDIA_PUBLIC_IP`；未配置或绑定失败时 `session.get.voice = { available: false, reason }`，开课前提示（P4-18）。
- **I12 回放**：开场时把本场 Workspace 的完整行（`replica_bootstrap` 同形）存为基线文件；`session.replay` 按调用者**当前**权限过滤基线行与各 Commit 的 `rows`，连同有权限的非 Commit 事件、语音分段一次给出；浏览器用 WASM `Replica` 从基线起按 seq 应用到目标时刻，渲染宿主走只渲染模式（不挂 live Block、不提交）。
- **I13 课堂互动扩展**：Desktop 内置 Block 渲染器（`classroom.choice`、`classroom.audience`、`classroom.stats`），只靠绑定 feed + 调 `session.*` 契约；放在 `ui/classroom/`，与内核分开，规则（题目标识、复用、点名池、抢答细则）都在这里。

## 2. 工作项

| 阶段 | 内核原语（core／store／server） | 课堂互动扩展（Desktop `ui/classroom/`） |
| --- | --- | --- |
| P0 | 工作区已干净（HEAD e9a19faa1）；axum `ws` 特性；str0m 依赖；媒体端口配置 | — |
| M0 | §1 契约写进 README “Presentation sessions”；方法表 `session.*`（experimental）；feed 字段表；事件类型表 | 扩展接口：绑定哪些 feed、调用哪些 `session.*` |
| M1 | 注册表与状态机；整库克隆 + 历史裁剪 + 备注剥离；`session.create/get/list/start/end/invite/list_invites/revoke_invite/join/leave/takeover/set_access/delete`；`at_／dt_／pt_`；guest 开关；WebSocket hub、快照 + 补事件、长轮询降级；重启恢复（注册表与本场 Workspace 都在盘上） | — |
| M2 | 事件表与 Commit 同事务；`session` Source、七个 feed、样例实体、开场激活、节流刷新；task／response／result 类型与 scope；`session.task/claim/respond/review/publish/react`；采样（指针 10 Hz、ink 段、相机）；媒体状态事件与 sync 锚点；资产 Range 与上限；SFU 语音 | 选择题控件（发题、作答模式、统计）；在线观众控件（在线列表、对照名册）；名册导入（表格导入模块）与匹配；随机点名（种子入 task）；抢答；结果发布；弹幕与位置反馈 |
| M3 | 归档 manifest、基线／结束快照、`retentions` 新种类 `session`、语音分段索引、`session.replay`、未保存场次到期删除（服务级 `session_retention_days` 默认 7） | 回放播放器（播放／暂停／变速／定位、步骤与题目导航）；只渲染宿主 |
| M4 | 课后发帖（评论、评分、时间点感受）、评阅、作品历次演示索引、访问撤销与删除分离 | 课后面板；作品“历次演示”列表 |

## 3. 验证

- Rust：`cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1`；新增 `store/tests/session.rs`、`server/tests/session.rs`（真实进程 + WebSocket）。
- Desktop：`pnpm check`、eslint、e2e `tests/aiworkspace/session.spec.ts`。
- DV：本机 Zone（媒体端口、wss 透传）：`tests/dv/aiworkspace.session.spec.ts`。

## 4. 风险与未做

- 真实多设备、弱网、50 人容量需实际环境；本单只做单机多浏览器与脚本并发。
- cyfs-gateway／SN 媒体中继是网关独立需求（Q22），不在本单。

## 5. 进度

2026-10-30（未提交）：

- [x] P0 工作区干净；axum `ws`、str0m 依赖（用户确认 str0m）；媒体端口 `AIWS_MEDIA_PORT`／`AIWS_MEDIA_HOSTS`
- [x] M0 契约：服务 README “Presentation sessions”、方法表（`session.*` experimental）、HTTP 路由表
- [x] M1 注册表与状态机、整库克隆＋历史裁剪＋备注剥离、链接与身份、WebSocket hub、长轮询、重启恢复（store／server 测试）
- [x] M2 事件表（Commit 同事务）、feed、task／response／result、claim／respond／review／publish／react／sample、SFU 语音与录制、媒体状态命令、Range 地址；课堂互动扩展（选择题、在线观众、名册、点名、抢答、公布）
- [x] M3 归档 manifest 与 `session` 保留集、到期删除、回放（WASM 副本＋时间线＋语音分段）
- [x] M4 课后帖子（评论、时间点感受、评分）、访问与评论开关、删除、历次公开课列表
- [x] Desktop：房间页面（老师／观众／大屏／提示器）、开课入口、回放与课后、i18n、e2e `tests/aiworkspace/session.spec.ts`
- [x] 本机 Zone DV（`tests/dv/aiworkspace.session.spec.ts`）：网关 wss 透传、子域会话页、OOD 媒体端口、账号凭观众链接登录后加入
- [ ] NAT／跨网络／SN 中继下的媒体可达；未开始场次的过期策略
- [ ] 容量／弱网／多设备实测；评阅界面；回放关键帧

## 6. 与规划的出入

写在规划 §13（13-1 至 13-17），实施记录见 `doc/workspace/BuckyOS AI Workspace 第四期实施记录与验收报告.md`。
