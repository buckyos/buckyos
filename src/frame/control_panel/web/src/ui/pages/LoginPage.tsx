import { useEffect, useState } from 'react'
import { buckyos } from 'buckyos'
import { useLocation, useNavigate } from 'react-router-dom'

import { hasStoredSession, sanitizeRedirectPath } from '@/auth/session'
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

const LoginPage = () => {
  const location = useLocation()
  const navigate = useNavigate()
  const defaultUsername = useState(resolveDefaultUsernameFromHost)[0]
  const [username, setUsername] = useState('')
  const [usernameEditable, setUsernameEditable] = useState(defaultUsername.length === 0)
  const [password, setPassword] = useState('')
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [initError, setInitError] = useState<string | null>(null)
  const [messageModal, setMessageModal] = useState<LoginModalState | null>(null)
  const redirectTarget = sanitizeRedirectPath(new URLSearchParams(location.search).get('redirect'))

  useEffect(() => {
    document.title = 'BuckyOS | Control Panel Login'

    const init = async () => {
      try {
        await buckyos.initBuckyOS('control-panel')
        if (hasStoredSession()) {
          navigate(redirectTarget, { replace: true })
          return
        }

        if (defaultUsername) {
          setUsername((prev) => prev || defaultUsername)
        }

        setLoading(false)
      } catch (err) {
        console.error('initBuckyOS failed', err)
        setInitError('初始化 BuckyOS 失败，请检查网络或稍后再试。')
        setLoading(false)
      }
    }

    void init()
  }, [defaultUsername, navigate, redirectTarget])

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (loading || submitting) return

    if (!username.trim() || !password) {
      setMessageModal({
        tone: 'error',
        title: '登录失败',
        message: '请输入用户名和密码。',
      })
      return
    }

    setSubmitting(true)

    try {
      const accountInfo = await buckyos.doLogin(username.trim(), password)
      if (!accountInfo) {
        setMessageModal({
          tone: 'error',
          title: '登录失败',
          message: '未获取到会话信息，请重试。',
        })
        return
      }

      setMessageModal({
        tone: 'success',
        title: '登录成功',
        message: '会话已建立，点击确认进入控制台。',
        nextPath: redirectTarget,
      })
    } catch (err) {
      console.error('login failed', err)
      const message = err instanceof Error ? err.message : String(err)
      setMessageModal({
        tone: 'error',
        title: '登录失败',
        message: message || '登录失败，请重试。',
      })
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
          <div className="flex items-start gap-3">
            <div className="flex items-center gap-3">
              <span className="inline-flex size-10 items-center justify-center rounded-2xl bg-[var(--cp-primary)] text-lg font-semibold text-white shadow-lg shadow-emerald-200">
                B
              </span>
              <div className="leading-tight">
                <p className="text-xs font-semibold text-[var(--cp-muted)]">BuckyOS · Control Panel</p>
                <p className="text-lg font-semibold">控制台登录</p>
              </div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap items-center gap-2 text-xs font-semibold text-[var(--cp-muted)]">
            <span className="inline-flex items-center gap-2 rounded-full bg-white px-3 py-1.5 shadow-sm ring-1 ring-[var(--cp-border)]">
              <Icon name="shield" className="size-4 text-[var(--cp-primary)]" />
              App ID: control-panel
            </span>
          </div>

          <p className="mt-3 text-sm leading-relaxed text-[var(--cp-muted)]">登录后将进入控制台首页。第三方应用授权请使用 /sso/login。</p>

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
                    <p className="font-semibold">无法完成登录</p>
                    <p className="leading-relaxed text-amber-800">{initError}</p>
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
                  <div className="relative">
                    <input
                      autoFocus
                      autoComplete="username"
                      className={`${fieldClasses} !bg-slate-200 pr-24 placeholder:text-slate-500 ${
                        !usernameEditable && defaultUsername
                          ? 'text-slate-500'
                          : 'text-slate-600'
                      }`}
                      placeholder="输入管理员用户名"
                      value={username}
                      onChange={(event) => setUsername(event.target.value)}
                      aria-label="用户名"
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
                        {usernameEditable ? 'Use default' : 'Change'}
                      </button>
                    ) : null}
                  </div>
                  {defaultUsername ? (
                    <p className="text-[11px] leading-relaxed text-[var(--cp-muted)]">
                      默认用户名取自当前域名：{defaultUsername}。如需授权子账号，请点击 Change 手动填写。
                    </p>
                  ) : null}
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

                <button
                  type="submit"
                  disabled={disabled}
                  className="mt-1 inline-flex w-full items-center justify-center gap-2 rounded-2xl bg-[var(--cp-primary)] px-4 py-3 text-[15px] font-semibold text-white shadow-lg shadow-emerald-200 transition duration-200 hover:bg-[var(--cp-primary-strong)] disabled:cursor-not-allowed disabled:opacity-60"
                >
                  {submitting ? '正在登录…' : '登录'}
                </button>

                <p className="text-center text-[11px] leading-relaxed text-[var(--cp-muted)]">用于 control-panel 控制台访问。</p>
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
        confirmLabel={messageModal?.tone === 'success' ? '进入控制台' : '知道了'}
        onConfirm={() => {
          if (messageModal?.tone === 'success') {
            navigate(messageModal.nextPath || '/', { replace: true })
            return
          }

          setMessageModal(null)
        }}
      />
    </div>
  )
}

export default LoginPage
