/* S5: what other Apps put into the Agent panel of a conversation (the Agent browser's "using the browser" line).
 * Apps register here when they are listed (`app/registry.tsx`), so MessageHub does not know which Apps do. */

import type { ComponentType } from 'react'

export interface AgentSessionEntryProps {
  agentDid: string
  agentName: string
  /** The MessageHub session of the conversation. */
  sessionId: string
  /** The Turn in front, when there is one: a hint to look again when it changes. */
  hintTaskId?: string
  /** Changes with the conversation's messages. */
  signal?: unknown
}

const entries: Array<{ id: string; Entry: ComponentType<AgentSessionEntryProps> }> = []

/** An entry renders nothing when it has nothing to show for the conversation. */
export function registerAgentSessionEntry(id: string, Entry: ComponentType<AgentSessionEntryProps>): void {
  if (!entries.some((entry) => entry.id === id)) entries.push({ id, Entry })
}

export function agentSessionEntries(): ReadonlyArray<{ id: string; Entry: ComponentType<AgentSessionEntryProps> }> {
  return entries
}
