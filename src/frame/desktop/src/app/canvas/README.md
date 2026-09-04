# BuckyOS AI Canvas（原型 v0.1）

对应《BuckyOS AI Canvas PRD》的 P0 纵向闭环原型。桌面内 app id 为 `canvas`，在 `pnpm run dev` 的 mock 桌面（`/?scenario=normal`）第一页可以找到「AI 画布」图标。

## 目录结构

```
canvas/
  CanvasAppPanel.tsx     app 入口：首页 ↔ 编辑器，创建 store / runner
  canvas.css             作用域样式（.aic-*）
  events.ts              本地行为事件（localStorage，不含原始数据与完整 Prompt）
  domain/                领域层，不依赖 React / 画布库
    types.ts             CanvasDocument / Block / Wish / Binding / PresentationPath（PRD §12）
    commands.ts          CanvasCommand 联合类型 + 不进历史的 quiet 命令
    reducer.ts           纯函数 applyCommand；APPLY_AGENT_PATCH 原子应用补丁
    selectors.ts         派生状态：fresh/stale/broken、循环依赖检测、镜头适配
    factories.ts         块工厂、单元格类型推断、表头识别
  agent/                 Agent 协议（PRD §13）
    contracts.ts         AgentRunRequest / AgentRunEvent / CanvasPatch / Adapter 接口
    patch-validator.ts   补丁校验规则（数量、尺寸、引用、用户块保护、循环）
    context.ts           显式上下文快照（只发送用户选择的来源）
    mock.ts              MockCanvasAgentAdapter：确定性输出，支持 #fail/#invalid/#slow/#timeout
    mock-blocks.ts       生成器共用：块工厂 mk()、确定性 hash / rng
    aigc.ts              离线 AIGC 生成器：角色设定图 → 故事板 → 逐帧视频预览（SVG data URL）
    http.ts              HttpCanvasAgentAdapter：POST jobs → SSE events → GET result
    runner.ts            运行状态机：预检 → 请求 → 事件 → 校验（含修订冲突）→ 原子写入 → 历史
  storage/
    indexeddb.ts         IndexedDbCanvasStorage（文档 / 快照 / 反馈）
    export.ts            .aicanvas.json 导入导出（容错、拒绝高版本）
  data/
    csv.ts               CSV/TSV 解析、分隔符与 GBK 回退
    image.ts             图片文件 → data URL（>2048px 或 >1.5MB 时缩放重编码）
    xlsx.ts              无依赖 XLSX 读取（ZIP + DecompressionStream，只读缓存值）
    parse-file.ts        Web Worker 客户端（主线程回退）
  workers/spreadsheet.worker.ts
  fixtures/              季度销售示例、AI 短片工作流示例（aigc-workflow.ts，打开即为已执行状态）
  store/                 CanvasStore（文档 + 撤销栈 + UI 状态 + 自动保存）、settings
  ui/                    React 组件：EditorShell / InfiniteCanvas / BlockView / blocks/*（含 ImageBlock / VideoBlock）/ 面板 / 对话框
```

## 关键约束的落地

- 所有文档修改经 `store.dispatch(command)`；拖拽用 `beginTransient/endTransient` 合并为一条历史。
- Agent 只返回 `CanvasPatch`，经 `validatePatch` 后由 reducer 一次性应用；一次运行 = 一个撤销单位。
- 运行期间画布 revision 变化 → 补丁被拒绝并提示"画布在运行期间已变化"。
- 表格值编辑使 `dataRevision +1`，绑定结果自动变为「需要刷新」；手工修改过的结果在重跑时弹出 替换 / 保留 选项。
- Mock 与 HTTP 适配器共用同一协议；HTTP 不可用时可一键改用 Mock 重试。
- 许愿格可以串联：结果组可作为另一个许愿格的数据来源（上下文里以 `group` 项递归携带成员，图片带 `src`）。上游以"替换"方式重跑时，reducer 会把下游许愿格的 contextRefs、生成块的 sourceRevisions 和绑定重定向到新结果组，并把新组的 dataRevision 抬高，使下游显示"需要刷新"而不是"来源中断"。
- 图片块：工具栏「图片」/ 空白处右键「插入图片」/ 拖入文件 / Ctrl+V 粘贴图片；文件经 `data/image.ts` 缩放后以 data URL 存入文档。视频块目前只作为 Agent 输出（`src` 为真实视频，或 `frames` 逐帧预览）。
- 画布导航：空白处按住右键或中键拖动平移；空白处单击右键弹出插入菜单；Space+左键、Ctrl+滚轮同前。
- Mock Agent 的 AIGC 分支（`agent/aigc.ts`）按数据来源判定阶段：角色表→角色设定图；剧本/角色图→故事板（分镜表 + 每镜画面）；故事板→逐帧视频预览。全部离线、确定性（同输入同输出）。
- 未实现能力（协作、分享、后台任务、交互块沙箱）只在首页"未来能力"中说明，UI 中没有占位按钮。

## 本地验证

`pnpm run dev` 后打开桌面 → AI 画布 → 打开示例 → 运行许愿格。「AI 短片工作流示例」打开即为已执行状态：改动角色表 / 剧本后依次点各阶段的"重新运行"，观察下游"需要刷新"的传播。
`pnpm run check` 与 `pnpm run lint` 覆盖本目录。
