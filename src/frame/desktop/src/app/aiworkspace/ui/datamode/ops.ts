/* The writes of the object tree (UI 框架 PRD §5.2), each one ordinary, undoable Commit through the store: new data at a
 * chosen place (in a folder: at its end; on an object: right after it), imports, rename, move and reorder, place on a
 * canvas, and the delete pre-check that names only the referrers this caller can see (unchanged). */

import { randomId } from '../../api/ids'
import { describeError, type ImportedAsset } from '../../api/session'
import { DATA_ID, type CommitOutcome, type EntityEnvelope, type Json, type Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { defaultRendererFor } from '../blocks/ops'
import { entityLabel, importedAssetOps, newDataOps, type NewDataKind } from '../sources/dataOps'
import { insertLinkedSource } from '../sources/sourceLinks'
import { moveEntryOps } from '../surfaces/tree'
import { acceptsData } from './model'

/** Where new data goes: into `parentId`, right after `afterId` (or at its end). */
export interface Place { parentId: string; afterId: string | null }

export function accepted(outcome: CommitOutcome): boolean {
  return outcome.status === 'accepted' || outcome.status === 'saved_locally'
}

/** The place a menu opened on means: a folder takes the new object at its end; an object gets it as its next sibling;
 * the blank area is the data root. */
export function placeOf(store: WorkspaceStore, entityId: string | null): Place {
  const entity = entityId ? store.outline.get(entityId) : undefined
  if (!entity || entity.entity_id === DATA_ID) return { parentId: DATA_ID, afterId: null }
  if (acceptsData(entity)) return { parentId: entity.entity_id, afterId: null }
  const parent = entity.parent_id ? store.outline.get(entity.parent_id) : undefined
  return acceptsData(parent) && parent ? { parentId: parent.entity_id, afterId: entity.entity_id } : { parentId: DATA_ID, afterId: null }
}

/** `count` order keys in a row right after `afterId` under `parentId` (before its next sibling), or at the end. */
export function keysAt(store: WorkspaceStore, place: Place, count: number): string[] {
  const siblings = store.outline.childrenOf(place.parentId)
  const at = place.afterId ? siblings.findIndex((e) => e.entity_id === place.afterId) : -1
  let low = at >= 0 ? siblings[at].order_key : siblings.at(-1)?.order_key
  const high = at >= 0 ? siblings[at + 1]?.order_key : undefined
  const out: string[] = []
  for (let i = 0; i < count; i++) {
    low = store.core.order_key_between(low ?? undefined, high ?? undefined)
    out.push(low)
  }
  return out
}

/** Put the first-level creations of `ops` (those under `place.parentId`) at the place, in their order. */
function placed(store: WorkspaceStore, ops: Operation[], place: Place): Operation[] {
  const top = ops.filter((op) => op.op === 'entity.create' && op.parent_id === place.parentId && op.type_id !== 'buckyos.block-def')
  const keys = keysAt(store, place, top.length)
  return ops.map((op) => { const i = top.indexOf(op); return i >= 0 ? { ...op, order_key: keys[i] } : op })
}

const CREATE_LABEL: Record<NewDataKind, string> = {
  folder: 'aiworkspace.data.tree.newFolder', table: 'aiworkspace.data.tree.newTable', richtext: 'aiworkspace.data.tree.newRichtext',
  record: 'aiworkspace.data.tree.newRecord', wish: 'aiworkspace.data.tree.newWish', note: 'aiworkspace.data.tree.newNote', link: 'aiworkspace.data.tree.newLink',
  dir: 'aiworkspace.data.tree.newDir',
}

/** New data of `kind` at `place`; resolves with the new entity's id, or null when it was not created (the store keeps
 * the edit and says why). */
export async function createData(store: WorkspaceStore, place: Place, kind: Exclude<NewDataKind, 'link'>, title: string): Promise<string | null> {
  const ops = placed(store, newDataOps(store, place.parentId, kind, title.trim()), place)
  const outcome = await store.submit({ editId: `new-data:${randomId().slice(0, 6)}`, label: aiwsT(CREATE_LABEL[kind]), operations: ops })
  if (!accepted(outcome)) return null
  return (ops.filter((op) => op.op === 'entity.create').at(-1)?.entity_id as string | undefined) ?? null
}

/** A link as data (a publication of another Workspace, or a CSV / JSON / Markdown URL): created and refreshed at once. */
export async function createLinked(store: WorkspaceStore, place: Place, link: string, title: string): Promise<{ entityId: string; error: string | null }> {
  const { entityId, run } = await insertLinkedSource(store, place.parentId, link, title.trim())
  const error = run.state === 'succeeded' ? null : aiwsT('aiworkspace.data.source.failed', { error: (run.result as { error?: { detail?: string } } | null)?.error?.detail ?? run.state })
  if (place.afterId) await move(store, entityId, place.parentId, { after: place.afterId })
  return { entityId, error }
}

/** Files (or a folder's tree) as assets at `place`, one Commit; resolves with the first new entity's id. */
export async function importFiles(store: WorkspaceStore, place: Place, files: File[], folder: boolean, signal?: AbortSignal): Promise<string | null> {
  let items: ImportedAsset[]
  try {
    items = await store.session.importAssets(files, folder, signal)
    signal?.throwIfAborted()
  } catch (error) {
    if (!signal?.aborted) store.notify('error', aiwsT('aiworkspace.data.tree.uploadFailed', { error: describeError(error) }))
    return null
  }
  const operations = placed(store, importedAssetOps(store, place.parentId, items), place)
  const outcome = await store.submit({ editId: randomId('import'), label: aiwsT('aiworkspace.data.tree.importFiles'), operations })
  return accepted(outcome) ? (operations[0]?.entity_id as string | undefined) ?? null : null
}

/** Move into `parentId` (its end, or next to a sibling there); one Commit. */
export function move(store: WorkspaceStore, entityId: string, parentId: string, at: { before?: string; after?: string } | null = null): Promise<CommitOutcome> {
  const entity = store.outline.get(entityId)
  return store.submit({ editId: `entity:${entityId}`, label: aiwsT('aiworkspace.data.tree.moveEdit', { name: entity ? entityLabel(entity) : entityId }), operations: moveEntryOps(store, entityId, parentId, at) })
}

/** May `id` move into `parentId`: a folder that takes data, not itself or below itself. */
export function canMoveInto(store: WorkspaceStore, id: string, parentId: string): boolean {
  return acceptsData(store.outline.get(parentId)) && parentId !== id && !store.outline.ancestors(parentId).includes(id)
}

/** The folders an object can move into (the data root first). */
export function moveTargets(store: WorkspaceStore, id: string): EntityEnvelope[] {
  const out: EntityEnvelope[] = []
  const walk = (parentId: string) => {
    for (const child of store.outline.childrenOf(parentId)) {
      if (child.type_id !== 'buckyos.container') continue
      if (canMoveInto(store, id, child.entity_id)) out.push(child)
      walk(child.entity_id)
    }
  }
  const root = store.outline.get(DATA_ID)
  if (root) out.push(root)
  walk(DATA_ID)
  return out
}

/** Rename: an object with a title gets a new title; one without gets a new lookup name (its path step). */
export async function rename(store: WorkspaceStore, entity: EntityEnvelope, to: string): Promise<CommitOutcome> {
  const name = to.trim()
  if (entity.title !== undefined && entity.title !== null) {
    const read = await store.session.read<{ payload: Record<string, Json>; key_revs: Record<string, number> }>(entity.entity_id)
    return store.submit({ editId: `key:${entity.entity_id}:title`, label: aiwsT('aiworkspace.datamode.renameEdit', { name }), mine: name, hasMine: true,
      operations: [{ op: 'entity.set_keys', entity_id: entity.entity_id, keys: [{ key: 'title', value: name, expect: { rev: read.content.key_revs.title ?? 0 } }] }] })
  }
  return store.submit({ editId: `entity:${entity.entity_id}`, label: aiwsT('aiworkspace.datamode.renameEdit', { name }), mine: name, hasMine: true,
    operations: [{ op: 'entity.rename', entity_id: entity.entity_id, name: name === '' ? null : name, expect: { rev: entity.meta_rev } }] })
}

/** The deletion of an object with what is below it. */
export function deleteOp(store: WorkspaceStore, entity: EntityEnvelope): Operation {
  const descendants = store.outline.descendants(entity.entity_id)
  return { op: 'entity.delete', entity_id: entity.entity_id, ...(descendants.length ? { subtree: { delete: descendants.map((d) => d.entity_id) } } : {}), expect: { rev: entity.life_rev } }
}

export interface DeleteCheck { referrers: { entity_id: string; target: string }[]; hidden: boolean }

/** The pre-check (unchanged): what still references it, and whether references exist that this caller cannot see. */
export async function precheckDelete(store: WorkspaceStore, entity: EntityEnvelope): Promise<DeleteCheck | null> {
  try {
    const result = await store.session.prepare([deleteOp(store, entity)])
    if (result.status === 'ok') return { referrers: [], hidden: false }
    const data = result.status === 'rejected' ? (result.errors?.[0]?.data as { referrers?: { entity_id: string; target: string }[]; hidden_referrers?: boolean } | undefined) : undefined
    return { referrers: data?.referrers ?? [{ entity_id: result.code, target: '' }], hidden: Boolean(data?.hidden_referrers) }
  } catch (error) {
    store.notify('error', aiwsT('aiworkspace.data.tree.precheckFailed', { error: describeError(error) }))
    return null
  }
}

export function deleteData(store: WorkspaceStore, entity: EntityEnvelope): Promise<CommitOutcome> {
  return store.submit({ editId: `entity:${entity.entity_id}`, label: aiwsT('aiworkspace.data.tree.deleteEdit', { name: entityLabel(entity) }), operations: [deleteOp(store, entity)] })
}

/** A Block showing `entity` on a canvas ("放到画布…"): last on a flow page, below what is there on a free canvas. */
export async function placeOnCanvas(store: WorkspaceStore, entity: EntityEnvelope, surface: EntityEnvelope): Promise<string | null> {
  const renderer = defaultRendererFor(entity.type_id)
  if (!renderer) return null
  const free = surface.layout?.mode === 'free'
  const children = store.outline.childrenOf(surface.entity_id)
  const num = (v: unknown) => (typeof v === 'number' ? v : 0)
  const bottom = children.reduce((y, c) => (c.placement ? Math.max(y, num(c.placement.y) + num(c.placement.h)) : y), 0)
  const cellId = randomId('c')
  const op: Operation = {
    op: 'entity.create', entity_id: cellId, type_id: 'buckyos.cell', parent_id: surface.entity_id,
    order_key: store.core.order_key_between(children.at(-1)?.order_key ?? undefined, undefined),
    ...(free ? { placement: { x: 40, y: bottom + 40, w: 480, h: 320 } } : {}),
    payload: { view: { type: renderer }, source_ref: { entity_id: entity.entity_id } },
  }
  const outcome = await store.submit({ editId: `insert:${cellId}`, label: aiwsT('aiworkspace.datamode.placeEdit', { name: entityLabel(entity), canvas: entityLabel(surface) }), operations: [op] })
  return accepted(outcome) ? cellId : null
}
