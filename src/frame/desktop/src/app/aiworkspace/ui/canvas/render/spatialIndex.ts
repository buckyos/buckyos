/* Spatial index (phase two §9.3 rule 2): a uniform grid over the world-coordinate bounding boxes
 * of a Surface's Blocks, for visibility culling, hit testing, marquee selection and snapping. Hit
 * tests are geometric — no DOM event per Block. Self-implemented (no new dependency).
 * A connector (连接线实现方案 §9.4) is entered in the cells its path crosses, not every cell of its box, and
 * a point hits it within a tolerance of the path (or on its label). */

import { containsPoint, type Point } from '../geometry'
import { intersects, type Rect } from './camera'

/** A line's shape for the index: its path as a polyline and its label box. `distance` measures to the drawn path. */
export interface LineShape { flat: Point[]; label: Rect | null; distance: (p: Point) => number }

/** `rect` is the axis-aligned box the grid indexes; a rotated Block also gives its own rect and rotation
 * (`turned`), and a point hits it only inside the rotated shape; a connector gives its `line`. */
export interface Item { id: string; rect: Rect; /** Stacking rank (BlockTree pre-order, higher on top): the paint order. */ paint: number; turned?: { rect: Rect; rotation: number }; line?: LineShape }

export interface HitOptions {
  /** World distance a line may be from the point (screen tolerance over the zoom). */
  slop?: number
  /** A line's own tolerance (half its drawn width), when larger than `slop`. */
  lineTolerance?: (item: Item) => number
  /** Only these items count (e.g. connector targets while a line is being dragged). */
  accept?: (item: Item) => boolean
}

const CELL = 512

function cellRange(rect: Rect): [number, number, number, number] {
  return [Math.floor(rect.x / CELL), Math.floor(rect.y / CELL), Math.floor((rect.x + rect.w) / CELL), Math.floor((rect.y + rect.h) / CELL)]
}

/** The grid cells an item occupies: every cell of its box, or for a line the cells along its path (padded). */
function cellsOf(item: Item): string[] {
  const keys = new Set<string>()
  const addRect = (r: Rect) => {
    const [x1, y1, x2, y2] = cellRange(r)
    for (let cx = x1; cx <= x2; cx++) for (let cy = y1; cy <= y2; cy++) keys.add(`${cx}:${cy}`)
  }
  if (!item.line) { addRect(item.rect); return [...keys] }
  const pad = 32
  const { flat, label } = item.line
  for (let i = 0; i < flat.length; i++) {
    const a = flat[i]
    const b = flat[i + 1] ?? a
    const steps = Math.max(1, Math.ceil(Math.hypot(b.x - a.x, b.y - a.y) / (CELL / 4)))
    for (let k = 0; k <= steps; k++) {
      const x = a.x + ((b.x - a.x) * k) / steps
      const y = a.y + ((b.y - a.y) * k) / steps
      addRect({ x: x - pad, y: y - pad, w: pad * 2, h: pad * 2 })
    }
  }
  if (label) addRect(label)
  return [...keys]
}

export class SpatialIndex {
  private readonly cells = new Map<string, Set<string>>()
  private readonly items = new Map<string, Item>()

  clear() { this.cells.clear(); this.items.clear(); this.occupied.clear() }

  size(): number { return this.items.size }

  get(id: string): Item | undefined { return this.items.get(id) }

  private readonly occupied = new Map<string, string[]>()

  insert(item: Item) {
    this.remove(item.id)
    this.items.set(item.id, item)
    const keys = cellsOf(item)
    this.occupied.set(item.id, keys)
    for (const key of keys) {
      let set = this.cells.get(key)
      if (!set) { set = new Set(); this.cells.set(key, set) }
      set.add(item.id)
    }
  }

  remove(id: string) {
    const item = this.items.get(id)
    if (!item) return
    for (const key of this.occupied.get(id) ?? []) {
      const set = this.cells.get(key)
      if (set) { set.delete(id); if (set.size === 0) this.cells.delete(key) }
    }
    this.occupied.delete(id)
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
  hit(x: number, y: number, options: HitOptions = {}): Item | null {
    const slop = options.slop ?? 0
    const candidates = this.query({ x: x - slop, y: y - slop, w: slop * 2 + 0.001, h: slop * 2 + 0.001 })
    let best: Item | null = null
    for (const item of candidates) {
      if (options.accept && !options.accept(item)) continue
      if (item.line) {
        if (item.line.distance({ x, y }) > Math.max(slop, options.lineTolerance?.(item) ?? 0)) continue
      } else {
        if (x < item.rect.x || x > item.rect.x + item.rect.w || y < item.rect.y || y > item.rect.y + item.rect.h) continue
        if (item.turned && !containsPoint(item.turned.rect, item.turned.rotation, x, y)) continue
      }
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
