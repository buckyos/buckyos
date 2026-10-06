import { isTransferableRef, isTransferableSession, parentSource, type OpenRequest, type TransferableContentRef } from 'buckyos/content'
export function parseLaunch(url: string): OpenRequest | undefined {
  const params = new URL(url).searchParams; const raw = params.get('src')
  if (!raw) return undefined
  const source: TransferableContentRef = raw.startsWith('obj://') ? { kind: 'object-id', objectId: raw.slice(6) } : { kind: 'cyfs-path', path: raw }
  if (!isTransferableRef(source)) throw new Error('invalid-source')
  const mode = params.get('mode')
  if (mode && mode !== 'edit' && mode !== 'view') throw new Error('invalid-mode')
  const request: OpenRequest = { requestId: params.get('requestId') || crypto.randomUUID(), source, mode: mode === 'view' ? 'view' : 'edit' }
  const encoded = params.get('session')
  if (encoded) {
    try {
      const session = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(encoded.replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0))))
      if (isTransferableSession(session)) request.session = session
    } catch {}
  }
  request.session ??= implicitSession(source)
  return request
}
export function implicitSession(source: TransferableContentRef) {
  const parent = parentSource(source)
  return parent ? { kind: 'container' as const, container: parent, current: source } : { kind: 'single' as const }
}
