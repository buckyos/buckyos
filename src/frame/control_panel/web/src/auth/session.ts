const ACCOUNT_STORAGE_KEY = 'buckyos.account_info'
const SESSION_COOKIE_NAMES = ['control-panel_token', 'control_panel_token', 'auth']

export type StoredAccountInfo = {
  user_name?: string
  user_id?: string
  user_type?: string
  session_token?: string
  refresh_token?: string
}

export const getStoredAccountInfo = (): StoredAccountInfo | null => {
  if (typeof window === 'undefined') {
    return null
  }

  const raw = window.localStorage.getItem(ACCOUNT_STORAGE_KEY)
  if (!raw) {
    return null
  }

  try {
    const parsed = JSON.parse(raw) as StoredAccountInfo
    return parsed && typeof parsed === 'object' ? parsed : null
  } catch {
    return null
  }
}

export const saveStoredAccountInfo = (accountInfo: StoredAccountInfo) => {
  if (typeof window === 'undefined') {
    return
  }

  window.localStorage.setItem(ACCOUNT_STORAGE_KEY, JSON.stringify(accountInfo))
}

export const getStoredSessionToken = () => {
  const parsed = getStoredAccountInfo()
  const token = parsed?.session_token?.trim()
  return token ? token : null
}

export const getStoredRefreshToken = () => {
  const parsed = getStoredAccountInfo()
  const token = parsed?.refresh_token?.trim()
  return token ? token : null
}

export const getSessionTokenFromCookies = () => {
  if (typeof document === 'undefined') {
    return null
  }

  const rawCookies = document.cookie
  if (!rawCookies) {
    return null
  }

  for (const pair of rawCookies.split(';')) {
    const [rawKey, ...valueParts] = pair.split('=')
    if (!rawKey || valueParts.length === 0) {
      continue
    }

    const key = rawKey.trim()
    if (!SESSION_COOKIE_NAMES.includes(key)) {
      continue
    }

    const value = valueParts.join('=').trim()
    if (!value) {
      continue
    }

    try {
      return decodeURIComponent(value)
    } catch {
      return value
    }
  }

  return null
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
