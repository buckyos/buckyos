/* ── Third column: one person — identity, relation, Collections, reachability, source ── */

import { Button } from '@mui/material'
import { ExternalLink, Home, MessageSquare } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { ObjectTitle } from '../../../presentation/ObjectTitle'
import { networkActions } from '../../../components/network/cache'
import { useNetworkDialogs } from '../../../components/network/dialogs'
import { useLookup, useNetworkResource } from '../../../components/network/hooks'
import { KindLabel, ListSkeleton, Pill, PersonAvatar, ReachabilityChip, StateMessage } from '../../../components/network/Person'
import { usePersonPresentation } from '../../../components/network/personPresentation'
import { RelationActions } from '../../../components/network/RelationActions'
import { RelationBadge } from '../../../components/network/RelationBadge'
import { NOT_RELATED, networkErrorText, relationOfCandidate, relationOfContact, tagKind, tagLabel, timeMs, userTags, withPendingRequest, type RelationActionId, type RelationFacts } from '../../../components/network/relation'
import { TagEditor } from '../../../components/network/TagEditor'
import { isProjectedContact, useContactEditPermission, useUserGroupNames } from '../../../components/network/permissions'
import type { Contact, LookupCandidate } from '../../../components/network/types'
import { kindOfContact } from '../datamodel/collections'
import { homePageUrl, openHomeStation, openMessageHub } from '../datamodel/links'

function Card({ title, children, testId }: { title: string; children: ReactNode; testId?: string }) {
  return (
    <section className="rounded-[22px] border px-5 py-4" style={{ background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-border) 50%, transparent)' }} data-testid={testId}>
      <h3 className="mb-3 font-display text-sm font-semibold text-[color:var(--cp-text)]">{title}</h3>
      {children}
    </section>
  )
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5 py-1 sm:flex-row sm:items-baseline sm:gap-3">
      <span className="w-32 shrink-0 text-[12px] font-medium text-[color:var(--cp-muted)]">{label}</span>
      <span className="min-w-0 flex-1 break-words text-sm text-[color:var(--cp-text)]">{children}</span>
    </div>
  )
}

function CollectionsCard({ contact, suggestions, editable, groupNames }: { contact: Contact; suggestions: string[]; editable: boolean; groupNames: ReadonlyMap<string, string> }) {
  const { t } = useI18n()
  const dialogs = useNetworkDialogs()
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const tags = contact.tags ?? []
  const own = userTags(tags)
  const views = tags.filter(tag => tagKind(tag) !== 'user')

  const change = async (next: string[]) => {
    const add = next.filter(tag => !own.includes(tag))
    const remove = own.filter(tag => !next.includes(tag))
    if (remove.length) {
      const confirmed = await dialogs.confirm({
        strength: 'light',
        title: t('network.confirm.removeFromCollection.title'),
        body: t('network.confirm.removeFromCollection.body', undefined, { name: contact.name || contact.did, collection: remove.join(', ') }),
        confirmLabel: t('network.action.remove_from_collection'),
      })
      if (!confirmed) return
    }
    setBusy(true)
    setError(null)
    try {
      await networkActions.updateTags(contact.did, { add, remove })
    } catch (failure) {
      setError(networkErrorText(t, failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card title={t('myNetwork.detail.collections')} testId="contact-collections">
      <div className="space-y-3">
        {editable ? (
          <TagEditor value={own} onChange={next => void change(next)} suggestions={suggestions} disabled={busy} label={t('myNetwork.collection.section.user')} />
        ) : own.length ? (
          <div className="flex flex-wrap items-center gap-1.5" data-testid="contact-user-tags">
            {own.map(tag => <Pill key={tag} tone="var(--cp-accent)">{tag}</Pill>)}
          </div>
        ) : null}
        {views.length ? (
          <div className="space-y-1.5">
            <div className="text-[12px] font-medium text-[color:var(--cp-muted)]">{t('myNetwork.detail.views')}</div>
            <div className="flex flex-wrap items-center gap-1.5">
              {views.map(tag => <Pill key={tag} tone="var(--cp-muted)">{tagLabel(t, tag, groupNames)}</Pill>)}
              {views.some(tag => tag.startsWith('homestation.')) ? (
                <Button variant="text" size="small" startIcon={<ExternalLink size={13} />} onClick={openHomeStation}>{t('myNetwork.collection.manageInHomeStation')}</Button>
              ) : null}
            </div>
          </div>
        ) : null}
        {error ? <StateMessage tone="danger">{error}</StateMessage> : null}
      </div>
    </Card>
  )
}

function Header({ person, contact, candidate, relation }: { person: { did: string; name?: string; avatar?: string; kind: LookupCandidate['kind'] }; contact?: Contact; candidate?: LookupCandidate; relation: RelationFacts }) {
  const { t } = useI18n()
  const title = usePersonPresentation(person).title
  const reach = candidate?.reachability
  const home = homePageUrl({ did: person.did, kind: person.kind, zone: reach?.zone, local: reach?.state === 'local' })
  return (
    <div className="flex flex-col gap-4 rounded-[22px] border px-5 py-5 sm:flex-row sm:items-start" style={{ background: 'color-mix(in srgb, var(--cp-surface-2) 50%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-border) 60%, transparent)' }} data-testid="contact-header">
      <PersonAvatar person={person} size={56} />
      <div className="min-w-0 flex-1 space-y-2">
        <ObjectTitle title={title} did={person.did} showDid size="lg" />
        <div className="text-xs text-[color:var(--cp-muted)]">
          <KindLabel kind={person.kind} />{reach?.zone ? ` · ${t('network.profile.zone', undefined, { zone: reach.zone })}` : ''}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {person.kind !== 'group' ? <RelationBadge relation={relation} /> : null}
          {reach ? <ReachabilityChip reachability={reach} /> : null}
          {contact?.is_verified ? <Pill tone="var(--cp-success)">{t('myNetwork.detail.verified')}</Pill> : null}
        </div>
        <div className="flex flex-wrap gap-2 pt-1">
          <Button size="small" variant="contained" startIcon={<MessageSquare size={14} />} onClick={() => void openMessageHub(person.did)} data-testid="contact-send-message">
            {t('myNetwork.detail.sendMessage')}
          </Button>
          {home ? (
            <Button size="small" variant="outlined" startIcon={<Home size={14} />} href={home} target="_blank" rel="noreferrer" data-testid="contact-home-page">
              {t('myNetwork.detail.homePage')}
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  )
}

export function ContactDetailPage({ did, collectionTag, onRemoved }: { did: string; collectionTag?: string; onRemoved?: () => void }) {
  const { t } = useI18n()
  const contacts = useNetworkResource('contacts')
  const friendRequests = useNetworkResource('friendRequests')
  const lookup = useLookup(did, { debounceMs: 0 })
  const canEditContacts = useContactEditPermission()
  const groupNames = useUserGroupNames()
  const contact = contacts.data?.find(item => item.did === did)
  const candidate = lookup.result?.candidates.find(item => item.did === did)

  if (!contact && !candidate) {
    if (contacts.status === 'loading' || lookup.status === 'loading' || lookup.status === 'idle') return <ListSkeleton rows={3} />
    return (
      <StateMessage tone="danger" testId="contact-missing">
        {lookup.result?.error ? t(`network.lookup.${lookup.result.error.code}`) : lookup.status === 'error' ? networkErrorText(t, lookup.error) : t('myNetwork.detail.missing')}
      </StateMessage>
    )
  }

  const kind = candidate?.kind ?? (contact ? kindOfContact(contact) : 'unknown')
  const person = { did, name: contact?.name || candidate?.name, avatar: contact?.avatar || candidate?.avatar, kind }
  const base = contact ? relationOfContact(contact) : candidate ? relationOfCandidate(candidate) : NOT_RELATED
  const relation = withPendingRequest(base, did, friendRequests.data)
  const suggestions = [...new Set((contacts.data ?? []).flatMap(item => userTags(item.tags ?? [])))].sort()
  const created = timeMs(contact?.created_at)
  const updated = timeMs(contact?.updated_at)
  const done = (action: RelationActionId) => {
    if (action === 'remove_contact') onRemoved?.()
  }

  return (
    <div className="min-w-0 space-y-4" data-testid="contact-detail" data-did={did}>
      <Header person={person} contact={contact} candidate={candidate} relation={relation} />

      <Card title={t('myNetwork.detail.relationship')} testId="contact-relationship">
        {!contact ? <p className="mb-3 text-[13px] text-[color:var(--cp-muted)]">{t('myNetwork.detail.notContact')}</p> : null}
        {isProjectedContact(contact) ? (
          <p className="mb-3 text-[13px] text-[color:var(--cp-muted)]" data-testid="contact-projected">
            {t('myNetwork.detail.projected', undefined, { groups: (contact?.projected_from ?? []).map(tag => tagLabel(t, tag, groupNames)).join(', ') })}
          </p>
        ) : null}
        {canEditContacts ? (
          <RelationActions person={person} relation={relation} collectionTag={collectionTag} onDone={done} />
        ) : (
          <p className="text-[13px] text-[color:var(--cp-muted)]" data-testid="contact-read-only">{t('myNetwork.readOnly.detail')}</p>
        )}
      </Card>

      {contact ? (
        <CollectionsCard
          contact={contact}
          suggestions={suggestions}
          editable={canEditContacts}
          groupNames={groupNames}
        />
      ) : null}

      <Card title={t('myNetwork.detail.reachability')}>
        <Fact label="DID"><span className="break-all font-mono text-[12px]">{did}</span></Fact>
        {candidate?.reachability.zone ? <Fact label={t('myNetwork.detail.zone')}>{candidate.reachability.zone}</Fact> : null}
        {candidate ? (
          <Fact label={t('myNetwork.detail.delivery')}>
            {t(`network.reach.${candidate.reachability.state}`)}
            {candidate.reachability.reason ? ` · ${t(`network.reason.${candidate.reachability.reason}`, candidate.reachability.reason)}` : ''}
          </Fact>
        ) : null}
        {(contact?.bindings ?? []).map(binding => (
          <Fact key={`${binding.platform}:${binding.account_id}`} label={binding.platform}>{binding.display_id}</Fact>
        ))}
      </Card>

      {contact ? (
        <Card title={t('myNetwork.detail.source')}>
          <Fact label={t('myNetwork.detail.sourceLabel')}>{t(`myNetwork.source.${contact.source}`, contact.source)}</Fact>
          {created ? <Fact label={t('myNetwork.detail.created')}>{new Date(created).toLocaleString()}</Fact> : null}
          {updated ? <Fact label={t('myNetwork.detail.updated')}>{new Date(updated).toLocaleString()}</Fact> : null}
        </Card>
      ) : null}

      {contact?.note ? (
        <Card title={t('myNetwork.detail.note')}>
          <p className="text-sm text-[color:var(--cp-text)]">{contact.note}</p>
        </Card>
      ) : null}
    </div>
  )
}
