import type { ReactNode } from 'react'
import { Plus } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { ProviderCard } from './ProviderCard'
import type { ProviderView } from '../../../../api/aicc_mgr'

interface ProviderListProps {
  providers: ProviderView[]
  selectedId: string | null
  onSelect: (id: string) => void
  onAdd: () => void
  footer?: ReactNode
}

export function ProviderList({ providers, selectedId, onSelect, onAdd, footer }: ProviderListProps) {
  const { t } = useI18n()

  return (
    <div className="flex flex-col gap-1">
      {providers.map((p) => (
        <ProviderCard
          key={p.config.id}
          provider={p}
          selected={selectedId === p.config.id}
          onClick={() => onSelect(p.config.id)}
        />
      ))}
      <button
        type="button"
        onClick={onAdd}
        className="mt-2 flex min-h-11 items-center gap-2 rounded-lg px-3 py-2.5 text-sm transition-opacity hover:opacity-70"
        style={{ color: 'var(--cp-accent)' }}
      >
        <Plus size={16} />
        {t('aiCenter.providers.addProvider', 'Add Provider')}
      </button>
      {footer && (
        <div className="mt-2 border-t pt-2" style={{ borderColor: 'var(--cp-border)' }}>
          {footer}
        </div>
      )}
    </div>
  )
}
