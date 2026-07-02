import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useMediaQuery } from '@mui/material'
import { AlertTriangle, ArrowLeft, ChevronDown, ChevronRight, Filter, MoreHorizontal, RefreshCw, Save, Search, ShieldCheck, Trash2 } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore } from '../../hooks/use-aicc-store'
import { StatusBadge } from '../shared/StatusBadge'
import { ConfirmDialog } from '../shared/ConfirmDialog'
import { LongField } from '../shared/LongField'
import type { AuthStatus, ModelMetadata, ProviderView } from '../../../../api/aicc_mgr'

type TFn = (k: string, f: string) => string
type FilterKey = 'apiType' | 'logicalMount' | 'health' | 'costClass' | 'latencyClass' | 'tier'
type ProviderDetailSection = 'overview' | 'routing' | 'inventory'
type MultiFilter = {
  query: string
  selected: string[]
}
type InventoryFilters = Record<FilterKey, MultiFilter>
type ProviderMetric = {
  label: string
  value: string
  detail?: string
  tone?: 'default' | 'ok' | 'warning' | 'accent'
}

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
  onBack?: () => void
}

export function ProviderDetailPanel({ provider, routingWeight, onDeleted, onBack }: ProviderDetailPanelProps) {
  return (
    <ProviderDetailPanelBody
      key={provider.config.provider_instance_name}
      provider={provider}
      routingWeight={routingWeight}
      onDeleted={onDeleted}
      onBack={onBack}
    />
  )
}

function ProviderDetailPanelBody({ provider, routingWeight, onDeleted, onBack }: ProviderDetailPanelProps) {
  const { t } = useI18n()
  const store = useAICCStore()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [activeSection, setActiveSection] = useState<ProviderDetailSection>('overview')
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
  const [inventoryFiltersOpen, setInventoryFiltersOpen] = useState(false)
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(() => new Set())
  const actionsRef = useRef<HTMLDivElement | null>(null)

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

  useEffect(() => {
    if (!actionsOpen) return
    const handlePointerDown = (event: PointerEvent) => {
      if (!actionsRef.current?.contains(event.target as Node)) {
        setActionsOpen(false)
      }
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setActionsOpen(false)
      }
    }
    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [actionsOpen])

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
  const providerMetrics: ProviderMetric[] = [
    { label: t('aiCenter.providers.providerStatus', 'Provider Status'), value: config.name, detail: `${models.length} ${t('aiCenter.providers.models', 'Models')} / ${degradedCount + quotaWarningCount} ${t('aiCenter.providers.issues', 'Issues')}`, tone: degradedCount + quotaWarningCount > 0 ? 'warning' : 'ok' },
    { label: t('aiCenter.providers.inventoryModels', 'Inventory Models'), value: models.length.toString(), detail: inventory.inventory_revision, tone: 'accent' },
    { label: t('aiCenter.providers.quota', 'Quota'), value: quotaWarningCount ? `${quotaWarningCount}` : '0', detail: quotaWarningCount ? t('aiCenter.providers.quotaWarning', 'needs attention') : t('aiCenter.providers.quotaNormal', 'normal'), tone: quotaWarningCount ? 'warning' : 'ok' },
    { label: t('aiCenter.providers.routingWeight', 'Routing Weight'), value: formatWeight(routingWeight), detail: routingWeightLabel, tone: routingWeight === 0 ? 'warning' : 'default' },
  ]
  const desktopMetrics = [
    { label: t('aiCenter.providers.inventoryModels', 'Inventory Models'), value: models.length.toString(), detail: inventory.inventory_revision },
    { label: t('aiCenter.providers.health', 'Health'), value: `${models.length - degradedCount}/${models.length}`, detail: t('aiCenter.providers.availableModels', 'available models') },
    { label: t('aiCenter.providers.quota', 'Quota'), value: quotaWarningCount ? `${quotaWarningCount}` : '0', detail: quotaWarningCount ? t('aiCenter.providers.quotaWarning', 'needs attention') : t('aiCenter.providers.quotaNormal', 'normal') },
    { label: t('aiCenter.providers.routingWeight', 'Routing Weight'), value: formatWeight(routingWeight), detail: routingWeightLabel },
  ]

  return (
    <div className="flex flex-col gap-4">
      <div className={isMobile ? 'grid grid-cols-[2.25rem_minmax(0,1fr)_2.25rem] items-center gap-2' : 'flex items-center justify-between gap-3'}>
        <div className={isMobile ? 'flex items-center' : 'flex min-w-0 flex-1 items-center gap-2'}>
          {isMobile && onBack && (
            <button
              type="button"
              onClick={onBack}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg"
              style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
              aria-label={t('common.back', 'Back')}
            >
              <ArrowLeft size={18} />
            </button>
          )}
          {!isMobile && <h2 className="min-w-0 flex-1 truncate text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
            {config.name}
          </h2>}
        </div>
        {isMobile && <h2 className="min-w-0 truncate text-center text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>{config.name}</h2>}
        <div ref={actionsRef} className="relative shrink-0">
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
            isMobile ? (
              <div className="fixed inset-0 z-40 flex items-end justify-center">
                <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.35)' }} onClick={() => setActionsOpen(false)} />
                <div className="relative w-full rounded-t-xl p-3 shadow-lg" style={{ background: 'var(--cp-surface)', borderTop: '1px solid var(--cp-border)' }}>
                  <div className="mx-auto mb-3 h-1 w-10 rounded-full" style={{ background: 'var(--cp-border)' }} />
                  <div className="flex flex-col gap-1 pb-[env(safe-area-inset-bottom)]">
                    <MenuAction
                      icon={<ShieldCheck size={16} />}
                      label={t('aiCenter.providers.updateKey', 'Update Key')}
                      onClick={() => {
                        setActionsOpen(false)
                        setShowKeyDialog(true)
                        setKeyError(null)
                        setKeyFeedback(null)
                      }}
                    />
                    <MenuAction
                      icon={<RefreshCw size={16} className={refreshingModels ? 'animate-spin' : ''} />}
                      label={refreshingModels ? t('aiCenter.providers.refreshingModels', 'Refreshing Models') : t('aiCenter.providers.refreshModels', 'Refresh Models')}
                      onClick={() => {
                        setActionsOpen(false)
                        void handleRefreshModels()
                      }}
                      disabled={refreshingModels}
                    />
                    <MenuAction
                      icon={<Trash2 size={16} />}
                      label={t('aiCenter.providers.delete', 'Delete')}
                      onClick={() => {
                        setActionsOpen(false)
                        setConfirmDelete(true)
                        setDeleteError(null)
                      }}
                      danger
                    />
                  </div>
                </div>
              </div>
            ) : (
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
            )
          )}
        </div>
      </div>
      <LongField
        value={`${config.provider_instance_name} / ${config.provider_runtime_type} / ${config.provider_origin}`}
        className={isMobile ? 'text-left text-xs' : '-mt-2 text-xs'}
        tone="muted"
        copyable
      />

      <div className="flex min-h-10 flex-wrap items-center gap-1 rounded-xl p-1" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
        {([
          ['overview', t('aiCenter.providers.overview', 'Overview')],
          ['routing', t('aiCenter.providers.routing', 'Routing')],
          ['inventory', t('aiCenter.providers.inventory', 'Inventory')],
        ] as Array<[ProviderDetailSection, string]>).map(([section, label]) => (
          <button
            key={section}
            type="button"
            onClick={() => setActiveSection(section)}
            className="min-h-8 rounded-lg px-3 text-xs font-medium"
            style={{
              background: activeSection === section ? 'var(--cp-surface-2)' : 'transparent',
              color: activeSection === section ? 'var(--cp-text)' : 'var(--cp-muted)',
              border: activeSection === section ? '1px solid var(--cp-border)' : '1px solid transparent',
            }}
          >
            {label}
          </button>
        ))}
      </div>

      {activeSection === 'overview' && (
      <>
      <div className="md:hidden">
        <MetricCarousel metrics={providerMetrics} />
      </div>
      <div className="hidden grid-cols-4 gap-3 md:grid">
        {desktopMetrics.map((metric) => (
          <Metric key={metric.label} label={metric.label} value={metric.value} detail={metric.detail} />
        ))}
      </div>

      <div
        className="rounded-xl p-4 flex flex-col gap-3"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <Row label={t('aiCenter.providers.driver', 'Driver')} value={config.provider_driver} copyValue={config.provider_driver} />
        <Row label={t('aiCenter.providers.routingWeight', 'Routing Weight')} value={`${formatWeight(routingWeight)} / ${routingWeightLabel}`} />
        <Row label={t('aiCenter.providers.runtimeType', 'Runtime Type')} value={config.provider_runtime_type} copyValue={config.provider_runtime_type} />
        <Row label={t('aiCenter.providers.endpoint', 'Endpoint')} value={config.endpoint || t('aiCenter.providers.default', 'Default')} copyValue={config.endpoint} expandable />
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
      </>
      )}

      {activeSection === 'routing' && (
      <>
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
      </>
      )}

      {activeSection === 'inventory' && (
        <div className="flex flex-col gap-3">
          <InventoryToolbar
            t={t}
            searchQuery={searchQuery}
            onSearchChange={setSearchQuery}
            filters={filters}
            filterOptions={filterOptions}
            resultCount={groupedModels.reduce((count, group) => count + 1 + group.variants.length, 0)}
            onFilterChange={setFilter}
            filtersOpen={inventoryFiltersOpen}
            onToggleFilters={() => setInventoryFiltersOpen((value) => !value)}
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

function MetricCarousel({ metrics }: { metrics: ProviderMetric[] }) {
  const [activeIndex, setActiveIndex] = useState(0)
  if (metrics.length === 0) return null
  if (metrics.length === 1) {
    const metric = metrics[0]
    return <MobileMetricCard metric={metric} index={0} total={1} />
  }
  const previousIndex = () => setActiveIndex((current) => (current - 1 + metrics.length) % metrics.length)
  const nextIndex = () => setActiveIndex((current) => (current + 1) % metrics.length)
  const previous = (activeIndex - 1 + metrics.length) % metrics.length
  const next = (activeIndex + 1) % metrics.length
  const visible = [
    { index: previous, position: 'left' as const },
    { index: activeIndex, position: 'center' as const },
    { index: next, position: 'right' as const },
  ]

  return (
    <div className="overflow-hidden">
      <div className="relative h-[204px]">
        {visible.map(({ index, position }) => (
          <div
            key={`${metrics[index].label}-${position}`}
            className="absolute top-0 h-full w-[86%] max-w-[360px] transition-all duration-200"
            style={{
              left: position === 'left' ? '0%' : position === 'center' ? '50%' : '100%',
              transform: position === 'center'
                ? 'translateX(-50%)'
                : position === 'left'
                  ? 'translateX(-86%) scale(0.9)'
                  : 'translateX(-14%) scale(0.9)',
              opacity: position === 'center' ? 1 : 0.72,
              zIndex: position === 'center' ? 2 : 1,
            }}
          >
            <MobileMetricCard
              metric={metrics[index]}
              index={index}
              total={metrics.length}
              preview={position !== 'center'}
              onPreviewClick={position === 'left' ? previousIndex : position === 'right' ? nextIndex : undefined}
            />
          </div>
        ))}
      </div>
      <div className="mt-2 flex min-w-0 items-center justify-center gap-1.5">
        {metrics.map((metric, index) => (
          <button
            key={metric.label}
            type="button"
            onClick={() => setActiveIndex(index)}
            className="h-2.5 rounded-full transition-all"
            style={{
              width: index === activeIndex ? 22 : 10,
              background: index === activeIndex ? 'var(--cp-accent)' : 'var(--cp-border)',
            }}
            aria-label={metric.label}
          />
        ))}
      </div>
    </div>
  )
}

function MobileMetricCard({
  metric,
  index,
  total,
  preview = false,
  onPreviewClick,
}: {
  metric: ProviderMetric
  index: number
  total: number
  preview?: boolean
  onPreviewClick?: () => void
}) {
  const toneColor = metric.tone === 'ok'
    ? 'var(--cp-success)'
    : metric.tone === 'warning'
      ? 'var(--cp-warning)'
      : metric.tone === 'accent'
        ? 'var(--cp-accent)'
        : 'var(--cp-text)'

  return (
    <button
      type="button"
      onClick={preview ? onPreviewClick : undefined}
      className="h-[196px] w-full rounded-xl p-4 text-left"
      style={{
        background: preview ? 'var(--cp-surface)' : 'linear-gradient(180deg, color-mix(in oklch, var(--cp-accent), transparent 88%), var(--cp-bg))',
        border: `1px solid ${metric.tone === 'warning' ? 'var(--cp-warning)' : 'var(--cp-border)'}`,
        boxShadow: preview ? 'none' : '0 8px 22px color-mix(in oklch, var(--cp-accent), transparent 88%)',
      }}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-xs font-medium uppercase" style={{ color: 'var(--cp-muted)' }}>{metric.label}</div>
          <div className="mt-3 line-clamp-3 break-words text-xl font-semibold leading-tight" style={{ color: toneColor }}>
            {metric.value}
          </div>
        </div>
        <div className="shrink-0 rounded-full px-2 py-1 text-[11px] tabular-nums" style={{ color: 'var(--cp-muted)', background: 'var(--cp-surface)' }}>
          {index + 1}/{total}
        </div>
      </div>
      <div className="mt-4 line-clamp-3 min-h-14 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
        {metric.detail ?? ''}
      </div>
    </button>
  )
}

function InventoryToolbar({
  t,
  searchQuery,
  onSearchChange,
  filters,
  filterOptions,
  resultCount,
  onFilterChange,
  filtersOpen,
  onToggleFilters,
}: {
  t: TFn
  searchQuery: string
  onSearchChange: (value: string) => void
  filters: InventoryFilters
  filterOptions: Record<FilterKey, string[]>
  resultCount: number
  onFilterChange: (key: FilterKey, value: MultiFilter) => void
  filtersOpen: boolean
  onToggleFilters: () => void
}) {
  const activeFilterCount = Object.values(filters).reduce((count, filter) => count + filter.selected.length + (filter.query.trim() ? 1 : 0), 0)
  return (
    <div className="rounded-xl p-3 flex flex-col gap-3" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <label className="relative block">
        <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: 'var(--cp-muted)' }} />
        <input
          value={searchQuery}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={t('aiCenter.providers.searchModels', 'Search models')}
          className="w-full rounded-lg py-2 pl-9 pr-12 text-sm"
          style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        />
        <button
          type="button"
          onClick={onToggleFilters}
          className="absolute right-1.5 top-1/2 flex h-7 min-w-7 -translate-y-1/2 items-center justify-center gap-1 rounded-md px-1.5 text-xs"
          style={{
            color: filtersOpen || activeFilterCount > 0 ? 'var(--cp-accent)' : 'var(--cp-muted)',
            background: filtersOpen ? 'var(--cp-surface)' : 'transparent',
          }}
          aria-label={t('aiCenter.providers.filters', 'Filters')}
        >
          <Filter size={14} />
          <span>{resultCount}</span>
        </button>
      </label>
      {filtersOpen && <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-6 gap-2">
        <MultiSelectFilter label={t('aiCenter.providers.apiType', 'API Type')} value={filters.apiType} options={filterOptions.apiType} onChange={(value) => onFilterChange('apiType', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.logicalMount', 'Logical Mount')} value={filters.logicalMount} options={filterOptions.logicalMount} onChange={(value) => onFilterChange('logicalMount', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.health', 'Health')} value={filters.health} options={filterOptions.health} onChange={(value) => onFilterChange('health', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.costClass', 'Cost Class')} value={filters.costClass} options={filterOptions.costClass} onChange={(value) => onFilterChange('costClass', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.latencyClass', 'Latency Class')} value={filters.latencyClass} options={filterOptions.latencyClass} onChange={(value) => onFilterChange('latencyClass', value)} />
        <MultiSelectFilter label={t('aiCenter.providers.tier', 'Tier')} value={filters.tier} options={filterOptions.tier} onChange={(value) => onFilterChange('tier', value)} />
      </div>}
    </div>
  )
}

function MultiSelectFilter({ label, value, options, onChange }: { label: string; value: MultiFilter; options: string[]; onChange: (value: MultiFilter) => void }) {
  const { t } = useI18n()
  const selectedCount = value.selected.length
  const [open, setOpen] = useState(false)
  const [showAllOptions, setShowAllOptions] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const visibleOptions = showAllOptions
    ? options
    : Array.from(new Set([...options.slice(0, 6), ...value.selected]))
  const hiddenOptionCount = Math.max(0, options.length - visibleOptions.length)
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
          {visibleOptions.map((option) => (
            <label key={option} className="flex min-h-7 items-center gap-2 rounded px-2 py-1 text-xs" style={{ color: 'var(--cp-text)' }}>
              <input
                type="checkbox"
                checked={value.selected.includes(option)}
                onChange={() => toggleOption(option)}
              />
              <span className="truncate" title={option}>{option}</span>
            </label>
          ))}
          {hiddenOptionCount > 0 && (
            <button
              type="button"
              onClick={() => setShowAllOptions(true)}
              className="rounded px-2 py-1 text-left text-xs"
              style={{ color: 'var(--cp-accent)' }}
            >
              {t('aiCenter.providers.showMoreOptions', 'Show more')} ({hiddenOptionCount})
            </button>
          )}
          {showAllOptions && options.length > 6 && (
            <button
              type="button"
              onClick={() => setShowAllOptions(false)}
              className="rounded px-2 py-1 text-left text-xs"
              style={{ color: 'var(--cp-accent)' }}
            >
              {t('aiCenter.providers.showLessOptions', 'Show less')}
            </button>
          )}
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
          <LongField value={model.provider_model_id} className="text-sm font-medium" />
          <LongField
            value={`${groupLabel ? `${groupLabel} / ` : ''}${model.logical_mounts.join(', ')} / ${model.api_types.join(', ')}`}
            className="text-xs"
            tone="muted"
            copyable={false}
            expandable
          />
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
    <div className="fixed inset-0 z-50 flex items-end justify-center md:items-center">
      <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.4)' }} onClick={onCancel} />
      <div
        className="relative flex max-h-[calc(100dvh-1rem)] w-full flex-col overflow-hidden rounded-t-xl shadow-lg md:mx-4 md:max-w-md md:rounded-xl"
        style={{ background: 'var(--cp-surface)' }}
      >
        <div className="overflow-y-auto p-6 pb-4">
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
        </div>
        <div
          className="sticky bottom-0 flex flex-col-reverse gap-2 p-4 sm:flex-row sm:justify-end sm:gap-3"
          style={{
            background: 'var(--cp-surface)',
            borderTop: '1px solid var(--cp-border)',
            paddingBottom: 'calc(1rem + env(safe-area-inset-bottom))',
          }}
        >
          <button type="button" onClick={onCancel} disabled={loading} className="min-h-11 px-4 py-2 rounded-lg text-sm disabled:opacity-60" style={{ color: 'var(--cp-muted)' }}>
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={loading}
            className="inline-flex min-h-11 items-center justify-center gap-2 px-4 py-2 rounded-lg text-sm font-medium disabled:opacity-60"
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
      <LongField value={value} copyable={false} className="align-bottom" />
    </div>
  )
}

function Row({
  label,
  value,
  copyValue,
  expandable,
}: {
  label: string
  value: ReactNode
  copyValue?: string
  expandable?: boolean
}) {
  return (
    <div className="grid grid-cols-[minmax(7rem,0.45fr)_minmax(0,1fr)] items-center gap-4 text-sm">
      <span className="truncate" title={label} style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="min-w-0 justify-self-end text-right font-medium" style={{ color: 'var(--cp-text)' }}>
        {typeof value === 'string' ? (
          <LongField value={copyValue ?? value} title={value} expandable={expandable} />
        ) : value}
      </span>
    </div>
  )
}

function MenuAction({ icon, label, onClick, danger, disabled }: { icon: ReactNode; label: string; onClick: () => void; danger?: boolean; disabled?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex min-h-11 w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm font-medium transition-opacity hover:opacity-80 disabled:opacity-50 md:min-h-0 md:text-xs"
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
    model.health.quota_state,
  ].join(' ').toLowerCase()
  return query.split(/\s+/).some((item) => item && haystack.includes(item))
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
