/* Connectors in the canvas layout (连接线实现方案 §9.2): the second layout pass. The first pass gives every
 * Block and group its world rectangle and gives a connector its stored box (`rect`, used by everything that
 * writes placements); this pass resolves each connector's ends against those rectangles, routes it and sets
 * its `bounds` (path, label, room for width and caps: culling, hit testing, selection, fit). A gesture
 * re-runs it for the lines it touches with the moved rectangles (`lookupIn` overrides) and repaints them. */

import type { CellPayload, EntityEnvelope, KeyedContent } from '../../../api/types'
import type { OutlineModel } from '../../../state/outline'
import { Emitter } from '../../../state/emitter'
import type { WorkspaceStore } from '../../../state/store'
import { blockRegistry } from '../../blocks/registry'
import type { Laid } from '../layout'
import type { Point } from '../geometry'
import type { Rect } from '../render/camera'
import { routeConnector, storedEnds, type ConnectorGeometry, type Shape, type TargetGeom, type TargetLookup } from './geometry'
import { CONNECTOR_VIEW, connectorData, LABEL_FONT, type ConnectorData } from './model'
import { geometryKey } from './registry'

/** Room around a line's path in its layout bounds (width and caps of an ordinary line; hit slop). */
export const CONNECTOR_PAD = 12

/** `key`: a fingerprint of what is drawn (a frame skips rendering while it stays the same). */
export interface LaidConnector { data: ConnectorData; geom: ConnectorGeometry; key: string }

export function isConnector(entity: Pick<EntityEnvelope, 'type_id' | 'view_type'> | undefined): boolean {
  return entity?.type_id === 'buckyos.cell' && entity.view_type === CONNECTOR_VIEW
}

/** The outline a target Block declares (rect or ellipse). A definition that decides from the Cell's payload is
 * read once per content revision; until then the target counts as a rectangle. */
export class ShapeBook {
  private readonly known = new Map<string, { rev: number; shape: Shape }>()
  private readonly loading = new Set<string>()
  private readonly emitter = new Emitter()
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.version
  private readonly store: WorkspaceStore

  constructor(store: WorkspaceStore) { this.store = store }

  get(entity: EntityEnvelope | undefined): Shape {
    if (!entity || entity.type_id !== 'buckyos.cell') return 'rect'
    const shape = blockRegistry.get(entity.view_type ?? '')?.shape
    if (typeof shape !== 'function') return shape ?? 'rect'
    return this.known.get(entity.entity_id)?.shape ?? 'rect'
  }

  /** Read the payloads the outline does not carry, for targets whose definition decides from it. */
  ensure(entities: EntityEnvelope[]) {
    for (const entity of entities) {
      const shape = blockRegistry.get(entity.view_type ?? '')?.shape
      if (typeof shape !== 'function') continue
      const id = entity.entity_id
      const known = this.known.get(id)
      if ((known && known.rev === entity.content_rev) || this.loading.has(id)) continue
      this.loading.add(id)
      void this.store.readBatched<KeyedContent<CellPayload>>(id).then((read) => {
        this.loading.delete(id)
        const next = shape(read.content.payload)
        const before = this.known.get(id)?.shape
        this.known.set(id, { rev: entity.content_rev, shape: next })
        if (before !== next) { this.version += 1; this.emitter.emit() }
      }, () => { this.loading.delete(id) })
    }
  }
}

/** A target's rectangle as moved by a gesture. */
export type Override = (id: string) => { rect: Rect; rotation: number } | undefined

/** How a connector sees a bound target on this Surface: its (overridden) rect, or why it is not usable. */
export function lookupIn(laid: Map<string, Laid>, outline: OutlineModel, shapes: (id: string) => Shape, override?: Override): TargetLookup {
  return (id: string) => {
    const l = laid.get(id)
    if (!l) return { broken: outline.get(id) ? 'off_surface' : 'missing' }
    if (l.connector || isConnector(l.entity)) return { broken: 'invalid' }
    const moved = override?.(id)
    const geom: TargetGeom = { rect: moved?.rect ?? l.rect, rotation: moved?.rotation ?? l.rotation, shape: shapes(id) }
    return geom
  }
}

/** Route one connector from its stored box (moved by `offset` when the line itself is being dragged). */
export function routeLaid(l: Laid, data: ConnectorData, lookup: TargetLookup, offset?: Point): ConnectorGeometry {
  const rect = offset ? { ...l.rect, x: l.rect.x + offset.x, y: l.rect.y + offset.y } : l.rect
  return routeConnector(data, storedEnds(rect, data.flip), lookup, { title: l.entity.title ?? null, font: LABEL_FONT.m, pad: CONNECTOR_PAD })
}

/** The second layout pass: every connector's data, geometry and bounds. */
export function resolveConnectors(laid: Map<string, Laid>, outline: OutlineModel, shapes: (id: string) => Shape) {
  const lookup = lookupIn(laid, outline, shapes)
  for (const l of laid.values()) {
    if (!isConnector(l.entity)) continue
    const data = connectorData(l.entity.connector as Record<string, unknown> | null | undefined)
    const geom = routeLaid(l, data, lookup)
    l.connector = { data, geom, key: geometryKey(geom) }
    l.bounds = geom.bounds
  }
}

/** Which lines end on which target (a target's own lines only; a group's members are separate targets). */
export function connectorAdjacency(laid: Map<string, Laid>): Map<string, string[]> {
  const out = new Map<string, string[]>()
  for (const [id, l] of laid) {
    if (!l.connector) continue
    for (const end of [l.connector.data.start, l.connector.data.end]) {
      if (!end) continue
      const list = out.get(end.entity_id)
      if (list) { if (!list.includes(id)) list.push(id) } else out.set(end.entity_id, [id])
    }
  }
  return out
}

/** Does the line have an end bound to a target? (Bound lines are re-routed, not translated, by gestures.) */
export function hasBoundEnd(l: Laid | undefined): boolean {
  return Boolean(l?.connector && (l.connector.data.start || l.connector.data.end))
}
