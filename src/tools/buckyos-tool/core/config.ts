import { dirname, isAbsolute, join, resolve } from 'node:path'
import { homedir } from 'node:os'
import type { GlobalOptions, OutputFormat } from './argv.ts'
import { parseDuration } from './argv.ts'
import { ToolError, UsageError } from './errors.ts'

export interface ToolConfig {
  schema_version: 1
  default_profile?: string
  output?: OutputFormat
}

export interface ProfileConfig {
  schema_version: 1
  zone?: string
  endpoint?: string
  identity?: string
  default_protocol?: 'http://' | 'https://'
  output?: OutputFormat
}

export interface ImplicitDeviceIdentity {
  did: string
  name: string
  zoneDid: string
  buckyosRoot: string
  nodeIdentityPath: string
  publicRoot: string
  securityRoot: string
}

export interface ResolvedConfig {
  configDir: string
  profileName?: string
  zone?: string
  endpoint?: string
  identity?: string
  identityRoot?: string
  securityRoot?: string
  sessionToken?: string
  sessionTokenFile?: string
  output: OutputFormat
  defaultProtocol: 'http://' | 'https://'
  timeoutMs: number
  traceId?: string
  idempotencyKey?: string
  wait: boolean
  nonInteractive: boolean
  yes: boolean
  noColor: boolean
  verbose: boolean
  implicitDeviceIdentity?: ImplicitDeviceIdentity
  sources: Record<string, string>
}

export type Environment = Record<string, string | undefined>

export const ENVIRONMENT_NAMES = [
  'HOME',
  'USERPROFILE',
  'APPDATA',
  'BUCKYOS_TOOL_CONFIG_DIR',
  'BUCKYOS_TOOL_PROFILE',
  'BUCKYOS_TOOL_ZONE',
  'BUCKYOS_TOOL_ENDPOINT',
  'BUCKYOS_TOOL_IDENTITY',
  'BUCKYOS_TOOL_OUTPUT',
  'BUCKYOS_IDENTITY_ROOT',
  'BUCKYOS_SECURITY_ROOT',
  'BUCKYOS_APPCLIENT_SESSION_TOKEN',
  'BUCKYOS_ROOT',
] as const

const PROFILE_NAME = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/
const DEFAULT_TIMEOUT_MS = 30_000

export class ConfigStore {
  readonly root: string

  constructor(root: string) {
    this.root = resolve(root)
  }

  async readConfig(): Promise<ToolConfig> {
    return await this.#readJson<ToolConfig>(join(this.root, 'config.json'), {
      schema_version: 1,
    })
  }

  async readProfile(name: string): Promise<ProfileConfig | undefined> {
    validateProfileName(name)
    return await this.#readJson<ProfileConfig>(this.profilePath(name), undefined)
  }

  async listProfiles(): Promise<string[]> {
    const directory = join(this.root, 'profiles')
    try {
      const names: string[] = []
      for await (const entry of Deno.readDir(directory)) {
        if (entry.isFile && entry.name.endsWith('.json')) {
          const name = entry.name.slice(0, -5)
          if (PROFILE_NAME.test(name)) names.push(name)
        }
      }
      return names.sort()
    } catch (error) {
      if (error instanceof Deno.errors.NotFound) return []
      throw error
    }
  }

  async writeConfig(config: ToolConfig): Promise<void> {
    validateToolConfig(config, 'config.json')
    await this.#atomicWrite(join(this.root, 'config.json'), config)
  }

  async writeProfile(name: string, profile: ProfileConfig): Promise<void> {
    validateProfileName(name)
    validateProfileConfig(profile, `profiles/${name}.json`)
    await this.#atomicWrite(this.profilePath(name), profile)
  }

  profilePath(name: string): string {
    validateProfileName(name)
    return join(this.root, 'profiles', `${name}.json`)
  }

  historyPath(): string {
    return join(this.root, 'state', 'repl_history')
  }

  async #readJson<T>(path: string, missing: T | undefined): Promise<T> {
    try {
      const parsed = JSON.parse(await Deno.readTextFile(path)) as unknown
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
        throw new UsageError('INVALID_CONFIG', `${path} must contain a JSON object`)
      }
      if ((parsed as Record<string, unknown>).schema_version !== 1) {
        throw new UsageError(
          'UNSUPPORTED_CONFIG_VERSION',
          `${path} has an unsupported schema_version`,
        )
      }
      if (path.endsWith('config.json') && !path.includes('/profiles/')) {
        validateToolConfig(parsed as ToolConfig, path)
      } else {
        validateProfileConfig(parsed as ProfileConfig, path)
      }
      return parsed as T
    } catch (error) {
      if (error instanceof Deno.errors.NotFound && missing !== undefined) return missing
      if (error instanceof SyntaxError) {
        throw new UsageError('INVALID_CONFIG', `${path} is not valid JSON`)
      }
      throw error
    }
  }

  async #atomicWrite(path: string, value: unknown): Promise<void> {
    await Deno.mkdir(dirname(path), { recursive: true, mode: 0o700 })
    const temporary = `${path}.tmp-${Deno.pid}-${crypto.randomUUID()}`
    const body = `${JSON.stringify(value, null, 2)}\n`
    try {
      await Deno.writeTextFile(temporary, body, { mode: 0o600, createNew: true })
      await Deno.rename(temporary, path)
      if (Deno.build.os !== 'windows') await Deno.chmod(path, 0o600)
    } catch (error) {
      try {
        await Deno.remove(temporary)
      } catch (cleanupError) {
        void cleanupError
      }
      throw error
    }
  }
}

export async function resolveConfig(
  explicit: GlobalOptions,
  environment: Environment = readEnvironment(),
  options: { interactive?: boolean; cwd?: string; homeDir?: string } = {},
): Promise<{ resolved: ResolvedConfig; store: ConfigStore }> {
  const cwd = options.cwd ?? Deno.cwd()
  const homeDir = options.homeDir ?? environment.HOME ?? environment.USERPROFILE ?? homedir()
  const configDir = resolveConfigRoot(explicit, environment, cwd, homeDir)
  const store = new ConfigStore(configDir)
  const config = await store.readConfig()
  const profileName = explicit.profile ?? environment.BUCKYOS_TOOL_PROFILE ?? config.default_profile
  const profile = profileName ? await store.readProfile(profileName) : undefined
  if (profileName && !profile) {
    throw new UsageError('PROFILE_NOT_FOUND', `profile not found: ${profileName}`)
  }

  const sources: Record<string, string> = {
    config_dir: sourceOf(
      explicit.configDir,
      environment.BUCKYOS_TOOL_CONFIG_DIR,
      undefined,
      'default',
    ),
  }
  const zone = select(
    'zone',
    explicit.zone,
    environment.BUCKYOS_TOOL_ZONE,
    profile?.zone,
    undefined,
    sources,
  )
  const endpoint = select(
    'endpoint',
    explicit.endpoint,
    environment.BUCKYOS_TOOL_ENDPOINT,
    profile?.endpoint,
    undefined,
    sources,
  )
  const identity = select(
    'identity',
    explicit.identity,
    environment.BUCKYOS_TOOL_IDENTITY,
    profile?.identity,
    undefined,
    sources,
  )
  const identityRoot = select(
    'identity_root',
    explicit.identityRoot,
    environment.BUCKYOS_IDENTITY_ROOT,
    undefined,
    undefined,
    sources,
  )
  const securityRoot = select(
    'security_root',
    explicit.securityRoot,
    environment.BUCKYOS_SECURITY_ROOT,
    undefined,
    undefined,
    sources,
  )
  if (!!explicit.identityRoot !== !!explicit.securityRoot) {
    throw new UsageError(
      'IDENTITY_ROOT_PAIR_REQUIRED',
      '--identity-root and --security-root must be provided together',
    )
  }
  if (
    !!environment.BUCKYOS_IDENTITY_ROOT !== !!environment.BUCKYOS_SECURITY_ROOT &&
    !explicit.identityRoot
  ) {
    throw new UsageError(
      'IDENTITY_ROOT_PAIR_REQUIRED',
      'BUCKYOS_IDENTITY_ROOT and BUCKYOS_SECURITY_ROOT must be provided together',
    )
  }

  let output: OutputFormat
  if (options.interactive && explicit.output === undefined) {
    output = 'table'
    sources.output = 'interactive-default'
  } else {
    output = select(
      'output',
      explicit.output,
      asOutput(environment.BUCKYOS_TOOL_OUTPUT),
      profile?.output,
      config.output ?? 'json',
      sources,
    )!
  }
  const defaultProtocol = profile?.default_protocol ?? inferProtocol(endpoint) ?? 'https://'
  const timeoutMs = explicit.timeout ? parseDuration(explicit.timeout) : DEFAULT_TIMEOUT_MS

  if (endpoint) validateEndpoint(endpoint)
  const sessionToken = explicit.sessionToken ?? environment.BUCKYOS_APPCLIENT_SESSION_TOKEN
  if (sessionToken) sources.session_token = explicit.sessionToken ? 'argument' : 'environment'
  if (profileName) {
    sources.profile = explicit.profile
      ? 'argument'
      : environment.BUCKYOS_TOOL_PROFILE
      ? 'environment'
      : 'config'
  }

  return {
    store,
    resolved: {
      configDir,
      profileName,
      zone,
      endpoint,
      identity,
      identityRoot,
      securityRoot,
      sessionToken,
      sessionTokenFile: explicit.sessionTokenFile,
      output,
      defaultProtocol,
      timeoutMs,
      traceId: explicit.traceId,
      idempotencyKey: explicit.idempotencyKey,
      wait: explicit.wait ?? false,
      nonInteractive: explicit.nonInteractive ?? false,
      yes: explicit.yes ?? false,
      noColor: explicit.noColor ?? false,
      verbose: explicit.verbose ?? false,
      sources,
    },
  }
}

export function resolveConfigRoot(
  explicit: GlobalOptions,
  environment: Environment,
  cwd = Deno.cwd(),
  homeDir = environment.HOME ?? environment.USERPROFILE ?? homedir(),
): string {
  const value = explicit.configDir ?? environment.BUCKYOS_TOOL_CONFIG_DIR ??
    join(homeDir, '.buckyos_tool')
  return isAbsolute(value) ? value : resolve(cwd, value)
}

export function localResolvedConfig(
  explicit: GlobalOptions,
  environment: Environment = readEnvironment(),
  options: { cwd?: string; homeDir?: string; interactive?: boolean } = {},
): { resolved: ResolvedConfig; store: ConfigStore } {
  const cwd = options.cwd ?? Deno.cwd()
  const homeDir = options.homeDir ?? environment.HOME ?? environment.USERPROFILE ?? homedir()
  const configDir = resolveConfigRoot(explicit, environment, cwd, homeDir)
  const output = options.interactive && explicit.output === undefined
    ? 'table'
    : explicit.output ?? asOutput(environment.BUCKYOS_TOOL_OUTPUT) ?? 'json'
  return {
    store: new ConfigStore(configDir),
    resolved: {
      configDir,
      profileName: explicit.profile ?? environment.BUCKYOS_TOOL_PROFILE,
      zone: explicit.zone ?? environment.BUCKYOS_TOOL_ZONE,
      endpoint: explicit.endpoint ?? environment.BUCKYOS_TOOL_ENDPOINT,
      identity: explicit.identity ?? environment.BUCKYOS_TOOL_IDENTITY,
      identityRoot: explicit.identityRoot ?? environment.BUCKYOS_IDENTITY_ROOT,
      securityRoot: explicit.securityRoot ?? environment.BUCKYOS_SECURITY_ROOT,
      sessionToken: explicit.sessionToken ?? environment.BUCKYOS_APPCLIENT_SESSION_TOKEN,
      sessionTokenFile: explicit.sessionTokenFile,
      output,
      defaultProtocol: inferProtocol(explicit.endpoint ?? environment.BUCKYOS_TOOL_ENDPOINT) ??
        'https://',
      timeoutMs: explicit.timeout ? parseDuration(explicit.timeout) : DEFAULT_TIMEOUT_MS,
      traceId: explicit.traceId,
      idempotencyKey: explicit.idempotencyKey,
      wait: explicit.wait ?? false,
      nonInteractive: explicit.nonInteractive ?? false,
      yes: explicit.yes ?? false,
      noColor: explicit.noColor ?? false,
      verbose: explicit.verbose ?? false,
      sources: {
        config_dir: sourceOf(
          explicit.configDir,
          environment.BUCKYOS_TOOL_CONFIG_DIR,
          undefined,
          'default',
        ),
        output: explicit.output
          ? 'argument'
          : environment.BUCKYOS_TOOL_OUTPUT
          ? 'environment'
          : 'default',
      },
    },
  }
}

export function readEnvironment(): Environment {
  return Object.fromEntries(ENVIRONMENT_NAMES.map((name) => [name, Deno.env.get(name)]))
}

export function effectiveConfigView(config: ResolvedConfig): Record<string, unknown> {
  return {
    schema_version: 1,
    config_dir: config.configDir,
    profile: config.profileName ?? null,
    zone: config.zone ?? null,
    endpoint: redactUrl(config.endpoint),
    identity: config.identity ?? null,
    identity_root: config.identityRoot ?? null,
    security_root: config.securityRoot ? '[CONFIGURED]' : null,
    output: config.output,
    default_protocol: config.defaultProtocol,
    timeout_ms: config.timeoutMs,
    session_token: config.sessionToken || config.sessionTokenFile
      ? {
        present: true,
        source: config.sources.session_token ?? (config.sessionTokenFile ? 'file' : 'unknown'),
        summary: '[REDACTED]',
      }
      : { present: false },
    sources: config.sources,
  }
}

export function validateProfileName(name: string): void {
  if (!PROFILE_NAME.test(name)) {
    throw new UsageError('INVALID_PROFILE_NAME', `invalid profile name: ${name}`)
  }
}

function validateToolConfig(config: ToolConfig, source: string): void {
  if (config.schema_version !== 1) {
    throw new UsageError(
      'UNSUPPORTED_CONFIG_VERSION',
      `${source} has an unsupported schema_version`,
    )
  }
  if (config.default_profile !== undefined) validateProfileName(config.default_profile)
  if (config.output !== undefined) asOutput(config.output, source)
  rejectUnknownKeys(config as unknown as Record<string, unknown>, [
    'schema_version',
    'default_profile',
    'output',
  ], source)
}

function validateProfileConfig(profile: ProfileConfig, source: string): void {
  if (profile.schema_version !== 1) {
    throw new UsageError(
      'UNSUPPORTED_CONFIG_VERSION',
      `${source} has an unsupported schema_version`,
    )
  }
  if (profile.output !== undefined) asOutput(profile.output, source)
  if (
    profile.default_protocol !== undefined &&
    !['http://', 'https://'].includes(profile.default_protocol)
  ) {
    throw new UsageError('INVALID_CONFIG', `${source}.default_protocol must be http:// or https://`)
  }
  if (profile.endpoint) validateEndpoint(profile.endpoint)
  rejectUnknownKeys(
    profile as unknown as Record<string, unknown>,
    ['schema_version', 'zone', 'endpoint', 'identity', 'default_protocol', 'output'],
    source,
  )
}

function rejectUnknownKeys(
  value: Record<string, unknown>,
  allowed: string[],
  source: string,
): void {
  const allowedSet = new Set(allowed)
  const unknown = Object.keys(value).filter((key) => !allowedSet.has(key))
  if (unknown.length > 0) {
    throw new UsageError(
      'INVALID_CONFIG',
      `${source} contains unsupported fields: ${unknown.join(', ')}`,
    )
  }
}

function asOutput(
  value: string | undefined,
  source = 'BUCKYOS_TOOL_OUTPUT',
): OutputFormat | undefined {
  if (value === undefined) return undefined
  if (!['json', 'jsonl', 'table', 'text', 'raw'].includes(value)) {
    throw new UsageError('INVALID_OUTPUT_FORMAT', `${source} has invalid output format: ${value}`)
  }
  return value as OutputFormat
}

function select<T>(
  name: string,
  explicit: T | undefined,
  environment: T | undefined,
  profile: T | undefined,
  fallback: T | undefined,
  sources: Record<string, string>,
): T | undefined {
  if (explicit !== undefined) {
    sources[name] = 'argument'
    return explicit
  }
  if (environment !== undefined) {
    sources[name] = 'environment'
    return environment
  }
  if (profile !== undefined) {
    sources[name] = 'profile'
    return profile
  }
  if (fallback !== undefined) sources[name] = 'default'
  return fallback
}

function sourceOf(
  explicit: unknown,
  environment: unknown,
  profile: unknown,
  fallback: string,
): string {
  return explicit !== undefined
    ? 'argument'
    : environment !== undefined
    ? 'environment'
    : profile !== undefined
    ? 'profile'
    : fallback
}

function validateEndpoint(value: string): void {
  let url: URL
  try {
    url = new URL(value)
  } catch {
    throw new UsageError('INVALID_ENDPOINT', `invalid endpoint URL: ${value}`)
  }
  if (!['http:', 'https:'].includes(url.protocol)) {
    throw new UsageError('INVALID_ENDPOINT', 'endpoint must use http or https')
  }
}

function inferProtocol(endpoint: string | undefined): 'http://' | 'https://' | undefined {
  if (!endpoint) return undefined
  return endpoint.startsWith('http://')
    ? 'http://'
    : endpoint.startsWith('https://')
    ? 'https://'
    : undefined
}

function redactUrl(value: string | undefined): string | null {
  if (!value) return null
  try {
    const url = new URL(value)
    if (url.username || url.password) {
      url.username = '[REDACTED]'
      url.password = '[REDACTED]'
    }
    return url.toString()
  } catch {
    return '[INVALID_URL]'
  }
}

export function configValueError(message: string): ToolError {
  return new ToolError('INVALID_CONFIG_VALUE', message, 2)
}
