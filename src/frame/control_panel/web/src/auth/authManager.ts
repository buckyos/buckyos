import { buckyos } from 'buckyos'
import {
  CONTROL_PANEL_MOCK_REFRESH_TOKEN,
  CONTROL_PANEL_MOCK_SESSION_TOKEN,
  CONTROL_PANEL_MOCK_USER_ID,
  CONTROL_PANEL_MOCK_USER_TYPE,
  CONTROL_PANEL_MOCK_USERNAME,
  isMockRuntime,
} from '@/config/runtime'

import {
  clearStoredSession,
  getStoredAccountInfo,
  getStoredRefreshToken,
  saveSsoSessionCookie,
  saveStoredAccountInfo,
  type StoredAccountInfo,
} from './session'

const APP_ID = 'control-panel'
const VERIFY_CACHE_TTL_MS = 30_000
const authRpcClient = new buckyos.kRPCClient('/kapi/control-panel')

let runtimeReady = false
let runtimeInitPromise: Promise<void> | null = null
let refreshInFlight: Promise<StoredAccountInfo | null> | null = null

let cachedVerifyToken: string | null = null
let cachedVerifyResult = false
let cachedVerifyAt = 0

type NormalizedLoginResponse = {
  accountInfo: StoredAccountInfo
  ssoToken?: string
}

const normalizeLoginResponse = (response: unknown, fallbackUsername: string): NormalizedLoginResponse => {
  const payload = response as {
    user_info?: { show_name?: string; user_id?: string; user_type?: string }
    user_name?: string
    user_id?: string
    user_type?: string
    session_token?: string
    refresh_token?: string
    sso_token?: string
  }

  const ssoToken = payload.sso_token?.trim() || undefined
  if (payload.user_info) {
    return {
      accountInfo: {
        user_name: payload.user_info.show_name || fallbackUsername,
        user_id: payload.user_info.user_id,
        user_type: payload.user_info.user_type,
        session_token: payload.session_token,
        refresh_token: payload.refresh_token,
      },
      ssoToken,
    }
  }

  return {
    accountInfo: {
      user_name: payload.user_name || fallbackUsername,
      user_id: payload.user_id,
      user_type: payload.user_type,
      session_token: payload.session_token,
      refresh_token: payload.refresh_token,
    },
    ssoToken,
  }
}

const resetVerifyCache = () => {
  cachedVerifyToken = null
  cachedVerifyResult = false
  cachedVerifyAt = 0
}

const hasValidTokenPair = (accountInfo: StoredAccountInfo | null) =>
  Boolean(accountInfo?.session_token && accountInfo.session_token.trim())

const callAuthRpc = async <T>(method: string, params: Record<string, unknown>) =>
  authRpcClient.call<T, Record<string, unknown>>(method, params)

const seedMockSession = (username = CONTROL_PANEL_MOCK_USERNAME) => {
  saveStoredAccountInfo({
    user_name: username,
    user_id: CONTROL_PANEL_MOCK_USER_ID,
    user_type: CONTROL_PANEL_MOCK_USER_TYPE,
    session_token: CONTROL_PANEL_MOCK_SESSION_TOKEN,
    refresh_token: CONTROL_PANEL_MOCK_REFRESH_TOKEN,
  })
  saveSsoSessionCookie(CONTROL_PANEL_MOCK_SESSION_TOKEN)
  resetVerifyCache()
}

export const ensureAuthRuntime = async () => {
  if (isMockRuntime()) {
    seedMockSession()
    return
  }

  if (runtimeReady) {
    return
  }

  if (!runtimeInitPromise) {
    runtimeInitPromise = buckyos
      .initBuckyOS(APP_ID)
      .then(() => {
        runtimeReady = true
      })
      .catch((error) => {
        runtimeInitPromise = null
        throw error
      })
  }

  await runtimeInitPromise
}

const verifySessionToken = async (sessionToken: string) => {
  const normalizedToken = sessionToken.trim()
  if (!normalizedToken) {
    return false
  }

  const now = Date.now()
  if (cachedVerifyToken === normalizedToken && now - cachedVerifyAt <= VERIFY_CACHE_TTL_MS) {
    return cachedVerifyResult
  }

  try {
    const ok = await callAuthRpc<boolean>('auth.verify', {
      session_token: normalizedToken,
      appid: APP_ID,
    })

    cachedVerifyToken = normalizedToken
    cachedVerifyResult = Boolean(ok)
    cachedVerifyAt = now
    return cachedVerifyResult
  } catch {
    cachedVerifyToken = normalizedToken
    cachedVerifyResult = false
    cachedVerifyAt = now
    return false
  }
}

const refreshSessionWithToken = async (refreshToken: string, base: StoredAccountInfo | null) => {
  const nextTokens = await callAuthRpc<{ session_token?: string; refresh_token?: string }>('auth.refresh', {
    refresh_token: refreshToken,
  })

  const sessionToken = nextTokens?.session_token?.trim()
  if (!sessionToken) {
    throw new Error('refresh_token did not return a session token')
  }

  const updated: StoredAccountInfo = {
    ...(base || {}),
    session_token: sessionToken,
    refresh_token: nextTokens.refresh_token?.trim() || refreshToken,
  }

  saveStoredAccountInfo(updated)
  saveSsoSessionCookie(sessionToken)
  resetVerifyCache()
  return updated
}

const refreshSession = async () => {
  if (refreshInFlight) {
    return refreshInFlight
  }

  const refreshToken = getStoredRefreshToken()
  const base = getStoredAccountInfo()
  if (!refreshToken) {
    return null
  }

  refreshInFlight = refreshSessionWithToken(refreshToken, base)
    .catch(() => {
      clearStoredSession()
      resetVerifyCache()
      return null
    })
    .finally(() => {
      refreshInFlight = null
    })

  return refreshInFlight
}

type EnsureSessionOptions = {
  forceRefresh?: boolean
}

export const ensureSessionToken = async (options: EnsureSessionOptions = {}) => {
  await ensureAuthRuntime()

  if (isMockRuntime()) {
    seedMockSession()
    return CONTROL_PANEL_MOCK_SESSION_TOKEN
  }

  const forceRefresh = options.forceRefresh === true
  const stored = getStoredAccountInfo()
  if (!hasValidTokenPair(stored)) {
    return null
  }

  const sessionToken = stored?.session_token?.trim() || ''

  if (!forceRefresh) {
    const verified = await verifySessionToken(sessionToken)
    if (verified) {
      saveSsoSessionCookie(sessionToken)
      return sessionToken
    }
  }

  const refreshed = await refreshSession()
  if (!hasValidTokenPair(refreshed)) {
    return null
  }

  return refreshed?.session_token?.trim() || null
}

export const loginWithPassword = async (username: string, password: string, redirectUrl?: string | null) => {
  if (isMockRuntime()) {
    void password
    void redirectUrl
    seedMockSession(username.trim() || CONTROL_PANEL_MOCK_USERNAME)
    return getStoredAccountInfo() ?? {
      user_name: username.trim() || CONTROL_PANEL_MOCK_USERNAME,
      user_id: CONTROL_PANEL_MOCK_USER_ID,
      user_type: CONTROL_PANEL_MOCK_USER_TYPE,
      session_token: CONTROL_PANEL_MOCK_SESSION_TOKEN,
      refresh_token: CONTROL_PANEL_MOCK_REFRESH_TOKEN,
    }
  }

  await ensureAuthRuntime()

  const trimmedUsername = username.trim()
  const nonce = Date.now()
  const passwordHash = buckyos.hashPassword(trimmedUsername, password, nonce)
  authRpcClient.setSeq(nonce)
  const normalizedRedirectUrl =
    typeof redirectUrl === 'string' && /^https?:\/\//i.test(redirectUrl.trim()) ? redirectUrl.trim() : undefined

  const response = await callAuthRpc<unknown>('auth.login', {
    username: trimmedUsername,
    password: passwordHash,
    appid: APP_ID,
    source_url: window.location.href,
    login_nonce: nonce,
    ...(normalizedRedirectUrl ? { redirect_url: normalizedRedirectUrl } : {}),
  })

  const normalized = normalizeLoginResponse(response, trimmedUsername)
  const sessionToken = normalized.accountInfo.session_token?.trim()
  if (!sessionToken) {
    throw new Error('verify-hub did not return a session token')
  }
  if (normalizedRedirectUrl) {
    if (!normalized.ssoToken) {
      throw new Error('auth.login did not return sso token')
    }
    saveSsoSessionCookie(normalized.ssoToken)
  }

  saveStoredAccountInfo({
    ...normalized.accountInfo,
    session_token: sessionToken,
    refresh_token: normalized.accountInfo.refresh_token?.trim() || undefined,
  })
  saveSsoSessionCookie(sessionToken)
  resetVerifyCache()

  return normalized.accountInfo
}

export const issueSsoTokenForRedirect = async (redirectUrl: string) => {
  if (isMockRuntime()) {
    void redirectUrl
    seedMockSession()
    return CONTROL_PANEL_MOCK_SESSION_TOKEN
  }

  await ensureAuthRuntime()

  const normalizedRedirectUrl = redirectUrl.trim()
  if (!/^https?:\/\//i.test(normalizedRedirectUrl)) {
    throw new Error('redirect_url must be an absolute http(s) url')
  }

  const sessionToken = await ensureSessionToken()
  if (!sessionToken) {
    throw new Error('missing authenticated control-panel session')
  }

  const response = await callAuthRpc<{ sso_token?: string }>('auth.issue_sso_token', {
    session_token: sessionToken,
    redirect_url: normalizedRedirectUrl,
  })
  const ssoToken = response?.sso_token?.trim()
  if (!ssoToken) {
    throw new Error('auth.issue_sso_token did not return sso token')
  }

  saveSsoSessionCookie(ssoToken)
  return ssoToken
}

export const signOutSession = async () => {
  if (isMockRuntime()) {
    clearStoredSession()
    resetVerifyCache()
    return
  }

  const refreshToken = getStoredRefreshToken()

  try {
    await callAuthRpc('auth.logout', refreshToken ? { refresh_token: refreshToken } : {})
  } catch {
    // ignore logout RPC failures and continue local cleanup
  }

  clearStoredSession()
  resetVerifyCache()
}

export const getStoredUsername = () => getStoredAccountInfo()?.user_name || null
