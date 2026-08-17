import type { JsonSchema, RegisteredCommand } from './command.ts'
import { optionProperty } from './command.ts'
import { UsageError } from './errors.ts'

export const OUTPUT_FORMATS = ['json', 'jsonl', 'table', 'text', 'raw'] as const
export type OutputFormat = typeof OUTPUT_FORMATS[number]

export interface GlobalOptions {
  configDir?: string
  profile?: string
  zone?: string
  endpoint?: string
  identity?: string
  identityRoot?: string
  securityRoot?: string
  sessionToken?: string
  sessionTokenFile?: string
  cli?: boolean
  output?: OutputFormat
  input?: string
  timeout?: string
  traceId?: string
  idempotencyKey?: string
  wait?: boolean
  nonInteractive?: boolean
  yes?: boolean
  noColor?: boolean
  verbose?: boolean
  help?: boolean
  version?: boolean
}

export interface Invocation {
  global: GlobalOptions
  module?: string
  verb?: string
  actionArgv: string[]
}

export interface ParsedCommandArgs {
  input: Record<string, unknown>
  scoped: GlobalOptions
  help: boolean
}

const STRING_GLOBALS = new Map<string, keyof GlobalOptions>([
  ['config-dir', 'configDir'],
  ['profile', 'profile'],
  ['zone', 'zone'],
  ['endpoint', 'endpoint'],
  ['identity', 'identity'],
  ['identity-root', 'identityRoot'],
  ['security-root', 'securityRoot'],
  ['session-token', 'sessionToken'],
  ['session-token-file', 'sessionTokenFile'],
  ['output', 'output'],
  ['input', 'input'],
  ['timeout', 'timeout'],
  ['trace-id', 'traceId'],
  ['idempotency-key', 'idempotencyKey'],
])

const BOOLEAN_GLOBALS = new Map<string, keyof GlobalOptions>([
  ['cli', 'cli'],
  ['wait', 'wait'],
  ['non-interactive', 'nonInteractive'],
  ['yes', 'yes'],
  ['no-color', 'noColor'],
  ['verbose', 'verbose'],
  ['help', 'help'],
  ['version', 'version'],
])

const REPL_SCOPED_STRING_GLOBALS = new Set([
  'input',
  'timeout',
  'trace-id',
  'idempotency-key',
  'output',
])
const REPL_SCOPED_BOOLEAN_GLOBALS = new Set(['wait', 'yes'])

export function parseInvocation(argv: string[]): Invocation {
  const global: GlobalOptions = {}
  let index = 0
  while (index < argv.length) {
    const token = argv[index]
    if (token === '-h') {
      setGlobal(global, 'help', true)
      index += 1
      continue
    }
    if (token === '--') {
      index += 1
      break
    }
    if (!token.startsWith('--')) break
    const parsed = splitLongOption(token)
    const booleanProperty = BOOLEAN_GLOBALS.get(parsed.name)
    if (booleanProperty) {
      if (parsed.inlineValue !== undefined) {
        throw new UsageError('INVALID_ARGUMENT', `--${parsed.name} does not accept a value`)
      }
      setGlobal(global, booleanProperty, true)
      index += 1
      continue
    }
    const stringProperty = STRING_GLOBALS.get(parsed.name)
    if (!stringProperty) {
      throw new UsageError('UNKNOWN_OPTION', `unknown global option: --${parsed.name}`)
    }
    const { value, consumed } = readOptionValue(argv, index, parsed)
    setGlobal(global, stringProperty, normalizeGlobalValue(parsed.name, value))
    index += consumed
  }

  const module = argv[index]
  const verb = argv[index + 1]
  const actionArgv = module ? argv.slice(index + (verb ? 2 : 1)) : []
  if (module?.startsWith('-')) {
    throw new UsageError(
      'INVALID_ARGUMENT',
      `global options must appear before the module: ${module}`,
    )
  }
  return { global, module, verb, actionArgv }
}

export function parseCommandArgs(
  command: RegisteredCommand,
  argv: string[],
  inputObject: Record<string, unknown> | undefined,
  allowReplScopedGlobals = false,
  deferValidation = false,
): ParsedCommandArgs {
  const cliInput: Record<string, unknown> = {}
  const scoped: GlobalOptions = {}
  const positionals = command.positionals ?? []
  const optionByName = new Map((command.options ?? []).map((option) => [option.name, option]))
  let positionalIndex = 0
  let help = false

  for (let index = 0; index < argv.length;) {
    const token = argv[index]
    if (token === '--help' || token === '-h') {
      help = true
      index += 1
      continue
    }
    if (token === '--') {
      for (const positionalValue of argv.slice(index + 1)) {
        positionalIndex = assignPositional(
          cliInput,
          positionals,
          positionalIndex,
          positionalValue,
        )
      }
      break
    }
    if (!token.startsWith('--')) {
      positionalIndex = assignPositional(cliInput, positionals, positionalIndex, token)
      index += 1
      continue
    }

    const parsed = splitLongOption(token)
    if (allowReplScopedGlobals && REPL_SCOPED_BOOLEAN_GLOBALS.has(parsed.name)) {
      if (parsed.inlineValue !== undefined) {
        throw new UsageError('INVALID_ARGUMENT', `--${parsed.name} does not accept a value`)
      }
      const property = BOOLEAN_GLOBALS.get(parsed.name)!
      setGlobal(scoped, property, true)
      index += 1
      continue
    }
    if (allowReplScopedGlobals && REPL_SCOPED_STRING_GLOBALS.has(parsed.name)) {
      const property = STRING_GLOBALS.get(parsed.name)!
      const { value, consumed } = readOptionValue(argv, index, parsed)
      setGlobal(scoped, property, normalizeGlobalValue(parsed.name, value))
      index += consumed
      continue
    }
    if (
      allowReplScopedGlobals &&
      (STRING_GLOBALS.has(parsed.name) || BOOLEAN_GLOBALS.has(parsed.name))
    ) {
      throw new UsageError(
        'SESSION_OPTION_FROZEN',
        `--${parsed.name} is frozen for the interactive session`,
      )
    }

    const option = optionByName.get(parsed.name)
    if (!option) throw new UsageError('UNKNOWN_OPTION', `unknown option: --${parsed.name}`)
    const property = optionProperty(option)
    if (Object.hasOwn(cliInput, property)) {
      throw new UsageError('DUPLICATE_ARGUMENT', `argument provided more than once: ${property}`)
    }
    if (option.type === 'boolean') {
      if (parsed.inlineValue !== undefined) {
        throw new UsageError('INVALID_ARGUMENT', `--${parsed.name} does not accept a value`)
      }
      cliInput[property] = true
      index += 1
      continue
    }
    const { value, consumed } = readOptionValue(argv, index, parsed)
    cliInput[property] = parseOptionValue(option.type, value, parsed.name)
    index += consumed
  }

  for (let index = positionalIndex; !deferValidation && index < positionals.length; index++) {
    if (
      positionals[index].required !== false &&
      !Object.hasOwn(inputObject ?? {}, positionals[index].name)
    ) {
      throw new UsageError(
        'MISSING_ARGUMENT',
        `missing positional argument: ${positionals[index].name}`,
      )
    }
  }

  const input = mergeCommandInput(inputObject, cliInput)
  if (!deferValidation) validateSchema(input, command.inputSchema, 'input')
  return { input, scoped, help }
}

export function parseDuration(value: string): number {
  const match = /^(\d+)(ms|s|m|h)$/.exec(value.trim())
  if (!match) {
    throw new UsageError('INVALID_DURATION', `invalid duration: ${value}`)
  }
  const amount = Number(match[1])
  const scale = match[2] === 'ms'
    ? 1
    : match[2] === 's'
    ? 1_000
    : match[2] === 'm'
    ? 60_000
    : 3_600_000
  const result = amount * scale
  if (!Number.isSafeInteger(result) || result <= 0) {
    throw new UsageError('INVALID_DURATION', `invalid duration: ${value}`)
  }
  return result
}

export function parseShellLine(line: string): string[] {
  const tokens: string[] = []
  let current = ''
  let quote: 'single' | 'double' | null = null
  let escaped = false
  let started = false

  for (const character of line) {
    if (escaped) {
      current += character
      escaped = false
      started = true
      continue
    }
    if (character === '\\' && quote !== 'single') {
      escaped = true
      started = true
      continue
    }
    if (quote === 'single') {
      if (character === "'") quote = null
      else current += character
      continue
    }
    if (quote === 'double') {
      if (character === '"') quote = null
      else current += character
      continue
    }
    if (character === "'") {
      quote = 'single'
      started = true
      continue
    }
    if (character === '"') {
      quote = 'double'
      started = true
      continue
    }
    if (/\s/.test(character)) {
      if (started) {
        tokens.push(current)
        current = ''
        started = false
      }
      continue
    }
    current += character
    started = true
  }
  if (escaped) throw new UsageError('INVALID_COMMAND_LINE', 'unfinished escape sequence')
  if (quote) throw new UsageError('INVALID_COMMAND_LINE', `unterminated ${quote} quote`)
  if (started) tokens.push(current)
  return tokens
}

export function validateSchema(value: unknown, schema: JsonSchema, path: string): void {
  if (schema.enum && !schema.enum.some((candidate) => Object.is(candidate, value))) {
    throw new UsageError(
      'SCHEMA_VALIDATION_FAILED',
      `${path} must be one of ${schema.enum.join(', ')}`,
    )
  }
  if (schema.type === 'object') {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path} must be an object`)
    }
    const object = value as Record<string, unknown>
    for (const required of schema.required ?? []) {
      if (!Object.hasOwn(object, required)) {
        throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path}.${required} is required`)
      }
    }
    if (schema.additionalProperties === false) {
      for (const key of Object.keys(object)) {
        if (!Object.hasOwn(schema.properties ?? {}, key)) {
          throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path}.${key} is not allowed`)
        }
      }
    }
    for (const [key, propertyValue] of Object.entries(object)) {
      const propertySchema = schema.properties?.[key]
      if (propertySchema) validateSchema(propertyValue, propertySchema, `${path}.${key}`)
    }
    return
  }
  if (schema.type === 'array') {
    if (!Array.isArray(value)) {
      throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path} must be an array`)
    }
    if (schema.items) {
      value.forEach((item, index) => validateSchema(item, schema.items!, `${path}[${index}]`))
    }
    return
  }
  if (schema.type === 'string') {
    if (typeof value !== 'string') {
      throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path} must be a string`)
    }
    if (schema.minLength !== undefined && value.length < schema.minLength) {
      throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path} is too short`)
    }
    return
  }
  if (schema.type === 'boolean' && typeof value !== 'boolean') {
    throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path} must be a boolean`)
  }
  if (schema.type === 'integer' && (!Number.isInteger(value) || typeof value !== 'number')) {
    throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path} must be an integer`)
  }
  if (schema.type === 'number' && typeof value !== 'number') {
    throw new UsageError('SCHEMA_VALIDATION_FAILED', `${path} must be a number`)
  }
}

function assignPositional(
  input: Record<string, unknown>,
  positionals: NonNullable<RegisteredCommand['positionals']>,
  index: number,
  value: string,
): number {
  const positional = positionals[index]
  if (!positional) {
    throw new UsageError('TOO_MANY_ARGUMENTS', `unexpected positional argument: ${value}`)
  }
  input[positional.name] = value
  return index + 1
}

function splitLongOption(token: string): { name: string; inlineValue?: string } {
  const content = token.slice(2)
  const equalIndex = content.indexOf('=')
  return equalIndex < 0
    ? { name: content }
    : { name: content.slice(0, equalIndex), inlineValue: content.slice(equalIndex + 1) }
}

function readOptionValue(
  argv: string[],
  index: number,
  option: { name: string; inlineValue?: string },
): { value: string; consumed: number } {
  if (option.inlineValue !== undefined) {
    if (!option.inlineValue) {
      throw new UsageError('MISSING_ARGUMENT', `--${option.name} requires a value`)
    }
    return { value: option.inlineValue, consumed: 1 }
  }
  const value = argv[index + 1]
  if (value === undefined || value.startsWith('--')) {
    throw new UsageError('MISSING_ARGUMENT', `--${option.name} requires a value`)
  }
  return { value, consumed: 2 }
}

function parseOptionValue(type: string, value: string, name: string): unknown {
  if (type === 'string') return value
  const number = Number(value)
  if (!Number.isFinite(number) || (type === 'integer' && !Number.isInteger(number))) {
    throw new UsageError('INVALID_ARGUMENT', `--${name} requires a ${type}`)
  }
  return number
}

function mergeCommandInput(
  inputObject: Record<string, unknown> | undefined,
  cliInput: Record<string, unknown>,
): Record<string, unknown> {
  const result = { ...(inputObject ?? {}) }
  for (const [key, value] of Object.entries(cliInput)) {
    if (Object.hasOwn(result, key)) {
      throw new UsageError('ARGUMENT_CONFLICT', `${key} is present in both --input and argv`)
    }
    result[key] = value
  }
  return result
}

function normalizeGlobalValue(name: string, value: string): string | OutputFormat {
  if (name === 'output') {
    if (!OUTPUT_FORMATS.includes(value as OutputFormat)) {
      throw new UsageError('INVALID_OUTPUT_FORMAT', `invalid output format: ${value}`)
    }
    return value as OutputFormat
  }
  return value
}

function setGlobal<K extends keyof GlobalOptions>(
  target: GlobalOptions,
  property: K,
  value: GlobalOptions[K],
): void {
  if (target[property] !== undefined) {
    throw new UsageError('DUPLICATE_ARGUMENT', `option provided more than once: ${property}`)
  }
  target[property] = value
}
