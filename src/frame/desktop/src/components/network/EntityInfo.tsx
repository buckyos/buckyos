/* ── EntityInfo: who a DID is — identity, the owner's relation with it, how it is reached — and the entry to message it ── */

import { Button } from '@mui/material'
import { MessageSquare } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { ObjectTitle } from '../../presentation/ObjectTitle'
import { useLookup, useNetworkResource, useOwnerDid } from './hooks'
import { KindLabel, ListSkeleton, PersonAvatar, Pill, ReachabilityChip, StateMessage } from './Person'
import { usePersonPresentation } from './personPresentation'
import { useContactEditPermission } from './permissions'
import { NOT_RELATED, networkErrorText, relationOfCandidate, relationOfContact, withPendingRequest } from './relation'
import { RelationActions } from './RelationActions'
import { RelationBadge } from './RelationBadge'
import type { EntityInfoParams, PickedPerson } from './schemas'

export interface EntityInfoProps {
  /** `name`, `avatar` and `kind` are what the host already shows; the contact record and `contact.lookup` win where they know more. */
  params: EntityInfoParams
  /** "Send message"; without it the entry is not offered. */
  onMessage?: (person: PickedPerson) => void
}

export function EntityInfo({ params, onMessage }: EntityInfoProps) {
  const { t } = useI18n()
  const { did } = params
  const ownerDid = useOwnerDid()
  const contacts = useNetworkResource('contacts')
  const friendRequests = useNetworkResource('friendRequests')
  const lookup = useLookup(did, { debounceMs: 0 })
  const canEditContacts = useContactEditPermission()
  const contact = contacts.data?.find(item => item.did === did)
  const candidate = lookup.result?.candidates.find(item => item.did === did)
  const kind = candidate?.kind ?? params.kind ?? 'user'
  const person = { did, name: contact?.name || candidate?.name || params.name, avatar: contact?.avatar || candidate?.avatar || params.avatar, kind }
  const title = usePersonPresentation(person).title
  const isSelf = ownerDid === did
  const pending = !contact && !candidate && (contacts.status === 'loading' || lookup.status === 'loading' || lookup.status === 'idle')
  const relation = withPendingRequest(contact ? relationOfContact(contact) : candidate ? relationOfCandidate(candidate) : NOT_RELATED, did, friendRequests.data)
  const reach = candidate?.reachability
  const related = kind !== 'group' && !isSelf
  const failure = contact || candidate || pending ? null
    : lookup.result?.error ? t(`network.lookup.${lookup.result.error.code}`)
      : lookup.status === 'error' ? networkErrorText(t, lookup.error) : null

  return (
    <div className="min-w-0 space-y-4" data-testid="entity-info" data-did={did}>
      <div className="flex items-start gap-4">
        <PersonAvatar person={person} size={64} />
        <div className="min-w-0 flex-1 space-y-2">
          <ObjectTitle title={title} did={did} showDid size="lg" />
          <div className="text-xs text-[color:var(--cp-muted)]" data-testid="entity-info-kind">
            <KindLabel kind={kind} />{reach?.zone ? ` · ${t('network.profile.zone', undefined, { zone: reach.zone })}` : ''}
          </div>
          <div className="flex flex-wrap items-center gap-1.5 empty:hidden">
            {isSelf ? <Pill tone="var(--cp-accent)" testId="entity-info-self">{t('network.entityInfo.you')}</Pill> : null}
            {related && !pending ? <RelationBadge relation={relation} /> : null}
            {reach && !isSelf ? <ReachabilityChip reachability={reach} /> : null}
            {contact?.is_verified ? <Pill tone="var(--cp-success)">{t('network.entityInfo.verified')}</Pill> : null}
          </div>
        </div>
      </div>

      {pending ? <ListSkeleton rows={2} /> : null}
      {failure ? <StateMessage testId="entity-info-unresolved">{failure}</StateMessage> : null}
      {contact?.note ? <p className="whitespace-pre-wrap break-words rounded-xl bg-[color:color-mix(in_srgb,var(--cp-text)_4%,transparent)] px-3 py-2 text-sm text-[color:var(--cp-text)]" data-testid="entity-info-note">{contact.note}</p> : null}

      {onMessage && !isSelf ? (
        <Button
          size="small"
          variant="contained"
          startIcon={<MessageSquare size={14} />}
          onClick={() => onMessage({ did, kind, name: title, ...(person.avatar ? { avatar: person.avatar } : {}), ...(reach?.zone ? { zone: reach.zone } : {}), isContact: relation.isContact, friendState: relation.friendState })}
          data-testid="entity-info-message"
        >
          {t('network.entityInfo.sendMessage')}
        </Button>
      ) : null}

      {related && !pending && canEditContacts ? (
        <section className="space-y-2" data-testid="entity-info-relationship">
          <h3 className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">{t('network.entityInfo.relationship')}</h3>
          <RelationActions person={person} relation={relation} exclude={['remove_from_collection']} />
        </section>
      ) : null}
    </div>
  )
}
