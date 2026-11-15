/* The online items of extension packages, as Settings › Extension tools and the Agent Browser show them: what a
 * package only declares and each node downloads from the upstream (doc/app_service/paios容器需求.md §6.4). */

import type { ExtToolsOnlineStatus, ExtToolsPackage, ExtToolsStatusResponse } from '../../../api/settings'
import type { useI18n } from '../../../i18n/provider'

type Translate = ReturnType<typeof useI18n>['t']

/** How often `ext_tools.status` is read while a download shown is under way or asked for. */
export const ONLINE_POLL_MS = 2500
/** A node takes a request on its next round (about 15 seconds); a report that has not moved after this is the answer. */
const REQUEST_WAIT_MS = 60_000

/** A download asked of one node, and what that node reported before: until its report moves, it has not taken it. */
export interface OnlineRequest {
  at: number
  before: ExtToolsOnlineStatus | null
}

export function onlineRequest(before: ExtToolsOnlineStatus | null | undefined): OnlineRequest {
  return { at: Date.now(), before: before ?? null }
}

export function requestPending(request: OnlineRequest | undefined, status: ExtToolsOnlineStatus | undefined): boolean {
  if (!request || Date.now() - request.at > REQUEST_WAIT_MS) return false
  if (!status) return true
  if (status.state === 'downloading') return false
  const { before } = request
  if (!before) return status.state === 'missing'
  return status.state === before.state && status.updated_at_ms === before.updated_at_ms && (status.error ?? null) === (before.error ?? null)
}

/** Under way, or asked for and not taken yet. */
export function onlineBusy(status: ExtToolsOnlineStatus | undefined, request: OnlineRequest | undefined): boolean {
  return status?.state === 'downloading' || requestPending(request, status)
}

export function onlinePercent(status: ExtToolsOnlineStatus): number | null {
  return status.size_bytes > 0 ? Math.min(100, Math.floor((status.downloaded_bytes * 100) / status.size_bytes)) : null
}

/** `the browser` for `chromium`; the item's id when it has no name of its own. */
export function onlineItemName(item: string, t: Translate): string {
  return t(`settings.extTools.online.name.${item}`, item)
}

/** Not downloaded / Downloading 42% / Ready / Download failed; `pending`: asked for and not taken yet. */
export function onlineStateLabel(status: ExtToolsOnlineStatus, pending: boolean, t: Translate): string {
  if (pending) return t('settings.extTools.online.requested', 'Asked the node')
  const percent = onlinePercent(status)
  switch (status.state) {
    case 'missing':
      return t('settings.extTools.online.missing', 'Not downloaded')
    case 'downloading':
      return percent === null ? t('settings.extTools.online.downloadingPlain', 'Downloading') : t('settings.extTools.online.downloading', 'Downloading {{percent}}%', { percent })
    case 'ready':
      return t('settings.extTools.online.ready', 'Ready')
    case 'failed':
      return t('settings.extTools.online.failed', 'Download failed')
  }
}

/** What went wrong with the last download, in a sentence; a `ready` one is still in use. */
export function onlineErrorText(status: ExtToolsOnlineStatus, t: Translate): string | null {
  if (!status.error || status.state === 'downloading') return null
  return status.state === 'ready'
    ? t('settings.extTools.online.againFailed', 'Downloading it again failed: {{error}}. The copy in use is kept.', { error: status.error })
    : status.error
}

export function onlineBadge(status: ExtToolsOnlineStatus, pending: boolean): 'pass' | 'warn' | 'fail' | 'info' | 'neutral' {
  if (pending || status.state === 'downloading') return 'info'
  if (status.state === 'ready') return status.error ? 'warn' : 'pass'
  return status.state === 'failed' ? 'fail' : 'neutral'
}

/** The package's online items: those its published version declares and those its nodes report. */
export function onlineItemsOf(pkg: ExtToolsPackage): string[] {
  const declared = pkg.latest?.manifest.online?.map((item) => item.id) ?? []
  const reported = Object.values(pkg.nodes ?? {}).flatMap((node) => Object.keys(node.online ?? {}))
  return [...new Set([...declared, ...reported])].sort()
}

/** command -> the online items it waits for, on any node and tools root. */
export function waitingOf(pkg: ExtToolsPackage): Map<string, string[]> {
  const waiting = new Map<string, string[]>()
  for (const node of Object.values(pkg.nodes ?? {})) {
    for (const target of Object.values(node.targets ?? {})) {
      for (const [command, items] of Object.entries(target.waiting ?? {})) {
        waiting.set(command, [...new Set([...(waiting.get(command) ?? []), ...items])])
      }
    }
  }
  return waiting
}

/** The package that declares online item `item`, as the nodes report it. */
export function packageOfItem(status: ExtToolsStatusResponse | null, item: string): ExtToolsPackage | null {
  return status?.packages.find((pkg) => Object.values(pkg.nodes ?? {}).some((node) => node.online?.[item])) ?? null
}
