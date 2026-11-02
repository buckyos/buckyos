/* What an instance does when its control raises an event (扩展控件 P2, Q26): one row per event the definition declares,
 * each with up to four handlers — a typed write under one of the instance's bindings, a wish run, opening data, a
 * notice. Only declared events and existing bindings are offered (the business check the core leaves to clients);
 * a bound event the definition no longer declares can be removed. Saved as `entity.set_props { key: "on" }`. */

import { useState } from 'react'
import type { BlockDefPayload, EntityEnvelope, EventBindings, EventHandler, Json } from '../../api/types'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { handlersOf, MAX_HANDLERS } from './events'

type Kind = EventHandler['kind']
const KINDS: Kind[] = ['write', 'wish', 'open', 'notify']
const WHATS = ['recordSet', 'upsert', 'setMarkdown'] as const
const ARGS_TEMPLATE: Record<(typeof WHATS)[number], Json> = { recordSet: { props: {} }, upsert: { rows: [], key: [] }, setMarkdown: { markdown: '' } }

function blank(kind: Kind, target: string): EventHandler {
  switch (kind) {
    case 'write': return { kind, target, what: 'recordSet', args: { props: {} } }
    case 'wish': return { kind, wish_id: '' }
    case 'open': return { kind }
    case 'notify': return { kind, text: '' }
  }
}

export function EventsForm({ def, instance, on, bindings, hasSource, readOnly, onSave }: {
  def: BlockDefPayload
  instance: EntityEnvelope
  on: EventBindings
  /** The instance's binding names (`source` is added when the Cell has a main source). */
  bindings: string[]
  hasSource: boolean
  readOnly: string | null
  onSave: (next: EventBindings) => void
}) {
  const { t } = useAiwsText()
  const declared = Object.entries(def.events ?? {})
  const stale = Object.keys(on).filter((name) => !(name in (def.events ?? {})))
  const targets = [...(hasSource ? ['source'] : []), ...bindings.filter((b) => b !== 'source')]
  const save = (name: string, handlers: EventHandler[] | null) => {
    const next: EventBindings = { ...on }
    if (!handlers || handlers.length === 0) delete next[name]
    else next[name] = handlers.length === 1 ? handlers[0] : handlers
    onSave(next)
  }
  return (
    <div className="aiws-events-form" data-testid="aiws-events-form" data-instance={instance.entity_id}>
      {declared.length === 0 && <div className="aiws-muted">{t('aiworkspace.ext.events.none')}</div>}
      {declared.map(([name, event]) => (
        <EventRow key={`${name}:${JSON.stringify(on[name] ?? null)}`} name={name} title={event.title ?? name} description={event.description}
          params={Object.keys(event.params ?? {})} handlers={handlersOf(on[name])} targets={targets} readOnly={readOnly} onSave={(handlers) => save(name, handlers)} />
      ))}
      {stale.map((name) => (
        <div key={name} className="aiws-warning" data-testid={`aiws-events-stale-${name}`}>
          {t('aiworkspace.ext.events.stale', { name })}{' '}
          <button type="button" className="aiws-link" disabled={readOnly !== null} onClick={() => save(name, null)}>{t('aiworkspace.ext.events.remove')}</button>
        </div>
      ))}
    </div>
  )
}

function EventRow({ name, title, description, params, handlers: initial, targets, readOnly, onSave }: {
  name: string
  title: string
  description?: string
  params: string[]
  handlers: EventHandler[]
  targets: string[]
  readOnly: string | null
  onSave: (handlers: EventHandler[]) => void
}) {
  const { t } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const [rows, setRows] = useState<EventHandler[]>(initial)
  // the args of a write as typed (JSON), kept per row until they parse
  const [texts, setTexts] = useState<Record<number, string>>({})
  const [error, setError] = useState<string | null>(null)
  const wishes = store.outline.all().filter((e) => !e.deleted && e.type_id === 'buckyos.wish')
  const dirty = JSON.stringify(rows) !== JSON.stringify(initial) || Object.keys(texts).length > 0
  const set = (index: number, handler: EventHandler) => setRows((prev) => prev.map((h, i) => (i === index ? handler : h)))
  const commit = () => {
    const next = rows.map((h, i) => {
      if (h.kind !== 'write' || texts[i] === undefined) return h
      return { ...h, args: JSON.parse(texts[i]) as Record<string, Json> }
    })
    for (const h of next) {
      if (h.kind === 'write' && !h.target) throw new Error(t('aiworkspace.ext.events.needsTarget'))
      if (h.kind === 'wish' && !h.wish_id) throw new Error(t('aiworkspace.ext.events.needsWish'))
    }
    setTexts({})
    onSave(next)
  }
  return (
    <fieldset className="aiws-events-row" data-testid={`aiws-events-row-${name}`} disabled={readOnly !== null}>
      <legend><b>{title}</b> <code>{name}</code></legend>
      {description && <div className="aiws-muted">{description}</div>}
      {params.length > 0 && <div className="aiws-muted">{t('aiworkspace.ext.events.params', { names: params.map((p) => `{{event.${p}}}`).join(' ') })}</div>}
      {rows.map((h, i) => (
        <div key={i} className="aiws-events-handler" data-testid={`aiws-events-handler-${name}-${i}`}>
          <select aria-label={t('aiworkspace.ext.events.kind')} value={h.kind} data-testid={`aiws-events-kind-${name}-${i}`} onChange={(e) => { setTexts((prev) => { const next = { ...prev }; delete next[i]; return next }); set(i, blank(e.target.value as Kind, targets[0] ?? '')) }}>
            {KINDS.map((k) => <option key={k} value={k}>{kindLabel(k, t)}</option>)}
          </select>
          {h.kind === 'write' && <>
            <select aria-label={t('aiworkspace.ext.events.target')} value={h.target} data-testid={`aiws-events-target-${name}-${i}`} onChange={(e) => set(i, { ...h, target: e.target.value })}>
              {!targets.includes(h.target) && <option value={h.target}>{h.target || t('aiworkspace.ext.events.chooseTarget')}</option>}
              {targets.map((b) => <option key={b} value={b}>{b}</option>)}
            </select>
            <select aria-label={t('aiworkspace.ext.events.what')} value={h.what} data-testid={`aiws-events-what-${name}-${i}`} onChange={(e) => { const what = e.target.value as (typeof WHATS)[number]; setTexts((prev) => ({ ...prev, [i]: JSON.stringify(ARGS_TEMPLATE[what]) })); set(i, { ...h, what }) }}>
              {WHATS.map((w) => <option key={w} value={w}>{w}</option>)}
            </select>
            <textarea aria-label={t('aiworkspace.ext.events.args')} rows={2} className="aiws-events-args" data-testid={`aiws-events-args-${name}-${i}`} value={texts[i] ?? JSON.stringify(h.args ?? {})} onChange={(e) => setTexts((prev) => ({ ...prev, [i]: e.target.value }))} />
          </>}
          {h.kind === 'wish' && (
            <select aria-label={t('aiworkspace.ext.events.wish')} value={h.wish_id} data-testid={`aiws-events-wish-${name}-${i}`} onChange={(e) => set(i, { ...h, wish_id: e.target.value })}>
              <option value="">{t('aiworkspace.ext.events.chooseWish')}</option>
              {wishes.map((w) => <option key={w.entity_id} value={w.entity_id}>{w.title ?? w.name ?? w.entity_id}</option>)}
            </select>
          )}
          {h.kind === 'open' && (
            <select aria-label={t('aiworkspace.ext.events.target')} value={h.target ?? ''} data-testid={`aiws-events-target-${name}-${i}`} onChange={(e) => set(i, e.target.value ? { ...h, target: e.target.value } : { kind: 'open' })}>
              <option value="">{t('aiworkspace.ext.events.openSource')}</option>
              {targets.filter((b) => b !== 'source').map((b) => <option key={b} value={b}>{b}</option>)}
            </select>
          )}
          {h.kind === 'notify' && <input aria-label={t('aiworkspace.ext.events.text')} value={h.text} data-testid={`aiws-events-text-${name}-${i}`} onChange={(e) => set(i, { ...h, text: e.target.value })} />}
          <button type="button" className="aiws-link" data-testid={`aiws-events-drop-${name}-${i}`} onClick={() => { setTexts({}); setRows((prev) => prev.filter((_, j) => j !== i)) }}>{t('aiworkspace.ext.events.remove')}</button>
        </div>
      ))}
      <div className="aiws-dialog-actions">
        <button type="button" data-testid={`aiws-events-add-${name}`} disabled={rows.length >= MAX_HANDLERS} onClick={() => setRows((prev) => [...prev, blank(targets.length > 0 ? 'write' : 'notify', targets[0] ?? '')])}>{t('aiworkspace.ext.events.add')}</button>
        <button type="button" className="is-primary" data-testid={`aiws-events-save-${name}`} disabled={!dirty} onClick={() => { setError(null); try { commit() } catch (e) { setError(e instanceof SyntaxError ? t('aiworkspace.ext.events.badArgs', { error: e.message }) : e instanceof Error ? e.message : String(e)) } }}>{t('aiworkspace.common.save')}</button>
      </div>
      {error && <div className="aiws-error" role="alert">{error}</div>}
    </fieldset>
  )
}

function kindLabel(kind: Kind, t: (key: string) => string): string {
  switch (kind) {
    case 'write': return t('aiworkspace.ext.events.kindWrite')
    case 'wish': return t('aiworkspace.ext.events.kindWish')
    case 'open': return t('aiworkspace.ext.events.kindOpen')
    case 'notify': return t('aiworkspace.ext.events.kindNotify')
  }
}
