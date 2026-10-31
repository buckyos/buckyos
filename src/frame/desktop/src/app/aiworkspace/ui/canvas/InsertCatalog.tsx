/* The insert catalog dialog (UI improvement §6.3): search, categories, an entry's description and type
 * details, and the step the entry needs before it can be inserted (existing data, a file, a binding of an
 * extension). One catalog serves the main menu, the object toolbar's "+" and the blank-spot context menu.
 * "Existing data" only creates a view: the new Block shares the data, its view configuration is its own. */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import FocusTrap from '@mui/material/Unstable_TrapFocus'
import { ArrowDownToLine, Check, Database, FileUp, FlaskConical, FolderUp, Frame, Grid2X2, Image, Info, Layers3, LetterText, MousePointer2, Plus, Puzzle, Search, Shapes, Sparkles, Spline, StickyNote, Table, Type, X, Network, type LucideIcon } from 'lucide-react'
import type { BlockDefPayload, EntityEnvelope, Json } from '../../api/types'
import type { ReadOk } from '../../api/session'
import { useLoad, useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { blockRegistry, CATALOG_GROUP_LABEL, type CatalogGroup } from '../blocks/registry'
import { defaultRendererFor } from '../blocks/ops'
import { useOverlayMounted } from '../shell/popover'
import { ShapePicker } from '../blocks/shapeBlock'
import { svgShapeFromDocument } from '@drawio/svg'
import type { ShapeKind } from '@drawio/shapes'
import { allEntries, dataFor, entryId, entrySize, type CatalogEntry, type InsertRequest } from './catalog'
import { InstanceForm } from '../extensions/InstanceForm'
import { configProblems, missingBindings } from '../extensions/controls'
import { useShell } from '../shell/shellContext'

export type CatalogTab = 'all' | CatalogGroup | 'existing'
const TABS: CatalogTab[] = ['all', 'text', 'data', 'layout', 'ai', 'sample', 'extension', 'existing']
const TAB_ICONS: Record<CatalogTab, LucideIcon> = { all: Grid2X2, text: Type, data: Database, layout: Shapes, ai: Sparkles, sample: FlaskConical, extension: Puzzle, existing: Layers3 }
const ENTRY_ICONS: Record<string, LucideIcon> = { richtext: Type, 'shape-text': LetterText, note: StickyNote, shape: Shapes, connector: Spline, frame: Frame, mindmap: Network, table: Table, asset: Image, wish: Sparkles }

function EntryIcon({ entry }: { entry: CatalogEntry }) {
  const Icon = ENTRY_ICONS[entryId(entry)] ?? TAB_ICONS[entry.group]
  return <span className="aiws-catalog-entry-icon" data-group={entry.group}><Icon size={20} aria-hidden="true" /></span>
}

export function InsertCatalog({ initialTab = 'all', initialKey, canPlace, onClose, onInsert, onImport, onImportAssets, onImportTable }: {
  initialTab?: CatalogTab
  initialKey?: string
  /** "Import draw.io…" (draw.io 导入 TODO §3), when importing is possible here. */
  onImport?: () => void
  /** The table entry's "From a file…" (表格数据导入 TODO §5.1): straight to the file picker and the wizard. */
  onImportTable?: () => void
  onImportAssets?: (files: File[], folder: boolean, signal: AbortSignal) => Promise<void>
  /** A free Surface: the entry can also be placed by a click on the canvas. */
  canPlace: boolean
  onClose: () => void
  onInsert: (request: InsertRequest, how: 'center' | 'place') => void
}) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  useOutlineVersion()
  useOverlayMounted()
  useSyncExternalStore(blockRegistry.subscribe, blockRegistry.snapshot)
  const tabLabel = (item: CatalogTab) => (item === 'all' ? t('aiworkspace.canvas.catalog.all') : item === 'existing' ? t('aiworkspace.canvas.catalog.existing') : CATALOG_GROUP_LABEL[item])
  const [tab, setTab] = useState<CatalogTab>(initialTab)
  const [query, setQuery] = useState('')
  const entries = allEntries(store)
  const [chosenKey, setChosenKey] = useState<string | null>(initialKey ?? null)
  const q = query.trim().toLowerCase()
  const shown = tab === 'existing' ? [] : entries.filter((entry) => (tab === 'all' || entry.group === tab)
    && (!q || entry.title.toLowerCase().includes(q) || entry.catalog.description.toLowerCase().includes(q) || entry.definition.type.toLowerCase().includes(q)))
  const chosen = shown.find((entry) => entry.key === chosenKey) ?? shown[0]
  const dialogRef = useRef<HTMLDivElement>(null)
  const filesRef = useRef<HTMLInputElement>(null)
  const folderRef = useRef<HTMLInputElement>(null)
  const importController = useRef<AbortController | null>(null)
  const [importing, setImporting] = useState(false)
  useEffect(() => () => importController.current?.abort(), [])
  const importAssets = async (files: File[], folder: boolean) => {
    if (!onImportAssets || importing) return
    const controller = new AbortController()
    importController.current = controller
    setImporting(true)
    try { await onImportAssets(files, folder, controller.signal) }
    finally { importController.current = null; setImporting(false) }
  }
  return (
    <div className="aiws-modal-backdrop aiws-catalog-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <FocusTrap open>
        <div ref={dialogRef} tabIndex={-1} className="aiws-dialog aiws-catalog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.canvas.catalog.title')} data-testid="aiws-catalog"
          onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }}>
          <div className="aiws-catalog-head">
            <div>
              <h2>{t('aiworkspace.canvas.catalog.title')}</h2>
              <p>{t('aiworkspace.canvas.catalog.subtitle')}</p>
            </div>
            <button type="button" className="aiws-catalog-close" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} onClick={onClose}><X size={20} aria-hidden="true" /></button>
          </div>
          <div className="aiws-catalog-toolbar">
            <div className="aiws-catalog-search-wrap">
              <Search size={17} aria-hidden="true" />
              <input className="aiws-catalog-search" type="search" aria-label={t('aiworkspace.canvas.catalog.search')} placeholder={t('aiworkspace.canvas.catalog.searchPlaceholder')} autoFocus value={query} data-testid="aiws-catalog-search"
                onChange={(event) => { setQuery(event.target.value); if (tab === 'existing') setTab('all') }} />
            </div>
            {(onImport || onImportAssets) && <div className="aiws-catalog-imports" aria-busy={importing}>
              {onImport && <button type="button" data-testid="aiws-catalog-import-drawio" disabled={importing} onClick={onImport}><ArrowDownToLine size={16} aria-hidden="true" />{t('aiworkspace.canvas.catalog.importDrawio')}</button>}
              {onImportAssets && <>
                <button type="button" disabled={importing} onClick={() => filesRef.current?.click()}><FileUp size={16} aria-hidden="true" />{t('aiworkspace.data.tree.importFiles')}</button>
                <input ref={filesRef} type="file" hidden multiple disabled={importing} data-testid="aiws-catalog-import-files" onChange={(event) => { const files = Array.from(event.target.files ?? []); event.target.value = ''; if (files.length) void importAssets(files, false) }} />
                {'webkitdirectory' in document.createElement('input') && <>
                  <button type="button" disabled={importing} onClick={() => folderRef.current?.click()}><FolderUp size={16} aria-hidden="true" />{t('aiworkspace.data.tree.importFolder')}</button>
                  <input ref={folderRef} type="file" hidden multiple disabled={importing} {...{ webkitdirectory: '' }} data-testid="aiws-catalog-import-folder" onChange={(event) => { const files = Array.from(event.target.files ?? []); event.target.value = ''; if (files.length) void importAssets(files, true) }} />
                </>}
                {importing && <button type="button" onClick={() => importController.current?.abort()}>{t('aiworkspace.common.cancel')}</button>}
              </>}
            </div>}
          </div>
          <div className="aiws-catalog-body">
            <nav className="aiws-catalog-tabs" aria-label={t('aiworkspace.canvas.catalog.categories')}>
              <span className="aiws-catalog-section-label">{t('aiworkspace.canvas.catalog.categories')}</span>
              {TABS.map((item) => {
                const Icon = TAB_ICONS[item]
                return <button key={item} type="button" aria-pressed={tab === item} data-testid={`aiws-catalog-tab-${item}`} onClick={() => { setTab(item); setChosenKey(null) }}><Icon size={17} aria-hidden="true" /><span>{tabLabel(item)}</span></button>
              })}
            </nav>
            {tab === 'existing' ? <ExistingData onInsert={onInsert} canPlace={canPlace} /> : (
              <>
                <div className="aiws-catalog-browser">
                  <div className="aiws-catalog-section-label">{tabLabel(tab)}<span className="aiws-catalog-count">{shown.length}</span></div>
                  <ul className="aiws-catalog-list" aria-label={t('aiworkspace.canvas.catalog.objects')} data-testid="aiws-catalog-list">
                    {shown.length === 0 && <li className="aiws-catalog-empty"><Search size={24} aria-hidden="true" /><span>{t(tab === 'extension' && !q ? 'aiworkspace.canvas.catalog.noExtensions' : 'aiworkspace.canvas.catalog.noMatch')}</span>
                      {tab === 'extension' && !q && <button type="button" className="aiws-link" data-testid="aiws-catalog-manage-controls" onClick={() => { onClose(); shell.openDialog({ kind: 'extensions' }) }}>{t('aiworkspace.ext.manage')}</button>}</li>}
                    {tab === 'extension' && shown.length > 0 && <li><button type="button" className="aiws-link" data-testid="aiws-catalog-manage-controls" onClick={() => { onClose(); shell.openDialog({ kind: 'extensions' }) }}>{t('aiworkspace.ext.manage')}</button></li>}
                    {shown.map((entry) => (
                      <li key={entry.key}>
                        <button type="button" data-testid={`aiws-catalog-entry-${entry.key.replace(':', '-')}`} aria-pressed={chosen?.key === entry.key} className={chosen?.key === entry.key ? 'is-active' : undefined}
                          onClick={() => setChosenKey(entry.key)} onDoubleClick={() => { if (entry.catalog.needs === 'none') onInsert({ entry }, 'center') }}>
                          <EntryIcon entry={entry} />
                          <span className="aiws-catalog-entry-copy"><span className="aiws-catalog-entry-name">{entry.title}</span><span className="aiws-catalog-entry-description">{entry.catalog.description}</span></span>
                          {chosen?.key === entry.key && <Check className="aiws-catalog-entry-check" size={15} aria-hidden="true" />}
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
                {chosen ? <EntryDetail key={chosen.key} entry={chosen} canPlace={canPlace} onInsert={onInsert} onImportTable={onImportTable} /> : <div className="aiws-catalog-detail aiws-catalog-empty"><MousePointer2 size={28} aria-hidden="true" /><span>{t('aiworkspace.canvas.catalog.chooseEntry')}</span></div>}
              </>
            )}
          </div>
          {onImportAssets && <div className="aiws-catalog-footnote"><Info size={15} aria-hidden="true" /><p>{t('aiworkspace.data.tree.folderLimit')}</p></div>}
        </div>
      </FocusTrap>
    </div>
  )
}

function EntryDetail({ entry, canPlace, onInsert, onImportTable }: { entry: CatalogEntry; canPlace: boolean; onInsert: (request: InsertRequest, how: 'center' | 'place') => void; onImportTable?: () => void }) {
  const store = useStore()
  const { t, list } = useAiwsText()
  const [title, setTitle] = useState('')
  const [sourceId, setSourceId] = useState('')
  const [file, setFile] = useState<File | null>(null)
  const [kind, setKind] = useState<ShapeKind>('rect')
  const [choices, setChoices] = useState<Record<string, string>>({})
  const [svgText, setSvgText] = useState('')
  const svg = entryId(entry) === 'shape-svg' && svgText.trim() ? svgShapeFromDocument(svgText) : null
  const pinned = useUserState<string[]>('ui:pinned-defs') ?? []
  const defId = entry.defEntity?.entity_id ?? ''
  const loadDef = useCallback(async () => (defId ? store.readBatched<{ payload: BlockDefPayload }>(defId) : null), [store, defId])
  const def = useLoad<ReadOk<{ payload: BlockDefPayload }> | null>(loadDef).data?.content.payload
  // a control instance: its property overrides and named bindings (扩展控件 P0, B8)
  const [instanceConfig, setInstanceConfig] = useState<Record<string, Json>>({})
  const [instanceBindings, setInstanceBindings] = useState<Record<string, string>>({})
  const configIssues = entry.defEntity ? configProblems(def, instanceConfig) : []
  const unbound = entry.defEntity ? missingBindings(def, instanceBindings) : []
  const accepts = entry.defEntity ? (def?.accepts ?? entry.definition.accepts) : entry.definition.accepts
  // a composite shows no data of its own: its Blocks do (扩展控件 P1)
  const needsData = entry.catalog.needs === 'data' || (entry.defEntity !== undefined && def !== undefined && def.kind !== 'composite' && def.allow_no_source !== true)
  const optionalData = entry.defEntity !== undefined && def?.allow_no_source === true && accepts.length > 0
  const data = needsData || optionalData ? dataFor(store, accepts) : []
  const needsFile = entry.catalog.needs === 'file'
  const ready = (!needsData || sourceId !== '') && (!needsFile || file !== null) && (!entry.defEntity || def !== undefined) && (entryId(entry) !== 'shape-svg' || (svg !== null && svg.svg.body.length <= 48 * 1024))
    && configIssues.length === 0 && unbound.length === 0
  const svgSize = svg ? (() => { const k = Math.min(1, 320 / Math.max(svg.w, svg.h, 1)); return { w: Math.max(24, Math.round(svg.w * k)), h: Math.max(24, Math.round(svg.h * k)) } })() : null
  const request = (): InsertRequest => svg && svgSize ? ({ entry: { ...entry, size: svgSize }, title: title.trim() || undefined, config: { shape: 'svg', svg: svg.svg as unknown as Json, fill: svg.fill, stroke: svg.stroke ?? 'none', stroke_width: 1 } }) : ({
    entry, title: title.trim() || undefined, sourceId: sourceId || undefined, file: file ?? undefined,
    config: entry.defEntity ? { def_id: entry.defEntity.entity_id, ...instanceConfig } : entryId(entry) === 'shape' ? { shape: kind } : Object.keys(choices).length ? choices : undefined,
    bindings: entry.defEntity && Object.keys(instanceBindings).length > 0 ? instanceBindings : undefined,
  })
  const missing = needsData && sourceId === '' ? t('aiworkspace.canvas.catalog.missingData') : needsFile && !file ? t('aiworkspace.canvas.catalog.missingFile')
    : unbound.length > 0 ? t('aiworkspace.ext.form.missingBindings', { names: list(unbound) }) : configIssues.length > 0 ? t('aiworkspace.ext.form.invalidConfig', { issues: list(configIssues) }) : null
  return (
    <div className="aiws-catalog-detail" data-testid="aiws-catalog-detail" data-entry={entry.key}>
      <div className="aiws-catalog-detail-content">
        <div className="aiws-catalog-detail-heading"><EntryIcon entry={entry} /><div><span className="aiws-catalog-section-label">{CATALOG_GROUP_LABEL[entry.group]}</span><h3 className="aiws-catalog-title">{entry.title}</h3></div></div>
        <p>{entry.defEntity ? (def?.description ?? entry.catalog.description) : entry.catalog.description}</p>
        <dl className="aiws-catalog-meta">
          <dt>{t('aiworkspace.canvas.catalog.type')}</dt><dd><code>{entry.definition.type}</code> v{entry.defEntity ? (def?.version ?? '…') : entry.definition.version}</dd>
          {entry.defEntity && <><dt>{t('aiworkspace.canvas.catalog.definition')}</dt><dd>{t('aiworkspace.canvas.catalog.nameWithType', { name: entry.defEntity.title ?? entry.defEntity.name ?? '', type: entry.defEntity.def_kind === 'html' ? 'HTML' : t('aiworkspace.canvas.catalog.declarative') })}</dd></>}
          {accepts.length > 0 && <><dt>{t('aiworkspace.canvas.catalog.accepts')}</dt><dd>{list(accepts.map((type) => type.replace('buckyos.', '')))}</dd></>}
          <dt>{t('aiworkspace.canvas.catalog.defaultSize')}</dt><dd>{(def?.default_size ?? entrySize(entry)).w} × {(def?.default_size ?? entrySize(entry)).h}</dd>
        </dl>
        {entryId(entry) === 'shape' && <ShapePicker value={kind} onPick={setKind} />}
        {entryId(entry) === 'shape-svg' && (
          <>
            <label className="aiws-field">{t('aiworkspace.canvas.catalog.svgCode')}<textarea rows={5} value={svgText} placeholder="<svg …>…</svg>" data-testid="aiws-catalog-svg-text" onChange={(e) => setSvgText(e.target.value)} /></label>
            <label className="aiws-field">{t('aiworkspace.canvas.catalog.svgFile')}<input type="file" accept=".svg,image/svg+xml" data-testid="aiws-catalog-svg-file" onChange={(e) => { const f = e.target.files?.[0]; if (f) void f.text().then(setSvgText) }} /></label>
            {svgText.trim() && !svg && <div className="aiws-warning">{t('aiworkspace.canvas.catalog.svgInvalid')}</div>}
            {svg && svg.svg.body.length > 48 * 1024 && <div className="aiws-warning">{t('aiworkspace.canvas.catalog.svgTooLarge')}</div>}
          </>
        )}
        {(entry.options ?? []).map((option) => (
          <label key={option.key} className="aiws-field">{option.label}
            <select aria-label={option.label} data-testid={`aiws-catalog-option-${option.key}`} value={choices[option.key] ?? String(entry.config?.[option.key] ?? '')} onChange={(event) => setChoices({ ...choices, [option.key]: event.target.value })}>
              {option.choices.map((c) => <option key={c.value} value={c.value}>{c.label}</option>)}
            </select>
          </label>
        ))}
        {entry.catalog.needs === 'none' && !entry.defEntity && (
          <label className="aiws-field">{t('aiworkspace.canvas.catalog.titleOptional')}<input aria-label={t('aiworkspace.canvas.catalog.objectTitle')} value={title} data-testid="aiws-catalog-title" onChange={(event) => setTitle(event.target.value)} /></label>
        )}
        {(needsData || optionalData) && (
          <label className="aiws-field">{t(needsData ? 'aiworkspace.canvas.catalog.dataToShow' : 'aiworkspace.canvas.catalog.bindData')}
            <select aria-label={t('aiworkspace.canvas.catalog.data')} data-testid="aiws-catalog-data" value={sourceId} onChange={(event) => setSourceId(event.target.value)}>
              <option value="">{t(data.length === 0 ? 'aiworkspace.canvas.catalog.noData' : 'aiworkspace.canvas.catalog.chooseData')}</option>
              {data.map((e) => <option key={e.entity_id} value={e.entity_id}>{t('aiworkspace.canvas.catalog.nameWithType', { name: e.title ?? e.name ?? e.entity_id, type: e.type_id.replace('buckyos.', '') })}</option>)}
            </select>
            <span className="aiws-muted">{t('aiworkspace.canvas.catalog.sharesData')}</span>
          </label>
        )}
        {entry.defEntity && def && <InstanceForm def={def} config={instanceConfig} bindings={instanceBindings} onConfig={setInstanceConfig} onBindings={setInstanceBindings} />}
        {needsFile && (
          <label className="aiws-field">{t('aiworkspace.canvas.catalog.file')}<input type="file" aria-label={t('aiworkspace.canvas.catalog.chooseFile')} data-testid="aiws-catalog-file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} /></label>
        )}
        {missing && <div className="aiws-muted" data-testid="aiws-catalog-missing">{missing}</div>}
      </div>
      <div className="aiws-dialog-actions aiws-catalog-actions">
        <button type="button" className="is-primary" data-testid="aiws-catalog-insert" disabled={!ready} onClick={() => onInsert(request(), 'center')}><Plus size={16} aria-hidden="true" />{t('aiworkspace.canvas.catalog.insertCenter')}</button>
        {canPlace && <button type="button" data-testid="aiws-catalog-place" disabled={!ready} onClick={() => onInsert(request(), 'place')}><MousePointer2 size={16} aria-hidden="true" />{t('aiworkspace.canvas.catalog.place')}</button>}
        {entryId(entry) === 'table' && onImportTable && <button type="button" data-testid="aiws-catalog-import-table" onClick={onImportTable}><FileUp size={16} aria-hidden="true" />{t('aiworkspace.canvas.catalog.importTable')}</button>}
        {entry.defEntity && (
          <button type="button" className="aiws-link" data-testid="aiws-catalog-pin" onClick={() => store.userState.set('ui:pinned-defs', pinned.includes(defId) ? pinned.filter((id) => id !== defId) : [...pinned, defId].slice(-6))}>
            {t(pinned.includes(defId) ? 'aiworkspace.canvas.catalog.unpin' : 'aiworkspace.canvas.catalog.pin')}
          </button>
        )}
      </div>
    </div>
  )
}

function ExistingData({ canPlace, onInsert }: { canPlace: boolean; onInsert: (request: InsertRequest, how: 'center' | 'place') => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const [chosen, setChosen] = useState('')
  const [renderer, setRenderer] = useState('')
  const data = dataFor(store, null)
  const entity: EntityEnvelope | undefined = chosen ? store.outline.get(chosen) : undefined
  const renderers = entity ? blockRegistry.forSource(entity.type_id).filter((def) => def.create && def.catalog?.needs !== 'definition') : []
  const definition = renderer ? blockRegistry.get(renderer) : entity ? blockRegistry.get(defaultRendererFor(entity.type_id) ?? '') : undefined
  const request = (): InsertRequest | null => definition?.catalog ? ({ entry: { key: `block:${definition.type}`, definition, catalog: definition.catalog, title: definition.title, group: definition.catalog.group }, sourceId: chosen }) : null
  return (
    <div className="aiws-catalog-detail aiws-catalog-existing" data-testid="aiws-catalog-existing">
      <div className="aiws-catalog-detail-content">
        <div className="aiws-catalog-detail-heading"><span className="aiws-catalog-entry-icon"><Layers3 size={24} aria-hidden="true" /></span><h3 className="aiws-catalog-title">{t('aiworkspace.canvas.catalog.addExisting')}</h3></div>
        <p>{t('aiworkspace.canvas.catalog.addExistingNote')}</p>
        <label className="aiws-field">{t('aiworkspace.canvas.catalog.data')}
          <select aria-label={t('aiworkspace.canvas.catalog.data')} data-testid="aiws-picker-data" value={chosen} onChange={(event) => { setChosen(event.target.value); setRenderer('') }}>
            <option value="">{t('aiworkspace.canvas.catalog.chooseData')}</option>
            {data.map((e) => <option key={e.entity_id} value={e.entity_id}>{t('aiworkspace.canvas.catalog.nameWithType', { name: e.title ?? e.name ?? e.entity_id, type: e.type_id.replace('buckyos.', '') })}</option>)}
          </select>
        </label>
        {renderers.length > 1 && (
          <label className="aiws-field">{t('aiworkspace.canvas.catalog.renderer')}
            <select aria-label={t('aiworkspace.canvas.catalog.renderer')} data-testid="aiws-picker-renderer" value={renderer} onChange={(event) => setRenderer(event.target.value)}>
              <option value="">{t('aiworkspace.canvas.catalog.defaultRenderer')}</option>
              {renderers.map((d) => <option key={d.type} value={d.type}>{d.title}</option>)}
            </select>
          </label>
        )}
        {entity && !definition && <div className="aiws-warning">{t('aiworkspace.canvas.catalog.noRenderer')}</div>}
      </div>
      <div className="aiws-dialog-actions aiws-catalog-actions">
        <button type="button" className="is-primary" data-testid="aiws-picker-confirm" disabled={!request()} onClick={() => { const r = request(); if (r) onInsert(r, 'center') }}><Plus size={16} aria-hidden="true" />{t('aiworkspace.canvas.catalog.add')}</button>
        {canPlace && <button type="button" data-testid="aiws-catalog-place" disabled={!request()} onClick={() => { const r = request(); if (r) onInsert(r, 'place') }}><MousePointer2 size={16} aria-hidden="true" />{t('aiworkspace.canvas.catalog.place')}</button>}
      </div>
    </div>
  )
}
