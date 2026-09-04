/* ── Shared helpers for offline generators: deterministic block ids and relative layout ── */

import type { CanvasBlock } from '../domain/types'

export interface Layout {
  sheetId: string
  x: number
  y: number
  runId: string
  n: number
}

export interface MockResult {
  blocks: CanvasBlock[]
  warnings: string[]
  assumptions: string[]
  summary: string
}

export function mk(layout: Layout, type: CanvasBlock['type'], title: string, rect: { x: number; y: number; width: number; height: number }, content: CanvasBlock['content']): CanvasBlock {
  layout.n += 1
  const ts = new Date().toISOString()
  return {
    id: `${layout.runId}_b${layout.n}`,
    sheetId: layout.sheetId,
    type,
    title,
    rect: { x: layout.x + rect.x, y: layout.y + rect.y, width: rect.width, height: rect.height },
    zIndex: 2,
    locked: false,
    contentRevision: 0,
    dataRevision: 0,
    content,
    createdAt: ts,
    updatedAt: ts,
  } as CanvasBlock
}

/** Stable 32-bit hash (FNV-1a) for seeding deterministic "generation". */
export function hash(text: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i)
    h = Math.imul(h, 0x01000193) >>> 0
  }
  return h >>> 0
}

/** Tiny seeded PRNG (mulberry32) so the same prompt always draws the same picture. */
export function rng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
