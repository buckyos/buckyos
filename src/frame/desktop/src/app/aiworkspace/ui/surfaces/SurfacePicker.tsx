/* The canvas picker (UI 框架 PRD §4.1.1, §6.5, Q5, Q9): the drop-down beside the canvas name — on a phone the whole
 * title opens it, filling the screen. Head: the workspace and “back to home”; a search that always shows, over the
 * whole tree (canvas and folder names, no case), flat with each hit's folders, Enter opens the first; the recent
 * canvases (more than 20 canvases: the last 5 this person opened); the folder tree, collapsible, the current canvas
 * marked; at the foot “new canvas” and “new folder” (`structure`). Every row has the same menu on ⋯, a right click
 * or a long press: rename, copy, move to…, permissions…, delete, copy path; a folder also “new canvas here”. On a
 * computer rows drag to reorder or into a folder; touch moves with “move to…”. Folders are the shared tree (ordinary
 * undoable Commits); what is collapsed, the order by name and the recent list are this person's work state. The list
 * renders only what is on screen, so hundreds of canvases stay quick. */

import { useEffect, useMemo, useRef, useState, type DragEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { ArrowDownAZ, ChevronDown, ChevronLeft, ChevronRight, Ellipsis, Folder, FolderOpen, FolderPlus, House, LayoutDashboard, Plus, Search, X } from 'lucide-react'
import { describeError } from '../../api/session'
import type { EntityEnvelope } from '../../api/types'
import { useOutlineVersion, useReadOnlyReason, useStore, useUserState } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { SurfaceIcon } from '../canvas/icons'
import { accepted, deleteSurfaceFolder, renameBase, renameSurface, type RenameBase } from '../canvas/surfaceManage'
import { duplicateSurfaceOps } from '../canvas/surfaceOps'
import { MenuButton, PopoverPanel, usePopover, type MenuItem } from '../shell/popover'
import { useShell } from '../shell/shellContext'
import {
  allFolders, canMoveInto, COLLAPSED_KEY, createFolderOps, entityPath, entriesOf, entryLabel, folderTrail, moveEntryOps, RECENT_KEY, requestEntityPermissions,
  searchEntries, SORT_KEY, SURFACES_ROOT, treeRows, type SortMode,
} from './tree'
import './surfaces.css'

/** More canvases than this: the recent ones are listed first (PRD §4.1.1). */
const RECENT_FROM = 20
const RECENT_SHOWN = 5
const RECENT_KEPT = 8
const LONG_PRESS_MS = 550

type Row =
  | { type: 'heading'; key: string; label: string }
  | { type: 'empty'; key: string; label: string }
  | { type: 'entry'; key: string; entity: EntityEnvelope; depth: number; collapsed: boolean; childCount: number; trail: string | null; testId: string }

type DropAt = 'before' | 'after' | 'into'

function coarsePointer(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(pointer: coarse)').matches
}

/** The picker's trigger and its panel. The phone's trigger is the canvas icon and name. */
export function SurfacePicker({ phone = false }: { phone?: boolean }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const active = shell.activeSurface
  const activeId = active?.entity_id ?? null
  const workspaceTitle = store.session.info().title
  // the canvases this person opened, newest first (user work state; the list shows the first few that still exist)
  useEffect(() => {
    if (!activeId) return
    let live = true
    void store.userReady.then(() => {
      if (!live) return
      const prev = store.userState.get<string[]>(RECENT_KEY) ?? []
      if (prev[0] === activeId) return
      store.userState.set(RECENT_KEY, [activeId, ...prev.filter((id) => id !== activeId && store.outline.get(id))].slice(0, RECENT_KEPT))
    })
    return () => { live = false }
  }, [store, activeId])
  return (
    <span ref={bindAnchor} className={`aiws-anchor${phone ? ' aiws-surface-anchor' : ''}`}>
      {phone ? (
        <button ref={bindTrigger} type="button" className="aiws-surface-trigger" aria-label={t('aiworkspace.shell.switcher.label', { name: active ? entryLabel(active) : workspaceTitle })} title={t('aiworkspace.shell.switchCanvas')} aria-haspopup="dialog" aria-expanded={open}
          data-testid="aiws-surface-switch" data-icon={active?.icon ?? ''} onClick={toggle}>
          {active ? <SurfaceIcon surface={active} size={20} /> : <LayoutDashboard size={20} aria-hidden="true" />}
          <span className="aiws-surface-trigger-name" data-testid="aiws-surface-name">{active ? entryLabel(active) : workspaceTitle}</span>
          <ChevronDown size={16} aria-hidden="true" />
        </button>
      ) : (
        <button ref={bindTrigger} type="button" className="aiws-tool aiws-tool-narrow" aria-label={t('aiworkspace.shell.switchCanvas')} title={t('aiworkspace.shell.switchCanvas')} aria-haspopup="dialog" aria-expanded={open} data-testid="aiws-surface-switch" onClick={toggle}>
          <ChevronDown size={16} />
        </button>
      )}
      {open && (phone
        ? <div className="aiws-popover aiws-surface-list aiws-sp aiws-sp-phone" role="dialog" aria-label={t('aiworkspace.surfaces.picker')} data-testid="aiws-surface-list"><PickerBody phone onClose={close} /></div>
        : <PopoverPanel label={t('aiworkspace.surfaces.picker')} testId="aiws-surface-list" className="aiws-surface-list aiws-sp"><PickerBody phone={false} onClose={close} /></PopoverPanel>)}
    </span>
  )
}

function PickerBody({ phone, onClose }: { phone: boolean; onClose: () => void }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const version = useOutlineVersion()
  const readOnly = useReadOnlyReason()
  const [query, setQuery] = useState('')
  const [moving, setMoving] = useState<string | null>(null)
  const [renaming, setRenaming] = useState<{ id: string; value: string; base: Promise<RenameBase> } | null>(null)
  const [newFolder, setNewFolder] = useState<{ parentId: string; value: string } | null>(null)
  const [drag, setDrag] = useState<{ id: string; over: { id: string; at: DropAt } | null } | null>(null)
  const [reveal, setReveal] = useState<string | null>(null)
  const collapsedList = useUserState<string[]>(COLLAPSED_KEY)
  const collapsed = useMemo(() => new Set(collapsedList ?? []), [collapsedList])
  const sort: SortMode = useUserState<SortMode>(SORT_KEY) === 'name' ? 'name' : 'manual'
  const recentIds = useUserState<string[]>(RECENT_KEY)
  const coarse = useMemo(coarsePointer, [])
  const rowHeight = phone || coarse ? 48 : 34
  const activeId = shell.activeSurface?.entity_id ?? null
  const count = shell.surfaces.length
  const root = store.outline.get(SURFACES_ROOT)
  const canStructure = (e: EntityEnvelope | undefined) => Boolean(e?.capabilities.includes('structure'))
  const rootReason = readOnly ?? (canStructure(root) ? null : t('aiworkspace.surfaces.noStructureHere'))
  const guest = shell.identity.guest

  const rows = useMemo<Row[]>(() => {
    void version
    const byName = sort === 'name'
    const entryRow = (entity: EntityEnvelope, depth: number, collapsedNow: boolean, childCount: number, trail: string | null, testId: string): Row =>
      ({ type: 'entry', key: testId, entity, depth, collapsed: collapsedNow, childCount, trail, testId })
    const idOf = (e: EntityEnvelope) => (e.kind === 'folder' ? `aiws-surface-folder-${e.entity_id}` : `aiws-surface-item-${e.entity_id}`)
    const trailOf = (id: string) => folderTrail(store, id).map(entryLabel).join(' / ')
    if (query.trim()) {
      const hits = searchEntries(store, query, byName)
      if (hits.length === 0) return [{ type: 'empty', key: 'empty', label: t('aiworkspace.surfaces.noMatch', { query: query.trim() }) }]
      return hits.map((hit) => entryRow(hit.entity, 0, false, 0, hit.trail || null, idOf(hit.entity)))
    }
    const out: Row[] = []
    if (count > RECENT_FROM) {
      const recent = (recentIds ?? []).map((id) => store.outline.get(id)).filter((e): e is EntityEnvelope => Boolean(e && !e.deleted && e.kind === 'surface')).slice(0, RECENT_SHOWN)
      if (recent.length > 0) {
        out.push({ type: 'heading', key: 'h-recent', label: t('aiworkspace.surfaces.recent') })
        for (const e of recent) out.push(entryRow(e, 0, false, 0, trailOf(e.entity_id) || null, `aiws-surface-recent-${e.entity_id}`))
        out.push({ type: 'heading', key: 'h-all', label: t('aiworkspace.surfaces.all') })
      }
    }
    const tree = treeRows(store, collapsed, byName)
    if (tree.length === 0) out.push({ type: 'empty', key: 'empty', label: t('aiworkspace.surfaces.empty') })
    for (const r of tree) out.push(entryRow(r.entity, r.depth, r.collapsed, r.childCount, null, idOf(r.entity)))
    return out
  }, [store, version, query, sort, collapsed, recentIds, count, t])

  const listRef = useRef<HTMLDivElement>(null)
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns unstable functions by design; nothing here is memoised on them.
  const virtualizer = useVirtualizer({ count: rows.length, getScrollElement: () => listRef.current, estimateSize: (i) => (rows[i]?.type === 'heading' ? 26 : rowHeight), overscan: 12 })
  // the current canvas in view when the picker opens; a folder found by the search, once revealed
  const scrolled = useRef(false)
  useEffect(() => {
    const target = reveal ?? (scrolled.current ? null : activeId)
    if (!target) return
    const index = rows.findIndex((r) => r.type === 'entry' && r.entity.entity_id === target && !r.testId.startsWith('aiws-surface-recent-'))
    if (index >= 0) virtualizer.scrollToIndex(index, { align: 'auto' })
    scrolled.current = true
    if (reveal) setReveal(null)
  }, [rows, reveal, activeId, virtualizer])

  const fail = (outcome: Awaited<ReturnType<typeof store.submit>>) => { if (!accepted(outcome)) store.notify('error', t('aiworkspace.surfaces.failed', { code: 'code' in outcome ? outcome.code : outcome.status })) }
  const setCollapsed = (next: Set<string>) => store.userState.set(COLLAPSED_KEY, next.size > 0 ? [...next].filter((id) => store.outline.get(id)) : null)
  const toggleFolder = (id: string) => { const next = new Set(collapsed); if (next.has(id)) next.delete(id); else next.add(id); setCollapsed(next) }
  const pick = (id: string) => { shell.selectSurface(id); shell.setTopMode('canvas'); onClose() }
  const showFolder = (id: string) => {
    const next = new Set(collapsed)
    for (const x of [id, ...store.outline.ancestors(id)]) next.delete(x)
    if (next.size !== collapsed.size) setCollapsed(next)
    setQuery('')
    setReveal(id)
  }
  const openEntry = (e: EntityEnvelope, fromSearch: boolean) => {
    if (e.kind === 'folder') { if (fromSearch) showFolder(e.entity_id); else toggleFolder(e.entity_id) } else pick(e.entity_id)
  }
  const newCanvas = (folderId?: string) => { onClose(); shell.openDialog({ kind: 'new', tab: 'canvas', ...(folderId ? { folderId } : {}) }) }
  const createFolder = async () => {
    if (!newFolder) return
    const title = newFolder.value.trim() || t('aiworkspace.surfaces.folderDefault')
    const { ops, folderId } = createFolderOps(store, title, newFolder.parentId)
    setNewFolder(null)
    const outcome = await store.submit({ editId: `surface:${folderId}`, label: t('aiworkspace.surfaces.createFolderLabel', { name: title }), operations: ops })
    fail(outcome)
    if (accepted(outcome)) showFolder(folderId)
  }
  const rename = async () => {
    if (!renaming) return
    const entity = store.outline.get(renaming.id)
    const title = renaming.value.trim()
    setRenaming(null)
    if (!entity || !title || title === entryLabel(entity)) return
    let base: RenameBase
    try { base = await renaming.base } catch (failure) { store.notify('error', t('aiworkspace.shell.cannotReadVersions', { error: describeError(failure) })); return }
    fail(await renameSurface(store, entity, title, base))
  }
  const startRename = (e: EntityEnvelope) => {
    const base = renameBase(store, e)
    base.catch(() => undefined)
    setRenaming({ id: e.entity_id, value: entryLabel(e), base })
  }
  const move = async (id: string, parentId: string, at: { before?: string; after?: string } | null = null) => {
    const entity = store.outline.get(id)
    if (!entity || !canMoveInto(store, id, parentId)) return
    fail(await store.submit({ editId: `surface:${id}:move`, label: t('aiworkspace.surfaces.moveLabel', { name: entryLabel(entity) }), operations: moveEntryOps(store, id, parentId, at) }))
  }
  const duplicate = (e: EntityEnvelope) => {
    void duplicateSurfaceOps(store, e.entity_id)
      .then((made) => store.submit({ editId: `surface:${made.surfaceId}`, label: t('aiworkspace.surfaces.duplicate'), operations: made.ops }).then((outcome) => { fail(outcome); if (accepted(outcome)) pick(made.surfaceId) }))
      .catch((error: unknown) => store.notify('error', describeError(error)))
  }
  const copyPath = async (e: EntityEnvelope) => {
    const path = entityPath(store, e.entity_id)
    try { await navigator.clipboard.writeText(path); store.notify('info', t('aiworkspace.surfaces.pathCopied', { path })) } catch { store.notify('info', t('aiworkspace.surfaces.pathNotCopied', { path })) }
  }
  const permissions = (e: EntityEnvelope) => {
    onClose()
    if (requestEntityPermissions(e.entity_id)) return
    // until the per-entity view exists (PRD §8.4): the workspace permissions of the data view
    if (!phone) shell.setTopMode('sources')
    store.notify('info', t('aiworkspace.surfaces.permissionsFallback'))
  }

  const menuFor = (e: EntityEnvelope): MenuItem[] => {
    const id = e.entity_id
    const folder = e.kind === 'folder'
    const parent = store.outline.get(e.parent_id ?? '')
    const own = readOnly ?? (canStructure(e) ? null : t('aiworkspace.surfaces.noStructure'))
    const inside = readOnly ?? (canStructure(e) ? null : t('aiworkspace.surfaces.noStructureHere'))
    const beside = readOnly ?? (canStructure(parent) ? null : t('aiworkspace.surfaces.noStructureHere'))
    const items: MenuItem[] = []
    if (folder) {
      items.push({ id: 'new-here', testId: `aiws-surface-new-here-${id}`, label: t(phone ? 'aiworkspace.surfaces.newFlowPageHere' : 'aiworkspace.surfaces.newCanvasHere'), disabled: inside !== null, reason: inside, run: () => newCanvas(id) })
      items.push({ id: 'folder-here', testId: `aiws-surface-new-folder-here-${id}`, label: t('aiworkspace.surfaces.newFolderHere'), disabled: inside !== null, reason: inside, run: () => setNewFolder({ parentId: id, value: '' }) })
      items.push({ id: 'sep-new', separator: true })
    }
    items.push({ id: 'rename', testId: `aiws-surface-rename-${id}`, label: t('aiworkspace.surfaces.rename'), disabled: own !== null, reason: own, run: () => startRename(e) })
    if (!folder) items.push({ id: 'duplicate', testId: `aiws-surface-duplicate-${id}`, label: t('aiworkspace.surfaces.duplicate'), disabled: beside !== null, reason: beside, run: () => duplicate(e) })
    items.push({ id: 'move', testId: `aiws-surface-move-${id}`, label: t('aiworkspace.surfaces.move'), disabled: own !== null, reason: own, run: () => setMoving(id) })
    items.push({ id: 'permissions', testId: `aiws-surface-permissions-${id}`, label: t('aiworkspace.surfaces.permissions'), run: () => permissions(e) })
    items.push({ id: 'copy-path', testId: `aiws-surface-copy-path-${id}`, label: t('aiworkspace.surfaces.copyPath'), run: () => { void copyPath(e) } })
    items.push({ id: 'sep-delete', separator: true })
    if (folder) {
      const notEmpty = entriesOf(store, id).length > 0
      const why = readOnly ?? (!e.capabilities.includes('delete') ? t('aiworkspace.surfaces.noDelete') : notEmpty ? t('aiworkspace.surfaces.notEmpty') : null)
      items.push({ id: 'delete', testId: `aiws-surface-delete-${id}`, label: t('aiworkspace.surfaces.deleteFolder'), danger: true, disabled: why !== null, reason: why, explain: notEmpty,
        run: () => { void deleteSurfaceFolder(store, e).then(fail) } })
    } else {
      const why = readOnly ?? (e.capabilities.includes('delete') ? null : t('aiworkspace.shell.switcher.noDeletePermission'))
      items.push({ id: 'delete', testId: `aiws-surface-delete-${id}`, label: t('aiworkspace.shell.switcher.delete'), danger: true, disabled: why !== null, reason: why, run: () => { onClose(); shell.openDialog({ kind: 'delete-surface', surface: e }) } })
    }
    return items
  }

  // ---- dragging (a mouse; touch moves with “move to…”)
  const dropAt = (event: DragEvent<HTMLElement>, e: EntityEnvelope): DropAt => {
    const box = event.currentTarget.getBoundingClientRect()
    const y = (event.clientY - box.top) / Math.max(1, box.height)
    if (e.kind === 'folder') return y < 0.25 ? 'before' : y > 0.75 ? 'after' : 'into'
    return y < 0.5 ? 'before' : 'after'
  }
  const dropParent = (target: EntityEnvelope, at: DropAt) => (at === 'into' ? target.entity_id : target.parent_id ?? SURFACES_ROOT)
  const canDrop = (dragId: string, target: EntityEnvelope, at: DropAt) => {
    const parentId = dropParent(target, at)
    return dragId !== target.entity_id && canMoveInto(store, dragId, parentId) && canStructure(store.outline.get(parentId))
  }

  // ---- a long press opens the row's menu (a right click and Android's long press come as `contextmenu`)
  const press = useRef<{ timer: number; x: number; y: number; fired: boolean } | null>(null)
  const openMenu = (row: HTMLElement) => {
    const button = row.querySelector<HTMLButtonElement>(':scope > .aiws-anchor > button[aria-haspopup="menu"]')
    if (button && button.getAttribute('aria-expanded') !== 'true') button.click()
  }
  const pressStart = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.pointerType !== 'touch') return
    const row = event.currentTarget
    const state = { timer: 0, x: event.clientX, y: event.clientY, fired: false }
    state.timer = window.setTimeout(() => { state.fired = true; openMenu(row) }, LONG_PRESS_MS)
    press.current = state
  }
  const pressMove = (event: ReactPointerEvent<HTMLElement>) => {
    const state = press.current
    if (state && Math.hypot(event.clientX - state.x, event.clientY - state.y) > 10) { window.clearTimeout(state.timer); press.current = null }
  }
  const pressEnd = () => { if (press.current && !press.current.fired) { window.clearTimeout(press.current.timer); press.current = null } }
  const pressed = () => { const fired = press.current?.fired ?? false; press.current = null; return fired }

  const movingEntity = moving ? store.outline.get(moving) : undefined
  if (movingEntity) return <MoveView entity={movingEntity} phone={phone} onBack={() => setMoving(null)} onMove={(parentId) => { setMoving(null); void move(movingEntity.entity_id, parentId) }} />

  const results = Boolean(query.trim())
  const firstEntry = rows.find((r): r is Extract<Row, { type: 'entry' }> => r.type === 'entry')
  const rootChildren = entriesOf(store, SURFACES_ROOT).length
  return (
    <>
      <div className="aiws-surface-list-head aiws-sp-head">
        <div className="aiws-sp-title">
          <span className="aiws-muted">{t('aiworkspace.surfaces.workspace')}</span>
          <b data-testid="aiws-workspace-title">{store.session.info().title}</b>
        </div>
        {!guest && (
          <button type="button" className="aiws-tool aiws-tool-text aiws-sp-home" data-testid="aiws-switch-workspace" onClick={() => { onClose(); shell.close() }}>
            <House size={16} aria-hidden="true" /><span>{t('aiworkspace.surfaces.home')}</span>
          </button>
        )}
        {phone && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.surfaces.close')} title={t('aiworkspace.surfaces.close')} data-testid="aiws-surface-close" onClick={onClose}><X size={18} /></button>}
      </div>
      {phone && <div className="aiws-muted aiws-sp-note" data-testid="aiws-phone-note">{t('aiworkspace.shell.switcher.phoneNote')}</div>}
      <div className="aiws-sp-searchbar">
        <label className="aiws-search">
          <Search size={14} aria-hidden="true" />
          <input type="search" aria-label={t('aiworkspace.surfaces.search')} placeholder={t('aiworkspace.surfaces.searchPlaceholder')} value={query} autoFocus={!phone} data-testid="aiws-surface-search"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => { if (event.key === 'Enter' && firstEntry && results) { event.preventDefault(); openEntry(firstEntry.entity, true) } }} />
        </label>
        <button type="button" className="aiws-tool aiws-tool-small" aria-pressed={sort === 'name'} data-testid="aiws-surface-sort" aria-label={t(sort === 'name' ? 'aiworkspace.surfaces.sortManual' : 'aiworkspace.surfaces.sortByName')}
          title={t(sort === 'name' ? 'aiworkspace.surfaces.sortManual' : 'aiworkspace.surfaces.sortByName')} onClick={() => store.userState.set(SORT_KEY, sort === 'name' ? null : 'name')}>
          <ArrowDownAZ size={16} />
        </button>
      </div>
      <div ref={listRef} className="aiws-surface-items aiws-sp-list" role="menu" aria-label={t('aiworkspace.surfaces.list')} data-testid="aiws-surface-rows" data-count={rows.length}
        onDragOver={(event) => { if (drag && rootChildren === 0) event.preventDefault() }}>
        <div className="aiws-sp-rows" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index]
            const style = { top: item.start, height: item.size }
            if (row.type === 'heading') return <div key={row.key} className="aiws-sp-heading aiws-muted" style={style} role="presentation">{row.label}</div>
            if (row.type === 'empty') return <div key={row.key} className="aiws-sp-empty aiws-muted" style={style} role="presentation">{row.label}</div>
            const e = row.entity
            const folder = e.kind === 'folder'
            const isActive = !folder && e.entity_id === activeId
            const over = drag?.over?.id === e.entity_id ? drag.over.at : null
            const draggable = !coarse && !phone && !results && !renaming && readOnly === null && canStructure(e)
            return (
              <div key={row.key} className={`aiws-surface-item aiws-sp-row${isActive ? ' is-active' : ''}${folder ? ' is-folder' : ''}${over ? ` is-drop-${over}` : ''}${drag?.id === e.entity_id ? ' is-dragging' : ''}`}
                style={style} data-testid={row.testId} data-entity-id={e.entity_id} data-depth={row.depth}
                draggable={draggable}
                onDragStart={(event) => { event.dataTransfer.setData('application/x-aiws-entry', e.entity_id); event.dataTransfer.effectAllowed = 'move'; setDrag({ id: e.entity_id, over: null }) }}
                onDragOver={(event) => {
                  if (!drag) return
                  const at = dropAt(event, e)
                  if (!canDrop(drag.id, e, at)) { if (drag.over) setDrag({ ...drag, over: null }); return }
                  event.preventDefault()
                  event.dataTransfer.dropEffect = 'move'
                  if (drag.over?.id !== e.entity_id || drag.over.at !== at) setDrag({ ...drag, over: { id: e.entity_id, at } })
                }}
                onDragLeave={(event) => { if (drag?.over?.id === e.entity_id && !event.currentTarget.contains(event.relatedTarget as Node | null)) setDrag({ ...drag, over: null }) }}
                onDrop={(event) => {
                  event.preventDefault()
                  const id = drag?.id
                  setDrag(null)
                  if (!id) return
                  const at = dropAt(event, e)
                  if (!canDrop(id, e, at)) return
                  void move(id, dropParent(e, at), at === 'before' ? { before: e.entity_id } : at === 'after' ? { after: e.entity_id } : null)
                }}
                onDragEnd={() => setDrag(null)}
                onContextMenu={(event) => { event.preventDefault(); openMenu(event.currentTarget) }}
                onPointerDown={pressStart} onPointerMove={pressMove} onPointerUp={pressEnd} onPointerCancel={pressEnd}>
                <span className="aiws-sp-indent" style={{ width: row.depth * 16 }} aria-hidden="true" />
                {folder && !results ? (
                  <button type="button" className="aiws-sp-toggle" aria-expanded={!row.collapsed} aria-label={t(row.collapsed ? 'aiworkspace.surfaces.expand' : 'aiworkspace.surfaces.collapse', { name: entryLabel(e) })}
                    data-testid={`aiws-surface-folder-toggle-${e.entity_id}`} onClick={() => toggleFolder(e.entity_id)}>
                    {row.collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                  </button>
                ) : <span className="aiws-sp-toggle" aria-hidden="true" />}
                {renaming?.id === e.entity_id ? (
                  <input autoFocus className="aiws-sp-rename" aria-label={t('aiworkspace.surfaces.renameInput')} data-testid="aiws-surface-rename-input" value={renaming.value} maxLength={256}
                    onChange={(event) => setRenaming({ ...renaming, value: event.target.value })}
                    onBlur={() => { void rename() }}
                    onKeyDown={(event) => { if (event.key === 'Enter') (event.target as HTMLInputElement).blur(); if (event.key === 'Escape') { event.stopPropagation(); setRenaming(null) } }} />
                ) : (
                  <button type="button" role="menuitem" className="aiws-surface-pick" aria-current={isActive || undefined} aria-expanded={folder && !results ? !row.collapsed : undefined}
                    onClick={() => { if (!pressed()) openEntry(e, results) }}>
                    {folder ? (row.collapsed || results ? <Folder size={16} aria-hidden="true" /> : <FolderOpen size={16} aria-hidden="true" />) : <SurfaceIcon surface={e} size={16} />}
                    <span className="aiws-surface-item-name">
                      <span className="aiws-sp-name">{entryLabel(e)}</span>
                      {row.trail && <small className="aiws-sp-trail aiws-muted" data-testid="aiws-surface-trail">{row.trail}</small>}
                    </span>
                    {folder
                      ? row.childCount === 0 && !results && <span className="aiws-chip">{t('aiworkspace.surfaces.emptyFolder')}</span>
                      : <span className="aiws-chip" data-testid="aiws-surface-mode">{e.layout?.mode === 'free' ? t('aiworkspace.surfaces.free') : t('aiworkspace.surfaces.flow')}</span>}
                  </button>
                )}
                <MenuButton label={t('aiworkspace.surfaces.more', { name: entryLabel(e) })} className="aiws-tool aiws-tool-small aiws-sp-more" testId={`aiws-surface-more-${e.entity_id}`} align="end" floating items={() => menuFor(e)}>
                  <Ellipsis size={14} />
                </MenuButton>
              </div>
            )
          })}
        </div>
      </div>
      <div className="aiws-sp-foot">
        {newFolder ? (
          <form className="aiws-sp-folder-form" onSubmit={(event) => { event.preventDefault(); void createFolder() }}>
            <span className="aiws-muted">{newFolder.parentId === SURFACES_ROOT ? t('aiworkspace.surfaces.newFolder') : t('aiworkspace.surfaces.folderIn', { name: entryLabel(store.outline.get(newFolder.parentId) ?? { entity_id: newFolder.parentId } as EntityEnvelope) })}</span>
            <input autoFocus aria-label={t('aiworkspace.surfaces.folderTitle')} placeholder={t('aiworkspace.surfaces.folderDefault')} data-testid="aiws-surface-folder-input" value={newFolder.value} maxLength={256}
              onChange={(event) => setNewFolder({ ...newFolder, value: event.target.value })}
              onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); setNewFolder(null) } }} />
            <button type="submit" className="is-primary" data-testid="aiws-surface-folder-create">{t('aiworkspace.surfaces.create')}</button>
            <button type="button" onClick={() => setNewFolder(null)}>{t('aiworkspace.surfaces.cancel')}</button>
          </form>
        ) : rootReason === null && (
          <>
            <button type="button" className="aiws-popover-item" data-testid="aiws-surface-new" onClick={() => newCanvas()}>
              <Plus size={16} aria-hidden="true" /> {t(phone ? 'aiworkspace.surfaces.newFlowPage' : 'aiworkspace.surfaces.newCanvas')}
            </button>
            <button type="button" className="aiws-popover-item" data-testid="aiws-surface-new-folder" onClick={() => setNewFolder({ parentId: SURFACES_ROOT, value: '' })}>
              <FolderPlus size={16} aria-hidden="true" /> {t('aiworkspace.surfaces.newFolder')}
            </button>
          </>
        )}
      </div>
    </>
  )
}

/** “Move to…”: the top level and every folder; where it is now, itself, its own folders and where `structure` is
 * missing are not offered. */
function MoveView({ entity, phone, onBack, onMove }: { entity: EntityEnvelope; phone: boolean; onBack: () => void; onMove: (parentId: string) => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  const readOnly = useReadOnlyReason()
  const targets = [{ id: SURFACES_ROOT, label: t('aiworkspace.surfaces.top'), depth: 0 }, ...allFolders(store).map(({ entity: f, depth }) => ({ id: f.entity_id, label: entryLabel(f), depth: depth + 1 }))]
  return (
    <div className="aiws-sp-move" data-testid="aiws-surface-move-view">
      <div className="aiws-sp-head">
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.surfaces.back')} title={t('aiworkspace.surfaces.back')} data-testid="aiws-surface-move-back" onClick={onBack}><ChevronLeft size={18} /></button>
        <b className="aiws-sp-move-title">{t('aiworkspace.surfaces.moveTitle', { name: entryLabel(entity) })}</b>
      </div>
      <div className={`aiws-sp-move-list${phone ? ' is-phone' : ''}`} role="menu" aria-label={t('aiworkspace.surfaces.move')}>
        {targets.map((target) => {
          const here = (entity.parent_id ?? SURFACES_ROOT) === target.id
          const allowed = !here && readOnly === null && canMoveInto(store, entity.entity_id, target.id) && Boolean(store.outline.get(target.id)?.capabilities.includes('structure'))
          return (
            <button key={target.id} type="button" role="menuitem" className="aiws-popover-item aiws-sp-move-target" disabled={!allowed} style={{ paddingLeft: 8 + target.depth * 16 }}
              title={here ? t('aiworkspace.surfaces.moveHere') : !allowed ? readOnly ?? t('aiworkspace.surfaces.noStructureHere') : undefined} data-testid={`aiws-surface-move-to-${target.id}`} onClick={() => onMove(target.id)}>
              <Folder size={16} aria-hidden="true" /> <span>{target.label}</span>{here && <span className="aiws-chip">{t('aiworkspace.surfaces.moveHere')}</span>}
            </button>
          )
        })}
      </div>
    </div>
  )
}
