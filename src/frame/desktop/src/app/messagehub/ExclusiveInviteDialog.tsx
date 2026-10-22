import { useState, type FormEvent } from 'react'
import { accessDenialOf, type AccessDenial } from 'buckyos'
import { createExclusiveInvite, type ExclusiveInviteLink } from '../../api/sso'
import { DenialNotice } from '../../sso/DenialNotice'
import { OneTimeLink } from '../../sso/OneTimeLink'
import { DialogFocus, hubButtonClass, hubInputClass, hubPrimaryButtonClass } from './SessionDialogs'
import { isCustomerUserId, suggestCustomerUserId } from './guest/model'
import { useGuestText } from './guest/text'

const CHANNEL_KINDS = ['email', 'sms', 'other'] as const

export interface ExclusiveInviteTarget {
  groupDid: string
  /** Raw Group Session id (`to_session`). */
  sessionId: string
  title: string
}

/**
 * "Invite external customer (private link)" of a Group Session (SSO-06 /
 * contract §8.4): Control Panel creates (or reuses) a Limited account, its
 * exclusive invite and the Session Guest invitation of the account's DID; the
 * link comes back once and only this form shows it.
 */
export function ExclusiveInviteForm({ target, onCreated, onClose }: { target: ExclusiveInviteTarget; onCreated: () => void; onClose: () => void }) {
  const text = useGuestText()
  const [reuse, setReuse] = useState(false)
  const [name, setName] = useState('')
  const [userId, setUserId] = useState('')
  const [userIdEdited, setUserIdEdited] = useState(false)
  const [existingId, setExistingId] = useState('')
  const [channel, setChannel] = useState<typeof CHANNEL_KINDS[number]>('email')
  const [targetHint, setTargetHint] = useState('')
  const [pending, setPending] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)
  const [denial, setDenial] = useState<AccessDenial | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [created, setCreated] = useState<ExclusiveInviteLink | null>(null)
  const accountId = reuse ? existingId.trim() : (userIdEdited ? userId : suggestCustomerUserId(name)).trim()

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (pending) return
    setDenial(null); setFailure(null)
    if (!reuse && !name.trim()) { setProblem(text('invite.nameRequired')); return }
    if (!isCustomerUserId(accountId)) { setProblem(text('invite.invalidUserId')); return }
    if (!targetHint.trim()) { setProblem(text('invite.targetRequired')); return }
    setProblem(null)
    setPending(true)
    const { data, error } = await createExclusiveInvite({
      account: reuse ? { kind: 'existing', userId: accountId } : { kind: 'new', userId: accountId, showName: name.trim() },
      channel: { kind: channel, target_hint: targetHint.trim() },
      resource: { kind: 'group_session', group_did: target.groupDid, session_id: target.sessionId, title: target.title },
    })
    setPending(false)
    if (data?.link) { setCreated(data); onCreated(); return }
    const refused = accessDenialOf(error)
    if (refused) setDenial(refused)
    else setFailure(error instanceof Error ? error.message : '')
  }

  if (created) {
    return <div className="space-y-3" data-testid="exclusive-invite-created">
      <OneTimeLink link={created.link} onDismiss={onClose} testId="exclusive-invite-link" />
      {created.invite.resource_state === 'pending' && <p role="status" className="text-sm text-[color:var(--cp-warning)]" data-testid="exclusive-invite-pending">{text('invite.resourcePending')}</p>}
    </div>
  }
  return <DialogFocus onCancel={() => { if (!pending) onClose() }}><form className="space-y-3" onSubmit={event => void submit(event)} noValidate data-testid="exclusive-invite-form">
    <p className="text-sm text-[color:var(--cp-muted)]">{text('invite.intro')}</p>
    <label className="flex min-h-11 items-center gap-2 text-sm"><input type="checkbox" checked={reuse} onChange={event => setReuse(event.target.checked)} />{text('invite.reuse')}</label>
    {reuse
      ? <label className="block text-sm">{text('invite.reuseId')}<input data-autofocus className={hubInputClass} value={existingId} onChange={event => setExistingId(event.target.value)} autoComplete="off" /></label>
      : <>
        <label className="block text-sm">{text('invite.name')}<input data-autofocus className={hubInputClass} value={name} onChange={event => setName(event.target.value)} autoComplete="off" /></label>
        <label className="block text-sm">{text('invite.userId')}<input className={hubInputClass} value={userIdEdited ? userId : suggestCustomerUserId(name)} onChange={event => { setUserIdEdited(true); setUserId(event.target.value) }} autoComplete="off" spellCheck={false} /><span className="mt-1 block text-xs text-[color:var(--cp-muted)]">{text('invite.userIdHint')}</span></label>
      </>}
    <div className="grid gap-3 sm:grid-cols-[10rem_1fr]">
      <label className="block text-sm">{text('invite.channel')}<select className={hubInputClass} value={channel} onChange={event => setChannel(event.target.value as typeof CHANNEL_KINDS[number])}>{CHANNEL_KINDS.map(kind => <option key={kind} value={kind}>{text(`invite.channel.${kind}`)}</option>)}</select></label>
      <label className="block text-sm">{text('invite.target')}<input className={hubInputClass} value={targetHint} placeholder={text('invite.targetPlaceholder')} onChange={event => setTargetHint(event.target.value)} autoComplete="off" /></label>
    </div>
    {problem && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{problem}</p>}
    {denial && <DenialNotice denial={denial} showAdjust testId="exclusive-invite-denial" />}
    {failure !== null && <p role="alert" className="text-sm text-[color:var(--cp-danger)]" data-testid="exclusive-invite-failed">{text('invite.failed')}{failure ? <span className="mt-1 block break-words text-xs text-[color:var(--cp-muted)]">{failure}</span> : null}</p>}
    <div className="flex justify-end gap-2">
      <button type="button" className={hubButtonClass} disabled={pending} onClick={onClose}>{text('invite.cancel')}</button>
      <button type="submit" className={hubPrimaryButtonClass} disabled={pending} data-testid="exclusive-invite-submit">{text(pending ? 'invite.creating' : 'invite.create')}</button>
    </div>
  </form></DialogFocus>
}
