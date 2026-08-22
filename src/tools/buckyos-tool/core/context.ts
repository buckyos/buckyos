import type { OutputFormat } from './argv.ts'
import type { ConfigStore, ResolvedConfig } from './config.ts'
import type { RegisteredCommand } from './command.ts'
import type { ResolvedPrincipal, SessionController } from './auth.ts'
import type { ServiceClientRegistry } from './runtime.ts'

export interface ResolvedConnection {
  zone: string
  endpoint: string
  defaultProtocol: 'http://' | 'https://'
}

export interface CommandIO {
  stderr(value: string): Promise<void>
  prompt(message: string): Promise<string | null>
  inputIsTerminal: boolean
}

export interface CommandContext {
  command: { module: string; verb: string }
  definition: RegisteredCommand
  connection: ResolvedConnection
  principal: ResolvedPrincipal
  clients: ServiceClientRegistry
  output: { format: OutputFormat }
  traceId: string
  idempotencyKey?: string
  deadline?: number
  signal: AbortSignal
  cwd: string
  io: CommandIO
  interactive: boolean
  confirmed: boolean
  config: ResolvedConfig
  configStore: ConfigStore
  session?: SessionController
}

export interface MockContextOptions {
  command: RegisteredCommand
  config: ResolvedConfig
  configStore: ConfigStore
  clients: ServiceClientRegistry
  principal?: ResolvedPrincipal
  traceId?: string
}

export function createMockCommandContext(options: MockContextOptions): CommandContext {
  return {
    command: { module: options.command.module, verb: options.command.verb },
    definition: options.command,
    connection: {
      zone: options.config.zone ?? 'mock.invalid',
      endpoint: options.config.endpoint ?? 'https://mock.invalid',
      defaultProtocol: options.config.defaultProtocol,
    },
    principal: options.principal ?? {
      id: 'mock-user',
      appId: 'buckycli',
      authentication: 'mock',
    },
    clients: options.clients,
    output: { format: options.config.output },
    traceId: options.traceId ?? crypto.randomUUID(),
    signal: new AbortController().signal,
    cwd: Deno.cwd(),
    io: {
      stderr: () => Promise.resolve(),
      prompt: () => Promise.resolve(null),
      inputIsTerminal: false,
    },
    interactive: false,
    confirmed: false,
    config: options.config,
    configStore: options.configStore,
  }
}
