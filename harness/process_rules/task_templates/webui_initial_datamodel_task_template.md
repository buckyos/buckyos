# WebUI 初版 DataModel 任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：初版 UI DataModel 定义
- 负责人：

## 任务目标
- 定义 UI 层消费的第一版稳定数据边界。
- 明确 UI DataModel、状态模型和展示组织方式，不直接绑定 KRPC 原始结构。

## 输入
- 上一步产物：
  - 已审核的 PRD
  - UI Design Prompt
- 相关文档：
  - `harness/WebUI Dev Loop.md`
- 相关代码：
  - 现有相似页面的数据模型
  - 相关 KRPC / client model 定义
- 约束条件：
  - UI DataModel 不等同于 KRPC Model
  - 第一版 DataModel 由 UI 实现需求驱动

## 任务内容
- 设计 TypeScript interface 级别的 UI DataModel。
- 定义字段语义、聚合方式、分页方式和状态模型。
- 区分稳定边界字段与实现细节字段。

## 处理流程
### Step 1
- 动作：从 PRD 和页面结构反推 UI 所需的最小数据边界。
- Skill：`define_ui_datamodel.md`
- 输出：字段草图与页面到数据的映射。

### Step 2
- 动作：定义 UI DataModel、分页/聚合方式以及 loading / empty / error / progress 等状态。
- Skill：`define_ui_datamodel.md`
- 输出：UI DataModel 文档初稿。

### Step 3
- 动作：标记稳定边界、高影响变更点和暂不承诺的实现细节。
- Skill：`define_ui_datamodel.md`
- 输出：可用于 Prototype 的 DataModel 文档。

## 输出产物
- 产物 1：UI DataModel 文档
- 产物 2：TypeScript interface 草案
- 产物 3：字段稳定性说明

## 如何判断完成
- [ ] UI DataModel 已覆盖主要页面和关键状态
- [ ] 聚合方式、分页方式和字段语义已明确
- [ ] 稳定边界字段与实现细节字段已区分

## 下一步进入条件
- [ ] Prototype 可基于该模型直接构造 Mock Data
- [ ] 不存在阻塞 Mock-first 开发的数据建模分歧

## 风险 / 注意事项
- 如果直接把 KRPC 原始对象塞进 UI，会在集成阶段放大返工成本。
- 状态模型不清楚时，Prototype 往往只能做正常态演示，后续补状态代价很高。

## 允许修改范围
- 可以修改：
  - UI DataModel 文档
  - DataModel 相关 TS 类型定义
  - 与页面数据组织相关的说明文档
- 不可以修改：
  - 后端 KRPC 定义
  - 后端服务逻辑
  - 与当前任务无关的页面实现
