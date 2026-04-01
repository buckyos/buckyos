# WebUI PR 体验收敛任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：UI PR / 产品体验收敛 / DataModel 冻结
- 负责人：

## 任务目标
- 在 Mock 环境下完成主要产品体验问题的集中修正。
- 推动 UI DataModel 从“可用”进入“冻结”状态。

## 输入
- 上一步产物：
  - 可独立运行的 Prototype
  - Playwright 自动化
  - 初版 UI DataModel
- 相关文档：
  - `harness/WebUI Dev Loop.md`
  - PRD
  - UI Design Prompt
- 相关代码：
  - Prototype 页面与组件实现
- 约束条件：
  - 产品体验问题应尽量在 UI PR 阶段完成
  - DataModel 冻结后，结构性改动视为高影响事件

## 任务内容
- 组织版本负责人 / 产品负责人的 Mock 环境体验反馈。
- 修正布局、交互、文案、状态展示与细节体验问题。
- 复核并冻结 UI DataModel。

## 处理流程
### Step 1
- 动作：整理体验评审入口、演示路径与待反馈问题清单。
- Skill：`ui_developer_loop_playwright.md`
- 输出：UI PR 评审清单。

### Step 2
- 动作：根据反馈执行多轮 Developer Loop，修正布局、交互、文案和状态问题。
- Skill：`ui_developer_loop_playwright.md`
- 输出：体验问题修复结果。

### Step 3
- 动作：复核 DataModel，标记冻结边界和高影响变更点。
- Skill：`define_ui_datamodel.md`
- 输出：冻结后的 UI DataModel。

## 输出产物
- 产物 1：UI PR 评审记录
- 产物 2：体验问题修复代码
- 产物 3：冻结后的 UI DataModel 文档

## 如何判断完成
- [ ] 版本负责人已在 Mock 环境体验主路径
- [ ] 主要产品体验问题已被处理
- [ ] UI DataModel 已冻结并标出高影响变更点

## 下一步进入条件
- [ ] 可以进入真实后端集成阶段
- [ ] 后续工作主要聚焦真实数据与性能约束，不再做大规模界面重构

## 风险 / 注意事项
- 如果把体验问题拖到真实后端集成后再处理，修改成本会显著放大。
- DataModel 未冻结就接真实后端，常见结果是前后端同时反复返工。

## 允许修改范围
- 可以修改：
  - 页面、组件和交互代码
  - 文案与状态展示
  - UI DataModel 文档与类型定义
  - Playwright 用例
- 不可以修改：
  - 后端 KRPC 和服务逻辑
  - 为缩短评审时间而删除关键流程或关键状态
  - 与当前 feature 无关的全局 UI 规范
