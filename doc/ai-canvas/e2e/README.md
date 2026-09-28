# AI Canvas E2E 验收文档

本目录是 BuckyOS Desktop AI Canvas 通用 E2E 需求的维护入口。2026-09-28 从原应用侧测试文档
迁入；后续修改通用原型验收直接改本目录，不在下游应用仓库保留第二份规范。

先读场景，再让 Agent 对着当前 UI 完成任务；需要精确判定时查附录。

1. [协作场景主文档](test-requirements.md)：七条完整旅程，说明参与者的问题、系统解决思路、可观察结果及今天的能力边界。
2. [浏览器执行约定](execution-guide.md)：Agent 通过 Playwright 等工具观察、操作和留证；首条可跑的资料 → 修订 → 文件交接任务包。
3. [细粒度验收与追溯附录](assertions.md)：保留四组 PRD 验收、15 条 TC、3 条数据边界补充、23 条原型子断言、可观测性与八字段记录，逐条映射到场景。
4. [覆盖缺口附录](coverage-gaps.md)：33 个待补设计（12 P0 / 21 P1）和六组交叉条件；当前执行范围和未完成项见附录状态说明。

[产品 PRD](../../../src/frame/desktop/src/app/canvas/BuckyOS%20AI%20Canvas%20PRD.md) 与
[原型 README](../../../src/frame/desktop/src/app/canvas/README.md) 是产品范围和当前实现入口。
UI 改版时更新当次操作轨迹，不随意改变场景的成功标准。2026-09-28 的整理只调整文档组织，
当时没有新增执行结果；后续试跑见下文。文件交接不等于在线共享，双窗口竞争不等于真正多人协作通过。

课程、作业、教师站、学生权限等行业业务验收由应用仓库维护。本目录不包含这些应用设计、
账号 / 节点配置、私有接口或未核实的内核源纪要。T-B / T-C 必须另行满足接口与业务前置。

## 当前执行

[2026-09-28：S01–S03 浏览器连续旅程](runs/2026-09-28/browser-journey/README.md) 已归档。十个子目标 8 PASS / 1 FAIL /
1 INCONCLUSIVE；图表请求未交付，实际请求边界证据不足。风险附录 TA-N17 仅执行了销售额变体，
其余分支未完成；不能据此宣告整个场景或风险用例通过。

## 历史执行与迁移边界

2026-09-24 的阶段 1 为 15 PASS / 6 FAIL / 1 INCONCLUSIVE，阶段 2 为 19 PASS / 4 FAIL / 0 INCONCLUSIVE。
原始日志、导出、截图、脚本和 manifest 仍在原执行仓库归档；本次迁移需求，没有迁移或改写历史证据。
以下仅解释覆盖文档沿用的历史问题编号，不能作为当前版本复跑结论或替代原始证据。

| 历史编号 | 含义与关联用例 |
|---|---|
| CANVAS-L01 | XLSX worker 的 DOMParser 错误；TC-002 / PRD-21.3 |
| CANVAS-L02 | 大表内滚轮改变画布相机；TC-014 / PRD-21.3 |
| CANVAS-L03 | 只有输入修订标记，没有可恢复的当次输入快照；TA-C3 |
| CANVAS-L04 | 内部阶段事件与用户可见绘制进度不一致；TC-004 / PRD-21.1 |
| CANVAS-L05 | Mock 结果表数量与 PRD 口径不一致；PRD-21.1，规格待澄清 |
| CANVAS-L06 | createBinding 接受两向循环；TA-V11，未证明运行时死循环 |
| CANVAS-L07 | create 后 update 绕过文本上限；TA-V12 |
| CANVAS-L08 | createGroup 修改用户组成员；TA-V13 |
| CANVAS-L09 | 未知块数据保留但块体缺少回退说明；TA-F1 |
| CANVAS-G01 | 浏览器 TSV 夹具已覆盖，真实 Excel 原应用复制未覆盖；TC-003 |

这些编号的迁入说明不代表问题已修复，也不新增缺陷计数。Canvas 开发阶段新发现先记入本仓库
执行记录，不自动创建或重开 GitHub issue。新执行记录必须写清被测版本并按 [附录 §8](assertions.md#8-结果记录与问题归属) 保留证据。
