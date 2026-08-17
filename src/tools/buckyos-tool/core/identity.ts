import { join } from 'node:path'
import { namelib } from 'buckyos'
import { type Environment, readEnvironment, type ResolvedConfig } from './config.ts'
import { EXIT_AUTH, ToolError, UsageError } from './errors.ts'

export interface IdentityMaterial {
  did: string
  subject: string
  issuer: string
  publicRoot: string
  securityRoot: string
  documentPath: string
  privateKeyPath: string
  privateKeyPem: string
}

export interface IdentityRootPair {
  publicRoot: string
  securityRoot: string
  source: 'explicit' | 'tool' | 'environment' | 'buckyos-root'
}

export function identityRootPairs(
  config: ResolvedConfig,
  environment: Environment = readEnvironment(),
): IdentityRootPair[] {
  const pairs: IdentityRootPair[] = []
  if (config.identityRoot && config.securityRoot) {
    pairs.push({
      publicRoot: config.identityRoot,
      securityRoot: config.securityRoot,
      source: 'explicit',
    })
  }
  pairs.push({
    publicRoot: join(config.configDir, 'local', 'identity'),
    securityRoot: join(config.configDir, 'security'),
    source: 'tool',
  })
  if (environment.BUCKYOS_IDENTITY_ROOT && environment.BUCKYOS_SECURITY_ROOT) {
    const duplicate = config.identityRoot === environment.BUCKYOS_IDENTITY_ROOT &&
      config.securityRoot === environment.BUCKYOS_SECURITY_ROOT
    if (!duplicate) {
      pairs.push({
        publicRoot: environment.BUCKYOS_IDENTITY_ROOT,
        securityRoot: environment.BUCKYOS_SECURITY_ROOT,
        source: 'environment',
      })
    }
  }
  const buckyosRoot = environment.BUCKYOS_ROOT ?? defaultBuckyOSRoot()
  pairs.push({
    publicRoot: join(buckyosRoot, 'local', 'identity'),
    securityRoot: join(buckyosRoot, 'security'),
    source: 'buckyos-root',
  })
  return deduplicatePairs(pairs)
}

export async function resolveIdentityMaterial(
  selectedIdentity: string,
  config: ResolvedConfig,
  environment: Environment = readEnvironment(),
): Promise<IdentityMaterial> {
  const selected = selectedIdentity.trim()
  if (!selected) throw new UsageError('IDENTITY_REQUIRED', 'identity is empty')

  for (const roots of identityRootPairs(config, environment)) {
    for (const directory of await candidateDirectories(roots.publicRoot, selected)) {
      const documentPath = join(roots.publicRoot, directory, 'did.json')
      const document = await readIdentityDocument(documentPath)
      if (!document || !identityMatches(document, selected)) continue

      const did = typeof document.id === 'string' ? document.id : ''
      const subject = typeof document.name === 'string' && document.name.trim()
        ? document.name.trim()
        : did
      if (!did || !subject) continue

      const privateKeyPath = join(roots.securityRoot, directory, 'authentication.private.pem')
      try {
        const privateKeyPem = (await Deno.readTextFile(privateKeyPath)).trim()
        if (!privateKeyPem) continue
        return {
          did,
          subject,
          issuer: subject,
          publicRoot: roots.publicRoot,
          securityRoot: roots.securityRoot,
          documentPath,
          privateKeyPath,
          privateKeyPem,
        }
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error
      }

      const keyrefPath = join(roots.securityRoot, directory, 'authentication.keyref.json')
      try {
        await Deno.stat(keyrefPath)
        throw new ToolError(
          'IDENTITY_KEYREF_UNSUPPORTED',
          'the selected identity uses a key reference unsupported by this runtime',
          EXIT_AUTH,
          false,
          { identity: did, keyref_path: keyrefPath },
        )
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error
      }
    }
  }

  throw new ToolError(
    'IDENTITY_NOT_FOUND',
    `no usable authentication material found for identity ${selected}`,
    EXIT_AUTH,
  )
}

async function candidateDirectories(publicRoot: string, identity: string): Promise<string[]> {
  if (identity.startsWith('did:')) {
    try {
      return [namelib.DID.fromStr(identity).toFilename()]
    } catch {
      throw new UsageError('INVALID_IDENTITY', `invalid DID: ${identity}`)
    }
  }
  const directories: string[] = []
  try {
    for await (const entry of Deno.readDir(publicRoot)) {
      if (entry.isDirectory) directories.push(entry.name)
    }
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return []
    throw error
  }
  return directories.sort()
}

async function readIdentityDocument(path: string): Promise<Record<string, unknown> | null> {
  try {
    const value = JSON.parse(await Deno.readTextFile(path)) as unknown
    return value && typeof value === 'object' && !Array.isArray(value)
      ? value as Record<string, unknown>
      : null
  } catch (error) {
    if (error instanceof Deno.errors.NotFound || error instanceof SyntaxError) return null
    throw error
  }
}

function identityMatches(document: Record<string, unknown>, selected: string): boolean {
  if (typeof document.id === 'string' && document.id === selected) return true
  return typeof document.name === 'string' && document.name === selected
}

function defaultBuckyOSRoot(): string {
  if (Deno.build.os === 'windows') {
    const appData = Deno.env.get('APPDATA')
    return appData ? join(appData, 'buckyos') : 'C:\\BuckyOS'
  }
  return '/opt/buckyos'
}

function deduplicatePairs(pairs: IdentityRootPair[]): IdentityRootPair[] {
  const seen = new Set<string>()
  return pairs.filter((pair) => {
    const key = `${pair.publicRoot}\0${pair.securityRoot}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}
