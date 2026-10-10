/* ── Creation status panel: polls `agent.create.status` until ready or failed ── */

import { useEffect, useRef, useState } from 'react'
import { Alert, Button, CircularProgress, LinearProgress } from '@mui/material'
import { AlertCircle, CheckCircle2, Circle, CircleDashed, Loader2, MinusCircle } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import {
  cancelAgentCreate,
  deleteAgent,
  fetchAgentCreateStatus,
  fetchAgentDetail,
  retryAgentCreate,
  type AgentEntry,
  type AgentStatus,
} from '../../api/user_mgr'
import { notifyAgentsChanged } from './events'
import {
  agentDisplayName,
  canCancelCreation,
  classifyAgentError,
  creationStepPhases,
  isCreationInProgress,
  tunnelFailureKey,
  usableImageUrl,
  type ClassifiedAgentError,
  type CreationStepPhase,
} from './model'
import { AgentAvatar, Section, StatusPill } from './ui'

const POLL_MS = 1500
const POLL_RETRY_MS = 4000

type PendingAction = 'retry' | 'skip' | 'discard'

function StepIcon({ phase }: { phase: CreationStepPhase }) {
  switch (phase) {
    case 'done':
      return <CheckCircle2 size={18} className="text-[color:var(--cp-success)]" />
    case 'active':
      return <Loader2 size={18} className="animate-spin text-[color:var(--cp-accent)]" />
    case 'failed':
      return <AlertCircle size={18} className="text-[color:var(--cp-danger)]" />
    case 'skipped':
      return <MinusCircle size={18} className="text-[color:var(--cp-muted)]" />
    default:
      return <Circle size={18} className="text-[color:color-mix(in_srgb,var(--cp-muted)_60%,transparent)]" />
  }
}

export function CreationStatusPanel({
  agentId,
  fallbackName,
  fallbackAvatar,
  onReady,
  onDiscarded,
  onMissing,
  onClose,
  onSessionExpired,
}: {
  agentId: string
  fallbackName?: string
  fallbackAvatar?: string
  onReady: () => void
  onDiscarded: () => void
  onMissing: () => void
  onClose: () => void
  onSessionExpired: () => void
}) {
  const { t } = useI18n()
  const [status, setStatus] = useState<AgentStatus | null>(null)
  const [entry, setEntry] = useState<AgentEntry | null>(null)
  const [pollError, setPollError] = useState<ClassifiedAgentError | null>(null)
  const [pending, setPending] = useState<PendingAction | null>(null)
  const [actionError, setActionError] = useState<ClassifiedAgentError | null>(null)
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const [pollRound, setPollRound] = useState(0)
  const onReadyRef = useRef(onReady)
  useEffect(() => { onReadyRef.current = onReady })

  useEffect(() => {
    let cancelled = false
    void fetchAgentDetail(agentId).then(({ data }) => {
      if (!cancelled && data) setEntry(data)
    })
    return () => { cancelled = true }
  }, [agentId, pollRound])

  useEffect(() => {
    let cancelled = false
    let timer: number | undefined
    const tick = async () => {
      const { data, error } = await fetchAgentCreateStatus(agentId)
      if (cancelled) return
      if (data) {
        setStatus(data)
        setPollError(null)
        if (isCreationInProgress(data)) timer = window.setTimeout(() => void tick(), POLL_MS)
        else if (data.state === 'ready') {
          void fetchAgentDetail(agentId).then(({ data: detail }) => { if (!cancelled && detail) setEntry(detail) })
          notifyAgentsChanged()
          onReadyRef.current()
        }
        return
      }
      const classified = classifyAgentError(error)
      setPollError(classified)
      if (classified.kind === 'not_found' || classified.kind === 'session_expired') return
      timer = window.setTimeout(() => void tick(), POLL_RETRY_MS)
    }
    void tick()
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [agentId, pollRound])

  const name = entry ? agentDisplayName(entry) : fallbackName || agentId.split('.')[0]
  const avatar = usableImageUrl(entry?.profile?.avatar) ?? fallbackAvatar

  const run = async (action: PendingAction) => {
    if (pending || !status) return
    setPending(action)
    setActionError(null)
    if (action === 'discard') {
      const result = canCancelCreation(status) ? await cancelAgentCreate(agentId) : await deleteAgent(agentId)
      setPending(null)
      setConfirmDiscard(false)
      if (result.data) {
        notifyAgentsChanged()
        onDiscarded()
      } else setActionError(classifyAgentError(result.error))
      return
    }
    const { data, error } = await retryAgentCreate({ agentId, skipTunnel: action === 'skip' })
    setPending(null)
    if (data) {
      setStatus(data)
      setPollRound((value) => value + 1)
    } else setActionError(classifyAgentError(error))
  }

  if (pollError?.kind === 'not_found') {
    return (
      <Section testId="agent-setup-status-missing">
        <Alert severity="warning">{t('agentSetup.status.missing')}</Alert>
        <div className="mt-3 flex justify-end">
          <Button onClick={onMissing}>{t('agentSetup.status.restart')}</Button>
        </div>
      </Section>
    )
  }

  if (!status) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-16 text-sm text-[color:var(--cp-muted)]" role="status">
        {pollError?.kind === 'session_expired' ? (
          <Alert severity="warning" action={<Button size="small" variant="text" onClick={onSessionExpired}>{t('agentSetup.session.relogin')}</Button>}>
            {t('agentSetup.session.expired')}
          </Alert>
        ) : (
          <>
            <CircularProgress size={22} />
            {pollError ? t('agentSetup.status.pollRetry') : t('agentSetup.status.loading')}
          </>
        )}
      </div>
    )
  }

  const phases = creationStepPhases(status)
  const failed = status.state === 'failed'
  const ready = status.state === 'ready'
  const failedStep = failed ? status.last_error?.step ?? status.step : null
  const tunnelFailed = failedStep === 'tunnel'
  const tunnelReasonKey = tunnelFailed ? tunnelFailureKey(status.last_error?.code) : null
  const progress = status.runtime_progress
  const cancellable = canCancelCreation(status)

  return (
    <div className="space-y-4" data-testid="agent-setup-status" data-state={status.state}>
      <Section>
        <div className="flex items-center gap-3">
          <AgentAvatar name={name} src={avatar} size={56} />
          <div className="min-w-0 flex-1">
            <div className="truncate font-display text-lg font-semibold text-[color:var(--cp-text)]" data-testid="agent-setup-status-name">{name}</div>
            <div className="truncate text-[12px] text-[color:var(--cp-muted)]">{status.agent_did}</div>
          </div>
          {ready ? (
            <StatusPill tone="success">{t('agentSetup.status.stateReady')}</StatusPill>
          ) : failed ? (
            <StatusPill tone="danger">{t('agentSetup.status.stateFailed')}</StatusPill>
          ) : (
            <StatusPill tone="accent">{t('agentSetup.status.stateCreating')}</StatusPill>
          )}
        </div>
        {ready ? (
          <Alert severity="success" className="mt-4" data-testid="agent-setup-status-success">
            {t('agentSetup.status.success', undefined, { name })}
          </Alert>
        ) : failed ? null : (
          <p className="mt-4 text-[13px] leading-5 text-[color:var(--cp-muted)]">
            {t('agentSetup.status.creatingHint', undefined, { name })}
          </p>
        )}
        {pollError && !failed && !ready ? (
          <Alert
            severity="warning"
            className="mt-3"
            action={pollError.kind === 'session_expired'
              ? <Button size="small" variant="text" onClick={onSessionExpired}>{t('agentSetup.session.relogin')}</Button>
              : undefined}
          >
            {pollError.kind === 'session_expired' ? t('agentSetup.session.expired') : t('agentSetup.status.pollRetry')}
          </Alert>
        ) : null}
      </Section>

      <Section title={t('agentSetup.status.stepsTitle')}>
        <ol className="space-y-3" data-testid="agent-setup-status-steps">
          {phases.map(({ step, phase }) => (
            <li key={step} className="flex items-start gap-3" data-step={step} data-phase={phase}>
              <span className="mt-0.5 shrink-0"><StepIcon phase={phase} /></span>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2 text-sm font-medium text-[color:var(--cp-text)]">
                  {t(`agentSetup.status.step.${step}`)}
                  {phase === 'skipped' ? <StatusPill tone="muted">{t('agentSetup.status.skipped')}</StatusPill> : null}
                </div>
                <p className="text-[12px] leading-5 text-[color:var(--cp-muted)]">{t(`agentSetup.status.step.${step}Hint`)}</p>
                {step === 'runtime' && phase === 'active' && progress ? (
                  <div className="mt-1.5 space-y-1">
                    <LinearProgress
                      variant={typeof progress.percent === 'number' ? 'determinate' : 'indeterminate'}
                      value={typeof progress.percent === 'number' ? Math.max(0, Math.min(100, progress.percent)) : undefined}
                    />
                    {progress.message ? <p className="text-[12px] text-[color:var(--cp-muted)]">{progress.message}</p> : null}
                  </div>
                ) : null}
                {phase === 'failed' && status.last_error ? (
                  <div className="mt-1.5 rounded-[12px] px-3 py-2 text-[13px] leading-5" style={{ background: 'color-mix(in srgb, var(--cp-danger) 10%, var(--cp-surface))' }} data-testid="agent-setup-status-error">
                    <p className="text-[color:var(--cp-text)]">{status.last_error.message}</p>
                    <p className="text-[11px] text-[color:var(--cp-muted)]">{t('agentSetup.status.errorCode', undefined, { code: status.last_error.code })}</p>
                  </div>
                ) : null}
              </div>
            </li>
          ))}
        </ol>
        {status.tunnel_state !== 'none' ? (
          <div className="mt-4 flex flex-wrap items-center gap-2 border-t pt-3 text-[13px]" style={{ borderColor: 'var(--cp-border)' }} data-testid="agent-setup-status-channel">
            <span className="text-[color:var(--cp-muted)]">{t('agentSetup.status.channelState')}</span>
            <StatusPill tone={status.tunnel_state === 'bound' ? 'success' : status.tunnel_state === 'failed' ? 'danger' : 'muted'}>
              {t(`agentSetup.status.tunnel.${status.tunnel_state}`)}
            </StatusPill>
          </div>
        ) : null}
      </Section>

      {failed ? (
        <Section testId="agent-setup-status-failure">
          <p className="text-sm text-[color:var(--cp-text)]">
            {tunnelFailed ? t('agentSetup.status.tunnelFailed') : t('agentSetup.status.failed', undefined, { step: t(`agentSetup.status.step.${failedStep ?? 'runtime'}`) })}
          </p>
          {tunnelReasonKey ? (
            <p className="mt-1 text-[13px] leading-5 text-[color:var(--cp-muted)]" data-testid="agent-setup-tunnel-reason">{t(tunnelReasonKey)}</p>
          ) : null}
          {actionError ? (
            <Alert severity="error" className="mt-3">{t('agentSetup.status.actionFailed', undefined, { detail: actionError.detail })}</Alert>
          ) : null}
          {confirmDiscard ? (
            <div className="mt-3 rounded-[14px] border px-3 py-3" style={{ borderColor: 'color-mix(in srgb, var(--cp-danger) 36%, var(--cp-border))' }} data-testid="agent-setup-discard-confirm">
              <p className="text-[13px] leading-5 text-[color:var(--cp-text)]">
                {cancellable ? t('agentSetup.status.cancelConfirm') : t('agentSetup.status.deleteConfirm', undefined, { name })}
              </p>
              <div className="mt-2 flex flex-wrap justify-end gap-2">
                <Button size="small" variant="text" disabled={pending !== null} onClick={() => setConfirmDiscard(false)}>{t('agentSetup.keep')}</Button>
                <Button size="small" color="error" disabled={pending !== null} onClick={() => void run('discard')}>
                  {pending === 'discard' ? <CircularProgress size={14} color="inherit" /> : cancellable ? t('agentSetup.status.cancel') : t('agentSetup.status.delete')}
                </Button>
              </div>
            </div>
          ) : (
            <div className="mt-3 flex flex-wrap justify-end gap-2">
              <Button variant="text" disabled={pending !== null} onClick={() => setConfirmDiscard(true)}>
                {cancellable ? t('agentSetup.status.cancel') : t('agentSetup.status.delete')}
              </Button>
              {tunnelFailed ? (
                <Button variant="outlined" disabled={pending !== null} onClick={() => void run('skip')}>
                  {pending === 'skip' ? <CircularProgress size={14} color="inherit" /> : t('agentSetup.status.skipTunnel')}
                </Button>
              ) : null}
              <Button disabled={pending !== null || status.last_error?.retryable === false} onClick={() => void run('retry')}>
                {pending === 'retry' ? <CircularProgress size={14} color="inherit" /> : tunnelFailed ? t('agentSetup.status.retryTunnel') : t('agentSetup.status.retry')}
              </Button>
            </div>
          )}
        </Section>
      ) : null}

      {ready ? (
        <div className="flex justify-end">
          <Button onClick={onClose} data-testid="agent-setup-close">{t('common.close')}</Button>
        </div>
      ) : !failed ? (
        <p className="flex items-center gap-2 text-[12px] text-[color:var(--cp-muted)]">
          <CircleDashed size={13} />
          {t('agentSetup.status.background')}
        </p>
      ) : null}
    </div>
  )
}
