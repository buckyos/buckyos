import { namelib, parseSessionTokenClaims } from 'buckyos'
import {
  authenticatedSession,
  AuthenticationSession,
  type AuthenticationTransport,
} from '../core/auth.ts'
import { assert, assertEquals, assertRejects, jwt, testConfig } from './test_helpers.ts'

Deno.test('external session keeps token appid and principal', () => {
  const token = jwt({
    sub: 'alice',
    appid: 'jarvis',
    app_instance_id: 'jarvis@alice',
    exp: Math.floor(Date.now() / 1_000) + 600,
  })
  const session = authenticatedSession(token, 'session-token', false)
  assertEquals(session.principal.id, 'alice')
  assertEquals(session.principal.appId, 'jarvis')
  assertEquals(session.principal.appInstanceId, 'jarvis@alice')
  assertEquals(session.renewable, false)
})

Deno.test('expired external session returns stable SESSION_EXPIRED', async () => {
  const token = jwt({
    sub: 'alice',
    appid: 'jarvis',
    exp: Math.floor(Date.now() / 1_000) - 1,
  })
  await assertRejects(
    () => authenticatedSession(token, 'session-token', false),
    'SESSION_EXPIRED',
  )
})

Deno.test('session token file is reread on reconnect', async () => {
  const root = await Deno.makeTempDir()
  try {
    const path = `${root}/token.jwt`
    const first = jwt({ sub: 'alice', appid: 'jarvis', exp: Math.floor(Date.now() / 1_000) + 600 })
    const second = jwt({ sub: 'bob', appid: 'jarvis', exp: Math.floor(Date.now() / 1_000) + 600 })
    await Deno.writeTextFile(path, first)
    const authentication = new AuthenticationSession(
      testConfig({ configDir: root, sessionTokenFile: path }),
      {},
    )
    assertEquals((await authentication.connect()).principal.id, 'alice')
    await Deno.writeTextFile(path, second)
    assertEquals((await authentication.reconnect()).principal.id, 'bob')
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('identity login reads the new IdentityRoots layout and exchanges a signed JWT', async () => {
  const root = await Deno.makeTempDir()
  try {
    const publicRoot = `${root}/identity`
    const securityRoot = `${root}/security`
    const did = 'did:bns:alice'
    const directory = namelib.DID.fromStr(did).toFilename()
    await Deno.mkdir(`${publicRoot}/${directory}`, { recursive: true })
    await Deno.mkdir(`${securityRoot}/${directory}`, { recursive: true })
    await Deno.writeTextFile(
      `${publicRoot}/${directory}/did.json`,
      JSON.stringify({ id: did, name: 'alice' }),
    )
    const keyPair = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify'])
    const pkcs8 = new Uint8Array(await crypto.subtle.exportKey('pkcs8', keyPair.privateKey))
    await Deno.writeTextFile(
      `${securityRoot}/${directory}/authentication.private.pem`,
      pem(pkcs8),
    )

    let exchangedJwt = ''
    const finalToken = jwt({
      sub: 'alice',
      appid: 'buckycli',
      iss: 'verify-hub',
      exp: Math.floor(Date.now() / 1_000) + 600,
    })
    const transport: AuthenticationTransport = {
      loginByJwt: (_url, loginJwt) => {
        exchangedJwt = loginJwt
        return Promise.resolve(finalToken)
      },
      loginByPassword: () => Promise.reject(new Error('unexpected password login')),
    }
    const authentication = new AuthenticationSession(
      testConfig({
        configDir: root,
        identity: did,
        identityRoot: publicRoot,
        securityRoot,
        nonInteractive: true,
      }),
      {},
      { transport },
    )
    const session = await authentication.connect()
    assertEquals(session.principal.authentication, 'identity')
    assertEquals(session.principal.appId, 'buckycli')
    assert(exchangedJwt.length > 0)
    const claims = parseSessionTokenClaims(exchangedJwt)
    assertEquals(claims?.sub, 'alice')
    assertEquals(claims?.iss, 'alice')
    assertEquals(claims?.appid, 'buckycli')
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

function pem(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  const encoded = btoa(binary).match(/.{1,64}/g)!.join('\n')
  return `-----BEGIN PRIVATE KEY-----\n${encoded}\n-----END PRIVATE KEY-----\n`
}
