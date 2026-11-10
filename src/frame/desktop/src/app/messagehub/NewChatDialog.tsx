import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { Search } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useLookup } from '../../components/network/hooks'
import { PersonAvatar } from '../../components/network/Person'
import { personName, shortDid } from '../../components/network/personPresentation'
import { networkErrorText } from '../../components/network/relation'
import type { LookupCandidate } from '../../components/network/types'
import { rankEntities } from './entitySearch'
import { HubAvatar } from './HubAvatar'
import { DialogFocus, hubButtonClass, hubPrimaryButtonClass } from './SessionDialogs'
import { useMessageHubStore } from './store'
import type { Entity, MessageHubContext } from './types'

/** Whom the dialog was left with: an entity of the list, or someone found who has no conversation yet. */
export interface NewChatTarget {
  id: string
  name: string
  type: Entity['type']
  /** Already an entity of the owner's list. */
  listed: boolean
}

type Row =
  | { key: string; target: NewChatTarget; entity: Entity }
  | { key: string; target: NewChatTarget; candidate: LookupCandidate }

const LOOKUP_LIMIT = 20

/**
 * The list's "new chat": a fuzzy search over the owner's entities, continued by
 * `contact.lookup` for people and Agents not listed yet. The first match is
 * selected, the arrow keys move the selection, and Enter opens it.
 */
export function NewChatForm({ context, onEnter, onCancel }: { context: MessageHubContext; onEnter: (target: NewChatTarget) => void; onCancel: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const [query, setQuery] = useState('')
  const [picked, setPicked] = useState<string | null>(null)
  const listId = useId()
  const listRef = useRef<HTMLUListElement>(null)
  const canFind = context.mode === 'self' && context.ownerDid === context.viewerDid
  const lookup = useLookup(query, { kinds: ['user', 'agent'], limit: LOOKUP_LIMIT, enabled: canFind })
  const snapshot = store.getSnapshot()
  const entities = useMemo(() => store.entities(context).flatMap(entity => [entity, ...(entity.children ?? [])]), [store, snapshot, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps
  const rows = useMemo<Row[]>(() => {
    const listed = rankEntities(entities, query).map(entity => ({ key: entity.id, target: { id: entity.id, name: entity.name, type: entity.type, listed: true }, entity }))
    const known = new Set(entities.map(entity => entity.id))
    // The lookup answers the debounced input: results of an earlier input are not shown under a newer one.
    const found = lookup.input === query.trim() ? lookup.result?.candidates ?? [] : []
    const others = found.filter(candidate => !known.has(candidate.did) && candidate.did !== context.ownerDid && candidate.reachability.state !== 'unreachable')
      .map(candidate => ({ key: candidate.did, target: { id: candidate.did, name: personName(candidate), type: candidate.kind === 'agent' ? 'agent' as const : 'person' as const, listed: false }, candidate }))
    return [...listed, ...others]
  }, [entities, query, lookup.input, lookup.result, context.ownerDid])
  const active = rows.find(row => row.key === picked) ?? rows[0] ?? null
  const activeKey = active?.key
  const optionId = (key: string) => `${listId}-${rows.findIndex(row => row.key === key)}`

  useEffect(() => {
    listRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [activeKey])

  const move = (step: number) => {
    if (rows.length === 0) return
    const index = active ? rows.indexOf(active) : -1
    setPicked(rows[Math.min(rows.length - 1, Math.max(0, index + step))].key)
  }
  const typeLabel = (type: Entity['type']) => t(`messagehub.entityType.${type}`)
  const looking = canFind && (lookup.status === 'loading' || lookup.input !== query.trim())
  const lookupFailed = canFind && lookup.status === 'error' && query.trim() !== ''

  return <DialogFocus onCancel={onCancel}><form className="space-y-3" data-testid="new-chat" onSubmit={(event) => { event.preventDefault(); if (active) onEnter(active.target) }}>
    <label className="flex min-h-11 items-center gap-2 rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3">
      <Search size={16} className="shrink-0 text-[color:var(--cp-muted)]" aria-hidden />
      <input
        data-autofocus
        type="search"
        role="combobox"
        aria-expanded="true"
        aria-controls={listId}
        aria-activedescendant={active ? optionId(active.key) : undefined}
        aria-label={t('messagehub.newChat.search')}
        placeholder={t('messagehub.newChat.search')}
        autoComplete="off"
        value={query}
        onChange={(event) => { setQuery(event.target.value); setPicked(null) }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') { event.preventDefault(); move(1) }
          else if (event.key === 'ArrowUp') { event.preventDefault(); move(-1) }
        }}
        className="min-w-0 flex-1 border-none bg-transparent text-sm text-[color:var(--cp-text)] outline-none"
        data-testid="new-chat-search"
      />
    </label>
    <ul ref={listRef} id={listId} role="listbox" aria-label={t('messagehub.newChat.results')} className="shell-scrollbar -mx-1 max-h-[min(50vh,420px)] min-h-24 space-y-0.5 overflow-y-auto px-1" data-testid="new-chat-results">
      {rows.map((row) => {
        const selected = row === active
        return <li
          key={row.key}
          id={optionId(row.key)}
          role="option"
          aria-selected={selected}
          onClick={() => setPicked(row.key)}
          onDoubleClick={() => onEnter(row.target)}
          className="flex min-h-14 cursor-pointer items-center gap-3 rounded-xl border border-transparent px-2.5 py-1.5"
          style={selected ? { background: 'color-mix(in srgb, var(--cp-accent) 14%, transparent)', borderColor: 'color-mix(in srgb, var(--cp-accent) 30%, transparent)' } : undefined}
          data-testid="new-chat-option"
          data-entity-id={row.target.id}
          data-listed={row.target.listed}
        >
          {'entity' in row
            ? <HubAvatar type={row.entity.type} name={row.entity.name} avatar={row.entity.avatar} size={40} isOnline={row.entity.isOnline} systemNotify={row.entity.systemNotify} />
            : <PersonAvatar person={row.candidate} size={40} />}
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[15px] font-semibold text-[color:var(--cp-text)]">{row.target.name}</span>
            <span className="block truncate text-xs text-[color:var(--cp-muted)]" title={row.target.id}>
              {'entity' in row
                ? [row.entity.systemNotify ? '' : typeLabel(row.entity.type), row.entity.statusText, row.entity.id.startsWith('did:') ? shortDid(row.entity.id) : ''].filter(Boolean).join(' · ')
                : [typeLabel(row.target.type), row.candidate.reachability.zone, shortDid(row.candidate.did), t('messagehub.newChat.notListed')].filter(Boolean).join(' · ')}
            </span>
          </span>
        </li>
      })}
      {rows.length === 0 && !looking ? <li role="presentation" className="px-2.5 py-6 text-center text-sm text-[color:var(--cp-muted)]" data-testid="new-chat-empty">{t(canFind ? 'messagehub.newChat.empty' : 'messagehub.noSearchResults')}</li> : null}
    </ul>
    <p role="status" className="min-h-4 px-1 text-xs text-[color:var(--cp-muted)]">{lookupFailed ? networkErrorText(t, lookup.error) : looking && query.trim() ? t('messagehub.newChat.looking') : ''}</p>
    <div className="flex justify-end gap-2">
      <button type="button" className={hubButtonClass} onClick={onCancel}>{t('messagehub.cancel')}</button>
      <button type="submit" className={hubPrimaryButtonClass} disabled={!active} data-testid="new-chat-enter">{t('messagehub.newChat.enter')}</button>
    </div>
  </form></DialogFocus>
}
