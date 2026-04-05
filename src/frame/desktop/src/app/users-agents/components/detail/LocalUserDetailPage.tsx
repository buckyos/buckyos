/* ── Local space user detail page ── */

import { Chip, Button } from '@mui/material'
import { Trash2 } from 'lucide-react'
import type { LocalUserEntity } from '../../mock/types'
import { HeaderSection } from '../sections/HeaderSection'
import { BindingsSection } from '../sections/BindingsSection'
import { MetricCard } from '../../../../components/AppPanelPrimitives'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'

interface LocalUserDetailPageProps {
  user: LocalUserEntity
  onRemoved?: () => void
}

const roleColor = {
  admin: 'primary' as const,
  member: 'default' as const,
  guest: 'secondary' as const,
}

export function LocalUserDetailPage({ user, onRemoved }: LocalUserDetailPageProps) {
  const store = useUsersAgentsStore()

  const handleRemove = () => {
    if (window.confirm(`Remove user "${user.displayName}"? This cannot be undone.`)) {
      store.removeLocalUser(user.id)
      onRemoved?.()
    }
  }

  return (
    <div className="space-y-4">
      <HeaderSection
        name={user.displayName}
        kind="local-user"
        avatarUrl={user.avatarUrl}
        did={user.did}
        subtitle={`Local user · ${user.defaultGroup} group`}
        isOnline={user.isOnline}
        badges={
          <Chip
            label={user.role}
            size="small"
            color={roleColor[user.role]}
            variant="outlined"
          />
        }
      />

      {/* Quick stats */}
      <div className="grid gap-2 grid-cols-2 sm:grid-cols-3">
        <MetricCard label="Storage used" tone="accent" value={user.storageUsed} />
        <MetricCard label="Quota" tone="neutral" value={user.storageQuota} />
        <MetricCard label="Apps" tone="success" value={String(user.availableApps.length)} />
      </div>

      <BindingsSection bindings={user.bindings} />

      {/* Available apps */}
      <div
        className="rounded-[22px] px-5 py-4"
        style={{
          background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
        }}
      >
        <h3
          className="font-display text-sm font-semibold mb-3"
          style={{ color: 'var(--cp-text)' }}
        >
          Available Apps
        </h3>
        <div className="flex flex-wrap gap-1.5">
          {user.availableApps.map((app) => (
            <Chip key={app} label={app} size="small" variant="outlined" />
          ))}
        </div>
      </div>

      {/* Last active */}
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
          Account
        </h3>
        <div className="space-y-1.5">
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Last active
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {new Date(user.lastActive).toLocaleString()}
            </span>
          </div>
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Created
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {new Date(user.createdAt).toLocaleDateString()}
            </span>
          </div>
        </div>

        <div className="mt-4 pt-3" style={{ borderTop: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)' }}>
          <Button
            size="small"
            color="error"
            variant="outlined"
            startIcon={<Trash2 size={14} />}
            onClick={handleRemove}
          >
            Remove User
          </Button>
        </div>
      </div>
    </div>
  )
}
