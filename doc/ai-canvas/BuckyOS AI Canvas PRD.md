# BuckyOS AI Canvas PRD

> 面向 CodeAgent 的可执行原型开发说明  
> 文档版本：v0.1  
> 产品阶段：概念验证 / 可交互原型  
> 目标读者：产品、设计、CodeAgent、前后端工程师、首轮体验用户  
> 首轮体验用户：熟悉 Excel、经常用表格解决工作问题，但不从事软件开发的人  
> 更新日期：2026-09-03

---

## 0. 文档目的

本 PRD 用于指导 CodeAgent 完成一个可以真实运行、可供非开发用户试用并收集反馈的 **BuckyOS AI Canvas 原型**。

原型不是 Excel、Notion 或 PowerPoint 的完整替代品，也不是把聊天机器人嵌入白板。它需要验证一个更具体的产品假设：

> 当普通用户能够在类似 Excel 的工作环境中，直接用自然语言描述目标，并由 Agent 将目标转化为可编辑、可持续更新的内容块时，是否会形成比“聊天后复制答案”更自然的新型生产力工作流？

首版必须形成一条完整、连贯、可演示的主链路：

1. 用户创建或打开一张画布；
2. 导入一份 Excel/CSV 数据，或使用示例数据；
3. 在画布上创建“许愿格”；
4. 用自然语言描述需要的分析、内容或界面；
5. Agent 读取用户指定的数据和画布上下文；
6. Agent 生成表格、图表、文字或交互组件；
7. 结果作为画布中的可编辑对象保留下来；
8. 原始数据变化后，结果被标记为“需要刷新”，用户可以重新运行；
9. 用户把画布中的若干区域编排成讲述路径并播放；
10. 用户完成一组反馈问题。

CodeAgent 应以本 PRD 为产品与工程边界，不要在首版自行扩展成完整在线办公套件。

---

## 1. 产品定义

### 1.1 一句话定义

**BuckyOS AI Canvas 是一张可本地部署的 AI Native 无限画布。用户可以像使用 Excel 一样组织数据和内容，并在任意位置用自然语言“许愿”，让 Agent 将意图变成可编辑、可复用、可持续运行的内容或工具。**

### 1.2 用户容易理解的类比

> Excel 的自由组织能力  
> + Notion 的内容与协作能力  
> + PowerPoint 的讲述顺序  
> + Agent 的理解、生成和执行能力

这个类比仅用于降低理解门槛。产品在实现上不应简单拼接四套软件的界面。

### 1.3 核心产品判断

BuckyOS AI Canvas 采用以下判断作为产品基础：

1. **AI 时代的办公入口应从“无限画布”出发，而不是先要求用户选择 Word、Excel 或 PowerPoint。**
2. **Prompt 可以成为普通用户可理解的新型公式。** 用户表达意图，Agent 负责生成底层处理逻辑。
3. **Agent 不应每次从空白 HTML 开始重新开发完整应用。** BuckyOS 先提供稳定的画布、对象、权限、版本、数据绑定和运行能力，Agent 在高阶底座上生成。
4. **AI 生成的结果不是聊天记录，而是画布中的一等对象。** 它可以继续编辑、引用、刷新、评论、发布和形成版本。
5. **Excel 与 PowerPoint 的主要区别可以抽象为浏览方式。** Excel 偏自由探索，PowerPoint 偏固定讲述；同一张画布应同时支持两者。
6. **本地优先不是断网运行的同义词。** 用户拥有自己的文档和数据边界，同时可以选择连接在线模型、数据源和内容网络。

### 1.4 产品不是以下形态

- 不是在传统电子表格右侧增加一个 AI 聊天栏；
- 不是让用户每次通过聊天生成一张无法继续维护的静态图片；
- 不是任意代码无约束运行的网页生成器；
- 不是完整复刻 Microsoft Office 文件格式和全部功能；
- 不是首版即实现多人实时协作、内容交易和完整社交网络。

---

## 2. 首轮要验证的产品假设

| 编号 | 假设 | 原型中的验证方式 |
|---|---|---|
| H1 | 熟悉 Excel 的用户能在 5 分钟内理解“许愿格” | 首次引导后，让用户自行从数据生成结果 |
| H2 | 用户愿意用自然语言代替部分公式、宏和手工整理 | 观察用户是否能独立写出有效需求 |
| H3 | 将 AI 结果保留为可编辑对象，比聊天后复制答案更自然 | 让用户继续修改、移动、刷新生成结果 |
| H4 | “数据来源、最后运行时间、刷新状态”能提升用户信任 | 对比只显示结果和显示来源状态时的反馈 |
| H5 | 同一画布兼具自由浏览和讲述路径，能替代部分 Excel→PPT 搬运 | 让用户将分析结果直接编排为演示 |
| H6 | 用户接受“Agent 在稳定底座上生成”，而不是每次生成一套独立网页 | 观察用户对统一交互、可编辑性和稳定性的评价 |
| H7 | 本地部署、持续更新和可发布对象具有长期价值 | 访谈用户愿意将哪些工作长期放在画布中 |

原型不是为了证明所有假设成立，而是尽快暴露理解障碍、信任障碍和使用边界。

---

## 3. 目标用户

### 3.1 首轮核心用户

熟悉 Excel 的非开发用户，包括但不限于：

- 业务负责人；
- 财务、行政、人力资源和教务人员；
- 咨询、研究和运营人员；
- 教师和培训人员；
- 中小团队管理者；
- 经常用 Excel 搭建临时台账、分析表或小系统的人。

### 3.2 用户共同特征

- 能理解表格、行列、Sheet、筛选、图表等概念；
- 知道自己想解决什么业务问题；
- 不一定会复杂公式、VBA、SQL 或前端开发；
- 经常在 Excel、Word、PowerPoint、邮件和聊天工具间复制内容；
- 对 AI 有兴趣，但不希望所有工作都退化为聊天；
- 对数据错误、黑盒生成和自动修改存在顾虑。

### 3.3 首轮不重点服务的用户

- 需要完整财务建模与高精度 Excel 公式兼容的专业模型师；
- 依赖 VBA、复杂插件和企业级 Excel 宏系统的用户；
- 主要需求是矢量绘图、专业排版或视频剪辑的用户；
- 需要超大规模数据仓库分析的专业数据工程团队；
- 只需要一次性对话问答的用户。

---

## 4. 用户任务与典型场景

### 4.1 核心 Job To Be Done

> 当我手里有数据、材料或一个尚未成形的工作目标时，我希望在一个自由的工作空间里直接说明我想得到什么，让 AI 帮我形成可继续修改和持续更新的结果，而不必先学编程、反复复制内容或另外开发一套系统。

### 4.2 首版主场景：季度经营分析

用户拥有一份销售数据表，字段包括日期、区域、产品、销售额、成本、目标和负责人。

用户希望：

1. 找出增长与下滑最明显的区域；
2. 计算毛利率和目标完成率；
3. 生成管理层可读的摘要；
4. 生成图表；
5. 将结果直接排成一条汇报路径；
6. 原始数据变化后快速刷新结果。

建议示例 Prompt：

> 按区域和产品分析本季度销售表现，计算销售额、毛利率和目标完成率，找出增长最快和下滑最明显的三项，并生成一段适合管理层阅读的总结。请同时生成一张区域对比图和一张异常明细表。

### 4.3 次场景：课程画布

教师在一张大画布上组织知识框架、公式、案例、实验和练习，并定义讲述顺序。学生既可以跟随教师视角，也可以自由浏览；教师可以让指定学生在某个答题格中填写内容。

首版只需要通过一个预置模板和演示数据表达这一方向，不要求实现真实课堂、多用户同步和学生身份权限。

### 4.4 次场景：持续更新的同行跟踪

用户建立一张同行研究画布，数据源按计划更新，相关表格、图表和结论被重新生成。

首版只实现“手动刷新”和“页面打开时的模拟定时刷新”；真正的长期后台调度由 BuckyOS Scheduler 后续接入。

---

## 5. 设计原则

### 5.1 先像生产力工具，再像 AI 产品

界面主角是画布、表格和内容，不是聊天窗口。AI 出现在用户正在工作的单元格或内容块中。

### 5.2 直接操作与自然语言并存

用户既可以拖动、编辑、复制、粘贴，也可以通过 Prompt 生成。不要强迫用户用语言完成所有微小操作。

### 5.3 结果必须可见、可改、可追溯

每个 AI 结果至少显示：

- 由哪个许愿格生成；
- 使用了哪些数据来源；
- 最后运行时间；
- 当前是否过期；
- 重新运行、查看过程、复制和解除绑定的入口。

### 5.4 用户始终可撤销

Agent 对画布的修改必须通过统一命令和事务执行，一次运行可以整体撤销。不得由 Agent 直接操作 DOM 或绕过文档状态。

### 5.5 高阶底座优先于重复生成

常见表格、图表、文本、指标卡、表单等使用宿主提供的标准组件。只有无法表达的特殊界面，才使用受限的自定义 HTML 组件。

### 5.6 空白画布不能成为门槛

首次打开应提供示例画布、模板和明确的第一步，不让用户面对无提示的空白空间。

### 5.7 不伪装已经实现的能力

原型中不得放置无响应的“分享”“多人协作”“发布收费”等按钮。未实现能力只在产品说明或“未来能力”中展示。

---

## 6. 原型范围

### 6.1 P0：必须完成的纵向闭环

1. 本地创建、重命名、保存和打开画布；
2. 每张画布包含多个 Sheet，每个 Sheet 是无限画布；
3. 在画布上创建、选择、移动、缩放、复制、删除内容块；
4. 支持文本块、表格块、许愿格、图表块、指标块、结果组和框架块；
5. 支持导入 CSV 和 XLSX 的值数据；
6. 支持从 Excel 直接复制并粘贴二维数据；
7. 许愿格可以选择一个或多个数据来源并运行；
8. Agent 以流式状态展示分析过程；
9. Agent 返回结构化 `CanvasPatch`，生成文字、表格、指标和图表；
10. 结果与来源建立依赖关系；来源变化时结果显示“需要刷新”；
11. 支持重新运行、保留旧版本、替换结果和解除 AI 管理；
12. 支持撤销/重做；
13. 支持定义讲述路径并全屏播放；
14. 支持导出和导入 `.aicanvas.json`；
15. 提供 Mock Agent，保证无模型服务时仍可完整演示；
16. 提供标准 HTTP Agent Adapter，可接 BuckyOS Agent 服务；
17. 提供首轮用户反馈表和本地行为事件记录。

### 6.2 P0.5：时间允许时完成

- 自定义 HTML 交互块及安全沙箱；
- 块级评论；
- 页面打开期间的定时刷新；
- Markdown 文本块；
- 生成结果的简单差异查看；
- 画布缩略图和小地图；
- 将表格块导出为 CSV；
- 基础模板中心。

### 6.3 P1 及以后

- BuckyOS DID 身份、群组和细粒度权限；
- 多人实时协作与光标同步；
- 离线编辑后的冲突合并；
- BuckyOS Object ID、不可变版本和 Name 持续发布；
- 内容订阅、购买和更新授权；
- 网络数据连接器、企业数据连接器和网页研究 Agent；
- 真正的后台定时任务；
- 学生答题权限和教师跟随视角；
- 完整评论、任务、@和通知系统；
- Office 文档高保真导入导出；
- 模板与组件市场；
- 移动端编辑；
- 多 Agent 工作流编排。

### 6.4 明确不做

- 不实现完整 Excel 公式引擎；
- 不实现 VBA；
- 不保证 XLSX 样式、公式、宏和图表原样往返；
- 不允许 Agent 直接执行未经校验的文件系统、Shell 或网络写操作；
- 不实现 PowerPoint 像素级兼容；
- 不把所有内容都生成成独立网页；
- 不在首版引入复杂组织管理后台。

---

## 7. 核心概念与术语

### 7.1 Canvas Document / 画布文档

用户长期保存和分享的顶层工作成果。一个画布文档包含多个 Sheet、内容块、依赖关系和讲述路径。

### 7.2 Sheet / 工作页

类似 Excel 的 Sheet，但每个 Sheet 是一个可无限平移和缩放的空间，而不是只能由规则网格组成。

### 7.3 Block / 内容块

画布中的基本对象。首版支持：

- `text`：文本；
- `table`：二维表格；
- `wish`：许愿格；
- `metric`：指标卡；
- `chart`：图表；
- `frame`：区域框架；
- `group`：结果组或普通分组；
- `interactive`：受限交互组件，P0.5。

### 7.4 Table Cell / 表格单元格

表格块内部的普通数据单元。首版支持文字、数字、日期、布尔值和空值。

### 7.5 Wish Cell / 许愿格

保存用户意图、上下文引用、运行状态和更新规则的 AI 内容块。

首版提供两种入口：

1. **独立许愿格**：放在画布任意位置，可生成复杂结果组；
2. **表格内 AI 单元格**：从表格单元格转换，只生成标量、文字或二维数据；复杂结果自动在表格右侧创建关联结果组。

### 7.6 Result Group / 结果组

一次 Agent 运行产生的一组块，例如三张指标卡、一张图、一个明细表和一段摘要。结果组与许愿格关联，可整体刷新、替换、复制或解除管理。

### 7.7 Binding / 依赖关系

表示一个结果依赖某个表格、范围、文件或其他块。来源修订号发生变化时，结果被标记为 `stale`。

### 7.8 Presentation Path / 讲述路径

作者定义的画布浏览顺序。每一步保存目标区域、镜头位置、标题和说明。播放时相机在画布中移动和缩放。

### 7.9 CanvasPatch / 画布补丁

Agent 对画布提出的结构化修改计划。客户端校验后以事务方式应用。Agent 不直接修改页面。

---

## 8. 信息架构与页面布局

### 8.1 页面一：本地画布首页

首版首页包含：

- 最近打开的画布；
- “新建空白画布”；
- “从 Excel/CSV 开始”；
- “打开季度经营分析示例”；
- 导入 `.aicanvas.json`；
- 原型说明和反馈入口。

首页不要求账号登录。

### 8.2 页面二：画布编辑器

推荐布局：

```text
┌──────────────────────────────────────────────────────────────────┐
│ 返回  画布标题  保存状态     撤销/重做    运行模式    播放  反馈 │
├──────────────┬─────────────────────────────────────┬─────────────┤
│ Sheet 列表   │                                     │ 属性面板    │
│ 数据/附件    │           无限画布主区域             │ 来源/运行   │
│ 讲述路径     │                                     │ 样式/历史   │
│              │                                     │             │
├──────────────┴─────────────────────────────────────┴─────────────┤
│ 添加块：文本 / 表格 / 许愿格 / 框架     缩放比例 / 适应内容      │
└──────────────────────────────────────────────────────────────────┘
```

### 8.3 左侧栏

通过 Tab 切换：

1. **Sheet**：新建、重命名、排序、删除；
2. **数据**：已导入文件、表格块和可引用范围；
3. **讲述路径**：步骤列表、新建路径、排序和播放。

### 8.4 右侧属性面板

根据选择对象显示不同内容：

- 普通块：标题、位置、尺寸、锁定、复制、删除；
- 表格块：数据概况、范围、导出、来源；
- 许愿格：Prompt、上下文、输出偏好、运行方式、历史；
- AI 结果：来源、最后运行、状态、重新运行、解除绑定；
- 讲述步骤：标题、说明、镜头范围、切换时长。

### 8.5 画布操作约定

- 单击选择；
- 双击进入内容编辑；
- 拖动块标题或边缘移动；
- 拖动控制点缩放；
- `Space + 拖动` 平移画布；
- 触控板或 `Ctrl/Cmd + 滚轮` 缩放；
- 框选多选；
- `Ctrl/Cmd + C/V` 复制粘贴块；
- `Delete/Backspace` 删除选中块；
- `Ctrl/Cmd + Z` 撤销；
- `Ctrl/Cmd + Shift + Z` 重做；
- `Ctrl/Cmd + Enter` 运行当前许愿格；
- `Esc` 退出编辑或取消选择；
- `F` 适应全部内容；
- `P` 创建许愿格，但输入框聚焦时不触发。

---

## 9. 首次使用引导

首次打开示例画布时显示三步轻引导：

1. **这里是你的数据**：高亮示例销售表；
2. **在许愿格里直接写目标**：高亮预置 Prompt；
3. **按运行，让结果留在画布上**：高亮运行按钮。

引导可跳过，且只显示一次。

示例画布预置：

- 一张名为“原始销售数据”的表格块；
- 一个尚未运行的许愿格；
- 一个“运行后结果将出现在这里”的浅色框架；
- 一条空的讲述路径；
- 一个“完成体验后请反馈”的入口。

不要用长篇弹窗解释概念。用户应通过操作理解。

---

## 10. 核心用户流程

### 10.1 流程 A：从 Excel 生成分析结果

1. 用户从首页选择“从 Excel/CSV 开始”；
2. 用户选择文件；
3. 系统解析 Workbook，并让用户选择工作表；
4. 系统在新画布中创建表格块；
5. 用户选中表格块，点击“基于此数据创建许愿格”；
6. 许愿格自动引用该表格；
7. 用户输入 Prompt，选择“自动决定输出”；
8. 用户点击“运行”；
9. 系统显示步骤：理解目标 → 检查数据 → 生成结构 → 渲染结果；
10. Agent 返回 `CanvasPatch`；
11. 客户端校验并在许愿格右侧创建结果组；
12. 用户编辑标题、移动图表或修改 Prompt；
13. 用户保存。

### 10.2 流程 B：修改来源并刷新

1. 用户修改表格中的一个或多个值；
2. 表格块修订号增加；
3. 所有关联结果显示“数据已变化，需要刷新”；
4. 用户点击结果组的“刷新”；
5. 如果结果组包含用户手工修改，弹出选项：
   - 替换当前结果；
   - 保留当前结果并生成新版本；
   - 取消；
6. Agent 重新运行；
7. 结果状态恢复为“最新”。

### 10.3 流程 C：将分析变成汇报

1. 用户创建或选择一个讲述路径；
2. 用户框选标题与结论区，点击“加入讲述路径”；
3. 用户依次加入指标区、图表区和异常明细区；
4. 用户在左侧拖动调整步骤顺序；
5. 点击“播放”；
6. 系统全屏，按步骤移动镜头；
7. 用户可临时自由移动查看其他区域；
8. 点击“返回当前步骤”恢复演示；
9. `Esc` 退出播放。

### 10.4 流程 D：将 AI 结果转为普通内容

1. 用户选中 AI 生成块或结果组；
2. 点击“解除 AI 管理”；
3. 系统说明解除后不会被原许愿格自动刷新；
4. 用户确认；
5. 生成元数据和依赖关系被移除，内容保留并可自由编辑。

### 10.5 流程 E：收集首轮反馈

1. 用户点击顶栏“反馈”；
2. 系统显示 7 个问题；
3. 用户提交后数据保存在本地；
4. 用户可下载匿名反馈 JSON；
5. 默认不包含画布内容、Prompt 全文和导入数据，只包含用户主动填写的反馈和行为统计。

---

## 11. 功能需求

### 11.1 画布文档与本地保存

#### FR-DOC-001 新建画布

- 用户可以创建空白画布；
- 默认包含一个名为“Sheet 1”的工作页；
- 自动生成本地 UUID；
- 默认标题为“未命名画布”。

#### FR-DOC-002 自动保存

- 所有文档修改在 800ms 无新操作后写入 IndexedDB；
- 顶栏显示“正在保存 / 已保存 / 保存失败”；
- 关闭页面前若仍有未保存修改，触发浏览器离开提示；
- 保存失败不应静默。

#### FR-DOC-003 导入导出

- 可以导出 `.aicanvas.json`；
- 可以重新导入并恢复全部 P0 数据；
- 导入时校验 `schemaVersion`；
- 对未知字段应保留或忽略，但不得导致崩溃；
- 不支持的高版本文档显示明确错误。

#### FR-DOC-004 命名快照

- 用户可以创建命名快照；
- 快照保存当前文档完整状态；
- 用户可以预览并恢复；
- 恢复动作本身可撤销。

### 11.2 Sheet

#### FR-SHEET-001 管理工作页

- 新建、重命名、排序、复制、删除；
- 至少保留一个 Sheet；
- 每个 Sheet 保存独立相机位置；
- 切换 Sheet 不丢失未完成编辑。

### 11.3 无限画布

#### FR-CANVAS-001 相机

- 支持平移、缩放、适应全部内容、回到 100%；
- 缩放范围建议为 10%–400%；
- 缩放以鼠标指针所在位置为中心；
- 切换 Sheet 后恢复上次相机位置。

#### FR-CANVAS-002 内容块操作

- 创建、选择、多选、移动、缩放、复制、粘贴、删除、置顶、置底；
- 支持对齐参考线或最小网格吸附；
- 多选移动必须保持相对位置；
- 块可以锁定位置；
- 文本编辑状态下拖动不得误移动块。

#### FR-CANVAS-003 框架块

- 框架用于视觉分区和讲述路径目标；
- 可以设置标题；
- 移动框架时可选择是否连同内部块移动；
- 首版不要求严格的嵌套布局引擎。

#### FR-CANVAS-004 连接关系

- 许愿格与生成结果之间显示一条可选连接线；
- 默认仅在选中相关块时显示，避免画布过乱；
- 连接线不可作为自由流程图编辑器使用。

### 11.4 文本块

#### FR-TEXT-001 文本编辑

- 双击进入编辑；
- 支持标题、正文、列表、加粗、斜体和链接；
- P0 可使用简化富文本或 Markdown；
- 支持粘贴纯文本；
- 不要求 Word 级排版。

### 11.5 表格块与数据导入

#### FR-TABLE-001 创建表格

- 用户可创建空表格；
- 默认 10 行 × 5 列；
- 支持增删行列；
- 支持编辑表头和单元格；
- 支持矩形区域选择；
- 支持复制粘贴 TSV 数据。

#### FR-TABLE-002 文件导入

- 支持 `.csv`、`.xlsx`；
- XLSX 导入时显示工作表选择器；
- 首版导入单元格的“当前值”，不执行 VBA，不保证公式重算；
- 对公式单元格优先读取缓存值；没有缓存值时显示为空并给出提示；
- 自动识别第一行是否为表头，允许用户修正；
- 导入后展示行数、列数和被截断情况。

#### FR-TABLE-003 数据类型

- 支持 `string`、`number`、`date`、`boolean`、`null`；
- 显示值与内部值分离；
- 日期解析不确定时保留原文本并标记警告；
- 不在原型中自动修改用户源数据。

#### FR-TABLE-004 表格限制

建议首版工程限制：

- 单文件最大 20MB；
- 单表最多 20,000 行；
- 单表最多 100 列；
- 超限时允许用户选择前 N 行或取消；
- 大表必须使用虚拟滚动，解析放入 Web Worker；
- 不得因大文件阻塞主线程超过可感知时间。

#### FR-TABLE-005 数据修订

- 每次编辑表格值，表格 `dataRevision` 增加；
- 与该表格或范围绑定的结果被标记为 `stale`；
- 仅修改位置或外观不增加 `dataRevision`。

### 11.6 表格内 AI 单元格

#### FR-AICELL-001 转换入口

- 表格单元格右键菜单或快捷命令提供“转为 AI 单元格”；
- AI 单元格显示清晰的 AI 标识，但仍属于表格；
- 用户可以恢复为普通单元格。

#### FR-AICELL-002 输入与引用

- 用户可以在编辑器中写 Prompt；
- 默认上下文为同表当前区域或用户选中范围；
- 支持显示类似 `当前表!A1:F20` 的可读引用；
- 首版不要求用户手写复杂引用语法。

#### FR-AICELL-003 输出

- 标量或短文本写回当前单元格；
- 二维数据可从当前单元格向右下扩展；
- 若会覆盖非空单元格，必须提示；
- 图表、长文或交互内容在表格右侧生成关联结果组。

### 11.7 独立许愿格

#### FR-WISH-001 创建

用户可以通过以下方式创建：

- 底部工具栏“许愿格”；
- 快捷键 `P`；
- 选中表格后点击“基于此数据创建许愿格”；
- 画布空白处输入 `/AI`。

#### FR-WISH-002 内容

许愿格至少包含：

- Prompt 输入区；
- 数据来源列表；
- 输出偏好；
- 运行按钮；
- 运行状态；
- 最后运行时间；
- 生成结果数量；
- 刷新策略；
- 历史运行入口。

#### FR-WISH-003 数据来源

用户可以添加：

- 一个或多个表格块；
- 表格中的选定范围；
- 文本块；
- 已生成的结果块；
- 当前 Sheet 的用户选中内容。

首版不默认把整张画布全部发送给 Agent。必须让用户看见当前上下文。

#### FR-WISH-004 输出偏好

提供四个简单选项：

- 自动决定；
- 生成表格；
- 生成图表与指标；
- 生成汇报摘要。

这是软约束，Agent 可返回警告并补充其他必要块。

#### FR-WISH-005 运行前校验

以下情况禁止直接运行：

- Prompt 为空；
- 引用的来源已删除；
- 数据超过当前 Agent Adapter 允许的大小；
- Agent 服务不可用且 Mock 模式关闭。

错误必须在许愿格内就地说明。

### 11.8 Agent 运行

#### FR-AGENT-001 状态机

```text
idle
  → planning
  → running
  → validating
  → applying
  → succeeded

任一运行状态可进入 failed 或 cancelled。
需要额外授权时可进入 waiting_permission。
```

#### FR-AGENT-002 过程反馈

运行期间显示面向普通用户的状态，而不是底层日志：

- 正在理解目标；
- 正在检查数据；
- 正在生成分析结构；
- 正在创建图表；
- 正在写入画布。

允许展开“技术详情”，查看原始事件和耗时。

#### FR-AGENT-003 取消与超时

- 运行中允许取消；
- 默认超时建议 120 秒；
- 取消后不得应用未完成补丁；
- 已产生的临时结果必须清理；
- 超时保留可复制的错误信息和重试入口。

#### FR-AGENT-004 原子应用

- Agent 返回的所有操作先在内存中校验；
- 校验通过后一次性应用；
- 任一操作失败时全部回滚；
- 一次 Agent 运行在撤销栈中表现为一个动作。

#### FR-AGENT-005 Mock Agent

Mock Agent 是 P0 必须能力，而非临时代码。

它至少支持：

1. 季度经营分析示例；
2. 对任意数值表生成基础汇总；
3. 对文本块生成摘要；
4. 固定演示失败、取消和过期状态。

Mock 输出必须与真实 Agent 使用同一 `CanvasPatch` 协议。

#### FR-AGENT-006 Real Agent Adapter

实现可配置 HTTP Adapter：

- 基础地址由设置页配置；
- 文档中不保存密钥；
- 支持普通请求和 SSE 流式事件；
- Adapter 不直接依赖具体模型厂商；
- 失败时可以一键切换到 Mock 模式继续体验。

### 11.9 生成结果

#### FR-RESULT-001 标准块优先

Agent 优先返回宿主标准块：

- `metric`；
- `text`；
- `table`；
- `chart`；
- `group`。

只有标准块无法表达需求时，才返回 `interactive`。

#### FR-RESULT-002 结果组头部

结果组显示：

- 标题；
- “AI 生成”标识；
- 最后运行时间；
- 当前状态：最新 / 需要刷新 / 运行失败；
- 重新运行；
- 查看来源；
- 解除 AI 管理；
- 更多：复制为新版本、删除。

#### FR-RESULT-003 可编辑性

- 文本、表格标题、图表标题可直接编辑；
- 手工修改后结果组标记 `userModified=true`；
- 重新运行时不得无提示覆盖手工修改；
- 用户可以选择“替换”“生成副本”或“取消”。

#### FR-RESULT-004 来源与假设

每次结果保存：

- 来源块 ID 和来源修订号；
- 用户 Prompt；
- Agent Adapter 名称；
- 执行时间；
- Agent 返回的假设、警告和来源说明；
- 不要求首版保存模型完整思维过程。

### 11.10 图表块

#### FR-CHART-001 声明式图表

图表使用宿主渲染，不让 Agent 直接生成图表 DOM。

首版支持：

- 柱状图；
- 折线图；
- 饼图或环形图；
- 横向条形图。

图表 Spec 至少包括：

- 数据来源；
- X 轴字段；
- Y 轴字段；
- 系列字段；
- 聚合方式；
- 标题；
- 排序；
- 数值格式。

#### FR-CHART-002 编辑

右侧面板允许用户修改字段映射、图表类型和标题，不需要重新运行 Agent。

### 11.11 自定义交互块（P0.5）

#### FR-HTML-001 沙箱

- 使用独立 `iframe`；
- 默认 `sandbox="allow-scripts"`，不启用 `allow-same-origin`；
- 禁止顶层导航、弹窗、下载和直接访问宿主 DOM；
- 默认禁止网络访问；
- 与宿主仅通过 `postMessage` SDK 通信；
- HTML、CSS、JS 分字段保存；
- 应用前限制体积并校验 Manifest；
- 任意异常不得导致整个编辑器崩溃。

#### FR-HTML-002 宿主 SDK

最小 SDK：

```ts
interface CanvasWidgetSDK {
  getInput(name: string): Promise<unknown>;
  emitOutput(name: string, value: unknown): void;
  requestResize(height: number): void;
  reportError(message: string): void;
}
```

自定义交互块不是 Agent 的默认输出。

### 11.12 依赖与刷新

#### FR-BIND-001 状态判断

- 来源修订号与上次运行记录一致：`fresh`；
- 任一来源修订号变化：`stale`；
- 来源被删除：`broken`；
- 从未成功运行：`never_run`。

#### FR-BIND-002 刷新策略

首版支持：

- 手动；
- 来源变化后提醒；
- 来源变化后自动运行，默认关闭；
- 固定间隔，仅页面打开时生效，P0.5。

UI 必须明确说明：原型中的定时刷新在页面关闭后不会继续执行。

#### FR-BIND-003 防止循环

若 A 依赖 B、B 又依赖 A，客户端拒绝创建并显示循环依赖错误。首版至少实现有向图环检测。

### 11.13 撤销、重做与历史

#### FR-HISTORY-001 命令化修改

所有文档修改统一经过 Command 层，至少包括：

- 创建/删除块；
- 移动/缩放；
- 编辑内容；
- Agent Patch；
- Sheet 操作；
- 讲述路径操作；
- 恢复快照。

#### FR-HISTORY-002 运行历史

许愿格保存最近 10 次运行摘要：

- 时间；
- 状态；
- Prompt 摘要；
- 来源修订；
- 结果组 ID；
- 错误摘要。

### 11.14 讲述路径

#### FR-PRESENT-001 创建路径

- 一个画布可以有多条讲述路径；
- 每条路径有名称和步骤数组；
- 可以从当前视口、选中块或框架创建步骤；
- 步骤可排序和删除。

#### FR-PRESENT-002 播放

- 全屏或沉浸模式；
- 左右键切换；
- 显示当前步骤/总步骤；
- 镜头平滑移动；
- 允许临时自由浏览；
- 提供“返回当前步骤”；
- 支持减少动态效果的系统设置。

#### FR-PRESENT-003 首版边界

- 不实现复杂动画时间线；
- 不实现块逐字出现；
- 不导出 PPTX；
- 不实现远程观众实时跟随。

### 11.15 评论（P0.5）

- 可对块创建评论线程；
- 使用本地昵称；
- 支持回复和解决；
- 不实现 @、通知和实时同步；
- 评论不进入 Agent 上下文，除非用户明确选择。

### 11.16 反馈与测试数据

#### FR-FEEDBACK-001 反馈题目

1. 用你自己的话，这个产品是做什么的？
2. 你最想用它解决哪一项真实工作？
3. “许愿格”是否容易理解？1–5 分。
4. 你更愿意把它看成 Excel、Notion、PPT、白板还是其他？为什么？
5. 哪一部分让你最不放心？
6. 哪些自动更新可以让 AI 直接做，哪些必须先确认？
7. 体验后，你是否愿意把某项长期工作放在这张画布上？

#### FR-FEEDBACK-002 行为事件

本地记录：

- `canvas_created`；
- `sample_opened`；
- `file_imported`；
- `wish_created`；
- `wish_run_started`；
- `wish_run_succeeded`；
- `wish_run_failed`；
- `result_edited`；
- `result_refreshed`；
- `presentation_created`；
- `presentation_played`；
- `undo_used`；
- `feedback_submitted`。

默认事件不保存用户原始数据和完整 Prompt。

---

## 12. 数据模型

以下类型用于说明领域模型，CodeAgent 可按实际工程调整，但不得改变核心语义。

### 12.1 顶层文档

```ts
interface CanvasDocument {
  schemaVersion: "0.1";
  id: string;
  title: string;
  revision: number;
  activeSheetId: string;
  sheets: CanvasSheet[];
  blocks: Record<string, CanvasBlock>;
  bindings: DataBinding[];
  presentationPaths: PresentationPath[];
  comments: CommentThread[];
  createdAt: string;
  updatedAt: string;
  metadata: {
    ownerDid?: string;
    sourceTemplateId?: string;
    importedFrom?: string;
  };
}

interface CanvasSheet {
  id: string;
  name: string;
  order: number;
  blockIds: string[];
  camera: {
    x: number;
    y: number;
    zoom: number;
  };
}
```

### 12.2 通用块

```ts
type BlockType =
  | "text"
  | "table"
  | "wish"
  | "metric"
  | "chart"
  | "frame"
  | "group"
  | "interactive";

interface CanvasBlock<T = unknown> {
  id: string;
  sheetId: string;
  type: BlockType;
  title?: string;
  rect: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  zIndex: number;
  locked: boolean;
  contentRevision: number;
  dataRevision: number;
  content: T;
  generated?: GeneratedMeta;
  createdAt: string;
  updatedAt: string;
}
```

### 12.3 表格块

```ts
type CellPrimitive = string | number | boolean | null;

type TableCell =
  | {
      kind: "value";
      value: CellPrimitive;
      displayValue?: string;
      valueType: "string" | "number" | "date" | "boolean" | "null";
    }
  | {
      kind: "ai";
      wishId: string;
      displayValue?: string;
    };

interface TableBlockContent {
  columns: Array<{
    id: string;
    name: string;
    width?: number;
    inferredType?: string;
  }>;
  rows: Array<{
    id: string;
    cells: Record<string, TableCell>;
  }>;
  source?: {
    kind: "manual" | "csv" | "xlsx";
    filename?: string;
    worksheet?: string;
    importedAt?: string;
  };
}
```

### 12.4 许愿格

```ts
interface WishBlockContent {
  prompt: string;
  contextRefs: ContextRef[];
  outputPreference: "auto" | "table" | "visual" | "brief";
  refreshPolicy: {
    mode: "manual" | "notify_on_change" | "on_change" | "interval";
    intervalMinutes?: number;
  };
  state:
    | "idle"
    | "planning"
    | "waiting_permission"
    | "running"
    | "validating"
    | "applying"
    | "succeeded"
    | "failed"
    | "cancelled";
  lastRunId?: string;
  generatedGroupIds: string[];
  runHistory: WishRunSummary[];
}

type ContextRef =
  | { kind: "block"; blockId: string; revision: number }
  | {
      kind: "tableRange";
      blockId: string;
      range: { rowStart: number; rowEnd: number; colStart: number; colEnd: number };
      revision: number;
    };
```

### 12.5 生成信息与依赖

```ts
interface GeneratedMeta {
  runId: string;
  wishBlockId: string;
  agentAdapter: string;
  generatedAt: string;
  sourceRevisions: Array<{ refKey: string; revision: number }>;
  status: "fresh" | "stale" | "broken";
  userModified: boolean;
  detached: boolean;
  assumptions?: string[];
  warnings?: string[];
}

interface DataBinding {
  id: string;
  source: ContextRef;
  targetBlockId: string;
  createdByRunId: string;
}
```

### 12.6 图表 Spec

```ts
interface ChartBlockContent {
  chartType: "bar" | "line" | "pie" | "horizontalBar";
  data:
    | { kind: "inline"; rows: Record<string, CellPrimitive>[] }
    | { kind: "tableBlock"; blockId: string };
  xField?: string;
  yFields?: string[];
  seriesField?: string;
  aggregation?: "sum" | "avg" | "count" | "min" | "max";
  sort?: { field: string; direction: "asc" | "desc" };
  numberFormat?: "plain" | "percent" | "currency";
  caption?: string;
}
```

### 12.7 讲述路径

```ts
interface PresentationPath {
  id: string;
  name: string;
  steps: PresentationStep[];
  createdAt: string;
  updatedAt: string;
}

interface PresentationStep {
  id: string;
  title?: string;
  note?: string;
  camera: { x: number; y: number; zoom: number };
  targetBlockIds: string[];
  transitionMs: number;
}
```

---

## 13. Agent 协议

### 13.1 设计约束

1. Agent 读取的是显式上下文快照，不是完整浏览器状态；
2. Agent 返回结构化补丁，不直接执行画布 UI 操作；
3. 所有补丁必须通过 JSON Schema 校验；
4. 操作数量、输出体积和块类型有白名单限制；
5. 应用补丁前检查基准文档修订，防止覆盖用户刚刚完成的修改；
6. Agent 生成失败不能破坏现有画布。

### 13.2 请求结构

```ts
interface AgentRunRequest {
  protocolVersion: "0.1";
  runId: string;
  canvas: {
    id: string;
    revision: number;
    locale: "zh-CN" | string;
  };
  wish: {
    blockId: string;
    prompt: string;
    outputPreference: "auto" | "table" | "visual" | "brief";
  };
  context: AgentContextItem[];
  destination: {
    sheetId: string;
    anchor: { x: number; y: number };
    maxWidth: number;
  };
  capabilities: Array<
    | "read_canvas_context"
    | "create_standard_blocks"
    | "create_interactive_block"
  >;
}
```

`AgentContextItem` 应是经过序列化、尺寸限制和脱敏处理的数据快照。

### 13.3 HTTP API 建议

```text
GET    /api/agent/health
POST   /api/agent/jobs
GET    /api/agent/jobs/:jobId/events     # SSE
GET    /api/agent/jobs/:jobId/result
POST   /api/agent/jobs/:jobId/cancel
```

`POST /api/agent/jobs` 返回：

```json
{
  "jobId": "job_xxx",
  "status": "accepted"
}
```

SSE 事件建议：

```text
event: status
data: {"stage":"planning","message":"正在理解目标"}

event: progress
data: {"stage":"running","percent":45,"message":"正在汇总区域数据"}

event: warning
data: {"message":"两行数据缺少区域字段"}

event: completed
data: {"jobId":"job_xxx"}
```

### 13.4 CanvasPatch

```ts
interface CanvasPatch {
  protocolVersion: "0.1";
  runId: string;
  baseCanvasRevision: number;
  summary: string;
  assumptions: string[];
  warnings: string[];
  operations: CanvasPatchOperation[];
}

type CanvasPatchOperation =
  | { op: "createBlock"; block: CanvasBlock }
  | { op: "updateBlock"; blockId: string; patch: Partial<CanvasBlock> }
  | { op: "createBinding"; binding: DataBinding }
  | { op: "createGroup"; groupId: string; childBlockIds: string[] }
  | { op: "resizeToFit"; blockId: string }
  | { op: "addPresentationStep"; pathId: string; step: PresentationStep };
```

P0 不允许 Agent 返回：

- 删除用户已有块；
- 修改用户未选中的源数据；
- 修改 Sheet 权限；
- 执行系统命令；
- 访问未授权网络；
- 覆盖与 `baseCanvasRevision` 不一致的文档。

### 13.5 补丁校验规则

- `operations.length <= 50`；
- 单次最多创建 20 个可见块；
- 块坐标和尺寸必须为有限数；
- 不得创建小于最小可操作尺寸的块；
- 所有引用 ID 必须存在或在同一补丁中先创建；
- 表格输出限制在 5,000 个单元格以内；
- 文本块单块不超过 50,000 字符；
- 自定义 HTML、CSS、JS 分别限制体积；
- 禁止循环依赖；
- 补丁校验失败时展示可读错误，不应用任何操作。

### 13.6 Mock Agent 的确定性行为

对预置季度销售数据，Mock Agent 应固定生成：

1. “本季度销售额”指标；
2. “整体毛利率”指标；
3. “目标完成率”指标；
4. 区域销售额柱状图；
5. 产品毛利率表；
6. 三段式管理摘要；
7. 异常数据警告。

同一输入与 Prompt 应生成稳定输出，便于视觉回归测试。

---

## 14. BuckyOS 集成边界

原型应采用 Adapter/Port 结构，避免以后接入 BuckyOS 时重写领域层。

### 14.1 Storage Adapter

```ts
interface CanvasStorageAdapter {
  list(): Promise<Array<{ id: string; title: string; updatedAt: string }>>;
  load(id: string): Promise<CanvasDocument>;
  save(doc: CanvasDocument, expectedRevision?: number): Promise<void>;
  delete(id: string): Promise<void>;
  createSnapshot(doc: CanvasDocument, name: string): Promise<string>;
}
```

首版：`IndexedDbCanvasStorage`。  
后续：`BuckyObjectCanvasStorage`。

### 14.2 Agent Adapter

```ts
interface CanvasAgentAdapter {
  id: string;
  health(): Promise<{ available: boolean; message?: string }>;
  run(
    request: AgentRunRequest,
    onEvent: (event: AgentRunEvent) => void,
    signal: AbortSignal
  ): Promise<CanvasPatch>;
}
```

首版实现：

- `MockCanvasAgentAdapter`；
- `HttpCanvasAgentAdapter`。

### 14.3 Scheduler Adapter

```ts
interface CanvasSchedulerAdapter {
  schedule(task: ScheduledWishTask): Promise<string>;
  cancel(taskId: string): Promise<void>;
  list(): Promise<ScheduledWishTask[]>;
}
```

首版：浏览器打开期间的 `BrowserTimerScheduler`。  
后续：BuckyOS 系统 Scheduler。

### 14.4 Identity 与 Share Adapter

首版仅使用本地用户和本地文件交换，不展示虚假的在线分享能力。

后续映射：

- 用户与群组 → DID；
- 画布稳定入口 → Name；
- 某次不可变快照 → Object ID；
- 附件与引用 → ObjectRef；
- 持续更新发布 → Name 指向最新 Object ID；
- 固定售卖版本 → 指定 Object ID；
- 持续订阅版本 → 对 Name 的访问授权。

### 14.5 原型与最终对象模型的对应

| 原型 | BuckyOS 最终语义 |
|---|---|
| 本地 UUID | Object ID 的开发期替代 |
| 命名快照 | 不可变对象版本 |
| 画布标题 | Name 的显示名，不等同于稳定 Name |
| IndexedDB 附件 | ObjectRef 指向的对象 |
| 本地用户 | DID Identity |
| 导出 JSON | 跨环境传输的临时方式 |

---

## 15. 前端工程建议

以下是推荐实现，不是必须绑定的框架。CodeAgent 若调整，应保持领域模型、协议和验收标准不变。

### 15.1 推荐技术结构

- Web SPA：React + TypeScript；
- 构建：Vite；
- 状态：领域 Reducer/Command Bus，可配合轻量状态容器；
- 本地存储：IndexedDB；
- 大文件解析：Web Worker；
- 表格：支持虚拟滚动的 Grid 组件或自研轻量实现；
- 图表：通过统一 `ChartRenderer` Adapter；
- 无限画布：成熟画布库或基于 CSS Transform 的独立相机层；
- Schema 校验：JSON Schema；
- 测试：单元测试 + 组件测试 + 端到端测试；
- 可选服务端：BuckyOS Service 或轻量 Node 服务实现 Agent Proxy。

### 15.2 推荐目录

```text
src/
  app/
    App.tsx
    routes.tsx
  domain/
    canvas/
      types.ts
      commands.ts
      reducer.ts
      selectors.ts
      validation.ts
    agent/
      contracts.ts
      patch-validator.ts
    presentation/
      types.ts
  features/
    home/
    canvas-editor/
    sheet-nav/
    block-renderer/
    table-block/
    wish-block/
    agent-run/
    result-group/
    presentation/
    feedback/
  adapters/
    storage/
      indexeddb.ts
    agent/
      mock.ts
      http.ts
    scheduler/
      browser-timer.ts
    charts/
      host-chart-renderer.ts
  workers/
    spreadsheet.worker.ts
  fixtures/
    quarterly-sales.ts
    sample-canvas.ts
  tests/
server/                     # 若需要 HTTP Agent Proxy
  routes/
  services/
```

### 15.3 状态与画布库解耦

画布库只负责：

- 命中检测；
- 拖动缩放；
- 相机；
- 选择框；
- 渲染容器。

业务文档状态必须保存在领域层。不得把唯一数据源藏在画布库内部对象中。

### 15.4 Command Bus

推荐所有修改均表示为命令：

```ts
type CanvasCommand =
  | { type: "CREATE_BLOCK"; payload: CanvasBlock }
  | { type: "MOVE_BLOCKS"; payload: { ids: string[]; dx: number; dy: number } }
  | { type: "UPDATE_BLOCK_CONTENT"; payload: { id: string; content: unknown } }
  | { type: "APPLY_AGENT_PATCH"; payload: CanvasPatch }
  | { type: "RESTORE_SNAPSHOT"; payload: CanvasDocument };
```

每个命令应产生 inverse command 或前状态快照，确保撤销可靠。

### 15.5 性能建议

- 仅渲染可见区域或使用分层虚拟化；
- 拖动时避免持续写 IndexedDB；
- 自动保存去抖；
- 文件解析和重计算放 Worker；
- 大表与 Agent 上下文序列化不得阻塞主线程；
- 连接线和选择框使用独立覆盖层；
- 目标：200 个普通块时仍能流畅平移缩放。

---

## 16. 安全与数据边界

### 16.1 默认本地

- 画布、反馈和导入文件默认保存在本机；
- 只有用户运行真实 Agent 时，显式选择的上下文才发送到配置的 Agent 服务；
- UI 在首次使用真实 Agent 时说明将发送哪些块和数据量；
- Mock 模式不发送任何网络请求。

### 16.2 上下文最小化

- 默认仅发送用户选定的数据；
- 不发送其他 Sheet；
- 不发送评论；
- 不发送历史版本；
- 不发送隐藏元数据；
- 用户可以在运行前查看上下文清单。

### 16.3 自定义代码

- 自定义代码只在沙箱中运行；
- 不继承宿主 Cookie 和存储；
- 不允许直接 fetch；
- 网络能力以后通过带授权的 BuckyOS Proxy 提供；
- 自定义块必须可以被用户禁用和删除。

### 16.4 错误与隐私日志

- 错误日志默认不包含完整数据表；
- 复制技术详情时提示用户检查敏感内容；
- 用户反馈导出默认只包含事件类型、时间和自愿填写内容。

---

## 17. 异常状态与错误文案

CodeAgent 必须实现以下状态，不得只处理成功路径。

| 场景 | 用户可见处理 |
|---|---|
| 文件格式不支持 | “当前仅支持 CSV 和 XLSX 文件。” |
| XLSX 无可读取工作表 | 显示原因并允许重新选择文件 |
| 数据过大 | 显示限制，允许截取前 N 行 |
| Agent 不可用 | 提供重试和切换 Mock 模式 |
| Agent 超时 | 保留 Prompt 与上下文，允许重试 |
| 补丁校验失败 | 不修改画布，显示可复制的校验摘要 |
| 文档修订冲突 | 提示“画布在运行期间已变化”，允许重新基于最新内容运行 |
| 来源已删除 | 结果标记“来源中断”，引导重新绑定 |
| 更新会覆盖人工修改 | 提供替换、生成副本、取消 |
| 自动保存失败 | 顶栏持续告警，并提供导出本地备份 |
| 自定义块崩溃 | 仅该块显示错误占位，不影响其他块 |
| 循环依赖 | 拒绝创建，并指出涉及的块 |

错误文案面向普通用户；技术详情折叠显示。

---

## 18. 可访问性与易用性要求

- 关键操作可通过键盘完成；
- 所有图标有 Tooltip 和可访问名称；
- 焦点状态清晰；
- 不仅依靠颜色表示 `fresh/stale/error`；
- 图表提供简短文字摘要或数据表入口；
- 支持系统“减少动态效果”；
- 中文为默认语言，内部使用 UTF-8；
- AI 相关术语优先使用普通用户能懂的表达，例如“数据来源”“需要刷新”，不要只写“Context”“Re-run”。

---

## 19. 原型视觉方向

### 19.1 整体气质

- 应像成熟的生产力工具，而不是科幻聊天界面；
- 画布背景克制，内容块边界清晰；
- 表格应保留 Excel 用户熟悉的秩序感；
- AI 标识统一但不过度抢眼；
- 运行状态和来源状态清晰可见；
- 不用大量渐变、霓虹和悬浮动画表达“AI”。

### 19.2 许愿格

建议具有以下视觉层次：

- 顶部：“许愿格”与状态；
- 中部：自然语言输入；
- 下部：来源标签、输出偏好、运行按钮；
- 成功后显示“已生成 N 个结果”；
- 运行时原位展示进度，不弹出全屏聊天窗口。

### 19.3 结果组

- 使用轻量分组框；
- 标题栏固定展示状态；
- 内部块仍可单独选择和编辑；
- 结果组移动时默认整体移动；
- 用户可拆分结果组。

---

## 20. 埋点与成功指标

原型重点收集行为和访谈，不以日活为目标。

### 20.1 关键量化指标

- 首次打开到成功运行许愿格的中位时间；
- 无他人指导完成主流程的比例；
- 创建许愿格后成功运行的比例；
- 运行成功后继续编辑结果的比例；
- 修改源数据后主动刷新的比例；
- 成功创建讲述路径的比例；
- 许愿格易理解评分；
- 愿意用于真实工作的用户比例。

### 20.2 首轮建议目标

这些是判断方向的参考值，不是上线 KPI：

- 70% 的目标用户能在 5 分钟内成功运行示例；
- 60% 能用自己的话正确描述“许愿格”；
- 50% 能提出一个与自身工作有关的真实使用场景；
- 50% 认为结果留在画布上比聊天复制更方便；
- 至少 30% 明确表达愿意把一项重复工作长期放入该产品。

### 20.3 重点观察的失败信号

- 用户仍然把它理解成“聊天机器人生成 PPT”；
- 用户不知道数据会发送到哪里；
- 用户不敢修改 AI 结果；
- 用户认为无限画布比 Excel 更乱；
- 用户无法理解来源与结果的关系；
- 用户只想要“一键生成”，不愿维护长期画布；
- 用户无法判断何时应刷新；
- 讲述路径没有减少 Excel→PPT 的搬运工作。

---

## 21. 验收标准

### 21.1 主链路验收

在一台没有配置真实模型服务的电脑上，测试人员能够：

1. 打开原型；
2. 进入季度经营分析示例；
3. 查看示例表格；
4. 打开预置许愿格；
5. 点击运行；
6. 看到至少四阶段进度；
7. 得到三张指标卡、一张图表、一张表和一段摘要；
8. 修改原始销售数据；
9. 看到结果变为“需要刷新”；
10. 重新运行并生成新结果；
11. 将三片区域加入讲述路径；
12. 播放并前后切换；
13. 退出后刷新页面，画布仍然存在；
14. 导出 `.aicanvas.json`，删除本地文档后可重新导入；
15. 提交反馈并下载反馈 JSON。

以上链路不能依赖开发者打开控制台修改数据。

### 21.2 Agent 协议验收

- Mock 和 HTTP Adapter 使用同一请求、事件和补丁类型；
- 无效补丁无法写入文档；
- 补丁应用是原子的；
- Agent 运行可取消；
- 文档修订冲突时不覆盖用户修改；
- 一次 Agent 生成可整体撤销。

### 21.3 数据验收

- 导入中文文件名与中文表头正常；
- CSV 编码异常给出提示；
- XLSX 多工作表可选择；
- 20,000 行以内表格可滚动浏览；
- 编辑源值后依赖状态正确变化；
- 导出再导入后块位置、内容、依赖和讲述路径一致。

### 21.4 交互验收

- 画布可平移、缩放和适应内容；
- 块可移动、缩放、多选、复制和删除；
- 编辑文字或表格时不会误拖动画布；
- 快捷键在输入状态下不误触；
- 错误状态有用户可见反馈；
- 不存在点击后无反应的主按钮。

---

## 22. 测试用例

### TC-001 空白画布创建

- 操作：新建空白画布；
- 预期：出现 Sheet 1，可创建文本块和许愿格，自动保存成功。

### TC-002 Excel 导入

- 操作：导入包含三个工作表的 XLSX；
- 预期：出现工作表选择器，选定工作表后生成表格块，显示行列数量。

### TC-003 复制 Excel 区域

- 操作：从 Excel 复制 10×5 区域并粘贴到空画布；
- 预期：自动创建表格块，数据行列正确。

### TC-004 许愿格运行成功

- 操作：引用示例表，运行预置 Prompt；
- 预期：进度完整，结果组生成，状态为最新。

### TC-005 来源变化

- 操作：修改表格中的销售额；
- 预期：关联结果变为需要刷新，未关联结果不受影响。

### TC-006 人工修改保护

- 操作：编辑 AI 摘要，再点击刷新；
- 预期：系统询问替换、生成副本或取消。

### TC-007 Agent 取消

- 操作：运行后立即取消；
- 预期：状态为已取消，画布无半成品块。

### TC-008 无效补丁

- 操作：Mock 调试模式返回引用不存在块的补丁；
- 预期：校验失败，画布不变化，显示错误摘要。

### TC-009 撤销生成

- 操作：成功生成后按撤销；
- 预期：整组生成内容和绑定一次性移除；重做可恢复。

### TC-010 演示路径

- 操作：创建三步路径并播放；
- 预期：镜头依次移动，可自由浏览并返回当前步骤。

### TC-011 持久化

- 操作：编辑后刷新页面；
- 预期：文档、相机位置和活动 Sheet 恢复。

### TC-012 导入导出一致性

- 操作：导出再导入；
- 预期：主要状态一致，未知字段不导致失败。

### TC-013 Agent 服务不可用

- 操作：配置错误 HTTP 地址并运行；
- 预期：显示服务不可用，可切换 Mock，不丢 Prompt。

### TC-014 大表

- 操作：导入 20,000 行 CSV；
- 预期：主界面仍可响应，表格使用虚拟滚动。

### TC-015 循环依赖

- 操作：尝试让两个许愿格互相依赖；
- 预期：系统拒绝并提示循环关系。

---

## 23. CodeAgent 实施任务拆分

### Milestone 1：领域模型与本地文档

- [ ] 建立 TypeScript 类型；
- [ ] 实现 Canvas Reducer / Command Bus；
- [ ] 实现撤销重做；
- [ ] 实现 IndexedDB Storage Adapter；
- [ ] 实现首页和文档创建；
- [ ] 实现 JSON 导入导出；
- [ ] 添加领域单元测试。

**完成标准：** 可以创建文档、保存、刷新恢复、导入导出、撤销重做。

### Milestone 2：无限画布与基础块

- [ ] 相机平移缩放；
- [ ] Sheet 管理；
- [ ] 块创建、选择、多选、拖动、缩放、复制、删除；
- [ ] 文本块；
- [ ] 框架块；
- [ ] 结果组；
- [ ] 基础右侧属性面板。

**完成标准：** 用户可在多个 Sheet 中自由组织普通内容。

### Milestone 3：表格与数据导入

- [ ] 表格块；
- [ ] 区域选择和粘贴；
- [ ] CSV 导入；
- [ ] XLSX 导入与工作表选择；
- [ ] Worker 解析；
- [ ] 虚拟滚动；
- [ ] `dataRevision`；
- [ ] 示例销售数据。

**完成标准：** 用户可将真实 Excel 数据放到画布并编辑。

### Milestone 4：许愿格与 Mock Agent

- [ ] 许愿格 UI；
- [ ] 上下文选择；
- [ ] Agent 请求协议；
- [ ] 运行状态机与取消；
- [ ] Mock Adapter；
- [ ] CanvasPatch 校验；
- [ ] 原子应用补丁；
- [ ] 标准指标、文本、表格和图表块；
- [ ] 生成结果整体撤销。

**完成标准：** 无后端环境可以演示完整 AI 生成流程。

### Milestone 5：依赖、刷新与真实 Agent 接口

- [ ] DataBinding；
- [ ] 来源变化标记；
- [ ] 手动刷新；
- [ ] 人工修改保护；
- [ ] 运行历史；
- [ ] HTTP/SSE Adapter；
- [ ] 服务健康检查；
- [ ] 冲突检测。

**完成标准：** 结果具有持续工作属性，而不是一次性生成。

### Milestone 6：讲述路径与用户测试

- [ ] 讲述路径编辑；
- [ ] 播放模式；
- [ ] 首次引导；
- [ ] 示例画布；
- [ ] 行为事件；
- [ ] 反馈表；
- [ ] 反馈 JSON 导出；
- [ ] 端到端主链路测试。

**完成标准：** 可交给目标用户独立体验并收集反馈。

### Milestone 7：P0.5

- [ ] 受限交互 HTML 块；
- [ ] 评论；
- [ ] 页面打开期间的定时刷新；
- [ ] 数据表 CSV 导出；
- [ ] 小地图；
- [ ] 视觉与性能优化。

---

## 24. CodeAgent 开发约束

1. **先完成纵向闭环，不要先建设通用低代码平台。**
2. **不要把主要 AI 入口做成右侧聊天框。** 右侧面板只能承载属性和运行详情。
3. **不要让 Mock Agent 使用与真实 Agent 不同的数据结构。**
4. **不要让 Agent 输出直接绕过 Schema 写入状态。**
5. **不要在领域层绑定具体画布、表格或图表库。**
6. **不要为首版实现完整公式系统。**
7. **不要添加无功能按钮。**
8. **不要默认上传整张画布。**
9. **不要用“AI 正在思考”等不可验证文案冒充真实进度。** 状态应对应系统阶段。
10. **任何自动覆盖用户内容的行为都必须有撤销或确认。**
11. **所有核心功能必须有错误态和空态。**
12. **主流程在 Mock 模式下必须离线可演示。**

---

## 25. 演示脚本

建议首轮演示控制在 8–10 分钟。

### 第一段：Excel 用户熟悉的起点

1. 打开季度经营分析示例；
2. 指出这是一张普通销售表；
3. 修改一个数字，说明表格仍可直接操作。

### 第二段：在单元格旁边“说出目标”

1. 打开许愿格；
2. 展示已绑定的数据来源；
3. 读出 Prompt；
4. 点击运行；
5. 展示过程状态；
6. 得到指标、图表、表格和总结。

讲解重点：

> 这不是一段需要复制走的聊天答案，而是画布里的内容。

### 第三段：继续修改与刷新

1. 直接编辑摘要标题；
2. 修改源表中的销售额；
3. 指出结果变为“需要刷新”；
4. 重新运行并选择“保留旧版，生成新结果”。

讲解重点：

> 许愿格保存的不只是答案，也保存了产生答案的方法和数据关系。

### 第四段：从分析到汇报

1. 把标题、指标、图表和结论加入讲述路径；
2. 播放；
3. 中途自由缩放查看异常明细；
4. 返回当前步骤。

讲解重点：

> 同一份内容既可以像 Excel 一样自由浏览，也可以像 PowerPoint 一样按顺序讲。

### 第五段：询问真实反馈

不要先解释远期宏大愿景，直接询问：

- 你第一眼觉得这是什么？
- 你会把哪一项真实工作放进来？
- 哪一步最不像你熟悉的 Excel？
- 哪一步让你最不放心？

---

## 26. 后续演进方向

### 26.1 从文件到长期对象

首版画布仍以本地文档形式存在。接入 BuckyOS 后：

- 每张画布拥有稳定对象身份；
- 每次发布形成不可变版本；
- 稳定 Name 可以指向持续更新版本；
- 用户可以分享固定版本或持续版本；
- 画布可成为课程、报告、模板或小型应用。

### 26.2 从本地单人到内容网络

BuckyOS 的身份与社交网络可以让画布具备：

- DID 作者身份；
- 用户、群组和组织权限；
- 块级评论；
- 课程班级空间；
- 发布、订阅和通知；
- 固定版本购买与持续版本订阅。

### 26.3 从 Prompt 到持续 Agent

未来许愿格不只运行一次：

- 按数据变化触发；
- 按时间触发；
- 按外部事件触发；
- 产生提醒、任务或新版本；
- 在权限范围内与其他 Agent 协作。

### 26.4 从标准块到可组合应用

Agent 在标准块之上组合：

- 表单；
- 审批；
- 仪表盘；
- 课程互动；
- 项目跟踪；
- 轻量业务系统。

但产品仍应优先复用 BuckyOS 的稳定能力，而不是每次从零生成一个孤立应用。

---

## 27. 待用户验证的问题

以下问题不应由开发团队提前想当然决定：

1. “许愿格”这一名称是否直观，还是“AI 单元格”“任务格”更好？
2. Excel 用户更习惯从规则表格开始，还是愿意直接进入无限画布？
3. 独立许愿格与表格内 AI 单元格，哪一种更容易理解？
4. 用户希望 AI 直接生成结果，还是先展示执行计划？
5. 用户最需要哪类标准块：表格、图表、文本、表单还是仪表盘？
6. 用户愿意让哪些内容自动刷新？
7. 当 AI 结果被人工修改后，刷新应该怎样处理最自然？
8. 用户是否理解“固定版本”和“持续更新版本”的价值差异？
9. 讲述路径能否真正减少制作 PowerPoint 的需要？
10. 本地部署是首要购买理由，还是仅属于加分项？
11. 用户是否需要传统 Sheet 标签，还是更偏好页面树？
12. 多少来源信息足以建立信任，又不会让界面过于复杂？

原型测试的首要成果不是功能数量，而是得到这些问题的真实答案。

---

## 28. 最终完成定义（Definition of Done）

当且仅当以下条件同时满足，P0 原型可以认为完成：

- 主链路端到端可运行；
- Mock 模式离线可演示；
- Excel/CSV 数据可真实导入和编辑；
- 许愿格不是聊天浮层，而是画布对象；
- Agent 输出通过结构化补丁生成标准块；
- 生成结果可编辑、可追溯、可刷新、可解除绑定；
- 源数据变化能正确触发过期状态；
- Agent 修改可整体撤销；
- 讲述路径可创建和播放；
- 文档可本地保存并导入导出；
- 关键错误状态均有处理；
- 至少一条端到端自动化测试覆盖示例流程；
- 可以在不需要开发人员解释操作步骤的情况下交给熟悉 Excel 的用户体验；
- 可以导出用户反馈和匿名行为数据；
- 原型中没有伪功能和无响应主按钮。

---

## 结语

BuckyOS AI Canvas 的首版目标不是证明它能完成所有办公工作，而是让目标用户第一次真实感受到：

> 我可以把数据和材料放在一张自由画布上，在需要的位置直接写下目标；AI 不只是回答我，而是把目标变成留在这里、能继续修改、能随着数据更新、还能直接拿去讲述的工作成果。

只要这个体验能够被非开发用户自然理解，并让他们立即联想到自己的真实工作，原型就完成了最重要的验证。
