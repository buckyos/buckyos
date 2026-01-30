import { useEffect, useMemo, useState } from 'react'
import { buckyos } from 'buckyos'

import Icon from '../icons'

const fieldClasses =
  'w-full rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-3 text-[15px] text-[var(--cp-ink)] shadow-sm focus:border-[var(--cp-primary)] focus:outline-none focus:ring-2 focus:ring-[var(--cp-primary-soft)]'

const LoginPage = () => {
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
    document.title = 'BuckyOS | SSO Login'
    const params = new URLSearchParams(window.location.search)
    const id = params.get('client_id')

    if (!id) {
      setError('缺少 client_id 参数，无法完成登录。')
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
        setError('初始化 BuckyOS 失败，请检查网络或稍后再试。')
        setLoading(false)
      }
    }

    init()
  }, [])

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (loading || submitting) return

    if (!clientId) {
      setError('缺少 client_id，无法发起认证。')
      return
    }

    if (!username.trim() || !password) {
      setError('请输入用户名和密码。')
      return
    }

    setError(null)
    setHint(null)
    setSubmitting(true)

    try {
      const accountInfo = await buckyos.doLogin(username.trim(), password)
      const payload = JSON.stringify(accountInfo)

      if (window.opener) {
        window.opener.postMessage({ token: payload, client_id: clientId }, '*')
        setHint('登录成功，正在返回应用…')
        window.close()
      } else {
        setHint('登录成功，但未检测到调用方窗口。已为你复制 token，可手动返回应用粘贴。')
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
      setError(message || '登录失败，请重试。')
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
                <p className="text-xs font-semibold text-[var(--cp-muted)]">BuckyOS · SSO 弹窗</p>
                <p className="text-lg font-semibold">安全登录</p>
              </div>
            </div>
            <button
              type="button"
              className="rounded-full p-2 text-[var(--cp-muted)] transition hover:bg-[var(--cp-surface-muted)] hover:text-[var(--cp-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--cp-primary)]"
              onClick={() => window.close()}
              aria-label="关闭弹窗"
            >
              <Icon name="signout" className="size-5" />
            </button>
          </div>

          <div className="mt-4 flex flex-wrap items-center gap-2 text-xs font-semibold text-[var(--cp-muted)]">
            <span className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1.5 shadow-sm ring-1 ring-[var(--cp-border)]">
              <Icon name="shield" className="size-4 text-[var(--cp-primary)]" />
              {clientId ? `App ID: ${clientId}` : '等待 client_id…'}
            </span>
            {sourceHost ? (
              <span className="inline-flex items-center gap-2 rounded-full bg-[var(--cp-surface-muted)] px-3 py-1.5 ring-1 ring-[var(--cp-border)]">
                <Icon name="link" className="size-3.5" />
                来源 {sourceHost}
              </span>
            ) : null}
          </div>

          <p className="mt-3 text-sm leading-relaxed text-[var(--cp-muted)]">
            此页面仅用于生成并返回 session token。请确认来源和 App ID 后再登录。
          </p>

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
                    <p className="font-semibold">无法完成登录</p>
                    <p className="leading-relaxed text-amber-800">{error}</p>
                  </div>
                </div>
                <div className="flex gap-3 text-sm">
                  <button
                    type="button"
                    className="flex-1 rounded-2xl border border-[var(--cp-border)] bg-white px-4 py-3 font-semibold text-[var(--cp-ink)] transition hover:border-[var(--cp-primary)] hover:text-[var(--cp-primary)]"
                    onClick={() => window.location.reload()}
                  >
                    重试
                  </button>
                  <button
                    type="button"
                    className="flex-1 rounded-2xl bg-[var(--cp-primary)] px-4 py-3 font-semibold text-white shadow-lg shadow-emerald-200 transition hover:bg-[var(--cp-primary-strong)]"
                    onClick={() => window.close()}
                  >
                    关闭窗口
                  </button>
                </div>
              </div>
            ) : (
              <form className="space-y-4" onSubmit={handleSubmit}>
                <div className="space-y-1">
                  <label className="block text-sm font-semibold text-[var(--cp-muted)]">用户名</label>
                  <input
                    autoFocus
                    autoComplete="username"
                    className={fieldClasses}
                    placeholder="输入管理员用户名"
                    value={username}
                    onChange={(event) => setUsername(event.target.value)}
                    aria-label="用户名"
                    required
                  />
                </div>

                <div className="space-y-1">
                  <label className="block text-sm font-semibold text-[var(--cp-muted)]">密码</label>
                  <input
                    type="password"
                    autoComplete="current-password"
                    className={fieldClasses}
                    placeholder="输入密码"
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                    aria-label="密码"
                    required
                  />
                </div>

                {hint ? (
                  <div className="flex items-start gap-2 rounded-2xl bg-[var(--cp-surface-muted)] px-4 py-3 text-sm text-[var(--cp-muted)]">
                    <Icon name="spark" className="mt-0.5 size-4 text-[var(--cp-primary)]" />
                    <div className="leading-relaxed">
                      <p className="font-semibold text-[var(--cp-ink)]">{hint}</p>
                      {tokenPreview ? (
                        <p className="break-all text-xs text-[var(--cp-muted)]">{tokenPreview}</p>
                      ) : null}
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
                  {submitting ? '正在登录…' : '登录并返回'}
                </button>

                <p className="text-center text-[11px] leading-relaxed text-[var(--cp-muted)]">
                  登录即表示同意 BuckyOS 在本设备上生成会话令牌并返回给请求的应用。
                </p>
              </form>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

export default LoginPage
