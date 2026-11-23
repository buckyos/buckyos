import type { ListTasksReq, TaskSummary, TaskSummaryPage } from 'buckyos'
import { TaskHistory } from '../../src/api/task_history.ts'

const running = () => false

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`Expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

function task(id: number, revision = 1, created = id): TaskSummary {
  return {
    task_id: String(id), name: String(id), root_id: String(id), revision, created_at: created,
    updated_at: created, schema_id: 'test', schema_version: 1,
    creator: { user_id: 'test', app_id: 'app' }, storage_domain: 'User',
    phase: 'Terminal', executor_kind: 'Unbound',
  } as TaskSummary
}

Deno.test('newest first across more than ten ACL-filtered pages, with immediate publication before history', async () => {
  const requests: ListTasksReq[] = []
  const published: string[][] = []
  const historyPage = Promise.withResolvers<void>()
  const visible = Promise.withResolvers<void>()
  const history = new TaskHistory(async (req) => {
    equal(req.newest_first, true)
    requests.push(req)
    const index = req.cursor ? Number(req.cursor.split('/')[1]) : 0
    if (index === 2) await historyPage.promise
    const result = index === 1 ? [task(2001), task(2000)] : index === 12 ? [task(1)] : []
    return { tasks: result, next_cursor: index < 12 ? 'opaque/' + (index + 1) : undefined }
  })
  const load = history.refresh(() => {
    published.push(history.summaries.map((row) => row.task_id))
    visible.resolve()
  }, running)
  await visible.promise
  equal(published, [['2001', '2000']])
  equal(requests.map((req) => req.cursor), [undefined, 'opaque/1', 'opaque/2'])
  historyPage.resolve()
  await load
  equal(requests.length, 13)
  equal(published.at(-1), ['2001', '2000', '1'])
})

Deno.test('subsequent polls include same-time siblings, preserve newer revisions and remove inaccessible history', async () => {
  const requests: ListTasksReq[] = []
  let changed = false
  const history = new TaskHistory(async (req): Promise<TaskSummaryPage> => {
    requests.push(req)
    equal(req.newest_first, true)
    if (req.created_after !== undefined) {
      equal(req.created_after, 50)
      return req.cursor ? { tasks: [task(51, 1, 50), task(50)] } : {tasks: [], next_cursor: 'recent/2'}
    }
    if (!req.cursor) return { tasks: [task(50, 2)], next_cursor: 'older/2' }
    return { tasks: changed ? [] : [task(2)] }
  })
  await history.refresh(() => {}, running)
  changed = true
  for (const expectedCursor of [undefined, 'older/2']) {
    requests.length = 0
    await history.refresh(() => {}, running)
    equal(requests.map((req) => req.cursor), [undefined, 'recent/2', expectedCursor])
    equal(requests.filter((req) => req.created_after === undefined).length, 1)
    equal(history.summaries.find((row) => row.task_id === '50')?.revision, 2)
  }
  equal(history.summaries.map((row) => row.task_id), ['50', '51'])
})

Deno.test('a transient error retries the failed cursor and keeps previously published history', async () => {
  const cursors: Array<string | undefined> = []
  let fail = true
  const history = new TaskHistory(async (req) => {
    cursors.push(req.cursor)
    if (!req.cursor) return { tasks: [task(3)], next_cursor: 'older' }
    if (fail) throw new Error('temporary')
    return { tasks: [task(1)] }
  })
  let failed = false
  try { await history.refresh(() => {}, running) } catch { failed = true }
  equal(failed, true)
  equal(history.summaries.map((row) => row.task_id), ['3'])
  fail = false
  await history.refresh(() => {}, running)
  equal(cursors, [undefined, 'older', 'older'])
  equal(history.summaries.map((row) => row.task_id), ['3', '1'])
})

Deno.test('a disposed view discards late pages and a live view can resume that cursor', async () => {
  const reqs: Array<string | undefined> = []
  let closed = false
  const history = new TaskHistory(async (req) => {
    reqs.push(req.cursor)
    return req.cursor ? { tasks: [task(1)] } : { tasks: [task(2)], next_cursor: 'older' }
  })
  await history.refresh(() => { closed = true }, () => closed)
  equal(history.summaries.map((row) => row.task_id), ['2'])
  closed = false
  await history.refresh(() => {}, () => closed)
  equal(reqs, [undefined, 'older', 'older'])
  equal(history.summaries.map((row) => row.task_id), ['2', '1'])
})

Deno.test('an interrupted burst holds the time bound until every ACL page has been visited', async () => {
  let initial = true
  let fail = true
  const requests: ListTasksReq[] = []
  const history = new TaskHistory(async (req) => {
    requests.push(req)
    if (req.created_after === undefined) {
      return { tasks: initial ? [task(2)] : [task(103), task(102), task(101), task(2)] }
    }
    if (req.created_after === 102) return { tasks: [task(103)] }
    equal(req.created_after, 2)
    if (!req.cursor) return { tasks: [task(102)], next_cursor: 'private/1' }
    if (req.cursor === 'private/1') {
      if (fail) throw new Error('offline')
      return { tasks: [], next_cursor: 'private/2' }
    }
    if (req.cursor === 'private/2') return { tasks: [], next_cursor: 'permitted' }
    return { tasks: [task(101), task(2)] }
  })
  await history.refresh(() => {}, running)
  initial = false
  let interrupted = false
  try { await history.refresh(() => {}, running) } catch { interrupted = true }
  equal(interrupted, true)
  fail = false
  requests.length = 0
  await history.refresh(() => {}, running)
  equal(requests.map((req) => [req.created_after, req.cursor]), [[102, undefined], [2, 'private/1'], [2, 'private/2'], [undefined, undefined]])
  requests.length = 0
  await history.refresh(() => {}, running)
  equal(requests.map((req) => [req.created_after, req.cursor]), [[102, undefined], [2, 'permitted'], [undefined, undefined]])
  equal(history.summaries.map((row) => row.task_id).sort(), ['101', '102', '103', '2'])
})

Deno.test('an account with empty history discovers a newly visible task while its historical cursor is deep', async () => {
  const requests: ListTasksReq[] = []
  let visible = false
  const history = new TaskHistory(async (req) => {
    requests.push(req)
    if (!req.cursor) return { tasks: visible ? [task(70)] : [], next_cursor: 'older/1' }
    if (req.cursor === 'older/1') return { tasks: [], next_cursor: 'older/2' }
    return { tasks: [] }
  })
  await history.refresh(() => {}, running)
  await history.refresh(() => {}, running)
  equal(history.summaries, [])
  requests.length = 0
  visible = true
  const publications: string[][] = []
  await history.refresh(() => publications.push(history.summaries.map((row) => row.task_id)), running)
  equal(requests[0].cursor, undefined)
  equal(publications[0], ['70'])
  equal(history.summaries.map((row) => row.task_id), ['70'])
})
