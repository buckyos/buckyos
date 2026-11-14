/* The right sidebar of the data mode (UI 框架 PRD §5.4), following the current tab's object: 属性 (folding sections —
 * basics, source and sync, publication, generation and freshness, references and dependencies, write policy and lock,
 * diagnostics folded — their folding per type in the person's state), 权限 (the by-entity view, §8.4; a macro's
 * identity too) and 修改记录 (the object's versions; restoring one is an ordinary, undoable Commit). With the Workspace's
 * permission editor as the tab only 修改记录 is there: the permissions' operation record. */

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, Copy } from 'lucide-react'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Json, VersionInfo } from '../../api/types'
import { useLoad, useStore, useUserState, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { AclAuditList, EntityPermissions, MacroIdentity } from '../permissions'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { RelationsPanel } from '../sources/RelationsPanel'
import { PublicationSection, SourceSection } from '../sources/SourcePanel'
import type { PropSection, SideTab } from './actions'
import { nameOf, pathText, typeName } from './model'
import { useObjectSize } from './size'
import { PERMISSIONS_TAB } from './tabs'

export function SideContent({ id, entity, tab, focus }: { id: string | null; entity: EntityEnvelope | undefined; tab: SideTab; focus: { section: PropSection; nonce: number } | null }) {
  const { t } = useAiwsText()
  if (id === PERMISSIONS_TAB) return <AclAuditList />
  if (!id || !entity || entity.deleted) return <div className="aiws-muted aiws-dm-side-empty" data-testid="aiws-dm-side-empty">{t('aiworkspace.datamode.side.none')}</div>
  if (tab === 'permissions') {
    return (
      <div className="aiws-dm-side-perm" data-testid="aiws-dm-side-permissions">
        <EntityPermissions entityId={entity.entity_id} />
        {entity.type_id === 'buckyos.wish' && <MacroIdentity wishId={entity.entity_id} />}
      </div>
    )
  }
  if (tab === 'history') return <History key={entity.entity_id} entity={entity} />
  return <Properties key={entity.entity_id} entity={entity} focus={focus} />
}

// ---- 属性

function Section({ id, type, title, defaultOpen = true, focus, children, testId }: { id: PropSection; type: string; title: string; defaultOpen?: boolean; focus: { section: PropSection; nonce: number } | null; children: ReactNode; testId?: string }) {
  const store = useStore()
  const key = `ui:data-props:${type}`
  const folded = useUserState<Record<string, boolean>>(key)
  const ref = useRef<HTMLElement>(null)
  const stored = folded?.[id]
  const open = stored === undefined ? defaultOpen : !stored
  const set = (value: boolean) => store.userState.set(key, { ...(folded ?? {}), [id]: !value } as Json)
  // a menu asked for this section ("发布…", "查看引用与依赖"): unfold it and bring it into view
  const asked = focus?.section === id ? focus.nonce : 0
  useEffect(() => {
    if (!asked) return
    if (!open) store.userState.set(key, { ...(folded ?? {}), [id]: false } as Json)
    window.setTimeout(() => ref.current?.scrollIntoView({ block: 'nearest' }), 0)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per request
  }, [asked])
  return (
    <section ref={ref} className="aiws-dm-section" data-testid={testId ?? `aiws-dm-section-${id}`} data-open={open ? 'true' : 'false'}>
      <button type="button" className="aiws-dm-section-head" aria-expanded={open} data-testid={`aiws-dm-section-toggle-${id}`} onClick={() => set(!open)}>
        {open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}<span>{title}</span>
      </button>
      {open && <div className="aiws-dm-section-body">{children}</div>}
    </section>
  )
}

const PUBLISHABLE = new Set(['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.dir'])
const SYSTEM = new Set(['root', 'data', 'surfaces', 'canvas-content', 'shows'])

function Properties({ entity, focus }: { entity: EntityEnvelope; focus: { section: PropSection; nonce: number } | null }) {
  const { t, list } = useAiwsText()
  const store = useStore()
  // the lookup name being typed (the stored one until then)
  const [typed, setTyped] = useState<string | null>(null)
  const name = typed ?? entity.name ?? ''
  const setName = setTyped
  const can = (capability: string) => entity.capabilities.includes(capability as never)
  const isSystem = SYSTEM.has(entity.entity_id)
  const type = entity.type_id
  const size = useObjectSize(entity)
  const path = pathText(store, entity.entity_id)
  const submit = (label: string, operations: Parameters<typeof store.submit>[0]['operations']) => store.submit({ editId: `entity:${entity.entity_id}`, label, operations })
  const copy = async () => {
    try { await navigator.clipboard.writeText(path); store.notify('info', t('aiworkspace.datamode.pathCopied', { path })) } catch { store.notify('info', t('aiworkspace.datamode.pathNotCopied', { path })) }
  }
  const sectionProps = { type, focus }
  return (
    <div className="aiws-properties aiws-dm-props" data-testid="aiws-properties" data-entity-id={entity.entity_id}>
      <Section id="basic" title={t('aiworkspace.datamode.section.basic')} {...sectionProps}>
        <table className="aiws-props">
          <tbody>
            <tr><th>{t('aiworkspace.datamode.prop.name')}</th><td data-testid="aiws-dm-prop-name">{nameOf(store, entity, entity.entity_id)}</td></tr>
            <tr><th>{t('aiworkspace.data.props.type')}</th><td data-testid="aiws-dm-prop-type">{typeName(entity)}{entity.degraded ? <span className="aiws-chip aiws-chip-warn">{entity.degraded}</span> : null}</td></tr>
            <tr><th>{t('aiworkspace.datamode.prop.path')}</th><td><code data-testid="aiws-dm-prop-path">{path}</code> <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.datamode.copyPath')} title={t('aiworkspace.datamode.copyPath')} onClick={() => { void copy() }}><Copy size={13} /></button></td></tr>
            {size && <tr><th>{size.kind === 'rows' ? t('aiworkspace.datamode.prop.rows') : size.kind === 'members' ? t('aiworkspace.datamode.prop.members') : t('aiworkspace.datamode.prop.size')}</th><td data-testid="aiws-dm-prop-size">{size.text}</td></tr>}
          </tbody>
        </table>
        {!isSystem && can('structure') ? (
          <form className="aiws-inline-form" onSubmit={(event) => { event.preventDefault(); void submit(t('aiworkspace.data.props.renameEdit', { id: entity.entity_id }), [{ op: 'entity.rename', entity_id: entity.entity_id, name: name.trim() === '' ? null : name.trim(), expect: { rev: entity.meta_rev } }]).then(() => setTyped(null)) }}>
            <label className="aiws-dm-field">{t('aiworkspace.data.props.lookupName')}
              <input aria-label={t('aiworkspace.data.props.lookupName')} placeholder={t('aiworkspace.data.props.lookupNamePlaceholder')} value={name} onChange={(event) => setName(event.target.value)} />
            </label>
            <button type="submit" data-testid="aiws-rename">{t('aiworkspace.data.rename')}</button>
          </form>
        ) : <div className="aiws-muted">{t('aiworkspace.data.props.lookupName')}: {entity.name ?? '—'}</div>}
      </Section>
      {entity.source && (
        <Section id="source" title={t('aiworkspace.datamode.section.source')} {...sectionProps}><SourceSection entity={entity} /></Section>
      )}
      {PUBLISHABLE.has(type) && !entity.source && (
        <Section id="publish" title={t('aiworkspace.datamode.section.publish')} {...sectionProps}><PublicationSection entity={entity} /></Section>
      )}
      {(entity.derived || type === 'buckyos.wish') && (
        <Section id="derived" title={t('aiworkspace.datamode.section.derived')} {...sectionProps}>
          <div data-testid="aiws-props-derived">
            <FreshnessBadge entityId={entity.entity_id} />
            {entity.derived && <div className="aiws-muted">{t(entity.derived.simulated ? 'aiworkspace.data.props.derivedLineSimulated' : 'aiworkspace.data.props.derivedLine', { wish: entity.derived.wish_id, run: entity.derived.run_id, executor: entity.derived.executor, rev: entity.derived.generated_rev })}</div>}
            {entity.derived && <div className="aiws-muted">{t('aiworkspace.data.props.readSet', {
              inputs: entity.derived.inputs.map((i) => `${i.entity_id}${i.selector ? `/${i.selector.kind}${i.selector.field_id ? `:${i.selector.field_id}` : ''}` : ''}@${i.version.rev ?? i.version.hash ?? '?'}${i.version.mode === 'fixed' ? t('aiworkspace.data.props.fixed') : ''}`)
                .join(t('aiworkspace.data.props.readSetSeparator')),
            })}</div>}
          </div>
        </Section>
      )}
      {!isSystem && (
        <Section id="relations" title={t('aiworkspace.datamode.section.relations')} {...sectionProps}><RelationsPanel entityId={entity.entity_id} /></Section>
      )}
      {!isSystem && (
        <Section id="policy" title={t('aiworkspace.datamode.section.policy')} {...sectionProps}>
          <table className="aiws-props">
            <tbody>
              <tr><th>{t('aiworkspace.data.props.writePolicy')}</th><td data-testid="aiws-dm-prop-policy">{entity.write_policy === 'lock_required' ? t('aiworkspace.data.props.policyLocked') : t('aiworkspace.data.props.policyOpen')}{entity.lock_holder ? t('aiworkspace.data.props.lockHeld', { principal: entity.lock_holder.principal }) : ''}</td></tr>
              <tr><th>{t('aiworkspace.data.props.myCaps')}</th><td data-testid="aiws-props-caps">{list(entity.capabilities) || t('aiworkspace.data.props.noCaps')}</td></tr>
            </tbody>
          </table>
          {can('manage') && (
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
        </Section>
      )}
      <Section id="diagnostics" title={t('aiworkspace.datamode.section.diagnostics')} defaultOpen={false} {...sectionProps}>
        <table className="aiws-props">
          <tbody>
            <tr><th>{t('aiworkspace.data.props.identity')}</th><td><code data-testid="aiws-dm-prop-id">{entity.entity_id}</code></td></tr>
            <tr><th>{t('aiworkspace.datamode.prop.schema')}</th><td>{entity.type_id}{entity.kind ? ` · ${entity.kind}` : ''} v{entity.schema_version}</td></tr>
            <tr><th>{t('aiworkspace.data.props.revisions')}</th><td>{entity.struct_rev !== undefined
              ? t('aiworkspace.data.props.revsStruct', { content: entity.content_rev, meta: entity.meta_rev, life: entity.life_rev, struct: entity.struct_rev })
              : t('aiworkspace.data.props.revs', { content: entity.content_rev, meta: entity.meta_rev, life: entity.life_rev })}</td></tr>
            <tr><th>{t('aiworkspace.data.props.position')}</th><td>{entity.parent_id ?? '—'}{entity.order_key ? ` · ${entity.order_key}` : ''}</td></tr>
          </tbody>
        </table>
      </Section>
    </div>
  )
}

// ---- 修改记录

function originText(t: (key: string) => string, v: VersionInfo): string {
  if (v.run_id || v.derived || v.origin === 'program') return t('aiworkspace.datamode.origin.macro')
  switch (v.origin) {
    case 'agent': return t('aiworkspace.datamode.origin.agent')
    case 'import': return t('aiworkspace.datamode.origin.import')
    case 'source': return t('aiworkspace.datamode.origin.source')
    case 'system': return t('aiworkspace.datamode.origin.system')
    default: return t('aiworkspace.datamode.origin.human')
  }
}

function History({ entity }: { entity: EntityEnvelope }) {
  const { t, dateTime } = useAiwsText()
  const store = useStore()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.listVersions(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<{ versions: VersionInfo[]; content_rev: number }>(load, version)
  const versionable = entity.type_id !== 'buckyos.container' && entity.type_id !== 'buckyos.cell'
  const restore = async (v: VersionInfo) => {
    try {
      const plan = await store.session.restoreVersionPlan(entity.entity_id, v.content_rev)
      const outcome = await store.submit({ editId: `restore:${entity.entity_id}`, label: t('aiworkspace.data.props.restoreEdit', { rev: v.content_rev }), operations: plan.operations })
      if (outcome.status === 'accepted') store.notify('info', t('aiworkspace.data.props.restored', { rev: v.content_rev }))
      else if (outcome.status !== 'saved_locally') store.notify('error', t('aiworkspace.data.props.restoreRejected', { code: 'code' in outcome ? outcome.code : outcome.status }))
    } catch (error) { store.notify('error', t('aiworkspace.data.props.restoreFailed', { error: describeError(error) })) }
  }
  if (!versionable) return <div className="aiws-muted" data-testid="aiws-versions">{t('aiworkspace.datamode.history.none')}</div>
  const versions = read.data ? [...read.data.versions].sort((a, b) => b.content_rev - a.content_rev) : []
  return (
    <div className="aiws-versions aiws-dm-history" data-testid="aiws-versions">
      {read.error && <div className="aiws-error" role="alert">{read.error}</div>}
      {!read.data && !read.error && <div className="aiws-muted">{t('aiworkspace.blocks.loading')}</div>}
      {read.data && versions.length === 0 && <div className="aiws-muted">{t('aiworkspace.data.props.noVersions')}</div>}
      {versions.map((v) => {
        const current = v.content_rev === read.data?.content_rev
        return (
          <div key={v.content_rev} className="aiws-version aiws-dm-version" data-testid="aiws-version" data-rev={v.content_rev} data-current={current ? 'true' : 'false'}>
            <div className="aiws-dm-version-line">
              <b>{current ? t('aiworkspace.data.props.versionCurrent', { rev: v.content_rev }) : t('aiworkspace.data.props.version', { rev: v.content_rev })}</b>
              <span className="aiws-chip">{originText(t, v)}</span>
              <span className="aiws-grow" />
              {!current && entity.capabilities.includes('update') && <button type="button" className="aiws-link" data-testid={`aiws-restore-${v.content_rev}`} onClick={() => { void restore(v) }}>{t('aiworkspace.data.props.restore')}</button>}
            </div>
            <div className="aiws-muted">{dateTime(v.accepted_at ?? v.created_at)}{v.author ? ` · ${v.author}` : ''}{v.kind === 'generated' ? ` · ${t('aiworkspace.data.props.generated')}` : ''}</div>
            {v.message && <div className="aiws-dm-version-message">{v.message}</div>}
          </div>
        )
      })}
    </div>
  )
}
