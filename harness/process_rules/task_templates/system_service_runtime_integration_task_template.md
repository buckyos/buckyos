# 系统服务运行接入任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：build / scheduler / 运行接入
- 负责人：

## 任务目标
- 让服务进入 BuckyOS 的真实运行模型。
- 完成构建链路、Service Spec、默认 settings、Settings Schema、login、heartbeat 与日志接入。

## 输入
- 上一步产物：
  - 服务实现完成
  - `cargo test` 通过
- 相关文档：
  - `harness/Service Dev Loop.md`
  - Service Spec / Settings Schema 相关规范
- 相关代码：
  - build 目标定义
  - scheduler 配置和实例化流程
  - 现有系统服务的 startup / login / heartbeat 实现
- 约束条件：
  - 必须通过 `buckyos build`
  - 二进制必须进入 `rootfs/bin`
  - 必须能被 scheduler 识别并启动
  - 服务未 login 成功前不得视为合格系统服务

## 任务内容
- 将服务加入构建链路并验证产物进入安装包。
- 补齐 Service Spec、默认 settings、资源需求与 Settings Schema。
- 打通日志初始化、Service SDK（kAPI）初始化、login、heartbeat。
- 验证 scheduler 可以创建 instance，且服务未被标记 unavailable。

## 处理流程
### Step 1
- 动作：接入 build 流程，确认二进制进入 `rootfs/bin`。
- Skill：`buckyos-intergate-service`
- 输出：构建链路修改与构建验证结果。

### Step 2
- 动作：补齐 Service Spec、默认 settings、资源配置与 Settings Schema。
- Skill：`buckyos-intergate-service`
- 输出：spec / settings / schema 配置与说明。

### Step 3
- 动作：在真实运行环境验证启动、日志、login、heartbeat、scheduler instance 状态。
- Skill：`buckyos-intergate-service`
- 输出：运行接入验证记录。

## 输出产物
- 产物 1：build / 安装包接入修改
- 产物 2：Service Spec / 默认 settings / Settings Schema
- 产物 3：运行接入验证证据

## 如何判断完成
- [ ] `buckyos build` 通过，二进制进入 `rootfs/bin`
- [ ] scheduler 能识别并创建服务 instance
- [ ] 日志初始化、login、heartbeat 均正常
- [ ] 服务未被标记为 unavailable

## 下一步进入条件
- [ ] 已具备 DV Test 的真实运行前提
- [ ] Service Spec / Settings Schema 已文档化并可复现

## 风险 / 注意事项
- 资源配置、settings 默认值和 schema 兼容性错误，会直接导致实例化失败。
- 若运行接入阶段仍频繁改协议边界，说明前置设计任务拆分有问题。

## 允许修改范围
- 可以修改：
  - build 目标与安装包接入代码
  - Service Spec / settings / schema
  - 服务 startup / login / heartbeat / logging 接入代码
- 不可以修改：
  - 已冻结的协议文档边界
  - 与当前服务无关的调度器核心逻辑
  - UI 相关实现
