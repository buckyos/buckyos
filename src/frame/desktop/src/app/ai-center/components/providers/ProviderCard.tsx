import { Network, Zap, Cpu, Globe, Cloud, Server } from 'lucide-react'
import { StatusBadge } from '../shared/StatusBadge'
import { LongField } from '../shared/LongField'
import { isManagedSnProvider, type AuthStatus, type ProviderView } from '../../../../api/aicc_mgr'
import { useI18n } from '../../../../i18n/provider'

const providerIcons: Record<string, typeof Network> = {
  sn_router: Network,
  openai: Zap,
  anthropic: Cpu,
  google: Globe,
  openrouter: Cloud,
  custom: Server,
}

function authStatusToVariant(s: AuthStatus): 'ok' | 'warning' | 'error' | 'unknown' {
  switch (s) {
    case 'ok': return 'ok'
    case 'expired': return 'warning'
    case 'invalid': return 'error'
    default: return 'unknown'
  }
}

interface ProviderCardProps {
  provider: ProviderView
  selected: boolean
  onClick: () => void
}

export function ProviderCard({ provider, selected, onClick }: ProviderCardProps) {
  const { t } = useI18n()
  const Icon = providerIcons[provider.config.provider_type] ?? Server
  const modelCount = provider.status.discovered_models.length
  const degradedCount = provider.status.discovered_models.filter((m) => m.health.status !== 'available').length
  const managedSn = isManagedSnProvider(provider)
  const statusVariant = provider.status.model_sync_status === 'failed'
    ? 'warning'
    : authStatusToVariant(provider.status.auth_status)

  return (
    <button
      type="button"
      onClick={onClick}
      className="flex min-h-16 w-full items-start gap-3 rounded-lg px-3 py-3 text-left transition-colors"
      style={{
        background: selected ? 'var(--cp-surface-2)' : 'transparent',
      }}
    >
      <Icon size={18} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-muted)' }} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <LongField value={provider.config.name} className="text-sm font-medium" copyable={false} />
        <LongField
          value={`${provider.config.provider_instance_name}/${provider.config.provider_driver}`}
          className="text-[11px]"
          tone="muted"
          copyable={false}
        />
        {managedSn && (
          <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.providers.systemManaged', 'System managed')}
          </span>
        )}
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1">
        <StatusBadge
          status={statusVariant}
          label={provider.status.model_sync_status === 'failed'
            ? t('aiCenter.providers.syncFailedShort', 'Sync failed')
            : undefined}
        />
        <span className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
          {modelCount}{degradedCount > 0 ? `/${degradedCount}` : ''}
        </span>
      </div>
    </button>
  )
}
