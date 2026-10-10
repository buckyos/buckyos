/* ── Agent detail page ── */

import { useState } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { Alert, Button, Chip, CircularProgress, Switch } from '@mui/material'
import { Bot, MessageSquare, Radio, Settings2, Trash2, UserRound } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { desktopUIStore } from '../../../../models/DesktopUIDataModel'
import { getMessageHubStore } from '../../../messagehub/store'
import { openAgentSetup } from '../../../agent-setup/launch'
import { errorMessage, statusTime } from '../../../agent-setup/model'
import { HeaderSection } from '../sections/HeaderSection'
import { useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { agentStatusLabelKey, agentStatusTone } from '../shared/agentLabels'
import type { AgentEntity } from '../../datamodel/types'

const toneColor = {
  success: 'var(--cp-success)',
  warning: 'var(--cp-warning)',
  danger: 'var(--cp-danger)',
  muted: 'var(--cp-muted)',
}

function Card({ icon, title, children, testId }: { icon: React.ReactNode; title: string; children: React.ReactNode; testId?: string }) {
  return (
    <div
      className="rounded-[22px] px-5 py-4"
      data-testid={testId}
      style={{
        background: 'color-mix(in srgb, var(--cp-surface-2) 40%, var(--cp-surface))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 50%, transparent)',
      }}
    >
      <div className="mb-3 flex items-center gap-2">
        <span style={{ color: 'var(--cp-accent)' }}>{icon}</span>
        <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{title}</h3>
      </div>
      {children}
    </div>
  )
}

function Rows({ rows }: { rows: Array<[string, React.ReactNode]> }) {
  return (
    <div className="space-y-1.5">
      {rows.map(([label, value]) => (
        <div key={label} className="flex flex-col gap-0.5 sm:flex-row sm:items-baseline sm:gap-3">
          <span className="w-36 shrink-0 text-[12px] font-medium" style={{ color: 'var(--cp-muted)' }}>{label}</span>
          <span className="min-w-0 break-words text-sm" style={{ color: 'var(--cp-text)' }}>{value}</span>
        </div>
      ))}
    </div>
  )
}

export function AgentDetailPage({ agent, onRemoved }: { agent: AgentEntity; onRemoved?: () => void }) {
  const { t } = useI18n()
  const store = useUsersAgentsStore()
  const location = useLocation(), navigate = useNavigate()
  const [groupPending, setGroupPending] = useState(false)
  const [groupError, setGroupError] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const ready = agent.install?.state === 'ready'
  const unfinished = !ready
  const tone = toneColor[agentStatusTone(agent.status)]

  const openMessages = async (observe: boolean) => {
    const hub = getMessageHubStore()
    // The viewer is always the logged-in user; the store resolves it once.
    try { await hub.initialize() } catch { /* the view reports load errors itself */ }
    const defaultContext = hub.defaultContext()
    const ownerDid = agent.did ?? agent.id
    const context = observe ? { ...defaultContext, ownerDid, mode: 'observe' as const } : defaultContext
    const entityId = observe ? (hub.isMock ? 'did:buckyos:person:alice' : null) : ownerDid
    const payload = { kind: 'messagehub', entityId, context }
    if (location.pathname === '/desktop' || location.pathname === '/') desktopUIStore.openAppWindow('messagehub', { launch: { requestId: crypto.randomUUID(), payload } })
    else navigate(`/messagehub?${new URLSearchParams({ ...(entityId ? { entityId } : {}), ownerDid: context.ownerDid, mode: context.mode })}`)
  }

  const toggleGroup = async (allow: boolean) => {
    setGroupPending(true)
    setGroupError(null)
    const error = await store.setAgentAllowGroup(agent.id, allow)
    setGroupPending(false)
    if (error) setGroupError(t('usersAgents.agent.updateFailed', undefined, { detail: errorMessage(error) }))
  }

  const remove = async () => {
    setDeleting(true)
    setDeleteError(null)
    const error = await store.deleteAgent(agent.id)
    setDeleting(false)
    if (error) {
      setDeleteError(t('usersAgents.agent.deleteFailed', undefined, { detail: errorMessage(error) }))
      return
    }
    onRemoved?.()
  }

  const created = statusTime(agent.install?.created_at)
  const lastError = agent.install?.last_error
  const tunnelState = agent.install?.tunnel_state ?? 'none'

  return (
    <div className="space-y-4" data-testid="agent-detail" data-agent-id={agent.id}>
      <HeaderSection
        name={agent.displayName}
        kind="agent"
        avatarUrl={agent.avatarUrl}
        did={agent.did}
        subtitle={`@${agent.name} · ${t('usersAgents.agent.ownerShort', undefined, { owner: agent.ownerUserId })}`}
        previewUrl={`/profile/${encodeURIComponent(agent.did ?? agent.id)}`}
        badges={
          <>
            <Chip
              label={t(agentStatusLabelKey(agent.status))}
              size="small"
              variant="outlined"
              sx={{ color: `color-mix(in srgb, ${tone} 75%, var(--cp-text))`, borderColor: `color-mix(in srgb, ${tone} 40%, var(--cp-border))` }}
            />
            {agent.createdFromGuide ? <Chip label={t('usersAgents.agent.desktopEntry')} size="small" variant="outlined" /> : null}
          </>
        }
      />

      {ready ? (
        <div className="flex flex-wrap gap-2">
          <button type="button" className="min-h-11 rounded-lg border border-[color:var(--cp-border)] px-4 text-sm" onClick={() => void openMessages(false)}>{t('messagehub.chatWithAgent')}</button>
          <button type="button" className="min-h-11 rounded-lg border border-[color:var(--cp-border)] px-4 text-sm" onClick={() => void openMessages(true)}>{t('messagehub.viewAgentSessions')}</button>
        </div>
      ) : null}

      <Card icon={<Bot size={16} />} title={t('usersAgents.agent.install')} testId="agent-install-status">
        <Rows
          rows={[
            [t('usersAgents.agent.state'), t(agentStatusLabelKey(agent.status))],
            ...(unfinished ? [[t('usersAgents.agent.step'), t(`agentSetup.status.step.${agent.install?.step === 'done' ? 'start' : agent.install?.step ?? 'runtime'}`)] as [string, string]] : []),
            ...(lastError ? [[t('usersAgents.agent.lastError'), `${lastError.message} (${lastError.code})`] as [string, string]] : []),
            ...(created ? [[t('usersAgents.created'), created.toLocaleString()] as [string, string]] : []),
          ]}
        />
        {unfinished ? (
          <div className="mt-3">
            <Button
              size="small"
              variant="outlined"
              onClick={() => openAgentSetup({ source: agent.createdFromGuide ? 'jarvis_guide' : 'users_agents', agent_id: agent.id })}
            >
              {t('usersAgents.agent.viewProgress')}
            </Button>
          </div>
        ) : null}
      </Card>

      <Card icon={<UserRound size={16} />} title={t('usersAgents.section.profile')} testId="agent-profile">
        <Rows
          rows={[
            [t('agentSetup.name.label'), agent.name],
            [t('agentSetup.nickname.label'), agent.nickname ?? t('usersAgents.agent.nicknameFallback')],
            [t('agentSetup.bio.label'), agent.bio ?? t('agentSetup.confirm.empty')],
            [t('agentSetup.owner.title'), agent.ownerUserId],
          ]}
        />
      </Card>

      <Card icon={<MessageSquare size={16} />} title={t('agentSetup.access.title')} testId="agent-access">
        <div className="space-y-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2 text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                {t('agentSetup.access.sharing')}
                <Chip label={t('agentSetup.unsupported')} size="small" variant="outlined" />
              </div>
              <p className="mt-0.5 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('agentSetup.access.sharingUnsupported')}</p>
            </div>
            <Switch checked={false} disabled slotProps={{ input: { 'aria-label': t('agentSetup.access.sharing') } }} />
          </div>
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{t('agentSetup.access.group')}</div>
              <p className="mt-0.5 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('agentSetup.access.groupHint')}</p>
            </div>
            <span className="flex items-center gap-1">
              {groupPending ? <CircularProgress size={14} /> : null}
              <Switch
                checked={agent.settings?.allow_group ?? false}
                disabled={groupPending || !ready}
                onChange={(event) => void toggleGroup(event.target.checked)}
                slotProps={{ input: { 'aria-label': t('agentSetup.access.group') } }}
              />
            </span>
          </div>
          {groupError ? <Alert severity="error">{groupError}</Alert> : null}
        </div>
      </Card>

      <Card icon={<Settings2 size={16} />} title={t('agentSetup.confirm.runtime')} testId="agent-runtime">
        <Rows
          rows={[
            [t('agentSetup.loader.label'), 'OpenDAN'],
            [t('agentSetup.template.label'), agent.template ? `${agent.template.template_id} · ${t(agent.template.source === 'installed' ? 'agentSetup.template.sourceInstalled' : 'agentSetup.template.sourceBundled')}` : '-'],
            [t('agentSetup.confirm.version'), agent.template ? agent.template.loaded_version ?? agent.template.version : '-'],
            [t('agentSetup.template.autoUpdate'), agent.settings?.template_auto_update === false ? t('agentSetup.no') : t('agentSetup.yes')],
            [t('usersAgents.agent.role'), agent.settings?.role_supplement?.trim() || t('agentSetup.confirm.roleEmpty')],
            [t('usersAgents.agent.app'), agent.runtime?.app_instance_id ?? t('usersAgents.agent.appPending')],
            ...(agent.runtime ? [[t('usersAgents.agent.appState'), agent.runtime.state ?? t('usersAgents.agentStatus.unknown')] as [string, string]] : []),
          ]}
        />
      </Card>

      <Card icon={<Radio size={16} />} title={t('usersAgents.agent.channels')} testId="agent-channels">
        {(agent.settings?.msg_tunnels ?? []).length === 0 ? (
          <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.agent.noChannels')}</p>
        ) : (
          <Rows
            rows={(agent.settings?.msg_tunnels ?? []).map((tunnel) => [
              tunnel.platform === 'telegram' ? 'Telegram' : tunnel.platform,
              `${tunnel.bot_account_id ? `@${tunnel.bot_account_id}` : t('usersAgents.agent.channelConfigured')} · ${t(`agentSetup.status.tunnel.${tunnelState}`)}`,
            ])}
          />
        )}
      </Card>

      <div
        className="rounded-[22px] px-5 py-4"
        data-testid="agent-delete"
        style={{
          background: 'color-mix(in srgb, var(--cp-danger) 6%, var(--cp-surface))',
          border: '1px solid color-mix(in srgb, var(--cp-danger) 22%, var(--cp-border))',
        }}
      >
        <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('usersAgents.agent.deleteTitle')}</h3>
        <p className="mt-1 text-[12px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.agent.deleteHint')}</p>
        {deleteError ? <Alert severity="error" className="mt-2">{deleteError}</Alert> : null}
        {confirmDelete ? (
          <div className="mt-3 space-y-2">
            <p className="text-sm" style={{ color: 'var(--cp-text)' }}>{t('usersAgents.agent.deleteConfirm', undefined, { name: agent.displayName })}</p>
            <div className="flex flex-wrap gap-2">
              <Button size="small" variant="text" disabled={deleting} onClick={() => setConfirmDelete(false)}>{t('common.cancel')}</Button>
              <Button size="small" color="error" disabled={deleting} startIcon={deleting ? <CircularProgress size={13} color="inherit" /> : <Trash2 size={14} />} onClick={() => void remove()}>
                {t('usersAgents.agent.deleteAction')}
              </Button>
            </div>
          </div>
        ) : (
          <Button className="mt-3" size="small" color="error" variant="outlined" startIcon={<Trash2 size={14} />} onClick={() => setConfirmDelete(true)}>
            {t('usersAgents.agent.deleteAction')}
          </Button>
        )}
      </div>
    </div>
  )
}
