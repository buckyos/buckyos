import { Plus } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { ProviderCard } from './ProviderCard'
import type { ProviderView } from '../../mock/types'

interface ProviderListProps {
  providers: ProviderView[]
  selectedId: string | null
  onSelect: (id: string) => void
  onAdd: () => void
}

export function ProviderList({ providers, selectedId, onSelect, onAdd }: ProviderListProps) {
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
        className="flex items-center gap-2 px-3 py-2.5 rounded-lg text-sm mt-2 transition-opacity hover:opacity-70"
        style={{ color: 'var(--cp-accent)' }}
      >
        <Plus size={16} />
        {t('aiCenter.providers.addProvider', 'Add Provider')}
      </button>
    </div>
  )
}
