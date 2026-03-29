# System Service Trigger Rules

## 1. 文档目标

本文从 [System Service Dev Loop.md](/Users/liuzhicong/project/buckyos/harness/System%20Service%20Dev%20Loop.md) 中抽取“哪些资产变更必须触发额外检查”的规则，作为系统服务任务的统一触发与升级审查说明。

---

## 2. 适用范围

当任务满足以下任一条件时，默认适用本文：

- 新增或重构系统服务；
- 修改系统服务协议；
- 修改持久数据格式；
- 修改 Service Spec 或 Settings Schema；
- 修改 UI DataModel；
- 新增或重构带 WebUI 的系统服务。

---

## 3. 触发对象与额外检查

### 3.1 协议文档变更

触发条件：

- KRPC 方法新增、删除、改名；
- 输入 / 输出结构变更；
- 错误码语义变更；
- 幂等性、副作用或状态机说明变更。

必须追加的检查：

- 协议兼容性检查；
- SDK 影响面检查；
- 相关单测补齐；
- 若已接入 UI，则评估 DataModel 影响。

审查重点：

- 是否破坏既有客户端；
- 是否改变调用语义而未同步更新文档和测试。

### 3.2 持久数据格式文档变更

触发条件：

- Durable Data schema 变更；
- version / schema version 变更；
- 存储位置、字段语义、索引、兼容策略变更。

必须追加的检查：

- 数据兼容性检查；
- 升级 / 迁移路径说明；
- 读写测试补齐；
- 明确当前是否仍处于“无需兼容”的开发阶段。

审查重点：

- 是否会破坏覆盖安装；
- 是否会让旧数据失效但没有迁移策略；
- 是否误把可丢弃数据当成持久数据来处理，或反之。

### 3.3 Service Spec / Settings Schema 变更

触发条件：

- service spec 资源需求变更；
- 默认 settings 变更；
- settings schema 新增字段、改字段、删字段；
- scheduler 实例化相关约束变更。

必须追加的检查：

- 配置兼容性检查；
- 实例化验证；
- scheduler 接入验证；
- 文档同步更新默认值、含义与兼容性。

审查重点：

- 是否会导致实例无法创建；
- 是否改变默认行为但未明确说明；
- 是否引入高风险资源需求变更。

### 3.4 UI DataModel 相关 TS 文件变更

触发条件：

- UI DataModel interface 变更；
- 列表聚合、分页、状态模型变更；
- loading / empty / error / progress 等状态变更；
- KRPC Model 到 UI DataModel 的映射逻辑变更。

必须追加的检查：

- 更大范围 UI 自动化测试；
- DataModel 集成测试；
- 性能与读写放大检查；
- 标记为高影响 PR。

审查重点：

- 是否破坏 UI 的稳定边界；
- 是否让后端聚合成本失控；
- 是否在未显式评审的情况下扩大了前后端耦合。

---

## 4. 阶段升级门槛

### 4.1 进入 DV Test 前

系统服务 **MUST** 已满足：

- 协议文档存在；
- 持久数据格式文档存在；
- `cargo test` 通过；
- 服务已进入 build 与 scheduler 链路。

### 4.2 进入 Developer Loop 收敛前

系统服务 **MUST** 已满足：

- 服务能启动；
- login 成功；
- heartbeat 正常；
- 日志可观测；
- DV Test 主路径已跑通。

### 4.3 进入 Simple Integration Test 前

系统服务 **MUST** 已满足：

- 本地 Developer Loop 已收敛；
- 若有 SDK 改动，已准备纳入正式链路；
- 若有 UI，已满足 UI 集成前置条件。

---

## 5. 变更分级建议

### 5.1 低风险

- 文档补充但不改语义；
- mock data 扩展但不改 DataModel 稳定边界；
- 测试新增且不改对外契约。

### 5.2 中风险

- 新增协议字段但保持兼容；
- settings schema 增量扩展；
- UI DataModel 新增可选字段。

### 5.3 高风险

- 协议破坏性变更；
- 持久数据格式破坏性变更；
- Settings 默认行为改动；
- DataModel 冻结后仍做结构性修改；
- 导致 scheduler 资源配置或实例化逻辑变化的改动。

---

## 6. 结果输出要求

凡触发本文规则的 PR，结果说明中 **SHOULD** 明确写出：

- 变更触发了哪类检查；
- 实际跑了哪些验证；
- 哪些高成本验证尚未执行；
- 仍存在哪些兼容性或集成风险。
