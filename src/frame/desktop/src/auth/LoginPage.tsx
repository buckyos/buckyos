import { useCallback, useEffect, useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { accessDenialOf, ANONYMOUS_IDENTITY, DENY_REASONS, type AccessDenial, type DenyReason, type PublicOAuthSource } from 'buckyos'
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  IconButton,
  InputAdornment,
  Link,
  TextField,
  Typography,
} from '@mui/material'
import { Eye, EyeOff, ExternalLink, Fingerprint } from 'lucide-react'
import { requestServiceLaunch } from '../api/app_launch'
import { describeLoginTarget, fetchSsoConfig } from '../api/sso'
import { isZoneWorkspaceUrl } from '../app/aiworkspace/links'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { useZonePresentation } from '../presentation/zone'
import { denialText } from '../sso/denial'
import { loadCurrentIdentity } from '../sso/identity'
import { rememberedSharePage } from '../sso/shareReturn'
import { OAuthButtons } from '../sso/OAuthButtons'
import { DidLoginForm } from '../sso/DidLoginForm'
import { AuthPageFrame } from './AuthPageFrame'
import { loginWithPassword } from './passwordLogin'

type LoginError =
  | { kind: 'denied'; denial: AccessDenial }
  | { kind: 'empty_fields' }
  | { kind: 'invalid_redirect' }
  | { kind: 'init_failed'; detail: string }
  | { kind: 'invalid_password' }
  | { kind: 'user_not_found'; username: string }
  | { kind: 'account_locked'; username: string }
  | { kind: 'rate_limited' }
  | { kind: 'network' }
  | { kind: 'timeout' }
  | { kind: 'server'; detail: string }
  | { kind: 'unknown'; detail: string }

const classifyError = (error: unknown): LoginError => {
  const denial = accessDenialOf(error)
  if (denial) return { kind: 'denied', denial }
  const message =
    error instanceof Error ? error.message : String(error ?? '')
  const lower = message.toLowerCase()

  if (lower.includes('invalid password') || lower.includes('invalidpassword')) {
    return { kind: 'invalid_password' }
  }
  if (lower.includes('user not found') || lower.includes('user_not_found')) {
    return { kind: 'user_not_found', username: '' }
  }
  // Redirect-target problems (e.g. "http redirect_url is disabled" when the
  // zone forces https) must be classified before the generic "disabled"
  // check below, which would otherwise report a locked account.
  if (lower.includes('redirect_url') || lower.includes('redirect url')) {
    return { kind: 'invalid_redirect' }
  }
  if (
    /\b(account|user)\b.*\b(locked|disabled)\b/.test(lower) ||
    /\b(locked|disabled)\b/.test(lower)
  ) {
    return { kind: 'account_locked', username: '' }
  }
  // "rate" alone also matches "generate"/"certificate"; require the phrase.
  if (
    lower.includes('rate limit') ||
    lower.includes('rate_limit') ||
    lower.includes('too many') ||
    lower.includes('429')
  ) {
    return { kind: 'rate_limited' }
  }
  if (lower.includes('timeout') || lower.includes('timed out')) {
    return { kind: 'timeout' }
  }
  if (
    lower.includes('network') ||
    lower.includes('fetch') ||
    lower.includes('econnrefused') ||
    lower.includes('failed to fetch')
  ) {
    return { kind: 'network' }
  }
  if (lower.includes('500') || lower.includes('internal server')) {
    return { kind: 'server', detail: message }
  }
  return { kind: 'unknown', detail: message }
}

const errorMessage = (err: Exclude<LoginError, { kind: 'denied' }>, t: ReturnType<typeof useI18n>['t']): string => {
  switch (err.kind) {
    case 'empty_fields':
      return t('sso.login.error.empty_fields')
    case 'invalid_redirect':
      return t('sso.login.error.invalid_redirect')
    case 'init_failed':
      return t('sso.login.error.init_failed', undefined, { detail: err.detail })
    case 'invalid_password':
      return t('sso.login.error.invalid_password')
    case 'user_not_found':
      return err.username
        ? t('sso.login.error.user_not_found_named', undefined, { username: err.username })
        : t('sso.login.error.user_not_found')
    case 'account_locked':
      return err.username
        ? t('sso.login.error.account_locked_named', undefined, { username: err.username })
        : t('sso.login.error.account_locked')
    case 'rate_limited':
      return t('sso.login.error.rate_limited')
    case 'network':
      return t('sso.login.error.network')
    case 'timeout':
      return t('sso.login.error.timeout')
    case 'server':
      return t('sso.login.error.server', undefined, { detail: err.detail })
    case 'unknown':
      return err.detail || t('sso.login.error.unknown')
  }
}

/** `did_error` codes the DID callback sends back: a denial reason, or one of the flow outcomes below. */
function useDidErrorText(code: string): string | null {
  const { t } = useI18n()
  if (!code) return null
  if (code === 'canonical_unavailable') {
    return t('sso.login.didError.canonicalUnavailable', 'The home Zone of your DID could not be reached. Try again later.')
  }
  if (code === 'access_denied') return t('sso.login.didError.cancelled', 'DID sign-in was cancelled on your home Zone.')
  if (code === 'identity_conflict') return t('sso.login.didError.identityConflict', 'This browser is signed in as someone else. Sign out first, then use DID sign-in again.')
  if ((DENY_REASONS as readonly string[]).includes(code)) {
    return denialText(t, { reason: code as DenyReason, adjust: { party: 'none' } })
  }
  return t('sso.login.didError.failed', 'DID sign-in did not complete. Try again.')
}

/** The system service whose pages live on the origin of `address`, when the Zone login can hand its own session
 * over to it: an AI Workspace on `aiws-<id>.<zone>`, the personal site and reflow pages nfs-server serves on
 * `reflow.<zone>` (Personal Site Protocol §7). */
function handoffService(address: string): 'aiworkspace' | 'nfs-server' | null {
  if (isZoneWorkspaceUrl(address)) return 'aiworkspace'
  if (!URL.canParse(address)) return null
  const url = new URL(address)
  const { protocol, host } = window.location
  const reflow = url.host === `reflow.${host}` || url.host === `reflow-${host}`
  return reflow && url.protocol === protocol && !url.username && !url.password ? 'nfs-server' : null
}

/** On failure, the DID callback wraps its original return page in another /login URL. */
function loginDestination(params: URLSearchParams): string {
  const destination = params.get('redirect_url') ?? ''
  if (params.has('did_error') && URL.canParse(destination)) {
    const back = new URL(destination)
    const page = back.searchParams.get('redirect_url')
    if (back.origin === window.location.origin && back.pathname === '/login'
      && back.searchParams.get('did_return') === '1' && page && handoffService(page)) {
      return page
    }
  }
  return destination
}

const LoginPage = () => {
  const { t } = useI18n()
  const [searchParams] = useSearchParams()
  const redirectUrl = useMemo(
    () => loginDestination(searchParams),
    [searchParams],
  )
  const didErrorText = useDidErrorText(searchParams.get('did_error') ?? '')
  const didReturned = searchParams.get('did_return') === '1'
  const service = handoffService(redirectUrl)
  const zone = useZonePresentation()

  const [requester, setRequester] = useState<{ url: string; name: string } | null>(null)
  const requesterName = requester?.url === redirectUrl ? requester.name : null

  useEffect(() => {
    if (!redirectUrl) return
    let live = true
    void describeLoginTarget(redirectUrl).then(({ data }) => {
      if (live) setRequester(data ? { url: redirectUrl, name: data.display_name } : null)
    })
    return () => { live = false }
  }, [redirectUrl])

  const [username, setUsername] = useState(() => searchParams.get('username') ?? '')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<LoginError | null>(null)
  const [providers,setProviders] = useState<PublicOAuthSource[]>([])
  const [didLoginEnabled, setDidLoginEnabled] = useState(false)
  const [showDidLogin, setShowDidLogin] = useState(false)
  const [didErrorOpen, setDidErrorOpen] = useState(true)

  useDocumentTitle(t('sso.userInvite.signIn'))

  useEffect(() => {
    let live = true
    void fetchSsoConfig().then(({ data }) => {
      if (!live) return
      if (data?.zone_origin && window.location.origin !== data.zone_origin &&
          window.location.hostname === 'sys.' + new URL(data.zone_origin).hostname) {
        const root = new URL('/login',data.zone_origin)
        root.search = window.location.search
        root.hash = window.location.hash
        window.location.replace(root.href)
        return
      }
      const remembered = rememberedSharePage(redirectUrl,window.location.hash)
      if (remembered) { window.location.replace(remembered); return }
      if (live) { setDidLoginEnabled(data?.did_login_enabled === true); setProviders(data?.oauth_sources ?? []) }
    })
    return () => { live = false }
  }, [redirectUrl])

  // A page on a Workspace's own origin (`aiws-<id>.<zone>`, AI Workspace T4-8) or on `reflow.<zone>` sent its
  // visitor here: someone already signed in on this host is handed over through the Control Panel's launch — the
  // one-time callback on that origin signs them in there — without seeing the form. Anyone else signs in as usual.
  const [handoff, setHandoff] = useState(service !== null && !didErrorText)
  const [handoffFailed, setHandoffFailed] = useState(false)
  useEffect(() => {
    if (!handoff) return
    let live = true
    void (async () => {
      const identity = await loadCurrentIdentity().catch(() => ANONYMOUS_IDENTITY)
      if (!live) return
      const launch = identity.kind === 'anonymous' || !service ? null : await requestServiceLaunch(service, redirectUrl).catch(() => null)
      if (!live) return
      if (launch?.status === 'ready') window.location.replace(launch.callback_url)
      else {
        setHandoffFailed(service === 'aiworkspace' && (didReturned || identity.kind !== 'anonymous'))
        setHandoff(false)
      }
    })()
    return () => { live = false }
  }, [handoff, service, redirectUrl, didReturned])

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault()
      if (submitting) return

      const trimmedUsername = username.trim()
      if (!trimmedUsername || !password) {
        setError({ kind: 'empty_fields' })
        return
      }

      setError(null)
      setSubmitting(true)

      try {
        window.location.href = await loginWithPassword(trimmedUsername, password, redirectUrl)
        // Keep the form disabled: the navigation above is asynchronous and a
        // re-enabled button would allow a second login round-trip (new nonce,
        // second pending SSO login on the server) before the page unloads.
      } catch (err) {
        console.error('[login] error caught:', err)
        const classified = classifyError(err)
        if (
          classified.kind === 'user_not_found' ||
          classified.kind === 'account_locked'
        ) {
          ;(classified as { username: string }).username = trimmedUsername
        }
        setError(classified)
        setSubmitting(false)
      }
    },
    [username, password, submitting, redirectUrl],
  )

  const disabled = submitting
  // Federation sets the Zone root's cookie. Resume here to launch the page on its own host.
  let didRedirectUrl = redirectUrl || new URL('/', window.location.origin).href
  if (service) {
    const didReturn = new URL('/login', window.location.origin)
    didReturn.searchParams.set('redirect_url', redirectUrl)
    didReturn.searchParams.set('did_return', '1')
    didRedirectUrl = didReturn.href
  }

  if (handoff) {
    return (
      <Box sx={{ minHeight: '100dvh', display: 'grid', placeItems: 'center' }} data-testid="login-handoff">
        <CircularProgress size={28} />
      </Box>
    )
  }

  return (
    <AuthPageFrame
      title={t('sso.login.zone', undefined, { zone: zone.title })}
      header={
        <>
          {zone.description && (
            <Typography
              variant="body2"
              color="text.secondary"
              sx={{ mt: 0.5, textAlign: 'center' }}
              data-testid="login-zone-description"
            >
              {zone.description}
            </Typography>
          )}
          <Typography
            variant="body2"
            color="text.secondary"
            sx={{ mt: 0.5 }}
          >
            {t('sso.login.credentials')}
          </Typography>
          {requesterName && (
            <Box
              sx={{
                mt: 1.5,
                display: 'inline-flex',
                alignItems: 'center',
                gap: 0.75,
                px: 1.5,
                py: 0.5,
                borderRadius: 2,
                bgcolor: 'color-mix(in srgb, var(--cp-accent-soft) 18%, transparent)',
                border: '1px solid color-mix(in srgb, var(--cp-accent) 20%, var(--cp-border))',
              }}
            >
              <ExternalLink size={14} style={{ opacity: 0.6 }} />
              <Typography variant="caption" color="text.secondary">
                {t('sysdlg.select.requestedBy', undefined, { name: requesterName })}
              </Typography>
            </Box>
          )}
        </>
      }
    >
      {didErrorText && didErrorOpen && (
        <Alert severity="error" sx={{ mb: 2.5 }} onClose={() => setDidErrorOpen(false)} data-testid="login-did-error">
          {didErrorText}
        </Alert>
      )}

      {handoffFailed && (
        <Alert severity="error" sx={{ mb: 2.5 }} data-testid="login-workspace-error">
          {t('sso.login.workspaceFailed', 'Could not open this workspace with your current sign-in. Reload to retry or sign in again.')}
        </Alert>
      )}

      {error && (
        <Alert
          severity={
            error.kind === 'rate_limited' || error.kind === 'account_locked'
              ? 'warning'
              : 'error'
          }
          sx={{ mb: 2.5 }}
          onClose={() => setError(null)}
          data-testid="login-error"
        >
          {error.kind === 'denied' ? denialText(t, error.denial) : errorMessage(error, t)}
        </Alert>
      )}

      {showDidLogin ? (
        <DidLoginForm redirectUrl={didRedirectUrl} onCancel={() => setShowDidLogin(false)} />
      ) : (
        <Box component="form" onSubmit={handleSubmit} noValidate>
          <TextField
            label={t('sso.login.username')}
            autoComplete="username"
            slotProps={{ htmlInput: { autoCapitalize: 'none', autoCorrect: 'off', spellCheck: false } }}
            autoFocus
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            disabled={disabled}
            error={
              error?.kind === 'empty_fields' && !username.trim()
                ? true
                : error?.kind === 'user_not_found'
            }
            sx={{ mb: 2 }}
          />

          <TextField
            id="login-password"
            label={t('sso.register.password')}
            type={showPassword ? 'text' : 'password'}
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            disabled={disabled}
            error={
              error?.kind === 'empty_fields' && !password
                ? true
                : error?.kind === 'invalid_password'
            }
            slotProps={{
              input: {
                endAdornment: (
                  <InputAdornment position="end">
                    <IconButton
                      size="small"
                      aria-label={t('sso.login.showCharacters')}
                      aria-pressed={showPassword}
                      aria-controls="login-password"
                      onClick={() => setShowPassword((v) => !v)}
                      edge="end"
                      sx={{ border: 'none' }}
                    >
                      {showPassword ? (
                        <EyeOff size={18} />
                      ) : (
                        <Eye size={18} />
                      )}
                    </IconButton>
                  </InputAdornment>
                ),
              },
            }}
            sx={{ mb: 1.5 }}
          />

          <Box
            sx={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'flex-end',
              mb: 2.5,
            }}
          >
            <Link
              href="#"
              variant="body2"
              underline="hover"
              onClick={(e) => {
                e.preventDefault()
                // TODO: navigate to forgot-password flow
              }}
              sx={{ whiteSpace: 'nowrap' }}
            >
              {t('sso.login.forgotPassword')}
            </Link>
          </Box>

          <Button
            type="submit"
            fullWidth
            size="large"
            disabled={disabled}
            sx={{ position: 'relative' }}
          >
            {submitting ? (
              <CircularProgress size={22} color="inherit" />
            ) : (
              t('sso.login.submit')
            )}
          </Button>

          <OAuthButtons sources={providers} purpose="login" redirectUrl={didRedirectUrl} />
          {didLoginEnabled && (
            <Button
              fullWidth
              variant="text"
              startIcon={<Fingerprint size={16} />}
              disabled={disabled}
              onClick={() => setShowDidLogin(true)}
              sx={{ mt: 1.5 }}
              data-testid="login-use-did"
            >
              {t('sso.login.useDid', 'Sign in with a DID')}
            </Button>
          )}
        </Box>
      )}
    </AuthPageFrame>
  )
}

export default LoginPage
