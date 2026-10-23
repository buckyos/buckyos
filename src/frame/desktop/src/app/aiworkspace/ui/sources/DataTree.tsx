/* The data tree (phase two §6.1): first-level data and folders, the canvas content area collapsed
 * by default and grouped by Surface, type and origin filters, search, creation into the selected
 * folder, move, and deletion with the pre-check that names only visible referrers. Blocks are not
 * shown here; pure UI Blocks are managed on the canvas. */

import { useEffect, useMemo, useRef, useState } from 'react'
import { randomId } from '../../api/ids'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Operation } from '../../api/types'
import { useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { TYPE_LABEL, entityLabel, importedAssetOps, newDataOps, type NewDataKind } from './dataOps'
import { FreshnessBadge } from './FreshnessBadge'

type TypeFilter = 'all' | 'buckyos.table-source' | 'buckyos.richtext' | 'buckyos.record' | 'buckyos.asset-ref' | 'buckyos.annotation' | 'buckyos.wish' | 'buckyos.block-def'
type OriginFilter = 'all' | 'original' | 'generated'

/** The edit label of creating each kind of data. */
const CREATE_LABEL: Record<NewDataKind, string> = {
  folder: 'aiworkspace.data.tree.newFolder', table: 'aiworkspace.data.tree.newTable', richtext: 'aiworkspace.data.tree.newRichtext',
  record: 'aiworkspace.data.tree.newRecord', wish: 'aiworkspace.data.tree.newWish', note: 'aiworkspace.data.tree.newNote',
}

export function DataTree({ selected, onSelect }: { selected: string | null; onSelect: (id: string | null) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const [query, setQuery] = useState('')
  const [typeFilter, setTypeFilter] = useState<TypeFilter>('all')
  const [origin, setOrigin] = useState<OriginFilter>('all')
  const expandedState = useUserState<Record<string, boolean>>('datatree:expanded')
  const expanded = useMemo(() => expandedState ?? {}, [expandedState])
  const toggle = (id: string) => store.userState.set('datatree:expanded', { ...expanded, [id]: !(expanded[id] ?? false) })
  const isExpanded = (entity: EntityEnvelope) => expanded[entity.entity_id] ?? (entity.entity_id !== 'canvas-content' && entity.entity_id !== 'shows' && entity.system !== 'surface_content')
  const matches = (entity: EntityEnvelope): boolean => {
    if (typeFilter !== 'all' && entity.type_id !== typeFilter) return false
    if (origin === 'generated' && !entity.derived) return false
    if (origin === 'original' && entity.derived) return false
    if (query && !entityLabel(entity).toLowerCase().includes(query.toLowerCase()) && !entity.entity_id.includes(query)) return false
    return true
  }
  const filtering = query !== '' || typeFilter !== 'all' || origin !== 'all'
  const data = store.outline.get('data')
  const total = store.outline.descendants('data').filter((e) => e.type_id !== 'buckyos.cell').length
  return (
    <div className="aiws-datatree" data-testid="aiws-datatree">
      <div className="aiws-panel-title">{t('aiworkspace.data.tree.title')} <span className="aiws-muted">{t('aiworkspace.data.tree.count', { count: total })}</span></div>
      <div className="aiws-inline-form aiws-datatree-filters">
        <input aria-label={t('aiworkspace.data.tree.search')} placeholder={t('aiworkspace.data.tree.searchPlaceholder')} value={query} onChange={(event) => setQuery(event.target.value)} />
        <select aria-label={t('aiworkspace.data.tree.typeFilter')} value={typeFilter} onChange={(event) => setTypeFilter(event.target.value as TypeFilter)}>
          <option value="all">{t('aiworkspace.data.tree.allTypes')}</option>
          {Object.entries(TYPE_LABEL).filter(([id]) => !['buckyos.cell', 'buckyos.container', 'buckyos.viewport', 'buckyos.show-path'].includes(id)).map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </select>
        <select aria-label={t('aiworkspace.data.tree.originFilter')} value={origin} onChange={(event) => setOrigin(event.target.value as OriginFilter)}>
          <option value="all">{t('aiworkspace.data.tree.originAll')}</option><option value="original">{t('aiworkspace.data.tree.originOriginal')}</option><option value="generated">{t('aiworkspace.data.tree.originGenerated')}</option>
        </select>
      </div>
      <ul role="tree" className="aiws-tree">
        {data && store.outline.childrenOf('data').map((child) => (
          <TreeNode key={child.entity_id} entity={child} depth={0} selected={selected} onSelect={onSelect} isExpanded={isExpanded} toggle={toggle} matches={matches} filtering={filtering} />
        ))}
      </ul>
    </div>
  )
}

function TreeNode({ entity, depth, selected, onSelect, isExpanded, toggle, matches, filtering }: { entity: EntityEnvelope; depth: number; selected: string | null; onSelect: (id: string | null) => void; isExpanded: (e: EntityEnvelope) => boolean; toggle: (id: string) => void; matches: (e: EntityEnvelope) => boolean; filtering: boolean }) {
  const { t } = useAiwsText()
  const store = useStore()
  const isFolder = entity.type_id === 'buckyos.container'
  const children = isFolder ? store.outline.childrenOf(entity.entity_id).filter((c) => c.type_id !== 'buckyos.cell') : store.outline.childrenOf(entity.entity_id).filter((c) => c.type_id === 'buckyos.richtext')
  const visibleChildren = filtering ? children.filter((c) => matches(c) || (c.type_id === 'buckyos.container' && store.outline.descendants(c.entity_id).some(matches))) : children
  if (filtering && !matches(entity) && visibleChildren.length === 0) return null
  const open = filtering ? true : isExpanded(entity)
  const label = entity.entity_id === 'canvas-content' ? t('aiworkspace.data.canvasContent') : entity.entity_id === 'shows' ? t('aiworkspace.data.tree.showPaths') : entityLabel(entity)
  const surface = entity.system === 'surface_content' && entity.surface_id ? store.outline.get(entity.surface_id) : undefined
  return (
    <li role="treeitem" aria-expanded={isFolder ? open : undefined} aria-selected={selected === entity.entity_id} data-testid="aiws-tree-item" data-entity-id={entity.entity_id} data-type={entity.type_id} data-system={entity.system ?? undefined}>
      <div className={`aiws-tree-row${selected === entity.entity_id ? ' is-selected' : ''}`} style={{ paddingLeft: depth * 14 }}>
        {(isFolder || children.length > 0) ? <button type="button" className="aiws-tree-toggle" aria-label={open ? t('aiworkspace.data.tree.collapse') : t('aiworkspace.data.tree.expand')} onClick={() => toggle(entity.entity_id)}>{open ? '▾' : '▸'}</button> : <span className="aiws-tree-toggle" />}
        <button type="button" className="aiws-tree-label" data-testid={`aiws-tree-${entity.entity_id}`} onClick={() => onSelect(selected === entity.entity_id ? null : entity.entity_id)}>
          <span className="aiws-outline-type">{entity.system === 'canvas_content' || entity.system === 'shows' ? t('aiworkspace.data.tree.system') : entity.system === 'surface_content' ? t('aiworkspace.data.canvas') : TYPE_LABEL[entity.type_id] ?? entity.type_id}</span>
          <span className="aiws-outline-name">{label}{surface ? <span className="aiws-muted">{t('aiworkspace.data.tree.onCanvas', { name: entityLabel(surface) })}</span> : null}</span>
          {entity.derived && <span className="aiws-chip aiws-chip-derived" title={t('aiworkspace.data.tree.generatedTitle')}>{t('aiworkspace.data.tree.generated')}</span>}
          {entity.derived && <FreshnessBadge entityId={entity.entity_id} showManual={false} />}
          {entity.write_policy === 'lock_required' && <span title={t('aiworkspace.data.tree.lockTitle')}>🔒</span>}
          {entity.degraded && <span className="aiws-chip aiws-chip-warn" title={entity.degraded}>{t('aiworkspace.data.tree.degraded')}</span>}
        </button>
      </div>
      {open && visibleChildren.length > 0 && (
        <ul role="group">{visibleChildren.map((child) => <TreeNode key={child.entity_id} entity={child} depth={depth + 1} selected={selected} onSelect={onSelect} isExpanded={isExpanded} toggle={toggle} matches={matches} filtering={filtering} />)}</ul>
      )}
    </li>
  )
}

/** Folder actions below the tree: new data in the selected folder, move, delete with pre-check. */
export function DataTreeActions({ selected, onSelect }: { selected: string | null; onSelect: (id: string | null) => void }) {
  const { t, list } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const entity = selected ? store.outline.get(selected) : undefined
  const folder = entity?.type_id === 'buckyos.container' ? entity : (entity?.parent_id ? store.outline.get(entity.parent_id) : store.outline.get('data'))
  // the presentation folder holds paths and Viewports only (第三期规划 §6.2): new data goes to the data root instead
  const target = folder && folder.kind !== 'surfaces' && folder.kind !== 'surface' && folder.kind !== 'group' && folder.entity_id !== 'shows' ? folder : store.outline.get('data')
  const [kind, setKind] = useState<NewDataKind>('table')
  const [title, setTitle] = useState('')
  const importController = useRef<AbortController | null>(null)
  const [importing, setImporting] = useState(false)
  useEffect(() => () => importController.current?.abort(), [])
  const [deleting, setDeleting] = useState<{ referrers: { entity_id: string; target: string }[]; hidden: boolean } | null>(null)
  const canCreate = Boolean(target?.capabilities.includes('structure'))
  const create = async () => {
    if (!target) return
    const ops = newDataOps(store, target.entity_id, kind, title.trim())
    const outcome = await store.submit({ editId: `new-data:${randomId().slice(0, 6)}`, label: t(CREATE_LABEL[kind]), operations: ops })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { setTitle(''); const created = ops.filter((op) => op.op === 'entity.create').at(-1); if (created) onSelect(created.entity_id as string) }
  }
  const addImage = async (file: File) => {
    if (!target) return
    try {
      const uploaded = await store.session.uploadAsset(file, file.name)
      const key = store.core.order_key_between(store.outline.childrenOf(target.entity_id).at(-1)?.order_key ?? undefined, undefined)
      const id = randomId('a')
      const outcome = await store.submit({ editId: `new-data:${id}`, label: t('aiworkspace.data.tree.importImageEdit', { name: file.name }), operations: [{ op: 'entity.create', entity_id: id, type_id: 'buckyos.asset-ref', parent_id: target.entity_id, order_key: key, name: file.name, payload: { object_id: uploaded.object_id, file_name: file.name } }] })
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') onSelect(id)
    } catch (error) { store.notify('error', t('aiworkspace.data.tree.uploadFailed', { error: describeError(error) })) }
  }
  const importFiles = async (files: File[], folder: boolean) => {
    if (!target || importing) return
    const controller = new AbortController()
    importController.current = controller
    setImporting(true)
    try {
      const items = await store.session.importAssets(files, folder, controller.signal)
      controller.signal.throwIfAborted()
      const operations = importedAssetOps(store, target.entity_id, items)
      const id = randomId('import')
      const result = await store.submit({ editId: id, label: t('aiworkspace.data.tree.importFiles'), operations })
      if (result.status === 'accepted' || result.status === 'saved_locally') onSelect(operations[0].entity_id as string)
    } catch (error) {
      if (!controller.signal.aborted) store.notify('error', t('aiworkspace.data.tree.uploadFailed', { error: describeError(error) }))
    } finally { setImporting(false); importController.current = null }
  }
  const precheck = async () => {
    if (!entity) return
    const descendants = store.outline.descendants(entity.entity_id)
    const op: Operation = { op: 'entity.delete', entity_id: entity.entity_id, ...(descendants.length ? { subtree: { delete: descendants.map((d) => d.entity_id) } } : {}), expect: { rev: entity.life_rev } }
    try {
      const result = await store.session.prepare([op])
      if (result.status === 'ok') { setDeleting({ referrers: [], hidden: false }); return }
      const data = result.status === 'rejected' ? (result.errors?.[0]?.data as { referrers?: { entity_id: string; target: string }[]; hidden_referrers?: boolean } | undefined) : undefined
      setDeleting({ referrers: data?.referrers ?? [{ entity_id: result.code, target: '' }], hidden: Boolean(data?.hidden_referrers) })
    } catch (error) { store.notify('error', t('aiworkspace.data.tree.precheckFailed', { error: describeError(error) })) }
  }
  const doDelete = async () => {
    if (!entity) return
    const descendants = store.outline.descendants(entity.entity_id)
    const outcome = await store.submit({ editId: `entity:${entity.entity_id}`, label: t('aiworkspace.data.tree.deleteEdit', { name: entityLabel(entity) }), operations: [{ op: 'entity.delete', entity_id: entity.entity_id, ...(descendants.length ? { subtree: { delete: descendants.map((d) => d.entity_id) } } : {}), expect: { rev: entity.life_rev } }] })
    setDeleting(null)
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') onSelect(null)
  }
  const folders = store.outline.descendants('data').filter((e) => e.type_id === 'buckyos.container' && e.entity_id !== 'shows' && e.entity_id !== entity?.entity_id && !store.outline.ancestors(e.entity_id).includes(entity?.entity_id ?? '\0'))
  const moveTo = (parentId: string) => {
    if (!entity) return
    const key = store.core.order_key_between(store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined)
    void store.submit({ editId: `entity:${entity.entity_id}`, label: t('aiworkspace.data.tree.moveEdit', { name: entityLabel(entity) }), operations: [{ op: 'tree.move', entity_id: entity.entity_id, new_parent_id: parentId, order_key: key }] })
  }
  const isSystem = entity && ['data', 'canvas-content', 'shows'].includes(entity.entity_id)
  return (
    <div className="aiws-datatree-actions" data-testid="aiws-datatree-actions">
      {canCreate && (
        <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
          <span className="aiws-muted">{t('aiworkspace.data.tree.createIn', { folder: target?.entity_id === 'data' ? t('aiworkspace.data.tree.rootLevel') : entityLabel(target!) })}</span>
          <select aria-label={t('aiworkspace.data.tree.newKind')} data-testid="aiws-new-kind" value={kind} onChange={(event) => setKind(event.target.value as NewDataKind)}>
            <option value="table">{t('aiworkspace.data.tree.kindTable')}</option><option value="richtext">{t('aiworkspace.data.tree.kindRichtext')}</option><option value="record">{t('aiworkspace.data.tree.kindRecord')}</option>
            <option value="wish">{t('aiworkspace.data.tree.kindWish')}</option><option value="note">{t('aiworkspace.data.stickyNote')}</option><option value="folder">{t('aiworkspace.data.tree.kindFolder')}</option>
          </select>
          <input aria-label={t('aiworkspace.data.tree.titleField')} placeholder={t('aiworkspace.data.tree.titlePlaceholder')} value={title} onChange={(event) => setTitle(event.target.value)} />
          <button type="submit" data-testid="aiws-new-data">{t('aiworkspace.data.tree.create')}</button>
          <label className="aiws-link">{t('aiworkspace.data.tree.image')}<input type="file" hidden accept="image/*" data-testid="aiws-new-image" onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void addImage(file) }} /></label>
          <label className="aiws-link">{t('aiworkspace.data.tree.importFiles')}<input type="file" hidden multiple disabled={importing} data-testid="aiws-import-files" onChange={(event) => { const files = Array.from(event.target.files ?? []); event.target.value = ''; void importFiles(files, false) }} /></label>
          {'webkitdirectory' in document.createElement('input') && <label className="aiws-link">{t('aiworkspace.data.tree.importFolder')}<input type="file" hidden multiple disabled={importing} {...{ webkitdirectory: '' }} data-testid="aiws-import-folder" onChange={(event) => { const files = Array.from(event.target.files ?? []); event.target.value = ''; void importFiles(files, true) }} /></label>}
          <span className="aiws-muted">{t('aiworkspace.data.tree.folderLimit')}</span>
          {importing && <button type="button" onClick={() => importController.current?.abort()}>{t('aiworkspace.common.cancel')}</button>}
        </form>
      )}
      {entity && !isSystem && entity.system !== 'surface_content' && (
        <div className="aiws-inline-form">
          {entity.capabilities.includes('structure') && folders.length > 0 && (
            <select aria-label={t('aiworkspace.data.tree.moveTo')} value="" data-testid="aiws-move-to" onChange={(event) => { if (event.target.value) moveTo(event.target.value) }}>
              <option value="">{t('aiworkspace.data.tree.moveToOption')}</option>
              {folders.filter((f) => f.entity_id !== entity.parent_id).map((f) => <option key={f.entity_id} value={f.entity_id}>{f.entity_id === 'canvas-content' ? t('aiworkspace.data.canvasContent') : entityLabel(f)}</option>)}
            </select>
          )}
          {entity.capabilities.includes('delete') && <button type="button" data-testid="aiws-delete-entity" onClick={() => { void precheck() }}>{t('aiworkspace.data.tree.deleteOpen')}</button>}
        </div>
      )}
      {deleting && entity && (
        <div className="aiws-dialog" role="alertdialog" aria-label={t('aiworkspace.data.tree.deleteDialog')} data-testid="aiws-delete-dialog">
          <b>{store.outline.descendants(entity.entity_id).length > 0
            ? t('aiworkspace.data.tree.deleteTitleWithChildren', { name: entityLabel(entity), count: store.outline.descendants(entity.entity_id).length })
            : t('aiworkspace.data.tree.deleteTitle', { name: entityLabel(entity) })}</b>
          {deleting.referrers.length > 0 && (
            <div className="aiws-warning" data-testid="aiws-delete-blocked">{t(deleting.hidden ? 'aiworkspace.data.tree.stillReferencedHidden' : 'aiworkspace.data.tree.stillReferenced', {
              names: list(deleting.referrers.map((r) => entityLabel(store.outline.get(r.entity_id) ?? { entity_id: r.entity_id } as EntityEnvelope))),
            })}</div>
          )}
          {deleting.referrers.length === 0 && deleting.hidden && <div className="aiws-warning" data-testid="aiws-delete-blocked">{t('aiworkspace.data.tree.hiddenReferences')}</div>}
          <div className="aiws-inline-form">
            {deleting.referrers.length === 0 && !deleting.hidden && <button type="button" data-testid="aiws-delete-confirm" onClick={() => { void doDelete() }}>{t('aiworkspace.data.tree.confirmDelete')}</button>}
            <button type="button" onClick={() => setDeleting(null)}>{t('aiworkspace.common.cancel')}</button>
          </div>
        </div>
      )}
    </div>
  )
}
