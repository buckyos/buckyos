/* Going to a Workspace's own origin (T4-8). A signed-in Desktop user must arrive there signed in: the Control Panel
 * delegates the Desktop session to the `aiworkspace` service target and parks it behind a one-time `/sso_callback`
 * on that origin (`auth.prepare_app_launch { service_id: 'aiworkspace', redirect_url }`, the way Apps are launched);
 * the browser is sent to the callback, which sets this origin's cookies and lands on the page. Anyone else — the mock
 * runtime, the dev override against a standalone backend, a page already on a Workspace host (it holds its own
 * session), a guest — goes to the page directly. */

import { isMockRuntime } from '../../runtime'
import { requestServiceLaunch } from '../../api/app_launch'
import { AiwsClient } from './api/client'
import { DEV_OVERRIDE_KEY, resolveTransport } from './api/transport'
import { onWorkspaceHost, workspacePageUrl } from './links'

function devOverride(): boolean {
  try { return window.localStorage.getItem(DEV_OVERRIDE_KEY) !== null } catch { return false }
}

/** Where the browser should go to open `url` as the current Desktop user: the one-time callback, or the page. */
export async function workspaceLaunchUrl(url: string): Promise<string> {
  if (isMockRuntime() || devOverride() || onWorkspaceHost()) return url
  const launch = await requestServiceLaunch('aiworkspace', url).catch(() => null)
  return launch?.status === 'ready' ? launch.callback_url : url
}

/** A workspace by id (picked from the list, a fork, a New dialog's choice): its page as the service names it. False
 * when there is no page to go to (a deployment without a host). */
export async function openWorkspaceById(workspaceId: string): Promise<boolean> {
  const availability = resolveTransport()
  if (!availability.ok) return false
  const info = await new AiwsClient(availability.transport).wsGetInfo({ workspace_id: workspaceId })
  const url = info.ok ? workspacePageUrl(info.public_base, info.title) : null
  if (!url) return false
  await openWorkspacePage(url)
  return true
}

/** Open a Workspace page: in this tab, or in a new one (the window is opened before the asynchronous launch so that
 * popup blockers see a direct answer to the click). */
export async function openWorkspacePage(url: string, how: { newTab?: boolean } = {}): Promise<void> {
  if (!how.newTab) {
    window.location.assign(await workspaceLaunchUrl(url))
    return
  }
  const tab = window.open('', '_blank')
  const destination = await workspaceLaunchUrl(url)
  if (tab) {
    tab.opener = null
    tab.location.replace(destination)
  } else {
    window.open(destination, '_blank', 'noopener')
  }
}
