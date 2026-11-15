/* What the shell leaves to the conversation side (APP7, DG1–DG3), implemented in conversation/. The shell works
 * without an entry and shows the matching control only when it is set. */

import type { MessageHubStore } from '../messagehub/store'
import type { MessageHubContext, Session } from '../messagehub/types'
import type { BrowserAgent } from './api/backend'
import type { BrowserGroup } from './api/types'
import { isMarked } from './conversation/marks'
import { afterHandover, createThing, sendToGroup } from './conversation/session'
import type { Thing } from './store/things'

export interface ThingContext {
  agent: BrowserAgent
  hub: MessageHubStore
  /** The signed-in user looking at their own messages. */
  context: MessageHubContext
}

export interface HandoverEvent {
  kind: 'hand_back' | 'hand_to_agent'
  agent: BrowserAgent
  thing: Thing
  /** The group as the kRPC call returned it. */
  group: BrowserGroup
  url: string
  title: string
  /** What the owner wrote: added to the "continue from this page" message of `hand_to_agent`, the whole message of `hand_back`; empty sends nothing after a hand back. */
  note: string
}

export interface AgentBrowserExtensions {
  /** DG1: whether a session of the Agent is listed as a thing. Unset: every active session is. */
  listsThing?: (session: Session, context: ThingContext) => boolean
  /** DG1: "New task" — creates the session (with its source mark) and resolves to it; the shell then switches to its group. Unset: no button. */
  createThing?: (context: ThingContext) => Promise<Session | null>
  /** DG2 / DG3: runs after `browser.hand_back` / `browser.hand_to_agent` succeeded — the message that wakes the Agent is sent here. */
  afterHandover?: (event: HandoverEvent, context: ThingContext | null) => Promise<void>
  /** RF4: sends `text` as an ordinary message to the conversation of `group` (binding the default group's session first when it has none), which wakes the Agent. */
  askAgent?: (request: { group: string; text: string }, context: ThingContext) => Promise<void>
}

export const agentBrowserExtensions: AgentBrowserExtensions = {
  listsThing: (session, context) => isMarked(context, session.id),
  createThing,
  afterHandover,
  askAgent: ({ group, text }, context) => sendToGroup(group, text, context),
}
