import { useCallback, useMemo, useState } from 'react'
import { Pencil, Plus, Trash2 } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { EmptyView } from '../../components/empty-state/StateView'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { TableFilterForm } from '../../components/forms/TableFilterForm'
import { StatusBadge } from '../../components/status/StatusBadge'
import { buildOpsProviderPreview, buildPublishedProviderJson, filterProviders, getProviderModelCount, getProviderWarnings } from '../../datamodel/selectors'
import type { ProviderRecord } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { paginate, useShellContext } from '../pageUtils'

const pageSize = 8

export function ProvidersPage() {
  const { serviceRole } = useProviderMetadataStore()
  return serviceRole === 'ops' ? <OpsProvidersPage /> : <TechProvidersPage />
}

function TechProvidersPage() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const { workspace, viewMode, removeProvider } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [search, setSearch] = useState('')
  const [page, setPage] = useState(1)
  const providers = useMemo(() => filterProviders(data, search), [data, search])
  const paged = paginate(providers, page, pageSize)

  const columns = useMemo<Array<DataTableColumn<ProviderRecord>>>(() => [
    {
      key: 'provider',
      title: t('table.provider', 'Provider'),
      render: (provider) => (
        <div>
          <div className="font-semibold">{provider.name}</div>
          <div className="text-xs text-[color:var(--cp-muted)]">{provider.provider_key}</div>
        </div>
      ),
    },
    { key: 'driver', title: t('table.driver', 'Driver'), render: (provider) => provider.provider_driver },
    { key: 'protocol', title: t('providers.protocol', 'Protocol family'), render: (provider) => provider.protocol_family ?? '-' },
    { key: 'kind', title: t('table.kind', 'Kind'), render: (provider) => provider.provider_kind },
    { key: 'base', title: t('table.baseUrl', 'Base URL'), render: (provider) => provider.base_url ?? '-' },
    { key: 'models', title: t('table.modelCount', 'Models'), render: (provider) => getProviderModelCount(data, provider.provider_key) },
    {
      key: 'enabled',
      title: t('table.enabled', 'Enabled'),
      render: (provider) => <StatusBadge tone={provider.enabled ? 'success' : 'danger'}>{provider.enabled ? t('yes', 'Yes') : t('no', 'No')}</StatusBadge>,
    },
    {
      key: 'warning',
      title: t('table.warning', 'Warning'),
      render: (provider) => {
        const count = getProviderWarnings(data, provider.provider_key).length
        return count ? <StatusBadge tone="warning">{count}</StatusBadge> : <StatusBadge>{0}</StatusBadge>
      },
    },
  ], [data, t])

  const handleSelect = useCallback((provider: ProviderRecord) => {
    setInspector({
      title: provider.name,
      subtitle: provider.base_url ?? provider.provider_driver,
      status: provider.provider_kind,
      json: buildPublishedProviderJson(data, provider),
    })
  }, [data, navigate, setInspector])

  return (
    <div className="space-y-4" data-testid="providers-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('providers.title', 'Providers')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')}</p>
        </div>
        <div className="mobile-readonly-hide flex flex-wrap items-center justify-end gap-2">
          <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={() => navigate('/providers/wizard')}>
            <Plus size={16} />
            {t('providers.create', 'Create provider')}
          </button>
          <StatusBadge tone="accent">{providers.length}</StatusBadge>
        </div>
      </header>
      <div className="shell-card p-4">
        <TableFilterForm
          providers={data.providers.map((provider) => ({ key: provider.provider_key, label: provider.name }))}
          onChange={(value) => {
            setSearch(value.search)
            setPage(1)
          }}
        />
      </div>
      {paged.rows.length ? (
        <DataTable
          actions={viewMode === 'edit' ? (provider) => <div className="flex gap-1"><button aria-label={`Edit ${provider.name}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => navigate(`/providers/wizard?provider=${encodeURIComponent(provider.provider_key)}`)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${provider.name}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeProvider(provider.provider_key)} type="button"><Trash2 size={14} /></button></div> : undefined}
          columns={columns}
          onSelect={handleSelect}
          rowKey={(provider) => provider.provider_key}
          rows={paged.rows}
        />
      ) : (
        <EmptyView />
      )}
      <Pager page={paged.page} pages={paged.totalPages} setPage={setPage} />
    </div>
  )
}

function OpsProvidersPage() {
  const { t } = useI18n()
  const { workspace, viewMode } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [search, setSearch] = useState('')
  const [page, setPage] = useState(1)
  const providers = useMemo(() => filterProviders(data, search), [data, search])
  const paged = paginate(providers, page, pageSize)

  const columns = useMemo<Array<DataTableColumn<ProviderRecord>>>(() => [
    {
      key: 'provider',
      title: t('table.provider', 'Provider'),
      render: (provider) => (
        <div>
          <div className="font-semibold">{provider.name}</div>
          <div className="text-xs text-[color:var(--cp-muted)]">{provider.provider_key}</div>
        </div>
      ),
    },
    { key: 'driver', title: t('table.driver', 'Driver'), render: (provider) => provider.provider_driver },
    { key: 'protocol', title: t('providers.protocol', 'Protocol family'), render: (provider) => provider.protocol_family ?? '-' },
    { key: 'base', title: t('table.baseUrl', 'Base URL'), render: (provider) => provider.base_url ?? '-' },
    {
      key: 'visible',
      title: t('ops.clientVisible', 'Client visible'),
      render: (provider) => {
        const preview = buildOpsProviderPreview(data, provider)
        return <StatusBadge tone={preview.visible ? 'success' : 'danger'}>{preview.visible ? t('yes', 'Yes') : t('no', 'No')}</StatusBadge>
      },
    },
    {
      key: 'recommendation',
      title: t('ops.recommendation', 'Recommendation'),
      render: (provider) => buildOpsProviderPreview(data, provider).recommendation_level,
    },
    {
      key: 'priority',
      title: t('ops.displayPriority', 'Display priority'),
      render: (provider) => buildOpsProviderPreview(data, provider).display_priority,
    },
  ], [data, t])

  const handleSelect = useCallback((provider: ProviderRecord) => {
    setInspector({
      title: provider.name,
      subtitle: t('ops.providerReadonlyHint', 'Providers are read-only for operations parameters'),
      status: provider.enabled ? 'visible' : 'disabled',
      json: {
        technical: provider,
        published_json: buildPublishedProviderJson(data, provider),
        client_preview: buildOpsProviderPreview(data, provider),
      },
    })
  }, [data, setInspector, t])

  return (
    <div className="space-y-4" data-testid="ops-providers-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('ops.providersTitle', 'Operations Providers')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')} / {t('ops.providerReadonlyHint', 'Providers are read-only for operations parameters')}</p>
        </div>
        <StatusBadge tone="accent">{providers.length}</StatusBadge>
      </header>

      <div className="shell-card p-4">
        <TableFilterForm
          providers={data.providers.map((provider) => ({ key: provider.provider_key, label: provider.name }))}
          onChange={(value) => {
            setSearch(value.search)
            setPage(1)
          }}
        />
      </div>
      {paged.rows.length ? (
        <DataTable columns={columns} onSelect={handleSelect} rowKey={(provider) => provider.provider_key} rows={paged.rows} />
      ) : (
        <EmptyView />
      )}
      <Pager page={paged.page} pages={paged.totalPages} setPage={setPage} />
    </div>
  )
}

function Pager({ page, pages, setPage }: { page: number; pages: number; setPage: (page: number) => void }) {
  const { t } = useI18n()
  return (
    <div className="flex items-center justify-end gap-2 text-sm text-[color:var(--cp-muted)]">
      <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={page <= 1} onClick={() => setPage(page - 1)}>
        -
      </button>
      {t('pager.page', 'Page {{page}} of {{pages}}', { page, pages })}
      <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={page >= pages} onClick={() => setPage(page + 1)}>
        +
      </button>
    </div>
  )
}
