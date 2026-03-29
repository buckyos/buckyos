# BuckyOS Harness Documentation Map

## 1. 文档目标

`harness/` 目录用于承载 BuckyOS 在 AI / Human 协作开发中的流程、规则、模板与检查清单。

这里的文档分四类：

- **总纲文档**：定义整体流程和职责边界；
- **规则文档**：定义准入、触发、审查与模式声明；
- **模板文档**：给出可直接复用的 PR / 设计文档骨架；
- **检查清单**：把长文中的阶段性 Done 条件压缩为可执行清单。

---

## 2. 当前 canonical 文档

### 2.1 总纲

- [Harness Engineering.md](/Users/liuzhicong/project/buckyos/harness/Harness%20Engineering.md)
  - 仓库级 Harness Engineering 总流程；
  - 定义角色、Developer Loop、立项、PR 模式、模块分级等基础规则。

- [System Service Dev Loop.md](/Users/liuzhicong/project/buckyos/harness/System%20Service%20Dev%20Loop.md)
  - 系统服务任务模板；
  - 定义协议、持久数据、调度接入、DV Test、UI DataModel 与集成流程。

### 2.2 规则

- [PR Contribution Modes.md](/Users/liuzhicong/project/buckyos/harness/rules/PR%20Contribution%20Modes.md)
- [Module Tier and Admission Matrix.md](/Users/liuzhicong/project/buckyos/harness/rules/Module%20Tier%20and%20Admission%20Matrix.md)
- [System Service Trigger Rules.md](/Users/liuzhicong/project/buckyos/harness/rules/System%20Service%20Trigger%20Rules.md)

### 2.3 模板

- [System Service Design PR Template.md](/Users/liuzhicong/project/buckyos/harness/templates/System%20Service%20Design%20PR%20Template.md)
- [System Service UI PR Template.md](/Users/liuzhicong/project/buckyos/harness/templates/System%20Service%20UI%20PR%20Template.md)

### 2.4 检查清单

- [System Service Delivery Checklist.md](/Users/liuzhicong/project/buckyos/harness/checklists/System%20Service%20Delivery%20Checklist.md)

---

## 3. 使用顺序建议

### 3.1 当你在定义仓库级工程规则

按以下顺序阅读：

1. [Harness Engineering.md](/Users/liuzhicong/project/buckyos/harness/Harness%20Engineering.md)
2. [PR Contribution Modes.md](/Users/liuzhicong/project/buckyos/harness/rules/PR%20Contribution%20Modes.md)
3. [Module Tier and Admission Matrix.md](/Users/liuzhicong/project/buckyos/harness/rules/Module%20Tier%20and%20Admission%20Matrix.md)

### 3.2 当你在启动一个系统服务任务

按以下顺序阅读：

1. [System Service Dev Loop.md](/Users/liuzhicong/project/buckyos/harness/System%20Service%20Dev%20Loop.md)
2. [System Service Design PR Template.md](/Users/liuzhicong/project/buckyos/harness/templates/System%20Service%20Design%20PR%20Template.md)
3. [System Service Trigger Rules.md](/Users/liuzhicong/project/buckyos/harness/rules/System%20Service%20Trigger%20Rules.md)
4. [System Service Delivery Checklist.md](/Users/liuzhicong/project/buckyos/harness/checklists/System%20Service%20Delivery%20Checklist.md)

### 3.3 当任务包含 WebUI

额外补读：

1. [System Service UI PR Template.md](/Users/liuzhicong/project/buckyos/harness/templates/System%20Service%20UI%20PR%20Template.md)
2. [System Service Trigger Rules.md](/Users/liuzhicong/project/buckyos/harness/rules/System%20Service%20Trigger%20Rules.md)

---

## 4. 维护原则

- 长文负责定义原则、边界和完整流程；
- 子文档负责把长文拆成可引用、可复用、可执行的工程资产；
- 若子文档与长文冲突，以两份总纲文档为准；
- 新增任务模板时，优先放到 `templates/` 或 `checklists/`，避免继续把所有规则堆进单一长文。
