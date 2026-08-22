import type { CommandContext } from './context.ts'
import { EXIT_OPERATION, EXIT_TIMEOUT, ToolError } from './errors.ts'

const TASK_MANAGER_SERVICE = 'task-manager'
const DEFAULT_POLL_INTERVAL_MS = 500

export interface TaskObservation {
  phase: string
  outcome?: string
  message?: string
  error?: {
    code?: string
    message?: string
    retryable?: boolean
    details?: Record<string, unknown>
  }
  data: Record<string, unknown>
}

export interface TaskWaitOptions {
  observe?: (ctx: CommandContext, taskId: string) => Promise<TaskObservation>
  pollIntervalMs?: number
  sleep?: (milliseconds: number, signal: AbortSignal) => Promise<void>
}

export async function waitForTask(
  ctx: CommandContext,
  taskId: string,
  options: TaskWaitOptions = {},
): Promise<Record<string, unknown>> {
  const observe = options.observe ?? observeTask
  const sleep = options.sleep ?? abortableSleep
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS
  let lastProgress = ''

  while (true) {
    const observation = await observe(ctx, taskId)
    const progress = JSON.stringify({
      task_id: taskId,
      phase: observation.phase,
      outcome: observation.outcome,
      message: observation.message,
    })
    if (progress !== lastProgress) {
      await ctx.io.stderr(`${progress}\n`)
      lastProgress = progress
    }

    if (observation.phase === 'Terminal') {
      if (observation.outcome !== 'Succeeded') {
        const error = observation.error
        throw new ToolError(
          normalizeTaskErrorCode(error?.code, observation.outcome),
          error?.message ?? `task ${taskId} ended with ${observation.outcome ?? 'no outcome'}`,
          observation.outcome === 'Canceled' ? EXIT_TIMEOUT : EXIT_OPERATION,
          error?.retryable ?? false,
          { task_id: taskId, ...(error?.details ?? {}) },
        )
      }
      return observation.data
    }

    const remaining = (ctx.deadline ?? Date.now()) - Date.now()
    if (remaining <= 0) {
      throw new ToolError(
        'TIMEOUT',
        `timed out waiting for task ${taskId}`,
        EXIT_TIMEOUT,
        true,
        { task_id: taskId },
      )
    }
    await sleep(Math.min(pollIntervalMs, remaining), ctx.signal)
  }
}

async function observeTask(ctx: CommandContext, taskId: string): Promise<TaskObservation> {
  const response = await ctx.clients.call<unknown>(
    TASK_MANAGER_SERVICE,
    'get_task',
    { task_id: taskId },
    rpcOptions(ctx),
  )
  const envelope = expectObject(response, 'TaskManager get_task response')
  const task = expectObject(envelope.task ?? envelope, 'TaskManager task')
  const phase = expectString(task.phase, 'task.phase')
  const outcome = optionalString(task.outcome)
  const taskError = isObject(task.error) ? task.error : undefined
  return {
    phase,
    outcome,
    message: optionalString(task.message),
    error: taskError
      ? {
        code: optionalString(taskError.code),
        message: optionalString(taskError.message),
        details: isObject(taskError.detail) ? taskError.detail : undefined,
      }
      : undefined,
    data: {
      task_id: taskId,
      schema_id: optionalString(task.schema_id),
      phase,
      outcome,
      message: optionalString(task.message),
      result: task.result ?? null,
      error: task.error ?? null,
      updated_at: task.updated_at,
      completed_at: task.completed_at ?? null,
    },
  }
}

function rpcOptions(ctx: CommandContext): {
  traceId: string
  timeoutMs: number
  signal: AbortSignal
} {
  return {
    traceId: ctx.traceId,
    timeoutMs: Math.max(1, (ctx.deadline ?? Date.now()) - Date.now()),
    signal: ctx.signal,
  }
}

function normalizeTaskErrorCode(code: string | undefined, outcome: string | undefined): string {
  if (code) return code.trim().replaceAll(/[^A-Za-z0-9]+/g, '_').toUpperCase()
  return outcome === 'Canceled' ? 'CANCELED' : 'TASK_FAILED'
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function expectString(value: unknown, label: string): string {
  if (typeof value !== 'string' || !value) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', `${label} must be a string`, 9)
  }
  return value
}

function isObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

function expectObject(value: unknown, label: string): Record<string, unknown> {
  if (!isObject(value)) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', `${label} must be an object`, 9)
  }
  return value
}

function abortableSleep(milliseconds: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) {
    return Promise.reject(new ToolError('CANCELED', 'operation canceled', EXIT_TIMEOUT))
  }
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, milliseconds)
    const onAbort = () => {
      clearTimeout(timer)
      reject(new ToolError('CANCELED', 'operation canceled', EXIT_TIMEOUT))
    }
    signal.addEventListener('abort', onAbort, { once: true })
  })
}
