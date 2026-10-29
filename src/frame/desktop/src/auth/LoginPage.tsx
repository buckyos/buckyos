import { useCallback, useEffect, useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { accessDenialOf, ANONYMOUS_IDENTITY, DENY_REASONS, type AccessDenial, type DenyReason } from 'buckyos'
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
import { fetchSsoConfig } from '../api/sso'
import { onWorkspaceHost } from '../app/aiworkspace/links'
import { useI18n } from '../i18n/provider'
import { useDocumentTitle } from '../presentation/documentHead'
import { useZonePresentation } from '../presentation/zone'
import { denialText } from '../sso/denial'
import { loadCurrentIdentity } from '../sso/identity'
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

const errorMessage = (err: Exclude<LoginError, { kind: 'denied' }>): string => {
  switch (err.kind) {
    case 'empty_fields':
      return 'Please enter your username and password.'
    case 'invalid_redirect':
      return 'Invalid or missing redirect URL. Unable to continue sign-in.'
    case 'init_failed':
      return `Failed to initialize: ${err.detail}`
    case 'invalid_password':
      return 'Incorrect password. Please check and try again.'
    case 'user_not_found':
      return err.username
        ? `User "${err.username}" does not exist.`
        : 'User does not exist. Please check your username.'
    case 'account_locked':
      return err.username
        ? `Account "${err.username}" has been locked. Please contact the administrator.`
        : 'Your account has been locked. Please contact the administrator.'
    case 'rate_limited':
      return 'Too many login attempts. Please wait a moment and try again.'
    case 'network':
      return 'Unable to reach the server. Please check your network connection.'
    case 'timeout':
      return 'Request timed out. Please try again.'
    case 'server':
      return `Server error: ${err.detail}`
    case 'unknown':
      return err.detail || 'An unexpected error occurred. Please try again.'
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

const LoginPage = () => {
  const { t } = useI18n()
  const [searchParams] = useSearchParams()
  const redirectUrl = useMemo(
    () => searchParams.get('redirect_url') ?? '',
    [searchParams],
  )
  const didErrorText = useDidErrorText(searchParams.get('did_error') ?? '')
  const zone = useZonePresentation()

  const sourceAppId = useMemo(() => {
    if (!redirectUrl) return null
    try {
      const url = new URL(redirectUrl)
      const host = url.hostname.toLowerCase()
      // Extract subdomain prefix as app identifier
      // e.g. "myapp.zone.buckyos.io" => "myapp"
      const parts = host.split('.')
      if (parts.length >= 3) {
        const sub = parts[0]
        if (sub !== 'www' && sub !== 'sys') return sub
      }
      return host
    } catch {
      return null
    }
  }, [redirectUrl])

  const [username, setUsername] = useState(() => searchParams.get('username') ?? '')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<LoginError | null>(null)
  const [didLoginEnabled, setDidLoginEnabled] = useState(false)
  const [showDidLogin, setShowDidLogin] = useState(false)
  const [didErrorOpen, setDidErrorOpen] = useState(true)

  useDocumentTitle('Sign in')

  useEffect(() => {
    let live = true
    void fetchSsoConfig().then(({ data }) => {
      if (live) setDidLoginEnabled(data?.did_login_enabled === true)
    })
    return () => { live = false }
  }, [])

  // A page on a Workspace's own origin (`aiws-<id>.<zone>`, AI Workspace T4-8) sent its visitor here: someone
  // already signed in on this host is handed over through the Control Panel's launch — the one-time callback on
  // that origin signs them in there — without seeing the form. Anyone else signs in as usual.
  const [handoff, setHandoff] = useState(() => {
    try { return !!redirectUrl && onWorkspaceHost(new URL(redirectUrl).host) } catch { return false }
  })
  useEffect(() => {
    if (!handoff) return
    let live = true
    void (async () => {
      const identity = await loadCurrentIdentity().catch(() => ANONYMOUS_IDENTITY)
      if (!live) return
      const launch = identity.kind === 'anonymous' ? null : await requestServiceLaunch('aiworkspace', redirectUrl).catch(() => null)
      if (!live) return
      if (launch?.status === 'ready') window.location.replace(launch.callback_url)
      else setHandoff(false)
    })()
    return () => { live = false }
  }, [handoff, redirectUrl])

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
  // DID login comes back to the page that asked for sign-in, or to the Desktop.
  const didRedirectUrl = redirectUrl || new URL('/', window.location.origin).href

  if (handoff) {
    return (
      <Box sx={{ minHeight: '100dvh', display: 'grid', placeItems: 'center' }} data-testid="login-handoff">
        <CircularProgress size={28} />
      </Box>
    )
  }

  return (
    <AuthPageFrame
      title={<>Sign in to {zone.title}</>}
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
            Enter your credentials to continue
          </Typography>
          {sourceAppId && (
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
                Requested by <strong>{sourceAppId}</strong>
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
          {error.kind === 'denied' ? denialText(t, error.denial) : errorMessage(error)}
        </Alert>
      )}

      {showDidLogin ? (
        <DidLoginForm redirectUrl={didRedirectUrl} onCancel={() => setShowDidLogin(false)} />
      ) : (
        <Box component="form" onSubmit={handleSubmit} noValidate>
          <TextField
            label="Username"
            autoComplete="username"
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
            label="Password"
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
              Forgot password?
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
              'Sign In'
            )}
          </Button>

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
