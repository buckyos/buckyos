import { type AuthenticatedSession, type SessionController } from '../core/auth.ts'
import { BuckyOSToolApplication, type ToolStdio } from '../core/app.ts'
import type { ResolvedConfig } from '../core/config.ts'
import type { RpcCallOptions, RuntimeAdapter, ServiceClientRegistry } from '../core/runtime.ts'
import { assert, assertEquals } from './test_helpers.ts'

class CaptureStdio implements ToolStdio {
  stdoutText = ''
  stderrText = ''
  stdinText = ''

  stdout(value: string): Promise<void> {
    this.stdoutText += value
    return Promise.resolve()
  }

  stderr(value: string): Promise<void> {
    this.stderrText += value
    return Promise.resolve()
  }

  readStdin(): Promise<string> {
    return Promise.resolve(this.stdinText)
  }
}

class FakeAuthentication implements SessionController {
  readonly config: ResolvedConfig
  connectCount = 0
  ensureCount = 0
  reconnectCount = 0
  readonly session: AuthenticatedSession

  constructor(config: ResolvedConfig) {
    this.config = config
    this.session = {
      token: 'fake-token',
      claims: { sub: 'alice', appid: 'jarvis', exp: 4_000_000_000 },
      renewable: false,
      principal: {
        id: 'alice',
        appId: 'jarvis',
        authentication: 'mock',
        tokenExpiresAt: '2096-10-02T07:06:40.000Z',
      },
    }
  }

  connect(): Promise<AuthenticatedSession> {
    this.connectCount += 1
    return Promise.resolve(this.session)
  }

  ensureValid(): Promise<AuthenticatedSession> {
    this.ensureCount += 1
    return Promise.resolve(this.session)
  }

  reconnect(): Promise<AuthenticatedSession> {
    this.reconnectCount += 1
    return Promise.resolve(this.session)
  }

  current(): AuthenticatedSession {
    return this.session
  }

  status(): Record<string, unknown> {
    return { authenticated: true, principal: 'alice', appid: 'jarvis' }
  }
}

Deno.test('offline discovery works without config, network, or identity', async () => {
  const io = new CaptureStdio()
  const app = new BuckyOSToolApplication({
    environment: {},
    homeDir: '/missing-home',
    stdio: io,
  })
  assertEquals(await app.run(['command', 'describe', 'system', 'status']), 0)
  const envelope = JSON.parse(io.stdoutText)
  assertEquals(envelope.ok, true)
  assertEquals(envelope.data.module, 'system')
  assertEquals(envelope.data.requires_session, true)
  const sessionToken = envelope.data.global_options.find(
    (option: Record<string, unknown>) => option.name === 'session-token',
  )
  assertEquals(sessionToken.secret, true)
  assertEquals(sessionToken.scope, 'session')
})

Deno.test('errors use a stable JSON envelope and exit code', async () => {
  const io = new CaptureStdio()
  const app = new BuckyOSToolApplication({ environment: {}, stdio: io })
  assertEquals(await app.run(['unknown', 'list']), 2)
  const envelope = JSON.parse(io.stdoutText)
  assertEquals(envelope.ok, false)
  assertEquals(envelope.error.code, 'UNKNOWN_MODULE')
})

Deno.test('interactive commands reuse one session and reset command-scoped state', async () => {
  const root = await Deno.makeTempDir()
  try {
    const io = new CaptureStdio()
    let authentication: FakeAuthentication | undefined
    let runtimeInitializeCount = 0
    let rpcCount = 0
    const rpcTraceIds: string[] = []
    const runtime: RuntimeAdapter = {
      initialize: (_config, session) => {
        runtimeInitializeCount += 1
        assertEquals(session, authentication)
        return Promise.resolve({
          zone: 'test.example.com',
          endpoint: 'https://test.example.com',
          defaultProtocol: 'https://',
        })
      },
    }
    const clients: ServiceClientRegistry = {
      call: <T>(
        _service: string,
        _method: string,
        _params: Record<string, unknown>,
        options: RpcCallOptions,
      ) => {
        rpcTraceIds.push(options.traceId)
        return Promise.resolve({ state: 'online', request: ++rpcCount } as T)
      },
    }
    const app = new BuckyOSToolApplication({
      environment: { HOME: root },
      homeDir: root,
      stdio: io,
      runtime,
      createAuthentication: (config) => authentication = new FakeAuthentication(config),
      createClients: () => clients,
      repl: async (options) => {
        await options.execute(
          ['system', 'status', '--trace-id', 'fixed-trace'],
          new AbortController().signal,
        )
        await options.execute(['system', 'status'], new AbortController().signal)
      },
    })
    assertEquals(
      await app.run([
        '--endpoint',
        'https://test.example.com',
        '--session-token',
        'ignored',
        '--output',
        'json',
        '--cli',
      ]),
      0,
    )
    const envelopes = io.stdoutText.trim().split('\n').map((line) => JSON.parse(line))
    assertEquals(envelopes.length, 2)
    assertEquals(envelopes[0].meta.trace_id, 'fixed-trace')
    assert(envelopes[1].meta.trace_id !== 'fixed-trace')
    assertEquals(envelopes[0].data.request, 1)
    assertEquals(envelopes[1].data.request, 2)
    assertEquals(rpcTraceIds, [envelopes[0].meta.trace_id, envelopes[1].meta.trace_id])
    assertEquals(runtimeInitializeCount, 1)
    assertEquals(authentication?.connectCount, 1)
    assertEquals(authentication?.ensureCount, 2)
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('one online command failure does not prevent the next REPL command', async () => {
  const root = await Deno.makeTempDir()
  try {
    const io = new CaptureStdio()
    let calls = 0
    const app = new BuckyOSToolApplication({
      environment: { HOME: root },
      homeDir: root,
      stdio: io,
      runtime: {
        initialize: () =>
          Promise.resolve({
            zone: 'test.example.com',
            endpoint: 'https://test.example.com',
            defaultProtocol: 'https://',
          }),
      },
      createAuthentication: (config) => new FakeAuthentication(config),
      createClients: () => ({
        call: <T>() => {
          calls += 1
          return calls === 1
            ? Promise.reject(new Error('RPC call error: 503'))
            : Promise.resolve({ state: 'online' } as T)
        },
      }),
      repl: async (options) => {
        await options.execute(['system', 'status'], new AbortController().signal)
        await options.execute(['system', 'status'], new AbortController().signal)
      },
    })
    await app.run([
      '--endpoint',
      'https://test.example.com',
      '--session-token',
      'ignored',
      '--output',
      'json',
      '--cli',
    ])
    const envelopes = io.stdoutText.trim().split('\n').map((line) => JSON.parse(line))
    assertEquals(envelopes[0].ok, false)
    assertEquals(envelopes[0].error.code, 'SERVICE_UNAVAILABLE')
    assertEquals(envelopes[1].ok, true)
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})
