/* Where the pages go (PDF 导入方案 §3): each page at its own proportions, the long side of a landscape page (the
 * height of a portrait one) 1920 world units; pages left to right, top to bottom in a grid of
 * `clamp(ceil(√n), 1, 8)` columns, 160 apart; the stage takes the commonest page proportions. */

import type { Point } from '../import-drawio/geometry.ts'

export const PAGE_EDGE = 1920
export const PAGE_GAP = 160

/** World units per point for a page of `width` × `height` points. */
export function pageScale(width: number, height: number): number {
  return PAGE_EDGE / Math.max(1, width >= height ? width : height)
}

export function defaultColumns(n: number): number {
  return Math.min(8, Math.max(1, Math.ceil(Math.sqrt(n))))
}

/** Top-left world points of pages of these sizes, in a grid of `cols` columns (cells as large as the largest page). */
export function gridOrigins(sizes: { w: number; h: number }[], cols = defaultColumns(sizes.length), gap = PAGE_GAP): Point[] {
  const cellW = Math.max(1, ...sizes.map((s) => s.w))
  const cellH = Math.max(1, ...sizes.map((s) => s.h))
  return sizes.map((_, i) => ({ x: (i % cols) * (cellW + gap), y: Math.floor(i / cols) * (cellH + gap) }))
}

/** The stage of the presentation path: 1920 wide at the commonest page proportions (16 … 16384, the core's range). */
export function stageOf(sizes: { w: number; h: number }[]): { w: number; h: number } {
  const counts = new Map<string, { n: number; ratio: number }>()
  for (const s of sizes) {
    const ratio = s.h / Math.max(1, s.w)
    const key = ratio.toFixed(2)
    const e = counts.get(key) ?? { n: 0, ratio }
    e.n += 1
    counts.set(key, e)
  }
  const ratio = [...counts.values()].sort((a, b) => b.n - a.n)[0]?.ratio ?? 9 / 16
  return { w: 1920, h: Math.min(16384, Math.max(16, Math.round(1920 * ratio))) }
}
