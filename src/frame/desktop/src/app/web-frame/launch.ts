import { useCallback, useEffect, useState } from 'react'
import { buckyos, getActiveSessionToken } from 'buckyos'
import { mockRefreshDesktopSession, requestAppLaunch } from '../../api/app_launch'
import { redirectToDesktopLogin } from '../../auth/loginRedirect'
import { isMockRuntime } from '../../runtime'

export type AppLaunchOutcome =
  | { kind: 'ready'; url: string }
  | { kind: 'access_denied' | 'invalid_target' | 'session_inactive' | 'failed' | 'signed_out' }

let sessionRefresh: Promise<boolean> | undefined

/** One forced refresh of the desktop session shared by every window that needs it. */
function refreshDesktopSession(): Promise<boolean> {
  sessionRefresh ??= (async () => {
    if (isMockRuntime()) return mockRefreshDesktopSession()
    // The SDK only goes to /sso_refresh once it holds no usable token.
    buckyos.logout(false)
    return Boolean(await getActiveSessionToken().catch(() => null))
  })().finally(() => { sessionRefresh = undefined })
  return sessionRefresh
}

async function launchOnce(appInstanceId: string, appUrl: string): Promise<AppLaunchOutcome> {
  let result = await requestAppLaunch(appInstanceId, appUrl)
  if (result?.status === 'source_session_inactive') {
    if (!(await refreshDesktopSession())) {
      redirectToDesktopLogin()
      return { kind: 'signed_out' }
    }
    result = await requestAppLaunch(appInstanceId, appUrl)
  }
  switch (result?.status) {
    case 'ready': return { kind: 'ready', url: result.callback_url }
    case 'access_denied': return { kind: 'access_denied' }
    case 'invalid_target': return { kind: 'invalid_target' }
    case 'source_session_inactive': return { kind: 'session_inactive' }
    default: return { kind: 'failed' }
  }
}

// A StrictMode re-run of the same launch joins the request already in flight instead of
// issuing a second session.
const inflight = new Map<string, Promise<AppLaunchOutcome>>()

function launchApp(key: string, appInstanceId: string, appUrl: string): Promise<AppLaunchOutcome> {
  let pending = inflight.get(key)
  if (!pending) {
    pending = launchOnce(appInstanceId, appUrl).finally(() => inflight.delete(key))
    inflight.set(key, pending)
  }
  return pending
}

/**
 * Signs the desktop user into the App before its iframe loads: resolves to the one-time
 * callback URL to navigate the iframe to. A new `appUrl` (re-navigation) prepares a new launch;
 * answers for an older one are dropped.
 */
export function useAppLaunch(appInstanceId: string | undefined, appUrl: string | undefined) {
  const [attempt, setAttempt] = useState(0)
  const [settled, setSettled] = useState<{ key: string; outcome: AppLaunchOutcome }>()
  const key = appInstanceId && appUrl ? `${attempt}\n${appInstanceId}\n${appUrl}` : undefined
  useEffect(() => {
    if (!key || !appInstanceId || !appUrl) return
    let live = true
    void launchApp(key, appInstanceId, appUrl).then((outcome) => { if (live) setSettled({ key, outcome }) })
    return () => { live = false }
  }, [key, appInstanceId, appUrl])
  const retry = useCallback(() => setAttempt((value) => value + 1), [])
  return { outcome: settled && settled.key === key ? settled.outcome : undefined, retry }
}
