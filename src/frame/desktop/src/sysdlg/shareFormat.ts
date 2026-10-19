/** Formatting shared by the share dialog and My Content. */

import type { ShareError, ShareInfo } from '../api/shares'

type T = (key: string, fallback?: string, vars?: Record<string, string | number>) => string

export const DAY = 86400

export type ExpiryChoice = '1d' | '7d' | '30d' | 'never' | 'custom'

export const EXPIRY_CHOICES: ExpiryChoice[] = ['1d', '7d', '30d', 'never', 'custom']

const EXPIRY_SECONDS: Record<Exclude<ExpiryChoice, 'never' | 'custom'>, number> = { '1d': DAY, '7d': 7 * DAY, '30d': 30 * DAY }

export const nowSeconds = () => Math.floor(Date.now() / 1000)

/** Absolute expiry (unix seconds) for a choice; `custom` is a `yyyy-mm-dd` local date (end of that day). */
export function expiryFor(choice: ExpiryChoice, customDate?: string): number | null {
  if (choice === 'never') return null
  if (choice === 'custom') {
    if (!customDate) return nowSeconds() + 7 * DAY
    const end = new Date(`${customDate}T23:59:59`)
    return Math.floor(end.getTime() / 1000)
  }
  return nowSeconds() + EXPIRY_SECONDS[choice]
}

export function localDateInput(offsetDays: number) {
  const d = new Date(Date.now() + offsetDays * DAY * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

export const timeZone = () => {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}

export function formatBytes(n?: number | null): string {
  if (n == null) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${i ? v.toFixed(v < 10 ? 1 : 0) : v} ${units[i]}`
}

export function formatTime(ts?: number | null, locale?: string): string {
  if (!ts) return '—'
  return new Date(ts * 1000).toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short' })
}

export function formatDate(ts?: number | null, locale?: string, utc = false): string {
  if (!ts) return '—'
  return new Date(ts * 1000).toLocaleDateString(locale, { month: 'short', day: 'numeric', ...(utc ? { timeZone: 'UTC' } : {}) })
}

export function appName(app?: string | null): string {
  if (app === 'messagehub') return 'MessageHub'
  if (app === 'homestation') return 'HomeStation'
  return app ?? ''
}

export function accessLabel(t: T, share: Pick<ShareInfo, 'access' | 'kind' | 'target_app'>): string {
  if (share.kind === 'app_delivery') return appName(share.target_app)
  return share.access === 'password' ? t('share.dialog.access.password') : t('share.dialog.access.holder')
}

export function expiryLabel(t: T, expiresAt: number | null | undefined, locale?: string): string {
  if (expiresAt == null) return t('share.dialog.noExpiry')
  return formatTime(expiresAt, locale)
}

export function shareErrorMessage(t: T, err: ShareError | null | undefined): string {
  if (!err) return ''
  if (err.reason) {
    const key = `share.error.${err.reason}`
    const text = t(key, '')
    if (text && text !== key) return text
  }
  if (err.code === 'PERMISSION_DENIED') return t('share.error.source_not_readable')
  return t('share.error.generic', 'Something went wrong: {{message}}', { message: err.message })
}

/** DFS display path from `cyfs:///home/a/b`. */
export function displaySourcePath(sourcePath: string): string {
  return sourcePath.replace(/^cyfs:\/\//, '') || '/'
}

export function statusLabel(t: T, status: string): string {
  return t(`myContent.status.${status}`, status)
}
