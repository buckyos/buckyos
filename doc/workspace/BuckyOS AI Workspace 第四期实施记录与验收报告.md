# BuckyOS AI Workspace 第四期：实施记录与验收报告

> 日期：2026-10-30。对应设计：[第四期规划](<BuckyOS AI Workspace 第四期规划.md>) v0.3（§12 Q01–Q24 已定），实施 TODO 与自定裁决见 [notepads/aiworkspace-phase4-todo.md](<../../notepads/aiworkspace-phase4-todo.md>)（§1 I1–I13）。
>
> 环境：e2e 用独立运行的 `aiworkspace` 服务（调试构建）＋独立 nfs-server ＋ Vite 开发服务器，Chromium（Playwright）；语音用 str0m 充当浏览器对端，以及 Chromium 的假麦克风。DV 在本机单 OOD Zone `test.buckyos.io` 上运行（release 构建的服务与 `pnpm build` 的 Desktop 部署到 `/opt/buckyos/bin/`，经 cyfs-gateway 访问），浏览器与 OOD 在同一台机器（见 §5、§6）。工作区在 HEAD `e9a19faa1` 之上，改动未提交。

## 1. 结论

第四期 M0–M4 的主线已经实现，并在单机上端到端跑通：老师从工作区“开公开课”，得到同一 Host 上的房间地址；观众凭链接以自己的身份（Zone 账号、External DID、访客）加入，公共大屏和提示器各有自己的链接；服务端确认每一步并广播给所有终端；选择题可以发给所有人、随机点名（种子入任务）或限额抢答；答案只到本人和老师；老师公布的是一个新的结果实体；弹幕与位置反馈在舞台上；语音经 Host 的 SFU 转发并按时间轴录成 Ogg Opus 分段；下课后原链接变为回放，老师保存后归档，回放可定位、变速，评论和评分属于这场。

- **房间与本场副本（§3）**：注册表 `sessions.sqlite`；本场 Workspace 是整库克隆（`purpose=session`），克隆后裁历史到克隆点、富文本压成浅快照、授权重置为 Owner／Presenter、资产在本场自己的 key 下重新绑定、备注剥离到 Presenter 作用域的表；开场基线行另存为回放基线。状态 `prepared → live → ended → archived／expired`，访问开关、评论开关、删除相互独立。
- **权限（§4）**：内核新增 `presenter`、`participant:<p>` 两种 scope 和 `Access::reviewer`；邀请链接 `at_`／`dt_`／`pt_` 的权限属于本场，撤销即删其授予的读权限；大屏与提示器以 `link:<invite>` 主体只读本场 Workspace；guest 互动由房间开关决定，默认关闭。
- **同步（§5.1）**：服务端执行步骤规则并确认（Q01-C），转场由服务端决定；单活动控制端与显式接管（Q05）；WebSocket 通道（Q04），先快照再补事件，长轮询降级；在线＝连接存活且 30 s 内有心跳（Q18）。
- **任务与 feed（§5.2–§5.4）**：内核新增 `buckyos.task`／`buckyos.response`／`buckyos.result` 三种实体（只由 origin `session` 写）、原子认领 `session.claim`、`session` Source 与固定七个 feed；feed 在同一 Commit 内随任务和作答刷新，出席与反馈节流到每秒一次。课堂互动默认扩展（选择题、在线观众）在 Desktop 侧，只靠绑定 feed 和调用 `session.*` 契约。
- **记录（§6）**：本场 Workspace 的 `session_events` 是时间线；每个 Commit 在自己的事务里写事件（不存在“数据已写、事件未写”的窗口）；步骤、采样（指针、相机、设备切换）、出席、媒体状态、反馈、迟到提交都入时间线，按作用域过滤。
- **回放（§7）**：浏览器用 WASM 副本从基线行起应用本场 Commit 的行增量到任意时刻，非 Commit 状态从时间线读出；回放宿主的 Workspace 会话是只读代理，不提交、不运行。
- **语音与媒体（§7.3）**：OOD 内置只转发音频的 SFU（str0m，ICE-lite，单 UDP 端口），老师一路上行、观众各一路下行，录制按演示时间写 10 s Ogg Opus 分段（DTX 停顿补 20 ms 静音帧以保持时轴，丢包只计数）；媒体端口不可用时房间明确报“语音不可用”且不回落（Q22）。课件媒体不转发声音：老师的播放／暂停／定位／变速与每 5 s 的 `sync` 锚点是经服务端确认的命令，观众端按 Range 地址本地播放并按 400 ms 硬阈／80 ms 软阈追平。
- **保存与课后（§8）**：`session.save` 写归档 manifest 并把基线、结束快照与全部资产绑定为 `session` 保留集（不受 30 天历史回收）；未保存场次按服务级期限（默认 7 天）到期删除本场 Workspace、保留行；评论、时间点感受、评分是本场新数据；作品的历次公开课在开课对话框中列出。

## 2. 交付物

| 交付物 | 位置 |
| --- | --- |
| 内核：scope `presenter`／`participant:`、`Access::reviewer`；类型 `buckyos.task`／`response`／`result`（`SESSION_ONLY`）；Source kind `session` 与 feed 目录 `SESSION_FEEDS`；`session` 系统文件夹与内部操作 `system.ensure`；origin `session` 可写 session feed；信封投影 `scope: presenter｜participant` 与 `source.feed`；`richtext::export_shallow` | `core/src/{access,model,plan,types,read,richtext}.rs`；`schemas/operations.json` |
| 存储：注册表（会话、邀请、参与者、课后帖子）；`clone_for_session`、历史裁剪、备注剥离、基线；状态机、邀请、加入、舞台命令、任务／认领／作答／评阅／公布／反馈／采样；feed 计算与差分刷新；`session_events` 与 Commit 同事务；归档、到期、删除；回放基线过滤；语音分段入库；结束后本场只收 session 写入（`SESSION_CLOSED`） | `store/src/session/{mod,registry,feeds}.rs`；`store/src/workspace.rs`（`session_events`、`as_session`、提交内事件）；`store/src/{service,source,export}.rs`；新增 `store/tests/session.rs` |
| 服务：`session.*` 31 个方法与 `asset.stream_url`；链接令牌认证（kRPC、资产路由、通道）；按场 hub（广播、出席、控制端、节流 feed、到期清扫）；WebSocket `/kapi/aiworkspace/session/ws`；长轮询 `session.watch`；Range 媒体路由 `/kapi/aiworkspace/stream/<token>`；音频 SFU 与 Ogg Opus 录制；本场 Workspace 在场次未进行时不刷新 Source | `server/src/{session,media,stream,source,lib}.rs`；`server/Cargo.toml`（axum `ws` 特性、`str0m`）；新增 `server/tests/{session,voice,stream}.rs` |
| 契约 | 服务 README “Presentation sessions”、方法表与 HTTP 路由表 |
| WASM | `desktop/src/app/aiworkspace/wasm/`（`wasm/build.sh` 重新生成） |
| Desktop 会话层：类型、`SessionApi`、`SessionChannel`（握手、续接、心跳与时钟、调用、重连、降级）、`LinkTransport`、`Transport.channelToken/basePath`、`WorkspaceSession.call` | `session/{types,api,channel,labels}.ts`、`api/{transport,session}.ts`、`offline/replicaSession.ts` |
| Desktop 房间：页面（链接→传输→加入→本场 Workspace；未登录的访客可在加入页先以账号登录，无链接且未登录时转 Zone 登录页）、房间根节点 `data-channel` 标出通道状态、老师房间（控制条、按键、指针与相机采样、板书写成 ink Block、语音、链接／参与者／备注面板、接管、开始与下课）、观众房间（跟随／自由浏览、位置反馈、弹幕、纯净模式、语音收听、仅观看提示）、大屏、提示器 | `session/{SessionPage,Rooms,LiveStage,room,steps,context,voice,media}.tsx/ts`、`session/session.css`；`standalone/StandalonePage.tsx`（`?session=`）；`presentation/StageCanvas.tsx`（`follow`）；`ui/media/AssetMedia.tsx`（会话内媒体同步播放） |
| Desktop 回放与课后 | `session/{replay.ts,ReplayRoom.tsx}` |
| 开课入口与历次公开课 | `session/StartSessionPanel.tsx`、`presentation/StartShowDialog.tsx`、`ui/shell/WorkspaceShell.tsx` |
| 课堂互动默认扩展：选择题（发题、随机点名、抢答、收回、结束、公布、作答、名册）、在线观众（在线列表、对照名册、公开投影、导入名册） | `ui/classroom/{ChoiceBlock,AudienceBlock}.tsx`、`ui/classroom/{feeds,register}.ts`、`ui/blocks/registerAll.ts` |
| 文本 | `src/i18n/aiworkspace/session.ts`（英／中）、`tests/datamodel/aiworkspace-i18n.test.ts` |
| e2e 与 DV | `tests/aiworkspace/session.spec.ts`（S-01–S-03）；`tests/aiworkspace/start-backend.mjs`（媒体端口与分段长度）；`vite.config.ts`（`/kapi/aiworkspace` 代理 `ws: true`）；`tests/dv/aiworkspace.session.spec.ts`（真实 Zone：子域房间、wss、OOD 媒体端口、账号凭观众链接加入） |

## 3. 验证

```bash
# 在 buckyos/src
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1
# 其中新增：store tests/session.rs（6）、server tests/session.rs（1，真实进程 + WebSocket）、tests/voice.rs（2，str0m 对端）、tests/stream.rs（1）、media 单元（2）
# 在 desktop
pnpm check && npx eslint src/app/aiworkspace
deno test --allow-read --allow-env tests/datamodel/aiworkspace-i18n.test.ts
AIWS_BIN=… NFS_BIN=… pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/session.spec.ts
# 回归：放映、媒体、Block
AIWS_BIN=… NFS_BIN=… pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/presentation.spec.ts tests/aiworkspace/media.spec.ts tests/aiworkspace/blocks.spec.ts
# DV（本机 Zone；先部署 release 服务与 Desktop 构建）
BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/aiworkspace.session.spec.ts
```

结果（2026-10-30）：三个 Rust 包 248 项测试全部通过；`pnpm check`、eslint 无错；i18n 检查中本期新增的 `session` 区通过，另两项失败在本期之前就存在（`canvas.catalog.sample`／`extension` 未用键、`store/src/wish/outline.rs` 中的“字”）；e2e 与 DV 的结果见 §5。

## 4. 验收场景对照（§10）

| 编号 | 状态 | 证据与说明 |
| --- | --- | --- |
| P4-01 | 通过（单机） | store `ending_saving_and_restarting_keep_what_was_confirmed`：同一课件两场，本场 Workspace、事件、参与者独立；`session.list` 列出历次 |
| P4-02 | 通过（单机） | server `a_room_from_a_workspace…`：观众通道不出现他人的作答；大屏链接读不到备注、读不到源 Workspace；e2e S-01：观众页面不含备注 |
| P4-03 | 部分 | 服务端步骤与转场、观众跟随／自由浏览／回到老师视角已实现，e2e 覆盖步骤跟随；跟随老师自由相机的画面与动画中断未做自动断言 |
| P4-04 | 通过（单机） | store：两名额第三人 `CLAIM_FULL`、重复认领同一结果、迟到 `TASK_CLOSED` 记事件；server：五人并发抢一个名额恰一人得 |
| P4-05 | 通过（单机） | store／e2e：作答仅本人与老师可读；公布生成结果实体；评阅默认私有、`published` 后进入本人 `my_responses` |
| P4-06 | 部分 | 反馈、弹幕、板书（ink Block）、媒体状态、Commit 同在时间线；回放不运行原业务（只读会话、HTML 只渲染静态）；AI 输出在回放中按行增量呈现，未单独测 |
| P4-07 | 部分 | 回放按时间定位到步骤与内容，语音分段按偏移播放；“一笔未完成”：笔迹在落笔结束时入库，回放从提交时刻显示整笔，不显示中途笔段（§7 出入 13-9） |
| P4-08 | 部分 | 本场副本整库克隆、资产在本场 key 下重新绑定、源课件改动不影响本场；场次结束后本场 Source 不再刷新；外部资源失效的“缺失提示”未做专门 UI |
| P4-09 | 通过（设计上） | 事件在 Commit 事务内写入（不存在窗口）；store 测试核对每个 Commit 恰有一条事件 |
| P4-10 | 部分 | 控制端断开不影响观众，房间按服务端状态继续；语音停止记 `voice.stop`；暂存失败的界面提示与弱网未测 |
| P4-11 | 通过（单机） | store：非 Owner 的 Presenter 结束后不能保存，Owner 可保存 |
| P4-12 | 部分 | 课后评论／评分／时间点感受、撤销链接收回读权限、删除只释放本场；回放后撤销访问再读的 e2e 未做 |
| P4-13 | 通过（单机） | store 重开服务、server 重启真实进程：状态、舞台、事件、参与者在盘上，观众重连后补事件并继续 |
| P4-14 | e2e S-02（§5） | 对照名册的在册／缺席／不在册三组、随机点名不重复、被点者终端弹出作答、断线者收回后重选、点名入任务（种子在 `pick`） |
| P4-15 | 部分 | 语音：server `voice.rs`、e2e S-03 与 DV（OOD 的 4122/udp）证明经 Host 转发与录制分段；媒体同步算法与 Range 路由已实现并单测 Range；多观众对齐、换耳机、弱网、中途加入的实测未做 |
| P4-16 | 通过（单机＋本机 Zone DV） | 公开课 URL 在源 Workspace 的同一 Host（`?session=`）；所有终端只连 Host；老师浏览器不转发；DV 中房间开在 `aiws-<id>.test.buckyos.io`，通道经网关为 wss |
| P4-17 | 通过（单机） | store／e2e：未开 guest 互动的场次 guest 作答与反馈被拒、只能观看；开启的场次 guest 响应带 `level: guest`；点名池只取已认证身份；任何观众凭据读不到克隆点之前的历史与剥离后的备注 |
| P4-18 | 通过（单机） | server `a_host_without_its_media_port_says_so`：`AIWS_MEDIA_PORT=off` 时 `session.get.voice_service.available=false` 并给出原因，`session.rtc` 拒绝；开课前提示 |

## 5. e2e

`tests/aiworkspace/session.spec.ts`：

- **S-01**（P4-02／05／14／17 的界面部分）：老师、已登录观众、访客、公共大屏四个上下文；开始、发题、作答、计数、结束、公布、弹幕、翻页跟随、下课、保存、回放定位、课后评论、历次列表。
- **S-02**（P4-14）：名册三人、在线三人（含一名不在册的 Limited 账号）；对照名册三组计数；随机点名一人、其他人未被点；被点者断线、收回、再点到另一人并作答；池空时提示而不重复。
- **S-03**（P4-15 语音部分）：Chromium 假麦克风，老师广播、观众收听，停止后时间线出现 Ogg Opus 分段。

`tests/dv/aiworkspace.session.spec.ts`（本机 Zone）：老师登录到源 Workspace 的子域并打开房间，通道状态为 `open`（WebSocket，未降级为长轮询）；`session.get.voice_service.available=true`，开课、开麦；新建的第二个账号打开观众链接，在子域上是访客，点“以账号登录”经 Zone 登录页接力回到房间，以 `verified` 身份加入并收听语音；翻页经网关跟随；12 s 语音后时间线出现 `audio/ogg` 分段；下课、保存，原链接变为回放；最后删除场次与工作区。

运行结果（2026-10-30）：

| 用例 | 结果 |
| --- | --- |
| `tests/aiworkspace/session.spec.ts` S-01–S-03 | 3 passed（36.9 s） |
| 回归 `presentation.spec.ts`、`media.spec.ts`、`blocks.spec.ts` | 37 passed（5.0 min） |
| DV `tests/dv/aiworkspace.session.spec.ts` | 1 passed（23.8 s）；前两次失败是测试自身的问题（在已跳到子域后再取源域名、工作区标题不符合夹具的清理约定），已改测试 |

DV 中发现并修正两处服务端问题：场次删除后，仍连着的终端让 run Workspace 的 head 监听再读一次时间线并报 `NOT_FOUND`（`pump` 现在先确认房间仍在册）；公开版的 `participants` feed 实体名没有用上 `participants-public`。

## 6. 未验证与风险

- **DV 只覆盖本机单 OOD Zone**：网关对 `/kapi/aiworkspace/session/ws` 的 wss 透传、子域上的会话页面、账号凭链接加入、OOD 媒体端口都已在本机 Zone 实测，但浏览器与 OOD 在同一台机器上（媒体走 `192.168.1.165`／`127.0.0.1` 的 4122/udp）。NAT 之后、跨网络、经 SN 中继的访问没有测：服务只绑定本机地址，公网或中继部署需要网关侧的独立方案（Q22 已定为网关独立需求）。多 OOD／多节点没有测。
- **未开始的场次不过期**：到期清扫只处理已结束未保存的场次；创建后从未开始的场次一直保留本场 Workspace，直到 Owner 删除。
- **容量与弱网**：50 人并发、状态延迟 ≤2 s、语音 ≤500 ms、媒体偏差 ≤200 ms 都没有测量；自适应码率与阈值未做。
- **传输**：语音只有 UDP（无 ICE-TCP 备用），无 TURN。
- **回放性能**：没有关键快照，回退定位从基线重建；长场次需要补关键帧。
- **课堂互动扩展**以 Desktop 内置 Block 渲染器实现（规划允许“表达能力不够时降为内置 Block”），尚不能作为 HTML／组合控件包被替换；评阅只有服务端契约，没有界面。

## 7. cmd_skills

检查了 `aiworkspace_run` 与 `aiworkspace_template_dev`，都不需要修改：

- `aiworkspace_run`：run 的领取、快照、结果、心跳与 `proc.*` 协议没有变化；Wish 的渲染器目录 `aiws/renderers.json` 也没有变化（课堂 Block 依赖本场 feed，不是 Wish 的产出视图）。
- `aiworkspace_template_dev`：手册列出的是 Agent 能写的实体类型与 Source。`buckyos.task`／`response`／`result` 只能由 Session 服务以 origin `session` 写（其他写入得到 `SESSION_ONLY`），`session` Source 的样例实体由课堂 Block 在编辑器里通过 `session.feed_sample` 放置；`session.*` 是给人用的房间界面，本期没有 Agent 主持公开课的流程。导出、导入、发布与 `ws.grant` 的行为没有变化。

## 8. 与规划的出入

见 [第四期规划 §13](<BuckyOS AI Workspace 第四期规划.md>) 与 TODO §6。
