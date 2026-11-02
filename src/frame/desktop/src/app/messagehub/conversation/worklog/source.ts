import { buckyos } from 'buckyos'
import { fetchAgentList } from '../../../../api/user_mgr'
import type { WorklogPage, WorklogTarget, WorkSessionInfo } from './model'

export type WorklogCursor = { before?: number; after?: number }
export type WorklogSource = (cursor?: WorklogCursor) => Promise<WorklogPage>

const agentIds = new Map<string, Promise<string>>()

/** The Agent's OpenDAN service (`/kapi/<agent_id>`), its AgentId resolved once per DID through `agent.list`; calls use the viewer's own permissions. */
export async function agentServiceClient(agentDid: string) {
  let agentId = agentIds.get(agentDid)
  if (!agentId) {
    agentId = (async () => {
      const { data, error } = await fetchAgentList()
      if (error || !data) throw error ?? Error('Agent directory unavailable')
      const agent = data.agents.find(agent => agent.agent_did === agentDid)
      if (!agent || !/^[a-zA-Z0-9_-]+(?:\.[a-zA-Z0-9_-]+)*$/.test(agent.agent_id)) throw Error('Agent service unavailable')
      return agent.agent_id
    })()
    agentIds.set(agentDid, agentId)
    agentId.catch(() => { if (agentIds.get(agentDid) === agentId) agentIds.delete(agentDid) })
  }
  return buckyos.getServiceRpcClient(await agentId)
}

export function createWorklogSource(target: WorklogTarget): WorklogSource {
  return async (cursor = {}) => {
    const client = await agentServiceClient(target.agentDid)
    const page = await client.call<WorklogPage, Record<string, unknown>>('session.worklog', { sid: target.sessionId, ...(target.turn ? { turn: target.turn } : {}), limit: 50, ...cursor })
    if (page.agent_did !== target.agentDid || page.session_id !== target.sessionId || (page.turn ?? undefined) !== target.turn) throw Error('Worklog identity mismatch')
    return page
  }
}

/** `session.read {sid, worklog: 0, report: true}`: what the session is about, for the head of its work log. */
export async function readWorkSession(agentDid: string, sessionId: string): Promise<WorkSessionInfo> {
  const client = await agentServiceClient(agentDid)
  const view = await client.call<WorkSessionInfo, Record<string, unknown>>('session.read', { sid: sessionId, worklog: 0, report: true })
  if (view?.entry?.session_id !== sessionId) throw Error('Session identity mismatch')
  return view
}
