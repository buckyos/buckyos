/* Composite controls on the Desktop (扩展控件 P1): what an instance's definition says, read from the outline (the
 * definition's `composition` and config defaults are projected there), and the client-side business rules the core
 * leaves to clients (Q15): which slot a Block may fill, how many it holds, what a new instance starts with. */

import { compositeOf, slotAccepts, slotFor, type Composition } from '@controls/composite'
import { ALL_AUTO } from '@controls/placement'
import { randomId } from '../../../api/ids'
import type { EntityEnvelope, Json, Operation } from '../../../api/types'
import type { WorkspaceStore } from '../../../state/store'
import { blockRegistry, type CreateArgs } from '../../blocks/registry'

export const COMPOSITE = 'composite'

/** The composition and effective config of a composite instance. */
export function compositionOf(store: WorkspaceStore, containerId: string): { composition: Composition | null; config: Record<string, unknown>; defId: string | null } {
  return compositeOf(store.outline, containerId)
}

export function isComposite(e: { type_id: string; kind?: string | null } | undefined | null): boolean {
  return !!e && e.type_id === 'buckyos.container' && e.kind === COMPOSITE
}

/** Does this composite keep every child where it is put (`fixed`)? */
export function isFixedComposite(store: WorkspaceStore, containerId: string): boolean {
  return compositionOf(store, containerId).composition?.layout.algorithm === 'fixed'
}

/** A definition's identity (`<author_did>#<name>`), for `cell:<ident>` slots. */
export function identOf(store: WorkspaceStore): (defId: string) => string | null {
  return (defId) => store.outline.get(defId)?.ident ?? null
}

/** How many live Blocks of the composite fill `slot` (not counting `except`). */
export function slotCount(store: WorkspaceStore, containerId: string, slot: string, except: ReadonlySet<string> = new Set()): number {
  return store.outline.childrenOf(containerId).filter((c) => !c.deleted && c.slot === slot && !except.has(c.entity_id)).length
}

/** Why `block` cannot fill `slot` of the composite (null: it can). */
export function slotRefusal(store: WorkspaceStore, containerId: string, slot: string, block: Pick<EntityEnvelope, 'type_id' | 'view_type' | 'def_id'>, moving: ReadonlySet<string> = new Set()): 'unknown' | 'accepts' | 'full' | null {
  const composition = compositionOf(store, containerId).composition
  const s = composition?.slots?.[slot]
  if (!s) return 'unknown'
  if (!slotAccepts(s, block, identOf(store))) return 'accepts'
  if (s.max !== undefined && slotCount(store, containerId, slot, moving) >= s.max) return 'full'
  return null
}

/** The slot a Block dropped into the composite takes: beside `sibling`, its slot; else the first that accepts it and
 * has room. Null when none does. */
export function slotForDrop(store: WorkspaceStore, containerId: string, block: Pick<EntityEnvelope, 'type_id' | 'view_type' | 'def_id'>, sibling: string | null, moving: ReadonlySet<string> = new Set()): string | null {
  const composition = compositionOf(store, containerId).composition
  if (!composition) return null
  const besides = sibling ? store.outline.get(sibling)?.slot : null
  if (besides && slotRefusal(store, containerId, besides, block, moving) === null) return besides
  return slotFor(composition, block, identOf(store), (s) => slotCount(store, containerId, s, moving))
}

/** The definition entity of an identity in this Workspace (a `cell:<ident>` slot's control). */
export function defByIdent(store: WorkspaceStore, ident: string): EntityEnvelope | undefined {
  return store.outline.all().find((e) => !e.deleted && e.type_id === 'buckyos.block-def' && e.ident === ident)
}

/**
 * Insert an instance of composite definition `defId`: the container (its `config` the instance's overrides) and, for
 * every slot with a `min`, that many Blocks of the first kind the slot accepts that can be created here (a control's
 * instance for `cell:<ident>`, a registered view type, a group for `container`; any Block: a text). The container is
 * `args.cellId` (selected after insertion).
 */
export function compositeCreate(args: CreateArgs): Operation[] {
  const { def_id: defId, ...config } = args.config ?? {}
  const def = args.store.outline.get(String(defId ?? ''))
  const containerId = args.cellId
  const ops: Operation[] = [{
    op: 'entity.create', entity_id: containerId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
    placement: { x: Math.round(args.placement.x), y: Math.round(args.placement.y), w: 'auto', h: 'auto' },
    payload: { kind: COMPOSITE, def_ref: { entity_id: String(defId ?? '') }, ...(Object.keys(config).length > 0 ? { config: config as Json } : {}), ...(args.title ? { title: args.title } : {}) },
  }]
  const composition = def?.composition ?? null
  let key: string | undefined
  for (const [slot, s] of Object.entries(composition?.slots ?? {})) {
    for (let i = 0; i < (s.min ?? 0); i++) {
      key = args.store.core.order_key_between(key, undefined)
      const child = initialChild(args, containerId, slot, s.accepts ?? [], key)
      if (child) ops.push(...child)
    }
  }
  return ops
}

function initialChild(args: CreateArgs, containerId: string, slot: string, accepts: string[], orderKey: string): Operation[] | null {
  const fixed = args.store.outline.get(String(args.config?.def_id ?? ''))?.composition?.layout.algorithm === 'fixed'
  const place = (size: { w: number; h: number }) => (fixed ? { x: 12, y: 12, w: size.w, h: size.h } : { x: 'auto' as const, y: 'auto' as const, w: size.w, h: size.h })
  for (const a of accepts.length > 0 ? accepts : ['richtext']) {
    if (a === 'container') {
      return [{ op: 'entity.create', entity_id: randomId('g'), type_id: 'buckyos.container', parent_id: containerId, order_key: orderKey, placement: place({ w: 200, h: 120 }), payload: { kind: 'group', slot } }]
    }
    if (a.startsWith('cell:')) {
      const def = defByIdent(args.store, a.slice(5))
      if (!def || (def.def_kind !== 'html' && def.def_kind !== 'declarative')) continue
      const size = { w: 240, h: 120 }
      return [{ op: 'entity.create', entity_id: randomId('c'), type_id: 'buckyos.cell', parent_id: containerId, order_key: orderKey, placement: place(size),
        payload: { view: { type: def.def_kind }, def_ref: { entity_id: def.entity_id }, slot } }]
    }
    const host = blockRegistry.get(a)
    if (!host?.create || host.catalog?.needs === 'data' || host.catalog?.needs === 'file' || host.catalog?.needs === 'definition') continue
    const cellId = randomId('c')
    const made = host.create({ ...args, cellId, dataId: randomId('d'), parentId: containerId, orderKey, placement: { x: 0, y: 0, ...host.defaultSize }, config: undefined, title: undefined })
    for (const op of made) {
      if (op.op !== 'entity.create') continue
      if (op.entity_id === cellId) {
        op.placement = op.type_id === 'buckyos.container' ? { ...ALL_AUTO } : place(host.defaultSize)
        op.payload = { ...(op.payload as Record<string, Json>), slot }
      }
    }
    return made
  }
  return null
}
