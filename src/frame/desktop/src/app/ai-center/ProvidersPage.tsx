import { useEffect, useRef, useState } from 'react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { useProviders, useGlobalRoutingView } from './hooks/use-aicc-store'
import { ProviderList } from './components/providers/ProviderList'
import { ProviderDetailPanel } from './components/providers/ProviderDetailPanel'
import { CloudUpdateCard } from './components/home/CloudUpdateCard'
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
      <div className="mx-auto max-w-3xl">
        <EmptyState
          icon={<Plug size={48} />}
          title={t('aiCenter.providers.noProviders', 'No providers configured')}
          action={{
            label: t('aiCenter.providers.addProvider', 'Add Provider'),
            onClick: () => navigate('providers/add'),
          }}
        />
        <div data-testid="aicenter-provider-global-settings">
          <CloudUpdateCard />
        </div>
      </div>
    )
  }

  if (isMobile) {
    if (showMobileDetail && selectedProvider) {
      return (
        <div>
          <ProviderDetailPanel
            provider={selectedProvider}
            routingWeight={routingView.provider_weights[selectedProvider.config.provider_instance_name] ?? 1}
            onBack={() => setShowMobileDetail(false)}
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
        <div className="mt-6" data-testid="aicenter-provider-global-settings">
          <CloudUpdateCard />
        </div>
      </div>
    )
  }

  // Desktop: split view
  return (
    <div className="-mx-8 -my-6 flex h-full min-h-0 flex-col">
      <div className={`${isCompactDesktop ? 'flex flex-col' : 'flex'} min-h-0 flex-1 gap-6`}>
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
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
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
            <div className="flex h-full items-center justify-center text-sm" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.providers.detail', 'Provider Detail')}
            </div>
          )}
        </div>
      </div>
      <div
        className="shrink-0 px-6 py-3"
        data-testid="aicenter-provider-global-settings"
        style={{ borderTop: '1px solid var(--cp-border)', background: 'var(--cp-surface)' }}
      >
        <CloudUpdateCard />
      </div>
    </div>
  )
}
