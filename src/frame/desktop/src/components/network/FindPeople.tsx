/* ── FindPeople: one search box over `contact.lookup`, results in sections, actions from the host ── */

import { Button } from '@mui/material'
import { RefreshCw, Search } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { useLookup } from './hooks'
import { KindLabel, ListSkeleton, PersonRow, ReachabilityChip, SectionTitle, StateMessage } from './Person'
import { RelationBadge } from './RelationBadge'
import { networkErrorText, relationOfCandidate } from './relation'
import type { LookupCandidate, LookupKind } from './types'

export interface FindPeopleAction {
  id: string
  label: string
  icon?: ReactNode
  primary?: boolean
  /** Offer the action only for these results (default: every result). */
  appliesTo?: (candidate: LookupCandidate) => boolean
  onSelect: (candidate: LookupCandidate) => void | Promise<void>
}

/** A host-specific input parser (e.g. a HomeStation page URL); its results get their own section. */
export interface FindPeopleResolver {
  id: string
  label: string
  matches: (input: string) => boolean
  resolve: (input: string) => Promise<LookupCandidate[]>
}

export interface FindPeopleProps {
  initialInput?: string
  kinds?: LookupKind[]
  limit?: number
  actions?: FindPeopleAction[]
  resolvers?: FindPeopleResolver[]
  /** A click on the result itself (its name and picture). */
  onOpen?: (candidate: LookupCandidate) => void
  placeholder?: string
  autoFocus?: boolean
  /** List the owner's relations while the box is empty (default true). */
  showDefault?: boolean
  onInputChange?: (input: string) => void
}

const SECTION_ORDER = ['resolved', 'relation', 'zone'] as const

function useHostResults(resolvers: FindPeopleResolver[], input: string) {
  const [results, setResults] = useState<Record<string, { status: 'loading' | 'ready' | 'error'; items: LookupCandidate[] }>>({})
  const active = resolvers.filter(resolver => input.trim() && resolver.matches(input.trim()))
  const activeKey = active.map(resolver => resolver.id).join(',')
  useEffect(() => {
    let cancelled = false
    const trimmed = input.trim()
    const current = resolvers.filter(resolver => trimmed && resolver.matches(trimmed))
    const timer = window.setTimeout(() => {
      setResults(Object.fromEntries(current.map(resolver => [resolver.id, { status: 'loading' as const, items: [] }])))
      for (const resolver of current) {
        resolver.resolve(trimmed)
          .then(items => { if (!cancelled) setResults(previous => ({ ...previous, [resolver.id]: { status: 'ready', items } })) })
          .catch(() => { if (!cancelled) setResults(previous => ({ ...previous, [resolver.id]: { status: 'error', items: [] } })) })
      }
    }, 250)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
    // resolvers are host constants; the active set and the input decide when to re-run
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeKey, input])
  return active.map(resolver => ({ resolver, state: results[resolver.id] ?? { status: 'loading' as const, items: [] } }))
}

export function FindPeople({
  initialInput = '',
  kinds,
  limit = 30,
  actions = [],
  resolvers = [],
  onOpen,
  placeholder,
  autoFocus = false,
  showDefault = true,
  onInputChange,
}: FindPeopleProps) {
  const { t } = useI18n()
  const [input, setInput] = useState(initialInput)
  const lookup = useLookup(input, { kinds, limit, enabled: showDefault || Boolean(input.trim()) })
  const hosts = useHostResults(resolvers, input)
  const [busy, setBusy] = useState<string | null>(null)
  const candidates = lookup.result?.candidates ?? []
  const lookupError = lookup.result?.error
  const showLoading = lookup.status === 'loading' && !lookup.result
  const empty = lookup.status === 'ready' && !lookupError && candidates.length === 0 && hosts.every(host => host.state.status !== 'loading' && host.state.items.length === 0)
  const partial = candidates.some(item => item.reachability.state === 'unreachable') && candidates.some(item => item.reachability.state !== 'unreachable')

  const run = async (action: FindPeopleAction, candidate: LookupCandidate) => {
    setBusy(`${action.id}:${candidate.did}`)
    try { await action.onSelect(candidate) } finally { setBusy(null) }
  }

  const renderRow = (candidate: LookupCandidate) => {
    const relation = relationOfCandidate(candidate)
    const applicable = actions.filter(action => !action.appliesTo || action.appliesTo(candidate))
    return (
      <PersonRow
        key={`${candidate.section}:${candidate.did}`}
        testId="find-people-result"
        person={candidate}
        onClick={onOpen ? () => onOpen(candidate) : undefined}
        secondary={<><KindLabel kind={candidate.kind} />{candidate.reachability.zone ? ` · ${candidate.reachability.zone}` : ''}</>}
        trailing={
          <>
            {candidate.kind !== 'group' ? <RelationBadge relation={relation} compact /> : null}
            {candidate.reachability.state !== 'local' ? <ReachabilityChip reachability={candidate.reachability} /> : null}
            {applicable.map(action => (
              <Button
                key={action.id}
                size="small"
                variant={action.primary ? 'contained' : 'outlined'}
                startIcon={action.icon}
                disabled={busy !== null}
                onClick={() => void run(action, candidate)}
              >
                {action.label}
              </Button>
            ))}
          </>
        }
      />
    )
  }

  return (
    <div className="space-y-2" data-testid="find-people">
      <label className="flex min-h-11 items-center gap-2 rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3">
        <Search size={15} className="shrink-0 text-[color:var(--cp-muted)]" aria-hidden />
        <input
          type="search"
          value={input}
          autoFocus={autoFocus}
          onChange={(event) => {
            setInput(event.target.value)
            onInputChange?.(event.target.value)
          }}
          placeholder={placeholder ?? t('network.find.placeholder')}
          aria-label={t('network.find.label')}
          className="min-w-0 flex-1 border-none bg-transparent text-sm text-[color:var(--cp-text)] outline-none"
        />
      </label>

      {showLoading ? <ListSkeleton rows={3} /> : null}
      {lookup.status === 'error' ? (
        <StateMessage tone="danger" testId="find-people-error" action={<Button variant="text" size="small" startIcon={<RefreshCw size={13} />} onClick={lookup.retry}>{t('network.retry')}</Button>}>
          {networkErrorText(t, lookup.error)}
        </StateMessage>
      ) : null}
      {lookupError ? (
        <StateMessage tone={lookupError.code === 'not-found' ? 'muted' : 'danger'} testId="find-people-lookup-error">
          {t(`network.lookup.${lookupError.code}`)}
        </StateMessage>
      ) : null}
      {empty ? <StateMessage testId="find-people-empty">{input.trim() ? t('network.find.noMatches') : t('network.find.noRelations')}</StateMessage> : null}
      {partial ? <StateMessage tone="warning" testId="find-people-partial">{t('network.find.partial')}</StateMessage> : null}
      {!showDefault && !input.trim() ? <p className="px-1 text-[13px] text-[color:var(--cp-muted)]">{t('network.find.hint')}</p> : null}

      {SECTION_ORDER.map(section => {
        const items = candidates.filter(item => item.section === section)
        if (items.length === 0) return null
        return (
          <section key={section} data-testid={`find-people-section-${section}`}>
            <SectionTitle>{t(`network.section.${section}`)}</SectionTitle>
            <div className="space-y-0.5">{items.map(renderRow)}</div>
          </section>
        )
      })}
      {hosts.map(({ resolver, state }) => (
        <section key={resolver.id} data-testid={`find-people-section-${resolver.id}`}>
          <SectionTitle>{resolver.label}</SectionTitle>
          {state.status === 'loading' ? <ListSkeleton rows={1} /> : null}
          {state.status === 'error' ? <StateMessage tone="danger">{t('network.find.resolverFailed')}</StateMessage> : null}
          <div className="space-y-0.5">{state.items.map(renderRow)}</div>
        </section>
      ))}
    </div>
  )
}
