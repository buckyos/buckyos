import { cardUsage, displayModel, formatDuration, formatTokens, localizeLine, parseWorkCards, todoTitle, visibleTodos, workDuration, workTone, type WorkCard, type WorkCards, type WorkTodo } from '../../src/app/messagehub/conversation/worksessions/model.ts'
import { WorkCardsWatch } from '../../src/app/messagehub/conversation/worksessions/watch.ts'
import type { WorkCardsRequest } from '../../src/app/messagehub/conversation/worksessions/source.ts'
import { worklogRows, type WorklogEntry } from '../../src/app/messagehub/conversation/worklog/model.ts'

function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
async function until(check: () => boolean, label: string) {
  for (let attempt = 0; attempt < 400; attempt++) { if (check()) return; await sleep(5) }
  throw Error(`Timed out waiting for ${label}`)
}

const usage = (input: number, output: number, rounds: number) => ({ input_tokens: input, output_tokens: output, rounds, models: rounds ? [{ model: 'gpt-6.1-sol@cliproxy', input_tokens: input, output_tokens: output }] : [] })
function card(seed: Partial<WorkCard> = {}): WorkCard {
  return { session_id: 'work-1', shape: 'simple', title: 'Build', created_at_ms: 1_000, finished_at_ms: null, state: 'running', line: '', usage: usage(0, 0, 0), model: null, subtree_usage: null, todos: null, ...seed }
}
const todo = (seq: number, status: string, extra: Partial<WorkTodo> = {}): WorkTodo => ({ todo_id: `T${String(seq).padStart(2, '0')}`, seq, title: `Step ${seq}`, status, mode: 'serial', attempt: 1, blocked_by: [], block_reason: null, card: null, ...extra })

Deno.test('model names drop the provider suffix; request aliases and empty names are not shown', () => {
  equal([displayModel('gpt-6.1-sol@cliproxy'), displayModel('gpt-6.1-sol'), displayModel('gpt-6.1-sol:reasoning-medium@codex-proxy'), displayModel('llm.code'), displayModel('llm.code@aicc'), displayModel(''), displayModel(null)], ['gpt-6.1-sol', 'gpt-6.1-sol', 'gpt-6.1-sol', undefined, undefined, undefined, undefined])
})

Deno.test('durations and token counts use the card formats', () => {
  equal([0, 45_000, 192_000, 3_900_000, 183_600_000].map(formatDuration), ['0s', '45s', '3m12s', '1h05m', '2d3h'])
  equal([0, 999, 1000, 9_100, 86_400, 999_949, 999_950, 1_250_000].map(formatTokens), ['0', '999', '1k', '9.1k', '86.4k', '999.9k', '1M', '1.3M'])
  equal([workDuration({ created_at_ms: 1_000, finished_at_ms: null }, 61_000), workDuration({ created_at_ms: 1_000, finished_at_ms: 31_000 }, 999_000)], [60_000, 30_000])
})

Deno.test('known runtime lines are localized, the model\'s own words are kept, empty lines take the state default', () => {
  equal(localizeLine('running shell', 'running'), { key: 'messagehub.work.line.running', vars: { tool: 'shell' } })
  equal(localizeLine('working (work_simple)', 'running'), { key: 'messagehub.work.line.working' })
  equal(localizeLine('working', 'running'), { key: 'messagehub.work.line.working' })
  equal(localizeLine('waiting for 2 task(s)', 'running'), { key: 'messagehub.work.line.waitingTasks', vars: { count: 2 } })
  equal(localizeLine('waiting for 1 sub session(s)', 'running'), { key: 'messagehub.work.line.waitingChildren', vars: { count: 1 } })
  equal(localizeLine('Editing §3 of the design doc', 'running'), 'Editing §3 of the design doc')
  equal(localizeLine('running the two builds', 'running'), 'running the two builds')
  equal([localizeLine('', 'queued'), localizeLine(' ', 'needs_input'), localizeLine('', 'paused'), localizeLine('', 'stopped'), localizeLine('', 'succeeded')], [{ key: 'messagehub.work.line.queued' }, { key: 'messagehub.work.line.needsInput' }, { key: 'messagehub.work.line.paused' }, { key: 'messagehub.work.line.stopped' }, ''])
  equal((['queued', 'running', 'needs_input', 'paused', 'succeeded', 'failed', 'stopped'] as const).map(workTone), ['waiting', 'active', 'waiting', 'paused', 'success', 'failed', 'canceled'])
})

Deno.test('answers are validated: unknown states drop the card, todos are ordered, nested cards carry no todos', () => {
  const raw = {
    agent_did: 'did:agent:a', session_id: 'ui', task_id: 't', complete: false,
    sessions: [
      { ...card({ session_id: 'later', created_at_ms: 9 }) },
      { ...card({ session_id: 'bad' }), state: 'exploded' },
      { ...card({ session_id: 'plan', shape: 'plan', created_at_ms: 2 }), todos: { rev: 3, total: 2, done: 1, items: [todo(2, 'pending'), todo(1, 'completed', { card: { ...card({ session_id: 'child' }), todos: { rev: 1, total: 0, done: 0, items: [] } } as WorkCard })] } },
    ],
  }
  const parsed = parseWorkCards(raw)!
  equal(parsed.sessions.map(item => item.session_id), ['plan', 'later'])
  equal(parsed.sessions[0].todos!.items.map(item => item.todo_id), ['T01', 'T02'])
  equal(parsed.sessions[0].todos!.items[0].card?.todos, null)
  equal(parseWorkCards({ ...raw, sessions: 'x' }), null)
  equal(parseWorkCards(null), null)
})

Deno.test('a plan card counts itself and its Todos; Todo titles carry the id and the retry', () => {
  const plan = card({ shape: 'plan', usage: usage(10, 1, 1), subtree_usage: usage(30, 5, 4) })
  equal([cardUsage(plan).output_tokens, cardUsage(card({ usage: usage(7, 2, 1) })).output_tokens], [5, 2])
  equal([todoTitle(todo(2, 'running')), todoTitle(todo(3, 'running', { attempt: 2 }))], ['T02 Step 2', 'T03 Step 3 #2'])
})

Deno.test('long TodoLists keep the open Todos and the three that ended last', () => {
  const items = Array.from({ length: 14 }, (_, index) => todo(index + 1, index < 10 ? 'completed' : index === 10 ? 'running' : 'pending', index < 10 ? { card: card({ finished_at_ms: 100 + (index === 2 ? 500 : index) }) } : {}))
  const shown = visibleTodos(items, false)
  equal([shown.items.map(item => item.seq), shown.hidden], [[3, 9, 10, 11, 12, 13, 14], 7])
  equal(visibleTodos(items, true).hidden, 0)
  equal(visibleTodos(items.slice(0, 12), false).items.length, 12)
})

Deno.test('a whole-session worklog keeps every Turn and records outside a Turn', () => {
  const entries: WorklogEntry[] = [
    { seq: 1, t: 'turn_started', turn: 1, at_ms: 1 },
    { seq: 2, t: 'assistant_message', turn: 1, assistant: 'first' },
    { seq: 3, t: 'turn_ended', turn: 1, status: 'completed' },
    { seq: 4, t: 'outcome', kind: 'error', report: 'outside a turn' },
    { seq: 5, t: 'turn_started', turn: 2 },
    { seq: 6, t: 'assistant_message', turn: 2, assistant: 'second' },
  ]
  equal(worklogRows(entries, []).map(row => row.label), ['turn_started', 'assistant', 'turn_ended', 'error', 'turn_started', 'assistant'])
  equal(worklogRows(entries, [], 2).map(row => row.text ?? row.label), ['turn_started', 'second'])
})

class FakeHints {
  listeners = new Map<string, Set<() => void>>()
  subscribe(_scope: 'tree', id: string, onHint: () => void) {
    const set = this.listeners.get(id) ?? new Set()
    this.listeners.set(id, set)
    set.add(onHint)
    return () => { set.delete(onHint); if (set.size === 0) this.listeners.delete(id) }
  }
  emit(id: string) { this.listeners.get(id)?.forEach(listener => listener()) }
}

const REQUEST: WorkCardsRequest = { agentDid: 'did:agent:a', sessionId: 'ui', taskId: 'turn-1' }
const page = (sessions: WorkCard[], complete = false): WorkCards => ({ agent_did: 'did:agent:a', session_id: 'ui', task_id: 'turn-1', complete, sessions })

Deno.test('tree hints are debounced into one read; complete cards stop hints and polls and later mounts use the cache', async () => {
  const hints = new FakeHints()
  let reads = 0
  let answer = page([card()])
  const watch = new WorkCardsWatch(async () => { reads++; return structuredClone(answer) }, hints, { hintDebounceMs: 30, minIntervalMs: 60, pollMs: 10_000 })
  const release = watch.watch(REQUEST)
  await until(() => watch.snapshot(REQUEST)?.status === 'ready', 'first read')
  equal(reads, 1)
  for (let i = 0; i < 5; i++) hints.emit('turn-1')
  await sleep(120)
  equal(reads, 2)
  answer = page([card({ state: 'succeeded', finished_at_ms: 2_000 })], true)
  hints.emit('turn-1')
  await until(() => watch.snapshot(REQUEST)?.settled === true, 'complete')
  equal([reads, watch.isLive(REQUEST), hints.listeners.size], [3, false, 0])
  hints.emit('turn-1')
  await sleep(80)
  equal(reads, 3)
  release()
  const again = watch.watch(REQUEST)
  await sleep(50)
  equal([reads, watch.snapshot(REQUEST)?.cards?.sessions[0].state], [3, 'succeeded'])
  again()
  watch.dispose()
})

Deno.test('polls only while cards change; failures keep the last cards as stale; a Turn without work sessions stops once it ended', async () => {
  const hints = new FakeHints()
  let reads = 0
  let fail = false
  let answer = page([card()])
  const watch = new WorkCardsWatch(async () => { reads++; if (fail) throw Error('offline'); return structuredClone(answer) }, hints, { hintDebounceMs: 10, minIntervalMs: 10, pollMs: 40 })
  const release = watch.watch(REQUEST)
  await until(() => reads >= 3, 'polls')
  fail = true
  await until(() => watch.snapshot(REQUEST)?.stale === true, 'stale')
  equal(watch.snapshot(REQUEST)?.cards?.sessions.length, 1)
  fail = false
  await until(() => watch.snapshot(REQUEST)?.stale === false, 'recovered')
  // No work session (yet): only hints read, nothing is polled; the ended Turn is not followed.
  answer = page([])
  hints.emit('turn-1')
  await until(() => watch.snapshot(REQUEST)?.cards?.sessions.length === 0, 'empty')
  const before = reads
  await sleep(120)
  equal(reads, before)
  watch.setRootTerminal(REQUEST, true)
  equal([watch.snapshot(REQUEST)?.settled, hints.listeners.size], [true, 0])
  release()
  watch.dispose()
})

Deno.test('a read that never succeeded leaves the bubble on its task area and is given up after a few failures', async () => {
  const hints = new FakeHints()
  let reads = 0
  const watch = new WorkCardsWatch(async () => { reads++; throw Error('denied') }, hints, { hintDebounceMs: 5, minIntervalMs: 5, pollMs: 20, maxFailures: 3, retryMs: 1_000 })
  const release = watch.watch(REQUEST)
  await until(() => watch.snapshot(REQUEST)?.status === 'unavailable', 'unavailable')
  await sleep(60)
  equal(reads, 1)
  hints.emit('turn-1')
  await until(() => reads === 2, 'hint read')
  hints.emit('turn-1')
  await until(() => watch.snapshot(REQUEST)?.settled === true, 'given up')
  equal([reads, hints.listeners.size], [3, 0])
  release()
  watch.dispose()
})

Deno.test('a first read that failed is retried without hints (a finished Turn sends none)', async () => {
  const hints = new FakeHints()
  let reads = 0
  const watch = new WorkCardsWatch(async () => {
    reads++
    if (reads === 1) throw Error('service_not_ready')
    return page([card({ state: 'succeeded' })], true)
  }, hints, { retryMs: 20 })
  const release = watch.watch(REQUEST, true)
  await until(() => watch.snapshot(REQUEST)?.status === 'ready', 'retried')
  equal([reads, watch.snapshot(REQUEST)?.settled, hints.listeners.size], [2, true, 0])
  release()
  watch.dispose()
})
