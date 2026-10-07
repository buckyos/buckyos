/* Spatial index (phase two §9.3 rule 2): a uniform grid over the world-coordinate bounding boxes
 * of a Surface's Blocks, for visibility culling, hit testing, marquee selection and snapping. Hit
 * tests are geometric — no DOM event per Block. Self-implemented (no new dependency). */

import { containsPoint } from '../geometry'
import { intersects, type Rect } from './camera'

/** `rect` is the axis-aligned box the grid indexes; a rotated Block also gives its own rect and rotation
 * (`turned`), and a point hits it only inside the rotated shape. */
export interface Item { id: string; rect: Rect; /** Stacking rank (BlockTree pre-order, higher on top): the paint order. */ paint: number; turned?: { rect: Rect; rotation: number } }

const CELL = 512

function cellRange(rect: Rect): [number, number, number, number] {
  return [Math.floor(rect.x / CELL), Math.floor(rect.y / CELL), Math.floor((rect.x + rect.w) / CELL), Math.floor((rect.y + rect.h) / CELL)]
}

export class SpatialIndex {
  private readonly cells = new Map<string, Set<string>>()
  private readonly items = new Map<string, Item>()

  clear() { this.cells.clear(); this.items.clear() }

  size(): number { return this.items.size }

  get(id: string): Item | undefined { return this.items.get(id) }

  insert(item: Item) {
    this.remove(item.id)
    this.items.set(item.id, item)
    const [x1, y1, x2, y2] = cellRange(item.rect)
    for (let cx = x1; cx <= x2; cx++) for (let cy = y1; cy <= y2; cy++) {
      const key = `${cx}:${cy}`
      let set = this.cells.get(key)
      if (!set) { set = new Set(); this.cells.set(key, set) }
      set.add(item.id)
    }
  }

  remove(id: string) {
    const item = this.items.get(id)
    if (!item) return
    const [x1, y1, x2, y2] = cellRange(item.rect)
    for (let cx = x1; cx <= x2; cx++) for (let cy = y1; cy <= y2; cy++) {
      const key = `${cx}:${cy}`
      const set = this.cells.get(key)
      if (set) { set.delete(id); if (set.size === 0) this.cells.delete(key) }
    }
    this.items.delete(id)
  }

  /** Ids whose rect intersects `rect`. */
  query(rect: Rect): Item[] {
    const [x1, y1, x2, y2] = cellRange(rect)
    const seen = new Set<string>()
    const out: Item[] = []
    // a huge query (zoomed far out) is cheaper as a scan
    if ((x2 - x1 + 1) * (y2 - y1 + 1) > this.items.size) {
      for (const item of this.items.values()) if (intersects(item.rect, rect)) out.push(item)
      return out
    }
    for (let cx = x1; cx <= x2; cx++) for (let cy = y1; cy <= y2; cy++) {
      const set = this.cells.get(`${cx}:${cy}`)
      if (!set) continue
      for (const id of set) {
        if (seen.has(id)) continue
        seen.add(id)
        const item = this.items.get(id)
        if (item && intersects(item.rect, rect)) out.push(item)
      }
    }
    return out
  }

  /** The top-most item under a world point: the one painted last. */
  hit(x: number, y: number): Item | null {
    const candidates = this.query({ x, y, w: 0.001, h: 0.001 })
    let best: Item | null = null
    for (const item of candidates) {
      if (item.turned && !containsPoint(item.turned.rect, item.turned.rotation, x, y)) continue
      if (!best || item.paint > best.paint) best = item
    }
    return best
  }

  /** Items fully inside `rect` (marquee). */
  within(rect: Rect): Item[] {
    return this.query(rect).filter((item) => item.rect.x >= rect.x && item.rect.y >= rect.y && item.rect.x + item.rect.w <= rect.x + rect.w && item.rect.y + item.rect.h <= rect.y + rect.h)
  }

  all(): Item[] { return [...this.items.values()] }
}
