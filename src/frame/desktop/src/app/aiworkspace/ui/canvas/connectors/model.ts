/* The connector's data (连接线实现方案 §4, §5): a `buckyos.cell` with `view.type = connector`. The tree edge's
 * placement is the box around the two stored endpoints and `flip` says which corner is which; each end is a
 * coordinate (null) or a binding to one of a Block's anchors; `route` and `controls` give the path, `label`
 * places the title along it, `config` is the look. The outline projects everything but `config`
 * (`EntityEnvelope.connector`), so layout never waits for a read. Reading is lenient (a newer or broken value
 * falls back to the default); the core checks what is written. */

import type { Json } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { Anchor, Binding, Control, ConnectorData, Flip, Route } from '@drawio/connector'
import { ANCHOR_ID } from './anchors'

export type { Anchor, Binding, Control, ConnectorData, Flip, LabelPos, Route } from '@drawio/connector'

export const CONNECTOR_VIEW = 'connector'

/** A choice of a menu: its label is read in the current language when it is shown. */
const choice = <V extends string>(value: V, key: string) => ({ value, get label() { return aiwsT(key) } })

export const ROUTES: { value: Route; label: string }[] = [
  choice('straight', 'aiworkspace.connector.routes.straight'), choice('elbow', 'aiworkspace.connector.routes.elbow'), choice('curve', 'aiworkspace.connector.routes.curve'),
]
export const MAX_CONTROLS = 64

const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)

function binding(v: unknown): Binding | null {
  if (!v || typeof v !== 'object') return null
  const b = v as { entity_id?: unknown; anchor?: { kind?: unknown; id?: unknown } }
  if (typeof b.entity_id !== 'string' || !b.entity_id) return null
  const a = b.anchor
  if (a?.kind !== 'named' || typeof a.id !== 'string' || !ANCHOR_ID.test(a.id)) return null
  return { entity_id: b.entity_id, anchor: { kind: 'named', id: a.id } }
}

/** Two anchors are the same anchor of their target (a self loop needs two different ones). */
export const sameAnchor = (a: Anchor, b: Anchor) => a.id === b.id

/** The connector's fields from the outline projection (or a payload): read wide. */
export function connectorData(source: Record<string, unknown> | null | undefined): ConnectorData {
  const s = source ?? {}
  const flip = (s.flip && typeof s.flip === 'object' ? s.flip : {}) as Flip
  const route: Route = s.route === 'elbow' || s.route === 'curve' ? s.route : 'straight'
  const controls = Array.isArray(s.controls)
    ? (s.controls as Control[]).filter((c) => c && finite(c.u) && finite(c.v)).slice(0, MAX_CONTROLS).map((c) => ({ u: c.u, v: c.v, ...(finite(c.dx) && c.dx ? { dx: c.dx } : {}), ...(finite(c.dy) && c.dy ? { dy: c.dy } : {}) }))
    : []
  const l = (s.label && typeof s.label === 'object' ? s.label : {}) as { t?: unknown; offset?: unknown }
  return {
    start: binding(s.start), end: binding(s.end),
    flip: { h: flip.h === true, v: flip.v === true },
    route, controls,
    label: { t: finite(l.t) ? Math.min(1, Math.max(0, l.t)) : 0.5, offset: finite(l.offset) ? l.offset : 0 },
  }
}

/** The payload keys of `data` as written (defaults left out where the contract allows). */
export function connectorKeys(data: ConnectorData): Record<string, Json> {
  return {
    start: data.start as unknown as Json, end: data.end as unknown as Json,
    ...(data.flip.h || data.flip.v ? { flip: { ...(data.flip.h ? { h: true } : {}), ...(data.flip.v ? { v: true } : {}) } } : {}),
    ...(data.route !== 'straight' ? { route: data.route } : {}),
    ...(data.controls.length ? { controls: data.controls as unknown as Json } : {}),
  }
}

// ---- look (§5.2): `config` keys, checked here (the core only limits the size)

export type Cap = 'none' | 'arrow' | 'triangle' | 'triangle_outline' | 'diamond' | 'diamond_outline' | 'circle' | 'circle_outline' | 'bar'
  | 'cardinality_one' | 'cardinality_many' | 'cardinality_one_or_many' | 'cardinality_exactly_one' | 'cardinality_zero_or_one' | 'cardinality_zero_or_many'
export type Dash = 'solid' | 'dashed' | 'dotted'
export type LabelSize = 's' | 'm' | 'l'

export const CAPS: { value: Cap; label: string }[] = [
  choice('none', 'aiworkspace.common.none'), choice('arrow', 'aiworkspace.connector.cap.arrow'), choice('triangle', 'aiworkspace.connector.cap.triangle'),
  choice('triangle_outline', 'aiworkspace.connector.cap.triangleOutline'), choice('diamond', 'aiworkspace.connector.cap.diamond'),
  choice('diamond_outline', 'aiworkspace.connector.cap.diamondOutline'), choice('circle', 'aiworkspace.connector.cap.circle'),
  choice('circle_outline', 'aiworkspace.connector.cap.circleOutline'), choice('bar', 'aiworkspace.connector.cap.bar'),
  choice('cardinality_one', 'aiworkspace.connector.cap.one'), choice('cardinality_many', 'aiworkspace.connector.cap.many'),
  choice('cardinality_one_or_many', 'aiworkspace.connector.cap.oneOrMany'), choice('cardinality_exactly_one', 'aiworkspace.connector.cap.exactlyOne'),
  choice('cardinality_zero_or_one', 'aiworkspace.connector.cap.zeroOrOne'), choice('cardinality_zero_or_many', 'aiworkspace.connector.cap.zeroOrMany'),
]
export const DASHES: { value: Dash; label: string }[] = [choice('solid', 'aiworkspace.blocks.dash.solid'), choice('dashed', 'aiworkspace.blocks.dash.dashed'), choice('dotted', 'aiworkspace.blocks.dash.dotted')]
export const WIDTHS: { value: string; label: string }[] = [choice('1', 'aiworkspace.connector.widths.thin'), choice('2', 'aiworkspace.connector.widths.medium'), choice('4', 'aiworkspace.connector.widths.thick')]
export const LABEL_SIZES: { value: LabelSize; label: string }[] = [choice('s', 'aiworkspace.blocks.size.s'), choice('m', 'aiworkspace.blocks.size.m'), choice('l', 'aiworkspace.blocks.size.l')]
/** Label font size in world units. */
export const LABEL_FONT: Record<LabelSize, number> = { s: 12, m: 14, l: 18 }

export const DEFAULT_STROKE = '#64748b'

export interface ConnectorStyle {
  stroke: string
  opacity: number
  width: number
  dash: Dash
  startCap: Cap
  endCap: Cap
  rounded: boolean
  labelSize: LabelSize
  labelColor: string | null
  labelFill: string | null
}

const HEX = /^#[0-9a-f]{6}$/i
const CAP_SET = new Set<string>(CAPS.map((c) => c.value))

/** An unknown cap still says "this line has a direction" (§5.2): it is drawn as an arrow. */
function cap(v: unknown, fallback: Cap): Cap {
  if (typeof v !== 'string') return fallback
  return CAP_SET.has(v) ? (v as Cap) : 'arrow'
}

export function connectorStyle(config: Record<string, Json> | undefined | null): ConnectorStyle {
  const c = config ?? {}
  const width = finite(c.width) ? Math.min(24, Math.max(0.5, c.width)) : 2
  return {
    stroke: typeof c.stroke === 'string' && HEX.test(c.stroke) ? c.stroke : DEFAULT_STROKE,
    opacity: finite(c.opacity) ? Math.min(1, Math.max(0.1, c.opacity)) : 1,
    width,
    dash: c.dash === 'dashed' || c.dash === 'dotted' ? c.dash : 'solid',
    startCap: cap(c.start_cap, 'none'),
    endCap: cap(c.end_cap, 'arrow'),
    rounded: c.rounded !== false,
    labelSize: c.label_size === 's' || c.label_size === 'l' ? c.label_size : 'm',
    labelColor: typeof c.label_color === 'string' && HEX.test(c.label_color) ? c.label_color : null,
    labelFill: typeof c.label_fill === 'string' && HEX.test(c.label_fill) ? c.label_fill : null,
  }
}

export const DEFAULT_STYLE = connectorStyle(null)
