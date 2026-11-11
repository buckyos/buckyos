import { useEffect, useRef, useState } from 'react'
import clsx from 'clsx'
import { ChevronRight, Trash2 } from 'lucide-react'
import { useSWRConfig } from 'swr'
import useSWRInfinite from 'swr/infinite'
import { ago, t, type TextKey } from '../i18n'
import { goalHref, sessionHref, toText, useQuery } from '../lib'
import { dataModel, OpenDanError, type MemoryEntry, type MemoryLayer, type MemoryPage } from '../model'
import { Dialog, HomeShell } from '../shell'
import { ErrorBox } from '../ui'

const PAGE_SIZE = 20

const LAYER_LABEL: Record<MemoryLayer, TextKey> = { cognition: 'memCognitions', perception: 'memPerceptions' }

const STATE_LABEL: Record<string, TextKey> = {
  disputed: 'memDisputed',
  pending: 'memPending',
  deferred: 'memDeferred',
  absorbed: 'memAbsorbed',
  discarded: 'memDiscarded',
}

function EntryLine({ entry }: { entry: MemoryEntry }) {
  const state = STATE_LABEL[entry.state]
  return (
    <span className="block min-w-0 flex-1">
      <span className="line-clamp-2 break-words text-sm">{entry.summary}</span>
      <span className="block truncate text-xs text-mute">
        {ago(entry.at_ms)}
        {entry.revised_at_ms ? ` · ${t('memRevised', { time: ago(entry.revised_at_ms) })}` : ''}
        {state && ` · ${t(state)}`}
        {entry.subjects.length > 0 && ` · ${entry.subjects.join(', ')}`}
      </span>
    </span>
  )
}

export function MemoryCard() {
  // Counted from files: read when the page opens and on a manual refresh, not on the 4 s poll.
  const overview = useQuery(['memory.overview'], () => dataModel.memoryOverview(), false)
  const data = overview.data
  const recent = data ? [...data.recent.cognitions, ...data.recent.perceptions] : []
  return (
    <section className="card min-w-0 p-4" data-testid="memory-card">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="text-base font-semibold">{t('memory')}</h2>
        <a href="#/memory" className="flex min-h-11 items-center text-sm text-accent" data-testid="memory-all">
          {t('viewAll')}
          <ChevronRight size={16} />
        </a>
      </div>
      <ErrorBox error={overview.error} />
      {data && (
        <>
          <div className="grid grid-cols-2 gap-2">
            <div className="rounded-xl bg-soft px-3 py-2.5" data-testid="memory-cognitions">
              <div className="text-2xl font-semibold tabular-nums leading-tight">{data.cognitions.current}</div>
              <div className="text-xs text-mute">{t('memCognitions')}</div>
            </div>
            <div className="rounded-xl bg-soft px-3 py-2.5" data-testid="memory-perceptions">
              <div className="text-2xl font-semibold tabular-nums leading-tight">{data.perceptions.total}</div>
              <div className="text-xs text-mute">{t('memPerceptions')} · {t('memPendingCount', { n: data.perceptions.pending })}</div>
            </div>
          </div>
          <p className="mt-2 text-xs text-mute" data-testid="memory-consolidation">
            {data.maintenance_goal_id ? (
              <a href={goalHref(data.maintenance_goal_id)} className="inline-flex min-h-11 items-center text-accent">
                {data.last_consolidation_at_ms ? t('memConsolidated', { time: ago(data.last_consolidation_at_ms) }) : t('memNeverConsolidated')}
              </a>
            ) : t('memNoMaintenance')}
          </p>
          {recent.length ? (
            <ul className="divide-y divide-line">
              {recent.map((entry) => (
                <li key={entry.reference} className="flex items-center gap-2 py-2" data-testid={`memory-recent-${entry.layer}`}>
                  <span className="w-10 shrink-0 text-xs text-mute">{t(entry.layer === 'cognition' ? 'memCognitionShort' : 'memPerceptionShort')}</span>
                  <EntryLine entry={entry} />
                </li>
              ))}
            </ul>
          ) : (
            <p className="py-2 text-sm text-mute">{t('memEmpty')}</p>
          )}
        </>
      )}
      {!data && !overview.error && <p className="py-2 text-sm text-mute">{t('loading')}</p>}
    </section>
  )
}

function EntryRow({ entry, onForget }: { entry: MemoryEntry; onForget: (entry: MemoryEntry, revision?: number) => void }) {
  const [open, setOpen] = useState(false)
  const detail = useQuery(open ? ['memory.read', entry.reference] : null, () => dataModel.memoryRead(entry.reference), false)
  return (
    <li className="py-2" data-testid={`memory-entry-${entry.reference}`}>
      <div className="flex items-center gap-1">
        <button className="flex min-h-11 min-w-0 flex-1 items-center text-left" onClick={() => setOpen(!open)} aria-expanded={open}>
          <EntryLine entry={entry} />
        </button>
        {entry.deletable && (
          <button className="tap shrink-0 px-3 text-bad" aria-label={t('memDelete')} data-testid="memory-delete"
            onClick={() => onForget(entry, entry.revision)}>
            <Trash2 size={16} />
          </button>
        )}
      </div>
      {entry.absorbed_into?.length ? <p className="text-xs text-mute" data-testid="memory-absorbed">{t('memAbsorbedInto', { reference: entry.absorbed_into.join(', ') })}</p> : null}
      {open && (
        <div className="mt-1 text-xs" data-testid="memory-detail">
          <ErrorBox error={detail.error} />
          <p className="break-all font-mono text-mute">{entry.reference}</p>
          {entry.session_id && <a href={sessionHref(entry.session_id)} className="inline-flex min-h-11 items-center break-all text-accent">{t('memSource')} · {entry.session_id}</a>}
          {detail.data ? (
            <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-xl bg-bg p-2 font-mono">{toText(detail.data.detail)}</pre>
          ) : !detail.error && <p className="text-mute">{t('loading')}</p>}
        </div>
      )}
    </li>
  )
}

function EntryList({ layer, hidden }: { layer: MemoryLayer; hidden: boolean }) {
  const { mutate } = useSWRConfig()
  const pages = useSWRInfinite<MemoryPage, Error>(
    (_index: number, previous: MemoryPage | null) => (previous && !previous.next_cursor ? null : ['memory.list', layer, previous?.next_cursor ?? '']),
    ([, , cursor]: [string, MemoryLayer, string]) => dataModel.memoryList(layer, PAGE_SIZE, cursor || undefined),
    { revalidateOnFocus: false, revalidateFirstPage: false, shouldRetryOnError: false },
  )
  const [forgetting, setForgetting] = useState<{ entry: MemoryEntry; revision?: number } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const entries = pages.data?.flatMap((page) => page.entries) ?? []
  const more = Boolean(pages.data?.at(-1)?.next_cursor)
  const loading = pages.isValidating
  const { setSize } = pages
  const sentinel = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const node = sentinel.current
    if (!node || !more || loading) return
    const observer = new IntersectionObserver((seen) => {
      if (seen.some((s) => s.isIntersecting)) void setSize((size) => size + 1)
    })
    observer.observe(node)
    return () => observer.disconnect()
  }, [more, loading, setSize, entries.length])

  const forget = async () => {
    if (!forgetting) return
    setBusy(true)
    setError(undefined)
    try {
      await dataModel.memoryForget(forgetting.entry.reference, forgetting.revision)
      setForgetting(null)
      await Promise.all([pages.mutate(), mutate(['memory.overview'])])
    } catch (cause) {
      setError(cause instanceof OpenDanError && cause.kind === 'busy' ? new Error(t('memBusy')) : cause)
    } finally {
      setBusy(false)
    }
  }
  const cognition = layer === 'cognition'
  return (
    <section className={clsx('card min-w-0 p-4', hidden && 'hidden lg:block')} data-testid={`memory-list-${layer}`}>
      <h2 className="mb-1 hidden text-base font-semibold lg:block">{t(LAYER_LABEL[layer])}</h2>
      <ErrorBox error={pages.error} />
      {entries.length ? (
        <ul className="divide-y divide-line">
          {entries.map((entry) => <EntryRow key={entry.reference} entry={entry} onForget={(e, revision) => { setError(undefined); setForgetting({ entry: e, revision }) }} />)}
        </ul>
      ) : !pages.error && <p className="py-2 text-sm text-mute">{t(pages.isLoading ? 'loading' : 'memEmpty')}</p>}
      {more && (
        <div ref={sentinel}>
          <button className="tap mt-2 w-full border border-line" disabled={loading} onClick={() => void setSize((size) => size + 1)} data-testid="memory-more">
            {t(loading ? 'loading' : 'memMore')}
          </button>
        </div>
      )}
      {forgetting && (
        <Dialog title={t(cognition ? 'memDeleteCognitionTitle' : 'memDeletePerceptionTitle')} onSubmit={() => void forget()} busy={busy} danger
          onClose={() => setForgetting(null)} label={t('memDelete')} testId="memory-forget">
          <p className="my-3 break-words text-sm">{forgetting.entry.summary}</p>
          <p className="mb-4 text-sm text-mute">{t(cognition ? 'memDeleteCognitionHint' : 'memDeletePerceptionHint')}</p>
          <ErrorBox error={error} />
        </Dialog>
      )}
    </section>
  )
}

export default function MemoryPage() {
  const [tab, setTab] = useState<MemoryLayer>('cognition')
  const layers: MemoryLayer[] = ['cognition', 'perception']
  return (
    <HomeShell title={t('memory')} testId="memory-page" wide>
      <p className="text-xs text-mute">{t('memPageHint')}</p>
      <div className="flex gap-2 lg:hidden" role="tablist">
        {layers.map((layer) => (
          <button key={layer} role="tab" aria-selected={tab === layer} onClick={() => setTab(layer)}
            className={clsx('tap flex-1 border', tab === layer ? 'border-accent bg-accent text-panel' : 'border-line')}>
            {t(LAYER_LABEL[layer])}
          </button>
        ))}
      </div>
      <div className="grid grid-cols-[minmax(0,1fr)] items-start gap-3 lg:grid-cols-2">
        {layers.map((layer) => <EntryList key={layer} layer={layer} hidden={tab !== layer} />)}
      </div>
    </HomeShell>
  )
}
