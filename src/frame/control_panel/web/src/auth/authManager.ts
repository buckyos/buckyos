import { buckyos } from 'buckyos'

import {
  clearStoredSession,
  getStoredAccountInfo,
  getStoredRefreshToken,
  saveStoredAccountInfo,
  type StoredAccountInfo,
} from './session'

const APP_ID = 'control-panel'
const VERIFY_CACHE_TTL_MS = 30_000

let runtimeReady = false
let runtimeInitPromise: Promise<void> | null = null
let refreshInFlight: Promise<StoredAccountInfo | null> | null = null

let cachedVerifyToken: string | null = null
let cachedVerifyResult = false
let cachedVerifyAt = 0

const normalizeLoginResponse = (response: unknown, fallbackUsername: string): StoredAccountInfo => {
  const payload = response as {
    user_info?: { show_name?: string; user_id?: string; user_type?: string }
    user_name?: string
    user_id?: string
    user_type?: string
    session_token?: string
    refresh_token?: string
  }

  if (payload.user_info) {
    return {
      user_name: payload.user_info.show_name || fallbackUsername,
      user_id: payload.user_info.user_id,
      user_type: payload.user_info.user_type,
      session_token: payload.session_token,
      refresh_token: payload.refresh_token,
    }
  }

  return {
    user_name: payload.user_name || fallbackUsername,
    user_id: payload.user_id,
    user_type: payload.user_type,
    session_token: payload.session_token,
    refresh_token: payload.refresh_token,
  }
}

const resetVerifyCache = () => {
  cachedVerifyToken = null
  cachedVerifyResult = false
  cachedVerifyAt = 0
}

const hasValidTokenPair = (accountInfo: StoredAccountInfo | null) =>
  Boolean(accountInfo?.session_token && accountInfo.session_token.trim())

export const ensureAuthRuntime = async () => {
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
    const verifyHubClient = buckyos.getVerifyHubClient()
    const ok = await verifyHubClient.verifyToken({
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
  const verifyHubClient = buckyos.getVerifyHubClient()
  const nextTokens = await verifyHubClient.refreshToken({
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

  const forceRefresh = options.forceRefresh === true
  const stored = getStoredAccountInfo()
  if (!hasValidTokenPair(stored)) {
    return null
  }

  const sessionToken = stored?.session_token?.trim() || ''

  if (!forceRefresh) {
    const verified = await verifySessionToken(sessionToken)
    if (verified) {
      return sessionToken
    }
  }

  const refreshed = await refreshSession()
  if (!hasValidTokenPair(refreshed)) {
    return null
  }

  return refreshed?.session_token?.trim() || null
}

export const loginWithPassword = async (username: string, password: string) => {
  await ensureAuthRuntime()

  const trimmedUsername = username.trim()
  const nonce = Date.now()
  const passwordHash = buckyos.hashPassword(trimmedUsername, password, nonce)
  const verifyHubClient = buckyos.getVerifyHubClient()
  verifyHubClient.setSeq(nonce)

  const response = await verifyHubClient.loginByPassword({
    username: trimmedUsername,
    password: passwordHash,
    appid: APP_ID,
    source_url: window.location.href,
  })

  const normalized = normalizeLoginResponse(response, trimmedUsername)
  const sessionToken = normalized.session_token?.trim()
  if (!sessionToken) {
    throw new Error('verify-hub did not return a session token')
  }

  saveStoredAccountInfo({
    ...normalized,
    session_token: sessionToken,
    refresh_token: normalized.refresh_token?.trim() || undefined,
  })
  resetVerifyCache()

  return normalized
}

export const signOutSession = async () => {
  try {
    await ensureAuthRuntime()
    buckyos.logout(true)
  } catch {
    // fallback to local cleanup below
  }

  clearStoredSession()
  resetVerifyCache()
}

export const getStoredUsername = () => getStoredAccountInfo()?.user_name || null
