import { type AuthenticatedSession, type SessionController } from '../core/auth.ts'
import { BuckyOSToolApplication, type ToolStdio } from '../core/app.ts'
import type { ResolvedConfig } from '../core/config.ts'
import type { RpcCallOptions, RuntimeAdapter, ServiceClientRegistry } from '../core/runtime.ts'
import { assert, assertEquals } from './test_helpers.ts'
import { join } from 'node:path'

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

Deno.test('zero-config online command uses current device identity after confirmation', async () => {
  const root = await Deno.makeTempDir()
  try {
    const home = join(root, 'home')
    const buckyosRoot = join(root, 'buckyos')
    await writeNodeIdentity(buckyosRoot)
    const io = new CaptureStdio()
    let resolved: ResolvedConfig | undefined
    let confirmedDid: string | undefined
    const app = new BuckyOSToolApplication({
      environment: { HOME: home, BUCKYOS_ROOT: buckyosRoot },
      homeDir: home,
      stdio: io,
      confirmDeviceIdentity: (identity) => {
        confirmedDid = identity.did
        return Promise.resolve(true)
      },
      createAuthentication: (config) => {
        resolved = config
        return new FakeAuthentication(config)
      },
      runtime: {
        initialize: (config) =>
          Promise.resolve({
            zone: config.zone!,
            endpoint: config.endpoint!,
            defaultProtocol: config.defaultProtocol,
          }),
      },
      createClients: () => ({
        call: <T>() => Promise.resolve({ state: 'online' } as T),
      }),
    })
    assertEquals(await app.run(['system', 'status']), 0)
    assertEquals(confirmedDid, 'did:web:ood1.test.buckyos.io')
    assertEquals(resolved?.identity, 'did:web:ood1.test.buckyos.io')
    assertEquals(resolved?.zone, 'did:web:test.buckyos.io')
    assertEquals(resolved?.endpoint, 'http://127.0.0.1:3180')
    assertEquals(resolved?.identityRoot, join(buckyosRoot, 'local', 'identity'))
    assertEquals(resolved?.securityRoot, join(buckyosRoot, 'security'))
    assertEquals(resolved?.sources.identity, 'current-device')
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('non-interactive device fallback requires yes', async () => {
  const root = await Deno.makeTempDir()
  try {
    const home = join(root, 'home')
    const buckyosRoot = join(root, 'buckyos')
    await writeNodeIdentity(buckyosRoot)
    const io = new CaptureStdio()
    let authenticationCreated = false
    const app = new BuckyOSToolApplication({
      environment: { HOME: home, BUCKYOS_ROOT: buckyosRoot },
      homeDir: home,
      stdio: io,
      confirmDeviceIdentity: () => Promise.resolve(true),
      createAuthentication: (config) => {
        authenticationCreated = true
        return new FakeAuthentication(config)
      },
    })
    assertEquals(await app.run(['--non-interactive', 'system', 'status']), 4)
    assertEquals(authenticationCreated, false)
    const envelope = JSON.parse(io.stdoutText)
    assertEquals(envelope.error.code, 'CONFIRMATION_REQUIRED')
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('non-interactive device fallback accepts yes', async () => {
  const root = await Deno.makeTempDir()
  try {
    const home = join(root, 'home')
    const buckyosRoot = join(root, 'buckyos')
    await writeNodeIdentity(buckyosRoot)
    const io = new CaptureStdio()
    let confirmationCalled = false
    const app = new BuckyOSToolApplication({
      environment: { HOME: home, BUCKYOS_ROOT: buckyosRoot },
      homeDir: home,
      stdio: io,
      confirmDeviceIdentity: () => {
        confirmationCalled = true
        return Promise.resolve(false)
      },
      createAuthentication: (config) => new FakeAuthentication(config),
      runtime: {
        initialize: (config) =>
          Promise.resolve({
            zone: config.zone!,
            endpoint: config.endpoint!,
            defaultProtocol: config.defaultProtocol,
          }),
      },
      createClients: () => ({
        call: <T>() => Promise.resolve({ state: 'online' } as T),
      }),
    })
    assertEquals(
      await app.run(['--non-interactive', '--yes', 'system', 'status']),
      0,
    )
    assertEquals(confirmationCalled, false)
  } finally {
    await Deno.remove(root, { recursive: true })
  }
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

async function writeNodeIdentity(buckyosRoot: string): Promise<void> {
  const etc = join(buckyosRoot, 'etc')
  await Deno.mkdir(etc, { recursive: true })
  await Deno.writeTextFile(
    join(etc, 'node_identity.json'),
    JSON.stringify({
      schema: 'buckyos.node_identity.v2',
      zone_did: 'did:web:test.buckyos.io',
      device_name: 'ood1',
      device_did: 'did:web:ood1.test.buckyos.io',
    }),
  )
}
