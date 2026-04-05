import { Package } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useLocalModels } from './hooks/use-mock-store'
import { EmptyState } from './components/shared/EmptyState'

export function ModelsPage() {
  const { t } = useI18n()
  const models = useLocalModels()

  if (models.length === 0) {
    return (
      <EmptyState
        icon={<Package size={48} />}
        title={t('aiCenter.models.empty.title', 'No Local Models Installed')}
        description={t('aiCenter.models.empty.desc', 'Local models need to be installed through the Store.')}
        action={{
          label: t('aiCenter.models.empty.cta', 'Go to Store'),
          onClick: () => console.log('navigate to store'),
        }}
      />
    )
  }

  return (
    <div className="flex flex-col gap-3">
      {models.map((m) => (
        <div
          key={m.id}
          className="flex items-center justify-between rounded-xl p-4"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          <div>
            <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{m.name}</div>
            {m.size_bytes && (
              <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                {(m.size_bytes / (1024 * 1024 * 1024)).toFixed(1)} GB
              </div>
            )}
          </div>
          <span
            className="text-xs px-2 py-1 rounded"
            style={{
              background: m.status === 'ready'
                ? 'color-mix(in oklch, var(--cp-success), transparent 85%)'
                : m.status === 'error'
                  ? 'color-mix(in oklch, var(--cp-danger), transparent 85%)'
                  : 'var(--cp-surface-2)',
              color: m.status === 'ready'
                ? 'var(--cp-success)'
                : m.status === 'error'
                  ? 'var(--cp-danger)'
                  : 'var(--cp-muted)',
            }}
          >
            {m.status}
          </span>
        </div>
      ))}
    </div>
  )
}
