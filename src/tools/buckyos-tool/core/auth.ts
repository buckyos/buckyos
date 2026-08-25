import { buckyos, createAppInstanceId, parseSessionTokenClaims, VerifyHubClient } from 'buckyos'
import { type Environment, readEnvironment, type ResolvedConfig } from './config.ts'
import { EXIT_AUTH, ToolError } from './errors.ts'
import { resolveIdentityMaterial } from './identity.ts'
import { resolveServiceUrl } from './runtime.ts'

type AuthTarget =
  | { kind: 'app'; app_instance_id: string }
  | { kind: 'system'; service_id: string }

export type AuthenticationKind =
  | 'session-token'
  | 'session-token-file'
  | 'environment'
  | 'identity'
  | 'password'
  | 'mock'

export interface ResolvedPrincipal {
  id: string
  appId: string
  appInstanceId?: string
  authentication: AuthenticationKind
  tokenExpiresAt?: string
}

export interface AuthenticatedSession {
  token: string
  principal: ResolvedPrincipal
  claims: Record<string, unknown>
  renewable: boolean
}

export interface AuthenticationTransport {
  loginByJwt(url: string, jwt: string, target: AuthTarget, timeoutMs: number): Promise<string>
  loginByPassword(
    url: string,
    username: string,
    password: string,
    target: AuthTarget,
    timeoutMs: number,
  ): Promise<string>
}

export interface AuthenticationDependencies {
  transport?: AuthenticationTransport
  readPassword?: (prompt: string) => Promise<string>
  readUsername?: (prompt: string) => Promise<string>
  now?: () => number
}

export interface SessionController {
  readonly config: ResolvedConfig
  connect(): Promise<AuthenticatedSession>
  ensureValid(): Promise<AuthenticatedSession>
  reconnect(): Promise<AuthenticatedSession>
  current(): AuthenticatedSession
  status(): Record<string, unknown>
}

const LOGIN_APP_ID = 'buckycli'
const LOGIN_TOKEN_TTL_SECONDS = 10 * 60

export class AuthenticationSession implements SessionController {
  readonly config: ResolvedConfig
  readonly #environment: Environment
  readonly #transport: AuthenticationTransport
  readonly #readPassword: (prompt: string) => Promise<string>
  readonly #readUsername: (prompt: string) => Promise<string>
  readonly #now: () => number
  #session?: AuthenticatedSession

  constructor(
    config: ResolvedConfig,
    environment: Environment = readEnvironment(),
    dependencies: AuthenticationDependencies = {},
  ) {
    this.config = config
    this.#environment = environment
    this.#transport = dependencies.transport ?? new SdkAuthenticationTransport()
    this.#readPassword = dependencies.readPassword ?? readSecret
    this.#readUsername = dependencies.readUsername ?? readVisible
    this.#now = dependencies.now ?? Date.now
  }

  async connect(): Promise<AuthenticatedSession> {
    if (this.#session) return this.#session
    this.#session = await this.#authenticate(false)
    return this.#session
  }

  async ensureValid(): Promise<AuthenticatedSession> {
    const session = await this.connect()
    const exp = numberClaim(session.claims.exp)
    const nowSeconds = Math.floor(this.#now() / 1_000)
    if (exp === undefined || exp > nowSeconds + 15) return session
    if (!session.renewable) {
      throw new ToolError(
        'SESSION_EXPIRED',
        'the externally supplied session token has expired',
        EXIT_AUTH,
      )
    }
    this.#session = await this.#authenticate(true)
    return this.#session
  }

  async reconnect(): Promise<AuthenticatedSession> {
    this.#session = await this.#authenticate(true)
    return this.#session
  }

  current(): AuthenticatedSession {
    if (!this.#session) {
      throw new ToolError('AUTH_REQUIRED', 'session is not initialized', EXIT_AUTH)
    }
    return this.#session
  }

  status(): Record<string, unknown> {
    const session = this.current()
    const expiresAt = session.principal.tokenExpiresAt
    const exp = numberClaim(session.claims.exp)
    const remainingSeconds = exp === undefined
      ? null
      : Math.max(0, exp - Math.floor(this.#now() / 1_000))
    return {
      authenticated: true,
      principal: session.principal.id,
      appid: session.principal.appId,
      app_instance_id: session.principal.appInstanceId ?? null,
      authentication: session.principal.authentication,
      renewable: session.renewable,
      expires_at: expiresAt ?? null,
      remaining_seconds: remainingSeconds,
    }
  }

  async #authenticate(reconnect: boolean): Promise<AuthenticatedSession> {
    if (this.config.sessionToken) {
      const source = this.config.sources.session_token === 'environment'
        ? 'environment'
        : 'session-token'
      return externalSession(this.config.sessionToken, source, this.#now())
    }

    if (this.config.sessionTokenFile) {
      let token: string
      try {
        token = (await Deno.readTextFile(this.config.sessionTokenFile)).trim()
      } catch (error) {
        throw new ToolError(
          'SESSION_TOKEN_FILE_ERROR',
          `failed to read session token file: ${
            error instanceof Error ? error.message : String(error)
          }`,
          EXIT_AUTH,
        )
      }
      if (!token) {
        throw new ToolError('INVALID_SESSION_TOKEN', 'session token file is empty', EXIT_AUTH)
      }
      return externalSession(token, 'session-token-file', this.#now())
    }

    const injected = this.#environment.BUCKYOS_APPCLIENT_SESSION_TOKEN?.trim()
    if (injected) return externalSession(injected, 'environment', this.#now())

    if (this.config.identity) {
      try {
        const material = await resolveIdentityMaterial(
          this.config.identity,
          this.config,
          this.#environment,
        )
        const loginJwt = await createLoginJwt(
          material.subject,
          material.issuer,
          material.privateKeyPem,
          this.#now(),
        )
        const token = await this.#transport.loginByJwt(
          resolveServiceUrl(this.config, 'verify-hub'),
          loginJwt,
          identityAuthTarget(material.principalKind, material.subject),
          this.config.timeoutMs,
        )
        return authenticatedSession(token, 'identity', true, this.#now())
      } catch (error) {
        if (!(error instanceof ToolError) || !['IDENTITY_NOT_FOUND'].includes(error.code)) {
          throw error
        }
      }
    }

    if (this.config.nonInteractive) {
      throw new ToolError(
        'AUTH_REQUIRED',
        'no session token or usable identity is available in non-interactive mode',
        EXIT_AUTH,
      )
    }
    const username = this.config.identity?.startsWith('did:')
      ? await this.#readUsername('Username: ')
      : this.config.identity ?? await this.#readUsername('Username: ')
    if (!username.trim()) throw new ToolError('AUTH_REQUIRED', 'username is required', EXIT_AUTH)
    const password = await this.#readPassword(reconnect ? 'Password (reconnect): ' : 'Password: ')
    const token = await this.#transport.loginByPassword(
      resolveServiceUrl(this.config, 'verify-hub'),
      username.trim(),
      password,
      appAuthTarget(username.trim()),
      this.config.timeoutMs,
    )
    return authenticatedSession(token, 'password', true, this.#now())
  }
}

export class SdkAuthenticationTransport implements AuthenticationTransport {
  async loginByJwt(
    url: string,
    jwt: string,
    target: AuthTarget,
    timeoutMs: number,
  ): Promise<string> {
    const client = new buckyos.kRPCClient(url)
    const response = await withLocalTimeout(
      new VerifyHubClient(client).loginByJwt({ jwt, target }),
      timeoutMs,
    )
    if (!response.session_token) throw new Error('verify-hub returned no session token')
    return response.session_token
  }

  async loginByPassword(
    url: string,
    username: string,
    password: string,
    target: AuthTarget,
    timeoutMs: number,
  ): Promise<string> {
    const client = new buckyos.kRPCClient(url)
    const verifyHub = new VerifyHubClient(client)
    const nonce = Date.now()
    verifyHub.setSeq(nonce)
    const response = await withLocalTimeout(
      verifyHub.loginByPassword({
        username,
        password: (buckyos.hashPassword as unknown as (
          username: string,
          password: string,
          nonce?: number | null,
        ) => string)(username, password, nonce),
        target,
        login_nonce: nonce,
      }),
      timeoutMs,
    )
    const normalized = VerifyHubClient.normalizeLoginResponse(response)
    if (!normalized.session_token) throw new Error('verify-hub returned no session token')
    return normalized.session_token
  }
}

function identityAuthTarget(
  principalKind: 'user' | 'device',
  subject: string,
): AuthTarget {
  return principalKind === 'device'
    ? { kind: 'system', service_id: LOGIN_APP_ID }
    : appAuthTarget(subject)
}

function appAuthTarget(ownerUserId: string): AuthTarget {
  return {
    kind: 'app',
    app_instance_id: createAppInstanceId(LOGIN_APP_ID, ownerUserId),
  }
}

export function authenticatedSession(
  token: string,
  authentication: AuthenticationKind,
  renewable: boolean,
  nowMs = Date.now(),
): AuthenticatedSession {
  const claims = parseClaims(token)
  const exp = numberClaim(claims.exp)
  const nowSeconds = Math.floor(nowMs / 1_000)
  if (exp !== undefined && exp <= nowSeconds) {
    throw new ToolError('SESSION_EXPIRED', 'the session token has expired', EXIT_AUTH)
  }
  const id = stringClaim(claims.sub) ?? stringClaim(claims.userid)
  const appId = stringClaim(claims.appid) ?? stringClaim(claims.aud)
  if (!id || !appId) {
    throw new ToolError(
      'INVALID_SESSION_TOKEN',
      'session token is missing principal or appid claims',
      EXIT_AUTH,
    )
  }
  const appInstanceId = stringClaim(claims.app_instance_id) ??
    stringClaim((claims.extra as Record<string, unknown> | undefined)?.app_instance_id)
  return {
    token,
    claims,
    renewable,
    principal: {
      id,
      appId,
      appInstanceId,
      authentication,
      tokenExpiresAt: exp === undefined ? undefined : new Date(exp * 1_000).toISOString(),
    },
  }
}

function externalSession(
  token: string,
  authentication: Extract<
    AuthenticationKind,
    'session-token' | 'session-token-file' | 'environment'
  >,
  nowMs: number,
): AuthenticatedSession {
  return authenticatedSession(token, authentication, false, nowMs)
}

function parseClaims(token: string): Record<string, unknown> {
  const claims = parseSessionTokenClaims(token)
  if (!claims) {
    throw new ToolError('INVALID_SESSION_TOKEN', 'session token is not a valid JWT', EXIT_AUTH)
  }
  return claims as Record<string, unknown>
}

async function createLoginJwt(
  subject: string,
  issuer: string,
  privateKeyPem: string,
  nowMs: number,
): Promise<string> {
  const now = Math.floor(nowMs / 1_000)
  const header = { alg: 'EdDSA', kid: issuer, typ: 'JWT' }
  const payload = {
    token_type: 'Normal',
    appid: LOGIN_APP_ID,
    jti: crypto.randomUUID(),
    session: now,
    sub: subject,
    userid: subject,
    iss: issuer,
    exp: now + LOGIN_TOKEN_TTL_SECONDS,
    sudo: false,
    extra: {},
  }
  const signingInput = `${base64Url(JSON.stringify(header))}.${base64Url(JSON.stringify(payload))}`
  const keyBytes = pemBytes(privateKeyPem)
  let key: CryptoKey
  try {
    key = await crypto.subtle.importKey(
      'pkcs8',
      keyBytes.slice().buffer as ArrayBuffer,
      { name: 'Ed25519' },
      false,
      ['sign'],
    )
  } catch {
    throw new ToolError(
      'INVALID_PRIVATE_KEY',
      'identity authentication key is not a valid Ed25519 PKCS8 key',
      EXIT_AUTH,
    )
  }
  const signature = await crypto.subtle.sign('Ed25519', key, new TextEncoder().encode(signingInput))
  return `${signingInput}.${base64Url(new Uint8Array(signature))}`
}

function pemBytes(pem: string): Uint8Array {
  const encoded = pem.replace(/-----[^-]+-----/g, '').replaceAll(/\s/g, '')
  const binary = atob(encoded)
  return Uint8Array.from(binary, (character) => character.charCodeAt(0))
}

function base64Url(value: string | Uint8Array): string {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

function stringClaim(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

function numberClaim(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

async function withLocalTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new ToolError('TIMEOUT', 'authentication timed out', 8, true)),
      timeoutMs,
    )
  })
  try {
    return await Promise.race([promise, timeout])
  } finally {
    if (timer !== undefined) clearTimeout(timer)
  }
}

async function readVisible(prompt: string): Promise<string> {
  await Deno.stderr.write(new TextEncoder().encode(prompt))
  const buffer = new Uint8Array(1024)
  const count = await Deno.stdin.read(buffer)
  return count ? new TextDecoder().decode(buffer.subarray(0, count)).trim() : ''
}

async function readSecret(prompt: string): Promise<string> {
  if (!Deno.stdin.isTerminal()) {
    throw new ToolError(
      'INTERACTIVE_AUTH_UNAVAILABLE',
      'password input requires a terminal',
      EXIT_AUTH,
    )
  }
  await Deno.stderr.write(new TextEncoder().encode(prompt))
  const bytes: number[] = []
  Deno.stdin.setRaw(true)
  try {
    const buffer = new Uint8Array(1)
    while (true) {
      const count = await Deno.stdin.read(buffer)
      if (count === null || buffer[0] === 10 || buffer[0] === 13) break
      if (buffer[0] === 3) throw new ToolError('CANCELED', 'password input canceled', 8)
      if (buffer[0] === 8 || buffer[0] === 127) bytes.pop()
      else bytes.push(buffer[0])
    }
  } finally {
    Deno.stdin.setRaw(false)
    await Deno.stderr.write(new TextEncoder().encode('\n'))
  }
  return new TextDecoder().decode(Uint8Array.from(bytes))
}
