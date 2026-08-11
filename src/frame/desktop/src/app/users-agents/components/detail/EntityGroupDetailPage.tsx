/* ── Entity group detail page ── */

import { Chip, Button } from '@mui/material'
import { MessageSquare, Users2 } from 'lucide-react'
import type { EntityGroupEntity } from '../../datamodel/types'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { MetricCard } from '../../../../components/AppPanelPrimitives'

interface EntityGroupDetailPageProps {
  group: EntityGroupEntity
}

export function EntityGroupDetailPage({ group }: EntityGroupDetailPageProps) {
  const summaryItems = [
    ['Members', String(group.memberCount)],
    ['Type', group.isHostedBySelf ? 'Self-hosted' : 'Joined'],
    ...(group.ownerName ? [['Owner', group.ownerName]] : []),
  ]

  return (
    <div className="min-w-0 space-y-4">
      <HeaderSection
        name={group.displayName}
        kind="entity-group"
        avatarUrl={group.avatarUrl}
        did={group.did}
        subtitle={group.description}
        badges={
          <>
            {group.isHostedBySelf && (
              <Chip label="Hosted by you" size="small" color="primary" variant="outlined" />
            )}
            {group.canMessage && (
              <Chip
                icon={<MessageSquare size={12} />}
                label="Messageable"
                size="small"
                variant="outlined"
              />
            )}
          </>
        }
      />

      <div
        className="rounded-[22px] px-4 py-3 md:hidden"
        style={{
          background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
        }}
      >
        <dl className="divide-y" style={{ borderColor: 'color-mix(in srgb, var(--cp-border) 45%, transparent)' }}>
          {summaryItems.map(([label, value]) => (
            <div key={label} className="flex items-center justify-between gap-4 py-2 first:pt-0 last:pb-0">
              <dt className="shrink-0 text-[11px] font-semibold uppercase tracking-[0.16em]" style={{ color: 'var(--cp-muted)' }}>
                {label}
              </dt>
              <dd className="min-w-0 break-words text-right text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
                {value}
              </dd>
            </div>
          ))}
        </dl>
      </div>

      <div className="hidden gap-2 md:grid md:grid-cols-3">
        <MetricCard label="Members" tone="accent" value={String(group.memberCount)} />
        <MetricCard
          label="Type"
          tone="neutral"
          value={group.isHostedBySelf ? 'Self-hosted' : 'Joined'}
        />
        {group.ownerName && (
          <MetricCard label="Owner" tone="neutral" value={group.ownerName} />
        )}
      </div>

      <SocialAccountsSection entityId={group.id} accounts={group.socialAccounts} editable={false} />

      {/* Members preview */}
      <div
        className="min-w-0 rounded-[22px] px-5 py-4"
        style={{
          background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
        }}
      >
        <div className="flex items-center gap-2 mb-3">
          <Users2 size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3
            className="font-display text-sm font-semibold"
            style={{ color: 'var(--cp-text)' }}
          >
            Members ({group.memberCount})
          </h3>
        </div>
        <div className="flex min-w-0 flex-wrap gap-1.5">
          {group.memberIds.slice(0, 8).map((id) => (
            <span
              key={id}
              className="inline-flex max-w-full min-w-0 items-center rounded-full px-2.5 py-1 text-[12px] font-medium"
              style={{
                color: 'var(--cp-text)',
                border: '1px solid color-mix(in srgb, var(--cp-border) 70%, transparent)',
              }}
            >
              <span className="truncate">{id}</span>
            </span>
          ))}
          {group.memberIds.length > 8 && (
            <Chip label={`+${group.memberIds.length - 8} more`} size="small" variant="outlined" />
          )}
        </div>
      </div>

      {/* Group info */}
      <div
        className="min-w-0 rounded-[22px] px-5 py-4"
        style={{
          background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
        }}
      >
        <h3
          className="font-display text-sm font-semibold mb-2"
          style={{ color: 'var(--cp-text)' }}
        >
          Group Info
        </h3>
        <div className="space-y-3 md:space-y-1.5">
          <div className="flex flex-col gap-1 md:flex-row md:items-baseline md:gap-3">
            <span className="text-[12px] font-medium md:w-24 md:shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Created
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {new Date(group.createdAt).toLocaleDateString()}
            </span>
          </div>
          {group.did && (
            <div className="flex flex-col gap-1 md:flex-row md:items-baseline md:gap-3">
              <span className="text-[12px] font-medium md:w-24 md:shrink-0" style={{ color: 'var(--cp-muted)' }}>
                DID
              </span>
              <span className="min-w-0 break-all font-mono text-[12px] leading-5 md:text-sm" style={{ color: 'var(--cp-text)' }}>
                {group.did}
              </span>
            </div>
          )}
        </div>
      </div>

      {group.canMessage && (
        <div className="flex">
          <Button className="w-full md:w-auto" variant="contained" startIcon={<MessageSquare size={14} />}>
            Open in MessageHub
          </Button>
        </div>
      )}
    </div>
  )
}
