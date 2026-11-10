import { ArrowLeft, ChevronDown, ChevronUp, Loader2, Search } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import type { MessageObject } from '../../protocol/msgobj'
import type { ConversationMessageReader } from '../history/types'
import { searchMessages } from './model'

const QUERY_DEBOUNCE_MS = 200
/** Older pages one step back reads before it stops; the next step continues from there. */
const SEEK_PAGE_LIMIT = 20
/** A page that was not read (another read was running, or it failed) is asked for again this many times. */
const SEEK_RETRY_LIMIT = 5
const SEEK_RETRY_DELAY_MS = 400
/** How long a read page may take to show up as new rows before the step is given up. */
const SEEK_ARRIVAL_TIMEOUT_MS = 5_000
const NO_ROWS: readonly MessageObject[] = []

/** A step back through history that is not read yet: looking for a hit older than `before` (any hit when null). */
interface Seek {
  before: string | null
  pages: number
  retries: number
  /** The reader whose rows were searched last; its older page is being read. */
  searched: ConversationMessageReader | null
}

const iconButtonClass = 'flex min-h-11 min-w-11 shrink-0 items-center justify-center rounded-lg text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)] disabled:opacity-40 disabled:hover:text-[color:var(--cp-muted)] md:min-w-9'

/**
 * Search of one session, in place of the conversation header (after Telegram):
 * the newest match is located first, the arrows step through the others, and a
 * step past the oldest loaded match reads older history until it finds one.
 */
export function ConversationSearchBar({ reader, hasOlder, onLoadOlder, onFocus, onClose }: {
  reader: ConversationMessageReader
  hasOlder: boolean
  onLoadOlder?: () => Promise<boolean>
  /** Brings the row with this stable id into view. */
  onFocus: (messageId: string) => void
  onClose: () => void
}) {
  const { t } = useI18n()
  const [query, setQuery] = useState('')
  const [term, setTerm] = useState('')
  const [loaded, setLoaded] = useState<{ reader: ConversationMessageReader; rows: readonly MessageObject[] } | null>(null)
  const [selection, setSelection] = useState<{ term: string; id: string | null }>({ term: '', id: null })
  const [seek, setSeek] = useState<Seek | null>(null)
  const canReadOlder = hasOlder && Boolean(onLoadOlder)

  useEffect(() => {
    const timer = setTimeout(() => setTerm(query.trim()), QUERY_DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [query])

  useEffect(() => {
    let cancelled = false
    void reader.readRange(0, reader.totalCount).then(rows => { if (!cancelled) setLoaded({ reader, rows }) })
    return () => { cancelled = true }
  }, [reader])

  // The rows on hand are those of the current reader: a decision about the history waits for them.
  const settled = loaded?.reader === reader
  const rows = loaded?.rows ?? NO_ROWS
  const hits = useMemo(() => searchMessages(rows, term), [rows, term])
  const index = selection.term === term ? hits.findIndex(hit => hit.id === selection.id) : -1

  if (settled && selection.term !== term) {
    // A new query starts at its newest match; with none loaded it goes on into older history.
    const newest = hits.at(-1)
    setSelection({ term, id: newest?.id ?? null })
    setSeek(term && !newest && canReadOlder ? { before: null, pages: 0, retries: 0, searched: null } : null)
  } else if (settled && seek && seek.searched !== reader) {
    const from = seek.before ? hits.findIndex(hit => hit.id === seek.before) : hits.length
    if (from > 0) {
      setSelection({ term, id: hits[from - 1].id })
      setSeek(null)
    } else if (!canReadOlder || seek.pages >= SEEK_PAGE_LIMIT) {
      setSeek(null)
    } else {
      setSeek({ ...seek, pages: seek.pages + 1, searched: reader })
    }
  } else if (settled && !seek && selection.id && index < 0) {
    // The located message is gone (deleted, or cleared with the history).
    setSelection({ term, id: hits.at(-1)?.id ?? null })
  }

  useEffect(() => {
    if (selection.id) onFocus(selection.id)
  }, [selection, onFocus])

  useEffect(() => {
    if (!seek || seek.searched !== reader || !onLoadOlder) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    void onLoadOlder().then((read) => {
      if (cancelled) return
      // A page that was read arrives as a new reader, which the step above searches.
      if (read) timer = setTimeout(() => setSeek(null), SEEK_ARRIVAL_TIMEOUT_MS)
      else if (seek.retries >= SEEK_RETRY_LIMIT) setSeek(null)
      else timer = setTimeout(() => setSeek({ ...seek, pages: seek.pages - 1, retries: seek.retries + 1, searched: null }), SEEK_RETRY_DELAY_MS)
    })
    return () => { cancelled = true; clearTimeout(timer) }
  }, [seek, reader, onLoadOlder])

  const seeking = seek !== null
  const older = () => {
    if (!term || seeking) return
    if (index > 0) setSelection({ term, id: hits[index - 1].id })
    else if (canReadOlder) setSeek({ before: index === 0 ? hits[0].id : null, pages: 0, retries: 0, searched: null })
    else if (index === 0) setSelection({ term, id: hits[0].id })
  }
  const newer = () => {
    if (index >= 0 && index < hits.length - 1) setSelection({ term, id: hits[index + 1].id })
  }
  const position = index >= 0 ? hits.length - index : 0
  const status = !term ? ''
    : seeking ? t('messagehub.search.seeking')
      : hits.length === 0 ? t(canReadOlder ? 'messagehub.search.noneSoFar' : 'messagehub.search.none')
        : t(canReadOlder ? 'messagehub.search.countPartial' : 'messagehub.search.count', undefined, { position, total: hits.length })

  return (
    <div
      role="search"
      className="flex shrink-0 items-center gap-1 px-2 py-1.5 md:gap-2 md:px-3"
      style={{ borderBottom: '1px solid var(--cp-border)', background: 'var(--cp-surface)' }}
      data-testid="conversation-search"
      data-hits={hits.length}
      data-position={position}
    >
      <button type="button" onClick={onClose} className={`${iconButtonClass} -ml-2 md:ml-0`} aria-label={t('messagehub.search.close')} title={t('messagehub.search.close')} data-testid="conversation-search-close">
        <ArrowLeft size={20} />
      </button>
      <label className="flex min-h-9 min-w-0 flex-1 items-center gap-2 rounded-xl px-3" style={{ background: 'color-mix(in srgb, var(--cp-text) 6%, transparent)' }}>
        <Search size={15} className="shrink-0" style={{ color: 'var(--cp-muted)' }} aria-hidden />
        <input
          type="search"
          autoFocus
          value={query}
          onChange={event => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onClose() }
            else if (event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); if (event.shiftKey) newer(); else older() }
          }}
          placeholder={t('messagehub.search.placeholder')}
          aria-label={t('messagehub.search.placeholder')}
          className="min-w-0 flex-1 border-none bg-transparent text-sm outline-none"
          style={{ color: 'var(--cp-text)' }}
          data-testid="conversation-search-input"
        />
      </label>
      <span role="status" className="flex max-w-[40%] shrink-0 items-center gap-1.5 text-xs tabular-nums text-[color:var(--cp-muted)]" data-testid="conversation-search-status">
        {seeking ? <Loader2 size={14} className="shrink-0 animate-spin" aria-hidden /> : null}
        <span className="truncate">{status}</span>
      </span>
      <button type="button" onClick={older} disabled={!term || seeking || (index <= 0 && !canReadOlder)} className={iconButtonClass} aria-label={t('messagehub.search.older')} title={t('messagehub.search.older')} data-testid="conversation-search-older">
        <ChevronUp size={20} />
      </button>
      <button type="button" onClick={newer} disabled={index < 0 || index >= hits.length - 1} className={iconButtonClass} aria-label={t('messagehub.search.newer')} title={t('messagehub.search.newer')} data-testid="conversation-search-newer">
        <ChevronDown size={20} />
      </button>
    </div>
  )
}
