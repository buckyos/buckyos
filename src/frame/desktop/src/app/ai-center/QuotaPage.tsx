import { useEffect, useMemo, useState } from 'react'
import { Gauge, Plus, RefreshCw } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { fetchCurrentAccount } from '../../api/account'
import { fetchUserList } from '../../api/user_mgr'
import { useViewerIsAdmin } from '../settings/hooks/use-viewer-is-admin'
import { useProviders, useUsageSummary } from './hooks/use-aicc-store'
import { useQuotaApi } from './hooks/use-quota-api'
import {
  EMPTY_QUOTA_FILTER,
  filterRules,
  type QuotaFilter,
  type QuotaQueryResponse,
  type QuotaRuleDraft,
  type QuotaRuleView,
} from './datamodel/quota'
import { RuleList } from './components/quota/RuleList'
import { RuleEditor, type QuotaInventory } from './components/quota/RuleEditor'
import { RuleDetailPanel } from './components/quota/RuleDetailPanel'
import { Segmented } from './components/quota/QuotaWidgets'
import { buttonStyle } from './components/quota/quota-labels'

type QuotaTab = 'mine' | 'user' | 'app' | 'user_app'

interface QuotaPageProps {
  /** A draft prefilled from a Usage row, consumed once. */
  intent?: Partial<QuotaRuleDraft> | null
  onIntentConsumed?: () => void
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function QuotaPage({ intent, onIntentConsumed }: QuotaPageProps) {
  const { t } = useI18n()
  const api = useQuotaApi()
  const isAdmin = useViewerIsAdmin() === true
  const providers = useProviders()
  const usage = useUsageSummary()
  const [viewerId, setViewerId] = useState('')
  const [users, setUsers] = useState<string[]>([])
  const [tab, setTab] = useState<QuotaTab>('mine')
  const [filter, setFilter] = useState<QuotaFilter>(EMPTY_QUOTA_FILTER)
  const [mine, setMine] = useState<QuotaQueryResponse | null>(null)
  const [managed, setManaged] = useState<QuotaRuleView[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [opened, setOpened] = useState<QuotaRuleView | null>(null)
  const [editor, setEditor] = useState<{ initial?: Partial<QuotaRuleDraft> | null; editing?: QuotaRuleView | null } | null>(null)

  useEffect(() => {
    void fetchCurrentAccount().then((account) => setViewerId(account?.user_id ?? '')).catch(() => undefined)
  }, [])
  useEffect(() => {
    if (!isAdmin) return
    void fetchUserList().then(({ data }) => setUsers((data?.users ?? []).map((user) => user.user_id))).catch(() => undefined)
  }, [isAdmin])

  const [generation, setGeneration] = useState(0)
  const load = () => setGeneration((value) => value + 1)
  useEffect(() => {
    let live = true
    Promise.all([api.query(), isAdmin ? api.listRules('manage') : Promise.resolve(null)])
      .then(([own, all]) => {
        if (!live) return
        setError(null)
        setMine(own)
        if (all) setManaged(all)
      })
      .catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    return () => { live = false }
  }, [api, isAdmin, generation])

  // A Usage row names an App (an own limit on it) or a resource (the viewer's own limit on it;
  // an administrator's rule on themselves). It opens the editor until that editor closes.
  const intentEditor = useMemo(() => {
    if (!intent || !viewerId) return null
    const app = intent.object?.type === 'user_app' ? intent.object.app_id : undefined
    if (app || !isAdmin) {
      return { initial: { source: 'user' as const, object: { type: 'user_app' as const, user_id: viewerId, app_id: app ?? '' }, resource: intent.resource } }
    }
    return { initial: { object: { type: 'user' as const, user_id: viewerId }, resource: intent.resource } }
  }, [intent, viewerId, isAdmin])
  const activeEditor = editor ?? intentEditor
  const closeEditor = () => {
    setEditor(null)
    if (intent) onIntentConsumed?.()
  }

  const inventory: QuotaInventory[] = useMemo(() => providers
    .filter((provider) => provider.config.enabled)
    .map((provider) => ({
      provider_instance_name: provider.config.provider_instance_name,
      exact_models: provider.inventory.models.map((model) => model.exact_model),
    })), [providers])
  const apps = useMemo(() => [...new Set(['aiworkspace', ...Object.keys(usage.by_app ?? {})])].filter((app) => app && !app.startsWith('system:')), [usage.by_app])

  const rules = useMemo(() => {
    if (tab === 'mine') return filterRules(mine?.rules ?? [], filter)
    return filterRules((managed ?? []).filter((view) => view.rule.object.type === tab), filter)
  }, [tab, mine, managed, filter])
  const own = rules.filter((view) => view.rule.source === 'user')
  const fromAdmin = rules.filter((view) => view.rule.source === 'admin')

  const tabs: { value: QuotaTab; label: string }[] = [
    { value: 'mine', label: t('aiCenter.quota.tab.mine', 'My App quotas') },
    ...(isAdmin ? [
      { value: 'user' as const, label: t('aiCenter.quota.tab.users', 'User quotas') },
      { value: 'app' as const, label: t('aiCenter.quota.tab.apps', 'App quotas') },
      { value: 'user_app' as const, label: t('aiCenter.quota.tab.userApps', "Users' App quotas") },
    ] : []),
  ]

  const openNew = () => setEditor({
    initial: tab === 'mine' || !isAdmin
      ? { object: { type: 'user_app', user_id: viewerId, app_id: '' }, source: 'user' }
      : tab === 'app' ? { object: { type: 'app', app_id: '' } } : tab === 'user_app' ? { object: { type: 'user_app', user_id: users[0] ?? '', app_id: '' } } : { object: { type: 'user', user_id: users[0] ?? '' } },
  })

  return (
    <div className="flex flex-col gap-5" data-testid="quota-page">
      <header className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
        <div>
          <h1 className="text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.title', 'Token quota')}</h1>
          <p className="mt-1 max-w-3xl text-sm" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.quota.subtitle', 'Limits on the tokens AI calls through AICC may use, per user, App or Agent. Unrelated to the Provider account quota shown on Providers. Calls that bypass AICC with their own keys are not covered.')}
          </p>
        </div>
        <div className="flex gap-2">
          <button type="button" onClick={load} aria-label={t('aiCenter.quota.refresh', 'Refresh')} className="min-h-10 rounded-lg px-3" style={buttonStyle.secondary}><RefreshCw size={16} /></button>
          <button type="button" data-testid="quota-new" onClick={openNew} className="inline-flex min-h-10 items-center gap-1.5 whitespace-nowrap rounded-lg px-4 text-sm font-medium" style={buttonStyle.primary}>
            <Plus size={16} />
            {tab === 'mine' ? t('aiCenter.quota.newOwn', 'Limit one of my Apps') : t('aiCenter.quota.new', 'New quota')}
          </button>
        </div>
      </header>

      {mine && (mine.near_limit_count > 0 || mine.exhausted_count > 0) && (
        <div className="flex items-center gap-2 rounded-xl px-4 py-3 text-sm" data-testid="quota-warning" style={{ background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
          <Gauge size={16} style={{ color: 'var(--cp-warning)' }} />
          {[
            mine.exhausted_count > 0 ? t('aiCenter.quota.warningExhausted', '{{count}} exhausted', { count: String(mine.exhausted_count) }) : '',
            mine.near_limit_count > 0 ? t('aiCenter.quota.warningNear', '{{count}} near the limit', { count: String(mine.near_limit_count) }) : '',
          ].filter(Boolean).join(t('aiCenter.quota.listSeparator', ', '))}
        </div>
      )}

      {tabs.length > 1 && <Segmented label={t('aiCenter.quota.tabsLabel', 'Quota views')} value={tab} onChange={setTab} options={tabs} />}

      <div className="flex flex-col gap-2 md:flex-row md:items-center">
        <input
          value={filter.query}
          onChange={(event) => setFilter({ ...filter, query: event.target.value })}
          placeholder={t('aiCenter.quota.search', 'Search user, App, Provider or model')}
          aria-label={t('aiCenter.quota.search', 'Search user, App, Provider or model')}
          className="min-h-10 flex-1 rounded-lg px-3 text-sm"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
        />
        <select aria-label={t('aiCenter.quota.kindLabel', 'Kind')} value={filter.kind} onChange={(event) => setFilter({ ...filter, kind: event.target.value as QuotaFilter['kind'] })} className="min-h-10 rounded-lg px-3 text-sm" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
          <option value="all">{t('aiCenter.quota.filter.allKinds', 'All kinds')}</option>
          <option value="periodic">{t('aiCenter.quota.kind.periodic', 'Periodic')}</option>
          <option value="total">{t('aiCenter.quota.kind.total', 'Total')}</option>
        </select>
        <select aria-label={t('aiCenter.quota.statusLabel', 'Status')} value={filter.status} onChange={(event) => setFilter({ ...filter, status: event.target.value as QuotaFilter['status'] })} className="min-h-10 rounded-lg px-3 text-sm" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
          <option value="all">{t('aiCenter.quota.filter.allStates', 'Any state')}</option>
          <option value="limited">{t('aiCenter.quota.filter.limited', 'Near or at the limit')}</option>
          <option value="disabled">{t('aiCenter.quota.status.disabled', 'Disabled')}</option>
        </select>
      </div>

      {error && <p className="text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}

      {tab === 'mine' ? (
        <>
          <section className="flex flex-col gap-2" data-testid="quota-own">
            <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.ownSection', 'My own limits')}</h2>
            <RuleList rules={own} onOpen={setOpened} empty={t('aiCenter.quota.ownEmpty', 'No own limit. Your Apps and Agents use AI up to the administrator limits below.')} />
          </section>
          <section className="flex flex-col gap-2" data-testid="quota-admin-on-me">
            <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.quota.adminSection', 'Administrator limits that apply to me')}</h2>
            <RuleList rules={fromAdmin} onOpen={setOpened} empty={t('aiCenter.quota.adminEmpty', 'No administrator limit applies to you.')} />
          </section>
        </>
      ) : (
        <RuleList rules={rules} onOpen={setOpened} empty={t('aiCenter.quota.manageEmpty', 'No quota rule here. Without a rule there is no AICC token limit for this scope.')} />
      )}

      {opened && (
        <RuleDetailPanel
          api={api}
          view={opened}
          isAdmin={isAdmin}
          onClose={() => setOpened(null)}
          onEdit={(view) => { setOpened(null); setEditor({ editing: view }) }}
          onChanged={load}
        />
      )}
      {activeEditor && (
        <RuleEditor
          api={api}
          isAdmin={isAdmin && !(activeEditor.initial?.source === 'user')}
          viewerId={viewerId}
          inventory={inventory}
          users={users}
          apps={apps}
          initial={activeEditor.initial}
          editing={'editing' in activeEditor ? activeEditor.editing : undefined}
          onClose={closeEditor}
          onSaved={() => { closeEditor(); load() }}
        />
      )}
    </div>
  )
}
