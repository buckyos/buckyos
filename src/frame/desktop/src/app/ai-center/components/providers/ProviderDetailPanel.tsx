import { useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useMockStore } from '../../hooks/use-mock-store'
import { StatusBadge } from '../shared/StatusBadge'
import { ConfirmDialog } from '../shared/ConfirmDialog'
import type { ProviderView, AuthStatus } from '../../mock/types'

function authStatusLabel(s: AuthStatus, t: (k: string, f: string) => string): string {
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

interface ProviderDetailPanelProps {
  provider: ProviderView
  onDeleted: () => void
}

export function ProviderDetailPanel({ provider, onDeleted }: ProviderDetailPanelProps) {
  const { t } = useI18n()
  const store = useMockStore()
  const [showModels, setShowModels] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)

  const { config, status, account } = provider
  const models = status.discovered_models

  const handleDelete = () => {
    store.deleteProvider(config.id)
    onDeleted()
  }

  const balanceDisplay = account.balance_supported && account.balance_value != null
    ? `${account.balance_unit === 'usd' ? '$' : ''}${account.balance_value}${account.balance_unit === 'credit' ? ' Credit' : ''}`
    : '—'

  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
        {config.name}
      </h2>

      {/* Info rows */}
      <div
        className="rounded-xl p-4 flex flex-col gap-3"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <Row label={t('aiCenter.providers.type', 'Type')} value={config.provider_type} />
        <Row
          label={t('aiCenter.providers.endpoint', 'Endpoint')}
          value={config.endpoint || t('aiCenter.providers.default', 'Default')}
        />
        <Row
          label={t('aiCenter.providers.auth', 'Authentication')}
          value={
            <span className="inline-flex items-center gap-2">
              {config.auth_mode ?? '—'}
              <StatusBadge
                status={authStatusVariant(status.auth_status)}
                label={authStatusLabel(status.auth_status, t)}
              />
            </span>
          }
        />
        <Row
          label={t('aiCenter.providers.models', 'Models')}
          value={
            <button
              type="button"
              onClick={() => setShowModels(!showModels)}
              className="inline-flex items-center gap-1 text-sm"
              style={{ color: 'var(--cp-accent)' }}
            >
              {models.length}
              {showModels ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            </button>
          }
        />
        <Row
          label={t('aiCenter.providers.balance', 'Balance')}
          value={balanceDisplay}
        />
      </div>

      {/* Models list */}
      {showModels && models.length > 0 && (
        <div
          className="rounded-lg p-3 max-h-48 overflow-y-auto"
          style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
        >
          {models.map((m) => (
            <div key={m} className="text-xs py-0.5 font-mono" style={{ color: 'var(--cp-text)' }}>
              {m}
            </div>
          ))}
        </div>
      )}

      {/* Actions */}
      <div className="flex flex-wrap gap-2">
        <ActionButton
          label={t('aiCenter.providers.updateKey', 'Update Key')}
          onClick={() => console.log('update key')}
        />
        <ActionButton
          label={t('aiCenter.providers.refreshModels', 'Refresh Models')}
          onClick={() => store.refreshProviderModels(config.id)}
        />
        <ActionButton
          label={t('aiCenter.providers.delete', 'Delete')}
          onClick={() => setConfirmDelete(true)}
          danger
        />
      </div>

      <ConfirmDialog
        open={confirmDelete}
        title={t('aiCenter.providers.deleteTitle', 'Delete Provider')}
        message={t('aiCenter.providers.deleteConfirm', 'Are you sure you want to delete this provider? This action cannot be undone.')}
        confirmLabel={t('aiCenter.providers.delete', 'Delete')}
        onConfirm={handleDelete}
        onCancel={() => setConfirmDelete(false)}
      />
    </div>
  )
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex justify-between items-center text-sm">
      <span style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="font-medium" style={{ color: 'var(--cp-text)' }}>{value}</span>
    </div>
  )
}

function ActionButton({ label, onClick, danger }: { label: string; onClick: () => void; danger?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="px-3 py-1.5 rounded-lg text-xs font-medium transition-opacity hover:opacity-80"
      style={{
        border: `1px solid ${danger ? 'var(--cp-danger)' : 'var(--cp-border)'}`,
        color: danger ? 'var(--cp-danger)' : 'var(--cp-text)',
      }}
    >
      {label}
    </button>
  )
}
