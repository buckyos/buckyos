/* Mounted connector frames (连接线实现方案 §9.5): what a running gesture needs to repaint a line in place, and the
 * fingerprint that lets a line skip rendering when a new layout leaves its path as it was. */

import type { ConnectorGeometry } from './geometry'
import type { ConnectorStyle } from './model'
import type { PaintOptions } from './paint'

/** Low zoom draws lines without labels and with simple caps (§9.4). */
export const LINE_SIMPLIFIED_ZOOM = 0.4

/** What a gesture needs to repaint a mounted line in place. */
export interface LineEntry { el: HTMLDivElement; style: ConnectorStyle; options: PaintOptions }

export class LineRegistry {
  private readonly entries = new Map<string, LineEntry>()
  get(id: string): LineEntry | undefined { return this.entries.get(id) }
  set(id: string, entry: LineEntry | null) { if (entry) this.entries.set(id, entry); else this.entries.delete(id) }
  /** Half the drawn width (hit tolerance), 1 for a line not mounted. */
  halfWidth(id: string): number { const e = this.entries.get(id); return e ? Math.max(e.style.width, 1 / e.options.zoom) / 2 : 1 }
}

/** A cheap fingerprint of what the frame draws: a new layout object with the same path skips the render. */
export function geometryKey(g: ConnectorGeometry): string {
  const p = (pt: { x: number; y: number }) => `${Math.round(pt.x * 10)},${Math.round(pt.y * 10)}`
  return `${g.route}|${g.start.state}${g.end.state}|${g.pts.map(p).join(';')}|${g.segs.map((s) => (s.c1 && s.c2 ? `${p(s.c1)}${p(s.c2)}` : '')).join('')}|${p(g.label.point)}`
}
