import { useRef, useState, type FormEvent } from 'react'
import { Bot, Search, User, X } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { shortDid } from './api/projection'
import { defaultGroupName, GROUP_MEMBER_LIMIT, groupErrorText, isAgentGroupDisabled, memberCandidates, parseInviteLink, participating } from './groupModel'
import { AgentGroupSettingsLink } from './AgentGroupSettingsLink'
import { DialogFocus, hubButtonClass, hubInputClass, hubPrimaryButtonClass } from './SessionDialogs'
import { useMessageHubStore } from './store'
import type { Entity, GroupInfo, MessageHubContext } from './types'

export function MemberPicker({ candidates, selected, unavailable, onToggle }: { candidates: Entity[]; selected: string[]; unavailable?: Map<string, string>; onToggle: (id: string) => void }) {
  const { t } = useI18n()
  const [query, setQuery] = useState('')
  const byId = new Map(candidates.map(entity => [entity.id, entity]))
  const needle = query.trim().toLowerCase()
  const visible = needle ? candidates.filter(entity => entity.name.toLowerCase().includes(needle) || entity.id.toLowerCase().includes(needle)) : candidates
  const full = selected.length >= GROUP_MEMBER_LIMIT
  return <div className="space-y-2">
    {selected.length > 0 && <ul className="flex flex-wrap gap-1.5" aria-label={t('messagehub.group.selectedMembers')} data-testid="group-selected-members">
      {selected.map(id => <li key={id}><button type="button" onClick={() => onToggle(id)} className="flex min-h-8 items-center gap-1 rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)] py-1 pl-3 pr-2 text-[13px] text-[color:var(--cp-accent)]" aria-label={t('messagehub.group.unselect', undefined, { name: byId.get(id)?.name ?? id })}>
        <span className="max-w-[10rem] truncate">{byId.get(id)?.name ?? shortDid(id)}</span><X size={13} aria-hidden />
      </button></li>)}
    </ul>}
    <label className="flex min-h-11 items-center gap-2 rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3">
      <Search size={15} className="shrink-0 text-[color:var(--cp-muted)]" aria-hidden />
      <input type="search" value={query} onChange={event => setQuery(event.target.value)} placeholder={t('messagehub.group.searchContacts')} aria-label={t('messagehub.group.searchContacts')} className="min-w-0 flex-1 border-none bg-transparent text-sm outline-none" />
    </label>
    {candidates.length === 0 ? <p className="rounded-lg border border-dashed border-[color:var(--cp-border)] px-3 py-4 text-center text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.noContacts')}</p>
      : visible.length === 0 ? <p className="px-3 py-4 text-center text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.group.noMatches')}</p>
      : <ul className="shell-scrollbar max-h-64 overflow-y-auto rounded-xl border border-[color:var(--cp-border)] py-1" data-testid="group-member-picker">
        {visible.map(entity => {
          const note = unavailable?.get(entity.id)
          const checked = selected.includes(entity.id)
          return <li key={entity.id}>
            <label className={`flex min-h-11 items-center gap-3 px-3 py-1.5 ${note || (full && !checked) ? 'cursor-default opacity-60' : 'cursor-pointer hover:bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)]'}`}>
              <input type="checkbox" className="h-4 w-4 shrink-0 accent-[color:var(--cp-accent)]" checked={checked || Boolean(note)} disabled={Boolean(note) || (full && !checked)} onChange={() => onToggle(entity.id)} />
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full" style={{ background: `color-mix(in srgb, ${entity.type === 'agent' ? 'var(--cp-success)' : 'var(--cp-accent)'} 16%, transparent)`, color: entity.type === 'agent' ? 'var(--cp-success)' : 'var(--cp-accent)' }} aria-hidden>{entity.type === 'agent' ? <Bot size={15} /> : <User size={15} />}</span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">{entity.name}</span>
                <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">{entity.type === 'agent' ? `${t('messagehub.entityType.agent', 'Agent')} · ` : ''}{shortDid(entity.id)}</span>
              </span>
              {note && <span className="shrink-0 text-xs text-[color:var(--cp-muted)]">{note}</span>}
            </label>
          </li>
        })}
      </ul>}
  </div>
}

/** Joining through an invite link (`group.request_join` with the link's token). */
function JoinByLinkForm({ context, onJoined }: { context: MessageHubContext; onJoined: (groupDid: string) => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const [link, setLink] = useState(''), [pending, setPending] = useState(false), [error, setError] = useState(''), [status, setStatus] = useState('')
  const parsed = parseInviteLink(link)
  const join = async () => {
    if (!parsed || pending) return
    setPending(true); setError(''); setStatus('')
    try {
      const state = await store.requestGroupJoin(context, parsed.groupDid, parsed.invite)
      if (state === 'active') onJoined(parsed.groupDid)
      else setStatus(t('messagehub.group.acceptedPendingApproval'))
    } catch (failure) { setError(groupErrorText(t, failure)) } finally { setPending(false) }
  }
  return <details className="rounded-lg border border-[color:var(--cp-border)] px-3" data-testid="join-by-link">
    <summary className="flex min-h-11 cursor-pointer items-center text-sm font-medium">{t('messagehub.group.joinByLink')}</summary>
    <div className="space-y-2 pb-3">
      <label className="block text-sm">{t('messagehub.group.inviteLinkLabel')}<input className={hubInputClass} value={link} placeholder="did:…?invite=…" onChange={event => setLink(event.target.value)} aria-label={t('messagehub.group.inviteLinkLabel')} /></label>
      {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
      {status && <p role="status" className="text-sm text-[color:var(--cp-muted)]">{status}</p>}
      <button type="button" className={hubButtonClass} disabled={pending || !parsed} onClick={() => void join()}>{t(pending ? 'messagehub.group.joining' : 'messagehub.group.join')}</button>
    </div>
  </details>
}

export function CreateGroupForm({ context, initialMembers = [], onCreated, onCancel }: { context: MessageHubContext; initialMembers?: string[]; onCreated: (groupDid: string) => void; onCancel: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const candidates = memberCandidates(store, context)
  const [name, setName] = useState('')
  const [members, setMembers] = useState(() => initialMembers.filter(id => candidates.some(entity => entity.id === id)))
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  const [groupDisabledAgents, setGroupDisabledAgents] = useState<string[]>([])
  const busy = useRef(false)
  const names = new Map(candidates.map(entity => [entity.id, entity.name]))
  const suggested = defaultGroupName(members.map(id => names.get(id) ?? shortDid(id))).slice(0, 64)
  const finalName = name.trim() || suggested
  const toggle = (id: string) => setMembers(current => current.includes(id) ? current.filter(item => item !== id) : [...current, id])
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (busy.current || !finalName) return
    busy.current = true; setPending(true); setError(''); setGroupDisabledAgents([])
    try { onCreated(await store.createGroup(context, { name: finalName, members })) } catch (failure) {
      setError(groupErrorText(t, failure))
      if (isAgentGroupDisabled(failure)) setGroupDisabledAgents(members.filter(id => candidates.find(entity => entity.id === id)?.type === 'agent'))
    } finally { busy.current = false; setPending(false) }
  }
  return <DialogFocus onCancel={() => { if (!busy.current) onCancel() }}><form onSubmit={event => void submit(event)} className="space-y-4" data-testid="create-group-form">
    <label className="block text-sm">{t('messagehub.group.name')}<input data-autofocus className={hubInputClass} value={name} maxLength={64} placeholder={suggested || t('messagehub.group.namePlaceholder')} onChange={event => setName(event.target.value)} /></label>
    <fieldset className="space-y-2">
      <legend className="flex w-full items-baseline justify-between gap-2 text-sm"><span>{t('messagehub.group.addMembers')}</span><span className="text-xs text-[color:var(--cp-muted)]" aria-live="polite">{t('messagehub.group.selectedCount', undefined, { count: members.length })}</span></legend>
      <MemberPicker candidates={candidates} selected={members} onToggle={toggle} />
    </fieldset>
    <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.group.inviteHint')}</p>
    {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
    {groupDisabledAgents.length > 0 && <div className="flex flex-wrap gap-2">{groupDisabledAgents.map(did => <AgentGroupSettingsLink key={did} agentDid={did} name={names.get(did) ?? shortDid(did)} />)}</div>}
    <div className="flex justify-end gap-2">
      <button type="button" className={hubButtonClass} disabled={pending} onClick={onCancel}>{t('messagehub.cancel')}</button>
      <button type="submit" className={hubPrimaryButtonClass} disabled={pending || !finalName}>{t(pending ? 'messagehub.creating' : 'messagehub.group.createAction')}</button>
    </div>
    <JoinByLinkForm context={context} onJoined={onCreated} />
  </form></DialogFocus>
}

/** Picks active group members (for a Group Session) or outside contacts (for a guest invitation). */
export function PickMembersForm({ context, candidates, title, submitLabel, onSubmit, onCancel }: { context: MessageHubContext; candidates: Entity[]; title: string; submitLabel: string; onSubmit: (dids: string[]) => Promise<void>; onCancel: () => void }) {
  const { t } = useI18n()
  const [members, setMembers] = useState<string[]>([])
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  const busy = useRef(false)
  const toggle = (id: string) => setMembers(current => current.includes(id) ? current.filter(item => item !== id) : [...current, id])
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (busy.current || members.length === 0) return
    busy.current = true; setPending(true); setError('')
    try { await onSubmit(members) } catch (failure) { setError(groupErrorText(t, failure)) } finally { busy.current = false; setPending(false) }
  }
  void context
  return <DialogFocus onCancel={() => { if (!busy.current) onCancel() }}><form onSubmit={event => void submit(event)} className="space-y-4" data-testid="pick-members-form">
    <fieldset className="space-y-2">
      <legend className="flex w-full items-baseline justify-between gap-2 text-sm"><span>{title}</span><span className="text-xs text-[color:var(--cp-muted)]" aria-live="polite">{t('messagehub.group.selectedCount', undefined, { count: members.length })}</span></legend>
      <MemberPicker candidates={candidates} selected={members} onToggle={toggle} />
    </fieldset>
    {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
    <div className="flex justify-end gap-2">
      <button type="button" className={hubButtonClass} disabled={pending} onClick={onCancel}>{t('messagehub.cancel')}</button>
      <button type="submit" className={hubPrimaryButtonClass} disabled={pending || members.length === 0}>{t(pending ? 'messagehub.saving' : submitLabel)}</button>
    </div>
  </form></DialogFocus>
}

export function InviteMembersForm({ context, group, onDone, onCancel }: { context: MessageHubContext; group: GroupInfo; onDone: () => void; onCancel: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const candidates = memberCandidates(store, context)
  const unavailable = new Map((group.members ?? []).filter(participating).map(member => [member.did, t(`messagehub.group.state.${member.state}`)]))
  const [members, setMembers] = useState<string[]>([])
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  const [failed, setFailed] = useState<Array<{ did: string; reason: string }>>([])
  const busy = useRef(false)
  const toggle = (id: string) => setMembers(current => current.includes(id) ? current.filter(item => item !== id) : [...current, id])
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (busy.current || members.length === 0) return
    busy.current = true; setPending(true); setError(''); setFailed([])
    try {
      const failures = await store.inviteGroupMembers(context, group.did, members)
      if (failures.length === 0) onDone()
      else { setFailed(failures); setMembers(failures.map(item => item.did)) }
    } catch (failure) { setError(groupErrorText(t, failure)) } finally { busy.current = false; setPending(false) }
  }
  const nameOf = (did: string) => candidates.find(entity => entity.id === did)?.name ?? shortDid(did)
  return <DialogFocus onCancel={() => { if (!busy.current) onCancel() }}><form onSubmit={event => void submit(event)} className="space-y-4" data-testid="invite-members-form">
    <fieldset className="space-y-2">
      <legend className="flex w-full items-baseline justify-between gap-2 text-sm"><span>{t('messagehub.group.addMembers')}</span><span className="text-xs text-[color:var(--cp-muted)]" aria-live="polite">{t('messagehub.group.selectedCount', undefined, { count: members.length })}</span></legend>
      <MemberPicker candidates={candidates} selected={members} unavailable={unavailable} onToggle={toggle} />
    </fieldset>
    <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.group.inviteHint')}</p>
    {failed.length > 0 && <ul role="alert" className="space-y-1 text-sm text-[color:var(--cp-danger)]">{failed.map(item => <li key={item.did}>{nameOf(item.did)}: {groupErrorText(t, new Error(item.reason))}{isAgentGroupDisabled(item.reason) ? <div><AgentGroupSettingsLink agentDid={item.did} name={nameOf(item.did)} /></div> : null}</li>)}</ul>}
    {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
    <div className="flex justify-end gap-2">
      <button type="button" className={hubButtonClass} disabled={pending} onClick={failed.length ? onDone : onCancel}>{t(failed.length ? 'messagehub.close' : 'messagehub.cancel')}</button>
      <button type="submit" className={hubPrimaryButtonClass} disabled={pending || members.length === 0}>{t(pending ? 'messagehub.saving' : 'messagehub.group.invite')}</button>
    </div>
  </form></DialogFocus>
}
