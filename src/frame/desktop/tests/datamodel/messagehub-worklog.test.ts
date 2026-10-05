import { taskWorklogTarget, worklogRows, type WorklogEntry } from '../../src/app/messagehub/conversation/worklog/model.ts'
import type { TaskWatchDetail } from '../../src/api/task_mgr.ts'

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

Deno.test('worklog binding comes from the typed turn task input', () => {
  const task = { schemaId: 'opendan.agent_turn/v1', input: { agent_did: 'did:agent:one', session_id: 's1', turn: 7 } } as TaskWatchDetail
  equal(taskWorklogTarget(task), { agentDid: 'did:agent:one', sessionId: 's1', turn: 7 })
  equal(taskWorklogTarget({ ...task, schemaId: 'other/v1' }), null)
  equal(taskWorklogTarget({ ...task, input: { turn: '7' } }), null)
})

Deno.test('worklog pairs calls across pages and run boundaries without mixing turns', () => {
  const entries: WorklogEntry[] = [
    { seq: 1, t: 'assistant_message', turn: 7, run_id: 'a', tool_calls: [{ call_id: 'call', tool: 'Bash', args: { command: 'pwd' } }] },
    { seq: 2, t: 'action_result', turn: 7, run_id: 'a', call_id: 'call', status: 'pending', result: 'working' },
    { seq: 3, t: 'action_result', turn: 7, run_id: 'a', call_id: 'call', status: 'ok', result: '/workspace' },
    { seq: 4, t: 'step', turn: 7, run_id: 'b', actions: [{ call_id: 'call', tool: 'xagent', args: { objective: 'test' } }] },
    { seq: 5, t: 'action_result', turn: 7, run_id: 'b', call_id: 'call', status: 'error', result: '{"task_id":"task-child"}' },
    { seq: 6, t: 'user_message', turn: 8, content: 'another turn' },
  ]
  const child = { created_by_call: 'b/call', session_id: 'child-session', name: 'Tests' }
  const orphan = worklogRows(entries.slice(2), [child], 7)
  equal(orphan[0].label, 'result')
  const rows = worklogRows([...entries.slice(2), ...entries, entries[0]], [child], 7)
  equal(rows.length, 2)
  equal([rows[0].input, rows[0].output, rows[0].status], ['pwd', '/workspace', 'ok'])
  equal([rows[1].child, rows[1].taskId, rows[1].status], [child, 'task-child', 'error'])
})

Deno.test('plain output containing a task id is not turned into a task link', () => {
  const rows = worklogRows([{ seq: 1, t: 'action_result', turn: 1, result: 'task_id: arbitrary-text', status: 'ok' }], [], 1)
  equal(rows[0].taskId, undefined)
})
