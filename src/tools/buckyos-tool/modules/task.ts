import type { CommandDefinition, CommandModule, JsonSchema } from '../core/command.ts'
import type { CommandContext } from '../core/context.ts'
import { EXIT_INTERNAL, ToolError, UsageError } from '../core/errors.ts'
import {
  callService,
  expectObject,
  expectString,
  inputString,
  parseTimestamp,
  requiredInputString,
} from '../core/service.ts'
import { waitForTask } from '../core/task.ts'

const TASK_MANAGER = 'task-manager'
const CONTROL_PANEL = 'control-panel'
const OBJECT_OUTPUT: JsonSchema = { type: 'object', additionalProperties: true }

export function createTaskModule(): CommandModule {
  return {
    name: 'task',
    summary: 'Inspect, wait for, cancel, and retry durable Tasks',
    commands: [listCommand(), getCommand(), waitCommand(), cancelCommand(), retryCommand()],
  }
}

function listCommand(): CommandDefinition {
  return {
    verb: 'list',
    summary: 'List visible Tasks',
    options: [
      { name: 'owner', description: 'Creator user ID', type: 'string' },
      { name: 'type', description: 'Task schema ID', type: 'string' },
      { name: 'state', description: 'Task phase', type: 'string' },
      { name: 'since', description: 'Created at or after this time', type: 'string' },
      { name: 'until', description: 'Created at or before this time', type: 'string' },
      { name: 'cursor', description: 'Opaque page cursor', type: 'string' },
      { name: 'limit', description: 'Page size, at most 500', type: 'integer' },
      {
        name: 'include-archived',
        property: 'include_archived',
        description: 'Include archived Tasks',
        type: 'boolean',
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        owner: { type: 'string', minLength: 1 },
        type: { type: 'string', minLength: 1 },
        state: { type: 'string', minLength: 1 },
        since: {},
        until: {},
        cursor: { type: 'string', minLength: 1 },
        limit: { type: 'integer', minimum: 1 },
        include_archived: { type: 'boolean' },
      },
      additionalProperties: false,
    },
    outputSchema: pageSchema(),
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos task list --state running --limit 50'],
    handler: async (ctx, input) => {
      const limit = normalizeLimit(input.limit)
      const response = expectObject(
        await callService(
          ctx,
          TASK_MANAGER,
          'list_tasks',
          compact({
            creator_user_id: inputString(input, 'owner'),
            schema_id: inputString(input, 'type'),
            phase: input.state === undefined ? undefined : normalizePhase(String(input.state)),
            created_after: parseTimestamp(input.since, 'since'),
            created_before: parseTimestamp(input.until, 'until'),
            cursor: inputString(input, 'cursor'),
            limit,
            include_archived: input.include_archived === true,
          }),
        ),
        'TaskManager list_tasks response',
      )
      const items = response.tasks
      if (!Array.isArray(items)) invalidResponse('list_tasks.tasks')
      return { items, next_cursor: response.next_cursor ?? null }
    },
  }
}

function getCommand(): CommandDefinition {
  return {
    verb: 'get',
    summary: 'Get one Task snapshot and its direct children',
    positionals: [{ name: 'task_id', description: 'Opaque Task ID', required: true }],
    options: [
      {
        name: 'children-cursor',
        property: 'children_cursor',
        description: 'Child page cursor',
        type: 'string',
      },
      {
        name: 'children-limit',
        property: 'children_limit',
        description: 'Child page size',
        type: 'integer',
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        task_id: { type: 'string', minLength: 1 },
        children_cursor: { type: 'string', minLength: 1 },
        children_limit: { type: 'integer', minimum: 1 },
      },
      required: ['task_id'],
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos task get t-0123456789abcdef0123456789abcdef'],
    handler: async (ctx, input) => {
      const taskId = requiredInputString(input, 'task_id')
      const [taskResponse, childrenResponse] = await Promise.all([
        callService(ctx, TASK_MANAGER, 'get_task', { task_id: taskId }),
        callService(
          ctx,
          TASK_MANAGER,
          'get_subtasks',
          compact({
            task_id: taskId,
            cursor: inputString(input, 'children_cursor'),
            limit: normalizeLimit(input.children_limit),
          }),
        ),
      ])
      const taskEnvelope = expectObject(taskResponse, 'TaskManager get_task response')
      const childrenEnvelope = expectObject(childrenResponse, 'TaskManager get_subtasks response')
      if (!Array.isArray(childrenEnvelope.tasks)) invalidResponse('get_subtasks.tasks')
      return {
        task: expectObject(taskEnvelope.task ?? taskEnvelope, 'TaskManager task'),
        children: {
          items: childrenEnvelope.tasks,
          next_cursor: childrenEnvelope.next_cursor ?? null,
        },
      }
    },
  }
}

function waitCommand(): CommandDefinition {
  return {
    verb: 'wait',
    summary: 'Stream changes until a Task becomes terminal',
    positionals: [{ name: 'task_id', description: 'Opaque Task ID', required: true }],
    inputSchema: taskIdSchema(),
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'read' },
    asyncMode: 'stream',
    requiresSession: true,
    examples: ['buckyos --timeout 10m task wait t-0123456789abcdef0123456789abcdef'],
    handler: async (ctx, input) => {
      const taskId = requiredInputString(input, 'task_id')
      return await waitForTask(ctx, taskId, {
        failOnTaskFailure: false,
        onObservation: async (observation) => {
          await ctx.io.stdout(`${
            JSON.stringify({
              schema_version: 1,
              type: 'task-progress',
              task_id: taskId,
              revision: observation.revision ?? null,
              phase: observation.phase,
              outcome: observation.outcome ?? null,
              progress: observation.progress ?? null,
              message: observation.message ?? null,
            })
          }\n`)
        },
      })
    },
  }
}

function cancelCommand(): CommandDefinition {
  return {
    verb: 'cancel',
    summary: 'Request Task cancellation',
    positionals: [{ name: 'task_id', description: 'Opaque Task ID', required: true }],
    options: [
      { name: 'recursive', description: 'Request cancellation for descendants', type: 'boolean' },
      {
        name: 'expected-revision',
        property: 'expected_revision',
        description: 'Optimistic revision fence',
        type: 'integer',
      },
    ],
    inputSchema: {
      type: 'object',
      properties: {
        task_id: { type: 'string', minLength: 1 },
        recursive: { type: 'boolean' },
        expected_revision: { type: 'integer', minimum: 0 },
      },
      required: ['task_id'],
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'write' },
    asyncMode: 'sync',
    requiresSession: true,
    examples: ['buckyos task cancel t-0123456789abcdef0123456789abcdef --recursive'],
    handler: async (ctx, input) => {
      const taskId = requiredInputString(input, 'task_id')
      return await callService(
        ctx,
        TASK_MANAGER,
        'request_control',
        compact({
          task_id: taskId,
          action: 'Cancel',
          request_id: ctx.idempotencyKey ?? `ctl-${crypto.randomUUID().replaceAll('-', '')}`,
          recursive: input.recursive === true,
          expected_revision: input.expected_revision,
        }),
      )
    },
  }
}

function retryCommand(): CommandDefinition {
  return {
    verb: 'retry',
    summary: 'Create a new Task through the owning domain retry handler',
    positionals: [{ name: 'task_id', description: 'Failed terminal Task ID', required: true }],
    options: [{
      name: 'no-wait',
      property: 'no_wait',
      description: 'Return after creation',
      type: 'boolean',
    }],
    inputSchema: {
      type: 'object',
      properties: {
        task_id: { type: 'string', minLength: 1 },
        no_wait: { type: 'boolean' },
      },
      required: ['task_id'],
      additionalProperties: false,
    },
    outputSchema: OBJECT_OUTPUT,
    resultSchemaVersion: 1,
    access: { mode: 'fixed', level: 'write' },
    asyncMode: 'either',
    requiresSession: true,
    examples: ['buckyos --idempotency-key retry-42 task retry t-0123456789abcdef0123456789abcdef'],
    handler: async (ctx, input) => {
      const taskId = requiredInputString(input, 'task_id')
      const oldEnvelope = expectObject(
        await callService(ctx, TASK_MANAGER, 'get_task', { task_id: taskId }),
        'TaskManager get_task response',
      )
      const oldTask = expectObject(oldEnvelope.task ?? oldEnvelope, 'TaskManager task')
      if (oldTask.phase !== 'Terminal' || oldTask.outcome !== 'Failed') {
        throw new UsageError('TASK_NOT_RETRYABLE', 'only a terminal failed Task can be retried')
      }
      const response = expectObject(
        await callService(ctx, CONTROL_PANEL, 'task.retry', {
          task_id: taskId,
          idempotency_key: ctx.idempotencyKey ?? `retry-${crypto.randomUUID()}`,
        }),
        'Control Panel task.retry response',
      )
      const retryTaskId = expectString(response.task_id, 'task.retry.task_id')
      if (retryTaskId === taskId || response.retry_of !== taskId) {
        invalidResponse('task.retry identity')
      }
      if (input.no_wait === true) return response
      return await waitForTask(ctx, retryTaskId)
    },
  }
}

function taskIdSchema(): JsonSchema {
  return {
    type: 'object',
    properties: { task_id: { type: 'string', minLength: 1 } },
    required: ['task_id'],
    additionalProperties: false,
  }
}

function pageSchema(): JsonSchema {
  return {
    type: 'object',
    properties: {
      items: { type: 'array', items: {} },
      next_cursor: {},
    },
    required: ['items', 'next_cursor'],
    additionalProperties: false,
  }
}

function normalizePhase(value: string): string {
  const normalized = value.trim().toLowerCase()
  const phase = ['promised', 'accepted', 'running', 'waiting', 'paused', 'terminal']
    .find((candidate) => candidate === normalized)
  if (!phase) throw new UsageError('INVALID_ARGUMENT', `unknown Task phase: ${value}`)
  return phase[0].toUpperCase() + phase.slice(1)
}

function normalizeLimit(value: unknown): number | undefined {
  if (value === undefined) return undefined
  if (!Number.isSafeInteger(value) || Number(value) < 1 || Number(value) > 500) {
    throw new UsageError('INVALID_ARGUMENT', 'limit must be an integer between 1 and 500')
  }
  return Number(value)
}

function compact(value: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).filter(([, item]) => item !== undefined))
}

function invalidResponse(label: string): never {
  throw new ToolError('INVALID_SERVICE_RESPONSE', `${label} is invalid`, EXIT_INTERNAL)
}
