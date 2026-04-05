import { Network, Zap, Cpu, Globe, Cloud, Server } from 'lucide-react'
import { StatusBadge } from '../shared/StatusBadge'
import type { ProviderView } from '../../mock/types'
import type { AuthStatus } from '../../mock/types'

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

  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-3 w-full px-3 py-3 rounded-lg text-left transition-colors"
      style={{
        background: selected ? 'var(--cp-surface-2)' : 'transparent',
      }}
    >
      <Icon size={18} style={{ color: 'var(--cp-muted)' }} />
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium truncate" style={{ color: 'var(--cp-text)' }}>
          {provider.config.name}
        </div>
      </div>
      <StatusBadge status={authStatusToVariant(provider.status.auth_status)} />
      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {modelCount}
      </span>
    </button>
  )
}
