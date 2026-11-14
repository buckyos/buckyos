/* Operation builders shared by Block definitions and insertion. */

import { describeError } from '../../api/session'
import { aiwsT } from '../../i18n'
import type { AssetContent, CellPayload, Json, KeyedContent, Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import type { CreateArgs, RenderContext } from './registry'
import { mediaKindOf } from '../media/kinds'
import { stageMediaFile } from '../media/stage'

export const MAX_EMBED_DEPTH = 3

export function createOp(entityId: string, typeId: string, parentId: string, orderKey: string, payload: Record<string, Json>, name?: string, placement?: CreateArgs['placement']): Operation {
  return { op: 'entity.create', entity_id: entityId, type_id: typeId, parent_id: parentId, order_key: orderKey, ...(name ? { name } : {}), ...(placement ? { placement } : {}), payload }
}

export function cellOp(args: CreateArgs, view: string, sourceId: string | undefined, extra: Record<string, Json> = {}): Operation {
  const payload: Record<string, Json> = { view: { type: view }, ...(sourceId ? { source_ref: { entity_id: sourceId } } : {}), ...(args.title ? { title: args.title } : {}), ...extra }
  if (args.config) payload.config = args.config
  return createOp(args.cellId, 'buckyos.cell', args.parentId, args.orderKey, payload, undefined, args.placement)
}

/** The Renderer used to show a data entity by default ("add to canvas"). */
export function defaultRendererFor(typeId: string): string | null {
  switch (typeId) {
    case 'buckyos.table-source': return 'table'
    case 'buckyos.richtext': return 'richtext'
    case 'buckyos.record': return 'record'
    case 'buckyos.asset-ref': return 'asset'
    case 'buckyos.annotation': return 'note'
    case 'buckyos.wish': return 'wish'
    case 'buckyos.dir': return 'directory'
    default: return null
  }
}

/** Upload `file` (with its poster, 多媒体对象 §3) and point the asset at it (one undoable write); the backend drops the
 * old content's poster, size and length the new file does not bring. */
export async function replaceAsset(store: WorkspaceStore, entityId: string, file: File): Promise<string | null> {
  try {
    const current = await store.session.read<AssetContent>(entityId)
    const { payload } = await stageMediaFile(store, file)
    const revs = current.content.key_revs ?? {}
    const outcome = await store.submit({
      editId: `key:${entityId}:object_id`, label: aiwsT('aiworkspace.blocks.edit.replaceAsset', { name: current.content.payload.file_name ?? entityId }),
      operations: [{ op: 'entity.set_keys', entity_id: entityId, keys: Object.entries(payload).map(([key, value]) => ({ key, value: value as Json, expect: { rev: revs[key] ?? 0 } })) }],
    })
    return outcome.status === 'accepted' || outcome.status === 'saved_locally' ? null : aiwsT('aiworkspace.blocks.asset.replaceRejected')
  } catch (error) {
    return aiwsT('aiworkspace.blocks.asset.uploadFailed', { error: describeError(error) })
  }
}

/** What "replace" offers to pick: a file of the same kind; any file for a file card. */
export function replaceAccept(payload: AssetContent['payload']): string | undefined {
  switch (mediaKindOf(payload.media_type, payload.file_name)) {
    case 'image': return 'image/*'
    case 'video': return 'video/*'
    case 'audio': return 'audio/*'
    case 'pdf': return 'application/pdf,.pdf'
    default: return undefined
  }
}

/** The object keys whose properties are version cells of their own (扩展控件 P2, Q28). */
export type PropKey = 'config' | 'bindings' | 'on'

/** `entity.set_props` of the changed properties (`null` removes one), each guarded by its own version cell
 * `p:<key>.<name>` as last read (0 when never written on its own). */
export function setPropsOp(entityId: string, key: PropKey, props: Record<string, Json | null>, keyRevs: Record<string, number>): Operation {
  return { op: 'entity.set_props', entity_id: entityId, key, props, expect: Object.fromEntries(Object.keys(props).map((name) => [name, keyRevs[`p:${key}.${name}`] ?? 0])) }
}

/** The properties that differ between two values of an object key (`null` for a removed one). */
export function changedProps(before: Record<string, Json> | null | undefined, after: Record<string, Json> | null | undefined): Record<string, Json | null> {
  const a = before ?? {}
  const b = after ?? {}
  const out: Record<string, Json | null> = {}
  for (const name of new Set([...Object.keys(a), ...Object.keys(b)])) {
    if (!(name in b)) out[name] = null
    else if (!(name in a) || JSON.stringify(a[name]) !== JSON.stringify(b[name])) out[name] = b[name]
  }
  return out
}

/** The Commit-level precondition of a write to a control instance (Q29): the definition at the content revision this
 * client's outline knows. A definition rebuilt meanwhile refuses the write (online and on an offline replay alike). */
export function definitionPreconditions(store: WorkspaceStore, defId: string | null | undefined): unknown[] | undefined {
  const def = defId ? store.outline.get(defId) : undefined
  return def && def.type_id === 'buckyos.block-def' ? [{ target: { entity_id: def.entity_id }, expect: { rev: def.content_rev } }] : undefined
}

/** Write the changed properties of an instance's (or any Block's) object key: one undoable Commit of `entity.set_props`,
 * with the definition precondition when the entity is a control instance. Nothing is written when nothing changed. */
export async function submitProps(store: WorkspaceStore, args: { entityId: string; key: PropKey; props: Record<string, Json | null>; keyRevs: Record<string, number>; defId?: string | null; label: string; editId?: string; undoable?: boolean }) {
  if (Object.keys(args.props).length === 0) return null
  return store.submit({
    editId: args.editId ?? `${args.key}:${args.entityId}`, label: args.label, undoable: args.undoable,
    operations: [setPropsOp(args.entityId, args.key, args.props, args.keyRevs)],
    preconditions: definitionPreconditions(store, args.defId),
  })
}

const configChains = new Map<string, Promise<unknown>>()

/** Write `patch` into a Cell's `config` (null removes a key): one undoable `entity.set_props`, each property guarded by
 * its own version, so someone changing another property of the same Block keeps theirs (Q28). Patches of one Cell run
 * one after another, each on the versions as they are then (quick successive tool clicks would otherwise write against a
 * version the previous click already replaced). */
export function setCellConfig(context: RenderContext, store: WorkspaceStore, patch: Record<string, Json | null>, label: string) {
  const id = context.cell.entity_id
  const run = async () => {
    const read = await store.session.read<KeyedContent<CellPayload>>(id)
    const current = read.content.payload.config ?? {}
    const props = Object.fromEntries(Object.entries(patch).filter(([key, value]) => (value === null ? key in current : JSON.stringify(current[key]) !== JSON.stringify(value))))
    await submitProps(store, { entityId: id, key: 'config', props, keyRevs: read.content.key_revs, defId: read.content.payload.def_ref?.entity_id, label, editId: `config:${id}` })
  }
  const next = (configChains.get(id) ?? Promise.resolve()).then(run, run)
  configChains.set(id, next)
  void next.finally(() => { if (configChains.get(id) === next) configChains.delete(id) })
}

/** Set (or with null, remove) one key of a Cell. */
export function setCellKey(context: RenderContext, store: WorkspaceStore, key: string, value: Json | null, label: string) {
  const expect = { rev: context.keyRevs[key] ?? 0 }
  void store.submit({ editId: `key:${context.cell.entity_id}:${key}`, label, operations: [value === null
    ? { op: 'entity.unset_keys', entity_id: context.cell.entity_id, keys: [{ key, expect }] }
    : { op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key, value, expect }] }] })
}
