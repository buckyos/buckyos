/* The right column of the data-source view (phase two §6.1): type, stable identity, versions,
 * dependency record, permissions and write lock, lookup name, write policy, and the version
 * history with restore (an ordinary, undoable commit). Every editable item maps to a command. */

import { useCallback, useState } from 'react'
import { describeError } from '../../api/session'
import type { EntityEnvelope, VersionInfo } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { TYPE_LABEL } from './dataOps'
import { FreshnessBadge } from './FreshnessBadge'

export function PropertiesPanel({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const [name, setName] = useState(entity.name ?? '')
  const can = (capability: string) => entity.capabilities.includes(capability as never)
  const isSystem = ['root', 'data', 'surfaces', 'canvas-content', 'shows'].includes(entity.entity_id)
  const submit = (label: string, operations: Parameters<typeof store.submit>[0]['operations']) => store.submit({ editId: `entity:${entity.entity_id}`, label, operations })
  return (
    <div className="aiws-properties" data-testid="aiws-properties" data-entity-id={entity.entity_id}>
      <div className="aiws-panel-title">属性</div>
      <table className="aiws-props">
        <tbody>
          <tr><th>类型</th><td>{TYPE_LABEL[entity.type_id] ?? entity.type_id}{entity.kind ? ` · ${entity.kind}` : ''} v{entity.schema_version}{entity.degraded ? <span className="aiws-chip aiws-chip-warn">{entity.degraded}</span> : null}</td></tr>
          <tr><th>稳定身份</th><td><code>{entity.entity_id}</code></td></tr>
          <tr><th>版本</th><td>内容 {entity.content_rev} · 元数据 {entity.meta_rev} · 生命 {entity.life_rev}{entity.struct_rev !== undefined ? ` · 结构 ${entity.struct_rev}` : ''}</td></tr>
          <tr><th>位置</th><td>{entity.parent_id ?? '—'}{entity.order_key ? ` · ${entity.order_key}` : ''}</td></tr>
          <tr><th>我的权限</th><td data-testid="aiws-props-caps">{entity.capabilities.join('、') || '（无）'}</td></tr>
          <tr><th>写入策略</th><td>{entity.write_policy === 'lock_required' ? '需要写锁' : '开放'}{entity.lock_holder ? ` · ${entity.lock_holder.principal} 持有写锁` : ''}</td></tr>
          {entity.derived && (
            <tr><th>生成依赖</th><td data-testid="aiws-props-derived">
              <FreshnessBadge entityId={entity.entity_id} />
              <div className="aiws-muted">许愿格 {entity.derived.wish_id} · 运行 {entity.derived.run_id} · {entity.derived.executor}{entity.derived.simulated ? '（模拟）' : ''} · 生成版本 {entity.derived.generated_rev}</div>
              <div className="aiws-muted">读集合：{entity.derived.inputs.map((i) => `${i.entity_id}${i.selector ? `/${i.selector.kind}${i.selector.field_id ? `:${i.selector.field_id}` : ''}` : ''}@${i.version.rev ?? i.version.hash ?? '?'}${i.version.mode === 'fixed' ? '（固定）' : ''}`).join('，')}</div>
            </td></tr>
          )}
        </tbody>
      </table>
      {!isSystem && can('structure') && (
        <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void submit(`改名 ${entity.entity_id}`, [{ op: 'entity.rename', entity_id: entity.entity_id, name: name.trim() === '' ? null : name.trim(), expect: { rev: entity.meta_rev } }]) }}>
          <input aria-label="查找名" placeholder="查找名（可留空）" value={name} onChange={(event) => setName(event.target.value)} />
          <button type="submit" data-testid="aiws-rename">改名</button>
        </form>
      )}
      {!isSystem && can('manage') && (
        <div className="aiws-inline-form" data-testid="aiws-lock-admin">
          <button type="button" data-testid="aiws-toggle-policy" onClick={() => {
            const policy = entity.write_policy === 'lock_required' ? 'open' : 'lock_required'
            void submit(`写入策略 → ${policy}`, [{ op: 'entity.set_write_policy', entity_id: entity.entity_id, policy, expect: { rev: entity.meta_rev } }])
          }}>{entity.write_policy === 'lock_required' ? '改为开放' : '启用写锁'}</button>
          {entity.lock_holder && (
            <button type="button" data-testid="aiws-break-lock" onClick={() => {
              store.session.lockBreak(entity.entity_id).then(() => { store.notify('info', `已强制解除写锁。`); store.versions.bump(['outline']); void store.outline.reload() }, (error: unknown) => store.notify('error', `解除写锁失败：${describeError(error)}`))
            }}>强制解除 {entity.lock_holder.principal} 的锁</button>
          )}
        </div>
      )}
      {!isSystem && entity.type_id !== 'buckyos.container' && entity.type_id !== 'buckyos.cell' && <VersionHistory entity={entity} />}
    </div>
  )
}

function VersionHistory({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const [open, setOpen] = useState(false)
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(async () => (open ? store.session.listVersions(entity.entity_id) : null), [store, entity.entity_id, open])
  const read = useLoad<{ versions: VersionInfo[]; content_rev: number } | null>(load, version)
  const restore = async (v: VersionInfo) => {
    try {
      const plan = await store.session.restoreVersionPlan(entity.entity_id, v.content_rev)
      const outcome = await store.submit({ editId: `restore:${entity.entity_id}`, label: `恢复到版本 ${v.content_rev}`, operations: plan.operations })
      if (outcome.status === 'accepted') store.notify('info', `已恢复到版本 ${v.content_rev}（可撤销）。`)
      else if (outcome.status !== 'saved_locally') store.notify('error', `恢复未被接受（${'code' in outcome ? outcome.code : outcome.status}）。`)
    } catch (error) { store.notify('error', `恢复失败：${describeError(error)}`) }
  }
  return (
    <div className="aiws-versions" data-testid="aiws-versions">
      <button type="button" className="aiws-link" data-testid="aiws-versions-toggle" onClick={() => setOpen((o) => !o)}>版本历史 {open ? '▾' : '▸'}</button>
      {open && read.error && <div className="aiws-error">{read.error}</div>}
      {open && read.data && read.data.versions.length === 0 && <div className="aiws-muted">还没有可寻址的版本（生成结果和检查点会登记版本）。</div>}
      {open && read.data && read.data.versions.map((v) => (
        <div key={v.content_rev} className="aiws-version" data-testid="aiws-version" data-rev={v.content_rev}>
          <span>版本 {v.content_rev}{v.content_rev === read.data?.content_rev ? '（当前）' : ''}</span>
          <span className="aiws-muted"> {v.kind === 'generated' ? '生成' : '检查点'}{v.derived ? ` · 运行 ${v.derived.run_id}` : ''}{v.accepted_at ? ` · ${new Date(v.accepted_at).toLocaleString()}` : ''}{v.author ? ` · ${v.author}` : ''}</span>
          {v.content_rev !== read.data?.content_rev && entity.capabilities.includes('update') && <button type="button" className="aiws-link" data-testid={`aiws-restore-${v.content_rev}`} onClick={() => { void restore(v) }}>恢复</button>}
        </div>
      ))}
    </div>
  )
}
