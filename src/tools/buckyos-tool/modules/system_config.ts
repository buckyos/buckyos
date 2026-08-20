import type { CommandDefinition, CommandModule, JsonSchema } from '../core/command.ts'
import type { CommandContext } from '../core/context.ts'
import { EXIT_INTERNAL, ToolError, UsageError } from '../core/errors.ts'

const SERVICE_NAME = 'system_config'
const OBJECT_OUTPUT = { type: 'object' as const, additionalProperties: false }
const KEY_INPUT: JsonSchema = {
  type: 'object',
  properties: { key: { type: 'string', minLength: 1 } },
  required: ['key'],
  additionalProperties: false,
}

export function createSystemConfigModule(): CommandModule {
  return {
    name: 'system-config',
    summary: 'Read and modify the Zone system-config key-value store',
    commands: [
      {
        verb: 'get',
        summary: 'Get one system-config value',
        positionals: [{ name: 'key', description: 'System-config key' }],
        inputSchema: KEY_INPUT,
        outputSchema: {
          ...OBJECT_OUTPUT,
          properties: {
            key: { type: 'string' },
            value: { type: 'string' },
            version: { type: 'integer' },
            text: { type: 'string' },
          },
          required: ['key', 'value', 'version', 'text'],
        },
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: true,
        examples: [
          'buckyos --profile production system-config get boot/config',
          'buckyos --output text system-config get services/example/config',
        ],
        handler: async (ctx, input) => {
          const key = String(input.key)
          const result = await callSystemConfig<unknown>(ctx, 'sys_config_get', { key })
          if (result === null) {
            throw new ToolError(
              'RESOURCE_NOT_FOUND',
              `system-config key not found: ${key}`,
            )
          }
          if (!isConfigValue(result)) invalidResponse('sys_config_get')
          return { key, value: result.value, version: result.version, text: result.value }
        },
      },
      {
        verb: 'list',
        summary: 'List direct child keys under a system-config key',
        positionals: [
          { name: 'key', description: 'Parent key; omit to list the root', required: false },
        ],
        inputSchema: {
          type: 'object',
          properties: { key: { type: 'string' } },
          additionalProperties: false,
        },
        outputSchema: {
          ...OBJECT_OUTPUT,
          properties: {
            key: { type: 'string' },
            items: { type: 'array', items: { type: 'string' } },
          },
          required: ['key', 'items'],
        },
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: true,
        examples: [
          'buckyos system-config list',
          'buckyos system-config list services',
        ],
        handler: async (ctx, input) => {
          const key = typeof input.key === 'string' ? input.key : ''
          const result = await callSystemConfig<unknown>(ctx, 'sys_config_list', { key })
          if (!Array.isArray(result) || !result.every((item) => typeof item === 'string')) {
            invalidResponse('sys_config_list')
          }
          return { key, items: result as string[] }
        },
      },
      setCommand(),
      setFileCommand(),
      mutationCommand({
        verb: 'append',
        summary: 'Append text to an existing system-config value',
        method: 'sys_config_append',
        valueProperty: 'append_value',
        resultProperty: 'appended',
      }),
    ],
  }
}

function setCommand(): CommandDefinition {
  return {
    verb: 'set',
    summary: 'Set one system-config value',
    positionals: [{ name: 'key', description: 'System-config key' }],
    options: [
      { name: 'value', description: 'Value to store', type: 'string', required: true },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        key: { type: 'string', minLength: 1 },
        value: { type: 'string', minLength: 1 },
      },
      required: ['key', 'value'],
      additionalProperties: false,
    },
    outputSchema: {
      ...OBJECT_OUTPUT,
      properties: { key: { type: 'string' }, updated: { type: 'boolean' } },
      required: ['key', 'updated'],
    },
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'write' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos system-config set services/example/config --value enabled'],
    handler: async (ctx, input) => {
      const key = String(input.key)
      await callSystemConfig(ctx, 'sys_config_set', { key, value: String(input.value) })
      return { key, updated: true }
    },
  }
}

function setFileCommand(): CommandDefinition {
  return {
    verb: 'set-file',
    summary: 'Set one system-config value from a file',
    positionals: [{ name: 'key', description: 'System-config key' }],
    options: [
      {
        name: 'file',
        description: 'File whose content will be stored',
        type: 'string',
        required: true,
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        key: { type: 'string', minLength: 1 },
        file: { type: 'string', minLength: 1 },
      },
      required: ['key', 'file'],
      additionalProperties: false,
    },
    outputSchema: {
      ...OBJECT_OUTPUT,
      properties: { key: { type: 'string' }, updated: { type: 'boolean' } },
      required: ['key', 'updated'],
    },
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'write' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos system-config set-file services/example/config --file ./config.json'],
    handler: async (ctx, input) => {
      const key = String(input.key)
      const value = await readValueFile(String(input.file))
      if (!value) throw new UsageError('INVALID_ARGUMENT', 'system-config value must not be empty')
      await callSystemConfig(ctx, 'sys_config_set', { key, value })
      return { key, updated: true }
    },
  }
}

function mutationCommand(options: {
  verb: string
  summary: string
  method: string
  valueProperty: string
  resultProperty: string
}): CommandDefinition {
  return {
    verb: options.verb,
    summary: options.summary,
    positionals: [{ name: 'key', description: 'System-config key' }],
    options: [
      { name: 'value', description: 'Value to append', type: 'string', required: true },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        key: { type: 'string', minLength: 1 },
        value: { type: 'string', minLength: 1 },
      },
      required: ['key', 'value'],
      additionalProperties: false,
    },
    outputSchema: {
      ...OBJECT_OUTPUT,
      properties: {
        key: { type: 'string' },
        [options.resultProperty]: { type: 'boolean' },
      },
      required: ['key', options.resultProperty],
    },
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'write' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: [`buckyos system-config ${options.verb} system/rbac/policy --value 'p,...'`],
    handler: async (ctx, input) => {
      const key = String(input.key)
      await callSystemConfig(ctx, options.method, {
        key,
        [options.valueProperty]: String(input.value),
      })
      return { key, [options.resultProperty]: true }
    },
  }
}

async function callSystemConfig<T = unknown>(
  ctx: CommandContext,
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  return await ctx.clients.call<T>(SERVICE_NAME, method, params, {
    traceId: ctx.traceId,
    timeoutMs: Math.max(1, (ctx.deadline ?? Date.now()) - Date.now()),
    signal: ctx.signal,
  })
}

async function readValueFile(path: string): Promise<string> {
  try {
    return await Deno.readTextFile(path)
  } catch (error) {
    throw new UsageError(
      'INPUT_READ_FAILED',
      `failed to read value file: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

function isConfigValue(value: unknown): value is { value: string; version: number } {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const object = value as Record<string, unknown>
  return typeof object.value === 'string' && Number.isSafeInteger(object.version) &&
    Number(object.version) >= 0
}

function invalidResponse(method: string): never {
  throw new ToolError(
    'INVALID_SERVICE_RESPONSE',
    `system-config returned an invalid response for ${method}`,
    EXIT_INTERNAL,
  )
}
