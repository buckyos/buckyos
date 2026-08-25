import type { CommandModule } from '../core/command.ts'
import { ToolError, UsageError } from '../core/errors.ts'
import { callService, expectObject, inputString, parseTimestamp } from '../core/service.ts'

const CONTROL_PANEL = 'control-panel'

export function createAuditModule(): CommandModule {
  return {
    name: 'audit',
    summary: 'Query permission-trimmed durable audit events',
    commands: [{
      verb: 'query',
      summary: 'Query audit events in own or Zone scope',
      options: [
        {
          name: 'scope',
          description: 'Audit visibility scope',
          type: 'string',
          enum: ['own', 'zone'],
        },
        { name: 'actor', description: 'Actor user ID', type: 'string' },
        { name: 'actor-app', property: 'actor_app', description: 'Actor App ID', type: 'string' },
        { name: 'action', description: 'Exact action', type: 'string' },
        { name: 'resource', description: 'Exact resource', type: 'string' },
        { name: 'trace', description: 'Event trace ID', type: 'string' },
        { name: 'since', description: 'Created at or after this time', type: 'string' },
        { name: 'until', description: 'Created at or before this time', type: 'string' },
        { name: 'cursor', description: 'Opaque page cursor', type: 'string' },
        { name: 'limit', description: 'Page size, at most 500', type: 'integer' },
      ],
      inputSchema: {
        type: 'object',
        properties: {
          scope: { type: 'string', enum: ['own', 'zone'] },
          actor: { type: 'string', minLength: 1 },
          actor_app: { type: 'string', minLength: 1 },
          action: { type: 'string', minLength: 1 },
          resource: { type: 'string', minLength: 1 },
          trace: { type: 'string', minLength: 1 },
          since: {},
          until: {},
          cursor: { type: 'string', minLength: 1 },
          limit: { type: 'integer', minimum: 1 },
        },
        additionalProperties: false,
      },
      outputSchema: {
        type: 'object',
        properties: { items: { type: 'array', items: {} }, next_cursor: {} },
        required: ['items', 'next_cursor'],
        additionalProperties: false,
      },
      resultSchemaVersion: 1,
      access: { mode: 'fixed', level: 'privileged' },
      asyncMode: 'sync',
      requiresSession: true,
      examples: ['buckyos audit query --scope own --action task.cancel'],
      handler: async (ctx, input) => {
        const limit = input.limit === undefined ? undefined : Number(input.limit)
        if (limit !== undefined && (!Number.isSafeInteger(limit) || limit < 1 || limit > 500)) {
          throw new UsageError('INVALID_ARGUMENT', 'limit must be an integer between 1 and 500')
        }
        const response = expectObject(
          await callService(
            ctx,
            CONTROL_PANEL,
            'audit.query',
            compact({
              scope: inputString(input, 'scope') ?? 'own',
              actor: inputString(input, 'actor'),
              actor_app: inputString(input, 'actor_app'),
              action: inputString(input, 'action'),
              resource: inputString(input, 'resource'),
              trace_id: inputString(input, 'trace'),
              created_after: parseTimestamp(input.since, 'since'),
              created_before: parseTimestamp(input.until, 'until'),
              cursor: inputString(input, 'cursor'),
              limit,
            }),
          ),
          'Control Panel audit.query response',
        )
        if (!Array.isArray(response.items)) {
          throw new ToolError('INVALID_SERVICE_RESPONSE', 'audit.query items must be an array', 9)
        }
        return { items: response.items, next_cursor: response.next_cursor ?? null }
      },
    }],
  }
}

function compact(value: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).filter(([, item]) => item !== undefined))
}
