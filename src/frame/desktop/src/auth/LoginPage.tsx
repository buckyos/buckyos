import { useCallback, useEffect, useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { buckyos } from 'buckyos'
import {
  Alert,
  Box,
  Button,
  Checkbox,
  CircularProgress,
  Container,
  FormControlLabel,
  IconButton,
  InputAdornment,
  Link,
  Paper,
  TextField,
  Typography,
} from '@mui/material'
import { Eye, EyeOff, Lock, ExternalLink } from 'lucide-react'

const APP_ID = 'control-panel'

type LoginError =
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
  const message =
    error instanceof Error ? error.message : String(error ?? '')
  const lower = message.toLowerCase()

  if (lower.includes('invalid password') || lower.includes('invalidpassword')) {
    return { kind: 'invalid_password' }
  }
  if (lower.includes('user not found') || lower.includes('user_not_found')) {
    return { kind: 'user_not_found', username: '' }
  }
  if (lower.includes('locked') || lower.includes('disabled')) {
    return { kind: 'account_locked', username: '' }
  }
  if (lower.includes('rate') || lower.includes('too many') || lower.includes('429')) {
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

const errorMessage = (err: LoginError): string => {
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

const buildSsoCallbackUrl = (
  redirectUrl: string,
  nonce: number,
): string => {
  // Callback must be served on the redirect target origin so Set-Cookie applies to that host
  // (gateway matches path /sso_callback and forwards to control_panel for any app host).
  const target = new URL(redirectUrl, window.location.origin)
  const callback = new URL('/sso_callback', target.origin)
  callback.searchParams.set('nonce', String(nonce))
  callback.searchParams.set('redirect_url', redirectUrl)
  return callback.toString()
}

const LoginPage = () => {
  const [searchParams] = useSearchParams()
  const redirectUrl = useMemo(
    () => searchParams.get('redirect_url') ?? '',
    [searchParams],
  )
  const appid = useMemo(
    () => searchParams.get('appid') ?? APP_ID,
    [searchParams],
  )

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

  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [rememberMe, setRememberMe] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<LoginError | null>(null)

  useEffect(() => {
    document.title = 'BuckyOS | Login'
  }, [])

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault()
      // console.log('[login] handleSubmit fired, submitting =', submitting)
      if (submitting) return

      const trimmedUsername = username.trim()
      if (!trimmedUsername || !password) {
        // console.log('[login] empty fields, aborting')
        setError({ kind: 'empty_fields' })
        return
      }

      setError(null)
      setSubmitting(true)

      try {
        const nonce = Date.now()
        // console.log('[login] hashing password for', trimmedUsername, 'nonce =', nonce)
        const passwordHash = buckyos.hashPassword(
          trimmedUsername,
          password,
          nonce,
        )
        // console.log('[login] passwordHash done')

        const rpcClient = new buckyos.kRPCClient('/kapi/control-panel')
        rpcClient.setSeq(nonce)
        console.log('[login] calling auth.login...', redirectUrl)

        const response = (await rpcClient.call('auth.login', {
          username: trimmedUsername,
          password: passwordHash,
          appid,
          login_nonce: nonce,
          remember_me: rememberMe,
          ...(redirectUrl ? { redirect_url: redirectUrl } : {}),
        })) as Record<string, unknown>
        // console.log('[login] response', response)

        const sessionToken =
          typeof response.session_token === 'string'
            ? response.session_token.trim()
            : ''
        if (!sessionToken) {
          throw new Error('Login succeeded but no session token returned')
        }

        const ssoNonce =
          typeof response.sso_nonce === 'number' ? response.sso_nonce : 0
        // console.log('[login] sessionToken ok, ssoNonce =', ssoNonce, 'redirectUrl =', redirectUrl)

        if (redirectUrl && ssoNonce > 0) {
          // const callbackUrl = buildSsoCallbackUrl(redirectUrl, ssoNonce)
          // console.log('[login] callbackUrl =', callbackUrl)
          // alert(callbackUrl)
          window.location.href = buildSsoCallbackUrl(redirectUrl, ssoNonce)
        } else if (redirectUrl) {
          window.location.href = redirectUrl
        } else {
          window.location.href = '/'
        }
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
      } finally {
        setSubmitting(false)
      }
    },
    [username, password, submitting, appid, redirectUrl, rememberMe],
  )

  const disabled = submitting

  return (
    <Box
      sx={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: { xs: 'flex-start', sm: 'center' },
        justifyContent: 'center',
        background:
          'linear-gradient(135deg, var(--cp-bg) 0%, var(--cp-bg-strong) 100%)',
        px: 2,
        pt: { xs: 6, sm: 4 },
        pb: 4,
      }}
    >
      <Container maxWidth="xs" disableGutters>
        <Paper
          elevation={0}
          sx={{
            p: { xs: 3, sm: 4 },
            borderRadius: 4,
            border: '1px solid var(--cp-border)',
            backdropFilter: 'blur(24px)',
            boxShadow: 'var(--cp-panel-shadow)',
          }}
        >
          <Box
            sx={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              mb: 3,
            }}
          >
            <Box
              sx={{
                width: 48,
                height: 48,
                borderRadius: 3,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                bgcolor: 'primary.main',
                color: 'white',
                mb: 2,
              }}
            >
              <Lock size={24} />
            </Box>
            <Typography
              variant="h5"
              sx={{ fontWeight: 700, fontFamily: '"Space Grotesk", sans-serif' }}
            >
              Sign in to BuckyOS
            </Typography>
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
          </Box>

          {error && (
            <Alert
              severity={
                error.kind === 'rate_limited' || error.kind === 'account_locked'
                  ? 'warning'
                  : 'error'
              }
              sx={{ mb: 2.5 }}
              onClose={() => setError(null)}
            >
              {errorMessage(error)}
            </Alert>
          )}

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
                justifyContent: 'space-between',
                mb: 2.5,
              }}
            >
              <FormControlLabel
                control={
                  <Checkbox
                    size="small"
                    checked={rememberMe}
                    onChange={(e) => setRememberMe(e.target.checked)}
                    disabled={disabled}
                  />
                }
                label={
                  <Typography variant="body2" color="text.secondary">
                    Remember me for 30 days
                  </Typography>
                }
              />
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
          </Box>

          <Typography
            variant="caption"
            color="text.secondary"
            sx={{
              display: 'block',
              textAlign: 'center',
              mt: 3,
              lineHeight: 1.6,
            }}
          >
            By signing in, you agree to the BuckyOS terms of service.
          </Typography>
        </Paper>
      </Container>
    </Box>
  )
}

export default LoginPage
