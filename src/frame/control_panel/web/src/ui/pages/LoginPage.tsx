import { useEffect, useState } from 'react'
import { useLocation, useNavigate, type NavigateFunction } from 'react-router-dom'

import { useI18n } from '@/i18n'
import { issueSsoTokenForRedirect } from '@/auth/authManager'
import { useAuth } from '@/auth/useAuth'
import { sanitizeRedirectTarget } from '@/auth/session'
import MessageModal from '@/ui/components/MessageModal'

import Icon from '../icons'

const fieldClasses =
  'w-full rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-3 text-[15px] text-[var(--cp-ink)] shadow-sm focus:border-[var(--cp-primary)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]'

const resolveDefaultUsernameFromHost = () => {
  if (typeof window === 'undefined') {
    return ''
  }

  const rawHost = window.location.hostname.trim().toLowerCase()
  if (!rawHost) {
    return ''
  }

  const isIpv4Host = /^\d{1,3}(?:\.\d{1,3}){3}$/.test(rawHost)
    ? rawHost.split('.').every((segment) => {
        const value = Number.parseInt(segment, 10)
        return Number.isInteger(value) && value >= 0 && value <= 255
      })
    : false
  const isIpv6Host = rawHost.includes(':')
  if (rawHost === 'localhost' || isIpv4Host || isIpv6Host) {
    return ''
  }

  const hostWithoutSys = rawHost.startsWith('sys.') ? rawHost.slice(4) : rawHost
  const [firstLabel] = hostWithoutSys.split('.')
  if (!firstLabel) {
    return ''
  }

  return firstLabel.replace(/[^a-z0-9._-]/g, '')
}

type LoginModalState = {
  tone: 'success' | 'error'
  title: string
  message: string
  nextPath?: string
}

const getReadableLoginError = (rawError: unknown) => {
  const rawMessage = rawError instanceof Error ? rawError.message : String(rawError ?? '')
  const normalized = rawMessage.toLowerCase()

  if (
    normalized.includes('rpc call error: 401') ||
    normalized.includes('rpc call error: 403') ||
    normalized.includes('rpc call error: 500') ||
    normalized.includes('invalid password') ||
    normalized.includes('invalid username') ||
    normalized.includes('wrong password') ||
    normalized.includes('auth failed') ||
    normalized.includes('login failed')
  ) {
    return 'Authentication failed. Please check your username and password and try again.'
  }

  if (
    normalized.includes('failed to fetch') ||
    normalized.includes('network') ||
    normalized.includes('timeout') ||
    normalized.includes('timed out') ||
    normalized.includes('connection refused')
  ) {
    return 'Unable to reach the authentication service. Please check your network and try again.'
  }

  if (normalized.includes('too many') || normalized.includes('rate limit')) {
    return 'Too many sign-in attempts. Please wait a moment and try again.'
  }

  return 'Sign-in failed. Please try again.'
}

const redirectToTarget = (target: string, navigate: NavigateFunction) => {
  if (/^https?:\/\//i.test(target)) {
    window.location.replace(target)
    return
  }

  navigate(target, { replace: true })
}

const LoginPage = () => {
  const { t } = useI18n()
  const location = useLocation()
  const navigate = useNavigate()
  const { status, initError, signInWithPassword } = useAuth()
  const defaultUsername = useState(resolveDefaultUsernameFromHost)[0]
  const [username, setUsername] = useState('')
  const [usernameEditable, setUsernameEditable] = useState(defaultUsername.length === 0)
  const [password, setPassword] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [issuingSso, setIssuingSso] = useState(false)
  const [messageModal, setMessageModal] = useState<LoginModalState | null>(null)
  const searchParams = new URLSearchParams(location.search)
  const redirectTarget = sanitizeRedirectTarget(searchParams.get('redirect_url') ?? searchParams.get('redirect'))
  const needsSsoCookie = /^https?:\/\//i.test(redirectTarget)
  const loading = status === 'loading'

  useEffect(() => {
    document.title = t('login.documentTitle', 'Buckyos Login')
    if (defaultUsername) {
      setUsername((prev) => prev || defaultUsername)
    }
  }, [defaultUsername, t])

  useEffect(() => {
    if (status !== 'authenticated' || submitting || issuingSso || messageModal) {
      return
    }

    if (!needsSsoCookie) {
      redirectToTarget(redirectTarget, navigate)
      return
    }

    let cancelled = false
    setIssuingSso(true)

    void issueSsoTokenForRedirect(redirectTarget)
      .then(() => {
        if (!cancelled) {
          redirectToTarget(redirectTarget, navigate)
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setMessageModal({
            tone: 'error',
            title: t('login.failedTitle', 'Login Failed'),
            message: getReadableLoginError(error),
          })
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIssuingSso(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [issuingSso, messageModal, navigate, needsSsoCookie, redirectTarget, status, submitting, t])

  useEffect(() => {
    if (messageModal?.tone !== 'success') {
      return
    }

    const timer = window.setTimeout(() => {
      redirectToTarget(messageModal.nextPath || '/', navigate)
    }, 1500)

    return () => {
      window.clearTimeout(timer)
    }
  }, [messageModal, navigate])

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (loading || submitting || issuingSso) return

    if (!username.trim() || !password) {
      setMessageModal({
        tone: 'error',
        title: t('login.failedTitle', 'Login Failed'),
        message: t('login.missingCredentials', 'Please enter both username and password.'),
      })
      return
    }

    setSubmitting(true)

    try {
      await signInWithPassword(username.trim(), password, needsSsoCookie ? redirectTarget : null)

      setMessageModal({
        tone: 'success',
        title: t('login.successTitle', 'Login Successful'),
        message: t('login.redirecting', 'Session created. Redirecting...'),
        nextPath: redirectTarget,
      })
    } catch (err) {
      console.error('login failed', err)
      setMessageModal({
        tone: 'error',
        title: t('login.failedTitle', 'Login Failed'),
        message: getReadableLoginError(err),
      })
    } finally {
      setSubmitting(false)
    }
  }

  const disabled = loading || submitting || issuingSso

  return (
    <div className="min-h-screen bg-transparent px-4 py-6 text-[var(--cp-ink)]">
      <div className="mx-auto flex min-h-[520px] max-w-lg flex-col items-center justify-center">
        <div className="relative w-full rounded-3xl bg-white/90 p-6 shadow-[0_24px_80px_-40px_rgba(15,23,42,0.55)] backdrop-blur">
          <div className="absolute inset-0 -z-10 rounded-3xl bg-gradient-to-br from-[var(--cp-primary-soft)] via-white to-[#f6f3ec]" aria-hidden />
          <div className="flex items-start gap-3">
            <div className="flex items-center gap-3">
              <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-lg font-semibold text-white shadow-lg shadow-emerald-200">
                B
              </span>
              <div className="leading-tight">
                <p className="text-lg font-semibold">{t('login.pageTitle', 'BuckyOS Desktop Login')}</p>
              </div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap items-center gap-2 text-xs font-semibold text-[var(--cp-muted)]">
            <span className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1.5 shadow-sm ring-1 ring-[var(--cp-border)]">
              <Icon name="shield" className="size-4 text-[var(--cp-primary)]" />
              {t('login.appId', 'App ID: control-panel')}
            </span>
          </div>

          <div className="mt-6 space-y-6">
            {loading ? (
              <div className="space-y-3">
                <div className="h-4 w-32 animate-pulse rounded-full bg-[var(--cp-border)]/80" />
                <div className="h-11 w-full animate-pulse rounded-2xl bg-[var(--cp-border)]/60" />
                <div className="h-11 w-full animate-pulse rounded-2xl bg-[var(--cp-border)]/50" />
              </div>
            ) : initError ? (
              <div className="space-y-4">
                <div className="flex items-start gap-3 rounded-2xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900">
                  <Icon name="alert" className="mt-0.5 size-5" />
                  <div>
                    <p className="font-semibold">{t('login.unableTitle', 'Unable to Sign In')}</p>
                    <p className="leading-relaxed text-amber-800">{initError}</p>
                  </div>
                </div>
                <div className="flex gap-3 text-sm">
                  <button
                    type="button"
                    className="flex-1 rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-3 font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary)]"
                    onClick={() => window.location.reload()}
                  >
                    {t('login.retry', 'Retry')}
                  </button>
                  <button
                    type="button"
                    className="flex-1 rounded-2xl bg-[var(--cp-primary)] px-4 py-3 font-semibold text-white shadow-lg shadow-emerald-200 transition hover:bg-[var(--cp-primary-strong)]"
                    onClick={() => window.close()}
                  >
                    {t('login.closeWindow', 'Close Window')}
                  </button>
                </div>
              </div>
            ) : (
              <form className="space-y-4" onSubmit={handleSubmit}>
                <div className="space-y-1">
                  <label className="block text-sm font-semibold text-[var(--cp-muted)]">{t('login.username', 'Username')}</label>
                  <div className="relative">
                    <input
                      autoFocus
                      autoComplete="username"
                      className={`${fieldClasses} pr-24 placeholder:text-slate-500 ${
                        !usernameEditable && defaultUsername
                          ? '!bg-slate-200 text-slate-500'
                          : 'bg-white text-[var(--cp-ink)]'
                      }`}
                      placeholder={t('login.usernamePlaceholder', 'Enter username')}
                      value={username}
                      onChange={(event) => setUsername(event.target.value)}
                      aria-label={t('login.username', 'Username')}
                      readOnly={!usernameEditable && Boolean(defaultUsername)}
                      required
                    />
                    {defaultUsername ? (
                      <button
                        type="button"
                        className="absolute right-2 top-1/2 -translate-y-1/2 rounded-lg border border-[var(--cp-border)] bg-white px-2 py-1 text-[11px] font-semibold text-[var(--cp-muted)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary)]"
                        onClick={() => {
                          if (usernameEditable) {
                            setUsername(defaultUsername)
                            setUsernameEditable(false)
                          } else {
                            setUsernameEditable(true)
                          }
                        }}
                      >
                        {usernameEditable ? t('login.useDefault', 'Use default') : t('login.change', 'Change')}
                      </button>
                    ) : null}
                  </div>
                  {defaultUsername ? (
                    <p className="text-[11px] leading-relaxed text-[var(--cp-muted)]">
                       {t('login.defaultUsernameHint', 'Default username comes from current domain: {value}. Click Change to enter a delegated sub-account.', { value: defaultUsername })}
                    </p>
                  ) : null}
                </div>

                <div className="space-y-1">
                  <label className="block text-sm font-semibold text-[var(--cp-muted)]">{t('login.password', 'Password')}</label>
                  <input
                    type="password"
                    autoComplete="current-password"
                    className={fieldClasses}
                    placeholder={t('login.passwordPlaceholder', 'Enter password')}
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                    aria-label={t('login.password', 'Password')}
                    required
                  />
                </div>

                <button
                  type="submit"
                  disabled={disabled}
                  className="mt-1 inline-flex w-full items-center justify-center gap-2 rounded-2xl bg-[var(--cp-primary)] px-4 py-3 text-[15px] font-semibold text-white shadow-lg shadow-emerald-200 transition duration-200 hover:bg-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
                >
                  {submitting ? t('login.signingIn', 'Signing in...') : t('login.signIn', 'Sign in')}
                </button>
              </form>
            )}
          </div>
        </div>
      </div>

      <MessageModal
        open={Boolean(messageModal)}
        tone={messageModal?.tone ?? 'success'}
        title={messageModal?.title ?? ''}
        message={messageModal?.message ?? ''}
        showConfirm={messageModal?.tone === 'error'}
        confirmLabel={t('login.ok', 'OK')}
        onConfirm={() => {
          setMessageModal(null)
        }}
      />
    </div>
  )
}

export default LoginPage
