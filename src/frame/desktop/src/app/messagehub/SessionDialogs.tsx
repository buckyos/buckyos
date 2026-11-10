import { useEffect, useRef, useState, type ReactNode, type FormEvent } from 'react'
import { useForm, useWatch } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import type { z } from 'zod'
import { useI18n } from '../../i18n/provider'
import { creationReason, newSessionFormSchema } from './sessionModel'
import { groupErrorText } from './groupModel'
import { useMessageHubStore } from './store'
import type { MessageHubContext, Session } from './types'

export const hubInputClass = 'mt-1 min-h-11 w-full rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3 py-2 text-sm'
export const hubButtonClass = 'min-h-11 rounded-lg border border-[color:var(--cp-border)] px-3 py-2 text-sm disabled:opacity-40'
export const hubPrimaryButtonClass = 'min-h-11 rounded-lg border border-transparent bg-[color:var(--cp-accent)] px-4 py-2 text-sm font-semibold text-white disabled:opacity-40'
export const hubIconButtonClass = 'flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-[color:color-mix(in_srgb,var(--cp-text)_7%,transparent)] text-[color:var(--cp-muted)] transition-colors hover:text-[color:var(--cp-text)] disabled:opacity-40 md:h-9 md:w-9'

export function DialogFocus({ children, onCancel }: { children: ReactNode; onCancel: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const root = ref.current?.closest('[role="dialog"]')
    if (!(root instanceof HTMLElement)) return
    const buttons = () => [...root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled)')]
    buttons().find(element => element.hasAttribute('data-autofocus'))?.focus()
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); onCancel() }
      if (event.key !== 'Tab') return
      const items = buttons(), first = items[0], last = items.at(-1)
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
    }
    root.addEventListener('keydown', key)
    return () => root.removeEventListener('keydown', key)
  }, [onCancel])
  return <div ref={ref}>{children}</div>
}

/** A new session of one entity: its name, an optional description and the connection it runs over. */
export function CreateSessionForm({ context, entityId, onCreated, onCancel }: { context: MessageHubContext; entityId: string; onCreated: (session: Session) => void; onCancel: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const entity = store.findEntity(context, entityId)
  const choices = store.connections(context, entityId)
  const reasonFor = (connection: string) => entity ? creationReason(context, entity, store.policy(context, entityId), choices.find(choice => choice.id === connection)?.binding) : 'binding_unknown'
  // The native connection is the default; another one only when the native one cannot create a session here.
  const preferred = choices.find(choice => choice.binding.kind === 'native' && !reasonFor(choice.id)) ?? choices.find(choice => !reasonFor(choice.id)) ?? choices.find(choice => choice.binding.kind === 'native') ?? choices[0]
  const form = useForm<z.infer<typeof newSessionFormSchema>>({ resolver: zodResolver(newSessionFormSchema), defaultValues: { title: '', description: '', connection: preferred?.id ?? '' } })
  const connection = useWatch({ control: form.control, name: 'connection' })
  const reason = reasonFor(connection)
  const [error, setError] = useState('')
  const busy = useRef(false)
  const submit = (event: FormEvent<HTMLFormElement>) => { void form.handleSubmit(async values => {
    if (busy.current) return
    busy.current = true; setError('')
    try { onCreated(await store.create(context, { entityId, ...values })) } catch (failure) { setError(failure instanceof Error && failure.message.startsWith('rejected') ? groupErrorText(t, failure) : t('messagehub.operationFailed')) } finally { busy.current = false }
  })(event) }
  return <DialogFocus onCancel={onCancel}><form onSubmit={submit} className="space-y-4" data-testid="new-session">
    <label className="block text-sm">{t('messagehub.sessionName')}<input data-autofocus className={hubInputClass} autoComplete="off" {...form.register('title')} /></label>
    {form.formState.errors.title && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{t('messagehub.sessionNameRule')}</p>}
    <label className="block text-sm">{t('messagehub.optionalDescription')}<textarea rows={2} className={hubInputClass} {...form.register('description')} /></label>
    {form.formState.errors.description && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{t('messagehub.fieldLimit')}</p>}
    <label className="block text-sm">{t('messagehub.connection')}<select className={hubInputClass} {...form.register('connection')}>{choices.map(choice => <option value={choice.id} key={choice.id}>{choice.label}</option>)}</select></label>
    {reason && <p className="text-sm text-[color:var(--cp-muted)]">{t(`messagehub.reason.${reason}`)}</p>}
    {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
    <div className="flex justify-end gap-2"><button type="button" className={hubButtonClass} onClick={onCancel}>{t('messagehub.cancel')}</button><button type="submit" className={hubPrimaryButtonClass} disabled={form.formState.isSubmitting || !!reason}>{t(form.formState.isSubmitting ? 'messagehub.creating' : 'messagehub.create')}</button></div>
  </form></DialogFocus>
}

export function ManageSessionForm({ context, session, onDone, onCancel }: { context: MessageHubContext; session: Session; onDone: (action: 'archive' | 'restore' | 'delete') => void; onCancel: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  const busy = useRef(false)
  const act = async (action: 'archive' | 'restore' | 'delete') => {
    if (busy.current) return
    busy.current = true; setPending(true); setError('')
    try { await store.manage(context, session.id, action); onDone(action) } catch { setError(t('messagehub.operationFailed')) } finally { setPending(false); busy.current = false }
  }
  return <DialogFocus onCancel={() => { if (!busy.current) onCancel() }}><div className="space-y-4">
    <p className="text-sm">{t('messagehub.deleteExplanation')}</p>
    <p className="text-sm text-[color:var(--cp-muted)]">{t('messagehub.archiveExplanation')}</p>
    {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
    <div className="flex flex-wrap justify-end gap-2">
      <button data-autofocus type="button" disabled={pending} className={hubButtonClass} onClick={onCancel}>{t('messagehub.cancel')}</button>
      <button type="button" disabled={pending} className={hubButtonClass} onClick={() => void act(session.lifecycle === 'archived' ? 'restore' : 'archive')}>{t(pending ? 'messagehub.saving' : session.lifecycle === 'archived' ? 'messagehub.restore' : 'messagehub.archive')}</button>
      <button type="button" disabled={pending} className={`${hubButtonClass} text-[color:var(--cp-danger)]`} onClick={() => void act('delete')}>{t('messagehub.deletePermanently')}</button>
    </div>
  </div></DialogFocus>
}
