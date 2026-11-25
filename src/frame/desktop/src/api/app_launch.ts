import { prepareAppLaunch, usableAppLaunch, type AppLaunchResult, type LaunchTarget } from '../shared/controlPanel/appLaunch.ts'
import { isMockRuntime } from '../runtime'
import { shellChannel } from './channel.ts'

export type { AppLaunchResult }

async function requestLaunch(target: LaunchTarget, pageUrl: string): Promise<AppLaunchResult | null> {
  if (!isMockRuntime()) return prepareAppLaunch(shellChannel, target, pageUrl)
  const { data } = await (await import('./app_launch_mock.ts')).mockPrepareAppLaunch(pageUrl)
  return usableAppLaunch(data, pageUrl)
}

/** Signs the current desktop user into `appInstanceId` and lands on `appUrl`. Null on a transient failure. */
export function requestAppLaunch(appInstanceId: string, appUrl: string): Promise<AppLaunchResult | null> {
  return requestLaunch({ appInstanceId }, appUrl)
}

/** The same for a page of a system service on its own origin: an AI Workspace on `aiws-<id>.<zone>`. */
export function requestServiceLaunch(serviceId: string, pageUrl: string): Promise<AppLaunchResult | null> {
  return requestLaunch({ serviceId }, pageUrl)
}

/** Desktop session refresh for the mock runtime; the real one goes through the SDK. */
export async function mockRefreshDesktopSession(): Promise<boolean> {
  return (await import('./app_launch_mock.ts')).mockRefreshSession()
}
