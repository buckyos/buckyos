import type { ZoneChannel } from '../channel.ts'
import { callControlPanel } from './call.ts'

/** `auth.prepare_app_launch` answer. Expected failures are statuses; RPC errors are transient. */
export type AppLaunchResult =
  | { status: 'ready'; callback_url: string }
  | { status: 'source_session_inactive' | 'access_denied' | 'invalid_target' }

const STATUSES = new Set<string>(['ready', 'source_session_inactive', 'access_denied', 'invalid_target'])

/**
 * Who is being launched: an App instance, or a system service on an origin of its own (an AI Workspace on
 * `aiws-<id>.<zone>`).
 */
export type LaunchTarget = { appInstanceId: string } | { serviceId: string }

/** The answer when it is usable for `pageUrl`: a known status, and a callback on the page's own origin. Null otherwise. */
export function usableAppLaunch(data: AppLaunchResult | null | undefined, pageUrl: string): AppLaunchResult | null {
  if (!data || !STATUSES.has(data.status)) return null
  if (data.status === 'ready') {
    try {
      if (new URL(data.callback_url).origin !== new URL(pageUrl).origin) return null
    } catch {
      return null
    }
  }
  return data
}

/**
 * Asks Control Panel for a one-time `/sso_callback` URL on the target's host that signs the current user in there and
 * then lands on `pageUrl`. Resolves null on a transient failure (RPC error, network, malformed answer).
 */
export async function prepareAppLaunch(channel: ZoneChannel, target: LaunchTarget, pageUrl: string): Promise<AppLaunchResult | null> {
  try {
    const data = await callControlPanel<AppLaunchResult>(channel, 'auth.prepare_app_launch', {
      ...('appInstanceId' in target ? { app_instance_id: target.appInstanceId } : { service_id: target.serviceId }),
      redirect_url: pageUrl,
    })
    return usableAppLaunch(data, pageUrl)
  } catch {
    return null
  }
}
