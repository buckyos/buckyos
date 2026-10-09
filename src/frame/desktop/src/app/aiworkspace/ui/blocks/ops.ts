/* Operation builders shared by Block definitions and insertion. */

import { describeError } from '../../api/session'
import type { AssetContent, Json, Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import type { CreateArgs, RenderContext } from './registry'

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
    default: return null
  }
}

/** Upload `file` and point the asset at it (one undoable write). */
export async function replaceAsset(store: WorkspaceStore, entityId: string, file: File): Promise<string | null> {
  try {
    const current = await store.session.read<AssetContent>(entityId)
    const uploaded = await store.session.uploadAsset(file, file.name)
    const revs = current.content.key_revs ?? {}
    const outcome = await store.submit({
      editId: `key:${entityId}:object_id`, label: `替换资产 ${current.content.payload.file_name ?? entityId}`,
      operations: [{ op: 'entity.set_keys', entity_id: entityId, keys: [
        { key: 'object_id', value: uploaded.object_id, expect: { rev: revs.object_id ?? 0 } },
        { key: 'file_name', value: file.name, expect: { rev: revs.file_name ?? 0 } },
      ] }],
    })
    return outcome.status === 'accepted' || outcome.status === 'saved_locally' ? null : '替换未被接受，见“需要处理”'
  } catch (error) {
    return `上传失败：${describeError(error)}`
  }
}

/** Merge `patch` into a Cell's `config` (null removes a key): one undoable write, guarded by the key's version. */
export function setCellConfig(context: RenderContext, store: WorkspaceStore, patch: Record<string, Json | null>, label: string) {
  const config: Record<string, Json> = { ...(context.payload.config ?? {}) }
  for (const [key, value] of Object.entries(patch)) { if (value === null) delete config[key]; else config[key] = value }
  void store.submit({ editId: `config:${context.cell.entity_id}`, label, operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: config, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
}

/** Set (or with null, remove) one key of a Cell. */
export function setCellKey(context: RenderContext, store: WorkspaceStore, key: string, value: Json | null, label: string) {
  const expect = { rev: context.keyRevs[key] ?? 0 }
  void store.submit({ editId: `key:${context.cell.entity_id}:${key}`, label, operations: [value === null
    ? { op: 'entity.unset_keys', entity_id: context.cell.entity_id, keys: [{ key, expect }] }
    : { op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key, value, expect }] }] })
}
