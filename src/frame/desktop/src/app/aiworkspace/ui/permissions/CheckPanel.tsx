/* "Check permissions" (§8.3): a user and a path; each capability allowed or refused and the entry that decided it
 * (`acl.query { by: explain }`), with the fixed order of a decision in words. Also the explanation a view of one's own
 * permissions shows (MyPermissions). */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { AclExplain } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { useAcl } from './aclData'
import { CapName } from './CapabilityEditor'
import { useLabels, type Labels } from './labels'
import { checkResourcePath } from './logic'
import { ResourcePathInput } from './ResourcePathInput'
import { SubjectPicker } from './SubjectPicker'
import { useStore } from '../../state/hooks'

/** The order a decision follows (权限需求 §4.6), as the panel always explains it. */
export function DecisionOrder() {
  const { t } = useAiwsText()
  return <p className="aiws-muted aiws-perm-order" data-testid="aiws-perm-order">{t('aiworkspace.perm.check.order')}</p>
}

function sourceOf(t: (key: string, vars?: Record<string, string | number>) => string, explained: AclExplain, path: string): string {
  if (path === '/') return t('aiworkspace.perm.source.workspace')
  if (path === explained.resource.path) return t('aiworkspace.perm.source.direct')
  return t('aiworkspace.perm.source.inherited', { path })
}

/** Each capability: allowed or refused, by which entry (or the owner, or nothing mentioning it). */
export function ExplainTable({ explained, labels }: { explained: AclExplain; labels: Labels }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-perm-explain" role="table" data-testid="aiws-perm-explain">
      {explained.capabilities.map((d) => (
        <div key={d.capability} className={`aiws-perm-explain-row ${d.allowed ? 'is-allow' : 'is-deny'}`} role="row" data-testid="aiws-perm-explain-cap" data-cap={d.capability} data-allowed={d.allowed ? 'true' : 'false'}>
          <span role="cell" className="aiws-perm-explain-cap"><CapName cap={d.capability} /></span>
          <span role="cell"><span className={`aiws-perm-tag ${d.allowed ? 'is-allow' : 'is-deny'}`}>{d.allowed ? t('aiworkspace.perm.state.allow') : t('aiworkspace.perm.state.deny')}</span></span>
          <span role="cell" className="aiws-muted" data-testid="aiws-perm-explain-by">
            {d.owner ? t('aiworkspace.perm.check.byOwner')
              : d.decided_by ? t(d.decided_by.kind === 'allow' ? 'aiworkspace.perm.check.byAllow' : 'aiworkspace.perm.check.byDeny', { subject: labels.subject(d.decided_by.subject), where: sourceOf(t, explained, d.decided_by.resource.path) })
                : t('aiworkspace.perm.check.byNothing')}
          </span>
        </div>
      ))}
    </div>
  )
}

/** Who the explained caller is judged as: its groups and how it belongs to them, `@guest`, an Agent judged as its owner. */
export function JudgmentSummary({ explained, labels }: { explained: AclExplain; labels: Labels }) {
  const { t, list } = useAiwsText()
  const j = explained.judgment
  const groups = j.groups.map((g) => {
    const via = g.via.map((v) => (v.via_entry ? t('aiworkspace.perm.member.viaEntry', { entry: v.via_entry.split('~').pop()?.slice(-6) ?? v.via_entry }) : v.member === j.judged_as ? t('aiworkspace.perm.check.viaDirect') : labels.member(v.member)))
    return `${g.title || g.group_id}${via.length ? ` (${list(via)})` : ''}`
  })
  return (
    <div className="aiws-perm-judgment" data-testid="aiws-perm-judgment">
      {j.owner && <div>{t('aiworkspace.perm.check.isOwner')}</div>}
      {j.agent_as_owner && <div>{t('aiworkspace.perm.check.agentAsOwner', { owner: labels.member(j.judged_as) })}</div>}
      {!j.owner && <div data-testid="aiws-perm-judgment-groups">{groups.length ? t('aiworkspace.perm.check.groups', { groups: list(groups) }) : t('aiworkspace.perm.check.noGroups')}</div>}
      {j.guest && <div>{t('aiworkspace.perm.check.inGuest')}</div>}
      {!j.member && <div>{t('aiworkspace.perm.check.notMember')}</div>}
    </div>
  )
}

export function CheckPanel() {
  const { t } = useAiwsText()
  const store = useStore()
  const labels = useLabels()
  const { api, online } = useAcl()
  const [principal, setPrincipal] = useState('')
  const [path, setPath] = useState('/')
  const [result, setResult] = useState<AclExplain | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const check = checkResourcePath(store.outline, path)
  const run = async () => {
    if (!check.ok) return
    setBusy(true)
    setFailure(null)
    try { setResult(await api.explain(check.entity?.entity_id ?? '/', principal || undefined)) } catch (error) { setResult(null); setFailure(t('aiworkspace.perm.check.failed', { error: describeError(error) })) } finally { setBusy(false) }
  }
  return (
    <section className="aiws-perm-section" data-testid="aiws-perm-check">
      <div className="aiws-perm-section-head"><h3>{t('aiworkspace.perm.check.title')}</h3></div>
      <DecisionOrder />
      <form className="aiws-perm-form" onSubmit={(event) => { event.preventDefault(); if (online && !busy && check.ok) void run() }}>
        <label className="aiws-perm-field"><span>{t('aiworkspace.perm.check.who')}</span>
          <SubjectPicker value={principal} onChange={setPrincipal} labels={labels} groups={false} testId="aiws-perm-check-who" />
          <span className="aiws-muted">{t('aiworkspace.perm.check.whoHint')}</span>
        </label>
        <div className="aiws-perm-field"><span>{t('aiworkspace.perm.resource')}</span><ResourcePathInput value={path} onChange={setPath} testId="aiws-perm-check-path" /></div>
        <div className="aiws-dialog-actions"><button type="submit" className="is-primary" data-testid="aiws-perm-check-run" disabled={!online || busy || !check.ok}>{t('aiworkspace.perm.check.run')}</button></div>
      </form>
      {failure && <div className="aiws-error" role="alert" data-testid="aiws-perm-check-error">{failure}</div>}
      {result && (
        <div className="aiws-perm-check-result" data-testid="aiws-perm-check-result" data-principal={result.principal}>
          <div><b>{labels.subject(result.principal)}</b> · {labels.resource(result.resource)} <code className="aiws-muted">{result.resource.path}</code></div>
          <JudgmentSummary explained={result} labels={labels} />
          <ExplainTable explained={result} labels={labels} />
        </div>
      )}
    </section>
  )
}
