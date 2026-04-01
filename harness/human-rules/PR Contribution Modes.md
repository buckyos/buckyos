# PR Contribution Modes

## 1. 文档目标

本文从 [Harness Engineering.md](/Users/liuzhicong/project/buckyos/harness/Harness%20Engineering.md) 中抽取 PR 贡献模式声明机制，作为 reviewer、贡献者、AI Harness 都可直接引用的独立规则文档。

目标不是评价“哪种模式更高级”，而是让审查者快速知道：

- 代码主要由谁生成；
- 需要重点审什么；
- Prompt / 过程记录需要提交到什么程度；
- 哪些模块根本不接受某些模式。

---

## 2. 总规则

所有 PR **MUST** 声明自身的贡献模式。

模式声明的用途是：

- 帮助 reviewer 选择正确的风险模型；
- 帮助模块负责人判断是否满足准入规则；
- 帮助贡献者知道自己最低要提交哪些过程证据。

若 PR 未声明模式，则视为信息不完整，**SHOULD NOT** 进入正式审查。

---

## 3. 模式定义

### 3.1 模式一：纯人工编写

定义：

- 代码逐行主要由人编写；
- AI 最多用于测试、检查、局部建议；
- AI 不是主要代码生产者。

最小要求：

- 正常代码审查；
- 正常测试证据；
- 不强制提交完整 prompt history。

审查重点：

- 代码质量；
- 测试完整性；
- 是否满足模块验收标准。

### 3.2 模式二：Human-Agent Loop

定义：

- 人主导实现；
- AI 参与局部生成、修改、补全、测试或诊断；
- 最终方案主要由人把控。

最小要求：

- 提交关键 prompt；
- 说明主要迭代过程；
- 让 reviewer 看得出 AI 参与了哪些部分。

审查重点：

- 关键设计判断是否由人完成；
- AI 生成内容是否引入隐藏假设；
- 关键 prompt 是否足以解释实现来源。

### 3.3 模式三：Agent-Human Loop

定义：

- 人主要提供意图、任务边界和修正反馈；
- 代码主要由 Agent 生成；
- 人负责选择、修正和验收生成结果。

最小要求：

- 提供核心 prompt history；
- 说明任务输入文档来源；
- 说明主要迭代与修正轨迹；
- 附带测试与验证证据。

审查重点：

- 上游文档是否足够可靠；
- Prompt 链路是否覆盖关键决策；
- 测试是否足以覆盖 Agent 可能引入的系统性错误；
- 是否越过模块准入边界。

### 3.4 模式四：保留项

当前总纲中已预留第四类模式，但尚未正式冻结定义。

在补齐前：

- **MUST NOT** 自行发明新的模式名称并写入正式规则；
- 若确有特殊类型 PR，**SHOULD** 暂按最接近的前三种模式处理，并在说明中补充其特殊性。

---

## 4. 模式声明建议格式

PR 描述中建议至少包含以下字段：

```md
## Contribution Mode

- Mode: Human-Agent Loop
- Main Authoring Path: human-led implementation with AI-assisted testing and patch generation
- Prompt Evidence: attached / summarized in PR
- Validation: cargo test / pnpm build / DV test / manual check
```

若为 Agent-Human Loop，建议额外补充：

```md
## Agent Inputs

- Approved design docs
- Module rules
- Constraints / forbidden solutions
- Test targets and acceptance criteria
```

---

## 5. reviewer 快速判断矩阵

### 5.1 看到模式一时

优先看：

- 代码本身；
- 测试本身；
- 是否符合模块经验规则。

### 5.2 看到模式二时

除代码外，还要看：

- 关键 prompt；
- 是否有明显“AI 代替了人做架构决策”的迹象；
- 测试是否覆盖 AI 参与的区域。

### 5.3 看到模式三时

优先看：

- 输入文档是否可靠；
- Prompt history 是否能解释关键决策；
- 测试证据是否足够强；
- 模块是否允许该模式。

---

## 6. 与模块准入的关系

模式声明不等于模式自动获准。

最终是否允许采用某种模式，仍由模块分级规则决定。具体准入规则见 [Module Tier and Admission Matrix.md](/Users/liuzhicong/project/buckyos/harness/rules/Module%20Tier%20and%20Admission%20Matrix.md)。
