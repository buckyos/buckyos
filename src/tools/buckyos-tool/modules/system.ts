import type { CommandModule } from '../core/command.ts'

export function createSystemModule(): CommandModule {
  return {
    name: 'system',
    summary: 'Inspect BuckyOS Zone health and version state',
    commands: [
      {
        verb: 'status',
        summary: 'Get the Zone overview, health, services, and version',
        inputSchema: { type: 'object', properties: {}, additionalProperties: false },
        outputSchema: { type: 'object', additionalProperties: true },
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: true,
        examples: ['buckyos --profile production system status'],
        handler: async (ctx) =>
          await ctx.clients.call<Record<string, unknown>>(
            'control-panel',
            'system.status',
            {},
            {
              traceId: ctx.traceId,
              timeoutMs: Math.max(1, (ctx.deadline ?? Date.now()) - Date.now()),
              signal: ctx.signal,
            },
          ),
      },
    ],
  }
}
