import { useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { useProviders } from './hooks/use-mock-store'
import { ProviderList } from './components/providers/ProviderList'
import { ProviderDetailPanel } from './components/providers/ProviderDetailPanel'
import { EmptyState } from './components/shared/EmptyState'
import { Plug } from 'lucide-react'
import type { AICenterPage } from './components/layout/Sidebar'

interface ProvidersPageProps {
  navigate: (page: AICenterPage) => void
}

export function ProvidersPage({ navigate }: ProvidersPageProps) {
  const { t } = useI18n()
  const providers = useProviders()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [selectedId, setSelectedId] = useState<string | null>(
    providers.length > 0 ? providers[0].config.id : null,
  )
  // Mobile: detail view shown when a provider is selected and user tapped it
  const [showMobileDetail, setShowMobileDetail] = useState(false)

  const selectedProvider = providers.find((p) => p.config.id === selectedId)

  if (providers.length === 0) {
    return (
      <EmptyState
        icon={<Plug size={48} />}
        title={t('aiCenter.providers.noProviders', 'No providers configured')}
        action={{
          label: t('aiCenter.providers.addProvider', 'Add Provider'),
          onClick: () => navigate('providers/add'),
        }}
      />
    )
  }

  if (isMobile) {
    if (showMobileDetail && selectedProvider) {
      return (
        <div>
          <button
            type="button"
            onClick={() => setShowMobileDetail(false)}
            className="text-sm mb-3"
            style={{ color: 'var(--cp-accent)' }}
          >
            {t('common.back', 'Back')}
          </button>
          <ProviderDetailPanel
            provider={selectedProvider}
            onDeleted={() => {
              setShowMobileDetail(false)
              setSelectedId(providers.length > 1 ? providers[0].config.id : null)
            }}
          />
        </div>
      )
    }

    return (
      <ProviderList
        providers={providers}
        selectedId={selectedId}
        onSelect={(id) => {
          setSelectedId(id)
          setShowMobileDetail(true)
        }}
        onAdd={() => navigate('providers/add')}
      />
    )
  }

  // Desktop: split view
  return (
    <div className="flex gap-6 -mx-8 -my-6 h-full">
      <div
        className="w-80 shrink-0 py-4 px-4 overflow-y-auto"
        style={{ borderRight: '1px solid var(--cp-border)' }}
      >
        <ProviderList
          providers={providers}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onAdd={() => navigate('providers/add')}
        />
      </div>
      <div className="flex-1 py-6 px-6 overflow-y-auto">
        {selectedProvider ? (
          <ProviderDetailPanel
            provider={selectedProvider}
            onDeleted={() => {
              const remaining = providers.filter((p) => p.config.id !== selectedId)
              setSelectedId(remaining.length > 0 ? remaining[0].config.id : null)
            }}
          />
        ) : (
          <div className="flex items-center justify-center h-full text-sm" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.providers.detail', 'Provider Detail')}
          </div>
        )}
      </div>
    </div>
  )
}
