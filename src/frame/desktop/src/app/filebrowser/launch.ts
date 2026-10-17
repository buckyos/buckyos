import type { WindowLaunch } from '../../models/ui'

/** A request to show a folder in Files, e.g. the Desktop folder or a folder on the desktop. */
export interface FilesOpenPayload {
  kind: 'files-open'
  /** As Files shows it, e.g. `/home/alice/Desktop`. */
  path: string
}

export function filesOpenLaunch(path: string): WindowLaunch {
  return { requestId: crypto.randomUUID(), payload: { kind: 'files-open', path } satisfies FilesOpenPayload }
}

/** The folder a Files window was opened (or re-targeted) with. */
export function filesOpenRequest(launch: WindowLaunch | undefined): { requestId: string; path: string } | undefined {
  const payload = launch?.payload as Partial<FilesOpenPayload> | undefined
  return launch && payload?.kind === 'files-open' && typeof payload.path === 'string' && payload.path
    ? { requestId: launch.requestId, path: payload.path }
    : undefined
}
