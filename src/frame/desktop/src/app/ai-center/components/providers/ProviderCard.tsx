import { Network, Zap, Cpu, Globe, Cloud, Server } from 'lucide-react'
import { StatusBadge } from '../shared/StatusBadge'
import { LongField } from '../shared/LongField'
import type { ProviderView } from '../../../../api/aicc_mgr'
import type { AuthStatus } from '../../../../api/aicc_mgr'

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
  const Icon = providerIcons[provider.config.provider_type] ?? Server
  const modelCount = provider.status.discovered_models.length
  const degradedCount = provider.status.discovered_models.filter((m) => m.health.status !== 'available').length

  return (
    <button
      type="button"
      onClick={onClick}
      className="flex min-h-14 w-full items-center gap-3 rounded-lg px-3 py-3 text-left transition-colors"
      style={{
        background: selected ? 'var(--cp-surface-2)' : 'transparent',
      }}
    >
      <Icon size={18} style={{ color: 'var(--cp-muted)' }} />
      <div className="flex-1 min-w-0">
        <LongField value={provider.config.name} className="text-sm font-medium" copyable={false} />
        <LongField
          value={`${provider.config.provider_instance_name} / ${provider.config.provider_driver}`}
          className="text-[11px]"
          tone="muted"
          copyable={false}
        />
      </div>
      <StatusBadge status={authStatusToVariant(provider.status.auth_status)} />
      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {modelCount}{degradedCount > 0 ? `/${degradedCount}` : ''}
      </span>
    </button>
  )
}
