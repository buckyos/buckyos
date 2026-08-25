import type { CommandContext } from './context.ts'
import { EXIT_INTERNAL, ToolError, UsageError } from './errors.ts'

export function rpcOptions(ctx: CommandContext): {
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

export async function callService<T = unknown>(
  ctx: CommandContext,
  service: string,
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  try {
    return await ctx.clients.call<T>(service, method, params, rpcOptions(ctx))
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    const stable = message.match(/\b((?:TASK|AUDIT|DIAGNOSTIC|LOG)_[A-Z0-9_]+):/)
    if (stable) throw new ToolError(stable[1], message)
    throw error
  }
}

export function expectObject(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', `${label} must be an object`, EXIT_INTERNAL)
  }
  return value as Record<string, unknown>
}

export function expectString(value: unknown, label: string): string {
  if (typeof value !== 'string' || !value.trim()) {
    throw new ToolError('INVALID_SERVICE_RESPONSE', `${label} must be a string`, EXIT_INTERNAL)
  }
  return value
}

export function inputString(input: Record<string, unknown>, key: string): string | undefined {
  const value = input[key]
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

export function requiredInputString(input: Record<string, unknown>, key: string): string {
  const value = inputString(input, key)
  if (!value) throw new UsageError('INVALID_ARGUMENT', `${key} is required`)
  return value
}

export function parseTimestamp(value: unknown, key: string): number | undefined {
  if (value === undefined) return undefined
  if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) return value
  if (typeof value !== 'string' || !value.trim()) {
    throw new UsageError('INVALID_ARGUMENT', `${key} must be RFC 3339 or Unix milliseconds`)
  }
  if (/^\d+$/.test(value)) {
    const milliseconds = Number(value)
    if (Number.isSafeInteger(milliseconds)) return milliseconds
  }
  const milliseconds = Date.parse(value)
  if (!Number.isFinite(milliseconds)) {
    throw new UsageError('INVALID_ARGUMENT', `${key} must be RFC 3339 or Unix milliseconds`)
  }
  return milliseconds
}

export function splitServices(value: unknown): string[] {
  const items = Array.isArray(value) ? value : typeof value === 'string' ? value.split(',') : []
  return [...new Set(items.map((item) => String(item).trim()).filter(Boolean))]
}
