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
