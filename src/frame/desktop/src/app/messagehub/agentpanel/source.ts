import { agentServiceClient } from '../conversation/worklog/source'
import { parseWorkTree, type WorkTree } from '../conversation/worksessions/model'
import { mockAgentPanelSource } from '../mock/agentpanel'
import { isMessageHubMock } from '../store'
import { parseAgentSummary, parseDeliveries, parseUiConversation, type AgentSummary, type DeliveryPage, type UiConversation } from './model'

export type StopScope = 'turn' | 'session'

/**
 * The owner's view of one of their Agents, read from and operated on the
 * Agent's own OpenDAN service with the viewer's permissions: only the
 * Agent's owner gets an answer.
 */
export interface AgentPanelSource {
  /** `home.summary`. */
  summary(agentDid: string): Promise<AgentSummary>
  /** `ui.conversation {peer_did, session_id}`: the UI Session generations of the conversation with `peerDid` in the MessageHub session `sessionId`. */
  conversation(agentDid: string, peerDid: string, sessionId: string): Promise<UiConversation>
  /** `session.work_tree {sid, unfinished_only: true}`. */
  workTree(agentDid: string, sid: string): Promise<WorkTree>
  /** `deliveries.recent {limit, before?, route_key?}`, newest first. */
  deliveries(agentDid: string, query: { limit: number; before?: number; routeKey?: string }): Promise<DeliveryPage>
  /** `session.pause {sid}`: the session and its unfinished sub sessions infer no more until resumed. */
  pause(agentDid: string, sid: string): Promise<void>
  /** `session.resume {sid}`: also resumes the sub sessions paused with it. */
  resume(agentDid: string, sid: string): Promise<void>
  /** `session.stop {sid, scope}`: `turn` closes the Turn of the message being handled and keeps the session; `session` ends a work session. */
  stop(agentDid: string, sid: string, scope: StopScope): Promise<void>
  /** `ui.rotate {sid}`: archives the UI Session; the next message starts a new one. */
  rotate(agentDid: string, sid: string): Promise<void>
}

const apiAgentPanelSource: AgentPanelSource = {
  async summary(agentDid) {
    const client = await agentServiceClient(agentDid)
    const summary = parseAgentSummary(await client.call<unknown, Record<string, unknown>>('home.summary', {}))
    if (!summary) throw Error('Agent summary unavailable')
    return summary
  },
  async conversation(agentDid, peerDid, sessionId) {
    const client = await agentServiceClient(agentDid)
    const conversation = parseUiConversation(await client.call<unknown, Record<string, unknown>>('ui.conversation', { peer_did: peerDid, session_id: sessionId }))
    if (!conversation) throw Error('Agent conversation unavailable')
    return conversation
  },
  async workTree(agentDid, sid) {
    const client = await agentServiceClient(agentDid)
    const tree = parseWorkTree(await client.call<unknown, Record<string, unknown>>('session.work_tree', { sid, unfinished_only: true }))
    if (!tree || tree.agent_did !== agentDid || tree.session_id !== sid) throw Error('Work tree identity mismatch')
    return tree
  },
  async deliveries(agentDid, { limit, before, routeKey }) {
    const client = await agentServiceClient(agentDid)
    const page = parseDeliveries(await client.call<unknown, Record<string, unknown>>('deliveries.recent', { limit, ...(before !== undefined ? { before } : {}), ...(routeKey ? { route_key: routeKey } : {}) }))
    if (!page) throw Error('Deliveries unavailable')
    return page
  },
  async pause(agentDid, sid) {
    await (await agentServiceClient(agentDid)).call<unknown, Record<string, unknown>>('session.pause', { sid })
  },
  async resume(agentDid, sid) {
    await (await agentServiceClient(agentDid)).call<unknown, Record<string, unknown>>('session.resume', { sid })
  },
  async stop(agentDid, sid, scope) {
    await (await agentServiceClient(agentDid)).call<unknown, Record<string, unknown>>('session.stop', { sid, scope })
  },
  async rotate(agentDid, sid) {
    await (await agentServiceClient(agentDid)).call<unknown, Record<string, unknown>>('ui.rotate', { sid })
  },
}

export function getAgentPanelSource(): AgentPanelSource {
  return isMessageHubMock() ? mockAgentPanelSource : apiAgentPanelSource
}
