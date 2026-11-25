import { isTransferableRef, isTransferableSession, systemOpenUrl } from 'buckyos/content'
import type { PreviewLaunchPayload } from './types'

const PATH_LIMIT = 4096

/** The address of what Preview shows (`/?open=…&session=…`, as `systemOpenUrl` writes it); undefined for content that has none, such as a draft. */
export function previewPath({ source, session }: Pick<PreviewLaunchPayload, 'source' | 'session'>): string | undefined {
  if (!isTransferableRef(source)) return undefined
  const write = (shown: PreviewLaunchPayload['session']) => {
    const url = new URL(systemOpenUrl('https://zone/', { request: { source, ...(isTransferableSession(shown) ? { session: shown } : {}) }, target: 'preview', hints: {} }))
    url.searchParams.delete('intent')
    return `/${url.search}`
  }
  // A long browsing list does not fit an address; the item itself does.
  return [write(session), write(undefined)].find((path) => path.length <= PATH_LIMIT)
}
