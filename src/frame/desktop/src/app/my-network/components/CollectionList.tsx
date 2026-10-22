/* ── Second column: the people of Contacts, Friends or one Collection — search, filter, sort, multi-select ── */

import { Button, Checkbox, ToggleButton, ToggleButtonGroup } from '@mui/material'
import { ExternalLink, FolderMinus, FolderPlus, Pencil, RefreshCw, Trash2, UserPlus } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { networkCache } from '../../../components/network/cache'
import { useNetworkResource } from '../../../components/network/hooks'
import { ListSkeleton, Pill, PersonRow, StateMessage } from '../../../components/network/Person'
import { RelationBadge } from '../../../components/network/RelationBadge'
import { networkErrorText, relationOfContact, tagKind, tagLabel, withPendingRequest } from '../../../components/network/relation'
import type { Contact } from '../../../components/network/types'
import { filterPeople, kindOfContact, peopleOfView } from '../datamodel/collections'
import { openHomeStation } from '../datamodel/links'
import type { MyNetworkView, PeopleFilter, PeopleSort } from '../datamodel/types'
import { useCollectionActions } from './collectionActions'
import { isProjectedContact, useContactEditPermission, useUserGroupNames } from '../../../components/network/permissions'
import { dangerTextSx } from '../../../components/network/buttonStyles'

const FILTERS: PeopleFilter[] = ['all', 'friends', 'others', 'blocked']

function secondaryOf(contact: Contact) {
  return contact.note || contact.bindings?.[0]?.display_id || contact.did
}

export function CollectionList({
  view,
  selectedDid,
  onSelect,
  onCollectionGone,
  wide = false,
}: {
  view: Extract<MyNetworkView, { kind: 'contacts' | 'friends' | 'collection' }>
  selectedDid: string | null
  onSelect: (did: string) => void
  onCollectionGone: (renamedTo?: string) => void
  wide?: boolean
}) {
  const { t } = useI18n()
  const collectionActions = useCollectionActions()
  const contacts = useNetworkResource('contacts')
  const friendRequests = useNetworkResource('friendRequests')
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<PeopleFilter>('all')
  const [sort, setSort] = useState<PeopleSort>('name')
  const [checked, setChecked] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const canEditContacts = useContactEditPermission()
  const groupNames = useUserGroupNames()
  const tag = view.kind === 'collection' ? view.tag : null
  const kind = tag ? tagKind(tag) : null
  const editable = kind === 'user' && canEditContacts
  const members = useMemo(() => peopleOfView(contacts.data ?? [], view), [contacts.data, view])
  const visible = useMemo(() => filterPeople(members, query, filter, sort), [members, query, filter, sort])
  const title = tag ? tagLabel(t, tag, groupNames) : t(`myNetwork.view.${view.kind}`)
  const selection = checked.filter(did => members.some(contact => contact.did === did))

  const guarded = async (run: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try { await run() } catch (failure) { setError(networkErrorText(t, failure)) } finally { setBusy(false) }
  }

  const addMembers = () => guarded(async () => {
    if (tag) await collectionActions.addMembers(tag, members)
  })

  const batchAdd = () => guarded(async () => {
    const target = await collectionActions.choose(contacts.data ?? [])
    if (!target) return
    await collectionActions.addTo(target, selection)
    setChecked([])
  })

  const batchRemove = () => guarded(async () => {
    if (tag && await collectionActions.removeMany(tag, selection)) setChecked([])
  })

  const rename = () => guarded(async () => {
    if (!tag) return
    const next = await collectionActions.rename(tag, members, contacts.data ?? [])
    if (next) onCollectionGone(next)
  })

  const remove = () => guarded(async () => {
    if (tag && await collectionActions.remove(tag, members)) onCollectionGone()
  })

  return (
    <div className={`flex h-full min-h-0 flex-col overflow-hidden ${wide ? 'w-full' : 'w-80 shrink-0'}`} style={wide ? undefined : { borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)' }} data-testid="my-network-list">
      <div className="space-y-1 px-4 pb-1 pt-3">
        <div className="flex items-center justify-between gap-2">
          <h2 className="truncate font-display text-sm font-semibold text-[color:var(--cp-text)]" data-testid="my-network-list-title">{title}</h2>
          <span className="shrink-0 text-[11px] tabular-nums text-[color:var(--cp-muted)]">{t('myNetwork.list.count', undefined, { count: members.length })}</span>
        </div>
        {!canEditContacts ? (
          <p className="text-[11px] leading-4 text-[color:var(--cp-muted)]" data-testid="my-network-read-only">{t('myNetwork.readOnly.list')}</p>
        ) : null}
        {tag ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <Pill tone={editable ? 'var(--cp-accent)' : 'var(--cp-muted)'}>{t(editable ? 'myNetwork.collection.static' : 'myNetwork.collection.view')}</Pill>
            {kind === 'app' ? <span className="text-[11px] text-[color:var(--cp-muted)]">{t('myNetwork.collection.appReadOnly')}</span> : null}
            {kind === 'system' ? <span className="text-[11px] text-[color:var(--cp-muted)]">{t('myNetwork.collection.systemReadOnly')}</span> : null}
          </div>
        ) : canEditContacts ? (
          <p className="text-[11px] leading-4 text-[color:var(--cp-muted)]">{t(`myNetwork.view.${view.kind}.hint`)}</p>
        ) : null}
        {editable ? (
          <div className="flex flex-wrap gap-1 pt-1">
            <Button size="small" variant="outlined" startIcon={<UserPlus size={13} />} disabled={busy} onClick={() => void addMembers()} data-testid="collection-add-members">{t('myNetwork.collection.add')}</Button>
            <Button variant="text" size="small" startIcon={<Pencil size={13} />} disabled={busy} onClick={() => void rename()} data-testid="collection-rename">{t('myNetwork.collection.rename')}</Button>
            <Button variant="text" size="small" sx={dangerTextSx} startIcon={<Trash2 size={13} />} disabled={busy} onClick={() => void remove()} data-testid="collection-delete">{t('myNetwork.collection.delete')}</Button>
          </div>
        ) : null}
        {kind === 'app' && tag?.startsWith('homestation.') ? (
          <Button size="small" variant="outlined" startIcon={<ExternalLink size={13} />} onClick={openHomeStation} data-testid="collection-manage-homestation">
            {t('myNetwork.collection.manageInHomeStation')}
          </Button>
        ) : null}
      </div>

      <div className="px-3 pt-1">
        <input
          type="search"
          value={query}
          onChange={event => setQuery(event.target.value)}
          placeholder={t('myNetwork.list.search', undefined, { name: title })}
          aria-label={t('myNetwork.list.search', undefined, { name: title })}
          className="min-h-9 w-full rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3 text-sm text-[color:var(--cp-text)]"
        />
        <div className="mt-2 flex flex-wrap items-center gap-1" role="group" aria-label={t('myNetwork.list.filter')}>
          {FILTERS.map(value => (
            <button
              key={value}
              type="button"
              aria-pressed={filter === value}
              onClick={() => setFilter(value)}
              className="min-h-7 rounded-full border px-2.5 text-[12px]"
              style={filter === value
                ? { background: 'color-mix(in srgb, var(--cp-accent-soft) 22%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-accent) 26%, var(--cp-border))', color: 'var(--cp-text)' }
                : { borderColor: 'var(--cp-border)', color: 'var(--cp-muted)' }}
            >
              {t(`myNetwork.filter.${value}`)}
            </button>
          ))}
        </div>
        <ToggleButtonGroup className="mt-2" value={sort} exclusive size="small" fullWidth onChange={(_, value: PeopleSort | null) => { if (value) setSort(value) }}>
          <ToggleButton value="name">{t('myNetwork.sort.name')}</ToggleButton>
          <ToggleButton value="recent">{t('myNetwork.sort.recent')}</ToggleButton>
        </ToggleButtonGroup>
      </div>

      {selection.length > 0 ? (
        <div className="mx-3 mt-2 rounded-xl border px-2 py-2" style={{ background: 'color-mix(in srgb, var(--cp-accent-soft) 12%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))' }} data-testid="my-network-batch">
          <div className="mb-1.5 text-[12px] font-medium text-[color:var(--cp-text)]">{t('myNetwork.list.selected', undefined, { count: selection.length })}</div>
          <div className="flex flex-wrap gap-1.5">
            <Button size="small" variant="outlined" startIcon={<FolderPlus size={13} />} disabled={busy} onClick={() => void batchAdd()}>{t('myNetwork.list.addToCollection')}</Button>
            {editable ? <Button size="small" variant="outlined" startIcon={<FolderMinus size={13} />} disabled={busy} onClick={() => void batchRemove()}>{t('network.action.remove_from_collection')}</Button> : null}
            <Button variant="text" size="small" disabled={busy} onClick={() => setChecked([])}>{t('myNetwork.list.clearSelection')}</Button>
          </div>
        </div>
      ) : null}
      {error ? <div className="mx-3 mt-2"><StateMessage tone="danger">{error}</StateMessage></div> : null}

      <div className="desktop-scrollbar mt-1 min-h-0 flex-1 overflow-y-auto px-1.5 pb-2" data-testid="my-network-people">
        {contacts.status === 'loading' && !contacts.data ? <ListSkeleton rows={6} /> : null}
        {contacts.status === 'error' ? (
          <div className="px-1.5 pt-2">
            <StateMessage tone="danger" testId="my-network-list-error" action={<Button variant="text" size="small" startIcon={<RefreshCw size={13} />} onClick={() => networkCache.refresh('contacts')}>{t('network.retry')}</Button>}>
              {networkErrorText(t, contacts.error)}
            </StateMessage>
          </div>
        ) : null}
        {contacts.data && visible.length === 0 ? (
          <p className="px-4 py-8 text-center text-sm text-[color:var(--cp-muted)]" data-testid="my-network-list-empty">
            {query || filter !== 'all' ? t('myNetwork.list.noMatches') : t(tag ? 'myNetwork.list.emptyCollection' : `myNetwork.list.empty.${view.kind}`)}
          </p>
        ) : null}
        {visible.map(contact => (
          <PersonRow
            key={contact.did}
            testId="my-network-person"
            person={{ did: contact.did, name: contact.name, avatar: contact.avatar, kind: kindOfContact(contact) }}
            selected={contact.did === selectedDid}
            onClick={() => onSelect(contact.did)}
            secondary={secondaryOf(contact)}
            leading={canEditContacts ? (
              <Checkbox
                size="small"
                checked={checked.includes(contact.did)}
                onChange={() => setChecked(current => current.includes(contact.did) ? current.filter(item => item !== contact.did) : [...current, contact.did])}
                inputProps={{ 'aria-label': t('myNetwork.list.select', undefined, { name: contact.name || contact.did }) }}
                sx={{ p: 0.5 }}
              />
            ) : undefined}
            trailing={(
              <span className="flex shrink-0 items-center gap-1">
                {isProjectedContact(contact) ? <Pill tone="var(--cp-muted)" testId="person-projected">{t('myNetwork.projected.badge')}</Pill> : null}
                <RelationBadge relation={withPendingRequest(relationOfContact(contact), contact.did, friendRequests.data)} compact />
              </span>
            )}
          />
        ))}
      </div>
    </div>
  )
}
