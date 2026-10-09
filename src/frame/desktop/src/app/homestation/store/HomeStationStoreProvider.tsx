import { Loader2 } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { ErrorState } from '../ui/primitives'
import { HomeStationStoreContext, useHomeStationStore, useStoreSelector } from './context'
import { createPageStore } from './createStore'
import type { HomeStationStore } from './types'

const selectStatus = (store: HomeStationStore) => store.peekStatus()

function StoreGate({ children }: { children: ReactNode }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const status = useStoreSelector(selectStatus)
  const [attempt, setAttempt] = useState(0)
  useEffect(() => store.connect(), [store, attempt])
  if (status === 'ready') return children
  return (
    <div className="hs-root flex h-full w-full items-center justify-center" style={{ background: 'var(--cp-bg)' }} data-testid="hs-store-status" data-status={status}>
      {status === 'error' ? (
        <ErrorState onRetry={() => setAttempt(value => value + 1)} />
      ) : (
        <Loader2 size={22} className="animate-spin" style={{ color: 'var(--cp-muted)' }} aria-label={t('homestation.state.loading', 'Loading…')} />
      )}
    </div>
  )
}

export function HomeStationStoreProvider({ children }: { children: ReactNode }) {
  const [store] = useState(createPageStore)
  return (
    <HomeStationStoreContext.Provider value={store}>
      <StoreGate>{children}</StoreGate>
    </HomeStationStoreContext.Provider>
  )
}
