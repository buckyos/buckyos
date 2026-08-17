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

Deno.test('launcher does not grant all permissions or process execution', async () => {
  const launcher = await Deno.readTextFile(new URL('../buckyos', import.meta.url))
  assertEquals(/(?:^|\s)-A(?:\s|$)/m.test(launcher), false)
  assertEquals(launcher.includes('--allow-run'), false)
})

Deno.test('invalid token errors are not misclassified as missing resources', () => {
  const error = normalizeError(
    new Error('RPC call error: Invalid token: users/ood1/settings not found'),
  )
  assertEquals(error.code, 'INVALID_SESSION')
  assertEquals(error.exitCode, 3)
})
