/* ── Confirmation summary before `agent.create` ── */

import { useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { CurrentAccount } from '../../api/account'
import type { AgentTemplate, UserTunnelBinding } from '../../api/user_mgr'
import { draftDisplayName, type AgentSetupDraft } from './model'
import { AgentAvatar, Section, StatusPill, SummaryRows } from './ui'

export function ConfirmPage({
  draft,
  bio,
  account,
  didPreview,
  template,
  telegram,
  avatarFallback,
}: {
  draft: AgentSetupDraft
  bio: string
  account: CurrentAccount
  didPreview: string | null
  template: AgentTemplate | null
  telegram: UserTunnelBinding | null
  avatarFallback?: string
}) {
  const { t } = useI18n()
  const [roleOpen, setRoleOpen] = useState(false)
  const yes = t('agentSetup.yes')
  const no = t('agentSetup.no')
  const role = draft.roleSupplement.trim()
  const name = draftDisplayName(draft)

  return (
    <div className="space-y-4" data-testid="agent-setup-confirm">
      <Section title={t('agentSetup.confirm.identity')}>
        <div className="mb-3 flex items-center gap-3">
          <AgentAvatar name={name} src={draft.avatar ?? avatarFallback} size={48} />
          <div className="min-w-0">
            <div className="truncate font-display text-base font-semibold text-[color:var(--cp-text)]" data-testid="agent-setup-confirm-name">{name}</div>
            <div className="truncate text-[12px] text-[color:var(--cp-muted)]">{draft.name}</div>
          </div>
        </div>
        <SummaryRows
          rows={[
            [t('agentSetup.name.label'), draft.name],
            [t('agentSetup.nickname.label'), draft.displayName.trim() || t('agentSetup.confirm.nicknameFallback')],
            [t('agentSetup.did.label'), didPreview ? <code key="did" className="break-all text-[12px]">{didPreview}</code> : t('agentSetup.did.pending')],
            [t('agentSetup.owner.title'), `${account.user_name || account.user_id} (${account.user_id})`],
            [t('agentSetup.bio.label'), bio.trim() || t('agentSetup.confirm.empty')],
          ]}
        />
      </Section>

      <Section title={t('agentSetup.confirm.role')}>
        <div className="flex flex-wrap items-center gap-2 text-sm text-[color:var(--cp-text)]">
          {role ? t('agentSetup.role.filled') : t('agentSetup.confirm.roleEmpty')}
          <StatusPill tone="muted">{t('agentSetup.confirm.rolePrivate')}</StatusPill>
          {role ? (
            <button type="button" className="text-[13px] font-semibold text-[color:var(--cp-accent)]" aria-expanded={roleOpen} onClick={() => setRoleOpen((value) => !value)}>
              {roleOpen ? t('agentSetup.confirm.hide') : t('agentSetup.confirm.show')}
            </button>
          ) : null}
        </div>
        {role && roleOpen ? (
          <pre className="mt-2 whitespace-pre-wrap break-words rounded-[12px] px-3 py-2 text-[13px] text-[color:var(--cp-text)]" style={{ background: 'color-mix(in srgb, var(--cp-surface-2) 70%, var(--cp-surface))' }}>
            {role}
          </pre>
        ) : null}
      </Section>

      <Section title={t('agentSetup.permission.title')}>
        <p className="text-sm text-[color:var(--cp-text)]">{t('agentSetup.permission.same')}</p>
      </Section>

      <Section title={t('agentSetup.access.title')}>
        <SummaryRows
          rows={[
            [t('agentSetup.confirm.who'), t('agentSetup.confirm.ownerOnly')],
            [t('agentSetup.access.group'), draft.allowGroup ? yes : no],
            [t('agentSetup.confirm.sharing'), t('agentSetup.confirm.sharingOff')],
          ]}
        />
      </Section>

      <Section title={t('agentSetup.confirm.runtime')}>
        <SummaryRows
          rows={[
            [t('agentSetup.loader.label'), 'OpenDAN'],
            [t('agentSetup.template.label'), template ? `${template.show_name || template.name} · ${t(template.source === 'installed' ? 'agentSetup.template.sourceInstalled' : 'agentSetup.template.sourceBundled')}` : '-'],
            [t('agentSetup.confirm.version'), template?.version ?? '-'],
            [t('agentSetup.template.autoUpdate'), draft.templateAutoUpdate ? yes : no],
          ]}
        />
      </Section>

      <Section title={t('agentSetup.confirm.channel')} testId="agent-setup-confirm-channel">
        {draft.channel === 'telegram' && telegram ? (
          <SummaryRows
            rows={[
              [t('agentSetup.confirm.channelType'), 'Telegram'],
              [t('agentSetup.channel.statusIdentity'), t('agentSetup.channel.identityConfigured', undefined, { id: telegram.display_id || telegram.account_id })],
              [t('agentSetup.channel.statusParams'), t('agentSetup.confirm.tokenHidden')],
              [t('agentSetup.channel.statusBinding'), t('agentSetup.channel.bindingOnCreate')],
            ]}
          />
        ) : (
          <p className="text-sm text-[color:var(--cp-text)]">{t('agentSetup.confirm.noChannel')}</p>
        )}
      </Section>
    </div>
  )
}
