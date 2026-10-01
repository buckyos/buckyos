import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Bot, Crown, Link2, LogOut, MoreHorizontal, Pencil, Shield, Trash2, User, UserMinus, UserPlus } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useWindowDialog } from '../../desktop/windows/dialogs'
import { confirmGroupAction } from './confirmDialog'
import { friendlyDidName } from './api/projection'
import { InviteMembersForm } from './GroupDialogs'
import { groupErrorText, parseInviteLink, participating, sortGroupMembers } from './groupModel'
import { hubButtonClass, hubInputClass, hubPrimaryButtonClass } from './SessionDialogs'
import { useMessageHubStore } from './store'
import type { Entity, GroupInfo, GroupMember, MessageHubContext } from './types'

const MUTE_MS = 3600_000

/** Name and description of the group (`group.apply_config` on `profile`). */
function ProfileEditor({ group, context, onDone }: { group: GroupInfo; context: MessageHubContext; onDone: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const [name, setName] = useState(group.name), [description, setDescription] = useState(group.description)
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || !name.trim()) return
    setPending(true); setError('')
    try { await store.updateGroupProfile(context, group.did, { name, description }); onDone() } catch (failure) { setError(groupErrorText(t, failure)) } finally { setPending(false) }
  }
  return <form onSubmit={event => void submit(event)} className="mt-2 space-y-2" data-testid="group-profile-form">
    <label className="block text-sm">{t('messagehub.group.name')}<input data-autofocus className={hubInputClass} value={name} maxLength={64} onChange={event => setName(event.target.value)} /></label>
    <label className="block text-sm">{t('messagehub.description')}<textarea className={hubInputClass} value={description} maxLength={500} rows={2} onChange={event => setDescription(event.target.value)} /></label>
    {error && <p role="alert" className="text-xs text-[color:var(--cp-danger)]">{error}</p>}
    <div className="flex justify-end gap-2">
      <button type="button" className={hubButtonClass} disabled={pending} onClick={onDone}>{t('messagehub.cancel')}</button>
      <button type="submit" className={hubPrimaryButtonClass} disabled={pending || !name.trim()}>{t(pending ? 'messagehub.saving' : 'messagehub.save')}</button>
    </div>
  </form>
}

/** Per-member management menu (roles, ownership, moderation, approval). */
function MemberMenu({ group, member, context, name, disabled, onRun }: { group: GroupInfo; member: GroupMember; context: MessageHubContext; name: string; disabled: boolean; onRun: (operation: () => Promise<unknown>, confirm?: { title: string; body: string; confirm: string }) => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const close = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false) }
    document.addEventListener('pointerdown', close)
    return () => document.removeEventListener('pointerdown', close)
  }, [open])
  const self = member.did === context.ownerDid, active = member.state === 'active', owner = member.role === 'owner'
  const items: Array<{ key: string; label: string; danger?: boolean; run: () => void }> = []
  if (member.state === 'pending_admin_approval' && group.can.approve) {
    items.push({ key: 'approve', label: t('messagehub.group.approve'), run: () => onRun(() => store.approveGroupMember(context, group.did, member.did)) })
    items.push({ key: 'reject', label: t('messagehub.group.reject'), danger: true, run: () => onRun(() => store.rejectGroupMember(context, group.did, member.did)) })
  }
  if (active && !self && !owner && group.can.updateRole) {
    items.push(member.role === 'admin'
      ? { key: 'member', label: t('messagehub.group.makeMember'), run: () => onRun(() => store.updateGroupMemberRole(context, group.did, member.did, 'member')) }
      : { key: 'admin', label: t('messagehub.group.makeAdmin'), run: () => onRun(() => store.updateGroupMemberRole(context, group.did, member.did, 'admin')) })
  }
  if (active && !self && !owner && group.can.transferOwner && !group.pendingTransfer) {
    items.push({ key: 'transfer', label: t('messagehub.group.transferOwner'), run: () => onRun(() => store.transferGroupOwner(context, group.did, member.did), { title: t('messagehub.group.transferOwner'), body: t('messagehub.group.transferConfirm', undefined, { name }), confirm: t('messagehub.group.transferOwner') }) })
  }
  if (active && !self && !owner && group.can.moderate) {
    items.push({ key: 'mute', label: t('messagehub.group.mute'), run: () => onRun(() => store.moderateGroupMember(context, group.did, member.did, { mutedUntil: store.now() + MUTE_MS })) })
    items.push({ key: 'unmute', label: t('messagehub.group.unmute'), run: () => onRun(() => store.moderateGroupMember(context, group.did, member.did, { mutedUntil: null })) })
    items.push({ key: 'block', label: t('messagehub.group.block'), danger: true, run: () => onRun(() => store.moderateGroupMember(context, group.did, member.did, { blocked: true }), { title: t('messagehub.group.block'), body: t('messagehub.group.blockConfirm', undefined, { name }), confirm: t('messagehub.group.block') }) })
  }
  if (items.length === 0) return null
  return <div ref={root} className="relative shrink-0" onKeyDown={event => { if (event.key === 'Escape') setOpen(false) }}>
    <button type="button" disabled={disabled} aria-haspopup="menu" aria-expanded={open} aria-label={`${t('messagehub.group.memberActions')}: ${name}`} title={t('messagehub.group.memberActions')} onClick={() => setOpen(value => !value)} className="flex min-h-11 min-w-11 items-center justify-center rounded-lg text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)] disabled:opacity-40 md:min-h-8 md:min-w-8"><MoreHorizontal size={15} /></button>
    {open && <div role="menu" className="absolute right-0 z-30 mt-1 w-48 overflow-hidden rounded-xl bg-[color:var(--cp-surface)] py-1 shadow-lg" style={{ border: '1px solid var(--cp-border)' }}>
      {items.map(item => <button key={item.key} type="button" role="menuitem" className={`flex min-h-10 w-full items-center px-3 text-left text-[13px] hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)] ${item.danger ? 'text-[color:var(--cp-danger)]' : ''}`} onClick={() => { setOpen(false); item.run() }}>{item.label}</button>)}
    </div>}
  </div>
}

/** Membership and management of a self-host group, shown in the group's entity details. */
export function GroupPanel({ entity, context }: { entity: Entity; context: MessageHubContext }) {
  const { t } = useI18n(), store = useMessageHubStore(), dialog = useWindowDialog()
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  const [editing, setEditing] = useState(false)
  const [inviteLink, setInviteLink] = useState<string | null>(null), [copied, setCopied] = useState(false)
  useEffect(() => { void store.ensureGroup(context, entity.id) }, [store, context.ownerDid, context.viewerDid, context.mode, entity.id]) // eslint-disable-line react-hooks/exhaustive-deps
  const group = store.group(context, entity.id)
  const status = store.groupStatus(context, entity.id)
  if (!group) return status === 'loading' || status === 'idle' ? <p role="status" className="mb-4 text-sm text-[color:var(--cp-muted)]">{t('messagehub.group.loading')}</p> : null
  const run = async (operation: () => Promise<unknown>) => {
    if (pending) return
    setPending(true); setError('')
    try { await operation() } catch (failure) { setError(groupErrorText(t, failure)) } finally { setPending(false) }
  }
  const runConfirmed = (operation: () => Promise<unknown>, confirm?: { title: string; body: string; confirm: string }) => {
    void (async () => { if (confirm && !await confirmGroupAction(dialog, t, confirm)) return; await run(operation) })()
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
  const createLink = () => run(async () => { setInviteLink(await store.createGroupInviteLink(context, group.did, {})); setCopied(false) })
  const revokeLink = () => run(async () => { const token = inviteLink ? parseInviteLink(inviteLink)?.invite : undefined; if (token) await store.revokeGroupInviteLink(context, group.did, token); setInviteLink(null) })
  const copyLink = async () => { if (!inviteLink) return; try { await navigator.clipboard.writeText(inviteLink); setCopied(true) } catch { setCopied(false) } }
  const manageable = group.myRole && group.lifecycle !== 'deleted' && group.hosted
  return <section className="mb-4 rounded-xl p-3" style={{ background: 'color-mix(in srgb, var(--cp-text) 4%, transparent)' }} data-testid="group-panel" aria-label={t('messagehub.group.membersTitle')}>
    {manageable && (editing ? <ProfileEditor key={group.revision} group={group} context={context} onDone={() => setEditing(false)} /> : <div className="mb-3 flex items-start gap-2" data-testid="group-profile">
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-semibold">{group.name || entity.name}</p>
        {group.description ? <p className="whitespace-pre-wrap break-words text-[13px] text-[color:var(--cp-muted)]">{group.description}</p> : null}
      </div>
      {group.can.updateConfig && <button type="button" disabled={pending} className="flex min-h-11 min-w-11 shrink-0 items-center justify-center rounded-lg text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)] disabled:opacity-40 md:min-h-8 md:min-w-8" aria-label={t('messagehub.group.editProfile')} title={t('messagehub.group.editProfile')} onClick={() => setEditing(true)}><Pencil size={15} /></button>}
    </div>)}
    {manageable && group.pendingTransfer && group.can.transferOwner && <div className="mb-3 flex items-center gap-2 rounded-lg bg-[color:color-mix(in_srgb,var(--cp-warning)_12%,transparent)] px-3 py-2 text-[13px]" data-testid="pending-transfer">
      <span className="min-w-0 flex-1">{t('messagehub.group.transferPending', undefined, { name: nameOf(group.pendingTransfer.memberDid) })}</span>
      <button type="button" disabled={pending} className="shrink-0 font-medium text-[color:var(--cp-accent)] disabled:opacity-40" onClick={() => void run(() => store.cancelGroupOwnerTransfer(context, group.did))}>{t('messagehub.group.cancelTransfer')}</button>
    </div>}
    <div className="flex items-center justify-between gap-2">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]">{t('messagehub.group.membersTitle')}{group.members ? ` · ${activeCount}` : ''}{group.members && waitingCount > 0 ? <span className="font-normal normal-case tracking-normal"> · {t('messagehub.group.waitingCount', undefined, { count: waitingCount })}</span> : null}</h4>
      {group.can.invite && <div className="flex items-center gap-1">
        <button type="button" className="flex min-h-11 items-center gap-1.5 rounded-lg px-2 text-sm font-medium text-[color:var(--cp-accent)] disabled:opacity-40 md:min-h-9" disabled={pending} onClick={() => void createLink()} title={t('messagehub.group.createInviteLink')} aria-label={t('messagehub.group.createInviteLink')}><Link2 size={15} aria-hidden /></button>
        <button type="button" className="flex min-h-11 items-center gap-1.5 rounded-lg px-2 text-sm font-medium text-[color:var(--cp-accent)] disabled:opacity-40 md:min-h-9" disabled={pending} onClick={openInvite}><UserPlus size={15} aria-hidden />{t('messagehub.group.invite')}</button>
      </div>}
    </div>
    {inviteLink && <div className="mt-2 space-y-1 rounded-lg border border-[color:var(--cp-border)] p-2 text-xs" data-testid="group-invite-link">
      <p className="text-[color:var(--cp-muted)]">{t('messagehub.group.inviteLinkHint')}</p>
      <code className="block select-all break-all" data-testid="group-invite-link-text">{inviteLink}</code>
      <div className="flex flex-wrap gap-2">
        <button type="button" className={hubButtonClass} onClick={() => void copyLink()}>{t(copied ? 'messagehub.group.copied' : 'messagehub.group.copyLink')}</button>
        <button type="button" className={`${hubButtonClass} text-[color:var(--cp-danger)]`} disabled={pending} onClick={() => void revokeLink()}>{t('messagehub.group.revokeLink')}</button>
      </div>
    </div>}
    {group.lifecycle === 'deleted' ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.deleted')}</p>
      : !group.hosted ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.remoteHost')}</p>
      : !group.myRole ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.reason.group_not_member')}</p>
      : !group.members ? <p className="mt-2 text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.membersHidden')}</p>
      : <ul className="mt-1 divide-y divide-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)]" data-testid="group-members">
        {members.map(member => {
          const type = store.findEntity(context, member.did)?.type
          const removable = group.can.remove && member.did !== context.ownerDid && member.role !== 'owner'
          return <li key={member.did} className="flex min-h-11 items-center gap-2.5 py-1" data-member-did={member.did} data-role={member.role} title={member.did}>
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full" style={{ background: `color-mix(in srgb, ${type === 'agent' ? 'var(--cp-success)' : 'var(--cp-accent)'} 16%, transparent)`, color: type === 'agent' ? 'var(--cp-success)' : 'var(--cp-accent)' }} aria-hidden>{type === 'agent' ? <Bot size={15} /> : <User size={15} />}</span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm">{nameOf(member.did)}</span>
              {member.state !== 'active' && <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">{t(`messagehub.group.state.${member.state}`)}</span>}
            </span>
            {member.role !== 'member' && <span className="flex shrink-0 items-center gap-1 rounded-full bg-[color:color-mix(in_srgb,var(--cp-warning)_14%,transparent)] px-2 py-0.5 text-[11px] text-[color:color-mix(in_srgb,var(--cp-warning)_70%,var(--cp-text))]">{member.role === 'owner' ? <Crown size={11} aria-hidden /> : <Shield size={11} aria-hidden />}{t(`messagehub.group.role.${member.role}`)}</span>}
            <MemberMenu group={group} member={member} context={context} name={nameOf(member.did)} disabled={pending} onRun={runConfirmed} />
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
