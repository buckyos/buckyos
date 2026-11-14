/* The tool selection zone (UI 框架 PRD §4.3; UI improvement §7; 工具栏双排, 2026-11-12), from top to bottom: the
 * pointer group — select (with pan), design / run (Q3), pen | the object group — text, rich text, media, table, mind
 * map, shape, connector, frame, wish, sticky note, insert… | undo, redo. A flow page's pointer group is design / run
 * alone. A button with a corner mark opens its second column beside it: select / pan; table, calendar, to-do and the
 * standard tables; the basic shapes merged with the shape library. The others act at once: picking an object on a
 * free Surface starts a one-shot placement, on a flow page it appends. Which objects the group holds comes from the
 * toolbar configuration (built-in list, the canvas's `config.toolbar`). The workspace's extensions are reached through
 * "insert…"; the ones the user pinned follow it. The eraser is in the pen's bar. */

import { useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { ChevronsDown, ChevronsUp, Hand, MousePointer2, PenLine, Plus, Redo2, Shapes, Table, Undo2, type LucideIcon } from 'lucide-react'
import { useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import type { RunMode } from '../../state/runMode'
import { useAiwsText } from '../../useAiwsText'
import { blockRegistry } from '../blocks/registry'
import { ShapePicker } from '../blocks/shapeBlock'
import { usePopover } from '../shell/popover'
import { useFilePicker } from '../shell/filePicker'
import { ENTRY_ICONS, entryId, extensionEntries, registryEntries, type CatalogEntry } from './catalog'
import { ShapeLibraryPanel } from './ShapeLibraryPanel'
import { libraryShapeConfig } from './stencilLibrary'
import { RunModeToggle } from './tools'
import { TOOL_OBJECTS } from '../framework/config'
/** The table group's second column: the views, then the standard tables (系统已知 schema_id). */
const TABLE_GROUP = ['table', 'calendar', 'todo']
const STANDARD_TABLES = ['table-todo', 'table-calendar']

const iconOf = (entry: CatalogEntry): LucideIcon => ENTRY_ICONS[entryId(entry)] ?? Plus

/** The second column of a toolbar button: beside the toolbar, level with its button, kept inside the canvas area
 * (moved up when it would run past the bottom, its height capped by the area's). */
function SideColumn({ label, testId, className = '', children }: { label: string; testId: string; className?: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    const el = ref.current
    const anchor = el?.parentElement
    if (!el || !anchor) return
    const place = () => {
      const area = (el.closest('.aiws-canvas-main') ?? el.closest('.aiws-root') ?? document.documentElement).getBoundingClientRect()
      const top = anchor.getBoundingClientRect().top
      el.style.setProperty('--aiws-column-room', `${Math.max(120, area.height - 16)}px`)
      el.style.top = `${Math.max(area.top + 8 - top, Math.min(0, area.bottom - 8 - (top + el.offsetHeight)))}px`
    }
    place()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(place)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  return (
    <div ref={ref} className={`aiws-popover aiws-popover-side aiws-tool-column ${className}`} role="dialog" aria-label={label} data-testid={testId} onPointerDown={(event) => event.stopPropagation()}>
      {children}
    </div>
  )
}

function ColumnTool({ icon: Icon, label, title, pressed, testId, columnTestId, columnLabel, columnClass, onPress, children }: {
  icon: LucideIcon
  label: string
  title: string
  pressed: boolean
  testId: string
  columnTestId: string
  columnLabel?: string
  columnClass?: string
  onPress?: () => void
  children: (close: () => void) => ReactNode
}) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool has-column" aria-pressed={pressed} aria-label={label} title={title} aria-haspopup="dialog" aria-expanded={open}
        data-testid={testId} onClick={() => { onPress?.(); toggle() }}>
        <Icon size={20} />
      </button>
      {open && <SideColumn label={columnLabel ?? label} testId={columnTestId} className={columnClass}>{children(close)}</SideColumn>}
    </span>
  )
}

function ColumnItem({ icon: Icon, label, title, hint, pressed, testId, ai, onClick }: {
  icon: LucideIcon; label: string; title?: string; hint?: string; pressed: boolean; testId: string; ai?: boolean; onClick: () => void
}) {
  return (
    <button type="button" className={`aiws-column-item${ai ? ' is-ai' : ''}`} aria-pressed={pressed} title={title ?? label} data-testid={testId} onClick={onClick}>
      <Icon size={18} aria-hidden="true" /><span>{label}</span>{hint && <kbd>{hint}</kbd>}
    </button>
  )
}

/** Undo / redo of this window's edits (rich text included); also in the phone's one toolbar on a flow page (§16.5). */
export function HistoryButtons({ size = 20 }: { size?: number }) {
  const store = useStore()
  const { t } = useAiwsText()
  const undo = useSyncExternalStore(store.undo.subscribe, store.undo.snapshot)
  const top = undo.undo.at(-1)
  const undoLabel = top && (top.kind === 'commit' || top.kind === 'pending' || top.kind === 'resubmit') ? top.label : t('aiworkspace.canvas.tool.richTextEdit')
  const undoTitle = !top ? t('aiworkspace.canvas.tool.nothingToUndo')
    : t(top.kind === 'pending' ? 'aiworkspace.canvas.tool.undoPendingTip' : 'aiworkspace.canvas.tool.undoTip', { label: undoLabel })
  return (
    <>
      <button type="button" className="aiws-tool" data-testid="aiws-undo" data-count={undo.undo.length} aria-label={t('aiworkspace.canvas.tool.undo')} title={undoTitle} disabled={undo.undo.length === 0 || undo.busy} onClick={() => { void store.undo.undo() }}>
        <Undo2 size={size} /><span className="aiws-sr-only">{t('aiworkspace.canvas.tool.undoCount', { count: undo.undo.length })}</span>
      </button>
      <button type="button" className="aiws-tool" data-testid="aiws-redo" data-count={undo.redo.length} aria-label={t('aiworkspace.canvas.tool.redo')} title={t('aiworkspace.canvas.tool.redoTip')} disabled={undo.redo.length === 0 || undo.busy} onClick={() => { void store.undo.redo() }}>
        <Redo2 size={size} /><span className="aiws-sr-only">{t('aiworkspace.canvas.tool.redoCount', { count: undo.redo.length })}</span>
      </button>
    </>
  )
}

/** `connector`: press and drag draws a line (标准对象的交互改进 §6.1), one line at a time; `pen` writes and `eraser` erases
 * whole strokes until another tool is picked (手写笔 TODO). */
export type PointerTool = 'select' | 'hand' | 'connector' | 'pen' | 'eraser'

export function ObjectToolbar({ isFree, tool, onTool, placingKey, insertReason, noteReason, onPick, onPickFile, onOpenCatalog, collapsed, onCollapsed, runMode, objects: objectIds = TOOL_OBJECTS }: {
  isFree: boolean
  /** The canvas's design / run switch (扩展控件 P2, Q27), in the pointer group (Q3). */
  runMode?: RunMode
  /** The object group, by `entryId` (`table` and `shape` open their second column). */
  objects?: string[]
  tool: PointerTool
  onTool: (tool: PointerTool) => void
  /** The entry being placed (highlighted), if any. */
  placingKey: string | null
  /** Why inserting is not possible now (mode, capability, read-only), or null. */
  insertReason: string | null
  /** The same for notes, which need only `comment` (公开链接便签): a commenter is offered the note alone. */
  noteReason: string | null
  onPick: (entry: CatalogEntry) => void
  onPickFile: (entry: CatalogEntry, file: File) => void
  onOpenCatalog: (tab: 'all' | 'extension', key?: string) => void
  collapsed: boolean
  onCollapsed: (collapsed: boolean) => void
}) {
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  useSyncExternalStore(blockRegistry.subscribe, blockRegistry.snapshot)
  const pinned = useUserState<string[]>('ui:pinned-defs') ?? []
  const pickFile = useFilePicker((message) => store.notify('error', message))
  const barRef = useRef<HTMLDivElement>(null)
  /** How many of the object group's last buttons give way to a low window. */
  const [hidden, setHidden] = useState(0)
  const [pointer, setPointer] = useState<'select' | 'hand'>('select')
  if ((tool === 'select' || tool === 'hand') && tool !== pointer) setPointer(tool)
  const canInsert = insertReason === null
  const byId = new Map(registryEntries().map((entry) => [entryId(entry), entry]))
  const offered = (id: string): CatalogEntry | undefined => {
    const entry = byId.get(id)
    if (!entry || (!isFree && entry.definition.type === 'connector')) return undefined
    return canInsert || (noteReason === null && entry.definition.type === 'note') ? entry : undefined
  }
  const extensions = canInsert ? extensionEntries(store) : []
  const pinnedEntries = extensions.filter((entry) => pinned.includes(entry.defEntity?.entity_id ?? ''))
  const pick = (entry: CatalogEntry) => {
    if (entry.catalog.needs === 'file') { void pickFile.file().then((file) => { if (file) onPickFile(entry, file) }); return }
    if (entry.catalog.needs !== 'none') { onOpenCatalog(entry.group === 'extension' ? 'extension' : 'all', entry.key); return }
    onPick(entry)
  }
  const tipOf = (entry: CatalogEntry) => entry.group === 'ai' ? t('aiworkspace.canvas.tool.wishTip', { name: entry.title })
    : entry.definition.type === 'connector' ? t('aiworkspace.canvas.tool.connectorTip', { name: entry.title })
    : isFree ? t('aiworkspace.canvas.tool.placeTip', { name: entry.title }) : t('aiworkspace.canvas.insertNamed', { name: entry.title })
  const item = (entry: CatalogEntry, close: () => void) => (
    <ColumnItem key={entry.key} icon={iconOf(entry)} label={entry.title} title={tipOf(entry)} pressed={placingKey === entry.key} testId={`aiws-tool-insert-${entryId(entry)}`} onClick={() => { close(); pick(entry) }} />
  )
  const objectTool = (id: string): ReactNode => {
    const entry = offered(id)
    if (id === 'table') {
      const views = TABLE_GROUP.map(offered).filter((e): e is CatalogEntry => e !== undefined)
      const standard = STANDARD_TABLES.map(offered).filter((e): e is CatalogEntry => e !== undefined)
      if (views.length + standard.length === 0) return null
      return (
        <ColumnTool key="table" icon={Table} label={t('aiworkspace.canvas.tool.tables')} title={t('aiworkspace.canvas.tool.tablesTip')} testId="aiws-tool-table" columnTestId="aiws-tool-table-column"
          pressed={[...views, ...standard].some((e) => e.key === placingKey)}>
          {(close) => <>
            {views.map((e) => item(e, close))}
            {standard.length > 0 && <div className="aiws-column-title">{t('aiworkspace.canvas.tool.standardTables')}</div>}
            {standard.map((e) => item(e, close))}
          </>}
        </ColumnTool>
      )
    }
    if (!entry) return null
    if (id === 'shape') {
      return (
        <ColumnTool key="shape" icon={Shapes} label={entry.title} title={isFree ? t('aiworkspace.canvas.tool.shapeTip', { name: entry.title }) : t('aiworkspace.canvas.insertNamed', { name: entry.title })}
          testId={`aiws-tool-insert-${entryId(entry)}`} columnTestId="aiws-tool-shape-picker" columnLabel={t('aiworkspace.canvas.tool.pickShape')} columnClass="aiws-library-popover"
          pressed={placingKey === entry.key || placingKey === `${entry.key}:library`}>
          {(close) => (
            <ShapeLibraryPanel
              basic={<ShapePicker onPick={(kind) => { close(); onPick({ ...entry, config: { ...(entry.config ?? {}), shape: kind } }) }} />}
              onPick={(shape) => { close(); const { size, config } = libraryShapeConfig(shape); onPick({ ...entry, key: `${entry.key}:library`, title: shape.title, size, config }) }} />
          )}
        </ColumnTool>
      )
    }
    const Icon = iconOf(entry)
    return (
      <button key={entry.key} type="button" className={`aiws-tool${entry.group === 'ai' ? ' is-ai' : ''}`} aria-pressed={placingKey === entry.key} aria-label={entry.title} title={tipOf(entry)}
        data-testid={`aiws-tool-insert-${entryId(entry)}`} onClick={() => pick(entry)}>
        <Icon size={20} />
      </button>
    )
  }
  const objects = objectIds.map(objectTool).filter((node) => node !== null)
  const pinnedTools = pinnedEntries.map((entry) => (
    <button key={entry.key} type="button" className="aiws-tool" aria-label={entry.title} title={t('aiworkspace.canvas.tool.extensionTip', { name: entry.title })} data-testid={`aiws-tool-def-${entry.defEntity?.entity_id}`} onClick={() => onOpenCatalog('extension', entry.key)}>
      <span className="aiws-tool-letter">{entry.title.slice(0, 1)}</span>
    </button>
  ))
  // too low a window (UI 框架 PRD §4.3): the object group gives way from the bottom up — what goes is in "insert…" —
  // while the pointer and history groups stay
  const movable = objects.length + pinnedTools.length
  const shown = Math.max(0, movable - hidden)
  useLayoutEffect(() => {
    const bar = barRef.current
    const area = bar?.parentElement
    if (!bar || !area || collapsed) return
    const fit = () => {
      const step = (bar.querySelector<HTMLElement>('.aiws-tool-group > .aiws-tool, .aiws-tool-group > .aiws-anchor')?.offsetHeight ?? 36) + 2
      const room = area.clientHeight - (bar.offsetHeight - bar.clientHeight)
      const next = Math.max(0, Math.min(movable, Math.ceil((bar.scrollHeight + hidden * step - room) / step)))
      if (next !== hidden) setHidden(next)
    }
    fit()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(fit)
    observer.observe(area)
    return () => observer.disconnect()
  }, [hidden, movable, collapsed])
  const runToggle = runMode && !runMode.locked ? <RunModeToggle runMode={runMode} /> : null
  const PointerIcon = pointer === 'hand' ? Hand : MousePointer2
  return (
    <div ref={barRef} className={`aiws-panel aiws-object-toolbar${collapsed ? ' is-collapsed' : ''}`} role="toolbar" aria-orientation="vertical" aria-label={t('aiworkspace.canvas.tool.objectTools')} data-testid="aiws-object-toolbar"
      data-hidden={hidden > 0 ? hidden : undefined}>
      {!collapsed && (
        <>
          {!isFree && runToggle && <div className="aiws-tool-group">{runToggle}</div>}
          {isFree && (
            <div className="aiws-tool-group">
              <ColumnTool icon={PointerIcon} label={t('aiworkspace.canvas.tool.pointer')} title={t('aiworkspace.canvas.tool.pointerTip')} testId="aiws-tool-pointer" columnTestId="aiws-tool-pointer-column"
                pressed={(tool === 'select' || tool === 'hand') && !placingKey} onPress={() => { if (tool !== pointer || placingKey) onTool(pointer) }}>
                {(close) => <>
                  <ColumnItem icon={MousePointer2} label={t('aiworkspace.canvas.tool.select')} title={t('aiworkspace.canvas.tool.selectTip')} hint="V" pressed={tool === 'select' && !placingKey} testId="aiws-tool-select" onClick={() => { close(); onTool('select') }} />
                  <ColumnItem icon={Hand} label={t('aiworkspace.canvas.tool.hand')} title={t('aiworkspace.canvas.tool.handTip')} hint="H" pressed={tool === 'hand' && !placingKey} testId="aiws-tool-hand" onClick={() => { close(); onTool('hand') }} />
                </>}
              </ColumnTool>
              {runToggle}
              {canInsert && (
                <button type="button" className="aiws-tool" aria-pressed={tool === 'pen' || tool === 'eraser'} aria-label={t('aiworkspace.canvas.ink.pen')} title={t('aiworkspace.canvas.tool.penTip')} data-testid="aiws-tool-pen"
                  onClick={() => onTool(tool === 'pen' || tool === 'eraser' ? 'select' : 'pen')}><PenLine size={20} /></button>
              )}
            </div>
          )}
          {(objects.length > 0 || canInsert) && (
            <div className="aiws-tool-group">
              {objects.slice(0, shown)}
              {canInsert && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.canvas.tool.insert')} title={t('aiworkspace.canvas.tool.insertTip')} data-testid="aiws-tool-catalog" onClick={() => onOpenCatalog('all')}><Plus size={20} /></button>}
              {pinnedTools.slice(0, Math.max(0, shown - objects.length))}
            </div>
          )}
          <div className="aiws-tool-group aiws-tool-group-history">
            <HistoryButtons />
          </div>
        </>
      )}
      <button type="button" className="aiws-tool aiws-tool-collapse" aria-label={t(collapsed ? 'aiworkspace.canvas.tool.expand' : 'aiworkspace.canvas.tool.collapse')} title={t(collapsed ? 'aiworkspace.canvas.tool.expand' : 'aiworkspace.canvas.tool.collapse')} data-testid="aiws-tool-collapse" onClick={() => onCollapsed(!collapsed)}>
        {collapsed ? <ChevronsDown size={16} /> : <ChevronsUp size={16} />}
      </button>
      {!canInsert && insertReason && !collapsed && <span className="aiws-sr-only" data-testid="aiws-insert-unavailable">{insertReason}</span>}
    </div>
  )
}
