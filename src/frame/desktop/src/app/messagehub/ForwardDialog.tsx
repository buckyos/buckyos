import { Bot, Search, SlidersHorizontal, User, Users } from 'lucide-react'
import { useRef, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { messageSummaryText } from './conversation/history/relations'
import { groupErrorText } from './groupModel'
import type { MessageObject } from './protocol/msgobj'
import { DialogFocus, hubButtonClass, hubInputClass } from './SessionDialogs'
import { useMessageHubStore } from './store'
import type { Entity, MessageHubContext } from './types'

function TargetIcon({ type }: { type: Entity['type'] }) {
  if (type === 'agent') return <Bot size={16} />
  if (type === 'group') return <Users size={16} />
  if (type === 'service') return <SlidersHorizontal size={16} />
  return <User size={16} />
}

/** Picks the entity whose default session receives a copy of `message`. */
export function ForwardMessageForm({ context, message, confirmationFor, onDone, onCancel }: { context: MessageHubContext; message: MessageObject; confirmationFor: (sessionId: string) => string | undefined; onDone: (entity: Entity) => void; onCancel: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const [query, setQuery] = useState('')
  const [pending, setPending] = useState<string | null>(null)
  const [error, setError] = useState('')
  const busy = useRef(false)
  const needle = query.trim().toLowerCase()
  const targets = store.entities(context).flatMap(entity => [entity, ...(entity.children ?? [])]).filter(entity => !needle || entity.name.toLowerCase().includes(needle))
  const send = async (entity: Entity) => {
    if (busy.current) return
    busy.current = true; setPending(entity.id); setError('')
    try {
      const session = await store.ensureDefaultSession(context, entity.id)
      if (!session) throw Error('session_missing')
      await store.forward(context, session.id, message, confirmationFor(session.id))
      onDone(entity)
    } catch (failure) {
      setError(failure instanceof Error && failure.message.startsWith('rejected') ? groupErrorText(t, failure) : t('messagehub.forward.failed', undefined, { name: entity.name }))
    } finally { busy.current = false; setPending(null) }
  }
  return <DialogFocus onCancel={() => { if (!busy.current) onCancel() }}><div className="space-y-3" data-testid="forward-dialog">
    <p className="line-clamp-2 break-words rounded-lg border-l-2 border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)] px-3 py-2 text-[13px] text-[color:var(--cp-muted)]" data-testid="forward-preview">{messageSummaryText(message)}</p>
    <label className="relative block">
      <span className="sr-only">{t('messagehub.forward.search')}</span>
      <Search size={15} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[color:var(--cp-muted)]" aria-hidden />
      <input data-autofocus className={`${hubInputClass} mt-0 pl-9`} placeholder={t('messagehub.forward.search')} value={query} onChange={event => setQuery(event.target.value)} />
    </label>
    <ul className="max-h-[320px] space-y-0.5 overflow-y-auto" data-testid="forward-targets">
      {targets.map(entity => <li key={entity.id}>
        <button type="button" disabled={pending !== null} onClick={() => void send(entity)} className="flex min-h-11 w-full items-center gap-3 rounded-lg px-2 text-left text-sm hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)] disabled:opacity-50">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-text)_7%,transparent)] text-[color:var(--cp-muted)]" aria-hidden><TargetIcon type={entity.type} /></span>
          <span className="min-w-0 flex-1 truncate">{entity.name}</span>
          {pending === entity.id ? <span className="shrink-0 text-xs text-[color:var(--cp-muted)]">{t('messagehub.forward.sending')}</span> : null}
        </button>
      </li>)}
      {targets.length === 0 ? <li className="px-2 py-4 text-center text-sm text-[color:var(--cp-muted)]">{t('messagehub.forward.empty')}</li> : null}
    </ul>
    {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
    <div className="flex justify-end"><button type="button" className={hubButtonClass} disabled={pending !== null} onClick={onCancel}>{t('messagehub.cancel')}</button></div>
  </div></DialogFocus>
}
