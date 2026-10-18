/* ── JoinGroupDialog: group DID or invite link → preview (`group.get_doc`) → `group.request_join` ── */

import { zodResolver } from '@hookform/resolvers/zod'
import { Button } from '@mui/material'
import { Link2, Search, Users } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { useForm } from 'react-hook-form'
import { useI18n } from '../../i18n/provider'
import { ObjectAvatar } from '../../presentation/ObjectIcon'
import { ObjectTitle } from '../../presentation/ObjectTitle'
import { networkActions } from './cache'
import { useLookup } from './hooks'
import { ListSkeleton, Pill, PersonName, StateMessage } from './Person'
import { personName } from './personPresentation'
import { networkErrorCode, networkErrorText } from './relation'
import { joinGroupFormSchema, type JoinGroupForm, type JoinGroupParams, type JoinGroupResult } from './schemas'
import type { GroupDocInfo, JoinOutcome, LookupCandidate } from './types'

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline gap-3 text-sm">
      <span className="w-24 shrink-0 text-[12px] font-medium text-[color:var(--cp-muted)]">{label}</span>
      <span className="min-w-0 flex-1 break-words text-[color:var(--cp-text)]">{children}</span>
    </div>
  )
}

function GroupPreview({ candidate, onDone }: { candidate: LookupCandidate; onDone: (result: JoinGroupResult) => void }) {
  const { t } = useI18n()
  const info = candidate.group
  const hostedHere = info?.hosted_here === true
  const [doc, setDoc] = useState<{ did: string; value: GroupDocInfo | null } | null>(null)
  const [pending, setPending] = useState(false)
  const [outcome, setOutcome] = useState<JoinOutcome | null>(null)
  const [error, setError] = useState<{ code: string; text: string } | null>(null)

  useEffect(() => {
    if (!hostedHere) return
    let cancelled = false
    networkActions.getGroupDoc(candidate.did)
      .then(value => { if (!cancelled) setDoc({ did: candidate.did, value }) })
      .catch(() => { if (!cancelled) setDoc({ did: candidate.did, value: null }) })
    return () => { cancelled = true }
  }, [candidate.did, hostedHere])

  const loadedDoc = doc?.did === candidate.did ? doc.value : null
  const name = loadedDoc?.name ?? candidate.name ?? personName(candidate)
  const policy = loadedDoc?.joinPolicy ?? info?.join_policy
  const owner = loadedDoc?.owner ?? info?.owner
  const host = loadedDoc?.host ?? info?.host
  const state = candidate.relation.group_state
  const loadingDoc = hostedHere && doc?.did !== candidate.did

  const join = async () => {
    setPending(true)
    setError(null)
    try {
      setOutcome(await networkActions.requestJoin(candidate.did, info?.invite))
    } catch (failure) {
      setError({ code: networkErrorCode(failure), text: networkErrorText(t, failure) })
    } finally {
      setPending(false)
    }
  }

  if (outcome) {
    return (
      <div className="space-y-4" data-testid="join-group-outcome" data-outcome={outcome}>
        <StateMessage tone={outcome === 'active' ? 'success' : 'warning'} testId="join-group-outcome-text">
          {t(outcome === 'active' ? 'network.joinGroup.joined' : 'network.joinGroup.pendingApproval', undefined, { name })}
        </StateMessage>
        <div className="flex justify-end">
          <Button variant="contained" onClick={() => onDone({ action: 'joined', groupDid: candidate.did, name, state: outcome })}>{t('network.done')}</Button>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-4" data-testid="join-group-preview">
      <div className="flex gap-4 rounded-2xl border border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_50%,var(--cp-surface))] p-4">
        <ObjectAvatar title={name} icons={[candidate.avatar]} size={56} shape="square" tone="var(--cp-warning)" fallbackIcon={<Users size={26} />} decorative />
        <div className="min-w-0 flex-1 space-y-2">
          <ObjectTitle title={name} description={loadedDoc?.description ?? candidate.note} did={candidate.did} showDid size="lg" />
          <div className="flex flex-wrap gap-1.5">
            {hostedHere ? <Pill tone="var(--cp-success)">{t('network.group.hostedHere')}</Pill> : <Pill tone="var(--cp-muted)">{t('network.group.hostedElsewhere')}</Pill>}
            {policy ? <Pill tone="var(--cp-accent)">{t(`network.group.policy.${policy}`, policy)}</Pill> : null}
            {info?.invite ? <Pill tone="var(--cp-accent)"><Link2 size={11} />{t('network.joinGroup.hasInvite')}</Pill> : null}
          </div>
        </div>
      </div>
      {loadingDoc ? <ListSkeleton rows={1} /> : (
        <div className="space-y-1.5 px-1">
          {owner ? <Fact label={t('network.group.owner')}><PersonName person={{ did: owner }} /></Fact> : null}
          {host ? <Fact label={t('network.group.host')}>{host}</Fact> : null}
          <Fact label={t('network.group.members')}>{info?.member_count !== undefined ? t('network.group.memberCount', undefined, { count: info.member_count }) : t('network.group.membersHidden')}</Fact>
        </div>
      )}
      <p className="px-1 text-xs text-[color:var(--cp-muted)]">{t('network.joinGroup.risk')}</p>

      {!hostedHere ? <StateMessage tone="warning" testId="join-group-remote">{t('network.joinGroup.remoteUnsupported')}</StateMessage> : null}
      {hostedHere && state === 'active' ? <StateMessage tone="success" testId="join-group-member">{t('network.joinGroup.alreadyMember')}</StateMessage> : null}
      {hostedHere && state === 'pending_admin_approval' ? <StateMessage tone="warning">{t('network.joinGroup.alreadyPending')}</StateMessage> : null}
      {hostedHere && state === 'invited' ? <StateMessage>{t('network.joinGroup.invited')}</StateMessage> : null}
      {error ? (
        <StateMessage tone={error.code === 'invite-required' ? 'warning' : 'danger'} testId="join-group-error">
          {error.code === 'invite-required' ? t('network.joinGroup.inviteRequired') : error.text}
        </StateMessage>
      ) : null}

      <div className="flex flex-wrap justify-end gap-2">
        <Button variant="text" onClick={() => onDone({ action: 'cancelled' })}>{t('network.close')}</Button>
        {hostedHere && state !== 'active' && state !== 'pending_admin_approval' ? (
          <Button variant="contained" disabled={pending || loadingDoc} onClick={() => void join()} data-testid="join-group-submit">
            {t(pending ? 'network.joinGroup.joining' : policy === 'request_and_approve' && !info?.invite ? 'network.joinGroup.request' : 'network.joinGroup.join')}
          </Button>
        ) : null}
      </div>
    </div>
  )
}

function Lookup({ input, onDone }: { input: string; onDone: (result: JoinGroupResult) => void }) {
  const { t } = useI18n()
  const lookup = useLookup(input, { kinds: ['group'], debounceMs: 0 })
  const result = lookup.input === input.trim() ? lookup.result : undefined
  const candidate = result?.candidates.find(item => item.kind === 'group')
  if (!candidate && (!result || lookup.status === 'loading' || lookup.status === 'idle') && lookup.status !== 'error') return <ListSkeleton rows={2} />
  if (!candidate && lookup.status === 'error') {
    return <StateMessage tone="danger" action={<Button variant="text" size="small" onClick={lookup.retry}>{t('network.retry')}</Button>}>{networkErrorText(t, lookup.error)}</StateMessage>
  }
  if (!candidate) {
    return <StateMessage tone="danger" testId="join-group-lookup-error">{t(`network.lookup.${result?.error?.code ?? 'group-not-found'}`)}</StateMessage>
  }
  return <GroupPreview key={`${candidate.did}|${input}`} candidate={candidate} onDone={onDone} />
}

export function JoinGroupDialog({ params, onDone }: { params: JoinGroupParams; onDone: (result: JoinGroupResult) => void }) {
  const { t } = useI18n()
  const [submitted, setSubmitted] = useState(params.input?.trim() ?? '')
  const form = useForm<JoinGroupForm>({ resolver: zodResolver(joinGroupFormSchema), defaultValues: { input: params.input ?? '' } })
  return (
    <div className="space-y-4" data-testid="join-group">
      <form className="flex flex-col gap-1" onSubmit={form.handleSubmit(values => setSubmitted(values.input))}>
        <label className="text-sm text-[color:var(--cp-text)]" htmlFor="network-join-group-input">{t('network.joinGroup.inputLabel')}</label>
        <div className="flex gap-2">
          <input
            id="network-join-group-input"
            {...form.register('input')}
            autoFocus
            placeholder={t('network.joinGroup.placeholder')}
            className="min-h-10 min-w-0 flex-1 rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3 text-sm text-[color:var(--cp-text)]"
          />
          <Button type="submit" variant="outlined" startIcon={<Search size={14} />}>{t('network.joinGroup.find')}</Button>
        </div>
        {form.formState.errors.input ? <span role="alert" className="text-xs text-[color:var(--cp-danger)]">{t(form.formState.errors.input.message ?? '')}</span> : null}
      </form>
      {submitted ? <Lookup input={submitted} onDone={onDone} /> : (
        <div className="space-y-3">
          <p className="text-[13px] text-[color:var(--cp-muted)]">{t('network.joinGroup.intro')}</p>
          <div className="flex justify-end"><Button variant="text" onClick={() => onDone({ action: 'cancelled' })}>{t('network.cancel')}</Button></div>
        </div>
      )}
    </div>
  )
}
