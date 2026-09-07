# MessageHub 原型交付说明

2026-09-06。本轮范围为 TODO T0–T8 的可交互 mock 原型，不包含真实后端集成。

## 已完成实现

| TODO | 实现入口与结果 |
|---|---|
| T0 | `mock/store.ts`、`mock/hooks.ts`：统一可变 store、owner / viewer 作用域、IndexedDB 持久差量与文本 / 文件草稿、删除水位、可控时钟 / 失败 / 事件 |
| T1 | `sessionModel.ts`、`EntityList.tsx`：有效消息活动判定、max 时间推进、稳定排序、独立分钟时钟；共享状态、Action、投递和临时运行态不重排 |
| T2 | `SessionSidebar.tsx`：移除右侧选中竖条、显示固定宽度时间、独立可访问处理按钮、连接实例名称、触屏 / 键盘入口 |
| T3 | `SessionDialogs.tsx` 与 store：取消、归档、已归档查看 / 恢复、彻底删除、失败重试；新普通消息恢复归档，删除后的新 tunnel 内容不恢复旧历史 |
| T4 | 全局 / 会话标题 / Sidebar 新建入口，RHF + Zod 表单，策略与连接双重验证，独立 UUID 空会话，第一条消息与刷新恢复 |
| T5 | `SessionDetails.tsx`：独立于 EntityDetails，分别编辑共享状态、自己的昵称与个人偏好；风险确认写入；EntityDetails 配置创建策略 |
| T6 | `conversation/history/`：统一 Action 类别识别、安全摘要、原始索引不变的过滤投影、重新生成日期与可见计数、空过滤提示、长历史锚点恢复 |
| T7 | Agent 主页两类入口、路由与桌面启动 context、观察权限拒绝、写入与草稿隔离、临时确认与连接版本绑定 |
| T8 | 扩展两套 Playwright 回归、Deno 纯数据测试、中英文文案、模型说明和桌面 / 移动截图 |

CodeAssistant 仅同步必要的 DID 引用与共享组件调用方式，保留其既有长历史实现。没有新增依赖，也没有修改 Rust / KRPC 协议。

## 验证

在 `src/frame/desktop` 执行：

```bash
pnpm run check
pnpm run build
pnpm exec playwright test tests/e2e/pages/messagehub.spec.ts tests/e2e/pages/users-agents.spec.ts --project=chromium --workers=4
deno test -c tests/datamodel/messagehub.deno.json tests/datamodel/messagehub-session.test.ts
pnpm exec eslint src/app/messagehub src/app/codeassistant/CodeAssistantAppPanel.tsx src/app/codeassistant/mockHistory.ts src/app/users-agents/components/detail/AgentDetailPage.tsx src/i18n/messagehub.ts src/i18n/dictionaries.ts tests/e2e/pages/messagehub.spec.ts tests/e2e/pages/users-agents.spec.ts tests/datamodel/messagehub-session.test.ts
```

- 类型检查、生产构建通过。构建有现有应用整体 bundle 超过 500 kB 的提示，未因本轮引入额外依赖。
- Playwright：MessageHub 17 项与 Users & Agents 4 项，包含原有 Composer 与 1440px / 375px 长历史滚动回归。
- Deno：5 项通过，验证活动类别、相对时间、排序与标题优先级、owner / viewer 权限、过滤后的 raw messageIndex / 日期 / 增量计数。
- 对上述变更 TS / TSX 文件运行 ESLint，无错误或警告。

行为测试包括：同名创建、空历史、首条消息刷新恢复；文本 / 附件草稿持久化；创建 / 发送 / 删除失败重试与重复提交；取消慢创建后切换实体；状态 / Action / 投递 / 迟到消息不重排；归档与删除重放水位；非选中行操作；删除最后会话；多 tunnel 选择、平台限制与临时写入确认；观察权限与偏好隔离；未知 Action 版本 / actor / subject；长历史过滤与两种详情返回的锚点。

Playwright 使用仓库原有 config 的本地 Vite server；测试通过不代表真实平台创建、代发、权限或后端物理删除已验证。

## 截图证据

截图由 Playwright 流程生成，保存在 [screenshots](screenshots/)，不是手工拼装的设计图。

| 场景 | 桌面 1440px | 移动 375px |
|---|---|---|
| Session 行与操作入口 | [桌面](screenshots/messagehub-sessions-1440.png) | [移动](screenshots/messagehub-sessions-375.png) |
| 新建表单 | [桌面](screenshots/messagehub-create-1440.png) | [移动](screenshots/messagehub-create-375.png) |
| 归档 / 彻底删除 / 取消 | [桌面](screenshots/messagehub-manage-1440.png) | [移动](screenshots/messagehub-manage-375.png) |
| SessionDetails 编辑 | [桌面](screenshots/messagehub-details-1440.png) | [移动](screenshots/messagehub-details-375.png) |
| Action 展示与草稿 | [桌面](screenshots/messagehub-actions-1440.png) | [移动](screenshots/messagehub-actions-375.png) |
| Agent 只读观察与详情 | [桌面](screenshots/messagehub-observer-1440.png) | [移动](screenshots/messagehub-observer-375.png) |
| 原有长历史滚动 | [桌面](screenshots/messagehub-scroll-1440.png) | [移动](screenshots/messagehub-scroll-375.png) |
| 桌面系统内嵌观察入口 | [窗口](screenshots/messagehub-desktop-agent-observer.png) | [移动窗口](screenshots/messagehub-mobile-embedded-observer.png) |
| 内嵌处理对话框 | [窗口](screenshots/messagehub-desktop-manage.png) | [移动窗口](screenshots/messagehub-mobile-embedded-manage.png) |

移动桌面内嵌创建另见 [截图](screenshots/messagehub-mobile-embedded-create.png)。

## 后端边界

本轮的 create、manage、updateState、send 等均为 mock store 方法，不是新的 KRPC。
后续仍需消息活动时间与跨页排序游标、空会话登记、权威状态版本、可靠 Action 发布、平台连接能力、跨 owner 服务端授权、Session 级归档 / 删除与共享对象引用处理。
删除仅清理当前 owner 的 mock 会话与本地历史引用，不使用 `RecipientState.DELETED` 冒充物理删除；不可变 seed 夹具不作为恢复源。附件发送仍是原型摘要，不代表文件已上传到远端。

精确字段与持久状态见 [当前 UI Model Data](../../product/message_hub/MessageHub_Current_UI_Model_Data.md)；集成目标和新的时间口径见 [UI_DATAMODEL](../../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。

2026-09-07 源码 Review 后仅更新了文档，以上原型交付与测试记录没有新增代码验证。
真实接入还需修正群实体归属、本地已读 / 回执调用、提交结果与逐目标投递信息，并补请求处理、
native / 群权限、对象附件访问、非追加历史更新及断线对账。
后续任务见 [TODO §3](TODO.md)，真实接入验收以 [UI_DATAMODEL §9.4](../../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md#94-接入验收条件本次未执行) 为准。

## 2026-09-07 真实后端集成（TODO §3）

本节记录 TODO §3 的真实接入结果；上文 mock 原型的记录保持不变。

### 后端（msg-center）

| 改动 | 位置 |
|---|---|
| schema v9：`owner_sessions`（登记 / 生命周期 / 删除水位）、`owner_ui_session_states`（owner 范围 UI 状态）；启动时在 spec DDL 之后再应用编译内 DDL，旧 zone 升级不需重写 spec | `msg_center_client.rs`、`msg_box_db.rs::apply_schema` |
| `msg.create_session` / `msg.archive_session` / `msg.restore_session` / `msg.delete_session` / `msg.get_session_state`；`ui_session.*` 带 `owner` 时走 owner 范围表 | `msg_center_client.rs`（trait / client / 路由）、`owner_session.rs`、`owner_session_db.rs` |
| `msg.list_sessions` 新增 `lifecycle` / `order_by: activity`，`SessionSummary` 新增 `last_activity_ms` / `request_count` / `lifecycle` / `state`；`list_session` 与摘要均应用删除水位 | `owner_session.rs::list_sessions_scoped` / `list_session_scoped` |
| 新普通消息（chat / group_msg / deliver）提交后自动解除归档；事件与状态更新不会 | `msg_center.rs` 提交钩子 → `note_records_committed` |
| 授权：从 verify-hub 用户 token 解析 viewer；读取限本人或 zone 托管非用户身份（Agent），写动作限本人；服务 / 设备 token 与无 token 调用保持原行为 | `owner_session.rs::authorize_owner_read/write`，接入 post_send、get_next、peek/list_box、list_sessions/session、update_record_state、set_read_state、get_record、会话 / owner UI 状态 RPC |
| `GET /kapi/msg-center/objects/{obj_id}[/content]`：会话 token 鉴权的对象 JSON / FileObject 内容下载（64MB 上限） | `object_access.rs`、`main.rs::serve_request` |

### 前端（`src/frame/desktop/src/app/messagehub`）

- `store/`：`MessageHubStore` 接口与选择器（`VITE_MESSAGEHUB_USE_MOCK` 覆盖，否则跟随 `VITE_CP_USE_MOCK`）；组件只从 `./store` 取数据。
- `api/store.ts`：真实 store（owner 数据按 owner 隔离、epoch 丢弃迟到响应、轮询 + kevent 刷新、发送 / 已读 / 生命周期 / 偏好 / 准入）；
  `api/projection.ts`：实体归属、绑定、标题、摘要的纯投影；`api/reader.ts`：分页 reader 与 upsert / remove；
  `api/objects.ts` / `api/upload.ts`：对象访问与 NDM 上传；`api/local.ts`：viewer 本地草稿 / 过滤 / 创建策略。
- 组件：会话行请求标记、会话横幅与准入动作、实体详情准入区、逐条请求标记、投递详情、附件渲染与占位、
  历史向上翻页 / 记录级重建保持锚点、可见记录标已读、发送失败原因、实体列表“加载更多”与“请求”过滤。
- `mock/` 原型保持原行为并实现同一接口（mock Playwright 21 项仍全部通过）。
- 开发环境接入：`VITE_ZONE_PROXY` / `VITE_ZONE_PROXY_IP` 使 Vite 代理 `/kapi`、`/sso_*`、`/ndm` 到真实 zone 根域，
  `main.tsx` 在该模式下以开发源作为 zone host 初始化 SDK。

### 验证（均已真实运行）

```bash
# Rust：msg_center 77 项通过（含 5 项新增：生命周期 / 水位、活动排序与游标、登记与首条消息、owner UI 状态隔离、token 授权）
cd src && cargo test -p msg_center && cargo test -p buckyos-api msg_center

# 真实 zone RPC 验收（devtest 用户 token；观察 jarvis、拒绝越权、登记 / 发送 / 幂等 / 归档 / 删除 / 重放 / 对象路由）
cd test/test_msg_center && deno run --config ../deno.json --allow-net --allow-env --unsafely-ignore-certificate-errors test_messagehub_sessions.ts

# 前端
cd src/frame/desktop
pnpm run check && pnpm run build
pnpm exec eslint src/app/messagehub src/i18n/messagehub.ts src/main.tsx vite.config.ts tests/e2e/real tests/datamodel
deno test -c tests/datamodel/messagehub.deno.json tests/datamodel/messagehub-session.test.ts tests/datamodel/messagehub-projection.test.ts   # 9 项
pnpm exec playwright test tests/e2e/pages/messagehub.spec.ts tests/e2e/pages/users-agents.spec.ts --project=chromium --workers=4       # mock 21 项

# 真实 zone UI（先起代理 dev server，再运行）
VITE_CP_USE_MOCK=false VITE_ZONE_PROXY=https://test.buckyos.io VITE_ZONE_PROXY_IP=127.0.0.1 pnpm run dev --host 127.0.0.1 --port 4175 --strictPort
MESSAGEHUB_REAL_E2E=1 pnpm exec playwright test --config=playwright.real.config.ts   # 4 项：观察只读 + 请求记录、登记→发送→刷新→个人标题→归档→恢复→删除、375px、附件上传 / 下载
```

- 部署：重建的 `msg_center` 已替换 `/opt/buckyos/bin/msg-center/msg_center`（原二进制保留为 `msg_center.bak-20260907`），
  node_daemon 自动拉起，启动日志确认 `owner_sessions` / `owner_ui_session_states` 已创建，Telegram tunnel 正常重连。
- 真实 zone 数据：observe `did:web:jarvis.test.buckyos.io` 得到 Telegram 会话（SENT + REQUEST_BOX 记录）；
  devtest 自己的会话来自 Agent 回复（落入 REQUEST_BOX，UI 显示请求横幅与准入动作）。
- 真实 zone UI 通过 API 登录注入刷新 cookie（control-panel 只向 zone 根域签发系统 token，开发端口无法完成 SSO 跳转），
  与生产 SSO 回调留下的状态一致。

### 截图（[screenshots](screenshots/)）

mock 原型截图由 `messagehub.spec.ts` / `users-agents.spec.ts` 重新生成（19 张，文件名同上表）；真实 zone 截图：

| 场景 | 文件 |
|---|---|
| 观察 Agent（只读、请求记录、Telegram 会话） | `real-observer-1440.png`、`real-observer-375.png` |
| 登记会话后发送首条消息 | `real-session-1440.png` |
| 附件上传后按对象访问渲染 | `real-attachment-1440.png` |

### 仍未实现（见 TODO §3 末尾）

批量已读 / 回执持久化、接受后的请求记录迁移、共享 / 成员状态权威与 GroupEvent → Action Log 发布、
tunnel 能力声明与 Agent 代发授权、已加载尾页之外旧记录的变更游标、`ui_session` owner 范围批量读取。
这些在 UI 中以只读、原因说明或按需读取呈现，不冒充已完成。
