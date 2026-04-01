# WebUI Mock-first Prototype 任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：Mock-first Prototype
- 负责人：

## 任务目标
- 在完全脱离真实后端的条件下完成 UI Prototype。
- 让 `pnpm run dev` 独立启动，主路径、关键状态和 Playwright 自动化全部可跑通。

## 输入
- 上一步产物：
  - 已审核的 PRD
  - UI Design Prompt
  - 初版 UI DataModel
- 相关文档：
  - `harness/WebUI Dev Loop.md`
- 相关代码：
  - WebUI 工程骨架
  - 系统组件库
  - 现有 Mock Data / Playwright 配置
- 约束条件：
  - 必须 Mock-first，尽可能晚接入真实后端
  - `pnpm run dev` 必须可独立运行
  - 必须支持独立页面模式与模拟桌面窗口模式

## 任务内容
- 实现 Prototype 页面和主要组件。
- 构造覆盖主路径与关键状态的 Mock Data。
- 编写 Playwright 自动化，驱动 UI Developer Loop 收敛。

## 处理流程
### Step 1
- 动作：搭建 Prototype 页面、组件与视图模式，确保 `pnpm run dev` 可独立启动。
- Skill：`ui_prototype.md`
- 输出：可运行的 Prototype。

### Step 2
- 动作：补齐 Mock Data，覆盖正常态、空态、错误态、加载态、进度态。
- Skill：`ui_prototype.md`
- 输出：Mock Data 与状态演示。

### Step 3
- 动作：编写 Playwright 自动化，并通过截图和自动操作驱动 UI Developer Loop。
- Skill：`ui_developer_loop_playwright.md`
- 输出：可重复运行的 Playwright 用例。

## 输出产物
- 产物 1：Prototype 页面与组件代码
- 产物 2：Mock Data
- 产物 3：Playwright 自动化用例

## 如何判断完成
- [ ] `pnpm run dev` 可独立启动
- [ ] 主路径在 Mock 环境下完整走通
- [ ] 关键状态均有正确呈现
- [ ] Playwright 自动化可跑通

## 下一步进入条件
- [ ] 已具备 UI PR 体验评审条件
- [ ] 初版 DataModel 已在 Prototype 中被验证可用

## 风险 / 注意事项
- 不要在 Prototype 阶段过早接真实接口，否则很快失去低成本迭代窗口。
- Mock Data 只覆盖正常态，会让 UI PR 阶段的问题集中爆发。

## 允许修改范围
- 可以修改：
  - 页面与组件代码
  - Mock Data
  - 前端路由、状态和本地适配逻辑
  - Playwright 用例
- 不可以修改：
  - 后端服务逻辑
  - KRPC 定义
  - 为了绕过问题而删除关键状态或关键交互
