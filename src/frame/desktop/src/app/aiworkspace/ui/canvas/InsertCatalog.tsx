/* The insert catalog dialog (UI improvement §6.3): search, categories, an entry's description and type
 * details, and the step the entry needs before it can be inserted (existing data, a file, a binding of an
 * extension). One catalog serves the main menu, the object toolbar's "+" and the blank-spot context menu.
 * "Existing data" only creates a view: the new Block shares the data, its view configuration is its own. */

import { useCallback, useRef, useState, useSyncExternalStore } from 'react'
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

export type CatalogTab = 'all' | CatalogGroup | 'existing'
const TABS: CatalogTab[] = ['all', 'text', 'data', 'layout', 'ai', 'sample', 'extension', 'existing']

export function InsertCatalog({ initialTab = 'all', initialKey, canPlace, onClose, onInsert, onImport }: {
  initialTab?: CatalogTab
  initialKey?: string
  /** "Import draw.io…" (draw.io 导入 TODO §3), when importing is possible here. */
  onImport?: () => void
  /** A free Surface: the entry can also be placed by a click on the canvas. */
  canPlace: boolean
  onClose: () => void
  onInsert: (request: InsertRequest, how: 'center' | 'place') => void
}) {
  const store = useStore()
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
  const chosen = entries.find((entry) => entry.key === chosenKey) ?? (tab !== 'existing' ? shown[0] : undefined)
  const dialogRef = useRef<HTMLDivElement>(null)
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div ref={dialogRef} className="aiws-dialog aiws-catalog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.canvas.catalog.title')} data-testid="aiws-catalog"
        onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }}>
        <div className="aiws-dialog-head">
          <b>{t('aiworkspace.canvas.catalog.title')}</b>
          <input className="aiws-catalog-search" type="search" aria-label={t('aiworkspace.canvas.catalog.search')} placeholder={t('aiworkspace.canvas.catalog.searchPlaceholder')} autoFocus value={query} data-testid="aiws-catalog-search"
            onChange={(event) => { setQuery(event.target.value); if (tab === 'existing') setTab('all') }} />
          {onImport && <button type="button" className="aiws-link" data-testid="aiws-catalog-import-drawio" onClick={onImport}>{t('aiworkspace.canvas.catalog.importDrawio')}</button>}
          <button type="button" className="aiws-link" onClick={onClose}>{t('aiworkspace.common.close')}</button>
        </div>
        <div className="aiws-catalog-body">
          <nav className="aiws-catalog-tabs" role="tablist" aria-label={t('aiworkspace.canvas.catalog.categories')}>
            {TABS.map((item) => (
              <button key={item} type="button" role="tab" aria-selected={tab === item} data-testid={`aiws-catalog-tab-${item}`} onClick={() => { setTab(item); setChosenKey(null) }}>{tabLabel(item)}</button>
            ))}
          </nav>
          {tab === 'existing' ? <ExistingData onInsert={onInsert} canPlace={canPlace} /> : (
            <>
              <ul className="aiws-catalog-list" role="listbox" aria-label={t('aiworkspace.canvas.catalog.objects')} data-testid="aiws-catalog-list">
                {shown.length === 0 && <li className="aiws-muted">{t(tab === 'extension' ? 'aiworkspace.canvas.catalog.noExtensions' : 'aiworkspace.canvas.catalog.noMatch')}</li>}
                {shown.map((entry) => (
                  <li key={entry.key} role="option" aria-selected={chosen?.key === entry.key}>
                    <button type="button" data-testid={`aiws-catalog-entry-${entry.key.replace(':', '-')}`} className={chosen?.key === entry.key ? 'is-active' : undefined}
                      onClick={() => setChosenKey(entry.key)} onDoubleClick={() => { if (entry.catalog.needs === 'none') onInsert({ entry }, 'center') }}>
                      <span>{entry.title}</span>
                      {entry.group === 'sample' && <span className="aiws-chip">{t('aiworkspace.canvas.catalog.sample')}</span>}
                      {entry.group === 'extension' && <span className="aiws-chip">{t('aiworkspace.canvas.catalog.extension')}</span>}
                    </button>
                  </li>
                ))}
              </ul>
              {chosen ? <EntryDetail key={chosen.key} entry={chosen} canPlace={canPlace} onInsert={onInsert} /> : <div className="aiws-catalog-detail aiws-muted">{t('aiworkspace.canvas.catalog.chooseEntry')}</div>}
            </>
          )}
        </div>
      </div>
    </div>
  )
}

function EntryDetail({ entry, canPlace, onInsert }: { entry: CatalogEntry; canPlace: boolean; onInsert: (request: InsertRequest, how: 'center' | 'place') => void }) {
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
  const accepts = entry.defEntity ? (def?.accepts ?? entry.definition.accepts) : entry.definition.accepts
  const needsData = entry.catalog.needs === 'data' || (entry.defEntity !== undefined && def !== undefined && def.allow_no_source !== true)
  const optionalData = entry.defEntity !== undefined && def?.allow_no_source === true && accepts.length > 0
  const data = needsData || optionalData ? dataFor(store, accepts) : []
  const needsFile = entry.catalog.needs === 'file'
  const ready = (!needsData || sourceId !== '') && (!needsFile || file !== null) && (!entry.defEntity || def !== undefined) && (entryId(entry) !== 'shape-svg' || (svg !== null && svg.svg.body.length <= 48 * 1024))
  const svgSize = svg ? (() => { const k = Math.min(1, 320 / Math.max(svg.w, svg.h, 1)); return { w: Math.max(24, Math.round(svg.w * k)), h: Math.max(24, Math.round(svg.h * k)) } })() : null
  const request = (): InsertRequest => svg && svgSize ? ({ entry: { ...entry, size: svgSize }, title: title.trim() || undefined, config: { shape: 'svg', svg: svg.svg as unknown as Json, fill: svg.fill, stroke: svg.stroke ?? 'none', stroke_width: 1 } }) : ({
    entry, title: title.trim() || undefined, sourceId: sourceId || undefined, file: file ?? undefined,
    config: entry.defEntity ? { def_id: entry.defEntity.entity_id } : entryId(entry) === 'shape' ? { shape: kind } : Object.keys(choices).length ? choices : undefined,
  })
  const missing = needsData && sourceId === '' ? t('aiworkspace.canvas.catalog.missingData') : needsFile && !file ? t('aiworkspace.canvas.catalog.missingFile') : null
  return (
    <div className="aiws-catalog-detail" data-testid="aiws-catalog-detail" data-entry={entry.key}>
      <div className="aiws-catalog-title">{entry.title}{entry.group === 'sample' && <span className="aiws-chip">{t('aiworkspace.canvas.catalog.sample')}</span>}</div>
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
      {needsFile && (
        <label className="aiws-field">{t('aiworkspace.canvas.catalog.file')}<input type="file" aria-label={t('aiworkspace.canvas.catalog.chooseFile')} data-testid="aiws-catalog-file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} /></label>
      )}
      {missing && <div className="aiws-muted" data-testid="aiws-catalog-missing">{missing}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-catalog-insert" disabled={!ready} onClick={() => onInsert(request(), 'center')}>{t('aiworkspace.canvas.catalog.insertCenter')}</button>
        {canPlace && <button type="button" data-testid="aiws-catalog-place" disabled={!ready} onClick={() => onInsert(request(), 'place')}>{t('aiworkspace.canvas.catalog.place')}</button>}
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
      <div className="aiws-catalog-title">{t('aiworkspace.canvas.catalog.addExisting')}</div>
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
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-picker-confirm" disabled={!request()} onClick={() => { const r = request(); if (r) onInsert(r, 'center') }}>{t('aiworkspace.canvas.catalog.add')}</button>
        {canPlace && <button type="button" data-testid="aiws-catalog-place" disabled={!request()} onClick={() => { const r = request(); if (r) onInsert(r, 'place') }}>{t('aiworkspace.canvas.catalog.place')}</button>}
      </div>
    </div>
  )
}
