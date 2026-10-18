/* ── Joined Groups (PRD §8): groups from `group.list_by_member`; management lives in MessageHub's group panel ── */

import { Button } from '@mui/material'
import { LogOut, MessageSquare, RefreshCw, Settings2, Users } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useI18n } from '../../../i18n/provider'
import { ObjectAvatar } from '../../../presentation/ObjectIcon'
import { ObjectTitle } from '../../../presentation/ObjectTitle'
import { networkActions, networkCache } from '../../../components/network/cache'
import { useNetworkDialogs } from '../../../components/network/dialogs'
import { useLookup, useNetworkResource } from '../../../components/network/hooks'
import { ListSkeleton, Pill, PersonName, StateMessage } from '../../../components/network/Person'
import { personName } from '../../../components/network/personPresentation'
import { networkErrorText } from '../../../components/network/relation'
import type { GroupRef } from '../../../components/network/types'
import { openMessageHub } from '../datamodel/links'
import { dangerOutlinedSx } from '../../../components/network/buttonStyles'

function groupName(group: GroupRef) {
  return group.name || personName({ did: group.did })
}

function GroupAvatar({ group, size }: { group: GroupRef; size: number }) {
  return <ObjectAvatar title={groupName(group)} icons={[group.avatar]} size={size} shape="square" tone="var(--cp-warning)" fallbackIcon={<Users size={Math.round(size * 0.48)} />} decorative />
}

export function JoinedGroupList({ selectedDid, onSelect, onJoinGroup, wide = false }: { selectedDid: string | null; onSelect: (did: string) => void; onJoinGroup: () => void; wide?: boolean }) {
  const { t } = useI18n()
  const groups = useNetworkResource('groups')
  return (
    <div className={`flex h-full min-h-0 flex-col overflow-hidden ${wide ? 'w-full' : 'w-80 shrink-0'}`} style={wide ? undefined : { borderRight: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)' }} data-testid="my-network-groups">
      <div className="space-y-1 px-4 pb-2 pt-3">
        <div className="flex items-center justify-between gap-2">
          <h2 className="font-display text-sm font-semibold text-[color:var(--cp-text)]">{t('myNetwork.view.groups')}</h2>
          <span className="text-[11px] tabular-nums text-[color:var(--cp-muted)]">{t('myNetwork.list.count', undefined, { count: groups.data?.length ?? 0 })}</span>
        </div>
        <p className="text-[11px] leading-4 text-[color:var(--cp-muted)]">{t('myNetwork.view.groups.hint')}</p>
      </div>
      <div className="desktop-scrollbar min-h-0 flex-1 space-y-0.5 overflow-y-auto px-1.5 pb-2">
        {groups.status === 'loading' && !groups.data ? <ListSkeleton rows={4} /> : null}
        {groups.status === 'error' ? (
          <StateMessage tone="danger" action={<Button variant="text" size="small" startIcon={<RefreshCw size={13} />} onClick={() => networkCache.refresh('groups')}>{t('network.retry')}</Button>}>
            {networkErrorText(t, groups.error)}
          </StateMessage>
        ) : null}
        {groups.data && groups.data.length === 0 ? (
          <div className="space-y-3 px-3 py-8 text-center">
            <p className="text-sm text-[color:var(--cp-muted)]">{t('myNetwork.groups.empty')}</p>
            <Button size="small" variant="outlined" onClick={onJoinGroup}>{t('myNetwork.joinGroup')}</Button>
          </div>
        ) : null}
        {(groups.data ?? []).map(group => {
          const active = group.did === selectedDid
          return (
            <button
              key={group.did}
              type="button"
              onClick={() => onSelect(group.did)}
              aria-label={groupName(group)}
              data-testid="my-network-group"
              className="flex min-h-12 w-full items-center gap-2.5 rounded-xl border px-2.5 py-1.5 text-left"
              style={active
                ? { background: 'color-mix(in srgb, var(--cp-accent-soft) 20%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))' }
                : { borderColor: 'transparent' }}
            >
              <GroupAvatar group={group} size={34} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium text-[color:var(--cp-text)]">{groupName(group)}</span>
                <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">
                  {group.state === 'active' ? t(`myNetwork.groups.role.${group.role ?? 'member'}`) : t(`myNetwork.groups.state.${group.state}`)}
                </span>
              </span>
              {group.state !== 'active' ? <Pill tone="var(--cp-warning)">{t(`myNetwork.groups.state.${group.state}`)}</Pill> : group.hostedHere ? <Pill tone="var(--cp-success)">{t('network.group.hostedHere')}</Pill> : null}
            </button>
          )
        })}
      </div>
    </div>
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

export function JoinedGroupDetail({ did, onLeft }: { did: string; onLeft: () => void }) {
  const { t } = useI18n()
  const dialogs = useNetworkDialogs()
  const groups = useNetworkResource('groups')
  const lookup = useLookup(did, { kinds: ['group'], debounceMs: 0 })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const group = groups.data?.find(item => item.did === did)
  const memberCount = group?.memberCount ?? lookup.result?.candidates.find(item => item.did === did)?.group?.member_count
  if (!group) {
    if (groups.status === 'loading') return <ListSkeleton rows={3} />
    return <StateMessage testId="group-missing">{t('myNetwork.groups.missing')}</StateMessage>
  }
  const name = groupName(group)

  const leave = async () => {
    const confirmed = await dialogs.confirm({
      strength: 'explicit',
      danger: true,
      title: t('myNetwork.groups.leaveTitle', undefined, { name }),
      body: t('myNetwork.groups.leaveBody', undefined, { name }),
      consequences: [t('myNetwork.groups.leaveC1'), t(group.joinPolicy === 'open' ? 'myNetwork.groups.leaveC2open' : 'myNetwork.groups.leaveC2')],
      confirmLabel: t('myNetwork.groups.leave'),
    })
    if (!confirmed) return
    setBusy(true)
    setError(null)
    try {
      await networkActions.leaveGroup(group.did)
      onLeft()
    } catch (failure) {
      setError(networkErrorText(t, failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="min-w-0 space-y-4" data-testid="group-detail" data-did={group.did}>
      <div className="flex flex-col gap-4 rounded-[22px] border px-5 py-5 sm:flex-row sm:items-start" style={{ background: 'color-mix(in srgb, var(--cp-surface-2) 50%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-border) 60%, transparent)' }}>
        <GroupAvatar group={group} size={56} />
        <div className="min-w-0 flex-1 space-y-2">
          <ObjectTitle title={name} description={group.description} did={group.did} showDid size="lg" />
          <div className="flex flex-wrap gap-1.5">
            {group.hostedHere ? <Pill tone="var(--cp-success)">{t('network.group.hostedHere')}</Pill> : <Pill tone="var(--cp-muted)">{t('network.group.hostedElsewhere')}</Pill>}
            <Pill tone={group.state === 'active' ? 'var(--cp-accent)' : 'var(--cp-warning)'}>{group.state === 'active' ? t(`myNetwork.groups.role.${group.role ?? 'member'}`) : t(`myNetwork.groups.state.${group.state}`)}</Pill>
            {group.joinPolicy ? <Pill tone="var(--cp-muted)">{t(`network.group.policy.${group.joinPolicy}`, group.joinPolicy)}</Pill> : null}
          </div>
          <div className="flex flex-wrap gap-2 pt-1">
            <Button size="small" variant="contained" startIcon={<MessageSquare size={14} />} onClick={() => void openMessageHub(group.did)} data-testid="group-open-messagehub">
              {t('myNetwork.groups.openMessageHub')}
            </Button>
            {group.role === 'owner' || group.role === 'admin' ? (
              <Button size="small" variant="outlined" startIcon={<Settings2 size={14} />} onClick={() => void openMessageHub(group.did)}>{t('myNetwork.groups.manage')}</Button>
            ) : null}
            <Button size="small" variant="outlined" sx={dangerOutlinedSx} startIcon={<LogOut size={14} />} disabled={busy} onClick={() => void leave()} data-testid="group-leave">
              {t('myNetwork.groups.leave')}
            </Button>
          </div>
        </div>
      </div>
      {error ? <StateMessage tone="danger" testId="group-error">{error}</StateMessage> : null}
      <section className="rounded-[22px] border px-5 py-4" style={{ background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-border) 50%, transparent)' }}>
        <h3 className="mb-2 font-display text-sm font-semibold text-[color:var(--cp-text)]">{t('myNetwork.groups.profile')}</h3>
        {group.owner ? <Fact label={t('network.group.owner')}><PersonName person={{ did: group.owner }} /></Fact> : null}
        {group.host ? <Fact label={t('network.group.host')}>{group.host}</Fact> : null}
        <Fact label={t('network.group.members')}>{memberCount !== undefined ? t('network.group.memberCount', undefined, { count: memberCount }) : t('network.group.membersHidden')}</Fact>
        <Fact label={t('myNetwork.groups.myState')}>{t(`myNetwork.groups.state.${group.state}`)}</Fact>
        <Fact label={t('myNetwork.groups.canPost')}>
          {group.canPost === null ? t('myNetwork.groups.canPostUnknown') : group.canPost ? t('myNetwork.groups.canPostYes') : t('myNetwork.groups.canPostNo')}
        </Fact>
        <p className="pt-2 text-xs text-[color:var(--cp-muted)]">{t(group.hostedHere ? 'myNetwork.groups.manageHint' : 'myNetwork.groups.remoteHint')}</p>
      </section>
    </div>
  )
}
