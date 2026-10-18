import { useRef, useState, type FormEvent } from 'react'
import { useI18n } from '../../i18n/provider'
import { PeoplePicker } from '../../components/network/PeoplePicker'
import { peoplePickerInputSchema, type PickedPerson } from '../../components/network/schemas'
import { defaultGroupName, GROUP_MEMBER_KINDS, GROUP_MEMBER_LIMIT, groupErrorText, isAgentGroupDisabled } from './groupModel'
import { AgentGroupSettingsLink } from './AgentGroupSettingsLink'
import { DialogFocus, hubButtonClass, hubInputClass, hubPrimaryButtonClass } from './SessionDialogs'
import { useMessageHubStore } from './store'
import type { MessageHubContext } from './types'

export function CreateGroupForm({ context, initialMembers = [], onCreated, onCancel }: { context: MessageHubContext; initialMembers?: string[]; onCreated: (groupDid: string) => void; onCancel: () => void }) {
  const { t } = useI18n(), store = useMessageHubStore()
  const [params] = useState(() => peoplePickerInputSchema.parse({ limit: GROUP_MEMBER_LIMIT, filter: { kinds: GROUP_MEMBER_KINDS }, initialSelection: initialMembers }))
  const [name, setName] = useState('')
  const [members, setMembers] = useState<PickedPerson[]>([])
  const [pending, setPending] = useState(false), [error, setError] = useState('')
  const [groupDisabledAgents, setGroupDisabledAgents] = useState<PickedPerson[]>([])
  const busy = useRef(false)
  const suggested = defaultGroupName(members.map(member => member.name)).slice(0, 64)
  const finalName = name.trim() || suggested
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (busy.current || !finalName) return
    busy.current = true; setPending(true); setError(''); setGroupDisabledAgents([])
    try { onCreated(await store.createGroup(context, { name: finalName, members: members.map(member => member.did) })) } catch (failure) {
      setError(groupErrorText(t, failure))
      if (isAgentGroupDisabled(failure)) setGroupDisabledAgents(members.filter(member => member.kind === 'agent'))
    } finally { busy.current = false; setPending(false) }
  }
  return <DialogFocus onCancel={() => { if (!busy.current) onCancel() }}><form onSubmit={event => void submit(event)} className="space-y-4" data-testid="create-group-form">
    <label className="block text-sm">{t('messagehub.group.name')}<input data-autofocus className={hubInputClass} value={name} maxLength={64} placeholder={suggested || t('messagehub.group.namePlaceholder')} onChange={event => setName(event.target.value)} /></label>
    <fieldset className="space-y-2">
      <legend className="flex w-full items-baseline justify-between gap-2 text-sm"><span>{t('messagehub.group.addMembers')}</span><span className="text-xs text-[color:var(--cp-muted)]" aria-live="polite">{t('messagehub.group.selectedCount', undefined, { count: members.length })}</span></legend>
      <PeoplePicker params={params} embedded onChange={setMembers} />
    </fieldset>
    <p className="text-xs text-[color:var(--cp-muted)]">{t('messagehub.group.inviteHint')}</p>
    {error && <p role="alert" className="text-sm text-[color:var(--cp-danger)]">{error}</p>}
    {groupDisabledAgents.length > 0 && <div className="flex flex-wrap gap-2">{groupDisabledAgents.map(agent => <AgentGroupSettingsLink key={agent.did} agentDid={agent.did} name={agent.name} />)}</div>}
    <div className="flex justify-end gap-2">
      <button type="button" className={hubButtonClass} disabled={pending} onClick={onCancel}>{t('messagehub.cancel')}</button>
      <button type="submit" className={hubPrimaryButtonClass} disabled={pending || !finalName}>{t(pending ? 'messagehub.creating' : 'messagehub.group.createAction')}</button>
    </div>
  </form></DialogFocus>
}

/** Invitations that did not go out, with the host's reason (and the Agent setting that blocks an Agent). */
export function InviteFailures({ failures, nameOf, onClose }: { failures: Array<{ did: string; reason: string }>; nameOf: (did: string) => string; onClose: () => void }) {
  const { t } = useI18n()
  return <DialogFocus onCancel={onClose}><div className="space-y-4" data-testid="invite-failures">
    <ul role="alert" className="space-y-1 text-sm text-[color:var(--cp-danger)]">{failures.map(item => <li key={item.did}>{nameOf(item.did)}: {groupErrorText(t, new Error(item.reason))}{isAgentGroupDisabled(item.reason) ? <div><AgentGroupSettingsLink agentDid={item.did} name={nameOf(item.did)} /></div> : null}</li>)}</ul>
    <div className="flex justify-end"><button type="button" data-autofocus className={hubButtonClass} onClick={onClose}>{t('messagehub.close')}</button></div>
  </div></DialogFocus>
}
