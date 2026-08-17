import { createInterface } from 'node:readline/promises'
import { stderr, stdin } from 'node:process'
import type { CommandRegistry } from './registry.ts'
import type { ConfigStore, ResolvedConfig } from './config.ts'
import { effectiveConfigView } from './config.ts'
import { parseShellLine } from './argv.ts'
import { normalizeError } from './errors.ts'
import { ReplHistory } from './history.ts'
import type { InteractiveSession } from './runtime.ts'

export interface ReplOptions {
  registry: CommandRegistry
  config: ResolvedConfig
  configStore: ConfigStore
  session: InteractiveSession
  execute(tokens: string[], signal: AbortSignal): Promise<void>
}

export async function runRepl(options: ReplOptions): Promise<void> {
  const history = new ReplHistory(options.configStore.historyPath())
  const savedHistory = await history.load()
  const completer = (line: string): [string[], string] => {
    const fragment = line.match(/[^\s]*$/)?.[0] ?? ''
    const prefix = line.slice(0, line.length - fragment.length)
    let tokens: string[]
    try {
      tokens = parseShellLine(prefix)
    } catch {
      tokens = prefix.trim().split(/\s+/).filter(Boolean)
    }
    tokens.push(fragment)
    const candidates = options.registry.completionCandidates(tokens)
    return [candidates.filter((candidate) => candidate.startsWith(fragment)), fragment]
  }
  const readline = createInterface({
    input: stdin,
    output: stderr,
    terminal: true,
    history: [...savedHistory].reverse(),
    historySize: history.limit,
    completer,
  })
  const prompt = replPrompt(options.config)
  let running: AbortController | undefined

  readline.on('SIGINT', () => {
    if (running) running.abort()
    else {
      stderr.write('^C\n')
      readline.prompt()
    }
  })
  stderr.write('BuckyOS interactive session. Use :help for commands and :exit to quit.\n')
  readline.setPrompt(prompt)
  readline.prompt()

  try {
    for await (const rawLine of readline) {
      const line = rawLine.trim()
      if (!line) {
        readline.prompt()
        continue
      }
      if (line.startsWith(':')) {
        const shouldExit = await handleBuiltin(line, options, history)
        if (shouldExit) break
        readline.prompt()
        continue
      }

      let tokens: string[]
      try {
        tokens = parseShellLine(line)
      } catch (error) {
        const normalized = normalizeError(error)
        stderr.write(`${normalized.code}: ${normalized.message}\n`)
        readline.prompt()
        continue
      }
      let command
      try {
        if (tokens.length >= 2) command = options.registry.get(tokens[0], tokens[1])
      } catch {
        command = undefined
      }
      await history.add(line, command)
      running = new AbortController()
      try {
        await options.execute(tokens, running.signal)
      } catch (error) {
        const normalized = normalizeError(error)
        stderr.write(`${normalized.code}: ${normalized.message}\n`)
      } finally {
        running = undefined
      }
      readline.prompt()
    }
  } finally {
    readline.close()
  }
}

async function handleBuiltin(
  line: string,
  options: ReplOptions,
  history: ReplHistory,
): Promise<boolean> {
  const command = line.split(/\s+/, 1)[0]
  switch (command) {
    case ':exit':
    case ':quit':
      return true
    case ':help':
      stderr.write(
        [
          'Enter: <module> <verb> [primary-selector] [action-options]',
          'Built-ins: :help :context :session :history :reconnect :exit :quit',
          `Modules: ${options.registry.modules().map((module) => module.name).join(', ')}`,
          '',
        ].join('\n'),
      )
      break
    case ':context':
      stderr.write(`${JSON.stringify(effectiveConfigView(options.config), null, 2)}\n`)
      break
    case ':session':
      stderr.write(`${JSON.stringify(options.session.authentication.status(), null, 2)}\n`)
      break
    case ':history':
      stderr.write(
        `${history.entries().map((entry, index) => `${index + 1}  ${entry}`).join('\n')}\n`,
      )
      break
    case ':reconnect':
      try {
        await options.session.reconnect()
        stderr.write('session reconnected\n')
      } catch (error) {
        const normalized = normalizeError(error)
        stderr.write(`${normalized.code}: ${normalized.message}\n`)
      }
      break
    default:
      stderr.write(`unknown REPL command: ${command}\n`)
  }
  return false
}

function replPrompt(config: ResolvedConfig): string {
  const profile = config.profileName ?? 'default'
  const zone = config.zone ?? (config.endpoint ? new URL(config.endpoint).host : 'unresolved')
  const identity = config.identity ?? 'external-session'
  return `buckyos[${profile}|${zone}|${identity}]> `
}
