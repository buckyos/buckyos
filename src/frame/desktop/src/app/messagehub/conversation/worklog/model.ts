import type { TaskWatchDetail } from '../../../../api/task_mgr'

export interface WorklogTarget { agentDid: string; sessionId: string; turn: number }
export interface WorklogAction { call_id: string; tool: string; args?: unknown }
export interface WorklogEntry {
  seq: number
  t: string
  turn: number
  run_id?: string
  at_ms?: number
  content?: string
  assistant?: string
  tool_calls?: WorklogAction[]
  actions?: WorklogAction[]
  call_id?: string
  status?: string
  result?: string
  kind?: string
  report?: string
}
export interface WorklogChild { created_by_call: string; session_id: string; name: string }
export interface WorklogPage {
  agent_did: string
  session_id: string
  turn: number
  entries: WorklogEntry[]
  next_before: number | null
  next_after: number
  committed: number
  complete: boolean
  task_id?: string | null
  children: WorklogChild[]
}
export interface WorklogRow {
  id: string
  kind: 'tool' | 'user' | 'assistant' | 'event'
  label: string
  text?: string
  input?: string
  output?: string
  status?: string
  at?: number
  child?: WorklogChild
  taskId?: string
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
}

export function taskWorklogTarget(task: TaskWatchDetail): WorklogTarget | null {
  if (task.schemaId !== 'opendan.agent_turn/v1') return null
  const input = record(task.input)
  return typeof input.agent_did === 'string' && typeof input.session_id === 'string' && Number.isSafeInteger(input.turn) && Number(input.turn) > 0
    ? { agentDid: input.agent_did, sessionId: input.session_id, turn: Number(input.turn) } : null
}

export function worklogText(value: unknown): string {
  return typeof value === 'string' ? value : value == null ? '' : JSON.stringify(value, null, 2)
}

function resultTaskId(result?: string): string | undefined {
  if (!result) return undefined
  try {
    const value = record(JSON.parse(result))
    const id = value.task_id
    return typeof id === 'string' && id && !id.startsWith('session:') ? id : undefined
  } catch { return undefined }
}

export function worklogRows(entries: WorklogEntry[], children: WorklogChild[], turn: number): WorklogRow[] {
  const ordered = [...new Map(entries.filter(e => e.turn === turn).map(e => [e.seq, e])).values()].sort((a, b) => a.seq - b.seq)
  const results = new Map<string, WorklogEntry>()
  const calls = new Set<string>()
  const callKey = (run: string | undefined, call: string | undefined) => `${run ?? ''}/${call ?? ''}`
  const childByCall = new Map(children.map(child => [child.created_by_call, child]))
  for (const entry of ordered) {
    if (entry.t === 'action_result') results.set(callKey(entry.run_id, entry.call_id), entry)
    for (const action of entry.tool_calls ?? entry.actions ?? []) calls.add(callKey(entry.run_id, action.call_id))
  }
  const rows: WorklogRow[] = []
  for (const entry of ordered) {
    const id = String(entry.seq)
    if (entry.t === 'user_message') rows.push({ id, kind: 'user', label: 'user', text: entry.content })
    if (entry.t === 'step' || entry.t === 'assistant_message') {
      if (entry.assistant) rows.push({ id, kind: 'assistant', label: 'assistant', text: entry.assistant })
      for (const action of entry.tool_calls ?? entry.actions ?? []) {
        const key = callKey(entry.run_id, action.call_id)
        const result = results.get(key)
        const args = record(action.args)
        rows.push({ id: `${id}/${action.call_id}`, kind: 'tool', label: action.tool, text: typeof args.description === 'string' ? args.description : undefined, input: worklogText(args.command ?? args.cmd ?? action.args), output: result?.result, status: result?.status ?? 'pending', child: childByCall.get(key), taskId: resultTaskId(result?.result) })
      }
    }
    if (entry.t === 'action_result' && !calls.has(callKey(entry.run_id, entry.call_id))) {
      rows.push({ id, kind: 'tool', label: 'result', output: entry.result, status: entry.status, taskId: resultTaskId(entry.result) })
    }
    if (entry.t === 'turn_started' || entry.t === 'turn_ended') rows.push({ id, kind: 'event', label: entry.t, status: entry.status, at: entry.at_ms })
    if (entry.t === 'outcome' && (entry.report || entry.kind === 'error' || entry.kind === 'budget')) rows.push({ id, kind: 'event', label: entry.kind ?? 'outcome', text: entry.report })
  }
  return rows
}
