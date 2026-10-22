import { buckyos } from 'buckyos'
import { buildAuthLoginTargetParams } from './loginTarget'
import { isMockRuntime } from '../runtime'

export function buildSsoCallbackUrl(redirectUrl: string, nonce: number): string {
  // Callback must be served on the redirect target origin so Set-Cookie applies to that host
  // (gateway matches path /sso_callback and forwards to control_panel for any app host).
  const target = new URL(redirectUrl, window.location.origin)
  const callback = new URL('/sso_callback', target.origin)
  callback.searchParams.set('nonce', String(nonce))
  callback.searchParams.set('redirect_url', redirectUrl)
  return callback.toString()
}

/**
 * `auth.login` with the nonce-salted password digest. Resolves to where the browser goes next:
 * the one-time `/sso_callback` that sets the session cookie and lands on `redirectUrl`, or the
 * page itself when no callback was prepared. Throws the kRPC error on failure.
 */
export async function loginWithPassword(username: string, password: string, redirectUrl: string): Promise<string> {
  const nonce = Date.now()
  const params = {
    username,
    password: buckyos.hashPassword(username, password, nonce),
    login_nonce: nonce,
    ...buildAuthLoginTargetParams(redirectUrl),
  }
  let response: Record<string, unknown>
  if (isMockRuntime()) {
    const { callMockControlPanel } = await import('../api/control_panel_mock.ts')
    const { data, error } = await callMockControlPanel<Record<string, unknown>>('auth.login', params)
    if (!data) throw error
    response = data
  } else {
    // A plain client, not getServiceRpcClient: this call never carries the runtime's session token.
    const rpcClient = new buckyos.kRPCClient('/kapi/control-panel')
    rpcClient.setSeq(nonce)
    response = (await rpcClient.call('auth.login', params)) as Record<string, unknown>
  }

  const sessionToken = typeof response.session_token === 'string' ? response.session_token.trim() : ''
  if (!sessionToken) {
    throw new Error('Login succeeded but no session token returned')
  }
  const ssoNonce = typeof response.sso_nonce === 'number' ? response.sso_nonce : 0
  if (redirectUrl && ssoNonce > 0) return buildSsoCallbackUrl(redirectUrl, ssoNonce)
  return redirectUrl || '/'
}
