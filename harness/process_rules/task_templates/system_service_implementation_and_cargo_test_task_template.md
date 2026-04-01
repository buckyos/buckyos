# 系统服务实现与 cargo test 任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：实现主体与本地单元测试
- 负责人：

## 任务目标
- 将协议文档和持久数据格式文档映射为服务实现。
- 在进入运行接入前，完成必要单元测试并通过 `cargo test`。

## 输入
- 上一步产物：
  - 已批准的协议文档
  - 已批准的持久数据格式文档
- 相关文档：
  - `harness/Service Dev Loop.md`
- 相关代码：
  - 相似服务实现
  - RDB instance / object 管理 / task manager / keymessage queue / keyevent 相关基础设施
- 约束条件：
  - 结构化数据必须优先走 RDB instance
  - 非结构化数据应优先走 object 管理
  - 长任务场景必须遵循 task executor pattern
  - 未通过 `cargo test` 不得进入 DV Test

## 任务内容
- 实现服务主体、协议解析、数据读写与核心业务逻辑。
- 针对协议编解码、错误码、边界条件、数据格式读写补齐测试。
- 若存在长任务，补齐 task_id、keyevent、timeout 等模式实现。

## 处理流程
### Step 1
- 动作：搭建服务实现骨架，完成协议到内部逻辑的映射。
- Skill：`implement-system-service`
- 输出：可编译的服务主体代码。

### Step 2
- 动作：实现数据访问层、核心逻辑、长任务模式（若适用）。
- Skill：`implement-system-service`
- 输出：主要业务逻辑完成。

### Step 3
- 动作：基于协议文档和数据格式文档补齐单元测试并跑通 `cargo test`。
- Skill：`implement-system-service`
- 输出：测试代码与通过记录。

## 输出产物
- 产物 1：服务实现代码
- 产物 2：单元测试 / 协议编解码测试 / 数据格式测试
- 产物 3：`cargo test` 通过证据

## 如何判断完成
- [ ] 服务主体实现已能覆盖核心业务主路径
- [ ] 协议解析、错误码、边界条件、数据格式读写已被测试覆盖
- [ ] `cargo test` 全部通过

## 下一步进入条件
- [ ] 运行接入所需的服务代码已具备稳定接口
- [ ] 当前实现不存在阻塞 build / scheduler 接入的关键缺口

## 风险 / 注意事项
- 不要为了通过测试临时绕过协议或数据模型约束。
- 单测只应覆盖本地可验证内容，不要把 DV Test 责任塞回单测。

## 允许修改范围
- 可以修改：
  - 服务主体代码
  - 内部数据访问代码
  - 单元测试
  - 与实现强相关的内部模块
- 不可以修改：
  - scheduler / 安装包 / rootfs 接入逻辑
  - TypeScript SDK 正式版本
  - UI 相关实现
