import {
  CONTROL_PANEL_SERVICE_ID,
  buildAuthLoginTargetParams,
} from '../../src/auth/loginTarget.ts'

function assertEquals(actual: unknown, expected: unknown) {
  const actualJson = JSON.stringify(actual)
  const expectedJson = JSON.stringify(expected)
  if (actualJson !== expectedJson) {
    throw new Error(`expected ${expectedJson}, got ${actualJson}`)
  }
}

Deno.test('SSO login target contains only redirect_url', () => {
  assertEquals(
    buildAuthLoginTargetParams('https://files.test.buckyos.io/'),
    { redirect_url: 'https://files.test.buckyos.io/' },
  )
})

Deno.test('direct login targets the Control Panel system service', () => {
  assertEquals(buildAuthLoginTargetParams(''), {
    target: {
      kind: 'system',
      service_id: CONTROL_PANEL_SERVICE_ID,
    },
  })
})
