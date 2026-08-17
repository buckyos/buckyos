import { ConfigStore, resolveConfig } from '../core/config.ts'
import { identityRootPairs } from '../core/identity.ts'
import { assert, assertEquals, assertRejects, testConfig } from './test_helpers.ts'

Deno.test('configuration precedence is argument, environment, profile, global, default', async () => {
  const root = await Deno.makeTempDir()
  try {
    const store = new ConfigStore(root)
    await store.writeConfig({ schema_version: 1, default_profile: 'production', output: 'text' })
    await store.writeProfile('production', {
      schema_version: 1,
      zone: 'profile.example',
      identity: 'profile-user',
      output: 'table',
    })
    const { resolved } = await resolveConfig(
      { configDir: root, zone: 'argument.example' },
      {
        HOME: root,
        BUCKYOS_TOOL_IDENTITY: 'environment-user',
        BUCKYOS_TOOL_OUTPUT: 'jsonl',
      },
      { homeDir: root },
    )
    assertEquals(resolved.zone, 'argument.example')
    assertEquals(resolved.identity, 'environment-user')
    assertEquals(resolved.output, 'jsonl')
    assertEquals(resolved.profileName, 'production')
    assertEquals(resolved.sources.zone, 'argument')
    assertEquals(resolved.sources.identity, 'environment')
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('interactive output defaults to table unless explicitly selected', async () => {
  const root = await Deno.makeTempDir()
  try {
    const store = new ConfigStore(root)
    await store.writeConfig({ schema_version: 1, output: 'jsonl' })
    const implicit = await resolveConfig({ configDir: root }, { HOME: root }, {
      interactive: true,
      homeDir: root,
    })
    const explicit = await resolveConfig({ configDir: root, output: 'json' }, { HOME: root }, {
      interactive: true,
      homeDir: root,
    })
    assertEquals(implicit.resolved.output, 'table')
    assertEquals(explicit.resolved.output, 'json')
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('identity roots never include legacy buckycli paths', () => {
  const pairs = identityRootPairs(
    testConfig({ configDir: '/home/alice/.buckyos_tool' }),
    { BUCKYOS_ROOT: '/opt/buckyos' },
  )
  assert(pairs.every((pair) => !pair.publicRoot.includes('.buckycli')))
  assert(pairs.every((pair) => !pair.securityRoot.includes('.buckycli')))
  assertEquals(pairs[0].publicRoot, '/home/alice/.buckyos_tool/local/identity')
})

Deno.test('config files reject secret or unknown fields', async () => {
  const root = await Deno.makeTempDir()
  try {
    const store = new ConfigStore(root)
    await assertRejects(
      () =>
        store.writeProfile(
          'production',
          {
            schema_version: 1,
            session_token: 'secret',
          } as unknown as Parameters<ConfigStore['writeProfile']>[1],
        ),
      'INVALID_CONFIG',
    )
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('config writes leave no temporary file behind', async () => {
  const root = await Deno.makeTempDir()
  try {
    const store = new ConfigStore(root)
    await store.writeConfig({ schema_version: 1, output: 'json' })
    assertEquals((await Array.fromAsync(Deno.readDir(root))).map((entry) => entry.name), [
      'config.json',
    ])
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})
