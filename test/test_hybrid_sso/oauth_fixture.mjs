// Node's built-in TLS, crypto and HTTP APIs keep the provider fixture self-contained.
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { createServer } from 'node:https'
import { pathToFileURL } from 'node:url'

const random = () => randomBytes(32).toString('base64url')

export function startOAuthFixture({ key, cert, bind, port, oidcOrigin, oauthOrigin, callbackZone, secret }) {
  const providers = new Map([
    [new URL(oidcOrigin).host, 'oidc'],
    [new URL(oauthOrigin).host, 'oauth2'],
  ])
  for (const origin of [oidcOrigin, oauthOrigin]) {
    const url = new URL(origin)
    if (url.protocol !== 'https:' || !url.hostname.endsWith('.test') || origin !== url.origin) {
      throw new Error('Use separate HTTPS .test origins for the fixture providers')
    }
  }
  if (providers.size !== 2 || !secret) throw new Error('Two provider hosts and a fixture secret are required')
  const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 })
  const jwk = { ...publicKey.export({ format: 'jwk' }), use: 'sig', alg: 'RS256', kid: 'fixture' }
  const pending = new Map()
  function store(kind, value) {
    const id = random()
    for (const [old, record] of pending) if (record.expires < Date.now()) pending.delete(old)
    if (pending.size >= 512) throw new Error('Fixture full')
    pending.set(id, { ...value, kind, expires: Date.now() + 180000 })
    return id
  }
  function take(kind, id, provider) {
    const record = pending.get(id)
    if (!record || record.kind !== kind || record.provider !== provider || record.expires < Date.now())
      throw new Error('Invalid attempt')
    pending.delete(id)
    return record
  }
  async function form(req) {
    let body = ''
    for await (const chunk of req) {
      body += chunk
      if (body.length > 16384) throw new Error('Form too long')
    }
    return new URLSearchParams(body)
  }
  const server = createServer({ key, cert }, async (req, res) => {
    res.setHeader('Cache-Control', 'no-store')
    res.setHeader('Referrer-Policy', 'no-referrer')
    try {
      const provider = providers.get(req.headers.host)
      if (!provider) throw new Error('Unknown fixture host')
      const origin = provider === 'oidc' ? oidcOrigin : oauthOrigin
      const url = new URL(req.url, origin)
      const sourceId = 'dv-' + provider
      const callback = 'https://' + callbackZone + '/sso/oauth/' + sourceId + '/callback'
      if (req.method === 'GET' && url.pathname === '/authorize') {
        const p = url.searchParams
        if (
          p.get('response_type') !== 'code' ||
          p.get('client_id') !== sourceId ||
          p.get('redirect_uri') !== callback ||
          p.get('code_challenge_method') !== 'S256' ||
          !p.get('code_challenge') ||
          !p.get('state') ||
          (provider === 'oidc' && !p.get('nonce'))
        )
          throw new Error('Invalid test client')
        const id = store('request', {
          provider,
          state: p.get('state'),
          nonce: p.get('nonce'),
          challenge: p.get('code_challenge'),
        })
        res.setHeader('Content-Type', 'text/html; charset=utf-8')
        res.setHeader('Content-Security-Policy', "default-src 'none'; form-action 'self' https://" + callbackZone)
        res.end(
          '<title>OAuth test provider</title><form method="post"><input type="hidden" name="request" value="' +
            id +
            '"><label>Test identity <select name="person"><option>alice</option><option>renamed</option><option>replacement</option><option>unverified</option></select></label><button>Continue test sign-in</button><button name="decision" value="cancel">Cancel</button></form>',
        )
        return
      }
      if (req.method === 'POST' && url.pathname === '/authorize') {
        const p = await form(req)
        const request = take('request', p.get('request'), provider)
        if (p.get('decision') === 'cancel') {
          const location = new URL(callback)
          location.searchParams.set('state', request.state)
          location.searchParams.set('error', 'access_denied')
          res.writeHead(303, { Location: location.href }).end()
          return
        }
        const person = p.get('person')
        if (!['alice', 'renamed', 'replacement', 'unverified'].includes(person))
          throw new Error('Unknown test subject')
        const profile = {
          sub: person === 'replacement' ? '2' : '1',
          email: person === 'renamed' ? 'changed@company.test' : 'alice@company.test',
          email_verified: person !== 'unverified',
        }
        const code = store('code', { provider, nonce: request.nonce, challenge: request.challenge, profile })
        const to = new URL(callback)
        to.searchParams.set('state', request.state)
        to.searchParams.set('code', code)
        if (provider === 'oidc') to.searchParams.set('iss', origin)
        res.writeHead(303, { Location: to.href }).end()
        return
      }
      res.setHeader('Content-Type', 'application/json')
      if (req.method === 'GET' && url.pathname === '/keys' && provider === 'oidc') {
        res.end(JSON.stringify({ keys: [jwk] }))
        return
      }
      if (req.method === 'POST' && url.pathname === '/token') {
        const p = await form(req)
        const code = take('code', p.get('code'), provider)
        if (
          p.get('client_id') !== sourceId ||
          p.get('client_secret') !== secret ||
          p.get('grant_type') !== 'authorization_code' ||
          p.get('redirect_uri') !== callback ||
          createHash('sha256')
            .update(p.get('code_verifier') ?? '')
            .digest('base64url') !== code.challenge
        )
          throw new Error('Invalid exchange')
        const token = { token_type: 'Bearer', access_token: store('access', { provider, profile: code.profile }) }
        if (provider === 'oidc') {
          const now = Math.floor(Date.now() / 1000)
          const encode = (value) => Buffer.from(JSON.stringify(value)).toString('base64url')
          const unsigned =
            encode({ alg: 'RS256', kid: 'fixture', typ: 'JWT' }) +
            '.' +
            encode({ iss: origin, aud: sourceId, nonce: code.nonce, exp: now + 60, iat: now, ...code.profile })
          token.id_token =
            unsigned + '.' + sign('RSA-SHA256', Buffer.from(unsigned), privateKey).toString('base64url')
        }
        res.end(JSON.stringify(token))
        return
      }
      if (req.method === 'GET' && url.pathname === '/me' && provider === 'oauth2') {
        const access = take('access', req.headers.authorization?.replace(/^Bearer /, ''), provider)
        res.end(JSON.stringify(access.profile))
        return
      }
      res.writeHead(404).end('{}')
    } catch {
      res.writeHead(400, { 'Content-Type': 'application/json' }).end('{"error":"invalid_request"}')
    }
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, bind, () => resolve(server))
  })
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const env = process.env
  const oidcOrigin = env.BUCKYOS_DV_OIDC_ORIGIN || 'https://oidc.sso-dv.test:8443'
  const oauthOrigin = env.BUCKYOS_DV_OAUTH2_ORIGIN || 'https://oauth2.sso-dv.test:8443'
  if (!env.BUCKYOS_DV_OAUTH_ZONE || !env.BUCKYOS_DV_OAUTH_SECRET_FILE)
    throw new Error('A dedicated fixture Zone and secret file are required')
  await startOAuthFixture({
    key: readFileSync(env.BUCKYOS_DV_OAUTH_KEY),
    cert: readFileSync(env.BUCKYOS_DV_OAUTH_CERT),
    bind: env.BUCKYOS_DV_OAUTH_BIND || '127.0.0.1',
    port: Number(new URL(oidcOrigin).port || 443),
    oidcOrigin,
    oauthOrigin,
    callbackZone: env.BUCKYOS_DV_OAUTH_ZONE,
    secret: readFileSync(env.BUCKYOS_DV_OAUTH_SECRET_FILE, 'utf8').trimEnd(),
  })
  console.log('OAuth DV fixture listening on ' + oidcOrigin + ' and ' + oauthOrigin)
}
