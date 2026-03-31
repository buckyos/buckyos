# System Service Delivery Checklist

## 1. 文档目标

本文把 System Service Dev Loop.md 的阶段性要求压缩成一份可执行清单，用于贡献者自检、模块负责人过门槛、AI Harness 输出结果。

---

## 2. 无脸系统服务清单

### 2.1 设计阶段

- [ ] 已明确服务边界：做什么 / 不做什么
- [ ] 已有协议文档
- [ ] 已有持久数据格式文档
- [ ] 已区分 Durable Data 与 Disposable Data
- [ ] 已定义 Service Spec / Settings Schema
- [ ] 已说明默认 settings 与资源需求

### 2.2 实现阶段

- [ ] 已复用已有基础设施，而不是重造存储与访问机制
- [ ] 若涉及结构化数据，已使用系统 RDB instance
- [ ] 若核心依赖 filesystem，已在文档中说明理由
- [ ] 若涉及长任务，已使用 task manager / keymessage queue / executor pattern

### 2.3 局部验证

- [ ] `cargo test` 通过
- [ ] 单测覆盖协议解析
- [ ] 单测覆盖数据格式读写
- [ ] 单测覆盖错误码与边界条件

### 2.4 系统接入

- [ ] 已进入 BuckyOS build 目标
- [ ] 构建产物可进入 `rootfs/bin`
- [ ] scheduler 可识别并实例化该服务
- [ ] Service Spec、default settings、Settings Schema 已接入

### 2.5 运行接入

- [ ] 服务进程可启动
- [ ] 日志初始化正常
- [ ] login 成功
- [ ] heartbeat 正常
- [ ] 服务未被标记 unavailable

### 2.6 DV Test

- [ ] 已有 TypeScript SDK 或本地测试 patch
- [ ] TS 测试脚本可运行
- [ ] 能获取 session token
- [ ] 请求走真实 Gateway 路径
- [ ] 核心接口响应正确

### 2.7 Developer Loop 收敛

- [ ] 能自动运行测试
- [ ] 能读取关键日志
- [ ] 能区分覆盖安装与全量重装场景
- [ ] 关键业务测试通过
- [ ] 无关键错误日志
- [ ] 服务稳定运行

### 2.8 CI / 安装包验证

- [ ] 已准备进入 Simple Integration Test
- [ ] 安装包携带该服务
- [ ] 激活流程未被破坏
- [ ] 安装环境中的 DV Test 通过

---

## 3. 带 WebUI 的系统服务附加清单

### 3.1 UI 独立开发

- [ ] `pnpm run dev` 可独立启动
- [ ] UI 不依赖真实后端即可演示主流程
- [ ] 支持独立页面模式
- [ ] 支持模拟桌面窗口模式

### 3.2 PRD 与 Mock-first

- [ ] 已有 PRD 或最小交互草图
- [ ] Mock Data 覆盖主要用户路径
- [ ] Mock Data 覆盖 loading / empty / error / progress 状态
- [ ] Playwright 可跑主路径

### 3.3 UI DataModel

- [ ] 已定义 UI DataModel
- [ ] 已定义字段语义与稳定边界
- [ ] 已定义聚合与分页方式
- [ ] 已定义主要状态模型

### 3.4 UI PR 收敛

- [ ] 产品负责人已在 UI PR 阶段给出体验反馈
- [ ] 主要体验问题已在集成前解决
- [ ] DataModel 已冻结或明确冻结计划

### 3.5 DataModel × Backend 集成

- [ ] 已验证真实后端下 DataModel 可构造
- [ ] 已评估读放大 / 写放大
- [ ] 已评估分页、大列表、聚合性能
- [ ] 前后端已对最终模型达成一致

---

## 4. 结果输出模板

建议在任务收尾时按以下格式输出：

```md
## Delivery Result

- Changed: <改了什么>
- Validation: <跑了什么校验>
- Risks: <仍有哪些风险或未验证项>
- Stage: <当前完成到哪一阶段>
```
