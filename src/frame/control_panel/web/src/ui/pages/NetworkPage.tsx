import { useCallback, useEffect, useState } from 'react'

import { fetchNetworkOverview } from '@/api'
import NetworkOverviewPanel from '../components/NetworkOverviewPanel'
import Icon from '../icons'

const POLL_INTERVAL_MS = 4000

const toErrorText = (value: unknown) => {
  if (value instanceof Error) {
    return value.message
  }
  if (typeof value === 'string') {
    return value
  }
  return 'Network overview request failed.'
}

const NetworkPage = () => {
  const [overview, setOverview] = useState<NetworkOverview | null>(null)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  const load = useCallback(async (silent: boolean) => {
    if (silent) {
      setRefreshing(true)
    } else {
      setLoading(true)
    }

    const { data, error } = await fetchNetworkOverview()
    setOverview(data)
    setErrorMessage(error ? toErrorText(error) : null)
    setLoading(false)
    setRefreshing(false)
  }, [])

  useEffect(() => {
    let cancelled = false
    const safeLoad = async (silent: boolean) => {
      if (cancelled) {
        return
      }
      await load(silent)
    }

    void safeLoad(false)
    const intervalId = window.setInterval(() => {
      void safeLoad(true)
    }, POLL_INTERVAL_MS)

    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [load])

  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">Network Monitor</h1>
            <p className="text-sm text-[var(--cp-muted)]">
              Deep network telemetry with backend timeline and per-interface health counters.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span className="cp-pill bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
              {refreshing ? 'Refreshing' : 'Live'}
            </span>
            <span className="cp-pill bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="network" className="mr-1 inline size-3.5" /> Telemetry
            </span>
          </div>
        </div>
      </header>

      <section className="cp-panel p-6">
        <NetworkOverviewPanel
          overview={overview}
          loading={loading}
          errorMessage={errorMessage}
        />
      </section>
    </div>
  )
}

export default NetworkPage
