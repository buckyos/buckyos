# 系统服务 DV Test 任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：DV Test
- 负责人：

## 任务目标
- 用 TypeScript 走真实客户端链路验证系统服务行为。
- 确认 TS SDK、session token、gateway、权限与服务执行路径全部可用。

## 输入
- 上一步产物：
  - 服务已被 scheduler 拉起
  - login / heartbeat / 日志接入正常
- 相关文档：
  - `harness/Service Dev Loop.md`
  - 协议文档
  - DV 测试环境说明
- 相关代码：
  - TypeScript SDK
  - 现有 DV Test 脚本
  - gateway / session token 获取示例
- 约束条件：
  - DV Test 必须使用 TypeScript
  - 请求必须经过 gateway，不得绕过网关直打服务

## 任务内容
- 为服务接口补齐或 patch TS SDK。
- 编写或更新 DV Test 脚本，覆盖核心接口主路径。
- 验证 session token 获取、gateway 路由、权限检查、服务响应与状态变化。

## 处理流程
### Step 1
- 动作：梳理 DV 主路径，明确测试入口、账号、token、SDK 调用方式。
- Skill：`service-dv-test`
- 输出：DV Test 方案和脚本结构。

### Step 2
- 动作：实现或 patch TS SDK，并编写 TypeScript 测试脚本。
- Skill：`service-dv-test`
- 输出：可执行的 DV Test 脚本。

### Step 3
- 动作：运行 DV Test，记录请求经过 gateway、权限校验和服务返回结果。
- Skill：`service-dv-test`
- 输出：DV Test 结果与问题清单。

## 输出产物
- 产物 1：TypeScript DV Test 脚本
- 产物 2：TS SDK patch 或正式修改草案
- 产物 3：DV Test 运行证据

## 如何判断完成
- [ ] TS SDK 可调用目标接口
- [ ] 测试脚本能获取 session token 并经过 gateway 发起请求
- [ ] 核心接口响应与状态变化符合协议预期

## 下一步进入条件
- [ ] 已能稳定复现主要成功路径和主要失败路径
- [ ] 所有阻塞 Developer Loop 的环境问题已定位

## 风险 / 注意事项
- 通过 Rust 内部调用自证正确，不算 DV Test 完成。
- 若服务需要多个身份角色，脚本中必须明确每种身份的调用边界。

## 允许修改范围
- 可以修改：
  - DV Test 脚本
  - TS SDK patch 或 feature 分支内对应改动
  - 与测试接入相关的轻量配置
- 不可以修改：
  - gateway 基础路径规则
  - 为绕过权限而做的临时后门
  - UI 相关实现
