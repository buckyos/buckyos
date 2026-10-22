import type { AccessDenial } from 'buckyos'
import { adjustLocationLabel, denialAdjustText, denialText } from '../../src/sso/denial.ts'
import { maskChannelTarget } from '../../src/sso/channel.ts'

// The wire fixtures shared with buckyos-api (src/kernel/buckyos-api/tests/fixtures/hybrid_sso).
const FIXTURES = new URL('../../../../kernel/buckyos-api/tests/fixtures/hybrid_sso/', import.meta.url)

function assertEquals(actual: unknown, expected: unknown) {
  const actualJson = JSON.stringify(actual)
  const expectedJson = JSON.stringify(expected)
  if (actualJson !== expectedJson) {
    throw new Error(`expected ${expectedJson}, got ${actualJson}`)
  }
}

/** The English fallback with variables filled in, as the I18nProvider renders it without a table. */
const t = (key: string, fallback = key, variables?: Record<string, string | number>) =>
  Object.entries(variables ?? {}).reduce((text, [name, value]) => text.replaceAll(`{{${name}}}`, String(value)), fallback)

const fixture = (name: string): AccessDenial => JSON.parse(Deno.readTextFileSync(new URL(name, FIXTURES)))

Deno.test('denials from the shared fixtures read by reason, detail and adjust location', () => {
  assertEquals(denialText(t, fixture('access_denial_invite_expired.json')), 'This link has expired.')
  assertEquals(
    denialText(t, fixture('access_denial_grant_missing.json')),
    'You do not have access to this. Contact the person who shared it.',
  )
  assertEquals(
    denialAdjustText(t, fixture('access_denial_zone_disabled.json')),
    'An administrator adjusts this in Settings › Sign-in & Sharing.',
  )
  assertEquals(
    denialAdjustText(t, fixture('access_denial_kind_not_allowed.json')),
    'Product boundary: this kind of account does not support the feature; there is no setting for it.',
  )
  assertEquals(
    denialAdjustText(t, fixture('access_denial_grant_missing.json')),
    'The sharer adjusts this in the share panel of the resource.',
  )
})

Deno.test('every reason has its own visitor text and every location a page name', () => {
  const reasons = [
    'session_kind_not_allowed',
    'session_ceiling_exceeded',
    'resource_grant_missing',
    'entry_unavailable',
    'zone_method_disabled',
    'account_state',
    'credential_state',
    'source_revoked',
    'authorization_unavailable',
  ] as const
  const texts = reasons.map((reason) => denialText(t, { reason, adjust: { party: 'none' } }))
  assertEquals(new Set(texts).size, reasons.length)
  assertEquals(texts.every(Boolean), true)
  assertEquals(adjustLocationLabel(t, 'cp:exclusive_invites'), 'Users & Agents › Exclusive invites')
  assertEquals(adjustLocationLabel(t, 'cp:users'), 'Users & Agents')
})

Deno.test('a channel address is stored only as a masked hint', () => {
  assertEquals(maskChannelTarget('email', 'alice@acme.com'), 'a***@acme.com')
  assertEquals(maskChannelTarget('sms', '138 1234 5678'), '138****5678')
  assertEquals(maskChannelTarget('other', 'wx-9'), 'w***')
  assertEquals(maskChannelTarget('email', '  '), '')
})
