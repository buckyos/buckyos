/* ── Mock Agent: deterministic, offline, same CanvasPatch protocol as the real adapter (PRD FR-AGENT-005 / §13.6) ── */

import type { CanvasBlock, CellPrimitive, ChartBlockContent, MetricBlockContent, TableBlockContent } from '../domain/types'
import { tableContentFromMatrix } from '../domain/factories'
import { formatNumber } from '../domain/selectors'
import { detectAigc, generateAigc } from './aigc'
import { mk, type Layout, type MockResult } from './mock-blocks'
import {
  AgentRunError,
  type AgentContextItem,
  type AgentRunEvent,
  type AgentStage,
  type AgentRunRequest,
  type CanvasAgentAdapter,
  type CanvasPatch,
  type CanvasPatchOperation,
} from './contracts'

export type MockDebugMode = 'normal' | 'fail' | 'invalid_patch' | 'slow' | 'timeout'

export interface MockAgentOptions {
  debugMode: MockDebugMode
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) return reject(new AgentRunError('cancelled', '已取消'))
    const t = setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    const onAbort = () => {
      clearTimeout(t)
      reject(new AgentRunError('cancelled', '已取消'))
    }
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

type Row = Record<string, CellPrimitive>

function num(v: CellPrimitive): number {
  if (typeof v === 'number') return v
  if (typeof v === 'string') {
    const n = Number(v.replace(/[,¥$%\s]/g, ''))
    return Number.isFinite(n) ? n : 0
  }
  return 0
}

function findCol(cols: string[], ...names: string[]): string | undefined {
  for (const n of names) {
    const hit = cols.find((c) => c === n) ?? cols.find((c) => c.includes(n))
    if (hit) return hit
  }
  return undefined
}

function groupBy(rows: Row[], key: string): Map<string, Row[]> {
  const m = new Map<string, Row[]>()
  for (const r of rows) {
    const k = String(r[key] ?? '')
    if (!m.has(k)) m.set(k, [])
    m.get(k)!.push(r)
  }
  return m
}

function sum(rows: Row[], key: string): number {
  return rows.reduce((a, r) => a + num(r[key]), 0)
}

function metric(layout: Layout, i: number, content: MetricBlockContent): CanvasBlock {
  return mk(layout, 'metric', content.label, { x: i * 246, y: 0, width: 230, height: 120 }, content)
}

function tableFromMatrix(matrix: Array<Array<string | number>>): TableBlockContent {
  return tableContentFromMatrix(matrix, { hasHeader: true, source: { kind: 'manual' } })
}

/* ── quarterly sales analysis (PRD §13.6) ── */
function analyzeSales(item: Extract<AgentContextItem, { kind: 'table' }>, req: AgentRunRequest, layout: Layout) {
  const cols = item.columns.map((c) => c.name)
  const rev = findCol(cols, '销售额', '营收', '收入')!
  const cost = findCol(cols, '成本')
  const target = findCol(cols, '目标')
  const prev = findCol(cols, '上季度', '上期', '去年')
  const region = findCol(cols, '区域', '地区', '大区')
  const product = findCol(cols, '产品', '品类', '商品')
  const rows = item.rows
  const totalRev = sum(rows, rev)
  const totalCost = cost ? sum(rows, cost) : 0
  const totalTarget = target ? sum(rows, target) : 0
  const totalPrev = prev ? sum(rows, prev) : 0
  const margin = totalRev ? (totalRev - totalCost) / totalRev : 0
  const completion = totalTarget ? totalRev / totalTarget : 0
  const growth = totalPrev ? (totalRev - totalPrev) / totalPrev : 0
  const warnings: string[] = []
  const assumptions: string[] = ['毛利率 = (销售额 − 成本) / 销售额', '目标完成率 = 销售额 / 目标']
  if (!cost) warnings.push('未找到"成本"列，毛利率按 0 计算')
  if (!target) warnings.push('未找到"目标"列，无法计算目标完成率')
  if (prev) assumptions.push('增长率以"上季度销售额"列为基准')

  const blocks: CanvasBlock[] = []
  const pref = req.wish.outputPreference

  if (pref === 'auto' || pref === 'visual') {
    blocks.push(
      metric(layout, 0, { label: '本季度销售额', value: totalRev, format: 'currency', delta: prev ? { value: growth, label: '环比', format: 'percent' } : undefined, tone: growth >= 0 ? 'positive' : 'negative' }),
      metric(layout, 1, { label: '整体毛利率', value: margin, format: 'percent', tone: margin >= 0.3 ? 'positive' : 'warning', note: cost ? `成本合计 ${formatNumber(totalCost, 'currency')}` : undefined }),
      metric(layout, 2, { label: '目标完成率', value: completion, format: 'percent', tone: completion >= 1 ? 'positive' : completion >= 0.85 ? 'warning' : 'negative', note: target ? `目标 ${formatNumber(totalTarget, 'currency')}` : undefined }),
    )
  }

  // region breakdown
  const regionRows: Array<Array<string | number>> = []
  if (region) {
    for (const [name, rs] of groupBy(rows, region)) {
      const r = sum(rs, rev)
      const p = prev ? sum(rs, prev) : 0
      const t = target ? sum(rs, target) : 0
      regionRows.push([name, r, prev ? (p ? (r - p) / p : 0) : 0, target ? (t ? r / t : 0) : 0])
    }
  }
  if ((pref === 'auto' || pref === 'visual') && region) {
    const chart: ChartBlockContent = {
      chartType: 'bar',
      data: { kind: 'inline', rows: regionRows.map((r) => ({ 区域: r[0], 销售额: r[1], 目标: 0 })) },
      xField: '区域',
      yFields: ['销售额'],
      aggregation: 'sum',
      numberFormat: 'currency',
      caption: '各区域本季度销售额对比',
    }
    if (target) {
      const byRegion = groupBy(rows, region)
      chart.data = { kind: 'inline', rows: [...byRegion.entries()].map(([name, rs]) => ({ 区域: name, 销售额: sum(rs, rev), 目标: sum(rs, target) })) }
      chart.yFields = ['销售额', '目标']
    }
    blocks.push(mk(layout, 'chart', '区域销售额对比', { x: 0, y: 140, width: 480, height: 320 }, chart))
  }

  if ((pref === 'auto' || pref === 'table') && product) {
    const matrix: Array<Array<string | number>> = [['产品', '销售额', '成本', '毛利率', '目标完成率']]
    for (const [name, rs] of groupBy(rows, product)) {
      const r = sum(rs, rev)
      const c = cost ? sum(rs, cost) : 0
      const t = target ? sum(rs, target) : 0
      matrix.push([name, r, c, r ? `${(((r - c) / r) * 100).toFixed(1)}%` : '0%', t ? `${((r / t) * 100).toFixed(1)}%` : '—'])
    }
    blocks.push(mk(layout, 'table', '产品毛利率', { x: pref === 'table' ? 0 : 500, y: pref === 'table' ? 0 : 140, width: 480, height: 320 }, tableFromMatrix(matrix)))
    if (pref === 'table' && region) {
      const rm: Array<Array<string | number>> = [['区域', '销售额', '环比增长', '目标完成率'], ...regionRows.map((r) => [r[0], r[1], `${(num(r[2]) * 100).toFixed(1)}%`, `${(num(r[3]) * 100).toFixed(1)}%`])]
      blocks.push(mk(layout, 'table', '区域汇总', { x: 500, y: 0, width: 480, height: 320 }, tableFromMatrix(rm)))
    }
  }

  // growth ranking per row (region × product)
  const ranked = rows
    .map((r) => {
      const p = prev ? num(r[prev]) : 0
      const g = p ? (num(r[rev]) - p) / p : 0
      const label = [region ? r[region] : null, product ? r[product] : null].filter(Boolean).join(' · ')
      return { label, growth: g, rev: num(r[rev]), cost: cost ? num(r[cost]) : 0, target: target ? num(r[target]) : 0, row: r }
    })
    .sort((a, b) => b.growth - a.growth)
  const top = ranked.slice(0, 3)
  const bottom = [...ranked].reverse().slice(0, 3)
  const anomalies = ranked.filter((x) => (cost && x.cost > x.rev) || (target && x.target && x.rev / x.target < 0.7) || x.growth < -0.1)
  const regionBest = regionRows.length ? [...regionRows].sort((a, b) => num(b[2]) - num(a[2])) : []

  if (pref === 'auto' || pref === 'brief') {
    const lines = [
      `## 本季度经营总结`,
      `本季度总销售额 **${formatNumber(totalRev, 'currency')}**${prev ? `，环比${growth >= 0 ? '增长' : '下降'} ${(Math.abs(growth) * 100).toFixed(1)}%` : ''}${cost ? `，整体毛利率 ${(margin * 100).toFixed(1)}%` : ''}${target ? `，目标完成率 ${(completion * 100).toFixed(1)}%` : ''}。`,
      regionBest.length ? `- 增长最快的区域是 **${regionBest[0][0]}**（环比 ${(num(regionBest[0][2]) * 100).toFixed(1)}%），最弱的是 **${regionBest[regionBest.length - 1][0]}**（环比 ${(num(regionBest[regionBest.length - 1][2]) * 100).toFixed(1)}%）。` : '',
      top.length ? `- 增长最快的三项：${top.map((t) => `${t.label}（${(t.growth * 100).toFixed(1)}%）`).join('、')}。` : '',
      bottom.length ? `- 下滑最明显的三项：${bottom.map((t) => `${t.label}（${(t.growth * 100).toFixed(1)}%）`).join('、')}。` : '',
      anomalies.length ? `- 发现 **${anomalies.length}** 条需要关注的异常记录（负毛利、远低于目标或明显下滑），详见异常明细表。` : '- 未发现明显异常记录。',
      `\n建议：优先复盘下滑项的成本结构与目标设定，对增长项追加库存与渠道投入。`,
    ].filter(Boolean)
    blocks.push(mk(layout, 'text', '管理层总结', { x: 0, y: pref === 'brief' ? 0 : 480, width: 480, height: 260 }, { text: lines.join('\n'), format: 'markdown' }))
  }

  if (pref === 'auto' && anomalies.length) {
    const matrix: Array<Array<string | number>> = [['项目', '销售额', '目标完成率', '环比', '问题']]
    for (const a of anomalies) {
      const issues = []
      if (cost && a.cost > a.rev) issues.push('负毛利')
      if (target && a.target && a.rev / a.target < 0.7) issues.push('远低于目标')
      if (a.growth < -0.1) issues.push('明显下滑')
      matrix.push([a.label, a.rev, a.target ? `${((a.rev / a.target) * 100).toFixed(0)}%` : '—', `${(a.growth * 100).toFixed(1)}%`, issues.join('、')])
    }
    blocks.push(mk(layout, 'table', '异常明细', { x: 500, y: 480, width: 480, height: 260 }, tableFromMatrix(matrix)))
    warnings.push(`${anomalies.length} 条记录存在负毛利、远低于目标或明显下滑`)
  }
  return { blocks, warnings, assumptions, summary: `季度经营分析：销售额 ${formatNumber(totalRev, 'currency')}，毛利率 ${(margin * 100).toFixed(1)}%` }
}

/* ── generic numeric table ── */
function analyzeGeneric(item: Extract<AgentContextItem, { kind: 'table' }>, req: AgentRunRequest, layout: Layout) {
  const numericCols = item.columns.filter((c) => c.type === 'number').map((c) => c.name)
  const textCols = item.columns.filter((c) => c.type === 'string').map((c) => c.name)
  const rows = item.rows
  const blocks: CanvasBlock[] = []
  const pref = req.wish.outputPreference
  const warnings: string[] = []
  if (item.truncated) warnings.push(`表格较大，仅分析了前 ${rows.length} 行`)
  if (numericCols.length === 0) warnings.push('未找到数值列，仅生成文字摘要')

  if ((pref === 'auto' || pref === 'visual') && numericCols.length) {
    numericCols.slice(0, 3).forEach((c, i) => {
      const s = sum(rows, c)
      blocks.push(metric(layout, i, { label: `${c} 合计`, value: s, note: `平均 ${formatNumber(rows.length ? s / rows.length : 0)}` }))
    })
  }
  if ((pref === 'auto' || pref === 'visual') && numericCols.length && textCols.length) {
    const groups = [...groupBy(rows, textCols[0]).entries()].map(([k, rs]) => ({ [textCols[0]]: k, [numericCols[0]]: sum(rs, numericCols[0]) }))
    groups.sort((a, b) => num(b[numericCols[0]]) - num(a[numericCols[0]]))
    blocks.push(
      mk(layout, 'chart', `${numericCols[0]} 按 ${textCols[0]}`, { x: 0, y: 140, width: 480, height: 320 }, {
        chartType: groups.length > 8 ? 'horizontalBar' : 'bar',
        data: { kind: 'inline', rows: groups.slice(0, 12) },
        xField: textCols[0],
        yFields: [numericCols[0]],
        aggregation: 'sum',
        caption: `${textCols[0]} 维度的 ${numericCols[0]} 汇总（前 12 项）`,
      }),
    )
  }
  if ((pref === 'auto' || pref === 'table') && textCols.length && numericCols.length) {
    const matrix: Array<Array<string | number>> = [[textCols[0], ...numericCols.slice(0, 4).map((c) => `${c} 合计`), '记录数']]
    for (const [k, rs] of groupBy(rows, textCols[0])) {
      matrix.push([k, ...numericCols.slice(0, 4).map((c) => sum(rs, c)), rs.length])
    }
    blocks.push(mk(layout, 'table', `按 ${textCols[0]} 汇总`, { x: pref === 'table' ? 0 : 500, y: pref === 'table' ? 0 : 140, width: 480, height: 320 }, tableFromMatrix(matrix.slice(0, 200))))
  }
  if (pref === 'auto' || pref === 'brief' || blocks.length === 0) {
    const stats = numericCols.slice(0, 4).map((c) => {
      const vals = rows.map((r) => num(r[c]))
      const s = vals.reduce((a, b) => a + b, 0)
      return `- **${c}**：合计 ${formatNumber(s)}，最大 ${formatNumber(Math.max(...vals))}，最小 ${formatNumber(Math.min(...vals))}`
    })
    const text = [
      `## ${item.title} 摘要`,
      `共 ${item.totalRows} 行、${item.columns.length} 列（${numericCols.length} 个数值列，${textCols.length} 个文本列）。`,
      ...stats,
      `\n你的目标：${req.wish.prompt.slice(0, 120)}`,
    ].join('\n')
    blocks.push(mk(layout, 'text', '数据摘要', { x: 0, y: pref === 'brief' || blocks.length === 0 ? 0 : 480, width: 480, height: 240 }, { text, format: 'markdown' }))
  }
  return { blocks, warnings, assumptions: ['数值列按合计汇总'], summary: `${item.title} 基础汇总` }
}

function analyzeText(items: AgentContextItem[], req: AgentRunRequest, layout: Layout) {
  const texts = items.map((it) => (it.kind === 'text' ? it.text : it.kind === 'other' || it.kind === 'group' ? it.summary : it.kind === 'metric' ? `${it.label} ${it.value}` : it.kind === 'image' ? `[图片] ${it.title} ${it.alt ?? ''}` : ''))
  const joined = texts.join('\n')
  const sentences = joined.split(/[。！？\n]/).map((s) => s.trim()).filter((s) => s.length > 6)
  const text = [
    `## 内容摘要`,
    `共引用 ${items.length} 个来源，约 ${joined.length} 字。`,
    ...sentences.slice(0, 4).map((s) => `- ${s.replace(/^#+\s*/, '')}`),
    `\n针对目标「${req.wish.prompt.slice(0, 60)}」，以上要点可作为回答的骨架。`,
  ].join('\n')
  return {
    blocks: [mk(layout, 'text', '摘要', { x: 0, y: 0, width: 480, height: 240 }, { text, format: 'markdown' })],
    warnings: [] as string[],
    assumptions: ['摘要基于来源文本的前几句'],
    summary: '文本摘要',
  }
}

function isSalesTable(item: Extract<AgentContextItem, { kind: 'table' }>): boolean {
  const cols = item.columns.map((c) => c.name)
  return Boolean(findCol(cols, '销售额', '营收', '收入')) && Boolean(findCol(cols, '区域', '产品', '地区', '品类'))
}

/* ── table AI cell ── */
function analyzeCell(req: AgentRunRequest): { ops: CanvasPatchOperation[]; summary: string } {
  const cell = req.cell!
  const table = req.context.find((c): c is Extract<AgentContextItem, { kind: 'table' }> => c.kind === 'table' && c.blockId === cell.tableBlockId)
  const row = table?.rows[0] ?? {}
  const prompt = req.wish.prompt
  const cols = table?.columns.map((c) => c.name) ?? []
  const rev = findCol(cols, '销售额', '营收', '收入')
  const cost = findCol(cols, '成本')
  const target = findCol(cols, '目标')
  let value: CellPrimitive
  let display: string
  if (/毛利率|利润率/.test(prompt) && rev && cost) {
    const r = num(row[rev])
    value = r ? (r - num(row[cost])) / r : 0
    display = `${(value * 100).toFixed(1)}%`
  } else if (/利润|毛利/.test(prompt) && rev && cost) {
    value = num(row[rev]) - num(row[cost])
    display = formatNumber(value)
  } else if (/完成率|达成/.test(prompt) && rev && target) {
    value = num(row[target]) ? num(row[rev]) / num(row[target]) : 0
    display = `${(value * 100).toFixed(1)}%`
  } else if (/总结|摘要|评价|说明/.test(prompt)) {
    const parts = Object.entries(row).slice(0, 4).map(([k, v]) => `${k} ${v ?? ''}`)
    value = `${parts.join('，')}`
    display = value
  } else {
    const nums = Object.values(row).map(num).filter((n) => n !== 0)
    value = nums.reduce((a, b) => a + b, 0)
    display = formatNumber(value)
  }
  return {
    ops: [
      {
        op: 'updateTableCells',
        blockId: cell.tableBlockId,
        cells: [{ rowId: cell.rowId, columnId: cell.columnId, cell: { kind: 'ai', wishId: req.wish.blockId, value, displayValue: display, valueType: typeof value === 'number' ? 'number' : 'string' } }],
      },
    ],
    summary: `AI 单元格：${display}`,
  }
}

export class MockCanvasAgentAdapter implements CanvasAgentAdapter {
  id = 'mock'
  private readonly getOptions: () => MockAgentOptions
  constructor(getOptions: () => MockAgentOptions) {
    this.getOptions = getOptions
  }

  async health() {
    return { available: true, message: 'Mock Agent（离线）' }
  }

  async run(request: AgentRunRequest, onEvent: (e: AgentRunEvent) => void, signal: AbortSignal): Promise<CanvasPatch> {
    const prompt = request.wish.prompt
    const opt = this.getOptions()
    const mode: MockDebugMode = /#fail/.test(prompt) ? 'fail' : /#invalid/.test(prompt) ? 'invalid_patch' : /#slow/.test(prompt) ? 'slow' : /#timeout/.test(prompt) ? 'timeout' : opt.debugMode
    const slow = mode === 'slow' ? 4 : 1
    const step = async (stage: AgentStage, message: string, ms: number, percent?: number) => {
      await sleep(ms * slow, signal)
      if (percent === undefined) onEvent({ type: 'status', stage, message })
      else onEvent({ type: 'progress', stage, percent, message })
    }

    await step('planning', '正在理解目标', 350)
    onEvent({ type: 'log', message: `收到 ${request.context.length} 个数据来源，输出偏好 ${request.wish.outputPreference}` })
    await step('running', '正在检查数据', 400, 15)
    if (mode === 'timeout') {
      await sleep(10 * 60 * 1000, signal)
    }
    if (mode === 'fail') {
      await sleep(400, signal)
      throw new AgentRunError('failed', 'Mock Agent 演示失败：模型服务返回错误（示例）', ['这是 #fail 调试模式触发的固定失败'])
    }

    const layout: Layout = { sheetId: request.destination.sheetId, x: request.destination.anchor.x, y: request.destination.anchor.y, runId: request.runId, n: 0 }
    let ops: CanvasPatchOperation[] = []
    let warnings: string[] = []
    let assumptions: string[] = []
    let summary = ''

    if (request.cell) {
      await step('running', '正在计算单元格', 300, 60)
      const r = analyzeCell(request)
      ops = r.ops
      summary = r.summary
    } else {
      const tables = request.context.filter((c): c is Extract<AgentContextItem, { kind: 'table' }> => c.kind === 'table')
      const aigc = detectAigc(request)
      await step('running', aigc ? '正在生成画面' : '正在生成分析结果', 500, 45)
      let result: MockResult
      if (aigc) {
        onEvent({ type: 'log', message: `识别为 AIGC 任务：${aigc}` })
        result = generateAigc(aigc, request, layout)
      } else if (tables.length && isSalesTable(tables[0])) result = analyzeSales(tables[0], request, layout)
      else if (tables.length) result = analyzeGeneric(tables[0], request, layout)
      else if (request.context.length) result = analyzeText(request.context, request, layout)
      else {
        result = {
          blocks: [mk(layout, 'text', '说明', { x: 0, y: 0, width: 420, height: 160 }, { text: `没有选择数据来源，Mock Agent 只能基于目标生成说明：\n\n> ${prompt}\n\n请在许愿格中添加表格或文本作为数据来源后重新运行。`, format: 'markdown' })],
          warnings: ['没有数据来源'],
          assumptions: [],
          summary: '缺少数据来源',
        }
      }
      await step('running', aigc ? '正在整理输出' : '正在创建图表', 350, 80)
      for (const w of result.warnings) onEvent({ type: 'warning', message: w })
      const group = mk(layout, 'group', result.summary.slice(0, 40), { x: 0, y: 0, width: 10, height: 10 }, { childBlockIds: result.blocks.map((b) => b.id), summary: result.summary })
      ops = [...result.blocks.map((b): CanvasPatchOperation => ({ op: 'createBlock', block: b })), { op: 'createBlock', block: group }, { op: 'resizeToFit', blockId: group.id }]
      warnings = result.warnings
      assumptions = result.assumptions
      summary = result.summary
      if (mode === 'invalid_patch') {
        ops.push({ op: 'updateBlock', blockId: 'blk_does_not_exist', patch: { title: '无效引用' } })
      }
    }

    await step('validating', '正在校验结果', 250)
    onEvent({ type: 'completed', jobId: `mock_${request.runId}` })
    return {
      protocolVersion: '0.1',
      runId: request.runId,
      baseCanvasRevision: request.canvas.revision,
      summary,
      assumptions,
      warnings,
      operations: ops,
    }
  }
}
