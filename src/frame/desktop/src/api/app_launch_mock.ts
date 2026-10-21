import type { AppLaunchResult } from './app_launch.ts'

/**
 * Scenario of the mocked `auth.prepare_app_launch`, read from localStorage:
 * `ready` (default), `access_denied`, `invalid_target`, `error` (transient),
 * `inactive_once` (inactive until one session refresh), `inactive` (refresh succeeds, the
 * source stays inactive) and `signed_out` (the refresh fails too).
 */
export const MOCK_APP_LAUNCH_KEY = 'buckyos.mock.app-launch'
const REFRESHED_KEY = 'buckyos.mock.app-launch.refreshed'
/** Number of prepare calls this tab made, for tests that a launch is prepared once. */
export const MOCK_APP_LAUNCH_COUNT_KEY = 'buckyos.mock.app-launch.count'

function scenario(): string {
  try { return localStorage.getItem(MOCK_APP_LAUNCH_KEY) ?? 'ready' } catch { return 'ready' }
}

function refreshed(): boolean {
  try { return sessionStorage.getItem(REFRESHED_KEY) === '1' } catch { return false }
}

export async function mockPrepareAppLaunch(appUrl: string): Promise<{ data: AppLaunchResult | null; error: unknown }> {
  try { sessionStorage.setItem(MOCK_APP_LAUNCH_COUNT_KEY, String(Number(sessionStorage.getItem(MOCK_APP_LAUNCH_COUNT_KEY) ?? 0) + 1)) } catch { /* no storage */ }
  await new Promise((resolve) => window.setTimeout(resolve, 80))
  const status = scenario()
  if (status === 'error') return { data: null, error: new Error('mock prepare_app_launch failure') }
  if (status === 'access_denied' || status === 'invalid_target') return { data: { status }, error: null }
  if (status === 'inactive' || status === 'signed_out' || (status === 'inactive_once' && !refreshed())) {
    return { data: { status: 'source_session_inactive' }, error: null }
  }
  // Same shape as Control Panel: the App host's callback carries the whole target URL, and the
  // e2e fixtures answer it with the 302 the real callback sends.
  const callback = new URL('/sso_callback', new URL(appUrl).origin)
  callback.searchParams.set('nonce', String(Math.floor(Math.random() * 2 ** 52) + 1))
  callback.searchParams.set('redirect_url', appUrl)
  return { data: { status: 'ready', callback_url: callback.href }, error: null }
}

export async function mockRefreshSession(): Promise<boolean> {
  await new Promise((resolve) => window.setTimeout(resolve, 40))
  if (scenario() === 'signed_out') return false
  try { sessionStorage.setItem(REFRESHED_KEY, '1') } catch { /* no storage */ }
  return true
}
