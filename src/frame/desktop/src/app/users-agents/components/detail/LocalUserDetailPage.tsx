/* ── Local space user detail page ── */

import { useRef, useState } from 'react'
import { Alert, Chip, Button, CircularProgress } from '@mui/material'
import { Link, ShieldAlert, Trash2 } from 'lucide-react'
import type { LocalUserEntity } from '../../datamodel/types'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { AccountAdminSection } from '../sections/AccountAdminSection'
import { MetricCard } from '../../../../components/AppPanelPrimitives'
import { useUsersAgentsStore, useViewerIsAdmin } from '../../hooks/use-users-agents-store'
import { useI18n } from '../../../../i18n/provider'
import { useSudoByPassword } from '../../../../components/sudo'
import { errorMessage } from '../../../agent-setup/model'
import { fetchUserList } from '../../../../api/user_mgr'

interface LocalUserDetailPageProps {
  user: LocalUserEntity
  onRemoved?: () => void
  onNotice: (message: string) => void
}

const roleColor = {
  admin: 'primary' as const,
  user: 'default' as const,
  limited: 'secondary' as const,
  unknown: 'warning' as const,
}

const statusColor = {
  active: 'success' as const,
  'pending-invitation': 'warning' as const,
  suspended: 'error' as const,
}

export function LocalUserDetailPage({ user, onRemoved, onNotice }: LocalUserDetailPageProps) {
  const store = useUsersAgentsStore()
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin()
  const requestSudo = useSudoByPassword()
  const pending = useRef(false)
  const [busy, setBusy] = useState(false)
  const [committed, setCommitted] = useState(false)
  const [uncertain, setUncertain] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refreshAfterDelete = async () => {
    try {
      const snapshot = await store.reload()
      if (snapshot.localUsers.some((item) => item.id === user.id)) throw new Error(t('usersAgents.lifecycle.stillVisible'))
      onRemoved?.()
    } catch (failure) {
      setError(t('usersAgents.lifecycle.deleteReloadFailed', undefined, { reason: errorMessage(failure) }))
    }
  }

  const reconcileDelete = async (failure?: unknown) => {
    try {
      const result = await fetchUserList({ includeDeleted: true })
      if (!result.data) throw result.error ?? new Error('user.list failed')
      const current = result.data.users.find((item) => item.user_id === user.id)
      if (!current) throw new Error(t('usersAgents.lifecycle.accountMissing'))
      setUncertain(false)
      if (current.state === 'deleted') {
        setCommitted(true)
        onNotice(t('usersAgents.lifecycle.responseLost'))
        await refreshAfterDelete()
      } else {
        setError(t('usersAgents.lifecycle.deleteFailed', undefined, { reason: failure ? errorMessage(failure) : t('usersAgents.lifecycle.stillVisible') }))
      }
    } catch (readError) {
      setUncertain(true)
      setError(t('usersAgents.lifecycle.deleteUncertain', undefined, { reason: errorMessage(failure ?? readError) }))
    }
  }

  const handleRemove = async () => {
    if (pending.current || committed || uncertain || !isAdmin) return
    if (!window.confirm(t('usersAgents.localUser.removeConfirm', undefined, { name: user.displayName }))) return
    pending.current = true
    setBusy(true)
    setError(null)
    let mutationStarted = false
    try {
      const grant = await requestSudo({
        aud: 'system-config',
        appid: 'control-panel',
        username: store.getSnapshot().self.id,
        title: t('usersAgents.lifecycle.deleteTitle'),
        description: t('usersAgents.localUser.removeConfirm', undefined, { name: user.displayName }),
        reason: `Delete user ${user.id}`,
        confirmLabel: t('usersAgents.localUser.remove'),
      })
      if (!grant) return
      mutationStarted = true
      const { data, error: failure } = await store.removeLocalUser(user.id, { sessionToken: grant.sessionToken })
      if (!data?.ok || data.state !== 'deleted' || data.user_id !== user.id) {
        await reconcileDelete(failure ?? new Error('Invalid user.delete response'))
        return
      }
      setCommitted(true)
      if (data.warning || !data.rbac_refreshed) onNotice(data.warning ?? t('usersAgents.lifecycle.rbacPending'))
      await refreshAfterDelete()
    } catch (failure) {
      if (mutationStarted) await reconcileDelete(failure)
      else setError(t('usersAgents.lifecycle.deleteFailed', undefined, { reason: errorMessage(failure) }))
    } finally {
      pending.current = false
      setBusy(false)
    }
  }

  const retryRefresh = async () => {
    if (pending.current) return
    pending.current = true
    setBusy(true)
    setError(null)
    try { if (uncertain) await reconcileDelete(); else await refreshAfterDelete() } finally { pending.current = false; setBusy(false) }
  }

  return (
    <div className="space-y-4">
      <HeaderSection
        name={user.displayName}
        kind="local-user"
        avatarUrl={user.avatarUrl}
        did={user.did}
        subtitle={user.source === 'primary-did' ? t('usersAgents.localUser.primaryDid') : t('usersAgents.localUser.localAccount')}
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

      <AccountAdminSection user={user} />

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

        {isAdmin && <div className="mt-4 pt-3" style={{ borderTop: '1px solid color-mix(in srgb, var(--cp-border) 40%, transparent)' }}>
          {error ? <Alert severity={committed || uncertain ? 'warning' : 'error'} className="mb-3">{error}</Alert> : null}
          <Button
            size="small"
            color="error"
            variant="outlined"
            startIcon={busy ? <CircularProgress size={14} color="inherit" /> : <Trash2 size={14} />}
            onClick={() => void handleRemove()}
            disabled={busy || committed || uncertain}
            data-testid="local-user-delete"
          >
            {t('usersAgents.localUser.remove')}
          </Button>
          {(committed || uncertain) && error ? <Button disabled={busy} onClick={() => void retryRefresh()}>{t('usersAgents.lifecycle.refresh')}</Button> : null}
        </div>}
      </div>
    </div>
  )
}
