import { useCallback, useEffect, useState } from 'react'

import { fetchContainerOverview, runContainerAction } from '@/api'
import ContainerOverviewPanel from '../components/ContainerOverviewPanel'
import Icon from '../icons'

const POLL_INTERVAL_MS = 7000

const toErrorText = (value: unknown) => {
  if (value instanceof Error) {
    return value.message
  }
  if (typeof value === 'string') {
    return value
  }
  return 'Container telemetry request failed.'
}

const ContainerPage = () => {
  const [overview, setOverview] = useState<ContainerOverview | null>(null)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)
  const [actionLoadingId, setActionLoadingId] = useState<string | null>(null)

  const load = useCallback(async (silent: boolean) => {
    if (silent) {
      setRefreshing(true)
    } else {
      setLoading(true)
    }

    const { data, error } = await fetchContainerOverview()
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

  const handleContainerAction = useCallback(async (id: string, action: 'start' | 'stop' | 'restart') => {
    setActionLoadingId(id)
    const { error } = await runContainerAction(id, action)
    if (error) {
      setErrorMessage(toErrorText(error))
    } else {
      setErrorMessage(null)
      await load(true)
    }
    setActionLoadingId(null)
  }, [load])

  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">Container Manager</h1>
            <p className="text-sm text-[var(--cp-muted)]">Manage local Docker runtime and container states.</p>
          </div>
          <div className="flex items-center gap-2">
            <span className="cp-pill bg-[var(--cp-surface-muted)] text-[var(--cp-muted)]">
              {refreshing ? 'Refreshing' : 'Live'}
            </span>
            <span className="cp-pill bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
              <Icon name="container" className="mr-1 inline size-3.5" /> Docker
            </span>
          </div>
        </div>
      </header>

      <section className="cp-panel p-6">
        <ContainerOverviewPanel
          overview={overview}
          loading={loading}
          errorMessage={errorMessage}
          actionLoadingId={actionLoadingId}
          onContainerAction={handleContainerAction}
        />
      </section>
    </div>
  )
}

export default ContainerPage
