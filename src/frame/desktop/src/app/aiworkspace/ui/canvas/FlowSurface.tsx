/* A flow-layout Surface (phase two §8.1): Blocks and groups in sibling order, the phase-one flow
 * editor kept on the same BlockHost and mode policy. Edit mode edits; view mode reads; the
 * presentation-edit placeholder is static. Reordering is by buttons; moves are auto-merge. */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { EntityEnvelope } from '../../api/types'
import { orderKeyBetween } from '../../api/wasm'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { BlockHost } from '../blocks/BlockHost'
import { modePolicy, type CanvasMode } from '../blocks/registry'

export function FlowSurface({ surfaceId, mode, selected, onSelect, editing, onEditingChange, onInsert }: {
  surfaceId: string
  mode: CanvasMode
  selected: ReadonlySet<string>
  onSelect: (ids: Set<string>) => void
  editing: string | null
  onEditingChange: (id: string | null) => void
  onInsert: (parentId: string) => void
}) {
  const store = useStore()
  useOutlineVersion()
  const surface = store.outline.get(surfaceId)
  if (!surface) return null
  return (
    <div className="aiws-flow" data-testid="aiws-flow" data-surface-id={surfaceId} data-mode={mode}>
      <h2 className="aiws-page-title">{surface.title ?? surface.name ?? '画布'}</h2>
      <ContainerFlow container={surface} mode={mode} selected={selected} onSelect={onSelect} editing={editing} onEditingChange={onEditingChange} onInsert={onInsert} />
    </div>
  )
}

function ContainerFlow({ container, mode, selected, onSelect, editing, onEditingChange, onInsert }: {
  container: EntityEnvelope
  mode: CanvasMode
  selected: ReadonlySet<string>
  onSelect: (ids: Set<string>) => void
  editing: string | null
  onEditingChange: (id: string | null) => void
  onInsert: (parentId: string) => void
}) {
  const store = useStore()
  const policy = modePolicy(mode)
  const shown = store.outline.childrenOf(container.entity_id).filter((entity) => entity.type_id === 'buckyos.cell' || (entity.type_id === 'buckyos.container' && entity.kind === 'group'))
  return (
    <>
      {shown.map((entity, index) => (
        entity.type_id === 'buckyos.cell'
          ? <FlowBlock key={entity.entity_id} cell={entity} siblings={shown} index={index} mode={mode} selected={selected.has(entity.entity_id)} onSelect={() => policy.select && onSelect(new Set([entity.entity_id]))} editing={editing === entity.entity_id} onEditingChange={onEditingChange} />
          : (
            <section key={entity.entity_id} className={`aiws-group${selected.has(entity.entity_id) ? ' is-selected' : ''}`} data-testid={`aiws-group-${entity.entity_id}`} data-cell-id={entity.entity_id}>
              <header className="aiws-cell-head">
                <button type="button" className="aiws-cell-title" onClick={() => policy.select && onSelect(new Set([entity.entity_id]))}>分组 · {entity.title ?? entity.name ?? entity.entity_id}</button>
                <span className="aiws-grow" />
                {policy.layout && <ReorderButtons entity={entity} siblings={shown} index={index} />}
                {policy.layout && entity.capabilities.includes('delete') && (
                  <button type="button" className="aiws-icon" title="删除分组及其中的 Block" aria-label="删除分组" data-testid={`aiws-delete-group-${entity.entity_id}`} onClick={() => {
                    const subtree = store.outline.descendants(entity.entity_id).map((d) => d.entity_id)
                    void store.submit({ editId: `entity:${entity.entity_id}`, label: `删除分组 ${entity.title ?? entity.name ?? ''}`, operations: [{ op: 'entity.delete', entity_id: entity.entity_id, ...(subtree.length ? { subtree: { delete: subtree } } : {}), expect: { rev: entity.life_rev } }] })
                  }}>×</button>
                )}
              </header>
              <ContainerFlow container={entity} mode={mode} selected={selected} onSelect={onSelect} editing={editing} onEditingChange={onEditingChange} onInsert={onInsert} />
              {policy.insert && entity.capabilities.includes('structure') && <button type="button" className="aiws-link" onClick={() => onInsert(entity.entity_id)}>+ 添加到分组</button>}
            </section>
          )
      ))}
      {container.kind === 'surface' && policy.insert && container.capabilities.includes('structure') && (
        <div className="aiws-add"><button type="button" data-testid={`aiws-add-open-${container.entity_id}`} onClick={() => onInsert(container.entity_id)}>+ 添加到「{container.title ?? container.name ?? '画布'}」</button></div>
      )}
    </>
  )
}

function ReorderButtons({ entity, siblings, index }: { entity: EntityEnvelope; siblings: EntityEnvelope[]; index: number }) {
  const store = useStore()
  if (!entity.capabilities.includes('structure')) return null
  const move = (direction: -1 | 1) => {
    const all = store.outline.childrenOf(entity.parent_id ?? '')
    const target = siblings[index + direction]
    const at = all.findIndex((item) => item.entity_id === target.entity_id)
    const before = direction === -1 ? all[at - 1]?.order_key : all[at]?.order_key
    const after = direction === -1 ? all[at]?.order_key : all[at + 1]?.order_key
    let key: string
    try { key = orderKeyBetween(store.core, before, after) } catch (error) { store.notify('error', `无法生成顺序键：${describeError(error)}`); return }
    void store.submit({ editId: `entity:${entity.entity_id}`, label: `调整顺序 ${entity.title ?? entity.name ?? entity.entity_id}`, operations: [{ op: 'tree.place', entity_id: entity.entity_id, order_key: key }] })
  }
  return (
    <>
      <button type="button" className="aiws-icon" title="上移" aria-label="上移" data-testid={`aiws-up-${entity.entity_id}`} disabled={index === 0} onClick={() => move(-1)}>↑</button>
      <button type="button" className="aiws-icon" title="下移" aria-label="下移" data-testid={`aiws-down-${entity.entity_id}`} disabled={index === siblings.length - 1} onClick={() => move(1)}>↓</button>
    </>
  )
}

function FlowBlock({ cell, siblings, index, mode, selected, onSelect, editing, onEditingChange }: { cell: EntityEnvelope; siblings: EntityEnvelope[]; index: number; mode: CanvasMode; selected: boolean; onSelect: () => void; editing: boolean; onEditingChange: (id: string | null) => void }) {
  const store = useStore()
  const policy = modePolicy(mode)
  const [renaming, setRenaming] = useState<string | null>(null)
  const source = cell.source_id ? store.outline.get(cell.source_id) : undefined
  const title = cell.title ?? (source ? source.name ?? source.title ?? source.entity_id : cell.entity_id)
  const rename = async (next: string) => {
    setRenaming(null)
    if (next === (cell.title ?? '')) return
    const read = await store.session.read<{ key_revs: Record<string, number> }>(cell.entity_id)
    void store.submit({
      editId: `key:${cell.entity_id}:title`, label: `Block 标题 → ${next || '（清除）'}`, mine: next, hasMine: true,
      operations: [next === ''
        ? { op: 'entity.unset_keys', entity_id: cell.entity_id, keys: [{ key: 'title', expect: { rev: read.content.key_revs.title ?? 0 } }] }
        : { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'title', value: next, expect: { rev: read.content.key_revs.title ?? 0 } }] }],
    })
  }
  return (
    <section className={`aiws-cell${selected ? ' is-selected' : ''}`} data-testid={`aiws-cell-frame-${cell.entity_id}`} data-cell-id={cell.entity_id} data-cell-source={cell.source_id ?? undefined} data-view={cell.view_type ?? undefined} data-mode={mode}>
      <header className="aiws-cell-head">
        {renaming !== null ? (
          <form onSubmit={(event) => { event.preventDefault(); void rename(renaming.trim()) }}>
            <input aria-label="Block 标题" autoFocus value={renaming} onChange={(event) => setRenaming(event.target.value)} onBlur={() => setRenaming(null)} />
          </form>
        ) : (
          <button type="button" className="aiws-cell-title" data-testid={`aiws-cell-title-${cell.entity_id}`} onClick={onSelect} onDoubleClick={() => { if (policy.writes && cell.capabilities.includes('update')) setRenaming(cell.title ?? '') }} title={policy.writes ? '单击选中，双击改标题' : '单击选中'}>{title}</button>
        )}
        <span className="aiws-muted">{cell.view_type}</span>
        <span className="aiws-grow" />
        {policy.editContent && <span className="aiws-muted">编辑中</span>}
        {policy.layout && <ReorderButtons entity={cell} siblings={siblings} index={index} />}
        {policy.layout && cell.capabilities.includes('delete') && (
          <button type="button" className="aiws-icon" title="删除此 Block（不删除第一层数据）" aria-label="删除 Block" data-testid={`aiws-delete-block-${cell.entity_id}`} onClick={() => {
            void store.submit({ editId: `entity:${cell.entity_id}`, label: `删除 Block ${title}`, operations: [{ op: 'entity.delete', entity_id: cell.entity_id, expect: { rev: cell.life_rev } }] })
          }}>×</button>
        )}
      </header>
      <div className="aiws-cell-body">
        {/* a flow Surface works as the phase-one flow editor (phase two §8.1): its editors are mounted in edit mode */}
        <BlockHost cellId={cell.entity_id} mode={mode} view="canvas" selected={selected} editorActive={policy.editContent || editing} onActivate={() => onEditingChange(cell.entity_id)} onDeactivate={() => onEditingChange(null)} />
      </div>
    </section>
  )
}
