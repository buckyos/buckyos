import type { WorklogEntry, WorklogTarget } from '../conversation/worklog/model'
import type { WorklogSource } from '../conversation/worklog/source'

const sessions = new Map<string, { entries: WorklogEntry[]; complete: boolean }>()
export const mockWorklogs = {
  offline: false,
  reads: 0,
  append(sessionId: string, turn: number, entry: Omit<WorklogEntry, 'seq' | 'turn'>, complete = false) {
    const session = sessions.get(`${sessionId}/${turn}`)
    if (!session) throw Error('Mock worklog not open')
    session.entries.push({ ...entry, seq: session.entries.length + 1, turn })
    session.complete = complete
  },
}
if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __messageHubMockWorklogs: mockWorklogs })

export function mockWorklogSource(target: WorklogTarget): WorklogSource {
  const turn = target.turn
  const run_id = `run-${turn}`
  const child = target.sessionId.startsWith('work-')
  const entries: WorklogEntry[] = [
    { seq: 1, t: 'turn_started', turn, run_id, at_ms: Date.now() - 180000 },
    { seq: 2, t: 'user_message', turn, run_id, content: child ? 'Run the UI tests and report the result.' : 'Check the MessageHub release checklist.' },
  ]
  for (let i = 0; i < (child ? 2 : 30); i++) {
    const seq = entries.length + 1
    entries.push({ seq, t: 'assistant_message', turn, run_id, assistant: i === 0 ? 'I’ll inspect the implementation and verify the message flow.' : '', tool_calls: [{ call_id: `call-${i}`, tool: 'Bash', args: { command: i === 0 ? 'cat src/app/messagehub/conversation/history/relations.ts' : `pnpm exec playwright test messagehub-${i}.spec.ts`, description: i === 0 ? 'Read message projection' : `Verify message flow ${i}` } }] })
    entries.push({ seq: seq + 1, t: 'action_result', turn, run_id, call_id: `call-${i}`, status: 'ok', result: i === 0 ? 'export function foldMessageRelations(messages) {\n  return projectEdits(messages)\n}' : 'Running checks…\n' + '✓ Message projection verified\n'.repeat(8) + 'All checks passed.' })
  }
  if (!child) {
    entries.push({ seq: entries.length + 1, t: 'step', turn, run_id, assistant: 'I’m starting a work session to run the complete UI suite.', actions: [{ call_id: 'create-tests', tool: 'xagent', args: { objective: 'Run UI tests' } }] })
    entries.push({ seq: entries.length + 1, t: 'action_result', turn, run_id, call_id: 'create-tests', status: 'ok', result: JSON.stringify({ session_id: 'work-tests', status: 'created' }) })
  }
  const complete = child || turn !== 16
  if (complete) entries.push({ seq: entries.length + 1, t: 'turn_ended', turn, run_id, status: 'completed', at_ms: Date.now() })
  const key = `${target.sessionId}/${turn}`
  if (!sessions.has(key)) sessions.set(key, { entries, complete })
  return async (cursor = {}) => {
    mockWorklogs.reads++
    await new Promise(resolve => setTimeout(resolve, 30))
    if (mockWorklogs.offline) throw Error('Mock worklog offline')
    const { entries, complete } = sessions.get(key)!
    const end = cursor.before ?? entries.length
    const start = cursor.after ?? Math.max(0, end - 50)
    const slice = entries.slice(start, cursor.after == null ? end : start + 50)
    return {
      agent_did: target.agentDid, session_id: target.sessionId, turn,
      entries: slice, next_before: cursor.after == null && start > 0 ? start : null,
      next_after: cursor.after == null ? entries.length : start + slice.length,
      committed: entries.length, complete,
      task_id: child ? 'task-turn-checklist.tests' : undefined,
      children: child ? [] : [{ created_by_call: `${run_id}/create-tests`, session_id: 'work-tests', name: 'Run UI tests' }],
    }
  }
}
