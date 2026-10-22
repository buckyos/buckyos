/* ── Exclusive invite `/sso/invite/:inviteId#s=<secret>` (SSO-06, contract §5) ──
 *
 * The secret only lives in the fragment and in memory: it is taken out of the address bar at once
 * and goes to Control Panel in request bodies. A different signed-in identity is never replaced
 * silently; the visitor chooses to continue as the invited account or to cancel.
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { useParams } from 'react-router-dom'
import { Alert, Button, CircularProgress } from '@mui/material'
import { ArrowRight, LogIn, Send } from 'lucide-react'
import { accessDenialOf, ANONYMOUS_IDENTITY, type AccessDenial, type CurrentIdentity } from 'buckyos'
import { describeInvite, loginByInvite, requestInviteResend, type InviteDescribeResult, type InviteResendResult } from '../api/sso'
import { AuthPageFrame } from '../auth/AuthPageFrame'
import { endSessionInPlace } from '../auth/signOut'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { DenialNotice } from './DenialNotice'
import { IdentityCard } from './IdentityCard'
import { identityDisplayName, loadCurrentIdentity } from './identity'

const secrets = new Map<string, string>()

/** Takes `#s=` out of the address bar (once) and keeps it in memory for this invite. */
function takeInviteSecret(inviteId: string): string | null {
  const hash = window.location.hash.replace(/^#/, '')
  if (hash) {
    const secret = new URLSearchParams(hash).get('s')
    if (secret) secrets.set(inviteId, secret)
    window.history.replaceState(window.history.state, '', `${window.location.pathname}${window.location.search}`)
  }
  return secrets.get(inviteId) ?? null
}

type Active = Extract<InviteDescribeResult, { status: 'active' }>

type View =
  | { kind: 'loading' }
  | { kind: 'no_secret' }
  | { kind: 'failed' }
  | { kind: 'conflict'; invite: Active; current: CurrentIdentity }
  /** No session could be read, yet this browser shows signs of one: ask before replacing it. */
  | { kind: 'unconfirmed'; invite: Active }
  | { kind: 'cancelled'; current: CurrentIdentity }
  | { kind: 'entering'; invite: Active }
  | { kind: 'denied'; denial: AccessDenial; userId?: string; returnPath?: string; current: CurrentIdentity }

/**
 * Whether this browser looks signed in at this origin although no session was
 * read (the session check failed): the page cookie or a cached identity.
 */
function hasSessionTrace(): boolean {
  try {
    return document.cookie.split(';').some((part) => part.trim().startsWith('buckyos_session_token='))
      || window.localStorage.getItem('buckyos.identity') !== null
      || window.localStorage.getItem('user_info') !== null
  } catch {
    return false
  }
}

function passwordLoginUrl(returnPath: string | undefined, userId: string | undefined): string {
  const url = new URL('/login', window.location.origin)
  url.searchParams.set('redirect_url', new URL(returnPath || '/', window.location.origin).href)
  if (userId) url.searchParams.set('username', userId)
  return `${url.pathname}${url.search}`
}

function ResendPanel({ inviteId }: { inviteId: string }) {
  const { t } = useI18n()
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<InviteResendResult['status'] | 'failed' | null>(null)

  const request = async () => {
    setBusy(true)
    const { data } = await requestInviteResend(inviteId)
    setBusy(false)
    setResult(data?.status ?? 'failed')
  }

  const messages: Record<NonNullable<typeof result>, { severity: 'success' | 'warning' | 'error'; text: string }> = {
    submitted: { severity: 'success', text: t('sso.invite.resend.submitted', 'Request sent. The person who invited you will send a new link through the same channel as before.') },
    rate_limited: { severity: 'warning', text: t('sso.invite.resend.rateLimited', 'A new link was requested a moment ago. Wait a few minutes before asking again.') },
    not_applicable: { severity: 'warning', text: t('sso.invite.resend.notApplicable', 'A new link cannot be requested for this invitation. Contact the person who invited you.') },
    failed: { severity: 'error', text: t('sso.invite.resend.failed', 'The request could not be sent. Try again later.') },
  }

  return (
    <div className="space-y-3">
      {result ? <Alert severity={messages[result].severity} data-testid="sso-invite-resend-result" data-status={result}>{messages[result].text}</Alert> : null}
      {result !== 'submitted' ? (
        <Button fullWidth variant="contained" startIcon={<Send size={16} />} disabled={busy} onClick={() => void request()} data-testid="sso-invite-resend">
          {busy ? <CircularProgress size={18} color="inherit" /> : t('sso.invite.resend.request', 'Request a new link')}
        </Button>
      ) : null}
    </div>
  )
}

export function InvitePage() {
  const { t } = useI18n()
  const { inviteId = '' } = useParams()
  const [secret] = useState(() => takeInviteSecret(inviteId))
  const [view, setView] = useState<View>(() => (secret ? { kind: 'loading' } : { kind: 'no_secret' }))
  const started = useRef(false)

  useDocumentTitle(t('sso.invite.documentTitle', 'Invitation'))

  // Another link of the same invite opened in this tab only changes the fragment: start over with it.
  useEffect(() => {
    const onHashChange = () => {
      if (new URLSearchParams(window.location.hash.replace(/^#/, '')).get('s')) window.location.reload()
    }
    window.addEventListener('hashchange', onHashChange)
    return () => window.removeEventListener('hashchange', onHashChange)
  }, [])

  /** Signs in as the invited account; the session this browser had (the visitor chose to leave it) ends first. */
  const enter = useCallback(async (invite: Active, current: CurrentIdentity) => {
    if (!secret) return
    let viewer = current
    if (current.kind !== 'anonymous') {
      try {
        await endSessionInPlace()
      } catch {
        setView({ kind: 'failed' })
        return
      }
      viewer = ANONYMOUS_IDENTITY
    }
    const { data, error } = await loginByInvite(inviteId, secret)
    if (data?.status === 'ready') {
      window.location.assign(data.callback_url)
      return
    }
    const denial = data?.status === 'denied' ? data.denial : accessDenialOf(error)
    setView(denial ? { kind: 'denied', denial, returnPath: invite.return_path, userId: invite.user_id, current: viewer } : { kind: 'failed' })
  }, [inviteId, secret])

  useEffect(() => {
    if (!secret || started.current) return
    started.current = true
    void (async () => {
      const [described, current] = await Promise.all([describeInvite(inviteId, secret), loadCurrentIdentity().catch((): CurrentIdentity => ANONYMOUS_IDENTITY)])
      const result = described.data
      if (!result) {
        const denial = accessDenialOf(described.error)
        setView(denial ? { kind: 'denied', denial, current } : { kind: 'failed' })
        return
      }
      if (result.status === 'denied') {
        setView({ kind: 'denied', denial: result.denial, userId: result.user_id, returnPath: result.return_path, current })
        return
      }
      if (current.kind === 'account' && current.user_id === result.user_id) {
        window.location.assign(result.return_path)
        return
      }
      if (current.kind !== 'anonymous') {
        setView({ kind: 'conflict', invite: result, current })
        return
      }
      if (hasSessionTrace()) {
        setView({ kind: 'unconfirmed', invite: result })
        return
      }
      setView({ kind: 'entering', invite: result })
      await enter(result, current)
    })()
  }, [enter, inviteId, secret])

  let body: ReactNode
  switch (view.kind) {
    case 'loading':
      body = <div className="flex justify-center py-6"><CircularProgress size={28} /></div>
      break
    case 'entering':
      body = (
        <div className="flex flex-col items-center gap-3 py-4" data-testid="sso-invite-entering">
          <CircularProgress size={28} />
          <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
            {t('sso.invite.entering', 'Signing in as {{name}}…', { name: view.invite.show_name || view.invite.user_id })}
          </p>
        </div>
      )
      break
    case 'no_secret':
      body = (
        <div className="space-y-3" data-testid="sso-invite-no-secret">
          <Alert severity="info">
            {t('sso.invite.noSecret', 'This address is incomplete. Open the invitation link exactly as you received it, or sign in with your password if you have set one.')}
          </Alert>
          <Button fullWidth variant="outlined" startIcon={<LogIn size={16} />} href={passwordLoginUrl(undefined, undefined)}>
            {t('sso.invite.passwordLogin', 'Sign in with password')}
          </Button>
        </div>
      )
      break
    case 'failed':
      body = (
        <Alert severity="error" data-testid="sso-invite-failed">
          {t('sso.invite.failed', 'The invitation cannot be checked right now. Try again later.')}
        </Alert>
      )
      break
    case 'conflict': {
      const invited = view.invite.show_name || view.invite.user_id
      body = (
        <div className="space-y-3" data-testid="sso-invite-conflict">
          <Alert severity="warning">
            {t('sso.invite.conflict', 'You are signed in as {{current}}, but this link belongs to {{invited}}.', { current: identityDisplayName(view.current), invited })}
          </Alert>
          <IdentityCard identity={view.current} compact />
          <Button
            fullWidth
            variant="contained"
            endIcon={<ArrowRight size={16} />}
            onClick={() => { const { invite, current } = view; setView({ kind: 'entering', invite }); void enter(invite, current) }}
            data-testid="sso-invite-conflict-continue"
          >
            {t('sso.invite.continueAs', 'Continue as {{name}}', { name: invited })}
          </Button>
          <Button fullWidth variant="text" onClick={() => setView({ kind: 'cancelled', current: view.current })} data-testid="sso-invite-conflict-cancel">
            {t('sso.common.cancel', 'Cancel')}
          </Button>
        </div>
      )
      break
    }
    case 'unconfirmed': {
      const invited = view.invite.show_name || view.invite.user_id
      body = (
        <div className="space-y-3" data-testid="sso-invite-unconfirmed">
          <Alert severity="warning">
            {t('sso.invite.unconfirmed', 'This browser may still be signed in to another account, which could not be checked. Continuing signs that account out here.')}
          </Alert>
          <Button
            fullWidth
            variant="contained"
            endIcon={<ArrowRight size={16} />}
            onClick={() => {
              const { invite } = view
              setView({ kind: 'entering', invite })
              void endSessionInPlace().catch(() => undefined).then(() => enter(invite, ANONYMOUS_IDENTITY))
            }}
            data-testid="sso-invite-unconfirmed-continue"
          >
            {t('sso.invite.continueAs', 'Continue as {{name}}', { name: invited })}
          </Button>
          <Button fullWidth variant="text" href="/" data-testid="sso-invite-unconfirmed-cancel">
            {t('sso.common.cancel', 'Cancel')}
          </Button>
        </div>
      )
      break
    }
    case 'cancelled':
      body = (
        <div className="space-y-3" data-testid="sso-invite-cancelled">
          <Alert severity="info">
            {t('sso.invite.cancelled', 'Nothing changed: you are still signed in as {{name}}.', { name: identityDisplayName(view.current) })}
          </Alert>
          <Button fullWidth variant="outlined" href="/">
            {t('sso.invite.backHome', 'Back to my home page')}
          </Button>
        </div>
      )
      break
    case 'denied': {
      const { denial } = view
      const showAdjust = view.current.kind === 'account' && !view.current.restricted
      let extra: ReactNode = null
      if (denial.reason === 'credential_state' && denial.detail === 'expired') {
        extra = <ResendPanel inviteId={inviteId} />
      } else if (denial.reason === 'credential_state' && denial.detail === 'password_required') {
        // The link stays an entry: the account's own session goes on, anyone else signs in with the password.
        const signedInAs = view.current.kind === 'account' ? view.current.user_id : null
        const sameAccount = !!signedInAs && signedInAs === view.userId
        extra = (
          <>
            {signedInAs && (sameAccount || !view.userId) ? (
              <Button fullWidth variant="contained" endIcon={<ArrowRight size={16} />} href={view.returnPath || '/'} data-testid="sso-invite-continue-session">
                {t('sso.entry.continueAs', 'Continue as {{name}}', { name: identityDisplayName(view.current) })}
              </Button>
            ) : null}
            {!sameAccount ? (
              <Button fullWidth variant={signedInAs ? 'outlined' : 'contained'} startIcon={<LogIn size={16} />} href={passwordLoginUrl(view.returnPath, view.userId)} data-testid="sso-invite-password-login">
                {t('sso.invite.passwordLogin', 'Sign in with password')}
              </Button>
            ) : null}
          </>
        )
      }
      body = (
        <div className="space-y-3" data-testid="sso-invite-denied" data-reason={denial.reason} data-detail={denial.detail ?? ''}>
          <DenialNotice denial={denial} showAdjust={showAdjust} />
          {extra}
        </div>
      )
      break
    }
  }

  return (
    <AuthPageFrame
      testId="sso-invite-page"
      title={view.kind === 'conflict' || view.kind === 'entering' || view.kind === 'unconfirmed'
        ? t('sso.invite.titleFor', 'Invitation for {{name}}', { name: view.invite.show_name || view.invite.user_id })
        : t('sso.invite.title', 'Invitation')}
      header={(view.kind === 'conflict' || view.kind === 'entering' || view.kind === 'unconfirmed') && view.invite.resource_title ? (
        <p className="mt-1 text-center text-sm" style={{ color: 'var(--cp-muted)' }} data-testid="sso-invite-resource">
          {view.invite.resource_title}
        </p>
      ) : undefined}
    >
      {body}
    </AuthPageFrame>
  )
}
