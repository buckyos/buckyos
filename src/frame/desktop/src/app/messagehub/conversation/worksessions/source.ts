import { agentServiceClient } from '../worklog/source'
import { parseWorkCards, type WorkCards } from './model'

/** What a bubble asks for: the work sessions its UI Turn (`taskId`, a Turn of `sessionId`) created. */
export interface WorkCardsRequest { agentDid: string; sessionId: string; taskId: string }
export type WorkCardsSource = (request: WorkCardsRequest) => Promise<WorkCards>

/** `session.work_cards {sid, task_id}` on the Agent's own service; an answer about another Agent, session or Turn is refused. */
export const workCardsSource: WorkCardsSource = async ({ agentDid, sessionId, taskId }) => {
  const client = await agentServiceClient(agentDid)
  const cards = parseWorkCards(await client.call<unknown, Record<string, unknown>>('session.work_cards', { sid: sessionId, task_id: taskId }))
  if (!cards || cards.agent_did !== agentDid || cards.session_id !== sessionId || cards.task_id !== taskId) throw Error('Work cards identity mismatch')
  return cards
}
