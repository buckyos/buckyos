/* ── Account type, per-account permissions and system user groups of a listed user ── */

import { useState } from 'react'
import { Alert, Chip, CircularProgress, Switch, TextField } from '@mui/material'
import { X } from 'lucide-react'
import { changeTypeTargets, type AssignableUserType } from '../../../../api/user_type'
import { errorMessage } from '../../../agent-setup/model'
import { useI18n } from '../../../../i18n/provider'
import { useUserGroups, useUsersAgentsStore, useViewerIsAdmin } from '../../hooks/use-users-agents-store'
import type { LocalUserEntity } from '../../datamodel/types'
import { systemGroupName } from '../shared/userGroupLabels'

type OptionKey = 'allowPasswordChange' | 'allowContactEdit'

function OptionRow({
  label,
  checked,
  override,
  disabled,
  onChange,
  testId,
}: {
  label: string
  checked: boolean
  override: boolean | null
  disabled: boolean
  onChange: (value: boolean | null) => void
  testId: string
}) {
  const { t } = useI18n()
  return (
    <div className="flex items-center justify-between gap-3" data-testid={testId}>
      <div className="min-w-0">
        <div className="text-sm" style={{ color: 'var(--cp-text)' }}>{label}</div>
        <div className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>
          {override === null ? t('usersAgents.account.default') : t('usersAgents.account.overridden')}
          {override !== null && !disabled ? (
            <button
              type="button"
              className="ml-2 underline"
              style={{ color: 'var(--cp-accent)' }}
              onClick={() => onChange(null)}
            >
              {t('usersAgents.account.resetDefault')}
            </button>
          ) : null}
        </div>
      </div>
      <Switch
        size="small"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        inputProps={{ 'aria-label': label }}
      />
    </div>
  )
}

export function AccountAdminSection({ user }: { user: LocalUserEntity }) {
  const { t } = useI18n()
  const store = useUsersAgentsStore()
  const isAdmin = useViewerIsAdmin()
  const userGroups = useUserGroups()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const groups = userGroups?.groups
  const targets = changeTypeTargets(user.role === 'unknown' ? null : user.role)
  const joinable = (groups ?? []).filter((group) => !user.groups.includes(group.id))

  const run = async (change: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    const failure = await change()
    setBusy(false)
    if (failure) setError(t('usersAgents.error.change', undefined, { reason: errorMessage(failure) }))
  }

  const changeType = (target: AssignableUserType) => {
    const typeName = t(`usersAgents.role.${target}`)
    if (window.confirm(t('usersAgents.account.changeTypeConfirm', undefined, { name: user.displayName, type: typeName }))) {
      void run(() => store.changeUserType(user.id, target))
    }
  }

  const setOption = (key: OptionKey, value: boolean | null) => {
    void run(() => store.setAccountOptions(user.id, { [key]: value }))
  }

  return (
    <div
      className="rounded-[22px] px-5 py-4"
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
      data-testid="user-account-admin"
    >
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('usersAgents.account.title')}
        </h3>
        {busy ? <CircularProgress size={14} /> : null}
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <span className="shrink-0 whitespace-nowrap text-sm" style={{ color: 'var(--cp-text)' }}>{t('usersAgents.account.type')}</span>
          {isAdmin && targets.length > 0 ? (
            <TextField
              select
              size="small"
              value={user.role}
              disabled={busy}
              onChange={(event) => {
                const target = targets.find((item) => item === event.target.value)
                if (target) changeType(target)
              }}
              SelectProps={{ native: true }}
              inputProps={{ 'aria-label': t('usersAgents.account.type') }}
              sx={{ width: 200, flexShrink: 0 }}
            >
              <option value={user.role}>{t(`usersAgents.role.${user.role}`)}</option>
              {targets.map((target) => (
                <option key={target} value={target}>{t(`usersAgents.role.${target}`)}</option>
              ))}
            </TextField>
          ) : (
            <Chip size="small" variant="outlined" label={t(`usersAgents.role.${user.role}`)} />
          )}
        </div>
        {user.role === 'limited' ? (
          <div className="text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.account.limitedHint')}</div>
        ) : null}

        <OptionRow
          label={t('usersAgents.account.passwordChange')}
          checked={user.passwordChangeAllowed}
          override={user.passwordChangeOverride}
          disabled={!isAdmin || busy}
          onChange={(value) => setOption('allowPasswordChange', value)}
          testId="user-option-password"
        />
        <OptionRow
          label={t('usersAgents.account.contactEdit')}
          checked={user.contactEditAllowed}
          override={user.contactEditOverride}
          disabled={!isAdmin || busy}
          onChange={(value) => setOption('allowContactEdit', value)}
          testId="user-option-contacts"
        />

        <div data-testid="user-groups">
          <div className="mb-1.5 text-sm" style={{ color: 'var(--cp-text)' }}>{t('usersAgents.account.groups')}</div>
          <div className="flex flex-wrap items-center gap-1.5">
            {user.groups.length === 0 ? (
              <span className="text-[12px]" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.account.noGroups')}</span>
            ) : null}
            {user.groups.map((groupId) => {
              const name = systemGroupName(groupId, groups)
              return isAdmin ? (
                <Chip
                  key={groupId}
                  size="small"
                  variant="outlined"
                  label={name}
                  disabled={busy}
                  onDelete={() => void run(() => store.setUserGroupMembers(groupId, { remove: [user.id] }))}
                  deleteIcon={<X size={12} role="button" aria-label={t('usersAgents.account.leaveGroup', undefined, { group: name })} />}
                />
              ) : (
                <Chip key={groupId} size="small" variant="outlined" label={name} />
              )
            })}
            {isAdmin && joinable.length > 0 ? (
              <TextField
                select
                size="small"
                value=""
                disabled={busy}
                onChange={(event) => {
                  const groupId = event.target.value
                  if (groupId) void run(() => store.setUserGroupMembers(groupId, { add: [user.id] }))
                }}
                SelectProps={{ native: true }}
                inputProps={{ 'aria-label': t('usersAgents.account.addGroup') }}
                sx={{ width: 200 }}
              >
                <option value="">{t('usersAgents.account.addGroup')}</option>
                {joinable.map((group) => (
                  <option key={group.id} value={group.id}>{group.name}</option>
                ))}
              </TextField>
            ) : null}
          </div>
          {isAdmin && userGroups && userGroups.groups.length === 0 ? (
            <div className="mt-1 text-[12px]" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.account.noGroupsDefined')}</div>
          ) : null}
        </div>
      </div>
      {error ? <Alert className="mt-3" severity="error">{error}</Alert> : null}
    </div>
  )
}
