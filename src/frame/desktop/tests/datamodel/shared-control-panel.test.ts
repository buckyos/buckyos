import { prepareAppLaunch } from '../../src/shared/controlPanel/appLaunch.ts'
import { listApps } from '../../src/shared/controlPanel/apps.ts'
import { readAccountDid, readAgentDids } from '../../src/shared/controlPanel/account.ts'
import { DesktopApiError, readDesktopState, writeDesktopState } from '../../src/shared/controlPanel/desktopState.ts'
import { readSystemMetrics } from '../../src/shared/controlPanel/systemMetrics.ts'
import { accentPresets, accentText, normalizeAccent } from '../../src/shared/appearance/accent.ts'
import type { ZoneChannel } from '../../src/shared/channel.ts'

function equal(actual: unknown, expected: unknown, what = '') {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`${what}\nexpected ${JSON.stringify(expected)}\nreceived ${JSON.stringify(actual)}`)
}

function channel(answer: (method: string) => unknown, respond?: (path: string, init?: RequestInit) => Response) {
  const calls: unknown[] = []
  const value: ZoneChannel = {
    rpc: async <T>(service: string, method: string, params?: Record<string, unknown>) => { calls.push([service, method, params]); return answer(method) as T },
    fetch: async (path, init) => { calls.push([path, init?.method, init?.body && JSON.parse(String(init.body))]); if (!respond) throw Error('unreachable'); return respond(path, init) },
  }
  return { value, calls }
}

async function rejection(work: Promise<unknown>): Promise<Error> {
  try { await work } catch (error) { return error as Error }
  throw Error('did not reject')
}

Deno.test('control-panel calls go through the channel they are given', async () => {
  const { value, calls } = channel((method) => (method === 'agent.list' ? { agents: [{ agent_did: 'did:bns:a' }, {}, { agent_did: 'x' }] } : method === 'user.get' ? { profile: { did: 'did:bns:me' } } : { total: 0, apps: [] }))
  await listApps(value)
  await listApps(value, { userId: 'bob' })
  await readSystemMetrics(value)
  equal(await readAgentDids(value), ['did:bns:a'])
  equal(await readAccountDid(value, 'me'), 'did:bns:me')
  equal(calls, [
    ['control-panel', 'apps.list', {}],
    ['control-panel', 'apps.list', { user_id: 'bob' }],
    ['control-panel', 'system.metrics', { lite: true }],
    ['control-panel', 'agent.list', {}],
    ['control-panel', 'user.get', { user_id: 'me' }],
  ])
  equal((await rejection(listApps(channel(() => null).value))).message, 'Invalid apps.list response')
})

Deno.test('an app launch is used only when its callback is on the page it was asked for', async () => {
  const page = 'https://notes.zone/x'
  const ask = (data: unknown, target: Parameters<typeof prepareAppLaunch>[1] = { appInstanceId: 'notes@me' }) => {
    const fake = channel(() => data)
    return prepareAppLaunch(fake.value, target, page).then((result) => [result, fake.calls[0]])
  }
  const ready = { status: 'ready', callback_url: 'https://notes.zone/sso_callback?nonce=1' }
  equal(await ask(ready), [ready, ['control-panel', 'auth.prepare_app_launch', { app_instance_id: 'notes@me', redirect_url: page }]])
  equal((await ask(ready, { serviceId: 'aiworkspace' }))[1], ['control-panel', 'auth.prepare_app_launch', { service_id: 'aiworkspace', redirect_url: page }])
  equal((await ask({ status: 'ready', callback_url: 'https://evil.example/' }))[0], null)
  equal((await ask({ status: 'access_denied' }))[0], { status: 'access_denied' })
  equal((await ask({ status: 'whatever' }))[0], null)
  equal((await ask(null))[0], null, 'a failed call is transient')
})

Deno.test('the desktop state is read and written field by field', async () => {
  const ok = (data: unknown) => new Response(JSON.stringify({ ok: true, data }))
  const read = channel(() => null, () => ok({ value: { accent: '#112233' }, exists: true }))
  equal(await readDesktopState(read.value, 'appearance'), { value: { accent: '#112233' }, exists: true })
  equal(read.calls, [['/api/desktop', 'POST', { action: 'state.get', session_id: 'default', state_key: 'appearance' }]])
  equal(await readDesktopState(channel(() => null, () => ok({ value: null })).value, 'appearance'), { value: null, exists: false })
  const write = channel(() => null, () => ok({}))
  await writeDesktopState(write.value, 'appearance', null, 'accent')
  equal(write.calls, [['/api/desktop', 'POST', { action: 'state.set', session_id: 'default', state_key: 'appearance', json_path: 'accent', value: null }]])

  const refused = await rejection(readDesktopState(channel(() => null, () => new Response(JSON.stringify({ ok: false, error: 'authentication required' }), { status: 401 })).value, 'appearance'))
  equal([refused instanceof DesktopApiError, (refused as DesktopApiError).status, refused.message], [true, 401, 'authentication required'])
  equal(((await rejection(readDesktopState(channel(() => null).value, 'appearance'))) as DesktopApiError).status, 0, 'not reached')
})

Deno.test('a Zone colour is #RRGGBB, with text that reads on it', () => {
  equal([normalizeAccent('#AABBCC'), normalizeAccent('#abc'), normalizeAccent('red'), normalizeAccent(null)], ['#aabbcc', null, null, null])
  equal(accentPresets.map(normalizeAccent), accentPresets, 'the presets are stored as they are')
  equal(accentPresets.map(accentText), accentPresets.map(() => '#ffffff'))
  equal([accentText('#ffe14d'), accentText('#1b1b1a')], ['#1b1b1a', '#ffffff'])
})
