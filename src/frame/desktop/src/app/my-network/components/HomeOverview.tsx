/* ── Home (PRD §6.1): counts, recently added people, Requests, Collections and the two entry actions ── */

import { Button } from '@mui/material'
import { Inbox, UserPlus, Users2 } from 'lucide-react'
import type { ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useNetworkResource, useRequests } from '../../../components/network/hooks'
import { ListSkeleton, PersonRow, StateMessage } from '../../../components/network/Person'
import { RelationBadge } from '../../../components/network/RelationBadge'
import { actionableCount, networkErrorText, relationOfContact, tagLabel } from '../../../components/network/relation'
import { kindOfContact, recentlyAdded } from '../datamodel/collections'
import type { CollectionSections, MyNetworkView } from '../datamodel/types'

function Metric({ label, value, onClick, testId }: { label: string; value: ReactNode; onClick: () => void; testId: string }) {
  return (
    <button type="button" onClick={onClick} className="shell-subtle-panel min-w-0 px-4 py-3 text-left" data-testid={testId}>
      <div className="truncate text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">{label}</div>
      <div className="mt-1 font-display text-2xl font-semibold tabular-nums text-[color:var(--cp-text)]">{value}</div>
    </button>
  )
}

export function HomeOverview({
  collections,
  onNavigate,
  onOpenPerson,
  onAddFriend,
  onJoinGroup,
}: {
  collections: CollectionSections
  onNavigate: (view: MyNetworkView) => void
  onOpenPerson: (did: string) => void
  onAddFriend: () => void
  onJoinGroup: () => void
}) {
  const { t } = useI18n()
  const contacts = useNetworkResource('contacts')
  const groups = useNetworkResource('groups')
  const requests = useRequests()
  const count = (value: number | undefined) => value ?? '–'
  const friends = contacts.data?.filter(contact => contact.friend_state === 'friend').length
  const waiting = actionableCount(requests.items)
  const recent = recentlyAdded(contacts.data ?? [])

  return (
    <div className="desktop-scrollbar h-full min-w-0 flex-1 overflow-y-auto" data-testid="my-network-home">
      <div className="mx-auto max-w-3xl space-y-5 px-5 py-5">
        <section className="shell-panel px-5 py-5">
          <h2 className="font-display text-xl font-semibold text-[color:var(--cp-text)]">{t('myNetwork.home.title')}</h2>
          <p className="mt-1 text-sm leading-6 text-[color:var(--cp-muted)]">{t('myNetwork.home.body')}</p>
          <div className="mt-4 flex flex-wrap gap-2">
            <Button variant="contained" startIcon={<UserPlus size={15} />} onClick={onAddFriend}>{t('myNetwork.addFriend')}</Button>
            <Button variant="outlined" startIcon={<Users2 size={15} />} onClick={onJoinGroup}>{t('myNetwork.joinGroup')}</Button>
          </div>
        </section>

        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <Metric label={t('myNetwork.view.contacts')} value={count(contacts.data?.length)} onClick={() => onNavigate({ kind: 'contacts' })} testId="metric-contacts" />
          <Metric label={t('myNetwork.view.friends')} value={count(friends)} onClick={() => onNavigate({ kind: 'friends' })} testId="metric-friends" />
          <Metric label={t('myNetwork.view.groups')} value={count(groups.data?.filter(group => group.state === 'active').length)} onClick={() => onNavigate({ kind: 'groups' })} testId="metric-groups" />
          <Metric label={t('myNetwork.view.requests')} value={requests.status === 'loading' ? '–' : waiting} onClick={() => onNavigate({ kind: 'requests' })} testId="metric-requests" />
        </div>

        {waiting > 0 ? (
          <StateMessage tone="warning" testId="home-requests" action={<Button variant="text" size="small" startIcon={<Inbox size={13} />} onClick={() => onNavigate({ kind: 'requests' })}>{t('myNetwork.home.reviewRequests')}</Button>}>
            {t('myNetwork.home.requestsWaiting', undefined, { count: waiting })}
          </StateMessage>
        ) : null}

        <section>
          <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">{t('myNetwork.home.recent')}</h3>
          {contacts.status === 'loading' && !contacts.data ? <ListSkeleton rows={3} /> : null}
          {contacts.status === 'error' ? <StateMessage tone="danger">{networkErrorText(t, contacts.error)}</StateMessage> : null}
          {contacts.data && recent.length === 0 ? <StateMessage testId="home-empty">{t('myNetwork.home.empty')}</StateMessage> : null}
          <div className="space-y-0.5">
            {recent.map(contact => (
              <PersonRow
                key={contact.did}
                person={{ did: contact.did, name: contact.name, avatar: contact.avatar, kind: kindOfContact(contact) }}
                onClick={() => onOpenPerson(contact.did)}
                trailing={<RelationBadge relation={relationOfContact(contact)} compact />}
              />
            ))}
          </div>
        </section>

        <section>
          <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">{t('myNetwork.collections')}</h3>
          <div className="flex flex-wrap gap-1.5">
            {[...collections.user, ...collections.app].map(item => (
              <button
                key={item.tag}
                type="button"
                onClick={() => onNavigate({ kind: 'collection', tag: item.tag })}
                className="min-h-8 rounded-full border border-[color:var(--cp-border)] px-3 text-[13px] text-[color:var(--cp-text)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,var(--cp-surface))]"
              >
                {tagLabel(t, item.tag)} <span className="tabular-nums text-[color:var(--cp-muted)]">{item.count}</span>
              </button>
            ))}
          </div>
        </section>
      </div>
    </div>
  )
}
