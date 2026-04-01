# WebUI PRD 与设计输入任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：PRD / UI 设计输入
- 负责人：

## 任务目标
- 确认 UI 启动条件已经满足。
- 基于 PRD 形成可执行的 UI 设计输入，作为后续 Prototype 的实现基线。

## 输入
- 上一步产物：
  - 已稳定的后端 KRPC 接口说明
  - 后端运行状态说明（`cargo test`、scheduler、login、heartbeat）
- 相关文档：
  - `harness/WebUI Dev Loop.md`
  - feature proposal / PRD / 最小交互草图
- 相关代码：
  - 现有相似系统服务 WebUI
  - 系统组件库、主题与 i18n 规范
- 约束条件：
  - UI 不得在后端接口高频变动时启动
  - UI 必须遵守系统组件库、双端可用、深浅色主题和国际化要求

## 任务内容
- 检查 UI 启动条件是否满足，并记录缺口。
- 审核或补齐 PRD，明确用户任务、页面、状态、成功/失败路径。
- 产出 UI Design Prompt，明确布局、组件选型和视觉方向。

## 处理流程
### Step 1
- 动作：核对后端 KRPC 稳定性、服务运行状态和 UI 启动前提。
- Skill：`prd-to-ui-design-prompt`
- 输出：启动条件检查结果。

### Step 2
- 动作：整理或补齐 PRD，明确核心页面、交互流程、关键状态与反馈。
- Skill：`prd-to-ui-design-prompt`
- 输出：可评审的 PRD。

### Step 3
- 动作：产出 UI Design Prompt，作为后续 Prototype 的实现与验收输入。
- Skill：`prd-to-ui-design-prompt`
- 输出：UI Design Prompt。

## 输出产物
- 产物 1：PRD
- 产物 2：UI Design Prompt
- 产物 3：UI 启动条件检查记录

## 如何判断完成
- [ ] 后端接口稳定性与运行前提已核对
- [ ] PRD 已覆盖用户任务、页面、状态、成功/失败路径
- [ ] UI Design Prompt 已可直接作为 Prototype 输入

## 下一步进入条件
- [ ] 模块负责人确认允许启动 UI 开发
- [ ] 不存在阻塞建模与 Prototype 的关键需求歧义

## 风险 / 注意事项
- 若 PRD 只写 happy path，会导致后续空态、错误态、loading 态反复返工。
- 不要把 UI Design Prompt 写成纯视觉描述，必须能指导实现。

## 允许修改范围
- 可以修改：
  - PRD
  - UI Design Prompt
  - 与 UI 启动条件说明相关的文档
- 不可以修改：
  - 后端 KRPC 定义
  - 后端服务逻辑
  - 与当前 feature 无关的全局设计规范
