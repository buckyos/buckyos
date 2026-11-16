/* What the shell leaves to the conversation side (APP7, DG2, DG3, RF4), implemented in conversation/. The shell
 * works without an entry and shows the matching control only when it is set. */

import type { MessageHubStore } from '../messagehub/store'
import type { MessageHubContext } from '../messagehub/types'
import type { BrowserAgent } from './api/backend'
import type { BrowserGroup } from './api/types'
import { afterHandover, sendToBrowserSession } from './conversation/session'

export interface ConversationContext {
  agent: BrowserAgent
  hub: MessageHubStore
  /** The signed-in user looking at their own messages. */
  context: MessageHubContext
}

export interface HandoverEvent {
  kind: 'hand_back' | 'hand_to_agent'
  agent: BrowserAgent
  /** The group as the kRPC call returned it. */
  group: BrowserGroup
  url: string
  title: string
  /** What the owner wrote: added to the "continue from this page" message of `hand_to_agent`, the whole message of `hand_back`; empty sends nothing after a hand back. */
  note: string
}

export interface AgentBrowserExtensions {
  /** DG2 / DG3: runs after `browser.hand_back` / `browser.hand_to_agent` succeeded — the message that wakes the Agent is sent here. */
  afterHandover?: (event: HandoverEvent, context: ConversationContext | null) => Promise<void>
  /** RF4: sends `text` as an ordinary message to the browser conversation (found or started first), which wakes the Agent. */
  askAgent?: (request: { text: string }, context: ConversationContext) => Promise<void>
}

export const agentBrowserExtensions: AgentBrowserExtensions = {
  afterHandover,
  askAgent: ({ text }, context) => sendToBrowserSession(text, context),
}
