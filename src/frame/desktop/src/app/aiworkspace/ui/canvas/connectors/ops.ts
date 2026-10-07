/* Connector writes (连接线实现方案 §7): one gesture, one commit. Moving a coordinate end or the whole line writes
 * the placement only (auto-merged, D1); binding, rebinding and unbinding set the end key and, in the same
 * commit, write the corner at the end's current position; controls, label, route and look are payload keys
 * written with `expect`. Every placement is written with the corners at the current resolved ends, so the
 * stored spot of a bound end is where it was last seen (§4.3). Deleting a target freezes its lines (R3). */

import { randomId } from '../../../api/ids'
import type { CellPayload, ConnectorProjection, EntityEnvelope, Json, KeyedContent, Operation, Placement } from '../../../api/types'
import type { WorkspaceStore } from '../../../state/store'
import type { Point } from '../geometry'
import { placementOf, relativeTo, type Laid } from '../layout'
import { boxOf, decompose, simplify } from './geometry'
import { CONNECTOR_VIEW, connectorKeys, type Binding, type ConnectorData, type Flip, type LabelPos, type Route } from './model'

/** One end of a line being created or changed: its binding (null: a coordinate) and where it is now. */
export interface EndSpec { binding: Binding | null; point: Point }
export interface NewConnector { start: EndSpec; end: EndSpec; route: Route }
export type ConnectorChange =
  | { kind: 'end'; which: 'start' | 'end'; spec: EndSpec; route?: Route }
  /** World points the route passes (null: back to the automatic route). */
  | { kind: 'controls'; points: Point[] | null }
  | { kind: 'label'; label: LabelPos }

export interface ConnectorWrite {
  operations: Operation[]
  label: string
  /** The outline as it will be (applied at once; reverted when the commit is refused). */
  patch: Partial<EntityEnvelope>
  before: Partial<EntityEnvelope>
  placement?: Placement
}

export function projectionOf(data: ConnectorData): ConnectorProjection {
  return {
    ...(connectorKeys(data) as unknown as ConnectorProjection),
    ...(data.label.t !== 0.5 || data.label.offset ? { label: { t: data.label.t, ...(data.label.offset ? { offset: data.label.offset } : {}) } } : {}),
  }
}

const sameBinding = (a: Binding | null, b: Binding | null) => JSON.stringify(a) === JSON.stringify(b)
const flipValue = (flip: Flip): Json => ({ ...(flip.h ? { h: true } : {}), ...(flip.v ? { v: true } : {}) })
const sameFlip = (a: Flip, b: Flip) => Boolean(a.h) === Boolean(b.h) && Boolean(a.v) === Boolean(b.v)

async function keyRevs(store: WorkspaceStore, id: string): Promise<Record<string, number>> {
  const read = await store.session.read<KeyedContent<CellPayload>>(id)
  return read.content.key_revs ?? {}
}

/** A new line in `parentId` (on top of its siblings), its corners at the two ends. */
export function createConnectorOp(store: WorkspaceStore, laid: Map<string, Laid>, parentId: string, spec: NewConnector): { op: Operation; id: string; placement: Placement } {
  const { rect, flip } = boxOf(spec.start.point, spec.end.point)
  const id = randomId('c')
  const orderKey = store.core.order_key_between(store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined)
  const placement = relativeTo(laid, parentId, rect, 0, 0)
  const data: ConnectorData = { start: spec.start.binding, end: spec.end.binding, flip, route: spec.route, controls: [], label: { t: 0.5, offset: 0 } }
  return {
    id, placement,
    op: { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parentId, order_key: orderKey, placement, payload: { view: { type: CONNECTOR_VIEW, version: 1 }, ...connectorKeys(data) } },
  }
}

/** The writes of one edit of a laid-out line. */
export async function connectorChangeOps(store: WorkspaceStore, laid: Map<string, Laid>, l: Laid, change: ConnectorChange): Promise<ConnectorWrite | null> {
  const line = l.connector
  if (!line) return null
  const id = l.entity.entity_id
  const { data, geom } = line
  const revs = await keyRevs(store, id)
  const expect = (key: string) => ({ rev: revs[key] ?? 0 })
  const before: Partial<EntityEnvelope> = { connector: l.entity.connector ?? null, placement: l.entity.placement }
  if (change.kind === 'end') {
    const s = change.which === 'start' ? change.spec.point : geom.start.point
    const e = change.which === 'end' ? change.spec.point : geom.end.point
    const { rect, flip } = boxOf(s, e)
    const next: ConnectorData = { ...data, [change.which]: change.spec.binding, flip, ...(change.route ? { route: change.route } : {}) }
    const keys: { key: string; value: Json; expect: { rev: number } }[] = []
    if (!sameBinding(data[change.which], change.spec.binding)) keys.push({ key: change.which, value: change.spec.binding as unknown as Json, expect: expect(change.which) })
    if (!sameFlip(data.flip, flip)) keys.push({ key: 'flip', value: flipValue(flip), expect: expect('flip') })
    if (change.route && change.route !== data.route) keys.push({ key: 'route', value: change.route, expect: expect('route') })
    const placement = placementOf(laid, l, l.parentId, rect)
    // the end key before the corner: a box that is a point is only valid once an end is bound
    const operations: Operation[] = [...(keys.length ? [{ op: 'entity.set_keys', entity_id: id, keys } as Operation] : []), { op: 'tree.place', entity_id: id, placement }]
    const verb = change.spec.binding ? (data[change.which] ? (sameBinding(data[change.which], change.spec.binding) ? '移动连接点' : '重新连接') : '连接') : data[change.which] ? '断开' : '移动端点'
    return { operations, label: `连接线：${verb}`, patch: { placement, connector: projectionOf(next) }, before, placement }
  }
  if (change.kind === 'controls') {
    const s = geom.start.point
    const e = geom.end.point
    if (!change.points) {
      if (data.controls.length === 0) return null
      return { operations: [{ op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'controls', expect: expect('controls') }] }], label: '连接线：重置路由', patch: { connector: projectionOf({ ...data, controls: [] }) }, before }
    }
    const points = data.route === 'elbow' ? simplify([s, ...change.points, e]).slice(1, -1) : change.points
    const controls = points.slice(0, 64).map((p) => decompose(p, s, e))
    return {
      operations: [{ op: 'entity.set_keys', entity_id: id, keys: [{ key: 'controls', value: controls as unknown as Json, expect: expect('controls') }] }],
      label: '连接线：调整路径', patch: { connector: projectionOf({ ...data, controls }) }, before,
    }
  }
  const label = { t: Math.round(change.label.t * 10000) / 10000, offset: Math.round(change.label.offset * 10) / 10 }
  return {
    operations: [{ op: 'entity.set_keys', entity_id: id, keys: [{ key: 'label', value: { t: label.t, ...(label.offset ? { offset: label.offset } : {}) }, expect: expect('label') }] }],
    label: '连接线：移动标签', patch: { connector: projectionOf({ ...data, label }) }, before,
  }
}

/** Change the route (the stored controls belong to the old route and are dropped). */
export function routeOps(entityId: string, route: Route, hasControls: boolean, revs: Record<string, number>): Operation[] {
  return [
    { op: 'entity.set_keys', entity_id: entityId, keys: [{ key: 'route', value: route, expect: { rev: revs.route ?? 0 } }] },
    ...(hasControls ? [{ op: 'entity.unset_keys', entity_id: entityId, keys: [{ key: 'controls', expect: { rev: revs.controls ?? 0 } }] } as Operation] : []),
  ]
}

/** Deleting Blocks in this window: their lines keep the binding and stay where they are now (§6.2, R3). A line
 * whose container this user cannot restructure keeps its older stored spot. */
export async function freezeOps(store: WorkspaceStore, laid: Map<string, Laid>, doomed: ReadonlySet<string>): Promise<Operation[]> {
  const ops: Operation[] = []
  for (const l of laid.values()) {
    const line = l.connector
    if (!line || doomed.has(l.entity.entity_id)) continue
    const ends = [line.data.start, line.data.end]
    if (!ends.some((end) => end && doomed.has(end.entity_id))) continue
    const parent = store.outline.get(l.parentId)
    if (!parent?.capabilities.includes('structure')) continue
    const { rect, flip } = boxOf(line.geom.start.point, line.geom.end.point)
    const id = l.entity.entity_id
    if (!sameFlip(flip, line.data.flip)) {
      if (!l.entity.capabilities.includes('update')) continue
      const revs = await keyRevs(store, id)
      ops.push({ op: 'entity.set_keys', entity_id: id, keys: [{ key: 'flip', value: flipValue(flip), expect: { rev: revs.flip ?? 0 } }] })
    }
    ops.push({ op: 'tree.place', entity_id: id, placement: placementOf(laid, l, l.parentId, rect) })
  }
  return ops
}
