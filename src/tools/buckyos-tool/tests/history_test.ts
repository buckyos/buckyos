import { ReplHistory, shouldPersistHistory } from '../core/history.ts'
import { createRegistry } from '../core/app.ts'
import { assert, assertEquals } from './test_helpers.ts'

Deno.test('REPL history omits credential-looking commands', () => {
  assert(!shouldPersistHistory('auth login --session-token eyJsecret'))
  assert(!shouldPersistHistory('config set password --value secret'))
  assert(shouldPersistHistory('system status'))
})

Deno.test('REPL history is bounded and persisted', async () => {
  const root = await Deno.makeTempDir()
  try {
    const path = `${root}/state/repl_history`
    const history = new ReplHistory(path, 2)
    const command = createRegistry().get('system', 'status')
    await history.add('system status', command)
    await history.add('auth whoami')
    await history.add('command list')
    assertEquals(await history.load(), ['auth whoami', 'command list'])
    if (Deno.build.os !== 'windows') {
      assertEquals((await Deno.stat(path)).mode! & 0o777, 0o600)
    }
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})
