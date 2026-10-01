import { useEffect, useState } from 'react'
import { Bot, Crown, LogOut, Shield, Trash2, User, UserMinus, UserPlus } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useWindowDialog, type WindowDialogApi } from '../../desktop/windows/dialogs'
import { friendlyDidName } from './api/projection'
import { InviteMembersForm } from './GroupDialogs'
import { groupErrorText, participating, sortGroupMembers } from './groupModel'
import { DialogFocus, hubButtonClass } from './SessionDialogs'
import { useMessageHubStore } from './store'
import type { Entity, GroupMember, MessageHubContext } from './types'

/** Confirmation for destructive group actions; resolves true when confirmed. */
async function confirmGroupAction(dialog: WindowDialogApi, t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string, options: { title: string; body: string; confirm: string }) {
  const trigger = document.activeElement
  const result = await dialog.open<boolean>({ title: options.title, size: 'sm', dismissible: false, renderBody: controls => <DialogFocus onCancel={() => controls.close(false)}>
    <p className="text-sm">{options.body}</p>
    <div className="mt-4 flex justify-end gap-2">
      <button data-autofocus type="button" className={hubButtonClass} onClick={() => controls.close(false)}>{t('messagehub.cancel')}</button>
      <button type="button" className={`${hubButtonClass} font-semibold text-[color:var(--cp-danger)]`} onClick={() => controls.close(true)}>{options.confirm}</button>
    </div>
  </DialogFocus> })
  if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus()
  return result === true
}

/** Membership and management of a self-host group, shown in the group's entity details. */
export function GroupPanel({ entity, context }: { entity: Entity; context: MessageHubContext }) {
  const { t } = useI18n(), store = useMessageHubStore(), dialog = useWindowDialog()
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  useEffect(() => { void store.ensureGroup(context, entity.id) }, [store, context.ownerDid, context.viewerDid, context.mode, entity.id]) // eslint-disable-line react-hooks/exhaustive-deps
  const group = store.group(context, entity.id)
  const status = store.groupStatus(context, entity.id)
  if (!group) return status === 'loading' || status === 'idle' ? <p role="status" className="mb-4 text-sm text-[color:var(--cp-muted)]">{t('messagehub.group.loading')}</p> : null
  const run = async (operation: () => Promise<unknown>) => {
    if (pending) return
    setPending(true); setError('')
    try { await operation() } catch (failure) { setError(groupErrorText(t, failure)) } finally { setPending(false) }
  }
  const nameOf = (did: string) => did === context.ownerDid ? t('messagehub.you') : store.findEntity(context, did)?.name ?? friendlyDidName(did, false)
  const members = sortGroupMembers((group.members ?? []).filter(participating))
  const activeCount = members.filter(member => member.state === 'active').length
  const waitingCount = members.length - activeCount
  const openInvite = () => {
    const trigger = document.activeElement
    void dialog.open({ title: t('messagehub.group.inviteTitle', undefined, { name: group.name || entity.name }), size: 'md', dismissible: false, renderBody: controls => <InviteMembersForm context={context} group={group} onCancel={() => controls.close()} onDone={() => controls.close()} /> })
      .then(() => { if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus() })
  }
  const remove = async (member: GroupMember) => {
    const name = nameOf(member.did)
    const invited = member.state !== 'active'
    if (!await confirmGroupAction(dialog, t, { title: t(invited ? 'messagehub.group.cancelInvite' : 'messagehub.group.remove'), body: t(invited ? 'messagehub.group.cancelInviteConfirm' : 'messagehub.group.removeConfirm', undefined, { name }), confirm: t(invited ? 'messagehub.group.cancelInvite' : 'messagehub.group.remove') })) return
    await run(() => store.removeGroupMember(context, group.did, member.did))
  }
  const leave = async () => {
    if (!await confirmGroupAction(dialog, t, { title: t('messagehub.group.leave'), body: t('messagehub.group.leaveConfirm', undefined, { name: group.name || entity.name }), confirm: t('messagehub.group.leave') })) return
    await run(() => store.leaveGroup(context, group.did))
  }
  const disband = async () => {
    if (!await confirmGroupAction(dialog, t, { title: t('messagehub.group.delete'), body: t('messagehub.group.deleteConfirm', undefined, { name: group.name || entity.name }), confirm: t('messagehub.group.delete') })) return
    await run(() => store.deleteGroup(context, group.did))
  }
  return <section className="mb-4 rounded-xl p-3" style={{ background: 'color-mix(in srgb, var(--cp-text) 4%, transparent)' }} data-testid="group-panel" aria-label={t('messagehub.group.membersTitle')}>
    <div className="flex items-center justify-between gap-2">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]">{t('messagehub.group.membersTitle')}{group.members ? ` · ${activeCount}` : ''}{group.members && waitingCount > 0 ? <span className="font-normal normal-case tracking-normal"> · {t('messagehub.group.waitingCount', undefined, { count: waitingCount })}</span> : null}</h4>
      {group.can.invite && <button type="button" className="flex min-h-11 items-center gap-1.5 rounded-lg px-2 text-sm font-medium text-[color:var(--cp-accent)] disabled:opacity-40 md:min-h-9" disabled={pending} onClick={openInvite}><UserPlus size={15} aria-hidden />{t('messagehub.group.invite')}</button>}
    </div>
    {group.lifecycle === 'deleted' ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.deleted')}</p>
      : !group.hosted ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.remoteHost')}</p>
      : !group.myRole ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.reason.group_not_member')}</p>
      : !group.members ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.membersHidden')}</p>
      : <ul className="mt-1 divide-y divide-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)]" data-testid="group-members">
        {members.map(member => {
          const type = store.findEntity(context, member.did)?.type
          const removable = group.can.remove && member.did !== context.ownerDid && member.role !== 'owner'
          return <li key={member.did} className="flex min-h-11 items-center gap-2.5 py-1" data-member-did={member.did} title={member.did}>
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full" style={{ background: `color-mix(in srgb, ${type === 'agent' ? 'var(--cp-success)' : 'var(--cp-accent)'} 16%, transparent)`, color: type === 'agent' ? 'var(--cp-success)' : 'var(--cp-accent)' }} aria-hidden>{type === 'agent' ? <Bot size={15} /> : <User size={15} />}</span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm">{nameOf(member.did)}</span>
              {member.state !== 'active' && <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">{t(`messagehub.group.state.${member.state}`)}</span>}
            </span>
            {member.role !== 'member' && <span className="flex shrink-0 items-center gap-1 rounded-full bg-[color:color-mix(in_srgb,var(--cp-warning)_14%,transparent)] px-2 py-0.5 text-[11px] text-[color:color-mix(in_srgb,var(--cp-warning)_70%,var(--cp-text))]">{member.role === 'owner' ? <Crown size={11} aria-hidden /> : <Shield size={11} aria-hidden />}{t(`messagehub.group.role.${member.role}`)}</span>}
            {removable && <button type="button" disabled={pending} onClick={() => void remove(member)} aria-label={`${t(member.state === 'active' ? 'messagehub.group.remove' : 'messagehub.group.cancelInvite')}: ${nameOf(member.did)}`} title={t(member.state === 'active' ? 'messagehub.group.remove' : 'messagehub.group.cancelInvite')} className="flex min-h-11 min-w-11 shrink-0 items-center justify-center rounded-lg text-[color:var(--cp-muted)] hover:text-[color:var(--cp-danger)] disabled:opacity-40 md:min-h-8 md:min-w-8"><UserMinus size={15} /></button>}
          </li>
        })}
      </ul>}
    {group.myRole && group.lifecycle !== 'deleted' && <div className="mt-3 flex flex-wrap gap-2">
      {group.myRole === 'owner'
        ? <button type="button" disabled={pending} className={`${hubButtonClass} flex items-center gap-1.5 text-[color:var(--cp-danger)]`} onClick={() => void disband()}><Trash2 size={15} aria-hidden />{t('messagehub.group.delete')}</button>
        : <button type="button" disabled={pending} className={`${hubButtonClass} flex items-center gap-1.5 text-[color:var(--cp-danger)]`} onClick={() => void leave()}><LogOut size={15} aria-hidden />{t('messagehub.group.leave')}</button>}
    </div>}
    {error && <p role="alert" className="mt-2 text-xs text-[color:var(--cp-danger)]">{error}</p>}
  </section>
}
