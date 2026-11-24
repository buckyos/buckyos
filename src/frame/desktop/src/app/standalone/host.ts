/* ── Where a page that shows one app alone sends what the desktop's windows do ── */

import { buckyos } from 'buckyos'
import { appWindowId } from 'buckyos/open-app'
import { desktopUIStore, type OpenWindowRequest } from '../../models/DesktopUIDataModel'
import { translateNow } from '../../i18n/translate'
import { appTitleNow } from '../../presentation/appText'
import { currentZoneHost } from '../../presentation/zone'
import { isMockRuntime } from '../../runtime'
import { desktopCatalogIdForLogicalApp } from '../backend-apps'
import { pathOfLaunch } from '../launchPath'
import { agentGuide } from '../agent-setup/guide'
import { AGENT_GUIDE_APP_ID } from '../agent-setup/model'
import type { DesktopAppItem } from '../types'
import { systemAppAddress } from './address'
import { standaloneFrame } from './frame'

/** The address an installed web App has of its own. */
function webAppAddress(app: DesktopAppItem, path: string | undefined): string | null {
  const host = app.webHosts?.[0]
  if (!host) return null
  const zone = isMockRuntime() ? 'localhost' : buckyos.getZoneHostName()
  return new URL(path ?? '/', `${window.location.protocol}//${host}.${zone}${window.location.port ? `:${window.location.port}` : ''}`).href
}

/**
 * A system app's tab has the window's id as its name, so the same window is the same tab; a tab already open is only
 * brought to the front unless the request names a page.
 */
function openTab(address: string, name: string, navigate: boolean) {
  const tab = window.open('', name)
  if (!tab) {
    window.location.assign(address)
    return
  }
  if (navigate || tab.location.href === 'about:blank') tab.location.href = address
  tab.focus()
}

export function openStandaloneWindow(app: DesktopAppItem, request: OpenWindowRequest): string | null {
  if (app.id === AGENT_GUIDE_APP_ID) {
    void agentGuide.open()
    return null
  }
  const path = request.path ?? pathOfLaunch(app, request.launch)
  const catalogId = desktopCatalogIdForLogicalApp(app.logicalAppId ?? app.id)
  if (standaloneFrame.inClient) {
    void standaloneFrame.openApp({
      app: app.logicalAppId ?? app.id,
      ...(path ? { path } : {}),
      ...(request.windowPath !== undefined ? { window: request.windowPath } : {}),
      ...(request.mode ? { mode: request.mode } : {}),
    }).then((reply) => {
      if (!reply.ok) desktopUIStore.setSnackbar(translateNow('webFrame.unableToOpen').replace('{{app}}', () => appTitleNow(app)))
    })
    return app.id
  }
  if (app.webHosts?.length) {
    const address = webAppAddress(app, path)
    if (address) window.open(address, '_blank', 'noopener')
    return app.id
  }
  const name = request.mode === 'new' ? '_blank' : appWindowId(currentZoneHost(), app.id, request.windowPath)
  const address = systemAppAddress(catalogId, path)
  if (name === '_blank') window.open(address, name)
  else openTab(address, name, path !== undefined)
  return app.id
}

export function closeStandalonePage() {
  if (standaloneFrame.inClient) {
    standaloneFrame.requestClose()
    return
  }
  window.close()
  // A tab the user opened does not close itself; its app is left for the desktop.
  window.setTimeout(() => { if (!window.closed) window.location.assign('/') }, 300)
}
