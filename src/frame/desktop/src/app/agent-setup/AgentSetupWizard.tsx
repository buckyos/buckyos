/* ── Agent setup wizard: identity → runtime → channel → confirm → status ── */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { Alert, Button, CircularProgress, useMediaQuery } from '@mui/material'
import { Check, ChevronLeft, ChevronRight, UserRoundPlus } from 'lucide-react'
import clsx from 'clsx'
import { useI18n } from '../../i18n/provider'
import { fetchCurrentAccount, isLimitedUserType, type CurrentAccount } from '../../api/account'
import { createAgent } from '../../api/user_mgr'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import { useMobileBackHandler } from '../../desktop/windows/MobileNavContext'
import { openUsersAgents } from '../users-agents/launch'
import { ChannelPage } from './ChannelPage'
import { ConfirmPage } from './ConfirmPage'
import { CreationStatusPanel } from './CreationStatusPanel'
import { notifyAgentsChanged } from './events'
import {
  readStoredDraft,
  removeStoredDraft,
  useAgentNameCheck,
  useAgentTemplates,
  useOwnProfile,
  writeStoredDraft,
} from './hooks'
import { AccountPage, PermissionsPage } from './IdentityPages'
import {
  AGENT_LOADER_OPENDAN,
  AGENT_SETUP_PAGES,
  applyTemplateToDraft,
  buildAgentCreateRequest,
  classifyAgentError,
  draftDisplayName,
  isTelegramBotToken,
  newAgentSetupDraft,
  ownerChannelBinding,
  pickTemplate,
  templatesForLoader,
  usableImageUrl,
  type AgentSetupDraft,
  type AgentSetupPage,
  type AgentSetupSource,
  type ClassifiedAgentError,
} from './model'
import { RuntimePage } from './RuntimePage'
import { Section } from './ui'

const stepGroups: Array<{ key: string; pages: AgentSetupPage[] }> = [
  { key: 'identity', pages: ['account', 'permissions'] },
  { key: 'runtime', pages: ['runtime'] },
  { key: 'channel', pages: ['channel'] },
  { key: 'confirm', pages: ['confirm'] },
]

/** Sends the user to sign in again and brings them back to this wizard; the draft stays in storage. */
function relogin(source: AgentSetupSource, agentId: string | null) {
  const back = new URL('/', window.location.origin)
  back.searchParams.set('agent_setup', source)
  if (agentId) back.searchParams.set('agent_id', agentId)
  const login = new URL('/login', window.location.origin)
  login.searchParams.set('redirect_url', back.toString())
  window.location.assign(login.toString())
}

function StepIndicator({ page }: { page: AgentSetupPage }) {
  const { t } = useI18n()
  const activeIndex = stepGroups.findIndex((group) => group.pages.includes(page))
  return (
    <ol className="flex flex-wrap items-center gap-x-2 gap-y-1" data-testid="agent-setup-steps">
      {stepGroups.map((group, index) => {
        const done = index < activeIndex
        const active = index === activeIndex
        return (
          <li key={group.key} className="flex items-center gap-2" aria-current={active ? 'step' : undefined}>
            {index > 0 ? <span aria-hidden="true" className="h-px w-4 sm:w-6" style={{ background: 'var(--cp-border)' }} /> : null}
            <span
              className={clsx('flex size-5 items-center justify-center rounded-full text-[11px] font-semibold')}
              style={active
                ? { background: 'var(--cp-accent)', color: 'var(--cp-surface-opaque)' }
                : done
                  ? { background: 'color-mix(in srgb, var(--cp-accent) 18%, var(--cp-surface))', color: 'var(--cp-accent)' }
                  : { background: 'color-mix(in srgb, var(--cp-muted) 14%, var(--cp-surface))', color: 'var(--cp-muted)' }}
            >
              {done ? <Check size={12} /> : index + 1}
            </span>
            <span className={clsx('text-[12px]', active ? 'font-semibold text-[color:var(--cp-text)]' : 'hidden text-[color:var(--cp-muted)] sm:inline')}>
              {t(`agentSetup.steps.${group.key}`)}
            </span>
          </li>
        )
      })}
    </ol>
  )
}

function WizardFrame({ children, footer, page }: { children: React.ReactNode; footer?: React.ReactNode; page?: AgentSetupPage }) {
  const { t } = useI18n()
  return (
    <div className="flex h-full min-h-0 flex-col bg-[color:var(--cp-bg)]" data-testid="agent-setup">
      <header className="shrink-0 border-b px-4 pb-3 pt-4 sm:px-6" style={{ borderColor: 'var(--cp-border)' }}>
        <div className="mx-auto max-w-2xl">
          {/* On phones the shell's title bar already names the app. */}
          <div className={clsx('items-center gap-2', page ? 'hidden sm:flex' : 'flex')}>
            <UserRoundPlus size={18} className="text-[color:var(--cp-accent)]" />
            <h2 className="font-display text-base font-semibold text-[color:var(--cp-text)]">{t('agentSetup.title')}</h2>
          </div>
          <p className={clsx('mt-1 text-[13px] leading-5 text-[color:var(--cp-muted)]', page ? 'hidden sm:block' : '')}>{t('agentSetup.subtitle')}</p>
          {page ? <div className="sm:mt-3"><StepIndicator page={page} /></div> : null}
        </div>
      </header>
      <div className="desktop-scrollbar min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-6">
        <div className="mx-auto max-w-2xl">{children}</div>
      </div>
      {footer ? (
        <footer className="shrink-0 border-t px-4 py-3 sm:px-6" style={{ borderColor: 'var(--cp-border)' }}>
          <div className="mx-auto flex max-w-2xl flex-wrap items-center justify-end gap-2">{footer}</div>
        </footer>
      ) : null}
    </div>
  )
}

export function AgentSetupWizard({
  source,
  agentId,
  windowId,
}: {
  source: AgentSetupSource
  agentId?: string
  windowId?: string
}) {
  const { t } = useI18n()
  const [account, setAccount] = useState<{ status: 'loading' } | { status: 'ready'; account: CurrentAccount } | { status: 'error'; error: ClassifiedAgentError }>({ status: 'loading' })

  useEffect(() => {
    let cancelled = false
    fetchCurrentAccount()
      .then((value) => {
        if (cancelled) return
        setAccount(value
          ? { status: 'ready', account: value }
          : { status: 'error', error: { kind: 'session_expired', detail: '' } })
      })
      .catch((error: unknown) => {
        if (!cancelled) setAccount({ status: 'error', error: classifyAgentError(error) })
      })
    return () => { cancelled = true }
  }, [])

  if (account.status === 'loading') {
    return (
      <WizardFrame>
        <p role="status" className="flex items-center justify-center gap-2 py-16 text-sm text-[color:var(--cp-muted)]">
          <CircularProgress size={18} />
          {t('agentSetup.loading')}
        </p>
      </WizardFrame>
    )
  }
  if (account.status === 'error') {
    return (
      <WizardFrame>
        <Alert
          severity="warning"
          action={<Button size="small" variant="text" onClick={() => relogin(source, agentId ?? null)}>{t('agentSetup.session.relogin')}</Button>}
        >
          {account.error.kind === 'session_expired' ? t('agentSetup.session.expired') : t('agentSetup.accountFailed', undefined, { detail: account.error.detail })}
        </Alert>
      </WizardFrame>
    )
  }
  if (isLimitedUserType(account.account.user_type)) {
    return (
      <WizardFrame>
        <Alert severity="info" data-testid="agent-setup-limited">{t('agentSetup.error.limited')}</Alert>
      </WizardFrame>
    )
  }
  return <WizardBody account={account.account} source={source} initialAgentId={agentId ?? null} windowId={windowId} />
}

function WizardBody({
  account,
  source,
  initialAgentId,
  windowId,
}: {
  account: CurrentAccount
  source: AgentSetupSource
  initialAgentId: string | null
  windowId?: string
}) {
  const { t } = useI18n()
  const [draft, setDraft] = useState<AgentSetupDraft>(
    () => readStoredDraft(account.user_id, source) ?? newAgentSetupDraft(source, crypto.randomUUID()),
  )
  const [statusAgentId, setStatusAgentId] = useState<string | null>(() => initialAgentId ?? draft.submittedAgentId)
  const [botToken, setBotToken] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<ClassifiedAgentError | null>(null)
  const [nameConflict, setNameConflict] = useState(false)
  const templates = useAgentTemplates()
  const ownProfile = useOwnProfile(account.user_id)
  // From the desktop guide an untouched `jarvis` follows the suggestion the name check returns.
  const nameCheck = useAgentNameCheck(draft.name, statusAgentId === null, (result) => {
    const suggestion = result.suggestion
    if (result.available || !suggestion) return
    setDraft((current) => (current.nameEdited || current.name !== result.name ? current : { ...current, name: suggestion }))
  })

  useEffect(() => {
    writeStoredDraft(account.user_id, draft)
  }, [account.user_id, draft])

  const update = useCallback((patch: Partial<AgentSetupDraft>) => {
    setDraft((current) => ({ ...current, ...patch }))
    if ('name' in patch) setNameConflict(false)
  }, [])

  const compatibleTemplates = useMemo(
    () => templates.state.status === 'ready' ? templatesForLoader(templates.state.templates, AGENT_LOADER_OPENDAN) : [],
    [templates.state],
  )
  const selectedTemplate = pickTemplate(compatibleTemplates, draft.templateId)
  const effectiveDraft = selectedTemplate ? applyTemplateToDraft(draft, selectedTemplate) : draft
  const templateIcon = usableImageUrl(selectedTemplate?.icon)
  const displayName = draftDisplayName(effectiveDraft)
  const telegramIdentity = ownProfile.state.status === 'ready' ? ownerChannelBinding(ownProfile.state.detail, 'telegram') : null
  const channel = draft.channel === 'telegram' && telegramIdentity ? 'telegram' : 'none'
  const didPreview = nameCheck.state.status === 'checked' && nameCheck.state.result.available && nameCheck.state.result.name === draft.name
    ? nameCheck.state.result.agent_did
    : null

  const goTo = useCallback((page: AgentSetupPage) => {
    setSubmitError(null)
    setDraft((current) => ({ ...current, page }))
  }, [])
  const pageIndex = AGENT_SETUP_PAGES.indexOf(draft.page)
  const goBack = useCallback(() => {
    if (pageIndex > 0) goTo(AGENT_SETUP_PAGES[pageIndex - 1])
  }, [goTo, pageIndex])
  const mobile = useMediaQuery('(max-width: 767px)')
  useMobileBackHandler(mobile && statusAgentId === null && pageIndex > 0 ? goBack : null)

  const closeWindow = () => {
    notifyAgentsChanged()
    if (windowId) desktopUIStore.closeWindow(windowId)
  }

  const submit = async () => {
    if (submitting) return
    setSubmitting(true)
    setSubmitError(null)
    const request = buildAgentCreateRequest({ ...effectiveDraft, channel }, botToken)
    const { data, error } = await createAgent(request)
    setSubmitting(false)
    if (data) {
      setDraft((current) => ({ ...current, submittedAgentId: data.agent_id }))
      setStatusAgentId(data.agent_id)
      setBotToken('')
      notifyAgentsChanged()
      return
    }
    const classified = classifyAgentError(error)
    // A definite rejection created nothing, so the next submission is a new request.
    const definite = classified.kind !== 'network' && classified.kind !== 'session_expired' && classified.kind !== 'agent_busy'
    const freshKey = definite ? { idempotencyKey: crypto.randomUUID() } : {}
    if (classified.kind === 'name_conflict') {
      setNameConflict(true)
      setDraft((current) => ({ ...current, ...freshKey, page: 'account' }))
      nameCheck.retry()
      return
    }
    if (classified.kind === 'invalid_bot_token') {
      setDraft((current) => ({ ...current, ...freshKey, page: 'channel' }))
      setSubmitError(classified)
      return
    }
    if (classified.kind === 'owner_identity_missing') {
      setDraft((current) => ({ ...current, ...freshKey, page: 'channel' }))
      ownProfile.reload()
      setSubmitError(classified)
      return
    }
    if (classified.kind === 'template_unavailable') {
      setDraft((current) => ({ ...current, ...freshKey, page: 'runtime' }))
      templates.reload()
      setSubmitError(classified)
      return
    }
    if (definite) setDraft((current) => ({ ...current, ...freshKey }))
    setSubmitError(classified)
  }

  const discardSubmission = () => {
    setStatusAgentId(null)
    setDraft((current) => ({ ...current, submittedAgentId: null, idempotencyKey: crypto.randomUUID(), page: 'confirm' }))
  }

  const finishSubmission = () => {
    removeStoredDraft(account.user_id, source)
    setDraft(newAgentSetupDraft(source, crypto.randomUUID()))
  }

  if (statusAgentId) {
    return (
      <WizardFrame>
        <CreationStatusPanel
          key={statusAgentId}
          agentId={statusAgentId}
          fallbackName={draft.submittedAgentId === statusAgentId ? displayName : undefined}
          fallbackAvatar={draft.submittedAgentId === statusAgentId ? draft.avatar ?? undefined : undefined}
          onReady={finishSubmission}
          onDiscarded={discardSubmission}
          onMissing={discardSubmission}
          onClose={closeWindow}
          onSessionExpired={() => relogin(source, statusAgentId)}
        />
      </WizardFrame>
    )
  }

  const nameReady = nameCheck.state.status === 'checked' && nameCheck.state.result.available
  const tokenReady = channel === 'telegram' && isTelegramBotToken(botToken)
  const relogHere = () => relogin(source, null)

  const yes = t('agentSetup.yes')
  const no = t('agentSetup.no')
  const summaryRows: Array<[string, string]> = [
    [t('agentSetup.summary.agent'), `${displayName} (${effectiveDraft.name})`],
    [t('agentSetup.owner.title'), account.user_name || account.user_id],
    [t('agentSetup.permission.title'), t('agentSetup.permission.same')],
    [t('agentSetup.access.title'), `${t('agentSetup.confirm.ownerOnly')} · ${t('agentSetup.access.group')}: ${effectiveDraft.allowGroup ? yes : no}`],
    [t('agentSetup.loader.label'), 'OpenDAN'],
    [t('agentSetup.template.label'), selectedTemplate ? `${selectedTemplate.show_name || selectedTemplate.name} v${selectedTemplate.version}` : '-'],
    [t('agentSetup.template.autoUpdate'), effectiveDraft.templateAutoUpdate ? yes : no],
    [t('agentSetup.confirm.channel'), t('agentSetup.summary.channelOptional')],
  ]

  const back = pageIndex > 0 ? (
    <Button variant="text" startIcon={<ChevronLeft size={15} />} disabled={submitting} onClick={goBack}>
      {t('agentSetup.back')}
    </Button>
  ) : null
  const next = (enabled: boolean, target: AgentSetupPage) => (
    <Button endIcon={<ChevronRight size={15} />} disabled={!enabled} onClick={() => goTo(target)} data-testid="agent-setup-next">
      {t('agentSetup.next')}
    </Button>
  )

  let body: React.ReactNode
  let footer: React.ReactNode
  switch (draft.page) {
    case 'account':
      body = (
        <AccountPage
          draft={draft}
          update={update}
          nameCheck={nameCheck.state}
          onRetryNameCheck={nameCheck.retry}
          nameConflict={nameConflict}
          templateIcon={templateIcon}
          bio={effectiveDraft.bio}
          onSessionExpired={relogHere}
        />
      )
      footer = next(nameReady, 'permissions')
      break
    case 'permissions':
      body = <PermissionsPage draft={draft} update={update} account={account} />
      footer = <>{back}{next(true, 'runtime')}</>
      break
    case 'runtime':
      body = (
        <RuntimePage
          draft={draft}
          update={update}
          templates={templates.state}
          compatibleTemplates={compatibleTemplates}
          selectedTemplate={selectedTemplate}
          onReloadTemplates={templates.reload}
          summaryRows={summaryRows}
          displayName={displayName}
        />
      )
      footer = <>{back}{next(Boolean(selectedTemplate), 'channel')}</>
      break
    case 'channel':
      body = (
        <ChannelPage
          draft={draft}
          update={update}
          ownProfile={ownProfile.state}
          telegramIdentity={telegramIdentity}
          onRecheck={ownProfile.reload}
          onOpenProfile={() => openUsersAgents({ view: 'self' })}
          botToken={botToken}
          onBotTokenChange={setBotToken}
        />
      )
      footer = (
        <>
          {back}
          <span className="flex-1" />
          <Button
            variant="outlined"
            data-testid="agent-setup-skip-channel"
            onClick={() => {
              setBotToken('')
              setDraft((current) => ({ ...current, channel: 'none', page: 'confirm' }))
            }}
          >
            {t('agentSetup.channel.skip')}
          </Button>
          {next(tokenReady, 'confirm')}
        </>
      )
      break
    case 'confirm':
      body = (
        <>
          {submitError ? (
            <Alert
              severity="error"
              className="mb-4"
              data-testid="agent-setup-submit-error"
              action={submitError.kind === 'session_expired'
                ? <Button size="small" variant="text" onClick={relogHere}>{t('agentSetup.session.relogin')}</Button>
                : undefined}
            >
              {t(`agentSetup.submitError.${submitError.kind}`, t('agentSetup.submitError.other', undefined, { detail: submitError.detail }), { detail: submitError.detail })}
            </Alert>
          ) : null}
          <ConfirmPage
            draft={{ ...effectiveDraft, channel }}
            bio={effectiveDraft.bio}
            account={account}
            didPreview={didPreview}
            template={selectedTemplate}
            telegram={telegramIdentity}
            avatarFallback={templateIcon}
          />
        </>
      )
      footer = (
        <>
          {back}
          <Button
            disabled={submitting || !selectedTemplate || !nameReady}
            startIcon={submitting ? <CircularProgress size={14} color="inherit" /> : <UserRoundPlus size={15} />}
            onClick={() => void submit()}
            data-testid="agent-setup-create"
          >
            {submitting ? t('agentSetup.creating') : t('agentSetup.create')}
          </Button>
        </>
      )
      break
  }

  return (
    <WizardFrame page={draft.page} footer={footer}>
      {submitError && draft.page !== 'confirm' ? (
        <div className="mb-4">
          <Section>
            <p className="text-sm text-[color:var(--cp-danger)]" role="alert">
              {t(`agentSetup.submitError.${submitError.kind}`, t('agentSetup.submitError.other', undefined, { detail: submitError.detail }), { detail: submitError.detail })}
            </p>
          </Section>
        </div>
      ) : null}
      {body}
    </WizardFrame>
  )
}
