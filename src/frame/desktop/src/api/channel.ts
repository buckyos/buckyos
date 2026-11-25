import { buckyos, getActiveSessionToken } from 'buckyos'
import type { ZoneChannel } from '../shared/channel.ts'

async function authHeaders(): Promise<Record<string, string>> {
  // The session cookie also authenticates; the header covers a browser that keeps no cookie for the page.
  const account = await buckyos.getAccountInfo().catch(() => null)
  const token = account?.session_token || (await getActiveSessionToken().catch(() => null))
  return token ? { 'X-Auth': token } : {}
}

/** The Zone this page is served from, through the SDK's global runtime. */
export const shellChannel: ZoneChannel = {
  rpc: (service, method, params = {}) => buckyos.getServiceRpcClient(service).call(method, params),
  fetch: async (path, init = {}) => {
    const auth = await authHeaders()
    return fetch(path, { credentials: 'same-origin', ...init, headers: { ...auth, ...(init.headers as Record<string, string>) } })
  },
}

/** The same Zone without a session: for what signed-out pages (the login page) may read too. */
export const publicChannel: ZoneChannel = {
  rpc: (service, method, params = {}) => new buckyos.kRPCClient(`/kapi/${service}`).call(method, params),
  fetch: (path, init) => fetch(path, init),
}
