import { useRef, useState } from 'react'
import { X } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import type { z } from 'zod'
import { useI18n } from '../../i18n/provider'
import { useWindowDialog } from '../../desktop/windows/dialogs'
import { useMessageHubStore } from './mock/hooks'
import { DialogFocus, hubButtonClass, hubInputClass } from './SessionDialogs'
import { memberStateSchema, presentationSchema, sharedStateSchema } from './sessionModel'
import type { Entity, MessageHubContext, Session, SessionAccess } from './types'

export function SessionDetails({ session, entity, context, access, onClose, onManage, onWrite }: { session: Session; entity: Entity; context: MessageHubContext; access: SessionAccess; onClose: () => void; onManage: () => void; onWrite: (enabled: boolean) => void }) {
  const { t } = useI18n(), store = useMessageHubStore(), dialog = useWindowDialog()
  const enableWrite = async () => {
    const trigger = document.activeElement
    const result = await dialog.open<boolean>({ title: t('messagehub.enableWrite'), dismissible: false, size: 'sm', renderBody: controls => <DialogFocus onCancel={() => controls.close(false)}><p className="text-sm">{t('messagehub.writeWarning')}</p><div className="mt-4 flex justify-end gap-2"><button data-autofocus type="button" className={hubButtonClass} onClick={() => controls.close(false)}>{t('messagehub.cancel')}</button><button type="button" className={hubButtonClass} onClick={() => controls.close(true)}>{t('messagehub.enableWrite')}</button></div></DialogFocus> })
    if (result) onWrite(true)
    if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus()
  }
  const rows = [
    [t('messagehub.sessionType'), t(`messagehub.type.${session.type}`)],
    [t('messagehub.targetEntity'), entity.name],
    [t('messagehub.owner'), context.ownerDid === context.viewerDid ? t('messagehub.you') : t('messagehub.agentOwner')],
    [t('messagehub.connection'), session.binding.kind === 'tunnel' ? session.binding.connectionName : session.source ?? 'BuckyOS'],
    [t('messagehub.mode'), t(access.mode === 'read_write' ? 'messagehub.readWrite' : 'messagehub.readOnly')],
    [t('messagehub.createdAt'), new Date(session.createdAt).toLocaleString()],
    [t('messagehub.lastActivity'), session.lastActiveAt ? new Date(session.lastActiveAt).toLocaleString() : '—'],
    [t('messagehub.lifecycle'), t(session.lifecycle === 'active' ? 'messagehub.activeSessions' : 'messagehub.archived')],
  ]
  return <section className="flex h-full flex-col bg-[color:var(--cp-surface)]" data-testid="session-details">
    <header className="flex shrink-0 items-center justify-between border-b border-[color:var(--cp-border)] px-4 py-2"><h2 className="text-sm font-semibold">{t('messagehub.sessionDetails')}</h2><button type="button" className="min-h-11 min-w-11" aria-label={t('messagehub.close')} onClick={onClose}><X size={18} /></button></header>
    <div className="shell-scrollbar flex-1 overflow-y-auto p-4 text-sm space-y-5">
      <h3 className="break-words text-lg font-semibold">{store.title(context, session)}</h3>
      <dl className="space-y-3">{rows.map(([label, value]) => <div key={label}><dt className="text-xs text-[color:var(--cp-muted)]">{label}</dt><dd>{value}</dd></div>)}</dl>
      {access.readOnlyReason && <p>{t(`messagehub.reason.${access.readOnlyReason}`)}</p>}
      {!access.canManage && <p>{t('messagehub.reason.agent_observer')}</p>}
      <SharedEditor session={session} context={context} disabled={!access.canEditSharedState} />
      <MemberEditor session={session} context={context} disabled={!access.canEditOwnMemberState} />
      <div><h4 className="font-semibold">{t('messagehub.otherMembers')}</h4>{Object.entries(session.members).filter(([did]) => did !== context.ownerDid).map(([did, member]) => <p key={did} className="mt-1 break-words">{member.nickname || did}</p>)}</div>
      <PresentationEditor session={session} context={context} disabled={!access.canEditPresentation} />
      <details><summary className="cursor-pointer">{t('messagehub.sourceInfo')}</summary><dl className="mt-2 break-all text-xs space-y-2">{Object.entries({ sessionId: session.id, ownerDid: session.ownerDid, entityDid: entity.id, origin: session.origin, ...session.binding }).map(([key, value]) => <div key={key}><dt>{key}</dt><dd>{String(value)}</dd></div>)}</dl></details>
      <div className="flex flex-wrap gap-2">
        {access.canEnableWrite && <button type="button" className={hubButtonClass} onClick={() => void enableWrite()}>{t('messagehub.enableWrite')}</button>}
        {session.binding.kind === 'tunnel' && access.mode === 'read_write' && <button type="button" className={hubButtonClass} onClick={() => onWrite(false)}>{t('messagehub.restoreReadOnly')}</button>}
        <button type="button" className={hubButtonClass} disabled={!access.canManage} onClick={onManage}>{t('messagehub.manageSession')}</button>
      </div>
    </div>
  </section>
}

function SharedEditor({ session, context, disabled }: { session: Session; context: MessageHubContext; disabled: boolean }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const form = useForm<z.infer<typeof sharedStateSchema>>({ resolver: zodResolver(sharedStateSchema), values: { title: session.shared.title, description: session.shared.description } })
  const action = useSaveAction()
  return <form className="space-y-2" onSubmit={form.handleSubmit(values => action.save(() => store.updateState(context, session.id, 'shared', { title: values.title, description: values.description })))}><h4 className="font-semibold">{t('messagehub.sharedState')}</h4><fieldset disabled={disabled || action.pending} className="space-y-2 disabled:opacity-60">
    <label className="block">{t('messagehub.sharedTitle')}<input className={hubInputClass} {...form.register('title')} /></label>
    <label className="block">{t('messagehub.description')}<textarea className={hubInputClass} {...form.register('description')} /></label>
    {Object.keys(form.formState.errors).length > 0 && <p role="alert">{t('messagehub.fieldLimit')}</p>}
    <button type="submit" className={hubButtonClass}>{t(action.pending ? 'messagehub.saving' : 'messagehub.save')}</button>
  </fieldset><SaveStatus status={action.status} />{disabled && <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.stateReadOnly')}</p>}</form>
}
function MemberEditor({ session, context, disabled }: { session: Session; context: MessageHubContext; disabled: boolean }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const form = useForm<z.infer<typeof memberStateSchema>>({ resolver: zodResolver(memberStateSchema), values: { nickname: session.members[context.ownerDid]?.nickname ?? '' } })
  const action = useSaveAction()
  return <form className="space-y-2" onSubmit={form.handleSubmit(values => action.save(() => store.updateState(context, session.id, 'member', values)))}><h4 className="font-semibold">{t('messagehub.myMemberState')}</h4><fieldset disabled={disabled || action.pending} className="space-y-2 disabled:opacity-60"><label className="block">{t('messagehub.nickname')}<input className={hubInputClass} {...form.register('nickname')} /></label>{form.formState.errors.nickname && <p role="alert">{t('messagehub.titleLimit')}</p>}<button type="submit" className={hubButtonClass}>{t(action.pending ? 'messagehub.saving' : 'messagehub.save')}</button></fieldset><SaveStatus status={action.status} /></form>
}
function PresentationEditor({ session, context, disabled }: { session: Session; context: MessageHubContext; disabled: boolean }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const form = useForm<z.infer<typeof presentationSchema>>({ resolver: zodResolver(presentationSchema), values: store.preferences(context, session.id) })
  const action = useSaveAction()
  return <form className="space-y-2" onSubmit={form.handleSubmit(values => action.save(() => store.updatePreferences(context, session.id, values)))}><h4 className="font-semibold">{t('messagehub.personalPreferences')}</h4><fieldset disabled={disabled || action.pending} className="space-y-2 disabled:opacity-60"><label className="block">{t('messagehub.personalTitle')}<input className={hubInputClass} {...form.register('title')} /></label>{form.formState.errors.title && <p role="alert">{t('messagehub.titleLimit')}</p>}<label className="flex min-h-11 items-center gap-2"><input type="checkbox" {...form.register('pinned')} />{t('messagehub.pin')}</label><label className="flex min-h-11 items-center gap-2"><input type="checkbox" {...form.register('muted')} />{t('messagehub.mute')}</label><button type="submit" className={hubButtonClass}>{t(action.pending ? 'messagehub.saving' : 'messagehub.save')}</button></fieldset><SaveStatus status={action.status} /></form>
}
function useSaveAction() {
  const [status, setStatus] = useState(''), [pending, setPending] = useState(false), busy = useRef(false)
  const save = async (operation: () => Promise<unknown>) => {
    if (busy.current) return
    busy.current = true; setPending(true); setStatus('')
    try { await operation(); setStatus('saved') } catch { setStatus('operationFailed') } finally { busy.current = false; setPending(false) }
  }
  return { status, pending, save }
}
function SaveStatus({ status }: { status: string }) {
  const { t } = useI18n()
  return status ? <p role={status === 'saved' ? 'status' : 'alert'} className="text-xs">{t(`messagehub.${status}`)}</p> : null
}
