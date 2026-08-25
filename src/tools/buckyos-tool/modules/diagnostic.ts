import type { CommandDefinition, CommandModule, JsonSchema } from '../core/command.ts'
import type { ArtifactFetcher } from '../core/download.ts'
import { downloadArtifact } from '../core/download.ts'
import type { CommandContext } from '../core/context.ts'
import { ToolError, UsageError } from '../core/errors.ts'
import {
  callService,
  expectObject,
  expectString,
  inputString,
  requiredInputString,
  splitServices,
} from '../core/service.ts'
import { waitForTask } from '../core/task.ts'

const CONTROL_PANEL = 'control-panel'
const OBJECT_OUTPUT: JsonSchema = { type: 'object', additionalProperties: true }

export interface DiagnosticModuleDependencies {
  download?: ArtifactFetcher
}

export function createDiagnosticModule(
  dependencies: DiagnosticModuleDependencies = {},
): CommandModule {
  return {
    name: 'diagnostic',
    summary: 'Collect and export privileged redacted diagnostic bundles',
    commands: [collectCommand(), exportCommand(dependencies)],
  }
}

function collectCommand(): CommandDefinition {
  return {
    verb: 'collect',
    summary: 'Create a redacted diagnostic bundle Task',
    options: [
      {
        name: 'services',
        description: 'Comma-separated service IDs',
        type: 'string',
        required: true,
      },
      { name: 'since', description: 'RFC 3339 lower time boundary', type: 'string' },
      { name: 'until', description: 'RFC 3339 upper time boundary', type: 'string' },
      {
        name: 'no-wait',
        property: 'no_wait',
        description: 'Return after Task creation',
        type: 'boolean',
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        services: {},
        since: { type: 'string', minLength: 1 },
        until: { type: 'string', minLength: 1 },
        no_wait: { type: 'boolean' },
      },
      required: ['services'],
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'privileged' },
    asyncMode: 'task',
    requiresSession: true,
    examples: [
      'buckyos --idempotency-key diag-42 diagnostic collect --services scheduler,node_daemon --since 2026-08-25T00:00:00Z',
    ],
    handler: async (ctx, input) => {
      const services = splitServices(input.services)
      if (services.length === 0) {
        throw new UsageError('INVALID_ARGUMENT', 'at least one diagnostic service is required')
      }
      const response = expectObject(
        await callService(
          ctx,
          CONTROL_PANEL,
          'diagnostic.collect',
          compact({
            services,
            since: inputString(input, 'since'),
            until: inputString(input, 'until'),
            idempotency_key: ctx.idempotencyKey ?? `diagnostic-${crypto.randomUUID()}`,
          }),
        ),
        'Control Panel diagnostic.collect response',
      )
      const taskId = expectString(response.task_id, 'diagnostic.collect.task_id')
      if (input.no_wait === true) return { task_id: taskId }
      return await waitForTask(ctx, taskId)
    },
  }
}

function exportCommand(dependencies: DiagnosticModuleDependencies): CommandDefinition {
  return {
    verb: 'export',
    summary: 'Export one diagnostic bundle to a new file',
    positionals: [{ name: 'bundle_id', description: 'Opaque bundle ID', required: true }],
    options: [{
      name: 'path',
      description: 'New destination file',
      type: 'string',
      required: true,
    }],
    inputSchema: {
      type: 'object',
      properties: {
        bundle_id: { type: 'string', minLength: 1 },
        path: { type: 'string', minLength: 1 },
      },
      required: ['bundle_id', 'path'],
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'privileged' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos diagnostic export diag-opaque --path ./diagnostic.zip'],
    handler: async (ctx, input) => await exportDiagnostic(ctx, input, dependencies),
  }
}

async function exportDiagnostic(
  ctx: CommandContext,
  input: Record<string, unknown>,
  dependencies: DiagnosticModuleDependencies,
): Promise<Record<string, unknown>> {
  const response = expectObject(
    await callService(ctx, CONTROL_PANEL, 'diagnostic.export', {
      bundle_id: requiredInputString(input, 'bundle_id'),
    }),
    'Control Panel diagnostic.export response',
  )
  const downloaded = await downloadArtifact(
    ctx,
    CONTROL_PANEL,
    expectString(response.url, 'diagnostic.export.url'),
    requiredInputString(input, 'path'),
    dependencies.download,
  )
  const expected = typeof response.artifact_sha256 === 'string'
    ? response.artifact_sha256
    : undefined
  if (expected && downloaded.sha256 !== expected) {
    try {
      await Deno.remove(downloaded.path)
    } catch {}
    throw new ToolError('ARTIFACT_HASH_MISMATCH', 'diagnostic archive SHA-256 mismatch')
  }
  return {
    ...downloaded,
    bundle_id: response.bundle_id ?? input.bundle_id,
    content_sha256: response.sha256 ?? null,
    expires_at: response.expires_at ?? null,
  }
}

function compact(value: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).filter(([, item]) => item !== undefined))
}
