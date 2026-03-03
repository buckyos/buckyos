const ACCOUNT_STORAGE_KEY = 'buckyos.account_info'

type StoredAccountInfo = {
  session_token?: string
}

export const getStoredSessionToken = () => {
  if (typeof window === 'undefined') {
    return null
  }

  const raw = window.localStorage.getItem(ACCOUNT_STORAGE_KEY)
  if (!raw) {
    return null
  }

  try {
    const parsed = JSON.parse(raw) as StoredAccountInfo
    const token = parsed.session_token?.trim()
    return token ? token : null
  } catch {
    return null
  }
}

export const hasStoredSession = () => Boolean(getStoredSessionToken())

const expireCookie = (name: string) => {
  if (typeof document === 'undefined') {
    return
  }
  document.cookie = `${name}=; path=/; expires=${new Date(0).toUTCString()}; SameSite=Lax`
  document.cookie = `${name}=; path=/; max-age=0; SameSite=Lax`
}

export const clearStoredSession = () => {
  if (typeof window !== 'undefined') {
    window.localStorage.removeItem(ACCOUNT_STORAGE_KEY)
  }

  expireCookie('control-panel_token')
  expireCookie('control_panel_token')
}

export const sanitizeRedirectPath = (candidate: string | null | undefined, fallback = '/') => {
  if (!candidate) {
    return fallback
  }

  const normalized = candidate.trim()
  if (!normalized.startsWith('/') || normalized.startsWith('//')) {
    return fallback
  }

  if (normalized === '/login' || normalized.startsWith('/login?')) {
    return fallback
  }

  if (normalized === '/sso/login' || normalized.startsWith('/sso/login?')) {
    return fallback
  }

  return normalized
}
