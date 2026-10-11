import {
  didDisplayName,
  pickLocalized,
  presentAgent,
  presentApp,
  presentDidDocument,
  presentUser,
  presentZone,
  resolveImageReference,
  usableImageUrl,
} from '../../src/presentation/model.ts'
import { FAILED_RETRY_MS, PresentationStore, fetchDidDocument } from '../../src/presentation/store.ts'
import { imageDrawPlan, checkImageFile, ImageFileError } from '../../src/presentation/imageFile.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

Deno.test('DID fallback names come from the DID parser, per method', () => {
  assertEquals(didDisplayName('did:bns:myapp'), 'myapp', 'bns')
  assertEquals(didDisplayName('did:bns:assistant.alice#main_key'), 'assistant.alice', 'fragment dropped')
  assertEquals(didDisplayName('did:web:example.com'), 'example.com', 'web host')
  assertEquals(didDisplayName('did:web:example.com%3A8443:users:alice'), 'example.com:8443/users/alice', 'web path')
  assertEquals(didDisplayName('did:web:example.com/profile?x=1'), 'example.com', 'did url path and query')
  assertEquals(didDisplayName('did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK'), 'z6MkhaXg…ta2doK', 'key abbreviated')
  assertEquals(didDisplayName('did:dev:abc'), 'abc', 'other methods keep the id')
  assertEquals(didDisplayName('not-a-did'), 'not-a-did', 'non-DID text is kept')
})

Deno.test('localized values fall back from locale to base language, default language, then key order', () => {
  const title = { 'zh-CN': '笔记', zh: '笔记本', en: 'Notes', fr: 'Notes FR' }
  assertEquals(pickLocalized(title, 'zh-CN'), '笔记', 'exact locale')
  assertEquals(pickLocalized({ zh: '笔记本', en: 'Notes' }, 'zh-CN'), '笔记本', 'base language')
  assertEquals(pickLocalized({ en: 'Notes', fr: 'Notes FR' }, 'ja'), 'Notes', 'default language')
  assertEquals(pickLocalized({ fr: 'Notes FR', de: '  ', ar: 7 as unknown as string }, 'ja'), 'Notes FR', 'first non-empty by key')
  assertEquals(pickLocalized('Plain', 'ja'), 'Plain', 'plain string')
  assertEquals(pickLocalized({ en: ' ' }, 'en'), undefined, 'blank only')
})

Deno.test('zone presentation: settings → document → DID → host → default', () => {
  const zone = presentZone({
    zone_did: 'did:bns:home',
    hostname: 'home.web3.buckyos.ai',
    settings: { display_name: '  我的家庭云 ', icon: 'data:image/png;base64,AAAA' },
    document: { display_name: 'Doc Name', description: 'From doc', icon: 'https://doc/icon.png' },
  })
  assertEquals([zone.title, zone.description, zone.icons], ['我的家庭云', 'From doc', ['data:image/png;base64,AAAA', 'https://doc/icon.png']], 'explicit settings first')
  const cleared = presentZone({ zone_did: 'did:bns:home', settings: { display_name: '' }, document: {} })
  assertEquals([cleared.title, cleared.description, cleared.icons], ['home', undefined, []], 'DID fallback, no fake description')
  assertEquals(presentZone(null, { fallbackHost: 'test.buckyos.io' }).title, 'test.buckyos.io', 'host fallback')
  assertEquals(presentZone(null).title, 'BuckyOS', 'default title')
})

Deno.test('user presentation never treats a job title as the name and skips non-string fields', () => {
  const user = presentUser({
    userId: 'alice',
    profile: { did: 'did:bns:alice', display_name: 42, title: 'CTO', name: ' ', bio: ' ', headline: 'Builder', avatar: 'javascript:alert(1)' },
    document: { id: 'did:bns:alice', full_name: 'Alice Liu', avatar: 'https://doc/a.png' },
  })
  assertEquals([user.key, user.title, user.description, user.icons], ['did:bns:alice', 'Alice Liu', 'Builder', ['https://doc/a.png']], 'document fills gaps')
  const bare = presentUser({ did: 'did:web:carol.example.com' })
  assertEquals([bare.title, bare.icons], ['carol.example.com', []], 'DID fallback')
  assertEquals(presentUser({ userId: 'dave' }).title, 'dave', 'user id when no DID')
})

Deno.test('agent and app presentations follow their own candidate chains', () => {
  const agent = presentAgent({ did: 'did:bns:xiaobai.alice', name: 'xiaobai', profile: { display_name: ' ', avatar: 'data:image/jpeg;base64,BB', bio: '' }, publicDescription: 'Helps Alice' })
  assertEquals([agent.title, agent.description, agent.icons], ['xiaobai', 'Helps Alice', ['data:image/jpeg;base64,BB']], 'agent')

  const app = presentApp({
    key: 'notes@alice',
    appId: 'notes',
    appDid: 'did:bns:notes',
    showName: 'Notes App',
    presentation: { title: { en: 'Notes', 'zh-CN': '笔记' }, summary: { en: 'Take notes' }, description: { en: 'Long description' } },
    iconUrl: 'res/icon.png',
    iconBase: 'https://notes.home.example/',
  }, 'zh-CN')
  assertEquals([app.key, app.did, app.title, app.description, app.icons], ['notes@alice', 'did:bns:notes', '笔记', 'Take notes', ['https://notes.home.example/res/icon.png']], 'app')

  const echo = presentApp({ key: 'x@alice', appId: 'x', showName: 'X', presentation: { title: {}, summary: {}, description: { en: 'X' } } }, 'en')
  assertEquals([echo.title, echo.description], ['X', undefined], 'a description that repeats the title is dropped')
  assertEquals(presentApp({ key: 'myapp@alice', appDid: 'did:bns:myapp', showName: ' ' }, 'en').title, 'myapp', 'DID fallback')
})

Deno.test('image references: usable URLs pass, relative ones need a base, scripts never load', () => {
  assertEquals(usableImageUrl('//evil.example/x.png'), undefined, 'protocol-relative')
  assertEquals(usableImageUrl('/favicon.svg'), '/favicon.svg', 'same origin path')
  assertEquals(resolveImageReference('icon.png'), undefined, 'relative without base')
  assertEquals(resolveImageReference('javascript:alert(1)', 'https://a.example/'), undefined, 'other schemes')
})

Deno.test('DID documents project by their own shape', () => {
  const owner = presentDidDocument('did:bns:alice', { id: 'did:bns:alice', name: 'alice', display_name: 'Alice', avatar: 'https://a/png' }, 'en')
  assertEquals([owner.kind, owner.title, owner.icons], ['user', 'Alice', ['https://a/png']], 'owner')
  const agent = presentDidDocument('did:bns:bot.alice', { id: 'did:bns:bot.alice', owner: 'did:bns:alice', contact: {}, public_description: 'Bot' }, 'en')
  assertEquals([agent.kind, agent.title, agent.description], ['agent', 'bot.alice', 'Bot'], 'agent')
  const zone = presentDidDocument('did:bns:home', { id: 'did:bns:home', owner: 'did:bns:alice', oods: [], hostname: 'home.example', display_name: 'Home' }, 'en')
  assertEquals([zone.kind, zone.title], ['zone', 'Home'], 'zone')
})

Deno.test('the cache dedupes loads, retries failures later, and is isolated per Zone and viewer', async () => {
  let now = 1_000
  const store = new PresentationStore(() => now)
  store.setScope('home|alice')
  let calls = 0
  let fail = true
  const load = async () => {
    calls += 1
    return presentUser({ did: 'did:bns:bob', profile: { display_name: fail ? undefined : 'Bob' }, status: fail ? 'failed' : 'resolved' })
  }
  await Promise.all([store.ensure('did:bns:bob', load), store.ensure('did:bns:bob', load)])
  assertEquals(calls, 1, 'concurrent loads share one request')
  assertEquals(store.get('did:bns:bob')?.status, 'failed', 'failure cached')
  await store.ensure('did:bns:bob', load)
  assertEquals(calls, 1, 'no immediate retry')
  fail = false
  now += FAILED_RETRY_MS
  await store.ensure('did:bns:bob', load)
  assertEquals([calls, store.get('did:bns:bob')?.title], [2, 'Bob'], 'retried after the delay')
  await store.ensure('did:bns:bob', load)
  assertEquals(calls, 2, 'resolved entries are reused')

  store.setScope('home|carol')
  assertEquals(store.get('did:bns:bob'), undefined, 'another viewer starts empty')
  const slow = store.ensure('did:bns:bob', load)
  store.setScope('other|carol')
  await slow
  assertEquals(store.get('did:bns:bob'), undefined, 'a load from the previous scope is discarded')
})

Deno.test('DID documents are read from the Zone resolver, bare or in a resolution envelope', async () => {
  const responses: Record<string, unknown> = {
    '/1.0/identifiers/did%3Abns%3Aalice': { didDocument: { id: 'did:bns:alice', display_name: 'Alice' } },
    '/1.0/identifiers/did%3Abns%3Ahome': { id: 'did:bns:home', oods: [] },
  }
  const fetcher = (async (url: string) => {
    const body = responses[url]
    return new Response(body ? JSON.stringify(body) : 'missing', { status: body ? 200 : 404 })
  }) as typeof fetch
  assertEquals(await fetchDidDocument('did:bns:alice', fetcher), { id: 'did:bns:alice', display_name: 'Alice' }, 'envelope')
  assertEquals(await fetchDidDocument('did:bns:home', fetcher), { id: 'did:bns:home', oods: [] }, 'bare')
  let failed = false
  await fetchDidDocument('did:bns:nobody', fetcher).catch(() => { failed = true })
  assertEquals(failed, true, '404 rejects')
})

Deno.test('picked images are checked and fitted before encoding', () => {
  assertEquals(imageDrawPlan(400, 200, { maxPx: 192, fit: 'square' }), { width: 192, height: 192, sx: 100, sy: 0, sw: 200, sh: 200 }, 'square crop')
  assertEquals(imageDrawPlan(1024, 512, { maxPx: 256, fit: 'contain' }), { width: 256, height: 128, sx: 0, sy: 0, sw: 1024, sh: 512 }, 'contain keeps aspect')
  assertEquals(imageDrawPlan(64, 32, { maxPx: 256, fit: 'contain' }).width, 64, 'small images are not upscaled')
  const reasons: string[] = []
  for (const file of [{ type: 'image/svg+xml', size: 10 }, { type: 'image/png', size: 30 * 1024 * 1024 }]) {
    try {
      checkImageFile(file)
    } catch (error) {
      reasons.push((error as ImageFileError).reason)
    }
  }
  assertEquals(reasons, ['type', 'too_large'], 'rejections')
})
