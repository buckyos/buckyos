import { callRpc } from './rpc.ts'
import { isMockRuntime } from '../runtime'

/** `auth.prepare_app_launch` answer. Expected failures are statuses; RPC errors are transient. */
export type AppLaunchResult =
  | { status: 'ready'; callback_url: string }
  | { status: 'source_session_inactive' | 'access_denied' | 'invalid_target' }

const STATUSES = new Set<string>(['ready', 'source_session_inactive', 'access_denied', 'invalid_target'])

/**
 * Asks Control Panel for a one-time `/sso_callback` URL on the App host that signs the current
 * desktop user into `appInstanceId` and then lands on `appUrl`. Resolves null on a transient
 * failure (RPC error, network, malformed answer).
 */
export async function requestAppLaunch(appInstanceId: string, appUrl: string): Promise<AppLaunchResult | null> {
  const { data, error } = isMockRuntime()
    ? await (await import('./app_launch_mock.ts')).mockPrepareAppLaunch(appUrl)
    : await callRpc<AppLaunchResult>('auth.prepare_app_launch', { app_instance_id: appInstanceId, redirect_url: appUrl })
  if (error || !data || !STATUSES.has(data.status)) return null
  if (data.status === 'ready') {
    try {
      if (new URL(data.callback_url).origin !== new URL(appUrl).origin) return null
    } catch {
      return null
    }
  }
  return data
}

/**
 * The same for a page of a system service on its own origin — an AI Workspace on `aiws-<id>.<zone>`
 * (`auth.prepare_app_launch { service_id, redirect_url }`): the Control Panel delegates the Desktop session to
 * that service target and parks it behind the one-time `/sso_callback` of the page's origin.
 */
export async function requestServiceLaunch(serviceId: string, pageUrl: string): Promise<AppLaunchResult | null> {
  const { data, error } = isMockRuntime()
    ? await (await import('./app_launch_mock.ts')).mockPrepareAppLaunch(pageUrl)
    : await callRpc<AppLaunchResult>('auth.prepare_app_launch', { service_id: serviceId, redirect_url: pageUrl })
  if (error || !data || !STATUSES.has(data.status)) return null
  if (data.status === 'ready') {
    try {
      if (new URL(data.callback_url).origin !== new URL(pageUrl).origin) return null
    } catch {
      return null
    }
  }
  return data
}

/** Desktop session refresh for the mock runtime; the real one goes through the SDK. */
export async function mockRefreshDesktopSession(): Promise<boolean> {
  return (await import('./app_launch_mock.ts')).mockRefreshSession()
}
