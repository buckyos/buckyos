import type { GlobalOptions } from './argv.ts'
import { parseCommandArgs, parseDuration, parseInvocation, validateSchema } from './argv.ts'
import { AuthenticationSession, type SessionController } from './auth.ts'
import type { RegisteredCommand } from './command.ts'
import {
  type ConfigStore,
  type Environment,
  type ImplicitDeviceIdentity,
  localResolvedConfig,
  readEnvironment,
  resolveConfig,
  type ResolvedConfig,
} from './config.ts'
import type { CommandContext } from './context.ts'
import { EXIT_PERMISSION, EXIT_SUCCESS, normalizeError, ToolError, UsageError } from './errors.ts'
import { applyImplicitDeviceIdentity } from './identity.ts'
import { errorEnvelope, renderError, renderSuccess, successEnvelope } from './output.ts'
import { CommandRegistry } from './registry.ts'
import { runRepl } from './repl.ts'
import {
  BuckyOSRuntimeAdapter,
  BuckyOSServiceClientRegistry,
  InteractiveSession,
  type RuntimeAdapter,
  type ServiceClientRegistry,
} from './runtime.ts'
import { createAuthModule } from '../modules/auth.ts'
import { createCoreModules } from '../modules/core.ts'
import { createSystemModule } from '../modules/system.ts'
import { createSystemConfigModule } from '../modules/system_config.ts'
import { createPikgModule, type PikgModuleDependencies } from '../modules/pikg.ts'
import { type AppModuleDependencies, createAppModule } from '../modules/app.ts'
import { createAuditModule } from '../modules/audit.ts'
import { createDiagnosticModule, type DiagnosticModuleDependencies } from '../modules/diagnostic.ts'
import { createLogModule, type LogModuleDependencies } from '../modules/log.ts'
import { createTaskModule } from '../modules/task.ts'

export const VERSION = '0.1.0-phase4'

export interface ToolStdio {
  stdout(value: string): Promise<void>
  stderr(value: string): Promise<void>
  readStdin(): Promise<string>
  prompt?(message: string): Promise<string | null>
  inputIsTerminal?(): boolean
}

export interface ApplicationDependencies {
  environment?: Environment
  cwd?: string
  homeDir?: string
  stdio?: ToolStdio
  createAuthentication?: (config: ResolvedConfig) => SessionController
  runtime?: RuntimeAdapter
  createClients?: (
    config: ResolvedConfig,
    authentication: SessionController,
  ) => ServiceClientRegistry
  confirmDeviceIdentity?: (identity: ImplicitDeviceIdentity) => Promise<boolean>
  repl?: typeof runRepl
  pikg?: PikgModuleDependencies
  app?: AppModuleDependencies
  log?: LogModuleDependencies
  diagnostic?: DiagnosticModuleDependencies
}

export class BuckyOSToolApplication {
  readonly registry: CommandRegistry
  readonly #environment: Environment
  readonly #cwd: string
  readonly #homeDir?: string
  readonly #stdio: ToolStdio
  readonly #createAuthentication: (config: ResolvedConfig) => SessionController
  readonly #runtime: RuntimeAdapter
  readonly #createClients: (
    config: ResolvedConfig,
    authentication: SessionController,
  ) => ServiceClientRegistry
  readonly #confirmDeviceIdentity: (identity: ImplicitDeviceIdentity) => Promise<boolean>
  readonly #repl: typeof runRepl

  constructor(dependencies: ApplicationDependencies = {}) {
    this.registry = createRegistry(
      dependencies.pikg,
      dependencies.app,
      dependencies.log,
      dependencies.diagnostic,
    )
    this.#environment = dependencies.environment ?? readEnvironment()
    this.#cwd = dependencies.cwd ?? Deno.cwd()
    this.#homeDir = dependencies.homeDir
    this.#stdio = dependencies.stdio ?? defaultStdio()
    this.#createAuthentication = dependencies.createAuthentication ??
      ((config) => new AuthenticationSession(config, this.#environment))
    this.#runtime = dependencies.runtime ?? new BuckyOSRuntimeAdapter()
    this.#createClients = dependencies.createClients ??
      ((config, authentication) => new BuckyOSServiceClientRegistry(config, authentication))
    this.#confirmDeviceIdentity = dependencies.confirmDeviceIdentity ?? confirmDeviceIdentity
    this.#repl = dependencies.repl ?? runRepl
  }

  async run(argv: string[]): Promise<number> {
    const fallbackTraceId = crypto.randomUUID()
    try {
      const invocation = parseInvocation(argv)
      if (invocation.global.version) {
        if (invocation.module) {
          throw new UsageError('ARGUMENT_CONFLICT', '--version cannot be combined with a command')
        }
        await this.#stdio.stdout(`buckyos ${VERSION}\n`)
        return EXIT_SUCCESS
      }
      if (invocation.global.cli) {
        return await this.#runInteractive(invocation.global, invocation.module, invocation.verb)
      }
      if (!invocation.module) {
        if (invocation.global.help) {
          await this.#stdio.stdout(`${topLevelHelp(this.registry)}\n`)
          return EXIT_SUCCESS
        }
        throw new UsageError('COMMAND_REQUIRED', 'a module and verb are required')
      }
      if (!invocation.verb || invocation.verb === '--help' || invocation.verb === '-h') {
        await this.#stdio.stdout(`${moduleHelp(this.registry, invocation.module)}\n`)
        return EXIT_SUCCESS
      }

      const command = this.registry.get(invocation.module, invocation.verb)
      if (
        invocation.global.help || invocation.actionArgv.includes('--help') ||
        invocation.actionArgv.includes('-h')
      ) {
        await this.#stdio.stdout(`${commandHelp(this.registry, command)}\n`)
        return EXIT_SUCCESS
      }
      const setup = await this.#resolveForCommand(command, invocation.global)
      const inputObject = invocation.global.input
        ? await this.#readInputObject(invocation.global.input)
        : undefined
      const parsed = parseCommandArgs(command, invocation.actionArgv, inputObject)
      if (setup.resolved.output === 'raw' && !command.supportsRawOutput) {
        throw new UsageError(
          'RAW_OUTPUT_UNSUPPORTED',
          `${command.module} ${command.verb} does not support raw output`,
        )
      }
      const session = command.requiresSession
        ? await this.#createSession(setup.resolved)
        : undefined
      return await this.#executeAndEmit(
        command,
        parsed.input,
        setup.resolved,
        setup.store,
        session,
        false,
        new AbortController().signal,
      )
    } catch (error) {
      const normalized = normalizeError(error)
      const envelope = errorEnvelope(normalized, { command: 'core', trace_id: fallbackTraceId })
      await this.#stdio.stdout(`${renderError(envelope, 'json')}\n`)
      return normalized.exitCode
    }
  }

  async #runInteractive(global: GlobalOptions, module?: string, verb?: string): Promise<number> {
    if (module || verb) {
      throw new UsageError('ARGUMENT_CONFLICT', '--cli cannot be combined with a module or verb')
    }
    if (global.nonInteractive) {
      throw new UsageError('ARGUMENT_CONFLICT', '--cli conflicts with --non-interactive')
    }
    for (
      const [name, value] of [
        ['input', global.input],
        ['timeout', global.timeout],
        ['trace-id', global.traceId],
        ['idempotency-key', global.idempotencyKey],
        ['wait', global.wait],
        ['yes', global.yes],
      ] as const
    ) {
      if (value !== undefined) {
        throw new UsageError(
          'ARGUMENT_CONFLICT',
          `--${name} is command-scoped and cannot be set when entering --cli`,
        )
      }
    }
    const setup = await resolveConfig(global, this.#environment, {
      interactive: true,
      cwd: this.#cwd,
      homeDir: this.#homeDir,
    })
    setup.resolved = await applyImplicitDeviceIdentity(setup.resolved, this.#environment)
    const session = await this.#createSession(setup.resolved)
    await this.#repl({
      registry: this.registry,
      config: setup.resolved,
      configStore: setup.store,
      session,
      execute: async (tokens, signal) =>
        await this.#executeInteractiveLine(tokens, signal, setup.resolved, setup.store, session),
    })
    return EXIT_SUCCESS
  }

  async #executeInteractiveLine(
    tokens: string[],
    signal: AbortSignal,
    frozen: ResolvedConfig,
    store: ConfigStore,
    session: InteractiveSession,
  ): Promise<void> {
    if (tokens.length < 2) throw new UsageError('COMMAND_REQUIRED', 'enter <module> <verb>')
    const command = this.registry.get(tokens[0], tokens[1])
    const actionArgv = tokens.slice(2)
    if (actionArgv.includes('--help') || actionArgv.includes('-h')) {
      await this.#stdio.stdout(`${commandHelp(this.registry, command)}\n`)
      return
    }
    const preliminary = parseCommandArgs(command, actionArgv, undefined, true, true)
    if (preliminary.scoped.input === '-') {
      throw new UsageError('STDIN_UNAVAILABLE', '--input - is not available inside --cli')
    }
    const inputObject = preliminary.scoped.input
      ? await this.#readInputObject(preliminary.scoped.input)
      : undefined
    const parsed = parseCommandArgs(command, actionArgv, inputObject, true)
    const config: ResolvedConfig = {
      ...frozen,
      output: parsed.scoped.output ?? frozen.output,
      timeoutMs: parsed.scoped.timeout ? parseDuration(parsed.scoped.timeout) : frozen.timeoutMs,
      traceId: parsed.scoped.traceId,
      idempotencyKey: parsed.scoped.idempotencyKey,
      wait: parsed.scoped.wait ?? false,
      yes: parsed.scoped.yes ?? false,
    }
    if (config.output === 'raw' && !command.supportsRawOutput) {
      throw new UsageError(
        'RAW_OUTPUT_UNSUPPORTED',
        `${command.module} ${command.verb} does not support raw output`,
      )
    }
    await this.#executeAndEmit(command, parsed.input, config, store, session, true, signal)
  }

  async #executeAndEmit(
    command: RegisteredCommand,
    input: Record<string, unknown>,
    config: ResolvedConfig,
    store: ConfigStore,
    session: InteractiveSession | undefined,
    interactive: boolean,
    signal: AbortSignal,
  ): Promise<number> {
    const traceId = config.traceId ?? crypto.randomUUID()
    const startedAt = Date.now()
    try {
      if (command.requiresSession && !session) {
        throw new ToolError('AUTH_REQUIRED', 'authenticated session is required', 3)
      }
      if (command.requiresSession) await session!.authentication.ensureValid()
      const principal = session?.authentication.current().principal ?? {
        id: 'local',
        appId: 'buckyos-tool',
        authentication: 'mock' as const,
      }
      const context: CommandContext = {
        command: { module: command.module, verb: command.verb },
        definition: command,
        connection: session?.connection ?? {
          zone: config.zone ?? 'local',
          endpoint: config.endpoint ?? 'local://',
          defaultProtocol: config.defaultProtocol,
        },
        principal,
        clients: session?.clients ?? unavailableClients,
        output: { format: config.output },
        traceId,
        idempotencyKey: config.idempotencyKey,
        deadline: Date.now() + config.timeoutMs,
        signal,
        cwd: this.#cwd,
        io: {
          stdout: (value) => this.#stdio.stdout(value),
          stderr: (value) => this.#stdio.stderr(value),
          prompt: (message) => this.#stdio.prompt?.(message) ?? Promise.resolve(null),
          inputIsTerminal: this.#stdio.inputIsTerminal?.() ?? false,
        },
        interactive,
        confirmed: config.yes,
        config,
        configStore: store,
        session: session?.authentication,
      }
      const data = await command.handler(context, input)
      try {
        validateSchema(data, command.outputSchema, 'output')
      } catch (error) {
        throw new ToolError(
          'INVALID_HANDLER_OUTPUT',
          error instanceof Error ? error.message : String(error),
          9,
        )
      }
      const envelope = successEnvelope(data, {
        command: `${command.module}.${command.verb}`,
        trace_id: traceId,
        duration_ms: Date.now() - startedAt,
      })
      await this.#stdio.stdout(
        `${renderSuccess(envelope, config.output)}${config.output === 'raw' ? '' : '\n'}`,
      )
      return EXIT_SUCCESS
    } catch (error) {
      const normalized = normalizeError(error)
      const envelope = errorEnvelope(normalized, {
        command: `${command.module}.${command.verb}`,
        trace_id: traceId,
        duration_ms: Date.now() - startedAt,
      })
      await this.#stdio.stdout(`${renderError(envelope, config.output)}\n`)
      if (config.verbose) await this.#stdio.stderr(`diagnostic: ${normalized.code}\n`)
      return normalized.exitCode
    }
  }

  async #resolveForCommand(
    command: RegisteredCommand,
    global: GlobalOptions,
  ): Promise<{ resolved: ResolvedConfig; store: ConfigStore }> {
    if (command.requiresSession || (command.module === 'config' && command.verb === 'check')) {
      const setup = await resolveConfig(global, this.#environment, {
        cwd: this.#cwd,
        homeDir: this.#homeDir,
      })
      if (command.requiresSession) {
        setup.resolved = await applyImplicitDeviceIdentity(setup.resolved, this.#environment)
      }
      return setup
    }
    return localResolvedConfig(global, this.#environment, {
      cwd: this.#cwd,
      homeDir: this.#homeDir,
    })
  }

  async #createSession(config: ResolvedConfig): Promise<InteractiveSession> {
    await this.#approveImplicitDeviceIdentity(config)
    const authentication = this.#createAuthentication(config)
    return await InteractiveSession.create(
      config,
      authentication,
      this.#runtime,
      this.#createClients(config, authentication),
    )
  }

  async #approveImplicitDeviceIdentity(config: ResolvedConfig): Promise<void> {
    const identity = config.implicitDeviceIdentity
    if (!identity || config.yes) return
    if (config.nonInteractive) {
      throw new ToolError(
        'CONFIRMATION_REQUIRED',
        'using the current device identity requires --yes in non-interactive mode',
        EXIT_PERMISSION,
        false,
        { identity: identity.did },
      )
    }
    if (!await this.#confirmDeviceIdentity(identity)) {
      throw new ToolError(
        'CONFIRMATION_DECLINED',
        'current device identity confirmation was declined',
        EXIT_PERMISSION,
        false,
        { identity: identity.did },
      )
    }
  }

  async #readInputObject(path: string): Promise<Record<string, unknown>> {
    let raw: string
    try {
      raw = path === '-' ? await this.#stdio.readStdin() : await Deno.readTextFile(path)
    } catch (error) {
      throw new UsageError(
        'INPUT_READ_FAILED',
        `failed to read input: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
    let parsed: unknown
    try {
      parsed = JSON.parse(raw)
    } catch {
      throw new UsageError('INVALID_INPUT_JSON', 'input is not valid JSON')
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new UsageError('INVALID_INPUT_JSON', 'input JSON must be an object')
    }
    return parsed as Record<string, unknown>
  }
}

export function createRegistry(
  pikgDependencies?: PikgModuleDependencies,
  appDependencies?: AppModuleDependencies,
  logDependencies?: LogModuleDependencies,
  diagnosticDependencies?: DiagnosticModuleDependencies,
): CommandRegistry {
  const registry = new CommandRegistry()
  for (const module of createCoreModules(registry)) registry.register(module)
  registry.register(createAuthModule())
  registry.register(createPikgModule(pikgDependencies))
  registry.register(createSystemModule())
  registry.register(createSystemConfigModule())
  registry.register(createAppModule(appDependencies))
  registry.register(createTaskModule())
  registry.register(createAuditModule())
  registry.register(createLogModule(logDependencies))
  registry.register(createDiagnosticModule(diagnosticDependencies))
  return registry
}

const unavailableClients: ServiceClientRegistry = {
  call: () => Promise.reject(new ToolError('INTERNAL_ERROR', 'service clients are unavailable', 9)),
}

function topLevelHelp(registry: CommandRegistry): string {
  return [
    `BuckyOS Tool ${VERSION}`,
    '',
    'Usage:',
    '  buckyos [global-options] <module> <verb> [primary-selector] [action-options]',
    '  buckyos [session-options] --cli',
    '',
    'Modules:',
    ...registry.modules().map((module) => `  ${module.name.padEnd(12)} ${module.summary}`),
    '',
    'Global options:',
    '  --config-dir <path>  --profile <name>  --zone <host-or-did>',
    '  --endpoint <url>      --identity <did-or-name>',
    '  --session-token <token> | --session-token-file <path>',
    '    Prefer --session-token-file for automation; argv tokens may appear in process listings.',
    '  --output <json|jsonl|table|text|raw>  --input <path|->',
    '  --timeout <duration>  --trace-id <id>  --non-interactive  --yes',
    '  --cli  --help  --version',
    '',
    'Use `buckyos command describe <module> <verb>` for machine-readable schemas.',
  ].join('\n')
}

function moduleHelp(registry: CommandRegistry, moduleName: string): string {
  const module = registry.modules().find((candidate) => candidate.name === moduleName)
  if (!module) throw new UsageError('UNKNOWN_MODULE', `unknown module: ${moduleName}`)
  return [
    `${module.name}: ${module.summary}`,
    '',
    'Commands:',
    ...module.commands.map((command) => `  ${command.verb.padEnd(18)} ${command.summary}`),
  ].join('\n')
}

function commandHelp(registry: CommandRegistry, command: RegisteredCommand): string {
  return [
    command.summary,
    '',
    `Usage: ${registry.syntax(command)}`,
    ...(command.description ? ['', command.description] : []),
    ...((command.positionals?.length ?? 0) > 0
      ? [
        '',
        'Arguments:',
        ...command.positionals!.map((position) =>
          `  ${position.name.padEnd(20)} ${position.description}`
        ),
      ]
      : []),
    ...((command.options?.length ?? 0) > 0
      ? [
        '',
        'Options:',
        ...command.options!.map((option) => `  --${option.name.padEnd(18)} ${option.description}`),
      ]
      : []),
    ...((command.examples?.length ?? 0) > 0
      ? ['', 'Examples:', ...command.examples!.map((example) => `  ${example}`)]
      : []),
  ].join('\n')
}

function defaultStdio(): ToolStdio {
  const encoder = new TextEncoder()
  return {
    stdout: async (value) => {
      await Deno.stdout.write(encoder.encode(value))
    },
    stderr: async (value) => {
      await Deno.stderr.write(encoder.encode(value))
    },
    readStdin: async () => await new Response(Deno.stdin.readable).text(),
    prompt: async (message) => {
      await Deno.stderr.write(encoder.encode(message))
      const buffer = new Uint8Array(4096)
      const count = await Deno.stdin.read(buffer)
      return count === null ? null : new TextDecoder().decode(buffer.subarray(0, count)).trim()
    },
    inputIsTerminal: () => Deno.stdin.isTerminal(),
  }
}

async function confirmDeviceIdentity(identity: ImplicitDeviceIdentity): Promise<boolean> {
  if (!Deno.stdin.isTerminal()) {
    throw new ToolError(
      'CONFIRMATION_REQUIRED',
      'using the current device identity requires an interactive terminal or --yes',
      EXIT_PERMISSION,
      false,
      { identity: identity.did },
    )
  }
  const prompt = `Use current device identity ${identity.name} (${identity.did})? ` +
    'This identity may have broad privileges. Continue? [y/N] '
  await Deno.stderr.write(new TextEncoder().encode(prompt))
  const buffer = new Uint8Array(32)
  const count = await Deno.stdin.read(buffer)
  const answer = count ? new TextDecoder().decode(buffer.subarray(0, count)).trim() : ''
  return answer.toLowerCase() === 'y' || answer.toLowerCase() === 'yes'
}
