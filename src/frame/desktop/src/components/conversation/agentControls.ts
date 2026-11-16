import { createContext } from 'react'

/**
 * The owner's controls over the sessions of their Agents, with the viewer's permissions (only the Agent's owner
 * gets an answer). Without them the Turn and work session controls are not offered.
 */
export interface AgentSessionControls {
  /** `session.pause {sid}`: the session and its unfinished sub sessions infer no more until resumed. */
  pause(agentDid: string, sid: string): Promise<void>
  /** `session.resume {sid}`: also resumes the sub sessions paused with it. */
  resume(agentDid: string, sid: string): Promise<void>
  /** `session.stop {sid, scope}`: `turn` closes the Turn of the message being handled and keeps the session; `session` ends a work session. */
  stop(agentDid: string, sid: string, scope: 'turn' | 'session'): Promise<void>
}

export const AgentControlsContext = createContext<AgentSessionControls | null>(null)
