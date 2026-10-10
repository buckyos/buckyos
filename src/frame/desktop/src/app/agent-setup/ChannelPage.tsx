/* ── Step 3: optional external message channel ── */

import { useState } from 'react'
import { Alert, Button, CircularProgress, TextField } from '@mui/material'
import { ChevronDown, ChevronRight, RefreshCw, UserRound } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { UserTunnelBinding } from '../../api/user_mgr'
import type { OwnProfileState } from './hooks'
import { isTelegramBotToken, type AgentSetupDraft } from './model'
import { OptionCard, Section, StatusPill } from './ui'

export function ChannelPage({
  draft,
  update,
  ownProfile,
  telegramIdentity,
  onRecheck,
  onOpenProfile,
  botToken,
  onBotTokenChange,
}: {
  draft: AgentSetupDraft
  update: (patch: Partial<AgentSetupDraft>) => void
  ownProfile: OwnProfileState
  telegramIdentity: UserTunnelBinding | null
  onRecheck: () => void
  onOpenProfile: () => void
  botToken: string
  onBotTokenChange: (value: string) => void
}) {
  const { t } = useI18n()
  const [more, setMore] = useState(false)
  const checking = ownProfile.status === 'loading'
  const identityReady = ownProfile.status === 'ready' && telegramIdentity !== null
  const telegramSelected = draft.channel === 'telegram' && identityReady
  const tokenFilled = botToken.trim().length > 0
  const tokenValid = isTelegramBotToken(botToken)

  const identityPill = checking
    ? <StatusPill tone="muted" testId="agent-setup-telegram-identity">{t('agentSetup.channel.identityChecking')}</StatusPill>
    : ownProfile.status === 'error'
      ? <StatusPill tone="danger" testId="agent-setup-telegram-identity">{t('agentSetup.channel.identityCheckFailed')}</StatusPill>
      : identityReady
        ? <StatusPill tone="success" testId="agent-setup-telegram-identity">{t('agentSetup.channel.identityReady')}</StatusPill>
        : <StatusPill tone="warning" testId="agent-setup-telegram-identity">{t('agentSetup.channel.identityMissing')}</StatusPill>

  return (
    <div className="space-y-4">
      <Section title={t('agentSetup.channel.title')} description={t('agentSetup.channel.description')}>
        <div className="space-y-3" role="radiogroup" aria-label={t('agentSetup.channel.title')}>
          <OptionCard
            testId="agent-setup-channel-telegram"
            selected={telegramSelected}
            disabled={!identityReady}
            onSelect={() => update({ channel: 'telegram' })}
            title="Telegram"
            badge={identityPill}
            description={t('agentSetup.channel.telegramHint')}
          >
            {telegramSelected ? (
              <div className="space-y-3">
                <TextField
                  label={t('agentSetup.channel.botToken')}
                  type="password"
                  autoComplete="off"
                  value={botToken}
                  error={tokenFilled && !tokenValid}
                  inputProps={{ 'data-testid': 'agent-setup-bot-token', spellCheck: false }}
                  onChange={(event) => onBotTokenChange(event.target.value)}
                  helperText={tokenFilled && !tokenValid ? t('agentSetup.channel.botTokenInvalid') : t('agentSetup.channel.botTokenHint')}
                />
                <ul className="space-y-1.5 text-[13px]" data-testid="agent-setup-channel-status">
                  <li className="flex flex-wrap items-center gap-2">
                    <span className="w-32 shrink-0 text-[color:var(--cp-muted)]">{t('agentSetup.channel.statusIdentity')}</span>
                    <StatusPill tone="success">{t('agentSetup.channel.identityConfigured', undefined, { id: telegramIdentity?.display_id || telegramIdentity?.account_id || '' })}</StatusPill>
                  </li>
                  <li className="flex flex-wrap items-center gap-2">
                    <span className="w-32 shrink-0 text-[color:var(--cp-muted)]">{t('agentSetup.channel.statusParams')}</span>
                    {tokenValid
                      ? <StatusPill tone="success">{t('agentSetup.channel.paramsFilled')}</StatusPill>
                      : tokenFilled
                        ? <StatusPill tone="danger">{t('agentSetup.channel.paramsInvalid')}</StatusPill>
                        : <StatusPill tone="muted">{t('agentSetup.channel.paramsEmpty')}</StatusPill>}
                  </li>
                  <li className="flex flex-wrap items-center gap-2">
                    <span className="w-32 shrink-0 text-[color:var(--cp-muted)]">{t('agentSetup.channel.statusBinding')}</span>
                    <StatusPill tone="muted">{t('agentSetup.channel.bindingOnCreate')}</StatusPill>
                  </li>
                </ul>
                {!tokenFilled ? <p className="text-[12px] leading-5 text-[color:var(--cp-muted)]">{t('agentSetup.channel.tokenNotSaved')}</p> : null}
              </div>
            ) : null}
          </OptionCard>

          {ownProfile.status === 'error' ? (
            <Alert
              severity="error"
              data-testid="agent-setup-identity-error"
              action={<Button size="small" variant="text" onClick={onRecheck}>{t('common.retry')}</Button>}
            >
              {t('agentSetup.channel.identityError', undefined, { detail: ownProfile.error.detail })}
            </Alert>
          ) : ownProfile.status === 'ready' && !telegramIdentity ? (
            <Alert severity="warning" data-testid="agent-setup-identity-missing">
              <p>{t('agentSetup.channel.identityMissingBody')}</p>
              <div className="mt-2 flex flex-wrap gap-2">
                <Button size="small" variant="outlined" startIcon={<UserRound size={14} />} onClick={onOpenProfile}>
                  {t('agentSetup.channel.openProfile')}
                </Button>
                <Button size="small" variant="text" startIcon={<RefreshCw size={14} />} onClick={onRecheck}>
                  {t('agentSetup.channel.recheck')}
                </Button>
              </div>
            </Alert>
          ) : checking ? (
            <p role="status" className="flex items-center gap-2 text-[13px] text-[color:var(--cp-muted)]">
              <CircularProgress size={12} color="inherit" />
              {t('agentSetup.channel.identityChecking')}
            </p>
          ) : null}

          <OptionCard
            testId="agent-setup-channel-lark"
            selected={false}
            disabled
            title={t('agentSetup.channel.lark')}
            badge={<StatusPill tone="warning">{t('agentSetup.unsupported')}</StatusPill>}
            description={t('agentSetup.channel.larkHint')}
          />
        </div>

        <div className="mt-3">
          <button
            type="button"
            aria-expanded={more}
            className="inline-flex items-center gap-1.5 text-[13px] font-semibold text-[color:var(--cp-accent)]"
            onClick={() => setMore((value) => !value)}
          >
            {more ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
            {t('agentSetup.channel.moreTitle')}
          </button>
          {more ? <p className="mt-1.5 text-[13px] leading-5 text-[color:var(--cp-muted)]">{t('agentSetup.channel.moreBody')}</p> : null}
        </div>
      </Section>
    </div>
  )
}
