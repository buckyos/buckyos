/* The right column of the data-source view (phase two §6.1): type, stable identity, versions,
 * dependency record, permissions and write lock, lookup name, write policy, and the version
 * history with restore (an ordinary, undoable commit). Every editable item maps to a command. */

import { useCallback, useState } from 'react'
import { describeError } from '../../api/session'
import type { EntityEnvelope, VersionInfo } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { TYPE_LABEL } from './dataOps'
import { FreshnessBadge } from './FreshnessBadge'

export function PropertiesPanel({ entity }: { entity: EntityEnvelope }) {
  const { t, list } = useAiwsText()
  const store = useStore()
  const [name, setName] = useState(entity.name ?? '')
  const can = (capability: string) => entity.capabilities.includes(capability as never)
  const isSystem = ['root', 'data', 'surfaces', 'canvas-content', 'shows'].includes(entity.entity_id)
  const submit = (label: string, operations: Parameters<typeof store.submit>[0]['operations']) => store.submit({ editId: `entity:${entity.entity_id}`, label, operations })
  return (
    <div className="aiws-properties" data-testid="aiws-properties" data-entity-id={entity.entity_id}>
      <div className="aiws-panel-title">{t('aiworkspace.data.props.title')}</div>
      <table className="aiws-props">
        <tbody>
          <tr><th>{t('aiworkspace.data.props.type')}</th><td>{TYPE_LABEL[entity.type_id] ?? entity.type_id}{entity.kind ? ` · ${entity.kind}` : ''} v{entity.schema_version}{entity.degraded ? <span className="aiws-chip aiws-chip-warn">{entity.degraded}</span> : null}</td></tr>
          <tr><th>{t('aiworkspace.data.props.identity')}</th><td><code>{entity.entity_id}</code></td></tr>
          <tr><th>{t('aiworkspace.data.props.revisions')}</th><td>{entity.struct_rev !== undefined
            ? t('aiworkspace.data.props.revsStruct', { content: entity.content_rev, meta: entity.meta_rev, life: entity.life_rev, struct: entity.struct_rev })
            : t('aiworkspace.data.props.revs', { content: entity.content_rev, meta: entity.meta_rev, life: entity.life_rev })}</td></tr>
          <tr><th>{t('aiworkspace.data.props.position')}</th><td>{entity.parent_id ?? '—'}{entity.order_key ? ` · ${entity.order_key}` : ''}</td></tr>
          <tr><th>{t('aiworkspace.data.props.myCaps')}</th><td data-testid="aiws-props-caps">{list(entity.capabilities) || t('aiworkspace.data.props.noCaps')}</td></tr>
          <tr><th>{t('aiworkspace.data.props.writePolicy')}</th><td>{entity.write_policy === 'lock_required' ? t('aiworkspace.data.props.policyLocked') : t('aiworkspace.data.props.policyOpen')}{entity.lock_holder ? t('aiworkspace.data.props.lockHeld', { principal: entity.lock_holder.principal }) : ''}</td></tr>
          {entity.derived && (
            <tr><th>{t('aiworkspace.data.props.derived')}</th><td data-testid="aiws-props-derived">
              <FreshnessBadge entityId={entity.entity_id} />
              <div className="aiws-muted">{t(entity.derived.simulated ? 'aiworkspace.data.props.derivedLineSimulated' : 'aiworkspace.data.props.derivedLine', { wish: entity.derived.wish_id, run: entity.derived.run_id, executor: entity.derived.executor, rev: entity.derived.generated_rev })}</div>
              <div className="aiws-muted">{t('aiworkspace.data.props.readSet', {
                inputs: entity.derived.inputs.map((i) => `${i.entity_id}${i.selector ? `/${i.selector.kind}${i.selector.field_id ? `:${i.selector.field_id}` : ''}` : ''}@${i.version.rev ?? i.version.hash ?? '?'}${i.version.mode === 'fixed' ? t('aiworkspace.data.props.fixed') : ''}`)
                  .join(t('aiworkspace.data.props.readSetSeparator')),
              })}</div>
            </td></tr>
          )}
        </tbody>
      </table>
      {!isSystem && can('structure') && (
        <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void submit(t('aiworkspace.data.props.renameEdit', { id: entity.entity_id }), [{ op: 'entity.rename', entity_id: entity.entity_id, name: name.trim() === '' ? null : name.trim(), expect: { rev: entity.meta_rev } }]) }}>
          <input aria-label={t('aiworkspace.data.props.lookupName')} placeholder={t('aiworkspace.data.props.lookupNamePlaceholder')} value={name} onChange={(event) => setName(event.target.value)} />
          <button type="submit" data-testid="aiws-rename">{t('aiworkspace.data.rename')}</button>
        </form>
      )}
      {!isSystem && can('manage') && (
        <div className="aiws-inline-form" data-testid="aiws-lock-admin">
          <button type="button" data-testid="aiws-toggle-policy" onClick={() => {
            const policy = entity.write_policy === 'lock_required' ? 'open' : 'lock_required'
            void submit(t('aiworkspace.data.props.policyEdit', { policy }), [{ op: 'entity.set_write_policy', entity_id: entity.entity_id, policy, expect: { rev: entity.meta_rev } }])
          }}>{entity.write_policy === 'lock_required' ? t('aiworkspace.data.props.makeOpen') : t('aiworkspace.data.props.requireLock')}</button>
          {entity.lock_holder && (
            <button type="button" data-testid="aiws-break-lock" onClick={() => {
              store.session.lockBreak(entity.entity_id).then(() => { store.notify('info', t('aiworkspace.data.props.lockBroken')); store.versions.bump(['outline']); void store.outline.reload() }, (error: unknown) => store.notify('error', t('aiworkspace.data.props.lockBreakFailed', { error: describeError(error) })))
            }}>{t('aiworkspace.data.props.breakLock', { principal: entity.lock_holder.principal })}</button>
          )}
        </div>
      )}
      {!isSystem && entity.type_id !== 'buckyos.container' && entity.type_id !== 'buckyos.cell' && <VersionHistory entity={entity} />}
    </div>
  )
}

function VersionHistory({ entity }: { entity: EntityEnvelope }) {
  const { t, dateTime } = useAiwsText()
  const store = useStore()
  const [open, setOpen] = useState(false)
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(async () => (open ? store.session.listVersions(entity.entity_id) : null), [store, entity.entity_id, open])
  const read = useLoad<{ versions: VersionInfo[]; content_rev: number } | null>(load, version)
  const restore = async (v: VersionInfo) => {
    try {
      const plan = await store.session.restoreVersionPlan(entity.entity_id, v.content_rev)
      const outcome = await store.submit({ editId: `restore:${entity.entity_id}`, label: t('aiworkspace.data.props.restoreEdit', { rev: v.content_rev }), operations: plan.operations })
      if (outcome.status === 'accepted') store.notify('info', t('aiworkspace.data.props.restored', { rev: v.content_rev }))
      else if (outcome.status !== 'saved_locally') store.notify('error', t('aiworkspace.data.props.restoreRejected', { code: 'code' in outcome ? outcome.code : outcome.status }))
    } catch (error) { store.notify('error', t('aiworkspace.data.props.restoreFailed', { error: describeError(error) })) }
  }
  return (
    <div className="aiws-versions" data-testid="aiws-versions">
      <button type="button" className="aiws-link" data-testid="aiws-versions-toggle" onClick={() => setOpen((o) => !o)}>{t('aiworkspace.data.props.history')} {open ? '▾' : '▸'}</button>
      {open && read.error && <div className="aiws-error">{read.error}</div>}
      {open && read.data && read.data.versions.length === 0 && <div className="aiws-muted">{t('aiworkspace.data.props.noVersions')}</div>}
      {open && read.data && read.data.versions.map((v) => (
        <div key={v.content_rev} className="aiws-version" data-testid="aiws-version" data-rev={v.content_rev}>
          <span>{v.content_rev === read.data?.content_rev ? t('aiworkspace.data.props.versionCurrent', { rev: v.content_rev }) : t('aiworkspace.data.props.version', { rev: v.content_rev })}</span>
          <span className="aiws-muted"> {v.kind === 'generated' ? t('aiworkspace.data.props.generated') : t('aiworkspace.data.props.checkpoint')}{v.derived ? t('aiworkspace.data.props.run', { run: v.derived.run_id }) : ''}{v.accepted_at ? ` · ${dateTime(v.accepted_at)}` : ''}{v.author ? ` · ${v.author}` : ''}</span>
          {v.content_rev !== read.data?.content_rev && entity.capabilities.includes('update') && <button type="button" className="aiws-link" data-testid={`aiws-restore-${v.content_rev}`} onClick={() => { void restore(v) }}>{t('aiworkspace.data.props.restore')}</button>}
        </div>
      ))}
    </div>
  )
}
