import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { ArrowLeft, Plus, RefreshCw, ShieldCheck } from 'lucide-react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { fetchCurrentAccount } from '../../api/account'
import { fetchUserGroups, fetchUserList } from '../../api/user_mgr'
import type { PolicyApi } from '../../api/aicc_policy'
import { useViewerIsAdmin } from '../settings/hooks/use-viewer-is-admin'
import { useProviders, useUsageSummary } from './hooks/use-aicc-store'
import { useMyEffective, usePolicyApi } from './hooks/use-policy-api'
import {
  USER_TYPE_SUBJECTS,
  editorModeOf,
  emptyPolicy,
  formatRate,
  limitsValid,
  normalizeBody,
  parseSubject,
  policyOf,
  sameBody,
  sortSubjects,
  splitExactModel,
  summarizeBody,
  type AiccPolicyView,
  type PolicyBody,
  type PolicyDraft,
  type PolicySource,
} from './datamodel/policy'
import { PolicyEditor, type EditorProvider } from './components/policy/PolicyEditor'
import { PolicyBodySummary } from './components/policy/PolicyBodySummary'
import { PolicySaveDialog } from './components/policy/PolicySaveDialog'
import { EffectivePreview } from './components/policy/EffectivePreview'
import { Modal, StatusDot } from './components/policy/PolicyWidgets'
import { buttonStyle, errorText, fieldStyle, formatLocal, panelStyle, useSubjectLabel } from './components/policy/policy-labels'
import { ConfirmDialog } from './components/shared/ConfirmDialog'

/** A Usage row opens a policy focused on its Provider or model: the user's own (`user:`), or the user's policy
 * on its App (`user_app:`). `userId` is the viewer when absent. */
export interface PolicyIntent { userId?: string; appId?: string; provider?: string; model?: string }

interface PoliciesPageProps {
  intent?: PolicyIntent | null
  onIntentConsumed?: () => void
}

interface Selection { subject: string; source: PolicySource }

const key = (selection: Selection) => `${selection.source}\u0000${selection.subject}`

function useBodySummary() {
  const { t, locale } = useI18n()
  return (view: AiccPolicyView | undefined) => {
    if (!view) return t('aiCenter.policy.noPolicy', 'No policy')
    if (!view.policy.enabled) return t('aiCenter.policy.disabled', 'Disabled')
    const summary = summarizeBody(view.policy.body)
    const parts = [
      summary.allProviders ? t('aiCenter.policy.summary.all', 'All Providers') : '',
      summary.coefficient === undefined ? '' : summary.coefficient === 0 ? t('aiCenter.policy.coefficient.free', 'Free') : `×${formatRate(summary.coefficient, locale)}`,
      summary.providers ? t('aiCenter.policy.summary.providers', 'Providers: {{count}}', { count: summary.providers }) : '',
      summary.hiddenProviders ? t('aiCenter.policy.summary.hidden', 'Hidden: {{count}}', { count: summary.hiddenProviders }) : '',
      summary.limits ? t('aiCenter.policy.summary.limits', 'Limits: {{count}}', { count: summary.limits }) : '',
    ].filter(Boolean)
    return parts.length ? parts.join(' · ') : t('aiCenter.policy.summary.empty', 'This policy sets nothing.')
  }
}

function SubjectButton({ selection, active, label, summary, onSelect, sourceNote }: {
  selection: Selection
  active: boolean
  label: string
  summary: string
  onSelect: () => void
  sourceNote?: string
}) {
  return (
    <button
      type="button"
      data-testid="policy-subject"
      data-subject={selection.subject}
      data-source={selection.source}
      onClick={onSelect}
      aria-current={active}
      className="flex w-full flex-col items-start rounded-lg px-3 py-2 text-left"
      style={{
        background: active ? 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))' : 'var(--cp-surface)',
        border: `1px solid ${active ? 'var(--cp-accent)' : 'var(--cp-border)'}`,
      }}
    >
      <span className="w-full truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{label}</span>
      <span className="w-full truncate text-xs" style={{ color: 'var(--cp-muted)' }}>{sourceNote ? `${sourceNote} · ` : ''}{summary}</span>
    </button>
  )
}

function Section({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2 px-1">
        <h3 className="text-xs font-semibold" style={{ color: 'var(--cp-muted)' }}>{title}</h3>
        {action}
      </div>
      {children}
    </div>
  )
}

function AddSubjectDialog({ kind, users, apps, viewerId, isAdmin, onAdd, onClose }: {
  kind: 'user' | 'app' | 'user_app'
  users: string[]
  apps: string[]
  viewerId: string
  isAdmin: boolean
  onAdd: (subject: string) => void
  onClose: () => void
}) {
  const { t } = useI18n()
  const [user, setUser] = useState(isAdmin ? users[0] ?? '' : viewerId)
  const [app, setApp] = useState('')
  const subject = kind === 'user' ? (user ? `user:${user}` : '') : kind === 'app' ? (app.trim() ? `app:${app.trim()}` : '') : (user && app.trim() ? `user_app:${user}/${app.trim()}` : '')
  const title = kind === 'user' ? t('aiCenter.policy.addUser', 'Policy for a user') : kind === 'app' ? t('aiCenter.policy.addApp', 'Policy for an App') : isAdmin ? t('aiCenter.policy.addUserApp', 'Policy for a user’s App') : t('aiCenter.policy.newOwn', 'Limit one of my Apps')
  return (
    <Modal title={title} onClose={onClose} testId="policy-add-subject">
      <div className="flex flex-col gap-3">
        {kind !== 'app' && isAdmin && (
          <label className="flex flex-col gap-1 text-sm">
            <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.user', 'User')}</span>
            <select data-testid="policy-add-user" value={user} onChange={(event) => setUser(event.target.value)} className="min-h-10 rounded-lg px-3" style={fieldStyle}>
              {users.map((item) => <option key={item} value={item}>{item}</option>)}
            </select>
          </label>
        )}
        {kind !== 'user' && (
          <label className="flex flex-col gap-1 text-sm">
            <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.app', 'App or Agent')}</span>
            <input data-testid="policy-add-app" list="policy-add-apps" value={app} onChange={(event) => setApp(event.target.value)} placeholder="aiworkspace" className="min-h-10 rounded-lg px-3" style={fieldStyle} />
            <datalist id="policy-add-apps">{apps.map((item) => <option key={item} value={item} />)}</datalist>
          </label>
        )}
      </div>
      <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <button type="button" onClick={onClose} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
        <button type="button" data-testid="policy-add-confirm" disabled={!subject} onClick={() => onAdd(subject)} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('aiCenter.policy.continue', 'Continue')}</button>
      </div>
    </Modal>
  )
}

interface DetailProps {
  api: PolicyApi
  viewerId: string
  selection: Selection
  stored?: AiccPolicyView
  adminPolicy?: AiccPolicyView
  providers: EditorProvider[]
  focus?: { provider?: string; model?: string }
  users?: string[]
  apps: string[]
  subjectLabel: (subject: string) => string
  generation: number
  onSaved: () => void
}

function PolicyDetail({ api, viewerId, selection, stored, adminPolicy, providers, focus, users, apps, subjectLabel, generation, onSaved }: DetailProps) {
  const { t, locale } = useI18n()
  const base = stored?.policy ?? emptyPolicy(selection.subject, selection.source)
  const editable = stored ? stored.editable : true
  const mode = editorModeOf(selection.subject, selection.source)
  const [body, setBody] = useState<PolicyBody>(() => structuredClone(base.body))
  const [enabled, setEnabled] = useState(base.enabled)
  const [pending, setPending] = useState<PolicyDraft | null>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [editorKey, setEditorKey] = useState(0)
  const dirty = !sameBody(body, base.body) || enabled !== base.enabled || !stored
  const invalidLimits = ![body.limits, ...Object.values(body.providers ?? {}).map((entry) => entry.limits), ...Object.values(body.models ?? {}).map((entry) => entry.limits)].every(limitsValid)
  const hiddenConflict = Object.entries(body.models ?? {}).some(([model, entry]) => entry.visible === true && body.providers?.[splitExactModel(model).provider]?.visible === 'hidden')
  const parts = parseSubject(selection.subject)

  const remove = async () => {
    if (!stored) return
    setDeleting(true)
    setDeleteError(null)
    try {
      await api.deletePolicy(stored.policy.policy_id, stored.policy.version)
      setConfirmDelete(false)
      onSaved()
    } catch (reason) {
      setDeleteError(errorText(reason))
    } finally {
      setDeleting(false)
    }
  }

  return (
    <div className="flex min-w-0 flex-col gap-4" data-testid="policy-detail" data-subject={selection.subject} data-source={selection.source}>
      <header className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{subjectLabel(selection.subject)}</h2>
          <span className="rounded px-1.5 py-0.5 text-xs" style={{ border: '1px solid var(--cp-border)', color: 'var(--cp-muted)' }}>
            {selection.source === 'admin' ? t('aiCenter.policy.source.admin', 'Set by an administrator') : t('aiCenter.policy.source.user', 'Own policy')}
          </span>
          {!stored && <StatusDot tone="muted" label={t('aiCenter.policy.notSaved', 'Not saved yet')} />}
        </div>
        <span className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>
          {selection.subject}
          {stored ? ` · ${t('aiCenter.policy.updated', 'Updated {{time}} by {{user}}', { time: formatLocal(stored.policy.updated_at_ms, locale), user: stored.policy.updated_by })}` : ''}
        </span>
        {parts?.kind === 'app' && <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.appSharedHint', 'Applies to every user’s calls through this App or Agent.')}</span>}
      </header>

      {!editable ? (
        <section className="flex flex-col gap-2 rounded-xl p-4" style={panelStyle}>
          <PolicyBodySummary body={base.body} />
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
            {selection.source === 'admin'
              ? t('aiCenter.policy.readOnlyAdmin', 'Set by an administrator; it applies to you and cannot be changed here.')
              : t('aiCenter.policy.readOnlyUser', 'A user’s own policy; only that user changes it.')}
          </p>
        </section>
      ) : (
        <>
          <label className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
            <input type="checkbox" data-testid="policy-enabled" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
            {t('aiCenter.policy.enabled', 'Enabled')}
          </label>
          <PolicyEditor key={editorKey} mode={mode} body={body} onChange={setBody} providers={providers} focus={focus} />
          {invalidLimits && <p className="text-xs" data-testid="policy-invalid" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.policy.invalidLimits', 'Fix the limits: each needs a valid amount, one per window and unit.')}</p>}
          {hiddenConflict && <p className="text-xs" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.policy.hiddenConflict', 'A model is Visible while its Provider is Hidden in this policy.')}</p>}
          <div className="sticky bottom-0 flex flex-wrap items-center justify-end gap-2 py-2" style={{ background: 'var(--cp-bg)' }}>
            {stored && (
              <button type="button" data-testid="policy-delete" onClick={() => setConfirmDelete(true)} className="mr-auto min-h-11 rounded-lg px-3 text-sm" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.policy.delete', 'Delete policy')}</button>
            )}
            <button type="button" disabled={!dirty} onClick={() => { setBody(structuredClone(base.body)); setEnabled(base.enabled); setEditorKey((value) => value + 1) }} className="min-h-11 rounded-lg px-4 text-sm disabled:opacity-50" style={buttonStyle.secondary}>{t('aiCenter.policy.discard', 'Discard')}</button>
            <button
              type="button"
              data-testid="policy-review"
              disabled={!dirty || invalidLimits || hiddenConflict}
              onClick={() => setPending({ source: selection.source, subject: selection.subject, enabled, body: normalizeBody(body) })}
              className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50"
              style={buttonStyle.primary}
            >
              {t('aiCenter.policy.review', 'Review and save')}
            </button>
          </div>
        </>
      )}

      <EffectivePreview
        key={selection.subject}
        api={api}
        users={users}
        apps={apps}
        initialUser={parts?.userId ?? (users ? undefined : viewerId)}
        initialApp={parts?.appId}
        subjectLabel={subjectLabel}
        generation={generation}
      />

      {pending && (
        <PolicySaveDialog
          api={api}
          viewerId={viewerId}
          draft={pending}
          stored={stored}
          adminPolicy={adminPolicy}
          subjectLabel={subjectLabel}
          onClose={() => setPending(null)}
          onSaved={() => { setPending(null); onSaved() }}
        />
      )}
      <ConfirmDialog
        open={confirmDelete}
        title={t('aiCenter.policy.deleteTitle', 'Delete this policy?')}
        message={t('aiCenter.policy.deleteMessage', 'Its settings stop applying; less specific layers decide again. Usage history stays.')}
        confirmLabel={t('aiCenter.policy.delete', 'Delete policy')}
        confirming={deleting}
        error={deleteError}
        onConfirm={() => void remove()}
        onCancel={() => setConfirmDelete(false)}
      />
    </div>
  )
}

export function PoliciesPage({ intent, onIntentConsumed }: PoliciesPageProps) {
  const { t } = useI18n()
  const api = usePolicyApi()
  const isAdmin = useViewerIsAdmin()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const storeProviders = useProviders()
  const usage = useUsageSummary()
  const mine = useMyEffective()
  const [viewerId, setViewerId] = useState('')
  const [users, setUsers] = useState<string[]>([])
  const [groups, setGroups] = useState<{ id: string; name: string }[]>([])
  const [policies, setPolicies] = useState<AiccPolicyView[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [generation, setGeneration] = useState(0)
  const [selected, setSelected] = useState<Selection | null>(null)
  const [added, setAdded] = useState<Selection[]>([])
  const [adding, setAdding] = useState<'user' | 'app' | 'user_app' | null>(null)
  const [focus, setFocus] = useState<{ provider?: string; model?: string } | undefined>(undefined)

  useEffect(() => {
    void fetchCurrentAccount().then((account) => setViewerId(account?.user_id ?? '')).catch(() => undefined)
    void fetchUserGroups().then(({ data }) => setGroups((data?.groups ?? []).map((group) => ({ id: group.group_id, name: group.name })))).catch(() => undefined)
  }, [])
  useEffect(() => {
    if (!isAdmin) return
    void fetchUserList().then(({ data }) => setUsers((data?.users ?? []).filter((user) => user.user_type !== 'root').map((user) => user.user_id))).catch(() => undefined)
  }, [isAdmin])
  useEffect(() => {
    if (isAdmin === null) return
    let live = true
    api.listPolicies(isAdmin ? 'manage' : 'mine')
      .then((list) => { if (live) { setPolicies(list); setError(null) } })
      .catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    return () => { live = false }
  }, [api, isAdmin, generation])

  const groupNames = useMemo(() => Object.fromEntries(groups.map((group) => [group.id, group.name])), [groups])
  const subjectLabel = useSubjectLabel(groupNames)
  const bodySummary = useBodySummary()
  const apps = useMemo(() => [...new Set(['aiworkspace', ...Object.keys(usage.by_app ?? {})])].filter((app) => app && !app.startsWith('system:')).sort(), [usage.by_app])

  // Administrators edit against the whole inventory; a user only names what they can see.
  const providers: EditorProvider[] = useMemo(() => {
    if (isAdmin) {
      return storeProviders.map((provider) => ({ name: provider.config.provider_instance_name, enabled: provider.config.enabled, models: provider.inventory.models.map((model) => model.exact_model) }))
    }
    const byProvider = new Map<string, string[]>()
    for (const model of mine?.models ?? []) byProvider.set(model.provider_instance_name, [...(byProvider.get(model.provider_instance_name) ?? []), model.exact_model])
    return [...byProvider.entries()].map(([name, models]) => ({ name, enabled: true, models }))
  }, [isAdmin, mine, storeProviders])

  // A Usage row: open (or start) the policy of its subject, focused on its Provider or model, until the
  // viewer picks something else.
  const fromIntent = useMemo(() => {
    if (!intent || isAdmin === null || !viewerId) return null
    const user = intent.userId ?? viewerId
    // A user configures only their own Apps: a Provider or model row asks which App first.
    const subject = intent.appId ? `user_app:${user}/${intent.appId}` : isAdmin ? `user:${user}` : undefined
    return {
      selection: subject ? { subject, source: (isAdmin ? 'admin' : 'user') as PolicySource } : null,
      focus: { provider: intent.provider, model: intent.model },
    }
  }, [intent, isAdmin, viewerId])
  const consumeIntent = () => { if (intent) onIntentConsumed?.() }

  const all = useMemo(() => policies ?? [], [policies])
  const selections = useMemo(() => {
    const out = new Map<string, Selection>()
    const push = (selection: Selection) => out.set(key(selection), selection)
    if (isAdmin) {
      USER_TYPE_SUBJECTS.forEach((subject) => push({ subject, source: 'admin' }))
      groups.forEach((group) => push({ subject: `group:${group.id}`, source: 'admin' }))
    }
    all.forEach((view) => push({ subject: view.policy.subject, source: view.policy.source }))
    added.forEach(push)
    if (fromIntent?.selection) push(fromIntent.selection)
    return sortSubjects([...new Set([...out.values()].map((item) => item.subject))]).flatMap((subject) =>
      [...out.values()].filter((item) => item.subject === subject).sort((a, b) => a.source.localeCompare(b.source)))
  }, [isAdmin, groups, all, added, fromIntent])

  const current = selected ?? fromIntent?.selection ?? (isAdmin ? { subject: 'type:user', source: 'admin' as const } : null)
  const currentFocus = selected ? focus : fromIntent?.focus
  const addingKind = adding ?? (fromIntent && !fromIntent.selection ? 'user_app' : null)
  const stored = current ? policyOf(all, current.subject, current.source) : undefined
  const adminPolicy = policyOf(all, 'type:admin', 'admin')
  const choose = (selection: Selection) => { setSelected(selection); setFocus(undefined); consumeIntent() }
  const addSubject = (subject: string) => {
    const selection: Selection = { subject, source: isAdmin ? 'admin' : 'user' }
    setAdded((list) => list.some((item) => key(item) === key(selection)) ? list : [...list, selection])
    setSelected(selection)
    setFocus(fromIntent?.focus)
    setAdding(null)
    consumeIntent()
  }

  const item = (selection: Selection) => {
    const view = policyOf(all, selection.subject, selection.source)
    const own = selection.source === 'user'
    return (
      <SubjectButton
        key={key(selection)}
        selection={selection}
        active={!!current && key(current) === key(selection)}
        label={subjectLabel(selection.subject)}
        summary={bodySummary(view)}
        sourceNote={isAdmin && own ? t('aiCenter.policy.source.userOf', 'Own policy of {{user}}', { user: parseSubject(selection.subject)?.userId ?? '' }) : undefined}
        onSelect={() => choose(selection)}
      />
    )
  }
  const ofKind = (kinds: string[], source?: PolicySource) => selections.filter((selection) => kinds.includes(parseSubject(selection.subject)?.kind ?? '') && (!source || selection.source === source))
  const addButton = (kind: 'user' | 'app' | 'user_app', label: string, testId: string) => (
    <button type="button" data-testid={testId} onClick={() => setAdding(kind)} aria-label={label} title={label} className="inline-flex min-h-8 items-center gap-1 rounded-md px-2 text-xs" style={{ color: 'var(--cp-accent)' }}>
      <Plus size={14} />{t('aiCenter.policy.add', 'Add')}
    </button>
  )

  const list = isAdmin ? (
    <nav className="flex flex-col gap-4" aria-label={t('aiCenter.policy.subjects', 'Who policies apply to')}>
      <Section title={t('aiCenter.policy.section.types', 'User types')}>{ofKind(['type']).map(item)}</Section>
      <Section title={t('aiCenter.policy.section.groups', 'User groups')}>
        {ofKind(['group']).map(item)}
        {ofKind(['group']).length === 0 && <span className="px-1 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.noGroups', 'No user group yet. Groups are managed in Users & Agents.')}</span>}
      </Section>
      <Section title={t('aiCenter.policy.section.users', 'Users')} action={addButton('user', t('aiCenter.policy.addUser', 'Policy for a user'), 'policy-add-user-open')}>
        {ofKind(['user']).map(item)}
      </Section>
      <Section title={t('aiCenter.policy.section.apps', 'Apps and Agents')} action={addButton('app', t('aiCenter.policy.addApp', 'Policy for an App'), 'policy-add-app-open')}>
        {ofKind(['app']).map(item)}
      </Section>
      <Section title={t('aiCenter.policy.section.userApps', 'Users’ Apps')} action={addButton('user_app', t('aiCenter.policy.addUserApp', 'Policy for a user’s App'), 'policy-add-user-app-open')}>
        {ofKind(['user_app']).map(item)}
      </Section>
    </nav>
  ) : (
    <nav className="flex flex-col gap-4" aria-label={t('aiCenter.policy.subjects', 'Who policies apply to')}>
      <Section title={t('aiCenter.policy.section.myApps', 'My Apps')} action={addButton('user_app', t('aiCenter.policy.newOwn', 'Limit one of my Apps'), 'policy-new-own')}>
        {ofKind(['user_app'], 'user').map(item)}
        {ofKind(['user_app'], 'user').length === 0 && <span className="px-1 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.ownEmpty', 'No own policy. Your Apps and Agents use AI within the administrator policies below.')}</span>}
      </Section>
      <Section title={t('aiCenter.policy.section.adminOnMe', 'Administrator policies that apply to me')}>
        {selections.filter((selection) => selection.source === 'admin').map(item)}
        {selections.filter((selection) => selection.source === 'admin').length === 0 && <span className="px-1 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.adminEmpty', 'No administrator policy applies to you.')}</span>}
      </Section>
    </nav>
  )

  const detail = current ? (
    <PolicyDetail
      key={`${key(current)}\u0000${stored?.policy.version ?? 0}`}
      api={api}
      viewerId={viewerId}
      selection={current}
      stored={stored}
      adminPolicy={adminPolicy}
      providers={providers}
      focus={currentFocus}
      users={isAdmin ? users : undefined}
      apps={apps}
      subjectLabel={subjectLabel}
      generation={generation}
      onSaved={() => setGeneration((value) => value + 1)}
    />
  ) : (
    <EffectivePreview api={api} apps={apps} subjectLabel={subjectLabel} generation={generation} />
  )

  return (
    <div className="flex flex-col gap-5" data-testid="policies-page">
      <header className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
        <div>
          <h1 className="text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.policy.title', 'Policies')}</h1>
          <p className="mt-1 max-w-3xl text-sm" style={{ color: 'var(--cp-muted)' }}>
            {isAdmin
              ? t('aiCenter.policy.subtitle', 'Which Providers and models each user, group and App may use, what it costs them in AI Credit and how much they may use in 5 hours, 24 hours or 7 days. Users and groups come from Users & Agents.')
              : t('aiCenter.policy.subtitleUser', 'What you may use, what it costs you in AI Credit and your limits. You can narrow and limit your own Apps and Agents.')}
          </p>
        </div>
        <button type="button" onClick={() => setGeneration((value) => value + 1)} aria-label={t('aiCenter.policy.refresh', 'Refresh')} title={t('aiCenter.policy.refresh', 'Refresh')} className="min-h-10 self-start rounded-lg px-3" style={buttonStyle.secondary}><RefreshCw size={16} /></button>
      </header>
      {error && <p className="text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      {isAdmin === null || !policies ? (
        <div role="status" className="p-4 text-sm" style={{ color: 'var(--cp-muted)' }}>{error ? null : t('common.loading', 'Loading...')}</div>
      ) : isMobile ? (
        selected ?? fromIntent?.selection ? (
          <div className="flex flex-col gap-3">
            <button type="button" onClick={() => { setSelected(null); consumeIntent() }} className="inline-flex min-h-10 items-center gap-1 self-start text-sm" style={{ color: 'var(--cp-accent)' }}>
              <ArrowLeft size={16} />{t('aiCenter.policy.backToList', 'All policies')}
            </button>
            {detail}
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            {list}
            {!isAdmin && <EffectivePreview api={api} apps={apps} subjectLabel={subjectLabel} generation={generation} />}
          </div>
        )
      ) : (
        <div className="flex gap-6">
          <div className="w-64 shrink-0">{list}</div>
          <section className="min-w-0 flex-1">
            {detail}
          </section>
        </div>
      )}
      {addingKind && (
        <AddSubjectDialog
          kind={addingKind}
          users={users}
          apps={apps}
          viewerId={viewerId}
          isAdmin={isAdmin === true}
          onAdd={addSubject}
          onClose={() => { setAdding(null); consumeIntent() }}
        />
      )}
      {isAdmin === false && policies && policies.length === 0 && !selected && (
        <div className="flex items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}><ShieldCheck size={14} />{t('aiCenter.policy.noneApply', 'No policy applies to you yet.')}</div>
      )}
    </div>
  )
}
