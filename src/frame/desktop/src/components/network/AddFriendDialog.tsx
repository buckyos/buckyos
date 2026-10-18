/* ── AddFriendDialog: find a person, see their profile card, send a friend request or save a contact ── */

import { zodResolver } from '@hookform/resolvers/zod'
import { Button } from '@mui/material'
import { ArrowLeft, Check, Copy, UserPlus } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Controller, useForm } from 'react-hook-form'
import { useI18n } from '../../i18n/provider'
import { ObjectTitle } from '../../presentation/ObjectTitle'
import { networkActions, networkCache } from './cache'
import { FindPeople } from './FindPeople'
import { useLookup, useNetworkResource } from './hooks'
import { KindLabel, ListSkeleton, PersonAvatar, ReachabilityChip, StateMessage } from './Person'
import { pickedPerson, usePersonPresentation } from './personPresentation'
import { RelationBadge } from './RelationBadge'
import { FRIEND_MESSAGE_MAX, networkErrorCode, networkErrorText, relationOfCandidate, userTags } from './relation'
import { addFriendFormSchema, type AddFriendForm, type AddFriendOutcome, type AddFriendParams, type AddFriendResult } from './schemas'
import { TagEditor } from './TagEditor'
import type { FriendRequestResult, LookupCandidate } from './types'

const PEOPLE_KINDS = ['user', 'agent', 'msgtunnel', 'unknown'] as const

function outcomeOf(result: FriendRequestResult): AddFriendOutcome {
  if (result.friend_state === 'friend') return 'friends'
  if (result.request?.delivery?.state === 'pending') return 'request_queued'
  if (result.request?.delivery?.state === 'failed') return 'request_failed'
  return 'request_pending'
}

function ShareLinkButton() {
  const { t } = useI18n()
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    const text = await networkCache.shareText().catch(() => '')
    if (!text) return
    await navigator.clipboard?.writeText(text).catch(() => undefined)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1600)
  }
  return (
    <Button size="small" variant="outlined" startIcon={copied ? <Check size={13} /> : <Copy size={13} />} onClick={() => void copy()} data-testid="copy-share-link">
      {t(copied ? 'network.addFriend.shareCopied' : 'network.addFriend.copyShareLink')}
    </Button>
  )
}

function ProfileCard({ candidate }: { candidate: LookupCandidate }) {
  const { t } = useI18n()
  const presentation = usePersonPresentation(candidate)
  return (
    <div className="flex gap-4 rounded-2xl border border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_50%,var(--cp-surface))] p-4" data-testid="network-profile-card">
      <PersonAvatar person={candidate} size={56} />
      <div className="min-w-0 flex-1 space-y-1.5">
        <ObjectTitle title={presentation.title} did={candidate.did} showDid size="lg" />
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-[color:var(--cp-muted)]">
          <span><KindLabel kind={candidate.kind} /></span>
          {candidate.reachability.zone ? <span>· {t('network.profile.zone', undefined, { zone: candidate.reachability.zone })}</span> : null}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <RelationBadge relation={relationOfCandidate(candidate)} />
          <ReachabilityChip reachability={candidate.reachability} />
        </div>
        {candidate.note ? <p className="text-[13px] text-[color:var(--cp-muted)]">{candidate.note}</p> : null}
      </div>
    </div>
  )
}

function ProfileStep({ candidate, initial, onBack, onDone }: {
  candidate: LookupCandidate
  initial: { message: string; tags: string[] }
  onBack?: () => void
  onDone: (result: AddFriendResult) => void
}) {
  const { t } = useI18n()
  const contacts = useNetworkResource('contacts')
  const relation = relationOfCandidate(candidate)
  const name = usePersonPresentation(candidate).title
  const [pending, setPending] = useState<'friend' | 'save' | 'other' | null>(null)
  const [error, setError] = useState<{ text: string; unreachable: boolean } | null>(null)
  const [outcome, setOutcome] = useState<{ outcome: AddFriendOutcome; tags: string[] } | null>(null)
  const existingTags = userTags(relation.tags)
  const form = useForm<AddFriendForm>({ resolver: zodResolver(addFriendFormSchema), defaultValues: { message: initial.message, tags: [...new Set([...existingTags, ...initial.tags])] } })
  const suggestions = useMemo(() => [...new Set((contacts.data ?? []).flatMap(contact => userTags(contact.tags ?? [])))].sort(), [contacts.data])
  const unreachable = candidate.reachability.state === 'unreachable'
  const blocked = relation.accessLevel === 'block'
  const isGroup = candidate.kind === 'group'
  const befriendable = candidate.kind !== 'msgtunnel'

  const applyTags = async (tags: string[]) => {
    const add = tags.filter(tag => !existingTags.includes(tag))
    const remove = existingTags.filter(tag => !tags.includes(tag))
    if (add.length || remove.length) await networkActions.updateTags(candidate.did, { add, remove })
  }

  const act = async (kind: 'friend' | 'save', values: AddFriendForm) => {
    setPending(kind)
    setError(null)
    try {
      if (kind === 'friend') {
        const result = await networkActions.sendFriendRequest(candidate.did, values.message || undefined)
        await applyTags(values.tags)
        setOutcome({ outcome: outcomeOf(result), tags: values.tags })
      } else {
        await networkActions.saveContact(candidate.did, candidate.name)
        await applyTags(values.tags)
        setOutcome({ outcome: 'saved', tags: values.tags })
      }
    } catch (failure) {
      setError({ text: networkErrorText(t, failure), unreachable: networkErrorCode(failure) === 'unreachable' })
    } finally {
      setPending(null)
    }
  }

  const other = async (run: () => Promise<unknown>, next?: AddFriendOutcome) => {
    setPending('other')
    setError(null)
    try {
      await run()
      if (next) setOutcome({ outcome: next, tags: form.getValues('tags') })
    } catch (failure) {
      setError({ text: networkErrorText(t, failure), unreachable: false })
    } finally {
      setPending(null)
    }
  }

  if (outcome) {
    const done = () => onDone({ action: 'done', outcome: outcome.outcome, person: { ...pickedPerson(candidate), isContact: true }, tags: outcome.tags })
    const tone = outcome.outcome === 'request_failed' ? 'danger' : outcome.outcome === 'request_queued' ? 'warning' : 'success'
    return (
      <div className="space-y-4" data-testid="add-friend-outcome" data-outcome={outcome.outcome}>
        <ProfileCard candidate={candidate} />
        <StateMessage tone={tone} testId="add-friend-outcome-text">
          {t(`network.addFriend.outcome.${outcome.outcome}${outcome.outcome === 'friends' && candidate.reachability.state === 'local' ? 'SameZone' : ''}`, undefined, { name })}
        </StateMessage>
        {outcome.outcome === 'request_failed' ? <ShareLinkButton /> : null}
        <div className="flex justify-end"><Button variant="contained" data-autofocus onClick={done}>{t('network.done')}</Button></div>
      </div>
    )
  }

  const busy = pending !== null
  return (
    <form className="space-y-4" onSubmit={form.handleSubmit(values => act('friend', values))} data-testid="add-friend-profile">
      {onBack ? (
        <Button variant="text" size="small" startIcon={<ArrowLeft size={14} />} onClick={onBack}>{t('network.back')}</Button>
      ) : null}
      <ProfileCard candidate={candidate} />

      {isGroup ? <StateMessage>{t('network.addFriend.isGroup')}</StateMessage> : null}
      {blocked ? (
        <StateMessage tone="danger" action={<Button variant="outlined" size="small" disabled={busy} onClick={() => void other(() => networkActions.unblockContact(candidate.did))}>{t('network.action.unblock')}</Button>}>
          {t('network.addFriend.blocked', undefined, { name })}
        </StateMessage>
      ) : null}
      {!blocked && relation.friendState === 'friend' ? (
        <StateMessage tone="success" testId="add-friend-already">{t(candidate.reachability.state === 'local' ? 'network.addFriend.alreadySameZone' : 'network.addFriend.already', undefined, { name })}</StateMessage>
      ) : null}
      {!blocked && relation.friendState === 'outgoing_pending' ? (
        <StateMessage action={relation.pendingRequestId ? <Button variant="outlined" size="small" disabled={busy} onClick={() => void other(() => networkActions.cancelFriendRequest(relation.pendingRequestId!))}>{t('network.action.cancel_request')}</Button> : undefined}>
          {t('network.addFriend.waiting', undefined, { name })}
        </StateMessage>
      ) : null}
      {!blocked && relation.friendState === 'incoming_pending' ? (
        <StateMessage action={relation.pendingRequestId ? <Button size="small" variant="contained" disabled={busy} onClick={() => void other(() => networkActions.respondFriendRequest(relation.pendingRequestId!, 'accept'), 'friends')}>{t('network.action.accept_request')}</Button> : undefined}>
          {t('network.addFriend.incoming', undefined, { name })}
        </StateMessage>
      ) : null}
      {unreachable && !blocked ? (
        <StateMessage tone="danger" testId="add-friend-unreachable" action={<ShareLinkButton />}>
          {t('network.addFriend.unreachable', undefined, { name, reason: candidate.reachability.reason ? t(`network.reason.${candidate.reachability.reason}`, candidate.reachability.reason) : t('network.reason.unknown') })}
        </StateMessage>
      ) : null}

      {!isGroup && !blocked ? (
        <>
          {!befriendable ? <StateMessage testId="add-friend-not-befriendable">{t('network.addFriend.notBefriendable', undefined, { name })}</StateMessage> : null}
          {relation.friendState === 'none' && befriendable ? (
            <label className="block text-sm text-[color:var(--cp-text)]">
              {t('network.addFriend.message')}
              <textarea
                {...form.register('message')}
                rows={2}
                maxLength={FRIEND_MESSAGE_MAX}
                disabled={busy || unreachable}
                placeholder={t('network.addFriend.messagePlaceholder')}
                className="mt-1 w-full rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3 py-2 text-sm text-[color:var(--cp-text)]"
              />
              {form.formState.errors.message ? <span role="alert" className="text-xs text-[color:var(--cp-danger)]">{t(form.formState.errors.message.message ?? '')}</span> : null}
            </label>
          ) : null}
          <Controller
            control={form.control}
            name="tags"
            render={({ field }) => <TagEditor value={field.value} onChange={field.onChange} suggestions={suggestions} disabled={busy} />}
          />
        </>
      ) : null}

      {error ? (
        <StateMessage tone="danger" testId="add-friend-error" action={error.unreachable ? <ShareLinkButton /> : undefined}>{error.text}</StateMessage>
      ) : null}

      {!isGroup && !blocked ? (
        <div className="flex flex-wrap justify-end gap-2">
          {!relation.isContact ? (
            <Button variant="outlined" disabled={busy} onClick={form.handleSubmit(values => act('save', values))} data-testid="add-friend-save-contact">
              {t(pending === 'save' ? 'network.saving' : 'network.addFriend.saveOnly')}
            </Button>
          ) : relation.friendState !== 'none' || !befriendable ? (
            <Button variant="outlined" disabled={busy} onClick={form.handleSubmit(async (values) => { await other(() => applyTags(values.tags), 'saved') })}>
              {t('network.addFriend.saveTags')}
            </Button>
          ) : null}
          {relation.friendState === 'none' && befriendable ? (
            <Button type="submit" variant="contained" startIcon={<UserPlus size={14} />} disabled={busy || unreachable} data-testid="add-friend-send">
              {t(pending === 'friend' ? 'network.addFriend.sending' : 'network.addFriend.send')}
            </Button>
          ) : null}
        </div>
      ) : null}
    </form>
  )
}

function Target({ did, seed, params, onBack, onDone }: { did: string; seed?: LookupCandidate; params: AddFriendParams; onBack?: () => void; onDone: (result: AddFriendResult) => void }) {
  const { t } = useI18n()
  const lookup = useLookup(did, { debounceMs: 0 })
  const result = lookup.input === did.trim() ? lookup.result : undefined
  const candidate = result?.candidates.find(item => item.did === did) ?? result?.candidates[0] ?? seed
  if (!candidate && (!result || lookup.status === 'loading' || lookup.status === 'idle') && lookup.status !== 'error') return <ListSkeleton rows={2} />
  if (!candidate && lookup.status === 'error') return <StateMessage tone="danger" action={<Button variant="text" size="small" onClick={lookup.retry}>{t('network.retry')}</Button>}>{networkErrorText(t, lookup.error)}</StateMessage>
  if (!candidate) return <StateMessage tone="danger" testId="add-friend-lookup-error">{t(`network.lookup.${result?.error?.code ?? 'not-found'}`)}</StateMessage>
  return <ProfileStep candidate={candidate} initial={{ message: params.message ?? '', tags: params.tags ?? [] }} onBack={onBack} onDone={onDone} />
}

export function AddFriendDialog({ params, onDone }: { params: AddFriendParams; onDone: (result: AddFriendResult) => void }) {
  const { t } = useI18n()
  const [target, setTarget] = useState<{ did: string; seed?: LookupCandidate } | null>(params.did ? { did: params.did } : null)
  const [input, setInput] = useState(params.input ?? '')
  if (target) {
    return <Target did={target.did} seed={target.seed} params={params} onBack={params.did ? undefined : () => setTarget(null)} onDone={onDone} />
  }
  const open = (candidate: LookupCandidate) => setTarget({ did: candidate.did, seed: candidate })
  return (
    <div className="space-y-3" data-testid="add-friend-search">
      <p className="text-[13px] text-[color:var(--cp-muted)]">{t('network.addFriend.intro')}</p>
      <FindPeople
        initialInput={input}
        onInputChange={setInput}
        kinds={[...PEOPLE_KINDS]}
        showDefault={false}
        autoFocus
        placeholder={t('network.addFriend.placeholder')}
        onOpen={open}
        actions={[{ id: 'open', label: t('network.addFriend.view'), onSelect: open }]}
      />
      <div className="flex justify-end"><Button variant="text" onClick={() => onDone({ action: 'cancelled' })}>{t('network.cancel')}</Button></div>
    </div>
  )
}
