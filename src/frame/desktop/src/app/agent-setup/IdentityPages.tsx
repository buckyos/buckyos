/* ── Step 1: identity (account profile) and Agent permissions ── */

import { useState } from 'react'
import { Alert, Button, CircularProgress, Switch, TextField } from '@mui/material'
import { CheckCircle2, ChevronDown, ChevronRight, ShieldCheck, Users } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { CurrentAccount } from '../../api/account'
import { AvatarPicker } from './AvatarPicker'
import type { NameCheckState } from './hooks'
import { draftDisplayName, type AgentSetupDraft, type ClassifiedAgentError } from './model'
import { AgentAvatar, Section, StatusPill } from './ui'

const NICKNAME_MAX = 64
const BIO_MAX = 500
const ROLE_MAX = 4000

function NameStatus({ state, onUseSuggestion, onRetry, conflict }: {
  state: NameCheckState
  onUseSuggestion: (name: string) => void
  onRetry: () => void
  conflict: boolean
}) {
  const { t } = useI18n()
  switch (state.status) {
    case 'empty':
      return <span>{t('agentSetup.name.hint')}</span>
    case 'invalid':
      return <span className="text-[color:var(--cp-danger)]">{t('agentSetup.name.reason.invalid')}</span>
    case 'checking':
      return (
        <span className="inline-flex items-center gap-1.5" data-testid="agent-setup-name-checking">
          <CircularProgress size={11} color="inherit" />
          {t('agentSetup.name.checking')}
        </span>
      )
    case 'error':
      return (
        <span className="inline-flex flex-wrap items-center gap-2 text-[color:var(--cp-danger)]">
          {state.error.kind === 'limited_user'
            ? t('agentSetup.error.limited')
            : t('agentSetup.name.checkFailed', undefined, { detail: state.error.detail })}
          <button type="button" className="underline" onClick={onRetry}>{t('common.retry')}</button>
        </span>
      )
    case 'checked': {
      const { result } = state
      if (result.available) {
        return (
          <span className="inline-flex items-center gap-1 text-[color:color-mix(in_srgb,var(--cp-success)_70%,var(--cp-text))]" data-testid="agent-setup-name-available">
            <CheckCircle2 size={13} />
            {t('agentSetup.name.available')}
          </span>
        )
      }
      const reasonKey = `agentSetup.name.reason.${result.reason ?? 'other'}`
      return (
        <span className="flex flex-col gap-1 text-[color:var(--cp-danger)]" data-testid="agent-setup-name-unavailable">
          <span>{conflict ? t('agentSetup.name.conflictOnSubmit') : t(reasonKey, result.message ?? t('agentSetup.name.reason.other'))}</span>
          {result.suggestion ? (
            <span className="text-[color:var(--cp-text)]">
              {t('agentSetup.name.suggestion')}{' '}
              <button
                type="button"
                className="font-semibold text-[color:var(--cp-accent)] underline"
                onClick={() => onUseSuggestion(result.suggestion as string)}
              >
                {result.suggestion}
              </button>
            </span>
          ) : null}
        </span>
      )
    }
  }
}

export function AccountPage({
  draft,
  update,
  nameCheck,
  onRetryNameCheck,
  nameConflict,
  templateIcon,
  bio,
  onSessionExpired,
}: {
  draft: AgentSetupDraft
  update: (patch: Partial<AgentSetupDraft>) => void
  nameCheck: NameCheckState
  onRetryNameCheck: () => void
  nameConflict: boolean
  templateIcon?: string
  bio: string
  onSessionExpired?: (error: ClassifiedAgentError) => void
}) {
  const { t } = useI18n()
  const [roleOpen, setRoleOpen] = useState(Boolean(draft.roleSupplement))
  const didPreview = nameCheck.status === 'checked' && nameCheck.result.available ? nameCheck.result.agent_did : null
  const nameError = nameCheck.status === 'invalid' || (nameCheck.status === 'checked' && !nameCheck.result.available)
  const sessionError = nameCheck.status === 'error' && nameCheck.error.kind === 'session_expired' ? nameCheck.error : null

  return (
    <div className="space-y-4">
      {nameConflict ? (
        <Alert severity="error" data-testid="agent-setup-name-conflict">{t('agentSetup.name.conflictOnSubmit')}</Alert>
      ) : null}
      {sessionError && onSessionExpired ? (
        <Alert
          severity="warning"
          action={<Button size="small" variant="text" onClick={() => onSessionExpired(sessionError)}>{t('agentSetup.session.relogin')}</Button>}
        >
          {t('agentSetup.session.expired')}
        </Alert>
      ) : null}
      <Section title={t('agentSetup.account.title')} description={t('agentSetup.account.description')}>
        <div className="space-y-4">
          <AvatarPicker
            name={draftDisplayName(draft)}
            avatar={draft.avatar}
            fallbackSrc={templateIcon}
            onChange={(avatar) => update({ avatar })}
          />
          <div>
            <TextField
              label={t('agentSetup.name.label')}
              required
              value={draft.name}
              autoComplete="off"
              inputProps={{ 'data-testid': 'agent-setup-name', spellCheck: false, maxLength: 63 }}
              error={nameError}
              onChange={(event) => update({ name: event.target.value.toLowerCase().replace(/\s+/g, ''), nameEdited: true })}
              helperText={
                <NameStatus
                  state={nameCheck}
                  conflict={nameConflict}
                  onRetry={onRetryNameCheck}
                  onUseSuggestion={(name) => update({ name, nameEdited: true })}
                />
              }
            />
            <div className="mt-2 flex flex-wrap items-baseline gap-2 text-[12px]">
              <span className="font-medium text-[color:var(--cp-muted)]">{t('agentSetup.did.label')}</span>
              {didPreview ? (
                <code data-testid="agent-setup-did" className="break-all rounded-md px-1.5 py-0.5 text-[color:var(--cp-text)]" style={{ background: 'color-mix(in srgb, var(--cp-accent-soft) 14%, var(--cp-surface))' }}>
                  {didPreview}
                </code>
              ) : (
                <span className="text-[color:var(--cp-muted)]">{t('agentSetup.did.pending')}</span>
              )}
            </div>
          </div>
          <TextField
            label={t('agentSetup.nickname.label')}
            value={draft.displayName}
            inputProps={{ maxLength: NICKNAME_MAX, 'data-testid': 'agent-setup-nickname' }}
            onChange={(event) => update({ displayName: event.target.value })}
            helperText={t('agentSetup.nickname.hint')}
          />
          <TextField
            label={t('agentSetup.bio.label')}
            value={bio}
            multiline
            minRows={2}
            maxRows={6}
            inputProps={{ maxLength: BIO_MAX, 'data-testid': 'agent-setup-bio' }}
            onChange={(event) => update({ bio: event.target.value, bioEdited: true })}
            helperText={t('agentSetup.bio.hint')}
          />
        </div>
      </Section>

      <Section>
        <button
          type="button"
          aria-expanded={roleOpen}
          className="flex w-full items-center gap-2 text-left"
          onClick={() => setRoleOpen((value) => !value)}
        >
          {roleOpen ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
          <span className="font-display text-sm font-semibold text-[color:var(--cp-text)]">{t('agentSetup.role.title')}</span>
          <StatusPill tone="muted">{t('agentSetup.optional')}</StatusPill>
          {draft.roleSupplement.trim() && !roleOpen ? <StatusPill tone="accent">{t('agentSetup.role.filled')}</StatusPill> : null}
        </button>
        {roleOpen ? (
          <div className="mt-3 space-y-2">
            <p className="text-[13px] leading-5 text-[color:var(--cp-muted)]">{t('agentSetup.role.hint')}</p>
            <TextField
              label={t('agentSetup.role.label')}
              value={draft.roleSupplement}
              multiline
              minRows={3}
              maxRows={10}
              placeholder={t('agentSetup.role.placeholder')}
              inputProps={{ maxLength: ROLE_MAX, 'data-testid': 'agent-setup-role' }}
              onChange={(event) => update({ roleSupplement: event.target.value })}
              helperText={t('agentSetup.role.private')}
            />
          </div>
        ) : null}
      </Section>
    </div>
  )
}

function userTypeLabelKey(userType: string) {
  const normalized = userType.toLowerCase()
  if (normalized === 'root') return 'agentSetup.userType.root'
  if (normalized === 'admin') return 'agentSetup.userType.admin'
  return 'agentSetup.userType.user'
}

export function PermissionsPage({
  draft,
  update,
  account,
}: {
  draft: AgentSetupDraft
  update: (patch: Partial<AgentSetupDraft>) => void
  account: CurrentAccount
}) {
  const { t } = useI18n()
  return (
    <div className="space-y-4">
      <Section title={t('agentSetup.owner.title')} description={t('agentSetup.owner.description')}>
        <div className="flex items-center gap-3" data-testid="agent-setup-owner">
          <AgentAvatar name={account.user_name || account.user_id} size={40} />
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold text-[color:var(--cp-text)]">{account.user_name || account.user_id}</div>
            <div className="truncate text-[12px] text-[color:var(--cp-muted)]">{account.user_id}</div>
          </div>
          <StatusPill tone="muted">{t('agentSetup.readonly')}</StatusPill>
        </div>
      </Section>

      <Section title={t('agentSetup.permission.title')}>
        <div className="flex items-start gap-3" data-testid="agent-setup-permission">
          <ShieldCheck size={18} className="mt-0.5 shrink-0 text-[color:var(--cp-accent)]" />
          <div className="space-y-1">
            <p className="text-sm font-medium text-[color:var(--cp-text)]">{t('agentSetup.permission.same')}</p>
            <p className="text-[12px] leading-5 text-[color:var(--cp-muted)]">
              {t('agentSetup.permission.role', undefined, { role: t(userTypeLabelKey(account.user_type)) })}
              {' '}
              {t('agentSetup.permission.advanced')}
            </p>
          </div>
        </div>
      </Section>

      <Section title={t('agentSetup.access.title')}>
        <div className="space-y-4">
          <div className="flex items-start justify-between gap-3" data-testid="agent-setup-sharing">
            <div className="flex min-w-0 items-start gap-3">
              <Users size={18} className="mt-0.5 shrink-0 text-[color:var(--cp-muted)]" />
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2 text-sm font-medium text-[color:var(--cp-text)]">
                  {t('agentSetup.access.sharing')}
                  <StatusPill tone="warning">{t('agentSetup.unsupported')}</StatusPill>
                </div>
                <p className="mt-0.5 text-[12px] leading-5 text-[color:var(--cp-muted)]">{t('agentSetup.access.sharingHint')}</p>
                <p className="text-[12px] leading-5 text-[color:var(--cp-muted)]">{t('agentSetup.access.sharingUnsupported')}</p>
              </div>
            </div>
            <Switch checked={false} disabled slotProps={{ input: { 'aria-label': t('agentSetup.access.sharing') } }} />
          </div>
          <div className="flex items-start justify-between gap-3" data-testid="agent-setup-group">
            <div className="min-w-0 pl-[30px]">
              <div className="text-sm font-medium text-[color:var(--cp-text)]">{t('agentSetup.access.group')}</div>
              <p className="mt-0.5 text-[12px] leading-5 text-[color:var(--cp-muted)]">{t('agentSetup.access.groupHint')}</p>
            </div>
            <Switch
              checked={draft.allowGroup}
              onChange={(event) => update({ allowGroup: event.target.checked })}
              slotProps={{ input: { 'aria-label': t('agentSetup.access.group') } }}
            />
          </div>
        </div>
      </Section>
    </div>
  )
}
