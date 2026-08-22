import type { CommandDefinition, CommandModule, JsonSchema, RegisteredCommand } from './command.ts'
import { optionProperty } from './command.ts'
import { UsageError } from './errors.ts'

const KEBAB_CASE = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/

const GLOBAL_OPTIONS = [
  { name: 'config-dir', property: 'configDir', type: 'string', scope: 'process' },
  { name: 'profile', property: 'profile', type: 'string', scope: 'session' },
  { name: 'zone', property: 'zone', type: 'string', scope: 'session' },
  { name: 'endpoint', property: 'endpoint', type: 'string', scope: 'session' },
  { name: 'identity', property: 'identity', type: 'string', scope: 'session' },
  { name: 'identity-root', property: 'identityRoot', type: 'string', scope: 'session' },
  { name: 'security-root', property: 'securityRoot', type: 'string', scope: 'session' },
  {
    name: 'session-token',
    property: 'sessionToken',
    type: 'string',
    scope: 'session',
    secret: true,
  },
  {
    name: 'session-token-file',
    property: 'sessionTokenFile',
    type: 'string',
    scope: 'session',
    secret: true,
  },
  { name: 'cli', property: 'cli', type: 'boolean', scope: 'process' },
  {
    name: 'output',
    property: 'output',
    type: 'string',
    scope: 'command',
    enum: ['json', 'jsonl', 'table', 'text', 'raw'],
  },
  { name: 'input', property: 'input', type: 'string', scope: 'command' },
  { name: 'timeout', property: 'timeout', type: 'duration', scope: 'command' },
  { name: 'trace-id', property: 'traceId', type: 'string', scope: 'command' },
  {
    name: 'idempotency-key',
    property: 'idempotencyKey',
    type: 'string',
    scope: 'command',
  },
  { name: 'wait', property: 'wait', type: 'boolean', scope: 'command' },
  {
    name: 'non-interactive',
    property: 'nonInteractive',
    type: 'boolean',
    scope: 'process',
  },
  { name: 'yes', property: 'yes', type: 'boolean', scope: 'command' },
  { name: 'no-color', property: 'noColor', type: 'boolean', scope: 'process' },
  { name: 'verbose', property: 'verbose', type: 'boolean', scope: 'process' },
  { name: 'help', property: 'help', type: 'boolean', scope: 'process' },
  { name: 'version', property: 'version', type: 'boolean', scope: 'process' },
]

const REPL_COMMAND_OPTION_NAMES = new Set([
  'output',
  'input',
  'timeout',
  'trace-id',
  'idempotency-key',
  'wait',
  'yes',
])

export class CommandRegistry {
  #modules = new Map<string, CommandModule>()
  #commands = new Map<string, RegisteredCommand>()

  register(module: CommandModule): void {
    if (!KEBAB_CASE.test(module.name)) {
      throw new Error(`invalid module name: ${module.name}`)
    }
    if (this.#modules.has(module.name)) {
      throw new Error(`duplicate module: ${module.name}`)
    }
    for (const definition of module.commands) {
      this.#validateDefinition(module.name, definition)
      const key = `${module.name}.${definition.verb}`
      if (this.#commands.has(key)) throw new Error(`duplicate command: ${key}`)
      this.#commands.set(key, {
        ...definition,
        module: module.name,
        moduleSummary: module.summary,
      })
    }
    this.#modules.set(module.name, module)
  }

  get(module: string, verb: string): RegisteredCommand {
    const command = this.#commands.get(`${module}.${verb}`)
    if (!command) {
      if (!this.#modules.has(module)) {
        throw new UsageError('UNKNOWN_MODULE', `unknown module: ${module}`)
      }
      throw new UsageError('UNKNOWN_COMMAND', `unknown command: ${module} ${verb}`)
    }
    return command
  }

  modules(): CommandModule[] {
    return [...this.#modules.values()].sort((a, b) => a.name.localeCompare(b.name))
  }

  commands(): RegisteredCommand[] {
    return [...this.#commands.values()].sort((a, b) =>
      `${a.module}.${a.verb}`.localeCompare(`${b.module}.${b.verb}`)
    )
  }

  describe(module: string, verb: string): Record<string, unknown> {
    const command = this.get(module, verb)
    return {
      schema_version: 1,
      module: command.module,
      verb: command.verb,
      summary: command.summary,
      description: command.description ?? command.summary,
      syntax: this.syntax(command),
      global_options: GLOBAL_OPTIONS,
      repl_command_options: GLOBAL_OPTIONS.filter((option) =>
        REPL_COMMAND_OPTION_NAMES.has(option.name)
      ),
      positionals: command.positionals ?? [],
      options: (command.options ?? []).map((option) => ({
        ...option,
        property: optionProperty(option),
      })),
      input_schema: command.inputSchema,
      output_schema: command.outputSchema,
      result_schema_version: command.resultSchemaVersion,
      access: command.access,
      async_mode: command.asyncMode,
      requires_session: command.requiresSession,
      execution: command.execution ?? (command.requiresSession ? 'service' : 'local'),
      network_access: command.networkAccess ?? command.requiresSession,
      examples: command.examples ?? [],
    }
  }

  syntax(command: RegisteredCommand): string {
    const positionals = (command.positionals ?? []).map((position) =>
      position.required === false ? `[${position.name}]` : `<${position.name}>`
    )
    const options = (command.options ?? []).map((option) =>
      option.type === 'boolean'
        ? `[--${option.name}]`
        : `[--${option.name} <${option.property ?? option.name}>]`
    )
    return ['buckyos', command.module, command.verb, ...positionals, ...options].join(' ')
  }

  completionCandidates(tokens: string[]): string[] {
    if (tokens.length <= 1) return this.modules().map((module) => module.name)
    const module = this.#modules.get(tokens[0])
    if (!module) return this.modules().map((candidate) => candidate.name)
    if (tokens.length === 2 && !tokens[1].startsWith('--')) {
      return module.commands.map((command) => command.verb).sort()
    }
    const verb = tokens[1]
    const command = this.#commands.get(`${module.name}.${verb}`)
    if (!command) return module.commands.map((candidate) => candidate.verb).sort()
    return [
      ...(command.options ?? []).map((option) => `--${option.name}`),
      '--input',
      '--timeout',
      '--trace-id',
      '--idempotency-key',
      '--output',
      '--wait',
      '--yes',
      '--help',
    ].sort()
  }

  #validateDefinition(module: string, definition: CommandDefinition): void {
    if (!KEBAB_CASE.test(definition.verb)) {
      throw new Error(`invalid verb name: ${module} ${definition.verb}`)
    }
    const positionals = definition.positionals ?? []
    const positionalNames = new Set<string>()
    for (const positional of positionals) {
      if (positionalNames.has(positional.name)) {
        throw new Error(`duplicate positional ${positional.name} in ${module}.${definition.verb}`)
      }
      positionalNames.add(positional.name)
    }
    const optionNames = new Set<string>()
    for (const option of definition.options ?? []) {
      if (!KEBAB_CASE.test(option.name) || optionNames.has(option.name)) {
        throw new Error(
          `invalid or duplicate option --${option.name} in ${module}.${definition.verb}`,
        )
      }
      optionNames.add(option.name)
    }
    this.#assertObjectSchema(definition.inputSchema, `${module}.${definition.verb} input`)
    this.#assertObjectSchema(definition.outputSchema, `${module}.${definition.verb} output`)
  }

  #assertObjectSchema(schema: JsonSchema, label: string): void {
    if (schema.type !== 'object') throw new Error(`${label} schema must be an object`)
  }
}
