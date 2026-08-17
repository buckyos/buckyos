import type { CommandContext } from './context.ts'

export type JsonSchema = {
  type?: 'object' | 'array' | 'string' | 'boolean' | 'integer' | 'number' | 'null'
  title?: string
  description?: string
  properties?: Record<string, JsonSchema>
  required?: string[]
  additionalProperties?: boolean
  items?: JsonSchema
  enum?: unknown[]
  minLength?: number
  minimum?: number
  default?: unknown
  secret?: boolean
}

export interface PositionalDefinition {
  name: string
  description: string
  required?: boolean
}

export interface OptionDefinition {
  name: string
  property?: string
  description: string
  type: 'string' | 'boolean' | 'integer' | 'number'
  required?: boolean
  secret?: boolean
  enum?: string[]
}

export type AccessLevel = 'read' | 'write' | 'destructive' | 'privileged'

export type AccessPolicy =
  | { mode: 'fixed'; level: AccessLevel }
  | { mode: 'operation'; operationIdField: string; possibleLevels: AccessLevel[] }

export interface CommandDefinition {
  verb: string
  summary: string
  description?: string
  positionals?: PositionalDefinition[]
  options?: OptionDefinition[]
  inputSchema: JsonSchema
  outputSchema: JsonSchema
  resultSchemaVersion: number
  access: AccessPolicy
  asyncMode: 'sync' | 'task' | 'either' | 'stream'
  requiresSession: boolean
  examples?: string[]
  supportsRawOutput?: boolean
  handler(ctx: CommandContext, input: Record<string, unknown>): Promise<unknown>
}

export interface CommandModule {
  name: string
  summary: string
  commands: CommandDefinition[]
}

export interface RegisteredCommand extends CommandDefinition {
  module: string
  moduleSummary: string
}

export function optionProperty(option: OptionDefinition): string {
  return option.property ?? option.name.replaceAll('-', '_')
}
