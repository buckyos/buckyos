/* eslint-disable react-refresh/only-export-components -- Surface operations with their dialog */
/* Managing Surfaces (UI improvement §5): rename with the version seen when editing started (a concurrent
 * rename is a conflict, never silently overwritten), the shared preset icon, and the delete pre-check
 * (phase two §4.3: the BlockTree and the content folder go together; blocked references are listed,
 * readable ones only). */

import { useState } from 'react'
import { describeError } from '../../api/session'
import { SYSTEM_IDS, type CommitOutcome, type EntityEnvelope, type Json, type Operation } from '../../api/types'
import { useOverlayMounted } from '../shell/popover'
import type { WorkspaceStore } from '../../state/store'
import { useStore } from '../../state/hooks'
import { surfacesOf } from './surfaceOps'

export interface RenameBase { titleRev: number; metaRev: number }

/** The versions a rename will expect: read when the user starts editing. */
export async function renameBase(store: WorkspaceStore, surface: EntityEnvelope): Promise<RenameBase> {
  const read = await store.session.read<{ payload: Record<string, Json>; key_revs: Record<string, number> }>(surface.entity_id)
  return { titleRev: read.content.key_revs.title ?? 0, metaRev: read.meta_rev }
}

export function renameSurface(store: WorkspaceStore, surface: EntityEnvelope, title: string, base: RenameBase): Promise<CommitOutcome> {
  const ops: Operation[] = [
    { op: 'entity.set_keys', entity_id: surface.entity_id, keys: [{ key: 'title', value: title, expect: { rev: base.titleRev } }] },
    { op: 'entity.rename', entity_id: surface.entity_id, name: title, expect: { rev: base.metaRev } },
  ]
  return store.submit({ editId: `surface:${surface.entity_id}:title`, label: `重命名画布 → ${title}`, mine: title, hasMine: true, operations: ops })
}

export async function setSurfaceIcon(store: WorkspaceStore, surface: EntityEnvelope, icon: string | null): Promise<CommitOutcome> {
  const read = await store.session.read<{ payload: Record<string, Json>; key_revs: Record<string, number> }>(surface.entity_id)
  const rev = read.content.key_revs.icon ?? 0
  const op: Operation = icon
    ? { op: 'entity.set_keys', entity_id: surface.entity_id, keys: [{ key: 'icon', value: icon, expect: { rev } }] }
    : { op: 'entity.unset_keys', entity_id: surface.entity_id, keys: [{ key: 'icon', expect: { rev } }] }
  return store.submit({ editId: `surface:${surface.entity_id}:icon`, label: icon ? '更换画布图标' : '恢复默认画布图标', operations: [op] })
}

export function accepted(outcome: CommitOutcome): boolean {
  return outcome.status === 'accepted' || outcome.status === 'saved_locally'
}

/** Why a Surface's own properties (name, icon) cannot be changed now, or null. */
export function surfaceWriteReason(store: WorkspaceStore, surface: EntityEnvelope | null | undefined, viewOnly: boolean, readOnlyNow: string | null): string | null {
  if (!surface) return '没有画布'
  if (!surface.capabilities.includes('structure')) return '没有修改这张画布的权限'
  if (readOnlyNow) return readOnlyNow
  if (viewOnly) return '当前不是编辑模式'
  void store
  return null
}

export function SurfaceDeleteDialog({ surface, onClose, onDeleted }: { surface: EntityEnvelope; onClose: () => void; onDeleted: (nextSurfaceId: string | null) => void }) {
  const store = useStore()
  useOverlayMounted()
  const [state, setState] = useState<{ referrers: { entity_id: string; target: string }[] | null; hidden: boolean; busy: boolean }>({ referrers: null, hidden: false, busy: true })
  const [started, setStarted] = useState(false)
  if (!started) {
    setStarted(true)
    const folderId = surface.content_folder_id ?? ''
    const folder = folderId ? store.outline.get(folderId) : undefined
    const blocks = store.outline.descendants(surface.entity_id).map((e) => e.entity_id)
    const ops: Operation[] = [{ op: 'entity.delete', entity_id: surface.entity_id, ...(blocks.length ? { subtree: { delete: blocks } } : {}), expect: { rev: surface.life_rev } }]
    if (folder) {
      const inside = store.outline.descendants(folder.entity_id).map((e) => e.entity_id)
      ops.push({ op: 'entity.delete', entity_id: folder.entity_id, ...(inside.length ? { subtree: { delete: inside } } : {}), expect: { rev: folder.life_rev } })
    }
    store.session.prepare(ops).then((result) => {
      if (result.status === 'ok') { setState({ referrers: [], hidden: false, busy: false }); return }
      const data = result.status === 'rejected' ? (result.errors?.[0]?.data as { referrers?: { entity_id: string; target: string }[]; hidden_referrers?: boolean } | undefined) : undefined
      setState({ referrers: data?.referrers ?? [{ entity_id: result.code, target: '' }], hidden: Boolean(data?.hidden_referrers), busy: false })
    }, (error: unknown) => {
      store.notify('error', `删除预检失败：${describeError(error)}`)
      onClose()
    })
  }
  const remove = async (moveFirst: boolean) => {
    const folderId = surface.content_folder_id ?? ''
    const folder = folderId ? store.outline.get(folderId) : undefined
    const ops: Operation[] = []
    if (moveFirst && folder) {
      // "move to the data tree, then delete": the folder's children move under `data`
      let key = store.outline.childrenOf('data').filter((e) => !SYSTEM_IDS.has(e.entity_id)).at(-1)?.order_key
      for (const child of store.outline.childrenOf(folder.entity_id)) { key = store.core.order_key_between(key, undefined); ops.push({ op: 'tree.move', entity_id: child.entity_id, new_parent_id: 'data', order_key: key }) }
    }
    const blocks = store.outline.descendants(surface.entity_id).map((e) => e.entity_id)
    ops.push({ op: 'entity.delete', entity_id: surface.entity_id, ...(blocks.length ? { subtree: { delete: blocks } } : {}), expect: { rev: surface.life_rev } })
    if (folder) {
      const rest = moveFirst ? [] : store.outline.descendants(folder.entity_id).map((e) => e.entity_id)
      ops.push({ op: 'entity.delete', entity_id: folder.entity_id, ...(rest.length ? { subtree: { delete: rest } } : {}), expect: { rev: folder.life_rev } })
    }
    const outcome = await store.submit({ editId: `surface:${surface.entity_id}:delete`, label: `删除画布 ${surface.title ?? surface.name ?? ''}`, operations: ops })
    onClose()
    if (accepted(outcome)) onDeleted(surfacesOf(store).find((s) => s.entity_id !== surface.entity_id)?.entity_id ?? null)
  }
  const blocked = state.referrers !== null && (state.referrers.length > 0 || state.hidden)
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dialog" role="alertdialog" aria-modal="true" aria-label="删除画布" data-testid="aiws-surface-delete-dialog" onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
        <b>删除画布「{surface.title ?? surface.name}」</b>
        <div className="aiws-muted">画布上的 Block 和它的画布内容区会在同一次提交中删除；工作区和其他画布不受影响。</div>
        {state.busy && <div className="aiws-muted">正在预检…</div>}
        {state.referrers && state.referrers.length > 0 && (
          <div className="aiws-warning" data-testid="aiws-surface-delete-blocked">
            其中的数据仍被引用，不能直接删除：{state.referrers.map((r) => `${store.outline.get(r.entity_id)?.title ?? store.outline.get(r.entity_id)?.name ?? r.entity_id} → ${r.target}`).join('；')}
            {state.hidden && '；另有无权查看的引用'}
          </div>
        )}
        {state.referrers && state.referrers.length === 0 && state.hidden && <div className="aiws-warning">存在受限引用（你无权查看），不能直接删除。</div>}
        <div className="aiws-dialog-actions">
          {state.referrers && !blocked && <button type="button" className="is-danger" data-testid="aiws-surface-delete-confirm" autoFocus onClick={() => { void remove(false) }}>删除画布</button>}
          {blocked && <button type="button" data-testid="aiws-surface-delete-move" onClick={() => { void remove(true) }}>把画布内容移到数据树后再删除</button>}
          <button type="button" onClick={onClose}>取消</button>
        </div>
      </div>
    </div>
  )
}
