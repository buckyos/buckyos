/* ── Local space user detail page ── */

import { Alert, Chip, Button } from '@mui/material'
import { Link, ShieldAlert, Trash2 } from 'lucide-react'
import type { LocalUserEntity } from '../../datamodel/types'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { MetricCard } from '../../../../components/AppPanelPrimitives'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'

interface LocalUserDetailPageProps {
  user: LocalUserEntity
  onRemoved?: () => void
}

const roleColor = {
  admin: 'primary' as const,
  user: 'default' as const,
  limited: 'secondary' as const,
}

const statusColor = {
  active: 'success' as const,
  'pending-invitation': 'warning' as const,
  suspended: 'error' as const,
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
        subtitle={`${user.source === 'primary-did' ? 'Primary DID user' : 'Local account'} · ${user.defaultGroup}`}
        isOnline={user.isOnline}
        badges={
          <>
            <Chip
              label={user.role}
              size="small"
              color={roleColor[user.role]}
              variant="outlined"
            />
            <Chip
              label={user.status.replace('-', ' ')}
              size="small"
              color={statusColor[user.status]}
              variant="outlined"
            />
          </>
        }
      />

      <div className="grid gap-2 grid-cols-2 sm:grid-cols-3">
        <MetricCard
          label="Source"
          tone={user.source === 'primary-did' ? 'accent' : 'neutral'}
          value={user.source === 'primary-did' ? 'BNS / DID' : 'Local'}
        />
        <MetricCard label="Storage used" tone="accent" value={user.storageUsed} />
        <MetricCard label="Quota" tone="neutral" value={user.storageQuota} />
        <MetricCard label="Apps" tone="success" value={String(user.availableApps.length)} />
      </div>

      {user.invitation && (
        <div
          className="rounded-[22px] px-5 py-4"
          style={{
            background: 'color-mix(in srgb, var(--cp-warning) 8%, var(--cp-surface))',
            border: '1px solid color-mix(in srgb, var(--cp-warning) 22%, transparent)',
          }}
        >
          <div className="mb-3 flex items-center gap-2">
            <ShieldAlert size={16} style={{ color: 'var(--cp-warning)' }} />
            <h3
              className="font-display text-sm font-semibold"
              style={{ color: 'var(--cp-text)' }}
            >
              Pending DID Confirmation
            </h3>
          </div>
          <Alert severity="warning">
            The target user must confirm this Zone with their own identity key before the account becomes active.
          </Alert>
          <div className="mt-3 space-y-1.5">
            {[
              ['Invite URL', user.invitation.inviteUrl],
              ['Target Zone', user.invitation.targetZone],
              ['Requested DID', user.invitation.requestedDid],
              ['Expires', new Date(user.invitation.expiresAt).toLocaleString()],
            ].map(([label, value]) => (
              <div key={label} className="flex items-baseline gap-3">
                <span className="w-28 shrink-0 text-[12px] font-medium" style={{ color: 'var(--cp-muted)' }}>
                  {label}
                </span>
                <span className="min-w-0 break-all text-sm" style={{ color: 'var(--cp-text)' }}>
                  {value}
                </span>
              </div>
            ))}
          </div>
          <div className="mt-3 flex">
            <Button
              size="small"
              variant="outlined"
              startIcon={<Link size={14} />}
              onClick={() => navigator.clipboard.writeText(user.invitation?.inviteUrl ?? '')}
            >
              Copy invitation link
            </Button>
          </div>
        </div>
      )}

      <InfoFieldsSection title="Profile" fields={user.profile} editable={false} />

      <SocialAccountsSection entityId={user.id} accounts={user.socialAccounts} editable={false} />

      <InfoFieldsSection title="Settings" fields={user.settings} editable={false} />

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
              {user.status === 'pending-invitation'
                ? 'Not activated'
                : new Date(user.lastActive).toLocaleString()}
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
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Credential
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {user.credentialStatus.replace('-', ' ')}
            </span>
          </div>
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              Password
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {user.canChangePassword ? 'Change allowed' : 'Change restricted'}
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
