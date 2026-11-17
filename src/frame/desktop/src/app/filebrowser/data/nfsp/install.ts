/**
 * Wires the NFSP-backed data sources into the File Browser registries —
 * the real-backend counterpart of the mock registrations (UI_DATAMODEL.md
 * §8.1). Components keep consuming FolderReader/FileItemList and the
 * data-layer registries; no NFSP wire type crosses this boundary.
 */

import type { FileItem } from '../FolderReader'
import { registerMyHomeProvider } from '../myHome'
import { registerPreviewEnricher } from '../usePreview'
import { ensureSession, nfspClient } from './client'
import { registerNfspCollectionReader } from './collections'
import { registerNfspCollectionDirectory } from './collectionDirectory'
import { registerNfspFolderOps } from './folderOps'
import { registerNfspHostBackend } from './host'
import { applyMetaRecords } from './mapping'
import { registerNfspPermissionSource } from './permissions'
import { registerNfspReaders } from './readers'
import { registerNfspSearchProvider } from './search'
import { registerNfspSidebarSources } from './sidebar'
import { registerNfspTransferExecutor } from './transfers'

/** get_meta enrichment for the preview panel (§8.2 meta mapping). */
async function enrichPreviewItem(item: FileItem): Promise<FileItem> {
  // The computer's files carry no Zone meta (AI summary, tags, story).
  if (item.entry.path.startsWith('device://')) return item
  await ensureSession()
  const meta = await nfspClient()
    .getMeta(item.entry.path.startsWith('/') ? item.entry.path : { uri: item.entry.path })
    .catch(() => null)
  if (!meta || meta.records.length === 0) return item
  return { ...item, entry: applyMetaRecords(item.entry, meta.records) }
}

export function installNfspFileBrowserData() {
  registerNfspReaders()
  registerNfspCollectionReader()
  registerNfspCollectionDirectory()
  registerNfspSidebarSources()
  registerNfspSearchProvider()
  registerNfspTransferExecutor()
  registerNfspFolderOps()
  registerNfspPermissionSource()
  registerNfspHostBackend()
  registerPreviewEnricher(enrichPreviewItem)
  registerMyHomeProvider(async () => {
    const userId = (await ensureSession()).identity?.user_id ?? null
    return { path: userId ? `/home/${userId}` : '/home', userId }
  })
  // Establish the session (and auto-watch) eagerly so the first listings and
  // watch invalidation don't wait for a lazy hello.
  void ensureSession().catch(() => {
    // Surfaced per-source by the readers when the user actually navigates.
  })
}
