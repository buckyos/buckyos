import { buckyos } from 'buckyos'
import { clearMessageHubLocalState } from '../app/messagehub/api/local'
import { isMockRuntime } from '../runtime'

function removeLocalAuthItems() {
  window.localStorage.removeItem('user_info')
  window.localStorage.removeItem('buckyos.identity')
  window.localStorage.removeItem('buckyos.account_info')
  window.localStorage.removeItem('buckyos.account_info.control-panel')
  document.cookie = 'control-panel_token=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; SameSite=Lax'
}

export function clearDesktopAuthState() {
  if (!isMockRuntime()) {
    buckyos.logout(true)
  }
  removeLocalAuthItems()
}

async function postSsoLogout() {
  const response = await fetch('/sso_logout', {
    method: 'POST',
    credentials: 'include',
    cache: 'no-store',
    headers: {
      Accept: 'application/json',
    },
  })
  if (!response.ok) {
    throw new Error(`Logout failed: ${response.status} ${response.statusText}`)
  }
}

export async function signOutToLogin() {
  await postSsoLogout()

  clearDesktopAuthState()
  await clearMessageHubLocalState()

  const loginUrl = new URL('/login', window.location.origin)
  loginUrl.searchParams.set('redirect_url', window.location.href)
  window.location.assign(loginUrl.toString())
}

/**
 * Ends the current session and its local state but stays on the page (switching identity). The
 * logout completes before this resolves, so a sign-in that follows cannot have its new cookies
 * cleared by a late logout response.
 */
export async function endSessionInPlace() {
  if (isMockRuntime()) {
    const { mockEndSession } = await import('../api/control_panel_mock.ts')
    mockEndSession()
  } else {
    await postSsoLogout()
    buckyos.logout(false)
  }
  removeLocalAuthItems()
  await clearMessageHubLocalState()
}
