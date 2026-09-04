/* ── Resolve chart content into categories/series (host-rendered, declarative) ── */

import { cellNumber, tableToRecords } from '../domain/selectors'
import type { CanvasDocument, CellPrimitive, ChartBlockContent } from '../domain/types'

export interface ChartSeriesData {
  categories: string[]
  series: Array<{ name: string; values: number[] }>
  fields: string[]
  numericFields: string[]
}

function toNum(v: CellPrimitive): number | null {
  return cellNumber({ kind: 'value', value: v, valueType: typeof v === 'number' ? 'number' : 'string' })
}

export function chartRows(doc: CanvasDocument, content: ChartBlockContent): Record<string, CellPrimitive>[] {
  if (content.data.kind === 'inline') return content.data.rows
  const t = doc.blocks[content.data.blockId]
  if (!t || t.type !== 'table') return []
  return tableToRecords(t.content)
}

export function resolveChart(doc: CanvasDocument, content: ChartBlockContent): ChartSeriesData {
  const rows = chartRows(doc, content)
  const fields = rows.length ? Object.keys(rows[0]) : []
  const numericFields = fields.filter((f) => rows.some((r) => toNum(r[f]) !== null && typeof r[f] !== 'string') || rows.every((r) => toNum(r[f]) !== null))
  const xField = content.xField && fields.includes(content.xField) ? content.xField : fields.find((f) => !numericFields.includes(f)) ?? fields[0]
  const yFields = (content.yFields ?? []).filter((f) => fields.includes(f))
  const ys = yFields.length ? yFields : numericFields.filter((f) => f !== xField).slice(0, 1)
  const agg = content.aggregation ?? 'sum'
  const groups = new Map<string, Record<string, number[]>>()
  for (const r of rows) {
    const key = String(r[xField] ?? '')
    if (!groups.has(key)) groups.set(key, {})
    const g = groups.get(key)!
    for (const y of ys) {
      const n = toNum(r[y])
      if (n === null) continue
      ;(g[y] ??= []).push(n)
    }
  }
  const reduce = (vals: number[] = []) => {
    if (!vals.length) return 0
    if (agg === 'count') return vals.length
    if (agg === 'avg') return vals.reduce((a, b) => a + b, 0) / vals.length
    if (agg === 'min') return Math.min(...vals)
    if (agg === 'max') return Math.max(...vals)
    return vals.reduce((a, b) => a + b, 0)
  }
  let categories = [...groups.keys()]
  const series = ys.map((y) => ({ name: y, values: categories.map((c) => reduce(groups.get(c)![y])) }))
  if (content.sort && series.length) {
    const s = series.find((x) => x.name === content.sort!.field) ?? series[0]
    const order = categories.map((c, i) => ({ c, v: s.values[i] })).sort((a, b) => (content.sort!.direction === 'asc' ? a.v - b.v : b.v - a.v))
    const idx = order.map((o) => categories.indexOf(o.c))
    categories = idx.map((i) => categories[i])
    for (const ser of series) ser.values = idx.map((i) => ser.values[i])
  }
  return { categories, series, fields, numericFields }
}
