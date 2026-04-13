# buckyos desktop 网格机制优化方案

## 1. 背景与问题

当前 desktop 网格机制的主要问题，不在“如何画出网格”，而在于**状态、布局、交互三层逻辑混在一起**，导致 resize、拖拽、序列化互相污染，行为越来越复杂。结合语音记录，当前问题可归纳为：

1. **状态层不稳定**  
   现在的实现倾向于直接序列化位置，甚至在 resize 过程中持续重布局，导致“当前位置”“用户手工调整的位置”“系统自动补位的位置”混在一起，难以维护。

2. **desktop 端 resize 影响大**  
   移动端通常不会频繁对整个 desktop 做 resize，而 desktop 端窗口大小、分辨率、可用区域都更容易变化。  
   如果底层模型不稳定，desktop 端会出现大量无意义重排。

3. **缺少“屏 + 槽位”的清晰抽象**  
   当前更像是在管理“像素位置”或“扁平顺序”，而不是管理“第几屏的第几个槽位”。这会导致：
   - resize 后大量图标位置不可预测；
   - 用户手工制造的空隙被系统自动填掉；
   - 跨屏挤压逻辑难以解释；
   - 新装 App、旧布局恢复、拖拽碰撞都在复用同一套不合适的逻辑。

4. **拖拽逻辑与重布局逻辑没有分治**  
   resize 补位是一种系统级重排；拖拽碰撞是一次用户主动交互。两者的目标不同，不应该共用一套槽位分配规则。

---

## 2. 设计目标

本次优化建议以 desktop 为重点，目标如下：

- 以**逻辑槽位**而不是像素位置作为唯一事实来源
- 把布局系统拆成**状态层 / 布局层 / 交互层**
- 桌面端采用**列优先（竖向优先）编号**，降低横向 resize 的副作用
- 引入**按屏管理**的二维模型，而不是全局一维数组
- 对 resize 后失效的图标采用**尾部追加**策略，**不自动填洞**
- 拖拽碰撞与 resize 重布局采用**两套不同规则**
- desktop 端网格大小采用**按字体档位决定的固定格子尺寸**
- 重布局只在 **resize 结束后** 执行，避免持续抖动与频繁序列化

---

## 3. 非目标

以下内容不建议作为本期默认行为：

- **不做自动回流**：窗口重新变大后，之前被挤到下一屏的图标，不自动回到前一屏
- **不做自动填洞**：系统补位、新装 App、resize 修复都不主动填补用户留下的内部空位
- **不在 desktop 端做连续拉伸网格**：desktop 网格不随着窗口变大而不断放大格子，额外空间只形成留白
- **不依赖第三方拖拽库决定产品规则**：库只负责拖拽传感与基础动画，碰撞/补位规则应由业务层掌控

---

## 4. 核心设计原则

### 4.1 槽位优先，像素位置派生
系统底层只管理：

- 第几屏（page）
- 第几个槽位（slot）
- 槽位上是谁（item）

像素坐标 `x/y` 一律由当前 viewport、网格尺寸、分页参数实时推导，**不作为权威状态持久化**。

### 4.2 屏幕是第一维，槽位是第二维
不要把整个桌面视为一个连续一维数组，而应该视为：

```ts
pages[pageIndex].slots[slotIndex]
```

也就是说，所有逻辑都基于“第 0 屏第 15 个槽位”“第 1 屏第 3 个槽位”来思考，而不是“全局第 18 个元素”。

### 4.3 desktop 端采用列优先编号
desktop 端建议使用**从上到下、从左到右**的槽位编号方式：

```text
第 0 列: 0, 1, 2, 3
第 1 列: 4, 5, 6, 7
第 2 列: 8, 9, 10, 11
...
```

在这种模型下：

- 高度决定一列能放多少个槽位（rows）
- 宽度决定一屏能放多少列（cols）
- 横向 resize 主要影响“这一屏能容纳多少列”，但**不改变已有槽位在列内的顺序**
- 这比横向优先编号更适合 desktop 场景

### 4.4 自动补位不填洞，只从尾部追加
用户在某一屏内部留出的空洞，应视为**用户有意为之**。因此系统自动补位时：

- **不填内部空位**
- 只从该屏**当前最大已用槽位之后**开始追加
- 当前屏放不下，再挤到下一屏尾部
- 屏幕数量视为无限扩展

### 4.5 resize 补位与拖拽碰撞分治
- **resize / 初始化 / 新装 App 补位**：按“尾部追加、尊重空洞”的规则
- **拖拽过程碰撞**：按“局部碰撞”或“同屏重排”的规则

这两类问题必须拆开，否则交互会越来越怪。

---

## 5. 总体分层架构

建议拆成三层：

### 5.1 状态层（Source of Truth）
负责维护逻辑布局状态，只保存：

- `itemId`
- `pageIndex`
- `slotIndex | undefined`
- `preferredPage`
- `placementType`（`manual` / `auto` / `reflow`）
- `seq`（稳定排序字段，例如安装顺序或首次进入桌面的顺序）
- `layoutVersion`

建议的数据结构示意：

```ts
type SlotRef = {
  page: number
  slot?: number   // undefined 表示当前 viewport 下无有效槽位
  preferredPage: number
}

type PlacementType = 'manual' | 'auto' | 'reflow'

interface DesktopItemPlacement {
  itemId: string
  ref: SlotRef
  placementType: PlacementType
  seq: number
}

interface DesktopLayoutState {
  version: 2
  mode: 'desktop'
  order: 'column-major'
  items: Record<string, DesktopItemPlacement>
}
```

### 5.2 布局层（Viewport -> Geometry）
负责根据当前桌面可用区域计算：

- `usableWidth`
- `usableHeight`
- `cellWidth`
- `cellHeight`
- `rows`
- `cols`
- `pageCapacity = rows * cols`

同时提供 `slot -> rect` 的映射，不直接改动状态层。

### 5.3 交互层（Drag / Hover / Drop）
负责：

- 拖拽预览
- 命中槽位判定
- 占位动画
- 放手后的正式提交

交互层维护的是**临时态**，只有 drop 成功后才写回状态层并序列化。

---

## 6. desktop 端网格模型

### 6.1 可用区域定义

desktop 网格计算必须基于**实际可用区域**，需要扣除：

- 顶部状态条 / 标题区
- 底部分页器 / 多屏选择器
- 安全区与外层容器 padding

即：

```ts
usableWidth  = viewportWidth  - horizontalInsets
usableHeight = viewportHeight - topBarHeight - bottomPagerHeight - verticalInsets
```

### 6.2 固定网格尺寸
desktop 端建议采用**固定格子尺寸**，由字体档位决定最小可读尺寸，而不是跟随窗口连续缩放。

建议：

- 小 / 中 / 大字体，对应不同的 `cellHeightMin`
- `cellWidth` 按图标与文本比例固定推导
- 当窗口变大时，**格子不继续变大**，多余空间只形成留白

例如：

```ts
rows = floor(usableHeight / cellHeight)
cols = floor(usableWidth  / cellWidth)
pageCapacity = rows * cols
```

如果 `usableHeight = 420`、`cellHeight = 100`，则：

- `rows = 4`
- 剩余 `20px` 不触发新增槽位
- 这 20px 只作为底部或容器留白

### 6.3 与 mobile 的差异
这部分是本方案的关键边界：

| 维度 | Desktop | Mobile |
|---|---|---|
| 底层状态 | 共用“屏 + 槽位”模型 | 共用“屏 + 槽位”模型 |
| 编号方式 | 列优先（column-major） | 可保留现有行优先（row-major） |
| 网格尺寸 | 固定尺寸，由字体档位决定 | 受最小尺寸约束，并可在阈值间均匀拉伸 |
| resize 策略 | 区分 pure resize / reflow resize | 主要处理字号变化、方向变化、少量尺寸变化 |
| 重点诉求 | 降低横向 resize 副作用 | 保持小屏可读性与均匀分布 |

---

## 7. 槽位分配与重布局规则

### 7.1 统一概念：已分配 / 未分配

每个 item 在任意时刻只有两种状态：

1. **已分配**
   - `page` 有效
   - `slot` 有效

2. **未分配**
   - `page` 仍然保留“期望所在屏”（`preferredPage`）
   - `slot = undefined`

未分配对象包括：

- resize 后原槽位失效的图标
- 初始化时旧布局无法适配当前窗口的图标
- 新安装、首次出现、还未落位的 App

### 7.2 失效判定
当 viewport 变化后，先用新参数重新解释已有布局。

若某 item 满足：

```ts
slot >= pageCapacity
```

则它在当前屏中已无有效槽位，应变成：

```ts
{ page: 原 page, slot: undefined, preferredPage: 原 page }
```

### 7.3 尾部追加算法
对所有 `slot = undefined` 的 item，执行统一补位。

规则如下：

1. 从 `preferredPage` 开始找位置
2. 该屏的可追加位置定义为：

```ts
appendSlot = maxUsedSlot(page) + 1
```

3. 如果 `appendSlot < pageCapacity`，则落到该槽位
4. 如果当前屏放不下，则转到下一屏继续找
5. **不检查内部空洞，不填洞**

伪代码如下：

```ts
function appendUndefinedItems(queue, pages, pageCapacity) {
  for (const item of queue) {
    let page = item.ref.preferredPage

    while (true) {
      const appendSlot = getMaxUsedSlot(page, pages) + 1

      if (appendSlot < pageCapacity) {
        assign(item, page, appendSlot)
        break
      }

      page += 1
    }
  }
}
```

### 7.4 队列顺序
为保证稳定性，`undefined` 队列建议按以下顺序排序：

1. 原 `preferredPage`
2. 原 `slot`（若有）
3. `seq`（安装顺序/首次进入顺序）

这样可保证：

- resize 溢出的元素顺序稳定
- 新装 App 的落位行为可预测
- 初始化恢复时不会随机抖动

---

## 8. 示例：缩屏后的补位行为

假设原来：

- 每屏容量为 20
- 第 0 屏放了 20 个图标
- 第 1 屏放了 5 个图标

当布局从 `4 x 5 = 20` 变成 `4 x 4 = 16` 后：

- 第 0 屏原本的槽位 `16, 17, 18, 19` 失效
- 这 4 个图标进入 `preferredPage = 0, slot = undefined`

再看第 1 屏，如果它的已用槽位是：

```text
0: 有
1: 空
2: 有
3: 有
4: 有
5: 有
```

那么系统不应把溢出图标放进 `1` 这个空位，而应该依次放到：

```text
6, 7, 8, 9
```

原因是：

- `1` 是用户手工保留下来的空洞
- 自动补位只能从该屏尾部追加
- 这样既尊重用户布局，也能保证规则简单一致

---

## 9. resize 处理机制

### 9.1 两类 resize

### A. Pure Resize（纯 resize）
只重算 viewport 与几何，不改逻辑槽位。典型场景：

- 宽高变化但没有 item 溢出
- 或者只是留白变化
- 或者只是同一套槽位映射到新的像素位置

特点：

- 不触发逻辑重排
- 不立即序列化 item 布局
- 只更新渲染层

### B. Reflow Resize（触发重布局的 resize）
当 resize 导致部分 item 的逻辑槽位失效时，才执行补位流程。典型触发条件：

- `pageCapacity` 变小
- 某些 `slot >= pageCapacity`
- 初始化恢复旧布局时发现当前窗口放不下
- 有新装 App 需要落位

### 9.2 执行时机
**重布局建议在 resize 结束后执行**，而不是每一帧都执行。

推荐做法：

- `resize` 过程中：只做 pure resize 级别的视觉更新
- `resize end / debounce` 后：统一校验失效槽位并执行一次补位
- 补位完成后再一次性持久化

### 9.3 不自动回流
当窗口变小，把部分图标从第 0 屏挤到第 1 屏后，若窗口再变大：

- 这些图标**默认不自动回到第 0 屏**
- 因为它们此时在第 1 屏已经拥有了合法槽位
- 系统不应在没有用户动作的前提下再次改动它们

如果未来需要“整理图标 / 紧凑布局”，应做成**显式用户操作**，不作为默认 resize 行为。

---

## 10. 初始化与持久化策略

### 10.1 初始化流程
初始化时，不要直接相信历史序列化结果，而应经过“校验 + 修复”流程：

1. 读取持久化布局
2. 依据当前 viewport 计算 `rows/cols/pageCapacity`
3. 校验每个 item 的 `(page, slot)` 是否仍有效
4. 对无效 item 标记为 `slot = undefined`
5. 对所有 `undefined` item 执行尾部追加补位
6. 输出稳定的当前布局

### 10.2 持久化内容
建议持久化以下内容：

- `layoutVersion`
- `itemId -> { page, slot, preferredPage, placementType, seq }`
- 字体档位 / 视图模式等影响网格的参数

不建议持久化：

- 实时像素位置
- resize 中间过程状态
- hover / drag preview 临时态

### 10.3 旧数据迁移
若当前版本保存的是“像素位置”或“扁平位置”：

- 在首次升级时，将旧布局排序为稳定顺序
- 按新模型转换为 `page + slot`
- 转换成功后仅写入新版本格式

---

## 11. 拖拽逻辑设计

拖拽逻辑应与 resize 补位彻底分开。

### 11.1 基本原则
拖拽的本质是：

> 把一个已经有槽位的可见对象，从当前槽位移动到另一个槽位。

拖拽过程中：

- 使用临时预览态，不立即改持久化状态
- 只有 drop 成功后，才提交新的逻辑槽位
- 不在 hover 过程中频繁序列化

### 11.2 规则一：拖到空槽位
若目标槽位为空，则直接放入，100% 成功。

### 11.3 规则二：拖到占用槽位，且目标屏未满
如果目标屏未满，不建议做整屏级别重排，而建议做**局部碰撞补位**：

1. 被占用的目标 item 寻找目标附近的空位
2. 搜索范围以邻近格子为主
3. 优先选“扰动最小”的位置
4. desktop 端因为是列优先编号，若 `下` 与 `右` 都可用，则优先 `下`
5. mobile 端若保留行优先编号，则 `右` 与 `下` 都可用时优先 `右`

更稳妥的工程定义可以是：

- 先按**曼哈顿距离最小**搜索
- 再按**槽位编号变化最小**排序
- 最后用平台优先级做 tie-break

这样能兼顾“碰撞感”和“稳定性”。

### 11.4 规则三：拖到占用槽位，且目标屏已满
若目标屏已满，不建议把目标 item 挤到下一屏。  
desktop 端更合理的行为是：**同屏重排（reorder）**。

例如：

- 用户把同屏第 3 个槽位的 item 拖到第 15 个槽位
- 则该屏内部按线性顺序重排，而不是把某个元素挤去下一屏

建议规则：

- 若 `source < target`：区间 `(source+1 ... target)` 依次前移
- 若 `source > target`：区间 `(target ... source-1)` 依次后移

伪代码：

```ts
function reorderWithinPage(pageItems, sourceSlot, targetSlot) {
  if (sourceSlot < targetSlot) {
    shiftLeft(pageItems, sourceSlot + 1, targetSlot)
  } else if (sourceSlot > targetSlot) {
    shiftRight(pageItems, targetSlot, sourceSlot - 1)
  }
  placeDraggedItem(targetSlot)
}
```

这样有几个好处：

- 用户更容易理解：这是“换位置 / 调顺序”，不是“挤爆下一屏”
- 同屏满载时交互更稳定
- 不会把无关页面卷进来

### 11.5 跨屏拖拽
跨屏拖拽建议只在**用户明确拖到目标屏**时发生，不建议由本屏碰撞隐式触发跨屏连锁挤压。

也就是说：

- 用户明确拖到第 1 屏：就在第 1 屏内应用上述规则
- 用户只是在第 0 屏内交换位置：就只处理第 0 屏，不影响第 1 屏

---

## 12. “用户手工布局”与“系统自动布局”的边界

语音中反复提到一个关键点：

> 用户自己调过的位置，和系统自动插入的位置，需要在产品语义上分开。

建议最少引入 `placementType`：

- `manual`：用户通过拖拽明确放过
- `auto`：系统自动分配（如新装 App 初始落位）
- `reflow`：因 resize / 初始化修复而被系统重新放置

本期 V1 中，`placementType` 可以先主要用于：

- 调试与问题追踪
- 数据分析
- 后续“自动整理 / 智能补位”能力预留

V1 默认仍然统一遵守：

- 不自动填洞
- 不自动回流
- 不擅自修改已稳定的合法槽位

---

## 13. 对第三方拖拽库的建议

如果当前拖拽是基于第三方 grid/drag 库实现，建议重新划分职责：

### 库负责
- Pointer / mouse / touch 事件采集
- 基础 drag sensor
- ghost / preview 跟手
- 基础动画能力

### 业务层负责
- 槽位命中计算
- 占位策略
- 同屏重排策略
- 局部碰撞补位策略
- drop 后的正式提交与序列化

如果第三方库无法支持自定义碰撞与重排规则，应考虑：

- 仅保留其 drag sensor
- 或替换为更轻量的拖拽基础库
- 槽位系统与布局规则完全自管

---

## 14. 工程实现建议

### 14.1 事件流
建议实现为以下流程：

```text
加载布局
  -> 计算 viewport / grid metrics
  -> 校验已有 page+slot
  -> 生成 undefined queue
  -> 尾部追加补位
  -> 渲染

resize
  -> 只更新 geometry（pure resize）
  -> resize end 后校验失效项
  -> 必要时执行 reflow
  -> 一次性持久化

drag start
  -> 创建 drag preview state

drag hover
  -> 只更新 hover / preview
  -> 不改持久化状态

drop
  -> 按拖拽规则生成新 page+slot
  -> 提交状态
  -> 持久化
```

### 14.2 性能建议
- `resize` 期间不要频繁写持久化
- `hover` 期间不要频繁改真实状态
- 页面占用关系建议维护为 `Map<page, Map<slot, itemId>>`
- `maxUsedSlot` 可缓存或按页增量更新
- 动画只作用于变更项，不整页重绘

### 14.3 调试输出
建议在开发模式输出以下信息：

- `rows / cols / pageCapacity`
- 每个 page 的 `occupiedSlots`
- `undefined queue`
- 每次 reflow 的移动明细
- 拖拽命中的 target page/slot
- 本次行为类型：`pureResize` / `reflowResize` / `dragReorder` / `dragCollision`

---

## 15. 验收场景

| 场景 | 预期结果 |
|---|---|
| desktop 宽度缩小，第一页尾部放不下 | 尾部元素变为 `undefined`，并从下一屏尾部追加 |
| 第二屏中间有空洞 | resize 补位不会填洞，只会从 `maxUsedSlot + 1` 继续放 |
| 窗口恢复变大 | 已被挤到下一屏的图标不自动回流 |
| 初始化时读取旧布局，当前窗口更小 | 无效槽位自动修复，顺序稳定，不随机抖动 |
| 新装 App 出现 | 作为未分配对象进入统一补位流程 |
| 未满屏时拖到占位图标上 | 优先局部碰撞补位，不做整屏挤压 |
| 满屏时同屏拖拽到占位图标上 | 在同屏内部按线性顺序重排，不把图标挤到下一屏 |
| 拖拽 hover 中 | 只更新预览态，不持久化 |
| resize 过程中 | 只做视觉更新，重布局在 resize 结束后执行 |

---

## 16. 分阶段落地建议

### Phase 1：状态模型重构
- 引入 `page + slot` 作为唯一事实源
- 引入 `undefined / preferredPage / placementType`
- 完成旧数据迁移

### Phase 2：desktop 布局引擎
- 实现 desktop 固定格子尺寸
- 实现列优先编号
- 实现 `slot <-> rect` 双向映射
- 接入可用区域扣减逻辑

### Phase 3：resize 分治
- 拆分 pure resize 与 reflow resize
- 加入 `resize end` 触发补位
- 接入尾部追加算法

### Phase 4：拖拽逻辑重写
- 拖拽预览态独立
- 实现“空槽直接放”“未满屏局部补位”“满屏同屏重排”
- 将第三方库降级为传感器/动画层

### Phase 5：测试与数据验证
- 回归 resize / 初始化 / 新装 App / 跨屏拖拽
- 验证“空洞不被填补”“不会自动回流”
- 观察用户是否理解同屏重排的反馈

---

## 17. 最终结论

这次优化的关键，不是再补一套更复杂的 if/else，而是要把整个 desktop 网格机制换成一套更稳定的思维方式：

1. **底层只认“屏 + 槽位”**
2. **desktop 使用列优先编号**
3. **状态、布局、交互三层彻底分离**
4. **resize 修复只做尾部追加，不填洞、不回流**
5. **拖拽碰撞与 resize 重布局分治**
6. **desktop 网格尺寸固定，由字体档位决定**

只要这六点立住，desktop 端的 resize、初始化恢复、拖拽、分页扩展就会从“规则打架”变成“规则可推导”，整体复杂度会明显下降，后续也更容易继续演进。

