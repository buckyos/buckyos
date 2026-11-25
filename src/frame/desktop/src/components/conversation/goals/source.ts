import { isMockRuntime } from '../../../runtime'
import { mockGoalSource } from '../mock/goals'
import { agentServiceClient } from '../worklog/source'
import type { WorklogEntry, WorklogPage } from '../worklog/model'
import { parseWorkTree, type WorkTree } from '../worksessions/model'
import { parseActivations, parseGoal, parseGoals, parseGoalSchedule, parseGoalState, parseOpenActivation, type Goal, type GoalActivation, type GoalSchedule, type GoalState, type OpenActivation } from './model'

/** The Goals of one of the viewer's own Agents, on the Agent's OpenDAN service with the viewer's permissions. */
export interface GoalSource {
  /** `goals.list`. */
  list(agentDid: string): Promise<Goal[]>
  /** `goals.get`; null: there is no such Goal. */
  get(agentDid: string, goalId: string): Promise<Goal | null>
  /** `goals.state`; null before its first session. */
  state(agentDid: string, goalId: string): Promise<GoalState | null>
  /** `goals.schedule`; null before the host looked at it. */
  schedule(agentDid: string, goalId: string): Promise<GoalSchedule | null>
  /** `goals.activations`: the newest `limit`, oldest first. */
  activations(agentDid: string, goalId: string, limit: number): Promise<GoalActivation[]>
  /** The Activation in progress in a Goal Session (`session.read`). */
  open(agentDid: string, sid: string): Promise<OpenActivation | null>
  /** `session.work_tree {sid}`: the Work of a Goal Session. */
  works(agentDid: string, sid: string): Promise<WorkTree>
  /** The first records of a Turn of a Goal Session (`session.worklog`). */
  opening(agentDid: string, sid: string, turn: number): Promise<WorklogEntry[]>
  /** `goals.post`: becomes a `feedback` Activation; resolves to its input key. */
  post(agentDid: string, goalId: string, text: string): Promise<string>
}

const OPENING_RECORDS = 8
/** Pages of a log read to find where a Turn begins (each covers up to 2000 records). */
const OPENING_PAGES = 12

async function call(agentDid: string, method: string, params: Record<string, unknown>): Promise<unknown> {
  return (await agentServiceClient(agentDid)).call<unknown, Record<string, unknown>>(method, params)
}

function parsed<T>(value: T | null, what: string): T {
  if (value === null) throw Error(`${what} unavailable`)
  return value
}

const apiGoalSource: GoalSource = {
  list: async agentDid => parsed(parseGoals(await call(agentDid, 'goals.list', {})), 'Goals'),
  get: async (agentDid, goalId) => parseGoal(await call(agentDid, 'goals.get', { goal_id: goalId })),
  state: async (agentDid, goalId) => parseGoalState(await call(agentDid, 'goals.state', { goal_id: goalId })),
  schedule: async (agentDid, goalId) => parseGoalSchedule(await call(agentDid, 'goals.schedule', { goal_id: goalId })),
  activations: async (agentDid, goalId, limit) => parsed(parseActivations(await call(agentDid, 'goals.activations', { goal_id: goalId, query: { limit } })), 'Goal activations'),
  open: async (agentDid, sid) => parseOpenActivation(await call(agentDid, 'session.read', { sid, worklog: 0, report: false })),
  async works(agentDid, sid) {
    const tree = parseWorkTree(await call(agentDid, 'session.work_tree', { sid }))
    if (!tree || tree.agent_did !== agentDid || tree.session_id !== sid) throw Error('Work tree identity mismatch')
    return tree
  },
  async opening(agentDid, sid, turn) {
    const entries: WorklogEntry[] = []
    let after = 0
    for (let index = 0; index < OPENING_PAGES && entries.length < OPENING_RECORDS; index++) {
      const page = await call(agentDid, 'session.worklog', { sid, turn, after, limit: OPENING_RECORDS - entries.length }) as WorklogPage
      if (page.agent_did !== agentDid || page.session_id !== sid || page.turn !== turn) throw Error('Worklog identity mismatch')
      entries.push(...page.entries)
      if (page.next_after >= page.committed) break
      after = page.next_after
    }
    return entries
  },
  async post(agentDid, goalId, text) {
    const answer = await call(agentDid, 'goals.post', { goal_id: goalId, text }) as { key?: unknown } | null
    if (typeof answer?.key !== 'string') throw Error('result_unknown')
    return answer.key
  },
}

export function getGoalSource(): GoalSource {
  return isMockRuntime() ? mockGoalSource : apiGoalSource
}
