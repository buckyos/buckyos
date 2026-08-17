import { dirname } from 'node:path'
import type { RegisteredCommand } from './command.ts'
import { optionProperty } from './command.ts'
import { parseShellLine } from './argv.ts'

const SECRET_WORDS =
  /(?:session[-_]?token|refresh[-_]?token|password|private[-_]?key|sudo[-_]?token)/i

export class ReplHistory {
  readonly path: string
  readonly limit: number
  #entries: string[] = []

  constructor(path: string, limit = 500) {
    this.path = path
    this.limit = limit
  }

  async load(): Promise<string[]> {
    try {
      this.#entries = (await Deno.readTextFile(this.path))
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean)
        .slice(-this.limit)
    } catch (error) {
      if (!(error instanceof Deno.errors.NotFound)) throw error
      this.#entries = []
    }
    return [...this.#entries]
  }

  entries(): string[] {
    return [...this.#entries]
  }

  async add(line: string, command?: RegisteredCommand): Promise<void> {
    if (!shouldPersistHistory(line, command)) return
    if (this.#entries.at(-1) !== line) this.#entries.push(line)
    this.#entries = this.#entries.slice(-this.limit)
    await Deno.mkdir(dirname(this.path), { recursive: true, mode: 0o700 })
    await Deno.writeTextFile(this.path, `${this.#entries.join('\n')}\n`, { mode: 0o600 })
    if (Deno.build.os !== 'windows') await Deno.chmod(this.path, 0o600)
  }
}

export function shouldPersistHistory(line: string, command?: RegisteredCommand): boolean {
  if (SECRET_WORDS.test(line)) return false
  if (!command) return true
  if (Object.values(command.inputSchema.properties ?? {}).some((schema) => schema.secret)) {
    return false
  }
  let tokens: string[]
  try {
    tokens = parseShellLine(line)
  } catch {
    return false
  }
  const secretOptions = new Set(
    (command.options ?? []).filter((option) => option.secret).flatMap((option) => [
      `--${option.name}`,
      optionProperty(option),
    ]),
  )
  return !tokens.some((token) => secretOptions.has(token.split('=', 1)[0]))
}
