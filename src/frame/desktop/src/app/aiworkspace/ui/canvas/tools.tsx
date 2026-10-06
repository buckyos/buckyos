/* Canvas tools (phase two §8.1, §8.4): the Surface switcher, the near toolbar of the selection
 * (screen coordinates, edge avoidance), the insert menu of a blank spot, and the Block inspector.
 * Tools follow the task: nothing sits permanently on every Block. */

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Json, Operation } from '../../api/types'
import { useOutlineVersion, useStore, useWorkspaceUi } from '../../state/hooks'
import { modePolicy, type CanvasMode } from '../blocks/registry'
import type { Rect } from './render/camera'
import { createSurfaceOps, surfacesOf } from './surfaceOps'

// ---- Surface navigation

export function SurfaceNav({ active, onSelect }: { active: string | null; onSelect: (surfaceId: string) => void }) {
  const store = useStore()
  useOutlineVersion()
  const surfaces = surfacesOf(store)
  const [open, setOpen] = useState(false)
  const [creating, setCreating] = useState<null | { title: string; mode: 'free' | 'flow' }>(null)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<{ id: string; referrers: { entity_id: string; target: string }[] | null; hidden: boolean; busy: boolean } | null>(null)
  const canStructure = store.session.info().capabilities.includes('structure')
  const current = surfaces.find((s) => s.entity_id === active)
  const create = async () => {
    if (!creating) return
    const title = creating.title.trim() || `画布 ${surfaces.length + 1}`
    const { ops, surfaceId } = createSurfaceOps(store, title, creating.mode)
    const outcome = await store.submit({ editId: `surface:new`, label: `新建画布 ${title}`, operations: ops })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { setCreating(null); setOpen(false); onSelect(surfaceId) }
  }
  const rename = async (surface: EntityEnvelope, title: string) => {
    setRenaming(null)
    if (!title.trim() || title === surface.title) return
    const read = await store.session.read<{ payload: Record<string, Json>; key_revs: Record<string, number> }>(surface.entity_id)
    const ops: Operation[] = [
      { op: 'entity.set_keys', entity_id: surface.entity_id, keys: [{ key: 'title', value: title, expect: { rev: read.content.key_revs.title ?? 0 } }] },
      { op: 'entity.rename', entity_id: surface.entity_id, name: title, expect: { rev: surface.meta_rev } },
    ]
    void store.submit({ editId: `surface:${surface.entity_id}:title`, label: `重命名画布 → ${title}`, operations: ops })
  }
  /** Delete pre-check (§4.3): the BlockTree and the content folder go together; blocked references are listed (visible ones only). */
  const precheckDelete = async (surface: EntityEnvelope) => {
    setDeleting({ id: surface.entity_id, referrers: null, hidden: false, busy: true })
    const folderId = surface.content_folder_id ?? ''
    const subtree = [...store.outline.descendants(surface.entity_id), ...(folderId ? store.outline.descendants(folderId) : [])].map((e) => e.entity_id)
    const folder = folderId ? store.outline.get(folderId) : undefined
    const ops: Operation[] = [
      { op: 'entity.delete', entity_id: surface.entity_id, ...(store.outline.descendants(surface.entity_id).length ? { subtree: { delete: store.outline.descendants(surface.entity_id).map((e) => e.entity_id) } } : {}), expect: { rev: surface.life_rev } },
    ]
    if (folder) ops.push({ op: 'entity.delete', entity_id: folder.entity_id, ...(store.outline.descendants(folder.entity_id).length ? { subtree: { delete: store.outline.descendants(folder.entity_id).map((e) => e.entity_id) } } : {}), expect: { rev: folder.life_rev } })
    void subtree
    try {
      const result = await store.session.prepare(ops)
      if (result.status === 'ok') { setDeleting({ id: surface.entity_id, referrers: [], hidden: false, busy: false }); return }
      const data = result.status === 'rejected' ? (result.errors?.[0]?.data as { referrers?: { entity_id: string; target: string }[]; hidden_referrers?: boolean } | undefined) : undefined
      setDeleting({ id: surface.entity_id, referrers: data?.referrers ?? [{ entity_id: result.code, target: '' }], hidden: Boolean(data?.hidden_referrers), busy: false })
    } catch (error) {
      store.notify('error', `删除预检失败：${describeError(error)}`)
      setDeleting(null)
    }
  }
  const doDelete = async (surface: EntityEnvelope, moveFirst: boolean) => {
    const folderId = surface.content_folder_id ?? ''
    const folder = folderId ? store.outline.get(folderId) : undefined
    const ops: Operation[] = []
    if (moveFirst && folder) {
      // "move to the data tree, then delete": the folder's children move under `data`
      const last = store.outline.childrenOf('data').filter((e) => e.entity_id !== 'canvas-content').at(-1)?.order_key
      let key = last
      for (const child of store.outline.childrenOf(folder.entity_id)) { key = store.core.order_key_between(key, undefined); ops.push({ op: 'tree.move', entity_id: child.entity_id, new_parent_id: 'data', order_key: key }) }
    }
    const blocks = store.outline.descendants(surface.entity_id).map((e) => e.entity_id)
    ops.push({ op: 'entity.delete', entity_id: surface.entity_id, ...(blocks.length ? { subtree: { delete: blocks } } : {}), expect: { rev: surface.life_rev } })
    if (folder) {
      const rest = moveFirst ? [] : store.outline.descendants(folder.entity_id).map((e) => e.entity_id)
      ops.push({ op: 'entity.delete', entity_id: folder.entity_id, ...(rest.length ? { subtree: { delete: rest } } : {}), expect: { rev: folder.life_rev } })
    }
    const outcome = await store.submit({ editId: `surface:${surface.entity_id}:delete`, label: `删除画布 ${surface.title ?? surface.name ?? ''}`, operations: ops })
    setDeleting(null)
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') onSelect(surfaces.find((s) => s.entity_id !== surface.entity_id)?.entity_id ?? '')
  }
  return (
    <div className="aiws-surface-nav" data-testid="aiws-surface-nav">
      <button type="button" className="aiws-surface-current" data-testid="aiws-surface-switch" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        {current ? `${current.title ?? current.name ?? current.entity_id}` : '（没有画布）'} <span className="aiws-muted">{current?.layout?.mode === 'free' ? '自由' : current ? '流式' : ''}</span> ▾
      </button>
      {open && (
        <div className="aiws-menu aiws-surface-list" role="menu" data-testid="aiws-surface-list">
          {surfaces.map((surface) => (
            <div key={surface.entity_id} className={`aiws-surface-item${surface.entity_id === active ? ' is-active' : ''}`} data-testid={`aiws-surface-item-${surface.entity_id}`}>
              {renaming === surface.entity_id ? (
                <input autoFocus aria-label="画布标题" defaultValue={surface.title ?? surface.name ?? ''} onBlur={(event) => { void rename(surface, event.target.value) }} onKeyDown={(event) => { if (event.key === 'Enter') (event.target as HTMLInputElement).blur(); if (event.key === 'Escape') setRenaming(null) }} />
              ) : (
                <button type="button" role="menuitem" className="aiws-link" onClick={() => { onSelect(surface.entity_id); setOpen(false) }}>{surface.title ?? surface.name ?? surface.entity_id}</button>
              )}
              <span className="aiws-muted">{surface.layout?.mode === 'free' ? '自由' : '流式'}</span>
              {surface.capabilities.includes('structure') && <button type="button" className="aiws-icon" title="重命名" onClick={() => setRenaming(surface.entity_id)}>✎</button>}
              {surface.capabilities.includes('delete') && <button type="button" className="aiws-icon" title="删除画布（先预检）" data-testid={`aiws-surface-delete-${surface.entity_id}`} onClick={() => { void precheckDelete(surface) }}>×</button>}
            </div>
          ))}
          {canStructure && (creating ? (
            <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
              <input autoFocus aria-label="新画布标题" placeholder="画布标题" value={creating.title} onChange={(event) => setCreating({ ...creating, title: event.target.value })} />
              <select aria-label="布局" value={creating.mode} onChange={(event) => setCreating({ ...creating, mode: event.target.value === 'flow' ? 'flow' : 'free' })}><option value="free">自由画布</option><option value="flow">流式页</option></select>
              <button type="submit" data-testid="aiws-surface-create">创建</button>
              <button type="button" onClick={() => setCreating(null)}>取消</button>
            </form>
          ) : <button type="button" data-testid="aiws-surface-new" onClick={() => setCreating({ title: '', mode: 'free' })}>+ 新建画布</button>)}
        </div>
      )}
      {deleting && (() => {
        const surface = surfaces.find((s) => s.entity_id === deleting.id)
        if (!surface) return null
        return (
          <div className="aiws-dialog" role="alertdialog" aria-label="删除画布" data-testid="aiws-surface-delete-dialog">
            <b>删除画布「{surface.title ?? surface.name}」</b>
            <div className="aiws-muted">画布上的 Block 和它的画布内容区会在同一次提交中删除。</div>
            {deleting.busy && <div className="aiws-muted">正在预检…</div>}
            {deleting.referrers && deleting.referrers.length > 0 && (
              <div className="aiws-warning" data-testid="aiws-surface-delete-blocked">
                其中的数据仍被引用，不能直接删除：{deleting.referrers.map((r) => `${store.outline.get(r.entity_id)?.title ?? store.outline.get(r.entity_id)?.name ?? r.entity_id} → ${r.target}`).join('；')}
                {deleting.hidden && '；另有无权查看的引用'}
              </div>
            )}
            {deleting.referrers && deleting.referrers.length === 0 && deleting.hidden && <div className="aiws-warning">存在无权查看的引用，不能直接删除。</div>}
            <div className="aiws-inline-form">
              {deleting.referrers && deleting.referrers.length === 0 && !deleting.hidden && <button type="button" data-testid="aiws-surface-delete-confirm" onClick={() => { void doDelete(surface, false) }}>确认删除</button>}
              {deleting.referrers && (deleting.referrers.length > 0 || deleting.hidden) && <button type="button" data-testid="aiws-surface-delete-move" onClick={() => { void doDelete(surface, true) }}>把画布内容移到数据树后再删除</button>}
              <button type="button" onClick={() => setDeleting(null)}>取消</button>
            </div>
          </div>
        )
      })()}
    </div>
  )
}

// ---- near toolbar

export interface NearAction { id: string; label: string; run: () => void; disabled?: boolean; title?: string; key?: string }

export function NearToolbar({ bbox, actions, viewport }: { bbox: Rect; actions: NearAction[]; viewport: { w: number; h: number } }) {
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  useLayoutEffect(() => { const el = ref.current; if (el) setSize({ w: el.offsetWidth, h: el.offsetHeight }) }, [actions.length])
  // above the selection, inside the viewport (edge avoidance, §8.4)
  let top = bbox.y - size.h - 8
  if (top < 4) top = Math.min(viewport.h - size.h - 4, bbox.y + bbox.h + 8)
  let left = bbox.x + bbox.w / 2 - size.w / 2
  left = Math.max(4, Math.min(left, viewport.w - size.w - 4))
  if (actions.length === 0) return null
  return (
    <div ref={ref} className="aiws-near" role="toolbar" aria-label="就近工具" data-testid="aiws-near-toolbar" style={{ left, top }} onPointerDown={(event) => event.stopPropagation()}>
      {actions.map((action) => <button key={action.id} type="button" data-testid={`aiws-near-${action.id}`} disabled={action.disabled} title={action.title ?? (action.key ? `${action.label}（${action.key}）` : action.label)} onClick={action.run}>{action.label}</button>)}
    </div>
  )
}

// ---- context menu

export function ContextMenu({ at, items, onClose }: { at: { x: number; y: number }; items: { id: string; label: string; run: () => void; disabled?: boolean; separator?: boolean }[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey) }
  }, [onClose])
  return (
    <div ref={ref} className="aiws-menu aiws-context-menu" role="menu" data-testid="aiws-context-menu" style={{ left: at.x, top: at.y }}>
      {items.map((item) => item.separator ? <hr key={item.id} /> : (
        <button key={item.id} type="button" role="menuitem" data-testid={`aiws-menu-${item.id}`} disabled={item.disabled} onClick={() => { onClose(); item.run() }}>{item.label}</button>
      ))}
    </div>
  )
}

// ---- inspector

export function BlockInspector({ cellId, mode, children }: { cellId: string; mode: CanvasMode; children?: ReactNode }) {
  const ui = useWorkspaceUi()
  const entity = ui.byId.get(cellId)
  const policy = modePolicy(mode)
  if (!entity) return null
  return (
    <div className="aiws-inspector" data-testid="aiws-inspector" data-mode={mode}>
      <div className="aiws-panel-title">属性 <span className="aiws-muted">{entity.view_type ?? entity.kind}{entity.view_version ? ` v${entity.view_version}` : ''}</span></div>
      <div className="aiws-muted">{cellId}{entity.source_id ? ` · 数据 ${entity.source_id}` : ' · 纯 UI Block'}</div>
      {!policy.writes && <div className="aiws-muted">{mode === 'view' ? '查看模式：属性只读' : '播放编辑占位：只读'}</div>}
      {children}
    </div>
  )
}
