/* The insert catalog dialog (UI improvement §6.3): search, categories, an entry's description and type
 * details, and the step the entry needs before it can be inserted (existing data, a file, a binding of an
 * extension). One catalog serves the main menu, the object toolbar's "+" and the blank-spot context menu.
 * "Existing data" only creates a view: the new Block shares the data, its view configuration is its own. */

import { useCallback, useRef, useState, useSyncExternalStore } from 'react'
import type { BlockDefPayload, EntityEnvelope } from '../../api/types'
import type { ReadOk } from '../../api/session'
import { useLoad, useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import { blockRegistry, CATALOG_GROUP_LABEL, type CatalogGroup } from '../blocks/registry'
import { defaultRendererFor } from '../blocks/ops'
import { useOverlayMounted } from '../shell/popover'
import { allEntries, dataFor, type CatalogEntry, type InsertRequest } from './catalog'

export type CatalogTab = 'all' | CatalogGroup | 'existing'
const TABS: CatalogTab[] = ['all', 'text', 'data', 'layout', 'ai', 'sample', 'extension', 'existing']
const TAB_LABEL: Record<CatalogTab, string> = { all: '全部', ...CATALOG_GROUP_LABEL, existing: '已有数据' }

export function InsertCatalog({ initialTab = 'all', initialKey, canPlace, onClose, onInsert }: {
  initialTab?: CatalogTab
  initialKey?: string
  /** A free Surface: the entry can also be placed by a click on the canvas. */
  canPlace: boolean
  onClose: () => void
  onInsert: (request: InsertRequest, how: 'center' | 'place') => void
}) {
  const store = useStore()
  useOutlineVersion()
  useOverlayMounted()
  useSyncExternalStore(blockRegistry.subscribe, blockRegistry.snapshot)
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
      <div ref={dialogRef} className="aiws-dialog aiws-catalog" role="dialog" aria-modal="true" aria-label="插入对象" data-testid="aiws-catalog"
        onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }}>
        <div className="aiws-dialog-head">
          <b>插入对象</b>
          <input className="aiws-catalog-search" type="search" aria-label="搜索对象" placeholder="搜索对象…" autoFocus value={query} data-testid="aiws-catalog-search"
            onChange={(event) => { setQuery(event.target.value); if (tab === 'existing') setTab('all') }} />
          <button type="button" className="aiws-link" onClick={onClose}>关闭</button>
        </div>
        <div className="aiws-catalog-body">
          <nav className="aiws-catalog-tabs" role="tablist" aria-label="对象分类">
            {TABS.map((t) => (
              <button key={t} type="button" role="tab" aria-selected={tab === t} data-testid={`aiws-catalog-tab-${t}`} onClick={() => { setTab(t); setChosenKey(null) }}>{TAB_LABEL[t]}</button>
            ))}
          </nav>
          {tab === 'existing' ? <ExistingData onInsert={onInsert} canPlace={canPlace} /> : (
            <>
              <ul className="aiws-catalog-list" role="listbox" aria-label="对象" data-testid="aiws-catalog-list">
                {shown.length === 0 && <li className="aiws-muted">{tab === 'extension' ? '这个工作区还没有 Block 定义（扩展）。可在数据源中导入或由许愿格生成。' : '没有匹配的对象。'}</li>}
                {shown.map((entry) => (
                  <li key={entry.key} role="option" aria-selected={chosen?.key === entry.key}>
                    <button type="button" data-testid={`aiws-catalog-entry-${entry.key.replace(':', '-')}`} className={chosen?.key === entry.key ? 'is-active' : undefined}
                      onClick={() => setChosenKey(entry.key)} onDoubleClick={() => { if (entry.catalog.needs === 'none') onInsert({ entry }, 'center') }}>
                      <span>{entry.title}</span>
                      {entry.group === 'sample' && <span className="aiws-chip">样本</span>}
                      {entry.group === 'extension' && <span className="aiws-chip">扩展</span>}
                    </button>
                  </li>
                ))}
              </ul>
              {chosen ? <EntryDetail key={chosen.key} entry={chosen} canPlace={canPlace} onInsert={onInsert} /> : <div className="aiws-catalog-detail aiws-muted">选择一个对象查看说明。</div>}
            </>
          )}
        </div>
      </div>
    </div>
  )
}

function EntryDetail({ entry, canPlace, onInsert }: { entry: CatalogEntry; canPlace: boolean; onInsert: (request: InsertRequest, how: 'center' | 'place') => void }) {
  const store = useStore()
  const [title, setTitle] = useState('')
  const [sourceId, setSourceId] = useState('')
  const [file, setFile] = useState<File | null>(null)
  const pinned = useUserState<string[]>('ui:pinned-defs') ?? []
  const defId = entry.defEntity?.entity_id ?? ''
  const loadDef = useCallback(async () => (defId ? store.readBatched<{ payload: BlockDefPayload }>(defId) : null), [store, defId])
  const def = useLoad<ReadOk<{ payload: BlockDefPayload }> | null>(loadDef).data?.content.payload
  const accepts = entry.defEntity ? (def?.accepts ?? entry.definition.accepts) : entry.definition.accepts
  const needsData = entry.catalog.needs === 'data' || (entry.defEntity !== undefined && def !== undefined && def.allow_no_source !== true)
  const optionalData = entry.defEntity !== undefined && def?.allow_no_source === true && accepts.length > 0
  const data = needsData || optionalData ? dataFor(store, accepts) : []
  const needsFile = entry.catalog.needs === 'file'
  const ready = (!needsData || sourceId !== '') && (!needsFile || file !== null) && (!entry.defEntity || def !== undefined)
  const request = (): InsertRequest => ({
    entry, title: title.trim() || undefined, sourceId: sourceId || undefined, file: file ?? undefined,
    config: entry.defEntity ? { def_id: entry.defEntity.entity_id } : undefined,
  })
  const missing = needsData && sourceId === '' ? '先选择要显示的数据' : needsFile && !file ? '先选择文件' : null
  return (
    <div className="aiws-catalog-detail" data-testid="aiws-catalog-detail" data-entry={entry.key}>
      <div className="aiws-catalog-title">{entry.title}{entry.group === 'sample' && <span className="aiws-chip">样本</span>}</div>
      <p>{entry.defEntity ? (def?.description ?? entry.catalog.description) : entry.catalog.description}</p>
      <dl className="aiws-catalog-meta">
        <dt>类型</dt><dd><code>{entry.definition.type}</code> v{entry.defEntity ? (def?.version ?? '…') : entry.definition.version}</dd>
        {entry.defEntity && <><dt>定义</dt><dd>{entry.defEntity.title ?? entry.defEntity.name}（{entry.defEntity.def_kind === 'html' ? 'HTML' : '声明式'}）</dd></>}
        {accepts.length > 0 && <><dt>支持的数据</dt><dd>{accepts.map((t) => t.replace('buckyos.', '')).join('、')}</dd></>}
        <dt>默认尺寸</dt><dd>{(def?.default_size ?? entry.definition.defaultSize).w} × {(def?.default_size ?? entry.definition.defaultSize).h}</dd>
      </dl>
      {entry.catalog.needs === 'none' && !entry.defEntity && (
        <label className="aiws-field">标题（可留空）<input aria-label="对象标题" value={title} data-testid="aiws-catalog-title" onChange={(event) => setTitle(event.target.value)} /></label>
      )}
      {(needsData || optionalData) && (
        <label className="aiws-field">{needsData ? '显示哪份数据' : '绑定数据（可选）'}
          <select aria-label="数据" data-testid="aiws-catalog-data" value={sourceId} onChange={(event) => setSourceId(event.target.value)}>
            <option value="">{data.length === 0 ? '没有可用的数据' : '选择数据…'}</option>
            {data.map((e) => <option key={e.entity_id} value={e.entity_id}>{e.title ?? e.name ?? e.entity_id}（{e.type_id.replace('buckyos.', '')}）</option>)}
          </select>
          <span className="aiws-muted">新 Block 与原数据共享内容，视图配置独立。</span>
        </label>
      )}
      {needsFile && (
        <label className="aiws-field">文件<input type="file" aria-label="选择文件" data-testid="aiws-catalog-file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} /></label>
      )}
      {missing && <div className="aiws-muted" data-testid="aiws-catalog-missing">{missing}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-catalog-insert" disabled={!ready} onClick={() => onInsert(request(), 'center')}>插入到视图中央</button>
        {canPlace && <button type="button" data-testid="aiws-catalog-place" disabled={!ready} onClick={() => onInsert(request(), 'place')}>在画布上放置</button>}
        {entry.defEntity && (
          <button type="button" className="aiws-link" data-testid="aiws-catalog-pin" onClick={() => store.userState.set('ui:pinned-defs', pinned.includes(defId) ? pinned.filter((id) => id !== defId) : [...pinned, defId].slice(-6))}>
            {pinned.includes(defId) ? '从工具栏取消固定' : '固定到工具栏'}
          </button>
        )}
      </div>
    </div>
  )
}

function ExistingData({ canPlace, onInsert }: { canPlace: boolean; onInsert: (request: InsertRequest, how: 'center' | 'place') => void }) {
  const store = useStore()
  const [chosen, setChosen] = useState('')
  const [renderer, setRenderer] = useState('')
  const data = dataFor(store, null)
  const entity: EntityEnvelope | undefined = chosen ? store.outline.get(chosen) : undefined
  const renderers = entity ? blockRegistry.forSource(entity.type_id).filter((def) => def.create && def.catalog?.needs !== 'definition') : []
  const definition = renderer ? blockRegistry.get(renderer) : entity ? blockRegistry.get(defaultRendererFor(entity.type_id) ?? '') : undefined
  const request = (): InsertRequest | null => definition?.catalog ? ({ entry: { key: `block:${definition.type}`, definition, catalog: definition.catalog, title: definition.title, group: definition.catalog.group }, sourceId: chosen }) : null
  return (
    <div className="aiws-catalog-detail aiws-catalog-existing" data-testid="aiws-catalog-existing">
      <div className="aiws-catalog-title">添加已有数据</div>
      <p>只创建引用这份数据的视图：与原视图共享数据，视图配置独立。不会复制数据。</p>
      <label className="aiws-field">数据
        <select aria-label="数据" data-testid="aiws-picker-data" value={chosen} onChange={(event) => { setChosen(event.target.value); setRenderer('') }}>
          <option value="">选择数据…</option>
          {data.map((e) => <option key={e.entity_id} value={e.entity_id}>{e.title ?? e.name ?? e.entity_id}（{e.type_id.replace('buckyos.', '')}）</option>)}
        </select>
      </label>
      {renderers.length > 1 && (
        <label className="aiws-field">展现方式
          <select aria-label="展现方式" data-testid="aiws-picker-renderer" value={renderer} onChange={(event) => setRenderer(event.target.value)}>
            <option value="">默认展现</option>
            {renderers.map((d) => <option key={d.type} value={d.type}>{d.title}</option>)}
          </select>
        </label>
      )}
      {entity && !definition && <div className="aiws-warning">这类数据没有可用的展现方式。</div>}
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-picker-confirm" disabled={!request()} onClick={() => { const r = request(); if (r) onInsert(r, 'center') }}>添加</button>
        {canPlace && <button type="button" data-testid="aiws-catalog-place" disabled={!request()} onClick={() => { const r = request(); if (r) onInsert(r, 'place') }}>在画布上放置</button>}
      </div>
    </div>
  )
}
