import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { AlertTriangle, ChevronDown, ChevronRight, MoreHorizontal, RefreshCw, Save, Search, ShieldCheck, Trash2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore } from '../../hooks/use-aicc-store'
import { StatusBadge } from '../shared/StatusBadge'
import { ConfirmDialog } from '../shared/ConfirmDialog'
import type { AuthStatus, ModelMetadata, ProviderView } from '../../../../api/aicc_mgr'

type TFn = (k: string, f: string) => string
type FilterKey = 'apiType' | 'logicalMount' | 'health' | 'costClass' | 'latencyClass' | 'tier'
type MultiFilter = {
  query: string
  selected: string[]
}
type InventoryFilters = Record<FilterKey, MultiFilter>

function emptyMultiFilter(): MultiFilter {
  return { query: '', selected: [] }
}

function emptyInventoryFilters(): InventoryFilters {
  return {
    apiType: emptyMultiFilter(),
    logicalMount: emptyMultiFilter(),
    health: emptyMultiFilter(),
    costClass: emptyMultiFilter(),
    latencyClass: emptyMultiFilter(),
    tier: emptyMultiFilter(),
  }
}

function authStatusLabel(s: AuthStatus, t: TFn): string {
  switch (s) {
    case 'ok': return t('aiCenter.providers.authOk', 'Valid')
    case 'expired': return t('aiCenter.providers.authExpired', 'Expired')
    case 'invalid': return t('aiCenter.providers.authInvalid', 'Invalid')
    default: return t('aiCenter.providers.authUnknown', 'Unknown')
  }
}

function authStatusVariant(s: AuthStatus): 'ok' | 'warning' | 'error' | 'unknown' {
  switch (s) {
    case 'ok': return 'ok'
    case 'expired': return 'warning'
    case 'invalid': return 'error'
    default: return 'unknown'
  }
}

function modelHealthVariant(status: ModelMetadata['health']['status']): 'ok' | 'warning' | 'error' {
  if (status === 'available') return 'ok'
  if (status === 'degraded') return 'warning'
  return 'error'
}

interface ProviderDetailPanelProps {
  provider: ProviderView
  routingWeight: number
  onDeleted: () => void
}

export function ProviderDetailPanel({ provider, routingWeight, onDeleted }: ProviderDetailPanelProps) {
  return (
    <ProviderDetailPanelBody
      key={provider.config.provider_instance_name}
      provider={provider}
      routingWeight={routingWeight}
      onDeleted={onDeleted}
    />
  )
}

function ProviderDetailPanelBody({ provider, routingWeight, onDeleted }: ProviderDetailPanelProps) {
  const { t } = useI18n()
  const store = useAICCStore()
  const [showModels, setShowModels] = useState(true)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [showKeyDialog, setShowKeyDialog] = useState(false)
  const [apiKeyDraft, setApiKeyDraft] = useState('')
  const [updatingKey, setUpdatingKey] = useState(false)
  const [keyError, setKeyError] = useState<string | null>(null)
  const [keyFeedback, setKeyFeedback] = useState<string | null>(null)
  const [refreshingModels, setRefreshingModels] = useState(false)
  const [refreshFeedback, setRefreshFeedback] = useState<string | null>(null)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [weightDraft, setWeightDraft] = useState(() => formatWeight(routingWeight))
  const [savingWeight, setSavingWeight] = useState(false)
  const [weightError, setWeightError] = useState<string | null>(null)
  const [actionsOpen, setActionsOpen] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [filters, setFilters] = useState<InventoryFilters>(() => emptyInventoryFilters())
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(() => new Set())

  const { config, status, account, inventory } = provider
  const models = status.discovered_models
  const degradedCount = models.filter((m) => m.health.status === 'degraded').length
  const quotaWarningCount = models.filter((m) => m.health.quota_state === 'near_limit' || m.health.quota_state === 'exhausted').length
  const groupedModels = useMemo(
    () => groupInventoryModels(models, searchQuery, filters),
    [models, searchQuery, filters],
  )
  const filterOptions = useMemo(() => buildInventoryFilterOptions(models), [models])

  const parsedWeight = Number(weightDraft)
  const weightValid = Number.isFinite(parsedWeight) && parsedWeight >= 0
  const clampedSliderWeight = weightValid ? Math.min(parsedWeight, 5) : 1
  const weightChanged = weightValid && Math.abs(parsedWeight - routingWeight) > 0.0001

  const updateWeightDraft = (value: string) => {
    setWeightDraft(value)
    setWeightError(null)
  }

  const handleSaveWeight = () => {
    if (!weightValid) {
      setWeightError(t('aiCenter.providers.routingWeightInvalid', 'Use a non-negative number.'))
      return
    }
    setSavingWeight(true)
    void store.setProviderRoutingWeight(config.provider_instance_name, parsedWeight)
      .catch((error) => {
        console.error('aicc.setProviderRoutingWeight failed', error)
        setWeightError(t('aiCenter.providers.routingWeightSaveFailed', 'Could not save routing weight.'))
      })
      .finally(() => setSavingWeight(false))
  }

  const handleUpdateKey = async () => {
    const nextKey = apiKeyDraft.trim()
    if (!nextKey) {
      setKeyError(t('aiCenter.providers.apiKeyRequired', 'Enter an API key.'))
      return
    }
    setUpdatingKey(true)
    setKeyError(null)
    try {
      await store.updateProviderKey(provider, nextKey)
      setApiKeyDraft('')
      setShowKeyDialog(false)
      setKeyFeedback(t('aiCenter.providers.updateKeySuccess', 'API key updated and AICC reloaded.'))
      setRefreshFeedback(t('aiCenter.providers.modelsRefreshedNow', 'Provider models refreshed just now.'))
    } catch (error) {
      console.error('aicc.updateProviderKey failed', error)
      setKeyError(errorMessage(error, t('aiCenter.providers.updateKeyFailed', 'Could not update API key.')))
    } finally {
      setUpdatingKey(false)
    }
  }

  const handleRefreshModels = async () => {
    setRefreshingModels(true)
    setRefreshError(null)
    setRefreshFeedback(null)
    try {
      await store.refreshProviderModels(config.id)
      setRefreshFeedback(t('aiCenter.providers.refreshModelsSuccess', 'Provider models refreshed.'))
    } catch (error) {
      console.error('aicc.refreshProviderModels failed', error)
      setRefreshError(errorMessage(error, t('aiCenter.providers.refreshModelsFailed', 'Could not refresh provider models.')))
    } finally {
      setRefreshingModels(false)
    }
  }

  const handleDelete = async () => {
    setDeleting(true)
    setDeleteError(null)
    try {
      await store.deleteProvider(config.id)
      setConfirmDelete(false)
      onDeleted()
    } catch (error) {
      console.error('aicc.deleteProvider failed', error)
      setDeleteError(errorMessage(error, t('aiCenter.providers.deleteFailed', 'Could not delete provider.')))
    } finally {
      setDeleting(false)
    }
  }

  const setFilter = (key: FilterKey, value: MultiFilter) => {
    setFilters((current) => ({ ...current, [key]: value }))
  }

  const toggleGroup = (groupKey: string) => {
    setExpandedGroups((current) => {
      const next = new Set(current)
      if (next.has(groupKey)) next.delete(groupKey)
      else next.add(groupKey)
      return next
    })
  }

  const balanceDisplay = account.balance_supported && account.balance_value != null
    ? `${account.balance_unit === 'usd' ? '$' : ''}${account.balance_value}${account.balance_unit === 'credit' ? ' Credit' : ''}`
    : t('aiCenter.providers.usageOnly', 'Usage only')
  const routingWeightLabel = routingWeight === 0
    ? t('aiCenter.providers.routingDisabled', 'Disabled for routing')
    : routingWeight < 1
      ? t('aiCenter.providers.routingDownweighted', 'Downweighted')
      : routingWeight > 1
        ? t('aiCenter.providers.routingUpweighted', 'Upweighted')
        : t('aiCenter.providers.routingDefault', 'Default')

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <h2 className="truncate text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
            {config.name}
          </h2>
          <div className="truncate text-xs" style={{ color: 'var(--cp-muted)' }}>
            {config.provider_instance_name} / {config.provider_runtime_type} / {config.provider_origin}
          </div>
        </div>
        <div className="relative shrink-0">
          <button
            type="button"
            onClick={() => setActionsOpen((value) => !value)}
            className="flex h-9 w-9 items-center justify-center rounded-lg"
            style={{ color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}
            aria-label={t('aiCenter.providers.actions', 'Provider actions')}
          >
            <MoreHorizontal size={18} />
          </button>
          {actionsOpen && (
            <div
              className="absolute right-0 top-10 z-10 flex w-48 flex-col rounded-lg p-1 shadow-lg"
              style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
            >
              <MenuAction
                icon={<ShieldCheck size={14} />}
                label={t('aiCenter.providers.updateKey', 'Update Key')}
                onClick={() => {
                  setActionsOpen(false)
                  setShowKeyDialog(true)
                  setKeyError(null)
                  setKeyFeedback(null)
                }}
              />
              <MenuAction
                icon={<RefreshCw size={14} className={refreshingModels ? 'animate-spin' : ''} />}
                label={refreshingModels ? t('aiCenter.providers.refreshingModels', 'Refreshing Models') : t('aiCenter.providers.refreshModels', 'Refresh Models')}
                onClick={() => {
                  setActionsOpen(false)
                  void handleRefreshModels()
                }}
                disabled={refreshingModels}
              />
              <MenuAction
                icon={<Trash2 size={14} />}
                label={t('aiCenter.providers.delete', 'Delete')}
                onClick={() => {
                  setActionsOpen(false)
                  setConfirmDelete(true)
                  setDeleteError(null)
                }}
                danger
              />
            </div>
          )}
        </div>
      </div>

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-[minmax(260px,0.9fr)_minmax(0,2fr)]">
        <div className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          <div className="min-w-0 text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.providers.providerStatus', 'Provider Status')}
          </div>
          <div className="mt-2 truncate text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{config.name}</div>
          <div className="mt-3 grid grid-cols-2 gap-2">
            <StatusTile label={t('aiCenter.providers.auth', 'Authentication')} value={authStatusLabel(status.auth_status, t)} tone={authStatusVariant(status.auth_status)} />
            <StatusTile label={t('aiCenter.providers.health', 'Health')} value={`${models.length - degradedCount}/${models.length}`} tone={degradedCount > 0 ? 'warning' : 'ok'} />
            <StatusTile label={t('aiCenter.providers.models', 'Models')} value={models.length.toString()} />
            <StatusTile label={t('aiCenter.providers.issues', 'Issues')} value={(degradedCount + quotaWarningCount).toString()} tone={degradedCount + quotaWarningCount > 0 ? 'warning' : 'ok'} />
          </div>
        </div>
        <div className="relative min-w-0">
          <div className="flex snap-x snap-mandatory gap-3 overflow-x-auto pb-1 pr-12 md:grid md:grid-cols-4 md:overflow-visible md:pr-0">
            <Metric label={t('aiCenter.providers.inventoryModels', 'Inventory Models')} value={models.length.toString()} detail={inventory.inventory_revision} />
            <Metric label={t('aiCenter.providers.health', 'Health')} value={`${models.length - degradedCount}/${models.length}`} detail={t('aiCenter.providers.availableModels', 'available models')} />
            <Metric label={t('aiCenter.providers.quota', 'Quota')} value={quotaWarningCount ? `${quotaWarningCount}` : '0'} detail={quotaWarningCount ? t('aiCenter.providers.quotaWarning', 'needs attention') : t('aiCenter.providers.quotaNormal', 'normal')} />
            <Metric label={t('aiCenter.providers.routingWeight', 'Routing Weight')} value={formatWeight(routingWeight)} detail={routingWeightLabel} />
          </div>
          <div
            className="pointer-events-none absolute bottom-1 right-0 top-0 w-14 md:hidden"
            style={{ background: 'linear-gradient(90deg, transparent, var(--cp-bg))' }}
          />
        </div>
      </div>

      <div
        className="rounded-xl p-4 flex flex-col gap-3"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <Row label={t('aiCenter.providers.driver', 'Driver')} value={config.provider_driver} />
        <Row label={t('aiCenter.providers.routingWeight', 'Routing Weight')} value={`${formatWeight(routingWeight)} / ${routingWeightLabel}`} />
        <Row label={t('aiCenter.providers.runtimeType', 'Runtime Type')} value={config.provider_runtime_type} />
        <Row label={t('aiCenter.providers.endpoint', 'Endpoint')} value={config.endpoint || t('aiCenter.providers.default', 'Default')} />
        <Row
          label={t('aiCenter.providers.auth', 'Authentication')}
          value={
            <span className="inline-flex items-center gap-2">
              {config.auth_mode ?? '-'}
              <StatusBadge status={authStatusVariant(status.auth_status)} label={authStatusLabel(status.auth_status, t)} />
            </span>
          }
        />
        <Row label={t('aiCenter.providers.pricingMode', 'Pricing Mode')} value={account.pricing_mode} />
        <Row label={t('aiCenter.providers.balance', 'Balance')} value={balanceDisplay} />
        <Row label={t('aiCenter.providers.lastSync', 'Last Sync')} value={formatTimestamp(status.last_model_sync_at)} />
        <Row
          label={t('aiCenter.providers.autoSync', 'Auto Sync')}
          value={config.auto_sync_models ? t('common.on', 'On') : t('common.off', 'Off')}
        />
      </div>

      <div
        className="rounded-xl p-4 flex flex-col gap-3"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
              {t('aiCenter.providers.routingWeight', 'Routing Weight')}
            </div>
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {routingWeightLabel}
            </div>
          </div>
          <button
            type="button"
            onClick={handleSaveWeight}
            disabled={!weightChanged || savingWeight}
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-medium disabled:opacity-45"
            style={{
              border: '1px solid var(--cp-border)',
              color: 'var(--cp-text)',
            }}
          >
            <Save size={14} />
            {savingWeight ? t('common.saving', 'Saving') : t('common.save', 'Save')}
          </button>
        </div>
        <div className="grid grid-cols-[1fr_5rem] gap-3 items-center">
          <input
            type="range"
            min="0"
            max="5"
            step="0.1"
            value={clampedSliderWeight}
            onChange={(event) => updateWeightDraft(event.target.value)}
            aria-label={t('aiCenter.providers.routingWeight', 'Routing Weight')}
          />
          <input
            type="number"
            min="0"
            step="0.1"
            value={weightDraft}
            onChange={(event) => updateWeightDraft(event.target.value)}
            className="w-20 rounded-lg px-2 py-1 text-sm"
            style={{
              background: 'var(--cp-bg)',
              color: 'var(--cp-text)',
              border: '1px solid var(--cp-border)',
            }}
            aria-label={t('aiCenter.providers.routingWeightValue', 'Routing weight value')}
          />
        </div>
        <div className="text-xs" style={{ color: weightError ? 'var(--cp-danger)' : 'var(--cp-muted)' }}>
          {weightError ?? t('aiCenter.providers.routingWeightHint', '0 disables routing, 1 keeps the default, values above 1 prefer this provider.')}
        </div>
      </div>

      {status.model_sync_status !== 'ok' && (
        <InlineNotice tone="warning">
          <AlertTriangle size={16} style={{ color: 'var(--cp-warning)' }} />
          <span>{t('aiCenter.providers.syncFailed', 'Last inventory sync failed. Existing models remain usable, but refresh is recommended.')}</span>
        </InlineNotice>
      )}
      {keyFeedback && <InlineNotice tone="success">{keyFeedback}</InlineNotice>}
      {refreshFeedback && (
        <InlineNotice tone="success">
          {refreshFeedback} {t('aiCenter.providers.updatedAt', 'Updated at')} {new Date().toLocaleTimeString()}
        </InlineNotice>
      )}
      {refreshError && <InlineNotice tone="error">{refreshError}</InlineNotice>}

      <button
        type="button"
        onClick={() => setShowModels(!showModels)}
        className="inline-flex items-center gap-1 text-sm self-start"
        style={{ color: 'var(--cp-accent)' }}
      >
        {showModels ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        {t('aiCenter.providers.inventory', 'Provider Inventory')}
      </button>

      {showModels && (
        <div className="flex flex-col gap-3">
          <InventoryToolbar
            t={t}
            searchQuery={searchQuery}
            onSearchChange={setSearchQuery}
            filters={filters}
            filterOptions={filterOptions}
            onFilterChange={setFilter}
          />
          {groupedModels.length > 0 ? (
            <div className="grid grid-cols-1 gap-3">
              {groupedModels.map((group) => (
                <ModelInventoryGroup
                  key={group.key}
                  group={group}
                  expanded={expandedGroups.has(group.key)}
                  onToggle={() => toggleGroup(group.key)}
                  t={t}
                />
              ))}
            </div>
          ) : (
            <div className="rounded-xl p-4 text-sm" style={{ background: 'var(--cp-surface)', color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}>
              {t('aiCenter.providers.noModelsMatch', 'No models match the current search and filters.')}
            </div>
          )}
        </div>
      )}

      <UpdateKeyDialog
        open={showKeyDialog}
        apiKey={apiKeyDraft}
        error={keyError}
        loading={updatingKey}
        t={t}
        onChange={(value) => {
          setApiKeyDraft(value)
          setKeyError(null)
        }}
        onConfirm={handleUpdateKey}
        onCancel={() => {
          if (!updatingKey) {
            setShowKeyDialog(false)
            setApiKeyDraft('')
            setKeyError(null)
          }
        }}
      />

      <ConfirmDialog
        open={confirmDelete}
        title={t('aiCenter.providers.deleteTitle', 'Delete Provider')}
        message={t('aiCenter.providers.deleteConfirm', 'Are you sure you want to delete this provider? This action cannot be undone.')}
        confirmLabel={t('aiCenter.providers.delete', 'Delete')}
        confirming={deleting}
        error={deleteError}
        onConfirm={() => { void handleDelete() }}
        onCancel={() => {
          if (!deleting) setConfirmDelete(false)
        }}
      />
    </div>
  )
}

function Metric({ label, value, detail }: { label: string; value: string; detail?: string }) {
  return (
    <div
      className="grid h-full min-h-[92px] min-w-[72%] snap-start grid-rows-[2rem_1.75rem_1rem] content-start gap-1 rounded-xl p-3 sm:min-w-[220px] md:min-w-0"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
    >
      <div className="text-xs leading-4" style={{ color: 'var(--cp-muted)' }}>{label}</div>
      <div className="text-xl font-semibold leading-tight" style={{ color: 'var(--cp-text)' }}>{value}</div>
      <div className="text-[11px] truncate" style={{ color: 'var(--cp-muted)' }}>{detail ?? ''}</div>
    </div>
  )
}

function StatusTile({ label, value, tone = 'unknown' }: { label: string; value: string; tone?: 'ok' | 'warning' | 'error' | 'unknown' }) {
  const color = tone === 'ok'
    ? 'var(--cp-success)'
    : tone === 'warning'
      ? 'var(--cp-warning)'
      : tone === 'error'
        ? 'var(--cp-danger)'
        : 'var(--cp-text)'
  return (
    <div className="min-w-0 rounded-lg px-2 py-2" style={{ background: 'var(--cp-bg)' }}>
      <div className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{label}</div>
      <div className="mt-1 truncate text-sm font-semibold" style={{ color }}>{value}</div>
    </div>
  )
}

function InventoryToolbar({
  t,
  searchQuery,
  onSearchChange,
  filters,
  filterOptions,
  onFilterChange,
}: {
  t: TFn
  searchQuery: string
  onSearchChange: (value: string) => void
  filters: InventoryFilters
  filterOptions: Record<FilterKey, string[]>
  onFilterChange: (key: FilterKey, value: MultiFilter) => void
}) {
  return (
    <div className="rounded-xl p-3 flex flex-col gap-3" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <label className="relative block">
        <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: 'var(--cp-muted)' }} />
        <input
          value={searchQuery}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={t('aiCenter.providers.searchModels', 'Search models')}
          className="w-full rounded-lg py-2 pl-9 pr-3 text-sm"
          style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        />
      </label>
      <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6 gap-2">
        <MultiSelectFilter label={t('aiCenter.providers.apiType', 'API Type')} value={filters.apiType} options={filterOptions.apiType} onChange={(value) => onFilterChange('apiType', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.logicalMount', 'Logical Mount')} value={filters.logicalMount} options={filterOptions.logicalMount} onChange={(value) => onFilterChange('logicalMount', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.health', 'Health')} value={filters.health} options={filterOptions.health} onChange={(value) => onFilterChange('health', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.costClass', 'Cost Class')} value={filters.costClass} options={filterOptions.costClass} onChange={(value) => onFilterChange('costClass', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.latencyClass', 'Latency Class')} value={filters.latencyClass} options={filterOptions.latencyClass} onChange={(value) => onFilterChange('latencyClass', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.tier', 'Tier')} value={filters.tier} options={filterOptions.tier} onChange={(value) => onFilterChange('tier', value)} />
      </div>
    </div>
  )
}

function MultiSelectFilter({ label, value, options, onChange }: { label: string; value: MultiFilter; options: string[]; onChange: (value: MultiFilter) => void }) {
  const selectedCount = value.selected.length
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const toggleOption = (option: string) => {
    const selected = value.selected.includes(option)
      ? value.selected.filter((item) => item !== option)
      : [...value.selected, option]
    onChange({ ...value, selected })
  }

  useEffect(() => {
    if (!open) return
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false)
      }
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false)
      }
    }
    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [open])

  return (
    <div ref={rootRef} className="relative flex flex-col gap-1 min-w-0">
      <span className="truncate text-[11px]" title={label} style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <div
        className="flex min-h-8 items-center rounded-lg"
        style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
      >
        <input
          value={value.query}
          onChange={(event) => onChange({ ...value, query: event.target.value })}
          placeholder={selectedCount > 0 ? `${selectedCount} selected` : 'All'}
          className="min-w-0 flex-1 rounded-l-lg bg-transparent px-2 py-1.5 text-xs outline-none"
          style={{ color: 'var(--cp-text)' }}
        />
        <button
          type="button"
          onClick={() => setOpen((current) => !current)}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-r-lg"
          style={{ color: selectedCount > 0 ? 'var(--cp-accent)' : 'var(--cp-muted)', borderLeft: '1px solid var(--cp-border)' }}
          aria-label={`${label} options`}
        >
          <ChevronDown size={14} />
        </button>
      </div>
      {open && (
        <div
          className="absolute left-0 top-[3.35rem] z-20 flex max-h-56 w-full min-w-48 flex-col gap-1 overflow-auto rounded-lg p-2 shadow-lg"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          <button
            type="button"
            onClick={() => onChange({ ...value, selected: [] })}
            className="rounded px-2 py-1 text-left text-xs"
            style={{ color: 'var(--cp-accent)' }}
          >
            All
          </button>
          {options.map((option) => (
            <label key={option} className="flex min-h-7 items-center gap-2 rounded px-2 py-1 text-xs" style={{ color: 'var(--cp-text)' }}>
              <input
                type="checkbox"
                checked={value.selected.includes(option)}
                onChange={() => toggleOption(option)}
              />
              <span className="truncate" title={option}>{option}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  )
}

type ModelGroup = {
  key: string
  label: string
  primary: ModelMetadata
  variants: ModelMetadata[]
}

function ModelInventoryGroup({ group, expanded, onToggle, t }: { group: ModelGroup; expanded: boolean; onToggle: () => void; t: TFn }) {
  const hasVariants = group.variants.length > 0
  return (
    <div className="rounded-xl overflow-hidden" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <ModelInventoryRow model={group.primary} t={t} groupLabel={group.label} />
      {hasVariants && (
        <button
          type="button"
          onClick={onToggle}
          className="w-full flex items-center justify-between px-3 py-2 text-xs"
          style={{ color: 'var(--cp-accent)', borderTop: '1px solid var(--cp-border)' }}
        >
          <span>{group.variants.length} {t('aiCenter.providers.variants', 'variants')}</span>
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </button>
      )}
      {hasVariants && expanded && (
        <div className="grid grid-cols-1 gap-2 p-2" style={{ borderTop: '1px solid var(--cp-border)' }}>
          {group.variants.map((model) => (
            <ModelInventoryRow key={model.exact_model} model={model} t={t} compact />
          ))}
        </div>
      )}
    </div>
  )
}

function ModelInventoryRow({ model, t, compact = false, groupLabel }: { model: ModelMetadata; t: TFn; compact?: boolean; groupLabel?: string }) {
  return (
    <div className={compact ? 'rounded-lg p-3' : 'p-3'} style={compact ? { background: 'var(--cp-bg)' } : undefined}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="text-sm font-medium truncate" style={{ color: 'var(--cp-text)' }}>{model.provider_model_id}</div>
          <div className="text-xs truncate" style={{ color: 'var(--cp-muted)' }}>
            {groupLabel ? `${groupLabel} / ` : ''}{model.logical_mounts.join(', ')} / {model.api_types.join(', ')}
          </div>
        </div>
        <StatusBadge status={modelHealthVariant(model.health.status)} label={model.health.status} />
      </div>
      <div className="grid grid-cols-2 md:grid-cols-5 gap-2 mt-3 text-xs">
        <Chip label={t('aiCenter.providers.quality', 'quality')} value={model.attributes.quality_score?.toString() ?? '-'} />
        <Chip label={t('aiCenter.providers.tier', 'tier')} value={model.attributes.tier ?? '-'} />
        <Chip label={t('aiCenter.providers.latency', 'latency')} value={`${model.attributes.latency_class}${model.health.p95_latency_ms ? ` p95 ${model.health.p95_latency_ms}ms` : ''}`} />
        <Chip label={t('aiCenter.providers.cost', 'cost')} value={model.attributes.cost_class} />
        <Chip label={t('aiCenter.providers.quota', 'quota')} value={model.health.quota_state} />
      </div>
    </div>
  )
}

function UpdateKeyDialog({
  open,
  apiKey,
  error,
  loading,
  t,
  onChange,
  onConfirm,
  onCancel,
}: {
  open: boolean
  apiKey: string
  error: string | null
  loading: boolean
  t: TFn
  onChange: (value: string) => void
  onConfirm: () => void
  onCancel: () => void
}) {
  if (!open) return null
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.4)' }} onClick={onCancel} />
      <div className="relative rounded-xl p-6 max-w-md w-full mx-4 shadow-lg" style={{ background: 'var(--cp-surface)' }}>
        <h3 className="text-base font-semibold mb-2" style={{ color: 'var(--cp-text)' }}>
          {t('aiCenter.providers.updateKey', 'Update Key')}
        </h3>
        <p className="text-sm mb-4" style={{ color: 'var(--cp-muted)' }}>
          {t('aiCenter.providers.updateKeyHint', 'The key is written through the current provider configuration path, then AICC is reloaded.')}
        </p>
        <label className="flex flex-col gap-1 text-sm">
          <span style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.providers.apiKey', 'API Key')}</span>
          <input
            type="password"
            value={apiKey}
            onChange={(event) => onChange(event.target.value)}
            autoFocus
            className="rounded-lg px-3 py-2"
            style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          />
        </label>
        {error && <div className="mt-3 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</div>}
        <div className="flex justify-end gap-3 mt-6">
          <button type="button" onClick={onCancel} disabled={loading} className="px-4 py-2 rounded-lg text-sm disabled:opacity-60" style={{ color: 'var(--cp-muted)' }}>
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={loading}
            className="inline-flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium disabled:opacity-60"
            style={{ background: 'var(--cp-accent)', color: '#fff' }}
          >
            {loading && <RefreshCw size={14} className="animate-spin" />}
            {loading ? t('common.saving', 'Saving') : t('common.save', 'Save')}
          </button>
        </div>
      </div>
    </div>
  )
}

function InlineNotice({ tone, children }: { tone: 'success' | 'warning' | 'error'; children: ReactNode }) {
  const color = tone === 'success' ? 'var(--cp-success)' : tone === 'warning' ? 'var(--cp-warning)' : 'var(--cp-danger)'
  return (
    <div
      className="flex items-start gap-2 rounded-lg p-3 text-sm"
      style={{
        background: `color-mix(in oklch, ${color}, transparent 88%)`,
        color: 'var(--cp-text)',
      }}
    >
      {children}
    </div>
  )
}

function Chip({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md px-2 py-1 min-w-0" style={{ background: 'var(--cp-bg)' }}>
      <span style={{ color: 'var(--cp-muted)' }}>{label}: </span>
      <span className="break-all" style={{ color: 'var(--cp-text)' }}>{value}</span>
    </div>
  )
}

function Row({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex justify-between gap-4 items-center text-sm">
      <span style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="font-medium text-right break-all" style={{ color: 'var(--cp-text)' }}>{value}</span>
    </div>
  )
}

function MenuAction({ icon, label, onClick, danger, disabled }: { icon: ReactNode; label: string; onClick: () => void; danger?: boolean; disabled?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-xs font-medium transition-opacity hover:opacity-80 disabled:opacity-50"
      style={{
        color: danger ? 'var(--cp-danger)' : 'var(--cp-text)',
      }}
    >
      {icon}
      {label}
    </button>
  )
}

function groupInventoryModels(models: ModelMetadata[], query: string, filters: InventoryFilters): ModelGroup[] {
  const normalizedQuery = query.trim().toLowerCase()
  const filtered = models
    .filter((model) => modelMatchesQuery(model, normalizedQuery))
    .filter((model) => modelMatchesFilters(model, filters))
    .sort(compareModelPriority)

  const groups = new Map<string, ModelMetadata[]>()
  for (const model of filtered) {
    const key = baseModelKey(model)
    groups.set(key, [...(groups.get(key) ?? []), model])
  }

  return Array.from(groups.entries())
    .map(([key, items]) => {
      const sorted = [...items].sort(compareModelPriority)
      const [primary, ...variants] = sorted
      if (!primary) return null
      return {
        key,
        label: key,
        primary,
        variants,
      }
    })
    .filter((group): group is ModelGroup => group != null)
    .sort((left, right) => compareModelPriority(left.primary, right.primary))
}

function modelMatchesQuery(model: ModelMetadata, query: string): boolean {
  if (!query) return true
  const haystack = [
    model.provider_model_id,
    model.exact_model,
    model.model_driver,
    ...model.api_types,
    ...model.logical_mounts,
    model.attributes.tier ?? '',
    model.attributes.cost_class,
    model.attributes.latency_class,
    model.health.status,
  ].join(' ').toLowerCase()
  return haystack.includes(query)
}

function modelMatchesFilters(model: ModelMetadata, filters: InventoryFilters): boolean {
  return multiFilterMatches(filters.apiType, model.api_types)
    && multiFilterMatches(filters.logicalMount, model.logical_mounts)
    && multiFilterMatches(filters.health, [model.health.status])
    && multiFilterMatches(filters.costClass, [model.attributes.cost_class])
    && multiFilterMatches(filters.latencyClass, [model.attributes.latency_class])
    && multiFilterMatches(filters.tier, [model.attributes.tier ?? '-'])
}

function multiFilterMatches(filter: MultiFilter, values: string[]): boolean {
  if (filter.selected.length > 0 && !values.some((value) => filter.selected.includes(value))) return false
  const query = filter.query.trim().toLowerCase()
  if (query && !values.some((value) => value.toLowerCase().includes(query))) return false
  return true
}

function buildInventoryFilterOptions(models: ModelMetadata[]): Record<FilterKey, string[]> {
  return {
    apiType: uniqueSorted(models.flatMap((model) => model.api_types)),
    logicalMount: uniqueSorted(models.flatMap((model) => model.logical_mounts)),
    health: uniqueSorted(models.map((model) => model.health.status)),
    costClass: uniqueSorted(models.map((model) => model.attributes.cost_class)),
    latencyClass: uniqueSorted(models.map((model) => model.attributes.latency_class)),
    tier: uniqueSorted(models.map((model) => model.attributes.tier ?? '-')),
  }
}

function uniqueSorted(values: string[]): string[] {
  return Array.from(new Set(values.filter(Boolean))).sort((left, right) => left.localeCompare(right))
}

function compareModelPriority(left: ModelMetadata, right: ModelMetadata): number {
  return mainModelScore(right) - mainModelScore(left)
    || (right.attributes.quality_score ?? 0) - (left.attributes.quality_score ?? 0)
    || tierScore(right.attributes.tier) - tierScore(left.attributes.tier)
    || versionScore(right.provider_model_id) - versionScore(left.provider_model_id)
    || left.provider_model_id.localeCompare(right.provider_model_id)
}

function mainModelScore(model: ModelMetadata): number {
  let score = 0
  if (model.logical_mounts.some((mount) => mount === 'llm' || mount.endsWith('.standard') || mount.endsWith('.balanced'))) score += 4
  if (model.attributes.tier === 'flagship') score += 3
  if (variantKind(model) === 'base') score += 2
  if (model.health.status === 'available') score += 1
  return score
}

function tierScore(tier?: ModelMetadata['attributes']['tier']): number {
  if (tier === 'flagship') return 3
  if (tier === 'mid') return 2
  if (tier === 'nano') return 1
  return 0
}

function versionScore(value: string): number {
  const matches = value.match(/\d+(?:\.\d+)?/g) ?? []
  return matches.reduce((score, item, index) => score + Number(item) / (index + 1), 0)
}

function baseModelKey(model: ModelMetadata): string {
  return (model.provider_actual_model_id ?? model.provider_model_id)
    .replace(/@(.*)$/, '')
    .replace(/[-_.](reasoning|thinking|think|mini|nano|small|flash|lite|preview|latest|turbo|fast)$/i, '')
}

function variantKind(model: ModelMetadata): 'base' | 'variant' {
  if (model.provider_actual_model_id && model.provider_actual_model_id !== model.provider_model_id) return 'variant'
  return baseModelKey(model).toLowerCase() === model.provider_model_id.replace(/@(.*)$/, '').toLowerCase() ? 'base' : 'variant'
}

function formatWeight(weight: number): string {
  return weight.toFixed(2).replace(/\.?0+$/, '')
}

function formatTimestamp(value?: string): string {
  if (!value) return '-'
  return value.slice(0, 16).replace('T', ' ')
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback
}
