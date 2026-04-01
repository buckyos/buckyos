# WebUI DataModel × Backend 集成与性能任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：DataModel × Backend 集成 / 性能测试
- 负责人：

## 任务目标
- 让 UI DataModel 在真实后端条件下成立。
- 通过真实或近真实数据验证聚合方式、分页方式、RPC 粒度与性能结论。

## 输入
- 上一步产物：
  - 冻结后的 UI DataModel
  - 可独立运行的 UI Prototype
- 相关文档：
  - `harness/WebUI Dev Loop.md`
  - UI DataModel 文档
  - KRPC 协议说明
- 相关代码：
  - Web SDK / client model
  - 页面数据映射层
  - TypeScript 测试脚本框架
- 约束条件：
  - 集成目标不是“接口接上”，而是“真实条件下 DataModel 成立”
  - 必须用 TypeScript 测试脚本验证规模、分页和性能

## 任务内容
- 实现 UI DataModel 到真实后端数据的映射。
- 编写独立 TypeScript 性能测试脚本，覆盖 1 / 10 / 1000 / 1000000 条数据及随机分页访问。
- 识别读放大、写放大、聚合成本和 RPC 次数/延迟问题，推动前后端 tradeoff。

## 处理流程
### Step 1
- 动作：接入真实 Web SDK / client model，完成 DataModel 映射层。
- Skill：`integrate-ui-datamodel-with-backend`
- 输出：真实后端下的数据映射代码。

### Step 2
- 动作：编写 TypeScript 测试脚本，验证规模、分页、聚合结构和延迟预算。
- Skill：`bechmark-ui-datamodel`
- 输出：性能测试脚本。

### Step 3
- 动作：分析性能结果，提出并落实 UI DataModel 或 KRPC 的必要修正建议。
- Skill：`integrate-ui-datamodel-with-backend`
- 输出：性能结论与前后端收敛结果。

## 输出产物
- 产物 1：真实后端 DataModel 映射代码
- 产物 2：TypeScript 性能测试脚本
- 产物 3：性能测试结论与 tradeoff 记录

## 如何判断完成
- [ ] UI DataModel 能在真实后端下被正确构造
- [ ] 性能测试脚本已执行并产出结论
- [ ] 无明显读放大 / 写放大问题
- [ ] 前后端已对最终模型达成一致

## 下一步进入条件
- [ ] 后续只剩真实链路验证与细节修补
- [ ] UI DV Test 的前置问题已清理完毕

## 风险 / 注意事项
- 如果没有独立性能脚本，读放大和分页问题通常会被 UI 主流程掩盖。
- 此阶段允许提出 DataModel 或 KRPC 调整建议，但不能无记录地随意漂移边界。

## 允许修改范围
- 可以修改：
  - 前端数据映射层
  - UI DataModel 文档与类型定义
  - TypeScript 性能测试脚本
  - 为性能收敛所必需的前后端接口适配代码
- 不可以修改：
  - 与当前集成问题无关的大范围页面重做
  - 无证据地随意扩大数据请求范围
  - 为隐藏性能问题而删除真实场景
