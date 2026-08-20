import { createMockCommandContext } from '../core/context.ts'
import { ConfigStore } from '../core/config.ts'
import { CommandRegistry } from '../core/registry.ts'
import type { RpcCallOptions, ServiceClientRegistry } from '../core/runtime.ts'
import { createSystemConfigModule } from '../modules/system_config.ts'
import { assertEquals, assertRejects, testConfig } from './test_helpers.ts'

interface RecordedCall {
  service: string
  method: string
  params: Record<string, unknown>
  options: RpcCallOptions
}

Deno.test('system-config module exposes the migrated basic operations', () => {
  const registry = systemConfigRegistry()
  assertEquals(
    registry.commands().map((command) => command.verb),
    ['append', 'get', 'list', 'set', 'set-file'],
  )
})

Deno.test('system-config commands map to the current kRPC methods', async () => {
  const calls: RecordedCall[] = []
  const responses = new Map<string, unknown>([
    ['sys_config_get', { value: '{"enabled":true}', version: 7 }],
    ['sys_config_list', ['alpha', 'beta']],
  ])
  const clients: ServiceClientRegistry = {
    call: <T>(
      service: string,
      method: string,
      params: Record<string, unknown>,
      options: RpcCallOptions,
    ) => {
      calls.push({ service, method, params, options })
      return Promise.resolve((responses.get(method) ?? null) as T)
    },
  }
  const registry = systemConfigRegistry()

  assertEquals(
    await run(registry, clients, 'get', { key: 'services/example/config' }),
    {
      key: 'services/example/config',
      value: '{"enabled":true}',
      version: 7,
      text: '{"enabled":true}',
    },
  )
  assertEquals(await run(registry, clients, 'list', { key: 'services' }), {
    key: 'services',
    items: ['alpha', 'beta'],
  })
  assertEquals(
    await run(registry, clients, 'set', { key: 'a', value: 'one' }),
    { key: 'a', updated: true },
  )
  assertEquals(
    await run(registry, clients, 'append', { key: 'a', value: 'two' }),
    { key: 'a', appended: true },
  )
  assertEquals(
    calls.map(({ service, method, params }) => ({ service, method, params })),
    [
      {
        service: 'system_config',
        method: 'sys_config_get',
        params: { key: 'services/example/config' },
      },
      {
        service: 'system_config',
        method: 'sys_config_list',
        params: { key: 'services' },
      },
      {
        service: 'system_config',
        method: 'sys_config_set',
        params: { key: 'a', value: 'one' },
      },
      {
        service: 'system_config',
        method: 'sys_config_append',
        params: { key: 'a', append_value: 'two' },
      },
    ],
  )
})

Deno.test('system-config set-file reads the complete file content', async () => {
  const path = await Deno.makeTempFile()
  try {
    await Deno.writeTextFile(path, '{"source":"file"}\n')
    let params: Record<string, unknown> | undefined
    const clients: ServiceClientRegistry = {
      call: <T>(
        _service: string,
        _method: string,
        input: Record<string, unknown>,
      ) => {
        params = input
        return Promise.resolve(null as T)
      },
    }
    const registry = systemConfigRegistry()
    assertEquals(
      await run(registry, clients, 'set-file', { key: 'a', file: path }),
      { key: 'a', updated: true },
    )
    assertEquals(params, { key: 'a', value: '{"source":"file"}\n' })
  } finally {
    await Deno.remove(path)
  }
})

Deno.test('system-config get reports a missing key', async () => {
  const clients: ServiceClientRegistry = {
    call: <T>() => Promise.resolve(null as T),
  }
  await assertRejects(
    () => run(systemConfigRegistry(), clients, 'get', { key: 'missing' }),
    'RESOURCE_NOT_FOUND',
  )
})

function systemConfigRegistry(): CommandRegistry {
  const registry = new CommandRegistry()
  registry.register(createSystemConfigModule())
  return registry
}

async function run(
  registry: CommandRegistry,
  clients: ServiceClientRegistry,
  verb: string,
  input: Record<string, unknown>,
): Promise<unknown> {
  const command = registry.get('system-config', verb)
  return await command.handler(
    createMockCommandContext({
      command,
      config: testConfig(),
      configStore: new ConfigStore('/tmp/buckyos-tool-system-config-test'),
      clients,
      traceId: 'system-config-test',
    }),
    input,
  )
}
