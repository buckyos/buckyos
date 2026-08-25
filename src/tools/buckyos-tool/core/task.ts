import type { CommandContext } from './context.ts'
import { EXIT_OPERATION, EXIT_TIMEOUT, ToolError } from './errors.ts'

const TASK_MANAGER_SERVICE = 'task-manager'
const DEFAULT_POLL_INTERVAL_MS = 500

export interface TaskObservation {
  revision?: number
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
  progress?: unknown
}

export interface TaskWaitOptions {
  observe?: (ctx: CommandContext, taskId: string) => Promise<TaskObservation>
  pollIntervalMs?: number
  sleep?: (milliseconds: number, signal: AbortSignal) => Promise<void>
  failOnTaskFailure?: boolean
  onObservation?: (observation: TaskObservation) => Promise<void>
}

export async function waitForTask(
  ctx: CommandContext,
  taskId: string,
  options: TaskWaitOptions = {},
): Promise<Record<string, unknown>> {
  const observe = options.observe ?? observeTask
  const sleep = options.sleep ?? abortableSleep
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS
  const failOnTaskFailure = options.failOnTaskFailure ?? true
  let lastProgress = ''
  let lastRevision: number | undefined
  let reader: Awaited<ReturnType<NonNullable<typeof ctx.clients.createEventReader>>> | undefined

  try {
    const remaining = Math.max(1, (ctx.deadline ?? Date.now()) - Date.now())
    const eventSignal = AbortSignal.any([ctx.signal, AbortSignal.timeout(remaining)])
    reader = await ctx.clients.createEventReader?.(`/task_mgr/${taskId}`, eventSignal)
  } catch {
    reader = undefined
  }

  try {
    while (true) {
      const observation = await observe(ctx, taskId)
      const progress = JSON.stringify({
        task_id: taskId,
        revision: observation.revision,
        phase: observation.phase,
        outcome: observation.outcome,
        progress: observation.progress,
        message: observation.message,
      })
      const changed = observation.revision === undefined
        ? progress !== lastProgress
        : observation.revision !== lastRevision
      if (changed) {
        if (options.onObservation) await options.onObservation(observation)
        else await ctx.io.stderr(`${progress}\n`)
        lastProgress = progress
        lastRevision = observation.revision
      }

      if (observation.phase === 'Terminal') {
        if (failOnTaskFailure && observation.outcome !== 'Succeeded') {
          const error = observation.error
          throw new ToolError(
            normalizeTaskErrorCode(error?.code, observation.outcome),
            error?.message ?? `task ${taskId} ended with ${observation.outcome ?? 'no outcome'}`,
            observation.outcome === 'Canceled' ? EXIT_TIMEOUT : EXIT_OPERATION,
            error?.retryable ?? false,
            { task_id: taskId },
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
      const interval = Math.min(pollIntervalMs, remaining)
      if (reader) await reader.pullEvent(interval)
      else await sleep(interval, ctx.signal)
    }
  } finally {
    await reader?.close().catch(() => undefined)
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
    revision: typeof task.revision === 'number' ? task.revision : undefined,
    phase,
    outcome,
    message: optionalString(task.message),
    error: taskError
      ? {
        code: optionalString(taskError.code),
        message: optionalString(taskError.message),
        retryable: typeof taskError.retryable === 'boolean' ? taskError.retryable : undefined,
        details: isObject(taskError.detail) ? taskError.detail : undefined,
      }
      : undefined,
    progress: task.progress,
    data: task,
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
