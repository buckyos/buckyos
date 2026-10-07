/* The host side of the HTML extension API (phase two §10.4): everything an extension can do goes
 * through here, so its writes carry the same permissions, locks, undo and save states as a user's.
 * Not a security boundary (D16) — a boundary of bookkeeping. */

import type { CellPayload, EntityEnvelope, Json, KeyedContent, Operation, QueryParams, Selector } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { modePolicy, type CanvasMode } from '../blocks/registry'
import type { HostBridge } from './htmlRuntime'

export interface BridgeTarget {
  cell: EntityEnvelope
  payload: CellPayload
  keyRevs: Record<string, number>
  source: EntityEnvelope | undefined
  mode: CanvasMode
  /** Extra context handed to the extension (wish executors get the wish and its inputs). */
  extra?: Record<string, Json>
}

export function makeBridge(store: WorkspaceStore, target: BridgeTarget, onSnapshot?: (objectId: string, mediaType: string) => void): HostBridge {
  const policy = modePolicy(target.mode)
  return {
    context: () => ({
      cell_id: target.cell.entity_id,
      title: target.payload.title ?? null,
      config: (target.payload.config ?? {}) as Json,
      source: target.source ? { entity_id: target.source.entity_id, type_id: target.source.type_id, name: target.source.name, title: target.source.title ?? null } : null,
      mode: target.mode,
      principal: store.session.principal,
      workspace_id: store.session.workspaceId,
      ...(target.extra ?? {}),
    }),
    read: async (entityId, selector) => {
      const result = await store.session.read<Json>(entityId, (selector ?? undefined) as Selector | undefined)
      return result as unknown as Json
    },
    query: async (params) => (await store.session.query(params as unknown as QueryParams)) as unknown as Json,
    submit: async (operations, label) => {
      if (!policy.writes) throw new Error(`当前子模式（${target.mode}）不允许扩展写入文档`)
      if (!Array.isArray(operations) || operations.length === 0) throw new Error('operations 必须是非空数组')
      const outcome = await store.submit({ editId: `ext:${target.cell.entity_id}:${Date.now()}`, label: label || `扩展 ${target.payload.view.type} 的修改`, operations: operations as Operation[] })
      return outcome as unknown as Json
    },
    upload: async (bytes, fileName, mediaType) => {
      const blob = new Blob([bytes as BlobPart], mediaType ? { type: mediaType } : undefined)
      return store.session.uploadAsset(blob, fileName)
    },
    snapshot: async (dataUrl) => {
      if (!policy.writes) throw new Error(`当前子模式（${target.mode}）不允许扩展写入文档`)
      // the static snapshot is an asset: what the Block shows before it is activated (§10.4 lifecycle)
      const match = /^data:([^;,]+)(;base64)?,(.*)$/s.exec(dataUrl)
      if (!match) throw new Error('snapshot 需要 data URL')
      const mediaType = match[1]
      const bytes = match[2] ? Uint8Array.from(atob(match[3]), (c) => c.charCodeAt(0)) : new TextEncoder().encode(decodeURIComponent(match[3]))
      const uploaded = await store.session.uploadAsset(new Blob([bytes], { type: mediaType }), `snapshot.${mediaType.includes('svg') ? 'svg' : mediaType.includes('png') ? 'png' : 'bin'}`)
      const snapshot = { object_id: uploaded.object_id, media_type: uploaded.media_type, size: uploaded.size }
      // the snapshot is bookkeeping of the Block, not a user edit: no undo entry; a concurrent config edit wins
      const current = await store.session.read<KeyedContent<CellPayload>>(target.cell.entity_id)
      const outcome = await store.submit({
        editId: `ext-snapshot:${target.cell.entity_id}`, label: '扩展快照', undoable: false,
        operations: [{ op: 'entity.set_keys', entity_id: target.cell.entity_id, keys: [{ key: 'config', value: { ...(current.content.payload.config ?? {}), snapshot } as Json, expect: { rev: current.content.key_revs.config ?? 0 } }] }],
      })
      if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') throw new Error('扩展快照未保存，请查看保存状态后重试')
      onSnapshot?.(uploaded.object_id, uploaded.media_type)
    },
    notify: (text) => store.notify('info', text),
  }
}
