import { useEffect, useMemo, useState } from 'react'
import { Eye, Users } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { fetchUserGroups, fetchUserList, type UserGroupListItem } from '../../api/user_mgr'
import { useViewerIsAdmin } from '../settings/hooks/use-viewer-is-admin'
import { useQuotaApi } from './hooks/use-quota-api'
import {
  fromGroupDraft,
  sortSubjects,
  splitExactModel,
  toGroupDraft,
  type GroupVisibilityDraft,
  type ProviderChoice,
  type VisibilityEntry,
  type VisibilityPreview,
  type VisibilityState,
} from './datamodel/quota'
import { Segmented } from './components/quota/QuotaWidgets'
import { buttonStyle } from './components/quota/quota-labels'
import { EmptyState } from './components/shared/EmptyState'

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

const TYPE_SUBJECTS = ['type:admin', 'type:user', 'type:limited']

export function VisibilityPage() {
  const { t } = useI18n()
  const api = useQuotaApi()
  const isAdmin = useViewerIsAdmin()
  const [state, setState] = useState<VisibilityState | null>(null)
  const [groups, setGroups] = useState<UserGroupListItem[]>([])
  const [users, setUsers] = useState<string[]>([])
  const [subject, setSubject] = useState('type:admin')
  // Edits belong to the stored policy they started from; a new policy (other group, saved) resets them.
  const [edit, setEdit] = useState<{ base: VisibilityEntry[]; draft: GroupVisibilityDraft } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState<VisibilityPreview | null>(null)
  const [saving, setSaving] = useState(false)
  const [previewUser, setPreviewUser] = useState('')
  const [userPreview, setUserPreview] = useState<VisibilityPreview | null>(null)

  useEffect(() => {
    if (!isAdmin) return
    let live = true
    Promise.all([api.getVisibility(), fetchUserGroups(), fetchUserList()])
      .then(([visibility, groupList, userList]) => {
        if (!live) return
        setError(null)
        setState(visibility)
        setGroups(groupList.data?.groups ?? [])
        setUsers((userList.data?.users ?? []).filter((user) => user.user_type !== 'root').map((user) => user.user_id))
      })
      .catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    return () => { live = false }
  }, [api, isAdmin])

  const stored = useMemo(() => state?.subjects.find((item) => item.subject === subject)?.entries ?? [], [state, subject])
  const storedDraft = useMemo(() => toGroupDraft(stored), [stored])
  const draft = edit && edit.base === stored ? edit.draft : storedDraft
  const setDraft = (next: GroupVisibilityDraft) => setEdit({ base: stored, draft: next })

  const subjects = useMemo(() => sortSubjects([...TYPE_SUBJECTS, ...groups.map((group) => `group:${group.group_id}`)]), [groups])
  const subjectName = (value: string) => {
    if (value === 'type:admin') return t('aiCenter.visibility.type.admin', 'Administrators')
    if (value === 'type:user') return t('aiCenter.visibility.type.user', 'Users')
    if (value === 'type:limited') return t('aiCenter.visibility.type.limited', 'Limited users')
    const group = groups.find((item) => `group:${item.group_id}` === value)
    return group ? group.name || group.group_id : value
  }
  const summary = (value: string) => {
    const entries = state?.subjects.find((item) => item.subject === value)?.entries ?? []
    if (entries.some((entry) => entry.type === 'all_providers')) return t('aiCenter.visibility.summary.all', 'All Providers (following the list)')
    if (entries.length === 0) return t('aiCenter.visibility.summary.none', 'Nothing open')
    return t('aiCenter.visibility.summary.count', 'Providers: {{count}}', { count: String(entries.length) })
  }

  const setChoice = (provider: string, choice: ProviderChoice) => setDraft({ ...draft, providers: { ...draft.providers, [provider]: choice } })
  const dirty = JSON.stringify(fromGroupDraft(draft)) !== JSON.stringify(fromGroupDraft(storedDraft))

  const review = async () => {
    setError(null)
    try {
      setPending(await api.previewVisibility(undefined, { subject, entries: fromGroupDraft(draft) }))
    } catch (reason) {
      setError(errorText(reason))
    }
  }
  const confirm = async () => {
    if (!state) return
    setSaving(true)
    try {
      setState(await api.updateVisibility(state.revision, subject, fromGroupDraft(draft)))
      setPending(null)
    } catch (reason) {
      setError(errorText(reason))
      setPending(null)
    } finally {
      setSaving(false)
    }
  }
  const openUnexposed = () => {
    if (!state) return
    const next = { ...draft, providers: { ...draft.providers } }
    for (const provider of state.providers) {
      const fresh = provider.exact_models.filter((model) => state.unexposed_models.includes(model))
      if (fresh.length === 0) continue
      const current = next.providers[provider.provider_instance_name] ?? { mode: 'closed' }
      if (current.mode === 'all_models') continue
      const existing = current.mode === 'models' ? current.exact_models : []
      next.providers[provider.provider_instance_name] = { mode: 'models', exact_models: [...new Set([...existing, ...fresh])] }
    }
    setDraft(next)
  }
  const runUserPreview = async (userId: string) => {
    setPreviewUser(userId)
    setUserPreview(null)
    if (!userId) return
    try {
      setUserPreview(await api.previewVisibility(userId))
    } catch (reason) {
      setError(errorText(reason))
    }
  }

  if (isAdmin === false) {
    return <EmptyState icon={<Eye size={40} />} title={t('aiCenter.visibility.adminOnly', 'Only administrators manage model visibility')} />
  }

  return (
    <div className="flex flex-col gap-5" data-testid="visibility-page">
      <header>
        <h1 className="text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.visibility.title', 'Model visibility')}</h1>
        <p className="mt-1 max-w-3xl text-sm" style={{ color: 'var(--cp-muted)' }}>
          {t('aiCenter.visibility.subtitle', 'Which Providers and models each user group may see and use. A user gets the union of their user-type group and system groups, always within the actual model list of each Provider. Groups and members are managed in Users & Agents.')}
        </p>
      </header>
      {error && <p className="text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      {state && state.unexposed_models.length > 0 && (
        <div data-testid="visibility-unexposed" className="flex flex-col gap-2 rounded-xl px-4 py-3 text-sm md:flex-row md:items-center" style={{ background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
          <span>{t('aiCenter.visibility.unexposed', '{{count}} models are not open to any group yet', { count: String(state.unexposed_models.length) })}</span>
          <button type="button" onClick={openUnexposed} className="min-h-9 rounded-lg px-3 text-sm md:ml-auto" style={buttonStyle.secondary}>
            {t('aiCenter.visibility.openUnexposed', 'Add them to {{group}}', { group: subjectName(subject) })}
          </button>
        </div>
      )}
      <div className="flex flex-col gap-4 md:flex-row">
        <nav className="flex shrink-0 flex-col gap-1 md:w-64" aria-label={t('aiCenter.visibility.groups', 'Groups')}>
          {subjects.map((value) => {
            const active = value === subject
            return (
              <button
                key={value}
                type="button"
                data-testid="visibility-subject"
                data-subject={value}
                onClick={() => setSubject(value)}
                className="flex flex-col items-start rounded-lg px-3 py-2 text-left"
                style={{
                  background: active ? 'color-mix(in srgb, var(--cp-accent) 10%, var(--cp-surface))' : 'var(--cp-surface)',
                  border: `1px solid ${active ? 'var(--cp-accent)' : 'var(--cp-border)'}`,
                }}
              >
                <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{subjectName(value)}</span>
                <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{value} · {summary(value)}</span>
              </button>
            )
          })}
        </nav>
        <section className="flex min-w-0 flex-1 flex-col gap-3">
          <label className="flex items-start gap-2 rounded-xl p-4 text-sm" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
            <input type="checkbox" data-testid="visibility-all" checked={draft.allProviders} onChange={(event) => setDraft({ ...draft, allProviders: event.target.checked })} className="mt-0.5" />
            <span>
              <span className="font-medium">{t('aiCenter.visibility.allProviders', 'All Providers (following the list)')}</span>
              <span className="block text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.visibility.allProvidersHint', 'Every Provider and model, including ones added later.')}</span>
            </span>
          </label>
          {(state?.providers ?? []).map((provider) => {
            const choice = draft.allProviders ? { mode: 'all_models' as const } : draft.providers[provider.provider_instance_name] ?? { mode: 'closed' as const }
            return (
              <div key={provider.provider_instance_name} data-testid="visibility-provider" data-provider={provider.provider_instance_name} className="flex flex-col gap-2 rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', opacity: draft.allProviders ? 0.6 : 1 }}>
                <div className="flex flex-col gap-2 md:flex-row md:items-center">
                  <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {provider.provider_instance_name}
                    {!provider.enabled && <span className="ml-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.visibility.disabledProvider', 'disabled')}</span>}
                  </span>
                  <div className="md:ml-auto md:w-[420px]">
                    <Segmented
                      label={t('aiCenter.visibility.providerChoice', 'Access to {{provider}}', { provider: provider.provider_instance_name })}
                      value={choice.mode}
                      onChange={(mode) => setChoice(provider.provider_instance_name, mode === 'models' ? { mode, exact_models: choice.mode === 'models' ? choice.exact_models : [] } : { mode })}
                      options={[
                        { value: 'closed', label: t('aiCenter.visibility.closed', 'Not open'), disabled: draft.allProviders },
                        { value: 'all_models', label: t('aiCenter.visibility.allModels', 'All models'), disabled: draft.allProviders },
                        { value: 'models', label: t('aiCenter.visibility.chosenModels', 'Chosen models'), disabled: draft.allProviders },
                      ]}
                    />
                  </div>
                </div>
                <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {choice.mode === 'all_models'
                    ? t('aiCenter.visibility.followHint', 'Follows the model list: new models and variants open automatically.')
                    : choice.mode === 'models'
                      ? t('aiCenter.visibility.chosenHint', 'Only the models ticked below; models found later stay hidden until opened.')
                      : t('aiCenter.visibility.closedHint', 'This group does not get this Provider (another group of the user may).')}
                </span>
                {choice.mode === 'models' && !draft.allProviders && (
                  <div className="flex flex-col gap-1">
                    <button type="button" onClick={() => setChoice(provider.provider_instance_name, { mode: 'models', exact_models: [...provider.exact_models] })} className="self-start text-xs underline" style={{ color: 'var(--cp-accent)' }}>
                      {t('aiCenter.visibility.selectCurrent', 'Select the current models')}
                    </button>
                    {provider.exact_models.length === 0 && <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.visibility.noModels', 'No model available yet')}</span>}
                    {provider.exact_models.map((model) => {
                      const parts = splitExactModel(model)
                      return (
                        <label key={model} className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
                          <input
                            type="checkbox"
                            data-testid="visibility-model"
                            data-model={model}
                            checked={choice.exact_models.includes(model)}
                            onChange={(event) => setChoice(provider.provider_instance_name, {
                              mode: 'models',
                              exact_models: event.target.checked ? [...choice.exact_models, model] : choice.exact_models.filter((item) => item !== model),
                            })}
                          />
                          <span title={model}>{parts.model}{parts.variant ? `:${parts.variant}` : ''}<span style={{ color: 'var(--cp-muted)' }}>@{parts.provider}</span></span>
                          {state?.unexposed_models.includes(model) && <span className="text-xs" style={{ color: 'var(--cp-warning)' }}>{t('aiCenter.visibility.new', 'not open to any group')}</span>}
                        </label>
                      )
                    })}
                  </div>
                )}
              </div>
            )
          })}
          <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
            <button type="button" disabled={!dirty} onClick={() => setEdit(null)} className="min-h-11 rounded-lg px-4 text-sm disabled:opacity-50" style={buttonStyle.secondary}>{t('aiCenter.visibility.discard', 'Discard')}</button>
            <button type="button" data-testid="visibility-review" disabled={!dirty} onClick={() => void review()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('aiCenter.visibility.review', 'Review and save')}</button>
          </div>

          <section className="mt-2 flex flex-col gap-2 rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <h2 className="flex items-center gap-2 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}><Users size={16} />{t('aiCenter.visibility.userPreview', 'What a user sees')}</h2>
            <select data-testid="visibility-user" value={previewUser} onChange={(event) => void runUserPreview(event.target.value)} className="min-h-10 rounded-lg px-3 text-sm" style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
              <option value="">{t('aiCenter.visibility.pickUser', 'Pick a user')}</option>
              {users.map((user) => <option key={user} value={user}>{user}</option>)}
            </select>
            {userPreview && (
              <div className="flex flex-col gap-1" data-testid="visibility-user-grants">
                {userPreview.grants.length === 0 && <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.visibility.userNothing', 'This user cannot use any model.')}</span>}
                {userPreview.grants.map((grant) => (
                  <div key={`${grant.provider_instance_name}/${grant.exact_model ?? ''}`} className="flex flex-wrap items-center gap-2 text-xs" style={{ color: 'var(--cp-text)' }}>
                    <span>{grant.exact_model ?? `${grant.provider_instance_name} · ${t('aiCenter.visibility.noModels', 'No model available yet')}`}</span>
                    <span className="ml-auto" style={{ color: 'var(--cp-muted)' }}>{grant.sources.map(subjectName).join(', ')}</span>
                  </div>
                ))}
              </div>
            )}
          </section>
        </section>
      </div>

      {pending && (
        <div className="fixed inset-0 z-50 flex items-end justify-center md:items-center" data-testid="visibility-confirm">
          <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.4)' }} onClick={() => setPending(null)} />
          <div role="dialog" aria-label={t('aiCenter.visibility.confirmTitle', 'Save the change?')} className="relative max-h-[calc(100dvh-1rem)] w-full overflow-y-auto rounded-t-xl p-6 shadow-lg md:mx-4 md:max-w-lg md:rounded-xl" style={{ background: 'var(--cp-surface)' }}>
            <h3 className="mb-3 text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.visibility.confirmTitle', 'Save the change?')}</h3>
            {(pending.caller_loses?.length ?? 0) > 0 && (
              <p className="mb-3 text-sm" data-testid="visibility-self-loss" style={{ color: 'var(--cp-danger)' }}>
                {t('aiCenter.visibility.selfLoss', 'You yourself will no longer be able to use: {{models}}', { models: pending.caller_loses!.join(', ') })}
              </p>
            )}
            <p className="text-sm" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.visibility.added', 'Opened: {{models}}', { models: pending.added?.length ? pending.added.join(', ') : '—' })}</p>
            <p className="text-sm" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.visibility.removed', 'Removed: {{models}}', { models: pending.removed?.length ? pending.removed.join(', ') : '—' })}</p>
            <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.visibility.affected', 'Members affected: {{users}}. Another group of a user may still grant what is removed here.', { users: pending.affected_users?.length ? pending.affected_users.join(', ') : '—' })}
            </p>
            <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <button type="button" onClick={() => setPending(null)} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
              <button type="button" data-testid="visibility-save" disabled={saving} onClick={() => void confirm()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('aiCenter.visibility.save', 'Save')}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
