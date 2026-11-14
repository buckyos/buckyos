/* The page of one Workspace on its own origin (T4-8): the host names the workspace, `?id=` the Surface and `?block=`
 * the Block to open. Leaving goes to the Zone host — the list on closing, the Desktop from "home" — and signing out
 * ends this origin's session before the Zone host's login page is shown. */

import { useEffect, useMemo, useState } from 'react'
import { buckyos } from 'buckyos'
import { useI18n } from '../../../i18n/provider'
import { useDocumentTitle } from '../../../presentation/documentHead'
import { isMockRuntime } from '../../../runtime'
import { AIWorkspaceAppPanel, type TabHost } from '../AIWorkspaceAppPanel'
import { AiwsClient } from '../api/client'
import { resolveTransport } from '../api/transport'
import { openWorkspaceById } from '../launch'
import { loginUrl, targetOf, workspaceIdOfHost, WORKSPACE_ROUTE, zoneOrigin } from '../links'
import { SessionPage } from '../session/SessionPage'

/** This origin's session is ended here (`/sso_logout` of this host), then the Zone host asks to sign in again. */
async function signOut() {
  if (!isMockRuntime()) {
    const response = await fetch('/sso_logout', { method: 'POST', credentials: 'include', cache: 'no-store', headers: { Accept: 'application/json' } })
    if (!response.ok) throw new Error(`Logout failed: ${response.status} ${response.statusText}`)
    buckyos.logout(true)
  }
  window.location.assign(loginUrl())
}

/** Another workspace (a clone, a New dialog's pick): its own origin; the list when the service names no page. */
async function goToWorkspace(workspaceId: string) {
  if (!(await openWorkspaceById(workspaceId))) window.location.assign(`${zoneOrigin()}${WORKSPACE_ROUTE}`)
}

/** Arriving from an entry link (`?entry=<entry id>`): the visitor joins the entry's group before the workspace opens
 * (权限需求 R8) — a guest with this origin's own guest token, which only this page holds. A join that fails (the entry
 * closed, expired or full) leaves the workspace to say what the visitor may do. */
function useEntryJoin(): boolean {
  const [entryId] = useState(() => new URLSearchParams(window.location.search).get('entry'))
  const [joined, setJoined] = useState(!entryId)
  useEffect(() => {
    if (!entryId) return
    const availability = resolveTransport()
    const join = availability.ok ? new AiwsClient(availability.transport).wsJoinEntry(entryId).catch(() => null) : Promise.resolve(null)
    void join.then(() => {
      const url = new URL(window.location.href)
      url.searchParams.delete('entry')
      window.history.replaceState(window.history.state, '', url)
      setJoined(true)
    })
  }, [entryId])
  return joined
}

export function StandalonePage() {
  const { t } = useI18n()
  const workspaceId = useMemo(() => workspaceIdOfHost(), [])
  const joined = useEntryJoin()
  const [target] = useState(() => targetOf(new URLSearchParams(window.location.search)))
  const appName = t('apps.aiworkspace')
  const [shownTitle, setShownTitle] = useState<string | null>(null)
  useDocumentTitle(shownTitle ? `${shownTitle} · ${appName}` : appName)

  if (!workspaceId) {
    return <main className="h-dvh w-full overflow-hidden bg-[color:var(--cp-bg)]"><div className="aiws-root"><div className="aiws-error" role="alert">{t('aiworkspace.core.app.notWorkspaceHost')}</div></div></main>
  }

  // a session of this workspace (第四期规划 §1: its rooms live on the workspace's own host)
  const sessionId = new URLSearchParams(window.location.search).get('session')
  if (sessionId) {
    return <main className="h-dvh w-full overflow-hidden bg-[color:var(--cp-bg)]" data-testid="aiws-session-main"><SessionPage sessionId={sessionId} /></main>
  }

  const tab: TabHost = {
    workspaceId,
    target,
    navigate: (id) => {
      if (id === null) window.location.assign(`${zoneOrigin()}${WORKSPACE_ROUTE}`)
      else if (id !== workspaceId) void goToWorkspace(id)
    },
    // the address keeps naming the workspace; what was asked for has been opened
    consumeTarget: () => {
      const url = new URL(window.location.href)
      url.searchParams.delete('id')
      url.searchParams.delete('block')
      window.history.replaceState(window.history.state, '', url)
    },
    onShown: (shown) => setShownTitle(shown?.title ?? null),
    home: () => window.location.assign(`${zoneOrigin()}/`),
    signOut,
  }

  return (
    <main className="h-dvh w-full overflow-hidden bg-[color:var(--cp-bg)]" data-testid="aiws-tab">
      {joined ? <AIWorkspaceAppPanel tab={tab} /> : <div className="aiws-root"><div className="aiws-muted" data-testid="aiws-entry-joining">{t('aiworkspace.core.app.joiningEntry')}</div></div>}
    </main>
  )
}
