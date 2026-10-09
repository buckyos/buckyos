import { buckyos } from 'buckyos'
import { clearMessageHubLocalState } from '../app/messagehub/api/local'
import { isMockRuntime } from '../runtime'

export function clearDesktopAuthState() {
  if (!isMockRuntime()) {
    buckyos.logout(true)
  }

  window.localStorage.removeItem('user_info')
  window.localStorage.removeItem('buckyos.account_info')
  window.localStorage.removeItem('buckyos.account_info.control-panel')
  document.cookie = 'control-panel_token=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; SameSite=Lax'
}

export async function signOutToLogin() {
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

  clearDesktopAuthState()
  await clearMessageHubLocalState()

  const loginUrl = new URL('/login', window.location.origin)
  loginUrl.searchParams.set('redirect_url', window.location.href)
  window.location.assign(loginUrl.toString())
}
