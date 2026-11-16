/* The conversation of the Agent's panel (C2-1): one MessageHub session with each of the viewer's Agents, whatever the
 * workspace, started here with the source mark `ui.source = aiworkspace` (components/conversation/agentSessions) — the
 * Agent Browser's D35 rule without its binding, since nothing on the Agent's side is routed to this session. The
 * latest marked one is it; none, it is started the first time the panel needs it. */

import type { DID } from 'buckyos'
import { createSourcedSession, isSourceKnown, sourcedSessions } from '../../../../components/conversation/agentSessions'
import { loadConversationBackend } from '../../../../components/conversation/backend'
import { aiwsT } from '../../i18n'

export const PANEL_SOURCE = 'aiworkspace'

export interface PanelAgent { did: DID; name: string }

let agent: Promise<PanelAgent | null> | null = null

/** The viewer's own Agent (the one MessageHub's own Agent conversations default to), kept once found. */
export function panelAgent(): Promise<PanelAgent | null> {
  if (!agent) {
    const lookup = loadConversationBackend().then((backend) => backend.defaultAgent())
    agent = lookup
    lookup.then((found) => { if (!found && agent === lookup) agent = null }, () => { if (agent === lookup) agent = null })
  }
  return agent
}

async function findSession({ did, name }: PanelAgent): Promise<string> {
  const backend = await loadConversationBackend()
  await backend.ensureReady()
  const [latest] = await sourcedSessions(did, PANEL_SOURCE)
  if (latest) return latest.id
  // a mark that could not be read may be this one's: no second conversation is started then
  if (backend.sessions(did).some((session) => !isSourceKnown(session.id))) throw Error('marks_unavailable')
  return (await createSourcedSession(did, PANEL_SOURCE, aiwsT('aiworkspace.jarvis.sessionTitle'), name)).id
}

const finding = new Map<string, Promise<string>>()

/** The panel's conversation with the Agent; calls at the same time share one lookup. */
export function panelSession(of: PanelAgent): Promise<string> {
  let pending = finding.get(of.did)
  if (!pending) {
    pending = findSession(of).finally(() => { finding.delete(of.did) })
    finding.set(of.did, pending)
  }
  return pending
}

/** A message of the viewer to the Agent in that conversation (a data source the owner asks it for). */
export async function sendToPanelSession(of: PanelAgent, text: string): Promise<void> {
  const sessionId = await panelSession(of)
  const backend = await loadConversationBackend()
  await backend.send(sessionId, { text })
}
