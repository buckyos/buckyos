import { useEffect, useMemo, useState } from 'react'
import { buckyos } from 'buckyos'

import { useI18n } from '@/i18n'
import { saveSsoSessionCookie } from '@/auth/session'

import Icon from '../icons'

type SsoAccountInfo = {
  user_name: string
  user_id?: string
  user_type?: string
  session_token: string
  refresh_token?: string
}

const normalizeSsoLoginResponse = (response: unknown, fallbackUsername: string): SsoAccountInfo => {
  const payload = response as {
    user_info?: { show_name?: string; user_id?: string; user_type?: string }
    user_name?: string
    user_id?: string
    user_type?: string
    session_token?: string
    refresh_token?: string
  }

  const sessionToken = payload.session_token?.trim()
  if (!sessionToken) {
    throw new Error('login response missing session token')
  }

  if (payload.user_info) {
    return {
      user_name: payload.user_info.show_name || fallbackUsername,
      user_id: payload.user_info.user_id,
      user_type: payload.user_info.user_type,
      session_token: sessionToken,
      refresh_token: payload.refresh_token?.trim() || undefined,
    }
  }

  return {
    user_name: payload.user_name || fallbackUsername,
    user_id: payload.user_id,
    user_type: payload.user_type,
    session_token: sessionToken,
    refresh_token: payload.refresh_token?.trim() || undefined,
  }
}

const fieldClasses =
  'w-full rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-3 text-[15px] text-[var(--cp-ink)] shadow-sm focus:border-[var(--cp-primary)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]'

const SsoLoginPage = () => {
  const { t } = useI18n()
  const [clientId, setClientId] = useState<string | null>(null)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [hint, setHint] = useState<string | null>(null)
  const [tokenPreview, setTokenPreview] = useState<string | null>(null)

  const sourceUrl = useMemo(() => document.referrer || '', [])
  const sourceHost = useMemo(() => {
    try {
      return sourceUrl ? new URL(sourceUrl).hostname : ''
    } catch {
      return ''
    }
  }, [sourceUrl])

  useEffect(() => {
    document.title = t('sso.documentTitle', 'BuckyOS | SSO Login')
    const params = new URLSearchParams(window.location.search)
    const id = params.get('client_id')

    if (!id) {
      setError(t('sso.missingClientId', 'Missing client_id parameter. Unable to continue sign-in.'))
      setLoading(false)
      return
    }

    setClientId(id)

    const init = async () => {
      try {
        await buckyos.initBuckyOS(id)
        setLoading(false)
      } catch (err) {
        console.error('initBuckyOS failed', err)
        setError(t('sso.initFailed', 'Failed to initialize BuckyOS. Please check your network or try again later.'))
        setLoading(false)
      }
    }

    void init()
  }, [t])

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (loading || submitting) return

    if (!clientId) {
      setError(t('sso.missingClientIdStart', 'Missing client_id. Unable to start authentication.'))
      return
    }

    if (!username.trim() || !password) {
      setError(t('sso.missingCredentials', 'Please enter username and password.'))
      return
    }

    setError(null)
    setHint(null)
    setSubmitting(true)

    try {
      const trimmedUsername = username.trim()
      const nonce = Date.now()
      const passwordHash = buckyos.hashPassword(trimmedUsername, password, nonce)
      const authRpcClient = new buckyos.kRPCClient('/kapi/control-panel')
      authRpcClient.setSeq(nonce)

      const response = await authRpcClient.call<unknown, Record<string, unknown>>('auth.login', {
        username: trimmedUsername,
        password: passwordHash,
        appid: clientId,
        source_url: window.location.href,
        login_nonce: nonce,
      })
      const accountInfo = normalizeSsoLoginResponse(response, trimmedUsername)
      const payload = JSON.stringify(accountInfo)

      saveSsoSessionCookie(accountInfo.session_token)

      if (window.opener) {
        window.opener.postMessage({ token: payload, client_id: clientId }, '*')
        setHint(t('sso.successReturning', 'Sign-in successful. Returning to the app...'))
        window.close()
      } else {
        setHint(t('sso.successCopied', 'Sign-in successful, but no caller window was detected. The token has been copied so you can paste it back manually.'))
        setTokenPreview(payload)
        try {
          await navigator.clipboard.writeText(payload)
        } catch (copyError) {
          console.warn('clipboard copy failed', copyError)
        }
      }
    } catch (err) {
      console.error('login failed', err)
      const message = err instanceof Error ? err.message : String(err)
      setError(message || t('sso.failedRetry', 'Sign-in failed. Please try again.'))
    } finally {
      setSubmitting(false)
    }
  }

  const disabled = loading || submitting

  return (
    <div className="min-h-screen bg-transparent px-4 py-6 text-[var(--cp-ink)]">
      <div className="mx-auto flex min-h-[520px] max-w-lg flex-col items-center justify-center">
        <div className="relative w-full rounded-3xl bg-white/90 p-6 shadow-[0_24px_80px_-40px_rgba(15,23,42,0.55)] backdrop-blur">
          <div className="absolute inset-0 -z-10 rounded-3xl bg-gradient-to-br from-[var(--cp-primary-soft)] via-white to-[#f6f3ec]" aria-hidden />
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-center gap-3">
              <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-lg font-semibold text-white shadow-lg shadow-emerald-200">
                B
              </span>
              <div className="leading-tight">
                <p className="text-xs font-semibold text-[var(--cp-muted)]">{t('sso.badge', 'BuckyOS · SSO Popup')}</p>
                <p className="text-lg font-semibold">{t('sso.pageTitle', 'Secure Sign-In')}</p>
              </div>
            </div>
            <button
              type="button"
              className="rounded-full p-2 text-[var(--cp-muted)] transition hover:bg-[var(--cp-surface-muted)] hover:text-[var(--cp-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--cp-primary)]"
              onClick={() => window.close()}
              aria-label={t('sso.closePopup', 'Close popup')}
            >
              <Icon name="signout" className="size-5" />
            </button>
          </div>

          <div className="mt-4 flex flex-wrap items-center gap-2 text-xs font-semibold text-[var(--cp-muted)]">
            <span className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1.5 shadow-sm ring-1 ring-[var(--cp-border)]">
              <Icon name="shield" className="size-4 text-[var(--cp-primary)]" />
              {clientId ? t('sso.appId', 'App ID: {value}', { value: clientId }) : t('sso.waitingClientId', 'Waiting for client_id...')}
            </span>
            {sourceHost ? (
              <span className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-surface-muted)] px-3 py-1.5 ring-1 ring-[var(--cp-border)]">
                <Icon name="link" className="size-3.5" />
                {t('sso.sourceHost', 'Source {value}', { value: sourceHost })}
              </span>
            ) : null}
          </div>

          <p className="mt-3 text-sm leading-relaxed text-[var(--cp-muted)]">{t('sso.description', 'This page is only used to generate and return a session token. Verify the source and App ID before signing in.')}</p>

          <div className="mt-6 space-y-6">
            {loading ? (
              <div className="space-y-3">
                <div className="h-4 w-32 animate-pulse rounded-full bg-[var(--cp-border)]/80" />
                <div className="h-11 w-full animate-pulse rounded-2xl bg-[var(--cp-border)]/60" />
                <div className="h-11 w-full animate-pulse rounded-2xl bg-[var(--cp-border)]/50" />
              </div>
            ) : error ? (
              <div className="space-y-4">
                <div className="flex items-start gap-3 rounded-2xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900">
                  <Icon name="alert" className="mt-0.5 size-5" />
                  <div>
                      <p className="font-semibold">{t('sso.unableTitle', 'Unable to complete sign-in')}</p>
                    <p className="leading-relaxed text-amber-800">{error}</p>
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
                  <input
                    autoFocus
                    autoComplete="username"
                    className={fieldClasses}
                    placeholder={t('sso.usernamePlaceholder', 'Enter admin username')}
                    value={username}
                    onChange={(event) => setUsername(event.target.value)}
                    aria-label={t('login.username', 'Username')}
                    required
                  />
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

                {hint ? (
                  <div className="flex items-start gap-2 rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-muted)]">
                    <Icon name="spark" className="mt-0.5 size-4 text-[var(--cp-primary)]" />
                    <div className="leading-relaxed">
                      <p className="font-semibold text-[var(--cp-ink)]">{hint}</p>
                      {tokenPreview ? <p className="break-all text-xs text-[var(--cp-muted)]">{tokenPreview}</p> : null}
                    </div>
                  </div>
                ) : null}

                {error ? (
                  <div className="flex items-start gap-2 rounded-2xl bg-red-50 px-4 py-3 text-sm text-red-800">
                    <Icon name="alert" className="mt-0.5 size-4" />
                    <p className="leading-relaxed">{error}</p>
                  </div>
                ) : null}

                <button
                  type="submit"
                  disabled={disabled}
                  className="mt-1 inline-flex w-full items-center justify-center gap-2 rounded-2xl bg-[var(--cp-primary)] px-4 py-3 text-[15px] font-semibold text-white shadow-lg shadow-emerald-200 transition duration-200 hover:bg-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
                >
                  {submitting ? t('sso.signingIn', 'Signing in...') : t('sso.signInReturn', 'Sign in and return')}
                </button>

                <p className="text-center text-[11px] leading-relaxed text-[var(--cp-muted)]">{t('sso.consent', 'By signing in, you allow BuckyOS to generate a session token on this device and return it to the requesting app.')}</p>
              </form>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

export default SsoLoginPage
