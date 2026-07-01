import { useEffect, useRef, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { useProviders, useGlobalRoutingView } from './hooks/use-aicc-store'
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
  const routingView = useGlobalRoutingView()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const isCompactDesktop = useMediaQuery('(min-width: 768px) and (max-width: 1100px)')
  const [selectedId, setSelectedId] = useState<string | null>(
    providers.length > 0 ? providers[0].config.id : null,
  )
  // Mobile: detail view shown when a provider is selected and user tapped it
  const [showMobileDetail, setShowMobileDetail] = useState(false)
  const mobileListRef = useRef<HTMLDivElement | null>(null)
  const mobileListScrollTop = useRef(0)

  const selectedProvider = providers.find((p) => p.config.id === selectedId)

  useEffect(() => {
    if (!isMobile || showMobileDetail) return
    const node = mobileListRef.current
    if (!node) return
    node.scrollTop = mobileListScrollTop.current
  }, [isMobile, showMobileDetail])

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
            className="mb-3 inline-flex min-h-11 items-center rounded-lg px-3 text-sm font-medium"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
          >
            {t('common.back', 'Back')}
          </button>
          <ProviderDetailPanel
            provider={selectedProvider}
            routingWeight={routingView.provider_weights[selectedProvider.config.provider_instance_name] ?? 1}
            onDeleted={() => {
              setShowMobileDetail(false)
              setSelectedId(providers.length > 1 ? providers[0].config.id : null)
            }}
          />
        </div>
      )
    }

    return (
      <div ref={mobileListRef} className="max-h-full overflow-y-auto pb-[calc(1rem+env(safe-area-inset-bottom))]">
        <ProviderList
          providers={providers}
          selectedId={selectedId}
          onSelect={(id) => {
            mobileListScrollTop.current = mobileListRef.current?.scrollTop ?? 0
            setSelectedId(id)
            setShowMobileDetail(true)
          }}
          onAdd={() => navigate('providers/add')}
        />
      </div>
    )
  }

  // Desktop: split view
  return (
    <div className={`${isCompactDesktop ? 'flex flex-col' : 'flex'} gap-6 -mx-8 -my-6 h-full`}>
      <div
        className={isCompactDesktop ? 'max-h-72 shrink-0 overflow-y-auto px-4 py-4' : 'w-80 shrink-0 overflow-y-auto px-4 py-4'}
        style={isCompactDesktop ? { borderBottom: '1px solid var(--cp-border)' } : { borderRight: '1px solid var(--cp-border)' }}
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
            routingWeight={routingView.provider_weights[selectedProvider.config.provider_instance_name] ?? 1}
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
