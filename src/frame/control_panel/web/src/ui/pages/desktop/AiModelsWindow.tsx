import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'

import {
  fetchAiDiagnostics,
  fetchAiModelCatalog,
  fetchAiModelsOverview,
  fetchAiPolicies,
  fetchAiProviders,
  reloadAiProviderSettings,
  runAiProviderDiagnostic,
  saveAiModelCatalogEntry,
  saveAiPolicy,
  saveAiProvider,
  type AiDiagnosticEntry,
  type AiModelCatalogEntry,
  type AiModelOverview,
  type AiPolicyEntry,
  type AiProviderCard,
} from '@/api'
import Icon from '../../icons'

type TabId = 'providers' | 'models' | 'policies' | 'diagnostics'

type TabDef = {
  id: TabId
  label: string
  icon: IconName
}

type SelectionState =
  | { type: 'provider'; id: string }
  | { type: 'model'; id: string }
  | { type: 'policy'; id: string }
  | { type: 'diagnostic'; id: string }
  | null

const TABS: TabDef[] = [
  { id: 'providers', label: 'Providers', icon: 'server' },
  { id: 'models', label: 'Models', icon: 'apps' },
  { id: 'policies', label: 'Policies', icon: 'function' },
  { id: 'diagnostics', label: 'Diagnostics', icon: 'activity' },
]

const AiModelsWindow = () => {
  const [activeTab, setActiveTab] = useState<TabId>('providers')
  const [overview, setOverview] = useState<AiModelOverview | null>(null)
  const [providers, setProviders] = useState<AiProviderCard[]>([])
  const [models, setModels] = useState<AiModelCatalogEntry[]>([])
  const [policies, setPolicies] = useState<AiPolicyEntry[]>([])
  const [diagnostics, setDiagnostics] = useState<AiDiagnosticEntry[]>([])
  const [selection, setSelection] = useState<SelectionState>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [reloading, setReloading] = useState(false)
  const [flashMessage, setFlashMessage] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    const load = async () => {
      setLoading(true)
      const [overviewResult, providersResult, modelsResult, policiesResult, diagnosticsResult] = await Promise.all([
        fetchAiModelsOverview(),
        fetchAiProviders(),
        fetchAiModelCatalog(),
        fetchAiPolicies(),
        fetchAiDiagnostics(),
      ])

      if (cancelled) return

      setOverview(overviewResult.data)
      setProviders(providersResult.data ?? [])
      setModels(modelsResult.data ?? [])
      setPolicies(policiesResult.data ?? [])
      setDiagnostics(diagnosticsResult.data ?? [])
      setLoading(false)
    }

    void load()

    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (!flashMessage) return undefined
    const timer = window.setTimeout(() => setFlashMessage(null), 2600)
    return () => window.clearTimeout(timer)
  }, [flashMessage])

  const selectedProvider = selection?.type === 'provider' ? providers.find((item) => item.id === selection.id) ?? null : null
  const selectedModel = selection?.type === 'model' ? models.find((item) => item.alias === selection.id) ?? null : null
  const selectedPolicy = selection?.type === 'policy' ? policies.find((item) => item.id === selection.id) ?? null : null
  const selectedDiagnostic =
    selection?.type === 'diagnostic' ? diagnostics.find((item) => item.id === selection.id) ?? null : null

  const handleProviderSave = async (nextProvider: AiProviderCard, apiKey?: string) => {
    setSaving(true)
    const { data, error } = await saveAiProvider(nextProvider, apiKey)
    setSaving(false)

    if (!data) {
      setFlashMessage(readErrorMessage(error, 'Failed to save provider.'))
      return
    }

    setProviders((current) => current.map((item) => (item.id === data.id ? data : item)))
    setFlashMessage(`Saved ${data.displayName}. Reload to apply.`)
  }

  const handleModelSave = async (nextModel: AiModelCatalogEntry) => {
    setSaving(true)
    const { data, error } = await saveAiModelCatalogEntry(nextModel)
    setSaving(false)

    if (!data) {
      setFlashMessage(readErrorMessage(error, 'Failed to save model.'))
      return
    }

    setModels((current) => current.map((item) => (item.alias === data.alias ? data : item)))
    setFlashMessage(`Saved ${data.alias}.`)
  }

  const handlePolicySave = async (nextPolicy: AiPolicyEntry) => {
    setSaving(true)
    const { data, error } = await saveAiPolicy(nextPolicy)
    setSaving(false)

    if (!data) {
      setFlashMessage(readErrorMessage(error, 'Failed to save policy.'))
      return
    }

    setPolicies((current) => current.map((item) => (item.id === data.id ? data : item)))
    setOverview((current) => {
      if (!current) return current
      const next = { ...current }
      if (data.id === 'message_hub.reply') next.defaultReplyModel = data.primaryModel
      if (data.id === 'message_hub.summary') next.defaultSummaryModel = data.primaryModel
      if (data.id === 'message_hub.task_extract') next.defaultTaskExtractModel = data.primaryModel
      if (data.id === 'agent.raw_explain') next.defaultAgentModel = data.primaryModel
      return next
    })
    setFlashMessage(`Saved ${data.label}.`)
  }

  const handleDiagnosticRun = async (diagnosticId: string) => {
    const providerId =
      diagnosticId === 'diag-openai'
        ? 'openai-main'
        : diagnosticId === 'diag-google'
          ? 'google-main'
          : 'openai-compatible'

    const { data, error } = await runAiProviderDiagnostic(providerId)
    const detail = data?.detail ?? readErrorMessage(error, 'Provider test failed.')

    setDiagnostics((current) =>
      current.map((item) =>
        item.id === diagnosticId
          ? {
              ...item,
              status: data?.status ?? 'warn',
              detail,
              actionLabel: diagnosticId === 'diag-local' ? 'Review checklist' : 'Run again',
            }
          : item,
      ),
    )
    setFlashMessage(data?.ok ? 'Provider test finished.' : detail)
  }

  const handleReload = async () => {
    setReloading(true)
    const { data, error } = await reloadAiProviderSettings()
    setReloading(false)

    if (!data?.ok) {
      setFlashMessage(readErrorMessage(error, 'Failed to reload AICC settings.'))
      return
    }

    const registered = data.result?.providers_registered
    setOverview((current) => {
      if (!current) return current
      const timestamp = new Date()
      const label = `${timestamp.getHours().toString().padStart(2, '0')}:${timestamp.getMinutes().toString().padStart(2, '0')}`
      return {
        ...current,
        providersOnline: typeof registered === 'number' ? registered : current.providersOnline,
        lastDiagnosticsAt: `Today ${label}`,
      }
    })
    setFlashMessage(
      typeof registered === 'number'
        ? `Reloaded AICC. ${registered} provider instance(s) active.`
        : 'Reloaded AICC settings.',
    )
  }

  return (
    <div className="grid h-full min-h-0 grid-cols-[220px_minmax(0,1fr)] gap-3 bg-[linear-gradient(180deg,_#eff6f4_0%,_#e5efeb_100%)] p-0">
      <aside className="flex min-h-0 flex-col overflow-hidden rounded-[24px] border border-[var(--cp-border)] bg-white shadow-sm">
        <div className="border-b border-[var(--cp-border)] px-4 py-4">
          <div className="flex items-center gap-3">
            <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-white">
              <Icon name="spark" className="size-5" />
            </span>
            <div>
              <p className="text-sm font-semibold text-[var(--cp-ink)]">AI Models</p>
            </div>
          </div>
        </div>

        <nav className="flex-1 space-y-1 overflow-auto p-2">
          {TABS.map((tab) => {
            const active = tab.id === activeTab
            return (
              <button
                key={tab.id}
                type="button"
                onClick={() => setActiveTab(tab.id)}
                className={`flex w-full items-center gap-3 rounded-2xl px-3 py-3 text-left text-sm font-medium transition ${
                  active
                    ? 'bg-[var(--cp-primary)] text-white'
                    : 'text-[var(--cp-ink)] hover:bg-[var(--cp-surface-muted)]'
                }`}
              >
                <span className={`inline-flex size-8 items-center justify-center rounded-xl ${active ? 'bg-white/15' : 'bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]'}`}>
                  <Icon name={tab.icon} className="size-4" />
                </span>
                {tab.label}
              </button>
            )
          })}
        </nav>

        <div className="border-t border-[var(--cp-border)] px-4 py-3">
          <button
            type="button"
            disabled={reloading}
            onClick={() => void handleReload()}
            className="inline-flex min-h-10 w-full items-center justify-center rounded-full bg-[var(--cp-primary)] px-4 py-2 text-sm font-semibold text-white transition hover:bg-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
          >
            {reloading ? 'Reloading...' : 'Reload AICC'}
          </button>
        </div>
      </aside>

      <main className="min-w-0 overflow-hidden rounded-[24px] border border-[var(--cp-border)] bg-white shadow-sm">
        <div className="border-b border-[var(--cp-border)] px-5 py-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="text-xl font-semibold text-[var(--cp-ink)]">
                {TABS.find((tab) => tab.id === activeTab)?.label}
              </h2>
            </div>
            <div className="flex flex-wrap gap-2">
              <MiniStat label="Providers" value={`${overview?.providersOnline ?? 0}/${overview?.providersTotal ?? providers.length}`} />
              <MiniStat label="Models" value={String(models.length)} />
              <MiniStat label="Policies" value={String(policies.length)} />
            </div>
          </div>
          {flashMessage ? (
            <div className="mt-3 rounded-2xl border border-emerald-200 bg-emerald-50 px-4 py-3 text-sm text-emerald-700">
              {flashMessage}
            </div>
          ) : null}
        </div>

        <div className={`grid h-[calc(100%-88px)] min-h-0 gap-4 p-4 ${activeTab === 'providers' ? 'grid-cols-1' : 'grid-cols-[minmax(0,1fr)_320px]'}`}>
          <section className="min-w-0 overflow-auto">
            {activeTab === 'providers' ? (
              <ProvidersTab
                providers={providers}
                loading={loading}
                selectedId={selectedProvider?.id ?? null}
                onSelect={(id) => setSelection({ type: 'provider', id })}
              />
            ) : null}
            {activeTab === 'models' ? (
              <ModelsTab
                models={models}
                loading={loading}
                selectedId={selectedModel?.alias ?? null}
                onSelect={(id) => setSelection({ type: 'model', id })}
              />
            ) : null}
            {activeTab === 'policies' ? (
              <PoliciesTab
                policies={policies}
                loading={loading}
                selectedId={selectedPolicy?.id ?? null}
                onSelect={(id) => setSelection({ type: 'policy', id })}
              />
            ) : null}
            {activeTab === 'diagnostics' ? (
              <DiagnosticsTab
                diagnostics={diagnostics}
                loading={loading}
                selectedId={selectedDiagnostic?.id ?? null}
                onSelect={(id) => setSelection({ type: 'diagnostic', id })}
                onRun={handleDiagnosticRun}
              />
            ) : null}
          </section>

          {activeTab === 'providers' ? null : (
            <aside className="min-h-0 overflow-auto rounded-[20px] border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] p-4">
              <InspectorPanel
                selection={selection}
                provider={selectedProvider}
                model={selectedModel}
                policy={selectedPolicy}
                diagnostic={selectedDiagnostic}
                aliases={models.map((item) => item.alias)}
                saving={saving}
                onClose={() => setSelection(null)}
                onProviderSave={handleProviderSave}
                onModelSave={handleModelSave}
                onPolicySave={handlePolicySave}
                onDiagnosticRun={handleDiagnosticRun}
              />
            </aside>
          )}
        </div>

        {activeTab === 'providers' ? (
          <ProviderModal
            provider={selectedProvider}
            saving={saving}
            onClose={() => setSelection(null)}
            onSave={handleProviderSave}
            onRun={handleDiagnosticRun}
          />
        ) : null}
      </main>
    </div>
  )
}

const ProvidersTab = (props: {
  providers: AiProviderCard[]
  loading: boolean
  selectedId: string | null
  onSelect: (id: string) => void
}) => {
  const { providers, loading, selectedId, onSelect } = props
  if (loading) return <EmptyState label="Loading..." />

  return (
    <div className="space-y-2">
      {providers.map((provider) => (
        <div
          key={provider.id}
          role="button"
          tabIndex={0}
          onClick={() => onSelect(provider.id)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault()
              onSelect(provider.id)
            }
          }}
          className={`rounded-2xl border bg-white p-4 transition ${selectedId === provider.id ? 'border-[var(--cp-primary)]' : 'border-[var(--cp-border)]'} cursor-pointer`}
        >
          <div className="block w-full text-left">
            <div className="min-w-0">
              <p className="text-sm font-semibold text-[var(--cp-ink)]">{provider.displayName}</p>
              <p className="truncate text-xs text-[var(--cp-muted)]">{provider.endpoint}</p>
            </div>
          </div>

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <span className="text-xs text-[var(--cp-muted)]">
              {provider.credentialConfigured ? 'Key set' : 'Key missing'}
            </span>
            <StateBadge tone={provider.status}>{provider.status.replace('_', ' ')}</StateBadge>
          </div>
        </div>
      ))}
    </div>
  )
}

const ProviderModal = (props: {
  provider: AiProviderCard | null
  saving: boolean
  onClose: () => void
  onSave: (provider: AiProviderCard, apiKey?: string) => Promise<void>
  onRun: (diagnosticId: string) => Promise<void>
}) => {
  const { provider, saving, onClose, onSave, onRun } = props

  if (!provider) return null

  const diagnosticId =
    provider.id === 'google-main'
      ? 'diag-google'
      : provider.id === 'openai-main'
        ? 'diag-openai'
        : 'diag-local'

  return (
    <div className="absolute inset-0 z-20 flex items-center justify-center bg-slate-900/24 p-6 backdrop-blur-[2px]">
      <div className="relative flex h-[min(760px,calc(100vh-48px))] w-full max-w-2xl flex-col overflow-hidden rounded-[24px] border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] shadow-[0_24px_80px_-32px_rgba(15,23,42,0.35)]">
        <div className="flex items-center justify-between gap-3 border-b border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-5 py-4">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">Provider</p>
            <h3 className="mt-1 text-lg font-semibold text-[var(--cp-ink)]">{provider.displayName}</h3>
          </div>
          <button type="button" onClick={onClose} className="rounded-full border border-[var(--cp-border)] bg-white p-2 text-[var(--cp-muted)]">
            <Icon name="close" className="size-4" />
          </button>
        </div>

        <div className="min-h-0 flex flex-1 overflow-hidden">
          <ProviderEditor provider={provider} saving={saving} onSave={onSave} onRun={() => onRun(diagnosticId)} />
        </div>
      </div>
    </div>
  )
}

const ModelsTab = (props: {
  models: AiModelCatalogEntry[]
  loading: boolean
  selectedId: string | null
  onSelect: (id: string) => void
}) => {
  const { models, loading, selectedId, onSelect } = props
  if (loading) return <EmptyState label="Loading..." />

  return (
    <div className="space-y-2">
      {models.map((model) => (
        <button key={model.alias} type="button" onClick={() => onSelect(model.alias)} className={`flex w-full items-center justify-between rounded-2xl border bg-white px-4 py-3 text-left ${selectedId === model.alias ? 'border-[var(--cp-primary)]' : 'border-[var(--cp-border)]'}`}>
          <div>
            <p className="text-sm font-semibold text-[var(--cp-ink)]">{model.alias}</p>
            <p className="text-xs text-[var(--cp-muted)]">{model.providerModel}</p>
          </div>
          <span className="text-xs text-[var(--cp-muted)]">{model.providerId}</span>
        </button>
      ))}
    </div>
  )
}

const PoliciesTab = (props: {
  policies: AiPolicyEntry[]
  loading: boolean
  selectedId: string | null
  onSelect: (id: string) => void
}) => {
  const { policies, loading, selectedId, onSelect } = props
  if (loading) return <EmptyState label="Loading..." />

  return (
    <div className="space-y-2">
      {policies.map((policy) => (
        <button key={policy.id} type="button" onClick={() => onSelect(policy.id)} className={`flex w-full items-center justify-between rounded-2xl border bg-white px-4 py-3 text-left ${selectedId === policy.id ? 'border-[var(--cp-primary)]' : 'border-[var(--cp-border)]'}`}>
          <div>
            <p className="text-sm font-semibold text-[var(--cp-ink)]">{policy.label}</p>
            <p className="text-xs text-[var(--cp-muted)]">{policy.primaryModel}</p>
          </div>
          <StateBadge tone={policy.status}>{policy.status}</StateBadge>
        </button>
      ))}
    </div>
  )
}

const DiagnosticsTab = (props: {
  diagnostics: AiDiagnosticEntry[]
  loading: boolean
  selectedId: string | null
  onSelect: (id: string) => void
  onRun: (id: string) => Promise<void>
}) => {
  const { diagnostics, loading, selectedId, onSelect, onRun } = props
  if (loading) return <EmptyState label="Loading..." />

  return (
    <div className="space-y-2">
      {diagnostics.map((diagnostic) => (
        <div key={diagnostic.id} className={`rounded-2xl border bg-white p-4 ${selectedId === diagnostic.id ? 'border-[var(--cp-primary)]' : 'border-[var(--cp-border)]'}`}>
          <div className="flex items-center justify-between gap-3">
            <button type="button" onClick={() => onSelect(diagnostic.id)} className="text-left">
              <p className="text-sm font-semibold text-[var(--cp-ink)]">{diagnostic.title}</p>
            </button>
            <div className="flex items-center gap-2">
              <StateBadge tone={diagnostic.status}>{diagnostic.status}</StateBadge>
              <button type="button" onClick={() => void onRun(diagnostic.id)} className="rounded-full border border-[var(--cp-border)] px-3 py-1.5 text-xs font-semibold text-[var(--cp-ink)]">
                Run
              </button>
            </div>
          </div>
        </div>
      ))}
    </div>
  )
}

const InspectorPanel = (props: {
  selection: SelectionState
  provider: AiProviderCard | null
  model: AiModelCatalogEntry | null
  policy: AiPolicyEntry | null
  diagnostic: AiDiagnosticEntry | null
  aliases: string[]
  saving: boolean
  onClose: () => void
  onProviderSave: (provider: AiProviderCard, apiKey?: string) => Promise<void>
  onModelSave: (model: AiModelCatalogEntry) => Promise<void>
  onPolicySave: (policy: AiPolicyEntry) => Promise<void>
  onDiagnosticRun: (id: string) => Promise<void>
}) => {
  const { selection, provider, model, policy, diagnostic, aliases, saving, onClose, onProviderSave, onModelSave, onPolicySave, onDiagnosticRun } = props

  if (!selection) {
    return <EmptyState label="Select an item" />
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm font-semibold text-[var(--cp-ink)]">
          {selection.type === 'provider' && provider?.displayName}
          {selection.type === 'model' && model?.alias}
          {selection.type === 'policy' && policy?.label}
          {selection.type === 'diagnostic' && diagnostic?.title}
        </p>
        <button type="button" onClick={onClose} className="rounded-full border border-[var(--cp-border)] p-2 text-[var(--cp-muted)]">
          <Icon name="close" className="size-4" />
        </button>
      </div>

      {selection.type === 'provider' && provider ? <ProviderEditor provider={provider} saving={saving} onSave={onProviderSave} onRun={() => Promise.resolve()} /> : null}
      {selection.type === 'model' && model ? <ModelEditor model={model} saving={saving} onSave={onModelSave} /> : null}
      {selection.type === 'policy' && policy ? <PolicyEditor policy={policy} aliases={aliases} saving={saving} onSave={onPolicySave} /> : null}
      {selection.type === 'diagnostic' && diagnostic ? <DiagnosticDetails diagnostic={diagnostic} onRun={onDiagnosticRun} /> : null}
    </div>
  )
}

const ProviderEditor = (props: {
  provider: AiProviderCard
  saving: boolean
  onSave: (provider: AiProviderCard, apiKey?: string) => Promise<void>
  onRun: () => Promise<void>
}) => {
  const { provider, saving, onSave, onRun } = props
  const [draft, setDraft] = useState(provider)
  const [apiKey, setApiKey] = useState('')
  const [editing, setEditing] = useState(false)

  useEffect(() => {
    setDraft(provider)
    setApiKey('')
    setEditing(false)
  }, [provider])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-auto px-5 py-5">
        <div className="space-y-3">
          {editing ? (
            <>
              <Field label="Name"><input value={draft.displayName} onChange={(event) => setDraft({ ...draft, displayName: event.target.value })} className={inputClassName} /></Field>
              <Field label="Endpoint"><input value={draft.endpoint} onChange={(event) => setDraft({ ...draft, endpoint: event.target.value })} className={inputClassName} /></Field>
              <Field label="Auth mode"><input value={draft.authMode} onChange={(event) => setDraft({ ...draft, authMode: event.target.value })} className={inputClassName} /></Field>
              <Field label="Default model">
                <select value={draft.defaultModel} onChange={(event) => setDraft({ ...draft, defaultModel: event.target.value })} className={inputClassName}>
                  {(draft.availableModels?.length ? draft.availableModels : [draft.defaultModel]).map((model) => (
                    <option key={model} value={model}>{model}</option>
                  ))}
                </select>
              </Field>
              <Field label="Status">
                <select value={draft.status} onChange={(event) => setDraft({ ...draft, status: event.target.value as AiProviderCard['status'] })} className={inputClassName}>
                  <option value="healthy">healthy</option>
                  <option value="needs_setup">needs_setup</option>
                  <option value="degraded">degraded</option>
                  <option value="planned">planned</option>
                </select>
              </Field>
              <Field label="Note"><textarea value={draft.note} onChange={(event) => setDraft({ ...draft, note: event.target.value })} className={`${inputClassName} min-h-24 resize-none`} /></Field>
              <Field label={draft.credentialConfigured ? 'Replace API key' : 'API key'}>
                <input value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder={draft.credentialConfigured ? 'Leave blank to keep current key' : 'Enter API key'} className={inputClassName} />
              </Field>
            </>
          ) : (
            <div className="space-y-3">
              <ReadOnlyField label="Name" value={draft.displayName} />
              <ReadOnlyField label="Endpoint" value={draft.endpoint} />
              <ReadOnlyField label="Auth mode" value={draft.authMode} />
              <ReadOnlyField label="Default model" value={draft.defaultModel} />
              <ReadOnlyField label="Status" value={draft.status} />
              <ReadOnlyField label="API key" value={draft.maskedApiKey ?? 'Not configured'} />
              <ReadOnlyField label="Note" value={draft.note || '-'} multiline />
            </div>
          )}

          {!editing && !draft.credentialConfigured ? (
            <p className="text-xs text-[var(--cp-muted)]">`needs_setup` usually means there is no usable key yet.</p>
          ) : null}
        </div>
      </div>

      <div className="shrink-0 flex flex-wrap gap-2 border-t border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-5 py-4">
        {editing ? (
          <>
            <button
              type="button"
              disabled={saving}
              onClick={async () => {
                await onSave(draft, apiKey)
                setEditing(false)
              }}
              className={primaryButtonClassName}
            >
              {saving ? 'Saving...' : 'Save'}
            </button>
            <button
              type="button"
              disabled={saving}
              onClick={() => {
                setDraft(provider)
                setApiKey('')
                setEditing(false)
              }}
              className="inline-flex min-h-10 items-center justify-center rounded-full border border-[var(--cp-border)] px-4 py-2 text-sm font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
            >
              Cancel
            </button>
          </>
        ) : (
          <button type="button" onClick={() => setEditing(true)} className={primaryButtonClassName}>Edit</button>
        )}
        <button
          type="button"
          disabled={saving}
          onClick={() => void onRun()}
          className="inline-flex min-h-10 items-center justify-center rounded-full border border-[var(--cp-border)] px-4 py-2 text-sm font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
        >
          Test
        </button>
      </div>
    </div>
  )
}

const ReadOnlyField = (props: { label: string; value: string; multiline?: boolean }) => (
  <div className="space-y-2">
    <span className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">{props.label}</span>
    <div className={`rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-3 text-sm text-[var(--cp-ink)] ${props.multiline ? 'whitespace-pre-wrap leading-6' : ''}`}>
      {props.value}
    </div>
  </div>
)

const ModelEditor = (props: {
  model: AiModelCatalogEntry
  saving: boolean
  onSave: (model: AiModelCatalogEntry) => Promise<void>
}) => {
  const { model, saving, onSave } = props
  const [draft, setDraft] = useState(model)

  useEffect(() => setDraft(model), [model])

  return (
    <div className="space-y-3">
      <Field label="Provider model"><input value={draft.providerModel} onChange={(event) => setDraft({ ...draft, providerModel: event.target.value })} className={inputClassName} /></Field>
      <Field label="Features"><textarea value={draft.features.join(', ')} onChange={(event) => setDraft({ ...draft, features: splitCsv(event.target.value) })} className={`${inputClassName} min-h-24 resize-none`} /></Field>
      <Field label="Use cases"><textarea value={draft.useCases.join(', ')} onChange={(event) => setDraft({ ...draft, useCases: splitCsv(event.target.value) })} className={`${inputClassName} min-h-24 resize-none`} /></Field>
      <button type="button" disabled={saving} onClick={() => void onSave(draft)} className={primaryButtonClassName}>{saving ? 'Saving...' : 'Save'}</button>
    </div>
  )
}

const PolicyEditor = (props: {
  policy: AiPolicyEntry
  aliases: string[]
  saving: boolean
  onSave: (policy: AiPolicyEntry) => Promise<void>
}) => {
  const { policy, aliases, saving, onSave } = props
  const [draft, setDraft] = useState(policy)

  useEffect(() => setDraft(policy), [policy])

  return (
    <div className="space-y-3">
      <Field label="Primary model">
        <select value={draft.primaryModel} onChange={(event) => setDraft({ ...draft, primaryModel: event.target.value })} className={inputClassName}>
          {aliases.map((alias) => (
            <option key={alias} value={alias}>{alias}</option>
          ))}
        </select>
      </Field>
      <Field label="Fallbacks"><textarea value={draft.fallbackModels.join(', ')} onChange={(event) => setDraft({ ...draft, fallbackModels: splitCsv(event.target.value) })} className={`${inputClassName} min-h-24 resize-none`} /></Field>
      <Field label="Objective"><textarea value={draft.objective} onChange={(event) => setDraft({ ...draft, objective: event.target.value })} className={`${inputClassName} min-h-24 resize-none`} /></Field>
      <Field label="Status">
        <select value={draft.status} onChange={(event) => setDraft({ ...draft, status: event.target.value as AiPolicyEntry['status'] })} className={inputClassName}>
          <option value="active">active</option>
          <option value="review">review</option>
          <option value="planned">planned</option>
        </select>
      </Field>
      <button type="button" disabled={saving} onClick={() => void onSave(draft)} className={primaryButtonClassName}>{saving ? 'Saving...' : 'Save'}</button>
    </div>
  )
}

const DiagnosticDetails = (props: {
  diagnostic: AiDiagnosticEntry
  onRun: (id: string) => Promise<void>
}) => {
  const { diagnostic, onRun } = props

  return (
    <div className="space-y-3">
      <div className="rounded-2xl border border-[var(--cp-border)] bg-white p-4 text-sm leading-6 text-[var(--cp-muted)]">
        {diagnostic.detail}
      </div>
      <div className="flex items-center justify-between gap-3">
        <StateBadge tone={diagnostic.status}>{diagnostic.status}</StateBadge>
        <button type="button" onClick={() => void onRun(diagnostic.id)} className={primaryButtonClassName}>
          {diagnostic.actionLabel}
        </button>
      </div>
    </div>
  )
}

const Field = (props: { label: string; children: ReactNode }) => (
  <label className="block space-y-2">
    <span className="text-xs font-semibold uppercase tracking-wide text-[var(--cp-muted)]">{props.label}</span>
    {props.children}
  </label>
)

const MiniStat = (props: { label: string; value: string }) => (
  <div className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1.5 text-xs text-[var(--cp-muted)]">
    <span className="font-semibold text-[var(--cp-ink)]">{props.value}</span> {props.label}
  </div>
)

const StateBadge = (props: { tone: string; children: ReactNode }) => {
  const className =
    props.tone === 'healthy' || props.tone === 'active' || props.tone === 'pass'
      ? 'bg-emerald-100 text-emerald-700'
      : props.tone === 'needs_setup' || props.tone === 'review' || props.tone === 'warn'
        ? 'bg-amber-100 text-amber-700'
        : props.tone === 'degraded'
          ? 'bg-rose-100 text-rose-700'
          : 'bg-slate-100 text-slate-600'

  return <span className={`cp-pill ${className}`}>{props.children}</span>
}

const EmptyState = (props: { label: string }) => (
  <div className="rounded-3xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-5 py-8 text-sm text-[var(--cp-muted)]">
    {props.label}
  </div>
)

const readErrorMessage = (error: unknown, fallback: string) => {
  if (error instanceof Error && error.message.trim()) return error.message
  if (typeof error === 'string' && error.trim()) return error
  return fallback
}

const splitCsv = (value: string) =>
  value
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean)

const inputClassName =
  'w-full rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-3 text-sm text-[var(--cp-ink)] outline-none transition focus:border-[var(--cp-primary)] focus:ring-2 focus:ring-[color:rgba(15,118,110,0.12)]'

const primaryButtonClassName =
  'inline-flex min-h-10 items-center justify-center rounded-full bg-[var(--cp-primary)] px-4 py-2 text-sm font-semibold text-white transition hover:bg-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60'

export default AiModelsWindow
