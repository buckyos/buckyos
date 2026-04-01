# WebUI DV Test 任务

## 所属 Feature / 流程阶段
- Feature：
- 阶段：UI DV Test
- 负责人：

## 任务目标
- 在真实系统链路中验证 UI 可正确消费后端数据并完成核心交互。
- 确认浏览器 / Playwright、session token、Web SDK、Gateway、Service 和 UI 渲染链路全部可用。

## 输入
- 上一步产物：
  - 已完成真实后端集成
  - 性能测试通过
- 相关文档：
  - `harness/WebUI Dev Loop.md`
  - PRD
  - UI DataModel 文档
- 相关代码：
  - Web SDK
  - Playwright 用例
  - UI 页面与数据映射层
- 约束条件：
  - 必须验证真实身份、Gateway、SDK、服务与 UI 渲染全链路
  - 不得用 Mock 环境结果代替 DV Test

## 任务内容
- 在真实环境启动 UI，获取 session token，通过 Web SDK 访问真实服务。
- 运行 Playwright 核心用例，验证主路径、数据展示和错误处理。
- 检查 console error、接口失败和渲染异常。

## 处理流程
### Step 1
- 动作：配置真实环境的启动方式、测试身份、token 和 Playwright 入口。
- Skill：`ui-dv-test`
- 输出：DV Test 运行方案。

### Step 2
- 动作：执行真实链路测试，验证主路径、真实数据展示和关键交互。
- Skill：`ui-dv-test`
- 输出：DV Test 运行结果。

### Step 3
- 动作：复核 console、网络请求和渲染状态，补齐失败案例记录。
- Skill：`ui-dv-test`
- 输出：DV Test 证据与问题清单。

## 输出产物
- 产物 1：UI DV Test 运行记录
- 产物 2：Playwright 核心用例结果
- 产物 3：真实链路问题清单与修复建议

## 如何判断完成
- [ ] UI 在真实后端环境下可正常启动
- [ ] 主要用户流程走通
- [ ] 数据展示与预期一致
- [ ] 无关键 console error、接口失败或渲染异常
- [ ] Playwright 核心用例通过

## 下一步进入条件
- [ ] 满足 WebUI 整体 Definition of Done
- [ ] 可进入版本验收、主干合入或发布分支

## 风险 / 注意事项
- 如果真实链路和 Mock 环境行为差异很大，应优先回查 DataModel 映射层和权限链路。
- DV Test 的目标是验证“真实系统能跑通”，不是只验证页面能打开。

## 允许修改范围
- 可以修改：
  - Playwright 用例
  - 前端数据映射与错误处理代码
  - 与真实环境接入直接相关的前端配置
- 不可以修改：
  - 为了绕过问题而把真实请求替换回 Mock
  - 无人工确认地擅自改后端 KRPC 契约
  - 与当前 feature 无关的系统服务逻辑
