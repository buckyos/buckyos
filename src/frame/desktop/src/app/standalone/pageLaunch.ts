import type { WindowLaunch } from '../../models/ui'
import { launchOfPath } from '../launchPath'
import { systemAppPath } from './address'
import { systemApps } from './catalog'

/** The launch the address of a system app's page stands for. */
export function pageLaunch(appId: string, location: Pick<Location, 'pathname' | 'search' | 'hash'>): WindowLaunch | undefined {
  const app = systemApps().find((item) => item.id === appId)
  return app ? launchOfPath(app, systemAppPath(appId, location)) : undefined
}
