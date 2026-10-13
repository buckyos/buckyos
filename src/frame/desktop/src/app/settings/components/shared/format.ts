const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB']

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return '—'
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value >= 100 || unit === 0 ? Math.round(value) : Number(value.toFixed(1))} ${UNITS[unit]}`
}

/** `used / total` when both are known, otherwise whichever is. */
export function formatUsage(used: number | null | undefined, total: number | null | undefined): string {
  if (used != null && total != null && total > 0) {
    return `${formatBytes(used)} / ${formatBytes(total)} (${Math.round((used / total) * 100)}%)`
  }
  return total != null ? formatBytes(total) : formatBytes(used)
}

export function formatDateTime(value: string | number | null | undefined, locale: string): string {
  if (value == null || value === '') return '—'
  const date = typeof value === 'number' ? new Date(value * 1000) : new Date(value)
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString(locale)
}

export function formatDate(value: string | null | undefined, locale: string): string {
  if (!value) return '—'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleDateString(locale)
}

/** "3 minutes ago" style text for a Unix-seconds timestamp. */
export function formatRelative(unixSeconds: number | null | undefined, locale: string): string {
  if (unixSeconds == null || unixSeconds <= 0) return '—'
  const seconds = Math.round(unixSeconds - Date.now() / 1000)
  const format = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' })
  const abs = Math.abs(seconds)
  if (abs < 60) return format.format(seconds, 'second')
  if (abs < 3600) return format.format(Math.round(seconds / 60), 'minute')
  if (abs < 86_400) return format.format(Math.round(seconds / 3600), 'hour')
  return format.format(Math.round(seconds / 86_400), 'day')
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}

export function downloadText(filename: string, text: string, type = 'application/json') {
  const url = URL.createObjectURL(new Blob([text], { type }))
  downloadUrl(url, filename)
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
}

export function downloadUrl(url: string, filename: string) {
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.rel = 'noopener'
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
}
