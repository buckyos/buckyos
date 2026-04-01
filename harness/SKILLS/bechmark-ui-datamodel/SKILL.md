# bechmark-ui-datamodel skill

# Role

You are an expert Performance Engineer & AI Harness Agent specializing in BuckyOS. Your task is to对已集成的 UI DataModel 进行独立性能基准测试，量化 DataModel 在不同数据规模、分页模式、并发场景下的表现，产出可复现的性能报告，为 DataModel 最终定型提供数据支撑。

# Context

本 Skill 对应 WebUI Dev Loop 中 **DataModel × Backend 集成之后、UI DV Test 之前** 的独立性能验证环节。

```
前置阶段（已完成）：
  webui-prototype → UI 原型已收敛，DataModel 已冻结
  integrate-ui-datamodel-with-backend → DataModel 映射层已实现，基本集成已通过

本阶段：
  bechmark-ui-datamodel → 独立性能基准测试，量化瓶颈

后续阶段：
  ui-dv-test → 真实系统链路端到端验证
```

与 `integrate-ui-datamodel-with-backend` 中的性能测试不同，本 Skill 聚焦于**系统性、可复现的基准测试**：

- `integrate` 阶段的性能测试：验证集成方案基本可行，发现明显问题。
- `benchmark` 阶段的性能测试：量化边界、建立基线、生成可对比的报告。

# Applicable Scenarios

Use this skill when:

- DataModel × Backend 集成已完成，需要进行系统性性能评估。
- 需要量化 DataModel 在大规模数据下的瓶颈。
- 需要为性能优化决策提供数据支撑。
- 版本发布前需要性能基线报告。
- DataModel 发生变更后需要回归对比。

Do NOT use this skill when:

- DataModel 映射层尚未实现（use `integrate-ui-datamodel-with-backend`）。
- UI 原型尚未完成（use `webui-prototype`）。
- 任务是真实系统链路 DV 测试（use `ui-dv-test`）。
- 后端 KRPC 接口仍在频繁变化（等后端稳定）。

# Input

1. **UI DataModel 文档** — 最终版 TypeScript interface + 状态定义。
2. **DataModel 映射层代码** — `integrate` 阶段产出的 api.ts / transforms.ts / hooks.ts。
3. **KRPC Protocol Document** — 后端接口定义。
4. **Service Source Code** — 后端实现（用于理解数据生成与查询机制）。
5. **Performance Requirements (Optional)** — 具体性能指标要求（如 P95 延迟 < 200ms）。
6. **Previous Benchmark Report (Optional)** — 历史基线，用于对比回归。

# Output

1. **Benchmark 测试脚本** — 可独立执行的 TypeScript 测试脚本。
2. **数据构造脚本** — 用于在测试环境中生成不同规模的测试数据。
3. **性能报告** — 包含量化指标、瓶颈分析、优化建议的结构化报告。
4. **性能基线记录** — 可供后续版本回归对比的基线数据。

---

# 操作步骤

## Step 1: 测试环境准备

### 1.1 确认测试环境

- 后端服务正常运行（scheduler 拉起、login/heartbeat 正常）。
- KRPC 接口可正常响应。
- 确认测试环境与目标部署环境的差异（记录在报告中）。

### 1.2 创建测试目录

```
tests/
  benchmark/
    setup.ts              ← 环境初始化与清理
    data-generator.ts     ← 测试数据构造
    bench-list.test.ts    ← 列表类接口基准
    bench-detail.test.ts  ← 详情类接口基准
    bench-aggregate.test.ts ← 聚合类接口基准
    bench-concurrent.test.ts ← 并发场景基准
    reporter.ts           ← 报告生成
    results/              ← 测试结果输出目录
```

### 1.3 安装依赖

```bash
pnpm add -D vitest
```

## Step 2: 构造测试数据

### 2.1 数据规模梯度

**MUST** 覆盖以下数据规模：

| 规模等级 | 数量 | 用途 |
|---------|------|------|
| Minimal | 1 条 | 冷启动、边界验证 |
| Small | 10 条 | 日常使用场景 |
| Medium | 100 条 | 正常业务规模 |
| Large | 1,000 条 | 压力场景 |
| XLarge | 10,000 条 | 边界探测 |
| Massive | 1,000,000 条 | 极端场景（如适用） |

### 2.2 数据构造脚本模板

```typescript
// tests/benchmark/data-generator.ts

export interface DataGeneratorConfig {
  count: number;
  /** 是否包含边界值（超长字符串、特殊字符等） */
  includeEdgeCases: boolean;
}

/**
 * 通过 KRPC 接口向后端写入测试数据。
 * 每个服务需要根据实际接口实现具体的写入逻辑。
 */
export async function generateTestData(config: DataGeneratorConfig): Promise<void> {
  const { count, includeEdgeCases } = config;

  for (let i = 0; i < count; i++) {
    // 调用 KRPC 写入接口创建测试数据
    // 示例：await krpcClient.createItem({ ... });
  }
}

/**
 * 清理测试数据。
 */
export async function cleanupTestData(): Promise<void> {
  // 调用 KRPC 接口清理
}
```

### 2.3 数据特征要求

测试数据 **SHOULD**：

- 模拟真实数据分布（非全相同数据）。
- 包含不同状态值的混合（正常、异常、进行中等）。
- 包含不同长度的字符串字段。
- 包含时间跨度（非全部集中在同一秒）。

## Step 3: 编写基准测试

### 3.1 列表类接口基准

```typescript
// tests/benchmark/bench-list.test.ts

import { describe, it, beforeAll, afterAll } from 'vitest';
import { generateTestData, cleanupTestData } from './data-generator';

describe('List API Benchmark', () => {
  // ── 数据规模测试 ──────────────────────────────

  describe('Data Scale', () => {
    it.each([
      { count: 1,     label: 'Minimal' },
      { count: 10,    label: 'Small' },
      { count: 100,   label: 'Medium' },
      { count: 1000,  label: 'Large' },
      { count: 10000, label: 'XLarge' },
    ])('$label ($count items): fetch first page', async ({ count, label }) => {
      await generateTestData({ count, includeEdgeCases: false });

      const metrics = await measureFetch(() => fetchListPage(1, 20));

      recordResult('list-first-page', label, metrics);
      await cleanupTestData();
    });
  });

  // ── 分页访问模式测试 ─────────────────────────

  describe('Pagination Patterns', () => {
    beforeAll(async () => {
      await generateTestData({ count: 1000, includeEdgeCases: false });
    });

    afterAll(async () => {
      await cleanupTestData();
    });

    it.each([
      { page: 1,  label: 'First page' },
      { page: 10, label: 'Page 10' },
      { page: 50, label: 'Page 50 (mid)' },
      { page: 50, label: 'Last page' },
    ])('$label', async ({ page }) => {
      const metrics = await measureFetch(() => fetchListPage(page, 20));
      recordResult('list-pagination', `page-${page}`, metrics);
    });
  });

  // ── 每页大小测试 ──────────────────────────────

  describe('Page Size Impact', () => {
    beforeAll(async () => {
      await generateTestData({ count: 1000, includeEdgeCases: false });
    });

    afterAll(async () => {
      await cleanupTestData();
    });

    it.each([10, 20, 50, 100, 200])('pageSize=%d', async (pageSize) => {
      const metrics = await measureFetch(() => fetchListPage(1, pageSize));
      recordResult('list-pagesize', `size-${pageSize}`, metrics);
    });
  });
});
```

### 3.2 详情类接口基准

```typescript
// tests/benchmark/bench-detail.test.ts

describe('Detail API Benchmark', () => {
  // ── 单条详情获取 ──────────────────────────────

  it('single item detail fetch', async () => {
    const metrics = await measureFetch(() => fetchItemDetail(testItemId));
    recordResult('detail-single', 'single', metrics);
  });

  // ── 批量详情获取（检测 N+1） ──────────────────

  describe('Batch Detail (N+1 detection)', () => {
    it.each([5, 20, 50, 100])('fetch details for %d items', async (count) => {
      const ids = testItemIds.slice(0, count);
      const metrics = await measureFetch(() =>
        Promise.all(ids.map(id => fetchItemDetail(id)))
      );

      recordResult('detail-batch', `batch-${count}`, {
        ...metrics,
        rpcCount: count,  // 每条一次 RPC = N+1
        rpcPerItem: 1,
      });
    });

    // 如果有批量接口，对比测试
    it.each([5, 20, 50, 100])('batch API for %d items (if available)', async (count) => {
      const ids = testItemIds.slice(0, count);
      const metrics = await measureFetch(() => fetchItemsBatch(ids));

      recordResult('detail-batch-api', `batch-${count}`, {
        ...metrics,
        rpcCount: 1,
        rpcPerItem: 1 / count,
      });
    });
  });
});
```

### 3.3 聚合类接口基准

```typescript
// tests/benchmark/bench-aggregate.test.ts

describe('Aggregation Benchmark', () => {
  // ── 客户端聚合 vs 服务端聚合 ──────────────────

  describe('Client-side aggregation cost', () => {
    it.each([100, 1000, 10000])('aggregate %d items on client', async (count) => {
      await generateTestData({ count, includeEdgeCases: false });

      // 拉取全部数据后客户端聚合
      const metrics = await measureFetch(async () => {
        const allItems = await fetchAllItems();
        return computeAggregation(allItems);
      });

      recordResult('aggregate-client', `count-${count}`, metrics);
      await cleanupTestData();
    });
  });

  // ── 如果后端有聚合接口，对比测试 ──────────────

  describe('Server-side aggregation (if available)', () => {
    it.each([100, 1000, 10000])('aggregate %d items on server', async (count) => {
      await generateTestData({ count, includeEdgeCases: false });

      const metrics = await measureFetch(() => fetchAggregation());

      recordResult('aggregate-server', `count-${count}`, metrics);
      await cleanupTestData();
    });
  });
});
```

### 3.4 并发场景基准

```typescript
// tests/benchmark/bench-concurrent.test.ts

describe('Concurrent Access Benchmark', () => {
  beforeAll(async () => {
    await generateTestData({ count: 1000, includeEdgeCases: false });
  });

  afterAll(async () => {
    await cleanupTestData();
  });

  it.each([1, 5, 10, 20])('concurrent=%d list requests', async (concurrency) => {
    const metrics = await measureConcurrent(
      () => fetchListPage(1, 20),
      concurrency,
    );

    recordResult('concurrent-list', `c-${concurrency}`, metrics);
  });
});
```

## Step 4: 度量与报告工具

### 4.1 度量函数

```typescript
// tests/benchmark/setup.ts

export interface BenchmarkMetrics {
  /** 总耗时（ms） */
  totalMs: number;
  /** 最小耗时（多次执行时） */
  minMs: number;
  /** 最大耗时 */
  maxMs: number;
  /** P50 */
  p50Ms: number;
  /** P95 */
  p95Ms: number;
  /** P99 */
  p99Ms: number;
  /** RPC 调用次数 */
  rpcCount: number;
  /** 传输数据量（bytes，如可测量） */
  payloadBytes?: number;
}

/**
 * 度量单次请求的性能指标。
 * 执行 warmupRuns 次预热 + runs 次正式测量。
 */
export async function measureFetch<T>(
  fn: () => Promise<T>,
  options: { runs?: number; warmupRuns?: number } = {},
): Promise<BenchmarkMetrics> {
  const { runs = 5, warmupRuns = 1 } = options;

  // Warmup
  for (let i = 0; i < warmupRuns; i++) {
    await fn();
  }

  // Measure
  const durations: number[] = [];
  for (let i = 0; i < runs; i++) {
    const start = performance.now();
    await fn();
    durations.push(performance.now() - start);
  }

  durations.sort((a, b) => a - b);

  return {
    totalMs: durations.reduce((a, b) => a + b, 0),
    minMs: durations[0],
    maxMs: durations[durations.length - 1],
    p50Ms: percentile(durations, 50),
    p95Ms: percentile(durations, 95),
    p99Ms: percentile(durations, 99),
    rpcCount: 0, // 由具体测试填充
  };
}

/**
 * 度量并发请求的性能指标。
 */
export async function measureConcurrent<T>(
  fn: () => Promise<T>,
  concurrency: number,
  options: { runs?: number } = {},
): Promise<BenchmarkMetrics> {
  const { runs = 3 } = options;
  const durations: number[] = [];

  for (let r = 0; r < runs; r++) {
    const start = performance.now();
    await Promise.all(Array.from({ length: concurrency }, () => fn()));
    durations.push(performance.now() - start);
  }

  durations.sort((a, b) => a - b);

  return {
    totalMs: durations.reduce((a, b) => a + b, 0),
    minMs: durations[0],
    maxMs: durations[durations.length - 1],
    p50Ms: percentile(durations, 50),
    p95Ms: percentile(durations, 95),
    p99Ms: percentile(durations, 99),
    rpcCount: concurrency,
  };
}

function percentile(sorted: number[], p: number): number {
  const idx = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, idx)];
}
```

### 4.2 结果记录与报告生成

```typescript
// tests/benchmark/reporter.ts

interface BenchmarkRecord {
  category: string;
  label: string;
  metrics: BenchmarkMetrics;
  timestamp: string;
}

const results: BenchmarkRecord[] = [];

export function recordResult(
  category: string,
  label: string,
  metrics: BenchmarkMetrics,
): void {
  const record: BenchmarkRecord = {
    category,
    label,
    metrics,
    timestamp: new Date().toISOString(),
  };
  results.push(record);

  // 实时输出到控制台
  console.log(
    `[${category}] ${label}: ` +
    `P50=${metrics.p50Ms.toFixed(0)}ms ` +
    `P95=${metrics.p95Ms.toFixed(0)}ms ` +
    `P99=${metrics.p99Ms.toFixed(0)}ms ` +
    `RPCs=${metrics.rpcCount}`
  );
}

/**
 * 生成 Markdown 格式的性能报告。
 */
export function generateReport(serviceName: string): string {
  const lines: string[] = [
    `# ${serviceName} DataModel Benchmark Report`,
    '',
    `**Date:** ${new Date().toISOString()}`,
    `**Environment:** ${process.env.BENCHMARK_ENV || 'local'}`,
    '',
    '---',
    '',
  ];

  // 按 category 分组
  const grouped = new Map<string, BenchmarkRecord[]>();
  for (const r of results) {
    const list = grouped.get(r.category) ?? [];
    list.push(r);
    grouped.set(r.category, list);
  }

  for (const [category, records] of grouped) {
    lines.push(`## ${category}`, '');
    lines.push('| Label | P50 (ms) | P95 (ms) | P99 (ms) | Min (ms) | Max (ms) | RPCs |');
    lines.push('|-------|----------|----------|----------|----------|----------|------|');

    for (const r of records) {
      const m = r.metrics;
      lines.push(
        `| ${r.label} | ${m.p50Ms.toFixed(0)} | ${m.p95Ms.toFixed(0)} | ` +
        `${m.p99Ms.toFixed(0)} | ${m.minMs.toFixed(0)} | ${m.maxMs.toFixed(0)} | ${m.rpcCount} |`
      );
    }
    lines.push('');
  }

  lines.push('## Summary', '');
  lines.push('### Findings', '');
  lines.push('<!-- AI: 填写性能发现 -->');
  lines.push('');
  lines.push('### Bottlenecks', '');
  lines.push('<!-- AI: 填写瓶颈分析 -->');
  lines.push('');
  lines.push('### Recommendations', '');
  lines.push('<!-- AI: 填写优化建议 -->');
  lines.push('');

  return lines.join('\n');
}
```

## Step 5: 执行基准测试

### 5.1 执行流程

```bash
# 1. 确认后端服务运行正常
# 2. 执行基准测试
pnpm run test:benchmark

# 或使用 vitest 直接执行
npx vitest run tests/benchmark/ --reporter=verbose
```

### 5.2 执行注意事项

- 每次执行前 **MUST** 确认后端服务状态正常。
- 测试期间 **SHOULD NOT** 有其他负载干扰。
- 大规模数据测试（10000+ 条）可能需要较长时间，注意超时设置。
- 如果测试数据通过 KRPC 写入，注意写入本身的耗时不计入读取性能。

## Step 6: 分析结果与产出报告

### 6.1 性能评估标准

| 指标 | 优秀 | 可接受 | 需优化 | 不可接受 |
|------|------|--------|--------|----------|
| 列表首页 P95 | < 100ms | < 300ms | < 1000ms | > 1000ms |
| 详情页 P95 | < 50ms | < 200ms | < 500ms | > 500ms |
| 翻页 P95 | < 200ms | < 500ms | < 1000ms | > 1000ms |
| 单页 RPC 次数 | 1-2 | 3-5 | 6-10 | > 10 |
| 客户端聚合 | < 50ms | < 200ms | < 500ms | > 500ms |

注意：以上为参考值，实际阈值应根据服务特点和产品要求调整。

### 6.2 瓶颈分析维度

对每个超出"可接受"标准的测试项，分析：

1. **瓶颈在哪一层？**
   - 网络传输（payload 过大）
   - 后端处理（查询慢）
   - 数据转换（transforms 计算量大）
   - RPC 次数过多（N+1 或扇出）

2. **是否随数据量线性增长？**
   - 线性：可能是遍历/全量查询
   - 超线性：可能是嵌套查询或笛卡尔积

3. **是否有优化空间？**
   - 缓存
   - 批量接口
   - 分页策略调整
   - 聚合下推到后端

### 6.3 报告结构

最终报告 **MUST** 包含以下章节：

```markdown
# [Service Name] DataModel Benchmark Report

## 1. Test Environment
- 硬件/环境描述
- 服务版本
- 测试时间

## 2. Results Summary
- 各类别测试结果表格（由 reporter.ts 生成）

## 3. Findings
- 关键发现（正面与负面）

## 4. Bottleneck Analysis
- 超标项的逐一分析

## 5. Recommendations
- 优化建议（分优先级）
- 是否需要 DataModel 变更（MUST 标注需人工确认）
- 是否需要 KRPC 接口调整（MUST 标注需与后端协商）

## 6. Baseline
- 本次测试作为基线的关键指标
- 与历史基线的对比（如有）

## 7. Raw Data
- 完整测试数据（JSON）
```

### 6.4 基线存档

每次基准测试的结果 **SHOULD** 存档到 `tests/benchmark/results/` 目录：

```
results/
  baseline-2026-03-31.json    ← 关键指标 JSON
  report-2026-03-31.md        ← 完整报告
```

---

# 回归对比

当 DataModel 发生变更后，重新执行基准测试并与历史基线对比：

```typescript
// 加载历史基线
const baseline = loadBaseline('baseline-2026-03-31.json');

// 对比当前结果
for (const [key, current] of currentResults) {
  const prev = baseline[key];
  if (prev) {
    const regression = ((current.p95Ms - prev.p95Ms) / prev.p95Ms) * 100;
    if (regression > 20) {
      console.warn(`⚠ REGRESSION: ${key} P95 increased ${regression.toFixed(0)}%`);
    }
  }
}
```

回归阈值参考：

- P95 增长 > 20%：警告，需调查。
- P95 增长 > 50%：阻塞，MUST 修复后才能继续。
- RPC 次数增加：逐一审查原因。

---

# Common Failure Modes

## 1. 测试数据不具代表性

**症状**: 基准测试结果乐观，上线后性能差。
**原因**: 测试数据全部相同（如所有 name 都是 "test"），无法触发真实查询路径。
**修复**: 使用多样化数据（不同长度、不同状态、不同时间分布）。

## 2. 未隔离测试环境

**症状**: 同一测试多次执行结果差异巨大（> 30%）。
**原因**: 测试期间有其他服务或进程在消耗资源。
**修复**: 尽量隔离测试环境；无法隔离时增加执行次数并取中位数。

## 3. 忽略冷启动

**症状**: 首次请求延迟远高于后续请求，但报告中只体现了预热后的数据。
**原因**: 预热轮数设置过多，掩盖了冷启动问题。
**修复**: 分别记录冷启动和预热后的指标。

## 4. 只测 Happy Path

**症状**: 正常数据下性能良好，边界数据下崩溃。
**原因**: 未测试空列表、超长字符串、特殊字符等边界情况。
**修复**: 数据构造脚本 MUST 包含 `includeEdgeCases` 选项。

## 5. 大规模数据构造失败

**症状**: 10000+ 条数据写入超时或失败。
**原因**: 逐条写入太慢，或后端无批量写入接口。
**修复**: 使用批量写入；或直接操作数据存储层构造数据（需记录在报告中）。

## 6. 基线丢失无法回归

**症状**: 无法判断性能变化是优化还是退化。
**原因**: 历史测试结果未存档。
**修复**: 每次执行后自动存档到 `results/` 目录。

## 7. 并发测试与实际使用模式不符

**症状**: 并发测试通过但实际使用中出现争用。
**原因**: 测试中所有并发请求访问相同数据，实际场景可能有写-读竞争。
**修复**: 设计混合读写的并发测试场景。

---

# AI 行为规则

1. **数据构造与测试执行自主完成。** 编写脚本、构造数据、运行测试——不需要问人。
2. **报告分析自主完成。** 基于数据产出瓶颈分析和优化建议——不需要问人。
3. **优化建议涉及 DataModel 变更时必须问人。** 提出建议并说明原因，等人工确认。
4. **优化建议涉及 KRPC 变更时必须问人。** 提出建议但 MUST NOT 自行修改后端代码。
5. **如实报告。** 不美化数据，不隐藏不理想的结果。
6. **记录环境与条件。** 报告必须包含测试环境描述，使结果可复现。
7. **不碰后端业务逻辑。** 数据构造只通过 KRPC 接口或经人工确认的方式进行。

---

# Pass Criteria

本 Skill 全部完成的标志：

- [ ] 数据构造脚本已实现，可生成不同规模的测试数据。
- [ ] 列表类接口基准测试已编写并执行（覆盖 1 / 10 / 100 / 1000 / 10000 条）。
- [ ] 分页访问模式基准测试已执行（首页 / 中间页 / 末页）。
- [ ] 每页大小影响测试已执行。
- [ ] 详情类接口基准测试已执行（含 N+1 检测）。
- [ ] 聚合类接口基准测试已执行（客户端 vs 服务端对比，如适用）。
- [ ] 并发场景基准测试已执行。
- [ ] 性能报告已产出，包含量化指标、瓶颈分析、优化建议。
- [ ] 所有"不可接受"级别的性能问题已解决或已记录 tradeoff（经人工确认）。
- [ ] 基线数据已存档到 `results/` 目录。
- [ ] 报告可交付人工 review 性能结论与 tradeoff。
