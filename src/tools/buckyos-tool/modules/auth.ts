import type { CommandModule } from '../core/command.ts'
import { ToolError } from '../core/errors.ts'

const EMPTY_INPUT = { type: 'object' as const, properties: {}, additionalProperties: false }
const OBJECT_OUTPUT = { type: 'object' as const, additionalProperties: true }

export function createAuthModule(): CommandModule {
  return {
    name: 'auth',
    summary: 'Inspect the current authenticated session',
    commands: [
      {
        verb: 'whoami',
        summary: 'Show the effective principal and application identity',
        inputSchema: EMPTY_INPUT,
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: true,
        examples: ['buckyos --profile production auth whoami'],
        handler: (ctx) =>
          Promise.resolve({
            principal: ctx.principal.id,
            appid: ctx.principal.appId,
            app_instance_id: ctx.principal.appInstanceId ?? null,
            authentication: ctx.principal.authentication,
            zone: ctx.connection.zone,
          }),
      },
      {
        verb: 'session-status',
        summary: 'Show the in-memory session state without exposing credentials',
        inputSchema: EMPTY_INPUT,
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: true,
        examples: ['buckyos --profile production auth session-status'],
        handler: (ctx) => {
          if (!ctx.session) {
            throw new ToolError('INTERNAL_ERROR', 'authenticated session is unavailable', 9)
          }
          return Promise.resolve(ctx.session.status())
        },
      },
    ],
  }
}
