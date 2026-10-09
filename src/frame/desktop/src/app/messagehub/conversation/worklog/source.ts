import { buckyos } from 'buckyos'
import { fetchAgentList } from '../../../../api/user_mgr'
import type { WorklogPage, WorklogTarget } from './model'

export type WorklogCursor = { before?: number; after?: number }
export type WorklogSource = (cursor?: WorklogCursor) => Promise<WorklogPage>

export function createWorklogSource(target: WorklogTarget): WorklogSource {
  let agentId: Promise<string> | undefined
  const resolve = async () => {
    const { data, error } = await fetchAgentList()
    if (error || !data) throw error ?? Error('Agent directory unavailable')
    const agent = data.agents.find(agent => [agent.agent_did, agent.did, agent.id].includes(target.agentDid))
    if (!agent || !/^[a-zA-Z0-9_-]+(?:\.[a-zA-Z0-9_-]+)*$/.test(agent.agent_id)) throw Error('Agent service unavailable')
    return agent.agent_id
  }
  return async (cursor = {}) => {
    agentId ??= resolve().catch(error => { agentId = undefined; throw error })
    const client = buckyos.getServiceRpcClient(await agentId)
    const page = await client.call<WorklogPage, Record<string, unknown>>('session.worklog', { sid: target.sessionId, turn: target.turn, limit: 50, ...cursor })
    if (page.agent_did !== target.agentDid || page.session_id !== target.sessionId || page.turn !== target.turn) throw Error('Worklog identity mismatch')
    return page
  }
}
