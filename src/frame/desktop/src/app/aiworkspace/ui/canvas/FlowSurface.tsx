/* A flow-layout Surface (phase two §8.1; 流式页交互改进 §2): a document column in Notion's manner. Blocks and
 * groups follow in sibling order with no frame of their own; in edit mode every editor is live, so clicking puts
 * the caret where it lands. Hovering a Block shows its handles in the left margin: ＋ inserts below, ⋮⋮ drags it
 * to another place (or into a group) and, clicked, selects it and opens its menu. Below the last Block a click
 * continues writing. Moves are auto-merge (`tree.place` / `tree.move`). */

import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { GripVertical, Plus } from 'lucide-react'
import { describeError } from '../../api/session'
import type { EntityEnvelope } from '../../api/types'
import { orderKeyBetween } from '../../api/wasm'
import { aiwsT } from '../../i18n'
import { useOutlineVersion, useReadOnlyReason, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { BlockHost } from '../blocks/BlockHost'
import { requestIntent } from '../blocks/editorToolbar'
import { FlowBlockContext, type FlowBlockApi } from '../blocks/flowContext'
import { modePolicy, type CanvasMode } from '../blocks/registry'
import { MenuList, useOverlayMounted, type MenuItem } from '../shell/popover'
import { registryEntries, type CatalogEntry } from './catalog'
import { deleteOps, flowChildren, moveInFlow } from './flowOps'
import { accepted, renameBase, renameSurface, surfaceWriteReason } from './surfaceManage'

/** Types a page shows a heading line for (a table, a record…); text, pictures and notes carry their own. */
const UNTITLED = new Set(['richtext', 'asset', 'note', 'shape', 'frame'])
/** Kinds offered by ＋ (canvas-only ones left out). */
const FLOW_SKIP = new Set(['note', 'shape', 'frame', 'connector'])

interface DropTarget { parentId: string; refId: string | null; before: boolean; line: { top: number; left: number; width: number } }

export interface FlowSurfaceProps {
  surfaceId: string
  mode: CanvasMode
  selected: ReadonlySet<string>
  onSelect: (ids: Set<string>) => void
  /** The full insert menu (catalog, group, existing data, paste) for a container, at a point of the canvas area. */
  onInsertMenu: (parentId: string, at: { x: number; y: number }) => void
  /** Insert a Block of `entry` into `parentId` right after `afterId` (first when null). */
  insertAt: (entry: CatalogEntry, parentId: string, afterId: string | null) => void
  /** Insert a group into `parentId` right after `afterId`. */
  insertGroup: (parentId: string, afterId: string | null) => void
  /** Open the Block's properties in the side panel. */
  onInspect: (id: string) => void
}

export function FlowSurface(props: FlowSurfaceProps) {
  const { surfaceId, mode, onSelect } = props
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  const policy = modePolicy(mode)
  const surface = store.outline.get(surfaceId)
  const [column, bindColumn] = useState<HTMLDivElement | null>(null)
  const [drag, setDrag] = useState<{ id: string; target: DropTarget | null } | null>(null)
  if (!surface) return null
  const canStructure = policy.insert && surface.capabilities.includes('structure')
  const top = flowChildren(store, surfaceId)
  const last = top.at(-1)

  // a click below the last Block continues writing: in the last text, or in a new one
  const continueWriting = () => {
    if (last?.view_type === 'richtext' && last.type_id === 'buckyos.cell') { requestIntent(last.entity_id, 'newline'); return }
    const text = registryEntries().find((entry) => entry.definition.type === 'richtext')
    if (text) props.insertAt(text, surfaceId, last?.entity_id ?? null)
  }

  /** Where a dragged item would land for a pointer at `y` (screen): before or after the item under it, or into a group's tail. */
  const dropTarget = (dragged: HTMLElement, y: number): DropTarget | null => {
    if (!column) return null
    const box = column.getBoundingClientRect()
    const items = [...column.querySelectorAll<HTMLElement>('[data-flow-item]')].filter((el) => el !== dragged && !dragged.contains(el))
    let best: HTMLElement | null = null
    let nearest: { el: HTMLElement; d: number } | null = null
    for (const el of items) {
      const r = el.getBoundingClientRect()
      if (y >= r.top && y <= r.bottom) best = el
      const d = y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0
      if (!nearest || d < nearest.d) nearest = { el, d }
    }
    const el = best ?? nearest?.el
    if (!el) return null
    const r = el.getBoundingClientRect()
    if (el.dataset.flowInto) return { parentId: el.dataset.flowInto, refId: null, before: false, line: { top: r.top - box.top, left: r.left - box.left, width: r.width } }
    const before = y < r.top + r.height / 2
    return { parentId: el.dataset.parentId ?? surfaceId, refId: el.dataset.cellId ?? null, before, line: { top: (before ? r.top : r.bottom) - box.top, left: r.left - box.left, width: r.width } }
  }

  const drop = (id: string, target: DropTarget) => {
    const entity = store.outline.get(id)
    if (!entity) return
    const siblings = store.outline.childrenOf(target.parentId).filter((item) => item.entity_id !== id)
    let beforeKey: string | undefined
    let afterKey: string | undefined
    if (target.refId === null) beforeKey = siblings.at(-1)?.order_key
    else {
      const index = siblings.findIndex((item) => item.entity_id === target.refId)
      if (index < 0) return
      beforeKey = target.before ? siblings[index - 1]?.order_key : siblings[index].order_key
      afterKey = target.before ? siblings[index].order_key : siblings[index + 1]?.order_key
    }
    // dropped where it already is
    const current = store.outline.childrenOf(target.parentId)
    const at = current.findIndex((item) => item.entity_id === id)
    if (entity.parent_id === target.parentId && at >= 0 && current[at - 1]?.order_key === beforeKey && current[at + 1]?.order_key === afterKey) return
    let key: string
    try { key = orderKeyBetween(store.core, beforeKey, afterKey) } catch (error) { store.notify('error', aiwsT('aiworkspace.canvas.flow.orderKeyFailed', { reason: describeError(error) })); return }
    const label = aiwsT('aiworkspace.canvas.moveNamedLabel', { name: entity.title ?? entity.name ?? entity.entity_id })
    void store.submit({ editId: `entity:${id}`, label, operations: [entity.parent_id === target.parentId
      ? { op: 'tree.place', entity_id: id, order_key: key }
      : { op: 'tree.move', entity_id: id, new_parent_id: target.parentId, order_key: key }] })
  }

  /** Press on ⋮⋮: a drag moves the item, a click selects it and opens its menu. */
  const startHandle = (event: ReactPointerEvent<HTMLButtonElement>, entity: EntityEnvelope, openMenu: () => void) => {
    if (event.button !== 0) return
    event.preventDefault()
    const handle = event.currentTarget
    const item = handle.closest<HTMLElement>('[data-flow-item]')
    const host = handle.closest<HTMLElement>('.aiws-flow-host')
    const movable = policy.layout && entity.capabilities.includes('structure') && Boolean(item)
    const start = { x: event.clientX, y: event.clientY }
    let dragging = false
    let target: DropTarget | null = null
    handle.setPointerCapture(event.pointerId)
    const move = (e: PointerEvent) => {
      if (!movable || !item) return
      if (!dragging && Math.hypot(e.clientX - start.x, e.clientY - start.y) < 5) return
      dragging = true
      // near the top or bottom edge the page scrolls
      if (host) {
        const r = host.getBoundingClientRect()
        if (e.clientY < r.top + 90) host.scrollTop -= 14
        else if (e.clientY > r.bottom - 50) host.scrollTop += 14
      }
      target = dropTarget(item, e.clientY)
      setDrag({ id: entity.entity_id, target })
    }
    const end = (e: PointerEvent) => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', end)
      handle.removeEventListener('pointercancel', end)
      setDrag(null)
      if (dragging) { if (target && e.type === 'pointerup') drop(entity.entity_id, target) }
      else if (e.type === 'pointerup') { onSelect(new Set([entity.entity_id])); openMenu() }
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', end)
    handle.addEventListener('pointercancel', end)
  }

  const shared = { ...props, startHandle, dragging: drag?.id ?? null, policy }
  return (
    <div ref={bindColumn} className={`aiws-flow${drag ? ' is-dragging' : ''}`} data-testid="aiws-flow" data-surface-id={surfaceId} data-mode={mode}>
      <PageTitle surface={surface} mode={mode} />
      <ContainerFlow container={surface} {...shared} />
      {canStructure ? (
        <div className="aiws-flow-tail">
          <button type="button" className="aiws-flow-tail-add" aria-label={t('aiworkspace.canvas.flow.insertBlock')} title={t('aiworkspace.canvas.flow.insertBlockTip')} data-testid={`aiws-add-open-${surfaceId}`}
            onClick={(event) => props.onInsertMenu(surfaceId, menuPoint(event.currentTarget))}><Plus size={16} aria-hidden="true" /></button>
          <button type="button" className="aiws-flow-continue" data-testid="aiws-flow-continue" onClick={continueWriting}>
            {t(top.length === 0 ? 'aiworkspace.canvas.flow.startWriting' : 'aiworkspace.canvas.flow.continueWriting')}
          </button>
        </div>
      ) : top.length === 0 && <div className="aiws-muted aiws-flow-empty">{t('aiworkspace.canvas.flow.empty')}</div>}
      {drag?.target && <div className="aiws-flow-drop" data-testid="aiws-flow-drop" style={{ top: drag.target.line.top - 1, left: drag.target.line.left, width: drag.target.line.width }} />}
    </div>
  )
}

/** A point of the canvas area just below `element` (where the insert menu opens). */
function menuPoint(element: HTMLElement): { x: number; y: number } {
  const area = element.closest('.aiws-canvas-main') ?? element.offsetParent
  const box = area?.getBoundingClientRect() ?? { left: 0, top: 0 }
  const r = element.getBoundingClientRect()
  return { x: Math.round(r.left - box.left), y: Math.round(r.bottom - box.top + 4) }
}

/** The page title, renamed in place (UI improvement §5: the version read when editing starts is expected). */
function PageTitle({ surface, mode }: { surface: EntityEnvelope; mode: CanvasMode }) {
  const store = useStore()
  const { t } = useAiwsText()
  const readOnlyNow = useReadOnlyReason()
  const [editing, setEditing] = useState<{ text: string; base: Awaited<ReturnType<typeof renameBase>> } | null>(null)
  const title = surface.title ?? surface.name ?? t('aiworkspace.canvas.defaultName')
  const reason = surfaceWriteReason(store, surface, mode !== 'edit', readOnlyNow)
  const done = async (save: boolean) => {
    const current = editing
    setEditing(null)
    const next = current?.text.trim() ?? ''
    if (!save || !current || !next || next === title) return
    const outcome = await renameSurface(store, surface, next, current.base)
    if (!accepted(outcome)) store.notify('error', aiwsT('aiworkspace.canvas.flow.renameFailed', { reason: outcome.status === 'rejected' || outcome.status === 'conflict' ? outcome.code : outcome.status }))
  }
  if (editing) {
    return (
      <input className="aiws-page-title aiws-page-title-input" aria-label={t('aiworkspace.canvas.flow.pageTitle')} data-testid="aiws-page-title-input" autoFocus value={editing.text}
        onChange={(event) => setEditing({ ...editing, text: event.target.value })} onBlur={() => { void done(true) }}
        onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); void done(true) } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); void done(false) } }} />
    )
  }
  return (
    <h1 className={`aiws-page-title${reason === null ? ' is-editable' : ''}`} data-testid="aiws-page-title" title={reason === null ? t('aiworkspace.canvas.clickToRename') : undefined}
      onClick={() => { if (reason === null) void renameBase(store, surface).then((base) => setEditing({ text: title, base }), (error: unknown) => store.notify('error', describeError(error))) }}>
      {title}
    </h1>
  )
}

interface ItemProps extends FlowSurfaceProps {
  startHandle: (event: ReactPointerEvent<HTMLButtonElement>, entity: EntityEnvelope, openMenu: () => void) => void
  dragging: string | null
  policy: ReturnType<typeof modePolicy>
}

function ContainerFlow({ container, ...props }: ItemProps & { container: EntityEnvelope }) {
  const store = useStore()
  const shown = flowChildren(store, container.entity_id)
  return (
    <>
      {shown.map((entity, index) => (
        entity.type_id === 'buckyos.cell'
          ? <FlowBlock key={entity.entity_id} cell={entity} siblings={shown} index={index} {...props} />
          : <FlowGroup key={entity.entity_id} group={entity} siblings={shown} index={index} {...props} />
      ))}
    </>
  )
}

/** ＋ and ⋮⋮ in the left margin of a hovered Block or group, with the ＋ menu and the item menu. */
function Gutter({ entity, siblings, index, kind, ...props }: ItemProps & { entity: EntityEnvelope; siblings: EntityEnvelope[]; index: number; kind: 'block' | 'group' }) {
  const store = useStore()
  const { t } = useAiwsText()
  const [open, setOpen] = useState<'add' | 'menu' | null>(null)
  useOverlayMounted(open !== null)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const onDown = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) setOpen(null) }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); setOpen(null) } }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true) }
  }, [open])
  const { policy } = props
  const parentId = entity.parent_id ?? props.surfaceId
  const parent = store.outline.get(parentId)
  const canInsert = policy.insert && Boolean(parent?.capabilities.includes('structure'))
  const canMove = policy.layout && entity.capabilities.includes('structure')
  const canDelete = policy.layout && entity.capabilities.includes('delete')
  if (!policy.select && !canInsert) return null
  const name = entity.title ?? entity.name ?? (kind === 'group' ? t('aiworkspace.canvas.group') : entity.view_type ?? 'Block')
  const addItems: MenuItem[] = [
    ...registryEntries().filter((entry) => entry.catalog.standard && !FLOW_SKIP.has(entry.definition.type)).map((entry): MenuItem => ({
      id: `add-${entry.definition.type}`, testId: `aiws-flow-add-${entry.definition.type}`, label: entry.title, run: () => props.insertAt(entry, parentId, entity.entity_id),
    })),
    { id: 'add-group', testId: 'aiws-flow-add-group', label: t('aiworkspace.canvas.group'), run: () => props.insertGroup(parentId, entity.entity_id) },
    { id: 'sep-add', separator: true },
    { id: 'add-more', testId: 'aiws-flow-add-more', label: t('aiworkspace.canvas.flow.more'), run: () => { if (ref.current) props.onInsertMenu(parentId, menuPoint(ref.current)) } },
  ]
  const menuItems: MenuItem[] = [
    { id: 'up', testId: `aiws-up-${entity.entity_id}`, label: t('aiworkspace.canvas.flow.moveUp'), hint: 'Ctrl+Shift+↑', disabled: !canMove || index === 0, run: () => moveInFlow(store, entity, -1) },
    { id: 'down', testId: `aiws-down-${entity.entity_id}`, label: t('aiworkspace.canvas.flow.moveDown'), hint: 'Ctrl+Shift+↓', disabled: !canMove || index === siblings.length - 1, run: () => moveInFlow(store, entity, 1) },
    { id: 'inspect', testId: `aiws-inspect-${entity.entity_id}`, label: t('aiworkspace.canvas.inspector.title'), hint: 'F2', run: () => props.onInspect(entity.entity_id) },
    { id: 'sep-delete', separator: true },
    { id: 'delete', testId: kind === 'group' ? `aiws-delete-group-${entity.entity_id}` : `aiws-delete-block-${entity.entity_id}`, danger: true,
      label: kind === 'group' ? t('aiworkspace.canvas.flow.deleteGroup') : t('aiworkspace.common.delete'), hint: 'Delete', disabled: !canDelete,
      run: () => { void store.submit({ editId: `entity:${entity.entity_id}`, label: aiwsT(kind === 'group' ? 'aiworkspace.canvas.flow.deleteGroupLabel' : 'aiworkspace.canvas.flow.deleteBlockLabel', { name }), operations: deleteOps(store, entity) }) } },
  ]
  return (
    <div ref={ref} className={`aiws-flow-gutter${open ? ' is-open' : ''}`} onPointerDown={(event) => event.stopPropagation()}>
      {canInsert && (
        <button type="button" className="aiws-flow-handle" aria-label={t('aiworkspace.canvas.flow.insertBelow')} title={t('aiworkspace.canvas.flow.insertBelow')} data-testid={`aiws-flow-plus-${entity.entity_id}`} aria-expanded={open === 'add'}
          onMouseDown={(event) => event.preventDefault()} onClick={() => setOpen(open === 'add' ? null : 'add')}><Plus size={16} aria-hidden="true" /></button>
      )}
      {policy.select && (
        <button type="button" className="aiws-flow-handle aiws-flow-grip" aria-label={t('aiworkspace.canvas.flow.handleNamed', { name })} title={t('aiworkspace.canvas.flow.handle')} data-testid={`aiws-flow-handle-${entity.entity_id}`}
          aria-expanded={open === 'menu'} onPointerDown={(event) => props.startHandle(event, entity, () => setOpen('menu'))}
          onKeyDown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); props.onSelect(new Set([entity.entity_id])); setOpen('menu') } }}>
          <GripVertical size={16} aria-hidden="true" />
        </button>
      )}
      {open === 'add' && <MenuList items={addItems} label={t('aiworkspace.canvas.flow.insertBelow')} className="aiws-popover aiws-popover-start aiws-flow-menu" testId="aiws-flow-add-menu" onDone={() => setOpen(null)} />}
      {open === 'menu' && <MenuList items={menuItems} label={name} className="aiws-popover aiws-popover-start aiws-flow-menu" testId="aiws-flow-block-menu" onDone={() => setOpen(null)} />}
    </div>
  )
}

function FlowGroup({ group, siblings, index, ...props }: ItemProps & { group: EntityEnvelope; siblings: EntityEnvelope[]; index: number }) {
  const store = useStore()
  const { t } = useAiwsText()
  const { policy } = props
  const isSelected = props.selected.has(group.entity_id)
  const canInsert = policy.insert && group.capabilities.includes('structure')
  const children = flowChildren(store, group.entity_id)
  return (
    <section className={`aiws-group aiws-fgroup${isSelected ? ' is-selected' : ''}${props.dragging === group.entity_id ? ' is-dragged' : ''}`} data-testid={`aiws-group-${group.entity_id}`}
      data-cell-id={group.entity_id} data-parent-id={group.parent_id ?? undefined} data-flow-item="group" tabIndex={-1}>
      <Gutter entity={group} siblings={siblings} index={index} kind="group" {...props} />
      <div className="aiws-fgroup-title">{group.title ?? group.name ?? t('aiworkspace.canvas.group')}</div>
      <ContainerFlow container={group} {...props} />
      {canInsert && (
        <div className="aiws-fgroup-tail" data-flow-item="tail" data-flow-into={group.entity_id}>
          <button type="button" className="aiws-link" data-testid={`aiws-add-open-${group.entity_id}`} onClick={(event) => props.onInsertMenu(group.entity_id, menuPoint(event.currentTarget))}>
            {children.length === 0 ? t('aiworkspace.canvas.flow.addToGroup') : '+'}
          </button>
        </div>
      )}
    </section>
  )
}

function FlowBlock({ cell, siblings, index, ...props }: ItemProps & { cell: EntityEnvelope; siblings: EntityEnvelope[]; index: number }) {
  const store = useStore()
  const { policy, mode } = props
  const isSelected = props.selected.has(cell.entity_id)
  const ref = useRef<HTMLElement>(null)
  // a selected Block takes the keyboard focus (arrows, Enter, Delete reach the page)
  useLayoutEffect(() => {
    const el = ref.current
    if (isSelected && el && !el.contains(document.activeElement)) el.focus({ preventScroll: false })
  }, [isSelected])
  const api: FlowBlockApi = {
    cellId: cell.entity_id,
    insertAfter: (entry) => props.insertAt(entry, cell.parent_id ?? props.surfaceId, cell.entity_id),
    select: () => { props.onSelect(new Set([cell.entity_id])); ref.current?.focus() },
  }
  const source = cell.source_id ? store.outline.get(cell.source_id) : undefined
  const title = cell.title ?? (source ? source.title ?? source.name ?? '' : '')
  const showTitle = !UNTITLED.has(cell.view_type ?? '')
  // Esc in a field hands the selection to the page (Notion): the next Esc clears it. Rich text does the same
  // through FlowBlockContext (ProseMirror swallows Esc); a field that used Esc itself (cancel) prevents it.
  const onKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key !== 'Escape' || event.defaultPrevented || !policy.select) return
    const target = event.target as HTMLElement
    if (!target.closest('textarea, input, select, .aiws-table [data-row]') || target.closest('.aiws-popover, .aiws-menu, .aiws-slash')) return
    event.preventDefault()
    target.blur()
    props.onSelect(new Set([cell.entity_id]))
    ref.current?.focus()
  }
  return (
    <FlowBlockContext.Provider value={api}>
      <section ref={ref} className={`aiws-cell aiws-fblock${isSelected ? ' is-selected' : ''}${props.dragging === cell.entity_id ? ' is-dragged' : ''}`} tabIndex={-1}
        data-testid={`aiws-cell-frame-${cell.entity_id}`} data-cell-id={cell.entity_id} data-cell-source={cell.source_id ?? undefined} data-view={cell.view_type ?? undefined}
        data-mode={mode} data-parent-id={cell.parent_id ?? undefined} data-flow-item="block"
        onKeyDown={onKeyDown} onPointerDown={() => { if (props.selected.size > 0) props.onSelect(new Set()) }}>
        <Gutter entity={cell} siblings={siblings} index={index} kind="block" {...props} />
        {showTitle && <BlockTitle cell={cell} title={title} canRename={policy.writes && mode === 'edit' && cell.capabilities.includes('update')} />}
        <div className="aiws-cell-body">
          {/* every editor of a page in edit mode is live (phase two §8.1): no activation step */}
          <BlockHost cellId={cell.entity_id} mode={mode} view="canvas" selected={isSelected} editorActive={policy.editContent} />
        </div>
      </section>
    </FlowBlockContext.Provider>
  )
}

/** The heading line of a table, record or wish on a page: the Block's title, renamed in place (an empty one clears it). */
function BlockTitle({ cell, title, canRename }: { cell: EntityEnvelope; title: string; canRename: boolean }) {
  const store = useStore()
  const { t } = useAiwsText()
  const [text, setText] = useState<string | null>(null)
  const save = async (next: string) => {
    setText(null)
    if (next === (cell.title ?? title)) return
    const read = await store.session.read<{ key_revs: Record<string, number> }>(cell.entity_id)
    const rev = read.content.key_revs.title ?? 0
    void store.submit({
      editId: `key:${cell.entity_id}:title`, label: next ? aiwsT('aiworkspace.canvas.blockTitleLabel', { title: next }) : aiwsT('aiworkspace.canvas.blockTitleClearedLabel'), mine: next, hasMine: true,
      operations: [next === ''
        ? { op: 'entity.unset_keys', entity_id: cell.entity_id, keys: [{ key: 'title', expect: { rev } }] }
        : { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'title', value: next, expect: { rev } }] }],
    })
  }
  if (text !== null) {
    return (
      <input className="aiws-fblock-title aiws-fblock-title-input" aria-label={t('aiworkspace.canvas.blockTitle')} data-testid={`aiws-cell-title-input-${cell.entity_id}`} autoFocus value={text} placeholder={t('aiworkspace.canvas.noTitle')}
        onChange={(event) => setText(event.target.value)} onBlur={() => { void save(text.trim()) }}
        onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); void save(text.trim()) } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setText(null) } }} />
    )
  }
  return (
    <div className={`aiws-fblock-title${canRename ? ' is-editable' : ''}${title ? '' : ' is-empty'}`} data-testid={`aiws-cell-title-${cell.entity_id}`}
      title={canRename ? t('aiworkspace.canvas.clickToRename') : undefined} onClick={() => { if (canRename) setText(title) }}>
      {title || t('aiworkspace.canvas.noTitle')}
    </div>
  )
}
