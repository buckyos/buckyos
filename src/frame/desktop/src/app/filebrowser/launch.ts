import type { WindowLaunch } from '../../models/ui'

/** A request to show a folder in Files, e.g. the Desktop folder or a folder on the desktop, or to add files to the user's own. */
export interface FilesOpenPayload {
  kind: 'files-open'
  /** As Files shows it, e.g. `/home/alice/Desktop`. */
  path?: string
  /** On a phone: the sheet that asks what to upload. */
  upload?: boolean
}

export interface FilesOpenRequest { requestId: string; path?: string; upload?: boolean }

export function filesOpenLaunch(path: string): WindowLaunch {
  return { requestId: crypto.randomUUID(), payload: { kind: 'files-open', path } satisfies FilesOpenPayload }
}

/** What a Files window was opened (or re-targeted) with. */
export function filesOpenRequest(launch: WindowLaunch | undefined): FilesOpenRequest | undefined {
  const payload = launch?.payload as Partial<FilesOpenPayload> | undefined
  if (!launch || payload?.kind !== 'files-open') return undefined
  const path = typeof payload.path === 'string' && payload.path ? payload.path : undefined
  if (!path && payload.upload !== true) return undefined
  return { requestId: launch.requestId, ...(path ? { path } : {}), ...(payload.upload === true ? { upload: true } : {}) }
}
