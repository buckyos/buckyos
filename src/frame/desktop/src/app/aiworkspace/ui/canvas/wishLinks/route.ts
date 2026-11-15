/* Routing and painting a wish's lines with the connectors' own geometry and drawing (连接线实现方案 §4.3, §9.4): each end
 * sits on the anchor of its Block facing the other one, the route is a curve, the look is a light dashed line with an
 * arrow at the far end (blue into the wish, orange out of it). The two lines of a derived column leave and enter by the
 * anchors beside the facing ones, each on its left, so they run apart. */

import { centerOf } from '../geometry'
import { sidesOf } from '../connectors/anchors'
import { anchorsOf, anchorWorld, facingAnchor, leftOf, routeConnector, type Anchor, type ConnectorGeometry, type TargetGeom } from '../connectors/geometry'
import { DEFAULT_STYLE, type ConnectorStyle } from '../connectors/model'
import { connectorMarkup, frameBox, type PaintOptions } from '../connectors/paint'
import { LINE_SIMPLIFIED_ZOOM } from '../connectors/registry'
import type { Rect } from '../render/camera'
import type { LinkKind } from './plan'

export const INPUT_STROKE = '#60a5fa'
export const RESULT_STROKE = '#fb923c'
const LOOK: ConnectorStyle = { ...DEFAULT_STYLE, width: 2, opacity: 0.9, dash: 'dashed', startCap: 'none', endCap: 'arrow' }
export const LINK_STYLE: Record<LinkKind, ConnectorStyle> = { input: { ...LOOK, stroke: INPUT_STROKE }, result: { ...LOOK, stroke: RESULT_STROKE } }

const FROM = '@from'
const TO = '@to'
/** An anchor id no target has: the end floats where the ray from the centre leaves the outline. */
const FLOATING: Anchor = { kind: 'named', id: '~' }

/** The single-sided anchor next to `base` on its side, towards `normal` (none: `base`). */
function besideAnchor(t: TargetGeom, base: Anchor, normal: { x: number; y: number }): Anchor {
  const defs = anchorsOf(t)
  const own = defs.find((d) => d.id === base.id)
  const side = own ? sidesOf(own) : []
  if (side.length !== 1) return base
  const at = anchorWorld(t, base, centerOf(t.rect))
  let best = base
  let bestD = Infinity
  for (const d of defs) {
    const s = sidesOf(d)
    if (d.id === base.id || s.length !== 1 || s[0] !== side[0]) continue
    const p = anchorWorld(t, { kind: 'named', id: d.id }, at)
    const v = { x: p.x - at.x, y: p.y - at.y }
    const along = v.x * normal.x + v.y * normal.y
    const dd = Math.hypot(v.x, v.y)
    if (along > 1e-6 && dd < bestD) { best = { kind: 'named', id: d.id }; bestD = dd }
  }
  return best
}

/** The route of one line from `from` to `to`. */
export function routeLink(from: TargetGeom, to: TargetGeom, paired: boolean): ConnectorGeometry {
  const a = centerOf(from.rect)
  const b = centerOf(to.rect)
  let start = facingAnchor(from, b) ?? FLOATING
  let end = facingAnchor(to, a) ?? FLOATING
  if (paired) {
    const l = Math.hypot(b.x - a.x, b.y - a.y) || 1
    const normal = leftOf({ x: (b.x - a.x) / l, y: (b.y - a.y) / l })
    start = besideAnchor(from, start, normal)
    end = besideAnchor(to, end, normal)
  }
  return routeConnector({ start: { entity_id: FROM, anchor: start }, end: { entity_id: TO, anchor: end }, flip: {}, route: 'curve', controls: [], label: { t: 0.5, offset: 0 } },
    [a, b], (id) => (id === FROM ? from : to), { pad: 0 })
}

export interface PaintedLink { box: Rect; markup: string }

/** The frame box and SVG content of a routed line at `zoom` (`id`: unique in the page). */
export function paintLink(geom: ConnectorGeometry, kind: LinkKind, zoom: number, id: string): PaintedLink {
  const style = LINK_STYLE[kind]
  const options: PaintOptions = { zoom, simplified: zoom < LINE_SIMPLIFIED_ZOOM, showBroken: false, id, title: null }
  return { box: frameBox(geom, style, options), markup: connectorMarkup(geom, style, options) }
}

export function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}
