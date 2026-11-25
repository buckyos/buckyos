/* ── An `OpenAppRequest` in the desktop: from an App's frame, or from `/?open-app=…` ── */

import type { AppOpenReply } from 'buckyos/app-frame'
import type { AppOpenRequest } from 'buckyos/open-app'
import { desktopUIStore } from '../models/DesktopUIDataModel'
import { agentGuide } from './agent-setup/guide'
import { AGENT_GUIDE_APP_ID } from './agent-setup/model'
import { desktopCatalogIdForLogicalApp } from './backend-apps'
import { launchOfPath } from './launchPath'
import { systemAppAddress } from './standalone/address'

/** `navigate` (a phone-sized desktop): some system apps have a route there instead of a window. */
export function openAppRequest(request: Omit<AppOpenRequest, 'zone'>, navigate?: (path: string) => void): AppOpenReply {
  const { apps } = desktopUIStore.getSnapshot()
  const app = apps.find((item) => item.id === request.app) ?? apps.find((item) => item.logicalAppId === request.app || item.appInstanceId === request.app)
  if (!app) return { ok: false, code: 'not_found' }
  if (app.id === AGENT_GUIDE_APP_ID) {
    void agentGuide.open()
    return { ok: true }
  }
  if (!app.loader) return { ok: false, code: 'unsupported' }
  if (navigate && app.manifest.mobileRedirectPath) {
    navigate(systemAppAddress(desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id), request.path))
    return { ok: true }
  }
  desktopUIStore.openAppWindow(app.id, {
    path: request.path,
    launch: launchOfPath(app, request.path),
    windowPath: request.window,
    mode: request.mode,
  })
  return { ok: true }
}
