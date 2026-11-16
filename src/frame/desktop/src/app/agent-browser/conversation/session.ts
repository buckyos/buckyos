/* The browser conversation (protocol §1, D35): one MessageHub session with the Agent, started here with the source
 * mark (`ui.source` = the App's id, kept by components/conversation/agentSessions), where everything about its
 * browser is said. The first time it is needed it is looked up among the Agent's marked sessions (the bound one
 * first, else the latest active one) or started, and bound with `browser.bind_default`. */

import { createSourcedSession, isSourced, isSourceKnown, loadSources } from '../../../components/conversation/agentSessions'
import { translate } from '../../messagehub/translate'
import type { Session } from '../../messagehub/types'
import { getBrowserBackend } from '../api/backend'
import type { BrowserStatus } from '../api/types'
import type { ConversationContext, HandoverEvent } from '../extensions'
import { AGENT_BROWSER_APP_ID } from '../launch'

const finding = new Map<string, Promise<Session>>()
const listeners = new Set<(agentId: string, sessionId: string) => void>()

/** The browser conversation of an Agent was found (or started) and is bound. */
export function subscribeBrowserSession(listener: (agentId: string, sessionId: string) => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** DG1: the session was started in the Agent browser (its `ui.source` is the App's id). */
const isBrowserSession = (sessionId: string) => isSourced(sessionId, AGENT_BROWSER_APP_ID)

/** A new session with the Agent that carries the source mark. */
async function startBrowserSession({ agent, hub, context }: ConversationContext): Promise<Session> {
  const created = await createSourcedSession(agent.agentDid, AGENT_BROWSER_APP_ID, translate('agentBrowser.session.title', 'Browser'), agent.name)
  const session = hub.sessions(context, agent.agentDid, 'active').find((item) => item.id === created.id)
  if (!session) throw Error('session_missing')
  return session
}

async function findBrowserSession(conversation: ConversationContext): Promise<Session> {
  const { agent, hub, context } = conversation
  const backend = await getBrowserBackend()
  await hub.ensureOwner(context)
  const { default_session_id: bound } = await backend.call<BrowserStatus>(agent, 'browser.status', {})
  const active = hub.sessions(context, agent.agentDid, 'active')
  const current = active.find((item) => item.id === bound)
  if (current) await loadSources([current.id])
  let session = current && isBrowserSession(current.id) ? current : undefined
  if (!session) {
    await loadSources(active.map((item) => item.id))
    session = active.filter((item) => isBrowserSession(item.id)).sort((a, b) => b.lastActiveAt - a.lastActiveAt)[0]
    // A mark that could not be read may be the browser conversation's: no second one is started then.
    if (!session && active.some((item) => !isSourceKnown(item.id))) throw Error('marks_unavailable')
    session ??= await startBrowserSession(conversation)
  }
  if (session.id !== bound) await backend.call(agent, 'browser.bind_default', { session_id: session.id })
  for (const listener of [...listeners]) listener(agent.agentId, session.id)
  return session
}

/** The Agent's browser conversation, bound; calls at the same time share one lookup. */
export function ensureBrowserSession(conversation: ConversationContext): Promise<Session> {
  const { agentId } = conversation.agent
  let pending = finding.get(agentId)
  if (!pending) {
    pending = findBrowserSession(conversation).finally(() => { finding.delete(agentId) })
    finding.set(agentId, pending)
  }
  return pending
}

/** An ordinary message from the owner in the browser conversation: what wakes the Agent. */
export async function sendToBrowserSession(text: string, conversation: ConversationContext): Promise<void> {
  const session = await ensureBrowserSession(conversation)
  await conversation.hub.send(conversation.context, session.id, { content: text, attachments: [] }, undefined)
}

/** DG2: what the Agent is told after `hand_to_agent`: to go on from the page, which page that is, and what the owner added. */
function handToAgentText(event: HandoverEvent): string {
  const lines = [translate('agentBrowser.handover.continue', 'Please continue from the current page.')]
  if (event.url && event.url !== 'about:blank') {
    if (event.title) lines.push(translate('agentBrowser.handover.page', 'Page: {{title}}').replace('{{title}}', () => event.title))
    lines.push(translate('agentBrowser.handover.url', 'URL: {{url}}').replace('{{url}}', () => event.url))
  }
  const note = event.note.trim()
  return note ? `${lines.join('\n')}\n\n${note}` : lines.join('\n')
}

/** DG2 / DG3: the message that wakes the Agent after the page was handed to it or handed back. */
export async function afterHandover(event: HandoverEvent, conversation: ConversationContext | null): Promise<void> {
  const text = event.kind === 'hand_to_agent' ? handToAgentText(event) : event.note.trim()
  if (!text) return
  if (!conversation) throw Error('conversation_unavailable')
  await sendToBrowserSession(text, conversation)
}
