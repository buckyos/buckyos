import type { CommandDefinition, CommandModule, JsonSchema } from '../core/command.ts'
import type { CommandContext } from '../core/context.ts'
import { type ArtifactFetcher, downloadArtifact } from '../core/download.ts'
import { EXIT_TIMEOUT, ToolError, UsageError } from '../core/errors.ts'
import {
  callService,
  expectObject,
  expectString,
  inputString,
  requiredInputString,
  splitServices,
} from '../core/service.ts'

const CONTROL_PANEL = 'control-panel'
const OBJECT_OUTPUT: JsonSchema = { type: 'object', additionalProperties: true }

export interface LogModuleDependencies {
  sleep?: (milliseconds: number, signal: AbortSignal) => Promise<void>
  download?: ArtifactFetcher
}

export function createLogModule(dependencies: LogModuleDependencies = {}): CommandModule {
  return {
    name: 'log',
    summary: 'Query, follow, and export redacted system logs',
    commands: [queryCommand(), tailCommand(dependencies), exportCommand(dependencies)],
  }
}

function queryCommand(): CommandDefinition {
  return {
    verb: 'query',
    summary: 'Query structured redacted log entries',
    options: [
      ...filterOptions(false),
      {
        name: 'direction',
        description: 'Page direction',
        type: 'string',
        enum: ['forward', 'backward'],
      },
      { name: 'cursor', description: 'Opaque page cursor', type: 'string' },
      { name: 'limit', description: 'Page size, at most 500', type: 'integer' },
    ],
    inputSchema: filterSchema({
      direction: { type: 'string', enum: ['forward', 'backward'] },
      cursor: { type: 'string', minLength: 1 },
      limit: { type: 'integer', minimum: 1 },
    }),
    outputSchema: pageSchema(),
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos log query --service scheduler --level error --limit 100'],
    handler: async (ctx, input) => {
      const params = normalizeFilter(input, true)
      const response = expectObject(
        await callService(ctx, CONTROL_PANEL, 'system.logs.query', params),
        'Control Panel log query response',
      )
      if (!Array.isArray(response.entries)) invalidResponse('log query entries')
      return { items: response.entries, next_cursor: response.nextCursor ?? null }
    },
  }
}

function tailCommand(dependencies: LogModuleDependencies): CommandDefinition {
  const sleep = dependencies.sleep ?? abortableSleep
  return {
    verb: 'tail',
    summary: 'Continuously stream structured redacted log entries',
    options: [
      ...filterOptions(false),
      {
        name: 'from',
        description: 'Initial read position',
        type: 'string',
        enum: ['start', 'end'],
      },
    ],
    inputSchema: filterSchema({ from: { type: 'string', enum: ['start', 'end'] } }),
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'stream',
    requiresSession: true,
    examples: ['buckyos --timeout 10m log tail --service scheduler --from end'],
    handler: async (ctx, input) => {
      const params = normalizeFilter(input, false)
      if (params.services.length !== 1) {
        throw new UsageError('INVALID_ARGUMENT', 'log tail requires exactly one service')
      }
      let cursor: string | undefined
      let emitted = 0
      while (true) {
        const remaining = (ctx.deadline ?? Date.now()) - Date.now()
        if (remaining <= 0) throw timeoutError()
        const response = expectObject(
          await callService(ctx, CONTROL_PANEL, 'system.logs.tail', {
            ...params,
            cursor,
            from: inputString(input, 'from') ?? 'end',
            limit: 500,
          }),
          'Control Panel log tail response',
        )
        if (!Array.isArray(response.entries)) invalidResponse('log tail entries')
        for (const entry of response.entries) {
          await ctx.io.stdout(`${
            JSON.stringify({
              schema_version: 1,
              type: 'log-entry',
              entry,
            })
          }\n`)
          emitted += 1
        }
        cursor = expectString(response.nextCursor, 'log tail nextCursor')
        await sleep(
          Math.min(500, Math.max(1, (ctx.deadline ?? Date.now()) - Date.now())),
          ctx.signal,
        )
      }
      return { emitted }
    },
  }
}

function exportCommand(dependencies: LogModuleDependencies): CommandDefinition {
  return {
    verb: 'export',
    summary: 'Export a bounded redacted log archive to a new file',
    options: [
      ...filterOptions(true),
      { name: 'path', description: 'New destination file', type: 'string', required: true },
    ],
    inputSchema: {
      ...filterSchema({ path: { type: 'string', minLength: 1 } }),
      required: ['path'],
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: [
      'buckyos log export --services scheduler,node_daemon --since 2026-08-25T00:00:00Z --path ./logs.zip',
    ],
    handler: async (ctx, input) => {
      const params = normalizeFilter(input, false)
      if (params.services.length === 0) {
        throw new UsageError('INVALID_ARGUMENT', 'log export requires at least one service')
      }
      if (!params.since && !params.until) {
        throw new UsageError(
          'INVALID_ARGUMENT',
          'log export requires a since or until time boundary',
        )
      }
      const response = expectObject(
        await callService(ctx, CONTROL_PANEL, 'system.logs.download', {
          ...params,
          mode: 'filtered',
        }),
        'Control Panel log export response',
      )
      const downloaded = await downloadArtifact(
        ctx,
        CONTROL_PANEL,
        expectString(response.url, 'log export url'),
        requiredInputString(input, 'path'),
        dependencies.download,
      )
      return downloaded
    },
  }
}

function filterOptions(plural: boolean) {
  return [
    {
      name: plural ? 'services' : 'service',
      property: plural ? 'services' : 'service',
      description: plural ? 'Comma-separated service IDs' : 'Service ID',
      type: 'string' as const,
      required: true,
    },
    {
      name: 'file',
      description: 'Exact filename in the service log directory',
      type: 'string' as const,
    },
    { name: 'level', description: 'Exact normalized log level', type: 'string' as const },
    { name: 'keyword', description: 'Case-insensitive content filter', type: 'string' as const },
    { name: 'since', description: 'RFC 3339 lower time boundary', type: 'string' as const },
    { name: 'until', description: 'RFC 3339 upper time boundary', type: 'string' as const },
  ]
}

function filterSchema(extra: Record<string, JsonSchema>): JsonSchema {
  return {
    type: 'object',
    properties: {
      service: { type: 'string', minLength: 1 },
      services: {},
      file: { type: 'string', minLength: 1 },
      level: { type: 'string', minLength: 1 },
      keyword: { type: 'string', minLength: 1 },
      since: { type: 'string', minLength: 1 },
      until: { type: 'string', minLength: 1 },
      ...extra,
    },
    additionalProperties: false,
  }
}

function normalizeFilter(
  input: Record<string, unknown>,
  withPage: boolean,
): Record<string, unknown> & { services: string[] } {
  const services = splitServices(input.services ?? input.service)
  if (services.length === 0) throw new UsageError('INVALID_ARGUMENT', 'service is required')
  const limit = input.limit === undefined ? undefined : Number(input.limit)
  if (limit !== undefined && (!Number.isSafeInteger(limit) || limit < 1 || limit > 500)) {
    throw new UsageError('INVALID_ARGUMENT', 'limit must be an integer between 1 and 500')
  }
  return compact({
    services,
    file: inputString(input, 'file'),
    level: inputString(input, 'level')?.toLowerCase(),
    keyword: inputString(input, 'keyword'),
    since: normalizeLogTime(input, 'since'),
    until: normalizeLogTime(input, 'until'),
    direction: withPage ? inputString(input, 'direction') ?? 'forward' : undefined,
    cursor: withPage ? inputString(input, 'cursor') : undefined,
    limit: withPage ? limit : undefined,
  }) as Record<string, unknown> & { services: string[] }
}

function normalizeLogTime(input: Record<string, unknown>, key: string): string | undefined {
  const value = inputString(input, key)
  if (!value) return undefined
  if (!Number.isFinite(Date.parse(value))) {
    throw new UsageError('INVALID_ARGUMENT', `${key} must be RFC 3339`)
  }
  return value
}

function pageSchema(): JsonSchema {
  return {
    type: 'object',
    properties: { items: { type: 'array', items: {} }, next_cursor: {} },
    required: ['items', 'next_cursor'],
    additionalProperties: false,
  }
}

function compact(value: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).filter(([, item]) => item !== undefined))
}

function invalidResponse(label: string): never {
  throw new ToolError('INVALID_SERVICE_RESPONSE', `${label} is invalid`)
}

function timeoutError(): ToolError {
  return new ToolError('TIMEOUT', 'timed out following logs', EXIT_TIMEOUT, true)
}

function abortableSleep(milliseconds: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(new ToolError('CANCELED', 'operation canceled'))
  return new Promise((resolve, reject) => {
    const onAbort = () => {
      clearTimeout(timer)
      reject(new ToolError('CANCELED', 'operation canceled'))
    }
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, milliseconds)
    signal.addEventListener('abort', onAbort, { once: true })
  })
}
