import { useEffect, useMemo, useRef, useState } from 'react'

import { downloadSystemLogs, fetchLogServices, querySystemLogs, tailSystemLogs } from '@/api'
import Icon from '../icons'

type LogLevelFilter = 'all' | SystemLogLevel

const levelPills: Record<SystemLogLevel, string> = {
  info: 'bg-sky-100 text-sky-700',
  warning: 'bg-amber-100 text-amber-700',
  error: 'bg-rose-100 text-rose-700',
  unknown: 'bg-slate-100 text-slate-600',
}

const levelDots: Record<SystemLogLevel, string> = {
  info: 'bg-sky-500',
  warning: 'bg-amber-500',
  error: 'bg-rose-500',
  unknown: 'bg-slate-400',
}

const rangeOptions = [
  { value: '15m', label: 'Last 15m', ms: 15 * 60 * 1000 },
  { value: '1h', label: 'Last 1h', ms: 60 * 60 * 1000 },
  { value: '24h', label: 'Last 24h', ms: 24 * 60 * 60 * 1000 },
]

const tailIntervals = [
  { value: 3000, label: '3s' },
  { value: 5000, label: '5s' },
  { value: 10000, label: '10s' },
]

const LOG_PAGE_LIMIT = 100
const MAX_LOG_ENTRIES = 1000
const LOG_CONTAINER_HEIGHT = 480
const LOG_ROW_HEIGHT = 56
const LOG_OVERSCAN = 6
const LOG_PADDING = 8

const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

const highlightText = (text: string, query: string) => {
  if (!query) {
    return text
  }
  const escaped = escapeRegExp(query)
  const regex = new RegExp(`(${escaped})`, 'ig')
  const parts = text.split(regex)
  if (parts.length <= 1) {
    return text
  }
  return parts.map((part, index) =>
    index % 2 === 1 ? (
      <span
        key={`${part}-${index}`}
        className="rounded-sm px-0.5 text-[var(--cp-ink)]"
        style={{ backgroundColor: 'rgba(245, 158, 11, 0.25)' }}
      >
        {part}
      </span>
    ) : (
      part
    ),
  )
}

type LogListProps = {
  loading: boolean
  loadingMore: boolean
  hasMore: boolean
  entries: SystemLogEntry[]
  visibleEntries: SystemLogEntry[]
  topSpacerHeight: number
  bottomSpacerHeight: number
  listRef: React.RefObject<HTMLDivElement | null>
  onScroll: (event: React.UIEvent<HTMLDivElement>) => void
  highlightValue: (value: string) => React.ReactNode
  entryKey: (entry: SystemLogEntry) => string
  selectedServicesLength: number
}

const LogList = ({
  loading,
  loadingMore,
  hasMore,
  entries,
  visibleEntries,
  topSpacerHeight,
  bottomSpacerHeight,
  listRef,
  onScroll,
  highlightValue,
  entryKey,
  selectedServicesLength,
}: LogListProps) => {
  if (loading) {
    return (
      <div className="mt-5 rounded-2xl border border-[var(--cp-border)] bg-white">
        <div className="h-[480px] overflow-y-auto p-2">
          {Array.from({ length: 6 }).map((_, index) => (
            <div
              key={`log-skeleton-${index}`}
              className="animate-pulse px-2 py-2"
              style={{ borderBottom: '1px solid rgba(215, 225, 223, 0.25)' }}
            >
              <div className="flex items-center gap-3">
                <div className="size-2 flex-none rounded-full bg-[var(--cp-border)]" />
                <div className="h-3 w-24 rounded-full bg-[var(--cp-surface-muted)]" />
                <div className="h-3 w-16 rounded-full bg-[var(--cp-surface-muted)]" />
              </div>
              <div className="mt-2 h-3 w-2/3 rounded-full bg-[var(--cp-surface-muted)]" />
            </div>
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className="mt-5 rounded-2xl border border-[var(--cp-border)] bg-white">
      {entries.length ? (
        <div className="relative">
          <div className="pointer-events-none absolute left-3 right-3 top-2 z-10 flex justify-center">
            <div className="rounded-full border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-3 py-1 text-[11px] text-[var(--cp-muted)]">
              {loadingMore
                ? 'Loading 100 older logs…'
                : hasMore
                  ? 'Scroll up to load older logs'
                  : 'Oldest logs reached'}
            </div>
          </div>
          <div
            ref={listRef}
            className="h-[480px] overflow-y-auto p-2"
            style={{ overflowAnchor: 'none' }}
            onScroll={onScroll}
          >
            <div style={{ height: topSpacerHeight }} />
            {visibleEntries.map((log) => (
              <div
                key={entryKey(log)}
                className="flex h-[56px] items-center px-2 text-sm text-[var(--cp-muted)]"
                style={{ borderBottom: '1px solid rgba(215, 225, 223, 0.25)' }}
              >
                <span
                  className={`mr-3 inline-flex size-2 flex-none rounded-full ${levelDots[log.level]}`}
                  aria-hidden
                />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2 text-[11px] text-[var(--cp-muted)]">
                    <span className="font-mono text-[var(--cp-ink)]">
                      {log.timestamp || '—'}
                    </span>
                    <span className={`cp-pill uppercase tracking-wide ${levelPills[log.level]}`}>
                      {log.level}
                    </span>
                    <span className="truncate">{highlightValue(log.service)}</span>
                    <span className="truncate">{highlightValue(log.file)}</span>
                  </div>
                  <p className="truncate font-mono text-[13px] text-[var(--cp-ink)]" title={log.raw}>
                    {highlightValue(log.message || log.raw)}
                  </p>
                </div>
              </div>
            ))}
            <div style={{ height: bottomSpacerHeight }} />
          </div>
        </div>
      ) : selectedServicesLength ? (
        <div className="px-4 py-6 text-center text-sm text-[var(--cp-muted)]">
          <p className="text-[var(--cp-ink)]">No logs match your filters.</p>
          <p className="mt-1">Clear filters or refresh the query for new activity.</p>
        </div>
      ) : (
        <div className="px-4 py-6 text-center text-sm text-[var(--cp-muted)]">
          <p className="text-[var(--cp-ink)]">Select a service to view recent logs.</p>
          <p className="mt-1">Choose a service above to load the last 100 lines.</p>
        </div>
      )}
    </div>
  )
}

const SystemLogsPage = () => {
  const [services, setServices] = useState<SystemLogService[]>([])
  const [selectedServices, setSelectedServices] = useState<string[]>([])
  const [servicesReady, setServicesReady] = useState(false)
  const [entries, setEntries] = useState<SystemLogEntry[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [levelFilter, setLevelFilter] = useState<LogLevelFilter>('all')
  const [keyword, setKeyword] = useState('')
  const [debouncedKeyword, setDebouncedKeyword] = useState('')
  const [range, setRange] = useState<'15m' | '1h' | '24h'>('1h')
  const [tailEnabled, setTailEnabled] = useState(false)
  const [tailInterval, setTailInterval] = useState(3000)
  const [downloading, setDownloading] = useState<'filtered' | 'full' | null>(null)
  const [refreshingServices, setRefreshingServices] = useState<string[]>([])
  const tailCursorRef = useRef<string | null>(null)
  const listRef = useRef<HTMLDivElement | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const scrollToBottomOnceRef = useRef(false)
  const pendingScrollRestoreRef = useRef<
    { scrollTop: number; scrollHeight: number } | null
  >(null)
  const loadingMoreRef = useRef(false)
  const lastScrollTopRef = useRef(0)
  const lastLoadAtRef = useRef(0)
  const requestIdRef = useRef(0)
  const selectionKey = useMemo(
    () => selectedServices.slice().sort((a, b) => a.localeCompare(b)).join('|'),
    [selectedServices],
  )
  const refreshTokensRef = useRef<Map<string, string>>(new Map())

  const activeRange = rangeOptions.find((option) => option.value === range)
  const sinceIso = useMemo(() => {
    if (!activeRange) {
      return undefined
    }
    return new Date(Date.now() - activeRange.ms).toISOString()
  }, [activeRange])

  useEffect(() => {
    const handle = window.setTimeout(() => {
      setDebouncedKeyword(keyword.trim())
    }, 300)
    return () => window.clearTimeout(handle)
  }, [keyword])

  useEffect(() => {
    let cancelled = false
    const loadServices = async () => {
      const { data, error } = await fetchLogServices()
      if (cancelled) {
        return
      }
      if (error) {
        // eslint-disable-next-line no-console
        console.warn('Log services API unavailable, using mock data', error)
      }
      const list = data ?? []
      setServices(list)
      if (!servicesReady) {
        setServicesReady(true)
      }
    }
    loadServices()
    return () => {
      cancelled = true
    }
  }, [servicesReady])

  const levelParam = levelFilter === 'all' ? undefined : levelFilter

  const entryKey = (entry: SystemLogEntry) =>
    `${entry.service}-${entry.file}-${entry.line ?? ''}-${entry.timestamp}-${entry.message}`

  const mergeEntries = (existing: SystemLogEntry[], incoming: SystemLogEntry[]) => {
    const seen = new Set(existing.map(entryKey))
    const merged = [...existing]
    incoming.forEach((entry) => {
      const key = entryKey(entry)
      if (!seen.has(key)) {
        merged.push(entry)
        seen.add(key)
      }
    })
    return merged.slice(-MAX_LOG_ENTRIES)
  }

  const prependEntries = (existing: SystemLogEntry[], incoming: SystemLogEntry[]) => {
    const seen = new Set(existing.map(entryKey))
    const merged = [...incoming.filter((entry) => !seen.has(entryKey(entry))), ...existing]
    return merged.slice(-MAX_LOG_ENTRIES)
  }

  const cancelPendingRequests = () => {
    requestIdRef.current += 1
    setLoading(false)
    setLoadingMore(false)
    loadingMoreRef.current = false
  }

  const loadLogs = async (append = false) => {
    const requestId = requestIdRef.current + 1
    requestIdRef.current = requestId
    if (!selectedServices.length) {
      setEntries([])
      setCursor(null)
      setLoading(false)
      loadingMoreRef.current = false
      return
    }
    if (append) {
      setLoadingMore(true)
      loadingMoreRef.current = true
      if (listRef.current) {
        pendingScrollRestoreRef.current = {
          scrollTop: listRef.current.scrollTop,
          scrollHeight: listRef.current.scrollHeight,
        }
      }
    } else {
      setLoading(true)
    }
    const { data, error } = await querySystemLogs({
      services: selectedServices,
      level: levelParam,
      keyword: debouncedKeyword || undefined,
      since: sinceIso,
      limit: LOG_PAGE_LIMIT,
      direction: 'backward',
      cursor: append ? cursor ?? undefined : undefined,
    })
    if (requestId !== requestIdRef.current) {
      return
    }
    if (error) {
      // eslint-disable-next-line no-console
      console.warn('Log query failed', error)
      setError(error)
    } else {
      setError(null)
    }

    if (data?.entries) {
      setEntries((prev) => {
        if (append) {
          return prependEntries(prev, data.entries)
        }
        return data.entries
      })
      setCursor(data.nextCursor ?? null)
    } else if (!append) {
      setEntries([])
      setCursor(null)
    }

    setLoading(false)
    setLoadingMore(false)
    loadingMoreRef.current = false
  }

  const tailActive = tailEnabled && selectedServices.length === 1
  const activeService = tailActive ? selectedServices[0] : null

  useEffect(() => {
    if (!tailActive) {
      tailCursorRef.current = null
      return
    }
    setEntries([])
    setCursor(null)
    setLoading(false)
  }, [tailActive, activeService])

  useEffect(() => {
    if (tailActive) {
      return
    }
    loadLogs(false)
  }, [selectedServices, levelParam, debouncedKeyword, sinceIso, tailActive])

  useEffect(() => {
    if (!listRef.current) {
      return
    }
    setEntries([])
    setCursor(null)
    setScrollTop(0)
    lastScrollTopRef.current = 0
    lastLoadAtRef.current = 0
    pendingScrollRestoreRef.current = null
    scrollToBottomOnceRef.current = false
    listRef.current.scrollTop = 0
    refreshTokensRef.current.clear()
    setRefreshingServices([])
  }, [selectionKey])

  useEffect(() => {
    if (!tailActive || !activeService) {
      return
    }
    let cancelled = false
    const poll = async () => {
      const { data, error } = await tailSystemLogs({
        service: activeService,
        level: levelParam,
        keyword: debouncedKeyword || undefined,
        limit: LOG_PAGE_LIMIT,
        cursor: tailCursorRef.current ?? undefined,
        from: tailCursorRef.current ? 'start' : 'end',
      })
      if (cancelled) {
        return
      }
      if (error) {
        // eslint-disable-next-line no-console
        console.warn('Log tail failed', error)
        setError(error)
      } else {
        setError(null)
      }
      if (data?.entries?.length) {
        setEntries((prev) => mergeEntries(prev, data.entries))
      }
      if (data?.nextCursor) {
        tailCursorRef.current = data.nextCursor
      }
    }

    poll()
    const intervalId = window.setInterval(poll, tailInterval)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
    }
  }, [tailActive, activeService, levelParam, debouncedKeyword, tailInterval])

  const totals = useMemo(
    () => ({
      total: entries.length,
      warning: entries.filter((entry) => entry.level === 'warning').length,
      error: entries.filter((entry) => entry.level === 'error').length,
    }),
    [entries],
  )

  const startRefresh = (serviceId: string) => {
    const token = `${serviceId}-${Date.now()}`
    refreshTokensRef.current.set(serviceId, token)
    setRefreshingServices(Array.from(refreshTokensRef.current.keys()))
    return token
  }

  const endRefresh = (serviceId: string, token: string) => {
    if (refreshTokensRef.current.get(serviceId) === token) {
      refreshTokensRef.current.delete(serviceId)
      setRefreshingServices(Array.from(refreshTokensRef.current.keys()))
    }
  }

  const handleServiceClick = async (serviceId: string) => {
    const isActive = selectedServices.length === 1 && selectedServices[0] === serviceId
    cancelPendingRequests()
    if (!isActive) {
      scrollToBottomOnceRef.current = true
      setSelectedServices([serviceId])
      return
    }
    const token = startRefresh(serviceId)
    scrollToBottomOnceRef.current = true
    await loadLogs(false)
    endRefresh(serviceId, token)
  }

  const clearAll = () => {
    cancelPendingRequests()
    setSelectedServices([])
  }

  const handleDownload = async (mode: 'filtered' | 'full') => {
    if (!selectedServices.length) {
      setError(new Error('Select at least one service'))
      return
    }
    setDownloading(mode)
    const { data, error } = await downloadSystemLogs({
      services: selectedServices,
      mode,
      level: levelParam,
      keyword: debouncedKeyword || undefined,
      since: sinceIso,
      until: new Date().toISOString(),
    })
    if (error || !data?.url) {
      setError(error ?? new Error('Download failed'))
      setDownloading(null)
      return
    }
    setError(null)
    const url = new URL(data.url, window.location.origin).toString()
    window.open(url, '_blank')
    setDownloading(null)
  }

  const tailBlocked = selectedServices.length !== 1
  const highlightQuery = debouncedKeyword
  const highlightValue = (value: string) => highlightText(value, highlightQuery)

  const viewportHeight = listRef.current?.clientHeight ?? LOG_CONTAINER_HEIGHT
  const effectiveScrollTop = Math.max(scrollTop - LOG_PADDING, 0)
  const startIndex = Math.max(0, Math.floor(effectiveScrollTop / LOG_ROW_HEIGHT) - LOG_OVERSCAN)
  const endIndex = Math.min(
    entries.length,
    Math.ceil((effectiveScrollTop + viewportHeight) / LOG_ROW_HEIGHT) + LOG_OVERSCAN,
  )
  const visibleEntries = entries.slice(startIndex, endIndex)
  const topSpacerHeight = startIndex * LOG_ROW_HEIGHT
  const bottomSpacerHeight = (entries.length - endIndex) * LOG_ROW_HEIGHT

  useEffect(() => {
    const list = listRef.current
    if (!list) {
      return
    }
    if (pendingScrollRestoreRef.current) {
      const snapshot = pendingScrollRestoreRef.current
      const delta = list.scrollHeight - snapshot.scrollHeight
      if (Math.abs(list.scrollTop - snapshot.scrollTop) <= 12) {
        list.scrollTop = snapshot.scrollTop + delta
      }
      pendingScrollRestoreRef.current = null
    }
    if (scrollToBottomOnceRef.current) {
      list.scrollTop = list.scrollHeight
      scrollToBottomOnceRef.current = false
    }
  }, [entries.length])

  return (
    <div className="space-y-6">
      <header className="cp-panel px-8 py-6">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-3 text-lg font-semibold text-[var(--cp-ink)]">
              <span className="inline-flex size-9 items-center justify-center rounded-2xl bg-[var(--cp-primary-soft)] text-[var(--cp-primary-strong)]">
                <Icon name="chart" className="size-4" />
              </span>
              <h1 className="text-2xl font-semibold text-[var(--cp-ink)] sm:text-3xl">
                System Logs
              </h1>
            </div>
            <p className="mt-2 text-sm text-[var(--cp-muted)]">
              Query, tail, and download logs across core BuckyOS services.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <button
              type="button"
              onClick={() => handleDownload('filtered')}
              disabled={downloading !== null}
              className="inline-flex items-center gap-2 rounded-full border border-[var(--cp-border)] bg-white px-4 py-2 text-sm font-semibold text-[var(--cp-ink)] transition hover:bg-[var(--cp-surface-muted)] disabled:cursor-not-allowed disabled:opacity-60"
            >
              Download filtered
            </button>
            <button
              type="button"
              onClick={() => handleDownload('full')}
              disabled={downloading !== null}
              className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-primary)] px-4 py-2 text-sm font-semibold text-white shadow transition hover:bg-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
            >
              Download full
            </button>
          </div>
        </div>
      </header>

      <section className="cp-panel p-6">
        <div className="flex flex-col gap-5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={clearAll}
                className="cp-pill border border-[var(--cp-border)] bg-white text-[var(--cp-muted)] transition hover:bg-[var(--cp-surface-muted)]"
              >
                Clear
              </button>
              {services.map((service) => {
                const active = selectedServices.includes(service.id)
                const refreshing = refreshingServices.includes(service.id)
                return (
                  <button
                    key={service.id}
                    type="button"
                    onClick={() => {
                      void handleServiceClick(service.id)
                    }}
                    className={`cp-pill border border-[var(--cp-border)] text-[var(--cp-muted)] transition ${
                      active
                        ? 'border-transparent bg-[var(--cp-primary)] text-white shadow'
                        : 'bg-[var(--cp-surface-muted)]'
                    }`}
                  >
                    {service.label}
                    {refreshing ? (
                      <span
                        className={`inline-flex size-3 animate-spin rounded-full border-2 ${
                          active
                            ? 'border-white/60 border-t-white'
                            : 'border-[var(--cp-border)] border-t-[var(--cp-primary)]'
                        }`}
                        aria-hidden
                      />
                    ) : null}
                  </button>
                )
              })}
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <select
                value={range}
                onChange={(event) => setRange(event.target.value as '15m' | '1h' | '24h')}
                className="rounded-full border border-[var(--cp-border)] bg-white px-3 py-2 text-sm text-[var(--cp-ink)]"
              >
                {rangeOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <select
                value={tailInterval}
                onChange={(event) => setTailInterval(Number(event.target.value))}
                disabled={!tailEnabled}
                className="rounded-full border border-[var(--cp-border)] bg-white px-3 py-2 text-sm text-[var(--cp-ink)] disabled:opacity-60"
              >
                {tailIntervals.map((option) => (
                  <option key={option.value} value={option.value}>
                    Tail {option.label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                onClick={() => setTailEnabled((prev) => !prev)}
                disabled={tailBlocked}
                className={`cp-pill border border-[var(--cp-border)] text-[var(--cp-muted)] transition ${
                  tailEnabled
                    ? 'border-transparent bg-[var(--cp-primary)] text-white shadow'
                    : 'bg-[var(--cp-surface-muted)]'
                } ${tailBlocked ? 'cursor-not-allowed opacity-60' : ''}`}
              >
                {tailEnabled ? 'Tail on' : 'Tail off'}
              </button>
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-3">
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Total Logs</p>
              <p className="mt-2 text-2xl font-semibold text-[var(--cp-ink)]">{totals.total}</p>
            </div>
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Warnings</p>
              <p className="mt-2 text-2xl font-semibold text-[var(--cp-ink)]">{totals.warning}</p>
            </div>
            <div className="rounded-2xl border border-[var(--cp-border)] bg-[var(--cp-surface-muted)] px-4 py-3">
              <p className="text-xs uppercase tracking-wide text-[var(--cp-muted)]">Errors</p>
              <p className="mt-2 text-2xl font-semibold text-[var(--cp-ink)]">{totals.error}</p>
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap gap-2">
              {(['all', 'info', 'warning', 'error'] as const).map((value) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => setLevelFilter(value)}
                  className={`cp-pill border border-[var(--cp-border)] text-[var(--cp-muted)] transition ${
                    levelFilter === value
                      ? 'border-transparent bg-[var(--cp-primary)] text-white shadow'
                      : 'bg-[var(--cp-surface-muted)]'
                  }`}
                >
                  {value === 'all' ? 'All' : value.toUpperCase()}
                </button>
              ))}
            </div>
            <div className="flex min-w-[220px] flex-1 justify-end">
              <input
                value={keyword}
                onChange={(event) => setKeyword(event.target.value)}
                placeholder="Search logs"
                className="w-full max-w-sm rounded-full border border-[var(--cp-border)] bg-white px-4 py-2 text-sm text-[var(--cp-ink)] placeholder:text-[var(--cp-muted)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]"
              />
            </div>
          </div>
        </div>

        {tailBlocked && tailEnabled ? (
          <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
            Tail mode requires exactly one service. Select a single service to enable tail.
          </div>
        ) : null}

        {error ? (
          <div className="mt-4 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-xs text-amber-700">
            Unable to fetch logs. Showing last known data.
          </div>
        ) : null}

        <LogList
          loading={loading}
          loadingMore={loadingMore}
          hasMore={Boolean(cursor)}
          entries={entries}
          visibleEntries={visibleEntries}
          topSpacerHeight={topSpacerHeight}
          bottomSpacerHeight={bottomSpacerHeight}
          listRef={listRef}
          onScroll={(event) => {
            const target = event.currentTarget
            const currentTop = target.scrollTop
            setScrollTop(currentTop)
            const isScrollingUp = currentTop < lastScrollTopRef.current
            lastScrollTopRef.current = currentTop
            if (
              !tailActive &&
              cursor &&
              currentTop < 60 &&
              isScrollingUp &&
              !loadingMoreRef.current &&
              !loading
            ) {
              const now = Date.now()
              if (now - lastLoadAtRef.current > 500) {
                lastLoadAtRef.current = now
                loadLogs(true)
              }
            }
          }}
          highlightValue={highlightValue}
          entryKey={entryKey}
          selectedServicesLength={selectedServices.length}
        />

        {!tailActive && loadingMore ? (
          <div className="mt-4 flex justify-center text-xs text-[var(--cp-muted)]">
            Loading more logs...
          </div>
        ) : null}
      </section>
    </div>
  )
}

export default SystemLogsPage
