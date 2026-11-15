/* The conversation of a thing (protocol §1): a named group's is the MessageHub session of the same id. The default
 * group has none until one is bound to it: the first time its conversation is used, the owner's default conversation
 * with the Agent is taken (created when there is none) and `browser.bind_default` binds it. */

import { translate } from '../../messagehub/translate'
import type { Session } from '../../messagehub/types'
import { getBrowserBackend } from '../api/backend'
import { DEFAULT_GROUP, type BrowserStatus } from '../api/types'
import type { HandoverEvent, ThingContext } from '../extensions'
import { defaultThingSession, groupIdOfSession } from '../store/things'
import { isMarked, loadMarks, markSession } from './marks'

const binding = new Map<string, Promise<Session>>()
const listeners = new Set<(agentId: string) => void>()

/** `browser.bind_default` succeeded for this Agent: `default_session_id` of its `browser.status` changed. */
export function subscribeDefaultBinding(listener: (agentId: string) => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

function ensurePeer({ agent, hub, context }: ThingContext) {
  if (!hub.findEntity(context, agent.agentDid)) hub.addPeer(context, agent.agentDid, { name: agent.name, type: 'agent' })
}

async function bindDefault(thing: ThingContext): Promise<Session> {
  const { agent, hub, context } = thing
  const backend = await getBrowserBackend()
  await hub.ensureOwner(context)
  const { default_session_id: bound } = await backend.call<BrowserStatus>(agent, 'browser.status', { group_id: DEFAULT_GROUP })
  const active = () => hub.sessions(context, agent.agentDid, 'active')
  const current = defaultThingSession(active(), agent.agentDid, bound)
  if (current) return current
  ensurePeer(thing)
  let session = await hub.ensureDefaultSession(context, agent.agentDid)
  if (session) {
    // A thing started here keeps its own tabs: bound to the default group they would be folded into it.
    await loadMarks(thing, active().map((item) => item.id))
    if (isMarked(thing, session.id)) {
      session = active().filter((item) => item.binding.kind === 'native' && !isMarked(thing, item.id)).sort((a, b) => b.lastActiveAt - a.lastActiveAt)[0]
        ?? await hub.create(context, { entityId: agent.agentDid, title: '', connection: 'native' })
    }
  }
  if (!session) throw Error('session_missing')
  if (groupIdOfSession(session, agent.agentDid, null) !== DEFAULT_GROUP) {
    await backend.call(agent, 'browser.bind_default', { session_id: session.id })
    for (const listener of [...listeners]) listener(agent.agentId)
  }
  return session
}

/** The session of the default thing; one is bound to the default group when it has none (or the bound one is gone). */
export function ensureDefaultSession(thing: ThingContext): Promise<Session> {
  const { agentId } = thing.agent
  let pending = binding.get(agentId)
  if (!pending) {
    pending = bindDefault(thing).finally(() => { binding.delete(agentId) })
    binding.set(agentId, pending)
  }
  return pending
}

export async function sessionOfGroup(group: string, thing: ThingContext): Promise<Session> {
  if (group === DEFAULT_GROUP) return ensureDefaultSession(thing)
  const { agent, hub, context } = thing
  await hub.ensureOwner(context)
  const session = hub.sessions(context, agent.agentDid, 'active').find((item) => item.id === group)
  if (!session) throw Error('session_missing')
  return session
}

/** An ordinary message from the owner in the conversation of `group`: what wakes the Agent. */
export async function sendToGroup(group: string, text: string, thing: ThingContext): Promise<void> {
  const session = await sessionOfGroup(group, thing)
  await thing.hub.send(thing.context, session.id, { content: text, attachments: [] }, undefined)
}

const two = (value: number) => String(value).padStart(2, '0')

/** DG1: a new session with the Agent that carries the source mark; its name tells the things apart until the owner renames it. */
export async function createThing(thing: ThingContext): Promise<Session> {
  const { agent, hub, context } = thing
  await hub.ensureOwner(context)
  ensurePeer(thing)
  const now = new Date()
  const title = `${translate('agentBrowser.thing.newTitle', 'Browser task')} ${two(now.getMonth() + 1)}-${two(now.getDate())} ${two(now.getHours())}:${two(now.getMinutes())}`
  const session = await hub.create(context, { entityId: agent.agentDid, title, connection: 'native' })
  try {
    await markSession(thing, session.id)
  } catch (error) {
    await hub.manage(context, session.id, 'delete').catch(() => undefined)
    throw error
  }
  return session
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
export async function afterHandover(event: HandoverEvent, thing: ThingContext | null): Promise<void> {
  const text = event.kind === 'hand_to_agent' ? handToAgentText(event) : event.note.trim()
  if (!text) return
  if (!thing) throw Error('conversation_unavailable')
  await sendToGroup(event.thing.groupId, text, thing)
}
