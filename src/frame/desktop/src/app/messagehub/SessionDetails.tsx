import { useRef, useState, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import type { z } from 'zod'
import { useI18n } from '../../i18n/provider'
import { useWindowDialog } from '../../desktop/windows/dialogs'
import { useMessageHubStore } from './store'
import { DialogFocus, hubButtonClass, hubInputClass, hubPrimaryButtonClass } from './SessionDialogs'
import { memberStateSchema, presentationSchema, sharedStateSchema } from './sessionModel'
import type { Entity, MessageHubContext, Session, SessionAccess } from './types'

export function SessionDetails({ session, entity, context, access, showActions, onShowActions, onClose, onManage, onWrite }: { session: Session; entity: Entity; context: MessageHubContext; access: SessionAccess; showActions: boolean; onShowActions: (value: boolean) => Promise<void>; onClose: () => void; onManage: () => void; onWrite: (enabled: boolean) => void }) {
  const { t } = useI18n(), store = useMessageHubStore(), dialog = useWindowDialog()
  const enableWrite = async () => {
    const trigger = document.activeElement
    const result = await dialog.open<boolean>({ title: t('messagehub.enableWrite'), dismissible: false, size: 'sm', renderBody: controls => <DialogFocus onCancel={() => controls.close(false)}><p className="text-sm">{t('messagehub.writeWarning')}</p><div className="mt-4 flex justify-end gap-2"><button data-autofocus type="button" className={hubButtonClass} onClick={() => controls.close(false)}>{t('messagehub.cancel')}</button><button type="button" className={hubPrimaryButtonClass} onClick={() => controls.close(true)}>{t('messagehub.enableWrite')}</button></div></DialogFocus> })
    if (result) onWrite(true)
    if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus()
  }
  const connection = session.binding.kind === 'tunnel' ? session.binding.connectionName : session.source ?? 'BuckyOS'
  const rows = [
    [t('messagehub.sessionType'), t(`messagehub.type.${session.type}`)],
    [t('messagehub.targetEntity'), entity.id],
    [t('messagehub.owner'), context.ownerDid === context.viewerDid ? t('messagehub.you') : t('messagehub.agentOwner')],
    [t('messagehub.connection'), connection],
    [t('messagehub.mode'), t(access.mode === 'read_write' ? 'messagehub.readWrite' : 'messagehub.readOnly')],
    [t('messagehub.createdAt'), session.createdAt ? new Date(session.createdAt).toLocaleString() : '—'],
    [t('messagehub.lastActivity'), session.lastActiveAt ? new Date(session.lastActiveAt).toLocaleString() : '—'],
    ...Object.entries({ sessionId: session.id, ownerDid: session.ownerDid, origin: session.origin, attribution: session.attributionEvidence ?? 'seed', ...session.binding }).map(([key, value]) => [key, String(value)]),
  ]
  const notices = [
    access.readOnlyReason ? t(`messagehub.reason.${access.readOnlyReason}`) : '',
    !access.canManage ? t('messagehub.reason.agent_observer') : '',
    (session.requestCount ?? 0) > 0 ? t('messagehub.requestBanner', undefined, { count: session.requestCount ?? 0 }) : '',
  ].filter(Boolean)
  const members = Object.entries(session.members)
  const sharedEditors = <div className="space-y-5"><SharedEditor session={session} context={context} disabled={!access.canEditSharedState} /><MemberEditor session={session} context={context} disabled={!access.canEditOwnMemberState} /></div>
  return <section className="flex h-full flex-col bg-[color:var(--cp-surface)]" data-testid="session-details">
    <header className="flex shrink-0 items-center justify-between border-b border-[color:var(--cp-border)] py-1 pl-4 pr-1"><h2 className="text-sm font-semibold">{t('messagehub.sessionDetails')}</h2><button type="button" className="flex min-h-11 min-w-11 items-center justify-center" aria-label={t('messagehub.close')} title={t('messagehub.close')} onClick={onClose}><X size={18} /></button></header>
    <div className="shell-scrollbar flex-1 space-y-6 overflow-y-auto p-4 text-sm">
      <div>
        <h3 className="break-words text-lg font-semibold leading-snug">{store.title(context, session)}</h3>
        <p className="mt-1 break-words text-[13px] text-[color:var(--cp-muted)]">{entity.name} · {t(`messagehub.type.${session.type}`)}</p>
        <div className="mt-2 flex flex-wrap gap-1.5 empty:hidden">
          {session.lifecycle === 'archived' && <DetailBadge>{t('messagehub.archived')}</DetailBadge>}
          {access.mode !== 'read_write' && <DetailBadge>{t('messagehub.readOnly')}</DetailBadge>}
          {session.binding.kind === 'tunnel' && <DetailBadge>{session.binding.connectionName}</DetailBadge>}
        </div>
      </div>
      {notices.length > 0 && <div className="space-y-1 rounded-xl bg-[color:color-mix(in_srgb,var(--cp-warning)_10%,transparent)] px-3 py-2 text-[13px]">{notices.map(notice => <p key={notice}>{notice}</p>)}</div>}
      <DetailSection title={t('messagehub.members')}>
        <ul className="space-y-1.5">{members.map(([did, member]) => <li key={did} className="flex min-w-0 items-baseline gap-2" title={did}><span className="truncate">{did === context.ownerDid ? t('messagehub.you') : member.nickname || (did === entity.id ? entity.name : did)}</span>{did === context.ownerDid && member.nickname ? <span className="truncate text-xs text-[color:var(--cp-muted)]">{member.nickname}</span> : null}</li>)}</ul>
      </DetailSection>
      <DetailSection title={t('messagehub.preferences')}>
        <ShowActionsToggle value={showActions} onChange={onShowActions} />
        <PresentationEditor session={session} context={context} disabled={!access.canEditPresentation} />
      </DetailSection>
      <DetailSection title={t('messagehub.sharedState')}>
        {!store.isMock ? <p className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.sharedStateUnavailable')}</p>
          : !access.canEditSharedState && !access.canEditOwnMemberState ? <details><summary className="flex min-h-11 cursor-pointer items-center text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.stateReadOnly')}</summary>{sharedEditors}</details>
          : sharedEditors}
      </DetailSection>
      <div className="flex flex-wrap gap-2">
        {access.canEnableWrite && <button type="button" className={hubButtonClass} onClick={() => void enableWrite()}>{t('messagehub.enableWrite')}</button>}
        {session.binding.kind === 'tunnel' && access.mode === 'read_write' && <button type="button" className={hubButtonClass} onClick={() => onWrite(false)}>{t('messagehub.restoreReadOnly')}</button>}
        <button type="button" className={hubButtonClass} disabled={!access.canManage} onClick={onManage}>{t('messagehub.manageSession')}</button>
      </div>
      <details className="rounded-xl border border-[color:var(--cp-border)] px-3">
        <summary className="flex min-h-11 cursor-pointer items-center text-[13px] font-medium text-[color:var(--cp-muted)]">{t('messagehub.technicalInfo')}</summary>
        <dl className="space-y-2 pb-3 text-xs">{rows.map(([label, value]) => <div key={label}><dt className="text-[color:var(--cp-muted)]">{label}</dt><dd className="break-all">{value}</dd></div>)}</dl>
      </details>
    </div>
  </section>
}

function DetailSection({ title, children }: { title: string; children: ReactNode }) {
  return <section className="space-y-3"><h4 className="text-xs font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]">{title}</h4>{children}</section>
}

function DetailBadge({ children }: { children: ReactNode }) {
  return <span className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-text)_7%,transparent)] px-2 py-0.5 text-xs text-[color:var(--cp-muted)]">{children}</span>
}

function ShowActionsToggle({ value, onChange }: { value: boolean; onChange: (value: boolean) => Promise<void> }) {
  const { t } = useI18n()
  const [pending, setPending] = useState<boolean | null>(null), [failed, setFailed] = useState(false)
  return <div>
    <label className="flex min-h-11 items-center gap-2"><input type="checkbox" checked={pending ?? value} disabled={pending !== null} onChange={event => { const next = event.target.checked; setPending(next); setFailed(false); void onChange(next).catch(() => setFailed(true)).finally(() => setPending(null)) }} />{t('messagehub.showActions')}</label>
    {failed && <p role="alert" className="text-xs text-[color:var(--cp-danger)]">{t('messagehub.operationFailed')}</p>}
  </div>
}

function SharedEditor({ session, context, disabled }: { session: Session; context: MessageHubContext; disabled: boolean }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const form = useForm<z.infer<typeof sharedStateSchema>>({ resolver: zodResolver(sharedStateSchema), values: { title: session.shared.title, description: session.shared.description } })
  const action = useSaveAction()
  return <form className="space-y-2" onSubmit={form.handleSubmit(values => action.save(() => store.updateState(context, session.id, 'shared', { title: values.title, description: values.description })))}><fieldset disabled={disabled || action.pending} className="space-y-2 disabled:opacity-60">
    <label className="block">{t('messagehub.sharedTitle')}<input className={hubInputClass} {...form.register('title')} /></label>
    <label className="block">{t('messagehub.description')}<textarea className={hubInputClass} {...form.register('description')} /></label>
    {Object.keys(form.formState.errors).length > 0 && <p role="alert">{t('messagehub.fieldLimit')}</p>}
    <button type="submit" className={hubButtonClass}>{t(action.pending ? 'messagehub.saving' : 'messagehub.save')}</button>
  </fieldset><SaveStatus status={action.status} /></form>
}
function MemberEditor({ session, context, disabled }: { session: Session; context: MessageHubContext; disabled: boolean }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const form = useForm<z.infer<typeof memberStateSchema>>({ resolver: zodResolver(memberStateSchema), values: { nickname: session.members[context.ownerDid]?.nickname ?? '' } })
  const action = useSaveAction()
  return <form className="space-y-2" onSubmit={form.handleSubmit(values => action.save(() => store.updateState(context, session.id, 'member', values)))}><h5 className="text-[13px] font-medium">{t('messagehub.myMemberState')}</h5><fieldset disabled={disabled || action.pending} className="space-y-2 disabled:opacity-60"><label className="block">{t('messagehub.nickname')}<input className={hubInputClass} {...form.register('nickname')} /></label>{form.formState.errors.nickname && <p role="alert">{t('messagehub.titleLimit')}</p>}<button type="submit" className={hubButtonClass}>{t(action.pending ? 'messagehub.saving' : 'messagehub.save')}</button></fieldset><SaveStatus status={action.status} /></form>
}
function PresentationEditor({ session, context, disabled }: { session: Session; context: MessageHubContext; disabled: boolean }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const form = useForm<z.infer<typeof presentationSchema>>({ resolver: zodResolver(presentationSchema), values: store.preferences(context, session.id) })
  const action = useSaveAction()
  return <form className="space-y-2" onSubmit={form.handleSubmit(values => action.save(() => store.updatePreferences(context, session.id, values)))}><fieldset disabled={disabled || action.pending} className="space-y-2 disabled:opacity-60"><label className="block">{t('messagehub.personalTitle')}<input className={hubInputClass} {...form.register('title')} /></label>{form.formState.errors.title && <p role="alert">{t('messagehub.titleLimit')}</p>}<label className="flex min-h-11 items-center gap-2"><input type="checkbox" {...form.register('pinned')} />{t('messagehub.pin')}</label><label className="flex min-h-11 items-center gap-2"><input type="checkbox" {...form.register('muted')} />{t('messagehub.mute')}</label><button type="submit" className={hubButtonClass}>{t(action.pending ? 'messagehub.saving' : 'messagehub.save')}</button></fieldset><SaveStatus status={action.status} /></form>
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
