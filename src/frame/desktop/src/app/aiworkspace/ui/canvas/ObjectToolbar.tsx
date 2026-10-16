/* The vertical object toolbar (UI improvement §7): select / hand, the standard objects with the wish
 * (AI) entry, the workspace's pinned extensions, "all objects", and undo / redo at the bottom. Picking
 * an object on a free Surface starts a one-shot placement; on a flow page it appends. */

import { useRef, useSyncExternalStore, type ReactNode } from 'react'
import { ChevronsLeft, ChevronsRight, Frame, Hand, Image, LetterText, Library, MousePointer2, Plus, Puzzle, Redo2, Shapes, Sparkles, Spline, StickyNote, Table, Type, Undo2, type LucideIcon } from 'lucide-react'
import { useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { blockRegistry } from '../blocks/registry'
import { ShapePicker } from '../blocks/shapeBlock'
import { PopoverPanel, usePopover } from '../shell/popover'
import { entryId, extensionEntries, registryEntries, type CatalogEntry } from './catalog'
import { ShapeLibraryPanel } from './ShapeLibraryPanel'
import { libraryShapeConfig } from './stencilLibrary'

const ICONS: Record<string, LucideIcon> = { richtext: Type, 'shape-text': LetterText, note: StickyNote, shape: Shapes, connector: Spline, frame: Frame, table: Table, asset: Image, wish: Sparkles }

/** The shape library (§2.4): a picked shape is placed like any object. */
function LibraryTool({ entry, onPick }: { entry: CatalogEntry; onPick: (entry: CatalogEntry) => void }) {
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool" aria-pressed={open} aria-label={t('aiworkspace.canvas.library.title')} title={t('aiworkspace.canvas.tool.libraryTip')} aria-haspopup="dialog" aria-expanded={open}
        data-testid="aiws-tool-library" onClick={toggle}>
        <Library size={20} />
      </button>
      {open && (
        <PopoverPanel label={t('aiworkspace.canvas.library.title')} testId="aiws-tool-library-panel" className="aiws-popover-side aiws-library-popover">
          <ShapeLibraryPanel onPick={(shape) => { close(); const { size, config } = libraryShapeConfig(shape); onPick({ ...entry, key: `${entry.key}:library`, title: shape.title, size, config }) }} />
        </PopoverPanel>
      )}
    </span>
  )
}

/** The shape tool: a grouped picker of the basic shapes (draw.io 导入 TODO §2.1), the pick is placed like any object. */
function ShapeTool({ entry, pressed, title, onPick, extra }: { entry: CatalogEntry; pressed: boolean; title: string; onPick: (entry: CatalogEntry) => void; extra?: ReactNode }) {
  const { t } = useAiwsText()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool" aria-pressed={pressed || open} aria-label={entry.title} title={title} aria-haspopup="dialog" aria-expanded={open}
        data-testid={`aiws-tool-insert-${entryId(entry)}`} onClick={toggle}>
        <Shapes size={20} />
      </button>
      {open && (
        <PopoverPanel label={t('aiworkspace.canvas.tool.pickShape')} testId="aiws-tool-shape-picker" className="aiws-popover-side">
          <ShapePicker onPick={(kind) => { close(); onPick({ ...entry, config: { ...(entry.config ?? {}), shape: kind } }) }} extra={extra} />
        </PopoverPanel>
      )}
    </span>
  )
}

/** `connector`: press and drag draws a line (标准对象的交互改进 §6.1), one line at a time. */
export type PointerTool = 'select' | 'hand' | 'connector'

export function ObjectToolbar({ isFree, tool, onTool, placingKey, insertReason, noteReason, onPick, onPickFile, onOpenCatalog, collapsed, onCollapsed }: {
  isFree: boolean
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
  const undo = useSyncExternalStore(store.undo.subscribe, store.undo.snapshot)
  const pinned = useUserState<string[]>('ui:pinned-defs') ?? []
  const fileRef = useRef<HTMLInputElement>(null)
  const fileEntry = useRef<CatalogEntry | null>(null)
  // a flow page shows no connectors (连接线实现方案 §8.3)
  const standard = registryEntries().filter((entry) => entry.catalog.standard && (isFree || entry.definition.type !== 'connector'))
  const extensions = extensionEntries(store)
  const pinnedEntries = extensions.filter((entry) => pinned.includes(entry.defEntity?.entity_id ?? ''))
  const top = undo.undo.at(-1)
  const undoLabel = top && (top.kind === 'commit' || top.kind === 'pending' || top.kind === 'resubmit') ? top.label : t('aiworkspace.canvas.tool.richTextEdit')
  const undoTitle = !top ? t('aiworkspace.canvas.tool.nothingToUndo')
    : t(top.kind === 'pending' ? 'aiworkspace.canvas.tool.undoPendingTip' : 'aiworkspace.canvas.tool.undoTip', { label: undoLabel })
  const pick = (entry: CatalogEntry) => {
    if (entry.catalog.needs === 'file') { fileEntry.current = entry; fileRef.current?.click(); return }
    if (entry.catalog.needs !== 'none') { onOpenCatalog(entry.group === 'extension' ? 'extension' : 'all', entry.key); return }
    onPick(entry)
  }
  const canInsert = insertReason === null
  const offered = canInsert ? standard : noteReason === null ? standard.filter((entry) => entry.definition.type === 'note') : []
  return (
    <div className={`aiws-panel aiws-object-toolbar${collapsed ? ' is-collapsed' : ''}`} role="toolbar" aria-orientation="vertical" aria-label={t('aiworkspace.canvas.tool.objectTools')} data-testid="aiws-object-toolbar">
      {!collapsed && (
        <>
          {isFree && (
            <div className="aiws-tool-group">
              <button type="button" className="aiws-tool" aria-pressed={tool === 'select' && !placingKey} aria-label={t('aiworkspace.canvas.tool.select')} title={t('aiworkspace.canvas.tool.selectTip')} data-testid="aiws-tool-select" onClick={() => onTool('select')}><MousePointer2 size={20} /></button>
              <button type="button" className="aiws-tool" aria-pressed={tool === 'hand' && !placingKey} aria-label={t('aiworkspace.canvas.tool.hand')} title={t('aiworkspace.canvas.tool.handTip')} data-testid="aiws-tool-hand" onClick={() => onTool('hand')}><Hand size={20} /></button>
            </div>
          )}
          {offered.length > 0 && (
            <div className="aiws-tool-group">
              {offered.map((entry) => {
                const Icon = ICONS[entryId(entry)] ?? Plus
                const ai = entry.group === 'ai'
                if (entryId(entry) === 'shape') return [
                  <ShapeTool key={entry.key} entry={entry} pressed={placingKey === entry.key} title={isFree ? t('aiworkspace.canvas.tool.shapeTip', { name: entry.title }) : t('aiworkspace.canvas.insertNamed', { name: entry.title })} onPick={onPick} />,
                  ...(canInsert ? [<LibraryTool key="library" entry={entry} onPick={onPick} />] : []),
                ]
                return (
                  <button key={entry.key} type="button" className={`aiws-tool${ai ? ' is-ai' : ''}`} aria-pressed={placingKey === entry.key} aria-label={entry.title}
                    title={ai ? t('aiworkspace.canvas.tool.wishTip', { name: entry.title }) : entry.definition.type === 'connector' ? t('aiworkspace.canvas.tool.connectorTip', { name: entry.title }) : isFree ? t('aiworkspace.canvas.tool.placeTip', { name: entry.title }) : t('aiworkspace.canvas.insertNamed', { name: entry.title })}
                    data-testid={`aiws-tool-insert-${entryId(entry)}`} onClick={() => pick(entry)}>
                    <Icon size={20} />
                  </button>
                )
              })}
            </div>
          )}
          {canInsert && extensions.length > 0 && (
            <div className="aiws-tool-group">
              {pinnedEntries.map((entry) => (
                <button key={entry.key} type="button" className="aiws-tool" aria-label={entry.title} title={t('aiworkspace.canvas.tool.extensionTip', { name: entry.title })} data-testid={`aiws-tool-def-${entry.defEntity?.entity_id}`} onClick={() => onOpenCatalog('extension', entry.key)}>
                  <span className="aiws-tool-letter">{entry.title.slice(0, 1)}</span>
                </button>
              ))}
              <button type="button" className="aiws-tool" aria-label={t('aiworkspace.canvas.tool.extensions')} title={t('aiworkspace.canvas.tool.extensionsTip', { count: extensions.length })} data-testid="aiws-tool-extensions" onClick={() => onOpenCatalog('extension')}><Puzzle size={20} /></button>
            </div>
          )}
          {canInsert ? (
            <div className="aiws-tool-group">
              <button type="button" className="aiws-tool" aria-label={t('aiworkspace.canvas.tool.allObjects')} title={t('aiworkspace.canvas.tool.allObjectsTip')} data-testid="aiws-tool-catalog" onClick={() => onOpenCatalog('all')}><Plus size={20} /></button>
            </div>
          ) : null}
          <div className="aiws-tool-group aiws-tool-group-history">
            <button type="button" className="aiws-tool" data-testid="aiws-undo" data-count={undo.undo.length} aria-label={t('aiworkspace.canvas.tool.undo')} title={undoTitle} disabled={undo.undo.length === 0 || undo.busy} onClick={() => { void store.undo.undo() }}>
              <Undo2 size={20} /><span className="aiws-sr-only">{t('aiworkspace.canvas.tool.undoCount', { count: undo.undo.length })}</span>
            </button>
            <button type="button" className="aiws-tool" data-testid="aiws-redo" data-count={undo.redo.length} aria-label={t('aiworkspace.canvas.tool.redo')} title={t('aiworkspace.canvas.tool.redoTip')} disabled={undo.redo.length === 0 || undo.busy} onClick={() => { void store.undo.redo() }}>
              <Redo2 size={20} /><span className="aiws-sr-only">{t('aiworkspace.canvas.tool.redoCount', { count: undo.redo.length })}</span>
            </button>
          </div>
        </>
      )}
      <button type="button" className="aiws-tool aiws-tool-collapse" aria-label={t(collapsed ? 'aiworkspace.canvas.tool.expand' : 'aiworkspace.canvas.tool.collapse')} title={t(collapsed ? 'aiworkspace.canvas.tool.expand' : 'aiworkspace.canvas.tool.collapse')} data-testid="aiws-tool-collapse" onClick={() => onCollapsed(!collapsed)}>
        {collapsed ? <ChevronsRight size={16} /> : <ChevronsLeft size={16} />}
      </button>
      <input ref={fileRef} type="file" hidden aria-hidden="true" tabIndex={-1} data-testid="aiws-tool-file" onChange={(event) => {
        const file = event.target.files?.[0]
        const entry = fileEntry.current
        event.target.value = ''
        if (file && entry) onPickFile(entry, file)
      }} />
      {!canInsert && insertReason && !collapsed && <span className="aiws-sr-only" data-testid="aiws-insert-unavailable">{insertReason}</span>}
    </div>
  )
}
