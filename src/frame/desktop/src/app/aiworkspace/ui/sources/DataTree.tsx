/* The data tree (phase two §6.1): first-level data and folders, the canvas content area collapsed
 * by default and grouped by Surface, type and origin filters, search, creation into the selected
 * folder, move, and deletion with the pre-check that names only visible referrers. Blocks are not
 * shown here; pure UI Blocks are managed on the canvas. */

import { useMemo, useState } from 'react'
import { randomId } from '../../api/ids'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Operation } from '../../api/types'
import { useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import { TYPE_LABEL, entityLabel, newDataOps, type NewDataKind } from './dataOps'
import { FreshnessBadge } from './FreshnessBadge'

type TypeFilter = 'all' | 'buckyos.table-source' | 'buckyos.richtext' | 'buckyos.record' | 'buckyos.asset-ref' | 'buckyos.annotation' | 'buckyos.wish' | 'buckyos.block-def'
type OriginFilter = 'all' | 'original' | 'generated'

export function DataTree({ selected, onSelect }: { selected: string | null; onSelect: (id: string | null) => void }) {
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
      <div className="aiws-panel-title">数据树 <span className="aiws-muted">{total} 项</span></div>
      <div className="aiws-inline-form aiws-datatree-filters">
        <input aria-label="搜索数据" placeholder="搜索…" value={query} onChange={(event) => setQuery(event.target.value)} />
        <select aria-label="类型筛选" value={typeFilter} onChange={(event) => setTypeFilter(event.target.value as TypeFilter)}>
          <option value="all">全部类型</option>
          {Object.entries(TYPE_LABEL).filter(([id]) => !['buckyos.cell', 'buckyos.container', 'buckyos.viewport', 'buckyos.show-path'].includes(id)).map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </select>
        <select aria-label="来源筛选" value={origin} onChange={(event) => setOrigin(event.target.value as OriginFilter)}>
          <option value="all">原始与生成</option><option value="original">只看原始</option><option value="generated">只看生成结果</option>
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
  const store = useStore()
  const isFolder = entity.type_id === 'buckyos.container'
  const children = isFolder ? store.outline.childrenOf(entity.entity_id).filter((c) => c.type_id !== 'buckyos.cell') : store.outline.childrenOf(entity.entity_id).filter((c) => c.type_id === 'buckyos.richtext')
  const visibleChildren = filtering ? children.filter((c) => matches(c) || (c.type_id === 'buckyos.container' && store.outline.descendants(c.entity_id).some(matches))) : children
  if (filtering && !matches(entity) && visibleChildren.length === 0) return null
  const open = filtering ? true : isExpanded(entity)
  const label = entity.entity_id === 'canvas-content' ? '画布内容区' : entity.entity_id === 'shows' ? '演讲路径' : entityLabel(entity)
  const surface = entity.system === 'surface_content' && entity.surface_id ? store.outline.get(entity.surface_id) : undefined
  return (
    <li role="treeitem" aria-expanded={isFolder ? open : undefined} aria-selected={selected === entity.entity_id} data-testid="aiws-tree-item" data-entity-id={entity.entity_id} data-type={entity.type_id} data-system={entity.system ?? undefined}>
      <div className={`aiws-tree-row${selected === entity.entity_id ? ' is-selected' : ''}`} style={{ paddingLeft: depth * 14 }}>
        {(isFolder || children.length > 0) ? <button type="button" className="aiws-tree-toggle" aria-label={open ? '折叠' : '展开'} onClick={() => toggle(entity.entity_id)}>{open ? '▾' : '▸'}</button> : <span className="aiws-tree-toggle" />}
        <button type="button" className="aiws-tree-label" data-testid={`aiws-tree-${entity.entity_id}`} onClick={() => onSelect(selected === entity.entity_id ? null : entity.entity_id)}>
          <span className="aiws-outline-type">{entity.system === 'canvas_content' || entity.system === 'shows' ? '系统' : entity.system === 'surface_content' ? '画布' : TYPE_LABEL[entity.type_id] ?? entity.type_id}</span>
          <span className="aiws-outline-name">{label}{surface ? <span className="aiws-muted">（画布 {entityLabel(surface)}）</span> : null}</span>
          {entity.derived && <span className="aiws-chip aiws-chip-derived" title="由许愿格生成">生成</span>}
          {entity.derived && <FreshnessBadge entityId={entity.entity_id} showManual={false} />}
          {entity.write_policy === 'lock_required' && <span title="启用了写锁">🔒</span>}
          {entity.degraded && <span className="aiws-chip aiws-chip-warn" title={entity.degraded}>降级</span>}
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
  const store = useStore()
  useOutlineVersion()
  const entity = selected ? store.outline.get(selected) : undefined
  const folder = entity?.type_id === 'buckyos.container' ? entity : (entity?.parent_id ? store.outline.get(entity.parent_id) : store.outline.get('data'))
  // the presentation folder holds paths and Viewports only (第三期规划 §6.2): new data goes to the data root instead
  const target = folder && folder.kind !== 'surfaces' && folder.kind !== 'surface' && folder.kind !== 'group' && folder.entity_id !== 'shows' ? folder : store.outline.get('data')
  const [kind, setKind] = useState<NewDataKind>('table')
  const [title, setTitle] = useState('')
  const [deleting, setDeleting] = useState<{ referrers: { entity_id: string; target: string }[]; hidden: boolean } | null>(null)
  const canCreate = Boolean(target?.capabilities.includes('structure'))
  const create = async () => {
    if (!target) return
    const ops = newDataOps(store, target.entity_id, kind, title.trim())
    const outcome = await store.submit({ editId: `new-data:${randomId().slice(0, 6)}`, label: `新建${TYPE_LABEL[kind === 'folder' ? 'buckyos.container' : kind === 'table' ? 'buckyos.table-source' : kind === 'richtext' ? 'buckyos.richtext' : kind === 'record' ? 'buckyos.record' : kind === 'wish' ? 'buckyos.wish' : 'buckyos.annotation']}`, operations: ops })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { setTitle(''); const created = ops.filter((op) => op.op === 'entity.create').at(-1); if (created) onSelect(created.entity_id as string) }
  }
  const addImage = async (file: File) => {
    if (!target) return
    try {
      const uploaded = await store.session.uploadAsset(file, file.name)
      const key = store.core.order_key_between(store.outline.childrenOf(target.entity_id).at(-1)?.order_key ?? undefined, undefined)
      const id = randomId('a')
      const outcome = await store.submit({ editId: `new-data:${id}`, label: `导入图片 ${file.name}`, operations: [{ op: 'entity.create', entity_id: id, type_id: 'buckyos.asset-ref', parent_id: target.entity_id, order_key: key, name: file.name, payload: { object_id: uploaded.object_id, file_name: file.name } }] })
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') onSelect(id)
    } catch (error) { store.notify('error', `上传失败：${describeError(error)}`) }
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
    } catch (error) { store.notify('error', `删除预检失败：${describeError(error)}`) }
  }
  const doDelete = async () => {
    if (!entity) return
    const descendants = store.outline.descendants(entity.entity_id)
    const outcome = await store.submit({ editId: `entity:${entity.entity_id}`, label: `删除 ${entityLabel(entity)}`, operations: [{ op: 'entity.delete', entity_id: entity.entity_id, ...(descendants.length ? { subtree: { delete: descendants.map((d) => d.entity_id) } } : {}), expect: { rev: entity.life_rev } }] })
    setDeleting(null)
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') onSelect(null)
  }
  const folders = store.outline.descendants('data').filter((e) => e.type_id === 'buckyos.container' && e.entity_id !== 'shows' && e.entity_id !== entity?.entity_id && !store.outline.ancestors(e.entity_id).includes(entity?.entity_id ?? '\0'))
  const moveTo = (parentId: string) => {
    if (!entity) return
    const key = store.core.order_key_between(store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined)
    void store.submit({ editId: `entity:${entity.entity_id}`, label: `移动 ${entityLabel(entity)}`, operations: [{ op: 'tree.move', entity_id: entity.entity_id, new_parent_id: parentId, order_key: key }] })
  }
  const isSystem = entity && ['data', 'canvas-content', 'shows'].includes(entity.entity_id)
  return (
    <div className="aiws-datatree-actions" data-testid="aiws-datatree-actions">
      {canCreate && (
        <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void create() }}>
          <span className="aiws-muted">在「{target?.entity_id === 'data' ? '数据树第一层' : entityLabel(target!)}」新建</span>
          <select aria-label="新建类型" data-testid="aiws-new-kind" value={kind} onChange={(event) => setKind(event.target.value as NewDataKind)}>
            <option value="table">表格</option><option value="richtext">富文本</option><option value="record">记录</option><option value="wish">许愿格</option><option value="note">便签</option><option value="folder">文件夹</option>
          </select>
          <input aria-label="标题" placeholder="标题（可选）" value={title} onChange={(event) => setTitle(event.target.value)} />
          <button type="submit" data-testid="aiws-new-data">新建</button>
          <label className="aiws-link">图片…<input type="file" hidden accept="image/*" data-testid="aiws-new-image" onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void addImage(file) }} /></label>
        </form>
      )}
      {entity && !isSystem && entity.system !== 'surface_content' && (
        <div className="aiws-inline-form">
          {entity.capabilities.includes('structure') && folders.length > 0 && (
            <select aria-label="移动到" value="" data-testid="aiws-move-to" onChange={(event) => { if (event.target.value) moveTo(event.target.value) }}>
              <option value="">移动到…</option>
              {folders.filter((f) => f.entity_id !== entity.parent_id).map((f) => <option key={f.entity_id} value={f.entity_id}>{f.entity_id === 'canvas-content' ? '画布内容区' : entityLabel(f)}</option>)}
            </select>
          )}
          {entity.capabilities.includes('delete') && <button type="button" data-testid="aiws-delete-entity" onClick={() => { void precheck() }}>删除…</button>}
        </div>
      )}
      {deleting && entity && (
        <div className="aiws-dialog" role="alertdialog" aria-label="删除数据" data-testid="aiws-delete-dialog">
          <b>删除「{entityLabel(entity)}」{store.outline.descendants(entity.entity_id).length > 0 ? `（连同 ${store.outline.descendants(entity.entity_id).length} 个子对象）` : ''}</b>
          {deleting.referrers.length > 0 && (
            <div className="aiws-warning" data-testid="aiws-delete-blocked">仍被引用，不能删除：{deleting.referrers.map((r) => `${entityLabel(store.outline.get(r.entity_id) ?? { entity_id: r.entity_id } as EntityEnvelope)}`).join('、')}{deleting.hidden ? '；另有无权查看的引用' : ''}</div>
          )}
          {deleting.referrers.length === 0 && deleting.hidden && <div className="aiws-warning" data-testid="aiws-delete-blocked">存在无权查看的引用，不能删除。</div>}
          <div className="aiws-inline-form">
            {deleting.referrers.length === 0 && !deleting.hidden && <button type="button" data-testid="aiws-delete-confirm" onClick={() => { void doDelete() }}>确认删除</button>}
            <button type="button" onClick={() => setDeleting(null)}>取消</button>
          </div>
        </div>
      )}
    </div>
  )
}
