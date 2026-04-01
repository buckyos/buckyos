# 系统服务 SDK 与 Simple Integration Test 任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：SDK 收尾 / Simple Integration Test
- 负责人：

## 任务目标
- 将 DV 阶段使用的 TS SDK 改动正式提交到版本链路。
- 在安装包环境完成 Simple Integration Test，验证发布产物而不是开发产物。

## 输入
- 上一步产物：
  - Developer Loop 已收敛
  - 本地 DV Test 已通过
- 相关文档：
  - `harness/Service Dev Loop.md`
  - CI / 打包 / 安装 / 激活流程说明
- 相关代码：
  - TS SDK
  - 安装包构建脚本
  - 集成测试脚本
- 约束条件：
  - 若有 SDK 改动，必须进入正式版本链路
  - CI DV 测试的是安装包环境，不是开发机产物

## 任务内容
- 清理 DV 阶段的临时 patch，形成正式 TS SDK 修改。
- 在 CI 或等价安装包环境跑 `cargo test`、多平台构建、打包、安装、激活和安装环境 DV Test。
- 验证安装包已携带服务，系统主路径无明显回归。

## 处理流程
### Step 1
- 动作：整理并提交 TS SDK 正式改动，确保接口定义与协议一致。
- Skill：`service-dv-test`
- 输出：正式 SDK 改动。

### Step 2
- 动作：执行安装包环境的 Simple Integration Test 流程。
- Skill：`service-dv-test`
- 输出：构建、安装、激活、安装环境 DV 的测试结果。

### Step 3
- 动作：复核安装包携带结果、主路径回归风险和最终交付状态。
- Skill：`service-dv-test`
- 输出：最终验收记录。

## 输出产物
- 产物 1：TS SDK 正式修改
- 产物 2：Simple Integration Test 结果
- 产物 3：最终交付验收记录

## 如何判断完成
- [ ] TS SDK 改动已进入正式版本链路
- [ ] 安装包正确包含该服务
- [ ] 激活流程通过，安装环境 DV Test 通过
- [ ] 系统主路径无明显回归

## 下一步进入条件
- [ ] 满足系统服务整体 Definition of Done
- [ ] 可进入更高层集成、发布分支或版本验收

## 风险 / 注意事项
- 不要把本地开发机验证结果当作安装包环境验证结果替代。
- 若安装环境失败，要先判断是打包问题、激活问题、SDK 问题还是服务自身问题。

## 允许修改范围
- 可以修改：
  - TS SDK
  - 集成测试脚本
  - 打包、安装、激活流程中的必要接入
  - 与发布产物直接相关的配置
- 不可以修改：
  - 为通过集成测试临时降低系统主路径要求
  - 无关模块的大范围重构
  - UI 相关实现
