/* ── Local space user detail page ── */

import { Alert, Chip, Button } from '@mui/material'
import { Link, ShieldAlert, Trash2 } from 'lucide-react'
import type { LocalUserEntity } from '../../datamodel/types'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { MetricCard } from '../../../../components/AppPanelPrimitives'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { useI18n } from '../../../../i18n/provider'

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
  const { t } = useI18n()

  const handleRemove = () => {
    if (window.confirm(t('usersAgents.localUser.removeConfirm', undefined, { name: user.displayName }))) {
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
        subtitle={`${user.source === 'primary-did' ? t('usersAgents.localUser.primaryDid') : t('usersAgents.localUser.localAccount')} · ${user.defaultGroup}`}
        isOnline={user.isOnline}
        badges={
          <>
            <Chip
              label={t(`usersAgents.role.${user.role}`)}
              size="small"
              color={roleColor[user.role]}
              variant="outlined"
            />
            <Chip
              label={t(`usersAgents.userStatus.${user.status}`)}
              size="small"
              color={statusColor[user.status]}
              variant="outlined"
            />
          </>
        }
      />

      <div className="grid gap-2 grid-cols-2 sm:grid-cols-3">
        <MetricCard
          label={t('usersAgents.localUser.source')}
          tone={user.source === 'primary-did' ? 'accent' : 'neutral'}
          value={user.source === 'primary-did' ? 'BNS / DID' : t('usersAgents.source.local')}
        />
        <MetricCard label={t('usersAgents.localUser.storageUsed')} tone="accent" value={user.storageUsed} />
        <MetricCard label={t('usersAgents.localUser.quota')} tone="neutral" value={user.storageQuota} />
        <MetricCard label={t('usersAgents.localUser.apps')} tone="success" value={String(user.availableApps.length)} />
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
              {t('usersAgents.localUser.pendingDid')}
            </h3>
          </div>
          <Alert severity="warning">
            {t('usersAgents.localUser.pendingDidBody')}
          </Alert>
          <div className="mt-3 space-y-1.5">
            {[
              [t('usersAgents.localUser.inviteUrl'), user.invitation.inviteUrl],
              [t('usersAgents.localUser.targetZone'), user.invitation.targetZone],
              [t('usersAgents.localUser.requestedDid'), user.invitation.requestedDid],
              [t('usersAgents.localUser.expires'), new Date(user.invitation.expiresAt).toLocaleString()],
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
              {t('usersAgents.localUser.copyInvite')}
            </Button>
          </div>
        </div>
      )}

      <InfoFieldsSection title={t('usersAgents.section.profile')} fields={user.profile} editable={false} />

      <SocialAccountsSection entityId={user.id} accounts={user.socialAccounts} editable={false} />

      <InfoFieldsSection title={t('usersAgents.section.settings')} fields={user.settings} editable={false} />

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
          {t('usersAgents.localUser.availableApps')}
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
          {t('usersAgents.localUser.account')}
        </h3>
        <div className="space-y-1.5">
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              {t('usersAgents.localUser.lastActive')}
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {user.status === 'pending-invitation'
                ? t('usersAgents.localUser.notActivated')
                : new Date(user.lastActive).toLocaleString()}
            </span>
          </div>
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
            {t('usersAgents.created')}
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {new Date(user.createdAt).toLocaleDateString()}
            </span>
          </div>
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              {t('usersAgents.localUser.credential')}
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {t(`usersAgents.credential.${user.credentialStatus}`)}
            </span>
          </div>
          <div className="flex items-baseline gap-3">
            <span className="text-[12px] font-medium w-24 shrink-0" style={{ color: 'var(--cp-muted)' }}>
              {t('usersAgents.security.password')}
            </span>
            <span className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {user.canChangePassword ? t('usersAgents.localUser.passwordAllowed') : t('usersAgents.localUser.passwordRestricted')}
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
            {t('usersAgents.localUser.remove')}
          </Button>
        </div>
      </div>
    </div>
  )
}
