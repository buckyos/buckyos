import { join } from 'node:path'
import { createRegistry } from '../core/app.ts'
import { ConfigStore } from '../core/config.ts'
import { createMockCommandContext } from '../core/context.ts'
import { ToolError } from '../core/errors.ts'
import { CommandRegistry } from '../core/registry.ts'
import type { RpcCallOptions, ServiceClientRegistry } from '../core/runtime.ts'
import { waitForTask } from '../core/task.ts'
import { createAuditModule } from '../modules/audit.ts'
import { createDiagnosticModule } from '../modules/diagnostic.ts'
import { createLogModule } from '../modules/log.ts'
import { createTaskModule } from '../modules/task.ts'
import { assert, assertEquals, assertRejects, testConfig } from './test_helpers.ts'

interface RecordedCall {
  service: string
  method: string
  params: Record<string, unknown>
}

Deno.test('observability facade exposes all eleven frozen commands', () => {
  const registry = createRegistry()
  assertEquals(
    ['task', 'audit', 'log', 'diagnostic'].flatMap((module) =>
      registry.modules().find((candidate) => candidate.name === module)!.commands.map((command) =>
        `${module}.${command.verb}`
      )
    ),
    [
      'task.list',
      'task.get',
      'task.wait',
      'task.cancel',
      'task.retry',
      'audit.query',
      'log.query',
      'log.tail',
      'log.export',
      'diagnostic.collect',
      'diagnostic.export',
    ],
  )
})

Deno.test('task list maps filters and task get preserves independent child pagination', async () => {
  const calls: RecordedCall[] = []
  const clients = clientsFor(calls, (method, params) => {
    if (method === 'list_tasks') {
      assertEquals(params, {
        creator_user_id: 'alice',
        schema_id: 'app.install/v1',
        phase: 'Running',
        created_after: Date.parse('2026-08-31T00:00:00Z'),
        limit: 20,
        include_archived: false,
      })
      return { tasks: [{ task_id: 't-1' }], next_cursor: 'next' }
    }
    if (method === 'get_task') return { task: { task_id: 't-1', phase: 'Running' } }
    if (method === 'get_subtasks') {
      assertEquals(params, { task_id: 't-1', cursor: 'children', limit: 10 })
      return { tasks: [{ task_id: 't-child' }], next_cursor: null }
    }
    throw new Error(`unexpected method ${method}`)
  })
  assertEquals(
    await runTask('list', {
      owner: 'alice',
      type: 'app.install/v1',
      state: 'running',
      since: '2026-08-31T00:00:00Z',
      limit: 20,
    }, clients),
    { items: [{ task_id: 't-1' }], next_cursor: 'next' },
  )
  assertEquals(
    await runTask('get', {
      task_id: 't-1',
      children_cursor: 'children',
      children_limit: 10,
    }, clients),
    {
      task: { task_id: 't-1', phase: 'Running' },
      children: { items: [{ task_id: 't-child' }], next_cursor: null },
    },
  )
})

Deno.test('task cancel uses TaskManager control protocol without canceling on local timeout', async () => {
  const calls: RecordedCall[] = []
  const clients = clientsFor(calls, (method, params) => {
    if (method === 'request_control') return { kind: 'Task', task: { task_id: params.task_id } }
    if (method === 'get_task') {
      return { task: { task_id: 't-timeout', phase: 'Running', revision: 1 } }
    }
    throw new Error(`unexpected method ${method}`)
  })
  await runTask(
    'cancel',
    {
      task_id: 't-1',
      recursive: true,
      expected_revision: 7,
    },
    clients,
    'cancel-key',
  )
  assertEquals(calls[0].params, {
    task_id: 't-1',
    action: 'Cancel',
    request_id: 'cancel-key',
    recursive: true,
    expected_revision: 7,
  })
  const context = taskContext('wait', clients)
  context.deadline = Date.now() - 1
  await assertRejects(() => waitForTask(context, 't-timeout'), 'TIMEOUT')
  assertEquals(calls.filter((call) => call.method === 'request_control').length, 1)
})

Deno.test('task wait uses KEvent only as wakeup and returns terminal failures', async () => {
  const calls: RecordedCall[] = []
  let snapshot = 0
  let pulls = 0
  let closed = false
  const clients: ServiceClientRegistry = {
    call: <T>(service: string, method: string, params: Record<string, unknown>) => {
      calls.push({ service, method, params })
      snapshot += 1
      return Promise.resolve({
        task: snapshot === 1
          ? { task_id: 't-1', phase: 'Running', revision: 1, progress: { completed: 1 } }
          : {
            task_id: 't-1',
            phase: 'Terminal',
            outcome: 'Failed',
            revision: 2,
            result: { items: [{ ok: false }] },
          },
      } as T)
    },
    createEventReader: (pattern) => {
      assertEquals(pattern, '/task_mgr/t-1')
      return Promise.resolve({
        pullEvent: () => {
          pulls += 1
          return Promise.resolve({ eventid: pattern })
        },
        close: () => {
          closed = true
          return Promise.resolve()
        },
      })
    },
  }
  const output: string[] = []
  const context = taskContext('wait', clients)
  context.io.stdout = (value) => {
    output.push(value)
    return Promise.resolve()
  }
  const result = await context.definition.handler(context, { task_id: 't-1' }) as Record<
    string,
    unknown
  >
  assertEquals(result.outcome, 'Failed')
  assertEquals(pulls, 1)
  assertEquals(closed, true)
  assertEquals(calls.length, 2)
  assertEquals(output.map((line) => JSON.parse(line).revision), [1, 2])
})

Deno.test('task retry rejects nonterminal input and validates the replacement identity', async () => {
  const running = clientsFor([], () => ({ task: { phase: 'Running' } }))
  await assertRejects(() => runTask('retry', { task_id: 't-1' }, running), 'TASK_NOT_RETRYABLE')

  const calls: RecordedCall[] = []
  const retried = clientsFor(calls, (method) => {
    if (method === 'get_task') return { task: { phase: 'Terminal', outcome: 'Failed' } }
    if (method === 'task.retry') return { task_id: 't-2', retry_of: 't-1' }
    throw new Error(`unexpected method ${method}`)
  })
  assertEquals(await runTask('retry', { task_id: 't-1', no_wait: true }, retried, 'retry-key'), {
    task_id: 't-2',
    retry_of: 't-1',
  })
  assertEquals(calls[1], {
    service: 'control-panel',
    method: 'task.retry',
    params: { task_id: 't-1', idempotency_key: 'retry-key' },
  })
})

Deno.test('audit query forwards actor, trace, time, and opaque cursor filters', async () => {
  const calls: RecordedCall[] = []
  const clients = clientsFor(calls, () => ({ events: [] }))
  const registry = new CommandRegistry()
  registry.register(createAuditModule())
  const command = registry.get('audit', 'query')
  const context = commandContext(command, clients)
  const clientsWithPage = clientsFor(
    calls,
    () => ({ items: [{ audit_id: 'a-1' }], next_cursor: 'n' }),
  )
  context.clients = clientsWithPage
  assertEquals(
    await command.handler(context, {
      scope: 'zone',
      actor: 'alice',
      trace: 'trace-1',
      since: '1000',
      cursor: 'c',
      limit: 5,
    }),
    { items: [{ audit_id: 'a-1' }], next_cursor: 'n' },
  )
  assertEquals(calls[0].params, {
    scope: 'zone',
    actor: 'alice',
    trace_id: 'trace-1',
    created_after: 1000,
    cursor: 'c',
    limit: 5,
  })
})

Deno.test('log query and tail share filters and tail emits JSONL', async () => {
  const calls: RecordedCall[] = []
  const clients = clientsFor(calls, (method) => {
    if (method === 'system.logs.query') return { entries: [{ message: 'safe' }], nextCursor: 'q' }
    if (method === 'system.logs.tail') {
      return { entries: [{ message: 'safe-tail' }], nextCursor: 't' }
    }
    throw new Error(`unexpected method ${method}`)
  })
  const module = createLogModule({ sleep: () => Promise.reject(new ToolError('CANCELED', 'stop')) })
  const registry = new CommandRegistry()
  registry.register(module)
  const query = registry.get('log', 'query')
  assertEquals(
    await query.handler(commandContext(query, clients), {
      service: 'scheduler',
      level: 'ERROR',
      keyword: 'timeout',
    }),
    { items: [{ message: 'safe' }], next_cursor: 'q' },
  )
  const tail = registry.get('log', 'tail')
  const context = commandContext(tail, clients)
  let output = ''
  context.io.stdout = (value) => {
    output += value
    return Promise.resolve()
  }
  await assertRejects(
    () => tail.handler(context, { service: 'scheduler', from: 'end' }),
    'CANCELED',
  )
  assertEquals(JSON.parse(output).entry.message, 'safe-tail')
  assertEquals(calls[0].params.services, ['scheduler'])
  assertEquals(calls[1].params.services, ['scheduler'])
})

Deno.test('log and diagnostic exports create new files and verify archive bytes', async () => {
  const root = await Deno.makeTempDir()
  try {
    const bytes = new TextEncoder().encode('archive')
    const digest = await sha256(bytes)
    const calls: RecordedCall[] = []
    const clients = clientsFor(calls, (method) => {
      if (method === 'system.logs.download') {
        return { url: '/download/log-token', filename: 'logs.zip' }
      }
      if (method === 'diagnostic.export') {
        return {
          url: '/download/diagnostic-token',
          bundle_id: 'diag-1',
          artifact_sha256: digest,
          sha256: 'content-digest',
          expires_at: 10,
        }
      }
      throw new Error(`unexpected method ${method}`)
    })
    const fetcher = () => Promise.resolve(bytes)
    const logModule = createLogModule({ download: fetcher })
    const logRegistry = new CommandRegistry()
    logRegistry.register(logModule)
    const logExport = logRegistry.get('log', 'export')
    const logPath = join(root, 'logs.zip')
    const logResult = await logExport.handler(commandContext(logExport, clients, root), {
      services: 'scheduler',
      since: '2026-08-25T00:00:00Z',
      path: logPath,
    }) as Record<string, unknown>
    assertEquals(logResult.sha256, digest)
    assertEquals(logResult.url, undefined)

    const diagnosticRegistry = new CommandRegistry()
    diagnosticRegistry.register(createDiagnosticModule({ download: fetcher }))
    const diagnosticExport = diagnosticRegistry.get('diagnostic', 'export')
    const diagnosticPath = join(root, 'diagnostic.zip')
    const diagnosticResult = await diagnosticExport.handler(
      commandContext(diagnosticExport, clients, root),
      { bundle_id: 'diag-1', path: diagnosticPath },
    ) as Record<string, unknown>
    assertEquals(diagnosticResult.sha256, digest)
    assertEquals(diagnosticResult.content_sha256, 'content-digest')
    await assertRejects(
      () =>
        diagnosticExport.handler(commandContext(diagnosticExport, clients, root), {
          bundle_id: 'diag-1',
          path: diagnosticPath,
        }),
      'OUTPUT_EXISTS',
    )
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('diagnostic collect creates a Task and supports no-wait', async () => {
  const calls: RecordedCall[] = []
  const clients = clientsFor(calls, () => ({ task_id: 't-diag' }))
  const registry = new CommandRegistry()
  registry.register(createDiagnosticModule())
  const command = registry.get('diagnostic', 'collect')
  const context = commandContext(command, clients)
  context.idempotencyKey = 'diag-key'
  assertEquals(
    await command.handler(context, {
      services: 'scheduler,node_daemon',
      no_wait: true,
    }),
    { task_id: 't-diag' },
  )
  assertEquals(calls[0].params, {
    services: ['scheduler', 'node_daemon'],
    idempotency_key: 'diag-key',
  })
})

function clientsFor(
  calls: RecordedCall[],
  response: (method: string, params: Record<string, unknown>) => unknown,
): ServiceClientRegistry {
  return {
    call: <T>(
      service: string,
      method: string,
      params: Record<string, unknown>,
      _options: RpcCallOptions,
    ) => {
      calls.push({ service, method, params })
      return Promise.resolve(response(method, params) as T)
    },
  }
}

async function runTask(
  verb: string,
  input: Record<string, unknown>,
  clients: ServiceClientRegistry,
  idempotencyKey?: string,
): Promise<unknown> {
  const context = taskContext(verb, clients)
  context.idempotencyKey = idempotencyKey
  return await context.definition.handler(context, input)
}

function taskContext(verb: string, clients: ServiceClientRegistry) {
  const registry = new CommandRegistry()
  registry.register(createTaskModule())
  return commandContext(registry.get('task', verb), clients)
}

function commandContext(
  command: ReturnType<CommandRegistry['get']>,
  clients: ServiceClientRegistry,
  cwd?: string,
) {
  const context = createMockCommandContext({
    command,
    config: testConfig(),
    configStore: new ConfigStore('/tmp/buckyos-tool-observability-test'),
    clients,
    traceId: 'observability-test',
  })
  context.deadline = Date.now() + 10_000
  if (cwd) context.cwd = cwd
  return context
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', Uint8Array.from(bytes).buffer)
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('')
}
