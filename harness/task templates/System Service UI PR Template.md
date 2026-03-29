# System Service UI PR Template

## 1. 模板目标

本模板用于“带 WebUI 的系统服务”设计与实现阶段的文档整理。它建立在 [System Service Design PR Template.md](/Users/liuzhicong/project/buckyos/harness/templates/System%20Service%20Design%20PR%20Template.md) 之上，补充 UI、DataModel、Mock-first 与集成性能要求。

---

## 2. 适用场景

- 新增带 WebUI 的系统服务；
- 已有系统服务新增 WebUI；
- UI DataModel 需要结构性重做；
- 前后端要围绕 DataModel 做联合收敛。

---

## 3. 模板正文

```md
# <Service Name> UI PR Template

## 1. PRD / 交互目标

- 主要用户任务：
- 关键场景：
- 核心页面：
- 成功路径：
- 失败路径：
- loading / empty / error / progress 等状态：

## 2. UI 启动边界

- KRPC 是否已经基本稳定：
- 当前 UI 可以从哪个阶段启动：
- 哪些后端能力尚未冻结：

## 3. 独立运行要求

- `pnpm run dev` 是否可独立运行：
- 是否不依赖真实服务：
- 是否支持独立页面模式：
- 是否支持模拟桌面窗口模式：

## 4. Mock-first 方案

- 关键 Mock Data 列表：
- 覆盖的用户路径：
- 覆盖的组件状态：
- 哪些路径可由 Playwright 自动执行：

## 5. UI DataModel

### 5.1 数据结构

- TS interface：
- 字段语义：
- 聚合方式：
- 分页方式：

### 5.2 状态模型

- loading：
- empty：
- error：
- progress：
- optimistic / pending（若有）：

### 5.3 稳定边界

- 哪些字段是 UI 稳定边界：
- 哪些字段只是实现细节：
- 哪些变更会被视为高影响改动：

## 6. UI 自动化

- Playwright 主路径：
- 截图基线：
- 自动判断及格线的标准：
- 当前不能自动化的体验项：

## 7. UI PR 阶段的人审重点

- 产品负责人需要重点体验什么：
- 哪些体验问题必须在 UI PR 阶段解决：
- DataModel 预计何时冻结：

## 8. DataModel × Backend 集成

- 第一版 UI 驱动 DataModel：
- 第二版系统驱动 DataModel：
- 后端聚合成本：
- 可能的读放大 / 写放大：
- 预期需要修正的 KRPC 点：

## 9. 性能测试

- 1 / 10 / 1000 / 1000000 条数据下的验证计划：
- 分页随机访问验证：
- RPC 次数与延迟预算：
- UI 可接受性标准：

## 10. Done 前置条件

- [ ] UI 可独立运行
- [ ] Mock Data 覆盖主路径
- [ ] Playwright 自动验证可运行
- [ ] DataModel 已定义
- [ ] UI PR 阶段体验问题已收敛
- [ ] DataModel 已冻结或明确冻结计划
- [ ] Backend 集成风险已评估
```

---

## 4. 使用要求

- UI 不应直接把 KRPC 原始结构当成长期稳定边界；
- Mock-first 阶段 **SHOULD** 尽量晚接真实后端；
- DataModel 冻结后，任何结构性变更都 **SHOULD** 被标记为高影响改动并触发更大范围测试。
