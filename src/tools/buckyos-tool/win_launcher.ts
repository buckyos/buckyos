// Windows counterpart of the POSIX `buckyos` wrapper in this directory. Both derive the Deno
// permission set for the same main.ts, so keep them in sync when command options change.

import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const TOOL_DIR = dirname(fileURLToPath(import.meta.url))
const SDK_DIR = join(TOOL_DIR, '..', '..', 'apps', 'sys_test', 'node_modules', 'buckyos')

const ENV_NAMES = [
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
].join(',')

const GLOBAL_VALUE_OPTIONS = new Set([
  '--config-dir',
  '--profile',
  '--zone',
  '--endpoint',
  '--identity',
  '--identity-root',
  '--security-root',
  '--session-token',
  '--session-token-file',
  '--output',
  '--input',
  '--timeout',
  '--trace-id',
  '--idempotency-key',
])
const GLOBAL_READ_PATH_OPTIONS = new Set([
  '--input',
  '--session-token-file',
  '--identity-root',
  '--security-root',
])
const GLOBAL_PLAIN_VALUE_OPTIONS = new Set([
  '--profile',
  '--zone',
  '--endpoint',
  '--identity',
  '--session-token',
  '--output',
  '--timeout',
  '--trace-id',
  '--idempotency-key',
])
const COMMAND_READ_PATH_OPTIONS = new Set([
  '--input',
  '--session-token-file',
  '--identity-root',
  '--security-root',
  '--pikg',
])
const COMMAND_PLAIN_VALUE_OPTIONS = new Set([
  '--from',
  '--app-class',
  '--owner',
  '--policy',
  '--data',
  '--strategy',
])

interface CommandShape {
  moduleName: string
  verbName: string
  isPikg: boolean
}

function inlineValue(argument: string, options: Set<string>): string | undefined {
  const separator = argument.indexOf('=')
  if (separator < 0) return undefined
  return options.has(argument.slice(0, separator)) ? argument.slice(separator + 1) : undefined
}

function readCommandShape(args: string[]): CommandShape {
  let moduleName = ''
  let verbName = ''
  let moduleFound = false
  let expectValue = false
  for (const argument of args) {
    if (moduleFound) {
      if (!verbName) verbName = argument
      continue
    }
    if (expectValue) {
      expectValue = false
      continue
    }
    if (GLOBAL_VALUE_OPTIONS.has(argument)) {
      expectValue = true
      continue
    }
    if (argument.startsWith('--')) continue
    moduleFound = true
    moduleName = argument
  }
  return { moduleName, verbName, isPikg: moduleName === 'pikg' }
}

function computePermissionPaths(args: string[], shape: CommandShape) {
  const home = Deno.env.get('HOME') ?? Deno.env.get('USERPROFILE') ?? '.'
  const appData = Deno.env.get('APPDATA')
  const configDir = Deno.env.get('BUCKYOS_TOOL_CONFIG_DIR') ?? join(home, '.buckyos_tool')
  const runtimeRoot = Deno.env.get('BUCKYOS_ROOT') ??
    (appData ? join(appData, 'buckyos') : 'C:\\BuckyOS')

  const readPaths = [TOOL_DIR, SDK_DIR, configDir, runtimeRoot]
  const writePaths = [configDir]
  const identityRoot = Deno.env.get('BUCKYOS_IDENTITY_ROOT')
  if (identityRoot) readPaths.push(identityRoot)
  const securityRoot = Deno.env.get('BUCKYOS_SECURITY_ROOT')
  if (securityRoot) readPaths.push(securityRoot)

  const planWritable = shape.moduleName === 'app' && shape.verbName === 'fetch'
  const appSourceReadable = shape.moduleName === 'app' &&
    (shape.verbName === 'fetch' || shape.verbName === 'install')
  const addRead = (value: string) => {
    if (value !== '-') readPaths.push(value)
  }

  let expectPath: 'config' | 'read' | 'plan' | 'write' | '' = ''
  let expectValue = false
  let stage = 0
  let appSourceAdded = false

  for (const argument of args) {
    if (expectPath) {
      if (expectPath === 'config') {
        readPaths.push(argument)
        writePaths.push(argument)
      } else if (expectPath === 'read') {
        addRead(argument)
      } else if (expectPath === 'plan') {
        readPaths.push(argument)
        if (planWritable) writePaths.push(argument)
      } else {
        writePaths.push(argument)
      }
      expectPath = ''
      continue
    }
    if (expectValue) {
      expectValue = false
      continue
    }

    if (stage === 0) {
      if (argument === '--config-dir') {
        expectPath = 'config'
        continue
      }
      if (GLOBAL_READ_PATH_OPTIONS.has(argument)) {
        expectPath = 'read'
        continue
      }
      if (argument.startsWith('--config-dir=')) {
        const value = argument.slice('--config-dir='.length)
        readPaths.push(value)
        writePaths.push(value)
        continue
      }
      const globalRead = inlineValue(argument, GLOBAL_READ_PATH_OPTIONS)
      if (globalRead !== undefined) {
        addRead(globalRead)
        continue
      }
      if (GLOBAL_PLAIN_VALUE_OPTIONS.has(argument)) {
        expectValue = true
        continue
      }
      if (argument.startsWith('--')) continue
      stage = 1
      continue
    }
    if (stage === 1) {
      stage = 2
      continue
    }

    if (argument === '--config-dir') {
      expectPath = 'config'
      continue
    }
    if (argument.startsWith('--config-dir=')) {
      readPaths.push(argument.slice('--config-dir='.length))
      continue
    }
    if (COMMAND_READ_PATH_OPTIONS.has(argument)) {
      expectPath = 'read'
      continue
    }
    if (argument === '--file') {
      if (shape.moduleName === 'system-config' && shape.verbName === 'set-file') {
        expectPath = 'read'
      } else {
        expectValue = true
      }
      continue
    }
    if (argument === '--plan') {
      expectPath = 'plan'
      continue
    }
    if (argument === '--path') {
      expectPath = 'write'
      continue
    }
    const commandRead = inlineValue(argument, COMMAND_READ_PATH_OPTIONS)
    if (commandRead !== undefined) {
      addRead(commandRead)
      continue
    }
    if (argument.startsWith('--file=')) {
      if (shape.moduleName === 'system-config' && shape.verbName === 'set-file') {
        addRead(argument.slice('--file='.length))
      }
      continue
    }
    if (argument.startsWith('--plan=')) {
      const value = argument.slice('--plan='.length)
      readPaths.push(value)
      if (planWritable) writePaths.push(value)
      continue
    }
    if (argument.startsWith('--path=')) {
      writePaths.push(argument.slice('--path='.length))
      continue
    }
    if (COMMAND_PLAIN_VALUE_OPTIONS.has(argument)) {
      expectValue = true
      continue
    }
    if (argument.startsWith('--')) continue
    if (appSourceReadable && !appSourceAdded) {
      appSourceAdded = true
      if (!/^https?:\/\//.test(argument)) readPaths.push(argument)
    }
  }

  return { readPaths, writePaths }
}

export function buildDenoArgs(args: string[]): string[] {
  const shape = readCommandShape(args)
  const denoArgs = ['run', '--quiet']
  if (shape.isPikg) {
    denoArgs.push('--allow-read', '--allow-write', '--allow-run=docker')
  } else {
    const { readPaths, writePaths } = computePermissionPaths(args, shape)
    denoArgs.push(
      '--allow-net',
      `--allow-read=${readPaths.join(',')}`,
      `--allow-write=${writePaths.join(',')}`,
    )
  }
  denoArgs.push(`--allow-env=${ENV_NAMES}`, join(TOOL_DIR, 'main.ts'), ...args)
  return denoArgs
}

if (import.meta.main) {
  const child = new Deno.Command('deno', {
    args: buildDenoArgs(Deno.args),
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
  }).spawn()
  const status = await child.status
  Deno.exit(status.code)
}
