# 本次试跑：逐项八字段记录

以下为本次连续旅程的十个子目标，不是十条新增 PRD 承诺。完整运行环境见 [environment.json](environment.json)。

## S01/a · 创建画布并导入三行资料

| 字段 | 记录 |
|---|---|
| 用例编号 | S01/a · 创建画布并导入三行资料 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 02–11：从桌面进入，创建并命名空白画布；通过文件选择器导入 sales.csv，确认表头后从表格创建许愿格。 |
| 实际结果 | **PASS**。3 行 × 4 列；A/2/10/20、B/3/20/60、C/1/30/30；全表来源标签正确。 |
| 期望结果 | CSV 原值、行列与明确选源一致。 |
| 证据 | [09.json](evidence/09.json)、[10.json](evidence/10.json)、[initial.aicanvas.json](evidence/initial.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现本子项缺陷；相关 TC-001 空白画布创建 / TC-002 导入入口，不代表 XLSX 通过。 |

## S01/b · 销售额独立核算

| 字段 | 记录 |
|---|---|
| 用例编号 | S01/b · 销售额独立核算 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 12–17：使用明确请求销售额总计的 Prompt，以默认自动输出运行 Mock；从 UI 导出结果。 |
| 实际结果 | **PASS**。销售额指标为 110；随后更新 B 后，新版本为 140，旧版本仍为 110。 |
| 期望结果 | 独立手算 20+60+30=110；修改后 20+90+30=140。 |
| 证据 | [15.json](evidence/15.json)、[initial.aicanvas.json](evidence/initial.aicanvas.json)、[after-refresh.aicanvas.json](evidence/after-refresh.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现销售额变体缺陷；只覆盖 TA-N17 图表与指标数值独立核对的一部分。 |

## S01/c · 请求图表的交付结果

| 字段 | 记录 |
|---|---|
| 用例编号 | S01/c · 请求图表的交付结果 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 与 S01/b 同次运行；Prompt 明确要求总计、图表和汇报摘要；检查 UI 和下载文件的块类型。 |
| 实际结果 | **FAIL**。初次结果为 3 指标、1 表、1 摘要，加 1 个组；chart 块数为 0。 |
| 期望结果 | 本次用户目标要求至少一张图表，不能只以运行成功代替交付完整。 |
| 证据 | [15.json](evidence/15.json)、[15.png](evidence/15.png)、[initial.aicanvas.json](evidence/initial.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | BuckyOS 原型 Mock 能力与用户目标不符；不是内核 / Campus 缺陷。源码只在发现区域列时生成销售图，本数据只有产品列。 |

## S01/d · 所选来源与实际输入边界

| 字段 | 记录 |
|---|---|
| 用例编号 | S01/d · 所选来源与实际输入边界 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 11–15：明确选择销售表全表；步骤 13–14 添加未选中的合成文本；检查来源标签和下载文件。 |
| 实际结果 | **INCONCLUSIVE**。UI 和 contextRefs 都只选择 sales；未采集实际 Mock 请求内容，不能证明范围外哨兵没有发送。 |
| 期望结果 | TA-C1 上下文最小化需要实际请求证据，不能用引用列表替代。 |
| 证据 | [11.json](evidence/11.json)、[14.json](evidence/14.json)、[initial.aicanvas.json](evidence/initial.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 本轮可观测性不足；不推断产品或内核缺陷，也不推翻历史版本记录。 |

## S02/a · 取消刷新保留人工修订

| 字段 | 记录 |
|---|---|
| 用例编号 | S02/a · 取消刷新保留人工修订 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 17–20：双击摘要附加人工说明，修改源表 D2 从 60 到 90；导出；点击重新运行并在人工保护对话框取消；再次导出。 |
| 实际结果 | **PASS**。出现需要刷新及替换 / 保留 / 取消选择；取消前后完整文档 JSON 相同，revision=9，9 块、1 绑定。 |
| 期望结果 | 取消不改变任何业务内容，人工说明保留。 |
| 证据 | [19.json](evidence/19.json)、[19.png](evidence/19.png)、[before-refresh.aicanvas.json](evidence/before-refresh.aicanvas.json)、[after-cancel.aicanvas.json](evidence/after-cancel.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现本子项缺陷；对应 TC-005 来源变化、TC-006 人工修改保护的本次分支。 |

## S02/b · 保留旧结果并生成新版本

| 字段 | 记录 |
|---|---|
| 用例编号 | S02/b · 保留旧结果并生成新版本 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 20–21：再次运行，选择保留旧结果；等待运行终态，从 UI 导出。 |
| 实际结果 | **PASS**。新增 6 块（5 个结果 + 1 个组）和 1 绑定；旧用户块和旧结果不变；人工说明保留一次，两个销售额版本为 110 / 140。 |
| 期望结果 | 旧人工版本保留，新版本反映修改后的来源；原有用户内容不被覆盖。 |
| 证据 | [refreshed-ui.txt](evidence/refreshed-ui.txt)、[after-refresh.aicanvas.json](evidence/after-refresh.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现本子项缺陷；未覆盖替换失败、解除 AI 管理或多级结果链。 |

## S02/c · 一次撤销和重做恢复生成结果

| 字段 | 记录 |
|---|---|
| 用例编号 | S02/c · 一次撤销和重做恢复生成结果 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 21：新版本生成后导出；点一次撤销并导出；点一次重做并导出。 |
| 实际结果 | **PASS**。15 块 / 2 绑定 → 9 块 / 1 绑定 → 15 块 / 2 绑定；原有业务对象恢复；重做后与生成后完整文档相同。 |
| 期望结果 | TC-009 撤销生成：整组生成内容及绑定一次移除，重做恢复。运行历史、lastRunId 和当前相机不属于本断言要求回退的对象。 |
| 证据 | [after-refresh.aicanvas.json](evidence/after-refresh.aicanvas.json)、[after-undo.aicanvas.json](evidence/after-undo.aicanvas.json)、[after-redo.aicanvas.json](evidence/after-redo.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现本子项缺陷；不证明复杂手势、多人撤销或历史记录恢复语义。 |

## S03/a · 作者保存并重新打开

| 字段 | 记录 |
|---|---|
| 用例编号 | S03/a · 作者保存并重新打开 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 22–23：等待已保存到本机，刷新桌面，重新进入 Canvas，打开最近文档并导出。 |
| 实际结果 | **PASS**。15 块、2 绑定；与刷新前完整导出逐字段相同，包含活动 Sheet、相机、来源及人工说明。 |
| 期望结果 | 本地重开保持本次文档状态。 |
| 证据 | [22.json](evidence/22.json)、[after-redo.aicanvas.json](evidence/after-redo.aicanvas.json)、[author-reopened.aicanvas.json](evidence/author-reopened.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现本子项缺陷；TC-011 持久化的当前数据变体，未覆盖存储故障与异常退出。 |

## S03/b · 隔离浏览器接收文件

| 字段 | 记录 |
|---|---|
| 用例编号 | S03/b · 隔离浏览器接收文件 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 23–24：新建独立浏览器上下文，确认最近文档为空；从首页正常导入作者导出，再从 UI 导出。 |
| 实际结果 | **PASS**。15 块、2 绑定、旧人工说明、新旧结果及来源关系保留；仅文档 id、updatedAt、metadata 变化，其他顶层字段相同。 |
| 期望结果 | 接收者能独立打开交付文件；导入为新副本允许身份和导入元数据变化，业务内容应保持。 |
| 证据 | [23.json](evidence/23.json)、[24.json](evidence/24.json)、[24.png](evidence/24.png)、[author-reopened.aicanvas.json](evidence/author-reopened.aicanvas.json)、[receiver-imported.aicanvas.json](evidence/receiver-imported.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现本子项缺陷；只验证文件交接，不代表在线共享、媒体自包含或真实权限。 |

## S03/c · 接收者继续修订并保存

| 字段 | 记录 |
|---|---|
| 用例编号 | S03/c · 接收者继续修订并保存 |
| 层 | T-A |
| 环境 | Linux 单机原型；Mock 作者 / 接收者；commit `6c22d201d3cefa115c1a66632484d2effffa2239`；Chromium 148.0.7778.96 / Playwright 1.60.0；1600×1000；sales.csv 全表，详见环境记录。 |
| 复现步骤 | 步骤 25：在保留的人工摘要中追加接收者意见；保存、导出、刷新重开再导出；同时从作者上下文再导出。 |
| 实际结果 | **PASS**。接收者意见和原说明各 1 处；接收方重开前后完整文档相同；作者副本与交接前完整文档相同。 |
| 期望结果 | 接收者能继续工作，修改在自己的副本持久化，不影响作者副本。 |
| 证据 | [25.json](evidence/25.json)、[26.png](evidence/26.png)、[receiver-edited.aicanvas.json](evidence/receiver-edited.aicanvas.json)、[receiver-reopened.aicanvas.json](evidence/receiver-reopened.aicanvas.json)、[author-after-handoff.aicanvas.json](evidence/author-after-handoff.aicanvas.json)、[verification.json](evidence/verification.json) |
| 归属 | 未发现本子项缺陷；两个独立副本不是同一文档的实时协作。 |
