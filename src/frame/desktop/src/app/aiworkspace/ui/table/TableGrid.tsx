import { useMemo, useEffect, useId, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { GripHorizontal, GripVertical, Menu, Plus } from 'lucide-react'
import type { CapturedAnchor } from '../../anchors/registry'
import type { FieldDef } from '../../api/types'
import { useStore, type AnnotationMark } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useCoarse } from '../datamode/hooks'
import type { PagerSnapshot, TablePager } from '../tablePager'
import { FIELD_TYPE_LABEL, formatValue } from '../values'
import { MAX_ROW_HEIGHT, MIN_ROW_HEIGHT, ROW_HEIGHT } from './appearance'
import { fieldIcon } from './fieldIcons'
import { GridFieldName } from './GridFieldName'
import { trackGridDrag } from './gridDrag'
import { MAX_WIDTH, MIN_WIDTH, WIDTHS, type Column } from './model'
import { TableCell } from './TableCell'
import { anchorOf, TableMenu, type AnchorRect, type MenuItem } from './TablePop'

const PHONE_ROW_HEIGHT = 44
type Axis = 'row' | 'column'
type Side = 'before' | 'after'
type Track = { axis: Axis; id: string }

export interface CellRef { recordId: string; fieldId: string }

// Keep the button in the row's tab order, but paint it outside the grid's scroll clip.
function positionRowMenu(button: HTMLButtonElement | null) {
  const rail = button?.parentElement
  const grip = rail?.querySelector<HTMLButtonElement>('.aiws-track-grip')
  if (!button || !rail || !grip) return
  button.showPopover()
  let frame = 0
  let lastTransform = ''
  const update = () => {
    const handle = grip.getBoundingClientRect()
    const centerX = handle.left + handle.width / 2
    const centerY = handle.top + handle.height / 2
    const halfWidth = grip.offsetWidth / 2
    const hit = document.elementFromPoint(centerX, centerY)
    const visible = halfWidth > 0 && (grip.contains(hit) || button.contains(hit))
    if (visible) {
      // The zero-width rail runs through the grip's left edge. The vector to the
      // grip's center captures both canvas zoom and rotation in viewport coordinates.
      const bounds = rail.getBoundingClientRect()
      const x = bounds.left + bounds.width / 2
      const y = bounds.top + bounds.height / 2
      const a = (centerX - x) / halfWidth
      const b = (centerY - y) / halfWidth
      const transform = `matrix(${a},${b},${-b},${a},${x},${y}) translate(calc(-100% - ${rail.style.left}), -50%)`
      if (transform !== lastTransform) {
        button.style.transform = transform
        lastTransform = transform
      }
    }
    button.style.visibility = visible ? '' : 'hidden'
    frame = requestAnimationFrame(update)
  }
  update()
  return () => {
    cancelAnimationFrame(frame)
    button.hidePopover()
  }
}

export interface TableGridProps {
  tableId: string
  sourceId: string
  fields: FieldDef[]
  columns: Column[]
  rows: PagerSnapshot
  pager: TablePager
  scrollRef: RefObject<HTMLDivElement | null>
  viewportRef?: RefObject<{ left: number; top: number }>
  /** A bounded box (px), or null: fill the editor's height. */
  maxHeight: number | null
  compact: boolean
  phone: boolean
  editable: boolean
  resultTable: boolean
  titleField: FieldDef | undefined
  annotations: AnnotationMark[]
  onAnnotate?: (anchor: CapturedAnchor) => void
  onActivateAnnotation?: (entityId: string | null) => void
  showHandles: boolean
  selection: ReadonlySet<string> | null
  onToggle: (recordId: string, on: boolean) => void
  onToggleAll: (on: boolean) => void
  onOpen: (recordId: string) => void
  annotatedRecords: ReadonlySet<string>
  phoneCell: CellRef | null
  onPhoneCell: (cell: CellRef | null) => void
  onPhoneEdit: (cell: CellRef, editable: boolean) => void
  columnsEditable: boolean
  onResize: (fieldId: string, width: number) => void
  onMove: (fieldId: string, to: number) => void
  showHeader?: boolean
  rowHeights?: Record<string, number>
  sorted?: boolean
  limited?: boolean
  canRenameField?: boolean
  activateCells?: boolean
  allowNearRow?: boolean
  onInsertRow?: (anchorId?: string, side?: Side) => void | Promise<void>
  onInsertColumn?: (anchorId?: string, side?: Side) => void | Promise<void>
  onResizeRow?: (recordId: string, height: number) => void
  onMoveRow?: (recordId: string, anchorId: string, side: Side) => void
  onExitGrid?: () => void
  focusRequest?: { recordId: string; fieldId?: string; action: 'edit' | 'focus' | 'rename'; seq: number } | null
}

export function TableGrid(props: TableGridProps) {
  const { t } = useAiwsText()
  const store = useStore()
  const { tableId, columns, rows, pager, scrollRef, viewportRef, phone, showHandles, selection, rowHeights, focusRequest } = props
  const helpId = useId()
  const menuId = useId()
  const trackControls = useRef<HTMLDivElement>(null)
  const coarsePointer = useCoarse()
  const [touch, setTouch] = useState(coarsePointer)
  const [hoveredField, setHoveredField] = useState<string | null>(null)
  const [track, setTrack] = useState<Track | null>(null)
  const [menu, setMenu] = useState<{ track: Track; anchor: AnchorRect } | null>(null)
  const [preview, setPreview] = useState<(Track & { size: number }) | null>(null)
  const [drop, setDrop] = useState<(Track & { side: Side }) | null>(null)
  const [destination, setDestination] = useState<{ row: number; col: number } | null>(null)
  const handled = useRef<number | null>(null)
  const inserting = useRef(false)
  const suppressGripClick = useRef(false)
  const moveRow = useRef(props.onMoveRow)
  useLayoutEffect(() => { moveRow.current = props.onMoveRow }, [props.onMoveRow])
  const exitRef = useRef<HTMLSpanElement>(null)
  // Reserve room for touch targets even when a mouse moves over a touchscreen first.
  const touchControls = coarsePointer || navigator.maxTouchPoints > 0 || touch
  const defaultHeight = touchControls && (showHandles || selection) ? PHONE_ROW_HEIGHT : ROW_HEIGHT
  const controlSize = touchControls ? 44 : 24
  const headWidth = selection ? controlSize : 0
  const touchGripWidth = touchControls && showHandles ? controlSize : 0
  const rowToolsWidth = controlSize * (props.onResizeRow ? 3 : 2)
  const columnToolsHeight = props.onInsertColumn ? (touchControls ? 68 : 34) : (touchControls ? 60 : 28)
  const railHeight = showHandles ? columnToolsHeight : controlSize
  const headerHeight = (showHandles || selection ? railHeight : 0) + (props.showHeader !== false ? 33 : 0)
  const widthOf = (c: Column, i: number) => {
    const width = preview?.axis === 'column' && preview.id === c.field.field_id ? preview.size : c.width
    if (i !== 0 || !showHandles) return width
    const firstWidth = phone ? Math.min(width, WIDTHS.narrow) : width
    return Math.max(firstWidth, MIN_WIDTH + touchGripWidth, columns.length === 1 ? rowToolsWidth : 0)
  }
  const fieldsWidth = columns.reduce((sum, c, i) => sum + widthOf(c, i), 0)
  const totalWidth = headWidth + (showHandles && !columns.length ? rowToolsWidth : fieldsWidth)
  const frameStyle = { maxWidth: totalWidth + (props.onInsertColumn ? 20 : 0), '--row-control-size': `${controlSize}px` }
  const rowPreview = preview?.axis === 'row' ? preview : null
  const geometry = useMemo(() => ({
    getItemKey: (i: number) => rows.rows[i]?.record_id ?? i,
    estimateSize: (i: number) => {
      const id = rows.rows[i]?.record_id
      if (!id) return defaultHeight
      const px = rowPreview?.id === id ? rowPreview.size : rowHeights?.[id]
      return px && Number.isFinite(px) ? Math.max(MIN_ROW_HEIGHT, Math.min(MAX_ROW_HEIGHT, px)) : defaultHeight
    },
  }), [rows.rows, defaultHeight, rowHeights, rowPreview])
  // eslint-disable-next-line react-hooks/incompatible-library -- the virtualizer owns viewport measurements
  const virtualizer = useVirtualizer({ count: rows.total, getScrollElement: () => scrollRef.current, ...geometry,
    initialOffset: () => viewportRef?.current.top ?? 0, overscan: 8, scrollMargin: headerHeight, scrollPaddingStart: headerHeight })
  useLayoutEffect(() => {
    const root = scrollRef.current
    if (!root || !viewportRef) return
    root.scrollTo(viewportRef.current)
    root.style.setProperty('--aiws-grid-scroll-y', `${root.scrollTop}px`)
    return () => { viewportRef.current = { left: root.scrollLeft, top: root.scrollTop } }
  }, [scrollRef, viewportRef])
  const items = virtualizer.getVirtualItems()
  const lastIndex = items.at(-1)?.index ?? 0
  useEffect(() => { pager.ensure(lastIndex) }, [pager, lastIndex])
  const loaded = rows.rows
  const allSelected = Boolean(selection && loaded.length && loaded.every((row) => selection.has(row.record_id)))
  const selectedColumn = track?.axis === 'column' ? track.id : null

  useEffect(() => {
    if (!track) return
    const idAttribute = track.axis === 'column' ? 'data-field-id' : 'data-record-id'
    const owns = (target: EventTarget | null) => {
      if (!(target instanceof Element)) return false
      if (trackControls.current?.contains(target)) return true
      // The track menu is portaled, including its close button and the phone sheet's backdrop.
      const dialog = document.getElementById(menuId)?.closest('[role="dialog"]')
      if (dialog?.contains(target) || dialog?.closest('.aiws-sheet-layer') === target) return true
      return scrollRef.current?.contains(target) && target.closest(`.aiws-grid-resize.is-${track.axis}`)
        ?.closest(`[${idAttribute}]`)?.getAttribute(idAttribute) === track.id
    }
    const dismiss = () => { setTrack(null); setMenu(null) }
    const onPress = (event: PointerEvent) => { if (event.isPrimary && !owns(event.target)) dismiss() }
    const onBlur = (event: FocusEvent) => { if (owns(event.target) && !owns(event.relatedTarget)) dismiss() }
    document.addEventListener('pointerdown', onPress, true)
    document.addEventListener('focusout', onBlur, true)
    window.addEventListener('blur', dismiss)
    return () => {
      document.removeEventListener('pointerdown', onPress, true)
      document.removeEventListener('focusout', onBlur, true)
      window.removeEventListener('blur', dismiss)
    }
  }, [track, menuId, scrollRef])

  useLayoutEffect(() => {
    const request = focusRequest && handled.current !== focusRequest.seq ? focusRequest : null
    if (!request && !destination) return
    const root = scrollRef.current
    if (!root) return
    if (request?.action === 'rename') {
      const button = root.querySelector<HTMLElement>(`[data-field-name="${CSS.escape(request.fieldId ?? '')}"]`)
      if (button) { handled.current = request.seq; button.click() }
      return
    }
    const row = request ? loaded.findIndex((r) => r.record_id === request.recordId) : destination!.row
    if (row < 0 || row >= rows.total) return
    if (!loaded[row]) { pager.ensure(row); return }
    const col = request?.fieldId ? columns.findIndex((c) => c.field.field_id === request.fieldId) : destination?.col
    const targetRow = root.querySelector<HTMLElement>(`[data-row-index="${row}"]`)
    if (!targetRow) {
      if (items.length) virtualizer.scrollToIndex(row, { align: 'auto' })
      return
    }
    const button = col === undefined
      ? targetRow.querySelector<HTMLElement>('[data-editable="true"] [data-col], [data-editable="true"] [data-ref-edit]') ?? targetRow.querySelector<HTMLElement>('[data-col]')
      : targetRow.querySelector<HTMLElement>(`[data-col="${col}"]`)
    if (!button || (request?.action === 'edit' && !props.editable)) return
    if (request) handled.current = request.seq
    if (destination) setDestination(null)
    button.focus({ preventScroll: true })
    if (request?.action !== 'edit') {
      virtualizer.scrollToIndex(row, { align: 'auto' })
      const rect = button.getBoundingClientRect(), bounds = root.getBoundingClientRect()
      const scale = bounds.width / (root.offsetWidth || 1) || 1
      if (rect.right > bounds.right) root.scrollLeft += (rect.right - bounds.right) / scale
      else if (rect.left < bounds.left + headWidth * scale) root.scrollLeft -= (bounds.left + headWidth * scale - rect.left) / scale
    }
    if (request?.action === 'edit') button.click()
  }, [destination, focusRequest, loaded, rows.total, columns, items, pager, scrollRef, virtualizer, props.editable, headWidth])

  const navigate = (row: number, col: number) => {
    if (row >= 0 && row < rows.total && col >= 0 && col < columns.length) setDestination({ row, col })
  }
  const tab = (row: number, col: number, back: boolean) => {
    const next = row * columns.length + col + (back ? -1 : 1)
    if (next < 0) return
    if (next < rows.total * columns.length) { navigate(Math.floor(next / columns.length), next % columns.length); return }
    if (props.limited) { store.notify('info', t('aiworkspace.table.grid.limit')); return }
    if (!rows.done) { pager.ensure(row + 1); return }
    if (!props.onInsertRow) { store.notify('info', t('aiworkspace.table.readOnly.noAppend')); return }
    if (inserting.current) return
    inserting.current = true
    Promise.resolve(props.onInsertRow()).finally(() => { inserting.current = false })
  }

  const select = (target: Track, handle: HTMLElement) => {
    handle.focus({ preventScroll: true })
    setTrack(target)
  }
  const resize = (event: ReactPointerEvent<HTMLElement>, target: Track, size: number) => {
    const root = scrollRef.current
    if (!root) return
    const column = target.axis === 'column'
    const rect = root.getBoundingClientRect()
    const scale = (column ? rect.width / root.offsetWidth : rect.height / root.offsetHeight) || 1
    const start = column ? event.clientX : event.clientY
    let value = size
    trackGridDrag(event, (next) => {
      const delta = ((column ? next.clientX : next.clientY) - start) / scale
      value = Math.round(Math.min(column ? MAX_WIDTH : MAX_ROW_HEIGHT, Math.max(column ? MIN_WIDTH : MIN_ROW_HEIGHT, size + delta)))
      setPreview({ ...target, size: value })
    }, (cancelled) => {
      setPreview(null)
      if (cancelled || value === size) return
      if (column) props.onResize(target.id, value)
      else props.onResizeRow?.(target.id, value)
    })
  }
  const move = (event: ReactPointerEvent<HTMLElement>, target: Track) => {
    const root = scrollRef.current
    if (!root) return
    const handle = event.currentTarget
    const column = target.axis === 'column'
    const movable = column ? props.columnsEditable : props.onMoveRow && !props.sorted
    const start = column ? event.clientX : event.clientY
    let moved = false
    let position = start
    let frame = 0
    let result: { id: string; side: Side } | undefined
    const update = () => {
      const box = root.getBoundingClientRect()
      const scale = column ? box.width / root.offsetWidth : box.height / root.offsetHeight
      const near = column ? box.left : box.top + headerHeight * scale
      const far = column ? box.right : box.bottom
      const step = position < near + 16 ? -12 : position > far - 16 ? 12 : 0
      const previous = column ? root.scrollLeft : root.scrollTop
      if (column) root.scrollLeft += step
      else root.scrollTop += step
      const scrolled = previous !== (column ? root.scrollLeft : root.scrollTop)
      const targets = [...root.querySelectorAll<HTMLElement>(column ? '[data-col-track]' : '[data-row-index][data-record-id]')]
      const found = targets.find((el) => { const r = el.getBoundingClientRect(); return position < (column ? r.right : r.bottom) }) ?? targets.at(-1)
      if (!found) return
      const r = found.getBoundingClientRect()
      const id = column ? found.dataset.colTrack! : found.dataset.recordId!
      const side = position < (column ? r.left + r.width / 2 : r.top + r.height / 2) ? 'before' : 'after'
      result = { id, side }
      setDrop((current) => current?.axis === target.axis && current.id === id && current.side === side ? current : { axis: target.axis, id, side })
      if (scrolled) frame = requestAnimationFrame(update)
    }
    trackGridDrag(event, (next) => {
      position = column ? next.clientX : next.clientY
      if (!movable || (!moved && Math.abs(position - start) < 6)) return
      moved = true
      cancelAnimationFrame(frame)
      update()
    }, (cancelled) => {
      cancelAnimationFrame(frame)
      setDrop(null)
      suppressGripClick.current = true
      if (cancelled) return
      if (!moved) { select(target, handle); return }
      if (!result || result.id === target.id) return
      if (!column) { moveRow.current?.(target.id, result.id, result.side); return }
      const from = columns.findIndex((c) => c.field.field_id === target.id)
      const slot = columns.findIndex((c) => c.field.field_id === result!.id) + (result.side === 'after' ? 1 : 0)
      const to = slot > from ? slot - 1 : slot
      if (to !== from) props.onMove(target.id, to)
    })
  }

  const menuOpen = (target: Track) => menu?.track.axis === target.axis && menu.track.id === target.id
  const closeMenu = () => {
    if (document.getElementById(menuId)?.closest('[role="dialog"]')?.contains(document.activeElement)) {
      trackControls.current?.querySelector<HTMLButtonElement>('.aiws-track-grip')?.focus({ preventScroll: true })
    }
    setMenu(null)
  }
  const trackMenu = (target: Track, name: string) => (
    <button type="button" className="aiws-track-menu" aria-label={t('aiworkspace.table.grid.actions', { name })} aria-haspopup="menu"
      popover={target.axis === 'row' ? 'manual' : undefined} ref={target.axis === 'row' ? positionRowMenu : undefined}
      aria-expanded={menuOpen(target)} aria-controls={menuOpen(target) ? menuId : undefined}
      data-testid={`aiws-${target.axis === 'column' ? 'col' : 'row'}-menu-${target.id}`} onClick={(e) => setMenu({ track: target, anchor: anchorOf(e.currentTarget) })}><Menu size={14} /></button>
  )
  const grip = (target: Track, label: string, active: boolean) => (
    <button type="button" className="aiws-track-grip" aria-pressed={active} aria-label={label} title={label} data-testid={`aiws-${target.axis === 'column' ? 'col' : 'row'}-handle-${target.id}`}
      onClick={(e) => { if (e.detail === 0) select(target, e.currentTarget) }}
      onPointerDown={(e) => move(e, target)}>{target.axis === 'column' ? <GripHorizontal size={15} /> : <GripVertical size={15} />}</button>
  )
  const insertLabels = {
    row: { before: 'aiworkspace.table.grid.row.before', after: 'aiworkspace.table.grid.row.after' },
    column: { before: 'aiworkspace.table.grid.column.before', after: 'aiworkspace.table.grid.column.after' },
  }
  const menuItems: MenuItem[] = []
  if (menu) {
    const { axis, id } = menu.track
    const insert = axis === 'row' ? (props.allowNearRow === false ? undefined : props.onInsertRow) : props.onInsertColumn
    if (insert) for (const side of ['before', 'after'] as const) {
      menuItems.push({ id: `insert-${axis}-${side}`, label: t(insertLabels[axis][side]), run: () => void insert(id, side) })
    }
    if (axis === 'row') {
      menuItems.push({ id: 'open-record', label: t('aiworkspace.table.openRecord', { name: id }), run: () => props.onOpen(id) })
      if (props.sorted) menuItems.push({ id: 'move-sorted', label: t('aiworkspace.table.grid.sorted'), disabled: t('aiworkspace.table.grid.sorted'), run: () => {} })
    }
  }

  const headStyle = (col: Column, i: number) => ({ width: widthOf(col, i), ...(phone && showHandles && i === 0 ? { left: headWidth } : {}) })
  const separator = (target: Track, size: number, label: string, dot: boolean) => (
    <span role="separator" aria-orientation={target.axis === 'column' ? 'vertical' : 'horizontal'} aria-label={label}
      className={`aiws-grid-resize is-${target.axis}${dot ? ' has-dot' : ''}`}
      data-testid={`aiws-${target.axis === 'column' ? 'col' : 'row'}-resize-${target.id}`} onPointerDown={(e) => resize(e, target, size)} />
  )
  const exited = () => {
    if (props.onExitGrid) props.onExitGrid()
    else exitRef.current?.focus({ preventScroll: true })
  }
  return (
    <div className="aiws-grid-frame" data-pointer={touch ? 'touch' : 'mouse'} data-column-selected={selectedColumn || undefined}
      data-heading-rail={showHandles && !selection ? '' : undefined} style={frameStyle}
      onPointerDownCapture={() => { suppressGripClick.current = false }}
      onClickCapture={(e) => {
        if (e.detail && suppressGripClick.current) { e.preventDefault(); e.stopPropagation(); suppressGripClick.current = false }
      }}
      onPointerMove={(e) => {
        if (e.pointerType !== 'mouse' || e.buttons || !scrollRef.current) return
        const viewport = scrollRef.current.getBoundingClientRect()
        const threshold = 16 * (viewport.width / scrollRef.current.offsetWidth || 1)
        const bottom = Math.abs(viewport.bottom - e.clientY), right = Math.abs(viewport.right - e.clientX)
        const near = bottom < threshold && props.onInsertRow && (bottom <= right || !props.onInsertColumn) ? 'row'
          : right < threshold && props.onInsertColumn ? 'column' : ''
        if (e.currentTarget.dataset.near !== near) e.currentTarget.dataset.near = near
      }}
      onPointerOver={(event) => {
        setTouch(event.pointerType === 'touch')
        const cell = (event.target as HTMLElement).closest<HTMLElement>('[role=gridcell]')
        if (cell?.dataset.fieldId) setHoveredField(cell.dataset.fieldId)
      }} onPointerLeave={(e) => { delete e.currentTarget.dataset.near; setHoveredField(null) }}>
      <span className="aiws-grid-help" id={helpId}>{t('aiworkspace.table.grid.keys')}</span>
      <div ref={scrollRef} className="aiws-table-scroll" style={{ maxHeight: props.maxHeight ?? undefined, scrollPaddingInlineStart: phone ? headWidth + (columns[0] ? widthOf(columns[0], 0) : 0) : headWidth }}
        onScroll={({ currentTarget }) => {
          const { scrollLeft, scrollTop } = currentTarget
          if (viewportRef) viewportRef.current = { left: scrollLeft, top: scrollTop }
          currentTarget.style.setProperty('--aiws-grid-scroll-y', `${scrollTop}px`)
        }}
        data-testid={`aiws-table-scroll-${tableId}`} role="grid" aria-describedby={helpId}
        aria-rowcount={rows.total + (props.showHeader === false ? 0 : 1)} aria-colcount={columns.length} data-phone={phone ? 'true' : undefined}>
        <div className="aiws-grid-top" style={{ width: totalWidth }}>
          {(showHandles || selection) && (
            <div className="aiws-track-header" style={{ height: railHeight }}>
              {selection && <div className="aiws-track-corner" style={{ width: headWidth }}>
                <input type="checkbox" className="aiws-row-check" aria-label={t('aiworkspace.table.selectAllLoaded')} data-testid={`aiws-row-check-all-${tableId}`}
                  checked={allSelected} onChange={(e) => props.onToggleAll(e.target.checked)} />
              </div>}
              {showHandles && columns.map((column, i) => {
                const id = column.field.field_id, name = column.field.name
                const selected = selectedColumn === id
                return <div key={id} ref={selected ? trackControls : undefined} className={`aiws-col-track${selected || hoveredField === id ? ' is-active' : ''}${phone && i === 0 ? ' is-sticky' : ''}`} data-col-track={id} style={headStyle(column, i)}>
                  {grip({ axis: 'column', id }, t('aiworkspace.table.grid.selectColumn', { name }), selected)}
                  {selected && props.onInsertColumn && trackMenu({ axis: 'column', id }, name)}
                  {props.columnsEditable && separator({ axis: 'column', id }, widthOf(column, i), t('aiworkspace.table.column.resize', { name }), selected)}
                  {drop?.axis === 'column' && drop.id === id && <span className={`aiws-track-drop is-${drop.side}`} data-testid="aiws-col-drop" />}
                </div>
              })}
            </div>
          )}
          {props.showHeader !== false && <div className="aiws-table-head" role="row">
            {selection && <div className="aiws-th-rowhead" style={{ width: headWidth }} />}
            {columns.map((column, i) => {
              const { field } = column, Icon = fieldIcon(field)
              return <div key={field.field_id} className={`aiws-th${phone && i === 0 && showHandles ? ' is-sticky' : ''}${track?.axis === 'column' && track.id === field.field_id ? ' is-selected-column' : ''}`}
                role="columnheader" aria-label={field.name} data-field-id={field.field_id} data-type={field.type} data-testid={`aiws-th-${field.field_id}`}
                title={t('aiworkspace.data.table.columnTitle', { name: field.name, type: FIELD_TYPE_LABEL[field.type] })} style={headStyle(column, i)}>
                <Icon size={13} className="aiws-th-icon" aria-hidden="true" />
                <GridFieldName field={field} fields={props.fields} sourceId={props.sourceId} editable={Boolean(props.canRenameField)} />
                {field.maintained_by === 'program' && <span className="aiws-chip" title={t('aiworkspace.data.table.programTitle')}>{t('aiworkspace.data.table.program')}</span>}
                {props.columnsEditable && <span className="aiws-grid-resize is-column" aria-hidden="true" onPointerDown={(e) => resize(e, { axis: 'column', id: field.field_id }, widthOf(column, i))} />}
              </div>
            })}
          </div>}
        </div>
        <div className="aiws-grid-rows" style={{ height: virtualizer.getTotalSize(), width: totalWidth }}>
          {items.map((item) => {
            const row = rows.rows[item.index]
            const id = row?.record_id ?? ''
            const checked = Boolean(row && selection?.has(id))
            const active = track?.axis === 'row' && track.id === id
            const title = row && props.titleField ? formatValue(props.titleField, row.values[props.titleField.field_id]) || id : id
            return (
              <div key={item.key} className={`aiws-tr${checked || active ? ' is-selected' : ''}`} role="row" aria-rowindex={item.index + (props.showHeader === false ? 1 : 2)}
                data-row-index={item.index} data-last-row={item.index === rows.total - 1 || undefined} data-stripe={item.index % 2} data-drop={drop?.axis === 'row' && drop.id === id ? drop.side : undefined}
                data-testid="aiws-row" data-record-id={id || undefined} data-row-selected={active || undefined} aria-selected={selection ? checked : showHandles ? active : undefined}
                style={{ transform: `translateY(${item.start - headerHeight}px)`, height: item.size, width: totalWidth }}>
                {row ? <>
                  {selection && <div className="aiws-td aiws-td-rowhead" style={{ width: headWidth }}>
                    <label className="aiws-row-check-hit"><input type="checkbox" className="aiws-row-check"
                      aria-label={t('aiworkspace.table.selectRecord', { name: title })} data-testid={`aiws-row-check-${id}`} checked={checked} onChange={(e) => props.onToggle(id, e.target.checked)} /></label>
                  </div>}
                  {showHandles && <div className="aiws-row-tools" ref={active ? trackControls : undefined} style={{ left: headWidth }}>
                    {grip({ axis: 'row', id }, t('aiworkspace.table.selectRecord', { name: title }), active)}
                    {active && trackMenu({ axis: 'row', id }, title)}
                    {active && props.onResizeRow && separator({ axis: 'row', id }, item.size, t('aiworkspace.table.grid.resizeRow', { name: title }), true)}
                  </div>}
                  {columns.map((column, col) => <TableCell key={column.field.field_id} {...props} field={column.field} row={row} rowIndex={item.index} colIndex={col}
                    resizeColumn={!touch && props.columnsEditable ? (event) => resize(event, { axis: 'column', id: column.field.field_id }, widthOf(column, col)) : undefined}
                    activeColumn={track?.axis === 'column' && track.id === column.field.field_id}
                    width={widthOf(column, col)} sticky={phone && col === 0 && showHandles ? headWidth : null} onNavigate={navigate} onTab={tab} onExit={exited}
                    annotation={props.annotations.find((mark) => mark.payload.target?.entity_id === props.sourceId && mark.payload.target.selector?.kind === 'table_cell'
                      && mark.payload.target.selector.record_id === id && mark.payload.target.selector.field_id === column.field.field_id)} />)}
                  {!touch && props.onResizeRow && <span className="aiws-grid-resize is-row" style={{ left: headWidth }} aria-hidden="true"
                    onPointerDown={(event) => resize(event, { axis: 'row', id }, item.size)} />}
                </> : <div className="aiws-td aiws-muted" style={{ width: totalWidth }}>…</div>}
              </div>
            )
          })}
        </div>
      </div>
      {props.onInsertColumn && <button type="button" className="aiws-grid-edge is-right" data-testid={`aiws-edge-column-${tableId}`} aria-label={t('aiworkspace.table.grid.addColumn')}
        onClick={() => void props.onInsertColumn?.()}><Plus size={13} aria-hidden="true" /></button>}
      {props.onInsertRow && <button type="button" className="aiws-grid-edge is-bottom" data-testid={`aiws-add-record-${tableId}`} aria-label={t('aiworkspace.table.grid.addRow')}
        onClick={() => void props.onInsertRow?.()}><Plus size={13} aria-hidden="true" /></button>}
      {menu && menuItems.length > 0 && <TableMenu id={menuId} anchor={menu.anchor} label={t('aiworkspace.table.grid.actions', { name: menu.track.id })} items={menuItems} onClose={closeMenu} />}
      <span ref={exitRef} tabIndex={-1} className="aiws-grid-help" aria-label={t('aiworkspace.table.grid.keys')} />
    </div>
  )
}
