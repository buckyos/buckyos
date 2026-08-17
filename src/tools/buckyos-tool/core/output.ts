import type { OutputFormat } from './argv.ts'
import type { ToolError } from './errors.ts'

export interface EnvelopeMeta {
  command: string
  trace_id: string
  duration_ms: number
}

export interface SuccessEnvelope {
  schema_version: 1
  ok: true
  data: unknown
  meta: EnvelopeMeta
}

export interface ErrorEnvelope {
  schema_version: 1
  ok: false
  error: {
    code: string
    message: string
    retryable: boolean
    details: Record<string, unknown>
  }
  meta: Omit<EnvelopeMeta, 'duration_ms'> & { duration_ms?: number }
}

export function successEnvelope(data: unknown, meta: EnvelopeMeta): SuccessEnvelope {
  return { schema_version: 1, ok: true, data, meta }
}

export function errorEnvelope(
  error: ToolError,
  meta: Omit<EnvelopeMeta, 'duration_ms'> & { duration_ms?: number },
): ErrorEnvelope {
  return {
    schema_version: 1,
    ok: false,
    error: {
      code: error.code,
      message: error.message,
      retryable: error.retryable,
      details: error.details,
    },
    meta,
  }
}

export function renderSuccess(envelope: SuccessEnvelope, format: OutputFormat): string {
  if (format === 'json' || format === 'jsonl') return JSON.stringify(envelope)
  if (format === 'raw') {
    if (typeof envelope.data !== 'string') throw new Error('raw output requires a string result')
    return envelope.data
  }
  if (format === 'text') return renderText(envelope.data)
  return renderTable(envelope.data)
}

export function renderError(envelope: ErrorEnvelope, format: OutputFormat): string {
  if (format === 'json' || format === 'jsonl' || format === 'raw') return JSON.stringify(envelope)
  const retryable = envelope.error.retryable ? ' (retryable)' : ''
  return `${envelope.error.code}${retryable}: ${envelope.error.message}`
}

function renderText(data: unknown): string {
  if (typeof data === 'string') return data
  if (data && typeof data === 'object' && !Array.isArray(data)) {
    const object = data as Record<string, unknown>
    if (typeof object.script === 'string') return object.script
    if (typeof object.text === 'string') return object.text
  }
  return JSON.stringify(data, null, 2)
}

function renderTable(data: unknown): string {
  if (Array.isArray(data)) return renderRows(data)
  if (data && typeof data === 'object') {
    const object = data as Record<string, unknown>
    const arrayEntry = Object.entries(object).find(([, value]) => Array.isArray(value))
    if (arrayEntry) return renderRows(arrayEntry[1] as unknown[])
    const rows = Object.entries(object).map(([key, value]) => ({ key, value: displayValue(value) }))
    return renderRows(rows)
  }
  return String(data ?? '')
}

function renderRows(rows: unknown[]): string {
  if (rows.length === 0) return '(empty)'
  const objects = rows.map((row) =>
    row && typeof row === 'object' && !Array.isArray(row)
      ? row as Record<string, unknown>
      : { value: row }
  )
  const columns = [...new Set(objects.flatMap((row) => Object.keys(row)))]
  const rendered = objects.map((row) => columns.map((column) => displayValue(row[column])))
  const widths = columns.map((column, index) =>
    Math.max(column.length, ...rendered.map((row) => row[index].length))
  )
  const header = columns.map((column, index) => column.padEnd(widths[index])).join('  ')
  const separator = widths.map((width) => '-'.repeat(width)).join('  ')
  const body = rendered.map((row) =>
    row.map((value, index) => value.padEnd(widths[index])).join('  ')
  )
  return [header, separator, ...body].join('\n')
}

function displayValue(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return JSON.stringify(value)
}
