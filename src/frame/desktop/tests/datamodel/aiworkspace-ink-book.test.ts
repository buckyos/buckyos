import { InkBook } from '../../src/app/aiworkspace/ink/book.ts'
import * as core from '../../src/app/aiworkspace/wasm/aiworkspace_wasm.js'
import type { CommitOutcome, EntityEnvelope, Operation } from '../../src/app/aiworkspace/api/types.ts'
import type { WorkspaceStore, SubmitOptions } from '../../src/app/aiworkspace/state/store.ts'

core.initSync({ module: Deno.readFileSync(new URL('../../src/app/aiworkspace/wasm/aiworkspace_wasm_bg.wasm', import.meta.url)) })
Object.defineProperty(globalThis, 'window', { value: { setTimeout: () => 0, clearTimeout: () => {} }, configurable: true })
const tick = () => new Promise((resolve) => setTimeout(resolve, 0))
function equal(a: unknown, b: unknown) { if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${JSON.stringify(a)} !== ${JSON.stringify(b)}`) }
function ok(a: unknown, message: string) { if (!a) throw new Error(message) }
const accepted = { status: 'accepted', seq: 10, commit_id: 'commit', idempotency_key: 'key' } as CommitOutcome
const rejected = { status: 'rejected', code: 'INVALID_OPERATION', errors: [{ code: 'INVALID_OPERATION', detail: 'refused detail' }], retryable: false, idempotency_key: 'key' } as CommitOutcome
const conflict = { status: 'conflict', code: 'REVISION_CONFLICT', conflicts: [], retryable: true, idempotency_key: 'key' } as CommitOutcome
const stroke = [{ x: 1, y: 1, t: 0 }, { x: 3, y: 3, t: 1 }]
const style = { color: '#111111', width: 2, pressure: false }

function harness(keys: string[] = []) {
  const entries = new Map<string, any>()
  const unsaved = new Map<string, unknown>()
  const listeners = new Set<() => void>()
  const subscribers = new Set<(event: any) => void>()
  const notices: { action?: { run: () => void } }[] = []
  const children = keys.map((order_key, i) => ({ entity_id: `existing${i}`, parent_id: 'surface', order_key, struct_rev: i + 1 })) as EntityEnvelope[]
  const requests: { options: SubmitOptions; finish: (outcome: CommitOutcome, apply?: boolean) => void }[] = []
  let reloads = 0
  const apply = (operations: Operation[]) => {
    for (const op of operations) {
      if (op.op === 'tree.place') {
        const child = children.find((c) => c.entity_id === op.entity_id)!
        child.order_key = String(op.order_key)
        child.struct_rev = 10
      } else if (op.op === 'entity.create') children.push({ entity_id: String(op.entity_id), parent_id: String(op.parent_id), order_key: String(op.order_key), struct_rev: 10 } as EntityEnvelope)
    }
  }
  const store = {
    core,
    edits: { subscribe: (fn: () => void) => listeners.add(fn), get: (id: string) => entries.get(id), set: (entry: any) => { entries.set(entry.id, entry); listeners.forEach((fn) => fn()) } },
    session: { subscribeStatus: () => {}, offline: { subscribeSubmissions: (fn: (event: any) => void) => subscribers.add(fn) } },
    outline: { childrenOf: () => [...children].sort((a, b) => a.order_key! < b.order_key! ? -1 : 1), get: (id: string) => children.find((c) => c.entity_id === id), reload: async () => { reloads++ } },
    noteUnsaved: (id: string, input: unknown) => { if (input) unsaved.set(id, input); else unsaved.delete(id) },
    notify: (_kind: string, _text: string, action?: { run: () => void }) => notices.push({ action }),
    submit: (options: SubmitOptions) => new Promise<CommitOutcome>((resolve) => {
      entries.set(options.editId, { id: options.editId, state: 'unsaved' })
      requests.push({ options, finish: (outcome, shouldApply = outcome.status === 'accepted') => {
        if (shouldApply) apply(options.operations)
        resolve(outcome)
      } })
    }),
  } as unknown as WorkspaceStore
  const book = new InkBook(store)
  return {
    book, store, requests, children, unsaved, notices, entries, apply, get reloads() { return reloads },
    write: () => book.write(stroke, style, 1, 'surface', new Map()),
    dismiss: (id: string) => { entries.delete(id); listeners.forEach((fn) => fn()) },
    event: (event: any) => subscribers.forEach((fn) => fn(event)),
  }
}

Deno.test('500 fast strokes use actual WASM keys and reserve the order of in-flight strokes', () => {
  const h = harness()
  for (let i = 0; i < 500; i++) h.write()
  equal(h.requests.length, 500)
  const keys = h.requests.map((r) => String(r.options.operations[0].order_key))
  equal(new Set(keys).size, 500)
  equal([...keys].sort(), keys)
  ok(keys.every((key) => key.length <= 8), 'sequential writing should stay compact')
})

Deno.test('64z starts non-undoable maintenance with membership and revision guards; all strokes wait for acceptance', async () => {
  const h = harness(['a', 'z'.repeat(64)])
  h.write(); h.write(); h.write()
  await tick()
  equal(h.requests.length, 1)
  const maintenance = h.requests[0].options
  equal(maintenance.undoable, false)
  equal(maintenance.operations.map((op) => op.op), ['tree.place', 'tree.place'])
  equal(maintenance.operations.map((op) => op.expect), [{ rev: 1 }, { rev: 2 }])
  ok(maintenance.preconditions?.length === 1, 'membership guard')
  equal(h.unsaved.size, 3)
  h.requests[0].finish(accepted)
  await tick()
  equal(h.requests.length, 4)
  ok(h.requests.slice(1).every((r) => r.options.undoable && r.options.operations.every((op) => op.op === 'entity.create')), 'only handwriting enters undo history')
  const keys = h.requests.slice(1).map((r) => r.options.operations[0].order_key as string)
  equal([...keys].sort(), keys)
  ok(h.children[1].order_key! < keys[0], 'existing stacking stays before new writing')
})

Deno.test('rejected reservations are released, input stays exportable, dismissal does not disturb another stroke', async () => {
  const h = harness(['z'.repeat(63) + 'y'])
  const first = h.write()!
  h.write()
  equal(h.requests.length, 1)
  h.requests[0].finish(rejected)
  await tick()
  equal(h.requests.length, 2)
  equal(h.requests[1].options.operations[0].order_key, 'z'.repeat(64))
  equal(h.entries.get(`ink:${first}`).detail, 'refused detail')
  ok(h.unsaved.has(`ink:${first}`), 'refused stroke is exportable')
  h.dismiss(`ink:${first}`)
  equal(h.book.waitingOn('surface').length, 1)
  ok(!h.unsaved.has(`ink:${first}`), 'dismiss drops only the discarded input')
})

Deno.test('unknown and failed storage retry the identical immutable request and unblock maintenance only after success', async () => {
  for (const status of ['unknown', 'storage_failed'] as const) {
    const h = harness(['z'.repeat(64)])
    h.write(); h.write()
    await tick()
    const original = h.requests[0].options
    h.requests[0].finish({ status, detail: 'unavailable', idempotency_key: '' })
    await tick()
    equal(h.requests.length, 1)
    h.book.retryUnknown()
    equal(h.requests.length, 2)
    equal(h.requests[1].options, original)
    h.requests[1].finish(accepted)
    await tick()
    equal(h.requests.length, 4)
  }
})

Deno.test('saved_locally reserves keys while outline catches up and later refusal restores exportable input', async () => {
  const h = harness()
  const first = h.write()!
  const key = h.requests[0].options.idempotencyKey!
  h.requests[0].finish({ status: 'saved_locally', idempotency_key: key, provisional_seq: 1 }, false)
  await tick()
  h.write()
  ok(String(h.requests[0].options.operations[0].order_key) < String(h.requests[1].options.operations[0].order_key), 'stale outline cannot reuse a local key')
  h.event({ key, state: 'rejected', result: rejected })
  await tick()
  ok(h.unsaved.has(`ink:${first}`), 'offline refusal retains original stroke')
  equal(h.entries.get(`ink:${first}`).state, 'needs_attention')
})

Deno.test('offline maintenance keeps the barrier until server acceptance, even after the local outline updates', async () => {
  const h = harness(['z'.repeat(64)])
  h.write(); h.write()
  await tick()
  const request = h.requests[0]
  const key = request.options.idempotencyKey!
  request.finish({ status: 'saved_locally', idempotency_key: key, provisional_seq: 1 }, true)
  await tick()
  h.write()
  equal(h.requests.length, 1)
  equal(h.unsaved.size, 3)
  h.event({ key, state: 'accepted' })
  await tick()
  equal(h.requests.length, 4)
})

Deno.test('maintenance conflicts refresh guarded order with a new request key; repeated conflict remains recoverable', async () => {
  const h = harness(['z'.repeat(64)])
  const first = h.write()!
  await tick()
  for (let i = 0; i < 3; i++) {
    h.requests[i].finish(conflict)
    await tick()
  }
  equal(h.requests.length, 3)
  equal(new Set(h.requests.map((r) => r.options.idempotencyKey)).size, 3)
  equal(h.entries.get(`ink:${first}`).state, 'needs_attention')
  ok(h.unsaved.has(`ink:${first}`), 'exhausted retry budget preserves input')
  h.notices.at(-1)!.action!.run()
  await tick()
  equal(h.requests.length, 4)
})

Deno.test('offline terminal events arriving before saved_locally cannot strand the maintenance barrier', async () => {
  for (const state of ['accepted', 'rejected', 'discarded'] as const) {
    const h = harness(['z'.repeat(64)])
    const first = h.write()!
    await tick()
    const req = h.requests[0]
    const key = req.options.idempotencyKey!
    if (state === 'accepted') h.apply(req.options.operations)
    h.event({ key, state, result: rejected })
    req.finish({ status: 'saved_locally', idempotency_key: key, provisional_seq: 1 }, false)
    await tick()
    if (state === 'accepted') equal(h.requests.length, 2)
    if (state === 'rejected') equal(h.entries.get(`ink:${first}`).state, 'needs_attention')
    if (state === 'discarded') equal(h.book.waitingOn('surface').length, 0)
  }
})

Deno.test('offline refusal retains the pending key for discard and retry cleanup', async () => {
  const h = harness()
  const first = h.write()!
  const key = h.requests[0].options.idempotencyKey!
  h.requests[0].finish({ status: 'saved_locally', idempotency_key: key, provisional_seq: 1 }, false)
  await tick()
  h.entries.set(`ink:${first}`, { id: `ink:${first}`, state: 'needs_attention', pendingKey: key })
  h.event({ key, state: 'rejected', result: rejected })
  await tick()
  equal(h.entries.get(`ink:${first}`).pendingKey, key)
  h.notices.at(-1)!.action!.run()
  equal(h.requests.length, 2)
})
