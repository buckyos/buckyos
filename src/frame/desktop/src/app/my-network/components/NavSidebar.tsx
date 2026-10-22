/* ── First column (PRD §5.3): overview, the built-in views, Requests and the three Collection sections ── */

import { Button, IconButton } from '@mui/material'
import { Bot, Contact, Eye, FolderOpen, Heart, Home, Inbox, Plus, ShieldCheck, UserPlus, Users2, UsersRound } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { tagLabel } from '../../../components/network/relation'
import { useContactEditPermission, useUserGroupNames } from '../../../components/network/permissions'
import { parseUserGroupRef } from '../../../api/user_type'
import type { CollectionSections, CollectionSummary, MyNetworkView } from '../datamodel/types'
import { NavItem } from './CollectionCard'

export interface NavCounts {
  contacts?: number
  friends?: number
  groups?: number
  requests?: number
}

function sameView(a: MyNetworkView, b: MyNetworkView) {
  return a.kind === b.kind && (a.kind !== 'collection' || (b.kind === 'collection' && a.tag === b.tag))
}

function collectionIcon(item: CollectionSummary) {
  if (item.kind === 'user') return <FolderOpen size={14} />
  if (item.kind === 'app') return <Eye size={14} />
  if (parseUserGroupRef(item.tag)) return <UsersRound size={14} />
  return item.tag === 'zone_agent' ? <Bot size={14} /> : <ShieldCheck size={14} />
}

export function NavSidebar({
  view,
  counts,
  collections,
  onSelect,
  onAddFriend,
  onJoinGroup,
  onCreateCollection,
  wide = false,
}: {
  view: MyNetworkView | null
  counts: NavCounts
  collections: CollectionSections
  onSelect: (view: MyNetworkView) => void
  onAddFriend: () => void
  onJoinGroup: () => void
  onCreateCollection: () => void
  wide?: boolean
}) {
  const { t } = useI18n()
  const canEditContacts = useContactEditPermission()
  const groupNames = useUserGroupNames()
  const item = (target: MyNetworkView, icon: React.ReactNode, label: string, extra: { count?: number; badge?: number; hint?: string; testId?: string } = {}) => (
    <NavItem key={target.kind === 'collection' ? `c:${target.tag}` : target.kind} icon={icon} label={label} active={view ? sameView(view, target) : false} onClick={() => onSelect(target)} {...extra} />
  )
  const collectionItem = (summary: CollectionSummary) => item(
    { kind: 'collection', tag: summary.tag },
    collectionIcon(summary),
    tagLabel(t, summary.tag, groupNames),
    { count: summary.count, testId: `nav-collection-${summary.tag}`, hint: summary.kind === 'app' ? t('myNetwork.collection.appHint', undefined, { app: summary.app ? t(`apps.${summary.app}`, summary.app) : '' }) : undefined },
  )

  return (
    <nav
      aria-label={t('myNetwork.nav')}
      className={`desktop-scrollbar flex h-full shrink-0 flex-col overflow-y-auto ${wide ? 'w-full' : 'w-64'}`}
      style={wide ? undefined : { borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)' }}
      data-testid="my-network-nav"
    >
      <div className="px-3 pb-2 pt-3">
        <div className="font-display text-base font-semibold text-[color:var(--cp-text)]">{t('apps.myNetwork', 'My Network')}</div>
        <div className={`mt-3 grid gap-2 ${canEditContacts ? 'grid-cols-2' : 'grid-cols-1'}`}>
          {canEditContacts ? (
            <Button size="small" variant="contained" className="whitespace-nowrap" startIcon={<UserPlus size={14} />} onClick={onAddFriend} data-testid="my-network-add-friend" sx={{ paddingInline: 1.25 }}>
              {t('myNetwork.addFriend')}
            </Button>
          ) : null}
          <Button size="small" variant="outlined" className="whitespace-nowrap" startIcon={<Users2 size={14} />} onClick={onJoinGroup} data-testid="my-network-join-group" sx={{ paddingInline: 1.25 }}>
            {t('myNetwork.joinGroup')}
          </Button>
        </div>
        {!canEditContacts ? (
          <p className="mt-2 text-[11px] leading-4 text-[color:var(--cp-muted)]" data-testid="my-network-nav-read-only">{t('myNetwork.readOnly.nav')}</p>
        ) : null}
      </div>

      <div className="space-y-0.5 px-2">
        {item({ kind: 'home' }, <Home size={14} />, t('myNetwork.view.home'), { testId: 'nav-home' })}
        {item({ kind: 'contacts' }, <Contact size={14} />, t('myNetwork.view.contacts'), { count: counts.contacts, testId: 'nav-contacts' })}
        {item({ kind: 'friends' }, <Heart size={14} />, t('myNetwork.view.friends'), { count: counts.friends, testId: 'nav-friends' })}
        {item({ kind: 'requests' }, <Inbox size={14} />, t('myNetwork.view.requests'), { badge: counts.requests || undefined, count: counts.requests ? undefined : 0, testId: 'nav-requests' })}
        {item({ kind: 'groups' }, <Users2 size={14} />, t('myNetwork.view.groups'), { count: counts.groups, testId: 'nav-groups' })}
      </div>

      <div className="mt-3 flex items-center justify-between px-4">
        <span className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">{t('myNetwork.collections')}</span>
        {canEditContacts ? (
          <IconButton size="small" aria-label={t('myNetwork.collection.create')} onClick={onCreateCollection} data-testid="my-network-create-collection">
            <Plus size={14} />
          </IconButton>
        ) : null}
      </div>
      <div className="space-y-2 px-2 pb-4" data-testid="my-network-collections">
        <section aria-label={t('myNetwork.collection.section.user')}>
          <div className="px-2 pb-0.5 pt-1 text-[11px] text-[color:var(--cp-muted)]">{t('myNetwork.collection.section.user')}</div>
          {collections.user.length ? collections.user.map(collectionItem) : (
            <p className="px-2 py-1 text-[12px] text-[color:var(--cp-muted)]">{t('myNetwork.collection.noneYet')}</p>
          )}
        </section>
        <section aria-label={t('myNetwork.collection.section.app')}>
          <div className="px-2 pb-0.5 pt-1 text-[11px] text-[color:var(--cp-muted)]">{t('myNetwork.collection.section.app')}</div>
          {collections.app.map(collectionItem)}
        </section>
        <section aria-label={t('myNetwork.collection.section.system')}>
          <div className="px-2 pb-0.5 pt-1 text-[11px] text-[color:var(--cp-muted)]">{t('myNetwork.collection.section.system')}</div>
          {collections.system.map(collectionItem)}
        </section>
      </div>
    </nav>
  )
}
