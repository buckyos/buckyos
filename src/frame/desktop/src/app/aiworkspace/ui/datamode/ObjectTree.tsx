/* The object tree (UI 框架 PRD §5.2): the workspace's entity tree, by default only the data source objects and the
 * folders holding them; "画布自带的数据" folded; then the "宏" folder — a folder like the others, listing the wishes that
 * hold a macro wherever they are — and, always the last top-level row, "工作区权限" (owner 11-14: no group or area of
 * their own). Its head is the data mode's way back to the canvas and to the main menu (`lead`, owner 11-14: no title
 * row above the data mode) and ＋; the filter is at the end of the search row (owner 11-14: the Workspace's name is in
 * the status bar, folding the tree is the tab row's). Search keeps the
 * levels; the filters (types, origin, everything) and what is unfolded are the person's (user state). Each row: the
 * type's icon, the name, its marks (a generated result and its freshness, a write lock, published, an external source,
 * degraded content). A single click opens the object in the preview tab, a double click in a tab of its own. The menu
 * of a row (right click, a long press, its ⋯ — always shown on a touch screen), of the blank area and of the ＋ above the
 * tree is the data actions' (actions.ts). On a desktop rows drag into a folder or next to another row; on a touch screen
 * "移动到…" picks the folder and the place in it.
 *
 * The keyboard (WAI-ARIA tree pattern): the tree is one tab stop (a roving tabindex on the rows); ↑ ↓ move the focus,
 * → unfolds (or goes to the first child), ← folds (or goes to the parent), Home / End to the first / last row, Enter
 * opens the object in a tab of its own, Space in the preview tab, the menu key or Shift+F10 opens the row's menu (focus
 * comes back when it closes), and typing the start of a name moves to the next row so named.
 *
 * A folder the person cannot read, above something it can (the outline's path node), is a row that only folds: no name,
 * no menu, it neither opens nor drags nor takes a drop; Enter and Space fold it. So does "宏" (it is no entity: no menu);
 * a macro in it opens on its code, has the object's menu, and neither drags nor takes a drop (where it is in the tree
 * is not "宏"). A row is known by its key — the entity's ID, `@macros/<ID>` for a macro in "宏" (with "显示全部对象" the
 * same wish is a row where it is too), `@macros`, `@permissions`. */

import { createElement, useCallback, useId, useMemo, useRef, useState, type DragEvent as ReactDragEvent, type FocusEvent as ReactFocusEvent, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, Ellipsis, Filter, Lock, Plus, Search, X } from 'lucide-react'
import { DATA_ID, SHOWS_ID, SURFACES_ID, type EntityEnvelope, type Json, type Publication } from '../../api/types'
import { useLoad, useOutlineVersion, useStore, useUserState, useVersion } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { useActionContext } from '../framework/hooks'
import { useMacroWatch } from '../framework/macros'
import { holdsMacro } from '../wish/macro'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { dataMenu, type DataHost } from './actions'
import { FloatingMenu, SheetMenu, useLongPress, type MenuAt } from './menus'
import { DEFAULT_FILTER, FILTER_KEY, FILTER_TYPES, filtering, iconOf, isFolder, MACROS_NODE, macroShown, nameOf, parseFilter, SESSION_FOLDER, treeView, typeName, type Origin, type TreeFilter } from './model'
import { canMoveInto, move } from './ops'
import { PERMISSIONS_TAB } from './tabs'
import './objectTree.css'

const EXPANDED_KEY = 'datatree:expanded'
const DRAG_MIME = 'application/x-aiws-entity'

/** Folded unless unfolded: the canvas content area, each canvas's folder, presentations, a session's data, canvases. */
function foldedByDefault(entity: EntityEnvelope): boolean {
  return entity.entity_id === 'canvas-content' || entity.entity_id === SHOWS_ID || entity.entity_id === SESSION_FOLDER || entity.entity_id === SURFACES_ID || entity.system === 'surface_content' || entity.kind === 'surface'
}

type MenuState = { target: string | null; at: MenuAt; sheet: boolean; /** Opened from the keyboard on this row: focus goes back to it. */ returnTo?: string }
type DropAt = { id: string; where: 'before' | 'after' | 'into' }

/** A row as the keyboard walks the tree: in the order shown, with its parent row's key (null at the top level). `id`
 * is the entity, or the node (`@macros`, `@permissions`). */
interface FlatRow { key: string; id: string; kind: 'entity' | 'macros' | 'macro' | 'permissions'; parent: string | null; expandable: boolean; open: boolean }

const macroKey = (id: string) => `${MACROS_NODE}/${id}`
const noop = () => undefined

/** A row that unfolds: a folder, a canvas, or anything with children. */
function expandableOf(store: WorkspaceStore, entity: EntityEnvelope): boolean {
  return isFolder(entity) || entity.kind === 'surface' || entity.entity_id === SURFACES_ID || store.outline.treeChildrenOf(entity.entity_id).length > 0
}

const TYPEAHEAD_MS = 600

export function ObjectTree({ activeId, host, phone, coarse, lead, tail }: {
  activeId: string | null; host: DataHost; phone: boolean; coarse: boolean
  /** At the start of the head: "返回画布" and the main menu. */
  lead?: ReactNode
  /** Before ＋ in the head (a phone's count of open objects). */
  tail?: ReactNode
}) {
  const { t } = useAiwsText()
  const store = useStore()
  const outlineVersion = useOutlineVersion()
  const actions = useActionContext(null)
  const [query, setQuery] = useState('')
  const [filterOpen, setFilterOpen] = useState(false)
  const filterState = useUserState<Json>(FILTER_KEY)
  const filter = useMemo(() => parseFilter(filterState), [filterState])
  const setFilter = (next: TreeFilter) => store.userState.set(FILTER_KEY, filtering(next) ? next as unknown as Json : null)
  const expandedState = useUserState<Record<string, boolean>>(EXPANDED_KEY)
  const expanded = useMemo(() => expandedState ?? {}, [expandedState])
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the outline version invalidates the view
  const view = useMemo(() => treeView(store, filter, query), [store, filter, query, outlineVersion])
  const any = useVersion('any')
  const loadPublished = useCallback(async () => new Set((await store.session.listPublications().catch(() => [] as Publication[])).filter((p) => p.current !== false).map((p) => p.entity_id)), [store])
  const published = useLoad(loadPublished, any).data ?? new Set<string>()
  const macros = useMacros()
  const [menu, setMenu] = useState<MenuState | null>(null)
  const [drop, setDrop] = useState<DropAt | null>(null)
  const sheet = phone
  const openMenu = (targetId: string | null, at: MenuAt, returnTo?: string) => setMenu({ target: targetId, at, sheet, ...(returnTo ? { returnTo } : {}) })
  // the search and a type filter open every level: folding then means nothing
  const forcedOpen = view.searching || (filtering(filter) && filter.types.length > 0)
  const isOpen = (e: EntityEnvelope) => forcedOpen ? true : expanded[e.entity_id] ?? !foldedByDefault(e)
  const toggle = (e: EntityEnvelope) => store.userState.set(EXPANDED_KEY, { ...expanded, [e.entity_id]: !isOpen(e) })
  // "宏": open unless folded; shown unless a search or a filter leaves nothing in it
  const listed = macros.filter((m) => macroShown(store, m, filter, query))
  const macrosShown = !(view.searching || filter.types.length > 0 || filter.origin !== 'all') || listed.length > 0
  const macrosOpen = forcedOpen || (expanded[MACROS_NODE] ?? true)
  const toggleMacros = () => store.userState.set(EXPANDED_KEY, { ...expanded, [MACROS_NODE]: !macrosOpen })
  const treeRef = useRef<HTMLUListElement>(null)
  const [focusId, setFocusId] = useState<string | null>(null)
  const typed = useRef<{ text: string; at: number }>({ text: '', at: 0 })

  const onDrop = (event: ReactDragEvent, at: DropAt) => {
    event.preventDefault()
    setDrop(null)
    const id = event.dataTransfer.getData(DRAG_MIME)
    const over = store.outline.get(at.id)
    if (!id || !over || id === at.id) return
    if (at.where === 'into') { if (canMoveInto(store, id, at.id)) void move(store, id, at.id); return }
    const parentId = over.parent_id ?? DATA_ID
    if (canMoveInto(store, id, parentId)) void move(store, id, parentId, at.where === 'before' ? { before: at.id } : { after: at.id })
  }

  const rows = (parentId: string, depth: number): ReactNode[] => store.outline.treeChildrenOf(parentId).filter((c) => view.shown.has(c.entity_id)).map((entity) => (
    <TreeRow key={entity.entity_id} rowKey={entity.entity_id} entity={entity} depth={depth} activeId={activeId} open={isOpen(entity)} hit={view.hits.has(entity.entity_id)} coarse={coarse} tabStop={tabStop === entity.entity_id}
      published={published.has(entity.entity_id)} drop={drop?.id === entity.entity_id ? drop.where : null}
      onToggle={() => toggle(entity)} onOpen={(preview) => host.open(entity.entity_id, { preview })} onMenu={(at) => openMenu(entity.entity_id, at)}
      onDragOver={(where) => setDrop({ id: entity.entity_id, where })} onDragLeave={() => setDrop((d) => (d?.id === entity.entity_id ? null : d))} onDrop={(event, where) => onDrop(event, { id: entity.entity_id, where })}>
      {isOpen(entity) && rows(entity.entity_id, depth + 1)}
    </TreeRow>
  ))
  // the rows in the order shown, as the keyboard walks them
  const flat: FlatRow[] = []
  const walk = (parentId: string, parent: string | null) => {
    for (const entity of store.outline.treeChildrenOf(parentId)) {
      if (!view.shown.has(entity.entity_id)) continue
      const open = isOpen(entity)
      flat.push({ key: entity.entity_id, id: entity.entity_id, kind: 'entity', parent, expandable: expandableOf(store, entity), open })
      if (open) walk(entity.entity_id, entity.entity_id)
    }
  }
  walk(DATA_ID, null)
  if (view.shown.has(SURFACES_ID)) walk('root', null)
  if (macrosShown) {
    flat.push({ key: MACROS_NODE, id: MACROS_NODE, kind: 'macros', parent: null, expandable: true, open: macrosOpen })
    if (macrosOpen) for (const m of listed) flat.push({ key: macroKey(m.entity_id), id: m.entity_id, kind: 'macro', parent: MACROS_NODE, expandable: false, open: false })
  }
  flat.push({ key: PERMISSIONS_TAB, id: PERMISSIONS_TAB, kind: 'permissions', parent: null, expandable: false, open: false })
  // the tree's one tab stop: the row last focused, else the open object's, else the first
  const tabStop = flat.some((r) => r.key === focusId) ? focusId : flat.find((r) => r.id === activeId)?.key ?? flat[0].key
  const rowElement = (key: string) => [...(treeRef.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? [])].find((el) => el.dataset.rowKey === key) ?? null
  const focusRow = (key: string) => { setFocusId(key); rowElement(key)?.focus() }
  const rowMenuAt = (key: string): MenuAt => {
    const r = rowElement(key)?.querySelector('.aiws-tree-row')?.getBoundingClientRect()
    return r ? { x: r.left + 24, y: r.bottom + 2 } : { x: 0, y: 0 }
  }
  /** "宏" and a path node only fold: no menu, they do not open. */
  const foldsOnly = (row: FlatRow) => row.kind === 'macros' || (row.kind === 'entity' && store.outline.node(row.id)?.path_only === true)
  const closeMenu = () => {
    const back = menu?.returnTo
    setMenu(null)
    // the menu took the focus: back to the row, unless a dialog it opened has it now
    if (back) window.setTimeout(() => { if (!document.activeElement || document.activeElement === document.body) focusRow(back) }, 0)
  }
  const onTreeFocus = (event: ReactFocusEvent<HTMLUListElement>) => {
    const key = (event.target as HTMLElement).closest<HTMLElement>('[role="treeitem"]')?.dataset.rowKey
    if (key && key !== focusId) setFocusId(key)
  }
  const onTreeKey = (event: ReactKeyboardEvent<HTMLUListElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return
    const rowKey = (event.target as HTMLElement).closest<HTMLElement>('[role="treeitem"]')?.dataset.rowKey
    const index = rowKey ? flat.findIndex((r) => r.key === rowKey) : -1
    if (!rowKey || index < 0) return
    const row = flat[index]
    const entity = row.kind === 'entity' ? store.outline.node(row.id) : undefined
    const foldOnly = foldsOnly(row)
    const toggleRow = () => { if (row.kind === 'macros') toggleMacros(); else if (entity) toggle(entity) }
    const openRow = (preview: boolean) => host.open(row.id, row.kind === 'macro' ? { preview, page: 'code' } : { preview })
    const key = event.key
    const done = () => { event.preventDefault(); event.stopPropagation() }
    if (key === 'ContextMenu' || (key === 'F10' && event.shiftKey)) { done(); if (!foldOnly) openMenu(row.id, rowMenuAt(rowKey), rowKey); return }
    if (key.length === 1 && key !== ' ') {
      // type-ahead: the next row whose name starts with what was typed
      const now = event.timeStamp
      const text = (now - typed.current.at < TYPEAHEAD_MS ? typed.current.text : '') + key.toLowerCase()
      typed.current = { text, at: now }
      const from = text.length > 1 ? index : index + 1
      for (let i = 0; i < flat.length; i++) {
        const candidate = flat[(from + i) % flat.length]
        if (nameOf(store, store.outline.node(candidate.id), candidate.id).toLowerCase().startsWith(text)) { focusRow(candidate.key); break }
      }
      done()
      return
    }
    if (event.shiftKey) return
    switch (key) {
      case 'ArrowDown': if (flat[index + 1]) focusRow(flat[index + 1].key); break
      case 'ArrowUp': if (index > 0) focusRow(flat[index - 1].key); break
      case 'Home': focusRow(flat[0].key); break
      case 'End': focusRow(flat[flat.length - 1].key); break
      case 'ArrowRight':
        if (row.expandable && !row.open) toggleRow()
        else if (row.open && flat[index + 1]?.parent === rowKey) focusRow(flat[index + 1].key)
        break
      case 'ArrowLeft':
        if (row.expandable && row.open && !forcedOpen) toggleRow()
        else if (row.parent) focusRow(row.parent)
        break
      // a path node and "宏" do not open: they fold
      case 'Enter': if (foldOnly) { if (row.expandable && !forcedOpen) toggleRow() } else openRow(false); break
      case ' ': if (foldOnly) { if (row.expandable && !forcedOpen) toggleRow() } else openRow(true); break
      default: return
    }
    done()
  }
  // with "everything", the canvases follow the data (the root's other child; `data` itself is never a row); then "宏",
  // then "工作区权限", always
  const dataRows = [...rows(DATA_ID, 0), ...(view.shown.has(SURFACES_ID) ? rows('root', 0) : [])]
  const narrowedOrFiltered = view.searching || filtering(filter)
  const empty = dataRows.length === 0 && (!narrowedOrFiltered || listed.length === 0)
  const top = [
    ...dataRows,
    macrosShown && (
      <NodeRow key={MACROS_NODE} id={MACROS_NODE} testId="aiws-dm-macros" labelTestId="aiws-dm-macros-label" count={listed.length} active={false} tabStop={tabStop === MACROS_NODE} coarse={coarse}
        hint={macros.length === 0 ? t('aiworkspace.datamode.noMacros') : undefined} open={macrosOpen} onToggle={toggleMacros}>
        {macrosOpen && listed.map((m) => (
          <TreeRow key={macroKey(m.entity_id)} rowKey={macroKey(m.entity_id)} macro entity={m} depth={1} activeId={activeId} open={false} hit={view.searching} coarse={coarse} tabStop={tabStop === macroKey(m.entity_id)}
            published={published.has(m.entity_id)} drop={null} onToggle={noop} onOpen={(preview) => host.open(m.entity_id, { preview, page: 'code' })} onMenu={(at) => openMenu(m.entity_id, at)}
            onDragOver={noop} onDragLeave={noop} onDrop={noop}>{null}</TreeRow>
        ))}
      </NodeRow>
    ),
    <NodeRow key={PERMISSIONS_TAB} id={PERMISSIONS_TAB} testId="aiws-dm-perm-item" labelTestId="aiws-dm-perm-node" active={activeId === PERMISSIONS_TAB} tabStop={tabStop === PERMISSIONS_TAB} coarse={coarse}
      onOpen={(preview) => host.open(PERMISSIONS_TAB, { preview })} onMenu={(at) => openMenu(PERMISSIONS_TAB, at)} />,
  ]
  const typeLabel = (id: string) => typeName({ type_id: id } as EntityEnvelope)
  const menuItems = menu ? dataMenu({ ...actions, target: menu.target, host }) : []
  return (
    <div className="aiws-dm-tree" data-testid="aiws-datatree" data-filtering={filtering(filter) ? 'true' : 'false'}>
      <div className="aiws-dm-tree-head">
        {lead}
        <span className="aiws-grow" />
        {tail}
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.add')} title={t('aiworkspace.datamode.addHint')} data-testid="aiws-dm-tree-add"
          onClick={(event) => { const r = event.currentTarget.getBoundingClientRect(); openMenu(activeId && activeId !== PERMISSIONS_TAB ? activeId : null, { x: r.left, y: r.bottom + 4 }) }}><Plus size={18} /></button>
      </div>
      <div className="aiws-dm-search-row">
        <label className="aiws-dm-search">
          <Search size={16} aria-hidden="true" />
          <input type="search" aria-label={t('aiworkspace.data.tree.search')} placeholder={t('aiworkspace.datamode.searchPlaceholder')} data-testid="aiws-dm-search" value={query} onChange={(event) => setQuery(event.target.value)} />
          {query && <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.datamode.clearSearch')} onClick={() => setQuery('')}><X size={14} /></button>}
        </label>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.filter')} title={t('aiworkspace.datamode.filter')} aria-pressed={filtering(filter)} aria-expanded={filterOpen} data-testid="aiws-dm-filter" onClick={() => setFilterOpen((v) => !v)}><Filter size={18} /></button>
      </div>
      {filterOpen && (
        <div className="aiws-dm-filter" data-testid="aiws-dm-filter-panel">
          <div className="aiws-muted">{t('aiworkspace.data.tree.typeFilter')}</div>
          <div className="aiws-dm-filter-types">
            {FILTER_TYPES.map((id) => (
              <label key={id} className="aiws-dm-check">
                <input type="checkbox" data-testid={`aiws-dm-filter-type-${id.replace('buckyos.', '')}`} checked={filter.types.includes(id)}
                  onChange={(event) => setFilter({ ...filter, types: event.target.checked ? [...filter.types, id] : filter.types.filter((x) => x !== id) })} />
                {createElement(iconOf({ type_id: id } as EntityEnvelope), { size: 14, 'aria-hidden': true })}{typeLabel(id)}
              </label>
            ))}
          </div>
          <label className="aiws-dm-filter-line">{t('aiworkspace.data.tree.originFilter')}
            <select data-testid="aiws-dm-filter-origin" value={filter.origin} onChange={(event) => setFilter({ ...filter, origin: event.target.value as Origin })}>
              <option value="all">{t('aiworkspace.data.tree.originAll')}</option><option value="original">{t('aiworkspace.data.tree.originOriginal')}</option><option value="generated">{t('aiworkspace.data.tree.originGenerated')}</option>
            </select>
          </label>
          <label className="aiws-dm-check"><input type="checkbox" data-testid="aiws-dm-filter-all" checked={filter.all} onChange={(event) => setFilter({ ...filter, all: event.target.checked })} />{t('aiworkspace.datamode.showAll')}</label>
          {filtering(filter) && <button type="button" className="aiws-link" data-testid="aiws-dm-filter-reset" onClick={() => setFilter(DEFAULT_FILTER)}>{t('aiworkspace.datamode.resetFilter')}</button>}
        </div>
      )}
      <div className="aiws-dm-tree-scroll" onContextMenu={(event) => { if (event.target === event.currentTarget || (event.target as HTMLElement).classList.contains('aiws-tree')) { event.preventDefault(); openMenu(null, { x: event.clientX, y: event.clientY }) } }}>
        {empty && <div className="aiws-muted aiws-dm-tree-empty" data-testid="aiws-dm-tree-empty">{narrowedOrFiltered ? t('aiworkspace.datamode.noMatch') : t('aiworkspace.datamode.emptyTree')}</div>}
        <ul ref={treeRef} role="tree" className="aiws-tree" aria-label={t('aiworkspace.datamode.tree')} onKeyDown={onTreeKey} onFocus={onTreeFocus}
          onContextMenu={(event) => {
            // the menu key on a focused row (its own handler took the keydown): no browser menu over it
            const target = event.target as HTMLElement
            if (target.getAttribute('role') !== 'treeitem') return
            event.preventDefault()
            event.stopPropagation()
            const row = flat.find((r) => r.key === target.dataset.rowKey)
            if (row && !foldsOnly(row)) openMenu(row.id, rowMenuAt(row.key), row.key)
          }}>{top}</ul>
      </div>
      {menu && (menu.sheet
        ? <SheetMenu items={menuItems} label={menu.target ? nameOf(store, store.outline.get(menu.target), menu.target) : t('aiworkspace.datamode.tree')} testId="aiws-dm-menu" onClose={closeMenu} />
        : <FloatingMenu at={menu.at} items={menuItems} label={t('aiworkspace.datamode.menu')} testId="aiws-dm-menu" onClose={closeMenu} />)}
    </div>
  )
}

/** The wishes holding a macro, the "宏" folder (§5.2, Q27): what `holdsMacro` says from their freshness (kept fetched
 * by `useMacroWatch`) — the collaboration zone's "宏" asks the same, and also whether it can run now. */
function useMacros(): EntityEnvelope[] {
  const store = useStore()
  useMacroWatch()
  return store.outline.all().filter((e) => holdsMacro(e, store.freshness.get(e.entity_id)))
}

function Marks({ entity, published }: { entity: EntityEnvelope; published: boolean }) {
  const { t } = useAiwsText()
  return (
    <>
      {entity.derived && <span className="aiws-chip aiws-chip-derived" title={t('aiworkspace.data.tree.generatedTitle')} data-testid="aiws-dm-mark-generated">{t('aiworkspace.data.tree.generated')}</span>}
      {entity.derived && <FreshnessBadge entityId={entity.entity_id} showManual={false} />}
      {entity.write_policy === 'lock_required' && <span className="aiws-dm-mark" title={t('aiworkspace.data.tree.lockTitle')} data-testid="aiws-dm-mark-lock"><Lock size={13} aria-label={t('aiworkspace.data.tree.lockTitle')} /></span>}
      {published && <span className="aiws-chip aiws-chip-ok" title={t('aiworkspace.datamode.publishedTitle')} data-testid="aiws-dm-mark-published">{t('aiworkspace.datamode.published')}</span>}
      {entity.source && <span className="aiws-chip" title={entity.source.url ?? entity.source.workspace_id ?? ''} data-testid="aiws-dm-mark-source">{t('aiworkspace.datamode.external')}</span>}
      {entity.degraded && <span className="aiws-chip aiws-chip-warn" title={entity.degraded} data-testid="aiws-dm-mark-degraded">{t('aiworkspace.data.tree.degraded')}</span>}
    </>
  )
}

interface RowProps {
  /** The row's key (the entity's ID; `@macros/<ID>` in "宏"). */
  rowKey: string
  /** A macro in "宏": opens on its code, neither drags nor takes a drop. */
  macro?: boolean
  entity: EntityEnvelope
  depth: number
  activeId: string | null
  open: boolean
  hit: boolean
  coarse: boolean
  /** The tree's one tab stop (roving tabindex). */
  tabStop: boolean
  published: boolean
  drop: DropAt['where'] | null
  onToggle: () => void
  onOpen: (preview: boolean) => void
  onMenu: (at: MenuAt) => void
  onDragOver: (where: DropAt['where']) => void
  onDragLeave: () => void
  onDrop: (event: ReactDragEvent, where: DropAt['where']) => void
  children: ReactNode
}

function TreeRow({ rowKey, macro = false, entity, depth, activeId, open, hit, coarse, tabStop, published, drop, onToggle, onOpen, onMenu, onDragOver, onDragLeave, onDrop, children }: RowProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const labelId = useId()
  const folder = isFolder(entity) || entity.kind === 'surface' || entity.entity_id === SURFACES_ID
  const hasChildren = Array.isArray(children) ? children.length > 0 : Boolean(children)
  const expandable = !macro && expandableOf(store, entity)
  const pathOnly = entity.path_only === true
  // a path node takes no drop; nor does a macro in "宏" (where it is in the tree is elsewhere)
  const dropless = pathOnly || macro
  const press = useLongPress(pathOnly ? () => undefined : onMenu)
  const draggable = !dropless && !coarse && entity.capabilities.includes('structure') && entity.parent_id !== undefined && !entity.system && entity.type_id !== 'buckyos.cell' && entity.kind !== 'surface' && entity.entity_id !== SURFACES_ID
  const name = nameOf(store, entity, entity.entity_id)
  const where = (event: ReactDragEvent<HTMLDivElement>): DropAt['where'] => {
    const r = event.currentTarget.getBoundingClientRect()
    const y = (event.clientY - r.top) / Math.max(1, r.height)
    return folder && y > 0.25 && y < 0.75 ? 'into' : y < 0.5 ? 'before' : 'after'
  }
  const menuAt = (event: ReactMouseEvent<HTMLElement>): MenuAt => { const r = event.currentTarget.getBoundingClientRect(); return { x: r.left, y: r.bottom + 2 } }
  return (
    <li role="treeitem" aria-expanded={expandable ? open : undefined} aria-selected={activeId === entity.entity_id} aria-level={depth + 1} aria-labelledby={labelId} tabIndex={tabStop ? 0 : -1}
      data-testid={macro ? 'aiws-dm-macro' : 'aiws-tree-item'} data-entity-id={entity.entity_id} data-row-key={rowKey} data-type={entity.type_id} data-system={entity.system ?? undefined} data-hit={hit ? 'true' : undefined} data-path-only={pathOnly ? 'true' : undefined}>
      <div className={`aiws-tree-row aiws-dm-row${activeId === entity.entity_id ? ' is-selected' : ''}${drop ? ` is-drop-${drop}` : ''}${pathOnly ? ' is-path-only' : ''}`} style={{ paddingLeft: 4 + depth * 14 }}
        draggable={draggable}
        onDragStart={(event) => { event.dataTransfer.setData(DRAG_MIME, entity.entity_id); event.dataTransfer.effectAllowed = 'move' }}
        onDragOver={(event) => { if (dropless || !event.dataTransfer.types.includes(DRAG_MIME)) return; event.preventDefault(); event.dataTransfer.dropEffect = 'move'; onDragOver(where(event)) }}
        onDragLeave={onDragLeave}
        onDrop={(event) => { if (!dropless) onDrop(event, where(event)) }}
        onContextMenu={(event) => { event.preventDefault(); if (!pathOnly) onMenu({ x: event.clientX, y: event.clientY }) }}
        onPointerDown={press.onPointerDown} onPointerMove={press.onPointerMove} onPointerUp={press.onPointerUp} onPointerCancel={press.onPointerCancel}>
        {expandable ? (
          <button type="button" className="aiws-tree-toggle" tabIndex={-1} aria-label={open ? t('aiworkspace.data.tree.collapse') : t('aiworkspace.data.tree.expand')} onClick={onToggle}>
            {open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}
          </button>
        ) : <span className="aiws-tree-toggle" />}
        {pathOnly ? (
          <button type="button" id={labelId} className="aiws-tree-label" tabIndex={-1} data-testid={`aiws-tree-${entity.entity_id}`} title={t('aiworkspace.pathnode.hint')} onClick={() => { if (expandable) onToggle() }}>
            <span className="aiws-dm-type">{createElement(iconOf(entity), { size: 16, 'aria-hidden': true })}</span>
            <span className="aiws-outline-name">{name}</span>
          </button>
        ) : (
          <button type="button" id={labelId} className="aiws-tree-label" tabIndex={-1} data-testid={macro ? `aiws-dm-macro-${entity.entity_id}` : `aiws-tree-${entity.entity_id}`} title={`${typeName(entity)} · ${name}`}
            onClick={() => { if (!press.consumed()) onOpen(true) }} onDoubleClick={() => onOpen(false)}>
            <span className="aiws-dm-type" title={typeName(entity)}>{createElement(iconOf(entity), { size: 16, 'aria-hidden': true })}</span>
            <span className="aiws-outline-name">{name}</span>
            <Marks entity={entity} published={published} />
          </button>
        )}
        {!pathOnly && (
          <button type="button" className={`aiws-dm-row-more${coarse ? ' is-shown' : ''}`} tabIndex={-1} aria-label={t('aiworkspace.datamode.more', { name })} title={t('aiworkspace.datamode.moreTitle')}
            data-testid={macro ? `aiws-dm-macro-more-${entity.entity_id}` : `aiws-tree-more-${entity.entity_id}`} onClick={(event) => onMenu(menuAt(event))}><Ellipsis size={16} aria-hidden="true" /></button>
        )}
      </div>
      {open && hasChildren && <ul role="group">{children}</ul>}
    </li>
  )
}

/** A top-level row that is no entity: "宏" (a folder that only folds, no menu) or "工作区权限" (opens the permission
 * editor, the menu "打开 / 在新标签页打开"). Looks and walks like any other row. */
function NodeRow({ id, testId, labelTestId, count, hint, active, tabStop, coarse, open = false, onToggle, onOpen, onMenu, children }: {
  id: string; testId: string; labelTestId: string; count?: number; hint?: string; active: boolean; tabStop: boolean; coarse: boolean
  open?: boolean; onToggle?: () => void; onOpen?: (preview: boolean) => void; onMenu?: (at: MenuAt) => void; children?: ReactNode
}) {
  const { t } = useAiwsText()
  const store = useStore()
  const labelId = useId()
  const press = useLongPress(onMenu ?? noop)
  const name = nameOf(store, undefined, id)
  const folder = onToggle !== undefined
  const hasChildren = Array.isArray(children) ? children.length > 0 : Boolean(children)
  return (
    <li role="treeitem" aria-expanded={folder ? open : undefined} aria-selected={active} aria-level={1} aria-labelledby={labelId} tabIndex={tabStop ? 0 : -1}
      data-testid={testId} data-entity-id={id} data-row-key={id} data-count={count}>
      <div className={`aiws-tree-row aiws-dm-row${active ? ' is-selected' : ''}`} style={{ paddingLeft: 4 }}
        onContextMenu={(event) => { event.preventDefault(); onMenu?.({ x: event.clientX, y: event.clientY }) }}
        onPointerDown={press.onPointerDown} onPointerMove={press.onPointerMove} onPointerUp={press.onPointerUp} onPointerCancel={press.onPointerCancel}>
        {folder ? (
          <button type="button" className="aiws-tree-toggle" tabIndex={-1} aria-label={open ? t('aiworkspace.data.tree.collapse') : t('aiworkspace.data.tree.expand')} data-testid={`${labelTestId}-toggle`} onClick={onToggle}>
            {open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}
          </button>
        ) : <span className="aiws-tree-toggle" />}
        <button type="button" id={labelId} className="aiws-tree-label" tabIndex={-1} data-testid={labelTestId} title={hint ?? name}
          onClick={() => { if (press.consumed()) return; if (onOpen) onOpen(true); else onToggle?.() }} onDoubleClick={() => onOpen?.(false)}>
          <span className="aiws-dm-type">{createElement(iconOf(undefined, id), { size: 16, 'aria-hidden': true })}</span>
          <span className="aiws-outline-name">{name}</span>
        </button>
        {onMenu && (
          <button type="button" className={`aiws-dm-row-more${coarse ? ' is-shown' : ''}`} tabIndex={-1} aria-label={t('aiworkspace.datamode.more', { name })} title={t('aiworkspace.datamode.moreTitle')} data-testid={`${labelTestId}-more`}
            onClick={(event) => { const r = event.currentTarget.getBoundingClientRect(); onMenu({ x: r.left, y: r.bottom + 2 }) }}><Ellipsis size={16} aria-hidden="true" /></button>
        )}
      </div>
      {folder && open && hasChildren && <ul role="group">{children}</ul>}
    </li>
  )
}
