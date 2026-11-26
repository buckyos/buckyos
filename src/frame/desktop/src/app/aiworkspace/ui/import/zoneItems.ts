/* Zone files handed to the Workspace (剪贴板规范 §2.2, §7 cases 1–2): a paste or drop of Zone references, the Shell's
 * delivery and "Import to AI Workspace" all name Zone content; its bytes are read here with the user's own session
 * (a path as the system file picker reads it, an object through the Preview resolver) and then go the way of a file
 * from this device — the canvas's drop path, a directory's upload. A reference grants nothing: a read the user may
 * not make fails here. */

import type { ZoneContent } from 'buckyos/content'
import { ensurePreviewProvider } from '../../../../components/preview/provider'
import { readZoneFiles } from '../../../../sysdlg/select/zoneFiles'
import { installFileBrowserData } from '../../../filebrowser/data/install'

/** The display name of a Zone item: its hint, else the last segment of its address. */
export function zoneItemName(item: ZoneContent): string {
  if (item.hints.name) return item.hints.name
  const tail = item.source.kind === 'cyfs-path' ? item.source.path : item.source.innerPath ?? item.source.objectId
  return tail.split('/').filter(Boolean).pop() ?? ''
}

async function readObject(item: ZoneContent, signal?: AbortSignal): Promise<File> {
  const provider = await ensurePreviewProvider()
  const resolved = await provider.resolvePreviewSource(item.source, { signal })
  const ref = resolved.readRef
  const blob = ref.kind === 'blob' ? ref.blob : await fetch(ref.url, { signal }).then((r) => { if (!r.ok) throw new Error(`HTTP ${r.status}`); return r.blob() })
  return new File([blob], item.hints.name ?? resolved.displayName ?? zoneItemName(item), { type: item.hints.mime ?? blob.type })
}

/** The bytes of the Zone files among `items`, in order (folders are left out: see the callers). */
export async function readZoneItems(items: ZoneContent[], signal?: AbortSignal): Promise<File[]> {
  installFileBrowserData()
  const files: File[] = []
  for (const item of items) {
    if (item.kind !== 'file') continue
    signal?.throwIfAborted()
    if (item.source.kind === 'cyfs-path') files.push(...await readZoneFiles([item], signal))
    else files.push(await readObject(item, signal))
  }
  return files
}
