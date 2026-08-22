import { withDeadline } from '../core/runtime.ts'
import { normalizeError } from '../core/errors.ts'
import { assertEquals, assertRejects } from './test_helpers.ts'

Deno.test('local timeout returns the stable TIMEOUT error', async () => {
  await assertRejects(
    () => withDeadline(new Promise(() => {}), 1),
    'TIMEOUT',
  )
})

Deno.test('local cancellation does not mutate a remote operation', async () => {
  const controller = new AbortController()
  let remoteCompleted = false
  const remote = new Promise<string>((resolve) => {
    setTimeout(() => {
      remoteCompleted = true
      resolve('done')
    }, 5)
  })
  controller.abort()
  await assertRejects(() => withDeadline(remote, 100, controller.signal), 'CANCELED')
  await remote
  assertEquals(remoteCompleted, true)
})

Deno.test('launcher grants Docker execution only to the offline pikg branch', async () => {
  const launcher = await Deno.readTextFile(new URL('../buckyos', import.meta.url))
  assertEquals(/(?:^|\s)-A(?:\s|$)/m.test(launcher), false)
  assertEquals((launcher.match(/--allow-run=docker/g) ?? []).length, 1)
  const execLines = launcher.split('\n').filter((line) => line.startsWith('  exec deno run'))
  assertEquals(execLines.length, 1)
  assertEquals(execLines[0].includes('--allow-net'), false)
  assertEquals(execLines[0].includes('--allow-run=docker'), true)
  const onlineExec = launcher.split('\n').find((line) => line.startsWith('exec deno run'))!
  assertEquals(onlineExec.includes('--allow-net'), true)
  assertEquals(onlineExec.includes('--allow-run'), false)
})

Deno.test('invalid token errors are not misclassified as missing resources', () => {
  const error = normalizeError(
    new Error('RPC call error: Invalid token: users/ood1/settings not found'),
  )
  assertEquals(error.code, 'INVALID_SESSION')
  assertEquals(error.exitCode, 3)
})
