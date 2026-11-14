/* The operation record of the permissions (R19, §8.5; `acl.audit`, managers only): newest first — when, who (and as
 * whom an Agent was judged), what changed, on which object, before and after. Made for the data mode's right sidebar
 * ("修改记录" while the permission editor is the tab); `entityId` narrows it to the records naming one object. */

import { useCallback, useMemo, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import type { AclAuditItem, Capability, Json } from '../../api/types'
import { useLoad } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useAcl, useAclRev, useWorkspaceCaps, field } from './aclData'
import { CapTags } from './CapabilityEditor'
import { useLabels, type Labels } from './labels'

export interface AclAuditListProps {
  /** Only the records naming this entity (as a resource, a wish or a moved object). */
  entityId?: string | null
  /** How many records are shown before "show more" (default 50). */
  pageSize?: number
}

const KNOWN = new Set(['acl.set', 'acl.create_group', 'acl.update_group', 'acl.delete_group', 'acl.members', 'acl.set_anonymous', 'acl.bind_macro', 'acl.load_design',
  'entry.create', 'entry.close', 'entry.join', 'publish', 'unpublish', 'visibility.widened', 'macro.changed', 'hosted_exec.set', 'workspace.delete', 'session.link_member', 'session.revoke_link'])

function mentions(item: AclAuditItem, entityId: string): boolean {
  return JSON.stringify(item.target ?? null).includes(`"${entityId}"`)
}

function caps(value: Json | undefined): Capability[] {
  return Array.isArray(value) ? (value.filter((c) => typeof c === 'string') as Capability[]) : []
}

/** What one record is about, in words. */
function Summary({ item, labels }: { item: AclAuditItem; labels: Labels }) {
  const { t, list } = useAiwsText()
  const s = (v: Json | undefined) => (typeof v === 'string' ? v : '')
  const group = (v: Json | undefined) => labels.subject(`@${s(v)}`)
  switch (item.action) {
    case 'acl.set': {
      const after = Array.isArray(item.after) ? item.after : []
      const before = Array.isArray(item.before) ? item.before : []
      return (
        <ul className="aiws-perm-audit-changes">
          {after.map((a, i) => {
            const allow = caps(field(a, 'allow'))
            const deny = caps(field(a, 'deny'))
            const was = before[i]
            return (
              <li key={i}>
                {labels.subject(s(field(a, 'subject')))} · <code>{s(field(a, 'resource', 'path'))}</code>{' '}
                {allow.length === 0 && deny.length === 0 ? <span className="aiws-muted">{t('aiworkspace.perm.audit.removed')}</span> : <CapTags allow={allow} deny={deny} />}
                {was && typeof was === 'object' && <span className="aiws-muted"> {t('aiworkspace.perm.audit.was')} <CapTags allow={caps(field(was, 'allow'))} deny={caps(field(was, 'deny'))} /></span>}
              </li>
            )
          })}
        </ul>
      )
    }
    case 'acl.create_group': case 'acl.update_group': case 'acl.delete_group':
      return <span>{group(field(item.target, 'group_id'))}</span>
    case 'acl.members': {
      const added = (Array.isArray(field(item.after, 'added')) ? field(item.after, 'added') as Json[] : []).map((m) => labels.member(s(m)))
      const removed = (Array.isArray(field(item.before, 'removed')) ? field(item.before, 'removed') as Json[] : []).map((m) => labels.member(s(m)))
      return <span>{group(field(item.target, 'group_id'))}{added.length > 0 && <> · {t('aiworkspace.perm.audit.added', { names: list(added) })}</>}{removed.length > 0 && <> · {t('aiworkspace.perm.audit.removedMembers', { names: list(removed) })}</>}</span>
    }
    case 'acl.set_anonymous':
      return <span>{item.after === true ? t('aiworkspace.perm.audit.anonymousOn') : t('aiworkspace.perm.audit.anonymousOff')}</span>
    case 'acl.bind_macro': {
      const wish = s(field(item.target, 'wish', 'path'))
      return <span><code>{wish}</code> · {item.after ? t('aiworkspace.perm.audit.bound', { group: group(field(item.after, 'group_id')), mode: t(`aiworkspace.perm.macro.mode.${s(field(item.after, 'mode')) || 'union'}`) }) : t('aiworkspace.perm.audit.unbound')}</span>
    }
    case 'entry.create': case 'entry.close':
      return <span>{group(field(item.target, 'group_id') ?? field(item.after, 'group_id'))}</span>
    case 'entry.join': case 'session.link_member':
      return <span>{labels.member(s(field(item.after, 'member')))} → {group(field(item.target, 'group_id'))}</span>
    case 'publish': case 'unpublish': case 'visibility.widened': {
      const id = s(field(item.target, 'entity_id'))
      const e = labels.entity(id)
      return <span>{e ? labels.resource({ entity_id: id, path: id }) : id}</span>
    }
    case 'macro.changed': {
      const id = s(field(item.target, 'wish_id'))
      return <span>{labels.entity(id) ? labels.resource({ entity_id: id, path: id }) : id}</span>
    }
    default:
      return null
  }
}

export function AclAuditList({ entityId = null, pageSize = 50 }: AclAuditListProps) {
  const { t, dateTime } = useAiwsText()
  const labels = useLabels()
  const { api } = useAcl()
  const { capabilities } = useWorkspaceCaps()
  const manage = capabilities.includes('manage')
  // records outside permission changes (publications, moves, hosted execution) do not move the revision: "refresh"
  const rev = useAclRev()
  const [tick, setTick] = useState(0)
  const load = useCallback(() => (manage ? api.audit() : Promise.resolve([] as AclAuditItem[])), [api, manage])
  const read = useLoad(load, `${rev}:${tick}`)
  const [shown, setShown] = useState(pageSize)
  const items = useMemo(() => [...(read.data ?? [])].reverse().filter((i) => !entityId || mentions(i, entityId)), [read.data, entityId])
  if (!manage) return <div className="aiws-muted" data-testid="aiws-perm-audit-denied">{t('aiworkspace.perm.audit.managersOnly')}</div>
  return (
    <section className="aiws-perm-audit" data-testid="aiws-perm-audit">
      <div className="aiws-perm-audit-line">
        <span className="aiws-panel-title">{t('aiworkspace.perm.audit.title')}</span>
        <span className="aiws-grow" />
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.perm.audit.refresh')} title={t('aiworkspace.perm.audit.refresh')} data-testid="aiws-perm-audit-refresh" onClick={() => setTick((n) => n + 1)}><RefreshCw size={16} /></button>
      </div>
      {read.error && <div className="aiws-error" role="alert">{read.error}</div>}
      {read.loading && !read.data && <div className="aiws-muted">{t('aiworkspace.perm.loading')}</div>}
      {read.data && items.length === 0 && <div className="aiws-muted" data-testid="aiws-perm-audit-empty">{t('aiworkspace.perm.audit.none')}</div>}
      <ol className="aiws-perm-audit-list">
        {items.slice(0, shown).map((item) => (
          <li key={item.seq} className="aiws-perm-audit-item" data-testid="aiws-perm-audit-item" data-action={item.action} data-seq={item.seq}>
            <div className="aiws-perm-audit-line">
              <b>{KNOWN.has(item.action) ? t(`aiworkspace.perm.audit.action.${item.action}`) : item.action}</b>
              <span className="aiws-grow" />
              <span className="aiws-muted">{dateTime(item.at)}</span>
            </div>
            <div className="aiws-muted">{t('aiworkspace.perm.audit.by', { who: labels.member(item.actor) })}{item.judged_as ? ` ${t('aiworkspace.perm.audit.judgedAs', { who: labels.member(item.judged_as) })}` : ''}</div>
            <div className="aiws-perm-audit-summary"><Summary item={item} labels={labels} /></div>
          </li>
        ))}
      </ol>
      {items.length > shown && <button type="button" className="aiws-link" data-testid="aiws-perm-audit-more" onClick={() => setShown((n) => n + pageSize)}>{t('aiworkspace.perm.audit.more', { count: items.length - shown })}</button>}
    </section>
  )
}
