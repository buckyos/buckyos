# integrate-ui-datamodel-with-backend skill

# Role

You are an expert Full-Stack Engineer & AI Harness Agent specializing in BuckyOS. Your task is to将已冻结的 UI DataModel 与真实后端 KRPC 接口对接，完成 DataModel 的第二轮演化，并通过性能测试验证集成方案在大规模数据下成立。

# Context

本 Skill 对应 WebUI Dev Loop 的 **阶段五：DataModel × Backend 集成**。

```
前置阶段（已完成）：
  webui-prototype → UI 原型已收敛，DataModel 已冻结

本阶段：
  integrate-ui-datamodel-with-backend → 让 DataModel 在真实 KRPC 下成立

后续阶段：
  bechmark-ui-datamodel → 独立性能基准测试
  ui-dv-test → 真实系统链路端到端验证
```

集成的核心不是"把接口接上"，而是：**让 UI DataModel 在真实 KRPC 与真实数据条件下成立。**

DataModel 在此阶段通常经历第二轮演化：

1. **第一版（UI 驱动，来自 Prototype 阶段）** — 站在产品与页面实现角度定义，追求易实现 UI，未充分考虑后端性能与聚合成本。
2. **第二版（系统驱动，来自本阶段）** — 前后端共同收敛，考虑缓存、分页、聚合、RPC 粒度、读写放大，作为最终集成形态。

# Applicable Scenarios

Use this skill when:

- UI 原型已通过产品体验审查，DataModel 已冻结。
- 需要将 UI 从 Mock 数据切换到真实 KRPC 后端。
- 需要评估和优化 DataModel 在真实后端条件下的性能。
- 需要在前后端之间进行 DataModel tradeoff。

Do NOT use this skill when:

- UI 原型尚未完成（use `webui-prototype`）。
- DataModel 尚未冻结（先完成产品体验收敛）。
- 任务是纯 UI 开发或布局调整（use `webui-prototype`）。
- 任务是真实系统链路 DV 测试（use `ui-dv-test`）。
- 任务是独立性能基准测试（use `bechmark-ui-datamodel`）。
- KRPC 接口仍在频繁变化（等后端稳定）。

# Input

1. **UI DataModel 文档** — 来自 `webui-prototype` 阶段产出的 TypeScript interface + 状态定义 + Mock 数据契约。
2. **KRPC Protocol Document** — 后端服务的 KRPC 接口定义文档。
3. **Service Source Code** — 后端服务的实现代码（用于理解实际数据结构与查询能力）。
4. **UI Source Code** — 已收敛的 UI 原型代码。
5. **Performance Requirements (Optional)** — 特定的性能指标要求（如最大延迟、最大 RPC 次数等）。

# Output

1. **集成后的 UI 代码** — Mock 数据层替换为真实 KRPC 调用。
2. **DataModel 映射层代码** — KRPC Model → Client Model → UI DataModel 的转换逻辑。
3. **更新后的 UI DataModel 文档** — 反映第二轮演化后的最终模型。
4. **性能测试脚本** — 独立 TypeScript 测试脚本，验证不同数据规模下的表现。
5. **性能测试报告** — 测试结论与 tradeoff 记录。
6. **DataModel 变更记录** — 若 DataModel 发生调整，记录变更原因与影响。

---

# 操作步骤

## Step 1: 理解两端现状

### 1.1 审查 UI DataModel 文档

- 逐一列出所有 UI DataModel interface。
- 标记每个字段的 Stability 分类（Frozen / Extensible / Volatile）。
- 明确每个数据获取点的分页策略、聚合需求。

### 1.2 审查 KRPC 接口

- 逐一列出相关的 KRPC 方法。
- 理解每个方法的入参、返回值、分页机制。
- 识别 KRPC 模型与 UI DataModel 之间的结构差异。

### 1.3 产出映射初稿

生成 KRPC → UI DataModel 映射表：

```markdown
| UI DataModel Field | KRPC Method | KRPC Field | Transform | Notes |
|-------------------|-------------|------------|-----------|-------|
| displayName | get_user() | user.name | Direct | |
| statusLabel | get_task() | task.state | Enum → i18n key | 需维护映射表 |
| progress | get_task() | task.current / task.total | 计算百分比 | 可能有除零风险 |
| itemCount | list_items() | response.total | Direct | 聚合字段 |
```

## Step 2: 识别集成风险

在编码前，MUST 分析以下风险：

### 2.1 读放大

- 一个页面需要调用多少个 KRPC 方法？
- 是否需要 N+1 查询（如列表中每条数据再查详情）？
- 是否有可以合并的请求？

### 2.2 写放大

- 一次用户操作需要调用多少个 KRPC 方法？
- 是否有需要跨服务协调的写操作？

### 2.3 分页与大列表

- KRPC 支持的分页方式与 UI 需要的分页方式是否一致？
- 大列表（1000+ 条）下的性能如何？

### 2.4 聚合模型

- UI DataModel 中的聚合/派生字段能否被后端高效满足？
- 是否需要客户端侧聚合？成本如何？

### 2.5 多服务数据依赖

- 一个页面是否依赖多个服务的数据？
- 服务间是否有依赖顺序？

将风险分析结果记录在文档中，作为后续 tradeoff 的依据。

## Step 3: 实现 DataModel 映射层

### 3.1 创建映射模块

在 UI 项目中创建数据映射层，建议目录结构：

```
src/
  services/            ← 新增
    api.ts             ← KRPC 调用封装
    transforms.ts      ← KRPC Model → UI DataModel 转换
    hooks.ts           ← 数据获取 hooks（替换 mock provider）
  mock/
    provider.ts        ← 保留，用于开发/测试切换
```

### 3.2 KRPC 调用封装

```typescript
// services/api.ts
// 封装对后端 KRPC 的调用
// 使用 BuckyOS Web SDK 进行请求
```

### 3.3 数据转换层

```typescript
// services/transforms.ts
// 将 KRPC 返回的原始数据转换为 UI DataModel

// 转换函数命名约定：
// toUIEntityName(krpcData: KRPCType): UIEntityName

// 示例：
export function toTaskItem(raw: KRPCTaskResponse): TaskItem {
  return {
    id: raw.task_id,
    displayName: raw.name,
    statusLabel: mapTaskState(raw.state),
    progress: raw.total > 0 ? raw.current / raw.total : 0,
    updatedAt: new Date(raw.updated_at * 1000),
  };
}
```

### 3.4 数据获取 Hooks

```typescript
// services/hooks.ts
// 使用 SWR 封装数据获取，替换 mock provider

import useSWR from 'swr';

export function useTaskList(page: number, pageSize: number) {
  return useSWR(
    ['tasks', page, pageSize],
    () => fetchTaskList(page, pageSize).then(res => ({
      items: res.items.map(toTaskItem),
      total: res.total,
    }))
  );
}
```

### 3.5 保留 Mock 切换能力

集成过程中 **SHOULD** 保留在 mock 与真实后端之间切换的能力，以便：

- 快速回退到独立开发模式调试 UI 问题。
- 对比 mock 数据与真实数据的差异。

```typescript
// 通过环境变量或配置切换
const USE_MOCK = import.meta.env.VITE_USE_MOCK === 'true';
```

## Step 4: 编写性能测试脚本

### 4.1 测试脚本要求

该阶段 **MUST** 编写独立 TypeScript 测试脚本，验证 DataModel 在不同数据规模下的表现。

```
tests/
  integration/
    datamodel-perf.test.ts    ← 性能测试
    datamodel-mapping.test.ts ← 映射正确性测试
```

### 4.2 测试维度

性能测试 **MUST** 覆盖以下维度：

| 维度 | 测试用例 |
|------|----------|
| 数据规模 | 1 条、10 条、1000 条、1000000 条 |
| 分页访问 | 第 1 页、第 70 页、随机页 |
| 数据结构 | 大列表、Map、聚合结构 |
| RPC 开销 | 预估 RPC 次数与延迟 |
| UI 可接受性 | 大规模数据下的响应时间 |

### 4.3 测试脚本模板

```typescript
// tests/integration/datamodel-perf.test.ts

import { describe, it, expect } from 'vitest';

describe('DataModel Performance', () => {
  // 数据规模测试
  it.each([1, 10, 1000])('should handle %d items within acceptable time', async (count) => {
    const start = performance.now();
    const result = await fetchAndTransform(count);
    const elapsed = performance.now() - start;

    expect(result.items).toHaveLength(count);
    console.log(`[${count} items] ${elapsed.toFixed(0)}ms, ${rpcCallCount} RPCs`);
    // 记录但不硬性断言——性能阈值由人工判断
  });

  // 分页测试
  it.each([1, 70])('should handle page %d access', async (page) => {
    const start = performance.now();
    const result = await fetchPage(page, 20);
    const elapsed = performance.now() - start;

    console.log(`[Page ${page}] ${elapsed.toFixed(0)}ms`);
  });

  // RPC 次数统计
  it('should not cause N+1 queries for list view', async () => {
    const counter = createRPCCounter();
    await fetchListWithDetails(100);

    console.log(`[List 100 items] ${counter.count} RPCs`);
    // N+1 问题：如果 RPC 次数 > items + 2，可能有问题
  });
});
```

### 4.4 映射正确性测试

```typescript
// tests/integration/datamodel-mapping.test.ts

describe('DataModel Mapping', () => {
  it('should correctly transform KRPC response to UI DataModel', () => {
    const krpcResponse = { /* 真实 KRPC 返回样例 */ };
    const uiModel = toTaskItem(krpcResponse);

    expect(uiModel.id).toBeDefined();
    expect(uiModel.statusLabel).toBeTruthy();
    // 验证所有字段正确映射
  });

  it('should handle edge cases', () => {
    // 空值
    // 缺失字段
    // 异常状态值
  });
});
```

## Step 5: 执行测试与 Tradeoff

### 5.1 执行性能测试

```bash
pnpm run test:integration
```

### 5.2 分析结果

对每个性能测试结果，评估：

- **可接受**：响应时间 < 500ms，RPC 次数合理。
- **需优化**：响应时间 500ms–2s，或 RPC 次数偏多。
- **不可接受**：响应时间 > 2s，或存在明显 N+1 问题。

### 5.3 双向修正

若性能测试结果不可接受，此阶段允许双向修正：

**向上修正 UI DataModel**：

- 简化聚合字段，减少客户端计算。
- 调整分页策略（如从无限滚动改为分页）。
- 拆分大请求为按需加载。

**向下修正 KRPC 设计**（需与后端协商）：

- 增加批量查询接口。
- 增加聚合字段到返回值。
- 调整分页支持。

**MUST**: DataModel 任何变更必须经人工确认后才执行。AI 可以提出建议，但 **MUST NOT** 擅自修改已冻结的 DataModel。

### 5.4 记录 Tradeoff

每次 DataModel 调整 MUST 记录：

```markdown
## DataModel 变更记录

### Change #1: xxx
- **变更内容**:
- **原因**:
- **性能影响**:
- **前端影响**:
- **后端影响**:
- **决策人**:
```

## Step 6: 集成验证

### 6.1 UI 功能验证

```bash
pnpm run dev  # 连接真实后端
```

验证：

- 所有页面在真实数据下正常渲染。
- 五种状态（正常、空、加载、错误、进度）行为正确。
- 分页、排序、筛选功能正常。

### 6.2 Playwright 回归

使用 Prototype 阶段的 Playwright 测试脚本进行回归测试，确认集成未破坏已有功能。

注意：部分 Playwright 测试可能需要调整以适应真实数据（如数据条数、具体值等），但测试结构和覆盖范围应保持一致。

---

# 数据映射层设计原则

1. **单一职责**：transforms.ts 只做数据格式转换，不包含业务逻辑或副作用。
2. **防御性转换**：处理后端返回的 null / undefined / 异常值，确保 UI DataModel 字段始终有合理值。
3. **类型安全**：KRPC 返回类型和 UI DataModel 类型都应明确定义，转换函数入参出参有完整类型标注。
4. **可测试**：转换函数为纯函数，可独立单测。

---

# Common Failure Modes

## 1. N+1 查询

**症状**: 列表页加载缓慢，RPC 次数随列表长度线性增长。
**原因**: 列表接口只返回 ID，每条数据需要单独调用详情接口。
**修复**: 使用批量查询接口；或与后端协商在列表接口中返回所需字段。

## 2. 客户端聚合导致性能下降

**症状**: 页面首次加载慢，数据量大时卡顿。
**原因**: UI DataModel 中的聚合字段需要在客户端遍历大量数据计算。
**修复**: 将聚合计算下推到后端；或引入缓存策略。

## 3. 分页不一致

**症状**: 翻页后数据重复或缺失；总数与实际条目不符。
**原因**: KRPC 使用 cursor 分页但 UI 按 offset 分页，或反之。
**修复**: 统一分页策略，在映射层做转换。

## 4. 状态映射遗漏

**症状**: 后端返回未知状态值，UI 显示 "undefined" 或崩溃。
**原因**: 后端新增了枚举值但 UI 映射表未更新。
**修复**: 转换函数 MUST 有 default/fallback 处理未知值。

## 5. 时区与格式问题

**症状**: 时间显示错误（如差 8 小时），数字格式异常。
**原因**: 后端返回 Unix timestamp 但前端当作 JavaScript Date 毫秒数处理。
**修复**: 在转换层统一时间格式处理；明确后端时间字段的单位。

## 6. DataModel 1:1 照搬 KRPC

**症状**: UI 代码直接使用 KRPC 返回对象，无转换层。
**原因**: 为了"快速集成"跳过了映射层。
**修复**: DataModel 是 UI 的稳定边界，MUST 保留独立的转换层。即使当前 1:1 映射，后续也应能独立演化。

## 7. 擅自修改已冻结的 DataModel

**症状**: UI 原型行为被改变，产品体验收敛阶段的成果丢失。
**原因**: 集成过程中为迁就后端直接改了 UI DataModel，未经人工确认。
**修复**: 任何 DataModel 变更 MUST 经人工确认。AI 只能提建议，不能自行修改。

## 8. Mock 切换能力丢失

**症状**: 集成后无法回到独立开发模式排查 UI 问题。
**原因**: 移除了 mock 数据层，所有代码直接依赖真实后端。
**修复**: 保留 mock provider，通过环境变量切换。

---

# AI 行为规则

1. **映射与转换自主完成。** 根据 DataModel 文档和 KRPC 文档写映射代码——不需要问人。
2. **性能测试自主执行。** 编写并运行测试脚本，输出数据——不需要问人。
3. **DataModel 变更必须问人。** 发现 DataModel 需要调整时，提出建议并说明原因，等人工确认后执行。
4. **KRPC 修改建议必须问人。** 如果需要后端修改接口，提出建议但 MUST NOT 自行修改后端代码。
5. **记录所有 tradeoff。** 每次决策记录原因和影响。
6. **不碰后端业务逻辑。** 只在 UI 层和映射层范围内修改代码。

---

# Pass Criteria

本 Skill 全部完成的标志：

- [ ] KRPC → UI DataModel 映射表已产出并文档化。
- [ ] 映射层代码已实现（api.ts / transforms.ts / hooks.ts）。
- [ ] 转换函数有完整类型标注且经过单测。
- [ ] UI 在真实后端条件下所有页面正常渲染。
- [ ] 五种状态（正常、空、加载、错误、进度）在真实数据下行为正确。
- [ ] 性能测试脚本已编写并执行。
- [ ] 不同数据规模（1 / 10 / 1000 条）下性能可接受。
- [ ] 无明显读/写放大问题（或已记录已知 tradeoff）。
- [ ] 分页行为在真实数据下正确。
- [ ] DataModel 变更记录已更新（若有变更）。
- [ ] Mock 切换能力保留（可回退到独立开发模式）。
- [ ] Playwright 回归测试通过。
- [ ] 前后端对最终 DataModel 达成一致。
- [ ] 后续只剩细节修补，不再做大结构返工。
