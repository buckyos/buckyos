/* What the home's cards do besides opening in place: a public class in a tab of its own, a workspace in a new tab. */

import { unwrap, type AiwsClient } from '../../api/client'
import type { Transport } from '../../api/transport'
import { openWorkspacePage, workspaceLaunchUrl } from '../../launch'
import { workspacePageUrl } from '../../links'
import { listOpenSessions } from '../../session/api'
import type { OpenSession } from '../../session/types'
import { sessionAction } from './model'

/** The public classes the home lists (K7, `session.list_open`): every one the caller may open without a link — their
 * own, the ones they present or joined, and the open rooms of courseware they may read. */
export function loadHomeSessions(transport: Transport): Promise<OpenSession[]> {
  return listOpenSessions(transport)
}

/** Open a public class as its state allows, in a tab of its own; false when it cannot be opened (expired, no address). */
export function openSession(session: OpenSession): boolean {
  if (sessionAction(session) === 'expired' || !session.url) return false
  void openWorkspacePage(session.url, { newTab: true })
  return true
}

/** A workspace's page in a new tab. The tab is opened at once, in answer to the click (popup blockers), and sent to the
 * page once the service named it; a deployment without workspace pages (no `public_base`) has nothing to open. */
export async function openWorkspaceInNewTab(client: AiwsClient, workspaceId: string, noPage: string): Promise<void> {
  const tab = window.open('', '_blank')
  try {
    const info = unwrap(await client.wsGetInfo({ workspace_id: workspaceId }))
    const url = workspacePageUrl(info.public_base, info.title)
    if (!url) throw new Error(noPage)
    const destination = await workspaceLaunchUrl(url)
    if (tab) {
      tab.opener = null
      tab.location.replace(destination)
    } else window.open(destination, '_blank', 'noopener')
  } catch (error) {
    tab?.close()
    throw error
  }
}
