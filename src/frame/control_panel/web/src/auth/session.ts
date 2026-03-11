const ACCOUNT_STORAGE_KEY = 'buckyos.control_panel.account_info'
const LEGACY_ACCOUNT_STORAGE_KEY = 'buckyos.account_info'
const SESSION_COOKIE_NAMES = ['control-panel_token', 'control_panel_token', 'auth']
const SSO_SESSION_COOKIE_NAME = 'buckyos_session_token'

export type StoredAccountInfo = {
  user_name?: string
  user_id?: string
  user_type?: string
  session_token?: string
  refresh_token?: string
}

const parseAccountInfo = (raw: string | null): StoredAccountInfo | null => {
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

const hasRefreshToken = (accountInfo: StoredAccountInfo | null) =>
  Boolean(accountInfo?.refresh_token && accountInfo.refresh_token.trim())

export const getStoredAccountInfo = (): StoredAccountInfo | null => {
  if (typeof window === 'undefined') {
    return null
  }

  const scoped = parseAccountInfo(window.localStorage.getItem(ACCOUNT_STORAGE_KEY))
  if (scoped) {
    return scoped
  }

  const legacy = parseAccountInfo(window.localStorage.getItem(LEGACY_ACCOUNT_STORAGE_KEY))
  if (!hasRefreshToken(legacy)) {
    return null
  }

  window.localStorage.setItem(ACCOUNT_STORAGE_KEY, JSON.stringify(legacy))
  return legacy
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

const resolveCookieDomain = () => {
  if (typeof window === 'undefined') {
    return null
  }

  const hostname = window.location.hostname.trim().toLowerCase()
  if (!hostname) {
    return null
  }

  const isIpv4Host = /^\d{1,3}(?:\.\d{1,3}){3}$/.test(hostname)
  const isIpv6Host = hostname.includes(':')
  if (hostname === 'localhost' || isIpv4Host || isIpv6Host) {
    return null
  }

  if (hostname.startsWith('sys.')) {
    return hostname.slice(4) || null
  }

  return hostname
}

export const saveSsoSessionCookie = (sessionToken: string) => {
  if (typeof document === 'undefined') {
    return
  }

  const normalized = sessionToken.trim()
  if (!normalized) {
    return
  }

  const parts = [`${SSO_SESSION_COOKIE_NAME}=${encodeURIComponent(normalized)}`, 'path=/', 'SameSite=Lax']
  const domain = resolveCookieDomain()
  if (domain) {
    parts.push(`Domain=${domain}`)
  }
  if (typeof window !== 'undefined' && window.location.protocol === 'https:') {
    parts.push('Secure')
  }

  document.cookie = parts.join('; ')
}

const expireCookie = (name: string) => {
  if (typeof document === 'undefined') {
    return
  }

  const baseParts = ['path=/', 'SameSite=Lax']
  const domain = resolveCookieDomain()
  const secure = typeof window !== 'undefined' && window.location.protocol === 'https:' ? '; Secure' : ''

  document.cookie = `${name}=; ${baseParts.join('; ')}; expires=${new Date(0).toUTCString()}${domain ? `; Domain=${domain}` : ''}${secure}`
  document.cookie = `${name}=; ${baseParts.join('; ')}; max-age=0${domain ? `; Domain=${domain}` : ''}${secure}`
}

export const clearStoredSession = () => {
  if (typeof window !== 'undefined') {
    window.localStorage.removeItem(ACCOUNT_STORAGE_KEY)
  }

  expireCookie('control-panel_token')
  expireCookie('control_panel_token')
  expireCookie(SSO_SESSION_COOKIE_NAME)
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
