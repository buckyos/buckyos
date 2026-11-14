/* The object tree (UI 框架 PRD §5.2): the workspace's entity tree, by default only the data source objects and the
 * folders holding them; "画布自带的数据" folded; the "宏" group and the "工作区权限" node pinned below. Search keeps the
 * levels; the filters (types, origin, everything) and what is unfolded are the person's (user state). Each row: the
 * type's icon, the name, its marks (a generated result and its freshness, a write lock, published, an external source,
 * degraded content). A single click opens the object in the preview tab, a double click in a tab of its own. The menu
 * of a row (right click, a long press, its ⋯ — always shown on a touch screen), of the blank area and of the ＋ above the
 * tree is the data actions' (actions.ts). On a desktop rows drag into a folder or next to another row. */

import { createElement, useCallback, useMemo, useState, type DragEvent as ReactDragEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, Ellipsis, Filter, Lock, Plus, Search, X } from 'lucide-react'
import { DATA_ID, SHOWS_ID, SURFACES_ID, type EntityEnvelope, type Json, type Publication } from '../../api/types'
import { useLoad, useOutlineVersion, useStore, useUserState, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useActionContext } from '../framework/hooks'
import { useMacroWatch } from '../framework/macros'
import { holdsMacro } from '../wish/macro'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { dataMenu, type DataHost } from './actions'
import { FloatingMenu, SheetMenu, useLongPress, type MenuAt } from './menus'
import { DEFAULT_FILTER, FILTER_KEY, FILTER_TYPES, filtering, iconOf, isFolder, nameOf, parseFilter, SESSION_FOLDER, treeView, typeName, type Origin, type TreeFilter } from './model'
import { canMoveInto, move } from './ops'
import { PERMISSIONS_TAB } from './tabs'

const EXPANDED_KEY = 'datatree:expanded'
const DRAG_MIME = 'application/x-aiws-entity'

/** Folded unless unfolded: the canvas content area, each canvas's folder, presentations, a session's data, canvases. */
function foldedByDefault(entity: EntityEnvelope): boolean {
  return entity.entity_id === 'canvas-content' || entity.entity_id === SHOWS_ID || entity.entity_id === SESSION_FOLDER || entity.entity_id === SURFACES_ID || entity.system === 'surface_content' || entity.kind === 'surface'
}

type MenuState = { target: string | null; at: MenuAt; sheet: boolean }
type DropAt = { id: string; where: 'before' | 'after' | 'into' }

export function ObjectTree({ activeId, host, phone, coarse }: { activeId: string | null; host: DataHost; phone: boolean; coarse: boolean }) {
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
  const openMenu = (targetId: string | null, at: MenuAt) => setMenu({ target: targetId, at, sheet })
  const isOpen = (e: EntityEnvelope) => view.searching || (filtering(filter) && filter.types.length > 0) ? true : expanded[e.entity_id] ?? !foldedByDefault(e)
  const toggle = (e: EntityEnvelope) => store.userState.set(EXPANDED_KEY, { ...expanded, [e.entity_id]: !isOpen(e) })

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

  const rows = (parentId: string, depth: number): ReactNode[] => store.outline.childrenOf(parentId).filter((c) => view.shown.has(c.entity_id)).map((entity) => (
    <TreeRow key={entity.entity_id} entity={entity} depth={depth} activeId={activeId} open={isOpen(entity)} hit={view.hits.has(entity.entity_id)} coarse={coarse}
      published={published.has(entity.entity_id)} drop={drop?.id === entity.entity_id ? drop.where : null}
      onToggle={() => toggle(entity)} onOpen={(preview) => host.open(entity.entity_id, { preview })} onMenu={(at) => openMenu(entity.entity_id, at)}
      onDragOver={(where) => setDrop({ id: entity.entity_id, where })} onDragLeave={() => setDrop((d) => (d?.id === entity.entity_id ? null : d))} onDrop={(event, where) => onDrop(event, { id: entity.entity_id, where })}>
      {isOpen(entity) && rows(entity.entity_id, depth + 1)}
    </TreeRow>
  ))
  // with "everything", the canvases follow the data (the root's other child; `data` itself is never a row)
  const top = [...rows(DATA_ID, 0), ...(view.shown.has(SURFACES_ID) ? rows('root', 0) : [])]
  const typeLabel = (id: string) => typeName({ type_id: id } as EntityEnvelope)
  const menuItems = menu ? dataMenu({ ...actions, target: menu.target, host }) : []
  return (
    <div className="aiws-dm-tree" data-testid="aiws-datatree" data-filtering={filtering(filter) ? 'true' : 'false'}>
      <div className="aiws-dm-tree-head">
        <b>{t('aiworkspace.datamode.tree')}</b>
        <span className="aiws-grow" />
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.add')} title={t('aiworkspace.datamode.addHint')} data-testid="aiws-dm-tree-add"
          onClick={(event) => { const r = event.currentTarget.getBoundingClientRect(); openMenu(activeId && activeId !== PERMISSIONS_TAB ? activeId : null, { x: r.left, y: r.bottom + 4 }) }}><Plus size={18} /></button>
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.datamode.filter')} title={t('aiworkspace.datamode.filter')} aria-pressed={filtering(filter)} aria-expanded={filterOpen} data-testid="aiws-dm-filter" onClick={() => setFilterOpen((v) => !v)}><Filter size={18} /></button>
      </div>
      <label className="aiws-dm-search">
        <Search size={16} aria-hidden="true" />
        <input type="search" aria-label={t('aiworkspace.data.tree.search')} placeholder={t('aiworkspace.datamode.searchPlaceholder')} data-testid="aiws-dm-search" value={query} onChange={(event) => setQuery(event.target.value)} />
        {query && <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.datamode.clearSearch')} onClick={() => setQuery('')}><X size={14} /></button>}
      </label>
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
        <ul role="tree" className="aiws-tree" aria-label={t('aiworkspace.datamode.tree')}>{top}</ul>
        {top.length === 0 && <div className="aiws-muted aiws-dm-tree-empty" data-testid="aiws-dm-tree-empty">{view.searching || filtering(filter) ? t('aiworkspace.datamode.noMatch') : t('aiworkspace.datamode.emptyTree')}</div>}
      </div>
      <div className="aiws-dm-tree-pinned">
        <MacroGroup macros={macros.filter((m) => !query || nameOf(store, m, m.entity_id).toLowerCase().includes(query.trim().toLowerCase()))} activeId={activeId} coarse={coarse}
          onOpen={(id, preview) => host.open(id, { preview, page: 'code' })} onMenu={(id, at) => openMenu(id, at)} />
        <SpecialRow id={PERMISSIONS_TAB} testId="aiws-dm-perm-node" active={activeId === PERMISSIONS_TAB} coarse={coarse}
          onOpen={(preview) => host.open(PERMISSIONS_TAB, { preview })} onMenu={(at) => openMenu(PERMISSIONS_TAB, at)} />
      </div>
      {menu && (menu.sheet
        ? <SheetMenu items={menuItems} label={menu.target ? nameOf(store, store.outline.get(menu.target), menu.target) : t('aiworkspace.datamode.tree')} testId="aiws-dm-menu" onClose={() => setMenu(null)} />
        : <FloatingMenu at={menu.at} items={menuItems} label={t('aiworkspace.datamode.menu')} testId="aiws-dm-menu" onClose={() => setMenu(null)} />)}
    </div>
  )
}

/** The wishes holding a macro, the "宏" group (§5.2, Q27): what `holdsMacro` says from their freshness (kept fetched
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
  entity: EntityEnvelope
  depth: number
  activeId: string | null
  open: boolean
  hit: boolean
  coarse: boolean
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

function TreeRow({ entity, depth, activeId, open, hit, coarse, published, drop, onToggle, onOpen, onMenu, onDragOver, onDragLeave, onDrop, children }: RowProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const folder = isFolder(entity) || entity.kind === 'surface' || entity.entity_id === SURFACES_ID
  const hasChildren = Array.isArray(children) ? children.length > 0 : Boolean(children)
  const expandable = folder || store.outline.childrenOf(entity.entity_id).length > 0
  const press = useLongPress(onMenu)
  const draggable = !coarse && entity.capabilities.includes('structure') && entity.parent_id !== undefined && !entity.system && entity.type_id !== 'buckyos.cell' && entity.kind !== 'surface' && entity.entity_id !== SURFACES_ID
  const name = nameOf(store, entity, entity.entity_id)
  const where = (event: ReactDragEvent<HTMLDivElement>): DropAt['where'] => {
    const r = event.currentTarget.getBoundingClientRect()
    const y = (event.clientY - r.top) / Math.max(1, r.height)
    return folder && y > 0.25 && y < 0.75 ? 'into' : y < 0.5 ? 'before' : 'after'
  }
  const menuAt = (event: ReactMouseEvent<HTMLElement>): MenuAt => { const r = event.currentTarget.getBoundingClientRect(); return { x: r.left, y: r.bottom + 2 } }
  return (
    <li role="treeitem" aria-expanded={expandable ? open : undefined} aria-selected={activeId === entity.entity_id} data-testid="aiws-tree-item" data-entity-id={entity.entity_id} data-type={entity.type_id} data-system={entity.system ?? undefined} data-hit={hit ? 'true' : undefined}>
      <div className={`aiws-tree-row aiws-dm-row${activeId === entity.entity_id ? ' is-selected' : ''}${drop ? ` is-drop-${drop}` : ''}`} style={{ paddingLeft: 4 + depth * 14 }}
        draggable={draggable}
        onDragStart={(event) => { event.dataTransfer.setData(DRAG_MIME, entity.entity_id); event.dataTransfer.effectAllowed = 'move' }}
        onDragOver={(event) => { if (!event.dataTransfer.types.includes(DRAG_MIME)) return; event.preventDefault(); event.dataTransfer.dropEffect = 'move'; onDragOver(where(event)) }}
        onDragLeave={onDragLeave}
        onDrop={(event) => onDrop(event, where(event))}
        onContextMenu={(event) => { event.preventDefault(); onMenu({ x: event.clientX, y: event.clientY }) }}
        onPointerDown={press.onPointerDown} onPointerMove={press.onPointerMove} onPointerUp={press.onPointerUp} onPointerCancel={press.onPointerCancel}>
        {expandable ? (
          <button type="button" className="aiws-tree-toggle" aria-label={open ? t('aiworkspace.data.tree.collapse') : t('aiworkspace.data.tree.expand')} onClick={onToggle}>
            {open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}
          </button>
        ) : <span className="aiws-tree-toggle" />}
        <button type="button" className="aiws-tree-label" data-testid={`aiws-tree-${entity.entity_id}`} title={`${typeName(entity)} · ${name}`}
          onClick={() => { if (!press.consumed()) onOpen(true) }} onDoubleClick={() => onOpen(false)}>
          <span className="aiws-dm-type" title={typeName(entity)}>{createElement(iconOf(entity), { size: 16, 'aria-hidden': true })}</span>
          <span className="aiws-outline-name">{name}</span>
          <Marks entity={entity} published={published} />
        </button>
        <button type="button" className={`aiws-dm-row-more${coarse ? ' is-shown' : ''}`} aria-label={t('aiworkspace.datamode.more', { name })} title={t('aiworkspace.datamode.moreTitle')}
          data-testid={`aiws-tree-more-${entity.entity_id}`} onClick={(event) => onMenu(menuAt(event))}><Ellipsis size={16} aria-hidden="true" /></button>
      </div>
      {open && hasChildren && <ul role="group">{children}</ul>}
    </li>
  )
}

function SpecialRow({ id, testId, active, coarse, onOpen, onMenu }: { id: string; testId: string; active: boolean; coarse: boolean; onOpen: (preview: boolean) => void; onMenu: (at: MenuAt) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const press = useLongPress(onMenu)
  const name = nameOf(store, undefined, id)
  return (
    <div className={`aiws-tree-row aiws-dm-row aiws-dm-special${active ? ' is-selected' : ''}`} onContextMenu={(event) => { event.preventDefault(); onMenu({ x: event.clientX, y: event.clientY }) }}
      onPointerDown={press.onPointerDown} onPointerMove={press.onPointerMove} onPointerUp={press.onPointerUp} onPointerCancel={press.onPointerCancel}>
      <span className="aiws-tree-toggle" />
      <button type="button" className="aiws-tree-label" data-testid={testId} aria-pressed={active} onClick={() => { if (!press.consumed()) onOpen(true) }} onDoubleClick={() => onOpen(false)}>
        <span className="aiws-dm-type">{createElement(iconOf(undefined, id), { size: 16, 'aria-hidden': true })}</span>
        <span className="aiws-outline-name">{name}</span>
      </button>
      <button type="button" className={`aiws-dm-row-more${coarse ? ' is-shown' : ''}`} aria-label={t('aiworkspace.datamode.more', { name })} data-testid={`${testId}-more`}
        onClick={(event) => { const r = event.currentTarget.getBoundingClientRect(); onMenu({ x: r.left, y: r.bottom + 2 }) }}><Ellipsis size={16} aria-hidden="true" /></button>
    </div>
  )
}

function MacroGroup({ macros, activeId, coarse, onOpen, onMenu }: { macros: EntityEnvelope[]; activeId: string | null; coarse: boolean; onOpen: (id: string, preview: boolean) => void; onMenu: (id: string, at: MenuAt) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const folded = useUserState<boolean>('ui:data-macros-folded') === true
  return (
    <div className="aiws-dm-macros" data-testid="aiws-dm-macros" data-count={macros.length}>
      <div className="aiws-tree-row aiws-dm-row aiws-dm-group">
        <button type="button" className="aiws-tree-toggle" aria-expanded={!folded} aria-label={folded ? t('aiworkspace.data.tree.expand') : t('aiworkspace.data.tree.collapse')} data-testid="aiws-dm-macros-toggle"
          onClick={() => store.userState.set('ui:data-macros-folded', folded ? null : true)}>{folded ? <ChevronRight size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}</button>
        <span className="aiws-dm-group-label">{t('aiworkspace.datamode.macros')} <span className="aiws-muted">{macros.length}</span></span>
      </div>
      {!folded && macros.length === 0 && <div className="aiws-muted aiws-dm-macros-empty">{t('aiworkspace.datamode.noMacros')}</div>}
      {!folded && macros.map((m) => <MacroRow key={m.entity_id} entity={m} active={activeId === m.entity_id} coarse={coarse} onOpen={(preview) => onOpen(m.entity_id, preview)} onMenu={(at) => onMenu(m.entity_id, at)} />)}
    </div>
  )
}

function MacroRow({ entity, active, coarse, onOpen, onMenu }: { entity: EntityEnvelope; active: boolean; coarse: boolean; onOpen: (preview: boolean) => void; onMenu: (at: MenuAt) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const press = useLongPress(onMenu)
  const name = nameOf(store, entity, entity.entity_id)
  return (
    <div className={`aiws-tree-row aiws-dm-row${active ? ' is-selected' : ''}`} style={{ paddingLeft: 18 }} data-testid="aiws-dm-macro" data-entity-id={entity.entity_id}
      onContextMenu={(event) => { event.preventDefault(); onMenu({ x: event.clientX, y: event.clientY }) }}
      onPointerDown={press.onPointerDown} onPointerMove={press.onPointerMove} onPointerUp={press.onPointerUp} onPointerCancel={press.onPointerCancel}>
      <button type="button" className="aiws-tree-label" data-testid={`aiws-dm-macro-${entity.entity_id}`} onClick={() => { if (!press.consumed()) onOpen(true) }} onDoubleClick={() => onOpen(false)}>
        <span className="aiws-dm-type">{createElement(iconOf(entity), { size: 16, 'aria-hidden': true })}</span>
        <span className="aiws-outline-name">{name}</span>
        {entity.derived && <FreshnessBadge entityId={entity.entity_id} showManual={false} />}
      </button>
      <button type="button" className={`aiws-dm-row-more${coarse ? ' is-shown' : ''}`} aria-label={t('aiworkspace.datamode.more', { name })} data-testid={`aiws-dm-macro-more-${entity.entity_id}`}
        onClick={(event) => { const r = event.currentTarget.getBoundingClientRect(); onMenu({ x: r.left, y: r.bottom + 2 }) }}><Ellipsis size={16} aria-hidden="true" /></button>
    </div>
  )
}
