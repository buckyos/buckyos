/**
 * URL downloads in mock mode. A download's progress is a function of the
 * clock (it runs for `durationMs` from `createdAt`), so every reader —
 * the Task Center and the status-bar bell, in any tab — sees the same state
 * without a timer driving it. The list lives in localStorage; a change made
 * in one tab reaches the others as a `storage` event.
 *
 * A URL containing `fail` stops with an error halfway through.
 */

export interface MockDownload {
  taskId: string
  url: string
  filename: string
  createdAt: number
  durationMs: number
  totalBytes: number
  canceledAt?: number
}

export interface MockDownloadState {
  phase: 'Running' | 'Terminal'
  outcome?: 'Succeeded' | 'Failed' | 'Canceled'
  completedBytes: number
  updatedAt: number
  completedAt?: number
  error?: string
}

const STORAGE_KEY = 'buckyos.desktop.mock-downloads.v1'
const DEFAULT_DURATION_MS = 8_000
const FAIL_FRACTION = 0.5

const listeners = new Set<() => void>()
let storageAttached = false

function read(): MockDownload[] {
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? '[]')
    return Array.isArray(parsed) ? (parsed as MockDownload[]) : []
  } catch {
    return []
  }
}

function write(downloads: MockDownload[]) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(downloads))
  } catch {
    // Not persisted (private window): the change is lost on reload.
  }
  listeners.forEach((listener) => listener())
}

export function listMockDownloads(): MockDownload[] {
  return read()
}

export function subscribeMockDownloads(listener: () => void): () => void {
  if (!storageAttached && typeof window !== 'undefined') {
    storageAttached = true
    window.addEventListener('storage', (event) => {
      if (event.key === STORAGE_KEY) listeners.forEach((item) => item())
    })
  }
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function filenameFromUrl(url: string): string {
  try {
    const segment = new URL(url).pathname.split('/').filter(Boolean).pop()
    return segment ? decodeURIComponent(segment) : 'download'
  } catch {
    return 'download'
  }
}

export function createMockDownload(input: { url: string; filename?: string }): MockDownload {
  const download: MockDownload = {
    taskId: `mock-dl-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`,
    url: input.url,
    filename: input.filename?.trim() || filenameFromUrl(input.url),
    createdAt: Date.now(),
    durationMs: DEFAULT_DURATION_MS,
    totalBytes: 48 * 1024 * 1024,
  }
  write([...read(), download])
  return download
}

export function cancelMockDownload(taskId: string): void {
  const now = Date.now()
  write(
    read().map((download) =>
      download.taskId === taskId &&
      !download.canceledAt &&
      mockDownloadState(download, now).phase !== 'Terminal'
        ? { ...download, canceledAt: now }
        : download,
    ),
  )
}

export function mockDownloadState(download: MockDownload, now: number): MockDownloadState {
  const fails = download.url.toLowerCase().includes('fail')
  const end = download.createdAt + download.durationMs * (fails ? FAIL_FRACTION : 1)
  const stoppedAt = Math.min(now, download.canceledAt ?? Infinity, end)
  const fraction = Math.max(0, Math.min(1, (stoppedAt - download.createdAt) / download.durationMs))
  const completedBytes = Math.round(download.totalBytes * fraction)
  if (download.canceledAt !== undefined && download.canceledAt < end) {
    return { phase: 'Terminal', outcome: 'Canceled', completedBytes, updatedAt: download.canceledAt, completedAt: download.canceledAt }
  }
  if (now < end) return { phase: 'Running', completedBytes, updatedAt: now }
  if (fails) {
    return { phase: 'Terminal', outcome: 'Failed', completedBytes, updatedAt: end, completedAt: end, error: 'The server closed the connection.' }
  }
  return { phase: 'Terminal', outcome: 'Succeeded', completedBytes, updatedAt: end, completedAt: end }
}
