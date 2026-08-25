export const EXIT_SUCCESS = 0
export const EXIT_USAGE = 2
export const EXIT_AUTH = 3
export const EXIT_PERMISSION = 4
export const EXIT_UNAVAILABLE = 5
export const EXIT_OPERATION = 6
export const EXIT_PARTIAL = 7
export const EXIT_TIMEOUT = 8
export const EXIT_INTERNAL = 9

export class ToolError extends Error {
  readonly code: string
  readonly exitCode: number
  readonly retryable: boolean
  readonly details: Record<string, unknown>

  constructor(
    code: string,
    message: string,
    exitCode = EXIT_OPERATION,
    retryable = false,
    details: Record<string, unknown> = {},
  ) {
    super(message)
    this.name = 'ToolError'
    this.code = code
    this.exitCode = exitCode
    this.retryable = retryable
    this.details = details
  }
}

export class UsageError extends ToolError {
  constructor(code: string, message: string, details: Record<string, unknown> = {}) {
    super(code, message, EXIT_USAGE, false, details)
  }
}

const JWT_PATTERN = /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/g
const URL_CREDENTIAL_PATTERN = /(\w+:\/\/)[^\s/@:]+:[^\s/@]+@/g
const DATABASE_URI_PATTERN =
  /\b(?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|redis|rediss|sqlite):\/\/[^\s,;]+/gi

export function sanitizeMessage(message: string): string {
  return message
    .replaceAll(JWT_PATTERN, '[REDACTED_TOKEN]')
    .replaceAll(URL_CREDENTIAL_PATTERN, '$1[REDACTED]@')
    .replaceAll(DATABASE_URI_PATTERN, '[REDACTED_DATABASE_URI]')
    .replace(
      /(session[_-]?token|refresh[_-]?token|access[_-]?token|private[_-]?key|password|passwd|api[_-]?key|client[_-]?secret|secret)\s*[=:]\s*[^\s,;]+/gi,
      '$1=[REDACTED]',
    )
}

export function normalizeError(error: unknown): ToolError {
  if (error instanceof ToolError) {
    return new ToolError(
      error.code,
      sanitizeMessage(error.message),
      error.exitCode,
      error.retryable,
      error.details,
    )
  }

  const raw = error instanceof Error ? error.message : String(error)
  const message = sanitizeMessage(raw)
  const lower = message.toLowerCase()

  if (lower.includes('abort') || lower.includes('cancel')) {
    return new ToolError('CANCELED', 'operation canceled', EXIT_TIMEOUT)
  }
  if (lower.includes('timeout') || lower.includes('timed out')) {
    return new ToolError('TIMEOUT', 'operation timed out', EXIT_TIMEOUT, true)
  }
  if (
    lower.includes('permission denied') || lower.includes('no permission') ||
    lower.includes('rpc call error: 403')
  ) {
    return new ToolError('PERMISSION_DENIED', message, EXIT_PERMISSION)
  }
  if (
    lower.includes('rpc call error: 401') || lower.includes('unauthorized') ||
    lower.includes('token expired') || lower.includes('session expired') ||
    lower.includes('invalid token')
  ) {
    return new ToolError(
      lower.includes('expired') ? 'SESSION_EXPIRED' : 'INVALID_SESSION',
      lower.includes('expired') ? 'the session token has expired' : message,
      EXIT_AUTH,
    )
  }
  if (
    lower.includes('fetch failed') || lower.includes('connection refused') ||
    lower.includes('rpc call error: 502') || lower.includes('rpc call error: 503') ||
    lower.includes('rpc call error: 504')
  ) {
    return new ToolError('SERVICE_UNAVAILABLE', message, EXIT_UNAVAILABLE, true)
  }
  if (lower.includes('not found') || lower.includes('key not exist')) {
    return new ToolError('RESOURCE_NOT_FOUND', message, EXIT_OPERATION)
  }
  if (lower.includes('rpc call error:')) {
    return new ToolError('OPERATION_FAILED', message, EXIT_OPERATION)
  }
  return new ToolError('INTERNAL_ERROR', message || 'internal error', EXIT_INTERNAL)
}
