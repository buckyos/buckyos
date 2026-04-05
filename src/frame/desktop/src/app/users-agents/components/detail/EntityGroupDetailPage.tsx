/* ── Entity group detail page ── */

import { Chip, Button } from '@mui/material'
import { MessageSquare, Users2 } from 'lucide-react'
import type { EntityGroupEntity } from '../../mock/types'
import { HeaderSection } from '../sections/HeaderSection'
import { BindingsSection } from '../sections/BindingsSection'
import { MetricCard } from '../../../../components/AppPanelPrimitives'

interface EntityGroupDetailPageProps {
  group: EntityGroupEntity
}

export function EntityGroupDetailPage({ group }: EntityGroupDetailPageProps) {
  return (
    <div className="space-y-4">
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

      <div className="grid gap-2 grid-cols-2 sm:grid-cols-3">
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

      <BindingsSection bindings={group.bindings} />

      {/* Members preview */}
      <div
        className="rounded-[22px] px-5 py-4"
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
        <div className="flex flex-wrap gap-1.5">
          {group.memberIds.slice(0, 8).map((id) => (
            <Chip key={id} label={id} size="small" variant="outlined" />
          ))}
          {group.memberIds.length > 8 && (
            <Chip label={`+${group.memberIds.length - 8} more`} size="small" variant="outlined" />
          )}
        </div>
      </div>

      {/* Group info */}
      <div
        className="rounded-[22px] px-5 py-4"
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
        <div className="space-y-1.5">
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Created
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {new Date(group.createdAt).toLocaleDateString()}
            </span>
          </div>
          {group.did && (
            <div className="flex items-baseline gap-3">
              <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
                DID
              </span>
              <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
                {group.did}
              </span>
            </div>
          )}
        </div>
      </div>

      {group.canMessage && (
        <div className="flex">
          <Button variant="contained" startIcon={<MessageSquare size={14} />}>
            Open in MessageHub
          </Button>
        </div>
      )}
    </div>
  )
}
